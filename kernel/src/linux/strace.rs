//! `poly.strace`: one line on the serial port per system call of every
//! Linux process, for debugging programs on Poly (docs/DEBUGGING.md).
//! Unlike `poly.test.capture` (the test-run protocol, ADR 0027) it changes
//! nothing about the processes: they keep their terminals.
//!
//! ```text
//! strace: pid 7 openat(0xffffff9c, "/etc/inittab", 0x80000, 0x0, ..) = 3
//! strace: pid 7 ioctl(0x0, 0x5401, 0x7fffffffe9c0, ..) = -25 ENOTTY
//! ```
//!
//! The arguments are the six registers, in hex; those that the call takes
//! as a path or name are shown as strings, read before the call runs.
//! `poly.strace=<name>` limits the lines to processes whose `comm` (the
//! file name of the program, as `/proc/<pid>/comm` shows it) is `<name>`.
//!
//! `poly.exits` writes one line per process that ends, with its status or
//! the signal that killed it, and nothing else:
//!
//! ```text
//! linux: pid 143 (chromium) exit status 0
//! linux: pid 150 (chromium) killed by signal 9
//! ```
//!
//! `poly.strace.slow=<ms>` writes only the calls that took at least `<ms>`
//! milliseconds (of any process, or of `<name>`'s with `poly.strace=<name>`
//! too), with the thread and the time they took, for finding what a slow
//! program waits for:
//!
//! ```text
//! strace: pid 37 tid 52 epoll_wait(0x5, 0x7f.., 0x20, 0xffffffff, ..) = 1 [2134 ms, until 61234 ms]
//! ```

use crate::process::Process;
use crate::sync::SpinLock;
use crate::{log, user};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use poly_personality_linux::errno::Errno;

/// Kernel argument: trace system calls (`poly.strace` or `poly.strace=<comm>`).
pub const STRACE_ARG: &str = "poly.strace";

/// Kernel argument: trace only calls that took this many milliseconds.
pub const SLOW_ARG: &str = "poly.strace.slow";

static ON: AtomicBool = AtomicBool::new(false);
/// [`SLOW_ARG`] in nanoseconds; 0: every call.
static SLOW_NS: AtomicU64 = AtomicU64::new(0);
/// The program name to trace, empty for every process.
static ONLY: SpinLock<String> = SpinLock::new(String::new());

/// Kernel argument: trace only the calls with these names, comma separated
/// (`poly.strace.calls=rt_sigaction,execve`), which keeps a traced program
/// fast enough to run to its end.
pub const CALLS_ARG: &str = "poly.strace.calls";

/// The call names to trace; empty for every call.
static CALLS: SpinLock<String> = SpinLock::new(String::new());

/// Kernel argument: do not trace the calls with these names, comma
/// separated (`poly.strace.skip=epoll_wait,gettid`): the calls a busy
/// program makes thousands of times and nobody wants to read.
pub const SKIP_ARG: &str = "poly.strace.skip";

static SKIP: SpinLock<String> = SpinLock::new(String::new());

/// Kernel argument: with `poly.exits.trail`, write only the trails of the
/// processes whose name and command line contain this text
/// (`poly.exits.trail.only=NetworkService`).
pub const TRAIL_ONLY_ARG: &str = "poly.exits.trail.only";

static TRAIL_ONLY: SpinLock<String> = SpinLock::new(String::new());

/// Kernel argument: trace only the calls that failed (`poly.strace.errors`),
/// but for `EAGAIN` and `EINTR`, which every program meets all the time.
/// It traces every call like `poly.strace` does, but writes next to
/// nothing, so a long run reaches its end (with `poly.exits`, the failures
/// before a process ended). A call that does not return is not written.
pub const ERRORS_ARG: &str = "poly.strace.errors";

static ERRORS: AtomicBool = AtomicBool::new(false);

/// Kernel argument: keep the last calls of every process and write them
/// when it ends (`poly.exits.trail` for 32 calls, `poly.exits.trail=64`
/// for 64). Every call is recorded but none is written as it happens, so a
/// long run is not slowed by the serial line; the lines follow the
/// process's `poly.exits` line (`trail: ...`). For finding out why a
/// process ended that was not killed.
pub const TRAIL_ARG: &str = "poly.exits.trail";

/// Calls kept per process, 0: no trail.
static TRAIL_LEN: AtomicUsize = AtomicUsize::new(0);

/// The last calls of the processes still running, oldest first.
static TRAILS: SpinLock<BTreeMap<u32, VecDeque<String>>> = SpinLock::new(BTreeMap::new());

/// Kernel argument: log how every Linux process ends (`poly.exits`),
/// without tracing its calls.
pub const EXITS_ARG: &str = "poly.exits";

static EXITS: AtomicBool = AtomicBool::new(false);

/// Most bytes of a string argument shown.
const STRING_MAX: usize = 96;

/// Whether `poly.strace` is on: then the kernel also logs every stop,
/// continue and kill of a Linux process, which it otherwise logs only for
/// signals whose default action is a core dump.
pub fn verbose() -> bool {
    ON.load(Ordering::Relaxed) || EXITS.load(Ordering::Relaxed)
}

/// Whether the end of every Linux process is logged (`poly.exits`, or
/// `poly.strace`).
pub fn exits_logged() -> bool {
    verbose()
}

/// Turn tracing on if the command line asks for it.
pub fn configure(boot: &crate::boot_info::BootInfo) {
    if boot.has_flag(EXITS_ARG) {
        EXITS.store(true, Ordering::Release);
        crate::kprintln!("exits: logging how every Linux process ends");
    }
    let slow = boot.arg(SLOW_ARG).and_then(|ms| ms.parse::<u64>().ok());
    if let Some(ms) = slow {
        SLOW_NS.store(ms.max(1) * 1_000_000, Ordering::Relaxed);
        crate::kprintln!("strace: only calls of {ms} ms and longer");
    }
    if let Some(calls) = boot.arg(CALLS_ARG) {
        *CALLS.lock() = String::from(calls);
    }
    if let Some(skip) = boot.arg(SKIP_ARG) {
        *SKIP.lock() = String::from(skip);
    }
    if let Some(only) = boot.arg(TRAIL_ONLY_ARG) {
        *TRAIL_ONLY.lock() = String::from(only);
    }
    let trail = if boot.has_flag(TRAIL_ARG) { Some(32) } else { boot.arg(TRAIL_ARG).and_then(|n| n.parse().ok()) };
    if let Some(n) = trail.filter(|&n| n > 0) {
        TRAIL_LEN.store(n, Ordering::Release);
        EXITS.store(true, Ordering::Release);
        crate::kprintln!("exits: the last {n} calls of a process are written when it ends");
    }
    let errors = boot.has_flag(ERRORS_ARG);
    ERRORS.store(errors, Ordering::Release);
    let name = match boot.arg(STRACE_ARG) {
        Some(name) => name,
        None if boot.has_flag(STRACE_ARG) || slow.is_some() || errors || trail.is_some() => "",
        None => return,
    };
    *ONLY.lock() = String::from(name);
    ON.store(true, Ordering::Release);
    crate::kprintln!(
        "strace: tracing the system calls of {}",
        if name.is_empty() { "every Linux process" } else { name }
    );
}

/// The recorded calls of process `pid` (oldest first), which ended: they
/// are forgotten. Empty without `poly.exits.trail`.
pub fn take_trail(pid: u32) -> alloc::vec::Vec<String> {
    if TRAIL_LEN.load(Ordering::Relaxed) == 0 {
        return alloc::vec::Vec::new();
    }
    TRAILS.lock().remove(&pid).map_or_else(alloc::vec::Vec::new, |t| t.into_iter().collect())
}

/// Write the trail of process `pid` after the line about its end; `who` is
/// the name and command line [`crate::linux::strace`] logged it with.
pub fn write_trail(pid: u32, who: &str) {
    let trail = take_trail(pid);
    let only = TRAIL_ONLY.lock().clone();
    if !only.is_empty() && !who.contains(only.as_str()) {
        return;
    }
    for line in trail {
        crate::kprintln!("linux:   trail pid {pid}: {line}");
    }
}

/// The wall-clock time of day as `HHMMSS.mmm` (UTC), the form of the time
/// stamps of Chromium's log lines, so a kernel line can be matched to them.
pub fn stamp() -> String {
    let ns = crate::time::realtime_ns();
    let secs = ns / 1_000_000_000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    alloc::format!("{h:02}{m:02}{s:02}.{:03}", (ns / 1_000_000) % 1000)
}

/// A call being traced: the start of its line and when it started.
pub struct Trace {
    line: String,
    start_ns: u64,
    /// rt_sigaction(2)'s `oldact`, whose handler and flags the finished
    /// line shows (what the call reported).
    old_action: u64,
    pid: u32,
}

/// The start of a trace line for call `number` with `args` of `process`'s
/// running thread `tid`, or `None` if it is not traced. Called before the
/// call runs, so its string arguments are read as the caller passed them.
pub fn enter(process: &Process, tid: u32, number: u64, args: &[u64; 6]) -> Option<Trace> {
    if !ON.load(Ordering::Acquire) {
        return None;
    }
    // The name is copied out first: `comm` takes a sleeping lock, which
    // must not be waited for under the spinlock.
    let only = ONLY.lock().clone();
    if !only.is_empty() && crate::fs::proc::comm(process.pid).as_deref() != Some(only.as_bytes()) {
        return None;
    }
    let name = poly_personality_linux::table::name(number);
    {
        let calls = CALLS.lock();
        if !calls.is_empty() && !name.is_some_and(|n| calls.split(',').any(|c| c == n)) {
            return None;
        }
    }
    {
        let skip = SKIP.lock();
        if !skip.is_empty() && name.is_some_and(|n| skip.split(',').any(|c| c == n)) {
            return None;
        }
    }
    let mut line = String::with_capacity(128);
    if SLOW_NS.load(Ordering::Relaxed) != 0 {
        let _ = write!(line, "strace: pid {} tid {tid} ", process.pid);
    } else {
        let _ = write!(line, "strace: pid {} ", process.pid);
    }
    if ERRORS.load(Ordering::Relaxed) || TRAIL_LEN.load(Ordering::Relaxed) != 0 {
        // Failures and the trail are rare enough to carry the time they happened.
        line.insert_str(0, &alloc::format!("{} ", stamp()));
    }
    match name {
        Some(n) => line.push_str(n),
        None => {
            let _ = write!(line, "syscall_{number}");
        }
    }
    line.push('(');
    let strings = name.map_or(0, string_args);
    for (i, &a) in args.iter().enumerate().take(4) {
        if i > 0 {
            line.push_str(", ");
        }
        if strings & (1 << i) != 0 {
            push_string(&mut line, a);
        } else {
            let _ = write!(line, "{a:#x}");
        }
    }
    line.push_str(", ..)");
    // rt_sigaction(2): what is installed and what was there, for finding
    // who owns a signal (`act={handler, flags}`, `old={handler, flags}`).
    // futex(2) waits: the timeout, and for an absolute one how far off it is
    // (`timeout=+13.700s`, relative to the clock it counts on).
    if name == Some("futex") {
        push_futex_timeout(&mut line, args);
    }
    let mut old_action = 0;
    if name == Some("rt_sigaction") {
        if args[1] != 0 {
            push_action(&mut line, " act", args[1]);
        }
        old_action = args[2];
    }
    Some(Trace { line, start_ns: crate::time::now_ns(), old_action, pid: process.pid })
}

/// Finish and write the trace line with the call's result (`None`: it
/// does not return), unless it was quicker than `poly.strace.slow`.
pub fn exit(trace: Trace, ret: Option<u64>) {
    let Trace { mut line, start_ns, old_action, pid } = trace;
    let slow = SLOW_NS.load(Ordering::Relaxed);
    let took = crate::time::now_ns().saturating_sub(start_ns);
    if slow != 0 && took < slow {
        return;
    }
    if old_action != 0 && ret == Some(0) {
        push_action(&mut line, " old", old_action);
    }
    let keep = TRAIL_LEN.load(Ordering::Relaxed);
    if keep != 0 {
        // Kept for when the process ends, not written now.
        let mut done = line;
        match ret {
            Some(r) => match Errno::from_rax(r) {
                Some(e) => {
                    let _ = write!(done, " = -{} {}", e.get(), e.name().unwrap_or("?"));
                }
                None => {
                    let _ = write!(done, " = {r:#x}");
                }
            },
            None => done.push_str(" = ?"),
        }
        let mut trails = TRAILS.lock();
        let trail = trails.entry(pid).or_default();
        if trail.len() >= keep {
            trail.pop_front();
        }
        trail.push_back(done);
        return;
    }
    if ERRORS.load(Ordering::Relaxed) {
        // Only the failures (and calls that do not return: nothing to show).
        match ret.and_then(Errno::from_rax) {
            Some(e) if e != Errno::EAGAIN && e != Errno::EINTR => {}
            _ => return,
        }
    }
    match ret {
        Some(r) => match Errno::from_rax(r) {
            Some(e) => {
                let _ = writeln!(line, " = -{} {}", e.get(), e.name().unwrap_or("?"));
            }
            None if r > 0xffff_ffff => {
                let _ = writeln!(line, " = {r:#x}");
            }
            None => {
                let _ = writeln!(line, " = {r}");
            }
        },
        None => line.push_str(" = ?\n"),
    }
    if slow != 0 {
        line.pop();
        let _ = writeln!(line, " [{} ms, until {} ms]", took / 1_000_000, crate::time::now_ns() / 1_000_000);
    }
    log::write_console(&line);
}

/// ` timeout=+S.mmms` for a `FUTEX_WAIT` / `FUTEX_WAIT_BITSET` with a
/// timeout: how long from now it is.
fn push_futex_timeout(line: &mut String, args: &[u64; 6]) {
    const WAIT: u64 = 0;
    const WAIT_BITSET: u64 = 9;
    const CLOCK_REALTIME: u64 = 0x100;
    let cmd = args[1] & 0x7f;
    if (cmd != WAIT && cmd != WAIT_BITSET) || args[3] == 0 {
        return;
    }
    let mut raw = [0u8; 16];
    if user::copy_from_user(&mut raw, args[3]).is_err() {
        return;
    }
    let sec = i64::from_le_bytes(raw[..8].try_into().unwrap());
    let nsec = i64::from_le_bytes(raw[8..].try_into().unwrap());
    let mut ns = sec.saturating_mul(1_000_000_000).saturating_add(nsec);
    if cmd == WAIT_BITSET {
        let now = if args[1] & CLOCK_REALTIME != 0 { crate::time::realtime_ns() } else { crate::time::now_ns() };
        ns = ns.saturating_sub(now as i64);
    }
    let _ = write!(line, " timeout={:+}.{:03}s", ns / 1_000_000_000, (ns.abs() / 1_000_000) % 1000);
}

/// ` <label>={handler, flags}` of the `struct sigaction` at `addr`
/// (`sa_handler` and `sa_flags`, the first two 8-byte fields), if readable.
fn push_action(line: &mut String, label: &str, addr: u64) {
    let mut raw = [0u8; 16];
    if user::copy_from_user(&mut raw, addr).is_ok() {
        let handler = u64::from_le_bytes(raw[..8].try_into().unwrap());
        let flags = u64::from_le_bytes(raw[8..].try_into().unwrap());
        let _ = write!(line, "{label}={{{handler:#x}, {flags:#x}}}");
    }
}

/// Which of the first four arguments of call `name` are strings (bits).
fn string_args(name: &str) -> u8 {
    match name {
        "open" | "stat" | "lstat" | "access" | "execve" | "truncate" | "chdir" | "mkdir" | "rmdir" | "creat"
        | "unlink" | "readlink" | "chmod" | "chown" | "lchown" | "mknod" | "statfs" | "umount2" | "chroot" | "acct"
        | "swapon" | "swapoff" | "utimes" | "utime" => 0b1,
        "openat" | "newfstatat" | "faccessat" | "faccessat2" | "readlinkat" | "mkdirat" | "unlinkat" | "fchmodat"
        | "fchownat" | "mknodat" | "utimensat" | "execveat" | "statx" | "futimesat" => 0b10,
        "rename" | "link" | "symlink" | "pivot_root" => 0b11,
        "symlinkat" => 0b101,
        "renameat" | "renameat2" | "linkat" => 0b1010,
        "mount" => 0b111,
        _ => 0,
    }
}

/// The string at user address `addr`, quoted and cut at [`STRING_MAX`].
fn push_string(line: &mut String, addr: u64) {
    if addr == 0 {
        line.push_str("NULL");
        return;
    }
    let mut buf = [0u8; STRING_MAX];
    let mut len = 0;
    let mut ended = false;
    while len < STRING_MAX {
        // Never across a page boundary at once: the next page may be unmapped.
        let at = addr.wrapping_add(len as u64);
        let room = (4096 - (at % 4096)) as usize;
        let n = room.min(STRING_MAX - len);
        if user::copy_from_user(&mut buf[len..len + n], at).is_err() {
            break;
        }
        if let Some(nul) = buf[len..len + n].iter().position(|&b| b == 0) {
            len += nul;
            ended = true;
            break;
        }
        len += n;
    }
    if len == 0 && !ended {
        let _ = write!(line, "{addr:#x}");
        return;
    }
    line.push('"');
    for &b in &buf[..len] {
        match b {
            b'"' | b'\\' => {
                line.push('\\');
                line.push(b as char);
            }
            0x20..=0x7e => line.push(b as char),
            b'\n' => line.push_str("\\n"),
            _ => {
                let _ = write!(line, "\\x{b:02x}");
            }
        }
    }
    line.push('"');
    if !ended {
        line.push_str("...");
    }
}
