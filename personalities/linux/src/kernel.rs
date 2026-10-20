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

    /// The set with only `signal` (1-64).
    pub const fn only(signal: u32) -> SigSet {
        SigSet(1 << (signal - 1))
    }

    pub const fn contains(self, signal: u32) -> bool {
        signal >= 1 && signal <= signal::NSIG && self.0 & (1 << (signal - 1)) != 0
    }

    /// The signals that can never be blocked: `SIGKILL` and `SIGSTOP`.
    pub const UNBLOCKABLE: SigSet = SigSet((1 << (signal::SIGKILL - 1)) | (1 << (signal::SIGSTOP - 1)));
}

/// The disposition of every signal of a process, as `rt_sigaction` sets and
/// reports it. The kernel keeps one per process (threads share it) and
/// hands it out through [`LinuxKernel::signal_actions`]; the personality
/// does all the reading and writing. Nothing delivers signals yet.
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

    /// Store the thread's `clear_child_tid` address (set_tid_address(2)).
    /// When the thread exits and the address is not null, the kernel writes
    /// 0 there and wakes one futex waiter on it (once futexes exist).
    fn set_clear_child_tid(&mut self, addr: UserPtr);

    /// The calling thread's blocked-signal mask (per thread).
    fn signal_mask(&mut self) -> &mut SigSet;

    /// The calling process's signal dispositions (per process).
    fn signal_actions(&mut self) -> &mut SignalActions;

    // ---- exit ---------------------------------------------------------

    /// End the calling thread with exit status `status` (exit(2): the low
    /// 8 bits of the argument). If it is the last thread, the process ends.
    /// The thread must not return to user mode; the kernel switches away
    /// after the dispatcher returns (or never returns from here).
    fn exit_thread(&mut self, status: u8);

    /// End every thread of the calling process with exit status `status`
    /// (exit_group(2)). Same rule: the caller never returns to user mode.
    fn exit_group(&mut self, status: u8);

    // ---- time, randomness, system identity ----------------------------

    /// Read `clock`. `EINVAL` if the kernel does not support it (for now
    /// acceptable for the CPU-time clocks).
    fn clock(&self, clock: Clock) -> Result<Timespec, Errno>;

    /// Fill a prefix of `buf` with random bytes; return how many (at least
    /// 1 unless `buf` is empty). `EAGAIN` in [`RandomMode::NonBlock`] while
    /// the source is not initialised, `EINTR` if a blocking wait is
    /// interrupted.
    fn random_bytes(&mut self, buf: &mut [u8], mode: RandomMode) -> Result<usize, Errno>;

    /// The host name (`uname`'s `nodename`); at most 64 bytes are used.
    fn hostname(&self) -> &[u8] {
        b"poly"
    }

    /// The NIS domain name (`uname`'s `domainname`). Linux reports
    /// `(none)` until one is set.
    fn domainname(&self) -> &[u8] {
        b"(none)"
    }
}
