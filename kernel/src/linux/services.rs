//! The kernel's services for the Linux personality: [`LinuxKernel`] for the
//! running thread of a Linux process.
//!
//! | Trait methods | Kernel |
//! |---|---|
//! | ids, credentials | the [`Process`] (everyone is root until users arrive) |
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
//! | clocks, random bytes | `time::now_ns`, `time::realtime_ns` (the CMOS RTC read once), RDRAND or the TSC |
//!
//! Nothing here holds a lock while it touches user memory, except the
//! futex calls the personality makes with the table locked
//! (`futex_load`, `futex_cmpxchg`): those run with interrupts off and fail
//! with `EFAULT` on a page that is not present, which the personality
//! prevents by reading the word first, unlocked.
//!
//! Not done yet, each documented where it is: parent notification on stop
//! and continue (no `wait4` yet), core dumps, shared mappings of files the
//! page cache does not hold (tmpfs: `ENODEV`), `MAP_32BIT` placement (refused with `ENOMEM` when the address
//! is not low), `PROT_GROWSDOWN` (applied to the given range only), CPU-time
//! clocks (`EINVAL`).

use super::LinuxState;
use crate::arch::cpu::InterruptsOff;
use crate::fs::cache::FileSource;
use crate::process::{self, ExitStatus, Process, ThreadFlags, Unparked};
use crate::testrun;
use crate::user::{self, FileRef, Memory, MemoryError, UserRegs};
use crate::{kprintln, time};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::OnceCell;
use core::sync::atomic::Ordering;
use poly_personality_linux::abi::{FileStat, Timespec, makedev};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::futex::FutexTable;
use poly_personality_linux::kernel::{
    Backing, Clock, Credentials, Deadline, FpuState, LinuxKernel, MapRequest, Placement, ProcessInfo, ProcessSelector,
    Prot, RandomMode, Target, UserContext, UserPtr, Wakeup,
};
use poly_personality_linux::signal::LinuxProcess;
use poly_vfs::FileType;
use poly_vm::Access;

/// The running thread of a Linux process, for one system call or one
/// return to user mode.
pub struct Thread<'a> {
    process: &'a Process,
    linux: &'a LinuxState,
    tid: u32,
    flags: Arc<ThreadFlags>,
    /// During a system call: `rax` as the thread will see it (the call
    /// number until the dispatcher has written the result).
    rax: Option<u64>,
    /// `trapno`, `err`, `cr2` of the fault being turned into a signal.
    fault: Option<(u64, u64, u64)>,
    /// How the thread (or its process) is to end.
    exit: Option<ExitStatus>,
    replaced: bool,
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

    fn memory(&self) -> Result<Arc<Memory>, Errno> {
        user::current_memory().ok_or(Errno::ENOMEM)
    }

    /// The process `target` names (`ESRCH` if none).
    fn target(&self, target: Target) -> Result<TargetProcess<'a>, Errno> {
        match target {
            Target::Current => Ok(TargetProcess::Own(self.process)),
            Target::Process(pid) if pid > 0 => process::find(pid as u32).map(TargetProcess::Other).ok_or(Errno::ESRCH),
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
    // One process group and session per process, all root, until
    // setsid / setpgid and users arrive.
    ProcessInfo { pid, pgid: pid, sid: pid, zombie: p.has_ended(), ..ProcessInfo::default() }
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
        self.process.ppid as i32
    }

    fn credentials(&self) -> Credentials {
        let cred = self.process.files.cred();
        Credentials { uid: cred.uid, euid: cred.uid, gid: cred.gid, egid: cred.gid }
    }

    fn copy_from_user(&self, src: UserPtr, dst: &mut [u8]) -> Result<(), Errno> {
        user::copy_from_user(dst, src.0).map_err(|_| Errno::EFAULT)
    }

    fn copy_to_user(&mut self, dst: UserPtr, src: &[u8]) -> Result<(), Errno> {
        user::copy_to_user(dst.0, src).map_err(|_| Errno::EFAULT)
    }

    fn write(&mut self, fd: i32, buf: &[u8]) -> Result<usize, Errno> {
        let n = self.process.files.write(fd, buf).map_err(vfs_errno)?;
        if self.process.traced && (fd == 1 || fd == 2) && n > 0 {
            testrun::output(fd, &buf[..n.min(buf.len())]);
        }
        Ok(n)
    }

    fn read(&mut self, fd: i32, buf: &mut [u8]) -> Result<usize, Errno> {
        self.process.files.read(fd, buf).map_err(vfs_errno)
    }

    fn close(&mut self, fd: i32) -> Result<(), Errno> {
        self.process.files.close(fd).map_err(vfs_errno)
    }

    fn fstat(&mut self, fd: i32) -> Result<FileStat, Errno> {
        self.process.files.fstat(fd).map(|st| file_stat(&st)).map_err(vfs_errno)
    }

    fn ioctl(&mut self, fd: i32, request: u32, arg: u64) -> Result<u64, Errno> {
        self.process.files.ioctl(fd, request, arg).map_err(vfs_errno)
    }

    fn mmap(&mut self, r: &MapRequest) -> Result<u64, Errno> {
        let memory = self.memory()?;
        let prot = vm_prot(r.prot);
        let req = match r.backing {
            // Shared anonymous memory is private until fork shares it.
            Backing::Anonymous => poly_vm::MapRequest::anonymous(r.len, prot),
            Backing::File { fd, offset } => {
                let file = self.process.files.file(fd).map_err(vfs_errno)?;
                // mmap(2): "EACCES A file descriptor refers to a non-regular
                // file. Or a file mapping was requested, but fd is not open
                // for reading."
                if !file.readable() {
                    return Err(Errno::EACCES);
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
        self.process.pid as i32
    }

    fn sid(&self) -> i32 {
        self.process.pid as i32
    }

    fn with_process(
        &mut self,
        target: Target,
        f: &mut dyn FnMut(&mut LinuxProcess, &ProcessInfo),
    ) -> Result<(), Errno> {
        let target = self.target(target)?;
        let p = target.get();
        let linux = if core::ptr::eq(p, self.process) { Some(self.linux) } else { p.linux.as_ref() };
        let linux = linux.ok_or(Errno::ESRCH)?;
        let info = info_of(p);
        linux.with(|state| f(state, &info));
        Ok(())
    }

    fn process_ids(&self, selector: ProcessSelector) -> Vec<i32> {
        process::all()
            .iter()
            .filter(|p| p.linux.is_some())
            .map(|p| p.pid as i32)
            .filter(|&pid| match selector {
                ProcessSelector::All => true,
                // Every process is its own group for now.
                ProcessSelector::Group(g) => pid == g,
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
        // Parent notification (wait(2) WIFSTOPPED, SIGCHLD) waits for wait4.
        kprintln!("linux: pid {} stopped by signal {signal}", self.process.pid);
        self.process.stop();
        self.process.wait_while_stopped();
    }

    fn continue_process(&mut self, pid: i32) {
        let target = if pid == self.process.pid as i32 { None } else { process::find(pid as u32) };
        let p = target.as_deref().unwrap_or(self.process);
        if p.cont() {
            kprintln!("linux: pid {} continued", p.pid);
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
        // The wall clock moves with the monotonic one (it cannot be set
        // yet), so a wall-clock deadline is a fixed monotonic time.
        let deadline = deadline.map(|d| match d.clock {
            Clock::Realtime => to_ns(d.at).saturating_sub(time::realtime_ns() - time::now_ns()),
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
        // One CPU: with interrupts off nothing else runs between the read
        // and the write. TODO(smp): a `lock cmpxchg` with a fixup.
        let _irq = InterruptsOff::new();
        let mut word = [0u8; 4];
        user::copy_from_user(&mut word, addr.0).map_err(|_| Errno::EFAULT)?;
        let found = u32::from_le_bytes(word);
        if found == expected {
            user::copy_to_user(addr.0, &new.to_le_bytes()).map_err(|_| Errno::EFAULT)?;
        }
        Ok(found)
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
            // CPU time is not accounted per thread yet.
            Clock::ProcessCpuTime | Clock::ThreadCpuTime => Err(Errno::EINVAL),
        }
    }

    fn random_bytes(&mut self, buf: &mut [u8], _mode: RandomMode) -> Result<usize, Errno> {
        // The source never needs initialising, so no mode ever waits.
        for chunk in buf.chunks_mut(16) {
            let bytes = crate::arch::user::random_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(buf.len())
    }
}
