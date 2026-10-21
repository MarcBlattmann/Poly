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
//! Only our own `ntdll` calls these services with their numbers
//! ([`poly_personality_nt::Service`]); the numbering is ours and documented
//! there. The kernel maps the executable and `ntdll` ([`NTDLL`], ADR 0071)
//! and starts the first thread in `ntdll`, which binds the executable's
//! imports and calls its entry point ([`start`]).
//!
//! [ADR 0063]: ../../../docs/adr/0063-nt-processes-object-manager-and-memory.md

mod backend;
mod io;
mod test_io;
mod test_pe;
mod test_threads;
mod tests;

mod sched;

mod exception;
mod test_seh;
pub use exception::{fault, last_unhandled};

pub mod registry;
mod test_registry;

pub mod layout;
mod test_layout;

mod test_ldr;

/// Our `ntdll.dll` (ADR 0071), built by `cargo xtask` from
/// `personalities/nt/ntdll` (cached by content hash) and embedded here, as
/// Windows keeps `ntdll` in its `KnownDlls`: every process maps this copy,
/// and the layout puts the same bytes in `C:\Windows\System32` ([`layout`]).
/// Empty when the kernel was built without `xtask`.
pub static NTDLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ntdll.dll"));

use crate::process::{ExitStatus, Process, ThreadFlags};
use crate::sync::Mutex;
use crate::thread;
use crate::user::{self, Memory};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
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
    /// The exception the memory model chose for this thread's last
    /// unresolved page fault, and its address (ADR 0068).
    page_fault: Option<(u64, NtStatus)>,
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
            sys: registry::boot_system(),
            processes: BTreeMap::new(),
            sections: Sections::default(),
            ctl: BTreeMap::new(),
            timers: BTreeMap::new(),
        };
        // The Windows folders on C: and the registry values that follow
        // them (ADR 0073), before file I/O attaches C:.
        layout::prepare(&mut state.sys);
        // File I/O: the VFS's volumes and drive letters (ADR 0067).
        io::attach(&mut state.sys);
        state
    });
    f(state)
}

/// Run `f` on the NT state if the first NT use made it already.
fn with_existing<R>(f: impl FnOnce(&mut NtState) -> R) -> Option<R> {
    STATE.lock().as_mut().map(f)
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
    /// Exception dispatch's requests (ADR 0068).
    seh: exception::Pending,
}

impl Kernel {
    fn new(pid: ProcessId, tid: ThreadRef, memory: Arc<Memory>) -> Kernel {
        Kernel { pid, tid, memory, exit: None, exit_thread: None, seh: exception::Pending::default() }
    }
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
        let status = sched::test_alert(self.pid, self.tid);
        // After NtContinue(.., TestAlert): the APC resumes the loaded
        // context with its own RAX, not STATUS_USER_APC (ADR 0068).
        if status == poly_ntobj::status::STATUS_USER_APC
            && let Some(rax) = self.seh.rax
        {
            with(|s| {
                if let Some(saved) = s.ctl.get_mut(&self.tid).and_then(|c| c.apc_saved.as_mut()) {
                    saved.rax = rax;
                }
            });
        }
        status
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
        // The interrupted context's RAX comes back in full (it may be one
        // NtContinue loaded), not as a 32-bit status.
        let rax = with(|s| s.ctl.get(&self.tid).and_then(|c| c.apc_saved.map(|r| r.rax)));
        self.seh.rax = rax;
        sched::resume_apc(self.tid)
    }

    fn load_context(&mut self, ctx: &poly_seh::Context, flags: u32) -> Result<(), NtStatus> {
        if let Some(rax) = exception::load(ctx, flags)? {
            self.seh.rax = Some(rax);
        }
        Ok(())
    }

    fn unhandled_exception(&mut self, record: &poly_seh::ExceptionRecord, ctx: &poly_seh::Context) {
        self.seh.unhandled = Some((*record, ctx.clone()));
    }

    fn trace(&mut self, values: [u64; 3]) {
        crate::kprintln!("nt: pid {} PolyTrace {:#x} {:#x} {:#x}", self.pid.0, values[0], values[1], values[2]);
    }

    fn system_time(&self) -> u64 {
        registry::filetime_now()
    }

    fn save_hive(&mut self, file: &str, data: &[u8]) -> Result<(), NtStatus> {
        registry::save(file, data)
    }
}

/// Route an NT process's system call (its `SyscallHook`).
pub fn syscall(process: &Process, regs: &mut SyscallRegs) -> Outcome {
    let Some(memory) = user::current_memory() else {
        regs.rax = u64::from(poly_personality_nt::STATUS_NOT_IMPLEMENTED);
        return Outcome::Handled;
    };
    let tid = user::current_tid().unwrap_or(process.pid);
    let mut k = Kernel::new(ProcessId(process.pid), ThreadRef(u64::from(tid)), memory);
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
    // A context loaded by NtContinue or exception dispatch returns its
    // RAX in full; an unhandled exception ends the process here.
    if let Some(rax) = exception::finish(k) {
        regs.rax = rax;
    }
    outcome
}

/// A PE executable started as an NT process: its first thread's entry.
pub struct Start {
    pub entry: crate::arch::user::Entry,
    /// The GS base the thread runs with (its TEB).
    pub teb: u64,
}

/// How an NT process was started: its executable's DOS path and its
/// arguments (after the program name), for its process parameters
/// (ADR 0073).
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub image_path: String,
    pub args: Vec<String>,
}

impl Launch {
    /// The launch of the executable at VFS path `path` with `argv`
    /// (`argv[0]`, the program name, is replaced by the DOS path).
    pub fn of(path: &[u8], argv: &[&[u8]]) -> Launch {
        Launch {
            image_path: layout::dos_path(path),
            args: argv.iter().skip(1).map(|a| String::from_utf8_lossy(a).into_owned()).collect(),
        }
    }

    /// An image the kernel built itself (a test) has no file; it is named
    /// as if it were in `System32`.
    fn unnamed() -> Launch {
        Launch { image_path: "C:\\Windows\\System32\\poly-image.exe".into(), args: Vec::new() }
    }
}

/// Set up `process` (already made with [`Process::new`] and personality
/// `Nt`) to run the PE image `file` in `memory`, as its first thread `tid`.
/// Returns where to start it. The caller spawns the thread with
/// [`user::spawn_nt`] and [`DEFAULT_HOOK`](user::DEFAULT_HOOK).
pub fn start(process: &Arc<Process>, memory: &Arc<Memory>, file: &[u8], tid: u32) -> Result<Start, NtStatus> {
    start_with(process, memory, file, tid, &Launch::unnamed())
}

/// [`start`] with the executable's path and arguments, which go into the
/// process parameters with the default environment (ADR 0073).
pub fn start_with(
    process: &Arc<Process>,
    memory: &Arc<Memory>,
    file: &[u8],
    tid: u32,
    launch: &Launch,
) -> Result<Start, NtStatus> {
    let pid = ProcessId(process.pid);
    let thread = ThreadRef(u64::from(tid));
    if NTDLL.is_empty() {
        crate::kprintln!("nt: this kernel was built without ntdll.dll (build it with cargo xtask)");
        return Err(NtStatus(0xC000_0135)); // STATUS_DLL_NOT_FOUND (ntstatus.h)
    }
    // File I/O completions are written into this memory (ADR 0067).
    io::register(process.pid, memory);
    let started = with(|s| {
        let args: Vec<&str> = launch.args.iter().map(String::as_str).collect();
        let params = poly_personality_nt::params::for_program(&launch.image_path, &args, layout::environment(&s.sys));
        s.processes.insert(pid, NtProcess::new(pid));
        let proc = s.processes.get_mut(&pid).expect("just inserted");
        let mut b = MemBackend { memory, sections: &mut s.sections };
        let start = exec::start_process(&mut s.sys, proc, &mut b, file, NTDLL, None, thread).and_then(|ps| {
            // The process parameters: image path, command line and
            // environment (ADR 0073).
            poly_personality_nt::params::write(proc, &mut b, ps.peb, &params)?;
            Ok(ps)
        });
        match start {
            Ok(ps) => Ok(Start {
                // The first thread starts in ntdll's LdrInitializeThunk with
                // the image entry, its argument (the PEB) and the image base
                // (ADR 0071's start convention).
                entry: crate::arch::user::Entry {
                    rip: ps.entry,
                    rsp: ps.rsp,
                    args: [ps.start_routine, ps.argument, ps.image_base],
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
        // Its registry notifications go too (ADR 0066).
        s.sys.registry.process_ended(ProcessId(pid), &mut s.sys.objects);
    });

    // Its file objects closed with its handles: deliver the cancellations.
    io::drain();
    io::unregister(pid);
    sched::drain_wakes();
    registry::flush();
}

/// A page fault the memory map could not resolve, in NT process `pid`: a
/// guard page may fire and a stack grow (the thread goes on), else it is an
/// exception. Returns whether the faulting instruction should be retried.
pub fn resolve_fault(pid: u32, tid: u32, memory: &Arc<Memory>, addr: u64) -> bool {
    with(|s| {
        let Some(proc) = s.processes.get_mut(&ProcessId(pid)) else { return false };
        let mut b = MemBackend { memory, sections: &mut s.sections };
        let tref = ThreadRef(u64::from(tid));
        match exec::access_fault(proc, &mut b, tref, addr) {
            vm::AccessFault::StackGrown { .. } => true,
            // Remembered for the exception it raises (ADR 0068).
            vm::AccessFault::Exception(status) => {
                s.ctl.entry(tref).or_default().page_fault = Some((addr, status));
                false
            }
        }
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
/// `launch` names it and gives its arguments (its process parameters,
/// ADR 0073).
pub fn spawn_pe_with(
    process: &Arc<Process>,
    file: &[u8],
    hook: user::SyscallHook,
    launch: &Launch,
) -> Result<(), SpawnError> {
    use crate::user::memory::Layout;
    let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| SpawnError::Memory)?);
    let pid = process.pid;
    let start = start_with(process, &memory, file, pid, launch).map_err(SpawnError::Image)?;
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
