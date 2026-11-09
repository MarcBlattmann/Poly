//! Native AIO (io_setup(2) and friends) against a
//! [`LinuxFs`](crate::fs::LinuxFs) over a real `poly-vfs` (the fixture of
//! `files.rs`). One test only: the contexts are a global table and the
//! fake's address spaces are not unique between tests.

extern crate std;

use super::fake::*;
use super::files::{env, s};
use crate::aio::{AIO_RING_MAGIC, cmd};
use crate::{Errno, nr};

const CTX: u64 = 0x100;
const IOCB: u64 = 0x200;
const ARRAY: u64 = 0x300;
const EVENTS: u64 = 0x400;
const BUF: u64 = 0x600;
const TS: u64 = 0x700;

/// An iocb at `scratch(IOCB)` for `op` on `fd`.
fn iocb(k: &mut FakeKernel, op: u16, fd: u32, buf: u64, n: u64, off: i64, data: u64) {
    let mut b = [0u8; 64];
    b[..8].copy_from_slice(&data.to_le_bytes());
    b[16..18].copy_from_slice(&op.to_le_bytes());
    b[20..24].copy_from_slice(&fd.to_le_bytes());
    b[24..32].copy_from_slice(&buf.to_le_bytes());
    b[32..40].copy_from_slice(&n.to_le_bytes());
    b[40..48].copy_from_slice(&off.to_le_bytes());
    k.poke(scratch(IOCB), &b);
    k.poke(scratch(ARRAY), &scratch(IOCB).to_le_bytes());
}

#[test]
fn contexts_rings_and_synchronous_reads() {
    let mut e = env();
    let k = &mut e.k;
    let zero = [0u8; 16];
    k.poke(scratch(TS), &zero);

    // io_setup's checks, in order.
    assert_eq!(sys(k, nr::IO_SETUP, &[1, UNMAPPED]), Err(Errno::EFAULT));
    k.poke(scratch(CTX), &5u64.to_le_bytes());
    assert_eq!(sys(k, nr::IO_SETUP, &[1, scratch(CTX)]), Err(Errno::EINVAL));
    assert_eq!(k.peek_u64(scratch(CTX)), 5, "left alone");
    k.poke(scratch(CTX), &0u64.to_le_bytes());
    assert_eq!(sys(k, nr::IO_SETUP, &[0, scratch(CTX)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::IO_SETUP, &[0xffff_ffff, scratch(CTX)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::IO_SETUP, &[70000, scratch(CTX)]), Err(Errno::EAGAIN));
    assert_eq!(sys(k, nr::IO_SETUP, &[1 | 1 << 32, scratch(CTX)]), Ok(0));
    let ctx = k.peek_u64(scratch(CTX));
    assert!(ctx != 0 && ctx.is_multiple_of(4096));
    assert_eq!(k.peek_u32(ctx + 16), AIO_RING_MAGIC);
    assert_eq!(k.peek_u32(ctx + 4), 383, "three pages of slots");

    // A read of /dir/file completes inside io_submit.
    let path = s(k, 0x800, b"/dir/file");
    let fd = sys(k, nr::OPENAT, &[super::files::AT, path, 2, 0]).unwrap() as u32;
    iocb(k, cmd::PREAD, fd, scratch(BUF), 4, 2, 42);
    assert_eq!(sys(k, nr::IO_SUBMIT, &[ctx, 0, scratch(ARRAY)]), Ok(0));
    assert_eq!(sys(k, nr::IO_SUBMIT, &[ctx, u64::MAX, scratch(ARRAY)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::IO_SUBMIT, &[12345, 1, scratch(ARRAY)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::IO_SUBMIT, &[ctx, 1, scratch(ARRAY)]), Ok(1));
    assert_eq!(k.peek_u32(ctx + 12), 1, "tail");
    assert_eq!(k.peek(scratch(BUF), 4), b"nten");
    // Not pending: it completed already.
    assert_eq!(sys(k, nr::IO_CANCEL, &[ctx, scratch(IOCB), 0]), Err(Errno::EINVAL));
    // Unknown opcodes queue nothing.
    iocb(k, cmd::NOOP, fd, 0, 0, 0, 1);
    assert_eq!(sys(k, nr::IO_SUBMIT, &[ctx, 1, scratch(ARRAY)]), Err(Errno::EINVAL));

    assert_eq!(sys(k, nr::IO_GETEVENTS, &[ctx, 2, 1, scratch(EVENTS), scratch(TS)]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::IO_GETEVENTS, &[ctx, 0, 4, UNMAPPED, scratch(TS)]), Err(Errno::EFAULT));
    assert_eq!(sys(k, nr::IO_GETEVENTS, &[ctx, 0, 4, scratch(EVENTS), scratch(TS)]), Ok(1));
    assert_eq!(k.peek_u64(scratch(EVENTS)), 42);
    assert_eq!(k.peek_u64(scratch(EVENTS) + 8), scratch(IOCB));
    assert_eq!(k.peek_u64(scratch(EVENTS) + 16), 4);
    assert_eq!(k.peek_u32(ctx + 8), 1, "head");
    assert_eq!(sys(k, nr::IO_GETEVENTS, &[ctx, 0, 4, scratch(EVENTS), scratch(TS)]), Ok(0));

    // Unmapping the ring ends the context.
    assert_eq!(sys(k, nr::MUNMAP, &[ctx, 4096]), Ok(0));
    assert_eq!(sys(k, nr::IO_DESTROY, &[ctx]), Err(Errno::EINVAL));
    k.poke(scratch(CTX), &0u64.to_le_bytes());
    assert_eq!(sys(k, nr::IO_SETUP, &[4, scratch(CTX)]), Ok(0));
    let ctx = k.peek_u64(scratch(CTX));
    assert_eq!(sys(k, nr::IO_DESTROY, &[ctx]), Ok(0));
    assert_eq!(sys(k, nr::IO_DESTROY, &[ctx]), Err(Errno::EINVAL));
    assert_eq!(sys(k, nr::IO_DESTROY, &[0]), Err(Errno::EINVAL));
}
