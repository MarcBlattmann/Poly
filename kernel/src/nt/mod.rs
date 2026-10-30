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
pub mod console;
mod io;
mod test_console;
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

mod test_kernel32;

/// Our `kernelbase.dll` and `kernel32.dll` (ADR 0079), built by `cargo
/// xtask` from `personalities/nt/win32` and written to
/// `C:\Windows\System32` by [`layout`], where the loader finds them as
/// known DLLs. Empty when the kernel was built without `xtask`.
pub static KERNELBASE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kernelbase.dll"));
pub static KERNEL32: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kernel32.dll"));
/// The program the kernel32 ring-3 test runs (`test_kernel32`).
pub static K32TEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/k32test.exe"));
/// The program the console ring-3 test runs (`test_console`, ADR 0080).
pub static CONTEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/contest.exe"));

mod test_advapi32;

/// Our `advapi32.dll` (ADR 0083), built with the other Win32 modules and
/// written to `C:\Windows\System32` by [`layout`]; empty without `xtask`.
pub static ADVAPI32: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/advapi32.dll"));
/// The program the advapi32 ring-3 test runs (`test_advapi32`).
pub static A32TEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/a32test.exe"));

mod test_crt;

/// Our C runtime, `ucrtbase.dll` and `msvcrt.dll` (ADR 0084), installed in
/// `C:\Windows\System32` beside `kernel32` by [`layout`]; and the program the
/// C runtime ring-3 test runs (`test_crt`).
pub static UCRTBASE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ucrtbase.dll"));
pub static MSVCRT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/msvcrt.dll"));
pub static CRTTEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/crttest.exe"));

mod test_crtio;

/// The program the C runtime's files, processes and console ring-3 test
/// runs (`test_crtio`, ADR 0093).
pub static CRTIOTEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/crtiotest.exe"));

/// The first functions of `user32.dll` and `oleaut32.dll` (ADR 0087),
/// installed in `C:\Windows\System32` by [`layout`]; empty without `xtask`.
pub static USER32: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/user32.dll"));
pub static OLEAUT32: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/oleaut32.dll"));

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
    /// For a new process's first thread created suspended (ADR 0089): the
    /// process and memory it starts in when resumed.
    deferred_process: Option<(Arc<Process>, Arc<Memory>)>,
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
    /// Each live process's creation time, a `FILETIME`
    /// (`NtQueryInformationProcess(ProcessTimes)`, ADR 0087).
    created: BTreeMap<ProcessId, u64>,
}

static STATE: Mutex<Option<NtState>> = Mutex::new(None);

/// The full `NTSTATUS` the processes the test-run protocol watches ended
/// with ([`watch_exit_status`]; the process's own `ExitStatus` keeps only
/// its low 8 bits): `(status, final)`, final once `NtTerminateProcess`
/// set it, so the threads that end after it do not change it (ADR 0075).
static EXIT_WATCH: Mutex<BTreeMap<u32, Option<(u32, bool)>>> = Mutex::new(BTreeMap::new());

/// Remember the `NTSTATUS` NT process `pid` will end with, for
/// [`take_exit_status`] (the `poly.init` program: `POLY-EXIT ntstatus=`).
pub fn watch_exit_status(pid: u32) {
    EXIT_WATCH.lock().insert(pid, None);
}

/// The `NTSTATUS` a watched process ended with, once (`None` if it is not
/// watched or no status was recorded).
pub fn take_exit_status(pid: u32) -> Option<u32> {
    EXIT_WATCH.lock().remove(&pid).flatten().map(|(status, _)| status)
}

fn note_exit_status(pid: u32, status: NtStatus, by_process: bool) {
    if let Some(slot) = EXIT_WATCH.lock().get_mut(&pid)
        && !matches!(slot, Some((_, true)))
    {
        *slot = Some((status.0, by_process));
    }
}

fn with<R>(f: impl FnOnce(&mut NtState) -> R) -> R {
    let mut guard = STATE.lock();

    let state = guard.get_or_insert_with(|| {
        let mut state = NtState {
            sys: registry::boot_system(),
            processes: BTreeMap::new(),
            sections: Sections::default(),
            ctl: BTreeMap::new(),
            timers: BTreeMap::new(),
            created: BTreeMap::new(),
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
        note_exit_status(self.pid.0, status, true);
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

    fn create_process(
        &mut self,
        req: poly_personality_nt::ProcessCreate,
    ) -> Result<poly_personality_nt::ProcessCreated, NtStatus> {
        create_process(self.caller(), req)
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

    fn monotonic_100ns(&self) -> u64 {
        crate::time::now_ns() / 100
    }

    fn processor_count(&self) -> u32 {
        crate::thread::online_cpus().len() as u32
    }

    fn process_times(&self, pid: ProcessId) -> Option<[u64; 3]> {
        let created = with(|s| s.created.get(&pid).copied())?;
        let cpu = crate::process::find(pid.0).map(|p| crate::cputime::process(&p)).unwrap_or_default();
        Some([created, cpu.system_ns / 100, cpu.user_ns / 100])
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
    // Console output and events (ADR 0080).
    console::drive();
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
    // Its standard handles come from its descriptors 0-2 (ADR 0080).
    let sources = console::std_sources(process);
    start_with_std(process, memory, file, tid, launch, &sources)
}

/// [`start_with`] with the sources of the standard handles given.
pub fn start_with_std(
    process: &Arc<Process>,
    memory: &Arc<Memory>,
    file: &[u8],
    tid: u32,
    launch: &Launch,
    sources: &[console::StdSource; 3],
) -> Result<Start, NtStatus> {
    let params = with(|s| {
        let args: Vec<&str> = launch.args.iter().map(String::as_str).collect();
        poly_personality_nt::params::for_program(&launch.image_path, &args, layout::environment(&s.sys))
    });
    start_params(process, memory, file, tid, &params, sources, None).map(|(start, _)| start)
}

/// Who gets handles to a new process and its first thread, with which
/// access (`NtCreateUserProcess`, ADR 0079), and whether the new process
/// inherits that creator's inheritable handles (ADR 0089).
struct HandlesFor {
    caller: Caller,
    process_access: u32,
    thread_access: u32,
    inherit_handles: bool,
}

/// Handles to a new process and its first thread.
type ChildHandles = (poly_ntobj::Handle, poly_ntobj::Handle);

/// Set up `process` to run `file` with the process parameters `params`
/// (ADR 0073) and the standard handles `sources` (ADR 0080); with
/// `handles`, also open handles to its process and first thread for the
/// creator, before anything runs (ADR 0079), give it the creator's
/// inheritable handles first if asked, the creator's console, and the
/// standard handles the creator chose (ADR 0089).
fn start_params(
    process: &Arc<Process>,
    memory: &Arc<Memory>,
    file: &[u8],
    tid: u32,
    params: &poly_personality_nt::params::Parameters,
    sources: &[console::StdSource; 3],
    handles: Option<&HandlesFor>,
) -> Result<(Start, Option<ChildHandles>), NtStatus> {
    let pid = ProcessId(process.pid);
    let thread = ThreadRef(u64::from(tid));
    if NTDLL.is_empty() {
        crate::kprintln!("nt: this kernel was built without ntdll.dll (build it with cargo xtask)");
        return Err(NtStatus(0xC000_0135)); // STATUS_DLL_NOT_FOUND (ntstatus.h)
    }
    // File I/O completions are written into this memory (ADR 0067).
    io::register(process.pid, memory);
    // A console program gets a console (ADR 0080).
    let console_app =
        poly_pe::Pe::parse(file).is_ok_and(|pe| pe.info().optional.subsystem == poly_pe::subsystem::WINDOWS_CUI);
    let started = with(|s| {
        s.processes.insert(pid, NtProcess::new(pid));
        s.created.insert(pid, registry::filetime_now());
        let proc = s.processes.get_mut(&pid).expect("just inserted");
        let mut b = MemBackend { memory, sections: &mut s.sections };
        // Its creator's inheritable handles, before any other (ADR 0089).
        let inherit_from = handles.filter(|h| h.inherit_handles).map(|h| h.caller.process);
        let creator = console::FromCreator {
            given: params.given_std_handles(),
            console: handles.and_then(|h| s.sys.io.console_of_process(h.caller.process)),
        };
        let start = exec::start_process_from(&mut s.sys, proc, &mut b, file, NTDLL, None, thread, inherit_from);
        let start = start.and_then(|ps| {
            // The process parameters: image path, command line,
            // environment and current directory (ADR 0073, 0079).
            let base = poly_personality_nt::params::write(proc, &mut b, ps.peb, params)?;
            // Its console and standard handles (ADR 0080, 0089).
            let (connection, std) = console::setup(&mut s.sys, &mut b, pid, thread, console_app, sources, creator)?;
            poly_personality_nt::params::write_handles(&mut b, base, connection, std)?;
            // Handles for its creator (NtCreateUserProcess, ADR 0079).
            let opened = match handles {
                Some(h) => {
                    Some(exec::child_handles(&mut s.sys, &h.caller, pid, thread, h.process_access, h.thread_access)?)
                }
                None => None,
            };
            Ok((ps, opened))
        });
        match start {
            Ok((ps, opened)) => Ok((
                Start {
                    // The first thread starts in ntdll's LdrInitializeThunk
                    // with the image entry, its argument (the PEB) and the
                    // image base (ADR 0071's start convention).
                    entry: crate::arch::user::Entry {
                        rip: ps.entry,
                        rsp: ps.rsp,
                        args: [ps.start_routine, ps.argument, ps.image_base],
                    },
                    teb: ps.teb,
                },
                opened,
            )),
            Err(e) => {
                s.created.remove(&pid);
                let mut proc = s.processes.remove(&pid).expect("just inserted");
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

/// `NtCreateUserProcess` (ADR 0079): a new NT process running `req.image`
/// with `req.params`, the creator's console and standard handles (or those
/// it chose) and, if asked, its inheritable handles (ADR 0089), handles to
/// it and its first thread in the creator's table, and its first thread
/// started — or left suspended until `NtResumeThread` (ADR 0089).
fn create_process(
    creator: Caller,
    req: poly_personality_nt::ProcessCreate,
) -> Result<poly_personality_nt::ProcessCreated, NtStatus> {
    use crate::user::memory::Layout;
    use poly_binfmt::{Arch, Bits, Format};
    let memory = Arc::new(Memory::new(Layout::default()).map_err(|_| poly_ntobj::status::STATUS_NO_MEMORY)?);
    let creator_process = user::current_process();
    let traced = creator_process.as_ref().is_some_and(|p| p.traced);
    // The child shares its creator's console: the standard handles it
    // does not get from its creator come from the creator's descriptors
    // 0-2 (ADR 0080), else from the creator's console (ADR 0089).
    let sources = match &creator_process {
        Some(p) => console::std_sources(p),
        None => [console::StdSource::None, console::StdSource::None, console::StdSource::None],
    };
    let pid = crate::process::alloc_pid();
    let format = Format::Pe { bits: Bits::B64, arch: Arch::X86_64 };
    let process = Arc::new(Process::new_nt(pid, format, traced));
    let want = HandlesFor {
        caller: creator,
        process_access: req.process_access,
        thread_access: req.thread_access,
        inherit_handles: req.inherit_handles,
    };
    let (start, handles) = start_params(&process, &memory, &req.image, pid, &req.params, &sources, Some(&want))?;
    let (ph, th) = handles.expect("asked for");
    let first = ThreadRef(u64::from(pid));
    let created = poly_personality_nt::ProcessCreated {
        process: ph,
        thread: th,
        client: poly_personality_nt::ClientId { process: ProcessId(pid), thread: first },
    };
    if req.suspended {
        // CREATE_SUSPENDED (ADR 0089): the first thread starts when its
        // suspend count drops to zero (`sched::resume`).
        with(|s| {
            let c = s.ctl.entry(first).or_default();
            c.suspend_count = 1;
            c.deferred_start = Some((start.entry, start.teb));
            c.deferred_process = Some((process, memory));
        });
        return Ok(created);
    }
    match start_first_thread(&process, &memory, &start) {
        Ok(()) => Ok(created),
        Err(e) => {
            with(|s| {
                let _ = s.sys.objects.close(&creator, ph);
                let _ = s.sys.objects.close(&creator, th);
            });
            Err(e)
        }
    }
}

/// Start a new NT process's first thread in ring 3 at `start`; if it
/// cannot start, the process ends.
pub(super) fn start_first_thread(process: &Arc<Process>, memory: &Arc<Memory>, start: &Start) -> Result<(), NtStatus> {
    match user::spawn_nt(Arc::clone(process), Arc::clone(memory), start.entry, start.teb, user::DEFAULT_HOOK) {
        Ok(kthread) => {
            // The creator waits on the process object, not the thread.
            let _ = thread::detach(kthread);
            Ok(())
        }
        Err(_) => {
            process_exit(process.pid, memory);
            Err(poly_ntobj::status::STATUS_NO_MEMORY)
        }
    }
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
    let ended = with(|s| {
        if !s.processes.contains_key(&ProcessId(pid)) {
            return None;
        }
        let nt_status = s.ctl.get(&tref).and_then(|c| c.exit_status).unwrap_or_else(|| exit_to_nt(status));
        let proc = s.processes.get_mut(&ProcessId(pid)).expect("checked");
        let mut b = MemBackend { memory, sections: &mut s.sections };
        exec::thread_exit(&mut s.sys, proc, &mut b, tref, nt_status);
        s.ctl.remove(&tref);
        // Its pending I/O requests are cancelled (ADR 0067).
        s.sys.io.cancel_thread(tref);
        Some(nt_status)
    });
    // The last thread's status is the process's, unless NtTerminateProcess
    // gave one.
    if let Some(nt_status) = ended {
        note_exit_status(pid, nt_status, false);
    }
    sched::drain_wakes();
    io::drain();
}

/// The NT process `pid` has ended (its last thread is going): drop its
/// objects, views and handles. Its memory itself goes with `user::Memory`.
pub fn process_exit(pid: u32, memory: &Arc<Memory>) {
    with(|s| {
        s.created.remove(&ProcessId(pid));
        if let Some(mut proc) = s.processes.remove(&ProcessId(pid)) {
            let mut b = MemBackend { memory, sections: &mut s.sections };
            exec::process_exit(&mut s.sys, &mut proc, &mut b);
        }
        // Its registry notifications go too (ADR 0066).
        s.sys.registry.process_ended(ProcessId(pid), &mut s.sys.objects);
    });

    // Its file objects closed with its handles: deliver the cancellations.
    io::drain();
    // Its console handles too: a console it was the last on closes.
    console::drive();
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
    /// The image is not a loadable x64 PE (or the kernel has no `ntdll`):
    /// the `NTSTATUS`. Imports that cannot be bound end the started process
    /// instead, with the loader's status (ADR 0071).
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
