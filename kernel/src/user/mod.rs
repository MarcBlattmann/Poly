//! User mode, the architecture-neutral side: threads that run a process's
//! code in ring 3, their system calls and faults, and safe access to user
//! memory.
//!
//! # Running user code
//!
//! 1. [`exec::load`] builds the [`Memory`] of a static ELF executable
//!    (regions for its segments and stack, the initial stack with argv,
//!    envp and the auxiliary vector) and [`spawn_in`] starts a thread in it;
//!    [`exec::exec`] does both. Its pages come in on demand, at the first
//!    access ([`memory`]). The file comes from anywhere that implements
//!    [`PageSource`]: a VFS file (`fs::cache::FileSource`) or bytes in memory.
//!
//!    Tests also build bare address spaces by hand: [`AddressSpace::new`]
//!    makes an empty one (the kernel half is shared);
//!    [`AddressSpace::map_zeroed`], [`AddressSpace::write`] and
//!    [`AddressSpace::protect`] fill it. Such a space has no memory map,
//!    so every page fault in it is fatal.
//! 2. [`spawn`] starts a thread of a [`Process`] in it at an [`Entry`]
//!    (RIP, RSP and three arguments in RDI, RSI, RDX), with a
//!    [`SyscallHook`] that handles its system calls. [`DEFAULT_HOOK`]
//!    routes them to the process's personality (`syscall::dispatch`).
//! 3. Each `syscall` instruction arrives in [`syscall`] on the thread's
//!    kernel stack with interrupts on: the hook reads and writes the
//!    registers as a `poly_abi::SyscallRegs`. It may block, sleep, yield,
//!    copy from and to user memory, end the thread ([`exit`],
//!    [`request_exit`]), make it resume somewhere else
//!    ([`set_return_address`]) or replace all its registers
//!    ([`set_registers`]).
//! 4. A page fault on a page its [`Memory`] can bring in is resolved and the
//!    instruction retried ([`page_fault`]). Any other CPU exception in user
//!    mode goes to [`fault`]: it becomes a signal in a process with
//!    personality state, and otherwise kills the thread ([`kill_current`]):
//!    the kernel logs it and the thread exits with [`killed`]`(vector)`,
//!    which is what [`thread::join`] returns.
//!
//! # Threads and processes
//!
//! Every user thread is registered with its [`Process`]
//! ([`Process::attach_thread`]) and has an id there. A thread ends itself
//! ([`exit`], [`end_thread`], or [`kill_current`] for a fault); the last
//! one to end ends the process. Ending a whole process
//! ([`Process::group_exit`]) makes every thread end itself the next time it
//! is about to return to user mode: after its system call ([`syscall`]) or
//! after an interrupt or exception in ring 3 ([`return_to_user`]).
//!
//! A process with personality state (a Linux process made with
//! [`Process::new_linux`]) gets more than its system calls routed: a CPU
//! exception in ring 3 that demand paging does not resolve becomes a
//! signal ([`fault`], `crate::linux`) instead of killing the thread, its
//! threads act on pending signals before they return to user mode, and
//! its personality learns of every thread exit (robust futex lists,
//! `clear_child_tid`). Bare process records ([`Process::new`], the kernel
//! tests) keep the plain behaviour: a fault kills the faulting thread.
//!
//! # User memory
//!
//! [`copy_from_user`], [`copy_to_user`] and [`strncpy_from_user`] access the
//! address space that is loaded, i.e. the calling thread's, and return
//! [`Efault`] for any address that is not mapped user memory with the right
//! access, instead of faulting. Pages of the thread's [`Memory`] that are
//! not present yet are brought in first, as for an access from ring 3.
//! Code that runs on behalf of a process uses nothing else to touch its
//! memory. [`Memory::read`], [`Memory::write`],
//! [`AddressSpace::read`] and [`AddressSpace::write`] reach a space that is
//! not loaded (a loader filling a new process, a debugger).

pub mod exec;
mod exec_tests;
pub mod memory;
mod memory_tests;
mod programs;
mod test_elf;
mod tests;

pub use crate::arch::user::{
    AddressSpace, Efault, Entry, Fault, MapError, Mapping, Prot, USER_END, USER_START, UserFrame, UserRegs,
    copy_from_user, copy_to_user, fs_base, gs_base, is_user_range, set_fs_base, set_gs_base, strncpy_from_user,
    user_frames,
};
pub use memory::{AnonShared, FaultError, FileRef, Memory, MemoryError, MemoryImage, PageSource};

use crate::arch;
use crate::kprintln;
use crate::process::{ExitStatus, Process, ThreadFlags};
use crate::thread::{self, SpawnError, ThreadId};
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;
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

/// Exit codes of threads whose process a signal killed: this bit (and
/// [`KILLED`]), ORed with the signal number.
pub const SIGNALED: usize = KILLED | 1 << 62;

/// The thread exit code [`thread::join`] reports for a thread that ended
/// with `status`.
pub const fn exit_code(status: ExitStatus) -> usize {
    match status {
        ExitStatus::Exited(code) => code as usize,
        ExitStatus::Signaled { signal, .. } => SIGNALED | signal as usize,
        ExitStatus::Faulted(vector) => killed(vector),
    }
}

/// Everything the kernel keeps about a user thread besides its kernel
/// thread: kept in the thread table, dropped when the thread is reaped.
#[derive(Debug)]
pub struct UserThread {
    process: Arc<Process>,
    hook: SyscallHook,
    entry: Entry,
    /// Every register to start with, instead of `entry` (a child of
    /// `clone` or `fork`, [`spawn_child`]).
    start: Option<Box<UserRegs>>,
    /// The memory map page faults are resolved against; `None` for a bare
    /// address space ([`spawn`]), where every fault is fatal.
    memory: Option<Arc<Memory>>,
    /// Where to resume ring 3 after the current system call, if not after
    /// the `syscall` instruction ([`set_return_address`]).
    return_to: Option<u64>,
    /// The thread's id in its process ([`Process::attach_thread`]).
    tid: u32,
    flags: Arc<ThreadFlags>,
    /// End the thread with this status when the current system call
    /// returns ([`request_exit`]).
    exit: Option<ExitStatus>,
    /// Every user register was replaced during the current system call
    /// ([`set_registers`]).
    replaced: bool,
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
    spawn_thread(process, space, None, entry, hook)
}

/// Start a thread of `process` in `memory` (see [`spawn`]): its page faults
/// are resolved against `memory`'s regions, so its pages come in on
/// demand. Threads started in the same `memory` share it, as the threads of
/// one process do.
pub fn spawn_in(
    process: Arc<Process>,
    memory: Arc<Memory>,
    entry: Entry,
    hook: SyscallHook,
) -> Result<ThreadId, SpawnError> {
    let space = Arc::clone(memory.space());
    spawn_thread(process, space, Some(memory), entry, hook)
}

fn spawn_thread(
    process: Arc<Process>,
    space: Arc<AddressSpace>,
    memory: Option<Arc<Memory>>,
    entry: Entry,
    hook: SyscallHook,
) -> Result<ThreadId, SpawnError> {
    let arch = arch::user::ThreadState::new(space).ok_or(SpawnError::NoMemory)?;
    start_thread(process, arch, memory, entry, None, hook, None).map(|(id, _)| id)
}

/// Start a thread of `process` in `memory` that begins with every
/// register of `regs`, the FPU state `fpu` (an image from [`fpu_image`])
/// and FS / GS bases `fs_base` / `gs_base`: the child of `clone` or
/// `fork`, which resumes where its parent made the call. `tid`: the
/// thread id to take (from `process::alloc_pid`; `None` for the next one
/// the process hands out). Returns the kernel thread and the thread id.
#[allow(clippy::too_many_arguments)] // what a new thread is made of
pub fn spawn_child(
    process: Arc<Process>,
    memory: Arc<Memory>,
    regs: &UserRegs,
    fpu: &[u8],
    fs_base: u64,
    gs_base: u64,
    hook: SyscallHook,
    tid: Option<u32>,
) -> Result<(ThreadId, u32), SpawnError> {
    let space = Arc::clone(memory.space());
    let arch = arch::user::ThreadState::with_state(space, fpu, fs_base, gs_base).ok_or(SpawnError::NoMemory)?;
    let entry = Entry { rip: regs.rip, rsp: regs.rsp, args: [regs.rdi, regs.rsi, regs.rdx] };
    start_thread(process, arch, Some(memory), entry, Some(Box::new(*regs)), hook, tid)
}

fn start_thread(
    process: Arc<Process>,
    arch: arch::user::ThreadState,
    memory: Option<Arc<Memory>>,
    entry: Entry,
    start: Option<Box<UserRegs>>,
    hook: SyscallHook,
    tid: Option<u32>,
) -> Result<(ThreadId, u32), SpawnError> {
    // A process that has ended takes no new threads.
    let attached = match tid {
        None => process.attach_thread(),
        Some(_) => process.attach_thread_as(tid),
    };
    let (tid, flags) = attached.ok_or(SpawnError::NoMemory)?;
    let attached = Arc::clone(&process);
    let user = Box::new(UserThread {
        process,
        hook,
        entry,
        start,
        memory,
        return_to: None,
        tid,
        flags,
        exit: None,
        replaced: false,
        arch,
    });
    thread::spawn_user("user", user, user_main, 0).map(|id| (id, tid)).inspect_err(|_| attached.cancel_thread(tid))
}

impl Drop for UserThread {
    fn drop(&mut self) {
        // The scheduler drops an exited thread's state wherever it reaps it,
        // perhaps in an interrupt handler. If this were the process's last
        // reference, dropping it there would let go of its file system
        // context (inodes, maybe on a disk mount, whose locks sleep): the
        // writeback thread drops it instead. Its open files were closed by
        // its last thread already (`Process::detach_thread`).
        if thread::reaping() {
            crate::fs::cache::release_later(Box::new(Arc::clone(&self.process)));
        }
    }
}

/// The kernel side of a new user thread: everything is loaded by the switch
/// that started it, so it only has to leave for ring 3.
fn user_main(_: usize) -> usize {
    let (entry, start) =
        thread::with_current_user(|u| (u.entry, u.start.take())).expect("user thread without user state");
    match start {
        Some(regs) => {
            let regs = *regs;
            arch::user::enter_user_regs(&regs)
        }
        None => arch::user::enter_user(&entry),
    }
}

/// The running user thread's process, borrowed for as long as the thread
/// runs, with its flags and thread id. Nothing is cloned, so a thread that
/// ends itself while holding these leaks nothing.
fn current() -> Option<(&'static Process, &'static ThreadFlags, u32)> {
    let (process, flags, tid) = thread::with_current_user(|u| (Arc::as_ptr(&u.process), Arc::as_ptr(&u.flags), u.tid))?;
    // SAFETY: the `UserThread` holding both `Arc`s stays in the thread table,
    // unchanged, until this thread has exited and been reaped, so the
    // pointees outlive every use of these references, which are only used
    // on this thread (`thread::exit` never returns to a caller).
    Some(unsafe { (&*process, &*flags, tid) })
}

/// How the architecture returns to ring 3 after a system call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyscallReturn {
    /// Resume here instead of after the `syscall` instruction
    /// ([`set_return_address`]).
    pub rip: Option<u64>,
    /// Every user register was replaced ([`set_registers`]): restore them
    /// all, and take only `rax` from the system call's registers.
    pub restore_all: bool,
}

/// A system call from the running user thread, called by the architecture's
/// entry with interrupts on. Runs the thread's hook, then ends the thread if
/// the hook asked for that ([`request_exit`]) or its process is being ended,
/// and waits while the process is stopped. Says how to return to ring 3.
pub fn syscall(regs: &mut SyscallRegs) -> SyscallReturn {
    let hook = thread::with_current_user(|u| {
        u.replaced = false;
        u.hook
    })
    .expect("system call from a kernel thread");
    let (process, _, _) = current().expect("system call from a kernel thread");
    hook(process, regs);
    let (rip, restore_all, exit) =
        thread::with_current_user(|u| (u.return_to.take(), u.replaced, u.exit.take())).expect("still a user thread");
    if let Some(status) = exit {
        end_thread(status, exit_code(status));
    }
    leave_if_ended(process);
    SyscallReturn { rip, restore_all }
}

/// The running thread's flags, borrowed for as long as it runs.
fn current_thread_flags() -> Option<&'static ThreadFlags> {
    current().map(|(_, flags, _)| flags)
}

/// The running thread is about to return to ring 3 after an interrupt or
/// exception there, with interrupts off (called by the architecture on the
/// way out; does nothing for kernel threads). Ends the thread if its process
/// is being ended, waits while the process is stopped, and lets the
/// personality act on signals the thread was notified of.
pub fn return_to_user() {
    let Some((process, flags, _)) = current() else { return };
    let notified = flags.notified.load(Ordering::SeqCst);
    if !notified && !process.is_exiting() && !process.is_stopped() && !flags.killed.load(Ordering::SeqCst) {
        return;
    }
    // SAFETY: the thread came from ring 3, so it holds no lock and no
    // `InterruptsOff`; its trap frame is complete. Leaving the process,
    // waiting while it is stopped and writing a signal frame may block.
    unsafe { arch::cpu::enable_interrupts() };
    leave_if_ended(process);
    if notified && process.linux.is_some() && crate::linux::exit_to_user(process) == crate::linux::Leave::Killed {
        end_thread(ExitStatus::Exited(0), 0);
    }
    leave_if_ended(process);
    arch::cpu::disable_interrupts();
}

/// Wait while `process` is stopped; end the running thread if the process
/// is being ended (its status is the one the group exit chose).
fn leave_if_ended(process: &Process) {
    if process.is_stopped() && !process.is_exiting() {
        process.wait_while_stopped();
    }
    let killed = current_thread_flags().is_some_and(|f| f.killed.load(Ordering::SeqCst));
    if process.is_exiting() || killed {
        end_thread(ExitStatus::Exited(0), 0);
    }
}

/// End the running user thread with `status` once its current system call
/// returns, instead of returning to user mode. Fails on a kernel thread.
pub fn request_exit(status: ExitStatus) -> Result<(), ()> {
    thread::with_current_user(|u| u.exit = Some(status)).ok_or(())
}

/// Replace every user register of the running thread with `regs` (a signal
/// handler's entry, `rt_sigreturn`): it returns to ring 3 with exactly
/// these. Only while it is in the kernel from ring 3 (a system call, an
/// interrupt or an exception there). Fails on a kernel thread.
pub fn set_registers(regs: &UserRegs) -> Result<(), ()> {
    thread::with_current_user(|u| u.replaced = true).ok_or(())?;
    arch::user::set_user_regs(regs);
    Ok(())
}

/// The running user thread's registers as it would return to ring 3 now,
/// while it is in the kernel from ring 3. During a system call `rax` still
/// holds the call number and `rcx` / `r11` the return address and flags.
pub fn registers() -> UserRegs {
    arch::user::user_regs()
}

/// The running user thread's FPU / vector state, saved now: an XSAVE image
/// (`Some(XCR0)`) or an FXSAVE image (`None`). `None` on a kernel thread.
pub fn fpu_image() -> Option<(alloc::vec::Vec<u8>, Option<u64>)> {
    // Allocated before the thread table is locked.
    let mut image = alloc::vec![0u8; arch::user::fpu_size()];
    let features = thread::with_current_user(|u| u.arch.fpu_image(&mut image))?;
    Some((image, features))
}

/// Load the running user thread's FPU state from `image` (an FXSAVE image,
/// or an XSAVE image in the format [`fpu_image`] gives). Refused if loading
/// it would fault, or on a kernel thread.
pub fn load_fpu(image: &[u8]) -> Result<(), arch::user::BadImage> {
    thread::with_current_user(|u| u.arch.load_fpu(image)).unwrap_or(Err(arch::user::BadImage))
}

/// Give the running user thread the initial FPU state.
pub fn reset_fpu() {
    thread::with_current_user(|u| u.arch.reset_fpu());
}

/// End the running user thread: its personality's thread-exit duties
/// (robust futexes, `clear_child_tid`), then it leaves its process (the
/// last thread ends the process with `status`, unless a group exit chose
/// another) and exits with `code` (what [`thread::join`] returns). Needs
/// interrupts on and no lock held, as after a system call or a fault in
/// ring 3.
pub fn end_thread(status: ExitStatus, code: usize) -> ! {
    let (process, _, tid) = current().expect("end_thread on a kernel thread");
    if process.linux.is_some() {
        crate::linux::thread_exit(process, tid);
    }
    if let Some(status) = process.detach_thread(tid, status)
        && process.linux.is_some()
    {
        crate::linux::process_exit(process, status);
    }
    thread::exit(code)
}

/// Make `memory` the running thread's memory and load its address space
/// (`execve`). Returns the old memory and address space, which the caller
/// drops (that may sleep: page-cache pages, files). Only for a thread with
/// a memory map that is alone in its process.
pub fn replace_memory(memory: Arc<Memory>) -> Option<(Option<Arc<Memory>>, Arc<AddressSpace>)> {
    let space = Arc::clone(memory.space());
    thread::with_current_user(|u| {
        let old_space = u.arch.replace_space(space);
        (u.memory.replace(memory), old_space)
    })
}

/// Give the running user thread the id `tid` in its process (an `execve`
/// by a thread other than the main one, which takes over the pid).
pub fn set_current_tid(tid: u32) {
    thread::with_current_user(|u| u.tid = tid);
}

/// Make the running user thread resume ring 3 at `rip` when the current
/// system call returns. An address outside the lower half makes the return
/// fail, which kills the thread. Fails on a kernel thread.
pub fn set_return_address(rip: u64) -> Result<(), ()> {
    thread::with_current_user(|u| u.return_to = Some(rip)).ok_or(())
}

/// End the running user thread with `code` (what [`thread::join`] returns;
/// its process sees the low 8 bits as the exit status).
pub fn exit(code: usize) -> ! {
    end_thread(ExitStatus::Exited(code as u8), code)
}

/// A CPU exception the running user thread caused in ring 3 that demand
/// paging did not resolve, called with interrupts on. In a process with
/// personality state it becomes a signal: this returns if the thread goes
/// on (into a handler), and otherwise the process is killed. In a bare
/// process the thread is killed ([`kill_current`]).
pub fn fault(fault: &Fault) {
    let Some((process, _, _)) = current() else { panic!("{fault} in a kernel thread") };
    if process.linux.is_some() {
        match crate::linux::fault(process, fault) {
            crate::linux::Leave::Return => return,
            crate::linux::Leave::Killed => end_thread(ExitStatus::Exited(0), 0),
        }
    }
    kill_current(fault)
}

/// Kill the running user thread for `fault`, a CPU exception it caused in
/// ring 3. Logs one line and never returns. Needs interrupts on.
pub fn kill_current(fault: &Fault) -> ! {
    let Some((process, _, _)) = current() else { panic!("{fault} in a kernel thread") };
    kprintln!("user: pid {} thread {} killed: {fault}", process.pid, thread::current());
    if process.linux.is_some() {
        // A fault that did not become a signal (a failed return to ring 3):
        // the process dies as of an unhandled SIGSEGV (signal(7)).
        let status = ExitStatus::Signaled { signal: crate::linux::SIGSEGV, core: false };
        process.group_exit(status);
        end_thread(status, exit_code(status));
    }
    end_thread(ExitStatus::Faulted(fault.vector), killed(fault.vector))
}

/// The process of the running thread, if it is a user thread.
pub fn current_process() -> Option<Arc<Process>> {
    thread::with_current_user(|u| Arc::clone(&u.process))
}

/// The running user thread's system call hook (what its `clone`
/// children get).
pub fn current_hook() -> Option<SyscallHook> {
    thread::with_current_user(|u| u.hook)
}

/// The running user thread's id in its process.
pub fn current_tid() -> Option<u32> {
    thread::with_current_user(|u| u.tid)
}

/// The running user thread's flags (signal notification, unpark).
pub fn current_flags() -> Option<Arc<ThreadFlags>> {
    thread::with_current_user(|u| Arc::clone(&u.flags))
}

/// The address space of the running thread, if it is a user thread.
pub fn current_address_space() -> Option<Arc<AddressSpace>> {
    thread::with_current_user(|u| Arc::clone(u.arch.space()))
}

/// The memory map of the running thread, if it is a user thread started
/// with [`spawn_in`]. The system calls that manage memory (`mmap`, `brk`,
/// ...) work on this.
pub fn current_memory() -> Option<Arc<Memory>> {
    thread::with_current_user(|u| u.memory.clone()).flatten()
}

/// A page fault at the user address `addr` in the running thread, from
/// ring 3 or from a user-copy routine, with interrupts on: resolve it
/// against the thread's memory map ([`Memory::fault`]). On `Ok` the
/// faulting instruction is retried; an error kills the thread, or makes the
/// copy return [`Efault`].
pub fn page_fault(addr: u64, access: memory::Access) -> Result<(), FaultError> {
    let memory = current_memory().ok_or(FaultError::NoMemoryMap)?;
    memory.fault(addr, access)
}
