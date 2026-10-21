//! The trace of one test run, the same for every source (the ptrace
//! tracer, strace, a Poly serial log), and its normalised form.
//!
//! A trace records what the program did in order: system calls with
//! decoded arguments and results, signals, its stdout and stderr and how it
//! ended. Normalising masks what legitimately differs between two runs or
//! two systems (addresses, pids, times), so two normalised traces can be
//! compared line by line.

use crate::syscalls::{self, ArgKind, RetKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Written into every trace file; bump when the format changes.
pub const FORMAT: &str = "poly-trace/1";

/// Longest string or buffer content kept per argument.
pub const MAX_STR: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    pub format: String,
    /// Test name (the binary's name in the corpus).
    pub test: String,
    /// What ran it, e.g. `linux 6.8.0-45-generic` or `poly`.
    pub system: String,
    /// `ptrace`, `strace`, `poly-serial` or `none` (no syscalls recorded).
    pub tracer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_sha256: Option<String>,
    pub exit: Exit,
    pub stdout: String,
    pub stderr: String,
    /// Wall time from start to exit, as seen by the tracer.
    pub duration_ns: u64,
    pub events: Vec<Event>,
}

impl Trace {
    pub fn new(test: &str, system: &str, tracer: &str) -> Trace {
        Trace {
            format: FORMAT.into(),
            test: test.into(),
            system: system.into(),
            tracer: tracer.into(),
            binary_sha256: None,
            exit: Exit::Unknown,
            stdout: String::new(),
            stderr: String::new(),
            duration_ns: 0,
            events: Vec::new(),
        }
    }

    pub fn syscalls(&self) -> impl Iterator<Item = &Syscall> {
        self.events.iter().filter_map(|e| match e {
            Event::Syscall(c) => Some(c),
            Event::Signal { .. } => None,
        })
    }
}

/// How the program ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exit {
    /// `exit_group(code)`.
    Code(i32),
    /// Killed by this signal.
    Signal(i32),
    /// Did not finish in time and was killed.
    Timeout,
    /// The source did not say (e.g. a log that ends early).
    Unknown,
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exit::Code(c) => write!(f, "exit {c}"),
            Exit::Signal(s) => write!(f, "{}", syscalls::signal_name(*s)),
            Exit::Timeout => write!(f, "timeout"),
            Exit::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Syscall(Syscall),
    /// A signal delivered to process `pid`.
    Signal {
        pid: u32,
        signal: i32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Syscall {
    /// Process index in order of appearance (0 = the test program), not
    /// the real pid, so traces of two runs compare.
    pub pid: u32,
    pub nr: u64,
    pub name: String,
    pub args: Vec<Arg>,
    /// Raw return value (`-errno` on failure); `None` if it never returned
    /// (`exit_group`) or the trace ended first.
    #[serde(default)]
    pub ret: Option<i64>,
    /// Nanoseconds since the program started, at syscall entry.
    #[serde(default)]
    pub t_ns: u64,
}

/// A decoded argument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", content = "v", rename_all = "snake_case")]
pub enum Arg {
    Int(i64),
    Hex(u64),
    Ptr(u64),
    /// String or buffer contents, already rendered: quoted, escaped, with
    /// `...` after the closing quote if cut at `MAX_STR`.
    Str(String),
    /// A pointer whose contents the tracer tried to read and could not.
    Unreadable(u64),
    /// A pointer whose contents the source did not provide (a Poly log
    /// without data for it): matches anything when compared.
    Opaque(u64),
    Pid(i64),
    /// Text from strace that is not one of the above (structs, arrays).
    Raw(String),
}

/// Read access to the traced program's memory.
pub trait Memory {
    /// Up to `len` bytes at `addr`; fewer (or none) where memory ends.
    fn read(&self, addr: u64, len: usize) -> Vec<u8>;
}

/// Render bytes the way strace does: quoted, C escapes, `...` if cut.
pub fn render_bytes(bytes: &[u8], truncated: bool) -> String {
    let mut s = String::with_capacity(bytes.len() + 2);
    s.push('"');
    for &b in bytes {
        match b {
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            b'\r' => s.push_str("\\r"),
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s.push('"');
    if truncated {
        s.push_str("...");
    }
    s
}

/// A C `int` passed in a 64-bit register: the upper half may be zero, so a
/// negative int (`-1`, `AT_FDCWD`) shows as a large positive number.
fn int_value(raw: u64) -> i64 {
    if raw >> 32 == 0 { i64::from(raw as u32 as i32) } else { raw as i64 }
}

fn read_cstr(mem: &dyn Memory, addr: u64) -> Arg {
    let mut out = Vec::new();
    while out.len() < MAX_STR {
        // Never read across a page boundary in one go: the next page may
        // be unmapped although the string ends before it.
        let at = addr + out.len() as u64;
        let chunk = (4096 - (at % 4096) as usize).min(MAX_STR - out.len());
        let bytes = mem.read(at, chunk);
        if let Some(nul) = bytes.iter().position(|&b| b == 0) {
            out.extend_from_slice(&bytes[..nul]);
            return Arg::Str(render_bytes(&out, false));
        }
        if bytes.is_empty() {
            return if out.is_empty() { Arg::Unreadable(addr) } else { Arg::Str(render_bytes(&out, true)) };
        }
        out.extend_from_slice(&bytes);
    }
    Arg::Str(render_bytes(&out, true))
}

fn read_buf(mem: &dyn Memory, addr: u64, len: u64) -> Arg {
    if len == 0 {
        return Arg::Str(render_bytes(&[], false));
    }
    let want = (len as usize).min(MAX_STR);
    let bytes = mem.read(addr, want);
    if bytes.len() < want { Arg::Unreadable(addr) } else { Arg::Str(render_bytes(&bytes, want as u64 != len)) }
}

/// Decode the arguments of syscall `name` at entry, reading strings and
/// input buffers from `mem`.
pub fn decode_entry(name: &str, raw: &[u64; 6], mem: &dyn Memory) -> Vec<Arg> {
    let (kinds, _) = syscalls::call_signature(name, raw);
    kinds
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            let v = raw[i];
            match *kind {
                ArgKind::Int => Arg::Int(int_value(v)),
                ArgKind::Hex => Arg::Hex(v),
                ArgKind::Ptr | ArgKind::OutBuf => Arg::Ptr(v),
                ArgKind::Pid => Arg::Pid(int_value(v)),
                ArgKind::CStr | ArgKind::InBuf(_) if v == 0 => Arg::Ptr(0),
                ArgKind::CStr => read_cstr(mem, v),
                ArgKind::InBuf(len) => read_buf(mem, v, raw[len]),
            }
        })
        .collect()
}

/// Fill in output buffers once the call returned `ret`.
pub fn decode_exit(name: &str, raw: &[u64; 6], ret: i64, args: &mut [Arg], mem: &dyn Memory) {
    let (kinds, _) = syscalls::call_signature(name, raw);
    for (i, kind) in kinds.iter().enumerate() {
        if *kind == ArgKind::OutBuf && ret > 0 && raw[i] != 0 {
            args[i] = read_buf(mem, raw[i], ret as u64);
        }
    }
}

/// Replace hex numbers of 8 digits or more (addresses) in strace text.
/// Shorter ones are flags or addresses inside a non-PIE static binary,
/// which are the same on every run of the same binary.
fn mask_addresses(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("0x") {
        out.push_str(&rest[..i]);
        let digits = rest[i + 2..].bytes().take_while(u8::is_ascii_hexdigit).count();
        if digits >= 8 {
            out.push_str("<addr>");
        } else {
            out.push_str(&rest[i..i + 2 + digits]);
        }
        rest = &rest[i + 2 + digits..];
    }
    out.push_str(rest);
    out
}

/// One argument, normalised (`normalize`) or as recorded.
pub fn render_arg(arg: &Arg, known_call: bool, normalize: bool) -> String {
    match arg {
        Arg::Int(v) => v.to_string(),
        // Unknown calls: anything that looks like a user address is masked.
        Arg::Hex(v) if normalize && !known_call && *v >= 0x10000 => "<addr?>".into(),
        Arg::Hex(v) => format!("{v:#x}"),
        Arg::Ptr(0) => "NULL".into(),
        Arg::Ptr(_) if normalize => "<ptr>".into(),
        Arg::Ptr(v) => format!("{v:#x}"),
        Arg::Str(s) => s.clone(),
        Arg::Unreadable(_) if normalize => "<unreadable>".into(),
        Arg::Unreadable(v) => format!("{v:#x} <unreadable>"),
        Arg::Opaque(_) if normalize => WILDCARD.into(),
        Arg::Opaque(v) => format!("{v:#x}"),
        Arg::Pid(v) if normalize && *v > 0 => "<pid>".into(),
        // A process group given as -pgid (wait4, kill); -1 means "all".
        Arg::Pid(v) if normalize && *v < -1 => "-<pid>".into(),
        Arg::Pid(v) => v.to_string(),
        Arg::Raw(s) if normalize => mask_addresses(s),
        Arg::Raw(s) => s.clone(),
    }
}

/// A return value of kind `kind`, normalised or as recorded.
pub fn render_ret(kind: RetKind, ret: Option<i64>, normalize: bool) -> String {
    let Some(r) = ret else { return "?".into() };
    if syscalls::is_error(r) {
        return format!("-1 {}", syscalls::errno_name(-r));
    }
    match kind {
        RetKind::Addr if normalize => "<addr>".into(),
        RetKind::Addr => format!("{r:#x}"),
        RetKind::Pid if normalize && r > 0 => "<pid>".into(),
        RetKind::Time if normalize => "<time>".into(),
        RetKind::Pid | RetKind::Int | RetKind::Time => r.to_string(),
    }
}

/// Matches any value in a normalised comparison.
pub const WILDCARD: &str = "*";

/// One normalised event: a syscall or a signal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NormCall {
    pub pid: u32,
    /// Syscall name, or `--- SIGNAME ---` for a signal.
    pub name: String,
    pub args: Vec<String>,
    /// Empty for signals.
    pub ret: String,
}

impl NormCall {
    /// Equal, where `*` on either side matches any one argument.
    pub fn matches(&self, other: &NormCall, names_only: bool) -> bool {
        if self.pid != other.pid || self.name != other.name {
            return false;
        }
        if names_only {
            return true;
        }
        let same = |a: &String, b: &String| a == b || a == WILDCARD || b == WILDCARD;
        self.args.len() == other.args.len()
            && self.args.iter().zip(&other.args).all(|(a, b)| same(a, b))
            && same(&self.ret, &other.ret)
    }
}

impl fmt::Display for NormCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pid != 0 {
            write!(f, "[p{}] ", self.pid)?;
        }
        if self.ret.is_empty() && self.name.starts_with("---") {
            return write!(f, "{}", self.name);
        }
        write!(f, "{}({}) = {}", self.name, self.args.join(", "), self.ret)
    }
}

/// What to leave out of the comparison.
#[derive(Clone, Debug, Default)]
pub struct NormOptions {
    /// Syscalls dropped from both sides (e.g. calls a C library makes that
    /// do not matter for a test).
    pub ignore: BTreeSet<String>,
}

/// The raw values behind decoded arguments, as far as they are numbers:
/// enough to pick a multiplexed call's signature again (its option or
/// command argument is always one).
fn raw_values(args: &[Arg]) -> [u64; 6] {
    let mut raw = [0u64; 6];
    for (slot, a) in raw.iter_mut().zip(args) {
        *slot = match *a {
            Arg::Int(v) | Arg::Pid(v) => v as u64,
            Arg::Hex(v) | Arg::Ptr(v) | Arg::Unreadable(v) | Arg::Opaque(v) => v,
            Arg::Str(_) | Arg::Raw(_) => 0,
        };
    }
    raw
}

pub fn normalize_call(c: &Syscall, normalize: bool) -> NormCall {
    let known = syscalls::is_known_signature(&c.name);
    let (_, ret_kind) = syscalls::call_signature(&c.name, &raw_values(&c.args));
    NormCall {
        pid: c.pid,
        name: c.name.clone(),
        args: c.args.iter().map(|a| render_arg(a, known, normalize)).collect(),
        ret: render_ret(ret_kind, c.ret, normalize),
    }
}

/// Linux's internal "restart" results, which ptrace shows at the exit of an
/// interrupted syscall but the program never sees: ERESTARTSYS,
/// ERESTARTNOINTR, ERESTARTNOHAND, ERESTART_RESTARTBLOCK.
const RESTART_ERRNOS: [i64; 4] = [512, 513, 514, 516];

/// What the program saw of a syscall that ended with a restart result:
/// nothing if it was restarted (ERESTARTNOINTR always is; the others when
/// the same pid's next syscall is the same call again), else EINTR.
fn restart_outcome(events: &[Event], i: usize, c: &Syscall) -> Option<i64> {
    let errno = -c.ret?;
    if !RESTART_ERRNOS.contains(&errno) {
        return None;
    }
    if errno == 513 {
        return Some(0);
    }
    let next = events[i + 1..].iter().find_map(|e| match e {
        Event::Syscall(n) if n.pid == c.pid => Some(n),
        _ => None,
    });
    let again = next.is_some_and(|n| n.nr == c.nr && normalize_call(n, true).args == normalize_call(c, true).args);
    Some(if again { 0 } else { -4 })
}

/// The normalised event sequence of `trace`.
pub fn normalize(trace: &Trace, opts: &NormOptions) -> Vec<NormCall> {
    trace
        .events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::Syscall(c) if opts.ignore.contains(&c.name) => None,
            Event::Syscall(c) => match restart_outcome(&trace.events, i, c) {
                // Restarted: the program sees only the next attempt.
                Some(0) => None,
                Some(seen) => Some(normalize_call(&Syscall { ret: Some(seen), ..c.clone() }, true)),
                None => Some(normalize_call(c, true)),
            },
            Event::Signal { pid, signal } => Some(signal_event(*pid, *signal)),
        })
        .collect()
}

/// The trace as text, one event per line, values as recorded.
pub fn render_raw(trace: &Trace) -> String {
    let mut out = String::new();
    for e in &trace.events {
        let line = match e {
            Event::Syscall(c) => normalize_call(c, false).to_string(),
            Event::Signal { pid, signal } => signal_event(*pid, *signal).to_string(),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&format!("+++ {} +++\n", trace.exit));
    out
}

fn signal_event(pid: u32, signal: i32) -> NormCall {
    NormCall { pid, name: format!("--- {} ---", syscalls::signal_name(signal)), args: Vec::new(), ret: String::new() }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Memory made of a few regions, for tests.
    pub struct Regions(pub BTreeMap<u64, Vec<u8>>);

    impl Memory for Regions {
        fn read(&self, addr: u64, len: usize) -> Vec<u8> {
            for (&start, bytes) in &self.0 {
                let end = start + bytes.len() as u64;
                if (start..end).contains(&addr) {
                    let from = (addr - start) as usize;
                    return bytes[from..(from + len).min(bytes.len())].to_vec();
                }
            }
            Vec::new()
        }
    }

    pub fn call(name: &str, args: Vec<Arg>, ret: Option<i64>) -> Event {
        Event::Syscall(Syscall {
            pid: 0,
            nr: syscalls::number(name).unwrap_or(0),
            name: name.into(),
            args,
            ret,
            t_ns: 0,
        })
    }

    #[test]
    fn decodes_write_with_its_buffer() {
        let mem = Regions(BTreeMap::from([(0x1000, b"hi\n".to_vec())]));
        let args = decode_entry("write", &[1, 0x1000, 3, 0, 0, 0], &mem);
        assert_eq!(args, [Arg::Int(1), Arg::Str("\"hi\\n\"".into()), Arg::Int(3)]);
        let bad = decode_entry("write", &[1, 8, 5, 0, 0, 0], &mem);
        assert_eq!(bad[1], Arg::Unreadable(8));
        let empty = decode_entry("write", &[1, 8, 0, 0, 0, 0], &mem);
        assert_eq!(empty[1], Arg::Str("\"\"".into()));
    }

    #[test]
    fn decodes_strings_across_pages_and_negative_ints() {
        let mut path = b"/proc/self/exe".to_vec();
        path.push(0);
        // The string starts 4 bytes before a page boundary.
        let mem = Regions(BTreeMap::from([(0x1ffc, path)]));
        let args = decode_entry("openat", &[0xffff_ff9c, 0x1ffc, 0, 0, 0, 0], &mem);
        assert_eq!(args[0], Arg::Int(-100));
        assert_eq!(args[1], Arg::Str("\"/proc/self/exe\"".into()));
    }

    #[test]
    fn output_buffers_are_read_at_exit() {
        let mem = Regions(BTreeMap::from([(0x2000, b"abcdef".to_vec())]));
        let raw = [0, 0x2000, 100, 0, 0, 0];
        let mut args = decode_entry("read", &raw, &mem);
        assert_eq!(args[1], Arg::Ptr(0x2000));
        decode_exit("read", &raw, 4, &mut args, &mem);
        assert_eq!(args[1], Arg::Str("\"abcd\"".into()));
    }

    #[test]
    fn renders_like_strace() {
        assert_eq!(render_bytes(b"a\"b\\\x01\t", false), "\"a\\\"b\\\\\\x01\\t\"");
        assert_eq!(render_bytes(b"abc", true), "\"abc\"...");
    }

    /// Restart results become what the program saw: EINTR after a handler,
    /// nothing when the call was simply made again.
    #[test]
    fn restart_results_are_what_the_program_saw() {
        let mut t = Trace::new("x", "linux", "ptrace");
        t.events = vec![
            call("ppoll", vec![Arg::Ptr(0x7000), Arg::Int(1), Arg::Ptr(0), Arg::Ptr(0x7100), Arg::Int(8)], Some(-514)),
            Event::Signal { pid: 0, signal: 10 },
            call("rt_sigreturn", vec![], Some(-4)),
            call("read", vec![Arg::Int(3), Arg::Ptr(0x7000), Arg::Int(1)], Some(-512)),
            call("read", vec![Arg::Int(3), Arg::Ptr(0x7000), Arg::Int(1)], Some(1)),
            call("fork", vec![], Some(-513)),
            call("fork", vec![], Some(55)),
        ];
        let lines: Vec<String> = normalize(&t, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "ppoll(<ptr>, 1, NULL, <ptr>, 8) = -1 EINTR",
                "--- SIGUSR1 ---",
                "rt_sigreturn() = -1 EINTR",
                "read(3, <ptr>, 1) = 1",
                "fork() = <pid>",
            ]
        );
    }

    #[test]
    fn normalisation_masks_clock_results_and_group_ids() {
        let mut t = Trace::new("x", "linux", "ptrace");
        t.events = vec![
            call("time", vec![Arg::Ptr(0)], Some(1_700_000_000)),
            call("times", vec![Arg::Ptr(0x7fff_0000)], Some(4_242_424)),
            call("getpgid", vec![Arg::Pid(0)], Some(4321)),
            call("setpgid", vec![Arg::Pid(4322), Arg::Pid(4322)], Some(0)),
            call("wait4", vec![Arg::Pid(-4322), Arg::Ptr(0), Arg::Hex(0), Arg::Ptr(0)], Some(4323)),
            call("wait4", vec![Arg::Pid(-1), Arg::Ptr(0), Arg::Hex(1), Arg::Ptr(0)], Some(0)),
            call("time", vec![Arg::Ptr(8)], Some(-14)),
        ];
        let lines: Vec<String> = normalize(&t, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "time(NULL) = <time>",
                "times(<ptr>) = <time>",
                "getpgid(0) = <pid>",
                "setpgid(<pid>, <pid>) = 0",
                "wait4(-<pid>, NULL, 0x0, NULL) = <pid>",
                "wait4(-1, NULL, 0x1, NULL) = 0",
                "time(<ptr>) = -1 EFAULT",
            ]
        );
        assert!(render_raw(&t).contains("time(NULL) = 1700000000"));
    }

    #[test]
    fn normalisation_masks_addresses_and_pids() {
        let mut t = Trace::new("x", "linux", "ptrace");
        t.events = vec![
            call(
                "mmap",
                vec![Arg::Ptr(0), Arg::Int(8192), Arg::Hex(3), Arg::Hex(0x22), Arg::Int(-1), Arg::Hex(0)],
                Some(0x7f00_0000_0000),
            ),
            call("brk", vec![Arg::Ptr(0x4000_0000)], Some(0x4000_0000)),
            call("getpid", vec![], Some(1234)),
            call("kill", vec![Arg::Pid(-1234), Arg::Int(15)], Some(0)),
            call("kill", vec![Arg::Pid(0), Arg::Int(15)], Some(0)),
            call("rt_sigqueueinfo", vec![Arg::Pid(1234), Arg::Int(10), Arg::Ptr(0x7000)], Some(0)),
            call("write", vec![Arg::Int(1), Arg::Unreadable(8), Arg::Int(5)], Some(-14)),
            call("exit_group", vec![Arg::Int(0)], None),
            Event::Signal { pid: 0, signal: 11 },
        ];
        let lines: Vec<String> = normalize(&t, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "mmap(NULL, 8192, 0x3, 0x22, -1, 0x0) = <addr>",
                "brk(<ptr>) = <addr>",
                "getpid() = <pid>",
                "kill(-<pid>, 15) = 0",
                "kill(0, 15) = 0",
                "rt_sigqueueinfo(<pid>, 10, <ptr>) = 0",
                "write(1, <unreadable>, 5) = -1 EFAULT",
                "exit_group(0) = ?",
                "--- SIGSEGV ---",
            ]
        );
        let ignore = NormOptions { ignore: BTreeSet::from(["getpid".to_string()]) };
        assert_eq!(normalize(&t, &ignore).len(), 8);
        assert!(render_raw(&t).contains("brk(0x40000000) = 0x40000000"));
    }

    /// Two runs that pass different stack addresses to a prctl option
    /// writing through arg2 compare equal; an option whose arg2 is an
    /// integer still compares it. Likewise fcntl's owner pid.
    #[test]
    fn multiplexed_calls_mask_by_option() {
        let mem = Regions(BTreeMap::new());
        let norm = |name: &str, raw: [u64; 6], ret: i64| {
            let args = decode_entry(name, &raw, &mem);
            let c = Syscall {
                pid: 0,
                nr: syscalls::number(name).unwrap(),
                name: name.into(),
                args,
                ret: Some(ret),
                t_ns: 0,
            };
            normalize_call(&c, true)
        };
        // PR_GET_CHILD_SUBREAPER, two stack addresses 160 bytes apart.
        let linux = norm("prctl", [37, 0x7ffc_9a3e_1f40, 0, 0, 0, 0], 0);
        let poly = norm("prctl", [37, 0x7fff_ffff_ee60 - 160, 0, 0, 0, 0], 0);
        assert_eq!(linux.to_string(), "prctl(37, <ptr>, 0x0, 0x0, 0x0) = 0");
        assert!(linux.matches(&poly, false));
        assert_eq!(norm("prctl", [37, 0, 0, 0, 0, 0], -14).to_string(), "prctl(37, NULL, 0x0, 0x0, 0x0) = -1 EFAULT");
        // PR_SET_CHILD_SUBREAPER: 1 and 0 are different calls.
        let set = norm("prctl", [36, 1, 0, 0, 0, 0], 0);
        assert_eq!(set.to_string(), "prctl(36, 0x1, 0x0, 0x0, 0x0) = 0");
        assert!(!set.matches(&norm("prctl", [36, 0, 0, 0, 0, 0], 0), false));
        // F_SETOWN with the caller's pid, F_GETOWN returning it, F_GETFL without its unused argument.
        assert_eq!(norm("fcntl", [3, 8, 4321, 0, 0, 0], 0).to_string(), "fcntl(3, 8, <pid>) = 0");
        assert_eq!(norm("fcntl", [3, 9, 0x7ffd_0000, 0, 0, 0], 4321).to_string(), "fcntl(3, 9) = <pid>");
        assert_eq!(norm("fcntl", [3, 3, 0x7ffd_0000, 0, 0, 0], 2).to_string(), "fcntl(3, 3) = 2");
        assert_eq!(norm("fcntl", [3, 6, 0x7ffd_0000, 0, 0, 0], 0).to_string(), "fcntl(3, 6, <ptr>) = 0");
    }

    #[test]
    fn unknown_calls_mask_address_like_values() {
        let c = Syscall {
            pid: 0,
            nr: 1000,
            name: "syscall_0x3e8".into(),
            args: vec![Arg::Hex(1), Arg::Hex(0x7fff_0000)],
            ret: Some(-38),
            t_ns: 0,
        };
        assert_eq!(normalize_call(&c, true).to_string(), "syscall_0x3e8(0x1, <addr?>) = -1 ENOSYS");
    }

    #[test]
    fn wildcards_match_anything() {
        let a = NormCall { pid: 0, name: "write".into(), args: vec!["1".into(), "\"hi\"".into()], ret: "2".into() };
        let b = NormCall { args: vec!["1".into(), WILDCARD.into()], ..a.clone() };
        let c = NormCall { args: vec!["2".into(), WILDCARD.into()], ..a.clone() };
        assert!(a.matches(&b, false) && b.matches(&a, false));
        assert!(!a.matches(&c, false));
        assert!(a.matches(&c, true));
    }

    #[test]
    fn strace_text_is_masked() {
        assert_eq!(mask_addresses("{st_rdev=0x103, p=0x7fff1cc97940}"), "{st_rdev=0x103, p=<addr>}");
    }

    #[test]
    fn json_roundtrip() {
        let mut t = Trace::new("hello", "linux", "ptrace");
        t.exit = Exit::Signal(11);
        t.events = vec![call("write", vec![Arg::Int(1), Arg::Str("\"x\"".into()), Arg::Int(1)], Some(1))];
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("\"event\":\"syscall\"") && json.contains("\"exit\":{\"signal\":11}"));
        assert_eq!(serde_json::from_str::<Trace>(&json).unwrap(), t);
    }
}
