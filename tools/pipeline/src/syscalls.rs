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
        // x86-64 order: flags, stack, parent_tid, child_tid, tls (clone(2),
        // NOTES). With CLONE_SETTLS the last one is the new thread's TLS
        // address, so it is masked like any pointer, not compared as a number.
        "clone" => (&[Hex, Ptr, Ptr, Ptr, Ptr], RetKind::Pid),
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
        // prctl, fcntl, futex, arch_prctl and waitid are refined by their
        // option or command argument in `call_signature`.
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

/// Argument and return kinds of one call of `name` made with the raw
/// arguments `raw`: `signature(name)`, except for the multiplexed calls
/// whose later arguments change kind with an option or command argument
/// (`prctl`, `arch_prctl`, `fcntl`, `futex`, `waitid`). There a user
/// address that one option takes is decoded as a pointer (masked when
/// compared), while another option's integer in the same slot is still
/// compared as a number.
pub fn call_signature(name: &str, raw: &[u64; 6]) -> (&'static [ArgKind], RetKind) {
    // The selectors are C `int`s (`unsigned int` for fcntl's cmd): the
    // upper half of the register is not part of the value.
    let sel = |i: usize| raw[i] as u32;
    let refined = match name {
        "prctl" => prctl_args(sel(0), raw[1]),
        "arch_prctl" => arch_prctl_args(sel(0)),
        "fcntl" => return fcntl_signature(sel(1)),
        "futex" => futex_args(sel(1)),
        "waitid" => waitid_args(sel(0)),
        _ => None,
    };
    match refined {
        Some(args) => (args, signature(name).1),
        None => signature(name),
    }
}

/// `prctl(2)` options and their argument kinds. Values from
/// `<linux/prctl.h>`; which argument is a user address from the prctl(2)
/// man page. Options not listed keep the default `[Int, Hex, Hex, Hex,
/// Hex]` (their arguments are integers, flags or unused).
mod pr {
    pub const GET_PDEATHSIG: u32 = 2; // int *arg2
    pub const GET_UNALIGN: u32 = 5; // unsigned int *arg2
    pub const GET_FPEMU: u32 = 9; // int *arg2
    pub const GET_FPEXC: u32 = 11; // int *arg2
    pub const SET_NAME: u32 = 15; // char *arg2 (read, up to 16 bytes)
    pub const GET_NAME: u32 = 16; // char arg2[16] (written)
    pub const GET_ENDIAN: u32 = 19; // int *arg2
    pub const SET_SECCOMP: u32 = 22; // arg2 mode, struct sock_fprog *arg3
    pub const GET_TSC: u32 = 25; // int *arg2
    pub const SET_MM: u32 = 35; // arg2 sub-option, see below
    pub const GET_CHILD_SUBREAPER: u32 = 37; // int *arg2
    pub const GET_TID_ADDRESS: u32 = 40; // int **arg2
    pub const SET_SYSCALL_USER_DISPATCH: u32 = 59; // arg2 mode, arg3 offset (address), arg4 length, char *arg5
    pub const SCHED_CORE: u32 = 62; // arg2 command, pid_t arg3, arg4 pid type, unsigned long *arg5
    pub const SET_PTRACER: u32 = 0x5961_6d61; // pid_t arg2 (or PR_SET_PTRACER_ANY)
    pub const SET_VMA: u32 = 0x5356_4d41; // arg2 attribute, arg3 address, arg4 size, char *arg5
    pub const GET_AUXV: u32 = 0x4155_5856; // void *arg2, unsigned long arg3 size

    // PR_SET_MM sub-options (arg2).
    pub const SET_MM_ENV_END: u64 = 11; // 1..=11: arg3 is an address in the map
    pub const SET_MM_AUXV: u64 = 12; // arg3 address of the vector, arg4 its size
    pub const SET_MM_EXE_FILE: u64 = 13; // arg3 a file descriptor
    pub const SET_MM_MAP: u64 = 14; // struct prctl_mm_map *arg3, arg4 its size
    pub const SET_MM_MAP_SIZE: u64 = 15; // unsigned int *arg3
}

fn prctl_args(option: u32, arg2: u64) -> Option<&'static [ArgKind]> {
    use ArgKind::*;
    Some(match option {
        pr::GET_PDEATHSIG
        | pr::GET_UNALIGN
        | pr::GET_FPEMU
        | pr::GET_FPEXC
        | pr::SET_NAME
        | pr::GET_NAME
        | pr::GET_ENDIAN
        | pr::GET_TSC
        | pr::GET_CHILD_SUBREAPER
        | pr::GET_TID_ADDRESS => &[Int, Ptr, Hex, Hex, Hex],
        pr::SET_SECCOMP => &[Int, Int, Ptr, Hex, Hex],
        pr::SET_MM => match arg2 {
            1..=pr::SET_MM_ENV_END | pr::SET_MM_MAP_SIZE => &[Int, Int, Ptr, Hex, Hex],
            pr::SET_MM_AUXV | pr::SET_MM_MAP => &[Int, Int, Ptr, Int, Hex],
            pr::SET_MM_EXE_FILE => &[Int, Int, Int, Hex, Hex],
            _ => return None,
        },
        pr::SET_SYSCALL_USER_DISPATCH => &[Int, Int, Ptr, Int, Ptr],
        pr::SCHED_CORE => &[Int, Int, Pid, Int, Ptr],
        pr::SET_PTRACER => &[Int, Pid, Hex, Hex, Hex],
        pr::SET_VMA => &[Int, Int, Ptr, Int, Ptr],
        pr::GET_AUXV => &[Int, Ptr, Int, Hex, Hex],
        _ => return None,
    })
}

/// `arch_prctl(2)` options that take an integer rather than an address
/// (values from `<asm/prctl.h>`; arch_prctl(2), and the kernel's
/// Documentation/arch/x86 xstate.rst and shstk.rst for the newer ones).
/// Every other option takes or returns through an address (`ARCH_SET_FS`,
/// `ARCH_GET_FS`, `ARCH_GET_XCOMP_SUPP`, `ARCH_MAP_VDSO_64`, ...): the
/// default `[Hex, Ptr]`.
fn arch_prctl_args(option: u32) -> Option<&'static [ArgKind]> {
    use ArgKind::*;
    const ARCH_SET_CPUID: u32 = 0x1012; // 0 or 1
    const ARCH_REQ_XCOMP_PERM: u32 = 0x1023; // xstate feature number
    const ARCH_REQ_XCOMP_GUEST_PERM: u32 = 0x1025; // xstate feature number
    const ARCH_ENABLE_TAGGED_ADDR: u32 = 0x4002; // number of tag bits
    const ARCH_SHSTK_ENABLE: u32 = 0x5001; // ..=ARCH_SHSTK_UNLOCK: feature bits
    const ARCH_SHSTK_UNLOCK: u32 = 0x5004;
    Some(match option {
        ARCH_SET_CPUID | ARCH_REQ_XCOMP_PERM | ARCH_REQ_XCOMP_GUEST_PERM | ARCH_ENABLE_TAGGED_ADDR => &[Hex, Int],
        ARCH_SHSTK_ENABLE..=ARCH_SHSTK_UNLOCK => &[Hex, Hex],
        _ => return None,
    })
}

/// `fcntl(2)` by command (values from `<asm-generic/fcntl.h>` and
/// `<linux/fcntl.h>`, argument types from fcntl(2)): the lock and owner
/// commands take a struct pointer, `F_SETOWN` a pid (or -pgid) and
/// `F_GETOWN` returns one, the `(void)` commands have no third argument
/// (it is not shown or compared, as strace leaves it out), and the
/// descriptor, signal, lease and size commands take an int. Flag commands
/// (`F_SETFD`, `F_SETFL`, `F_NOTIFY`, `F_ADD_SEALS`) and unknown ones keep
/// the default `[Int, Int, Hex]`.
fn fcntl_signature(cmd: u32) -> (&'static [ArgKind], RetKind) {
    use ArgKind::*;
    const F_DUPFD: u32 = 0;
    const F_GETFD: u32 = 1;
    const F_GETFL: u32 = 3;
    const F_GETLK: u32 = 5;
    const F_SETLK: u32 = 6;
    const F_SETLKW: u32 = 7;
    const F_SETOWN: u32 = 8;
    const F_GETOWN: u32 = 9;
    const F_SETSIG: u32 = 10;
    const F_GETSIG: u32 = 11;
    const F_SETOWN_EX: u32 = 15;
    const F_GETOWN_EX: u32 = 16;
    const F_OFD_GETLK: u32 = 36;
    const F_OFD_SETLK: u32 = 37;
    const F_OFD_SETLKW: u32 = 38;
    const F_LINUX_SPECIFIC_BASE: u32 = 1024;
    const F_SETLEASE: u32 = F_LINUX_SPECIFIC_BASE;
    const F_GETLEASE: u32 = F_LINUX_SPECIFIC_BASE + 1;
    const F_DUPFD_CLOEXEC: u32 = F_LINUX_SPECIFIC_BASE + 6;
    const F_SETPIPE_SZ: u32 = F_LINUX_SPECIFIC_BASE + 7;
    const F_GETPIPE_SZ: u32 = F_LINUX_SPECIFIC_BASE + 8;
    const F_GET_SEALS: u32 = F_LINUX_SPECIFIC_BASE + 10;
    // F_GET_RW_HINT, F_SET_RW_HINT, F_GET_FILE_RW_HINT, F_SET_FILE_RW_HINT: uint64_t *.
    const F_GET_RW_HINT: u32 = F_LINUX_SPECIFIC_BASE + 11;
    const F_SET_FILE_RW_HINT: u32 = F_LINUX_SPECIFIC_BASE + 14;
    match cmd {
        F_GETLK | F_SETLK | F_SETLKW | F_OFD_GETLK | F_OFD_SETLK | F_OFD_SETLKW | F_SETOWN_EX | F_GETOWN_EX => {
            (&[Int, Int, Ptr], RetKind::Int)
        }
        F_GET_RW_HINT..=F_SET_FILE_RW_HINT => (&[Int, Int, Ptr], RetKind::Int),
        F_SETOWN => (&[Int, Int, Pid], RetKind::Int),
        F_GETOWN => (&[Int, Int], RetKind::Pid),
        F_GETFD | F_GETFL | F_GETSIG | F_GETLEASE | F_GETPIPE_SZ | F_GET_SEALS => (&[Int, Int], RetKind::Int),
        F_DUPFD | F_DUPFD_CLOEXEC | F_SETSIG | F_SETLEASE | F_SETPIPE_SZ => (&[Int, Int, Int], RetKind::Int),
        _ => signature("fcntl"),
    }
}

/// `futex(2)` by operation (values from `<linux/futex.h>`; the private
/// and clock flags are not part of it). The fourth argument is a timeout
/// pointer for the waits and the integer `val2` for the requeues and
/// `FUTEX_WAKE_OP`; `uaddr2` is an address for every operation that uses
/// it; `val3` is a count or a bit mask. Other operations keep the default
/// `[Ptr, Int, Int, Ptr, Ptr, Int]` (timeout, unused `uaddr2`).
fn futex_args(op: u32) -> Option<&'static [ArgKind]> {
    use ArgKind::*;
    const FUTEX_PRIVATE_FLAG: u32 = 128;
    const FUTEX_CLOCK_REALTIME: u32 = 256;
    const FUTEX_REQUEUE: u32 = 3;
    const FUTEX_CMP_REQUEUE: u32 = 4;
    const FUTEX_WAKE_OP: u32 = 5;
    const FUTEX_WAIT_BITSET: u32 = 9;
    const FUTEX_WAKE_BITSET: u32 = 10;
    const FUTEX_CMP_REQUEUE_PI: u32 = 12;
    Some(match op & !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME) {
        FUTEX_REQUEUE | FUTEX_CMP_REQUEUE | FUTEX_CMP_REQUEUE_PI => &[Ptr, Int, Int, Int, Ptr, Int],
        FUTEX_WAKE_OP => &[Ptr, Int, Int, Int, Ptr, Hex],
        FUTEX_WAIT_BITSET | FUTEX_WAKE_BITSET => &[Ptr, Int, Int, Ptr, Ptr, Hex],
        _ => return None,
    })
}

/// `waitid(2)`: with `P_PIDFD` (`<linux/wait.h>`) the id is a file
/// descriptor, compared as a number, not a pid.
fn waitid_args(idtype: u32) -> Option<&'static [ArgKind]> {
    use ArgKind::*;
    const P_PIDFD: u32 = 3;
    (idtype == P_PIDFD).then_some(&[Int, Int, Ptr, Hex, Ptr][..])
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

    fn args(name: &str, raw: [u64; 6]) -> &'static [ArgKind] {
        call_signature(name, &raw).0
    }

    /// prctl options that take a user address in arg2 (prctl(2)) decode it
    /// as a pointer; options with an integer there keep comparing it.
    #[test]
    fn prctl_pointer_arguments_follow_the_option() {
        use ArgKind::*;
        let stack = 0x7ffd_1234_5670;
        for option in [2, 5, 9, 11, 15, 16, 19, 25, 37, 40] {
            assert_eq!(args("prctl", [option, stack, 0, 0, 0, 0]), [Int, Ptr, Hex, Hex, Hex], "option {option}");
        }
        // PR_SET_CHILD_SUBREAPER, PR_SET_PDEATHSIG, PR_SET_DUMPABLE, PR_SET_NO_NEW_PRIVS: integers.
        for option in [36, 1, 4, 38] {
            assert_eq!(args("prctl", [option, 1, 0, 0, 0, 0]), signature("prctl").0, "option {option}");
        }
        // The option is an int: garbage in the upper half does not change it.
        assert_eq!(args("prctl", [0xdead_0000_0000_0025, stack, 0, 0, 0, 0])[1], Ptr);
        assert_eq!(args("prctl", [22, 2, stack, 0, 0, 0]), [Int, Int, Ptr, Hex, Hex]); // PR_SET_SECCOMP
        assert_eq!(args("prctl", [0x5961_6d61, 4321, 0, 0, 0, 0])[1], Pid); // PR_SET_PTRACER
        assert_eq!(args("prctl", [0x4155_5856, stack, 64, 0, 0, 0]), [Int, Ptr, Int, Hex, Hex]); // PR_GET_AUXV
        assert_eq!(args("prctl", [0x5356_4d41, 0, stack, 4096, stack, 0]), [Int, Int, Ptr, Int, Ptr]); // PR_SET_VMA
        assert_eq!(args("prctl", [62, 0, 4321, 0, stack, 0]), [Int, Int, Pid, Int, Ptr]); // PR_SCHED_CORE
        assert_eq!(args("prctl", [59, 1, 0x40_1000, 16, stack, 0]), [Int, Int, Ptr, Int, Ptr]); // PR_SET_SYSCALL_USER_DISPATCH
        // PR_SET_MM by sub-option: an address, the auxv and map structs with a size, an fd.
        assert_eq!(args("prctl", [35, 6, stack, 0, 0, 0]), [Int, Int, Ptr, Hex, Hex]);
        assert_eq!(args("prctl", [35, 12, stack, 64, 0, 0]), [Int, Int, Ptr, Int, Hex]);
        assert_eq!(args("prctl", [35, 14, stack, 104, 0, 0]), [Int, Int, Ptr, Int, Hex]);
        assert_eq!(args("prctl", [35, 15, stack, 0, 0, 0]), [Int, Int, Ptr, Hex, Hex]);
        assert_eq!(args("prctl", [35, 13, 3, 0, 0, 0]), [Int, Int, Int, Hex, Hex]);
        assert_eq!(args("prctl", [35, 99, 0, 0, 0, 0]), signature("prctl").0);
        assert_eq!(call_signature("prctl", &[37, stack, 0, 0, 0, 0]).1, RetKind::Int);
    }

    #[test]
    fn fcntl_arguments_and_result_follow_the_command() {
        use ArgKind::*;
        // F_GETLK, F_SETLK, F_SETLKW, F_SETOWN_EX, F_GETOWN_EX, F_OFD_*, F_GET_RW_HINT..F_SET_FILE_RW_HINT.
        for cmd in [5, 6, 7, 15, 16, 36, 37, 38, 1035, 1036, 1037, 1038] {
            assert_eq!(call_signature("fcntl", &[3, cmd, 0x7ffd_0000, 0, 0, 0]), (&[Int, Int, Ptr][..], RetKind::Int));
        }
        assert_eq!(call_signature("fcntl", &[3, 8, 4321, 0, 0, 0]), (&[Int, Int, Pid][..], RetKind::Int)); // F_SETOWN
        assert_eq!(call_signature("fcntl", &[3, 9, 0, 0, 0, 0]), (&[Int, Int][..], RetKind::Pid)); // F_GETOWN
        // The (void) commands: F_GETFD, F_GETFL, F_GETSIG, F_GETLEASE, F_GETPIPE_SZ, F_GET_SEALS.
        for cmd in [1, 3, 11, 1025, 1032, 1034] {
            assert_eq!(args("fcntl", [3, cmd, 0x7ffd_0000, 0, 0, 0]), [Int, Int], "cmd {cmd}");
        }
        // F_DUPFD, F_SETSIG, F_SETLEASE, F_DUPFD_CLOEXEC, F_SETPIPE_SZ: an int.
        for cmd in [0, 10, 1024, 1030, 1031] {
            assert_eq!(args("fcntl", [3, cmd, 10, 0, 0, 0]), [Int, Int, Int], "cmd {cmd}");
        }
        // F_SETFD, F_SETFL: flags, as before.
        assert_eq!(args("fcntl", [3, 2, 1, 0, 0, 0]), [Int, Int, Hex]);
        assert_eq!(args("fcntl", [3, 4, 0o4000, 0, 0, 0]), [Int, Int, Hex]);
    }

    #[test]
    fn arch_prctl_futex_and_waitid_by_option() {
        use ArgKind::*;
        // ARCH_SET_GS, ARCH_SET_FS, ARCH_GET_FS, ARCH_GET_GS, ARCH_GET_XCOMP_SUPP, ARCH_MAP_VDSO_64: addresses.
        for option in [0x1001, 0x1002, 0x1003, 0x1004, 0x1021, 0x2003] {
            assert_eq!(args("arch_prctl", [option, 0x7ffd_0000, 0, 0, 0, 0]), [Hex, Ptr]);
        }
        // ARCH_SET_CPUID, ARCH_REQ_XCOMP_PERM, ARCH_ENABLE_TAGGED_ADDR: integers.
        for option in [0x1012, 0x1023, 0x4002] {
            assert_eq!(args("arch_prctl", [option, 1, 0, 0, 0, 0]), [Hex, Int]);
        }
        assert_eq!(args("arch_prctl", [0x5001, 1, 0, 0, 0, 0]), [Hex, Hex]); // ARCH_SHSTK_ENABLE

        // FUTEX_WAIT(_PRIVATE): a timeout pointer.
        assert_eq!(args("futex", [0x1000, 0, 1, 0x7ffd_0000, 0, 0]), signature("futex").0);
        assert_eq!(args("futex", [0x1000, 128, 1, 0x7ffd_0000, 0, 0])[3], Ptr);
        // FUTEX_CMP_REQUEUE_PRIVATE: val2 is a count, uaddr2 an address.
        assert_eq!(args("futex", [0x1000, 4 | 128, 1, 0x7fff_ffff, 0x2000, 7]), [Ptr, Int, Int, Int, Ptr, Int]);
        assert_eq!(args("futex", [0x1000, 3, 1, 5, 0x2000, 0]), [Ptr, Int, Int, Int, Ptr, Int]); // FUTEX_REQUEUE
        assert_eq!(args("futex", [0x1000, 5, 1, 1, 0x2000, 0x1800_0001]), [Ptr, Int, Int, Int, Ptr, Hex]); // FUTEX_WAKE_OP
        // FUTEX_WAIT_BITSET | FUTEX_CLOCK_REALTIME: timeout pointer, bit mask.
        assert_eq!(args("futex", [0x1000, 9 | 256, 1, 0x7ffd_0000, 0, 0xffff_ffff]), [Ptr, Int, Int, Ptr, Ptr, Hex]);

        assert_eq!(args("waitid", [3, 5, 0x7ffd_0000, 4, 0, 0]), [Int, Int, Ptr, Hex, Ptr]); // P_PIDFD
        assert_eq!(args("waitid", [1, 4321, 0x7ffd_0000, 4, 0, 0]), signature("waitid").0); // P_PID
        // clone's last argument is the TLS address on x86-64.
        assert_eq!(signature("clone").0[4], Ptr);
        // Calls that are not multiplexed are unchanged.
        assert_eq!(call_signature("write", &[1, 2, 3, 0, 0, 0]), signature("write"));
    }
}
