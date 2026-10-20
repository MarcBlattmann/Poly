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
/// `asm/signal.h`).
pub mod signal {
    /// `_NSIG`: signals are numbered 1 to 64.
    pub const NSIG: u32 = 64;
    /// `sizeof(sigset_t)` as the kernel defines it: `_NSIG / 8` bytes.
    pub const SIGSET_SIZE: u64 = 8;
    pub const SIGBUS: u32 = 7;
    pub const SIGKILL: u32 = 9;
    pub const SIGSEGV: u32 = 11;
    pub const SIGPIPE: u32 = 13;
    pub const SIGSTOP: u32 = 19;
    pub const SIG_BLOCK: u32 = 0;
    pub const SIG_UNBLOCK: u32 = 1;
    pub const SIG_SETMASK: u32 = 2;
    pub const SIG_DFL: u64 = 0;
    pub const SIG_IGN: u64 = 1;
    pub const SA_SIGINFO: u64 = 0x0000_0004;
    pub const SA_RESTORER: u64 = 0x0400_0000;
    pub const SA_ONSTACK: u64 = 0x0800_0000;
    pub const SA_RESTART: u64 = 0x1000_0000;
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
    }
}
