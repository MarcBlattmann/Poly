//! What the NT services need from the kernel, as a trait the kernel
//! implements for the calling thread (one value per system call), so the
//! services run on the host against a fake.
//!
//! The object, memory and section services reach the kernel only through
//! [`NtKernel::with_state`] (they are pure logic over [`NtSystem`]); the
//! process, thread, synchronisation and APC services also need the kernel to
//! *act* — create and end ring-3 threads, block the caller until a wait is
//! satisfied, suspend a thread and read its context, deliver APCs, sleep and
//! yield — which [`NtKernel`]'s other methods do. The blocking, the clock
//! and SMP wake-ups are the kernel's (ADR 0030, ADR 0065); the decisions
//! (which wait is satisfied, mutant ownership, limits) are `poly-ntobj`'s.

use crate::backend::Backend;
use crate::context::NtContext;
use crate::system::{NtProcess, NtSystem};
use poly_ntobj::{Caller, NtStatus, ObjectId, ProcessId, ThreadRef, WaitId};

/// A user-memory access failed (the address is not mapped user memory
/// with the needed rights).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserFault;

/// A thread's client id (`CLIENT_ID`, `ntdef.h`): the owning process and the
/// thread. Poly's thread ids come from one space (the first thread's id is
/// the process id), as on Linux's `tgid`/`tid`, so both are plain ids here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientId {
    pub process: ProcessId,
    pub thread: ThreadRef,
}

/// A relative or absolute timeout for a wait, a delay or a sleep, decoded
/// from a `LARGE_INTEGER*` (Learn: "KeWaitForSingleObject", "NtWaitFor…":
/// a null pointer waits forever; a non-negative value is an absolute time in
/// 100 ns units since 1601; a negative value is a relative interval).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NtTimeout {
    /// A null `LARGE_INTEGER*`: wait forever.
    Infinite,
    /// A relative interval in 100 ns units (the magnitude of a negative
    /// `LARGE_INTEGER`). Zero means do not block.
    Relative(u64),
    /// An absolute time in 100 ns units since 1601 (a non-negative
    /// `LARGE_INTEGER`).
    Absolute(i64),
}

impl NtTimeout {
    /// Whether this timeout means "do not block" (a relative zero).
    pub fn is_zero(self) -> bool {
        self == NtTimeout::Relative(0)
    }

    /// The `poly_ntobj::Timeout` the dispatcher uses: only "not at all"
    /// versus "maybe block" matters there; the kernel runs the clock.
    pub fn dispatch(self) -> poly_ntobj::Timeout {
        match self {
            NtTimeout::Infinite => poly_ntobj::Timeout::Infinite,
            NtTimeout::Relative(0) => poly_ntobj::Timeout::Zero,
            _ => poly_ntobj::Timeout::Finite,
        }
    }
}

/// A user-mode APC the kernel queues on a thread (`NtQueueApcThread`): the
/// routine and its three arguments, delivered at an alertable wait or
/// `NtTestAlert` (Learn: "QueueUserAPC", "APCs").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Apc {
    pub routine: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
}

/// A request to create a thread in the calling process
/// (`NtCreateThreadEx`), once the service has decoded its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadCreate {
    /// The user routine the thread runs, called as `routine(argument)`.
    pub start_routine: u64,
    pub argument: u64,
    /// Requested stack reserve (0: the image default).
    pub stack_reserve: u64,
    /// Requested stack commit (0: the image default).
    pub stack_commit: u64,
    /// Start the thread suspended (`THREAD_CREATE_FLAGS_CREATE_SUSPENDED`).
    pub suspended: bool,
    /// The access mask the new thread handle is opened with.
    pub desired_access: u32,
}

/// What [`NtKernel::create_thread`] produced: a handle to the new thread's
/// object in the caller's table and the new thread's client id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadCreated {
    pub handle: poly_ntobj::Handle,
    pub client: ClientId,
}

/// The calling thread's view of the kernel.
pub trait NtKernel {
    /// Copy from the caller's memory.
    fn read_user(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault>;

    /// Copy to the caller's memory.
    fn write_user(&mut self, addr: u64, data: &[u8]) -> Result<(), UserFault>;

    /// Who is calling (process, thread, previous mode).
    fn caller(&self) -> Caller;

    /// Run `f` with the system-wide NT state, the calling process's state
    /// and the kernel's side of its memory, all locked. Nothing in `f`
    /// touches user memory through [`read_user`](Self::read_user) or
    /// [`write_user`](Self::write_user).
    fn with_state<R>(&mut self, f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R) -> R;

    /// End the calling process with `status` once the current call
    /// returns (`NtTerminateProcess` of the current process).
    fn terminate_process(&mut self, status: NtStatus);

    /// End the calling thread with `status` once the current call returns
    /// (`NtTerminateThread` of the current thread). If it is the last thread
    /// the process ends with that status.
    fn terminate_current_thread(&mut self, status: NtStatus);

    /// Create a thread in the calling process (`NtCreateThreadEx`). The
    /// kernel allocates a thread id and its stack and TEB, creates the thread
    /// object, starts it in ring 3 (suspended if asked) and returns a handle
    /// to it. Cross-process creation is `STATUS_NOT_IMPLEMENTED` for now.
    fn create_thread(&mut self, req: ThreadCreate) -> Result<ThreadCreated, NtStatus>;

    /// End another thread of this process (`NtTerminateThread` of a thread
    /// that is not the caller). `None` means it is already gone.
    fn terminate_thread(&mut self, thread: ThreadRef, status: NtStatus) -> Result<(), NtStatus>;

    /// Suspend a thread (`NtSuspendThread`): raise its suspend count and, if
    /// it was zero, stop it before it next runs user code. Returns the
    /// previous count.
    fn suspend_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus>;

    /// Resume a thread (`NtResumeThread`): lower its suspend count and let it
    /// run again when it reaches zero. Returns the previous count.
    fn resume_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus>;

    /// Read a suspended thread's CPU context (`NtGetContextThread`). Blocks
    /// until the thread is actually stopped so the context is stable; the
    /// caller's own context is read directly.
    fn get_context(&mut self, thread: ThreadRef) -> Result<NtContext, NtStatus>;

    /// Replace a suspended thread's CPU context (`NtSetContextThread`),
    /// keeping the fields `want` does not select.
    fn set_context(&mut self, thread: ThreadRef, ctx: &NtContext, want: u32) -> Result<(), NtStatus>;

    /// Block the calling thread until wait `id` is satisfied, the timeout
    /// expires (`STATUS_TIMEOUT`), or — if `alertable` — an APC is delivered
    /// (`STATUS_USER_APC`) or it is alerted (`STATUS_ALERTED`). Returns the
    /// `NTSTATUS` the wait finishes with.
    fn wait(&mut self, id: WaitId, timeout: NtTimeout, alertable: bool) -> NtStatus;

    /// Arm a waitable timer (`NtSetTimer`): the kernel runs the clock and,
    /// when the due time arrives, signals the timer object and (for a
    /// periodic timer) re-arms it `period_ms` later, delivering `apc` to the
    /// setting thread each time it fires. Returns the timer's previous signal
    /// state. Re-arming a set timer cancels the previous arming.
    fn arm_timer(
        &mut self,
        timer: ObjectId,
        due: NtTimeout,
        period_ms: u32,
        apc: Option<Apc>,
    ) -> Result<bool, NtStatus>;

    /// Disarm a waitable timer (`NtCancelTimer`); returns its signal state.
    fn disarm_timer(&mut self, timer: ObjectId) -> Result<bool, NtStatus>;

    /// Queue a user-mode APC on a thread (`NtQueueApcThread`).
    fn queue_apc(&mut self, thread: ThreadRef, apc: Apc) -> Result<(), NtStatus>;

    /// Deliver any pending user APC to the calling thread now
    /// (`NtTestAlert`): returns `STATUS_USER_APC` if one ran, else
    /// `STATUS_SUCCESS`.
    fn test_alert(&mut self) -> NtStatus;

    /// Sleep for `timeout` (`NtDelayExecution`); `alertable` lets an APC cut
    /// it short with `STATUS_USER_APC`. Returns `STATUS_SUCCESS` (or
    /// `STATUS_ALERTED`/`STATUS_USER_APC`).
    fn delay(&mut self, timeout: NtTimeout, alertable: bool) -> NtStatus;

    /// Yield the rest of the time slice (`NtYieldExecution`): `STATUS_SUCCESS`
    /// if another thread ran, else `STATUS_NO_YIELD_PERFORMED`.
    fn yield_execution(&mut self) -> NtStatus;

    /// The tail of the APC trampoline (`PolyResumeApc`): restore the context
    /// the calling thread had when the APC interrupted it, and return the
    /// status that context's wait should see (`STATUS_USER_APC`).
    fn resume_apc(&mut self) -> NtStatus;

    /// Log three values for a test program (`PolyTrace`).
    fn trace(&mut self, values: [u64; 3]);

    /// The system time as a `FILETIME` (100 ns since 1601-01-01 UTC), for
    /// registry last-write times (ADR 0066). A kernel without a clock
    /// answers 0.
    fn system_time(&self) -> u64 {
        0
    }

    /// The time since boot in 100 ns units, never going backwards
    /// (`NtQueryPerformanceCounter`, ADR 0076). A kernel without a clock
    /// answers 0.
    fn monotonic_100ns(&self) -> u64 {
        0
    }

    /// The number of processors programs are told about
    /// (`NtQuerySystemInformation(SystemBasicInformation)`, ADR 0076).
    fn processor_count(&self) -> u32 {
        1
    }

    /// Write hive file `file` (a name like `SOFTWARE`, no directory) with
    /// `data` and return once it is on disk (`NtFlushKey`, ADR 0066). A
    /// kernel with nowhere to keep it succeeds without writing.
    fn save_hive(&mut self, file: &str, data: &[u8]) -> Result<(), NtStatus> {
        let _ = (file, data);
        Ok(())
    }

    /// Make the calling thread resume user mode with the registers of `ctx`
    /// that `flags` selects (`CONTEXT_CONTROL`, `CONTEXT_INTEGER`,
    /// `CONTEXT_FLOATING_POINT`), once the current call or exception returns
    /// (exception dispatch and `NtContinue`, ADR 0068). All 64 bits of RAX
    /// come from `ctx`, not from the call's status. A kernel that cannot do
    /// it answers `STATUS_NOT_IMPLEMENTED`.
    fn load_context(&mut self, ctx: &poly_seh::Context, flags: u32) -> Result<(), NtStatus> {
        let _ = (ctx, flags);
        Err(poly_ntobj::status::STATUS_NOT_IMPLEMENTED)
    }

    /// An exception nobody handles (second chance, no dispatcher, a frame
    /// that cannot be written, a fail-fast request): end the calling
    /// process with the exception code as its exit status once the current
    /// call or exception returns (ADR 0068). `ctx` is where it happened, for
    /// the kernel's log.
    fn unhandled_exception(&mut self, record: &poly_seh::ExceptionRecord, ctx: &poly_seh::Context) {
        let _ = ctx;
        self.terminate_process(NtStatus(record.code));
    }
}
