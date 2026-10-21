//! Host tests of the console host through the services, against the fake
//! kernel of `crate::tests`: console handles opened by name, reads that
//! wait for input, writes, the control codes, control events and the end
//! of a console.

extern crate std;

use super::*;
use crate::io::default_volumes;
use crate::services::Service;
use crate::tests::Fake;
use alloc::boxed::Box;
use alloc::string::String;
use poly_console::consts::*;
use poly_ntobj::OBJ_CASE_INSENSITIVE;
use poly_vfs::{Clock, Timespec, Tmpfs, Vfs};

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_000_000_000, 0)
    }
}

struct T {
    f: Fake,
}

fn setup() -> T {
    let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))))));
    let mut f = Fake::new();
    let ctx = vfs.root_context().unwrap();
    let vols = default_volumes(vfs, &ctx);
    f.sys.io.attach(&mut f.sys.objects, vfs, &vols).unwrap();
    T { f }
}

const RW: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;

impl T {
    /// Open `name` (relative to `root` if non-zero); `sync` for a
    /// synchronous handle.
    fn open(&mut self, root: u64, name: &str, sync: bool) -> (NtStatus, u64) {
        let out = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let oa = if root == 0 {
            self.f.attrs(name, OBJ_CASE_INSENSITIVE)
        } else {
            self.f.attrs_in(root, name, OBJ_CASE_INSENSITIVE)
        };
        let options = if sync { FILE_SYNCHRONOUS_IO_NONALERT } else { 0 };
        let s = self.f.call(Service::OpenFile, &[out, u64::from(RW), oa, iosb, 3, u64::from(options)]);
        (s, self.f.u64_at(out))
    }

    fn console(&mut self) -> (u64, u64, u64, u32) {
        let (s, conn) = self.open(0, "\\Device\\PolyConsole\\New", true);
        assert_eq!(s, STATUS_SUCCESS);
        let (s, input) = self.open(conn, "Input", false);
        assert_eq!(s, STATUS_SUCCESS);
        let (s, output) = self.open(conn, "Output", true);
        assert_eq!(s, STATUS_SUCCESS);
        let (s, id) = self.ioctl(conn, ioctl::GET_ID, &[], 4);
        assert_eq!(s, STATUS_SUCCESS);
        (conn, input, output, u32::from_le_bytes(id[..4].try_into().unwrap()))
    }

    fn ioctl(&mut self, h: u64, code: u32, input: &[u8], outlen: usize) -> (NtStatus, Vec<u8>) {
        let inp = if input.is_empty() { 0 } else { self.f.put(input) };
        let out = self.f.alloc(outlen.max(1));
        let iosb = self.f.alloc(16);
        let s = self.f.call(
            Service::DeviceIoControlFile,
            &[h, 0, 0, 0, iosb, u64::from(code), inp, input.len() as u64, out, outlen as u64],
        );
        self.drain();
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(out, n.min(outlen)))
    }

    /// Deliver completions (the kernel's part).
    fn drain(&mut self) {
        while let Some(c) = self.f.sys.io.pop_completion() {
            if let Some((addr, data)) = &c.data {
                self.f.mem.write_bytes(*addr, data).unwrap();
            }
            if c.iosb != 0 {
                self.f.mem.write_bytes(c.iosb, &c.iosb_bytes()).unwrap();
            }
            let _ = self.f.sys.io.finish(&mut self.f.sys.objects, &c);
        }
    }

    fn mode(&mut self, h: u64) -> u32 {
        let (s, b) = self.ioctl(h, ioctl::GET_MODE, &[], 4);
        assert_eq!(s, STATUS_SUCCESS);
        u32::from_le_bytes(b[..4].try_into().unwrap())
    }

    fn output(&mut self) -> String {
        let out: Vec<u8> = self.f.sys.io.take_console_output().into_iter().flat_map(|(_, b)| b).collect();
        String::from_utf8(out).unwrap()
    }
}

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

#[test]
fn new_console_handles_and_modes() {
    let mut t = setup();
    let (conn, input, output, id) = t.console();
    assert_eq!(t.f.sys.io.take_console_events(), [ConsoleEvent::Created { id }]);
    assert_eq!(t.mode(input), poly_console::mode::DEFAULT_INPUT_MODE);
    assert_eq!(t.mode(output), poly_console::mode::DEFAULT_OUTPUT_MODE);
    assert_eq!(t.ioctl(conn, ioctl::GET_MODE, &[], 4).0, STATUS_INVALID_HANDLE);
    assert_eq!(t.ioctl(input, ioctl::SET_MODE, &0x7u32.to_le_bytes(), 0).0, STATUS_SUCCESS);
    assert_eq!(t.mode(input) & 0x7, 0x7);
    assert_eq!(t.ioctl(input, ioctl::SET_MODE, &ENABLE_ECHO_INPUT.to_le_bytes(), 0).0, STATUS_INVALID_PARAMETER);
    // The device type: FILE_DEVICE_CONSOLE.
    let buf = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let s =
        t.f.call(Service::QueryVolumeInformationFile, &[output, iosb, buf, 8, u64::from(FILE_FS_DEVICE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf), 0x50);
    // Unknown relative names and screen requests on the input fail.
    assert_eq!(t.open(conn, "Nope", true).0, STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(t.ioctl(input, ioctl::GET_INFO, &[], 96).0, STATUS_INVALID_HANDLE);
}

#[test]
fn writes_reach_the_screen_and_the_terminal() {
    let mut t = setup();
    let (_, _, output, id) = t.console();
    let (s, n) = t.ioctl(output, ioctl::WRITE_W, &wide("héllo\r\n"), 4);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(n, 7u32.to_le_bytes());
    let o = t.output();
    assert!(o.ends_with("héllo\x1b[2;1H"), "{o:?}");
    // NtWriteFile: the output code page (437 at first).
    let buf = t.f.put(&[0x82, b'!']);
    let iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[output, 0, 0, 0, iosb, buf, 2, 0, 0]), STATUS_SUCCESS);
    assert_eq!(t.f.sys.io.console(id).unwrap().screen.row_text(1), "é!");
    let (s, info) = t.ioctl(output, ioctl::GET_INFO, &[], 96);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(&info[8..12], &[2, 0, 1, 0]);
    assert_eq!(t.ioctl(output, ioctl::SET_CURSOR, &[99, 0, 0, 0], 0).0, STATUS_INVALID_PARAMETER);
    let fill = ioctl::Fill { kind: ioctl::FILL_CHARACTER, value: u32::from(b'#'), count: 3, x: 0, y: 2 };
    let (s, n) = t.ioctl(output, ioctl::FILL, &fill.to_bytes(), 4);
    assert_eq!((s, n), (STATUS_SUCCESS, 3u32.to_le_bytes().to_vec()));
    assert_eq!(t.f.sys.io.console(id).unwrap().screen.row_text(2), "###");
}

#[test]
fn a_read_waits_for_its_line() {
    let mut t = setup();
    let (_, input, _, id) = t.console();
    // An asynchronous input handle: the read is pending.
    let buf = t.f.alloc(64);
    let iosb = t.f.alloc(16);
    let s = t.f.call(Service::ReadFile, &[input, 0, 0, 0, iosb, buf, 64, 0, 0]);
    assert_eq!(s, STATUS_PENDING);
    t.f.sys.io.console_input(id, b"dir");
    t.drain();
    assert_eq!(t.f.u64_at(iosb + 8), 0);
    t.f.sys.io.console_input(id, b"\r");
    t.drain();
    assert_eq!(t.f.u32_at(iosb), 0);
    assert_eq!(t.f.u64_at(iosb + 8), 5);
    assert_eq!(t.f.bytes_at(buf, 5), b"dir\r\n");
    // Typed ahead: a Unicode read finishes at once.
    t.f.sys.io.console_input(id, b"ok\r");
    let (s, text) = t.ioctl(input, ioctl::READ_W, &[], 16);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(text, wide("ok\r\n"));
}

#[test]
fn input_records_and_ctrl_c() {
    let mut t = setup();
    let (conn, input, _, id) = t.console();
    assert_eq!(t.ioctl(conn, ioctl::SET_CTRL_ROUTINE, &0x1234_5678u64.to_le_bytes(), 0).0, STATUS_SUCCESS);
    t.f.sys.io.take_console_events();
    t.f.sys.io.console_input(id, b"q");
    let (s, n) = t.ioctl(input, ioctl::INPUT_COUNT, &[], 4);
    assert_eq!((s, n), (STATUS_SUCCESS, 2u32.to_le_bytes().to_vec()));
    let (s, peek) = t.ioctl(input, ioctl::READ_INPUT, &0u32.to_le_bytes(), 40);
    assert_eq!((s, peek.len()), (STATUS_SUCCESS, 40));
    let flags = ioctl::READ_INPUT_REMOVE | ioctl::READ_INPUT_WAIT;
    let (s, rec) = t.ioctl(input, ioctl::READ_INPUT, &flags.to_le_bytes(), 20);
    assert_eq!(s, STATUS_SUCCESS);
    let r = InputRecord::from_bytes(rec[..20].try_into().unwrap());
    assert_eq!(r.key().map(|k| (k.down, k.ch)), Some((true, u16::from(b'q'))));
    // Ctrl+C: a control event for the attached process, not input.
    t.f.sys.io.console_input(id, b"\x03");
    let events = t.f.sys.io.take_console_events();
    assert!(matches!(events.as_slice(), [ConsoleEvent::Ctrl { routine: 0x1234_5678, event: CTRL_C_EVENT, .. }]));
    assert_eq!(t.ioctl(input, ioctl::INPUT_COUNT, &[], 4).1, 1u32.to_le_bytes());
    assert_eq!(t.ioctl(input, ioctl::FLUSH_INPUT, &[], 0).0, STATUS_SUCCESS);
    assert_eq!(t.ioctl(input, ioctl::INPUT_COUNT, &[], 4).1, 0u32.to_le_bytes());
}

#[test]
fn code_pages_and_titles() {
    let mut t = setup();
    let (conn, _, _, _) = t.console();
    assert_eq!(t.ioctl(conn, ioctl::GET_CP, &1u32.to_le_bytes(), 4).1, 437u32.to_le_bytes());
    let set: Vec<u8> = [1u32, 65001].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(t.ioctl(conn, ioctl::SET_CP, &set, 0).0, STATUS_SUCCESS);
    assert_eq!(t.ioctl(conn, ioctl::GET_CP, &1u32.to_le_bytes(), 4).1, 65001u32.to_le_bytes());
    let bad: Vec<u8> = [0u32, 7].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(t.ioctl(conn, ioctl::SET_CP, &bad, 0).0, STATUS_INVALID_PARAMETER);
    assert_eq!(t.ioctl(conn, ioctl::SET_TITLE, &wide("Poly"), 0).0, STATUS_SUCCESS);
    assert_eq!(t.ioctl(conn, ioctl::GET_TITLE, &[], 64), (STATUS_SUCCESS, wide("Poly")));
    assert_eq!(t.ioctl(conn, ioctl::GET_TITLE, &[], 4), (STATUS_BUFFER_OVERFLOW, wide("Po")));
}

#[test]
fn the_last_connection_closes_the_console() {
    let mut t = setup();
    let (conn, input, output, id) = t.console();
    let (s, second) = t.open(0, &alloc::format!("\\Device\\PolyConsole\\{id}"), true);
    assert_eq!(s, STATUS_SUCCESS);
    t.ioctl(output, ioctl::WRITE_W, &wide("x"), 4);
    t.f.sys.io.take_console_events();
    t.output();
    assert_eq!(t.f.call(Service::Close, &[conn]), STATUS_SUCCESS);
    assert!(t.f.sys.io.console(id).is_some());
    assert_eq!(t.f.call(Service::Close, &[second]), STATUS_SUCCESS);
    assert!(t.f.sys.io.console(id).is_none());
    let events = t.f.sys.io.take_console_events();
    match events.as_slice() {
        [ConsoleEvent::Closed { id: closed, output }] => {
            assert_eq!(*closed, id);
            assert!(output.starts_with(b"\x1b[0m\x1b[r"), "{output:?}");
        }
        other => panic!("{other:?}"),
    }
    // The other handles are now invalid.
    assert_eq!(t.ioctl(output, ioctl::GET_INFO, &[], 96).0, STATUS_INVALID_HANDLE);
    assert_eq!(t.ioctl(input, ioctl::INPUT_COUNT, &[], 4).0, STATUS_INVALID_HANDLE);
    assert_eq!(t.open(0, &alloc::format!("\\Device\\PolyConsole\\{id}"), true).0, STATUS_OBJECT_NAME_NOT_FOUND);
}

#[test]
fn ioctls_on_files_are_invalid_requests() {
    let mut t = setup();
    let (s, h) = t.open(0, "\\??\\C:\\", true);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.ioctl(h, ioctl::GET_MODE, &[], 4).0, STATUS_INVALID_DEVICE_REQUEST);
}

#[test]
fn open_for_process_gives_three_handles() {
    let mut t = setup();
    let id = t.f.sys.io.create_console(80, 25);
    let caller = crate::kernel::NtKernel::caller(&t.f);
    let [conn, input, output, error] = open_for_process(&mut t.f.sys, &mut t.f.mem, &caller, id).unwrap();
    assert!(conn.0 != input.0 && input.0 != output.0 && output.0 != error.0);
    assert_eq!(t.f.sys.io.take_console_events(), []);
    assert_eq!(t.f.sys.io.console_processes(id), [caller.process]);
}
