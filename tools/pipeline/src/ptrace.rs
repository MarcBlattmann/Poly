//! A minimal system call tracer on ptrace(2): it starts the test program
//! with an empty environment, stdin on /dev/null and stdout/stderr on
//! pipes, stops it at every syscall entry and exit (`PTRACE_SYSCALL`),
//! reads the registers and decodes the arguments, following forks and
//! clones. Signals are recorded and passed on.
//!
//! Tracees are attached with `PTRACE_SEIZE`, so job control works as
//! without a tracer: a stopped tracee stays stopped (`PTRACE_LISTEN`) and
//! its parent sees the stop and the continue.
//!
//! App tests (`apps.rs`) run their launcher chrooted into the test's file
//! tree, in a mount namespace of its own with the host's `/dev` and
//! `/proc` bound in ([`trace_in`]); as root directly, otherwise in a user
//! namespace that maps the caller to root. The tree's `/sys` stays empty:
//! the host's describes its own devices and security modules (a
//! `/sys/fs/selinux` that C libraries probe for), which say nothing about
//! the program.
//!
//! The trace starts after `execve` of the test program has succeeded, so
//! its first event is the program's own first syscall, which is also where
//! a Poly trace starts (the kernel loads the program; nobody calls execve).
//!
//! Only the documented ptrace interface is used (man 2 ptrace); register
//! names are the x86-64 `user_regs_struct` from the libc crate.

use crate::syscalls;
use crate::trace::{self, Event, Exit, Memory, Syscall, Trace};
use crate::util::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Exit statuses the child uses when it fails before running the program.
const CHILD_SETUP_FAILED: i32 = 124;
const CHILD_EXEC_FAILED: i32 = 127;
const CHILD_JAIL_FAILED: i32 = 123;

/// `personality(2)` flag: no address space randomisation, so raw traces of
/// two runs are identical (normalised traces are anyway).
const ADDR_NO_RANDOMIZE: libc::c_ulong = 0x0040000;

/// The running system, e.g. `linux 6.8.0-45-generic`.
pub fn host_system() -> String {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    format!("linux {}", release.trim())
}

/// Memory of a stopped tracee through `/proc/<pid>/mem`.
struct ProcMem(Option<File>);

impl ProcMem {
    fn open(pid: libc::pid_t) -> ProcMem {
        ProcMem(File::open(format!("/proc/{pid}/mem")).ok())
    }
}

impl Memory for ProcMem {
    fn read(&self, addr: u64, len: usize) -> Vec<u8> {
        let Some(file) = &self.0 else { return Vec::new() };
        let mut out = Vec::with_capacity(len);
        // Page by page, so an unmapped page ends the read where it starts.
        while out.len() < len {
            let at = addr.wrapping_add(out.len() as u64);
            let chunk = (4096 - (at % 4096) as usize).min(len - out.len());
            let mut buf = vec![0u8; chunk];
            match file.read_at(&mut buf, at) {
                Ok(n) if n > 0 => {
                    out.extend_from_slice(&buf[..n]);
                    if n < chunk {
                        break;
                    }
                }
                _ => break,
            }
        }
        out
    }
}

/// One traced process.
struct Proc {
    /// Index in order of appearance; 0 is the test program.
    index: u32,
    mem: ProcMem,
    /// Between syscall entry and exit.
    in_syscall: bool,
    /// The event of the syscall it is in, and its raw arguments.
    pending: Option<(usize, [u64; 6])>,
    /// Running the test program: false for the first process until its
    /// execve succeeds (what it does before is the tracer's setup).
    started: bool,
    /// The next syscall stop is the exit of that execve: not recorded.
    skip_exec_exit: bool,
}

fn os_error(what: &str) -> String {
    format!("{what}: {}", io::Error::last_os_error())
}

fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is a valid array of two ints for pipe2 to fill.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(os_error("pipe2"));
    }
    // SAFETY: pipe2 succeeded, so both descriptors are open and owned by nobody else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn ptrace(request: libc::c_uint, pid: libc::pid_t, data: usize) -> io::Result<()> {
    // SAFETY: the requests used here (SETOPTIONS, SYSCALL) take no pointer;
    // `data` is an option mask or a signal number.
    let r = unsafe { libc::ptrace(request, pid, std::ptr::null_mut::<libc::c_void>(), data) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn get_regs(pid: libc::pid_t) -> io::Result<libc::user_regs_struct> {
    // SAFETY: user_regs_struct is plain integers; all zeroes is a valid value.
    let mut regs: libc::user_regs_struct = unsafe { std::mem::zeroed() };
    // SAFETY: PTRACE_GETREGS writes one user_regs_struct to the pointer,
    // which points at `regs`; `pid` is a tracee in a ptrace-stop.
    let r =
        unsafe { libc::ptrace(libc::PTRACE_GETREGS, pid, std::ptr::null_mut::<libc::c_void>(), &mut regs as *mut _) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(regs) }
}

fn event_msg(pid: libc::pid_t) -> io::Result<u64> {
    let mut msg: libc::c_ulong = 0;
    // SAFETY: PTRACE_GETEVENTMSG writes one unsigned long to the pointer.
    let r = unsafe {
        libc::ptrace(libc::PTRACE_GETEVENTMSG, pid, std::ptr::null_mut::<libc::c_void>(), &mut msg as *mut _)
    };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(msg) }
}

/// `waitpid` for one `pid` with `flags` (plus `__WALL`), retried on EINTR.
/// With WNOHANG it returns pid 0 when nothing happened.
fn wait_pid(pid: libc::pid_t, flags: libc::c_int) -> io::Result<(libc::pid_t, libc::c_int)> {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid int for waitpid to fill.
        let r = unsafe { libc::waitpid(pid, &mut status, flags | libc::__WALL) };
        if r >= 0 {
            return Ok((r, status));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// A chroot for the program, prepared before fork.
struct Jail {
    root: CString,
    /// (host directory, where it goes in the tree), bound recursively.
    binds: Vec<(CString, CString)>,
    /// (host file, the file of the tree it is bound over), after `binds`.
    overlays: Vec<(CString, CString)>,
    /// Without root: (file, contents) for the new user namespace's maps.
    maps: Option<Vec<(CString, CString)>>,
}

impl Jail {
    fn new(root: &Path, overlays: &[(PathBuf, String)]) -> Result<Jail> {
        let c = |s: &[u8]| CString::new(s).map_err(|e| e.to_string());
        let mut binds = Vec::new();
        for dir in ["dev", "proc"] {
            binds.push((c(format!("/{dir}").as_bytes())?, c(root.join(dir).as_os_str().as_bytes())?));
        }
        let overlays = overlays
            .iter()
            .map(|(from, to)| Ok((c(from.as_os_str().as_bytes())?, c(root.join(to).as_os_str().as_bytes())?)))
            .collect::<Result<Vec<_>>>()?;
        // SAFETY: geteuid and getegid cannot fail and touch no memory.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        let maps = (uid != 0).then(|| {
            vec![
                (c(b"/proc/self/setgroups").unwrap(), c(b"deny").unwrap()),
                (c(b"/proc/self/uid_map").unwrap(), c(format!("0 {uid} 1\n").as_bytes()).unwrap()),
                (c(b"/proc/self/gid_map").unwrap(), c(format!("0 {gid} 1\n").as_bytes()).unwrap()),
            ]
        });
        Ok(Jail { root: c(root.as_os_str().as_bytes())?, binds, overlays, maps })
    }

    /// Enter the jail (in the child, before exec): new namespaces, the
    /// binds, chroot. Whether it worked.
    ///
    /// # Safety
    ///
    /// Only async-signal-safe calls on strings prepared before fork; call
    /// it only in the child of a fork.
    unsafe fn enter(&self) -> bool {
        let none = std::ptr::null::<libc::c_char>();
        // SAFETY: the caller is the forked child; every pointer is a
        // NUL-terminated string of `self` or NULL where mount(2) allows it.
        unsafe {
            let flags = if self.maps.is_some() { libc::CLONE_NEWUSER | libc::CLONE_NEWNS } else { libc::CLONE_NEWNS };
            if libc::unshare(flags) < 0 {
                return false;
            }
            for (file, text) in self.maps.iter().flatten() {
                let fd = libc::open(file.as_ptr(), libc::O_WRONLY);
                let len = text.as_bytes().len();
                if fd < 0 || libc::write(fd, text.as_ptr().cast(), len) != len as isize {
                    return false;
                }
                libc::close(fd);
            }
            // Private first, so nothing bound below reaches the host's namespace.
            if libc::mount(none, c"/".as_ptr(), none, libc::MS_REC | libc::MS_PRIVATE, std::ptr::null()) < 0 {
                return false;
            }
            for (from, to) in &self.binds {
                if libc::mount(from.as_ptr(), to.as_ptr(), none, libc::MS_BIND | libc::MS_REC, std::ptr::null()) < 0 {
                    return false;
                }
            }
            for (from, to) in &self.overlays {
                if libc::mount(from.as_ptr(), to.as_ptr(), none, libc::MS_BIND, std::ptr::null()) < 0 {
                    return false;
                }
            }
            libc::chroot(self.root.as_ptr()) == 0 && libc::chdir(c"/".as_ptr()) == 0
        }
    }
}

/// `waitpid(-1)` for the children and tracees of the calling thread only
/// (`__WNOTHREAD`), retried on EINTR. A tracee can leave the test's process
/// group (setpgid, setsid), so waiting for the group would miss its stops;
/// other threads of this process may be tracing other programs.
fn wait_any() -> io::Result<(libc::pid_t, libc::c_int)> {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid int for waitpid to fill.
        let r = unsafe { libc::waitpid(-1, &mut status, libc::__WALL | libc::__WNOTHREAD) };
        if r >= 0 {
            return Ok((r, status));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Send SIGKILL to the test's process group and to every tracee known to
/// be alive (some may have left the group).
fn kill_all(pgid: libc::pid_t, live: &Mutex<BTreeSet<libc::pid_t>>) {
    // SAFETY: kill has no memory-safety requirements. The group is the
    // tracee's (it called setpgid(0, 0)); the pids are tracees that the
    // tracer has not reaped, so they cannot have been reused.
    unsafe { libc::kill(-pgid, libc::SIGKILL) };
    for &pid in live.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

/// Everything the child needs, prepared before fork: after fork the child
/// may only make async-signal-safe calls, so nothing is allocated there.
struct ChildSetup {
    path: CString,
    argv: [*const libc::c_char; 2],
    envp: [*const libc::c_char; 1],
    stdin: libc::c_int,
    stdout: libc::c_int,
    stderr: libc::c_int,
    jail: Option<Jail>,
}

/// The child half of `fork`: own process group, the jail if any, standard
/// streams, default signal state, stop until the tracer has seized it
/// (`PTRACE_SEIZE`), exec. Never returns.
fn child(s: &ChildSetup) -> ! {
    // SAFETY: only async-signal-safe calls (setpgid, the jail's, dup2,
    // personality, signal, sigemptyset, sigprocmask, getpid, kill, execve,
    // _exit) on descriptors, a stack sigset_t and strings prepared before
    // fork; the argv/envp arrays are NULL-terminated and their strings
    // outlive the call.
    unsafe {
        libc::setpgid(0, 0);
        if let Some(jail) = &s.jail
            && !jail.enter()
        {
            libc::_exit(CHILD_JAIL_FAILED);
        }
        if libc::dup2(s.stdin, 0) < 0 || libc::dup2(s.stdout, 1) < 0 || libc::dup2(s.stderr, 2) < 0 {
            libc::_exit(CHILD_SETUP_FAILED);
        }
        let current = libc::personality(0xffff_ffff);
        if current >= 0 {
            libc::personality(current as libc::c_ulong | ADDR_NO_RANDOMIZE);
        }
        // The program starts as from a shell: default signal actions and an
        // empty mask. The Rust runtime ignores SIGPIPE, and an ignored
        // signal stays ignored across execve.
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        let mut none: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut none);
        libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
        libc::kill(libc::getpid(), libc::SIGSTOP);
        libc::execve(s.path.as_ptr(), s.argv.as_ptr(), s.envp.as_ptr());
        libc::_exit(CHILD_EXEC_FAILED);
    }
}

fn read_all(fd: OwnedFd) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut out = Vec::new();
        let _ = File::from(fd).read_to_end(&mut out);
        out
    })
}

/// Run `program` under the tracer. `test` names the trace (and is argv[0]).
pub fn trace(program: &Path, test: &str, timeout: Duration) -> Result<Trace> {
    trace_with(program.as_os_str().as_bytes(), test, test, None, timeout)
}

/// Run `program` (a path inside `root`, which is also its argv[0])
/// chrooted into `root`, under the tracer. `overlays` are host files bound
/// over files of the tree (paths relative to `root`) once the host's
/// `/dev` and `/proc` are bound in. `test` names the trace.
pub fn trace_in(
    root: &Path,
    overlays: &[(PathBuf, String)],
    program: &str,
    test: &str,
    timeout: Duration,
) -> Result<Trace> {
    trace_with(program.as_bytes(), program, test, Some(Jail::new(root, overlays)?), timeout)
}

fn trace_with(program: &[u8], argv0: &str, test: &str, jail: Option<Jail>, timeout: Duration) -> Result<Trace> {
    let path = CString::new(program).map_err(|e| e.to_string())?;
    let argv0 = CString::new(argv0).map_err(|e| e.to_string())?;
    let devnull = File::open("/dev/null").map_err(|e| format!("/dev/null: {e}"))?;
    let (out_r, out_w) = pipe()?;
    let (err_r, err_w) = pipe()?;
    let setup = ChildSetup {
        path,
        argv: [argv0.as_ptr(), std::ptr::null()],
        envp: [std::ptr::null()],
        stdin: devnull.as_raw_fd(),
        stdout: out_w.as_raw_fd(),
        stderr: err_w.as_raw_fd(),
        jail,
    };

    let start = Instant::now();
    // SAFETY: the child only runs `child`, which makes async-signal-safe
    // calls on data prepared above, so forking a multi-threaded process is sound.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(os_error("fork"));
    }
    if pid == 0 {
        child(&setup);
    }
    drop((out_w, err_w, devnull));
    let stdout = read_all(out_r);
    let stderr = read_all(err_r);

    // The watchdog kills every tracee if the run takes too long.
    let (done, done_rx) = mpsc::channel::<()>();
    let timed_out = Arc::new(AtomicBool::new(false));
    let live = Arc::new(Mutex::new(BTreeSet::from([pid])));
    let watchdog = {
        let timed_out = Arc::clone(&timed_out);
        let live = Arc::clone(&live);
        thread::spawn(move || {
            if done_rx.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout) {
                timed_out.store(true, Ordering::SeqCst);
                kill_all(pid, &live);
            }
        })
    };

    let mut tracer = Tracer {
        start,
        events: Vec::new(),
        procs: BTreeMap::new(),
        numbers: TaskNumbers::default(),
        exit: Exit::Unknown,
        live: Arc::clone(&live),
    };
    let result = tracer.run(pid);
    if result.is_err() {
        kill_all(pid, &live);
        while wait_any().is_ok() {}
    }
    let _ = done.send(());
    let _ = watchdog.join();
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    result?;

    let mut t = Trace::new(test, &host_system(), "ptrace");
    t.exit = if timed_out.load(Ordering::SeqCst) { Exit::Timeout } else { tracer.exit };
    t.events = tracer.events;
    t.stdout = String::from_utf8_lossy(&stdout).into_owned();
    t.stderr = String::from_utf8_lossy(&stderr).into_owned();
    t.duration_ns = start.elapsed().as_nanos() as u64;
    Ok(t)
}

/// Task numbers in creation order, the same however the host schedules
/// the tracer and its tracees.
///
/// A new task shows up twice: in its creator's fork, vfork or clone event
/// stop, and in its own first stop (a seized tracee's attach stop), in
/// either order. It may even run and exit before the tracer takes its
/// creator's event. The number goes with whichever comes first; the other
/// is recognised and never numbers the task again (a late event once gave
/// short-lived children a second, empty number and shifted every later
/// one). Both come before the creator can make its next task, so a
/// creator's tasks are numbered in the order it made them.
#[derive(Debug, Default)]
struct TaskNumbers {
    next: u32,
    /// Tasks numbered at their own first stop whose creator's event has
    /// not been taken yet. They may have exited meanwhile.
    awaiting_event: BTreeSet<libc::pid_t>,
}

impl TaskNumbers {
    fn take(&mut self) -> u32 {
        self.next += 1;
        self.next - 1
    }

    /// The test program: 0.
    fn first(&mut self) -> u32 {
        self.take()
    }

    /// A stop of `pid`, which has a number already if `known`: the number
    /// of a new task whose first stop came before its creator's event.
    fn stopped(&mut self, pid: libc::pid_t, known: bool) -> Option<u32> {
        if known {
            return None;
        }
        self.awaiting_event.insert(pid);
        Some(self.take())
    }

    /// A creator's event names its new task `pid`: the task's number if
    /// this came before the task's own first stop.
    fn created(&mut self, pid: libc::pid_t, known: bool) -> Option<u32> {
        if self.awaiting_event.remove(&pid) || known {
            return None;
        }
        Some(self.take())
    }
}

struct Tracer {
    start: Instant,
    events: Vec<Event>,
    procs: BTreeMap<libc::pid_t, Proc>,
    numbers: TaskNumbers,
    exit: Exit,
    /// Tracees not reaped yet, for the watchdog.
    live: Arc<Mutex<BTreeSet<libc::pid_t>>>,
}

impl Tracer {
    /// Trace task `pid` as number `index` from now on.
    fn add(&mut self, pid: libc::pid_t, index: u32, started: bool) {
        let p = Proc { index, mem: ProcMem::open(pid), in_syscall: false, pending: None, started, skip_exec_exit: false };
        self.procs.insert(pid, p);
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(pid);
    }

    fn run(&mut self, pid: libc::pid_t) -> Result {
        // The child stops itself before execve; seize it there, so the
        // tracer sees job-control stops as such (PTRACE_EVENT_STOP) and
        // can leave a stopped tracee stopped (PTRACE_LISTEN).
        let (_, status) = wait_pid(pid, libc::WUNTRACED).map_err(|e| format!("waitpid: {e}"))?;
        if libc::WIFEXITED(status) {
            return Err(match libc::WEXITSTATUS(status) {
                CHILD_SETUP_FAILED => "the test program's standard streams could not be set up".into(),
                CHILD_JAIL_FAILED => "the chroot for app tests is not permitted here (it needs root, or \
                                      unprivileged user namespaces)"
                    .into(),
                code => format!("the test program exited with {code} before it was traced"),
            });
        }
        if !libc::WIFSTOPPED(status) || libc::WSTOPSIG(status) != libc::SIGSTOP {
            return Err(format!("unexpected first stop of the tracee (status {status:#x})"));
        }
        let options = libc::PTRACE_O_TRACESYSGOOD
            | libc::PTRACE_O_EXITKILL
            | libc::PTRACE_O_TRACECLONE
            | libc::PTRACE_O_TRACEFORK
            | libc::PTRACE_O_TRACEVFORK
            | libc::PTRACE_O_TRACEEXEC;
        ptrace(libc::PTRACE_SEIZE, pid, options as usize).map_err(|e| {
            format!(
                "PTRACE_SEIZE: {e} (ptrace is not permitted here: container seccomp or Yama policy?); use \
                 --tracer strace or --tracer none"
            )
        })?;
        let first = self.numbers.first();
        self.add(pid, first, false);
        // SAFETY: kill has no memory-safety requirements; `pid` is our
        // child, not yet reaped.
        unsafe { libc::kill(pid, libc::SIGCONT) };

        while !self.procs.is_empty() {
            let (wpid, status) = match wait_any() {
                Ok(w) => w,
                Err(e) if e.raw_os_error() == Some(libc::ECHILD) => break,
                Err(e) => return Err(format!("waitpid: {e}")),
            };
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                self.live.lock().unwrap_or_else(|e| e.into_inner()).remove(&wpid);
                if let Some(p) = self.procs.remove(&wpid)
                    && p.index == 0
                {
                    if !p.started && libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == CHILD_EXEC_FAILED {
                        return Err("the test program could not be executed".into());
                    }
                    self.exit = if libc::WIFEXITED(status) {
                        Exit::Code(libc::WEXITSTATUS(status))
                    } else {
                        Exit::Signal(libc::WTERMSIG(status))
                    };
                }
                continue;
            }
            if !libc::WIFSTOPPED(status) {
                continue;
            }
            // A stop of a task not seen yet: a new child whose attach stop
            // came before its creator's event.
            if let Some(index) = self.numbers.stopped(wpid, self.procs.contains_key(&wpid)) {
                self.add(wpid, index, true);
            }
            let sig = libc::WSTOPSIG(status);
            let event = status >> 16;
            let started = self.procs[&wpid].started;
            let mut inject = 0;
            if sig == libc::SIGTRAP | 0x80 {
                if started {
                    self.syscall_stop(wpid)?;
                }
            } else if event == libc::PTRACE_EVENT_STOP {
                if matches!(sig, libc::SIGSTOP | libc::SIGTSTP | libc::SIGTTIN | libc::SIGTTOU) {
                    // A group-stop: the tracee stays stopped (its parent
                    // sees it with WUNTRACED) until SIGCONT or SIGKILL.
                    let _ = ptrace(libc::PTRACE_LISTEN, wpid, 0);
                    continue;
                }
                // The first stop of a new tracee, or the end of a
                // group-stop: it runs on.
            } else if sig == libc::SIGTRAP && event != 0 {
                match event {
                    libc::PTRACE_EVENT_FORK | libc::PTRACE_EVENT_VFORK | libc::PTRACE_EVENT_CLONE => {
                        if let Ok(child) = event_msg(wpid) {
                            let child = child as libc::pid_t;
                            if let Some(index) = self.numbers.created(child, self.procs.contains_key(&child)) {
                                self.add(child, index, true);
                            }
                        }
                    }
                    // The address space was replaced: reopen its memory.
                    // The first process's execve starts the test program.
                    libc::PTRACE_EVENT_EXEC => {
                        if let Some(p) = self.procs.get_mut(&wpid) {
                            p.mem = ProcMem::open(wpid);
                            if !p.started {
                                p.started = true;
                                p.skip_exec_exit = true;
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                // A signal: recorded once the program runs (before, it is
                // the tracer's own SIGCONT), and passed on.
                if started {
                    let index = self.procs[&wpid].index;
                    self.events.push(Event::Signal { pid: index, signal: sig });
                }
                inject = sig as usize;
            }
            // ESRCH: it died in the meantime; its exit status comes next.
            let _ = ptrace(libc::PTRACE_SYSCALL, wpid, inject);
        }
        Ok(())
    }

    fn syscall_stop(&mut self, pid: libc::pid_t) -> Result {
        let regs = match get_regs(pid) {
            Ok(r) => r,
            // Killed between the stop and now; its exit status comes next.
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => return Ok(()),
            Err(e) => return Err(format!("PTRACE_GETREGS: {e}")),
        };
        let t_ns = self.start.elapsed().as_nanos() as u64;
        let p = self.procs.get_mut(&pid).unwrap();
        if p.skip_exec_exit {
            p.skip_exec_exit = false;
            if regs.orig_rax == libc::SYS_execve as u64 {
                return Ok(());
            }
        }
        if !p.in_syscall {
            let raw = [regs.rdi, regs.rsi, regs.rdx, regs.r10, regs.r8, regs.r9];
            let nr = regs.orig_rax;
            let name = syscalls::name(nr);
            let args = trace::decode_entry(&name, &raw, &p.mem);
            p.pending = Some((self.events.len(), raw));
            p.in_syscall = true;
            self.events.push(Event::Syscall(Syscall { pid: p.index, nr, name, args, ret: None, t_ns }));
        } else {
            p.in_syscall = false;
            if let Some((i, raw)) = p.pending.take()
                && let Event::Syscall(c) = &mut self.events[i]
            {
                let ret = regs.rax as i64;
                c.ret = Some(ret);
                trace::decode_exit(&c.name, &raw, ret, &mut c.args, &p.mem);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus;
    use crate::util;

    /// A child that leaves the process group, stops and is continued, and a
    /// child killed by SIGPIPE: the program sees what it would see without
    /// a tracer, and the run neither hangs nor leaves processes behind.
    #[test]
    fn job_control_and_process_groups_work_under_the_tracer() {
        let cc = corpus::compiler(None);
        if corpus::compiler_version(&cc).is_err() {
            eprintln!("skipped: no C compiler");
            return;
        }
        let root = crate::repo_root();
        let dir = std::env::temp_dir().join(format!("poly-ptrace-test-{}", std::process::id()));
        let source = dir.join("jobctl.c");
        let binary = dir.join("jobctl");
        util::write(
            &source,
            "#include \"poly_ipc.h\"\n\
             int test_main(void) {\n\
                 i64 pid = fork();\n\
                 if (pid == 0) { syscall2(SYS_setpgid, 0, 0); raise_sig(SIGSTOP); exit_group(3); }\n\
                 int st = 0;\n\
                 if (syscall4(SYS_wait4, pid, &st, 2, 0) != pid || (st & 0xff) != 0x7f) return 10;\n\
                 syscall2(SYS_kill, pid, SIGCONT);\n\
                 if (wait_exit(pid) != 3) return 11;\n\
                 pid = fork();\n\
                 if (pid == 0) { raise_sig(SIGPIPE); exit_group(4); }\n\
                 st = wait_status(pid);\n\
                 if (!W_SIGNALED(st) || W_TERMSIG(st) != SIGPIPE) return 12;\n\
                 return 0;\n\
             }\n",
        )
        .unwrap();
        let built = std::process::Command::new(&cc)
            .args(corpus::FREESTANDING_FLAGS)
            .arg("-o")
            .arg(&binary)
            .arg(&source)
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(built.success());
        let t = trace(&binary, "jobctl", Duration::from_secs(20)).unwrap();
        assert_eq!(t.exit, Exit::Code(0), "stdout: {}", t.stdout);
        let names: Vec<&str> = t
            .events
            .iter()
            .filter_map(|e| if let Event::Syscall(c) = e { Some(c.name.as_str()) } else { None })
            .collect();
        assert!(names.contains(&"setpgid") && names.contains(&"wait4"), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every order in which a new task's two notifications and its exit
    /// can reach the tracer gives it one number.
    #[test]
    fn task_numbers_whichever_notification_comes_first() {
        let mut n = TaskNumbers::default();
        assert_eq!(n.first(), 0);
        // The event first, then the child's first stop.
        assert_eq!(n.created(101, false), Some(1));
        assert_eq!(n.stopped(101, true), None);
        // The child's first stop first, then the event.
        assert_eq!(n.stopped(102, false), Some(2));
        assert_eq!(n.created(102, true), None);
        // The child's first stop, its whole life and its exit (no longer
        // known), then the event.
        assert_eq!(n.stopped(103, false), Some(3));
        assert_eq!(n.created(103, false), None);
        // The next task gets the next number, and nothing is left waiting.
        assert_eq!(n.created(104, false), Some(4));
        assert!(n.awaiting_event.is_empty());
    }

    /// Build `source` (corpus format) as `name` in a fresh directory.
    fn build(name: &str, source: &str) -> Option<(PathBuf, PathBuf)> {
        let cc = corpus::compiler(None);
        if corpus::compiler_version(&cc).is_err() {
            eprintln!("skipped: no C compiler");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("poly-ptrace-{name}-{}", std::process::id()));
        let (src, binary) = (dir.join(format!("{name}.c")), dir.join(name));
        util::write(&src, source).unwrap();
        let built = std::process::Command::new(&cc)
            .args(corpus::FREESTANDING_FLAGS)
            .arg("-o")
            .arg(&binary)
            .arg(&src)
            .current_dir(crate::repo_root())
            .status()
            .unwrap();
        assert!(built.success());
        Some((dir, binary))
    }

    /// Children that exit at once, forked by a process that is itself a
    /// child of the test program: the tracer often sees such a child's
    /// first stop, and even its exit, before its parent's fork event. Every
    /// task still gets exactly one number, in creation order: 0 the
    /// program, 1 its child, 2.. the grandchildren, and each has events.
    #[test]
    fn a_child_that_exits_before_its_fork_event_is_numbered_once() {
        const CHILDREN: u32 = 40;
        let source = format!(
            "#include \"poly_ipc.h\"\n\
             int test_main(void) {{\n\
                 i64 pid = fork();\n\
                 if (pid == 0) {{\n\
                     for (int i = 0; i < {CHILDREN}; i++) {{\n\
                         i64 c = fork();\n\
                         if (c == 0) exit_group(0);\n\
                         if (wait_exit(c) != 0) exit_group(20);\n\
                     }}\n\
                     exit_group(0);\n\
                 }}\n\
                 return wait_exit(pid);\n\
             }}\n"
        );
        let Some((dir, binary)) = build("quickchildren", &source) else { return };
        // Several runs: which notification comes first is up to the host.
        for _ in 0..5 {
            let t = trace(&binary, "quickchildren", Duration::from_secs(20)).unwrap();
            assert_eq!(t.exit, Exit::Code(0), "stdout: {}", t.stdout);
            let mut seen = BTreeSet::new();
            for e in &t.events {
                seen.insert(match e {
                    Event::Syscall(c) => c.pid,
                    Event::Signal { pid, .. } => *pid,
                });
            }
            let want: BTreeSet<u32> = (0..CHILDREN + 2).collect();
            assert_eq!(seen, want, "task numbers with events");
            // Each grandchild's only call is exit_group, in its own number.
            let exits: Vec<u32> = t
                .events
                .iter()
                .filter_map(|e| match e {
                    Event::Syscall(c) if c.name == "exit_group" && c.pid >= 2 => Some(c.pid),
                    _ => None,
                })
                .collect();
            let order: Vec<u32> = (2..CHILDREN + 2).collect();
            assert_eq!(exits[..CHILDREN as usize], order[..], "grandchildren in creation order");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
