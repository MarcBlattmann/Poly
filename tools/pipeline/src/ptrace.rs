//! A minimal system call tracer on ptrace(2): it starts the test program
//! with an empty environment, stdin on /dev/null and stdout/stderr on
//! pipes, stops it at every syscall entry and exit (`PTRACE_SYSCALL`),
//! reads the registers and decodes the arguments, following forks and
//! clones. Signals are recorded and passed on.
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
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Exit statuses the child uses when it fails before running the program.
const CHILD_SETUP_FAILED: i32 = 124;
const CHILD_TRACEME_FAILED: i32 = 125;
const CHILD_EXEC_FAILED: i32 = 127;

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
    /// Newly attached: its first SIGSTOP is the attach stop, not a signal.
    fresh: bool,
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

/// `waitpid` for `pid` (negative: a process group), retried on EINTR.
fn wait(pid: libc::pid_t) -> io::Result<(libc::pid_t, libc::c_int)> {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid int for waitpid to fill.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::__WALL) };
        if r >= 0 {
            return Ok((r, status));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
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
}

/// The child half of `fork`: own process group, standard streams, ask to
/// be traced, exec. Never returns.
fn child(s: &ChildSetup) -> ! {
    // SAFETY: only async-signal-safe calls (setpgid, dup2, personality,
    // ptrace, execve, _exit) on descriptors and strings prepared before
    // fork; the argv/envp arrays are NULL-terminated and their strings
    // outlive the call.
    unsafe {
        libc::setpgid(0, 0);
        if libc::dup2(s.stdin, 0) < 0 || libc::dup2(s.stdout, 1) < 0 || libc::dup2(s.stderr, 2) < 0 {
            libc::_exit(CHILD_SETUP_FAILED);
        }
        let current = libc::personality(0xffff_ffff);
        if current >= 0 {
            libc::personality(current as libc::c_ulong | ADDR_NO_RANDOMIZE);
        }
        let none = std::ptr::null_mut::<libc::c_void>();
        if libc::ptrace(libc::PTRACE_TRACEME, 0, none, none) < 0 {
            libc::_exit(CHILD_TRACEME_FAILED);
        }
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
    let path = CString::new(program.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let argv0 = CString::new(test).map_err(|e| e.to_string())?;
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
        next_index: 0,
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

struct Tracer {
    start: Instant,
    events: Vec<Event>,
    procs: BTreeMap<libc::pid_t, Proc>,
    next_index: u32,
    exit: Exit,
    /// Tracees not reaped yet, for the watchdog.
    live: Arc<Mutex<BTreeSet<libc::pid_t>>>,
}

impl Tracer {
    fn add(&mut self, pid: libc::pid_t, fresh: bool) {
        if !self.procs.contains_key(&pid) {
            let p = Proc { index: self.next_index, mem: ProcMem::open(pid), in_syscall: false, pending: None, fresh };
            self.next_index += 1;
            self.procs.insert(pid, p);
            self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(pid);
        }
    }

    fn run(&mut self, pid: libc::pid_t) -> Result {
        // First stop: the SIGTRAP after a successful execve.
        let (_, status) = wait(pid).map_err(|e| format!("waitpid: {e}"))?;
        if libc::WIFEXITED(status) {
            return Err(match libc::WEXITSTATUS(status) {
                CHILD_TRACEME_FAILED => "ptrace(PTRACE_TRACEME) is not permitted here (container seccomp or \
                                         Yama policy?); use --tracer strace or --tracer none"
                    .into(),
                CHILD_EXEC_FAILED => "the test program could not be executed".into(),
                code => format!("the test program exited with {code} before it was traced"),
            });
        }
        if !libc::WIFSTOPPED(status) || libc::WSTOPSIG(status) != libc::SIGTRAP {
            return Err(format!("unexpected first stop of the tracee (status {status:#x})"));
        }
        let options = libc::PTRACE_O_TRACESYSGOOD
            | libc::PTRACE_O_EXITKILL
            | libc::PTRACE_O_TRACECLONE
            | libc::PTRACE_O_TRACEFORK
            | libc::PTRACE_O_TRACEVFORK
            | libc::PTRACE_O_TRACEEXEC;
        ptrace(libc::PTRACE_SETOPTIONS, pid, options as usize).map_err(|e| format!("PTRACE_SETOPTIONS: {e}"))?;
        self.add(pid, false);
        ptrace(libc::PTRACE_SYSCALL, pid, 0).map_err(|e| format!("PTRACE_SYSCALL: {e}"))?;

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
            // A stop of a process not seen yet: a new child whose attach
            // stop arrived before its parent's fork event.
            self.add(wpid, true);
            let sig = libc::WSTOPSIG(status);
            let event = status >> 16;
            let mut inject = 0;
            if sig == libc::SIGTRAP | 0x80 {
                self.syscall_stop(wpid)?;
            } else if sig == libc::SIGTRAP && event != 0 {
                match event {
                    libc::PTRACE_EVENT_FORK | libc::PTRACE_EVENT_VFORK | libc::PTRACE_EVENT_CLONE => {
                        if let Ok(child) = event_msg(wpid) {
                            self.add(child as libc::pid_t, true);
                        }
                    }
                    // The address space was replaced: reopen its memory.
                    libc::PTRACE_EVENT_EXEC => {
                        if let Some(p) = self.procs.get_mut(&wpid) {
                            p.mem = ProcMem::open(wpid);
                        }
                    }
                    _ => {}
                }
            } else if sig == libc::SIGSTOP && self.procs.get(&wpid).is_some_and(|p| p.fresh) {
                self.procs.get_mut(&wpid).unwrap().fresh = false;
            } else {
                let index = self.procs[&wpid].index;
                self.events.push(Event::Signal { pid: index, signal: sig });
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
