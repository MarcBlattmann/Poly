//! The io_uring user ABI (UAPI `linux/io_uring.h`): setup flags and
//! parameters, the ring layout, SQEs and CQEs, and the opcodes.

use crate::errno::Errno;

const fn errno(value: u16) -> Errno {
    match Errno::new(value) {
        Some(e) => e,
        None => panic!("not an error number"),
    }
}

/// Error numbers the requests report that the personality's common set
/// does not name (`asm-generic/errno.h`).
pub mod err {
    use super::errno;
    use crate::errno::Errno;
    /// `ETIME`: a timeout passed.
    pub const ETIME: Errno = errno(62);
    /// `ENOTSOCK`.
    pub const ENOTSOCK: Errno = errno(88);
    /// `EALREADY`: a request that is running cannot be cancelled.
    pub const EALREADY: Errno = errno(114);
    /// `ECANCELED`: the request was cancelled.
    pub const ECANCELED: Errno = errno(125);
}

/// `IORING_SETUP_*` (io_uring_setup(2)).
pub mod setup {
    pub const IOPOLL: u32 = 1 << 0;
    pub const SQPOLL: u32 = 1 << 1;
    pub const SQ_AFF: u32 = 1 << 2;
    pub const CQSIZE: u32 = 1 << 3;
    pub const CLAMP: u32 = 1 << 4;
    pub const ATTACH_WQ: u32 = 1 << 5;
    pub const R_DISABLED: u32 = 1 << 6;
    pub const SUBMIT_ALL: u32 = 1 << 7;
    pub const COOP_TASKRUN: u32 = 1 << 8;
    pub const TASKRUN_FLAG: u32 = 1 << 9;
    pub const SQE128: u32 = 1 << 10;
    pub const CQE32: u32 = 1 << 11;
    pub const SINGLE_ISSUER: u32 = 1 << 12;
    pub const DEFER_TASKRUN: u32 = 1 << 13;
    pub const NO_MMAP: u32 = 1 << 14;
    pub const REGISTERED_FD_ONLY: u32 = 1 << 15;
    pub const NO_SQARRAY: u32 = 1 << 16;
    pub const HYBRID_IOPOLL: u32 = 1 << 17;
    pub const CQE_MIXED: u32 = 1 << 18;
    /// Every flag the call accepts (bits 0 to 18; bit 20 and bit 31 are
    /// `EINVAL`, io_uring_setup's spec sheet).
    pub const KNOWN: u32 = (1 << 19) - 1;
}

/// `IORING_FEAT_*`: what this io_uring promises (bits 0 to 17, the set the
/// spec sheet reports, `0x3ffff`).
pub mod feat {
    pub const SINGLE_MMAP: u32 = 1 << 0;
    pub const NODROP: u32 = 1 << 1;
    pub const SUBMIT_STABLE: u32 = 1 << 2;
    pub const RW_CUR_POS: u32 = 1 << 3;
    pub const EXT_ARG: u32 = 1 << 8;
    /// Every feature bit reported.
    pub const ALL: u32 = 0x3ffff;
}

/// `IORING_ENTER_*` (io_uring_enter(2)).
pub mod enter {
    pub const GETEVENTS: u32 = 1 << 0;
    pub const SQ_WAKEUP: u32 = 1 << 1;
    pub const SQ_WAIT: u32 = 1 << 2;
    pub const EXT_ARG: u32 = 1 << 3;
    pub const REGISTERED_RING: u32 = 1 << 4;
    pub const ABS_TIMER: u32 = 1 << 5;
    pub const EXT_ARG_REG: u32 = 1 << 6;
    pub const NO_IOWAIT: u32 = 1 << 7;
    /// Every flag the call accepts (0x100 and bit 31 are `EINVAL`).
    pub const KNOWN: u32 = (1 << 8) - 1;
}

/// SQ ring flags (`IORING_SQ_*`).
pub mod sq_flags {
    pub const NEED_WAKEUP: u32 = 1 << 0;
    pub const CQ_OVERFLOW: u32 = 1 << 1;
}

/// SQE flags (`IOSQE_*`).
pub mod sqe_flags {
    pub const FIXED_FILE: u8 = 1 << 0;
    pub const IO_DRAIN: u8 = 1 << 1;
    pub const IO_LINK: u8 = 1 << 2;
    pub const IO_HARDLINK: u8 = 1 << 3;
    pub const ASYNC: u8 = 1 << 4;
    pub const BUFFER_SELECT: u8 = 1 << 5;
    pub const CQE_SKIP_SUCCESS: u8 = 1 << 6;
    /// Bits 0 to 6; bit 7 is `EINVAL` in the CQE.
    pub const KNOWN: u8 = 0x7f;
}

/// CQE flags (`IORING_CQE_F_*`).
pub mod cqe_flags {
    pub const MORE: u32 = 1 << 1;
}

/// Request opcodes (`enum io_uring_op`).
pub mod op {
    pub const NOP: u8 = 0;
    pub const READV: u8 = 1;
    pub const WRITEV: u8 = 2;
    pub const FSYNC: u8 = 3;
    pub const READ_FIXED: u8 = 4;
    pub const WRITE_FIXED: u8 = 5;
    pub const POLL_ADD: u8 = 6;
    pub const POLL_REMOVE: u8 = 7;
    pub const SENDMSG: u8 = 9;
    pub const RECVMSG: u8 = 10;
    pub const TIMEOUT: u8 = 11;
    pub const TIMEOUT_REMOVE: u8 = 12;
    pub const ACCEPT: u8 = 13;
    pub const ASYNC_CANCEL: u8 = 14;
    pub const CONNECT: u8 = 16;
    pub const FALLOCATE: u8 = 17;
    pub const OPENAT: u8 = 18;
    pub const CLOSE: u8 = 19;
    pub const STATX: u8 = 21;
    pub const READ: u8 = 22;
    pub const WRITE: u8 = 23;
    pub const FADVISE: u8 = 24;
    pub const MADVISE: u8 = 25;
    pub const SEND: u8 = 26;
    pub const RECV: u8 = 27;
    pub const OPENAT2: u8 = 28;
    pub const SHUTDOWN: u8 = 34;
    pub const RENAMEAT: u8 = 35;
    pub const UNLINKAT: u8 = 36;
    pub const MKDIRAT: u8 = 37;
    pub const SYMLINKAT: u8 = 38;
    pub const LINKAT: u8 = 39;
    pub const SOCKET: u8 = 45;
    /// The highest opcode the header names (`IORING_OP_LAST` - 1), which
    /// `IORING_REGISTER_PROBE` reports as `last_op`.
    pub const LAST: u8 = 62;

    /// The opcodes this io_uring runs.
    pub const SUPPORTED: &[u8] = &[
        NOP,
        READV,
        WRITEV,
        FSYNC,
        READ_FIXED,
        WRITE_FIXED,
        POLL_ADD,
        POLL_REMOVE,
        SENDMSG,
        RECVMSG,
        TIMEOUT,
        TIMEOUT_REMOVE,
        ACCEPT,
        ASYNC_CANCEL,
        CONNECT,
        FALLOCATE,
        OPENAT,
        CLOSE,
        STATX,
        READ,
        WRITE,
        FADVISE,
        MADVISE,
        SEND,
        RECV,
        OPENAT2,
        SHUTDOWN,
        RENAMEAT,
        UNLINKAT,
        MKDIRAT,
        SYMLINKAT,
        LINKAT,
        SOCKET,
    ];

    pub fn supported(op: u8) -> bool {
        SUPPORTED.contains(&op)
    }
}

/// `IORING_TIMEOUT_*` (the op flags of `IORING_OP_TIMEOUT` and
/// `IORING_OP_TIMEOUT_REMOVE`).
pub mod timeout {
    pub const ABS: u32 = 1 << 0;
    pub const UPDATE: u32 = 1 << 1;
    pub const BOOTTIME: u32 = 1 << 2;
    pub const REALTIME: u32 = 1 << 3;
    pub const LINK_TIMEOUT_UPDATE: u32 = 1 << 4;
    pub const ETIME_SUCCESS: u32 = 1 << 5;
    pub const MULTISHOT: u32 = 1 << 6;
    /// Every flag of a timeout request.
    pub const KNOWN: u32 = ABS | BOOTTIME | REALTIME | ETIME_SUCCESS | MULTISHOT;
}

/// `IORING_FSYNC_DATASYNC`.
pub const FSYNC_DATASYNC: u32 = 1 << 0;
/// `IORING_POLL_ADD_MULTI` (in the SQE's `len` of a poll request).
pub const POLL_ADD_MULTI: u32 = 1 << 0;

/// `io_uring_register(2)` opcodes (`IORING_REGISTER_*`).
pub mod reg {
    pub const REGISTER_BUFFERS: u32 = 0;
    pub const UNREGISTER_BUFFERS: u32 = 1;
    pub const REGISTER_FILES: u32 = 2;
    pub const UNREGISTER_FILES: u32 = 3;
    pub const REGISTER_EVENTFD: u32 = 4;
    pub const UNREGISTER_EVENTFD: u32 = 5;
    pub const REGISTER_FILES_UPDATE: u32 = 6;
    pub const REGISTER_EVENTFD_ASYNC: u32 = 7;
    pub const REGISTER_PROBE: u32 = 8;
    pub const REGISTER_ENABLE_RINGS: u32 = 12;
    /// `IORING_REGISTER_USE_REGISTERED_RING`: a flag in the opcode.
    pub const USE_REGISTERED_RING: u32 = 1 << 31;
    /// The most buffers one registration takes (`UIO_MAXIOV` = 1024 is
    /// readv's limit; registration's is 16384, the spec sheet).
    pub const MAX_BUFFERS: u32 = 16384;
    /// The most probe entries (`IORING_OP_LAST` rounded to 256).
    pub const MAX_PROBE_OPS: u32 = 256;
    /// The most files one registration takes (`IORING_MAX_FIXED_FILES`).
    pub const MAX_FILES: u32 = 1 << 20;
    /// `IO_URING_OP_SUPPORTED` in a probe entry's flags.
    pub const OP_SUPPORTED: u16 = 1;
}

/// Where a ring region starts in the descriptor's mmap offsets
/// (`IORING_OFF_*`), compared after masking with [`OFF_MASK`].
pub const OFF_SQ_RING: u64 = 0;
pub const OFF_CQ_RING: u64 = 0x800_0000;
pub const OFF_SQES: u64 = 0x1000_0000;
pub const OFF_MASK: u64 = 0xf800_0000;

/// The most SQ entries (`IORING_MAX_ENTRIES`) and CQ entries
/// (`IORING_MAX_CQ_ENTRIES`).
pub const MAX_ENTRIES: u32 = 32768;
pub const MAX_CQ_ENTRIES: u32 = 2 * MAX_ENTRIES;

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    let mut x = [0; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// `struct io_uring_params` (120 bytes): what the caller passes in and
/// gets back. The offsets structures are written, not read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Params {
    pub sq_entries: u32,
    pub cq_entries: u32,
    pub flags: u32,
    pub sq_thread_cpu: u32,
    pub sq_thread_idle: u32,
    pub features: u32,
    pub wq_fd: u32,
    pub resv: [u32; 3],
}

impl Params {
    pub const SIZE: usize = 120;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Params {
        Params {
            sq_entries: le32(b, 0),
            cq_entries: le32(b, 4),
            flags: le32(b, 8),
            sq_thread_cpu: le32(b, 12),
            sq_thread_idle: le32(b, 16),
            features: le32(b, 20),
            wq_fd: le32(b, 24),
            resv: [le32(b, 28), le32(b, 32), le32(b, 36)],
        }
    }

    /// The structure as the call returns it: `orig` with the entry counts,
    /// the features and both offset structures filled in.
    pub fn result(orig: &[u8; Self::SIZE], layout: &Layout) -> [u8; Self::SIZE] {
        let mut b = *orig;
        put32(&mut b, 0, layout.sq_entries);
        put32(&mut b, 4, layout.cq_entries);
        put32(&mut b, 20, feat::ALL);
        // struct io_sqring_offsets at 40: head, tail, ring_mask,
        // ring_entries, flags, dropped, array, resv1, u64 user_addr.
        let sq = [
            Layout::SQ_HEAD,
            Layout::SQ_TAIL,
            Layout::SQ_MASK,
            Layout::SQ_ENTRIES,
            Layout::SQ_FLAGS,
            Layout::SQ_DROPPED,
            layout.array.unwrap_or(0),
            0,
        ];
        for (i, v) in sq.iter().enumerate() {
            put32(&mut b, 40 + 4 * i, *v);
        }
        b[72..80].fill(0);
        // struct io_cqring_offsets at 80: head, tail, ring_mask,
        // ring_entries, overflow, cqes, flags, resv1, u64 user_addr.
        let cq = [
            Layout::CQ_HEAD,
            Layout::CQ_TAIL,
            Layout::CQ_MASK,
            Layout::CQ_ENTRIES,
            Layout::CQ_OVERFLOW,
            Layout::CQES,
            Layout::CQ_FLAGS,
            0,
        ];
        for (i, v) in cq.iter().enumerate() {
            put32(&mut b, 80 + 4 * i, *v);
        }
        b[112..120].fill(0);
        b
    }
}

/// Where everything is in a ring's two regions, and how big they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub sq_entries: u32,
    pub cq_entries: u32,
    /// 64, or 128 with `IORING_SETUP_SQE128`.
    pub sqe_size: u32,
    /// 16, or 32 with `IORING_SETUP_CQE32`.
    pub cqe_size: u32,
    /// The SQ index array's offset; `None` with `IORING_SETUP_NO_SQARRAY`
    /// (SQ entry `i` is then SQE `i`).
    pub array: Option<u32>,
}

impl Layout {
    pub const SQ_HEAD: u32 = 0;
    pub const SQ_TAIL: u32 = 4;
    pub const CQ_HEAD: u32 = 8;
    pub const CQ_TAIL: u32 = 12;
    pub const SQ_MASK: u32 = 16;
    pub const CQ_MASK: u32 = 20;
    pub const SQ_ENTRIES: u32 = 24;
    pub const CQ_ENTRIES: u32 = 28;
    pub const SQ_DROPPED: u32 = 32;
    pub const SQ_FLAGS: u32 = 36;
    pub const CQ_FLAGS: u32 = 40;
    pub const CQ_OVERFLOW: u32 = 44;
    pub const CQES: u32 = 64;

    /// The layout io_uring_setup(entries, params) asks for, or its error:
    /// `entries` (0 or above 32768 without `CLAMP` is `EINVAL`) and, with
    /// `CQSIZE`, `cq_entries` (0, below the SQ size, or above 65536
    /// without `CLAMP` is `EINVAL`), both rounded up to powers of two.
    pub fn for_setup(entries: u32, p: &Params) -> Result<Layout, Errno> {
        let clamp = p.flags & setup::CLAMP != 0;
        if entries == 0 || (entries > MAX_ENTRIES && !clamp) {
            return Err(Errno::EINVAL);
        }
        let sq = entries.min(MAX_ENTRIES).next_power_of_two();
        let cq = if p.flags & setup::CQSIZE != 0 {
            if p.cq_entries == 0 || (p.cq_entries > MAX_CQ_ENTRIES && !clamp) {
                return Err(Errno::EINVAL);
            }
            let cq = p.cq_entries.min(MAX_CQ_ENTRIES).next_power_of_two();
            if cq < sq {
                return Err(Errno::EINVAL);
            }
            cq
        } else {
            2 * sq
        };
        let sqe_size = if p.flags & setup::SQE128 != 0 { 128 } else { 64 };
        let cqe_size = if p.flags & setup::CQE32 != 0 { 32 } else { 16 };
        let array = if p.flags & setup::NO_SQARRAY != 0 { None } else { Some(Self::CQES + cq * cqe_size) };
        Ok(Layout { sq_entries: sq, cq_entries: cq, sqe_size, cqe_size, array })
    }

    /// Bytes of the rings region (SQ and CQ ring fields, CQEs, SQ array).
    pub fn rings_len(&self) -> usize {
        let cqes_end = Self::CQES as usize + self.cq_entries as usize * self.cqe_size as usize;
        match self.array {
            Some(a) => a as usize + 4 * self.sq_entries as usize,
            None => cqes_end,
        }
    }

    /// Bytes of the SQE array.
    pub fn sqes_len(&self) -> usize {
        self.sq_entries as usize * self.sqe_size as usize
    }
}

/// `struct io_uring_sqe`: the fields the requests use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sqe {
    pub opcode: u8,
    pub flags: u8,
    pub ioprio: u16,
    pub fd: i32,
    /// `off` / `addr2`.
    pub off: u64,
    /// `addr` / `splice_off_in`.
    pub addr: u64,
    pub len: u32,
    /// The per-opcode flags union (`rw_flags`, `poll32_events`,
    /// `timeout_flags`, `accept_flags`, `open_flags`, `msg_flags`, ...).
    pub op_flags: u32,
    pub user_data: u64,
    pub buf_index: u16,
    pub personality: u16,
    pub file_index: i32,
    pub addr3: u64,
}

impl Sqe {
    pub const SIZE: usize = 64;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Sqe {
        Sqe {
            opcode: b[0],
            flags: b[1],
            ioprio: u16::from_le_bytes([b[2], b[3]]),
            fd: le32(b, 4) as i32,
            off: le64(b, 8),
            addr: le64(b, 16),
            len: le32(b, 24),
            op_flags: le32(b, 28),
            user_data: le64(b, 32),
            buf_index: u16::from_le_bytes([b[40], b[41]]),
            personality: u16::from_le_bytes([b[42], b[43]]),
            file_index: le32(b, 44) as i32,
            addr3: le64(b, 48),
        }
    }

    /// Whether the next SQE belongs to this one's chain.
    pub fn links(&self) -> bool {
        self.flags & (sqe_flags::IO_LINK | sqe_flags::IO_HARDLINK) != 0
    }
}

/// `struct io_uring_cqe`: 16 bytes (32 with `CQE32`, the rest zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cqe {
    pub user_data: u64,
    pub res: i32,
    pub flags: u32,
}

impl Cqe {
    pub fn to_bytes(self) -> [u8; 16] {
        let mut b = [0; 16];
        b[0..8].copy_from_slice(&self.user_data.to_le_bytes());
        b[8..12].copy_from_slice(&self.res.to_le_bytes());
        b[12..16].copy_from_slice(&self.flags.to_le_bytes());
        b
    }

    /// The CQE of a request that ended with `r`.
    pub fn of(user_data: u64, r: Result<u64, Errno>) -> Cqe {
        Cqe { user_data, res: res(r), flags: 0 }
    }
}

/// A result as a CQE's `res`: the value (truncated to 32 bits, as byte
/// counts and descriptors fit) or `-errno`; an interruption is `-EINTR`.
pub fn res(r: Result<u64, Errno>) -> i32 {
    match r {
        Ok(v) => v as i32,
        Err(e) if e.is_restart() => -i32::from(Errno::EINTR.get()),
        Err(e) => -i32::from(e.get()),
    }
}

/// Fill `IORING_REGISTER_PROBE`'s buffer (`struct io_uring_probe`, 16
/// bytes, then `nr` 8-byte `struct io_uring_probe_op`): `last_op`,
/// `ops_len` = min(nr, last_op + 1), and per op its number and
/// `IO_URING_OP_SUPPORTED`. Entries past `ops_len` stay as they are.
pub fn fill_probe(buf: &mut [u8], nr: usize) {
    let len = nr.min(usize::from(op::LAST) + 1);
    buf[0] = op::LAST;
    buf[1] = len as u8;
    for i in 0..len {
        let e = &mut buf[16 + 8 * i..16 + 8 * i + 8];
        e[0] = i as u8;
        let flags = if op::supported(i as u8) { reg::OP_SUPPORTED } else { 0 };
        e[2..4].copy_from_slice(&flags.to_le_bytes());
    }
}
