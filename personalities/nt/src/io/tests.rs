//! Host tests of NT file I/O through the services ([`crate::dispatch_with`])
//! on a real tmpfs VFS, against the fake kernel of `crate::tests`. The
//! kernel's part of asynchronous I/O — a worker running queued requests and
//! the delivery of completions — is played by [`run_worker`] and [`drain`].

extern crate std;

use super::consts::*;
use super::{IoState, default_volumes};
use crate::kernel::NtKernel;
use crate::services::Service;
use crate::tests::Fake;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_ntobj::status::*;
use poly_ntobj::{NtStatus, OBJ_CASE_INSENSITIVE};
use poly_vfs::{At, Clock, Cred, OpenFlags, Timespec, Tmpfs, Vfs};

const CI: u32 = OBJ_CASE_INSENSITIVE;
const RW: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const SYNC: u32 = FILE_SYNCHRONOUS_IO_NONALERT;

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_000_000_000, 0)
    }
}

struct T {
    f: Fake,
    vfs: &'static Vfs,
}

fn setup() -> T {
    let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))))));
    let mut f = Fake::new();
    let ctx = vfs.root_context().unwrap();
    let vols = default_volumes(vfs, &ctx);
    f.sys.io.attach(&mut f.sys.objects, vfs, &vols).unwrap();
    T { f, vfs }
}

impl T {
    /// `NtCreateFile` of `path` (an NT path): (status, handle, information).
    fn create(
        &mut self,
        path: &str,
        access: u32,
        share: u32,
        disp: u32,
        options: u32,
        attrs: u32,
    ) -> (NtStatus, u64, u64) {
        let out = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let oa = self.f.attrs(path, CI);
        let s = self.f.call(
            Service::CreateFile,
            &[
                out,
                u64::from(access),
                oa,
                iosb,
                0,
                u64::from(attrs),
                u64::from(share),
                u64::from(disp),
                u64::from(options),
                0,
                0,
            ],
        );
        (s, self.f.u64_at(out), self.f.u64_at(iosb + 8))
    }

    /// Create or open a file for synchronous read/write, sharing everything.
    fn file(&mut self, path: &str) -> u64 {
        let (s, h, _) = self.create(path, RW | DELETE, SHARE_ALL, FILE_OPEN_IF, SYNC | FILE_NON_DIRECTORY_FILE, 0);
        assert_eq!(s, STATUS_SUCCESS, "{path}");
        h
    }

    fn close(&mut self, h: u64) {
        assert_eq!(self.f.call(Service::Close, &[h]), STATUS_SUCCESS);
    }

    fn write(&mut self, h: u64, data: &[u8], offset: Option<i64>) -> (NtStatus, u64) {
        let buf = self.f.put(data);
        let iosb = self.f.alloc(16);
        let off = offset.map_or(0, |o| self.f.put_u64(o as u64));
        let s = self.f.call(Service::WriteFile, &[h, 0, 0, 0, iosb, buf, data.len() as u64, off, 0]);
        (s, self.f.u64_at(iosb + 8))
    }

    fn read(&mut self, h: u64, len: usize, offset: Option<i64>) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len.max(1));
        let iosb = self.f.alloc(16);
        let off = offset.map_or(0, |o| self.f.put_u64(o as u64));
        let s = self.f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, buf, len as u64, off, 0]);
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }

    fn query(&mut self, h: u64, class: u32, len: usize) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len);
        let iosb = self.f.alloc(16);
        let s = self.f.call(Service::QueryInformationFile, &[h, iosb, buf, len as u64, u64::from(class)]);
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }

    fn set(&mut self, h: u64, class: u32, data: &[u8]) -> NtStatus {
        let buf = self.f.put(data);
        let iosb = self.f.alloc(16);
        self.f.call(Service::SetInformationFile, &[h, iosb, buf, data.len() as u64, u64::from(class)])
    }

    fn lock(&mut self, h: u64, offset: u64, length: u64, key: u32, fail_now: bool, exclusive: bool) -> NtStatus {
        let iosb = self.f.alloc(16);
        let o = self.f.put_u64(offset);
        let l = self.f.put_u64(length);
        self.f.call(
            Service::LockFile,
            &[h, 0, 0, 0, iosb, o, l, u64::from(key), u64::from(fail_now), u64::from(exclusive)],
        )
    }

    fn unlock(&mut self, h: u64, offset: u64, length: u64, key: u32) -> NtStatus {
        let iosb = self.f.alloc(16);
        let o = self.f.put_u64(offset);
        let l = self.f.put_u64(length);
        self.f.call(Service::UnlockFile, &[h, iosb, o, l, u64::from(key)])
    }

    /// The VFS's view: whether `path` exists.
    fn exists(&self, path: &[u8]) -> bool {
        let ctx = self.vfs.root_context().unwrap();
        self.vfs.lookup(&ctx, &Cred::root(), At::Cwd, path, false).is_ok()
    }

    fn vfs_create(&self, path: &[u8], data: &[u8]) {
        let ctx = self.vfs.root_context().unwrap();
        let f = self.vfs.open(&ctx, &Cred::root(), At::Cwd, path, OpenFlags::CREAT | OpenFlags::RDWR, 0o644).unwrap();
        f.write(data).unwrap();
    }

    /// Run device `dev`'s queue the way the kernel's worker would.
    fn run_worker(&mut self, dev: u32) -> usize {
        let mut n = 0;
        while let Some(w) = self.f.sys.io.next_work(dev) {
            let r = w.run();
            self.f.sys.io.work_done(w.irp, r);
            n += 1;
        }
        n
    }

    /// Deliver completions the way the kernel does: memory first, then
    /// signals, then APCs.
    fn drain(&mut self) -> Vec<super::Completion> {
        let mut out = Vec::new();
        while let Some(c) = self.f.sys.io.pop_completion() {
            if let Some((addr, data)) = &c.data {
                self.f.mem.write_bytes(*addr, data).unwrap();
            }
            if c.iosb != 0 {
                self.f.mem.write_bytes(c.iosb, &c.iosb_bytes()).unwrap();
            }
            if let Some((t, apc)) = self.f.sys.io.finish(&mut self.f.sys.objects, &c) {
                self.f.apcs.push((t, apc));
            }
            out.push(c);
        }
        out
    }
}

fn wide_str(b: &[u8]) -> String {
    String::from_utf16(&b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap()
}

#[test]
fn drive_letters_and_devices_are_in_the_namespace() {
    let mut t = setup();
    assert!(t.exists(b"/nt/c"));
    let om = &mut t.f.sys.objects;
    let names = |om: &mut poly_ntobj::ObjectManager, dir: &str| -> Vec<(String, &'static str)> {
        let id = om.lookup(&poly_ntobj::name::wide(dir), true).unwrap();
        om.directory_entries(id).unwrap().into_iter().map(|(n, k)| (String::from_utf16(&n).unwrap(), k)).collect()
    };
    let devices = names(om, "\\Device");
    for d in ["PolyFs0", "PolyFs1", "PolyPipe"] {
        assert!(devices.contains(&(String::from(d), "Device")), "{d}");
    }
    let dos = names(om, "\\??");
    assert!(dos.contains(&(String::from("C:"), "SymbolicLink")));
    assert!(dos.contains(&(String::from("Z:"), "SymbolicLink")));
    assert_eq!(t.f.sys.io.volumes[1].path, b"/nt/c");
    assert_eq!(t.f.sys.io.volumes[0].letter, Some('Z'));
}

#[test]
fn create_write_read_close_under_c() {
    let mut t = setup();
    let (s, h, info) = t.create("\\??\\C:\\hello.txt", RW, 0, FILE_CREATE, SYNC, FILE_ATTRIBUTE_NORMAL);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    assert_eq!(t.write(h, b"hello, world", None), (STATUS_SUCCESS, 12));
    // The position moved; a read at the position hits the end.
    assert_eq!(t.read(h, 4, None).0, STATUS_END_OF_FILE);
    assert_eq!(t.read(h, 5, Some(0)), (STATUS_SUCCESS, b"hello".to_vec()));
    // The explicit offset moved the position too (synchronous handle).
    assert_eq!(t.read(h, 100, None), (STATUS_SUCCESS, b", world".to_vec()));
    t.close(h);
    assert_eq!(t.f.sys.io.file_count(), 0);
    assert!(t.exists(b"/nt/c/hello.txt"));
    // And it is on Z: too, the whole VFS.
    let (s, h, info) = t.create("\\??\\Z:\\nt\\c\\hello.txt", FILE_GENERIC_READ, 0, FILE_OPEN, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OPENED));
    assert_eq!(t.read(h, 100, None), (STATUS_SUCCESS, b"hello, world".to_vec()));
    t.close(h);
}

#[test]
fn create_dispositions() {
    let mut t = setup();
    let p = "\\??\\C:\\d.txt";
    assert_eq!(t.create(p, RW, 0, FILE_OPEN, SYNC, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(t.create(p, RW, 0, FILE_OVERWRITE, SYNC, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    let (s, h, info) = t.create(p, RW, 0, FILE_OPEN_IF, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.write(h, b"0123456789", None);
    t.close(h);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, SYNC, 0).0, STATUS_OBJECT_NAME_COLLISION);
    let (s, h, info) = t.create(p, RW, 0, FILE_OPEN_IF, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OPENED));
    assert_eq!(t.read(h, 3, None).1, b"012");
    t.close(h);
    let (s, h, info) = t.create(p, RW, 0, FILE_OVERWRITE, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OVERWRITTEN));
    assert_eq!(t.read(h, 3, None).0, STATUS_END_OF_FILE);
    t.write(h, b"abc", None);
    t.close(h);
    let (s, h, info) = t.create(p, RW, 0, FILE_SUPERSEDE, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_SUPERSEDED));
    t.close(h);
    let (s, h, info) = t.create("\\??\\C:\\new.txt", RW, 0, FILE_OVERWRITE_IF, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.close(h);
    let (s, h, info) = t.create("\\??\\C:\\new2.txt", RW, 0, FILE_SUPERSEDE, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.close(h);
}

#[test]
fn create_parameter_and_path_errors() {
    let mut t = setup();
    let p = "\\??\\C:\\x";
    let inval = STATUS_INVALID_PARAMETER;
    assert_eq!(t.create(p, RW, 0, 6, SYNC, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, FILE_DIRECTORY_FILE | FILE_NON_DIRECTORY_FILE, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_OVERWRITE_IF, FILE_DIRECTORY_FILE, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, SYNC | FILE_SYNCHRONOUS_IO_ALERT, 0).0, inval);
    assert_eq!(t.create(p, FILE_READ_DATA, 0, FILE_CREATE, SYNC, 0).0, inval);
    assert_eq!(t.create(p, RW, 0, FILE_CREATE, FILE_DELETE_ON_CLOSE, 0).0, inval);
    assert_eq!(t.create(p, RW, 8, FILE_CREATE, 0, 0).0, inval);
    assert_eq!(t.create("\\??\\C:\\no\\such\\file", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_PATH_NOT_FOUND);
    assert_eq!(t.create("\\??\\C:\\a*b", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\a\\..\\b", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\f:stream", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\Q:\\x", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_PATH_NOT_FOUND);
    let h = t.file("\\??\\C:\\plain");
    t.close(h);
    assert_eq!(t.create("\\??\\C:\\plain", RW, 0, FILE_OPEN, FILE_DIRECTORY_FILE, 0).0, STATUS_NOT_A_DIRECTORY);
    assert_eq!(t.create("\\??\\C:\\plain\\", RW, 0, FILE_OPEN, 0, 0).0, STATUS_OBJECT_NAME_INVALID);
    assert_eq!(t.create("\\??\\C:\\plain\\x", RW, 0, FILE_OPEN, 0, 0).0, STATUS_OBJECT_PATH_NOT_FOUND);
    let (s, h, info) =
        t.create("\\??\\C:\\Dir", FILE_LIST_DIRECTORY | SYNCHRONIZE, 0, FILE_CREATE, FILE_DIRECTORY_FILE, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_CREATED));
    t.close(h);
    assert!(t.exists(b"/nt/c/Dir"));
    assert_eq!(t.create("\\??\\C:\\dir", RW, 0, FILE_OPEN, FILE_NON_DIRECTORY_FILE, 0).0, STATUS_FILE_IS_A_DIRECTORY);
    assert_eq!(t.create("\\??\\C:\\dir", RW, 0, FILE_OVERWRITE, 0, 0).0, STATUS_FILE_IS_A_DIRECTORY);
    // Reading a directory is not a thing.
    let (s, h, _) = t.create("\\??\\C:\\DIR\\", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.read(h, 1, None).0, STATUS_INVALID_DEVICE_REQUEST);
    t.close(h);
    // No extended attributes.
    let out = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let oa = t.f.attrs("\\??\\C:\\ea", CI);
    let ea = t.f.alloc(16);
    let s = t.f.call(Service::CreateFile, &[out, u64::from(RW), oa, iosb, 0, 0, 0, u64::from(FILE_CREATE), 0, ea, 16]);
    assert_eq!(s, STATUS_EAS_NOT_SUPPORTED);
}

#[test]
fn case_insensitive_case_preserving_lookup() {
    let mut t = setup();
    let (s, h, _) = t.create("\\??\\C:\\MixedCase.TXT", RW, SHARE_ALL, FILE_CREATE, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.write(h, b"data", None);
    t.close(h);
    assert!(t.exists(b"/nt/c/MixedCase.TXT"));
    let (s, h, info) = t.create("\\??\\c:\\MIXEDcase.txt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!((s, info), (STATUS_SUCCESS, FILE_OPENED));
    assert_eq!(t.read(h, 10, None).1, b"data");
    // The name it reports is the one on disk.
    let (s, b) = t.query(h, FILE_NAME_INFORMATION, 100);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(wide_str(&b[4..]), "\\MixedCase.TXT");
    t.close(h);
    // A create in another case finds the file.
    assert_eq!(t.create("\\??\\C:\\mixedcase.txt", RW, 0, FILE_CREATE, 0, 0).0, STATUS_OBJECT_NAME_COLLISION);
    // Directories on the way too.
    let (s, h, _) =
        t.create("\\??\\C:\\SubDir", FILE_LIST_DIRECTORY | SYNCHRONIZE, 0, FILE_CREATE, FILE_DIRECTORY_FILE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
    let h = t.file("\\??\\C:\\subdir\\Inner.txt");
    t.close(h);
    assert!(t.exists(b"/nt/c/SubDir/Inner.txt"));
}

#[test]
fn names_that_differ_only_in_case() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/readme", b"lower");
    t.vfs_create(b"/nt/c/README", b"upper");
    t.vfs_create(b"/nt/c/ReadMe", b"mixed");
    fn read(t: &mut T, name: &str) -> Vec<u8> {
        let (s, h, _) = t.create(name, FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
        assert_eq!(s, STATUS_SUCCESS);
        let r = t.read(h, 10, None).1;
        t.close(h);
        r
    }
    // Exact matches win; otherwise the smallest name as bytes.
    assert_eq!(read(&mut t, "\\??\\C:\\readme"), b"lower");
    assert_eq!(read(&mut t, "\\??\\C:\\ReadMe"), b"mixed");
    assert_eq!(read(&mut t, "\\??\\C:\\README"), b"upper");
    assert_eq!(read(&mut t, "\\??\\C:\\rEADME"), b"upper");
}

#[test]
fn share_modes_across_handles() {
    let mut t = setup();
    let p = "\\??\\C:\\shared.txt";
    let (s, h1, _) = t.create(p, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_OPEN_IF, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.create(p, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_OPEN, SYNC, 0).0, STATUS_SHARING_VIOLATION);
    assert_eq!(t.create(p, FILE_GENERIC_READ, FILE_SHARE_WRITE, FILE_OPEN, SYNC, 0).0, STATUS_SHARING_VIOLATION);
    let (s, h2, _) = t.create(p, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // Attribute-only opens never conflict.
    let (s, h3, _) = t.create(p, FILE_READ_ATTRIBUTES | SYNCHRONIZE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // An overwrite refused for sharing leaves the file alone.
    t.close(h1);
    t.close(h2);
    let (_, w, _) = t.create(p, RW, FILE_SHARE_READ, FILE_OPEN, SYNC, 0);
    t.write(w, b"keep", None);
    assert_eq!(t.create(p, RW, SHARE_ALL, FILE_OVERWRITE, SYNC, 0).0, STATUS_SHARING_VIOLATION);
    assert_eq!(t.read(w, 4, Some(0)).1, b"keep");
    t.close(w);
    t.close(h3);
    let (s, h, _) = t.create(p, FILE_GENERIC_WRITE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
}

#[test]
fn readonly_hidden_and_times() {
    let mut t = setup();
    let p = "\\??\\C:\\ro.txt";
    let (s, h, _) = t.create(p, RW, 0, FILE_CREATE, SYNC, FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN);
    assert_eq!(s, STATUS_SUCCESS);
    // The creator may still write.
    assert_eq!(t.write(h, b"x", None).0, STATUS_SUCCESS);
    let (s, b) = t.query(h, FILE_BASIC_INFORMATION, 40);
    assert_eq!(s, STATUS_SUCCESS);
    let attrs = u32::from_le_bytes(b[32..36].try_into().unwrap());
    assert_eq!(attrs, FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE);
    // 2001-09-09 01:46:40 UTC, from the fixed clock, in 100 ns since 1601.
    let write_time = i64::from_le_bytes(b[16..24].try_into().unwrap());
    assert_eq!(write_time, 1_000_000_000 * 10_000_000 + super::info::EPOCH_DIFF_100NS);
    t.close(h);
    assert_eq!(t.create(p, FILE_GENERIC_WRITE, 0, FILE_OPEN, SYNC, 0).0, STATUS_ACCESS_DENIED);
    assert_eq!(t.create(p, RW | DELETE, 0, FILE_OPEN, FILE_DELETE_ON_CLOSE, 0).0, STATUS_ACCESS_DENIED);
    assert_eq!(t.create(p, FILE_GENERIC_READ | DELETE, 0, FILE_OPEN, FILE_DELETE_ON_CLOSE, 0).0, STATUS_CANNOT_DELETE);
    // A hidden file is not overwritten by a create that does not say hidden.
    assert_eq!(t.create(p, FILE_GENERIC_READ, 0, FILE_OVERWRITE_IF, SYNC, 0).0, STATUS_ACCESS_DENIED);
    // Clear READONLY and HIDDEN, set a write time.
    let (s, h, _) = t.create(p, FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let mut basic = [0u8; 40];
    let when: i64 = super::info::EPOCH_DIFF_100NS + 5 * 10_000_000;
    basic[16..24].copy_from_slice(&when.to_le_bytes());
    basic[0..8].copy_from_slice(&(when - 1).to_le_bytes());
    basic[32..36].copy_from_slice(&FILE_ATTRIBUTE_NORMAL.to_le_bytes());
    assert_eq!(t.set(h, FILE_BASIC_INFORMATION, &basic), STATUS_SUCCESS);
    let (_, b) = t.query(h, FILE_BASIC_INFORMATION, 40);
    assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), FILE_ATTRIBUTE_NORMAL);
    assert_eq!(i64::from_le_bytes(b[16..24].try_into().unwrap()), when);
    assert_eq!(i64::from_le_bytes(b[0..8].try_into().unwrap()), when - 1);
    // Setting needs FILE_WRITE_ATTRIBUTES.
    t.close(h);
    let (_, h, _) = t.create(p, FILE_GENERIC_READ, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(h, FILE_BASIC_INFORMATION, &basic), STATUS_ACCESS_DENIED);
    t.close(h);
    let (s, h, _) = t.create(p, FILE_GENERIC_WRITE, 0, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.close(h);
}

#[test]
fn information_classes() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\info.bin");
    t.write(h, &[7u8; 5000], None);
    let (s, b) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), 5000);
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), 8192);
    assert_eq!((b[20], b[21]), (0, 0));
    assert_eq!(t.query(h, FILE_STANDARD_INFORMATION, 23).0, STATUS_INFO_LENGTH_MISMATCH);
    let (_, b) = t.query(h, FILE_POSITION_INFORMATION, 8);
    assert_eq!(u64::from_le_bytes(b[..8].try_into().unwrap()), 5000);
    assert_eq!(t.set(h, FILE_POSITION_INFORMATION, &10u64.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(t.read(h, 2, None).1, [7, 7]);
    assert_eq!(t.set(h, FILE_POSITION_INFORMATION, &(-1i64).to_le_bytes()), STATUS_INVALID_PARAMETER);
    assert_eq!(t.set(h, FILE_END_OF_FILE_INFORMATION, &100u64.to_le_bytes()), STATUS_SUCCESS);
    let (_, b) = t.query(h, FILE_NETWORK_OPEN_INFORMATION, 56);
    assert_eq!(u64::from_le_bytes(b[40..48].try_into().unwrap()), 100);
    // All information: the name at 100, with overflow when it does not fit.
    let (s, b) = t.query(h, FILE_ALL_INFORMATION, 200);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u64::from_le_bytes(b[48..56].try_into().unwrap()), 100);
    assert_eq!(u32::from_le_bytes(b[76..80].try_into().unwrap()) & FILE_READ_DATA, FILE_READ_DATA);
    assert_eq!(u32::from_le_bytes(b[88..92].try_into().unwrap()), SYNC);
    assert_eq!(wide_str(&b[100..]), "\\info.bin");
    let (s, b) = t.query(h, FILE_ALL_INFORMATION, 104);
    assert_eq!((s, b.len()), (STATUS_BUFFER_OVERFLOW, 104));
    assert_eq!(u32::from_le_bytes(b[96..100].try_into().unwrap()), 18);
    assert_eq!(t.query(h, FILE_ALL_INFORMATION, 99).0, STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.query(h, 99, 100).0, STATUS_INVALID_INFO_CLASS);
    let (s, b) = t.query(h, FILE_ATTRIBUTE_TAG_INFORMATION, 8);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), FILE_ATTRIBUTE_ARCHIVE);
    t.close(h);
    // Query by name.
    let oa = t.f.attrs("\\??\\C:\\INFO.BIN", CI);
    let out = t.f.alloc(56);
    assert_eq!(t.f.call(Service::QueryFullAttributesFile, &[oa, out]), STATUS_SUCCESS);
    assert_eq!(t.f.u64_at(out + 40), 100);
    let out = t.f.alloc(40);
    assert_eq!(t.f.call(Service::QueryAttributesFile, &[oa, out]), STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(out + 32), FILE_ATTRIBUTE_ARCHIVE);
    let oa = t.f.attrs("\\??\\C:\\missing", CI);
    assert_eq!(t.f.call(Service::QueryAttributesFile, &[oa, out]), STATUS_OBJECT_NAME_NOT_FOUND);
    assert_eq!(t.f.sys.io.file_count(), 0);
}

#[test]
fn volume_information() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\v");
    let buf = t.f.alloc(64);
    let iosb = t.f.alloc(16);
    let s =
        t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_ATTRIBUTE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf) & FILE_CASE_PRESERVED_NAMES, FILE_CASE_PRESERVED_NAMES);
    assert_eq!(t.f.u32_at(buf) & FILE_CASE_SENSITIVE_SEARCH, 0);
    assert_eq!(t.f.u32_at(buf + 4), 255);
    let len = t.f.u32_at(buf + 8) as usize;
    assert_eq!(wide_str(&t.f.bytes_at(buf + 12, len)), "PolyFS");
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_DEVICE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf), FILE_DEVICE_DISK);
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_VOLUME_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(wide_str(&t.f.bytes_at(buf + 18, t.f.u32_at(buf + 12) as usize)), "System");
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 64, u64::from(FILE_FS_SIZE_INFORMATION)]);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(buf + 20), 512);
    let s = t.f.call(Service::QueryVolumeInformationFile, &[h, iosb, buf, 4, u64::from(FILE_FS_SIZE_INFORMATION)]);
    assert_eq!(s, STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.f.call(Service::FlushBuffersFile, &[h, iosb]), STATUS_SUCCESS);
    t.close(h);
}

/// Directory entries in a buffer: (name, attributes) of each.
fn entries(b: &[u8], class: u32) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let next = u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let (len_at, name_at) = match class {
            FILE_NAMES_INFORMATION => (8, 12),
            _ => (60, super::info::dir_fixed(class).unwrap()),
        };
        let len = u32::from_le_bytes(b[at + len_at..at + len_at + 4].try_into().unwrap()) as usize;
        let attrs = if class == FILE_NAMES_INFORMATION {
            0
        } else {
            u32::from_le_bytes(b[at + 56..at + 60].try_into().unwrap())
        };
        out.push((wide_str(&b[at + name_at..at + name_at + len]), attrs));
        if next == 0 {
            break;
        }
        assert_eq!(next % 8, 0);
        at += next;
    }
    out
}

impl T {
    fn list(
        &mut self,
        h: u64,
        class: u32,
        pattern: Option<&str>,
        single: bool,
        restart: bool,
        len: usize,
    ) -> (NtStatus, Vec<u8>) {
        let buf = self.f.alloc(len);
        let iosb = self.f.alloc(16);
        let name = pattern.map_or(0, |p| self.f.unicode(p));
        let s = self.f.call(
            Service::QueryDirectoryFile,
            &[h, 0, 0, 0, iosb, buf, len as u64, u64::from(class), u64::from(single), name, u64::from(restart)],
        );
        let n = self.f.u64_at(iosb + 8) as usize;
        (s, self.f.bytes_at(buf, n.min(len)))
    }
}

#[test]
fn directory_enumeration_with_wildcards() {
    let mut t = setup();
    let (s, d, _) = t.create(
        "\\??\\C:\\Docs",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(s, STATUS_SUCCESS);
    for name in ["a.txt", "B.TXT", "c.log", "d.txt.bak"] {
        let h = t.file(&alloc::format!("\\??\\C:\\Docs\\{name}"));
        t.close(h);
    }
    let (s, b) = t.list(d, FILE_DIRECTORY_INFORMATION, Some("*.txt"), false, false, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    let mut names: Vec<String> = entries(&b, FILE_DIRECTORY_INFORMATION).into_iter().map(|e| e.0).collect();
    names.sort();
    assert_eq!(names, ["B.TXT", "a.txt"]);
    // The pattern stays; the scan is done.
    assert_eq!(t.list(d, FILE_DIRECTORY_INFORMATION, None, false, false, 4096).0, STATUS_NO_MORE_FILES);
    // Restart, one at a time, with a `?` pattern.
    let (s, b) = t.list(d, FILE_BOTH_DIRECTORY_INFORMATION, Some("?.log"), true, true, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(entries(&b, FILE_BOTH_DIRECTORY_INFORMATION), [(String::from("c.log"), FILE_ATTRIBUTE_ARCHIVE)]);
    assert_eq!(t.list(d, FILE_BOTH_DIRECTORY_INFORMATION, None, true, false, 4096).0, STATUS_NO_MORE_FILES);
    // Everything, `.` and `..` included below the root.
    let (s, b) = t.list(d, FILE_ID_BOTH_DIRECTORY_INFORMATION, Some("*"), false, true, 4096);
    assert_eq!(s, STATUS_SUCCESS);
    let all = entries(&b, FILE_ID_BOTH_DIRECTORY_INFORMATION);
    assert_eq!(all.len(), 6);
    assert!(all.iter().any(|e| e.0 == "." && e.1 == FILE_ATTRIBUTE_DIRECTORY));
    assert!(all.iter().any(|e| e.0 == ".."));
    // Nothing matches on a fresh scan.
    let (s, _) = t.list(d, FILE_NAMES_INFORMATION, Some("*.zip"), false, true, 4096);
    assert_eq!(s, STATUS_NO_SUCH_FILE);
    // A buffer for exactly one entry, then overflow for a too-small one.
    let (s, b) = t.list(d, FILE_NAMES_INFORMATION, Some("a.TXT"), false, true, 12 + 10);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(entries(&b, FILE_NAMES_INFORMATION)[0].0, "a.txt");
    let (s, b) = t.list(d, FILE_NAMES_INFORMATION, Some("a.txt"), false, true, 14);
    assert_eq!((s, b.len()), (STATUS_BUFFER_OVERFLOW, 14));
    assert_eq!(t.list(d, FILE_NAMES_INFORMATION, None, false, true, 4).0, STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.list(d, FILE_BASIC_INFORMATION, None, false, true, 4096).0, STATUS_INVALID_INFO_CLASS);
    t.close(d);
    // The root lists no `.` or `..`; a file handle cannot be listed.
    let (_, root, _) = t.create("\\??\\C:\\", FILE_LIST_DIRECTORY | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    let (_, b) = t.list(root, FILE_NAMES_INFORMATION, None, false, false, 4096);
    let names: Vec<String> = entries(&b, FILE_NAMES_INFORMATION).into_iter().map(|e| e.0).collect();
    assert_eq!(names, ["Docs"]);
    // An open relative to a directory handle.
    let out = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let oa = t.f.attrs_in(root, "docs\\A.txt", CI);
    let s = t
        .f
        .call(Service::OpenFile, &[out, u64::from(FILE_GENERIC_READ), oa, iosb, u64::from(SHARE_ALL), u64::from(SYNC)]);
    assert_eq!(s, STATUS_SUCCESS);
    let rel = t.f.u64_at(out);
    let (_, b) = t.query(rel, FILE_NAME_INFORMATION, 100);
    assert_eq!(wide_str(&b[4..]), "\\Docs\\a.txt");
    assert_eq!(t.list(rel, FILE_NAMES_INFORMATION, None, false, false, 4096).0, STATUS_INVALID_PARAMETER);
    t.close(rel);
    t.close(root);
}

#[test]
fn delete_by_disposition_delete_on_close_and_nt_delete_file() {
    let mut t = setup();
    // Disposition: gone when the last handle closes.
    let h = t.file("\\??\\C:\\gone.txt");
    let h2 = t.file("\\??\\C:\\gone.txt");
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    let (_, b) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(b[20], 1, "DeletePending");
    assert_eq!(t.create("\\??\\C:\\gone.txt", RW, SHARE_ALL, FILE_OPEN, 0, 0).0, STATUS_DELETE_PENDING);
    t.close(h);
    assert!(t.exists(b"/nt/c/gone.txt"));
    t.close(h2);
    assert!(!t.exists(b"/nt/c/gone.txt"));
    // Undoing the mark keeps the file.
    let h = t.file("\\??\\C:\\kept.txt");
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[0]), STATUS_SUCCESS);
    t.close(h);
    assert!(t.exists(b"/nt/c/kept.txt"));
    // Needs DELETE access.
    let (_, h, _) = t.create("\\??\\C:\\kept.txt", RW, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(h, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_ACCESS_DENIED);
    t.close(h);
    // FILE_DELETE_ON_CLOSE.
    let (s, h, _) = t.create("\\??\\C:\\temp.txt", RW | DELETE, 0, FILE_CREATE, SYNC | FILE_DELETE_ON_CLOSE, 0);
    assert_eq!(s, STATUS_SUCCESS);
    t.write(h, b"scratch", None);
    assert!(t.exists(b"/nt/c/temp.txt"));
    t.close(h);
    assert!(!t.exists(b"/nt/c/temp.txt"));
    // Directories: only empty ones.
    let (_, d, _) = t.create(
        "\\??\\C:\\full",
        FILE_LIST_DIRECTORY | DELETE | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    let f = t.file("\\??\\C:\\full\\x");
    t.close(f);
    assert_eq!(t.set(d, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_DIRECTORY_NOT_EMPTY);
    // NtDeleteFile.
    let oa = t.f.attrs("\\??\\C:\\FULL\\X", CI);
    assert_eq!(t.f.call(Service::DeleteFile, &[oa]), STATUS_SUCCESS);
    assert!(!t.exists(b"/nt/c/full/x"));
    assert_eq!(t.set(d, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_SUCCESS);
    t.close(d);
    assert!(!t.exists(b"/nt/c/full"));
    // A sharing violation for NtDeleteFile while another handle denies delete.
    let (_, h, _) = t.create("\\??\\C:\\busy", RW, FILE_SHARE_READ, FILE_CREATE, SYNC, 0);
    let oa = t.f.attrs("\\??\\C:\\busy", CI);
    assert_eq!(t.f.call(Service::DeleteFile, &[oa]), STATUS_SHARING_VIOLATION);
    t.close(h);
    // The volume root cannot be deleted.
    let (_, root, _) = t.create("\\??\\C:\\", DELETE | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(root, FILE_DISPOSITION_INFORMATION, &[1]), STATUS_CANNOT_DELETE);
    t.close(root);
}

/// A `FILE_RENAME_INFORMATION` for `name`.
fn rename_info(replace: bool, root: u64, name: &str) -> Vec<u8> {
    let units: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut b = alloc::vec![0u8; 20];
    b[0] = u8::from(replace);
    b[8..16].copy_from_slice(&root.to_le_bytes());
    b[16..20].copy_from_slice(&(units.len() as u32).to_le_bytes());
    b.extend_from_slice(&units);
    b
}

#[test]
fn rename_in_place_with_replace_and_by_full_path() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\old.txt");
    t.write(h, b"moving", None);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "new.txt")), STATUS_SUCCESS);
    assert!(!t.exists(b"/nt/c/old.txt"));
    assert!(t.exists(b"/nt/c/new.txt"));
    let (_, b) = t.query(h, FILE_NAME_INFORMATION, 100);
    assert_eq!(wide_str(&b[4..]), "\\new.txt");
    // Onto an existing file.
    let other = t.file("\\??\\C:\\other.txt");
    t.close(other);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "OTHER.txt")), STATUS_OBJECT_NAME_COLLISION);
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(true, 0, "Other.txt")), STATUS_SUCCESS);
    assert!(!t.exists(b"/nt/c/other.txt"));
    assert!(t.exists(b"/nt/c/Other.txt"));
    // Only the case.
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "OTHER.TXT")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/OTHER.TXT"));
    assert!(!t.exists(b"/nt/c/Other.txt"));
    // A full path into a directory, and onto an open file.
    let (_, d, _) = t.create(
        "\\??\\C:\\sub",
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | SYNC,
        0,
    );
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "\\??\\C:\\SUB\\moved.txt")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/sub/moved.txt"));
    let busy = t.file("\\??\\C:\\busy.txt");
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(true, 0, "\\??\\C:\\busy.txt")), STATUS_ACCESS_DENIED);
    // Relative to a directory handle.
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, d, "again.txt")), STATUS_SUCCESS);
    assert!(t.exists(b"/nt/c/sub/again.txt"));
    // Another volume is another device.
    assert_eq!(t.set(h, FILE_RENAME_INFORMATION, &rename_info(false, 0, "\\??\\Z:\\x.txt")), STATUS_NOT_SAME_DEVICE);
    // Needs DELETE access.
    let (_, ro, _) = t.create("\\??\\C:\\sub\\again.txt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.set(ro, FILE_RENAME_INFORMATION, &rename_info(false, 0, "nope")), STATUS_ACCESS_DENIED);
    t.close(ro);
    assert_eq!(t.read(h, 6, Some(0)).1, b"moving");
    for x in [h, busy, d] {
        t.close(x);
    }
}

#[test]
fn byte_range_locks_against_io_and_each_other() {
    let mut t = setup();
    let a = t.file("\\??\\C:\\locked");
    let b = t.file("\\??\\C:\\locked");
    t.write(a, &[1u8; 100], Some(0));
    assert_eq!(t.lock(a, 0, 10, 0, true, true), STATUS_SUCCESS);
    // Another handle cannot read or write the region, the owner can.
    assert_eq!(t.read(b, 5, Some(0)).0, STATUS_FILE_LOCK_CONFLICT);
    assert_eq!(t.write(b, b"x", Some(9)).0, STATUS_FILE_LOCK_CONFLICT);
    assert_eq!(t.read(b, 5, Some(10)).0, STATUS_SUCCESS);
    assert_eq!(t.read(a, 5, Some(0)).0, STATUS_SUCCESS);
    assert_eq!(t.write(a, b"y", Some(0)).0, STATUS_SUCCESS);
    // The same handle with another key is another owner.
    let iosb = t.f.alloc(16);
    let buf = t.f.alloc(4);
    let off = t.f.put_u64(0);
    let key = t.f.put(&5u32.to_le_bytes());
    assert_eq!(t.f.call(Service::ReadFile, &[a, 0, 0, 0, iosb, buf, 4, off, key]), STATUS_FILE_LOCK_CONFLICT);
    // Conflicting lock requests.
    assert_eq!(t.lock(b, 5, 10, 0, true, false), STATUS_LOCK_NOT_GRANTED);
    assert_eq!(t.lock(a, 0, 10, 0, true, true), STATUS_LOCK_NOT_GRANTED);
    assert_eq!(t.unlock(a, 0, 5, 0), STATUS_RANGE_NOT_LOCKED);
    assert_eq!(t.unlock(b, 0, 10, 0), STATUS_RANGE_NOT_LOCKED);
    assert_eq!(t.unlock(a, 0, 10, 0), STATUS_SUCCESS);
    assert_eq!(t.lock(b, 5, 10, 0, true, false), STATUS_SUCCESS);
    // A shared lock: everyone reads, nobody writes.
    assert_eq!(t.read(a, 5, Some(5)).0, STATUS_SUCCESS);
    assert_eq!(t.write(b, b"z", Some(6)).0, STATUS_FILE_LOCK_CONFLICT);
    assert_eq!(t.lock(a, 8, 2, 0, true, false), STATUS_SUCCESS);
    // The whole file, as LockFileEx(0, 0, MAXDWORD, MAXDWORD).
    assert_eq!(t.lock(a, 0, u64::MAX, 0, true, true), STATUS_LOCK_NOT_GRANTED);
    assert_eq!(t.lock(a, 1, u64::MAX, 0, true, true), STATUS_INVALID_LOCK_RANGE);
    // Closing b drops its lock.
    t.close(b);
    assert_eq!(t.write(a, b"w", Some(12)).0, STATUS_SUCCESS);
    // Locks need read or write access.
    let (_, c, _) = t.create("\\??\\C:\\locked", FILE_READ_ATTRIBUTES | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.lock(c, 50, 1, 0, true, true), STATUS_ACCESS_DENIED);
    t.close(c);
    t.close(a);
}

#[test]
fn append_only_handles_write_at_the_end() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\log.txt");
    t.write(h, b"one,", None);
    t.close(h);
    let (s, h, _) = t.create("\\??\\C:\\log.txt", FILE_APPEND_DATA | SYNCHRONIZE, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.write(h, b"two", Some(0)), (STATUS_SUCCESS, 3));
    t.close(h);
    let h = t.file("\\??\\C:\\log.txt");
    assert_eq!(t.write(h, b"!", Some(FILE_WRITE_TO_END_OF_FILE)), (STATUS_SUCCESS, 1));
    assert_eq!(t.read(h, 100, Some(0)).1, b"one,two!");
    t.close(h);
    // Writes need write access.
    let (_, h, _) = t.create("\\??\\C:\\log.txt", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(t.write(h, b"x", Some(0)).0, STATUS_ACCESS_DENIED);
    t.close(h);
}

#[test]
fn asynchronous_read_completes_through_the_worker_event_and_apc() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\async.bin");
    t.write(h, b"0123456789", None);
    t.close(h);
    // An asynchronous handle (no FILE_SYNCHRONOUS_IO_*).
    let (s, h, _) = t.create("\\??\\C:\\async.bin", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    // Without a ByteOffset: refused.
    assert_eq!(t.read(h, 4, None).0, STATUS_INVALID_PARAMETER);
    // An event, an APC, an offset.
    let ev_out = t.f.alloc(8);
    assert_eq!(t.f.call(Service::CreateEvent, &[ev_out, 0x1F0003, 0, 0, 1]), STATUS_SUCCESS);
    let ev = t.f.u64_at(ev_out);
    let buf = t.f.alloc(8);
    let iosb = t.f.alloc(16);
    let off = t.f.put_u64(2);
    let s = t.f.call(Service::ReadFile, &[h, ev, 0x7000, 0xC0DE, iosb, buf, 4, off, 0]);
    assert_eq!(s, STATUS_PENDING);
    // The event was reset when the request was made.
    let event = t.f.sys.objects.object_by_handle(&t.f.caller(), poly_ntobj::Handle(ev), 0, None).unwrap();
    assert!(!t.f.sys.objects.is_ready_for(event, crate::tests::TID));
    assert_eq!(t.f.sys.io.pending_irps(), 1);
    assert_eq!(t.f.sys.io.take_kicks(), [1]);
    assert_eq!(t.run_worker(1), 1);
    let done = t.drain();
    assert_eq!(done.len(), 1);
    assert_eq!(t.f.u32_at(iosb), STATUS_SUCCESS.0);
    assert_eq!(t.f.u64_at(iosb + 8), 4);
    assert_eq!(t.f.bytes_at(buf, 4), b"2345");
    assert!(t.f.sys.objects.is_ready_for(event, crate::tests::TID));
    let (thread, apc) = *t.f.apcs.last().unwrap();
    assert_eq!(thread, crate::tests::TID);
    assert_eq!((apc.routine, apc.arg1, apc.arg2, apc.arg3), (0x7000, 0xC0DE, iosb, 0));
    // At the end of the file: STATUS_END_OF_FILE through the status block.
    let off = t.f.put_u64(10);
    assert_eq!(t.f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, buf, 4, off, 0]), STATUS_PENDING);
    t.run_worker(1);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_END_OF_FILE);
    assert_eq!(t.f.u32_at(iosb), STATUS_END_OF_FILE.0);
    // Without an event the file object is signalled.
    let file = t.f.sys.objects.object_by_handle(&t.f.caller(), poly_ntobj::Handle(h), 0, None).unwrap();
    assert!(t.f.sys.objects.is_ready_for(file, crate::tests::TID));
    t.close(h);
    t.close(ev);
}

#[test]
fn asynchronous_write_and_cancel_before_it_runs() {
    let mut t = setup();
    let (s, h, _) = t.create("\\??\\C:\\aw.bin", RW, SHARE_ALL, FILE_CREATE, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let data = t.f.put(b"async!");
    let iosb = t.f.alloc(16);
    let off = t.f.put_u64(0);
    assert_eq!(t.f.call(Service::WriteFile, &[h, 0, 0, 0, iosb, data, 6, off, 0]), STATUS_PENDING);
    t.run_worker(1);
    t.drain();
    assert_eq!(t.f.u64_at(iosb + 8), 6);
    // A queued request cancelled before the worker gets to it.
    let iosb2 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::WriteFile, &[h, 0, 0, 0, iosb2, data, 6, off, 0]), STATUS_PENDING);
    let cancel_iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::CancelIoFile, &[h, cancel_iosb]), STATUS_SUCCESS);
    assert_eq!(t.run_worker(1), 0);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_CANCELLED);
    assert_eq!(t.f.u32_at(iosb2), STATUS_CANCELLED.0);
    // Nothing left: CancelIoFileEx finds nothing.
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[h, 0, cancel_iosb]), STATUS_NOT_FOUND);
    t.close(h);
}

fn pipe(t: &mut T, name: &str) -> u64 {
    let (s, h, _) = t.create(&alloc::format!("\\Device\\PolyPipe\\{name}"), RW, SHARE_ALL, FILE_OPEN_IF, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    h
}

#[test]
fn pipe_reads_wait_for_writes_and_cancel() {
    let mut t = setup();
    let r = pipe(&mut t, "loop");
    let w = pipe(&mut t, "LOOP");
    let buf = t.f.alloc(16);
    let iosb = t.f.alloc(16);
    // Nothing there: the read waits.
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0x9000, 0x42, iosb, buf, 16, 0, 0]), STATUS_PENDING);
    assert!(t.drain().is_empty());
    // A write completes it.
    assert_eq!(t.write(w, b"ping", None).0, STATUS_SUCCESS);
    let done = t.drain();
    assert_eq!(done.len(), 1);
    assert_eq!((done[0].status, done[0].information), (STATUS_SUCCESS, 4));
    assert_eq!(t.f.bytes_at(buf, 4), b"ping");
    assert_eq!(t.f.apcs.last().unwrap().1.arg1, 0x42);
    // Data waiting: a read completes at once.
    t.write(w, b"pong", None);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb, buf, 2, 0, 0]), STATUS_SUCCESS);
    t.drain();
    assert_eq!(t.f.bytes_at(buf, 2), b"po");
    assert_eq!(t.f.sys.io.pipe_buffered(&super::names::fold(&poly_ntobj::name::wide("loop"))), Some(2));
    t.read(r, 2, None);
    t.drain();
    // A waiting read cancelled by its thread.
    let iosb2 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb2, buf, 16, 0, 0]), STATUS_PENDING);
    let ci = t.f.alloc(16);
    assert_eq!(t.f.call(Service::CancelIoFile, &[r, ci]), STATUS_SUCCESS);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_CANCELLED);
    assert_eq!(t.f.u32_at(iosb2), STATUS_CANCELLED.0);
    // CancelIoFileEx by status block.
    let iosb3 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb3, buf, 16, 0, 0]), STATUS_PENDING);
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[r, iosb2, ci]), STATUS_NOT_FOUND);
    assert_eq!(t.f.call(Service::CancelIoFileEx, &[r, iosb3, ci]), STATUS_SUCCESS);
    assert_eq!(t.drain()[0].status, STATUS_CANCELLED);
    // Closing the handle cancels what waits on it.
    let iosb4 = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0, 0, iosb4, buf, 16, 0, 0]), STATUS_PENDING);
    t.close(r);
    assert_eq!(t.drain()[0].status, STATUS_CANCELLED);
    t.close(w);
    assert_eq!(t.f.sys.io.pending_irps(), 0);
}

#[test]
fn waiting_lock_granted_on_unlock() {
    let mut t = setup();
    let a = t.file("\\??\\C:\\wl");
    let (s, b, _) = t.create("\\??\\C:\\wl", RW, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(t.lock(a, 0, 4, 0, true, true), STATUS_SUCCESS);
    // Asynchronous handle, wait for the lock.
    assert_eq!(t.lock(b, 2, 4, 0, false, true), STATUS_PENDING);
    assert!(t.drain().is_empty());
    assert_eq!(t.unlock(a, 0, 4, 0), STATUS_SUCCESS);
    let done = t.drain();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].status, STATUS_SUCCESS);
    assert_eq!(t.lock(a, 3, 1, 0, true, false), STATUS_LOCK_NOT_GRANTED);
    // A lock granted because the holder closed.
    let (_, c, _) = t.create("\\??\\C:\\wl", RW, SHARE_ALL, FILE_OPEN, 0, 0);
    assert_eq!(t.lock(c, 2, 1, 0, false, true), STATUS_PENDING);
    t.close(b);
    let done = t.drain();
    assert_eq!(done[0].status, STATUS_SUCCESS);
    t.close(c);
    t.close(a);
}

#[test]
fn thread_exit_cancels_without_apc() {
    let mut t = setup();
    let r = pipe(&mut t, "t");
    let buf = t.f.alloc(16);
    let iosb = t.f.alloc(16);
    assert_eq!(t.f.call(Service::ReadFile, &[r, 0, 0x9000, 0, iosb, buf, 16, 0, 0]), STATUS_PENDING);
    let apcs = t.f.apcs.len();
    assert_eq!(t.f.sys.io.cancel_thread(crate::tests::TID), 1);
    assert_eq!(t.drain()[0].status, STATUS_CANCELLED);
    assert_eq!(t.f.apcs.len(), apcs);
    t.close(r);
}

#[test]
fn unattached_and_non_file_handles() {
    let mut f = Fake::new();
    let out = f.alloc(8);
    let iosb = f.alloc(16);
    let oa = f.attrs("\\??\\C:\\x", CI);
    let s = f.call(Service::CreateFile, &[out, u64::from(RW), oa, iosb, 0, 0, 0, u64::from(FILE_CREATE), 0, 0, 0]);
    assert_eq!(s, STATUS_OBJECT_PATH_NOT_FOUND);
    // An event handle is not a file handle.
    let ev = f.alloc(8);
    f.call(Service::CreateEvent, &[ev, 0x1F0003, 0, 0, 0]);
    let h = f.u64_at(ev);
    let s = f.call(Service::ReadFile, &[h, 0, 0, 0, iosb, iosb, 1, 0, 0]);
    assert_eq!(s, STATUS_OBJECT_TYPE_MISMATCH);
    assert!(!IoState::new().is_attached());
}

// ----- processes and file mappings over NT file I/O (ADR 0079) -----------------

/// Parameters for `exe` built in the fake's scratch memory, as
/// `kernelbase` builds them for `NtCreateUserProcess`.
fn parameters_in(t: &mut T, exe: &str, cmd: &str, cwd: &str) -> (u64, crate::params::Parameters) {
    let p = crate::params::Parameters {
        image_path: exe.encode_utf16().collect(),
        command_line: cmd.encode_utf16().collect(),
        environment: "A=1\0POLY=child\0\0".encode_utf16().collect(),
        current_dir: cwd.encode_utf16().collect(),
        ..crate::params::Parameters::default()
    };
    let size = crate::params::image(0, &p).unwrap().len();
    let at = t.f.alloc(size);
    let bytes = crate::params::image(at, &p).unwrap();
    t.f.mem.write_bytes(at, &bytes).unwrap();
    (at, p)
}

fn create_process(t: &mut T, params: u64, thread_flags: u32) -> (NtStatus, u64, u64) {
    let (ph, th) = (t.f.alloc(8), t.f.alloc(8));
    let s = t.f.call(Service::CreateUserProcess, &[ph, th, 0, 0, 0, 0, 0, u64::from(thread_flags), params, 0, 0]);
    (s, t.f.u64_at(ph), t.f.u64_at(th))
}

fn exit_status_of(t: &mut T, h: u64) -> (NtStatus, u32, u64) {
    let buf = t.f.alloc(48);
    let s = t.f.call(Service::QueryInformationProcess, &[h, 0, buf, 48, 0]);
    (s, t.f.u32_at(buf), t.f.u64_at(buf + 32))
}

#[test]
fn create_user_process_starts_a_child_with_its_parameters() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    let (params, want) = parameters_in(&mut t, "C:\\app.exe", "app.exe one \"two words\"", "C:\\Work");
    let (s, ph, th) = create_process(&mut t, params, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert!(ph != 0 && th != 0 && ph != th);
    // The child got its own address space with its parameters in it.
    let (child, mem) = t.f.children.last_mut().unwrap();
    let mut pp = [0u8; 8];
    mem.read(child.peb + crate::params::PEB_PROCESS_PARAMETERS, &mut pp).unwrap();
    let got =
        crate::params::read(u64::from_le_bytes(pp), &mut |a, b| mem.read(a, b).map_err(|_| STATUS_ACCESS_VIOLATION));
    assert_eq!(got, Ok(want));
    let pid = child.pid;
    // Running: the exit status is pending and the handle not signalled.
    let (s, exit, id) = exit_status_of(&mut t, ph);
    assert_eq!((s, exit, id), (STATUS_SUCCESS, STATUS_PENDING.0, u64::from(pid.0)));
    let zero = t.f.put_u64(0);
    assert_eq!(t.f.call(Service::WaitForSingleObject, &[ph, 0, zero]), STATUS_TIMEOUT);
    // Its last thread ends with 0x2A: the process's status, its handle
    // signalled, and the status kept while the handle is open.
    let (mut child, mut mem) = t.f.children.pop().unwrap();
    let tid = child.threads[0].thread;
    assert!(crate::exec::thread_exit(&mut t.f.sys, &mut child, &mut mem, tid, NtStatus(0x2A)));
    crate::exec::process_exit(&mut t.f.sys, &mut child, &mut mem);
    assert_eq!(t.f.call(Service::WaitForSingleObject, &[ph, 0, zero]), STATUS_SUCCESS);
    assert_eq!(t.f.call(Service::WaitForSingleObject, &[th, 0, zero]), STATUS_SUCCESS);
    assert_eq!(exit_status_of(&mut t, ph).1, 0x2A);
    t.close(th);
    t.close(ph);
    assert!(t.f.sys.process_exits.is_empty(), "forgotten with the process object");
}

#[test]
fn create_user_process_failures() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    t.vfs_create(b"/nt/c/notpe.exe", b"MZ but nothing else");
    let (p, _) = parameters_in(&mut t, "C:\\missing.exe", "missing", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_OBJECT_NAME_NOT_FOUND);
    let (p, _) = parameters_in(&mut t, "app.exe", "app", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_OBJECT_PATH_SYNTAX_BAD);
    let (p, _) = parameters_in(&mut t, "C:\\notpe.exe", "notpe", "");
    assert_eq!(create_process(&mut t, p, 0).0, STATUS_INVALID_IMAGE_FORMAT);
    assert_eq!(create_process(&mut t, 0, 0).0, STATUS_INVALID_PARAMETER);
    // The NT path rules.
    let nt = |s: &str| {
        crate::threads::image_nt_name(&s.encode_utf16().collect::<Vec<_>>()).map(|v| String::from_utf16(&v).unwrap())
    };
    assert_eq!(nt("C:\\a\\b.exe"), Ok("\\??\\C:\\a\\b.exe".into()));
    assert_eq!(nt("c:/a/b.exe"), Ok("\\??\\c:\\a\\b.exe".into()));
    assert_eq!(nt("\\\\?\\C:\\x.exe"), Ok("\\??\\C:\\x.exe".into()));
    assert_eq!(nt("\\??\\Z:\\x.exe"), Ok("\\??\\Z:\\x.exe".into()));
    assert_eq!(nt("\\\\server\\share\\x.exe"), Err(STATUS_OBJECT_PATH_SYNTAX_BAD));
    assert!(t.f.children.is_empty());
}

/// `NtSetInformationObject` / `NtQueryObject` of Poly's
/// `ObjectHandleFlagInformation` (ADR 0089).
fn set_handle_flags(t: &mut T, h: u64, inherit: bool, protect: bool) -> NtStatus {
    let info = t.f.put(&[u8::from(inherit), u8::from(protect)]);
    t.f.call(Service::SetInformationObject, &[h, 4, info, 2])
}

fn handle_flags(t: &mut T, h: u64) -> (NtStatus, [u8; 2]) {
    let (buf, ret) = (t.f.alloc(8), t.f.alloc(4));
    let s = t.f.call(Service::QueryObject, &[h, 4, buf, 2, ret]);
    let mut b = [0u8; 2];
    t.f.mem.read(buf, &mut b).unwrap();
    (s, b)
}

#[test]
fn handle_flags_are_set_and_read() {
    let mut t = setup();
    let h = t.file("\\??\\C:\\flags.txt");
    assert_eq!(handle_flags(&mut t, h), (STATUS_SUCCESS, [0, 0]));
    assert_eq!(set_handle_flags(&mut t, h, true, true), STATUS_SUCCESS);
    assert_eq!(handle_flags(&mut t, h), (STATUS_SUCCESS, [1, 1]));
    // Protected from close until the flag is cleared.
    assert_eq!(t.f.call(Service::Close, &[h]), STATUS_HANDLE_NOT_CLOSABLE);
    assert_eq!(set_handle_flags(&mut t, h, true, false), STATUS_SUCCESS);
    assert_eq!(handle_flags(&mut t, h).1, [1, 0]);
    // The wrong length, class or handle.
    let info = t.f.put(&[1, 0]);
    assert_eq!(t.f.call(Service::SetInformationObject, &[h, 4, info, 4]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.f.call(Service::SetInformationObject, &[h, 0, info, 2]), STATUS_INVALID_INFO_CLASS);
    assert_eq!(t.f.call(Service::SetInformationObject, &[0x7777_0000, 4, info, 2]), STATUS_INVALID_HANDLE);
    assert_eq!(t.f.call(Service::SetInformationObject, &[u64::MAX, 4, info, 2]), STATUS_INVALID_HANDLE);
    let (buf, ret) = (t.f.alloc(8), t.f.alloc(4));
    assert_eq!(t.f.call(Service::QueryObject, &[h, 4, buf, 1, ret]), STATUS_INFO_LENGTH_MISMATCH);
    assert_eq!(t.f.u32_at(ret), 2);
    t.close(h);
}

fn create_process_with(t: &mut T, params: u64, process_flags: u32, thread_flags: u32) -> (NtStatus, u64, u64) {
    let (ph, th) = (t.f.alloc(8), t.f.alloc(8));
    let args = [ph, th, 0, 0, 0, 0, u64::from(process_flags), u64::from(thread_flags), params, 0, 0];
    let s = t.f.call(Service::CreateUserProcess, &args);
    (s, t.f.u64_at(ph), t.f.u64_at(th))
}

#[test]
fn create_user_process_inherits_inheritable_handles_at_the_same_values() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    let kept = t.file("\\??\\C:\\inherited.txt");
    let private = t.file("\\??\\C:\\private.txt");
    assert_eq!(set_handle_flags(&mut t, kept, true, false), STATUS_SUCCESS);
    let me = poly_ntobj::Caller::user(crate::tests::PID, crate::tests::TID);
    let object_of =
        |t: &T, c: &poly_ntobj::Caller, h: u64| t.f.sys.objects.object_by_handle(c, poly_ntobj::Handle(h), 0, None);
    let kept_object = object_of(&t, &me, kept).unwrap();

    // bInheritHandles: the inheritable handle at its value, still
    // inheritable; the other one not there.
    let (params, _) = parameters_in(&mut t, "C:\\app.exe", "app", "");
    let inherit = crate::consts::PROCESS_CREATE_FLAGS_INHERIT_HANDLES;
    let (s, ph, th) = create_process_with(&mut t, params, inherit, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (child, _) = t.f.children.last().unwrap();
    let them = poly_ntobj::Caller::user(child.pid, child.threads[0].thread);
    assert_eq!(object_of(&t, &them, kept), Ok(kept_object));
    assert!(t.f.sys.objects.handle_flags(&them, poly_ntobj::Handle(kept)).unwrap().inherit);
    assert_eq!(object_of(&t, &them, private), Err(STATUS_INVALID_HANDLE));
    // Its handle to the file counts: the creator may close its own.
    t.close(kept);
    assert_eq!(object_of(&t, &them, kept), Ok(kept_object));
    t.close(th);
    t.close(ph);

    // Without the flag, nothing is inherited.
    let again = t.file("\\??\\C:\\inherited.txt");
    assert_eq!(set_handle_flags(&mut t, again, true, false), STATUS_SUCCESS);
    let (s, ph, th) = create_process_with(&mut t, params, 0, 0);
    assert_eq!(s, STATUS_SUCCESS);
    let (child, _) = t.f.children.last().unwrap();
    let them = poly_ntobj::Caller::user(child.pid, child.threads[0].thread);
    assert_eq!(object_of(&t, &them, again), Err(STATUS_INVALID_HANDLE));
    assert_eq!(t.f.sys.objects.process_handle_count(child.pid), Some(0));
    t.close(th);
    t.close(ph);
    t.close(again);
    t.close(private);
}

#[test]
fn create_user_process_suspended_and_with_a_startupinfo() {
    let mut t = setup();
    t.vfs_create(b"/nt/c/app.exe", &crate::tests::test_exe(false));
    // A STARTUPINFO with STARTF_USESTDHANDLES and its three handles.
    let mut p = crate::params::Parameters {
        image_path: "C:\\app.exe".encode_utf16().collect(),
        command_line: "app".encode_utf16().collect(),
        environment: "A=1\0\0".encode_utf16().collect(),
        ..crate::params::Parameters::default()
    };
    p.startup.fixed[0..4].copy_from_slice(&104u32.to_le_bytes());
    p.startup.fixed[0x20..0x24].copy_from_slice(&crate::params::STARTF_USESTDHANDLES.to_le_bytes());
    p.startup.title = Some("child".encode_utf16().collect());
    p.std_handles = [0x10, 0x14, 0x14];
    let size = crate::params::image(0, &p).unwrap().len();
    let at = t.f.alloc(size);
    let bytes = crate::params::image(at, &p).unwrap();
    t.f.mem.write_bytes(at, &bytes).unwrap();

    let suspended = crate::consts::THREAD_CREATE_FLAGS_CREATE_SUSPENDED;
    let (s, ph, th) = create_process_with(&mut t, at, 0, suspended);
    assert_eq!(s, STATUS_SUCCESS);
    // The child's parameters hold the STARTUPINFO and the handles.
    let (child, mem) = t.f.children.last_mut().unwrap();
    let tid = child.threads[0].thread;
    let mut pp = [0u8; 8];
    mem.read(child.peb + crate::params::PEB_PROCESS_PARAMETERS, &mut pp).unwrap();
    let got =
        crate::params::read(u64::from_le_bytes(pp), &mut |a, b| mem.read(a, b).map_err(|_| STATUS_ACCESS_VIOLATION))
            .unwrap();
    assert_eq!(got, p);
    assert_eq!(got.given_std_handles(), Some([0x10, 0x14, 0x14]));
    // Its first thread waits for NtResumeThread on the creator's handle.
    assert_eq!(t.f.suspend.get(&tid), Some(&1));
    let prev = t.f.alloc(4);
    assert_eq!(t.f.call(Service::ResumeThread, &[th, prev]), STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(prev), 1);
    assert_eq!(t.f.suspend.get(&tid), Some(&0));
    t.close(th);
    t.close(ph);
}

fn create_section(t: &mut T, file: u64, size: u64, protect: u32) -> (NtStatus, u64) {
    let out = t.f.alloc(8);
    let max = if size == 0 { 0 } else { t.f.put_u64(size) };
    let s = t.f.call(
        Service::CreateSection,
        &[out, 0xF001F, 0, max, u64::from(protect), u64::from(crate::consts::SEC_COMMIT), file],
    );
    (s, t.f.u64_at(out))
}

fn map(t: &mut T, section: u64, protect: u32) -> (NtStatus, u64, u64) {
    let base = t.f.alloc(8);
    let size = t.f.alloc(8);
    let s = t
        .f
        .call(Service::MapViewOfSection, &[section, crate::tests::SELF, base, 0, 0, 0, size, 1, 0, u64::from(protect)]);
    (s, t.f.u64_at(base), t.f.u64_at(size))
}

#[test]
fn data_sections_of_files_map_their_contents_and_grow_them() {
    use crate::consts::{PAGE_READONLY, PAGE_READWRITE};
    let mut t = setup();
    t.vfs_create(b"/nt/c/data.bin", b"hello, mapping");
    let h = t.file("\\??\\C:\\data.bin");
    // The file's size by default.
    let (s, sec) = create_section(&mut t, h, 0, PAGE_READONLY);
    assert_eq!(s, STATUS_SUCCESS);
    let (s, base, size) = map(&mut t, sec, PAGE_READONLY);
    assert_eq!((s, size), (STATUS_SUCCESS, 0x1000));
    assert_eq!(t.f.bytes_at(base, 14), b"hello, mapping");
    // Flushing a view: rounded to pages; outside a view it fails.
    let (b, n, iosb) = (t.f.put_u64(base + 3), t.f.put_u64(10), t.f.alloc(16));
    assert_eq!(t.f.call(Service::FlushVirtualMemory, &[crate::tests::SELF, b, n, iosb]), STATUS_SUCCESS);
    assert_eq!((t.f.u64_at(b), t.f.u64_at(n)), (base, 0x1000));
    let (b, n) = (t.f.put_u64(0x1000), t.f.put_u64(0));
    assert_eq!(t.f.call(Service::FlushVirtualMemory, &[crate::tests::SELF, b, n, iosb]), STATUS_NOT_MAPPED_VIEW);
    // A read-only section cannot be larger than the file...
    assert_eq!(create_section(&mut t, h, 0x3000, PAGE_READONLY).0, STATUS_SECTION_TOO_BIG);
    // ... a writable one grows it (Learn: CreateFileMappingW).
    let (s, sec2) = create_section(&mut t, h, 0x3000, PAGE_READWRITE);
    assert_eq!(s, STATUS_SUCCESS);
    let (_, st) = t.query(h, FILE_STANDARD_INFORMATION, 24);
    assert_eq!(u64::from_le_bytes(st[8..16].try_into().unwrap()), 0x3000);
    // A file opened only for reading cannot back a writable section.
    let (s, ro, _) = t.create("\\??\\C:\\data.bin", FILE_GENERIC_READ, SHARE_ALL, FILE_OPEN, SYNC, 0);
    assert_eq!(s, STATUS_SUCCESS);
    assert_eq!(create_section(&mut t, ro, 0, PAGE_READWRITE).0, STATUS_ACCESS_DENIED);
    let (s, sec3) = create_section(&mut t, ro, 0, PAGE_READONLY);
    assert_eq!(s, STATUS_SUCCESS);
    // An empty file needs a size.
    t.vfs_create(b"/nt/c/empty.bin", b"");
    let e = t.file("\\??\\C:\\empty.bin");
    assert_eq!(create_section(&mut t, e, 0, PAGE_READWRITE).0, STATUS_MAPPED_FILE_SIZE_ZERO);
    for h in [sec, sec2, sec3, ro, e, h] {
        t.close(h);
    }
}
