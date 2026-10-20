//! seccomp constants: UAPI `linux/seccomp.h` (operations, flags, return
//! values, `struct seccomp_data`, `struct seccomp_notif_sizes`),
//! `linux/audit.h` (`AUDIT_ARCH_X86_64` = `EM_X86_64 | __AUDIT_ARCH_64BIT
//! | __AUDIT_ARCH_LE`, with `EM_X86_64` = 62 from `linux/elf-em.h`),
//! `asm-generic/siginfo.h` (`SYS_SECCOMP`), `linux/prctl.h`; the path
//! limit from seccomp(2) (`ENOMEM`).

/// seccomp(2) operations.
pub const SECCOMP_SET_MODE_STRICT: u32 = 0;
pub const SECCOMP_SET_MODE_FILTER: u32 = 1;
pub const SECCOMP_GET_ACTION_AVAIL: u32 = 2;
pub const SECCOMP_GET_NOTIF_SIZES: u32 = 3;

/// `SECCOMP_SET_MODE_FILTER` flags.
pub const SECCOMP_FILTER_FLAG_TSYNC: u32 = 1 << 0;
pub const SECCOMP_FILTER_FLAG_LOG: u32 = 1 << 1;
pub const SECCOMP_FILTER_FLAG_SPEC_ALLOW: u32 = 1 << 2;
pub const SECCOMP_FILTER_FLAG_NEW_LISTENER: u32 = 1 << 3;
pub const SECCOMP_FILTER_FLAG_TSYNC_ESRCH: u32 = 1 << 4;
pub const SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV: u32 = 1 << 5;
/// Every flag the header defines.
pub const SECCOMP_FILTER_FLAGS: u32 = SECCOMP_FILTER_FLAG_TSYNC
    | SECCOMP_FILTER_FLAG_LOG
    | SECCOMP_FILTER_FLAG_SPEC_ALLOW
    | SECCOMP_FILTER_FLAG_NEW_LISTENER
    | SECCOMP_FILTER_FLAG_TSYNC_ESRCH
    | SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV;

/// Filter return values: the action in the upper 16 bits.
pub const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
pub const SECCOMP_RET_KILL_THREAD: u32 = 0x0000_0000;
pub const SECCOMP_RET_TRAP: u32 = 0x0003_0000;
pub const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
pub const SECCOMP_RET_USER_NOTIF: u32 = 0x7fc0_0000;
pub const SECCOMP_RET_TRACE: u32 = 0x7ff0_0000;
pub const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;
pub const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
/// Masks of the action (with the sign bit) and of the data.
pub const SECCOMP_RET_ACTION_FULL: u32 = 0xffff_0000;
pub const SECCOMP_RET_ACTION: u32 = 0x7fff_0000;
pub const SECCOMP_RET_DATA: u32 = 0x0000_ffff;

/// Modes, as `PR_GET_SECCOMP` reports them.
pub const SECCOMP_MODE_DISABLED: u32 = 0;
pub const SECCOMP_MODE_STRICT: u32 = 1;
pub const SECCOMP_MODE_FILTER: u32 = 2;

/// prctl(2) options.
pub const PR_GET_SECCOMP: u32 = 21;
pub const PR_SET_SECCOMP: u32 = 22;

/// `AUDIT_ARCH_X86_64`.
pub const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

/// `si_code` of a seccomp `SIGSYS`.
pub const SYS_SECCOMP: i32 = 1;

/// `sizeof(struct seccomp_data)`.
pub const SECCOMP_DATA_SIZE: usize = 64;
/// `struct seccomp_notif` (`__u64 id; __u32 pid; __u32 flags; struct
/// seccomp_data data;`) and `struct seccomp_notif_resp` (`__u64 id; __s64
/// val; __s32 error; __u32 flags;`) sizes, for `SECCOMP_GET_NOTIF_SIZES`.
pub const SECCOMP_NOTIF_SIZE: u16 = 80;
pub const SECCOMP_NOTIF_RESP_SIZE: u16 = 24;

/// seccomp(2), `ENOMEM`: "The total length of all filter programs attached
/// to the calling thread would exceed MAX_INSNS_PER_PATH (32768). Note
/// that for the purposes of calculating this limit, each already existing
/// filter program incurs an overhead penalty of 4 instructions."
pub const MAX_INSNS_PER_PATH: usize = 32768;
pub const PATH_PENALTY: usize = 4;
