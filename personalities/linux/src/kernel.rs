//! What the Linux personality needs from the kernel: the [`LinuxKernel`]
//! trait and the types it speaks in.
//!
//! The personality decodes arguments, validates them the way the man pages
//! describe and encodes results; the kernel does the work with its shared
//! subsystems (VFS, VMA manager, scheduler, clocks). One `LinuxKernel` value
//! stands for **the calling thread** for the duration of one system call.
//!
//! The trait is object safe: handlers take `&mut dyn LinuxKernel`, so the
//! syscall table is a plain array of function pointers and the kernel can
//! hand in whatever type it likes. See ADR 0025.

use crate::abi::{FileStat, SigAction, Timespec, signal};
use crate::errno::Errno;
use crate::fileio::FileIo;
use crate::futex::{FutexKey, FutexTable};
use crate::signal::LinuxProcess;
use alloc::vec::Vec;

/// A user-space address as the caller passed it. It is never dereferenced
/// by the personality: all access goes through [`LinuxKernel::copy_from_user`]
/// and [`LinuxKernel::copy_to_user`], which fail with `EFAULT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserPtr(pub u64);

impl UserPtr {
    pub const NULL: UserPtr = UserPtr(0);

    pub const fn addr(self) -> u64 {
        self.0
    }

    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    /// The address `bytes` further on; `None` if it wraps around.
    pub const fn offset(self, bytes: u64) -> Option<UserPtr> {
        match self.0.checked_add(bytes) {
            Some(a) => Some(UserPtr(a)),
            None => None,
        }
    }
}

/// The real and effective user and group ids of the calling process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Credentials {
    pub uid: u32,
    pub euid: u32,
    pub gid: u32,
    pub egid: u32,
}

/// Page protection, as validated `PROT_*` bits ([`crate::abi::prot`]). Only
/// `PROT_READ`, `PROT_WRITE`, `PROT_EXEC` and `PROT_SEM` (no effect on
/// x86-64) reach the kernel through `mmap`; `mprotect` may also pass one of
/// `PROT_GROWSDOWN` / `PROT_GROWSUP`, never both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prot(pub u32);

impl Prot {
    pub const NONE: Prot = Prot(0);

    pub const fn readable(self) -> bool {
        self.0 & crate::abi::prot::PROT_READ != 0
    }

    pub const fn writable(self) -> bool {
        self.0 & crate::abi::prot::PROT_WRITE != 0
    }

    pub const fn executable(self) -> bool {
        self.0 & crate::abi::prot::PROT_EXEC != 0
    }

    /// `PROT_GROWSDOWN`: extend the change down to the start of a
    /// grows-down mapping (mprotect only).
    pub const fn grows_down(self) -> bool {
        self.0 & crate::abi::prot::PROT_GROWSDOWN != 0
    }

    /// `PROT_GROWSUP` (mprotect only; x86-64 has no grows-up mappings).
    pub const fn grows_up(self) -> bool {
        self.0 & crate::abi::prot::PROT_GROWSUP != 0
    }
}

/// Where a new mapping goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// The kernel chooses; `addr` of the request is a hint (possibly 0) it
    /// may use if that range is free. Never returns address 0.
    Hint,
    /// `MAP_FIXED`: exactly at `addr` (page aligned), replacing whatever is
    /// mapped there. Fails if the range is not usable user space.
    Fixed,
    /// `MAP_FIXED_NOREPLACE`: exactly at `addr`, failing with `EEXIST` if
    /// anything is mapped in the range.
    FixedNoReplace,
}

/// What backs a new mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backing {
    /// Zero-filled memory (`MAP_ANONYMOUS`).
    Anonymous,
    /// The file open as `fd` (not negative), from byte `offset` (page
    /// aligned; `offset + len` does not overflow).
    File { fd: i32, offset: u64 },
}

/// Hints that do not change what a mapping is, only how the kernel may
/// treat it. Each one may be ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapHints {
    /// `MAP_32BIT`: place it in the first 2 GiB (only set with `Placement::Hint`).
    pub low_2g: bool,
    /// `MAP_GROWSDOWN`: a stack that grows down on faults below it.
    pub grows_down: bool,
    /// `MAP_STACK`: a thread stack (a no-op on Linux).
    pub stack: bool,
    /// `MAP_POPULATE`: prefault the pages now (failure is not an error).
    pub populate: bool,
    /// `MAP_LOCKED`: like mlock(2), best effort.
    pub locked: bool,
    /// `MAP_NORESERVE`: do not reserve swap / commit charge.
    pub no_reserve: bool,
}

/// A validated `mmap` request. The personality has already checked the
/// flags, the alignment of `addr` (for fixed placements) and of the file
/// offset, and rounded `len` up to whole pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRequest {
    /// Exact address (fixed placements, page aligned) or hint (may be 0 or
    /// unaligned).
    pub addr: u64,
    /// Length in bytes: a non-zero multiple of the page size.
    pub len: u64,
    pub prot: Prot,
    pub placement: Placement,
    /// `MAP_SHARED` (or `MAP_SHARED_VALIDATE`) rather than `MAP_PRIVATE`.
    pub shared: bool,
    pub backing: Backing,
    pub hints: MapHints,
}

/// The clocks `clock_gettime` reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// Wall-clock time since the Unix epoch (`CLOCK_REALTIME`).
    Realtime,
    /// Time since an unspecified start that never goes backwards and is not
    /// affected by setting the wall clock (`CLOCK_MONOTONIC`).
    Monotonic,
    /// CPU time used by all threads of the calling process.
    ProcessCpuTime,
    /// CPU time used by the calling thread.
    ThreadCpuTime,
}

/// Whose CPU time [`LinuxKernel::cpu_times`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuTimesOf {
    /// Thread `tid` of the calling process.
    Thread(i32),
    /// Every thread of the calling process, those that have ended too.
    Process,
    /// The calling process's children that have ended and been waited for
    /// (getrusage(2) `RUSAGE_CHILDREN`, times(2) `tms_cutime`).
    Children,
}

/// CPU time in nanoseconds, split into time running user code and time
/// in the kernel on its behalf.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub user_ns: u64,
    pub system_ns: u64,
}

impl CpuTimes {
    pub const fn total_ns(self) -> u64 {
        self.user_ns.saturating_add(self.system_ns)
    }
}

/// How `getrandom` may wait for the random source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomMode {
    /// Block until the source is initialised.
    Block,
    /// `GRND_NONBLOCK`: fail with `EAGAIN` instead of blocking.
    NonBlock,
    /// `GRND_INSECURE`: never block; the bytes need not be
    /// cryptographically strong before the source is initialised.
    Insecure,
}

/// A signal set as the kernel stores it: bit `n - 1` stands for signal `n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigSet(pub u64);

impl SigSet {
    pub const EMPTY: SigSet = SigSet(0);
    pub const FULL: SigSet = SigSet(u64::MAX);

    /// The set with only `signal` (1-64).
    pub const fn only(signal: u32) -> SigSet {
        SigSet(1 << (signal - 1))
    }

    pub const fn contains(self, signal: u32) -> bool {
        signal >= 1 && signal <= signal::NSIG && self.0 & (1 << (signal - 1)) != 0
    }

    /// The signals that can never be blocked: `SIGKILL` and `SIGSTOP`.
    pub const UNBLOCKABLE: SigSet = SigSet((1 << (signal::SIGKILL - 1)) | (1 << (signal::SIGSTOP - 1)));

    /// The standard signals (1-31); the rest are real-time signals.
    pub const STANDARD: SigSet = SigSet((1 << (signal::SIGRTMIN - 1)) - 1);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn union(self, other: SigSet) -> SigSet {
        SigSet(self.0 | other.0)
    }

    pub const fn intersection(self, other: SigSet) -> SigSet {
        SigSet(self.0 & other.0)
    }

    /// The signals of `self` that are not in `other`.
    pub const fn without(self, other: SigSet) -> SigSet {
        SigSet(self.0 & !other.0)
    }

    /// Add `signal` (1-64).
    pub fn insert(&mut self, signal: u32) {
        self.0 |= SigSet::only(signal).0;
    }

    /// Remove `signal` (1-64).
    pub fn remove(&mut self, signal: u32) {
        self.0 &= !SigSet::only(signal).0;
    }

    /// The lowest-numbered signal in the set.
    pub const fn lowest(self) -> Option<u32> {
        if self.0 == 0 { None } else { Some(self.0.trailing_zeros() + 1) }
    }

    /// The signals in the set, lowest first.
    pub fn iter(self) -> impl Iterator<Item = u32> {
        (1..=signal::NSIG).filter(move |&s| self.contains(s))
    }
}

/// The disposition of every signal of a process, as `rt_sigaction` sets and
/// reports it. Part of [`LinuxProcess`] (threads share it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalActions {
    actions: [SigAction; signal::NSIG as usize],
}

impl Default for SignalActions {
    fn default() -> Self {
        SignalActions { actions: [SigAction::default(); signal::NSIG as usize] }
    }
}

impl SignalActions {
    /// Every signal at `SIG_DFL` with an empty mask and no flags.
    pub fn new() -> Self {
        Self::default()
    }

    /// The action for `signal` (1-64).
    pub fn get(&self, signal: u32) -> SigAction {
        self.actions[signal as usize - 1]
    }

    /// Replace the action for `signal` (1-64).
    pub fn set(&mut self, signal: u32, action: SigAction) {
        self.actions[signal as usize - 1] = action;
    }
}

/// The user registers of a thread, as it will see them when it returns to
/// user mode: what a signal frame saves (`struct sigcontext`) and what
/// rt_sigreturn(2) restores.
///
/// `trapno`, `err` and `cr2` describe the exception that caused a signal
/// (the vector, its error code and, for page faults, the address); the
/// kernel sets them for fault signals and leaves them 0 otherwise. During
/// a system call `rcx` and `r11` hold what the `syscall` instruction put
/// there (the return address and flags).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserContext {
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
    /// The user code and stack segment selectors.
    pub cs: u16,
    pub ss: u16,
    pub trapno: u64,
    pub err: u64,
    pub cr2: u64,
}

/// The calling thread's floating-point and vector state, as the CPU saves
/// it: an `FXSAVE` image (512 bytes) or, on CPUs with `XSAVE`, an XSAVE
/// image (the 512-byte legacy area, the 64-byte header, the extended
/// components) with the feature bits saved in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FpuState<'a> {
    pub image: &'a [u8],
    /// `Some(features)` for an XSAVE image, `None` for `FXSAVE`.
    pub xfeatures: Option<u64>,
}

/// Which process [`LinuxKernel::with_process`] acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The calling thread's process.
    Current,
    /// The process (thread group) with this id.
    Process(i32),
    /// The process that has a thread with this thread id.
    Thread(i32),
}

/// What the kernel tells the personality about a process: ids and
/// credentials for the permission checks of kill(2) and
/// get_robust_list(2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessInfo {
    /// The thread group id.
    pub pid: i32,
    pub pgid: i32,
    /// The session id.
    pub sid: i32,
    /// Real, effective and saved user ids.
    pub uid: u32,
    pub euid: u32,
    pub suid: u32,
    /// Real, effective and saved group ids.
    pub gid: u32,
    pub egid: u32,
    pub sgid: u32,
    /// The process has ended but has not been waited for. Signals to it
    /// succeed and do nothing (kill(2): "an existing process might be a
    /// zombie").
    pub zombie: bool,
}

/// Which processes [`LinuxKernel::process_ids`] lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessSelector {
    /// Every process the caller can see (kill(-1, sig)).
    All,
    /// The members of a process group.
    Group(i32),
}

/// Why [`LinuxKernel::block`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wakeup {
    /// [`LinuxKernel::unblock`] was called for the thread (perhaps before
    /// it blocked, or for an earlier wait: callers check their condition).
    Woken,
    /// The deadline passed.
    TimedOut,
    /// The thread's signal notification is set
    /// ([`LinuxKernel::notify_signal`]).
    Signal,
}

/// An absolute point in time on a clock (`Realtime` or `Monotonic`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    pub clock: Clock,
    pub at: Timespec,
}

/// Kernel services for the Linux personality, implemented by the kernel for
/// the calling thread. Errors are Linux errnos; each method says which ones
/// the man pages expect from it.
pub trait LinuxKernel {
    // ---- identity -----------------------------------------------------

    /// The process id (the thread group id), as `getpid` returns it.
    fn pid(&self) -> i32;
    /// The calling thread's id; equal to [`pid`](Self::pid) for the main thread.
    fn tid(&self) -> i32;
    /// The parent's process id (0 if the parent is not in this pid namespace).
    fn ppid(&self) -> i32;
    /// The process's real and effective user and group ids.
    fn credentials(&self) -> Credentials;

    // ---- user memory --------------------------------------------------

    /// Copy `dst.len()` bytes from user address `src`. Fails with `EFAULT`
    /// if any byte is not mapped readable in the caller's address space (or
    /// the range wraps or leaves user space). Must never touch kernel memory.
    fn copy_from_user(&self, src: UserPtr, dst: &mut [u8]) -> Result<(), Errno>;

    /// Copy `src` to user address `dst`. Fails with `EFAULT` if any byte is
    /// not mapped writable by user space. A failure may leave a prefix written.
    fn copy_to_user(&mut self, dst: UserPtr, src: &[u8]) -> Result<(), Errno>;

    /// Copy a NUL-terminated string from user space into `dst`, returning
    /// its length without the NUL. `ENAMETOOLONG` if `dst` fills up before a
    /// NUL is found, `EFAULT` if a byte before the NUL cannot be read.
    ///
    /// The default reads up to each page boundary with `copy_from_user`; a
    /// kernel with fault-recovering string copies can do better.
    fn strncpy_from_user(&self, src: UserPtr, dst: &mut [u8]) -> Result<usize, Errno> {
        let mut done = 0;
        while done < dst.len() {
            let at = src.offset(done as u64).ok_or(Errno::EFAULT)?;
            let to_page_end = (crate::abi::PAGE_SIZE - at.0 % crate::abi::PAGE_SIZE) as usize;
            let n = to_page_end.min(dst.len() - done);
            self.copy_from_user(at, &mut dst[done..done + n])?;
            if let Some(nul) = dst[done..done + n].iter().position(|&b| b == 0) {
                return Ok(done + nul);
            }
            done += n;
        }
        Err(Errno::ENAMETOOLONG)
    }

    // ---- files (delegated to the VFS) ---------------------------------

    /// Write `buf` to the open file `fd` at its current offset (or as the
    /// file type decides). Returns the bytes written, which may be fewer
    /// than asked. `buf` may be empty: then only check `fd` (write(2): a
    /// zero-length write returns 0 or an error). Errors: `EBADF` (closed, or
    /// not open for writing), `EPIPE`, `ENOSPC`, `EAGAIN`, `EINVAL`, ...
    fn write(&mut self, fd: i32, buf: &[u8]) -> Result<usize, Errno>;

    /// Read up to `buf.len()` bytes from `fd`. Returns the bytes read; 0 at
    /// end of file. Errors: `EBADF`, `EISDIR`, `EAGAIN`, `EINVAL`, ...
    fn read(&mut self, fd: i32, buf: &mut [u8]) -> Result<usize, Errno>;

    /// Close `fd`. `EBADF` if it is not open.
    fn close(&mut self, fd: i32) -> Result<(), Errno>;

    /// Describe the open file `fd`. `EBADF` if it is not open.
    fn fstat(&mut self, fd: i32) -> Result<FileStat, Errno>;

    /// A device-specific ioctl on `fd`, already known to be open. The
    /// personality calls this only for requests it does not answer itself
    /// (terminal requests on files that are not character devices get
    /// `ENOTTY` without reaching here). Return `ENOTTY` for requests the
    /// file does not support and `EFAULT` if `arg` cannot be accessed.
    fn ioctl(&mut self, fd: i32, request: u32, arg: u64) -> Result<u64, Errno>;

    // ---- memory (delegated to the VMA manager) ------------------------

    /// Create a mapping and return its start address. Errors: `ENOMEM` (no
    /// room, too many mappings, beyond user space), `EEXIST`
    /// (`FixedNoReplace` collides), `EBADF` (fd not open), `EACCES` (file
    /// not open for reading, or shared writable without write access),
    /// `ENODEV` (file cannot be mapped), `EPERM` (exec on a noexec mount,
    /// or below the lowest mappable address with a fixed placement).
    fn mmap(&mut self, request: &MapRequest) -> Result<u64, Errno>;

    /// Remove all mappings in `[addr, addr + len)`; both page aligned, `len`
    /// non-zero, no overflow. Unmapped holes are not an error. `EINVAL` if
    /// the range is outside user space, `ENOMEM` if splitting a mapping
    /// exceeds the mapping limit.
    fn munmap(&mut self, addr: u64, len: u64) -> Result<(), Errno>;

    /// Change the protection of `[addr, addr + len)`; both page aligned,
    /// `len` non-zero, no overflow. `ENOMEM` if any page in the range is not
    /// mapped (or splitting exceeds the mapping limit), `EACCES` if a file
    /// mapping cannot get the access asked for.
    fn mprotect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Errno>;

    /// The current program break.
    fn brk(&self) -> u64;

    /// Move the program break to `addr` (not rounded: the break is a byte
    /// address; the heap mapping ends at the next page boundary). Returns
    /// the new break, which is `addr`. Fails, leaving the break unchanged,
    /// if `addr` is below the initial break, would collide with another
    /// mapping, or exceeds the data size limit; the personality then
    /// reports the current break, so the error value itself is not seen.
    fn set_brk(&mut self, addr: u64) -> Result<u64, Errno>;

    // ---- thread state -------------------------------------------------

    /// The calling thread's FS segment base.
    fn fs_base(&self) -> u64;
    /// Set the FS base; the value is below [`crate::abi::USER_ADDRESS_LIMIT`].
    /// Takes effect when the thread returns to user mode.
    fn set_fs_base(&mut self, base: u64);
    /// The calling thread's GS segment base.
    fn gs_base(&self) -> u64;
    /// Set the GS base; the value is below [`crate::abi::USER_ADDRESS_LIMIT`].
    fn set_gs_base(&mut self, base: u64);

    // ---- processes, threads and signals (ADR 0031) ------------------

    /// The calling process's process group id.
    fn pgid(&self) -> i32;
    /// The calling process's session id.
    fn sid(&self) -> i32;

    /// Run `f` on the Linux state of a process ([`LinuxProcess`]: signal
    /// dispositions, pending signals, and every thread's mask, alternate
    /// stack, robust list and `clear_child_tid`) together with what the
    /// kernel knows about the process. `ESRCH` if there is no such process
    /// or thread.
    ///
    /// The kernel keeps one `LinuxProcess` per Linux process, creates it
    /// with [`LinuxProcess::new`] (or [`LinuxProcess::fork`]), tells it
    /// about new and ended threads ([`LinuxProcess::add_thread`],
    /// [`LinuxProcess::remove_thread`]) and holds the process's lock while
    /// `f` runs. `f` never blocks and never calls back into the kernel, so
    /// a spinlock will do; the personality never holds two processes'
    /// state at once.
    fn with_process(&mut self, target: Target, f: &mut dyn FnMut(&mut LinuxProcess, &ProcessInfo))
    -> Result<(), Errno>;

    /// The thread group ids of the processes `selector` names, in any
    /// order (empty if there are none). Process 1 and the caller are
    /// included; the personality applies kill(2)'s exceptions.
    fn process_ids(&self, selector: ProcessSelector) -> Vec<i32>;

    /// Thread `tid` has a signal to act on: set its signal notification,
    /// end its [`block`](Self::block) with [`Wakeup::Signal`], and end
    /// its interruptible kernel waits (a pipe read, say), which then
    /// return one of the `Errno::RESTART_*` codes. The notification stays
    /// set until the thread clears it
    /// ([`clear_signal_notification`](Self::clear_signal_notification)).
    /// Unknown threads are ignored.
    fn notify_signal(&mut self, tid: i32);

    /// Clear the calling thread's signal notification.
    fn clear_signal_notification(&mut self);

    /// Stop every thread of the calling process (the default action of
    /// `SIGSTOP`, `SIGTSTP`, `SIGTTIN`, `SIGTTOU`) and tell the parent
    /// (wait(2) `WIFSTOPPED`; `SIGCHLD` with `CLD_STOPPED` unless it set
    /// `SA_NOCLDSTOP`). Returns once the process is continued
    /// ([`continue_process`](Self::continue_process)) or killed.
    fn stop_process(&mut self, signal: u32);

    /// Continue process `pid` if it is stopped (`SIGCONT` was generated
    /// for it, whatever its disposition) and tell its parent
    /// (`CLD_CONTINUED`). Nothing happens if it is not stopped.
    fn continue_process(&mut self, pid: i32);

    /// End every thread of the calling process because of `signal`
    /// (wait(2) `WIFSIGNALED`, `WTERMSIG` = `signal`). `core_dump` says the
    /// default action is "Core"; the kernel reports `WCOREDUMP` only if it
    /// wrote a dump. Like [`exit_group`](Self::exit_group), the thread
    /// must not return to user mode.
    fn kill_process(&mut self, signal: u32, core_dump: bool);

    // ---- user registers (signal frames) ------------------------------

    /// The calling thread's user registers as it would return to user
    /// mode now.
    fn user_context(&self) -> UserContext;

    /// Replace the calling thread's user registers. The thread returns to
    /// user mode with exactly these (with `iret`, since `rcx` and `r11`
    /// matter), not with the `SyscallRegs` the dispatcher was given; the
    /// dispatcher writes only `rax` afterwards, with the same value.
    /// `cs` and `ss` are the current selectors; the kernel keeps
    /// `rflags.IF` set and the reserved bits as they must be.
    fn set_user_context(&mut self, ctx: &UserContext);

    /// The calling thread's user FPU / vector state.
    fn fpu_state(&self) -> FpuState<'_>;

    /// Load the calling thread's FPU state from a signal frame: an XSAVE
    /// image (when the frame says so) or a 512-byte `FXSAVE` image. The
    /// image comes from user memory: `EINVAL` if loading it would fault
    /// (reserved `MXCSR` bits, a bad XSAVE header, features the thread
    /// may not use); the thread then gets `SIGSEGV`.
    fn set_fpu_state(&mut self, image: &[u8]) -> Result<(), Errno>;

    /// Give the calling thread a clean FPU state (as after `FNINIT`, with
    /// the default `MXCSR`), for a signal handler.
    fn reset_fpu_state(&mut self);

    // ---- blocking -----------------------------------------------------

    /// Block the calling thread until [`unblock`](Self::unblock) is
    /// called for it, `deadline` passes, or its signal notification is
    /// set. An `unblock` that came before the call is not lost: the call
    /// returns `Woken` at once and consumes it. Callers re-check what they
    /// wait for, so spurious returns are fine.
    fn block(&mut self, deadline: Option<Deadline>) -> Wakeup;

    /// Wake thread `tid` from [`block`](Self::block), or make its next
    /// `block` return at once. Unknown threads are ignored.
    fn unblock(&mut self, tid: i32);

    // ---- futexes ------------------------------------------------------

    /// Run `f` with the system's futex table locked. `f` gets the kernel
    /// back for atomic user-memory access ([`futex_load`](Self::futex_load),
    /// [`futex_cmpxchg`](Self::futex_cmpxchg)) and
    /// [`unblock`](Self::unblock); it never blocks and never calls
    /// `with_futexes` or `with_process`. One table serves every process
    /// (shared futexes cross address spaces).
    fn with_futexes(&mut self, f: &mut dyn FnMut(&mut FutexTable, &mut dyn LinuxKernel));

    /// An id of the calling thread's address space, the same for every
    /// thread that shares it and different from every other live one.
    fn address_space_id(&self) -> u64;

    /// The key of the futex word at `addr` (4-byte aligned). Private
    /// futexes are keyed by address space and address. A shared futex in
    /// memory other processes can map is keyed by the object and offset,
    /// so every mapping of it finds the same waiters; in private memory it
    /// gets the same key as the private futex. `EFAULT` if `addr` is not
    /// mapped.
    ///
    /// The default treats all memory as private, which is right until the
    /// kernel has shared mappings.
    fn futex_key(&self, addr: UserPtr, private: bool) -> Result<FutexKey, Errno> {
        let _ = private;
        Ok(FutexKey::Private { space: self.address_space_id(), addr: addr.0 })
    }

    /// Read the aligned 32-bit word at `addr` in one access. `EFAULT` if
    /// it is not readable. It may be called with the futex table locked,
    /// so it must not sleep: the personality reads the word once before
    /// locking the table, which is when a kernel that loads pages lazily
    /// can do so.
    fn futex_load(&self, addr: UserPtr) -> Result<u32, Errno> {
        let mut b = [0; 4];
        self.copy_from_user(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// Atomically replace the aligned 32-bit word at `addr` with `new` if
    /// it holds `expected` (`lock cmpxchg`), and return the value found
    /// (equal to `expected` on success). `EFAULT` if the word is not
    /// readable and writable. Same rules as [`futex_load`](Self::futex_load).
    fn futex_cmpxchg(&mut self, addr: UserPtr, expected: u32, new: u32) -> Result<u32, Errno>;

    // ---- exit ---------------------------------------------------------

    /// End the calling thread with exit status `status` (exit(2): the low
    /// 8 bits of the argument). If it is the last thread, the process ends.
    /// The thread must not return to user mode; the kernel switches away
    /// after the dispatcher returns (or never returns from here).
    fn exit_thread(&mut self, status: u8);

    /// End every thread of the calling process with exit status `status`
    /// (exit_group(2)). Same rule: the caller never returns to user mode.
    fn exit_group(&mut self, status: u8);

    // ---- processes: clone, exec, the process tree (ADR 0041) ---------

    /// Create a thread of the calling process (`CLONE_THREAD`) or a new
    /// process as `req` describes ([`crate::task::clone::CloneRequest`]:
    /// what it shares, its stack, TLS and tid pointers) and start it.
    /// A new process gets a copy of the caller's memory (copy on write)
    /// unless it shares it (`CLONE_VM`), copies of its files and signal
    /// state (the personality's [`LinuxProcess::fork`]), and a node in
    /// the process tree under the caller (or the caller's parent with
    /// `CLONE_PARENT`) with `req.exit_signal`. With `CLONE_VFORK` the call
    /// returns once the child has exec'd or ended. Returns the child's
    /// thread id (its pid for a process). Errors: `EAGAIN` (no room for
    /// another thread), `ENOMEM`. The default: `ENOSYS`.
    fn clone_task(&mut self, req: &crate::task::clone::CloneRequest) -> Result<i32, Errno> {
        let _ = req;
        Err(Errno::ENOSYS)
    }

    /// Replace the calling process's program (execve(2)): open
    /// `req.path`, run `#!` scripts through their interpreter
    /// ([`crate::task::exec::parse_shebang`]), load the executable, and if
    /// that worked end every other thread, replace the memory, close the
    /// close-on-exec descriptors, reset the signal state
    /// ([`LinuxProcess::exec`]), FPU state and FS/GS bases, release a
    /// vfork parent, and set the registers for the new program's entry
    /// (with [`set_user_context`](Self::set_user_context) semantics: the
    /// return restores every register). Errors before that point leave
    /// the process unchanged: `ENOENT`, `EACCES`, `ENOEXEC`, `E2BIG`,
    /// `ELOOP`, `ENOMEM`, ... The default: `ENOSYS`.
    fn execve(&mut self, req: &crate::task::exec::ExecRequest<'_>) -> Result<(), Errno> {
        let _ = req;
        Err(Errno::ENOSYS)
    }

    /// Run `f` with the system's process tree locked
    /// ([`crate::task::tree::ProcessTree`]); `f` never blocks and never
    /// calls back into the kernel. The kernel lets go of the records of
    /// the processes `f` reaped
    /// ([`take_reaped`](crate::task::tree::ProcessTree::take_reaped)) and wakes the
    /// threads in [`wait_tree`](Self::wait_tree) when the generation
    /// moved. The default: `ENOSYS`.
    fn with_tree(&mut self, f: &mut dyn FnMut(&mut crate::task::tree::ProcessTree)) -> Result<(), Errno> {
        let _ = f;
        Err(Errno::ENOSYS)
    }

    /// Block until the process tree's generation is no longer
    /// `generation` ([`Wakeup::Woken`]) or the thread's signal
    /// notification is set ([`Wakeup::Signal`]). The default returns
    /// [`Wakeup::Signal`] at once.
    fn wait_tree(&mut self, generation: u64) -> Wakeup {
        let _ = generation;
        Wakeup::Signal
    }

    // ---- time, randomness, system identity ----------------------------

    /// Read `clock`. `EINVAL` if the kernel does not support it (for now
    /// acceptable for the CPU-time clocks).
    fn clock(&self, clock: Clock) -> Result<Timespec, Errno>;

    /// Fill a prefix of `buf` with random bytes; return how many (at least
    /// 1 unless `buf` is empty). `EAGAIN` in [`RandomMode::NonBlock`] while
    /// the source is not initialised, `EINTR` if a blocking wait is
    /// interrupted.
    fn random_bytes(&mut self, buf: &mut [u8], mode: RandomMode) -> Result<usize, Errno>;

    /// Set the wall clock (`CLOCK_REALTIME`) to `at`, a valid time the
    /// personality has checked the caller may set. `EINVAL` if it is below
    /// the monotonic clock (clock_settime(2)). The monotonic clocks do not
    /// move.
    fn set_realtime(&mut self, at: Timespec) -> Result<(), Errno> {
        let _ = at;
        Err(Errno::EPERM)
    }

    /// The time zone `settimeofday` stored, as `(tz_minuteswest,
    /// tz_dsttime)` (`(0, 0)` until one is set).
    fn timezone(&self) -> (i32, i32) {
        (0, 0)
    }

    /// Store `settimeofday`'s time zone, system-wide.
    fn set_timezone(&mut self, minuteswest: i32, dsttime: i32) {
        let _ = (minuteswest, dsttime);
    }

    /// The period of the kernel's timer tick in nanoseconds: the
    /// resolution of the coarse clocks.
    fn tick_ns(&self) -> u64 {
        1_000_000
    }

    /// CPU time used so far; `None` if the kernel does not account it (or,
    /// for [`CpuTimesOf::Thread`], there is no such thread in the calling
    /// process).
    fn cpu_times(&self, of: CpuTimesOf) -> Option<CpuTimes> {
        let _ = of;
        None
    }

    /// The calling process's timers changed (setitimer, alarm,
    /// timer_settime, ...): `next` says when they must next be looked at.
    /// When that time comes the kernel calls [`crate::time::fire`] on the
    /// process's state, with its lock held, and acts on the result (ADR
    /// 0044).
    fn timers_changed(&mut self, next: crate::time::TimerWakeup) {
        let _ = next;
    }

    /// The host name (`uname`'s `nodename`); at most 64 bytes are used.
    fn hostname(&self) -> &[u8] {
        b"poly"
    }

    /// The NIS domain name (`uname`'s `domainname`). Linux reports
    /// `(none)` until one is set.
    fn domainname(&self) -> &[u8] {
        b"(none)"
    }

    // ---- file systems and system information (ADR 0043) ---------------

    /// The file system services for the calling thread's process; `None`
    /// (the default) makes the file system calls fail with `ENOSYS`.
    fn fs(&mut self) -> Option<&mut dyn crate::fs::LinuxFs> {
        None
    }

    /// What sysinfo(2) reports. `ENOSYS` by default.
    fn system_info(&mut self) -> Result<crate::fs::SysInfo, Errno> {
        Err(Errno::ENOSYS)
    }

    // ---- pipes, readiness waits and event files (ADR 0042) -----------

    /// The kernel's pipes, `poll` / `epoll` waits and event files
    /// ([`FileIo`]); `None` if it has none, and those calls fail with
    /// `ENOSYS`.
    fn file_io(&mut self) -> Option<&mut dyn FileIo> {
        None
    }

    // ---- sockets (ADR 0048) ---------------------------------------------

    /// The kernel's sockets ([`crate::net::LinuxNet`]); `None` (the
    /// default) makes the socket calls fail with `ENOSYS`.
    fn net(&mut self) -> Option<&mut dyn crate::net::LinuxNet> {
        None
    }

    /// A signal was queued for process `pid` or one of its threads,
    /// blocked or not: wake the threads waiting to read signals through
    /// a signal file (signalfd(2)).
    fn signal_queued(&mut self, pid: i32) {
        let _ = pid;
    }
}
