//! Processes: the record every user thread belongs to, its threads, how it
//! ends, and who waits for it.
//!
//! The personality is decided once, at exec, from the image's binary format,
//! and every system call the process makes is routed by it.
//!
//! # Threads and the end of a process
//!
//! Every user thread registers with its process when it is started
//! ([`Process::attach_thread`], from `user::spawn_in`) and gets a thread id
//! from the same number space as process ids (the first thread's id is the
//! process id, as on Linux). It deregisters as it ends
//! ([`Process::detach_thread`]); the last one to go ends the process: its
//! files are closed, its [`ExitStatus`] is set and everyone in
//! [`Process::wait`] wakes up.
//!
//! A thread can only end itself. Ending a whole process (`exit_group`, a
//! fatal signal) therefore sets [`Process::is_exiting`] and wakes every
//! parked thread ([`park`]); each thread leaves the next time it is about to
//! return to user mode (after a system call, an interrupt or an exception
//! in ring 3), so a thread spinning in user mode leaves at the next timer
//! tick. A thread blocked in another kernel wait (a mutex, a sleep) leaves
//! when that wait ends.
//!
//! # Process table
//!
//! Processes with threads are listed by pid ([`find`], [`find_thread`],
//! [`all`]) for the calls that name other processes (kill, futex wakes).
//! Process ids come from [`alloc_pid`]: 1 for the first process
//! (`poly.init`), counting up. Kernel tests make bare records with fixed
//! pids of their own ([`Process::new`]); they are listed too, but nothing
//! looks them up.

use crate::linux::LinuxState;
use crate::sync::SpinLock;
use crate::thread::WaitQueue;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use poly_abi::Personality;
use poly_binfmt as binfmt;

#[derive(Debug)]
pub struct Process {
    pub pid: u32,
    /// The parent's process id when the process was made; 0 for processes
    /// the kernel started. A Linux process's current parent (after its
    /// parent ended) is in the personality's process tree (`linux::task`).
    pub ppid: u32,
    pub personality: Personality,
    pub format: binfmt::Format,
    /// Open files, root and working directory, umask, credentials
    /// (`fs::Files`): `/dev/console` on fds 0, 1 and 2 to start with.
    pub files: crate::fs::Files,
    /// The Linux personality's state (signals, futex robust lists) for a
    /// process made with [`Process::new_linux`]. `None` for bare records
    /// ([`Process::new`]): their system calls go to the personality
    /// without kernel services, and a fault kills only the faulting thread.
    pub linux: Option<LinuxState>,
    /// The native personality's state (its children) for a process made
    /// with [`Process::new_native`] (ADR 0125); `None` otherwise.
    pub native: Option<crate::native::NativeState>,
    /// The process reports its system calls, output, signals and exit on
    /// the serial port (`poly.test.capture`, ADR 0027).
    pub traced: bool,
    /// CPU time of its ended threads and waited-for children (ADR 0044).
    pub cpu: crate::cputime::ProcessCpu,
    threads: SpinLock<Threads>,
    /// The process has ended: [`Process::status`] says how.
    ended: AtomicBool,
    /// The process is being ended as a whole ([`Process::group_exit`]).
    exiting: AtomicBool,
    /// The process is stopped ([`Process::stop`]).
    stopped: AtomicBool,
    /// Live threads (the length of `threads.live`), readable without the
    /// lock.
    live: AtomicU32,
}

/// How a process (or a thread) ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// It called `exit` / `exit_group` (or a test hook's exit) with this
    /// status.
    Exited(u8),
    /// A signal killed it. `core` says a core dump was written (never yet).
    Signaled { signal: u32, core: bool },
    /// A CPU exception killed it (bare processes, whose faults are not
    /// turned into signals): the vector.
    Faulted(u8),
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            ExitStatus::Exited(code) => write!(f, "exit status {code}"),
            ExitStatus::Signaled { signal, .. } => write!(f, "killed by signal {signal}"),
            ExitStatus::Faulted(vector) => write!(f, "killed by CPU exception {vector}"),
        }
    }
}

/// Per-thread flags other threads set: the wake-ups of [`park`].
#[derive(Debug, Default)]
pub struct ThreadFlags {
    /// The thread has a signal to act on (`LinuxKernel::notify_signal`);
    /// cleared by the thread itself.
    pub notified: AtomicBool,
    /// An unpark is waiting to be consumed (`LinuxKernel::unblock`).
    pub unparked: AtomicBool,
    /// The thread must end, though its process goes on (another thread's
    /// `execve`, [`Process::end_other_threads`]).
    pub killed: AtomicBool,
    /// The thread is suspended (NT `NtSuspendThread`): it parks before it
    /// next runs user code until the count reaches zero (`crate::nt`).
    pub suspended: AtomicBool,
    /// Its kernel thread (`ThreadId::to_bits` + 1; 0 until it is known), to
    /// interrupt it when it runs on another CPU ([`ThreadFlags::kick`]).
    pub thread: AtomicU64,
    /// The thread has an rseq(2) area (ADR 0111): switching it out sets
    /// [`rseq_notify`](Self::rseq_notify).
    pub rseq: AtomicBool,
    /// The thread with an rseq area was switched out: before it returns to
    /// user mode the personality looks at the area
    /// (`Linux::rseq_resume`). Cleared by the thread itself.
    pub rseq_notify: AtomicBool,
}

impl ThreadFlags {
    /// The thread is being switched out (by the scheduler, with interrupts
    /// off): one with an rseq area must look at it before it next runs
    /// user code.
    pub fn switched_out(&self) {
        if self.rseq.load(Ordering::Relaxed) {
            self.rseq_notify.store(true, Ordering::Relaxed);
        }
    }

    /// Make the thread look at its flags soon: if it runs on another CPU
    /// (maybe in ring 3, where it would not look until its next interrupt
    /// or system call), interrupt that CPU; on its way back to user mode it
    /// sees what was set. A thread that is not running sees it when it
    /// next runs; a parked one is woken by the caller.
    pub fn kick(&self) {
        match self.thread.load(Ordering::SeqCst) {
            0 => {}
            bits => crate::thread::kick_thread(crate::thread::ThreadId::from_bits(bits - 1)),
        }
    }
}

#[derive(Debug, Default)]
struct Threads {
    /// Live threads: id and flags, oldest first.
    live: Vec<(u32, Arc<ThreadFlags>)>,
    /// Threads ever attached.
    started: u32,
    /// The status [`Process::group_exit`] asked for (the first one wins).
    group_exit: Option<ExitStatus>,
    /// The own status of the thread that started to exit last
    /// ([`Process::thread_exiting`]): the process's status without a group
    /// exit.
    last_exiting: Option<ExitStatus>,
    /// Live threads that have started to exit ([`Process::thread_exiting`]).
    exiting: Vec<u32>,
    /// Set when the last thread has gone.
    status: Option<ExitStatus>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExecError {
    Format(binfmt::Error),
    /// `#!` scripts are resolved by the exec path, not here.
    Script,
}

/// Threads waiting for a process to end, to be continued, or in [`park`]:
/// one queue, woken whole; each waiter checks its own condition.
static EVENTS: WaitQueue = WaitQueue::new();

/// The process table: every process that has (or had) threads and has not
/// ended, by pid.
static TABLE: SpinLock<BTreeMap<u32, Weak<Process>>> = SpinLock::new(BTreeMap::new());

static NEXT_PID: AtomicU32 = AtomicU32::new(1);

/// A new process (and thread) id: 1 first, then counting up.
pub fn alloc_pid() -> u32 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

impl Process {
    /// A bare process record for a personality and format decided
    /// elsewhere, without personality state (kernel tests).
    pub fn new(pid: u32, personality: Personality, format: binfmt::Format) -> Process {
        Process::with_state(pid, 0, personality, format, None, false, crate::fs::Files::with_console())
    }

    /// A Linux process with its personality state, whose system calls get
    /// the kernel's services (`crate::linux`) and whose faults become
    /// signals. `traced`: it reports what it does on the serial port
    /// (ADR 0027).
    pub fn new_linux(pid: u32, ppid: u32, format: binfmt::Format, traced: bool) -> Process {
        let linux = LinuxState::new(pid);
        // Its signals are seen as a tracer sees them (ADR 0027); children inherit it.
        linux.with(|p| p.traced = traced);
        let linux = Some(linux);
        // A traced test run keeps the kernel log's console (ADR 0046).
        let files = if traced { crate::tty::log_console_files() } else { crate::fs::Files::with_console() };
        Process::with_state(pid, ppid, Personality::Linux, format, linux, traced, files)
    }

    /// An NT process for a PE executable (`crate::nt`): no Linux state (its
    /// system calls go to the NT personality), the console on fds 0, 1 and
    /// 2. Its per-process NT state lives in `crate::nt`, keyed by pid.
    pub fn new_nt(pid: u32, format: binfmt::Format, traced: bool) -> Process {
        let files = if traced { crate::tty::log_console_files() } else { crate::fs::Files::with_console() };
        Process::with_state(pid, 0, Personality::Nt, format, None, traced, files)
    }

    /// A native Poly process (ADR 0125): its system calls go to the native
    /// personality (`crate::native`), the console on fds 0, 1 and 2 (the
    /// kernel's log console when `traced`).
    pub fn new_native(pid: u32, ppid: u32, format: binfmt::Format, traced: bool) -> Process {
        let files = if traced { crate::tty::log_console_files() } else { crate::fs::Files::with_console() };
        let mut p = Process::with_state(pid, ppid, Personality::Native, format, None, traced, files);
        p.native = Some(crate::native::NativeState::default());
        p
    }

    /// A Linux process made by `fork` / `clone` (`linux::task`): its files
    /// and personality state come from the parent.
    pub fn new_linux_child(
        pid: u32,
        ppid: u32,
        format: binfmt::Format,
        traced: bool,
        files: crate::fs::Files,
        linux: LinuxState,
    ) -> Process {
        Process::with_state(pid, ppid, Personality::Linux, format, Some(linux), traced, files)
    }

    fn with_state(
        pid: u32,
        ppid: u32,
        personality: Personality,
        format: binfmt::Format,
        linux: Option<LinuxState>,
        traced: bool,
        files: crate::fs::Files,
    ) -> Process {
        Process {
            pid,
            ppid,
            personality,
            format,
            files,
            linux,
            native: None,
            traced,
            cpu: crate::cputime::ProcessCpu::default(),
            threads: SpinLock::new(Threads::default()),
            ended: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            live: AtomicU32::new(0),
        }
    }

    /// Create the process record for an image. Loading and mapping the
    /// image is the personality's job and comes later.
    pub fn from_image(pid: u32, image: &[u8]) -> Result<Process, ExecError> {
        let format = binfmt::detect(image).map_err(ExecError::Format)?;
        let personality = format.personality().ok_or(ExecError::Script)?;
        Ok(Process::new(pid, personality, format))
    }

    /// Register a new thread: its id (the process id for the first one, a
    /// fresh id otherwise) and its flags. `None` if the process has ended.
    pub fn attach_thread(self: &Arc<Self>) -> Option<(u32, Arc<ThreadFlags>)> {
        self.attach_thread_as(None)
    }

    /// [`attach_thread`](Self::attach_thread) with the id given (`Some`: an
    /// id from [`alloc_pid`] the caller has prepared state for, a `clone`
    /// child's).
    pub fn attach_thread_as(self: &Arc<Self>, id: Option<u32>) -> Option<(u32, Arc<ThreadFlags>)> {
        let flags = Arc::new(ThreadFlags::default());
        let (tid, first) = {
            let mut t = self.threads.lock();
            if t.status.is_some() {
                return None;
            }
            let tid = id.unwrap_or_else(|| if t.started == 0 { self.pid } else { alloc_pid() });
            t.started += 1;
            t.live.push((tid, Arc::clone(&flags)));
            self.live.store(t.live.len() as u32, Ordering::SeqCst);
            (tid, t.live.len() == 1)
        };
        if first {
            TABLE.lock().insert(self.pid, Arc::downgrade(self));
        }
        if let Some(linux) = &self.linux {
            linux.add_thread(tid);
            // A fork child keeps its creator's rseq area (ADR 0111); it may
            // start on another CPU, so it looks at the area first.
            if linux.with(|p| p.thread(tid as i32).rseq.is_some()) {
                flags.rseq.store(true, Ordering::Relaxed);
                flags.rseq_notify.store(true, Ordering::Relaxed);
            }
        }
        Some((tid, flags))
    }

    /// Undo [`attach_thread`](Self::attach_thread) for a thread that could
    /// not be started: it never ran, so the process does not end because
    /// of it.
    pub fn cancel_thread(&self, tid: u32) {
        if let Some(linux) = &self.linux {
            linux.remove_thread(self.pid, tid);
        }
        let empty = {
            let mut t = self.threads.lock();
            t.live.retain(|(id, _)| *id != tid);
            self.live.store(t.live.len() as u32, Ordering::SeqCst);
            t.live.is_empty()
        };
        if empty {
            let mut table = TABLE.lock();
            if table.get(&self.pid).is_some_and(|w| core::ptr::eq(w.as_ptr(), self)) {
                table.remove(&self.pid);
            }
        }
    }

    /// Thread `tid` has ended with `own` (its exit status, or how it was
    /// killed). If it was the last one, the process ends: with the status
    /// [`group_exit`](Self::group_exit) asked for, else `own`; its files are
    /// closed and it leaves the process table. Returns the process's status
    /// if it ended: the caller finishes its personality's part of the exit
    /// and then calls [`ended`](Self::ended), which wakes its waiters.
    pub fn detach_thread(&self, tid: u32, own: ExitStatus) -> Option<ExitStatus> {
        if let Some(linux) = &self.linux {
            linux.remove_thread(self.pid, tid);
        }
        let status = {
            let mut t = self.threads.lock();
            t.live.retain(|(id, _)| *id != tid);
            t.exiting.retain(|&id| id != tid);
            self.live.store(t.live.len() as u32, Ordering::SeqCst);
            if !t.live.is_empty() || t.status.is_some() {
                drop(t);
                // A thread may wait for the others to go (`execve`).
                EVENTS.wake_all();
                return None;
            }
            let status = t.group_exit.or(t.last_exiting).unwrap_or(own);
            t.status = Some(status);
            status
        };
        self.files.close_all();
        {
            let mut table = TABLE.lock();
            if table.get(&self.pid).is_some_and(|w| core::ptr::eq(w.as_ptr(), self)) {
                table.remove(&self.pid);
            }
        }
        Some(status)
    }

    /// Thread `tid` starts to exit with `own`, before its last duties
    /// (clearing its `clear_child_tid` word, which another thread may be
    /// watching) and before [`detach_thread`](Self::detach_thread). The
    /// process's status, unless a group exit chose one, is that of the
    /// thread that started to exit last (the spec of exit: "the process
    /// ends when its last thread exits, and wait4 then reports the last
    /// thread's code"), whichever thread happens to detach last.
    pub fn thread_exiting(&self, tid: u32, own: ExitStatus) {
        let mut t = self.threads.lock();
        if t.live.iter().any(|(id, _)| *id == tid) {
            t.last_exiting = Some(own);
            if !t.exiting.contains(&tid) {
                t.exiting.push(tid);
            }
        }
    }

    /// Whether the process has ended or every live thread has started to
    /// exit (it no longer runs user code: pidfd_getfd(2) and
    /// process_vm_readv(2) treat it as gone, as once its last thread has
    /// cleared its `clear_child_tid` word).
    pub fn is_ending(&self) -> bool {
        if self.has_ended() {
            return true;
        }
        let t = self.threads.lock();
        t.status.is_some() || t.live.iter().all(|(id, _)| t.exiting.contains(id))
    }

    /// The process's exit is complete (after [`detach_thread`] and the
    /// personality's part): wake whoever waits for it ([`wait`]). Last, so
    /// that a waiter on another CPU sees everything the exit did.
    ///
    /// [`detach_thread`]: Self::detach_thread
    /// [`wait`]: Self::wait
    pub fn ended(&self) {
        self.ended.store(true, Ordering::SeqCst);
        EVENTS.wake_all();
    }

    /// End the whole process with `status` (exit_group(2), a fatal signal):
    /// every thread leaves at its next return to user mode, and parked
    /// threads wake now. The first request decides the status. Returns it.
    pub fn group_exit(&self, status: ExitStatus) -> ExitStatus {
        let status = {
            let mut t = self.threads.lock();
            *t.group_exit.get_or_insert(status)
        };
        self.exiting.store(true, Ordering::SeqCst);
        EVENTS.wake_all();
        self.kick_threads(None);
        status
    }

    /// [`ThreadFlags::kick`] every live thread but `except`.
    fn kick_threads(&self, except: Option<u32>) {
        let flags: Vec<Arc<ThreadFlags>> =
            self.threads.lock().live.iter().filter(|(id, _)| Some(*id) != except).map(|(_, f)| Arc::clone(f)).collect();
        for f in flags {
            f.kick();
        }
    }

    /// Whether the process is being ended as a whole: its threads must not
    /// return to user mode.
    pub fn is_exiting(&self) -> bool {
        self.exiting.load(Ordering::SeqCst)
    }

    /// How the process ended, once it has.
    pub fn status(&self) -> Option<ExitStatus> {
        self.threads.lock().status
    }

    /// Whether the process has ended.
    pub fn has_ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    /// Block until the process has ended; returns how.
    pub fn wait(&self) -> ExitStatus {
        EVENTS.wait_until(|| self.ended.load(Ordering::SeqCst));
        self.status().expect("an ended process has a status")
    }

    /// [`wait`](Self::wait), giving up at `deadline_ns` on the monotonic
    /// clock (`None` then).
    pub fn wait_until(&self, deadline_ns: u64) -> Option<ExitStatus> {
        if EVENTS.wait_until_deadline(|| self.ended.load(Ordering::SeqCst), deadline_ns) { self.status() } else { None }
    }

    /// The flags of live thread `tid`.
    pub fn thread_flags(&self, tid: u32) -> Option<Arc<ThreadFlags>> {
        self.threads.lock().live.iter().find(|(id, _)| *id == tid).map(|(_, f)| Arc::clone(f))
    }

    /// Live threads.
    pub fn thread_count(&self) -> usize {
        self.threads.lock().live.len()
    }

    /// End every thread but `tid` (`execve`): each leaves the next time it
    /// would return to user mode, parked ones at once; the process goes
    /// on. Then wait until they are gone (or the process is being ended
    /// as a whole, or `tid`, whose `flags` these are, is ended by another
    /// thread's `execve`). Returns whether `tid` is alone now.
    pub fn end_other_threads(&self, tid: u32, flags: &ThreadFlags) -> bool {
        {
            let t = self.threads.lock();
            for (id, flags) in &t.live {
                if *id != tid {
                    flags.killed.store(true, Ordering::SeqCst);
                }
            }
        }
        EVENTS.wake_all();
        self.kick_threads(Some(tid));
        let ended = || self.exiting.load(Ordering::SeqCst) || flags.killed.load(Ordering::SeqCst);
        EVENTS.wait_until(|| self.live.load(Ordering::SeqCst) <= 1 || ended());
        self.live.load(Ordering::SeqCst) == 1 && !ended()
    }

    /// Give the live thread `old` the id `new` (an `execve` by a thread
    /// other than the main one takes over the process id). Returns whether
    /// it was there.
    pub fn rename_thread(&self, old: u32, new: u32) -> bool {
        let mut t = self.threads.lock();
        match t.live.iter_mut().find(|(id, _)| *id == old) {
            Some(entry) => {
                entry.0 = new;
                true
            }
            None => false,
        }
    }

    /// Stop the process (a stop signal's default action): its threads wait
    /// in [`wait_while_stopped`](Self::wait_while_stopped) at their next
    /// return to user mode until [`cont`](Self::cont).
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.kick_threads(None);
    }

    /// Continue a stopped process. Returns whether it was stopped.
    pub fn cont(&self) -> bool {
        let was = self.stopped.swap(false, Ordering::SeqCst);
        if was {
            EVENTS.wake_all();
        }
        was
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Block while the process is stopped (and not being ended).
    pub fn wait_while_stopped(&self) {
        EVENTS.wait_until(|| !self.stopped.load(Ordering::SeqCst) || self.exiting.load(Ordering::SeqCst));
    }
}

/// Why [`park`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unparked {
    /// [`unpark`] was called (perhaps before the park).
    Woken,
    /// The deadline passed.
    TimedOut,
    /// The thread's signal notification is set, or its process is being
    /// ended.
    Interrupted,
}

/// Block the calling thread (of `process`, with `flags`) until [`unpark`],
/// its signal notification, the end of its process, or `deadline_ns` on
/// the monotonic clock. A pending unpark is consumed and returns at once.
pub fn park(process: &Process, flags: &ThreadFlags, deadline_ns: Option<u64>) -> Unparked {
    let ready = || {
        flags.notified.load(Ordering::SeqCst)
            || flags.unparked.load(Ordering::SeqCst)
            || flags.killed.load(Ordering::SeqCst)
            || process.exiting.load(Ordering::SeqCst)
    };
    let in_time = match deadline_ns {
        Some(d) => EVENTS.wait_until_deadline(ready, d),
        None => {
            EVENTS.wait_until(ready);
            true
        }
    };
    if flags.notified.load(Ordering::SeqCst) || flags.killed.load(Ordering::SeqCst) || process.is_exiting() {
        Unparked::Interrupted
    } else if flags.unparked.swap(false, Ordering::SeqCst) {
        Unparked::Woken
    } else if in_time {
        // Cannot happen (one of the conditions held); harmless either way.
        Unparked::Woken
    } else {
        Unparked::TimedOut
    }
}

/// Block the calling thread until `ready()` holds. It is checked again
/// whenever anything a parked thread may wait for happens: a process
/// ends, stops or continues, a signal notification, an unpark, a group
/// exit or an `execve` ending threads, and [`wake_waiters`]. So a wait
/// that also checks those (a `wait4`, a vfork parent) is interrupted by
/// them like [`park`].
pub fn wait_event(ready: impl FnMut() -> bool) {
    EVENTS.wait_until(ready);
}

/// Make every thread in [`wait_event`] (and [`park`]) check its condition
/// again.
pub fn wake_waiters() {
    EVENTS.wake_all();
}

/// Wake thread `flags` from [`park`], or make its next park return at once.
pub fn unpark(flags: &ThreadFlags) {
    flags.unparked.store(true, Ordering::SeqCst);
    EVENTS.wake_all();
}

/// Set thread `flags`' signal notification and end its [`park`].
pub fn notify(flags: &ThreadFlags) {
    flags.notified.store(true, Ordering::SeqCst);
    EVENTS.wake_all();
    flags.kick();
}

/// The process with id `pid`, if it is in the table.
pub fn find(pid: u32) -> Option<Arc<Process>> {
    TABLE.lock().get(&pid).and_then(Weak::upgrade)
}

/// The process that has a live thread `tid`, and the thread's flags.
pub fn find_thread(tid: u32) -> Option<(Arc<Process>, Arc<ThreadFlags>)> {
    // The thread with the process id is the main thread: try that first.
    if let Some(p) = find(tid)
        && let Some(f) = p.thread_flags(tid)
    {
        return Some((p, f));
    }
    all().into_iter().find_map(|p| {
        let f = p.thread_flags(tid)?;
        Some((p, f))
    })
}

/// List `process` in the table without a thread of its own: the kernel's
/// init of a traced test run (`testinit.rs`, ADR 0064), whose work a
/// kernel thread does. The caller keeps it alive (the table is weak).
pub fn register_threadless(process: &Arc<Process>) {
    TABLE.lock().insert(process.pid, Arc::downgrade(process));
}

/// Every process in the table, by pid.
pub fn all() -> Vec<Arc<Process>> {
    let weak: Vec<Weak<Process>> = TABLE.lock().values().cloned().collect();
    weak.iter().filter_map(Weak::upgrade).collect()
}
