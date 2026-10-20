//! User mode, the architecture-neutral side: threads that run a process's
//! code in ring 3, their system calls and faults, and safe access to user
//! memory.
//!
//! # Running user code
//!
//! 1. [`AddressSpace::new`] makes an empty address space (the kernel half is
//!    shared); [`AddressSpace::map_zeroed`], [`AddressSpace::write`] and
//!    [`AddressSpace::protect`] fill it (the ELF loader's job later).
//! 2. [`spawn`] starts a thread of a [`Process`] in it at an [`Entry`]
//!    (RIP, RSP and three arguments in RDI, RSI, RDX), with a
//!    [`SyscallHook`] that handles its system calls. [`DEFAULT_HOOK`]
//!    routes them to the process's personality (`syscall::dispatch`).
//! 3. Each `syscall` instruction arrives in [`syscall`] on the thread's
//!    kernel stack with interrupts on: the hook reads and writes the
//!    registers as a `poly_abi::SyscallRegs`. It may block, sleep, yield,
//!    copy from and to user memory, end the thread ([`exit`]) or make it
//!    resume somewhere else ([`set_return_address`]).
//! 4. A CPU exception in user mode kills the thread ([`kill_current`]): the
//!    kernel logs it and the thread exits with [`killed`]`(vector)`, which
//!    is what [`thread::join`] returns.
//!
//! Killing a thread does not (yet) end the other threads of its process:
//! there is no process-wide thread list until exit_group needs one.
//!
//! # User memory
//!
//! [`copy_from_user`], [`copy_to_user`] and [`strncpy_from_user`] access the
//! address space that is loaded, i.e. the calling thread's, and return
//! [`Efault`] for any address that is not mapped user memory with the right
//! access, instead of faulting. Code that runs on behalf of a process uses
//! nothing else to touch its memory. [`AddressSpace::read`] and
//! [`AddressSpace::write`] reach a space that is not loaded (a loader
//! filling a new process, a debugger).

mod programs;
mod tests;

pub use crate::arch::user::{
    AddressSpace, Efault, Entry, Fault, MapError, Mapping, Prot, USER_END, USER_START, copy_from_user, copy_to_user,
    fs_base, gs_base, is_user_range, set_fs_base, set_gs_base, strncpy_from_user,
};

use crate::arch;
use crate::kprintln;
use crate::process::Process;
use crate::thread::{self, SpawnError, ThreadId};
use alloc::boxed::Box;
use alloc::sync::Arc;
use poly_abi::SyscallRegs;

/// Handles one system call of a user thread: reads the arguments from
/// `regs` and leaves the results there, in the process's convention.
pub type SyscallHook = fn(&Process, &mut SyscallRegs);

/// Route system calls to the process's personality.
pub const DEFAULT_HOOK: SyscallHook = crate::syscall::dispatch_hook;

/// Exit codes of killed threads: this bit, ORed with the exception vector.
pub const KILLED: usize = 1 << 63;

/// The exit code of a thread killed by CPU exception `vector`.
pub const fn killed(vector: u8) -> usize {
    KILLED | vector as usize
}

/// Everything the kernel keeps about a user thread besides its kernel
/// thread: kept in the thread table, dropped when the thread is reaped.
#[derive(Debug)]
pub struct UserThread {
    process: Arc<Process>,
    hook: SyscallHook,
    entry: Entry,
    /// Where to resume ring 3 after the current system call, if not after
    /// the `syscall` instruction ([`set_return_address`]).
    return_to: Option<u64>,
    /// Address space, FPU state, FS/GS bases: switched by the scheduler.
    pub(crate) arch: arch::user::ThreadState,
}

/// Start a thread of `process` that runs in `space` from `entry`, its
/// system calls handled by `hook`. It runs once the threads ahead of it
/// have had their turn; join it like any thread.
pub fn spawn(
    process: Arc<Process>,
    space: Arc<AddressSpace>,
    entry: Entry,
    hook: SyscallHook,
) -> Result<ThreadId, SpawnError> {
    let arch = arch::user::ThreadState::new(space).ok_or(SpawnError::NoMemory)?;
    let user = Box::new(UserThread { process, hook, entry, return_to: None, arch });
    thread::spawn_user(user, user_main, 0)
}

/// The kernel side of a new user thread: everything is loaded by the switch
/// that started it, so it only has to leave for ring 3.
fn user_main(_: usize) -> usize {
    let entry = thread::with_current_user(|u| u.entry).expect("user thread without user state");
    arch::user::enter_user(&entry)
}

/// A system call from the running user thread, called by the architecture's
/// entry with interrupts on. Returns where to resume ring 3 if
/// [`set_return_address`] asked for somewhere other than the next
/// instruction.
pub fn syscall(regs: &mut SyscallRegs) -> Option<u64> {
    let (process, hook) =
        thread::with_current_user(|u| (Arc::clone(&u.process), u.hook)).expect("system call from a kernel thread");
    hook(&process, regs);
    thread::with_current_user(|u| u.return_to.take()).flatten()
}

/// Make the running user thread resume ring 3 at `rip` when the current
/// system call returns. An address outside the lower half makes the return
/// fail, which kills the thread. Fails on a kernel thread.
pub fn set_return_address(rip: u64) -> Result<(), ()> {
    thread::with_current_user(|u| u.return_to = Some(rip)).ok_or(())
}

/// End the running user thread with `code` (what [`thread::join`] returns).
pub fn exit(code: usize) -> ! {
    thread::exit(code)
}

/// Kill the running user thread for `fault`, a CPU exception it caused in
/// ring 3. Logs one line and never returns.
pub fn kill_current(fault: &Fault) -> ! {
    let pid = thread::with_current_user(|u| u.process.pid);
    match pid {
        Some(pid) => kprintln!("user: pid {pid} thread {} killed: {fault}", thread::current()),
        None => panic!("{fault} in a kernel thread"),
    }
    thread::exit(killed(fault.vector))
}

/// The process of the running thread, if it is a user thread.
pub fn current_process() -> Option<Arc<Process>> {
    thread::with_current_user(|u| Arc::clone(&u.process))
}

/// The address space of the running thread, if it is a user thread.
pub fn current_address_space() -> Option<Arc<AddressSpace>> {
    thread::with_current_user(|u| Arc::clone(u.arch.space()))
}
