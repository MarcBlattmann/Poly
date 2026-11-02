//! The native personality in the kernel (ADR 0125): the services
//! `poly_personality_native::NativeKernel` asks for, on the kernel's
//! shared objects. The checking and decoding of every call is in the
//! personality crate; this file only maps requests onto the kernel.
//!
//! ```text
//!  syscall from a native process ──► syscall::dispatch ──► native::syscall
//!       Native::dispatch_with(Thread, regs)   the handler, through NativeKernel
//!       exit ──► Process::group_exit + user::request_exit: the process ends
//! ```
//!
//! - Handles are descriptors of the process's [`Files`](crate::fs::Files),
//!   the table Linux processes have: the same [`OpenFile`](crate::fs::OpenFile)
//!   objects (pipes, sockets, devices, files), so a native and a Linux
//!   process can share one.
//! - Reads and writes block as the Linux personality's do
//!   ([`ThreadBlocker`]), so the end of the process interrupts them.
//! - `spawn` starts any program [`program::prepare`] can (native, Linux,
//!   Windows), with the caller's handles as its 0, 1 and 2; the child is
//!   kept in the caller's [`NativeState`] until `wait` reaps it.
//! - A native process has no signals; a CPU fault it does not survive
//!   kills it (`user::fault`), and its parent sees `Faulted`.

use crate::fs::poll::ThreadBlocker;
use crate::fs::{OpenFlags, Whence as FsWhence};
use crate::process::{self, ExitStatus, Process, ThreadFlags, Unparked};
use crate::sync::SpinLock;
use crate::user::{self, MemoryError};
use crate::{program, thread, time};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_abi::SyscallDispatch;
use poly_abi::{Outcome, SyscallRegs};
use poly_personality_native::abi::{self, Clock, Error, Handle, Whence};
use poly_personality_native::kernel::{OpenHow, Prot, Spawn};
use poly_personality_native::{Native, NativeKernel};

/// What the kernel keeps for a native process: the children it has not
/// waited for yet.
#[derive(Debug)]
pub struct NativeState {
    children: SpinLock<BTreeMap<u32, Arc<Process>>>,
}

impl Default for NativeState {
    fn default() -> NativeState {
        NativeState { children: SpinLock::new(BTreeMap::new()) }
    }
}

/// A system call from a native process.
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    // Always a user thread here; without one there are no services.
    let Some(flags) = user::current_flags() else { return Native::dispatch(regs) };
    let mut thread = Thread { process, flags };
    Native::dispatch_with(&mut thread, regs)
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

fn exit_status(s: ExitStatus) -> abi::ExitStatus {
    match s {
        ExitStatus::Exited(code) => abi::ExitStatus::Exited(code),
        ExitStatus::Signaled { signal, .. } => abi::ExitStatus::Signaled(signal as u8),
        ExitStatus::Faulted(vector) => abi::ExitStatus::Faulted(vector),
    }
}

impl Thread<'_> {
    fn memory(&self) -> Result<Arc<user::Memory>, Error> {
        user::current_memory().ok_or(Error::Unsupported)
    }

    fn state(&self) -> Result<&NativeState, Error> {
        self.process.native.as_ref().ok_or(Error::Unsupported)
    }

    /// Give the child `child` the caller's handle `from` as its `to`, or
    /// none there.
    fn give_handle(&self, child: &Process, to: i32, from: Option<Handle>) -> Result<(), Error> {
        let files = &child.files;
        let Some(from) = from else {
            let _ = files.close(to);
            return Ok(());
        };
        let file = self.process.files.file(fd(from)?).map_err(vfs_error)?;
        let at = files.install(file, false).map_err(vfs_error)?;
        if at != to {
            files.dup2(at, to).map_err(vfs_error)?;
            let _ = files.close(at);
        }
        Ok(())
    }
}

impl NativeKernel for Thread<'_> {
    fn pid(&self) -> u32 {
        self.process.pid
    }

    fn copy_from_user(&self, addr: u64, dst: &mut [u8]) -> Result<(), Error> {
        user::copy_from_user(dst, addr).map_err(|_| Error::BadAddress)
    }

    fn copy_to_user(&mut self, addr: u64, src: &[u8]) -> Result<(), Error> {
        user::copy_to_user(addr, src).map_err(|_| Error::BadAddress)
    }

    fn exit(&mut self, code: u8) {
        let status = self.process.group_exit(ExitStatus::Exited(code));
        let _ = user::request_exit(status);
    }

    fn write(&mut self, handle: Handle, buf: &[u8]) -> Result<usize, Error> {
        let files = &self.process.files;
        let file = files.file(fd(handle)?).map_err(vfs_error)?;
        files.before_write(&file, buf.len()).map_err(vfs_error)?;
        let mut blocker = ThreadBlocker::new(self.process, Arc::clone(&self.flags));
        file.write_wait(buf, &mut blocker).map_err(vfs_error)
    }

    fn read(&mut self, handle: Handle, buf: &mut [u8]) -> Result<usize, Error> {
        let file = self.process.files.file(fd(handle)?).map_err(vfs_error)?;
        let mut blocker = ThreadBlocker::new(self.process, Arc::clone(&self.flags));
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
        Handle::try_from(fd).map_err(|_| Error::BadHandle)
    }

    fn close(&mut self, handle: Handle) -> Result<(), Error> {
        self.process.files.close(fd(handle)?).map_err(vfs_error)?;
        // A socket only descriptors in flight held may go now (ADR 0048).
        crate::linux::net::after_close();
        Ok(())
    }

    fn dup(&mut self, handle: Handle) -> Result<Handle, Error> {
        let fd = self.process.files.dup(fd(handle)?).map_err(vfs_error)?;
        Handle::try_from(fd).map_err(|_| Error::BadHandle)
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
        let state = self.state()?;
        let argv: Vec<&[u8]> = spawn.args.iter().map(Vec::as_slice).collect();
        let envp: Vec<&[u8]> = spawn.env.iter().map(Vec::as_slice).collect();
        let prepared = program::prepare(&spawn.path, &argv, &envp, false).map_err(|e| match e {
            program::ExecPathError::Open(e) => vfs_error(e),
            program::ExecPathError::Unsupported(_) => Error::NotExecutable,
            program::ExecPathError::Exec(e) => Error::from_errno(e.linux_errno() as u16),
        })?;
        let child = Arc::clone(prepared.process());
        for (to, from) in (0..).zip(spawn.stdio) {
            self.give_handle(&child, to, from)?;
        }
        let child = prepared.start().map_err(|_| Error::NotExecutable)?;
        let pid = child.pid;
        state.children.lock().insert(pid, child);
        Ok(pid)
    }

    fn wait(&mut self, pid: u32) -> Result<abi::ExitStatus, Error> {
        let state = self.state()?;
        let child = state.children.lock().get(&pid).cloned().ok_or(Error::NoChild)?;
        let me = self.process;
        process::wait_event(|| child.has_ended() || me.is_exiting());
        if !child.has_ended() {
            return Err(Error::Interrupted);
        }
        state.children.lock().remove(&pid);
        child.status().map(exit_status).ok_or(Error::NoChild)
    }
}
