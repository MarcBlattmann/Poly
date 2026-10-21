//! The NT personality in the kernel: a PE executable runs as an NT process,
//! its system calls routed to `poly_personality_nt` ([ADR 0063]).
//!
//! The personality holds the machine-wide object manager and sections
//! ([`poly_personality_nt::NtSystem`]) and one
//! [`poly_personality_nt::NtProcess`] per process; the kernel keeps both
//! behind one lock ([`STATE`]) and lends them, with the calling process's
//! memory wrapped as a [`backend::MemBackend`], to each service through
//! [`Kernel::with_state`]. Section pages and minimal file objects live in
//! [`backend::Sections`].
//!
//! Only our own `ntdll` will call these services with their numbers
//! ([`poly_personality_nt::Service`]); the numbering is ours and documented
//! there. Until `ntdll` exists the kernel maps the executable's image and
//! starts its entry point itself ([`start`]).
//!
//! [ADR 0063]: ../../../docs/adr/0063-nt-processes-object-manager-and-memory.md

mod backend;
mod io;
mod test_io;
mod test_pe;
mod test_threads;
mod tests;

mod sched;

use crate::process::{ExitStatus, Process, ThreadFlags};
use crate::sync::Mutex;
use crate::thread;
use crate::user::{self, Memory};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use backend::{MemBackend, Sections};
use poly_abi::{Outcome, SyscallRegs};
use poly_ntobj::{Caller, NtStatus, ObjectId, ProcessId, ThreadRef, WaitId};
use poly_personality_nt::backend::Backend;
use poly_personality_nt::context::NtContext;
use poly_personality_nt::kernel::{Apc, NtKernel, NtTimeout, UserFault};
use poly_personality_nt::system::{NtProcess, NtSystem};
use poly_personality_nt::{exec, vm};

/// What the kernel keeps for one NT thread, besides its object (`poly-ntobj`)
/// and its TEB and stack (`NtProcess`): everything the blocking, suspension
/// and APC machinery needs (ADR 0065).
#[derive(Default)]
struct ThreadCtl {
    /// Its flags, for [`crate::process::park`] / `unpark` / `kick`.
    flags: Option<Arc<ThreadFlags>>,
    /// The NT wait it is blocked in, if any, and whether that wait is
    /// alertable (so a queued APC wakes it).
    blocked: Option<WaitId>,
    alertable: bool,
    /// The status a signaller recorded for its blocked wait, taken when the
    /// thread wakes.
    completion: Option<NtStatus>,
    /// `NtSuspendThread` count; the thread parks before user mode while > 0.
    suspend_count: u32,
    /// Its user context, saved while it is parked suspended (so another
    /// thread may read it with `NtGetContextThread`).
    suspended_ctx: Option<NtContext>,
    /// A context `NtSetContextThread` left for it to load when it resumes.
    set_on_resume: Option<NtContext>,
    /// Queued user APCs (`NtQueueApcThread`), delivered at an alertable wait.
    apcs: VecDeque<Apc>,
    /// The context an APC interrupted, restored by `PolyResumeApc`.
    apc_saved: Option<user::UserRegs>,
    /// A thread created `CREATE_SUSPENDED` whose ring-3 start is deferred
    /// until it is resumed: (entry, teb).
    deferred_start: Option<(user::Entry, u64)>,
    /// The exit status `NtTerminateThread` chose for the current thread,
    /// applied when its system call returns.
    exit_status: Option<NtStatus>,
}

/// The whole NT state behind one lock (ADR 0030: the object manager and
/// dispatcher are one lock; the per-process memory models and the section
/// store ride with it until SMP needs them split).
struct NtState {
    sys: NtSystem,
    processes: BTreeMap<ProcessId, NtProcess>,
    sections: Sections,
    /// Per-thread control blocks ([`ThreadCtl`]), keyed by thread id.
    ctl: BTreeMap<ThreadRef, ThreadCtl>,
    /// Armed waitable timers, keyed by their object id (`NtSetTimer`).
    timers: BTreeMap<ObjectId, Arc<sched::NtTimer>>,
}

static STATE: Mutex<Option<NtState>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut NtState) -> R) -> R {
    let mut guard = STATE.lock();
    let state = guard.get_or_insert_with(|| {
        let mut state = NtState {
            sys: NtSystem::new(),
            processes: BTreeMap::new(),
            sections: Sections::default(),
            ctl: BTreeMap::new(),
            timers: BTreeMap::new(),
        };
        // File I/O: the VFS's volumes and drive letters (ADR 0067).
        io::attach(&mut state.sys);
        state
    });
    f(state)
}

/// Whether `pid` is an NT process the kernel has set up.
pub fn is_nt_process(pid: u32) -> bool {
    with(|s| s.processes.contains_key(&ProcessId(pid)))
}

/// The calling thread's view of the kernel for one system call.
struct Kernel {
    pid: ProcessId,
    tid: ThreadRef,
    memory: Arc<Memory>,
    /// Set by `NtTerminateProcess`, acted on after the call returns.
    exit: Option<NtStatus>,
    /// Set by `NtTerminateThread(NtCurrentThread)`, acted on after the call.
    exit_thread: Option<NtStatus>,
}

impl NtKernel for Kernel {
    fn read_user(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault> {
        crate::arch::user::copy_from_user(buf, addr).map_err(|_| UserFault)
    }

    fn write_user(&mut self, addr: u64, data: &[u8]) -> Result<(), UserFault> {
        crate::arch::user::copy_to_user(addr, data).map_err(|_| UserFault)
    }

    fn caller(&self) -> Caller {
        Caller::user(self.pid, self.tid)
    }

    fn with_state<R>(&mut self, f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R) -> R {
        let memory = &self.memory;
        with(|s| {
            let proc = s.processes.get_mut(&self.pid).expect("an NT process");
            let mut b = MemBackend { memory, sections: &mut s.sections };
            f(&mut s.sys, proc, &mut b)
        })
    }

    fn terminate_process(&mut self, status: NtStatus) {
        self.exit = Some(status);
    }

    fn terminate_current_thread(&mut self, status: NtStatus) {
        with(|s| s.ctl.entry(self.tid).or_default().exit_status = Some(status));
        self.exit_thread = Some(status);
    }

    fn create_thread(
        &mut self,
        req: poly_personality_nt::ThreadCreate,
    ) -> Result<poly_personality_nt::ThreadCreated, NtStatus> {
        sched::create_thread(self.pid, self.tid, &self.memory, req)
    }

    fn terminate_thread(&mut self, thread: ThreadRef, status: NtStatus) -> Result<(), NtStatus> {
        sched::terminate_other(self.pid, thread, status)
    }

    fn suspend_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        sched::suspend(self.pid, thread)
    }

    fn resume_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        sched::resume(self.pid, &self.memory, thread)
    }

    fn get_context(&mut self, thread: ThreadRef) -> Result<NtContext, NtStatus> {
        sched::get_context(self.tid, thread)
    }

    fn set_context(&mut self, thread: ThreadRef, ctx: &NtContext, want: u32) -> Result<(), NtStatus> {
        sched::set_context(self.tid, thread, ctx, want)
    }

    fn wait(&mut self, id: WaitId, timeout: NtTimeout, alertable: bool) -> NtStatus {
        sched::wait(self.pid, self.tid, id, timeout, alertable)
    }

    fn arm_timer(&mut self, timer: ObjectId, due: NtTimeout, period: u32, apc: Option<Apc>) -> Result<bool, NtStatus> {
        sched::arm_timer(self.pid, self.tid, timer, due, period, apc)
    }

    fn disarm_timer(&mut self, timer: ObjectId) -> Result<bool, NtStatus> {
        sched::disarm_timer(timer)
    }

    fn queue_apc(&mut self, thread: ThreadRef, apc: Apc) -> Result<(), NtStatus> {
        sched::queue_apc(thread, apc)
    }

    fn test_alert(&mut self) -> NtStatus {
        sched::test_alert(self.pid, self.tid)
    }

    fn delay(&mut self, timeout: NtTimeout, alertable: bool) -> NtStatus {
        sched::delay(self.pid, self.tid, timeout, alertable)
    }

    fn yield_execution(&mut self) -> NtStatus {
        if thread::yield_now() {
            poly_ntobj::status::STATUS_SUCCESS
        } else {
            poly_ntobj::status::STATUS_NO_YIELD_PERFORMED
        }
    }

    fn resume_apc(&mut self) -> NtStatus {
        sched::resume_apc(self.tid)
    }

    fn trace(&mut self, values: [u64; 3]) {
        crate::kprintln!("nt: pid {} PolyTrace {:#x} {:#x} {:#x}", self.pid.0, values[0], values[1], values[2]);
    }
}

/// Route an NT process's system call (its `SyscallHook`).
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let Some(memory) = user::current_memory() else {
        regs.rax = u64::from(poly_personality_nt::STATUS_NOT_IMPLEMENTED);
        return Outcome::Handled;
    };
    let tid = user::current_tid().unwrap_or(process.pid);
    let mut k =
        Kernel { pid: ProcessId(process.pid), tid: ThreadRef(u64::from(tid)), memory, exit: None, exit_thread: None };
    let outcome = poly_personality_nt::dispatch_with(&mut k, regs);
    // Any wait a signalling call satisfied: wake those threads.
    sched::drain_wakes();
    // File I/O requests it queued or completed (ADR 0067).
    io::after_call();
    if let Some(status) = k.exit {
        let _ = user::request_exit(ExitStatus::Exited(status.0 as u8));
    } else if let Some(status) = k.exit_thread {
        let _ = user::request_exit(ExitStatus::Exited(status.0 as u8));
    }
    outcome
}

/// A PE executable started as an NT process: its first thread's entry.
pub struct Start {
    pub entry: crate::arch::user::Entry,
    /// The GS base the thread runs with (its TEB).
    pub teb: u64,
}

/// Set up `process` (already made with [`Process::new`] and personality
/// `Nt`) to run the PE image `file` in `memory`, as its first thread `tid`.
/// Returns where to start it. The caller spawns the thread with
/// [`user::spawn_nt`] and [`DEFAULT_HOOK`](user::DEFAULT_HOOK).
pub fn start(process: &Arc<Process>, memory: &Arc<Memory>, file: &[u8], tid: u32) -> Result<Start, NtStatus> {
    let pid = ProcessId(process.pid);
    let thread = ThreadRef(u64::from(tid));
    // File I/O completions are written into this memory (ADR 0067).
    io::register(process.pid, memory);
    let started = with(|s| {
        s.processes.insert(pid, NtProcess::new(pid));
        let proc = s.processes.get_mut(&pid).expect("just inserted");
        let mut b = MemBackend { memory, sections: &mut s.sections };
        let start = exec::start_process(&mut s.sys, proc, &mut b, file, None, thread);
        match start {
            Ok(ps) => Ok(Start {
                // The first thread also starts at the trampoline, which calls
                // the image entry with the PEB and ends the process cleanly
                // when the entry returns.
                entry: crate::arch::user::Entry {
                    rip: ps.entry,
                    rsp: ps.rsp,
                    args: [ps.start_routine, ps.argument, 0],
                },
                teb: ps.teb,
            }),
            Err(e) => {
                let proc = s.processes.remove(&pid).expect("just inserted");
                let mut proc = proc;
                let mut b = MemBackend { memory, sections: &mut s.sections };
                exec::process_exit(&mut s.sys, &mut proc, &mut b);
                Err(e)
            }
        }
    });
    if started.is_err() {
        io::unregister(process.pid);
    }
    started
}

pub use sched::wait_while_suspended;

/// Translate a thread's [`ExitStatus`] to the `NTSTATUS` its object reports.
fn exit_to_nt(status: ExitStatus) -> NtStatus {
    match status {
        ExitStatus::Exited(code) => NtStatus(u32::from(code)),
        ExitStatus::Faulted(vector) => exec::exception_status(vector),
        ExitStatus::Signaled { .. } => NtStatus(0xC000_0005), // STATUS_ACCESS_VIOLATION
    }
}

/// One thread of NT process `pid` has ended: free its stack and TEB, record
/// its exit status, signal its thread object and abandon its mutants (so
/// waiters wake). Called for every NT thread as it leaves, before it
/// detaches from its process.
pub fn thread_exit(pid: u32, tid: u32, status: ExitStatus, memory: &Arc<Memory>) {
    let tref = ThreadRef(u64::from(tid));
    with(|s| {
        if !s.processes.contains_key(&ProcessId(pid)) {
            return;
        }
        let nt_status = s.ctl.get(&tref).and_then(|c| c.exit_status).unwrap_or_else(|| exit_to_nt(status));
        let proc = s.processes.get_mut(&ProcessId(pid)).expect("checked");
        let mut b = MemBackend { memory, sections: &mut s.sections };
        exec::thread_exit(&mut s.sys, proc, &mut b, tref, nt_status);
        s.ctl.remove(&tref);
        // Its pending I/O requests are cancelled (ADR 0067).
        s.sys.io.cancel_thread(tref);
    });
    sched::drain_wakes();
    io::drain();
}

/// The NT process `pid` has ended (its last thread is going): drop its
/// objects, views and handles. Its memory itself goes with `user::Memory`.
pub fn process_exit(pid: u32, memory: &Arc<Memory>) {
    with(|s| {
        if let Some(mut proc) = s.processes.remove(&ProcessId(pid)) {
            let mut b = MemBackend { memory, sections: &mut s.sections };
            exec::process_exit(&mut s.sys, &mut proc, &mut b);
        }
    });
    // Its file objects closed with its handles: deliver the cancellations.
    io::drain();
    io::unregister(pid);
}

/// A page fault the memory map could not resolve, in NT process `pid`: a
/// guard page may fire and a stack grow (the thread goes on), else it is an
/// exception. Returns whether the faulting instruction should be retried.
pub fn resolve_fault(pid: u32, tid: u32, memory: &Arc<Memory>, addr: u64) -> bool {
    with(|s| {
        let Some(proc) = s.processes.get_mut(&ProcessId(pid)) else { return false };
        let mut b = MemBackend { memory, sections: &mut s.sections };
        matches!(exec::access_fault(proc, &mut b, ThreadRef(u64::from(tid)), addr), vm::AccessFault::StackGrown { .. })
    })
}

/// Why a PE executable could not be started.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // read through Debug when a start fails
pub enum SpawnError {
    /// No memory for the address space.
    Memory,
    /// The image is not a loadable x64 PE, or needs imports bound (our
    /// `ntdll`, a later milestone): the `NTSTATUS`.
    Image(NtStatus),
    /// The thread could not be started.
    Thread(thread::SpawnError),
}

/// Start the PE executable `file` in the already-made NT process
/// `process`, its first thread running the entry point. Returns `process`.
pub fn spawn_pe(process: &Arc<Process>, file: &[u8], hook: user::SyscallHook) -> Result<(), SpawnError> {
    use crate::user::memory::Layout;
    let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| SpawnError::Memory)?);
    let pid = process.pid;
    let start = start(process, &memory, file, pid).map_err(SpawnError::Image)?;
    match user::spawn_nt(Arc::clone(process), Arc::clone(&memory), start.entry, start.teb, hook) {
        Ok(tid) => {
            let _ = thread::detach(tid);
            Ok(())
        }
        Err(e) => {
            process_exit(pid, &memory);
            Err(SpawnError::Thread(e))
        }
    }
}
