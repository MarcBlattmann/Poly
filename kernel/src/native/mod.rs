//! The native personality in the kernel (ADR 0125): the services
//! `poly_personality_native::NativeKernel` asks for, on the kernel's
//! shared objects. The checking and decoding of every call is in the
//! personality crate; this module only maps requests onto the kernel.
//!
//! ```text
//!  syscall from a native process ──► syscall::dispatch ──► native::syscall
//!       Native::dispatch_with(Thread, regs)   the handler, through NativeKernel
//!       exit ──► Process::group_exit + user::request_exit: the process ends
//! ```
//!
//! - A process is native when it was started from a native program
//!   ([`Process::new_native`]) or a Linux process `execve`d one
//!   ([`NativeState::activate`]): it keeps its Linux state then (its place
//!   in the process tree, its parent's `wait4`, signals with their default
//!   actions), and its calls are native from the next one on.
//! - Handles are descriptors of the process's [`Files`](crate::fs::Files),
//!   the table Linux processes have: the same [`OpenFile`](crate::fs::OpenFile)
//!   objects (pipes, sockets, devices, files), so a native and a Linux
//!   process can share one. Sockets are `poly-unix`'s, in the one name
//!   space the Linux personality uses (`linux::net`), so a native server
//!   and Linux clients meet at the same path.
//! - Reads, writes, accepts and polls block as the Linux personality's do
//!   ([`ThreadBlocker`]), so the end of the process interrupts them.
//! - `spawn` starts any program [`program::prepare_child`] can (native,
//!   Linux, Windows), with the caller's handles as its 0, 1 and 2 and any
//!   extra ones; the child is kept in the caller's [`NativeState`] until
//!   `wait` reaps it, and a process handle ([`ProcessHandle`]) turns
//!   readable when it ends.
//! - A native process has no signals of its own; a CPU fault it does not
//!   survive kills it (`user::fault`), and its parent sees `Faulted`.

use crate::fs::poll::ThreadBlocker;
use crate::fs::{self, AT_FDCWD, AT_REMOVEDIR, FileType, OpenFile, OpenFlags, Whence as FsWhence};
use crate::process::{self, ExitStatus, Process, ThreadFlags, Unparked};
use crate::sync::{SetOnce, SpinLock};
use crate::user::{self, FileRef, MemoryError};
use crate::{program, thread, time};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_abi::{Outcome, SyscallDispatch, SyscallRegs};
use poly_personality_native::abi::{
    self, Clock, Error, FileInfo, Handle, InterfaceRecord, PeerInfo, PollEntry, Whence,
};
use poly_personality_native::kernel::{OpenHow, Prot, Signal, SocketKind, SocketName, Spawn};
use poly_personality_native::{Native, NativeKernel};
use poly_unix::{Ancillary, Credentials, SocketType, UnixFile, UnixSocket};
use poly_vfs::poll::{PollTarget, Timeout, WaitQueue};
use poly_vfs::{At, DeviceId, FileObject, PollEvents};

/// What the kernel keeps for every process about the native personality:
/// whether it is native, and the children it has not waited for yet.
#[derive(Debug)]
pub struct NativeState {
    active: AtomicBool,
    children: SpinLock<BTreeMap<u32, Arc<Process>>>,
}

impl NativeState {
    /// The state of a process that is not native (yet).
    pub fn new() -> NativeState {
        NativeState { active: AtomicBool::new(false), children: SpinLock::new(BTreeMap::new()) }
    }

    /// The process's calls are native from now on (a native program
    /// started, or `execve`d by a Linux process).
    pub fn activate(&self) {
        self.active.store(true, Ordering::SeqCst);
    }

    /// Whether the process's calls go to the native personality.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

impl Default for NativeState {
    fn default() -> NativeState {
        NativeState::new()
    }
}

/// A system call from a native process.
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    // Always a user thread here; without one there are no services.
    let Some(flags) = user::current_flags() else { return Native::dispatch(regs) };
    let mut thread = Thread { process, flags };
    Native::dispatch_with(&mut thread, regs)
}

/// The queue every process handle's waiters register on.
fn ended_queue() -> &'static Arc<WaitQueue> {
    static QUEUE: SetOnce<Arc<WaitQueue>> = SetOnce::new();
    if let Some(q) = QUEUE.get() {
        return q;
    }
    match QUEUE.set(Arc::new(WaitQueue::new())) {
        Ok(q) => q,
        Err(_) => QUEUE.get().expect("set by someone else"),
    }
}

/// A process ended (`Process::ended`): process handles look again.
pub fn process_ended() {
    ended_queue().notify(PollEvents::READABLE);
}

/// A process handle (`process_handle`): readable once its process has
/// ended. Reads and writes fail, as for any file without data.
#[derive(Debug)]
pub struct ProcessHandle {
    process: Arc<Process>,
    queue: Arc<WaitQueue>,
}

impl FileObject for ProcessHandle {
    fn read(&self, _buf: &mut [u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn write(&self, _buf: &[u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if self.process.has_ended() { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[poly-process]")
    }
}

/// The calling thread of a native process.
struct Thread<'a> {
    process: &'a Process,
    flags: Arc<ThreadFlags>,
}

fn vfs_error(e: poly_vfs::Errno) -> Error {
    Error::from_errno(e as u16)
}

fn memory_error(e: MemoryError) -> Error {
    Error::from_errno(e.linux_errno() as u16)
}

fn vm_prot(p: Prot) -> poly_vm::Prot {
    let bits = u64::from(p.read) | u64::from(p.write) << 1 | u64::from(p.exec) << 2;
    poly_vm::Prot::from_bits(bits).expect("three bits")
}

/// A descriptor for a handle (descriptors are `i32`; larger handles are
/// none).
fn fd(h: Handle) -> Result<i32, Error> {
    i32::try_from(h).map_err(|_| Error::BadHandle)
}

/// A handle for a descriptor.
fn handle(fd: i32) -> Result<Handle, Error> {
    Handle::try_from(fd).map_err(|_| Error::BadHandle)
}

fn exit_status(s: ExitStatus) -> abi::ExitStatus {
    match s {
        ExitStatus::Exited(code) => abi::ExitStatus::Exited(code),
        ExitStatus::Signaled { signal, .. } => abi::ExitStatus::Signaled(signal as u8),
        ExitStatus::Faulted(vector) => abi::ExitStatus::Faulted(vector),
    }
}

fn file_info(st: &poly_vfs::Stat) -> FileInfo {
    let kind = match st.mode & 0o170000 {
        0o040000 => abi::kind::DIRECTORY,
        0o120000 => abi::kind::SYMLINK,
        0o020000 => abi::kind::CHAR_DEVICE,
        0o060000 => abi::kind::BLOCK_DEVICE,
        0o010000 => abi::kind::FIFO,
        0o140000 => abi::kind::SOCKET,
        _ => abi::kind::FILE,
    };
    let device = if matches!(kind, abi::kind::CHAR_DEVICE | abi::kind::BLOCK_DEVICE) {
        u64::from(st.rdev.major) << 32 | u64::from(st.rdev.minor)
    } else {
        0
    };
    FileInfo { size: st.size, device, inode: st.ino, kind, mode: (st.mode & 0o7777) as u16, ..FileInfo::default() }
}

fn kind_of(t: FileType) -> u8 {
    match t {
        FileType::Regular => abi::kind::FILE,
        FileType::Directory => abi::kind::DIRECTORY,
        FileType::Symlink => abi::kind::SYMLINK,
        FileType::CharDevice => abi::kind::CHAR_DEVICE,
        FileType::BlockDevice => abi::kind::BLOCK_DEVICE,
        FileType::Fifo => abi::kind::FIFO,
        FileType::Socket => abi::kind::SOCKET,
    }
}

fn unix_type(k: SocketKind) -> SocketType {
    match k {
        SocketKind::Stream => SocketType::Stream,
        SocketKind::Datagram => SocketType::Datagram,
        SocketKind::SeqPacket => SocketType::SeqPacket,
    }
}

/// `SIGTERM` and `SIGKILL` (`asm-generic/signal.h`): what a Linux child
/// gets for `signal`'s terminate and kill, and what a native child's end
/// is reported as.
const SIGTERM: u32 = 15;
const SIGKILL: u32 = 9;

impl Thread<'_> {
    fn memory(&self) -> Result<Arc<user::Memory>, Error> {
        user::current_memory().ok_or(Error::Unsupported)
    }

    fn state(&self) -> &NativeState {
        &self.process.native
    }

    fn file(&self, h: Handle) -> Result<Arc<OpenFile>, Error> {
        self.process.files.file(fd(h)?).map_err(vfs_error)
    }

    fn blocker(&self) -> ThreadBlocker<'_> {
        ThreadBlocker::new(self.process, Arc::clone(&self.flags))
    }

    /// The child `pid`, if it is one.
    fn child(&self, pid: u32) -> Option<Arc<Process>> {
        self.state().children.lock().get(&pid).cloned()
    }

    /// Give the child `child` the caller's handle `from` as its `to`, or
    /// none there.
    fn give_handle(&self, child: &Process, to: i32, from: Option<Handle>) -> Result<(), Error> {
        let files = &child.files;
        let Some(from) = from else {
            let _ = files.close(to);
            return Ok(());
        };
        let file = self.file(from)?;
        let at = files.install(file, false).map_err(vfs_error)?;
        if at != to {
            files.dup2(at, to).map_err(vfs_error)?;
            let _ = files.close(at);
        }
        Ok(())
    }

    /// The process's credentials for its sockets: its pid and the ids its
    /// files are opened with.
    fn unix_creds(&self) -> Credentials {
        let cred = self.process.files.cred();
        Credentials { pid: self.process.pid as i32, uid: cred.uid, gid: cred.gid }
    }

    /// Open `object` on a new socket inode on the lowest free handle.
    fn install_socket(&self, object: Arc<dyn FileObject>, nonblock: bool) -> Result<Handle, Error> {
        let files = &self.process.files;
        let node = fs::vfs().anon_node(FileType::Socket, 0o777, &files.cred()).map_err(vfs_error)?;
        let flags = if nonblock { OpenFlags::RDWR | OpenFlags::NONBLOCK } else { OpenFlags::RDWR };
        let fd = files.install(Arc::new(OpenFile::on_object(node, object, flags)), false).map_err(vfs_error)?;
        handle(fd)
    }

    /// The Unix socket `h` is open on, and whether the handle is
    /// non-blocking.
    fn unix_socket(&self, h: Handle) -> Result<(Arc<UnixSocket>, bool), Error> {
        let file = self.file(h)?;
        let nonblock = file.flags().contains(OpenFlags::NONBLOCK);
        let unix = file.object_as::<UnixFile>().ok_or(Error::Unsupported)?;
        Ok((Arc::clone(unix.socket()), nonblock))
    }

    /// Run `attempt` until it stops failing with `EAGAIN`, waiting on
    /// `queue` for `interest` (or not at all when `nonblock`).
    fn wait_for<T>(
        &self,
        queue: &Arc<WaitQueue>,
        interest: PollEvents,
        nonblock: bool,
        attempt: impl FnMut() -> Result<T, poly_vfs::Errno>,
    ) -> Result<T, Error> {
        let wait = if nonblock { Timeout::Now } else { Timeout::Never };
        let mut blocker = self.blocker();
        poly_vfs::poll::wait_until_done(queue, interest, &mut blocker, wait, attempt).map_err(vfs_error)
    }
}

impl NativeKernel for Thread<'_> {
    fn pid(&self) -> u32 {
        self.process.pid
    }

    fn copy_from_user(&self, addr: u64, dst: &mut [u8]) -> Result<(), Error> {
        if dst.is_empty() {
            return Ok(());
        }
        user::copy_from_user(dst, addr).map_err(|_| Error::BadAddress)
    }

    fn copy_to_user(&mut self, addr: u64, src: &[u8]) -> Result<(), Error> {
        if src.is_empty() {
            return Ok(());
        }
        user::copy_to_user(addr, src).map_err(|_| Error::BadAddress)
    }

    fn exit(&mut self, code: u8) {
        let status = self.process.group_exit(ExitStatus::Exited(code));
        let _ = user::request_exit(status);
    }

    fn write(&mut self, handle: Handle, buf: &[u8]) -> Result<usize, Error> {
        let file = self.file(handle)?;
        self.process.files.before_write(&file, buf.len()).map_err(vfs_error)?;
        let mut blocker = self.blocker();
        file.write_wait(buf, &mut blocker).map_err(vfs_error)
    }

    fn read(&mut self, handle: Handle, buf: &mut [u8]) -> Result<usize, Error> {
        let file = self.file(handle)?;
        let mut blocker = self.blocker();
        file.read_wait(buf, &mut blocker).map_err(vfs_error)
    }

    fn open(&mut self, path: &[u8], how: &OpenHow) -> Result<Handle, Error> {
        let mut flags = match (how.read, how.write) {
            (true, true) => OpenFlags::RDWR,
            (false, true) => OpenFlags::WRONLY,
            _ => OpenFlags::RDONLY,
        };
        // A native handle never becomes a controlling terminal.
        flags = flags | OpenFlags::NOCTTY;
        for (set, flag) in [
            (how.create, OpenFlags::CREAT),
            (how.exclusive, OpenFlags::EXCL),
            (how.truncate, OpenFlags::TRUNC),
            (how.append, OpenFlags::APPEND),
            (how.nonblock, OpenFlags::NONBLOCK),
            (how.directory, OpenFlags::DIRECTORY),
        ] {
            if set {
                flags = flags | flag;
            }
        }
        let fd = self.process.files.open(path, flags, how.mode).map_err(vfs_error)?;
        handle(fd)
    }

    fn close(&mut self, handle: Handle) -> Result<(), Error> {
        self.process.files.close(fd(handle)?).map_err(vfs_error)?;
        // A socket only descriptors in flight held may go now (ADR 0048).
        crate::linux::net::after_close();
        Ok(())
    }

    fn dup(&mut self, h: Handle) -> Result<Handle, Error> {
        handle(self.process.files.dup(fd(h)?).map_err(vfs_error)?)
    }

    fn seek(&mut self, handle: Handle, offset: i64, whence: Whence) -> Result<u64, Error> {
        let whence = match whence {
            Whence::Start => FsWhence::Set,
            Whence::Current => FsWhence::Cur,
            Whence::End => FsWhence::End,
        };
        self.process.files.lseek(fd(handle)?, offset, whence).map_err(vfs_error)
    }

    fn map(&mut self, hint: u64, len: u64, prot: Prot) -> Result<u64, Error> {
        let placement = poly_vm::Placement::Hint(hint);
        self.memory()?.map_anonymous(placement, len, vm_prot(prot)).map_err(memory_error)
    }

    fn unmap(&mut self, addr: u64, len: u64) -> Result<(), Error> {
        self.memory()?.unmap(addr, len).map_err(memory_error)
    }

    fn protect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Error> {
        self.memory()?.protect(addr, len, vm_prot(prot)).map_err(memory_error)
    }

    fn clock(&self, clock: Clock) -> u64 {
        match clock {
            Clock::Monotonic => time::now_ns(),
            Clock::Realtime => time::realtime_ns(),
        }
    }

    fn sleep(&mut self, ns: u64) -> Result<(), Error> {
        let deadline = time::now_ns().saturating_add(ns);
        loop {
            match process::park(self.process, &self.flags, Some(deadline)) {
                Unparked::TimedOut => return Ok(()),
                Unparked::Interrupted => return Err(Error::Interrupted),
                // A stray unpark: sleep on until the deadline.
                Unparked::Woken if time::now_ns() >= deadline => return Ok(()),
                Unparked::Woken => {}
            }
        }
    }

    fn yield_now(&mut self) {
        thread::yield_now();
    }

    fn spawn(&mut self, spawn: &Spawn) -> Result<u32, Error> {
        let argv: Vec<&[u8]> = spawn.args.iter().map(Vec::as_slice).collect();
        let envp: Vec<&[u8]> = spawn.env.iter().map(Vec::as_slice).collect();
        let prepared = program::prepare_child(&spawn.path, &argv, &envp, self.process).map_err(|e| match e {
            program::ExecPathError::Open(e) => vfs_error(e),
            program::ExecPathError::Unsupported(_) => Error::NotExecutable,
            program::ExecPathError::Exec(e) => Error::from_errno(e.linux_errno() as u16),
        })?;
        let child = Arc::clone(prepared.process());
        for (to, from) in (0..).zip(spawn.stdio) {
            self.give_handle(&child, to, from)?;
        }
        for &(from, to) in &spawn.handles {
            self.give_handle(&child, fd(to)?, Some(from))?;
        }
        let child = prepared.start().map_err(|_| Error::NotExecutable)?;
        let pid = child.pid;
        self.state().children.lock().insert(pid, child);
        Ok(pid)
    }

    fn wait(&mut self, pid: u32, no_hang: bool) -> Result<abi::ExitStatus, Error> {
        let child = self.child(pid).ok_or(Error::NoChild)?;
        let me = self.process;
        if no_hang && !child.has_ended() {
            return Err(Error::WouldBlock);
        }
        process::wait_event(|| child.has_ended() || me.is_exiting());
        if !child.has_ended() {
            return Err(Error::Interrupted);
        }
        self.state().children.lock().remove(&pid);
        child.status().map(exit_status).ok_or(Error::NoChild)
    }

    // ---- ABI version 2 ------------------------------------------------------

    fn control(&mut self, handle: Handle, request: u32, arg: u64) -> Result<u64, Error> {
        self.process.files.ioctl(fd(handle)?, request, arg).map_err(vfs_error)
    }

    fn poll(&mut self, entries: &mut [PollEntry], timeout_ns: Option<u64>) -> Result<usize, Error> {
        let mut targets: Vec<poly_vfs::poll::PollEntry> = entries
            .iter()
            .map(|e| {
                let target = if e.handle == abi::NO_HANDLE {
                    PollTarget::Ignore
                } else {
                    fd(e.handle)
                        .ok()
                        .and_then(|fd| self.process.files.file(fd).ok())
                        .map_or(PollTarget::Closed, PollTarget::File)
                };
                poly_vfs::poll::PollEntry::new(target, PollEvents::from_bits(u32::from(e.events)))
            })
            .collect();
        let timeout = match timeout_ns {
            None => Timeout::Never,
            Some(0) => Timeout::Now,
            Some(ns) => Timeout::At(time::now_ns().saturating_add(ns)),
        };
        let mut blocker = self.blocker();
        let ready = poly_vfs::poll::poll(&mut targets, &mut blocker, timeout).map_err(vfs_error)?;
        for (e, t) in entries.iter_mut().zip(&targets) {
            e.revents = t.revents.bits() as u16;
        }
        Ok(ready)
    }

    fn map_handle(&mut self, handle: Handle, offset: u64, len: u64, prot: Prot, shared: bool) -> Result<u64, Error> {
        let file = self.file(handle)?;
        let memory = self.memory()?;
        if !file.readable() {
            return Err(Error::PermissionDenied);
        }
        if shared && prot.write && !file.writable() {
            return Err(Error::PermissionDenied);
        }
        let vprot = vm_prot(prot);
        let placement = poly_vm::Placement::Hint(0);
        // A DRM dumb buffer or a dma-buf: its frames (ADR 0108).
        if let Some(found) = crate::drm::mapping(&file, offset, len) {
            let (source, start) = found.map_err(vfs_error)?;
            let req = poly_vm::MapRequest::file(len, vprot, FileRef::new(source), start, shared);
            return memory.map(req.at(placement)).map_err(memory_error);
        }
        // The frame buffer device's shadow memory (ADR 0107).
        if let Some(source) = crate::fbdev::page_source(&file) {
            let req = poly_vm::MapRequest::file(len, vprot, FileRef::new(source), offset, shared);
            return memory.map(req.at(placement)).map_err(memory_error);
        }
        // Pipes, sockets and the like have nothing to map.
        if file.object().is_some() || file.kind() != FileType::Regular {
            return Err(Error::Unsupported);
        }
        // A memory object's write seals refuse writable shared mappings.
        let writable_map = match file.memfd() {
            Some(m) => m.map(shared, prot.write).map_err(vfs_error)?,
            None => None,
        };
        let source = fs::cache::FileSource::of(&file).holding(writable_map);
        let req = poly_vm::MapRequest::file(len, vprot, FileRef::new(Arc::new(source)), offset, shared);
        memory.map(req.at(placement)).map_err(memory_error)
    }

    fn memory_object(&mut self, size: u64) -> Result<Handle, Error> {
        let files = &self.process.files;
        let memfd = poly_vfs::memfd::Memfd::new(b"poly-memory", 0);
        let file = fs::vfs().memfd_file(&fs::memfs::memfd_mount(), &files.cred(), 0o600, memfd).map_err(vfs_error)?;
        let fd = files.install(file, false).map_err(vfs_error)?;
        if size > 0
            && let Err(e) = files.ftruncate(fd, size)
        {
            let _ = files.close(fd);
            return Err(vfs_error(e));
        }
        handle(fd)
    }

    fn seal(&mut self, handle: Handle, seals: u32) -> Result<(), Error> {
        let file = self.file(handle)?;
        let memfd = file.memfd().ok_or(Error::Unsupported)?;
        if !file.writable() {
            return Err(Error::PermissionDenied);
        }
        memfd.add_seals(seals).map_err(vfs_error)
    }

    fn set_nonblocking(&mut self, handle: Handle, on: bool) -> Result<(), Error> {
        let files = &self.process.files;
        let fd = fd(handle)?;
        let flags = files.status_flags(fd).map_err(vfs_error)?;
        let flags = if on { flags | OpenFlags::NONBLOCK } else { flags.without(OpenFlags::NONBLOCK) };
        files.set_status_flags(fd, flags).map_err(vfs_error)
    }

    fn handle_info(&mut self, handle: Handle) -> Result<FileInfo, Error> {
        self.process.files.fstat(fd(handle)?).map(|st| file_info(&st)).map_err(vfs_error)
    }

    fn stat(&mut self, path: &[u8]) -> Result<FileInfo, Error> {
        self.process.files.stat(path).map(|st| file_info(&st)).map_err(vfs_error)
    }

    fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Error> {
        let files = &self.process.files;
        files.renameat(AT_FDCWD, from, AT_FDCWD, Ok(to), poly_vfs::RenameFlags::default()).map_err(vfs_error)
    }

    fn remove(&mut self, path: &[u8], directory: bool) -> Result<(), Error> {
        let flags = if directory { AT_REMOVEDIR } else { 0 };
        self.process.files.unlinkat(AT_FDCWD, path, flags).map_err(vfs_error)
    }

    fn make_dir(&mut self, path: &[u8], mode: u16) -> Result<(), Error> {
        self.process.files.mkdirat(AT_FDCWD, path, mode).map_err(vfs_error)
    }

    fn set_mode(&mut self, path: &[u8], mode: u16) -> Result<(), Error> {
        self.process.files.fchmodat(AT_FDCWD, path, mode).map_err(vfs_error)
    }

    fn read_dir(&mut self, handle: Handle, emit: &mut dyn FnMut(u8, &[u8]) -> bool) -> Result<usize, Error> {
        let mut take = |e: &poly_vfs::DirEntry<'_>| emit(kind_of(e.kind), e.name);
        self.process.files.getdents(fd(handle)?, &mut take).map_err(vfs_error)
    }

    fn sync(&mut self) {
        // Always succeeds; failures are logged by the cache.
        let _ = fs::disk::sync_all();
    }

    fn pipe(&mut self) -> Result<[Handle; 2], Error> {
        let (r, w) = poly_vfs::pipe::Pipe::pair(false);
        let files = &self.process.files;
        let node = fs::vfs().anon_node(FileType::Fifo, 0o600, &files.cred()).map_err(vfs_error)?;
        let read = Arc::new(OpenFile::on_object(node.clone(), Arc::new(r), OpenFlags::RDONLY));
        let write = Arc::new(OpenFile::on_object(node, Arc::new(w), OpenFlags::WRONLY));
        let rfd = files.install(read, false).map_err(vfs_error)?;
        match files.install(write, false) {
            Ok(wfd) => Ok([handle(rfd)?, handle(wfd)?]),
            Err(e) => {
                let _ = files.close(rfd);
                Err(vfs_error(e))
            }
        }
    }

    fn make_fifo(&mut self, path: &[u8], mode: u16) -> Result<(), Error> {
        let files = &self.process.files;
        files.mknodat(AT_FDCWD, path, FileType::Fifo, mode, DeviceId::new(0, 0), false).map_err(vfs_error)
    }

    fn socket(&mut self, kind: SocketKind, nonblock: bool) -> Result<Handle, Error> {
        let sock = UnixSocket::new(crate::linux::net::namespace(), unix_type(kind), self.unix_creds());
        self.install_socket(Arc::new(UnixFile::new(sock)), nonblock)
    }

    fn socket_pair(&mut self, kind: SocketKind, nonblock: bool) -> Result<[Handle; 2], Error> {
        let (a, b) = UnixSocket::pair(crate::linux::net::namespace(), unix_type(kind), self.unix_creds());
        let ha = self.install_socket(Arc::new(UnixFile::new(a)), nonblock)?;
        match self.install_socket(Arc::new(UnixFile::new(b)), nonblock) {
            Ok(hb) => Ok([ha, hb]),
            Err(e) => {
                let _ = self.process.files.close(fd(ha)?);
                Err(e)
            }
        }
    }

    fn bind(&mut self, handle: Handle, name: &SocketName) -> Result<(), Error> {
        let (sock, _) = self.unix_socket(handle)?;
        match name {
            SocketName::Abstract(n) => sock.bind_abstract(n).map_err(vfs_error),
            SocketName::Path(p) => {
                let files = &self.process.files;
                sock.bind_path(fs::vfs(), &files.context(), &files.cred(), At::Cwd, p).map_err(vfs_error)
            }
        }
    }

    fn listen(&mut self, handle: Handle, backlog: u32) -> Result<(), Error> {
        let (sock, _) = self.unix_socket(handle)?;
        sock.listen(backlog as usize, self.unix_creds()).map_err(vfs_error)
    }

    fn accept(&mut self, handle: Handle, nonblock: bool) -> Result<Handle, Error> {
        let (sock, blocking) = self.unix_socket(handle)?;
        let conn = self.wait_for(sock.wait_queue(), PollEvents::READABLE, blocking, || sock.accept())?;
        self.install_socket(Arc::new(UnixFile::new(conn)), nonblock)
    }

    fn connect(&mut self, handle: Handle, name: &SocketName) -> Result<(), Error> {
        let (sock, nonblock) = self.unix_socket(handle)?;
        let ns = crate::linux::net::namespace();
        let target = match name {
            SocketName::Path(p) => {
                let files = &self.process.files;
                ns.lookup_path(fs::vfs(), &files.context(), &files.cred(), At::Cwd, p).map_err(vfs_error)?
            }
            SocketName::Abstract(n) => ns.lookup_abstract(n).ok_or(Error::ConnectionRefused)?,
        };
        let creds = self.unix_creds();
        self.wait_for(sock.wait_queue(), PollEvents::WRITABLE, nonblock, || sock.connect(&target, creds))
    }

    fn send(&mut self, handle: Handle, data: &[u8], handles: &[Handle]) -> Result<usize, Error> {
        let (sock, nonblock) = self.unix_socket(handle)?;
        let mut rights = Vec::with_capacity(handles.len());
        for &h in handles {
            rights.push(self.file(h)?);
        }
        let creds = self.unix_creds();
        let mut anc = Some(Ancillary { rights, creds: None });
        let stream = sock.kind() == SocketType::Stream;
        let mut done = 0;
        loop {
            let r = self.wait_for(sock.wait_queue(), PollEvents::WRITABLE, nonblock, || {
                // The handles go with the first bytes; an attempt that fails
                // keeps them for the next.
                let a = anc.take().unwrap_or_default();
                let held = Ancillary { rights: a.rights.clone(), creds: a.creds };
                let r = sock.send(&data[done..], None, held, creds);
                if r.is_err() {
                    anc = Some(a);
                }
                r
            });
            match r {
                Ok(n) => {
                    done += n;
                    if !stream || done >= data.len() || nonblock || n == 0 {
                        return Ok(done);
                    }
                }
                Err(_) if done > 0 => return Ok(done),
                Err(e) => return Err(e),
            }
        }
    }

    fn recv(&mut self, handle: Handle, buf: &mut [u8], max_handles: usize) -> Result<(usize, Vec<Handle>), Error> {
        let (sock, nonblock) = self.unix_socket(handle)?;
        let got = self.wait_for(sock.wait_queue(), PollEvents::READABLE, nonblock, || sock.recv(buf, false))?;
        let mut handles = Vec::new();
        for f in got.rights.into_iter().take(max_handles) {
            if let Ok(fd) = self.process.files.install(f, false) {
                handles.push(fd as Handle);
            }
        }
        // Rights past `max_handles` were dropped above: collect them.
        crate::linux::net::after_close();
        Ok((got.len, handles))
    }

    fn socket_peer(&mut self, handle: Handle) -> Result<PeerInfo, Error> {
        let (sock, _) = self.unix_socket(handle)?;
        let c = sock.peer_credentials().ok_or(Error::NotFound)?;
        Ok(PeerInfo { pid: c.pid as u32, uid: c.uid, gid: c.gid })
    }

    fn process_handle(&mut self, pid: u32) -> Result<Handle, Error> {
        let child = self.child(pid).ok_or(Error::NoChild)?;
        let object = Arc::new(ProcessHandle { process: child, queue: Arc::clone(ended_queue()) });
        let files = &self.process.files;
        let node = fs::vfs().anon_node(FileType::Regular, 0o700, &files.cred()).map_err(vfs_error)?;
        let file = Arc::new(OpenFile::on_object(node, object, OpenFlags::RDONLY));
        handle(files.install(file, false).map_err(vfs_error)?)
    }

    fn signal(&mut self, pid: u32, what: Signal) -> Result<(), Error> {
        let target = match what {
            Signal::Probe => return process::find(pid).map(|_| ()).ok_or(Error::NotFound),
            _ => self.child(pid).ok_or(Error::NotFound)?,
        };
        if target.has_ended() {
            return Ok(());
        }
        let signal = if what == Signal::Kill { SIGKILL } else { SIGTERM };
        if target.linux.is_some() {
            crate::tty::signal::send(pid, signal, None);
        } else {
            target.group_exit(ExitStatus::Signaled { signal, core: false });
        }
        Ok(())
    }

    fn net_interfaces(&mut self) -> Vec<(u32, Vec<u8>)> {
        let ifs = crate::linux::net::interfaces();
        ifs.links
            .iter()
            .map(|l| {
                let mut flags = 0;
                if l.up {
                    flags |= InterfaceRecord::UP;
                }
                if l.carrier {
                    flags |= InterfaceRecord::RUNNING;
                }
                if l.loopback {
                    flags |= InterfaceRecord::LOOPBACK;
                }
                if ifs.addrs.iter().any(|a| a.index == l.index) {
                    flags |= InterfaceRecord::ADDRESSED;
                }
                (flags, l.name.clone())
            })
            .collect()
    }

    fn truncate(&mut self, handle: Handle, len: u64) -> Result<(), Error> {
        self.process.files.ftruncate(fd(handle)?, len).map_err(vfs_error)
    }
}
