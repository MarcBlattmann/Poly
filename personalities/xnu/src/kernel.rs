//! What the XNU personality needs from the kernel: the [`XnuKernel`] trait.
//!
//! As for the Linux personality (ADR 0025), one `XnuKernel` value stands for
//! **the calling thread** for one system call. The personality decodes and
//! validates arguments the way the spec sheets record and keeps the Mach
//! state ([`MachState`], from `poly-mach`); the kernel copies user memory,
//! owns the address space and the page tables, and blocks and wakes threads.

use alloc::vec::Vec;
use poly_mach::{Ipc, PsynchTable, SenderInfo, TaskId, ThreadId, UlockTable, ulock};

/// The Mach state of the machine, kept by the kernel behind one lock.
#[derive(Debug, Default)]
pub struct MachState {
    pub ipc: Ipc,
    pub ulock: UlockTable,
    pub psynch: PsynchTable,
}

impl MachState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The architecture of the calling program: x86-64 natively, arm64 through
/// the translator (ADR 0151). It decides the `mach_absolute_time` unit and
/// whether memory may be writable and executable at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Arm64,
}

impl Arch {
    /// `mach_timebase_info`: numerator and denominator of absolute time
    /// units to nanoseconds. arm64: the 24 MHz counter, 125/3 (the spec
    /// sheet's oracle); x86-64: nanoseconds, 1/1.
    pub const fn timebase(self) -> (u32, u32) {
        match self {
            Arch::X86_64 => (1, 1),
            Arch::Arm64 => (125, 3),
        }
    }

    /// Absolute time units to nanoseconds.
    pub fn abs_to_ns(self, t: u64) -> u64 {
        let (n, d) = self.timebase();
        u64::try_from(u128::from(t) * u128::from(n) / u128::from(d)).unwrap_or(u64::MAX)
    }

    /// Nanoseconds to absolute time units.
    pub fn ns_to_abs(self, ns: u64) -> u64 {
        let (n, d) = self.timebase();
        u64::try_from(u128::from(ns) * u128::from(d) / u128::from(n)).unwrap_or(u64::MAX)
    }
}

/// How a block ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wakeup {
    /// [`XnuKernel::unblock`] (possibly an old one: check the wait's result).
    Woken,
    TimedOut,
    /// A signal is pending for the thread.
    Signal,
}

/// A user address that cannot be accessed (not mapped, or not with the
/// access asked for).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault;

/// What backs a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backing {
    Anonymous {
        shared: bool,
    },
    /// A file; `writable`: the descriptor it was mapped from was open for
    /// writing.
    File {
        shared: bool,
        writable: bool,
    },
}

/// One mapped region of the caller's address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub start: u64,
    pub end: u64,
    /// `VM_PROT_*` bits (the same values as `PROT_*`).
    pub prot: u32,
    pub max_prot: u32,
    pub backing: Backing,
}

/// `vm_inherit_t`: what a fork child gets of a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inherit {
    Share,
    Copy,
    None,
}

/// What mincore reports about one page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageInfo {
    pub resident: bool,
    pub referenced: bool,
    pub modified: bool,
}

/// A thread for bsdthread_create to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadStart {
    /// The function and argument the program gave.
    pub func: u64,
    pub arg: u64,
    /// The stack the program made, and its thread structure.
    pub stack: u64,
    pub pthread: u64,
    pub flags: u32,
}

pub trait XnuKernel {
    fn task(&self) -> TaskId;
    fn thread(&self) -> ThreadId;
    fn arch(&self) -> Arch;
    fn page_size(&self) -> u64;

    /// Copy from user memory, faulting pages in; `Err` for an address that
    /// is not mapped readable.
    fn copy_in(&self, addr: u64, buf: &mut [u8]) -> Result<(), Fault>;
    /// Copy to user memory; `Err` for an address that is not mapped writable.
    fn copy_out(&mut self, addr: u64, buf: &[u8]) -> Result<(), Fault>;
    /// Read without faulting a page in: `Ok(false)` when the page is mapped
    /// but not present in this process (a shared page it has not touched
    /// since fork), `Err` when it is not mapped.
    fn load_resident(&self, addr: u64, buf: &mut [u8]) -> Result<bool, Fault>;
    /// The key under which processes sharing the memory at `addr` meet:
    /// the memory object and offset for shared memory, the task and address
    /// for private memory. `Err` if not mapped.
    fn shared_key(&self, addr: u64) -> Result<ulock::Key, Fault>;

    /// Monotonic nanoseconds since boot (`mach_absolute_time` before the
    /// timebase).
    fn now_ns(&self) -> u64;
    /// Block the calling thread until [`XnuKernel::unblock`], the deadline
    /// (monotonic nanoseconds) or a signal.
    fn block(&mut self, deadline_ns: Option<u64>) -> Wakeup;
    fn unblock(&mut self, thread: ThreadId);

    /// Run `f` with the Mach state locked.
    fn with_mach(&mut self, f: &mut dyn FnMut(&mut MachState, &mut dyn XnuKernel));

    /// The sender of a message, for its trailer.
    fn sender_info(&self) -> SenderInfo {
        SenderInfo::default()
    }

    /// End the calling process with a signal (mach_msg_trap's guard
    /// violation: SIGKILL).
    fn kill_self(&mut self, signal: u32);

    // ----- the address space -----

    /// The regions overlapping `[start, end)`, in address order.
    fn regions(&self, start: u64, end: u64) -> Vec<Region>;
    /// Map zero-filled anonymous memory, read-write, maximum all: anywhere
    /// (`fixed` = `None`) or at that address (replacing mappings when
    /// `overwrite`). `Err` when there is no room.
    fn map_anon(&mut self, len: u64, fixed: Option<u64>, overwrite: bool) -> Result<u64, Fault>;
    fn unmap(&mut self, start: u64, end: u64);
    /// Set the current protection of the mapped range (the personality has
    /// checked it against the maximum).
    fn protect(&mut self, start: u64, end: u64, prot: u32);
    /// Lower the maximum protection (and the current one with it).
    fn set_max_prot(&mut self, start: u64, end: u64, max: u32);
    fn set_inherit(&mut self, start: u64, end: u64, inherit: Inherit);
    /// Zero the pages of the range (MADV_ZERO).
    fn zero(&mut self, start: u64, end: u64);
    fn set_locked(&mut self, start: u64, end: u64, locked: bool);
    /// Write dirty pages of shared file mappings in the range to their
    /// files, waiting with `wait`.
    fn sync(&mut self, start: u64, end: u64, wait: bool);
    fn page_info(&self, addr: u64) -> PageInfo;

    // ----- threads -----

    /// Whether libpthread registered its thread entry point
    /// (bsdthread_register).
    fn thread_entry_registered(&self) -> bool;
    /// Start a thread at the registered entry point; its pthread address.
    /// Poly's contract with its own libpthread (ADR 0257 addendum): the
    /// thread starts with the stack pointer at `stack` and, on x86-64,
    /// `rdi` = the thread structure, `rsi` = the start function, `rdx` =
    /// its argument.
    fn create_thread(&mut self, start: ThreadStart) -> Result<u64, u32>;

    /// bsdthread_register: remember libpthread's thread entry point; a
    /// second registration is `EINVAL` (the sheet).
    fn register_thread_entry(&mut self, _entry: u64) -> Result<(), u32> {
        Err(crate::errno::EINVAL as u32)
    }

    /// bsdthread_terminate: end the calling thread (not the process) once
    /// the call returns.
    fn end_thread(&mut self) {}
}

/// Run `f` with the Mach state locked and return its result.
pub fn with_mach<R>(k: &mut dyn XnuKernel, f: impl FnOnce(&mut MachState, &mut dyn XnuKernel) -> R) -> R {
    let mut f = Some(f);
    let mut out = None;
    k.with_mach(&mut |m, k| {
        if let Some(f) = f.take() {
            out = Some(f(m, k));
        }
    });
    out.expect("with_mach ran the closure")
}

/// Unblock the threads a waker woke.
pub fn unblock_all(k: &mut dyn XnuKernel, woken: Vec<ThreadId>) {
    for t in woken {
        k.unblock(t);
    }
}

/// The signal a fault on a page kills a process with when nothing handles
/// it: SIGBUS (10) for an access the protection forbids, SIGSEGV (11) for
/// an address that is not mapped (`mprotect.yaml`, `vm_protect` and
/// `minherit` sheets).
pub const fn fault_signal(mapped: bool) -> u32 {
    if mapped { 10 } else { 11 }
}
