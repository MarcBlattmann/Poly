//! The kernel's services for the Linux personality: [`LinuxKernel`] for the
//! running thread of a Linux process.
//!
//! | Trait methods | Kernel |
//! |---|---|
//! | ids, credentials | each thread's `LinuxThread::creds` in the [`LinuxState`](super::LinuxState) (`linux::cred`, ADR 0045) |
//! | `copy_*_user` | `user::copy_*_user` (demand paging, `EFAULT`) |
//! | `write`, `read`, `close`, `fstat`, `ioctl`, file paths | the process's [`crate::fs::Files`] |
//! | `mmap`, `munmap`, `mprotect`, `brk` | the thread's [`Memory`] (`user::current_memory`) |
//! | FS / GS base | `user::set_fs_base` (takes effect on the return to ring 3) |
//! | `with_process`, `process_ids` | the process table and each process's [`LinuxState`](super::LinuxState) |
//! | `block` / `unblock`, `notify_signal` | [`process::park`], [`process::unpark`], [`process::notify`] |
//! | `stop_process`, `continue_process`, `kill_process`, exits | [`Process::stop`], [`Process::cont`], [`Process::group_exit`], `user::request_exit` |
//! | `user_context`, `set_user_context` | the thread's trap frame (`user::registers`, `user::set_registers`) |
//! | FPU state | the thread's save area (`user::fpu_image`, `user::load_fpu`) |
//! | futexes | one global table under a spinlock ([`super::FUTEXES`]) |
//! | clocks, random bytes | `time::now_ns`, `time::realtime_ns` (the CMOS RTC read once, settable), `crate::random` (ChaCha20) |
//! | CPU times, timers, time zone | [`crate::cputime`], [`super::timers`], [`crate::vdso`] (ADR 0044) |
//! | `fs()`: paths, descriptors, `statfs`, `sync`; `system_info` | `files.rs` over `fs::Files`; `fs::proc` (ADR 0043) |
//! | `clone_task`, `execve`, `with_tree`, `wait_tree`, `ppid`, group and session ids | `super::task`: the process tree, `user::spawn_child`, `Memory::fork`, `exec::load` (ADR 0041) |
//! | `file_io`: pipes, `poll`, `epoll`, event files; blocking `read` / `write` | `fileio.rs` (ADR 0042) |
//!
//! Nothing here holds a lock while it touches user memory, except the
//! futex calls the personality makes with the table locked
//! (`futex_load`, `futex_cmpxchg`): those run with interrupts off and fail
//! with `EFAULT` on a page that is not present, which the personality
//! prevents by reading the word first, unlocked.
//!
//! Not done yet, each documented where it is: core dumps, shared mappings
//! of files the page cache does not hold (tmpfs: `ENODEV`), `MAP_32BIT` placement (refused with `ENOMEM` when the address
//! is not low), `PROT_GROWSDOWN` (applied to the given range only).

use super::LinuxState;
use crate::fs::cache::FileSource;
use crate::process::{self, ExitStatus, Process, ThreadFlags, Unparked};
use crate::testrun;
use crate::user::{self, FileRef, Memory, MemoryError, UserRegs};
use crate::{cputime, kprintln, time};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::OnceCell;
use core::sync::atomic::Ordering;
use poly_personality_linux::abi::{FileStat, Timespec, makedev};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::futex::FutexTable;
use poly_personality_linux::kernel::{
    Backing, Clock, CpuTimes, CpuTimesOf, Credentials, Deadline, FpuState, LinuxKernel, MapRequest, Placement,
    ProcessInfo, ProcessSelector, Prot, RandomMode, Target, UserContext, UserPtr, Wakeup,
};
use poly_personality_linux::signal::LinuxProcess;
use poly_personality_linux::time::TimerWakeup;
use poly_vfs::FileType;
use poly_vm::Access;

mod fileio;

/// The running thread of a Linux process, for one system call or one
/// return to user mode.
pub struct Thread<'a> {
    pub(super) process: &'a Process,
    pub(super) linux: &'a LinuxState,
    pub(super) tid: u32,
    pub(super) flags: Arc<ThreadFlags>,
    /// During a system call: `rax` as the thread will see it (the call
    /// number until the dispatcher has written the result).
    rax: Option<u64>,
    /// `trapno`, `err`, `cr2` of the fault being turned into a signal.
    fault: Option<(u64, u64, u64)>,
    /// How the thread (or its process) is to end.
    exit: Option<ExitStatus>,
    pub(super) replaced: bool,
    /// The FPU image, saved at the first request.
    fpu: OnceCell<(Vec<u8>, Option<u64>)>,
}

impl<'a> Thread<'a> {
    /// The running thread, which must belong to `process`, a Linux process.
    /// `None` if it does not (or `process` has no Linux state).
    pub fn current(process: &'a Process) -> Option<Thread<'a>> {
        let linux = process.linux.as_ref()?;
        let tid = user::current_tid()?;
        let flags = user::current_flags()?;
        Some(Thread {
            process,
            linux,
            tid,
            flags,
            rax: None,
            fault: None,
            exit: None,
            replaced: false,
            fpu: OnceCell::new(),
        })
    }

    /// A system call `number` starts.
    pub fn enter_syscall(&mut self, number: u64) {
        self.rax = Some(number);
    }

    /// The dispatcher wrote the result.
    pub fn set_result(&mut self, rax: u64) {
        self.rax = Some(rax);
    }

    /// The registers of the fault being raised as a signal.
    pub fn set_fault(&mut self, trapno: u64, err: u64, cr2: u64) {
        self.fault = Some((trapno, err, cr2));
    }

    /// How the thread is to end, if a call asked for it.
    pub fn take_exit(&mut self) -> Option<ExitStatus> {
        self.exit.take()
    }

    /// Whether `set_user_context` replaced the registers.
    pub fn registers_replaced(&self) -> bool {
        self.replaced
    }

    /// The thread's process.
    pub(super) fn process(&self) -> &'a Process {
        self.process
    }

    fn memory(&self) -> Result<Arc<Memory>, Errno> {
        user::current_memory().ok_or(Errno::ENOMEM)
    }

    /// The process `target` names (`ESRCH` if none).
    fn target(&self, target: Target) -> Result<TargetProcess<'a>, Errno> {
        match target {
            Target::Current => Ok(TargetProcess::Own(self.process)),
            Target::Process(pid) if pid > 0 => process::find(pid as u32)
                .or_else(|| super::task::zombie(pid as u32))
                .map(TargetProcess::Other)
                .ok_or(Errno::ESRCH),
            Target::Thread(tid) if tid > 0 => {
                process::find_thread(tid as u32).map(|(p, _)| TargetProcess::Other(p)).ok_or(Errno::ESRCH)
            }
            _ => Err(Errno::ESRCH),
        }
    }

    /// The flags of thread `tid`, of any process.
    fn flags_of(&self, tid: i32) -> Option<Arc<ThreadFlags>> {
        if tid == self.tid as i32 {
            return Some(Arc::clone(&self.flags));
        }
        process::find_thread(u32::try_from(tid).ok()?).map(|(_, f)| f)
    }

    /// The kernel thread of Linux thread `tid`, for sched_setaffinity(2) and
    /// sched_getaffinity(2). Threads of another process need the caller to
    /// have effective user id 0 (standing in for `CAP_SYS_NICE`, as the
    /// caller's other ids are not compared yet).
    fn sched_target(&self, tid: i32) -> Result<crate::thread::ThreadId, Errno> {
        if tid == self.tid as i32 {
            return Ok(crate::thread::current());
        }
        let (process, flags) =
            process::find_thread(u32::try_from(tid).map_err(|_| Errno::ESRCH)?).ok_or(Errno::ESRCH)?;
        if process.pid != self.process.pid && self.credentials().euid != 0 {
            return Err(Errno::EPERM);
        }
        match flags.thread.load(Ordering::SeqCst) {
            0 => Err(Errno::ESRCH),
            bits => Ok(crate::thread::ThreadId::from_bits(bits - 1)),
        }
    }
}

enum TargetProcess<'a> {
    Own(&'a Process),
    Other(Arc<Process>),
}

impl TargetProcess<'_> {
    fn get(&self) -> &Process {
        match self {
            TargetProcess::Own(p) => p,
            TargetProcess::Other(p) => p,
        }
    }
}

/// The Linux errno for a VFS error (the VFS uses Linux's numbers).
pub fn vfs_errno(e: poly_vfs::Errno) -> Errno {
    Errno::new(e as u16).unwrap_or(Errno::EIO)
}

fn memory_errno(e: MemoryError) -> Errno {
    Errno::new(e.linux_errno() as u16).unwrap_or(Errno::ENOMEM)
}

fn timespec(t: poly_vfs::Timespec) -> Timespec {
    Timespec { sec: t.sec, nsec: i64::from(t.nsec) }
}

/// `struct stat` for a VFS stat.
pub fn file_stat(st: &poly_vfs::Stat) -> FileStat {
    FileStat {
        dev: makedev(st.dev.major, st.dev.minor),
        ino: st.ino,
        nlink: u64::from(st.nlink),
        mode: st.mode,
        uid: st.uid,
        gid: st.gid,
        rdev: makedev(st.rdev.major, st.rdev.minor),
        size: st.size as i64,
        blksize: i64::from(st.blksize),
        blocks: st.blocks as i64,
        atime: timespec(st.atime),
        mtime: timespec(st.mtime),
        ctime: timespec(st.ctime),
    }
}

fn info_of(p: &Process) -> ProcessInfo {
    let pid = p.pid as i32;
    // Groups and sessions from the process tree; the ids are filled in
    // under the process's lock (`with_process`).
    let (pgid, sid) = super::task::group_and_session(p.pid);
    ProcessInfo { pid, pgid, sid, zombie: p.has_ended(), ..ProcessInfo::default() }
}

/// `info` with the ids of thread `tid` of `state`, or of the process
/// (`LinuxProcess::process_creds`) when `tid` is not one of its threads.
fn with_ids(mut info: ProcessInfo, state: &LinuxProcess, tid: i32) -> ProcessInfo {
    let creds = if state.has_thread(tid) { Some(&state.thread(tid).creds) } else { state.process_creds(info.pid) };
    if let Some(c) = creds {
        (info.uid, info.euid, info.suid) = (c.uid, c.euid, c.suid);
        (info.gid, info.egid, info.sgid) = (c.gid, c.egid, c.sgid);
    }
    info
}

/// The region protection for validated `PROT_*` bits (`PROT_SEM` and the
/// grows flags dropped).
fn vm_prot(prot: Prot) -> poly_vm::Prot {
    poly_vm::Prot::from_bits(u64::from(prot.0 & 0x7)).expect("three bits")
}

fn to_ns(t: Timespec) -> u64 {
    if t.sec < 0 { 0 } else { (t.sec as u64).saturating_mul(1_000_000_000).saturating_add(t.nsec.max(0) as u64) }
}

/// Where in the address space the user registers of `ctx` came from: the
/// trap frame with the personality's view written over it.
fn regs_from(ctx: &UserContext, old: &UserRegs) -> UserRegs {
    UserRegs {
        r15: ctx.r15,
        r14: ctx.r14,
        r13: ctx.r13,
        r12: ctx.r12,
        r11: ctx.r11,
        r10: ctx.r10,
        r9: ctx.r9,
        r8: ctx.r8,
        rbp: ctx.rbp,
        rdi: ctx.rdi,
        rsi: ctx.rsi,
        rdx: ctx.rdx,
        rcx: ctx.rcx,
        rbx: ctx.rbx,
        rax: ctx.rax,
        rip: ctx.rip,
        rflags: ctx.rflags,
        rsp: ctx.rsp,
        ..*old
    }
}

impl LinuxKernel for Thread<'_> {
    fn pid(&self) -> i32 {
        self.process.pid as i32
    }

    fn tid(&self) -> i32 {
        self.tid as i32
    }

    fn ppid(&self) -> i32 {
        super::task::parent_of(self.process) as i32
    }

    fn credentials(&self) -> Credentials {
        let (tid, pid) = (self.tid as i32, self.process.pid as i32);
        self.linux.with(|p| {
            let creds = if p.has_thread(tid) { Some(&p.thread(tid).creds) } else { p.process_creds(pid) };
            creds.map_or_else(Credentials::default, |c| c.summary())
        })
    }

    fn copy_from_user(&self, src: UserPtr, dst: &mut [u8]) -> Result<(), Errno> {
        user::copy_from_user(dst, src.0).map_err(|_| Errno::EFAULT)
    }

    fn copy_to_user(&mut self, dst: UserPtr, src: &[u8]) -> Result<(), Errno> {
        user::copy_to_user(dst.0, src).map_err(|_| Errno::EFAULT)
    }

    fn write(&mut self, fd: i32, buf: &[u8]) -> Result<usize, Errno> {
        let n = fileio::write(self, fd, buf)?;
        if self.process.traced && (fd == 1 || fd == 2) && n > 0 {
            testrun::output(fd, &buf[..n.min(buf.len())]);
        }
        Ok(n)
    }

    fn read(&mut self, fd: i32, buf: &mut [u8]) -> Result<usize, Errno> {
        fileio::read(self, fd, buf)
    }

    fn close(&mut self, fd: i32) -> Result<(), Errno> {
        self.process.files.close(fd).map_err(vfs_errno)?;
        // Sockets only descriptors in flight hold (ADR 0048).
        super::net::after_close();
        Ok(())
    }

    fn fstat(&mut self, fd: i32) -> Result<FileStat, Errno> {
        self.process.files.fstat(fd).map(|st| file_stat(&st)).map_err(vfs_errno)
    }

    fn ioctl(&mut self, fd: i32, request: u32, arg: u64) -> Result<u64, Errno> {
        // A terminal request interrupted (job control's SIGTTOU) restarts
        // under SA_RESTART, as on a slow device (signal(7), ADR 0046).
        self.process.files.ioctl(fd, request, arg).map_err(|e| match e {
            poly_vfs::Errno::EINTR => Errno::RESTART_IF_SA_RESTART,
            e => vfs_errno(e),
        })
    }

    fn mmap(&mut self, r: &MapRequest) -> Result<u64, Errno> {
        let memory = self.memory()?;
        let prot = vm_prot(r.prot);
        let req = match r.backing {
            // Shared anonymous memory is a shared mapping of an object of
            // its own, whose pages every process mapping it finds (ADR 0041).
            Backing::Anonymous if r.shared => {
                let object = FileRef::new(Arc::new(user::AnonShared::new(r.len)));
                poly_vm::MapRequest::file(r.len, prot, object, 0, true)
            }
            Backing::Anonymous => poly_vm::MapRequest::anonymous(r.len, prot),
            Backing::File { fd, offset } => {
                let file = self.process.files.file(fd).map_err(vfs_errno)?;
                // mmap(2): "EACCES A file descriptor refers to a non-regular
                // file. Or a file mapping was requested, but fd is not open
                // for reading."
                if !file.readable() {
                    return Err(Errno::EACCES);
                }
                // Pipes and event files have nothing to map.
                if file.object().is_some() {
                    return Err(Errno::ENODEV);
                }
                if file.kind() != FileType::Regular {
                    return Err(if file.kind() == FileType::CharDevice { Errno::ENODEV } else { Errno::EACCES });
                }
                if r.shared && r.prot.writable() && !file.writable() {
                    return Err(Errno::EACCES);
                }
                let source = FileSource::of(&file);
                poly_vm::MapRequest::file(r.len, prot, FileRef::new(Arc::new(source)), offset, r.shared)
            }
        };
        let placement = match r.placement {
            Placement::Hint => poly_vm::Placement::Hint(r.addr),
            Placement::Fixed => poly_vm::Placement::Fixed(r.addr),
            Placement::FixedNoReplace => poly_vm::Placement::FixedNoReplace(r.addr),
        };
        let addr = memory.map(req.at(placement)).map_err(memory_errno)?;
        // MAP_32BIT: the region map has no "below 2 GiB" placement yet; a
        // mapping that landed higher is taken back.
        if r.hints.low_2g && addr.saturating_add(r.len) > 1 << 31 {
            let _ = memory.unmap(addr, r.len);
            return Err(Errno::ENOMEM);
        }
        Ok(addr)
    }

    fn munmap(&mut self, addr: u64, len: u64) -> Result<(), Errno> {
        self.memory()?.unmap(addr, len).map_err(memory_errno)
    }

    fn mprotect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Errno> {
        // PROT_GROWSDOWN would extend the change to the start of a stack
        // region; it is applied to the given range only for now.
        self.memory()?.protect(addr, len, vm_prot(prot)).map_err(memory_errno)
    }

    fn madvise(
        &mut self,
        addr: u64,
        len: u64,
        advice: poly_personality_linux::sys::madvise::Advice,
    ) -> Result<(), Errno> {
        use crate::user::memory::Advice as Do;
        use poly_personality_linux::sys::madvise::Advice;
        let what = match advice {
            Advice::Hint => Do::Keep,
            Advice::DontNeed => Do::Discard,
            Advice::Free => Do::DiscardAnonymous,
            Advice::Remove => Do::FreeBacking,
        };
        self.memory()?.advise(addr, len, what).map_err(|e| Errno::new(e.linux_errno()).unwrap_or(Errno::EINVAL))
    }

    fn brk(&self) -> u64 {
        user::current_memory().map_or(0, |m| m.brk(0))
    }

    fn set_brk(&mut self, addr: u64) -> Result<u64, Errno> {
        let got = self.memory()?.brk(addr);
        if got == addr { Ok(got) } else { Err(Errno::ENOMEM) }
    }

    fn fs_base(&self) -> u64 {
        user::fs_base()
    }

    fn set_fs_base(&mut self, base: u64) {
        // The personality passes only user addresses, which are canonical.
        let _ = user::set_fs_base(base);
    }

    fn gs_base(&self) -> u64 {
        user::gs_base()
    }

    fn set_gs_base(&mut self, base: u64) {
        let _ = user::set_gs_base(base);
    }

    fn pgid(&self) -> i32 {
        super::task::group_and_session(self.process.pid).0
    }

    fn sid(&self) -> i32 {
        super::task::group_and_session(self.process.pid).1
    }

    fn with_process(
        &mut self,
        target: Target,
        f: &mut dyn FnMut(&mut LinuxProcess, &ProcessInfo),
    ) -> Result<(), Errno> {
        let tid = match target {
            Target::Thread(tid) => tid,
            Target::Current => self.tid as i32,
            Target::Process(pid) => pid,
        };
        let target = self.target(target)?;
        let p = target.get();
        let linux = if core::ptr::eq(p, self.process) { Some(self.linux) } else { p.linux.as_ref() };
        let linux = linux.ok_or(Errno::ESRCH)?;
        let info = info_of(p);
        linux.with(|state| {
            let info = with_ids(info, state, tid);
            f(state, &info)
        });
        Ok(())
    }

    fn process_ids(&self, selector: ProcessSelector) -> Vec<i32> {
        process::all()
            .iter()
            .filter(|p| p.linux.is_some())
            .map(|p| p.pid as i32)
            .filter(|&pid| match selector {
                ProcessSelector::All => true,
                ProcessSelector::Group(g) => super::task::group_and_session(pid as u32).0 == g,
            })
            .collect()
    }

    fn notify_signal(&mut self, tid: i32) {
        if let Some(flags) = self.flags_of(tid) {
            process::notify(&flags);
        }
    }

    fn clear_signal_notification(&mut self) {
        self.flags.notified.store(false, Ordering::SeqCst);
    }

    fn stop_process(&mut self, signal: u32) {
        kprintln!("linux: pid {} stopped by signal {signal}", self.process.pid);
        self.process.stop();
        // The parent learns of it: wait(2) WUNTRACED, SIGCHLD (ADR 0041).
        poly_personality_linux::task::process_stopped(self, signal);
        self.process.wait_while_stopped();
    }

    fn continue_process(&mut self, pid: i32) {
        let target = if pid == self.process.pid as i32 { None } else { process::find(pid as u32) };
        let p = target.as_deref().unwrap_or(self.process);
        if p.cont() {
            kprintln!("linux: pid {} continued", p.pid);
            poly_personality_linux::task::process_continued(self, pid);
        }
    }

    fn kill_process(&mut self, signal: u32, _core_dump: bool) {
        // No core dumps are written, so WCOREDUMP is never reported.
        let status = ExitStatus::Signaled { signal, core: false };
        if self.fault.is_none() {
            kprintln!("linux: pid {} killed by signal {signal}", self.process.pid);
            if self.process.traced {
                testrun::signal(self.process.pid, signal);
            }
        }
        self.process.group_exit(status);
        self.exit = Some(status);
    }

    fn user_context(&self) -> UserContext {
        let r = user::registers();
        let (trapno, err, cr2) = self.fault.unwrap_or_default();
        UserContext {
            r8: r.r8,
            r9: r.r9,
            r10: r.r10,
            r11: r.r11,
            r12: r.r12,
            r13: r.r13,
            r14: r.r14,
            r15: r.r15,
            rdi: r.rdi,
            rsi: r.rsi,
            rbp: r.rbp,
            rbx: r.rbx,
            rdx: r.rdx,
            rax: self.rax.unwrap_or(r.rax),
            rcx: r.rcx,
            rsp: r.rsp,
            rip: r.rip,
            rflags: r.rflags,
            cs: r.cs as u16,
            ss: r.ss as u16,
            trapno,
            err,
            cr2,
        }
    }

    fn set_user_context(&mut self, ctx: &UserContext) {
        let regs = regs_from(ctx, &user::registers());
        if user::set_registers(&regs).is_ok() {
            self.replaced = true;
            if self.rax.is_some() {
                self.rax = Some(ctx.rax);
            }
        }
    }

    fn fpu_state(&self) -> FpuState<'_> {
        let (image, xfeatures) = self.fpu.get_or_init(|| user::fpu_image().unwrap_or_default());
        FpuState { image, xfeatures: *xfeatures }
    }

    fn set_fpu_state(&mut self, image: &[u8]) -> Result<(), Errno> {
        self.fpu = OnceCell::new();
        user::load_fpu(image).map_err(|_| Errno::EINVAL)
    }

    fn reset_fpu_state(&mut self) {
        self.fpu = OnceCell::new();
        user::reset_fpu();
    }

    fn block(&mut self, deadline: Option<Deadline>) -> Wakeup {
        // A wall-clock deadline is the monotonic time the wall clock
        // reaches it, as the clock is set now; callers that care check the
        // wall clock again when the wait ends (it may have been set back).
        let deadline = deadline.map(|d| match d.clock {
            Clock::Realtime => to_ns(d.at).saturating_sub(time::realtime_offset_ns()),
            _ => to_ns(d.at),
        });
        match process::park(self.process, &self.flags, deadline) {
            Unparked::Woken => Wakeup::Woken,
            Unparked::TimedOut => Wakeup::TimedOut,
            Unparked::Interrupted => Wakeup::Signal,
        }
    }

    fn unblock(&mut self, tid: i32) {
        if let Some(flags) = self.flags_of(tid) {
            process::unpark(&flags);
        }
    }

    fn with_futexes(&mut self, f: &mut dyn FnMut(&mut FutexTable, &mut dyn LinuxKernel)) {
        let mut table = super::FUTEXES.lock();
        f(&mut table, self);
    }

    fn address_space_id(&self) -> u64 {
        user::current_address_space().map_or(0, |s| s.root())
    }

    fn futex_cmpxchg(&mut self, addr: UserPtr, expected: u32, new: u32) -> Result<u32, Errno> {
        if !addr.0.is_multiple_of(4) {
            return Err(Errno::EINVAL);
        }
        // Bring the page in writable first if that may block here; under the
        // futex table's lock (interrupts off) it must be present already.
        if crate::arch::cpu::interrupts_enabled() {
            let _ = user::page_fault(addr.0, Access::Write);
        }
        // Atomic on the user word: other threads of the process may run on
        // other CPUs meanwhile.
        user::cmpxchg_user_u32(addr.0, expected, new).map_err(|_| Errno::EFAULT)
    }

    fn exit_thread(&mut self, status: u8) {
        self.exit = Some(ExitStatus::Exited(status));
    }

    fn exit_group(&mut self, status: u8) {
        let status = self.process.group_exit(ExitStatus::Exited(status));
        self.exit = Some(status);
    }

    fn clock(&self, clock: Clock) -> Result<Timespec, Errno> {
        match clock {
            Clock::Realtime => Ok(Timespec::from_nanos(time::realtime_ns())),
            Clock::Monotonic => Ok(Timespec::from_nanos(time::now_ns())),
            Clock::ProcessCpuTime => Ok(Timespec::from_nanos(cputime::process(self.process).total_ns())),
            Clock::ThreadCpuTime => cputime::thread(self.process.pid, self.tid)
                .map(|t| Timespec::from_nanos(t.total_ns()))
                .ok_or(Errno::EINVAL),
        }
    }

    fn set_realtime(&mut self, at: Timespec) -> Result<(), Errno> {
        time::set_realtime_ns(to_ns(at)).map_err(|()| Errno::EINVAL)
    }

    fn timezone(&self) -> (i32, i32) {
        let tz = crate::vdso::timezone();
        (tz.minuteswest, tz.dsttime)
    }

    fn set_timezone(&mut self, minuteswest: i32, dsttime: i32) {
        crate::vdso::set_timezone(poly_vdso::Timezone { minuteswest, dsttime });
    }

    fn tick_ns(&self) -> u64 {
        time::NANOS_PER_TICK
    }

    fn current_cpu(&self) -> u32 {
        crate::thread::this_cpu() as u32
    }

    fn yield_cpu(&mut self) {
        crate::thread::yield_now();
    }

    fn cpu_affinity(&mut self, tid: i32) -> Result<u64, Errno> {
        let id = self.sched_target(tid)?;
        let mask = crate::thread::affinity(id).ok_or(Errno::ESRCH)?;
        Ok(mask.intersection(crate::thread::online_cpus()).bits())
    }

    fn set_cpu_affinity(&mut self, tid: i32, mask: u64) -> Result<(), Errno> {
        let id = self.sched_target(tid)?;
        let cpus = crate::thread::CpuSet::from_bits(mask).intersection(crate::thread::online_cpus());
        if cpus.is_empty() {
            return Err(Errno::EINVAL);
        }
        crate::thread::set_affinity(id, cpus).map_err(|_| Errno::ESRCH)
    }

    fn cpu_times(&self, of: CpuTimesOf) -> Option<CpuTimes> {
        match of {
            CpuTimesOf::Thread(tid) => cputime::thread(self.process.pid, u32::try_from(tid).ok()?),
            CpuTimesOf::Process => Some(cputime::process(self.process)),
            CpuTimesOf::Children => Some(self.process.cpu.children()),
        }
    }

    fn timers_changed(&mut self, _next: TimerWakeup) {
        // Recomputed with the process's state locked, so that concurrent
        // changes and the timer thread never store a stale wake-up.
        super::timers::changed(self.process);
    }

    fn clone_task(&mut self, req: &poly_personality_linux::task::clone::CloneRequest) -> Result<i32, Errno> {
        super::task::clone_task(self, req)
    }

    fn execve(&mut self, req: &poly_personality_linux::task::exec::ExecRequest<'_>) -> Result<(), Errno> {
        super::task::execve(self, req)
    }

    fn with_tree(
        &mut self,
        f: &mut dyn FnMut(&mut poly_personality_linux::task::tree::ProcessTree),
    ) -> Result<(), Errno> {
        super::task::with_tree(|t| f(t));
        Ok(())
    }

    fn wait_tree(&mut self, generation: u64) -> Wakeup {
        super::task::wait_tree(self, generation)
    }

    fn random_bytes(&mut self, buf: &mut [u8], _mode: RandomMode) -> Result<usize, Errno> {
        // The generator is seeded at its first use and never waits
        // (`crate::random`), so no mode ever blocks.
        crate::random::fill(buf);
        Ok(buf.len())
    }

    fn fs(&mut self) -> Option<&mut dyn poly_personality_linux::fs::LinuxFs> {
        Some(self)
    }

    fn system_info(&mut self) -> Result<poly_personality_linux::fs::SysInfo, Errno> {
        Ok(super::files::system_info())
    }

    fn file_io(&mut self) -> Option<&mut dyn poly_personality_linux::fileio::FileIo> {
        Some(self)
    }

    fn signal_queued(&mut self, pid: i32) {
        super::signalfd::signal_queued(pid);
    }

    fn net(&mut self) -> Option<&mut dyn poly_personality_linux::net::LinuxNet> {
        Some(self)
    }
}
