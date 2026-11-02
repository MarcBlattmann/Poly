//! Host tests of the native dispatch against a fake kernel.

extern crate std;

use crate::abi::{self, Clock, Error, ExitStatus, Handle, SpawnRequest, Str, Whence, nr};
use crate::kernel::{NativeKernel, OpenHow, Prot, Spawn};
use crate::{Native, table};
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use poly_abi::{Outcome, SyscallDispatch, SyscallRegs};

/// Where the fake's user memory starts.
const BASE: u64 = 0x10_0000;
const MEM: usize = 0x2_0000;

/// A fake kernel: one block of user memory, files in memory, a log of
/// what was asked.
#[derive(Default)]
struct Fake {
    mem: Vec<u8>,
    /// Open handles: the path of the file each one reads and writes, and
    /// its position.
    handles: BTreeMap<Handle, (Vec<u8>, u64)>,
    files: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Writes to handle 1, in order.
    stdout: Vec<u8>,
    /// The most a write takes (a full pipe).
    write_limit: Option<usize>,
    exited: Option<u8>,
    opened: Vec<OpenHow>,
    maps: Vec<(u64, u64, Prot)>,
    unmaps: Vec<(u64, u64)>,
    slept: Vec<u64>,
    spawned: Vec<Spawn>,
    children: BTreeMap<u32, ExitStatus>,
}

impl Fake {
    fn new() -> Fake {
        let mut handles = BTreeMap::new();
        handles.insert(1, (b"/dev/console".to_vec(), 0));
        Fake { mem: vec![0; MEM], handles, ..Fake::default() }
    }

    /// Put `bytes` into user memory at `at` (an offset from [`BASE`]);
    /// returns their address.
    fn put(&mut self, at: usize, bytes: &[u8]) -> u64 {
        self.mem[at..at + bytes.len()].copy_from_slice(bytes);
        BASE + at as u64
    }

    fn range(&self, addr: u64, len: usize) -> Result<core::ops::Range<usize>, Error> {
        let start = addr.checked_sub(BASE).ok_or(Error::BadAddress)? as usize;
        let end = start.checked_add(len).ok_or(Error::BadAddress)?;
        if end > self.mem.len() {
            return Err(Error::BadAddress);
        }
        Ok(start..end)
    }

    fn call(&mut self, number: u64, args: [u64; 6]) -> Result<u64, Error> {
        let mut regs = SyscallRegs {
            rax: number,
            rdi: args[0],
            rsi: args[1],
            rdx: args[2],
            r10: args[3],
            r8: args[4],
            r9: args[5],
            ..SyscallRegs::default()
        };
        let before = regs;
        assert_eq!(Native::dispatch_with(self, &mut regs), Outcome::Handled);
        // Only rax changes.
        assert_eq!(SyscallRegs { rax: before.rax, ..regs }, before);
        abi::decode(regs.rax)
    }
}

impl NativeKernel for Fake {
    fn pid(&self) -> u32 {
        7
    }

    fn copy_from_user(&self, addr: u64, dst: &mut [u8]) -> Result<(), Error> {
        let r = self.range(addr, dst.len())?;
        dst.copy_from_slice(&self.mem[r]);
        Ok(())
    }

    fn copy_to_user(&mut self, addr: u64, src: &[u8]) -> Result<(), Error> {
        let r = self.range(addr, src.len())?;
        self.mem[r].copy_from_slice(src);
        Ok(())
    }

    fn exit(&mut self, code: u8) {
        self.exited = Some(code);
    }

    fn write(&mut self, handle: Handle, buf: &[u8]) -> Result<usize, Error> {
        let n = self.write_limit.map_or(buf.len(), |l| l.min(buf.len()));
        let (path, pos) = self.handles.get_mut(&handle).ok_or(Error::BadHandle)?;
        if path == b"/dev/console" {
            self.stdout.extend_from_slice(&buf[..n]);
            return Ok(n);
        }
        let file = self.files.get_mut(path).unwrap();
        let at = *pos as usize;
        if file.len() < at + n {
            file.resize(at + n, 0);
        }
        file[at..at + n].copy_from_slice(&buf[..n]);
        *pos += n as u64;
        Ok(n)
    }

    fn read(&mut self, handle: Handle, buf: &mut [u8]) -> Result<usize, Error> {
        let (path, pos) = self.handles.get_mut(&handle).ok_or(Error::BadHandle)?;
        let file = self.files.get(path).ok_or(Error::Unsupported)?;
        let at = (*pos as usize).min(file.len());
        let n = buf.len().min(file.len() - at);
        buf[..n].copy_from_slice(&file[at..at + n]);
        *pos += n as u64;
        Ok(n)
    }

    fn open(&mut self, path: &[u8], how: &OpenHow) -> Result<Handle, Error> {
        self.opened.push(*how);
        match (self.files.contains_key(path), how.create, how.exclusive) {
            (true, _, true) if how.create => return Err(Error::AlreadyExists),
            (false, false, _) => return Err(Error::NotFound),
            (false, true, _) => {
                self.files.insert(path.to_vec(), Vec::new());
            }
            _ => {}
        }
        let h = (0..).find(|h| !self.handles.contains_key(h)).unwrap();
        self.handles.insert(h, (path.to_vec(), 0));
        Ok(h)
    }

    fn close(&mut self, handle: Handle) -> Result<(), Error> {
        self.handles.remove(&handle).map(|_| ()).ok_or(Error::BadHandle)
    }

    fn dup(&mut self, handle: Handle) -> Result<Handle, Error> {
        let entry = self.handles.get(&handle).ok_or(Error::BadHandle)?.clone();
        let h = (0..).find(|h| !self.handles.contains_key(h)).unwrap();
        self.handles.insert(h, entry);
        Ok(h)
    }

    fn seek(&mut self, handle: Handle, offset: i64, whence: Whence) -> Result<u64, Error> {
        let len = {
            let (path, _) = self.handles.get(&handle).ok_or(Error::BadHandle)?;
            self.files.get(path).map_or(0, Vec::len) as i64
        };
        let (_, pos) = self.handles.get_mut(&handle).unwrap();
        let base = match whence {
            Whence::Start => 0,
            Whence::Current => *pos as i64,
            Whence::End => len,
        };
        let new = base.checked_add(offset).filter(|&p| p >= 0).ok_or(Error::InvalidArgument)?;
        *pos = new as u64;
        Ok(*pos)
    }

    fn map(&mut self, hint: u64, len: u64, prot: Prot) -> Result<u64, Error> {
        self.maps.push((hint, len, prot));
        Ok(if hint != 0 { hint } else { 0x7000_0000 })
    }

    fn unmap(&mut self, addr: u64, len: u64) -> Result<(), Error> {
        self.unmaps.push((addr, len));
        Ok(())
    }

    fn protect(&mut self, _addr: u64, _len: u64, _prot: Prot) -> Result<(), Error> {
        Ok(())
    }

    fn clock(&self, clock: Clock) -> u64 {
        match clock {
            Clock::Monotonic => 5_000_000_000,
            Clock::Realtime => 1_700_000_000_000_000_000,
        }
    }

    fn sleep(&mut self, ns: u64) -> Result<(), Error> {
        self.slept.push(ns);
        Ok(())
    }

    fn yield_now(&mut self) {}

    fn spawn(&mut self, spawn: &Spawn) -> Result<u32, Error> {
        if spawn.path != b"/bin/child" {
            return Err(Error::NotFound);
        }
        self.spawned.push(spawn.clone());
        let pid = 100 + self.spawned.len() as u32;
        self.children.insert(pid, ExitStatus::Exited(42));
        Ok(pid)
    }

    fn wait(&mut self, pid: u32) -> Result<ExitStatus, Error> {
        self.children.remove(&pid).ok_or(Error::NoChild)
    }
}

#[test]
fn table_matches_the_numbers() {
    for (i, (number, name, _)) in table::TABLE.iter().enumerate() {
        assert_eq!(*number, i as u64, "{name} is out of order");
        assert_eq!(table::name(*number), Some(*name));
    }
    assert!(table::handler(table::TABLE.len() as u64).is_none());
    assert!(table::handler(u64::MAX).is_none());
}

#[test]
fn unknown_calls_are_unsupported_and_reported() {
    let mut k = Fake::new();
    let mut regs = SyscallRegs { rax: 999, ..SyscallRegs::default() };
    assert_eq!(Native::dispatch_with(&mut k, &mut regs), Outcome::Unimplemented { table: "native", number: 999 });
    assert_eq!(abi::decode(regs.rax), Err(Error::Unsupported));
    // Without the kernel's services every call is unsupported.
    let mut regs = SyscallRegs { rax: nr::WRITE, ..SyscallRegs::default() };
    assert!(matches!(Native::dispatch(&mut regs), Outcome::Unimplemented { number: nr::WRITE, .. }));
    assert_eq!(abi::decode(regs.rax), Err(Error::Unsupported));
}

#[test]
fn errors_round_trip_through_rax() {
    for e in Error::ALL {
        assert_eq!(abi::decode(abi::encode(Err(e))), Err(e));
        assert_eq!(Error::from_code(e.code()), e);
    }
    for v in [0, 1, 4096, u64::MAX - 4095] {
        assert_eq!(abi::decode(abi::encode(Ok(v))), Ok(v));
    }
    assert_eq!(abi::decode(u64::MAX - 4094), Err(Error::Io), "an unknown code is an I/O error");
    assert_eq!(Error::from_errno(2), Error::NotFound);
    assert_eq!(Error::from_errno(11), Error::WouldBlock);
    assert_eq!(Error::from_errno(9), Error::BadHandle);
    assert_eq!(Error::from_errno(500), Error::Io);
}

#[test]
fn exit_status_encoding() {
    for s in [ExitStatus::Exited(0), ExitStatus::Exited(255), ExitStatus::Signaled(9), ExitStatus::Faulted(14)] {
        assert_eq!(ExitStatus::decode(s.encode()), Some(s));
    }
    assert_eq!(ExitStatus::decode(3 << 8), None);
    assert!(ExitStatus::Exited(0).success() && !ExitStatus::Exited(1).success());
}

#[test]
fn write_copies_user_memory_to_the_handle() {
    let mut k = Fake::new();
    let msg = k.put(0, b"hello, native\n");
    assert_eq!(k.call(nr::WRITE, [1, msg, 14, 0, 0, 0]), Ok(14));
    assert_eq!(k.stdout, b"hello, native\n");
    assert_eq!(k.call(nr::WRITE, [9, msg, 14, 0, 0, 0]), Err(Error::BadHandle));
    assert_eq!(k.call(nr::WRITE, [1 << 40, msg, 14, 0, 0, 0]), Err(Error::BadHandle));
    assert_eq!(k.call(nr::WRITE, [1, 0x10, 14, 0, 0, 0]), Err(Error::BadAddress));
    assert_eq!(k.call(nr::WRITE, [1, u64::MAX - 4, 14, 0, 0, 0]), Err(Error::BadAddress));
    assert_eq!(k.call(nr::WRITE, [1, msg, 0, 0, 0, 0]), Ok(0));
}

#[test]
fn large_writes_go_in_chunks_and_stop_at_a_short_write() {
    let mut k = Fake::new();
    let data: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    let at = k.put(0, &data);
    assert_eq!(k.call(nr::WRITE, [1, at, data.len() as u64, 0, 0, 0]), Ok(100_000));
    assert_eq!(k.stdout, data);
    k.stdout.clear();
    k.write_limit = Some(1000);
    assert_eq!(k.call(nr::WRITE, [1, at, data.len() as u64, 0, 0, 0]), Ok(1000));
    // Memory that ends partway: what was copied goes out.
    k.write_limit = None;
    k.stdout.clear();
    let near_end = BASE + MEM as u64 - 70_000;
    assert_eq!(k.call(nr::WRITE, [1, near_end, 100_000, 0, 0, 0]), Ok(65_536));
}

#[test]
fn open_read_seek_close() {
    let mut k = Fake::new();
    k.files.insert(b"/etc/motd".to_vec(), b"Welcome to Poly.\n".to_vec());
    let path = k.put(0, b"/etc/motd");
    let h = k.call(nr::OPEN, [path, 9, abi::open::READ as u64, 0, 0, 0]).unwrap();
    assert_eq!(h, 0, "the lowest free handle");
    let buf = BASE + 0x1000;
    assert_eq!(k.call(nr::READ, [h, buf, 7, 0, 0, 0]), Ok(7));
    assert_eq!(&k.mem[0x1000..0x1007], b"Welcome");
    assert_eq!(k.call(nr::SEEK, [h, (-6i64) as u64, Whence::End as u64, 0, 0, 0]), Ok(11));
    assert_eq!(k.call(nr::READ, [h, buf, 100, 0, 0, 0]), Ok(6));
    assert_eq!(&k.mem[0x1000..0x1006], b"Poly.\n");
    assert_eq!(k.call(nr::READ, [h, buf, 100, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::SEEK, [h, 0, 3, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::READ, [h, 0x10, 10, 0, 0, 0]), Err(Error::BadAddress));
    let d = k.call(nr::DUP, [h, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(d, 2);
    assert_eq!(k.call(nr::CLOSE, [h, 0, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::CLOSE, [h, 0, 0, 0, 0, 0]), Err(Error::BadHandle));
    assert_eq!(k.call(nr::CLOSE, [d, 0, 0, 0, 0, 0]), Ok(0));
}

#[test]
fn open_checks_its_flags_and_path() {
    let mut k = Fake::new();
    let path = k.put(0, b"/tmp/new\0x");
    let (rd, wr, cr) = (abi::open::READ as u64, abi::open::WRITE as u64, abi::open::CREATE as u64);
    let bad = [
        (path, 8, 0, 0),                                     // neither read nor write
        (path, 8, rd | abi::open::TRUNCATE as u64, 0),       // truncate without write
        (path, 8, wr | abi::open::EXCLUSIVE as u64, 0),      // exclusive without create
        (path, 8, rd | abi::open::DIRECTORY as u64 | wr, 0), // a directory written
        (path, 8, rd | 1 << 20, 0),                          // an unknown flag
        (path, 8, wr | cr, 0o10000),                         // a mode out of range
        (path, 0, rd, 0),                                    // an empty path
        (path, 10, rd, 0),                                   // a NUL in it
    ];
    for (p, len, flags, mode) in bad {
        assert_eq!(k.call(nr::OPEN, [p, len, flags, mode, 0, 0]), Err(Error::InvalidArgument), "{flags:#x}");
    }
    assert_eq!(k.call(nr::OPEN, [path, 5000, rd, 0, 0, 0]), Err(Error::TooLarge));
    assert_eq!(k.call(nr::OPEN, [path, 8, rd, 0, 0, 0]), Err(Error::NotFound));
    assert!(k.opened.len() == 1, "only the valid request reached the kernel");
    let h = k.call(nr::OPEN, [path, 8, wr | cr | abi::open::TRUNCATE as u64, 0o644, 0, 0]).unwrap();
    let how = k.opened[1];
    assert!(how.write && how.create && how.truncate && !how.read && how.mode == 0o644);
    let data = k.put(0x100, b"abc");
    assert_eq!(k.call(nr::WRITE, [h, data, 3, 0, 0, 0]), Ok(3));
    assert_eq!(k.files[&b"/tmp/new"[..]], b"abc");
    let excl = wr | cr | abi::open::EXCLUSIVE as u64;
    assert_eq!(k.call(nr::OPEN, [path, 8, excl, 0o600, 0, 0]), Err(Error::AlreadyExists));
}

#[test]
fn map_unmap_protect_work_in_pages() {
    let mut k = Fake::new();
    let rw = (abi::prot::READ | abi::prot::WRITE) as u64;
    assert_eq!(k.call(nr::MAP, [0, 10, rw, 0, 0, 0]), Ok(0x7000_0000));
    assert_eq!(k.maps[0], (0, 4096, Prot { read: true, write: true, exec: false }));
    assert_eq!(k.call(nr::MAP, [0x4000_0000, 8192, rw, 0, 0, 0]), Ok(0x4000_0000));
    assert_eq!(k.call(nr::MAP, [0, 0, rw, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::MAP, [0, 4096, 8, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::MAP, [0, 4096, rw, 1, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::MAP, [123, 4096, rw, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::MAP, [0, u64::MAX, rw, 0, 0, 0]), Err(Error::NoMemory));
    assert_eq!(k.call(nr::UNMAP, [0x7000_0000, 1, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.unmaps, [(0x7000_0000, 4096)]);
    assert_eq!(k.call(nr::UNMAP, [0x7000_0001, 4096, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::UNMAP, [!4095, 8192, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::PROTECT, [0x7000_0000, 4096, abi::prot::READ as u64, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::PROTECT, [0x7000_0000, 4096, 16, 0, 0, 0]), Err(Error::InvalidArgument));
}

#[test]
fn time_sleep_yield_and_ids() {
    let mut k = Fake::new();
    assert_eq!(k.call(nr::CLOCK, [Clock::Monotonic as u64, 0, 0, 0, 0, 0]), Ok(5_000_000_000));
    assert_eq!(k.call(nr::CLOCK, [Clock::Realtime as u64, 0, 0, 0, 0, 0]), Ok(1_700_000_000_000_000_000));
    assert_eq!(k.call(nr::CLOCK, [2, 0, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::SLEEP, [1_000_000, 0, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.slept, [1_000_000]);
    assert_eq!(k.call(nr::YIELD, [0; 6]), Ok(0));
    assert_eq!(k.call(nr::PROCESS_ID, [0; 6]), Ok(7));
}

#[test]
fn exit_tells_the_kernel_the_low_byte() {
    let mut k = Fake::new();
    assert_eq!(k.call(nr::EXIT, [0x1_2a, 0, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.exited, Some(0x2a));
}

/// Lay out a spawn request in the fake's memory: the strings, the [`Str`]
/// arrays and the request. Returns the request's address.
fn put_spawn(k: &mut Fake, path: &[u8], args: &[&[u8]], env: &[&[u8]], stdio: [u32; 3]) -> u64 {
    let mut at = 0x4000;
    let mut put = |k: &mut Fake, b: &[u8]| {
        let addr = k.put(at, b);
        at += b.len().next_multiple_of(16);
        addr
    };
    let path = Str { ptr: put(k, path), len: path.len() as u64 };
    let mut array = |k: &mut Fake, list: &[&[u8]]| {
        let strs: Vec<Str> = list.iter().map(|s| Str { ptr: put(k, s), len: s.len() as u64 }).collect();
        let bytes: Vec<u8> = strs.iter().flat_map(|s| [s.ptr.to_le_bytes(), s.len.to_le_bytes()].concat()).collect();
        put(k, &bytes)
    };
    let args_at = array(k, args);
    let env_at = array(k, env);
    let req = SpawnRequest {
        path,
        args: args_at,
        arg_count: args.len() as u64,
        env: env_at,
        env_count: env.len() as u64,
        stdio,
        flags: 0,
    };
    let mut raw = Vec::new();
    for v in [req.path.ptr, req.path.len, req.args, req.arg_count, req.env, req.env_count] {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    for v in req.stdio.iter().chain([&req.flags]) {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(raw.len(), SpawnRequest::SIZE);
    assert_eq!(SpawnRequest::from_bytes(raw.as_slice().try_into().unwrap()), req);
    k.put(0x8000, &raw)
}

#[test]
fn spawn_copies_the_request_and_wait_reaps_once() {
    let mut k = Fake::new();
    let req = put_spawn(&mut k, b"/bin/child", &[b"child", b"--exit", b"42"], &[b"HOME=/"], [0, 1, abi::STDIO_NONE]);
    let pid = k.call(nr::SPAWN, [req, SpawnRequest::SIZE as u64, 0, 0, 0, 0]).unwrap();
    assert_eq!(pid, 101);
    let s = &k.spawned[0];
    assert_eq!(s.path, b"/bin/child");
    assert_eq!(s.args, [b"child".to_vec(), b"--exit".to_vec(), b"42".to_vec()]);
    assert_eq!(s.env, [b"HOME=/".to_vec()]);
    assert_eq!(s.stdio, [Some(0), Some(1), None]);
    assert_eq!(k.call(nr::WAIT, [pid, 0, 0, 0, 0, 0]).map(ExitStatus::decode), Ok(Some(ExitStatus::Exited(42))));
    assert_eq!(k.call(nr::WAIT, [pid, 0, 0, 0, 0, 0]), Err(Error::NoChild));
    assert_eq!(k.call(nr::WAIT, [u64::MAX, 0, 0, 0, 0, 0]), Err(Error::NoChild));
}

#[test]
fn spawn_checks_its_request() {
    let mut k = Fake::new();
    let req = put_spawn(&mut k, b"/bin/child", &[b"child"], &[], [0, 1, 2]);
    assert_eq!(k.call(nr::SPAWN, [req, 63, 0, 0, 0, 0]), Err(Error::InvalidArgument), "the wrong size");
    assert_eq!(k.call(nr::SPAWN, [0x10, 64, 0, 0, 0, 0]), Err(Error::BadAddress));
    // Flags, too many strings, a NUL in an argument, a missing program.
    let flags = req + 60;
    k.copy_to_user(flags, &1u32.to_le_bytes()).unwrap();
    assert_eq!(k.call(nr::SPAWN, [req, 64, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    k.copy_to_user(flags, &0u32.to_le_bytes()).unwrap();
    k.copy_to_user(req + 24, &5000u64.to_le_bytes()).unwrap();
    assert_eq!(k.call(nr::SPAWN, [req, 64, 0, 0, 0, 0]), Err(Error::TooLarge));
    let req = put_spawn(&mut k, b"/bin/child", &[b"ch\0ld"], &[], [0, 1, 2]);
    assert_eq!(k.call(nr::SPAWN, [req, 64, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    let req = put_spawn(&mut k, b"/bin/nope", &[b"nope"], &[], [0, 1, 2]);
    assert_eq!(k.call(nr::SPAWN, [req, 64, 0, 0, 0, 0]), Err(Error::NotFound));
    assert!(k.spawned.is_empty());
}

#[test]
fn the_abi_note_matches_what_binfmt_looks_for() {
    let n = abi::AbiNote::CURRENT;
    assert_eq!(n.namesz as usize, abi::NOTE_OWNER.len());
    assert_eq!(&n.name[..abi::NOTE_OWNER.len()], abi::NOTE_OWNER);
    assert_eq!(n.kind, abi::NT_POLY_ABI);
    assert_eq!((n.descsz, n.abi), (4, abi::ABI_VERSION));
    assert_eq!(core::mem::size_of::<abi::AbiNote>(), 24);
    assert_eq!(core::mem::size_of::<SpawnRequest>(), SpawnRequest::SIZE);
    assert_eq!(core::mem::size_of::<Str>(), Str::SIZE);
}
