//! Host tests of the native dispatch against a fake kernel.

extern crate std;

use crate::abi::{
    self, Clock, Error, ExitStatus, FileInfo, Handle, PeerInfo, PollEntry, SpawnRequest, Str, Whence, nr,
};
use crate::kernel::{NativeKernel, OpenHow, Prot, Signal, SocketKind, SocketName, Spawn};
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
    /// Children have not ended yet (`wait` without hanging fails).
    unfinished: bool,
    controls: Vec<(Handle, u32, u64)>,
    /// `map_handle` calls: handle, offset, length, protection, shared.
    handle_maps: Vec<(Handle, u64, u64, Prot, bool)>,
    seals: BTreeMap<Handle, u32>,
    nonblocking: Vec<(Handle, bool)>,
    /// File system calls, in order: name and path(s).
    fs_ops: Vec<(&'static str, Vec<u8>)>,
    /// A directory's entries and how many were read.
    dir: (Vec<(u8, Vec<u8>)>, usize),
    /// Sockets: their peer and what came in (bytes, the paths of the
    /// handles that came with them).
    sockets: BTreeMap<Handle, FakeSocket>,
    listening: BTreeMap<Vec<u8>, Handle>,
    signals: Vec<(u32, Signal)>,
}

/// A fake socket: its peer, and the messages that came in (bytes, the
/// paths of the handles that came with them).
type FakeSocket = (Option<Handle>, Vec<(Vec<u8>, Vec<Vec<u8>>)>);

/// A new handle of the fake: the lowest free number.
fn new_handle(handles: &mut BTreeMap<Handle, (Vec<u8>, u64)>, path: &[u8]) -> Handle {
    let h = (0..).find(|h| !handles.contains_key(h)).unwrap();
    handles.insert(h, (path.to_vec(), 0));
    h
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

    fn wait(&mut self, pid: u32, no_hang: bool) -> Result<ExitStatus, Error> {
        if !self.children.contains_key(&pid) {
            return Err(Error::NoChild);
        }
        if self.unfinished && no_hang {
            return Err(Error::WouldBlock);
        }
        self.children.remove(&pid).ok_or(Error::NoChild)
    }

    fn control(&mut self, handle: Handle, request: u32, arg: u64) -> Result<u64, Error> {
        if !self.handles.contains_key(&handle) {
            return Err(Error::BadHandle);
        }
        self.controls.push((handle, request, arg));
        Ok(7)
    }

    fn poll(&mut self, entries: &mut [PollEntry], timeout_ns: Option<u64>) -> Result<usize, Error> {
        let mut ready = 0;
        for e in entries.iter_mut() {
            if e.handle == abi::NO_HANDLE {
                continue;
            }
            e.revents = match self.sockets.get(&e.handle) {
                _ if !self.handles.contains_key(&e.handle) => abi::events::INVALID,
                Some((_, inbox)) if !inbox.is_empty() => e.events & abi::events::READABLE,
                Some((None, _)) => abi::events::HANGUP,
                _ => e.events & abi::events::WRITABLE,
            };
            ready += usize::from(e.revents != 0);
        }
        if ready == 0 && timeout_ns.is_none() {
            return Err(Error::Interrupted);
        }
        Ok(ready)
    }

    fn map_handle(&mut self, handle: Handle, offset: u64, len: u64, prot: Prot, shared: bool) -> Result<u64, Error> {
        if !self.handles.contains_key(&handle) {
            return Err(Error::BadHandle);
        }
        self.handle_maps.push((handle, offset, len, prot, shared));
        Ok(0x6000_0000)
    }

    fn memory_object(&mut self, size: u64) -> Result<Handle, Error> {
        let name = alloc::format!("memfd:{size}").into_bytes();
        self.files.insert(name.clone(), vec![0; size as usize]);
        Ok(new_handle(&mut self.handles, &name))
    }

    fn seal(&mut self, handle: Handle, seals: u32) -> Result<(), Error> {
        *self.seals.entry(handle).or_default() |= seals;
        Ok(())
    }

    fn set_nonblocking(&mut self, handle: Handle, on: bool) -> Result<(), Error> {
        self.nonblocking.push((handle, on));
        Ok(())
    }

    fn handle_info(&mut self, handle: Handle) -> Result<FileInfo, Error> {
        let (path, _) = self.handles.get(&handle).ok_or(Error::BadHandle)?;
        let size = self.files.get(path).map_or(0, Vec::len) as u64;
        Ok(FileInfo { size, kind: abi::kind::FILE, mode: 0o644, ..FileInfo::default() })
    }

    fn stat(&mut self, path: &[u8]) -> Result<FileInfo, Error> {
        let file = self.files.get(path).ok_or(Error::NotFound)?;
        Ok(FileInfo {
            size: file.len() as u64,
            kind: abi::kind::FILE,
            mode: 0o600,
            device: 0xe2_0000_0080,
            inode: 9,
            ..FileInfo::default()
        })
    }

    fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Error> {
        let data = self.files.remove(from).ok_or(Error::NotFound)?;
        self.files.insert(to.to_vec(), data);
        self.fs_ops.push(("rename", [from, b" ", to].concat()));
        Ok(())
    }

    fn remove(&mut self, path: &[u8], directory: bool) -> Result<(), Error> {
        self.fs_ops.push((if directory { "rmdir" } else { "remove" }, path.to_vec()));
        self.files.remove(path).map(|_| ()).ok_or(Error::NotFound)
    }

    fn make_dir(&mut self, path: &[u8], mode: u16) -> Result<(), Error> {
        self.fs_ops.push(("mkdir", [path, &mode.to_le_bytes()].concat()));
        Ok(())
    }

    fn set_mode(&mut self, path: &[u8], mode: u16) -> Result<(), Error> {
        self.fs_ops.push(("chmod", [path, &mode.to_le_bytes()].concat()));
        Ok(())
    }

    fn read_dir(&mut self, handle: Handle, emit: &mut dyn FnMut(u8, &[u8]) -> bool) -> Result<usize, Error> {
        if !self.handles.contains_key(&handle) {
            return Err(Error::BadHandle);
        }
        let (entries, at) = &mut self.dir;
        let start = *at;
        while let Some((kind, name)) = entries.get(*at) {
            if !emit(*kind, name) {
                break;
            }
            *at += 1;
        }
        Ok(*at - start)
    }

    fn sync(&mut self) {
        self.fs_ops.push(("sync", Vec::new()));
    }

    fn pipe(&mut self) -> Result<[Handle; 2], Error> {
        Ok([new_handle(&mut self.handles, b"pipe:r"), new_handle(&mut self.handles, b"pipe:w")])
    }

    fn make_fifo(&mut self, path: &[u8], _mode: u16) -> Result<(), Error> {
        self.fs_ops.push(("mkfifo", path.to_vec()));
        Ok(())
    }

    fn socket(&mut self, kind: SocketKind, _nonblock: bool) -> Result<Handle, Error> {
        if kind == SocketKind::SeqPacket {
            return Err(Error::Unsupported);
        }
        let h = new_handle(&mut self.handles, b"socket");
        self.sockets.insert(h, (None, Vec::new()));
        Ok(h)
    }

    fn socket_pair(&mut self, _kind: SocketKind, _nonblock: bool) -> Result<[Handle; 2], Error> {
        let a = new_handle(&mut self.handles, b"socket");
        let b = new_handle(&mut self.handles, b"socket");
        self.sockets.insert(a, (Some(b), Vec::new()));
        self.sockets.insert(b, (Some(a), Vec::new()));
        Ok([a, b])
    }

    fn bind(&mut self, handle: Handle, name: &SocketName) -> Result<(), Error> {
        let key = match name {
            SocketName::Path(p) => p.clone(),
            SocketName::Abstract(n) => [b"@", &n[..]].concat(),
        };
        if self.listening.contains_key(&key) {
            return Err(Error::AlreadyExists);
        }
        self.listening.insert(key, handle);
        Ok(())
    }

    fn listen(&mut self, handle: Handle, _backlog: u32) -> Result<(), Error> {
        self.sockets.get(&handle).map(|_| ()).ok_or(Error::BadHandle)
    }

    fn accept(&mut self, handle: Handle, nonblock: bool) -> Result<Handle, Error> {
        let (_, inbox) = self.sockets.get_mut(&handle).ok_or(Error::BadHandle)?;
        if inbox.is_empty() {
            return Err(if nonblock { Error::WouldBlock } else { Error::Interrupted });
        }
        inbox.remove(0);
        let h = new_handle(&mut self.handles, b"socket");
        self.sockets.insert(h, (None, Vec::new()));
        Ok(h)
    }

    fn connect(&mut self, handle: Handle, name: &SocketName) -> Result<(), Error> {
        let key = match name {
            SocketName::Path(p) => p.clone(),
            SocketName::Abstract(n) => [b"@", &n[..]].concat(),
        };
        let listener = *self.listening.get(&key).ok_or(Error::ConnectionRefused)?;
        self.sockets.get_mut(&listener).unwrap().1.push((Vec::new(), Vec::new()));
        self.sockets.get_mut(&handle).ok_or(Error::BadHandle)?.0 = Some(listener);
        Ok(())
    }

    fn send(&mut self, handle: Handle, data: &[u8], handles: &[Handle]) -> Result<usize, Error> {
        let peer = self.sockets.get(&handle).ok_or(Error::BadHandle)?.0.ok_or(Error::BrokenPipe)?;
        let paths = handles
            .iter()
            .map(|h| self.handles.get(h).map(|(p, _)| p.clone()).ok_or(Error::BadHandle))
            .collect::<Result<Vec<_>, _>>()?;
        self.sockets.get_mut(&peer).unwrap().1.push((data.to_vec(), paths));
        Ok(data.len())
    }

    fn recv(&mut self, handle: Handle, buf: &mut [u8], max_handles: usize) -> Result<(usize, Vec<Handle>), Error> {
        let (_, inbox) = self.sockets.get_mut(&handle).ok_or(Error::BadHandle)?;
        if inbox.is_empty() {
            return Ok((0, Vec::new()));
        }
        let (data, paths) = inbox.remove(0);
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        let got = paths.iter().take(max_handles).map(|p| new_handle(&mut self.handles, p)).collect();
        Ok((n, got))
    }

    fn socket_peer(&mut self, handle: Handle) -> Result<PeerInfo, Error> {
        self.sockets.get(&handle).ok_or(Error::BadHandle)?;
        Ok(PeerInfo { pid: 42, uid: 0, gid: 0 })
    }

    fn process_handle(&mut self, pid: u32) -> Result<Handle, Error> {
        if !self.children.contains_key(&pid) {
            return Err(Error::NoChild);
        }
        Ok(new_handle(&mut self.handles, b"process"))
    }

    fn signal(&mut self, pid: u32, what: Signal) -> Result<(), Error> {
        if !self.children.contains_key(&pid) {
            return Err(Error::NotFound);
        }
        self.signals.push((pid, what));
        Ok(())
    }

    fn net_interfaces(&mut self) -> Vec<(u32, Vec<u8>)> {
        use abi::InterfaceRecord as R;
        alloc::vec![(R::UP | R::RUNNING | R::LOOPBACK | R::ADDRESSED, b"lo".to_vec()), (R::UP, b"eth0".to_vec())]
    }

    fn truncate(&mut self, handle: Handle, len: u64) -> Result<(), Error> {
        let (path, _) = self.handles.get(&handle).ok_or(Error::BadHandle)?;
        self.files.get_mut(path).ok_or(Error::Unsupported)?.resize(len as usize, 0);
        Ok(())
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
    put_spawn_with(k, path, args, env, stdio, &[])
}

/// [`put_spawn`] with extra handles (an ABI version 2 request).
fn put_spawn_with(
    k: &mut Fake,
    path: &[u8],
    args: &[&[u8]],
    env: &[&[u8]],
    stdio: [u32; 3],
    handles: &[(u32, u32)],
) -> u64 {
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
    let pairs: Vec<u8> = handles.iter().flat_map(|(f, t)| [f.to_le_bytes(), t.to_le_bytes()].concat()).collect();
    let handles_at = put(k, &pairs);
    let req = SpawnRequest {
        path,
        args: args_at,
        arg_count: args.len() as u64,
        env: env_at,
        env_count: env.len() as u64,
        stdio,
        flags: 0,
        handles: handles_at,
        handle_count: handles.len() as u64,
    };
    let mut raw = Vec::new();
    for v in [req.path.ptr, req.path.len, req.args, req.arg_count, req.env, req.env_count] {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    for v in req.stdio.iter().chain([&req.flags]) {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    for v in [req.handles, req.handle_count] {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(raw.len(), SpawnRequest::SIZE);
    assert_eq!(SpawnRequest::from_bytes(&raw), req);
    let v1 = SpawnRequest::from_bytes(&raw[..SpawnRequest::SIZE_V1]);
    assert_eq!((v1.handles, v1.handle_count), (0, 0), "a version 1 request has no extra handles");
    k.put(0x8000, &raw)
}

#[test]
fn spawn_copies_the_request_and_wait_reaps_once() {
    let mut k = Fake::new();
    let (args, env): (&[&[u8]], &[&[u8]]) = (&[b"child", b"--exit", b"42"], &[b"HOME=/"]);
    let req = put_spawn_with(&mut k, b"/bin/child", args, env, [0, 1, abi::STDIO_NONE], &[(5, 10), (6, 11)]);
    let pid = k.call(nr::SPAWN, [req, SpawnRequest::SIZE as u64, 0, 0, 0, 0]).unwrap();
    assert_eq!(pid, 101);
    let s = &k.spawned[0];
    assert_eq!(s.path, b"/bin/child");
    assert_eq!(s.args, [b"child".to_vec(), b"--exit".to_vec(), b"42".to_vec()]);
    assert_eq!(s.env, [b"HOME=/".to_vec()]);
    assert_eq!(s.stdio, [Some(0), Some(1), None]);
    assert_eq!(s.handles, [(5, 10), (6, 11)]);
    // Not ended yet without waiting, then reaped once.
    k.unfinished = true;
    assert_eq!(k.call(nr::WAIT, [pid, abi::wait::NO_HANG, 0, 0, 0, 0]), Err(Error::WouldBlock));
    k.unfinished = false;
    assert_eq!(k.call(nr::WAIT, [pid, 2, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::WAIT, [pid, 0, 0, 0, 0, 0]).map(ExitStatus::decode), Ok(Some(ExitStatus::Exited(42))));
    assert_eq!(k.call(nr::WAIT, [pid, 0, 0, 0, 0, 0]), Err(Error::NoChild));
    assert_eq!(k.call(nr::WAIT, [u64::MAX, 0, 0, 0, 0, 0]), Err(Error::NoChild));
    // A version 1 request (64 bytes) still works, without extra handles.
    let req = put_spawn(&mut k, b"/bin/child", &[b"child"], &[], [0, 1, 2]);
    assert_eq!(k.call(nr::SPAWN, [req, SpawnRequest::SIZE_V1 as u64, 0, 0, 0, 0]), Ok(102));
    assert!(k.spawned[1].handles.is_empty());
    // Extra handles may not name 0, 1 or 2, or one child handle twice.
    for bad in [&[(5u32, 2u32)][..], &[(5, 10), (6, 10)]] {
        let req = put_spawn_with(&mut k, b"/bin/child", &[b"child"], &[], [0, 1, 2], bad);
        assert_eq!(k.call(nr::SPAWN, [req, SpawnRequest::SIZE as u64, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    }
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
    assert_eq!(core::mem::size_of::<PollEntry>(), PollEntry::SIZE);
    assert_eq!(core::mem::size_of::<FileInfo>(), FileInfo::SIZE);
    assert_eq!(core::mem::size_of::<PeerInfo>(), PeerInfo::SIZE);
    assert_eq!(core::mem::size_of::<abi::HandleMap>(), abi::HandleMap::SIZE);
}

// ---- ABI version 2 --------------------------------------------------------------

#[test]
fn control_passes_the_request_through() {
    let mut k = Fake::new();
    assert_eq!(k.call(nr::CONTROL, [1, 0xc010_64a0, 0x1234, 0, 0, 0]), Ok(7));
    assert_eq!(k.controls, [(1, 0xc010_64a0, 0x1234)]);
    assert_eq!(k.call(nr::CONTROL, [1, 1 << 32, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::CONTROL, [9, 1, 0, 0, 0, 0]), Err(Error::BadHandle));
}

#[test]
fn poll_copies_entries_in_and_reports_back() {
    let mut k = Fake::new();
    let [a, b] = k.socket_pair(SocketKind::Stream, false).unwrap();
    k.send(a, b"x", &[]).unwrap();
    let entries = [
        PollEntry { handle: b, events: abi::events::READABLE, revents: 0x7777 },
        PollEntry { handle: a, events: abi::events::READABLE, revents: 0 },
        PollEntry { handle: abi::NO_HANDLE, events: abi::events::READABLE, revents: 0 },
        PollEntry { handle: 50, events: abi::events::READABLE, revents: 0 },
    ];
    let bytes: Vec<u8> = entries.iter().flat_map(|e| e.to_bytes()).collect();
    let at = k.put(0x100, &bytes);
    assert_eq!(k.call(nr::POLL, [at, 4, 0, 0, 0, 0]), Ok(2));
    let got: Vec<PollEntry> =
        k.mem[0x100..0x120].chunks_exact(8).map(|c| PollEntry::from_bytes(c.try_into().unwrap())).collect();
    assert_eq!(got[0].revents, abi::events::READABLE, "revents start from 0");
    assert_eq!(got[1].revents, 0);
    assert_eq!(got[2].revents, 0, "no handle: skipped");
    assert_eq!(got[3].revents, abi::events::INVALID);
    assert_eq!(k.call(nr::POLL, [at, 2000, 0, 0, 0, 0]), Err(Error::TooLarge));
    assert_eq!(k.call(nr::POLL, [0x10, 1, 0, 0, 0, 0]), Err(Error::BadAddress));
    // Forever, and nothing ready: the fake says interrupted.
    let at = k.put(0x200, &PollEntry { handle: a, events: abi::events::READABLE, revents: 0 }.to_bytes());
    assert_eq!(k.call(nr::POLL, [at, 1, abi::FOREVER, 0, 0, 0]), Err(Error::Interrupted));
    assert_eq!(k.call(nr::POLL, [at, 0, 5, 0, 0, 0]), Ok(0), "no entries: a plain sleep");
}

#[test]
fn memory_objects_seal_truncate_and_map() {
    let mut k = Fake::new();
    let h = k.call(nr::MEMORY_OBJECT, [100, 0, 0, 0, 0, 0]).unwrap();
    let all = abi::seal::SHRINK | abi::seal::GROW | abi::seal::WRITE;
    assert_eq!(k.call(nr::SEAL, [h, u64::from(all), 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.seals[&(h as u32)], all);
    assert_eq!(k.call(nr::SEAL, [h, 0x10, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::TRUNCATE, [h, 8192, 0, 0, 0, 0]), Ok(0));
    let info = k.put(0x300, &[0; FileInfo::SIZE]);
    assert_eq!(k.call(nr::HANDLE_INFO, [h, info, 0, 0, 0, 0]), Ok(0));
    let got = FileInfo::from_bytes(k.mem[0x300..0x320].try_into().unwrap());
    assert_eq!((got.size, got.kind, got.mode), (8192, abi::kind::FILE, 0o644));
    let r = u64::from(abi::prot::READ);
    assert_eq!(k.call(nr::MAP_HANDLE, [h, 4096, 100, r, abi::map::SHARED, 0]), Ok(0x6000_0000));
    assert_eq!(k.handle_maps[0], (h as u32, 4096, 4096, Prot { read: true, write: false, exec: false }, true));
    assert_eq!(k.call(nr::MAP_HANDLE, [h, 100, 4096, r, 0, 0]), Err(Error::InvalidArgument), "offset not a page");
    assert_eq!(k.call(nr::MAP_HANDLE, [h, 0, 4096, r, 2, 0]), Err(Error::InvalidArgument), "unknown flag");
    assert_eq!(k.call(nr::MAP_HANDLE, [h, 0, 0, r, 0, 0]), Err(Error::InvalidArgument), "empty");
    assert_eq!(k.call(nr::SET_NONBLOCKING, [h, 1, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.nonblocking, [(h as u32, true)]);
}

#[test]
fn file_system_calls_check_paths_and_modes() {
    let mut k = Fake::new();
    k.files.insert(b"/tmp/a".to_vec(), b"abc".to_vec());
    let (a, b) = (k.put(0, b"/tmp/a"), k.put(0x10, b"/tmp/b"));
    let info = BASE + 0x400;
    assert_eq!(k.call(nr::STAT, [a, 6, info, 0, 0, 0]), Ok(0));
    let got = FileInfo::from_bytes(k.mem[0x400..0x420].try_into().unwrap());
    assert_eq!((got.size, got.device, got.inode, got.mode), (3, 0xe2_0000_0080, 9, 0o600));
    assert_eq!(k.call(nr::STAT, [b, 6, info, 0, 0, 0]), Err(Error::NotFound));
    assert_eq!(k.call(nr::RENAME, [a, 6, b, 6, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::MAKE_DIR, [a, 6, 0o700, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::MAKE_DIR, [a, 6, 0o10000, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::SET_MODE, [a, 6, 0o1777, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::MAKE_FIFO, [a, 6, 0o600, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::REMOVE, [b, 6, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::REMOVE, [a, 6, abi::remove::DIRECTORY, 0, 0, 0]), Err(Error::NotFound));
    assert_eq!(k.call(nr::REMOVE, [a, 6, 4, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::SYNC, [0; 6]), Ok(0));
    let ops: Vec<&str> = k.fs_ops.iter().map(|(n, _)| *n).collect();
    assert_eq!(ops, ["rename", "mkdir", "chmod", "mkfifo", "remove", "rmdir", "sync"]);
    assert_eq!(k.fs_ops[1].1, [&b"/tmp/a"[..], &0o700u16.to_le_bytes()].concat());
}

#[test]
fn read_dir_fills_records_and_resumes() {
    let mut k = Fake::new();
    k.dir.0 = (0..20).map(|i| (abi::kind::FILE, alloc::format!("file-{i:02}").into_bytes())).collect();
    let buf = BASE + 0x1000;
    // Room for three records of 3 + 7 bytes.
    assert_eq!(k.call(nr::READ_DIR, [1, buf, 35, 0, 0, 0]), Ok(30));
    let (first, rest) = abi::DirRecord::parse(&k.mem[0x1000..0x101e]).unwrap();
    assert_eq!((first.kind, first.name), (abi::kind::FILE, &b"file-00"[..]));
    let (third, rest) = abi::DirRecord::parse(abi::DirRecord::parse(rest).unwrap().1).unwrap();
    assert_eq!(third.name, b"file-02");
    assert!(rest.is_empty());
    assert_eq!(k.call(nr::READ_DIR, [1, buf, 5, 0, 0, 0]), Err(Error::TooLarge), "not even one fits");
    let mut seen = 3;
    loop {
        let n = k.call(nr::READ_DIR, [1, buf, 4096, 0, 0, 0]).unwrap();
        if n == 0 {
            break;
        }
        seen += n as usize / 10;
    }
    assert_eq!(seen, 20);
}

#[test]
fn sockets_pass_bytes_and_handles() {
    let mut k = Fake::new();
    let out = BASE + 0x500;
    assert_eq!(k.call(nr::SOCKET_PAIR, [abi::socket::STREAM, 0, out, 0, 0, 0]), Ok(0));
    let a = u32::from_le_bytes(k.mem[0x500..0x504].try_into().unwrap());
    let b = u32::from_le_bytes(k.mem[0x504..0x508].try_into().unwrap());
    k.files.insert(b"/etc/motd".to_vec(), b"hi".to_vec());
    let motd = k.put(0x600, b"/etc/motd");
    let f = k.call(nr::OPEN, [motd, 9, abi::open::READ as u64, 0, 0, 0]).unwrap() as u32;
    let msg = k.put(0x700, b"hello");
    let hs = k.put(0x710, &[f.to_le_bytes(), 1u32.to_le_bytes()].concat());
    assert_eq!(k.call(nr::SEND, [a.into(), msg, 5, hs, 2, 0]), Ok(5));
    assert_eq!(k.call(nr::SEND, [a.into(), msg, 5, hs, 300, 0]), Err(Error::TooLarge));
    // Room for one handle: the second is not taken.
    let (buf, got, count) = (BASE + 0x800, BASE + 0x900, BASE + 0x910);
    k.copy_to_user(count, &1u32.to_le_bytes()).unwrap();
    assert_eq!(k.call(nr::RECV, [b.into(), buf, 64, got, count, 0]), Ok(5));
    assert_eq!(&k.mem[0x800..0x805], b"hello");
    assert_eq!(u32::from_le_bytes(k.mem[0x910..0x914].try_into().unwrap()), 1);
    let new = u32::from_le_bytes(k.mem[0x900..0x904].try_into().unwrap());
    assert_eq!(k.handles[&new].0, b"/etc/motd", "the passed handle names the same file");
    // A receive whose handles cannot be written back closes them.
    k.send(a, b"again", &[f]).unwrap();
    let before = k.handles.len();
    assert_eq!(k.call(nr::RECV, [b.into(), buf, 64, 0x10, count, 0]), Err(Error::BadAddress));
    assert_eq!(k.handles.len(), before);
    let peer = BASE + 0xa00;
    assert_eq!(k.call(nr::SOCKET_PEER, [b.into(), peer, 0, 0, 0, 0]), Ok(0));
    assert_eq!(PeerInfo::from_bytes(k.mem[0xa00..0xa0c].try_into().unwrap()).pid, 42);
    assert_eq!(k.call(nr::SOCKET, [7, 0, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::SOCKET, [abi::socket::STREAM, 2, 0, 0, 0, 0]), Err(Error::InvalidArgument));
}

#[test]
fn sockets_bind_listen_connect_accept_by_path_and_abstract_name() {
    let mut k = Fake::new();
    let l = k.call(nr::SOCKET, [abi::socket::STREAM, abi::socket::NONBLOCK, 0, 0, 0, 0]).unwrap();
    let path = k.put(0, b"/run/user/0/wayland-0");
    assert_eq!(k.call(nr::BIND, [l, path, 21, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::BIND, [l, path, 21, 0, 0, 0]), Err(Error::AlreadyExists));
    assert_eq!(k.call(nr::LISTEN, [l, 128, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::ACCEPT, [l, abi::socket::NONBLOCK, 0, 0, 0, 0]), Err(Error::WouldBlock));
    let c = k.call(nr::SOCKET, [abi::socket::STREAM, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(k.call(nr::CONNECT, [c, path, 21, 0, 0, 0]), Ok(0));
    assert!(k.call(nr::ACCEPT, [l, 0, 0, 0, 0, 0]).is_ok());
    let x = k.put(0x40, b"/tmp/.X11-unix/X0");
    let lx = k.call(nr::SOCKET, [abi::socket::STREAM, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(k.call(nr::BIND, [lx, x, 17, abi::socket::ABSTRACT, 0, 0]), Ok(0));
    assert!(k.listening.contains_key(&b"@/tmp/.X11-unix/X0"[..]));
    assert_eq!(k.call(nr::CONNECT, [c, x, 17, 0, 0, 0]), Err(Error::ConnectionRefused), "not the path");
    assert_eq!(k.call(nr::BIND, [lx, x, 0, abi::socket::ABSTRACT, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::BIND, [lx, x, 200, abi::socket::ABSTRACT, 0, 0]), Err(Error::TooLarge));
    assert_eq!(k.call(nr::BIND, [lx, x, 17, 2, 0, 0]), Err(Error::InvalidArgument));
}

#[test]
fn pipes_processes_signals_and_interfaces() {
    let mut k = Fake::new();
    let out = BASE + 0x500;
    assert_eq!(k.call(nr::PIPE, [out, 0, 0, 0, 0, 0]), Ok(0));
    let r = u32::from_le_bytes(k.mem[0x500..0x504].try_into().unwrap());
    assert_eq!(k.handles[&r].0, b"pipe:r");
    let before = k.handles.len();
    assert_eq!(k.call(nr::PIPE, [0x10, 0, 0, 0, 0, 0]), Err(Error::BadAddress));
    assert_eq!(k.handles.len(), before, "handles nobody learns of are closed");
    k.children.insert(300, ExitStatus::Exited(0));
    assert!(k.call(nr::PROCESS_HANDLE, [300, 0, 0, 0, 0, 0]).is_ok());
    assert_eq!(k.call(nr::PROCESS_HANDLE, [301, 0, 0, 0, 0, 0]), Err(Error::NoChild));
    assert_eq!(k.call(nr::SIGNAL, [300, abi::signal::TERMINATE, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.call(nr::SIGNAL, [300, abi::signal::PROBE, 0, 0, 0, 0]), Ok(0));
    assert_eq!(k.signals, [(300, Signal::Terminate), (300, Signal::Probe)]);
    assert_eq!(k.call(nr::SIGNAL, [300, 9, 0, 0, 0, 0]), Err(Error::InvalidArgument));
    assert_eq!(k.call(nr::SIGNAL, [0, 0, 0, 0, 0, 0]), Err(Error::NotFound));
    let buf = BASE + 0x2000;
    let n = k.call(nr::NET_INTERFACES, [buf, 256, 0, 0, 0, 0]).unwrap() as usize;
    let (lo, rest) = abi::InterfaceRecord::parse(&k.mem[0x2000..0x2000 + n]).unwrap();
    assert_eq!(lo.name, b"lo");
    assert_ne!(lo.flags & abi::InterfaceRecord::LOOPBACK, 0);
    let (eth, rest) = abi::InterfaceRecord::parse(rest).unwrap();
    assert_eq!((eth.name, eth.flags), (&b"eth0"[..], abi::InterfaceRecord::UP));
    assert!(rest.is_empty());
    assert_eq!(k.call(nr::NET_INTERFACES, [buf, 4, 0, 0, 0, 0]), Err(Error::TooLarge));
    let _ = SocketName::Path(Vec::new());
}
