//! Running a test program on the host (the reference system): under our
//! ptrace tracer, under strace (fallback), or untraced.

use crate::ptrace;
use crate::sha256;
use crate::syscalls::{self, ArgKind};
use crate::trace::{Arg, Event, Exit, Syscall, Trace};
use crate::util::{self, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tracer {
    /// Our own tracer (`ptrace.rs`). The default.
    Ptrace,
    /// `strace -f`, parsed (`strace.rs` format below). Fallback where our
    /// tracer cannot run; traces from it compare only with strace traces.
    Strace,
    /// No syscalls: exit status and output only.
    None,
}

impl Tracer {
    pub fn parse(s: &str) -> Result<Tracer> {
        match s {
            "ptrace" => Ok(Tracer::Ptrace),
            "strace" => Ok(Tracer::Strace),
            "none" => Ok(Tracer::None),
            _ => Err(format!("unknown tracer {s} (ptrace, strace, none)")),
        }
    }
}

/// Run `program` on the host with `tracer`.
pub fn run(tracer: Tracer, program: &Path, test: &str, timeout: Duration) -> Result<Trace> {
    // Absolute: the program starts in `/` ([`reference`]).
    let program = &util::absolute(program);
    let mut t = match tracer {
        Tracer::Ptrace => ptrace::trace(program, test, timeout)?,
        Tracer::Strace => strace(program, test, timeout)?,
        Tracer::None => untraced(program, test, timeout)?,
    };
    t.binary_sha256 = Some(sha256::hex(&util::read(program)?));
    Ok(t)
}

/// How often a run on this machine that timed out is run again
/// ([`rerun_on_timeout`]).
pub const TIMEOUT_RERUNS: usize = 1;

/// `run`, a run of a test on this machine (the reference OS), run again
/// while it times out, up to [`TIMEOUT_RERUNS`] times; with how many runs
/// timed out. The reference always finishes, so a timeout is a test that
/// hangs (a lost wake-up, a signal lost to a race with the kernel's own
/// workers), not one that is slow: such a test passes when run again, and a
/// larger timeout would not help (ADR 0085). The result says it was rerun,
/// so the test can still be fixed.
pub fn rerun_on_timeout(mut run: impl FnMut() -> Result<Trace>) -> Result<(Trace, usize)> {
    let mut timeouts = 0;
    loop {
        let t = run()?;
        if t.exit != Exit::Timeout {
            return Ok((t, timeouts));
        }
        timeouts += 1;
        if timeouts > TIMEOUT_RERUNS {
            return Ok((t, timeouts));
        }
    }
}

/// Run the app test `test` in `tree` (an app test's file tree, written
/// with `apps::Tree::materialise`): its launcher chrooted into it. The
/// trace's stdout and stderr are what the launcher wrote to its own, then
/// what the programs wrote to `/poly-app/stdout` and `/poly-app/stderr`.
/// `overlays`: see `ptrace::trace_in`.
pub fn run_app(
    tracer: Tracer,
    tree: &Path,
    overlays: &[(std::path::PathBuf, String)],
    test: &str,
    timeout: Duration,
) -> Result<Trace> {
    let mut t = match tracer {
        Tracer::Ptrace => ptrace::trace_in(tree, overlays, crate::poly::INIT_PATH, test, timeout)?,
        Tracer::Strace | Tracer::None => {
            return Err("app tests run under the ptrace tracer only (--tracer ptrace)".into());
        }
    };
    for (stream, file) in [(&mut t.stdout, crate::apps::STDOUT_PATH), (&mut t.stderr, crate::apps::STDERR_PATH)] {
        if let Ok(bytes) = std::fs::read(tree.join(file)) {
            stream.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    t.binary_sha256 = Some(sha256::hex(&util::read(&tree.join(crate::poly::INIT_PATH.trim_start_matches('/')))?));
    Ok(t)
}

/// Output and status of a command run with a timeout.
pub(crate) struct Finished {
    pub exit: Exit,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration_ns: u64,
}

/// `cmd` for a test program on the reference side: it starts with only
/// stdin, stdout and stderr open, whatever this process inherited
/// ([`ptrace::cloexec_from_3`]), with the fixed umask
/// ([`crate::identity::REFERENCE_UMASK`]), in `/`: as Poly's test init
/// starts it.
fn reference(cmd: &mut Command) -> &mut Command {
    cmd.current_dir("/");
    // SAFETY: the hook runs in the child between fork and exec and makes
    // only async-signal-safe calls (close_range, or getrlimit and fcntl;
    // umask).
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(cmd, || {
            ptrace::cloexec_from_3();
            crate::identity::set_reference_umask();
            Ok(())
        })
    }
}

/// How long the output of a finished or killed command is waited for past
/// its deadline: a process outside its tree may still hold the pipes.
const OUTPUT_GRACE: Duration = Duration::from_secs(1);

/// Run `cmd` with its output captured. On timeout the command's whole
/// process tree is killed ([`kill_tree`]), not only the command: a test's
/// child that started a session of its own (a hanging `b_ioctl_ctty`) once
/// outlived it, kept the output pipes open and so hung this function, and
/// spun on for hours (ADR 0085). The output is waited for until the
/// deadline (plus [`OUTPUT_GRACE`]) at most, for the same reason.
pub(crate) fn run_command(cmd: &mut Command, timeout: Duration) -> Result<Finished> {
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    // Each reader appends to its buffer as it reads and says when its
    // stream has ended.
    let reader = |mut r: Box<dyn Read + Send>| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (done, ended) = mpsc::channel::<()>();
        let into = Arc::clone(&buf);
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match r.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => into.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = done.send(());
        });
        (buf, ended)
    };
    let out = reader(Box::new(child.stdout.take().unwrap()));
    let err = reader(Box::new(child.stderr.take().unwrap()));
    let deadline = start + timeout;
    let exit = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break match (status.code(), status.signal()) {
                (Some(c), _) => Exit::Code(c),
                (None, Some(s)) => Exit::Signal(s),
                _ => Exit::Unknown,
            };
        }
        if Instant::now() > deadline {
            kill_tree(child.id() as libc::pid_t);
            let _ = child.kill();
            let _ = child.wait();
            break Exit::Timeout;
        }
        thread::sleep(Duration::from_millis(2));
    };
    let duration_ns = start.elapsed().as_nanos() as u64;
    let output_deadline = deadline.max(Instant::now()) + OUTPUT_GRACE;
    let collect = |(buf, ended): (Arc<Mutex<Vec<u8>>>, mpsc::Receiver<()>)| {
        let _ = ended.recv_timeout(output_deadline.saturating_duration_since(Instant::now()));
        std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()))
    };
    Ok(Finished { exit, stdout: collect(out), stderr: collect(err), duration_ns })
}

/// The processes whose parent is in `parents`, from `/proc/<pid>/stat`.
fn children_of(parents: &BTreeSet<libc::pid_t>) -> BTreeSet<libc::pid_t> {
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<libc::pid_t>().ok()) else { continue };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // "pid (comm) state ppid ...": comm may contain anything, ")" too.
        let ppid = stat.rfind(')').and_then(|i| stat[i + 1..].split_whitespace().nth(1)?.parse().ok());
        if ppid.is_some_and(|p| parents.contains(&p)) {
            found.insert(pid);
        }
    }
    found
}

/// SIGKILL `pid` (a child of this process, not reaped yet) and every
/// process descended from it, wherever it went (another process group,
/// another session). The tree is stopped first, until a scan finds no new
/// process, so nothing forks past the kill.
fn kill_tree(pid: libc::pid_t) {
    let mut stopped = BTreeSet::from([pid]);
    // SAFETY: kill has no memory-safety requirements. `pid` is our
    // unreaped child, so it cannot have been reused.
    unsafe { libc::kill(pid, libc::SIGSTOP) };
    // One generation per scan (a deep tree, or one that forks meanwhile,
    // takes more), until a scan finds nothing new.
    for _ in 0..256 {
        let new: Vec<libc::pid_t> = children_of(&stopped).into_iter().filter(|p| !stopped.contains(p)).collect();
        if new.is_empty() {
            break;
        }
        for p in new {
            // SAFETY: as above. `p` was just seen as a child of a stopped
            // process of the tree: until that parent runs again (SIGKILL,
            // below) nobody can reap it, so its pid cannot be reused.
            unsafe { libc::kill(p, libc::SIGSTOP) };
            stopped.insert(p);
        }
    }
    for &p in &stopped {
        // SAFETY: as above; every process of the tree is stopped, and only
        // a parent's wait (its parent is stopped too, or is us) frees a pid.
        unsafe { libc::kill(p, libc::SIGKILL) };
    }
}

fn untraced(program: &Path, test: &str, timeout: Duration) -> Result<Trace> {
    let f = run_command(reference(Command::new(program).env_clear()), timeout)?;
    let mut t = Trace::new(test, &ptrace::host_system(), "none");
    t.exit = f.exit;
    t.stdout = String::from_utf8_lossy(&f.stdout).into_owned();
    t.stderr = String::from_utf8_lossy(&f.stderr).into_owned();
    t.duration_ns = f.duration_ns;
    Ok(t)
}

fn strace(program: &Path, test: &str, timeout: Duration) -> Result<Trace> {
    let strace = util::which("strace").ok_or("strace not found on PATH")?;
    let dir = std::env::temp_dir().join(format!("poly-strace-{}-{test}", std::process::id()));
    util::create_dir(&dir)?;
    let log = dir.join("strace.log");
    // -q: no attach/detach chatter (exit lines stay); -X raw: numbers, not constant names;
    // -s: whole strings. The program gets an empty environment.
    let f = run_command(
        reference(
            Command::new(strace)
                .args(["-f", "-q", "-X", "raw", "-s", "4096", "-o"])
                .arg(&log)
                .arg("--")
                .arg(program)
                .env_clear(),
        ),
        timeout,
    )?;
    let text = util::read_string(&log);
    let _ = std::fs::remove_dir_all(&dir);
    let mut t = parse_strace(&text?, test)?;
    t.stdout = String::from_utf8_lossy(&f.stdout).into_owned();
    t.stderr = String::from_utf8_lossy(&f.stderr).into_owned();
    t.duration_ns = f.duration_ns;
    if f.exit == Exit::Timeout {
        t.exit = Exit::Timeout;
    }
    Ok(t)
}

/// Split strace's argument list at top-level commas.
fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut quoted, mut escaped) = (Vec::new(), String::new(), 0i32, false, false);
    for ch in s.chars() {
        if quoted {
            cur.push(ch);
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn parse_number(s: &str) -> Option<i64> {
    if let Some(h) = s.strip_prefix("0x") { u64::from_str_radix(h, 16).ok().map(|v| v as i64) } else { s.parse().ok() }
}

/// One strace argument, typed by the syscall's signature where possible.
fn strace_arg(text: &str, kind: Option<ArgKind>) -> Arg {
    if text == "NULL" {
        return Arg::Ptr(0);
    }
    // `name=value`: strace's own argument layout (clone), not positional.
    if let Some((name, _)) = text.split_once('=')
        && !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Arg::Raw(text.to_string());
    }
    // Whatever strace decoded behind a plain pointer (a struct, random
    // bytes) is masked, as our tracer never reads it.
    if kind == Some(ArgKind::Ptr) {
        return Arg::Ptr(parse_number(text).map_or(1, |v| v as u64));
    }
    if text.starts_with('"') {
        return Arg::Str(text.to_string());
    }
    let Some(v) = parse_number(text) else { return Arg::Raw(text.to_string()) };
    match kind {
        Some(ArgKind::Int) => Arg::Int(v),
        Some(ArgKind::Ptr | ArgKind::OutBuf) => Arg::Ptr(v as u64),
        // A buffer strace printed as an address: it could not read it.
        Some(ArgKind::CStr | ArgKind::InBuf(_)) => Arg::Unreadable(v as u64),
        Some(ArgKind::Pid) => Arg::Pid(v),
        Some(ArgKind::Hex) | None => Arg::Hex(v as u64),
    }
}

/// `= 5`, `= 0x1f8e2000`, `= -1 EFAULT (Bad address)`, `= ?`.
fn strace_ret(s: &str) -> Option<i64> {
    let mut words = s.split_whitespace();
    let first = words.next()?;
    if first == "-1"
        && let Some(errno) = words.next().and_then(syscalls::errno_number)
    {
        return Some(-errno);
    }
    parse_number(first)
}

/// Parse the output of `strace -f -q -X raw -o FILE`.
pub fn parse_strace(text: &str, test: &str) -> Result<Trace> {
    let mut t = Trace::new(test, &ptrace::host_system(), "strace");
    let mut index: BTreeMap<u32, u32> = BTreeMap::new();
    let mut unfinished: BTreeMap<u32, String> = BTreeMap::new();
    let mut first_pid = None;
    for line in text.lines() {
        let line = line.trim_end();
        let Some((pid, rest)) = line.split_once(char::is_whitespace) else { continue };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        let rest = rest.trim_start();
        first_pid.get_or_insert(pid);
        let next = index.len() as u32;
        let p = *index.entry(pid).or_insert(next);
        if let Some(status) = rest.strip_prefix("+++ ").and_then(|r| r.strip_suffix(" +++")) {
            if Some(pid) == first_pid {
                if let Some(code) = status.strip_prefix("exited with ") {
                    t.exit = Exit::Code(code.parse().map_err(|_| format!("bad exit line: {line}"))?);
                } else if let Some(sig) = status.strip_prefix("killed by ") {
                    let name = sig.split_whitespace().next().unwrap_or_default();
                    t.exit = Exit::Signal(syscalls::signal_number(name).unwrap_or(0));
                }
            }
            continue;
        }
        if let Some(sig) = rest.strip_prefix("--- ") {
            let name = sig.split_whitespace().next().unwrap_or_default();
            t.events.push(Event::Signal { pid: p, signal: syscalls::signal_number(name).unwrap_or(0) });
            continue;
        }
        // `name(args <unfinished ...>` ... `<... name resumed>rest`.
        let call = if let Some(head) = rest.strip_suffix(" <unfinished ...>") {
            unfinished.insert(pid, head.to_string());
            continue;
        } else if let Some(tail) = rest.strip_prefix("<... ") {
            let Some((_, tail)) = tail.split_once(" resumed>") else { continue };
            format!("{}{}", unfinished.remove(&pid).unwrap_or_default(), tail.trim_start())
        } else {
            rest.to_string()
        };
        let Some(open) = call.find('(') else { continue };
        // The argument list ends at the last ')' followed by padding and "= ".
        let Some(close) =
            call.rmatch_indices(')').map(|(i, _)| i).find(|&i| call[i + 1..].trim_start().starts_with("= "))
        else {
            continue;
        };
        let name = call[..open].to_string();
        if name == "execve" && t.events.is_empty() && p == 0 {
            // Starting the program: our traces begin after it.
            continue;
        }
        let ret = strace_ret(call[close + 1..].trim_start().trim_start_matches("= "));
        let texts = split_args(&call[open + 1..close]);
        // A multiplexed call's argument kinds follow its option or command
        // argument, which `-X raw` prints as a number.
        let mut raw = [0u64; 6];
        for (slot, a) in raw.iter_mut().zip(&texts) {
            *slot = parse_number(a).map_or(0, |v| v as u64);
        }
        let (kinds, _) = syscalls::call_signature(&name, &raw);
        let known = syscalls::is_known_signature(&name);
        let args = texts
            .iter()
            .enumerate()
            .map(|(i, a)| strace_arg(a, if known { kinds.get(i).copied() } else { None }))
            .collect();
        let nr = syscalls::number(&name).unwrap_or(u64::MAX);
        t.events.push(Event::Syscall(Syscall { pid: p, nr, name, args, ret, t_ns: 0 }));
    }
    if first_pid.is_none() {
        return Err("strace log is empty".into());
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{NormOptions, normalize};

    const LOG: &str = r#"7711  execve("bin/libc-hello", ["bin/libc-hello"], 0x7ffc40c601a8 /* 0 vars */) = 0
7711  brk(NULL)                         = 0x1f8e2000
7711  arch_prctl(0x1002, 0x1f8e2380)    = 0
7711  set_tid_address(0x1f8e2650)       = 7711
7711  readlinkat(-100, "/proc/self/exe", "/tmp/x"..., 4096) = 92
7711  fstat(1, {st_mode=020666, st_rdev=0x103, ...}) = 0
7711  write(1, 0x8, 5)                  = -1 EFAULT (Bad address)
7711  write(1, ")\n, x", 5)             = 5
7711  syscall_0x3e8(0, 0, 0, 0, 0, 0)   = -1 ENOSYS (Function not implemented)
7711  clone(child_stack=NULL, flags=0x1200011 <unfinished ...>
7712  write(1, "child\n", 6)            = 6
7711  <... clone resumed>, child_tidptr=0x7f3aab98e000) = 7712
7712  +++ exited with 0 +++
7711  --- SIGSEGV {si_signo=11, si_code=0x1, si_addr=0x10} ---
7711  +++ killed by SIGSEGV +++
"#;

    #[test]
    fn parses_strace_output() {
        let t = parse_strace(LOG, "x").unwrap();
        assert_eq!(t.exit, Exit::Signal(11));
        let lines: Vec<String> = normalize(&t, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "brk(NULL) = <addr>",
                "arch_prctl(0x1002, <ptr>) = 0",
                "set_tid_address(<ptr>) = <pid>",
                "readlinkat(-100, \"/proc/self/exe\", <ptr>, 4096) = 92",
                "fstat(1, <ptr>) = 0",
                "write(1, <unreadable>, 5) = -1 EFAULT",
                "write(1, \")\\n, x\", 5) = 5",
                "syscall_0x3e8(0x0, 0x0, 0x0, 0x0, 0x0, 0x0) = -1 ENOSYS",
                "[p1] write(1, \"child\\n\", 6) = 6",
                "clone(child_stack=NULL, flags=0x1200011, child_tidptr=<addr>) = <pid>",
                "--- SIGSEGV ---",
            ]
        );
    }

    #[test]
    fn splits_nested_arguments() {
        assert_eq!(split_args(r#"1, {a=1, b=[2, 3]}, "x,\"y", NULL"#), ["1", "{a=1, b=[2, 3]}", r#""x,\"y""#, "NULL"]);
    }

    /// strace prints what a prctl option writes through arg2 (`[0]`): a
    /// pointer by the option, so it compares like our tracer's `<ptr>`.
    #[test]
    fn strace_multiplexed_calls_are_typed_by_option() {
        let log = "7711  prctl(0x25, [0])                  = 0\n\
                   7711  prctl(0x24, 0x1)                  = 0\n\
                   7711  fcntl(3, 0x9)                     = 7711\n\
                   7711  exit_group(0)                     = ?\n";
        let t = parse_strace(log, "x").unwrap();
        let lines: Vec<String> = normalize(&t, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(lines, ["prctl(37, <ptr>) = 0", "prctl(36, 0x1) = 0", "fcntl(3, 9) = <pid>", "exit_group(0) = ?"]);
    }

    #[test]
    fn empty_log_is_an_error() {
        assert!(parse_strace("", "x").is_err());
    }

    fn exiting(exit: Exit) -> Trace {
        let mut t = Trace::new("t", "linux", "ptrace");
        t.exit = exit;
        t
    }

    /// A run that timed out is run again, once; a run that did not time
    /// out is not (ADR 0085).
    #[test]
    fn a_timed_out_run_is_run_again_once() {
        let mut exits = vec![Exit::Code(0), Exit::Timeout];
        let (t, n) = rerun_on_timeout(|| Ok(exiting(exits.pop().unwrap()))).unwrap();
        assert_eq!((t.exit, n, exits.len()), (Exit::Code(0), 1, 0));
        let mut runs = 0;
        let (t, n) = rerun_on_timeout(|| {
            runs += 1;
            Ok(exiting(Exit::Timeout))
        })
        .unwrap();
        assert_eq!((t.exit, n, runs), (Exit::Timeout, 2, 1 + TIMEOUT_RERUNS));
        let mut runs = 0;
        let (t, n) = rerun_on_timeout(|| {
            runs += 1;
            Ok(exiting(Exit::Code(1)))
        })
        .unwrap();
        assert_eq!((t.exit, n, runs), (Exit::Code(1), 0, 1));
        assert!(rerun_on_timeout(|| Err("no".into())).is_err());
    }

    /// A timeout kills the command's whole tree: here a grandchild in a
    /// session of its own that keeps stdout open, as the child of a hanging
    /// `b_ioctl_ctty` did. Before, the run only killed the shell, then
    /// waited for the output for as long as the grandchild lived.
    #[test]
    fn a_timeout_kills_descendants_that_left_the_session() {
        let Some(setsid) = util::which("setsid") else { return eprintln!("skipped: no setsid(1)") };
        let script = format!("{} sh -c 'echo $$; exec sleep 1000' & wait", setsid.display());
        let start = Instant::now();
        let f = run_command(Command::new("sh").args(["-c", &script]), Duration::from_millis(500)).unwrap();
        assert_eq!(f.exit, Exit::Timeout);
        assert!(start.elapsed() < Duration::from_secs(10), "waited for the grandchild: {:?}", start.elapsed());
        let pid: u32 = String::from_utf8_lossy(&f.stdout).trim().parse().expect("the grandchild's pid");
        // Killed: gone, or a zombie until init reaps it.
        let alive = || {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|s| s.rfind(')').is_some_and(|i| !s[i + 1..].trim_start().starts_with('Z')))
        };
        let until = Instant::now() + Duration::from_secs(5);
        while alive() && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!alive(), "the grandchild {pid} survived the timeout");
    }
}
