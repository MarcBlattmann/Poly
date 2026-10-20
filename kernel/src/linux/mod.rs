//! The Linux personality in the kernel: the state the kernel keeps for each
//! Linux process, the kernel services the personality asks for
//! ([`LinuxKernel`](poly_personality_linux::LinuxKernel), implemented by
//! [`Thread`] in `services.rs`), and the places where the kernel hands
//! control to it (ADR 0025, 0031, 0038):
//!
//! ```text
//!  syscall from a Linux process ──► syscall::dispatch ──► linux::syscall
//!       Linux::dispatch_with(Thread, regs)   the handler, through LinuxKernel
//!       POLY-SYSCALL record (traced processes)
//!       exit / exit_group / kill ──► user::request_exit: the thread ends
//!       Linux::exit_to_user(Thread)          pending signals, restarts
//!  fault in ring 3 not resolved ──► user::fault ──► linux::fault
//!       Linux::fault_signal + exit_to_user   SIGSEGV etc.: handler or death
//!  interrupt from ring 3, thread notified ──► user::return_to_user ──► linux::exit_to_user
//!  thread ends ──► user::end_thread ──► linux::thread_exit
//!       Linux::thread_exit                   robust futexes, clear_child_tid
//!  last thread gone ──► user::end_thread ──► linux::process_exit
//!       task::process_exited                 the tree (zombie, orphans), SIGCHLD
//! ```
//!
//! Processes and threads (`clone`, `fork`, `execve`, `wait4`, the process
//! tree) are in `task.rs` (ADR 0041).
//!
//! A Linux process ([`crate::process::Process::new_linux`]) carries a
//! [`LinuxState`]: the personality's [`LinuxProcess`] (signal dispositions,
//! pending signals, per-thread masks, robust lists) behind a spinlock, lent
//! to the personality by `with_process`. One futex table serves every
//! process ([`FUTEXES`]).

mod fileio_tests;
mod files;
mod files_tests;
pub mod net;
mod services;
pub mod signalfd;
mod socket_tests;
pub mod task;
mod task_tests;
mod tests;
pub mod timers;

pub use services::Thread;

use crate::process::Process;
use crate::sync::SpinLock;
use crate::user::memory::SegvCode;
use crate::user::{self, Fault, FaultError};
use crate::{kprintln, testrun};
use alloc::sync::Arc;
use poly_abi::{Outcome, SyscallRegs};
use poly_personality_linux::futex::FutexTable;
use poly_personality_linux::signal::fault::{PageFault, fault_info};
use poly_personality_linux::{ExitToUser, Linux, LinuxKernel, LinuxProcess};

/// `SIGSEGV` (`asm-generic/signal.h`).
pub const SIGSEGV: u32 = poly_personality_linux::abi::signal::SIGSEGV;

/// The futex wait queues of every Linux process (futex(2): shared futexes
/// cross address spaces).
static FUTEXES: SpinLock<FutexTable> = SpinLock::new(FutexTable::new());

/// What the kernel keeps for the Linux personality per process.
#[derive(Debug)]
pub struct LinuxState {
    process: SpinLock<LinuxProcess>,
}

impl LinuxState {
    /// The state of a new process whose main thread has id `pid`: default
    /// signal dispositions, nothing blocked or pending.
    pub fn new(pid: u32) -> LinuxState {
        // A process the kernel starts: the root of a process tree.
        task::register_root(pid);
        LinuxState { process: SpinLock::new(LinuxProcess::new(pid as i32)) }
    }

    /// A thread `tid` was started (the main thread is there from the start):
    /// it inherits the signal mask of the oldest thread, as a clone(2)
    /// thread inherits its creator's.
    pub fn add_thread(&self, tid: u32) {
        let mut p = self.process.lock();
        let tid = tid as i32;
        if p.has_thread(tid) {
            return;
        }
        let parent = p.thread_ids().next();
        match parent {
            Some(parent) => p.add_thread(parent, tid, true),
            None => *p = LinuxProcess::new(tid),
        }
    }

    /// Thread `tid` has ended.
    pub fn remove_thread(&self, tid: u32) {
        self.process.lock().remove_thread(tid as i32);
    }

    /// Run `f` on the personality's state, locked. `f` must not block.
    pub fn with<R>(&self, f: impl FnOnce(&mut LinuxProcess) -> R) -> R {
        f(&mut self.process.lock())
    }
}

/// What the thread does after the personality acted on its signals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// Return to user mode (perhaps into a signal handler).
    Return,
    /// Its process was killed: the thread must end.
    Killed,
}

impl From<ExitToUser> for Leave {
    fn from(e: ExitToUser) -> Leave {
        match e {
            ExitToUser::Return => Leave::Return,
            ExitToUser::Killed => Leave::Killed,
        }
    }
}

/// A system call of a Linux process (`syscall::dispatch` routes here): run
/// it through the personality with the kernel's services, record it for a
/// traced process, end the thread if the call asked for that, and act on
/// pending signals before the return to user mode.
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let Some(mut k) = Thread::current(process) else {
        // Not a thread the process knows (cannot happen for threads started
        // by `user::spawn_in`): no services.
        return <Linux as poly_abi::SyscallDispatch>::dispatch(regs);
    };
    let number = regs.rax;
    let args = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9];
    k.enter_syscall(number);
    let outcome = Linux::dispatch_with(&mut k, regs);
    k.set_result(regs.rax);
    let exit = k.take_exit();
    if process.traced {
        testrun::syscall(process.pid, number, &args, exit.is_none().then_some(regs.rax));
    }
    if let Some(status) = exit {
        let _ = user::request_exit(status);
        return outcome;
    }
    if Linux::exit_to_user(&mut k) == ExitToUser::Killed {
        let status = k.take_exit().unwrap_or(crate::process::ExitStatus::Exited(0));
        let _ = user::request_exit(status);
        return outcome;
    }
    if k.registers_replaced() {
        // The return restores every register; rax must be the same value.
        regs.rax = user::registers().rax;
    }
    outcome
}

/// The running thread of `process` is about to return to user mode after
/// an interrupt or exception and was notified of a signal: act on it.
pub fn exit_to_user(process: &Process) -> Leave {
    match Thread::current(process) {
        Some(mut k) => Linux::exit_to_user(&mut k).into(),
        None => Leave::Return,
    }
}

/// `fault`, which demand paging did not resolve, becomes a signal for the
/// running thread of `process` (signal(7)), acted on at once: a handler
/// runs next ([`Leave::Return`]) or the process dies.
pub fn fault(process: &Process, fault: &Fault) -> Leave {
    let Some(mut k) = Thread::current(process) else { return Leave::Killed };
    let page = match (fault.address, fault.page) {
        (Some(addr), Some(why)) => Some((addr, page_fault(why))),
        (Some(addr), None) => Some((addr, PageFault::NotMapped)),
        _ => None,
    };
    let info = fault_info(fault.vector, page, fault.rip);
    k.set_fault(u64::from(fault.vector), fault.error_code, fault.address.unwrap_or(0));
    if process.traced {
        testrun::signal(process.pid, info.signal());
    }
    Linux::fault_signal(&mut k, info);
    let leave = Linux::exit_to_user(&mut k).into();
    if leave == Leave::Killed {
        kprintln!(
            "linux: pid {} killed by signal {} after {} at rip {:#x}{}",
            process.pid,
            info.signal(),
            fault.name,
            fault.rip,
            fault.address.map(|a| alloc::format!(", address {a:#x}")).unwrap_or_default()
        );
    }
    leave
}

fn page_fault(why: FaultError) -> PageFault {
    match why {
        FaultError::Segv(SegvCode::AccErr) => PageFault::Access,
        FaultError::Bus => PageFault::Bus,
        _ => PageFault::NotMapped,
    }
}

/// Thread `tid` of `process`, the running one, is ending: the personality's
/// duties (robust futex list, `clear_child_tid`).
pub fn thread_exit(process: &Process, tid: u32) {
    if let Some(mut k) = Thread::current(process) {
        debug_assert_eq!(k.tid(), tid as i32);
        // set_tid_address(2): "if the thread is sharing memory with other
        // threads": other threads, or another process (CLONE_VM).
        let shares_memory =
            process.thread_count() > 1 || user::current_memory().is_some_and(|m| Arc::strong_count(&m) > 2);
        Linux::thread_exit(&mut k, shares_memory);
    }
    task::vfork_release(tid);
}

/// The last thread of `process` (the running thread) has left it with
/// `status`: the process tree and the parent learn of it (`task.rs`).
pub fn process_exit(process: &Process, status: crate::process::ExitStatus) {
    task::process_exit(process, status);
}
