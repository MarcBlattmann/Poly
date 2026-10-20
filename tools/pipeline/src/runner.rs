//! Running a test program on the host (the reference system): under our
//! ptrace tracer, under strace (fallback), or untraced.

use crate::ptrace;
use crate::sha256;
use crate::syscalls::{self, ArgKind};
use crate::trace::{Arg, Event, Exit, Syscall, Trace};
use crate::util::{self, Result};
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};
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
    let mut t = match tracer {
        Tracer::Ptrace => ptrace::trace(program, test, timeout)?,
        Tracer::Strace => strace(program, test, timeout)?,
        Tracer::None => untraced(program, test, timeout)?,
    };
    t.binary_sha256 = Some(sha256::hex(&util::read(program)?));
    Ok(t)
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

pub(crate) fn run_command(cmd: &mut Command, timeout: Duration) -> Result<Finished> {
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    let reader = |mut r: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut out = Vec::new();
            let _ = r.read_to_end(&mut out);
            out
        })
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
            let _ = child.kill();
            let _ = child.wait();
            break Exit::Timeout;
        }
        thread::sleep(Duration::from_millis(2));
    };
    let duration_ns = start.elapsed().as_nanos() as u64;
    Ok(Finished { exit, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default(), duration_ns })
}

fn untraced(program: &Path, test: &str, timeout: Duration) -> Result<Trace> {
    let f = run_command(Command::new(program).env_clear(), timeout)?;
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
        Command::new(strace)
            .args(["-f", "-q", "-X", "raw", "-s", "4096", "-o"])
            .arg(&log)
            .arg("--")
            .arg(program)
            .env_clear(),
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
        let (kinds, _) = syscalls::signature(&name);
        let known = syscalls::is_known_signature(&name);
        let args = split_args(&call[open + 1..close])
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

    #[test]
    fn empty_log_is_an_error() {
        assert!(parse_strace("", "x").is_err());
    }
}
