//! Constants and structure layouts of the x86-64 Linux user ABI.
//!
//! Every value cites the Linux UAPI header it comes from (the headers that
//! define the user ABI, installed as `linux-libc-dev` on Debian/Ubuntu).
//! Structures are encoded to and decoded from little-endian bytes here, so
//! the personality never casts user memory to a Rust type.

/// Page size of x86-64 (4 KiB base pages).
pub const PAGE_SIZE: u64 = 4096;

/// End (exclusive) of the user half of the x86-64 address space: the lower
/// canonical half for 48-bit virtual addresses (Intel SDM vol. 1, 3.3.7.1).
/// A segment base at or above this cannot belong to the process.
pub const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;

/// The most bytes one `read`/`write`-family call transfers. read(2) NOTES:
/// "On Linux, read() (and similar system calls) will transfer at most
/// 0x7ffff000 (2,147,479,552) bytes, returning the number of bytes actually
/// transferred."
pub const MAX_RW_COUNT: u64 = 0x7fff_f000;

/// Round `value` up to a multiple of [`PAGE_SIZE`]; `None` on overflow.
pub const fn page_round_up(value: u64) -> Option<u64> {
    match value.checked_add(PAGE_SIZE - 1) {
        Some(v) => Some(v & !(PAGE_SIZE - 1)),
        None => None,
    }
}

/// True if `value` is a multiple of [`PAGE_SIZE`].
pub const fn page_aligned(value: u64) -> bool {
    value.is_multiple_of(PAGE_SIZE)
}

/// Memory protection bits (`asm-generic/mman-common.h`).
pub mod prot {
    pub const PROT_NONE: u32 = 0x0;
    pub const PROT_READ: u32 = 0x1;
    pub const PROT_WRITE: u32 = 0x2;
    pub const PROT_EXEC: u32 = 0x4;
    pub const PROT_SEM: u32 = 0x8;
    pub const PROT_GROWSDOWN: u32 = 0x0100_0000;
    pub const PROT_GROWSUP: u32 = 0x0200_0000;
}

/// `mmap` flags (`linux/mman.h`, `asm-generic/mman-common.h`,
/// `asm-generic/mman.h`, x86 `asm/mman.h`, `asm-generic/hugetlb_encode.h`).
pub mod map {
    pub const MAP_SHARED: u32 = 0x01;
    pub const MAP_PRIVATE: u32 = 0x02;
    pub const MAP_SHARED_VALIDATE: u32 = 0x03;
    pub const MAP_TYPE: u32 = 0x0f;
    pub const MAP_FIXED: u32 = 0x10;
    pub const MAP_ANONYMOUS: u32 = 0x20;
    pub const MAP_32BIT: u32 = 0x40;
    pub const MAP_GROWSDOWN: u32 = 0x0100;
    pub const MAP_DENYWRITE: u32 = 0x0800;
    pub const MAP_EXECUTABLE: u32 = 0x1000;
    pub const MAP_LOCKED: u32 = 0x2000;
    pub const MAP_NORESERVE: u32 = 0x4000;
    pub const MAP_POPULATE: u32 = 0x8000;
    pub const MAP_NONBLOCK: u32 = 0x1_0000;
    pub const MAP_STACK: u32 = 0x2_0000;
    pub const MAP_HUGETLB: u32 = 0x4_0000;
    pub const MAP_SYNC: u32 = 0x8_0000;
    pub const MAP_FIXED_NOREPLACE: u32 = 0x10_0000;
    pub const MAP_UNINITIALIZED: u32 = 0x400_0000;
    /// `MAP_HUGE_SHIFT` / `MAP_HUGE_MASK`: the huge page size (log2) used
    /// with `MAP_HUGETLB`, in bits 26-31. Bit 26 doubles as
    /// `MAP_UNINITIALIZED` for non-hugetlb mappings.
    pub const MAP_HUGE_SHIFT: u32 = 26;
    pub const MAP_HUGE_MASK: u32 = 0x3f;
}

/// `arch_prctl` operations (x86 `asm/prctl.h`).
pub mod arch_prctl {
    pub const ARCH_SET_GS: u32 = 0x1001;
    pub const ARCH_SET_FS: u32 = 0x1002;
    pub const ARCH_GET_FS: u32 = 0x1003;
    pub const ARCH_GET_GS: u32 = 0x1004;
}

/// `getrandom` flags (`linux/random.h`).
pub mod random {
    pub const GRND_NONBLOCK: u32 = 0x0001;
    pub const GRND_RANDOM: u32 = 0x0002;
    pub const GRND_INSECURE: u32 = 0x0004;
}

/// Clock ids (`linux/time.h`).
pub mod clock {
    pub const CLOCK_REALTIME: i32 = 0;
    pub const CLOCK_MONOTONIC: i32 = 1;
    pub const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;
    pub const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
    pub const CLOCK_MONOTONIC_RAW: i32 = 4;
    pub const CLOCK_REALTIME_COARSE: i32 = 5;
    pub const CLOCK_MONOTONIC_COARSE: i32 = 6;
    pub const CLOCK_BOOTTIME: i32 = 7;
}

/// ioctl requests (`asm-generic/ioctls.h`). The terminal requests all have
/// type byte `'T'` (0x54) in bits 8-15; so do a few generic file requests
/// (`FIONREAD`, `FIONBIO`, `FIONCLEX`, `FIOCLEX`, `FIOASYNC`, `FIOQSIZE`)
/// that apply to files of every kind.
pub mod ioctl {
    pub const TCGETS: u32 = 0x5401;
    pub const TCSETS: u32 = 0x5402;
    pub const TIOCGWINSZ: u32 = 0x5413;
    pub const FIONREAD: u32 = 0x541B;
    pub const FIONBIO: u32 = 0x5421;
    pub const FIONCLEX: u32 = 0x5450;
    pub const FIOCLEX: u32 = 0x5451;
    pub const FIOASYNC: u32 = 0x5452;
    pub const FIOQSIZE: u32 = 0x5460;
    /// The type byte of the terminal requests.
    pub const TERMINAL_TYPE: u32 = 0x54;

    /// True for terminal requests: type `'T'`, except the generic file
    /// requests listed above.
    pub const fn is_terminal_request(request: u32) -> bool {
        (request >> 8) & 0xff == TERMINAL_TYPE
            && !matches!(request, FIONREAD | FIONBIO | FIONCLEX | FIOCLEX | FIOASYNC | FIOQSIZE)
    }
}

/// Signals (`asm-generic/signal.h`, `asm-generic/signal-defs.h`, x86
/// `asm/signal.h`, `asm-generic/siginfo.h`, x86 `asm/ucontext.h`, x86
/// `asm/sigcontext.h`).
pub mod signal {
    /// `_NSIG`: signals are numbered 1 to 64.
    pub const NSIG: u32 = 64;
    /// `sizeof(sigset_t)` as the kernel defines it: `_NSIG / 8` bytes.
    pub const SIGSET_SIZE: u64 = 8;

    pub const SIGHUP: u32 = 1;
    pub const SIGINT: u32 = 2;
    pub const SIGQUIT: u32 = 3;
    pub const SIGILL: u32 = 4;
    pub const SIGTRAP: u32 = 5;
    pub const SIGABRT: u32 = 6;
    pub const SIGBUS: u32 = 7;
    pub const SIGFPE: u32 = 8;
    pub const SIGKILL: u32 = 9;
    pub const SIGUSR1: u32 = 10;
    pub const SIGSEGV: u32 = 11;
    pub const SIGUSR2: u32 = 12;
    pub const SIGPIPE: u32 = 13;
    pub const SIGALRM: u32 = 14;
    pub const SIGTERM: u32 = 15;
    pub const SIGSTKFLT: u32 = 16;
    pub const SIGCHLD: u32 = 17;
    pub const SIGCONT: u32 = 18;
    pub const SIGSTOP: u32 = 19;
    pub const SIGTSTP: u32 = 20;
    pub const SIGTTIN: u32 = 21;
    pub const SIGTTOU: u32 = 22;
    pub const SIGURG: u32 = 23;
    pub const SIGXCPU: u32 = 24;
    pub const SIGXFSZ: u32 = 25;
    pub const SIGVTALRM: u32 = 26;
    pub const SIGPROF: u32 = 27;
    pub const SIGWINCH: u32 = 28;
    pub const SIGIO: u32 = 29;
    pub const SIGPWR: u32 = 30;
    pub const SIGSYS: u32 = 31;
    /// The first real-time signal (`SIGRTMIN`); `SIGRTMAX` is [`NSIG`].
    pub const SIGRTMIN: u32 = 32;

    pub const SIG_BLOCK: u32 = 0;
    pub const SIG_UNBLOCK: u32 = 1;
    pub const SIG_SETMASK: u32 = 2;
    pub const SIG_DFL: u64 = 0;
    pub const SIG_IGN: u64 = 1;

    pub const SA_NOCLDSTOP: u64 = 0x0000_0001;
    pub const SA_NOCLDWAIT: u64 = 0x0000_0002;
    pub const SA_SIGINFO: u64 = 0x0000_0004;
    /// Never supported: lets programs detect that unknown flags are
    /// cleared on read-back (sigaction(2), "Dynamically probing for flag
    /// bit support").
    pub const SA_UNSUPPORTED: u64 = 0x0000_0400;
    pub const SA_EXPOSE_TAGBITS: u64 = 0x0000_0800;
    pub const SA_RESTORER: u64 = 0x0400_0000;
    pub const SA_ONSTACK: u64 = 0x0800_0000;
    pub const SA_RESTART: u64 = 0x1000_0000;
    pub const SA_NODEFER: u64 = 0x4000_0000;
    pub const SA_RESETHAND: u64 = 0x8000_0000;
    /// Every flag this personality knows on x86-64; the others are
    /// cleared when an action is stored, so `oldact` never reports them.
    pub const SA_KNOWN: u64 = SA_NOCLDSTOP
        | SA_NOCLDWAIT
        | SA_SIGINFO
        | SA_EXPOSE_TAGBITS
        | SA_RESTORER
        | SA_ONSTACK
        | SA_RESTART
        | SA_NODEFER
        | SA_RESETHAND;

    /// `stack_t.ss_flags` (x86 `asm/signal.h` via `linux/signal.h`).
    pub const SS_ONSTACK: u32 = 1;
    pub const SS_DISABLE: u32 = 2;
    pub const SS_AUTODISARM: u32 = 1 << 31;
    /// The smallest alternate signal stack `sigaltstack` accepts.
    pub const MINSIGSTKSZ: u64 = 2048;
    pub const SIGSTKSZ: u64 = 8192;

    /// `si_code` values (`asm-generic/siginfo.h`).
    pub const SI_USER: i32 = 0;
    pub const SI_KERNEL: i32 = 0x80;
    pub const SI_QUEUE: i32 = -1;
    pub const SI_TKILL: i32 = -6;
    pub const SEGV_MAPERR: i32 = 1;
    pub const SEGV_ACCERR: i32 = 2;

    /// `uc_flags` bits (x86 `asm/ucontext.h`).
    pub const UC_FP_XSTATE: u64 = 0x1;
    pub const UC_SIGCONTEXT_SS: u64 = 0x2;
    pub const UC_STRICT_RESTORE_SS: u64 = 0x4;

    /// Markers of an extended (XSAVE) FPU frame (x86 `asm/sigcontext.h`).
    pub const FP_XSTATE_MAGIC1: u32 = 0x4650_5853;
    pub const FP_XSTATE_MAGIC2: u32 = 0x4650_5845;
}

/// futex operations and robust list constants (`linux/futex.h`).
pub mod futex {
    pub const FUTEX_WAIT: u32 = 0;
    pub const FUTEX_WAKE: u32 = 1;
    pub const FUTEX_FD: u32 = 2;
    pub const FUTEX_REQUEUE: u32 = 3;
    pub const FUTEX_CMP_REQUEUE: u32 = 4;
    pub const FUTEX_WAKE_OP: u32 = 5;
    pub const FUTEX_LOCK_PI: u32 = 6;
    pub const FUTEX_UNLOCK_PI: u32 = 7;
    pub const FUTEX_TRYLOCK_PI: u32 = 8;
    pub const FUTEX_WAIT_BITSET: u32 = 9;
    pub const FUTEX_WAKE_BITSET: u32 = 10;
    pub const FUTEX_WAIT_REQUEUE_PI: u32 = 11;
    pub const FUTEX_CMP_REQUEUE_PI: u32 = 12;
    pub const FUTEX_LOCK_PI2: u32 = 13;
    pub const FUTEX_PRIVATE_FLAG: u32 = 128;
    pub const FUTEX_CLOCK_REALTIME: u32 = 256;
    pub const FUTEX_CMD_MASK: u32 = !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);

    pub const FUTEX_WAITERS: u32 = 0x8000_0000;
    pub const FUTEX_OWNER_DIED: u32 = 0x4000_0000;
    pub const FUTEX_TID_MASK: u32 = 0x3fff_ffff;
    /// The most entries the exit-time robust list walk visits.
    pub const ROBUST_LIST_LIMIT: usize = 2048;
    /// `sizeof(struct robust_list_head)` on x86-64: `next`, `futex_offset`,
    /// `list_op_pending`, 8 bytes each.
    pub const ROBUST_LIST_HEAD_SIZE: u64 = 24;
    pub const FUTEX_BITSET_MATCH_ANY: u32 = 0xffff_ffff;

    pub const FUTEX_OP_SET: u32 = 0;
    pub const FUTEX_OP_ADD: u32 = 1;
    pub const FUTEX_OP_OR: u32 = 2;
    pub const FUTEX_OP_ANDN: u32 = 3;
    pub const FUTEX_OP_XOR: u32 = 4;
    /// In the op field: use `1 << oparg` as the operand.
    pub const FUTEX_OP_OPARG_SHIFT: u32 = 8;

    pub const FUTEX_OP_CMP_EQ: u32 = 0;
    pub const FUTEX_OP_CMP_NE: u32 = 1;
    pub const FUTEX_OP_CMP_LT: u32 = 2;
    pub const FUTEX_OP_CMP_LE: u32 = 3;
    pub const FUTEX_OP_CMP_GT: u32 = 4;
    pub const FUTEX_OP_CMP_GE: u32 = 5;
}

/// File type bits of `st_mode` (`linux/stat.h`).
pub mod mode {
    pub const S_IFMT: u32 = 0o170000;
    pub const S_IFSOCK: u32 = 0o140000;
    pub const S_IFLNK: u32 = 0o120000;
    pub const S_IFREG: u32 = 0o100000;
    pub const S_IFBLK: u32 = 0o060000;
    pub const S_IFDIR: u32 = 0o040000;
    pub const S_IFCHR: u32 = 0o020000;
    pub const S_IFIFO: u32 = 0o010000;
}

/// `UIO_MAXIOV` (`linux/uio.h`): the most iovecs one `readv`/`writev` takes.
pub const IOV_MAX: u32 = 1024;

/// `struct __kernel_timespec` (`linux/time_types.h`): seconds and
/// nanoseconds, both 64-bit signed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timespec {
    pub sec: i64,
    pub nsec: i64,
}

impl Timespec {
    pub const SIZE: usize = 16;

    /// A timespec from nanoseconds.
    pub const fn from_nanos(ns: u64) -> Timespec {
        Timespec { sec: (ns / 1_000_000_000) as i64, nsec: (ns % 1_000_000_000) as i64 }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..8].copy_from_slice(&self.sec.to_le_bytes());
        b[8..16].copy_from_slice(&self.nsec.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Timespec {
        Timespec { sec: le_u64(&b[0..8]) as i64, nsec: le_u64(&b[8..16]) as i64 }
    }

    /// A valid timeout or time: `sec` not negative and `nsec` in
    /// `0..1_000_000_000` (the `EINVAL` rule of futex(2), sigtimedwait(2)
    /// and nanosleep(2)).
    pub const fn is_valid(self) -> bool {
        self.sec >= 0 && self.nsec >= 0 && self.nsec < 1_000_000_000
    }

    pub const fn is_zero(self) -> bool {
        self.sec == 0 && self.nsec == 0
    }

    /// `self + other` for valid values, saturating at the largest time
    /// (a deadline that far away never passes).
    pub const fn saturating_add(self, other: Timespec) -> Timespec {
        let mut sec = self.sec.saturating_add(other.sec);
        let mut nsec = self.nsec + other.nsec;
        if nsec >= 1_000_000_000 {
            nsec -= 1_000_000_000;
            sec = sec.saturating_add(1);
        }
        if sec == i64::MAX {
            nsec = 999_999_999;
        }
        Timespec { sec, nsec }
    }
}

/// `siginfo_t` (`asm-generic/siginfo.h`, x86-64): `si_signo`, `si_errno`,
/// `si_code` (4 bytes each), 4 bytes of padding (the union below holds
/// pointers), then the 112-byte `_sifields` union at offset 16; 128 bytes
/// in all (`SI_MAX_SIZE`). The union is kept as raw bytes; accessors read
/// the members this personality fills in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SigInfo {
    pub signo: i32,
    pub errno: i32,
    pub code: i32,
    pub fields: [u8; SigInfo::FIELDS_SIZE],
}

impl SigInfo {
    pub const SIZE: usize = 128;
    pub const FIELDS_OFFSET: usize = 16;
    pub const FIELDS_SIZE: usize = Self::SIZE - Self::FIELDS_OFFSET;

    /// A siginfo with only the signal and code set.
    pub const fn new(signo: u32, code: i32) -> SigInfo {
        SigInfo { signo: signo as i32, errno: 0, code, fields: [0; Self::FIELDS_SIZE] }
    }

    /// Sent by a process (`_kill` / `_rt`: `si_pid` at offset 16, `si_uid`
    /// at 20): kill(2) (`SI_USER`), tkill/tgkill (`SI_TKILL`).
    pub fn from_process(signo: u32, code: i32, pid: i32, uid: u32) -> SigInfo {
        let mut info = SigInfo::new(signo, code);
        info.fields[0..4].copy_from_slice(&pid.to_le_bytes());
        info.fields[4..8].copy_from_slice(&uid.to_le_bytes());
        info
    }

    /// A fault (`_sigfault`: `si_addr` at offset 16).
    pub fn fault(signo: u32, code: i32, addr: u64) -> SigInfo {
        let mut info = SigInfo::new(signo, code);
        info.fields[0..8].copy_from_slice(&addr.to_le_bytes());
        info
    }

    /// The signal number as stored (1-64 for queued signals).
    pub const fn signal(&self) -> u32 {
        self.signo as u32
    }

    /// `si_pid` (for `_kill`, `_rt`, `_sigchld`).
    pub fn pid(&self) -> i32 {
        i32::from_le_bytes([self.fields[0], self.fields[1], self.fields[2], self.fields[3]])
    }

    /// `si_uid`.
    pub fn uid(&self) -> u32 {
        u32::from_le_bytes([self.fields[4], self.fields[5], self.fields[6], self.fields[7]])
    }

    /// `si_value` (`_rt._sigval`, offset 24).
    pub fn value(&self) -> u64 {
        le_u64(&self.fields[8..16])
    }

    /// `si_addr` (`_sigfault`, offset 16).
    pub fn addr(&self) -> u64 {
        le_u64(&self.fields[0..8])
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..4].copy_from_slice(&self.signo.to_le_bytes());
        b[4..8].copy_from_slice(&self.errno.to_le_bytes());
        b[8..12].copy_from_slice(&self.code.to_le_bytes());
        b[Self::FIELDS_OFFSET..].copy_from_slice(&self.fields);
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> SigInfo {
        let int = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let mut fields = [0; Self::FIELDS_SIZE];
        fields.copy_from_slice(&b[Self::FIELDS_OFFSET..]);
        SigInfo { signo: int(0), errno: int(4), code: int(8), fields }
    }
}

/// `stack_t` (x86 `asm/signal.h`): `void *ss_sp; int ss_flags;
/// __kernel_size_t ss_size;`, so `ss_sp` at 0, `ss_flags` at 8, four
/// bytes of padding, `ss_size` at 16; 24 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackT {
    pub sp: u64,
    pub flags: u32,
    pub size: u64,
}

impl StackT {
    pub const SIZE: usize = 24;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> StackT {
        StackT { sp: le_u64(&b[0..8]), flags: u32::from_le_bytes([b[8], b[9], b[10], b[11]]), size: le_u64(&b[16..24]) }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..8].copy_from_slice(&self.sp.to_le_bytes());
        b[8..12].copy_from_slice(&self.flags.to_le_bytes());
        b[16..24].copy_from_slice(&self.size.to_le_bytes());
        b
    }
}

/// `struct iovec` (`linux/uio.h`): `void *iov_base; __kernel_size_t iov_len;`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Iovec {
    pub base: u64,
    pub len: u64,
}

impl Iovec {
    pub const SIZE: usize = 16;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Iovec {
        Iovec { base: le_u64(&b[0..8]), len: le_u64(&b[8..16]) }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..8].copy_from_slice(&self.base.to_le_bytes());
        b[8..16].copy_from_slice(&self.len.to_le_bytes());
        b
    }
}

/// The kernel's `struct sigaction` on x86-64 (x86 `asm/signal.h`, the
/// non-i386 branch): `sa_handler`, `sa_flags`, `sa_restorer`, `sa_mask`,
/// 8 bytes each. This is the layout `rt_sigaction` reads and writes, not
/// the C library's `struct sigaction`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: u64,
}

impl SigAction {
    pub const SIZE: usize = 32;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> SigAction {
        SigAction {
            handler: le_u64(&b[0..8]),
            flags: le_u64(&b[8..16]),
            restorer: le_u64(&b[16..24]),
            mask: le_u64(&b[24..32]),
        }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..8].copy_from_slice(&self.handler.to_le_bytes());
        b[8..16].copy_from_slice(&self.flags.to_le_bytes());
        b[16..24].copy_from_slice(&self.restorer.to_le_bytes());
        b[24..32].copy_from_slice(&self.mask.to_le_bytes());
        b
    }
}

/// What `fstat` reports about a file, filled in by the kernel from the VFS.
/// [`FileStat::to_bytes`] encodes it as the x86-64 `struct stat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileStat {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    /// File type ([`mode`]) and permission bits.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub size: i64,
    pub blksize: i64,
    /// Number of 512-byte blocks allocated.
    pub blocks: i64,
    pub atime: Timespec,
    pub mtime: Timespec,
    pub ctime: Timespec,
}

impl FileStat {
    /// `sizeof(struct stat)` on x86-64.
    pub const SIZE: usize = 144;

    /// Encode as the x86-64 `struct stat` (x86 `asm/stat.h`, the non-i386
    /// branch): `st_dev, st_ino, st_nlink` (8 bytes each), `st_mode, st_uid,
    /// st_gid, __pad0` (4 bytes each), `st_rdev, st_size, st_blksize,
    /// st_blocks`, then seconds and nanoseconds of `st_atime, st_mtime,
    /// st_ctime` (8 bytes each), and `__unused[3]`. Padding is zero.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        let mut w = Writer { buf: &mut b, at: 0 };
        w.u64(self.dev);
        w.u64(self.ino);
        w.u64(self.nlink);
        w.u32(self.mode);
        w.u32(self.uid);
        w.u32(self.gid);
        w.u32(0);
        w.u64(self.rdev);
        w.u64(self.size as u64);
        w.u64(self.blksize as u64);
        w.u64(self.blocks as u64);
        for t in [self.atime, self.mtime, self.ctime] {
            w.u64(t.sec as u64);
            w.u64(t.nsec as u64);
        }
        debug_assert_eq!(w.at, Self::SIZE - 24);
        b
    }

    /// The file type bits of `mode`.
    pub const fn file_type(&self) -> u32 {
        self.mode & mode::S_IFMT
    }
}

/// `__NEW_UTS_LEN + 1` (`linux/utsname.h`): every `struct new_utsname`
/// field is 65 bytes, NUL-terminated.
pub const UTS_FIELD_LEN: usize = 65;

/// Encode `struct new_utsname` (`linux/utsname.h`): `sysname, nodename,
/// release, version, machine, domainname`, 65 bytes each. Longer strings are
/// cut to 64 bytes so each field stays NUL-terminated.
pub fn utsname_bytes(fields: [&[u8]; 6]) -> [u8; 6 * UTS_FIELD_LEN] {
    let mut b = [0; 6 * UTS_FIELD_LEN];
    for (i, field) in fields.iter().enumerate() {
        let n = field.len().min(UTS_FIELD_LEN - 1);
        b[i * UTS_FIELD_LEN..i * UTS_FIELD_LEN + n].copy_from_slice(&field[..n]);
    }
    b
}

struct Writer<'a> {
    buf: &'a mut [u8],
    at: usize,
}

impl Writer<'_> {
    fn u64(&mut self, v: u64) {
        self.buf[self.at..self.at + 8].copy_from_slice(&v.to_le_bytes());
        self.at += 8;
    }

    fn u32(&mut self, v: u32) {
        self.buf[self.at..self.at + 4].copy_from_slice(&v.to_le_bytes());
        self.at += 4;
    }
}

/// Read a little-endian `u64` from the first 8 bytes of `b`.
pub fn le_u64(b: &[u8]) -> u64 {
    let mut a = [0; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_rounding() {
        assert_eq!(page_round_up(0), Some(0));
        assert_eq!(page_round_up(1), Some(4096));
        assert_eq!(page_round_up(4096), Some(4096));
        assert_eq!(page_round_up(16048), Some(16384));
        assert_eq!(page_round_up(u64::MAX), None);
        assert_eq!(page_round_up(u64::MAX - 4095), Some(u64::MAX - 4095));
        assert!(page_aligned(0x7000) && !page_aligned(0x7001));
    }

    #[test]
    fn stat_layout_matches_the_uapi_header() {
        let st = FileStat {
            dev: 1,
            ino: 2,
            nlink: 3,
            mode: 0o20666,
            uid: 4,
            gid: 5,
            rdev: 6,
            size: 7,
            blksize: 8,
            blocks: 9,
            atime: Timespec { sec: 10, nsec: 11 },
            mtime: Timespec { sec: 12, nsec: 13 },
            ctime: Timespec { sec: 14, nsec: 15 },
        };
        let b = st.to_bytes();
        let at = |o: usize| le_u64(&b[o..]);
        let at32 = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!((at(0), at(8), at(16)), (1, 2, 3));
        assert_eq!((at32(24), at32(28), at32(32), at32(36)), (0o20666, 4, 5, 0));
        assert_eq!((at(40), at(48), at(56), at(64)), (6, 7, 8, 9));
        assert_eq!((at(72), at(80), at(88), at(96), at(104), at(112)), (10, 11, 12, 13, 14, 15));
        assert!(b[120..].iter().all(|&x| x == 0));
        assert_eq!(st.file_type(), mode::S_IFCHR);
    }

    #[test]
    fn utsname_fields_are_65_bytes_and_terminated() {
        let long = [b'x'; 100];
        let b = utsname_bytes([b"Linux", b"host", &long, b"#1", b"x86_64", b""]);
        assert_eq!(b.len(), 390);
        assert_eq!(&b[0..6], b"Linux\0");
        assert_eq!(&b[65..70], b"host\0");
        assert!(b[130..194].iter().all(|&x| x == b'x'));
        assert_eq!(b[194], 0);
        assert_eq!(&b[260..267], b"x86_64\0");
    }

    #[test]
    fn struct_round_trips() {
        let sa = SigAction { handler: 1, flags: 2, restorer: 3, mask: 4 };
        assert_eq!(SigAction::from_bytes(&sa.to_bytes()), sa);
        let iov = Iovec { base: 0x1000, len: 14 };
        assert_eq!(Iovec::from_bytes(&iov.to_bytes()), iov);
        assert_eq!(Timespec::from_nanos(3_000_000_007), Timespec { sec: 3, nsec: 7 });
        assert_eq!(&Timespec { sec: -1, nsec: 2 }.to_bytes()[0..8], &[0xff; 8]);
        let ts = Timespec { sec: 5, nsec: 6 };
        assert_eq!(Timespec::from_bytes(&ts.to_bytes()), ts);
        let ss = StackT { sp: 0x7000, flags: signal::SS_AUTODISARM, size: 8192 };
        assert_eq!(StackT::from_bytes(&ss.to_bytes()), ss);
    }

    #[test]
    fn timespec_rules() {
        assert!(Timespec { sec: 0, nsec: 999_999_999 }.is_valid());
        assert!(!Timespec { sec: 0, nsec: 1_000_000_000 }.is_valid());
        assert!(!Timespec { sec: -1, nsec: 0 }.is_valid());
        assert!(!Timespec { sec: 0, nsec: -1 }.is_valid());
        let a = Timespec { sec: 1, nsec: 600_000_000 };
        assert_eq!(a.saturating_add(a), Timespec { sec: 3, nsec: 200_000_000 });
        let far = Timespec { sec: i64::MAX, nsec: 0 }.saturating_add(a);
        assert_eq!(far, Timespec { sec: i64::MAX, nsec: 999_999_999 });
    }

    #[test]
    fn siginfo_layout_matches_the_uapi_header() {
        let info = SigInfo::from_process(signal::SIGUSR1, signal::SI_TKILL, 1234, 1000);
        let b = info.to_bytes();
        assert_eq!(b.len(), 128);
        assert_eq!(&b[0..4], &10i32.to_le_bytes(), "si_signo");
        assert_eq!(&b[4..8], &[0; 4], "si_errno");
        assert_eq!(&b[8..12], &(-6i32).to_le_bytes(), "si_code");
        assert_eq!(&b[12..16], &[0; 4], "padding before the union");
        assert_eq!(&b[16..20], &1234i32.to_le_bytes(), "si_pid");
        assert_eq!(&b[20..24], &1000u32.to_le_bytes(), "si_uid");
        assert_eq!((info.pid(), info.uid()), (1234, 1000));
        assert_eq!(SigInfo::from_bytes(&b), info);
        let fault = SigInfo::fault(signal::SIGSEGV, signal::SEGV_MAPERR, 0xdead_beef);
        assert_eq!(le_u64(&fault.to_bytes()[16..24]), 0xdead_beef, "si_addr");
        assert_eq!(fault.addr(), 0xdead_beef);
    }

    #[test]
    fn stack_t_layout_matches_the_uapi_header() {
        let b = StackT { sp: 0x1122, flags: signal::SS_DISABLE, size: 0x3344 }.to_bytes();
        assert_eq!(le_u64(&b[0..8]), 0x1122);
        assert_eq!(&b[8..16], &[2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(le_u64(&b[16..24]), 0x3344);
    }
}
