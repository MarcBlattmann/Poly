//! What the tracer knows about x86-64 Linux system calls: names and numbers
//! (from the coverage map, which imported them from the kernel's public
//! syscall table), how to decode each argument, errno and signal names.
//!
//! Numbers and names are ABI facts. The errno values come from the UAPI
//! headers `asm-generic/errno-base.h` and `asm-generic/errno.h`, the signal
//! numbers from `asm/signal.h` (x86). Argument kinds follow the public man
//! pages (section 2).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The Linux x86-64 entries of the coverage map: the single list of
/// syscall names and numbers in the repository.
const COVERAGE_TABLE: &str = include_str!("../../../coverage/linux/syscall/x86_64.json");

#[derive(Deserialize)]
struct CoverageEntry {
    symbol: String,
    number: Option<u64>,
}

struct Table {
    by_number: BTreeMap<u64, String>,
    by_name: BTreeMap<String, u64>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let entries: Vec<CoverageEntry> = serde_json::from_str(COVERAGE_TABLE).expect("coverage map is valid JSON");
        let mut t = Table { by_number: BTreeMap::new(), by_name: BTreeMap::new() };
        for e in entries {
            if let Some(n) = e.number {
                t.by_number.insert(n, e.symbol.clone());
                t.by_name.insert(e.symbol, n);
            }
        }
        t
    })
}

/// The name of syscall `nr`, or `syscall_<hex>` (strace's spelling) for
/// numbers the table does not have.
pub fn name(nr: u64) -> String {
    table().by_number.get(&nr).cloned().unwrap_or_else(|| format!("syscall_{nr:#x}"))
}

/// The number of the syscall called `name` (also accepts `syscall_<hex>`).
pub fn number(name: &str) -> Option<u64> {
    if let Some(n) = table().by_name.get(name) {
        return Some(*n);
    }
    let hex = name.strip_prefix("syscall_0x")?;
    u64::from_str_radix(hex, 16).ok()
}

/// How an argument is decoded and normalised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// Signed integer, compared exactly (sizes, fds, clock ids, ...).
    Int,
    /// Flags or a code, printed in hex, compared exactly.
    Hex,
    /// An address: masked when compared, except NULL.
    Ptr,
    /// A NUL-terminated string the kernel reads (a path).
    CStr,
    /// A buffer the kernel reads; its length is the argument at this index.
    InBuf(usize),
    /// A buffer the kernel fills; its length is the return value.
    OutBuf,
    /// A process or thread id: masked.
    Pid,
}

/// How a return value is normalised (errors are always `-1 ENAME`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetKind {
    Int,
    /// An address (mmap, brk): masked.
    Addr,
    /// A process or thread id: masked.
    Pid,
    /// A clock reading (`time`, `times`): differs between runs, masked.
    Time,
}

/// Six raw arguments, for calls the table does not describe.
const UNKNOWN: [ArgKind; 6] = [ArgKind::Hex; 6];

/// Argument and return kinds of a syscall. Unknown calls get six `Hex`
/// arguments, which the normaliser treats with a heuristic (see
/// `trace::render_arg`).
pub fn signature(name: &str) -> (&'static [ArgKind], RetKind) {
    known_signature(name).unwrap_or((&UNKNOWN, RetKind::Int))
}

fn known_signature(name: &str) -> Option<(&'static [ArgKind], RetKind)> {
    use ArgKind::*;
    Some(match name {
        "read" | "pread64" => (&[Int, OutBuf, Int, Int][..if name == "read" { 3 } else { 4 }], RetKind::Int),
        "write" => (&[Int, InBuf(2), Int], RetKind::Int),
        "pwrite64" => (&[Int, InBuf(2), Int, Int], RetKind::Int),
        "readv" | "writev" => (&[Int, Ptr, Int], RetKind::Int),
        "open" => (&[CStr, Hex, Hex], RetKind::Int),
        "openat" => (&[Int, CStr, Hex, Hex], RetKind::Int),
        "close" | "dup" | "fdatasync" => (&[Int], RetKind::Int),
        "dup2" => (&[Int, Int], RetKind::Int),
        "dup3" => (&[Int, Int, Hex], RetKind::Int),
        "stat" | "lstat" => (&[CStr, Ptr], RetKind::Int),
        "fstat" => (&[Int, Ptr], RetKind::Int),
        "newfstatat" => (&[Int, CStr, Ptr, Hex], RetKind::Int),
        "statx" => (&[Int, CStr, Hex, Hex, Ptr], RetKind::Int),
        "access" => (&[CStr, Hex], RetKind::Int),
        "faccessat" | "faccessat2" => (&[Int, CStr, Hex, Hex], RetKind::Int),
        "lseek" => (&[Int, Int, Int], RetKind::Int),
        "mmap" => (&[Ptr, Int, Hex, Hex, Int, Hex], RetKind::Addr),
        "mprotect" => (&[Ptr, Int, Hex], RetKind::Int),
        "munmap" => (&[Ptr, Int], RetKind::Int),
        "mremap" => (&[Ptr, Int, Int, Hex, Ptr], RetKind::Addr),
        "madvise" => (&[Ptr, Int, Int], RetKind::Int),
        "brk" => (&[Ptr], RetKind::Addr),
        "rt_sigaction" => (&[Int, Ptr, Ptr, Int], RetKind::Int),
        "rt_sigprocmask" => (&[Int, Ptr, Ptr, Int], RetKind::Int),
        "rt_sigreturn" => (&[], RetKind::Int),
        "sigaltstack" => (&[Ptr, Ptr], RetKind::Int),
        "ioctl" => (&[Int, Hex, Ptr], RetKind::Int),
        "fcntl" => (&[Int, Int, Hex], RetKind::Int),
        "pipe" => (&[Ptr], RetKind::Int),
        "pipe2" => (&[Ptr, Hex], RetKind::Int),
        "sched_yield" | "pause" | "sync" => (&[], RetKind::Int),
        "nanosleep" => (&[Ptr, Ptr], RetKind::Int),
        "clock_gettime" | "clock_getres" => (&[Int, Ptr], RetKind::Int),
        "clock_nanosleep" => (&[Int, Hex, Ptr, Ptr], RetKind::Int),
        "gettimeofday" => (&[Ptr, Ptr], RetKind::Int),
        "time" => (&[Ptr], RetKind::Time),
        "getpid" | "getppid" | "gettid" | "fork" | "vfork" | "setsid" | "getpgrp" => (&[], RetKind::Pid),
        "getuid" | "geteuid" | "getgid" | "getegid" => (&[], RetKind::Int),
        "clone" => (&[Hex, Ptr, Ptr, Ptr, Hex], RetKind::Pid),
        "clone3" => (&[Ptr, Int], RetKind::Pid),
        "execve" => (&[CStr, Ptr, Ptr], RetKind::Int),
        "exit" | "exit_group" => (&[Int], RetKind::Int),
        "wait4" => (&[Pid, Ptr, Hex, Ptr], RetKind::Pid),
        "kill" => (&[Pid, Int], RetKind::Int),
        "tgkill" => (&[Pid, Pid, Int], RetKind::Int),
        "tkill" => (&[Pid, Int], RetKind::Int),
        "rt_sigqueueinfo" => (&[Pid, Int, Ptr], RetKind::Int),
        "rt_tgsigqueueinfo" => (&[Pid, Pid, Int, Ptr], RetKind::Int),
        "uname" | "sysinfo" => (&[Ptr], RetKind::Int),
        "arch_prctl" => (&[Hex, Ptr], RetKind::Int),
        "prctl" => (&[Int, Hex, Hex, Hex, Hex], RetKind::Int),
        "set_tid_address" => (&[Ptr], RetKind::Pid),
        "set_robust_list" => (&[Ptr, Int], RetKind::Int),
        "get_robust_list" => (&[Pid, Ptr, Ptr], RetKind::Int),
        "rseq" => (&[Ptr, Int, Hex, Hex], RetKind::Int),
        "futex" => (&[Ptr, Int, Int, Ptr, Ptr, Int], RetKind::Int),
        "prlimit64" => (&[Pid, Int, Ptr, Ptr], RetKind::Int),
        "getrlimit" | "setrlimit" => (&[Int, Ptr], RetKind::Int),
        "readlink" => (&[CStr, Ptr, Int], RetKind::Int),
        "readlinkat" => (&[Int, CStr, Ptr, Int], RetKind::Int),
        "getcwd" => (&[Ptr, Int], RetKind::Int),
        "chdir" | "rmdir" | "unlink" => (&[CStr], RetKind::Int),
        "mkdir" => (&[CStr, Hex], RetKind::Int),
        "getdents64" => (&[Int, Ptr, Int], RetKind::Int),
        "getrandom" => (&[Ptr, Int, Hex], RetKind::Int),
        "poll" => (&[Ptr, Int, Int], RetKind::Int),
        "sched_getaffinity" => (&[Pid, Int, Ptr], RetKind::Int),
        // Calls of the app tests' programs (coreutils, grep, sed).
        "umask" => (&[Hex], RetKind::Int),
        "statfs" => (&[CStr, Ptr], RetKind::Int),
        "fstatfs" => (&[Int, Ptr], RetKind::Int),
        "mkdirat" => (&[Int, CStr, Hex], RetKind::Int),
        "unlinkat" => (&[Int, CStr, Hex], RetKind::Int),
        "fchmod" => (&[Int, Hex], RetKind::Int),
        "fchmodat" => (&[Int, CStr, Hex], RetKind::Int),
        "fchown" => (&[Int, Int, Int], RetKind::Int),
        "utimensat" => (&[Int, CStr, Ptr, Hex], RetKind::Int),
        "getxattr" | "lgetxattr" => (&[CStr, CStr, Ptr, Int], RetKind::Int),
        "fgetxattr" => (&[Int, CStr, Ptr, Int], RetKind::Int),
        "listxattr" | "llistxattr" => (&[CStr, Ptr, Int], RetKind::Int),
        "flistxattr" => (&[Int, Ptr, Int], RetKind::Int),
        "fsetxattr" => (&[Int, CStr, Ptr, Int, Hex], RetKind::Int),
        "socket" => (&[Int, Hex, Int], RetKind::Int),
        "connect" => (&[Int, Ptr, Int], RetKind::Int),
        "fadvise64" => (&[Int, Int, Int, Int], RetKind::Int),
        "copy_file_range" => (&[Int, Ptr, Int, Ptr, Int, Hex], RetKind::Int),
        "ftruncate" => (&[Int, Int], RetKind::Int),
        "renameat2" => (&[Int, CStr, Int, CStr, Hex], RetKind::Int),
        "symlinkat" => (&[CStr, Int, CStr], RetKind::Int),
        "fsync" | "fchdir" => (&[Int], RetKind::Int),
        "rename" => (&[CStr, CStr], RetKind::Int),
        // Process groups and sessions: their ids differ between runs.
        "getpgid" | "getsid" => (&[Pid], RetKind::Pid),
        "setpgid" => (&[Pid, Pid], RetKind::Int),
        "waitid" => (&[Int, Pid, Ptr, Hex, Ptr], RetKind::Int),
        // Clock readings returned as the result.
        "times" => (&[Ptr], RetKind::Time),
        _ => return None,
    })
}

/// Whether `name` is in the signature table (so its arguments are decoded
/// rather than printed as six raw values).
pub fn is_known_signature(name: &str) -> bool {
    known_signature(name).is_some()
}

const ERRNO: &[(i64, &str)] = &[
    (1, "EPERM"),
    (2, "ENOENT"),
    (3, "ESRCH"),
    (4, "EINTR"),
    (5, "EIO"),
    (6, "ENXIO"),
    (7, "E2BIG"),
    (8, "ENOEXEC"),
    (9, "EBADF"),
    (10, "ECHILD"),
    (11, "EAGAIN"),
    (12, "ENOMEM"),
    (13, "EACCES"),
    (14, "EFAULT"),
    (15, "ENOTBLK"),
    (16, "EBUSY"),
    (17, "EEXIST"),
    (18, "EXDEV"),
    (19, "ENODEV"),
    (20, "ENOTDIR"),
    (21, "EISDIR"),
    (22, "EINVAL"),
    (23, "ENFILE"),
    (24, "EMFILE"),
    (25, "ENOTTY"),
    (26, "ETXTBSY"),
    (27, "EFBIG"),
    (28, "ENOSPC"),
    (29, "ESPIPE"),
    (30, "EROFS"),
    (31, "EMLINK"),
    (32, "EPIPE"),
    (33, "EDOM"),
    (34, "ERANGE"),
    (35, "EDEADLK"),
    (36, "ENAMETOOLONG"),
    (37, "ENOLCK"),
    (38, "ENOSYS"),
    (39, "ENOTEMPTY"),
    (40, "ELOOP"),
    (42, "ENOMSG"),
    (61, "ENODATA"),
    (75, "EOVERFLOW"),
    (84, "EILSEQ"),
    (88, "ENOTSOCK"),
    (95, "EOPNOTSUPP"),
    (98, "EADDRINUSE"),
    (104, "ECONNRESET"),
    (110, "ETIMEDOUT"),
    (111, "ECONNREFUSED"),
    (115, "EINPROGRESS"),
];

/// `EFAULT` for 14; `E<n>` for numbers not in the table.
pub fn errno_name(errno: i64) -> String {
    ERRNO.iter().find(|(n, _)| *n == errno).map_or_else(|| format!("E{errno}"), |(_, s)| s.to_string())
}

pub fn errno_number(name: &str) -> Option<i64> {
    ERRNO.iter().find(|(_, s)| *s == name).map(|(n, _)| *n).or_else(|| name.strip_prefix('E')?.parse().ok())
}

/// The kernel returns errors as -1..-4095 in rax.
pub fn is_error(ret: i64) -> bool {
    (-4095..0).contains(&ret)
}

const SIGNALS: &[&str] = &[
    "SIGHUP",
    "SIGINT",
    "SIGQUIT",
    "SIGILL",
    "SIGTRAP",
    "SIGABRT",
    "SIGBUS",
    "SIGFPE",
    "SIGKILL",
    "SIGUSR1",
    "SIGSEGV",
    "SIGUSR2",
    "SIGPIPE",
    "SIGALRM",
    "SIGTERM",
    "SIGSTKFLT",
    "SIGCHLD",
    "SIGCONT",
    "SIGSTOP",
    "SIGTSTP",
    "SIGTTIN",
    "SIGTTOU",
    "SIGURG",
    "SIGXCPU",
    "SIGXFSZ",
    "SIGVTALRM",
    "SIGPROF",
    "SIGWINCH",
    "SIGIO",
    "SIGPWR",
    "SIGSYS",
];

pub fn signal_name(sig: i32) -> String {
    usize::try_from(sig)
        .ok()
        .and_then(|s| s.checked_sub(1))
        .and_then(|i| SIGNALS.get(i))
        .map_or_else(|| format!("SIG{sig}"), |s| s.to_string())
}

pub fn signal_number(name: &str) -> Option<i32> {
    SIGNALS.iter().position(|s| *s == name).map(|i| i as i32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_numbers_come_from_the_coverage_map() {
        assert_eq!(name(1), "write");
        assert_eq!(name(231), "exit_group");
        assert_eq!(number("brk"), Some(12));
        assert_eq!(name(1000), "syscall_0x3e8");
        assert_eq!(number("syscall_0x3e8"), Some(1000));
    }

    #[test]
    fn errno_and_signal_tables() {
        assert_eq!(errno_name(14), "EFAULT");
        assert_eq!(errno_number("ENOSYS"), Some(38));
        assert_eq!(errno_name(200), "E200");
        assert_eq!(errno_number("E200"), Some(200));
        assert_eq!(signal_name(11), "SIGSEGV");
        assert_eq!(signal_number("SIGKILL"), Some(9));
        assert_eq!(signal_name(64), "SIG64");
        assert!(is_error(-14) && !is_error(0) && !is_error(-4096));
    }

    #[test]
    fn signatures() {
        assert_eq!(signature("write").0, [ArgKind::Int, ArgKind::InBuf(2), ArgKind::Int]);
        assert_eq!(signature("read").0.len(), 3);
        assert_eq!(signature("pread64").0.len(), 4);
        assert!(is_known_signature("mmap"));
        assert!(!is_known_signature("bpf"));
        assert_eq!(signature("getpgid"), (&[ArgKind::Pid][..], RetKind::Pid));
        assert_eq!(signature("time").1, RetKind::Time);
        assert_eq!(signature("times").1, RetKind::Time);
    }
}
