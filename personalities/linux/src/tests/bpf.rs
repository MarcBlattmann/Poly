//! bpf(2) against the fake kernel: the attribute rules, the map commands
//! through descriptors, program loading and who may create what.

extern crate std;

use super::fake::*;
use crate::bpf::{MAP_TYPE_ARRAY, MAP_TYPE_HASH};
use crate::cred::Creds;
use crate::errno::Errno;
use crate::nr;
use std::vec::Vec;

const ATTR: u64 = DATA;
const KEY: u64 = DATA + 0x400;
const VALUE: u64 = DATA + 0x500;
const INFO: u64 = DATA + 0x600;
const CODE: u64 = DATA + 0x800;
const LICENSE: u64 = DATA + 0xa00;
const LOG: u64 = DATA + 0xb00;

fn kernel() -> FakeKernel {
    let mut k = FakeKernel::new();
    *k.creds_mut() = Creds::root();
    k.poke(LICENSE, b"GPL\0");
    k
}

fn bpf(k: &mut FakeKernel, cmd: i64, size: u64) -> Result<u64, Errno> {
    sys(k, nr::BPF, &[cmd as u64, ATTR, size])
}

fn attr(k: &mut FakeKernel, words: &[u32]) {
    k.poke(ATTR, &[0u8; 256]);
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    k.poke(ATTR, &bytes);
}

fn create(k: &mut FakeKernel, kind: u32, key: u32, value: u32, max: u32, flags: u32) -> Result<u64, Errno> {
    attr(k, &[kind, key, value, max, flags]);
    bpf(k, 0, 64)
}

/// The element commands' attribute.
fn elem(k: &mut FakeKernel, fd: u64, key: u64, value: u64, flags: u64) {
    attr(k, &[fd as u32, 0]);
    k.poke(ATTR + 8, &key.to_le_bytes());
    k.poke(ATTR + 16, &value.to_le_bytes());
    k.poke(ATTR + 24, &flags.to_le_bytes());
}

#[test]
fn attribute_copy_rules() {
    let mut k = kernel();
    assert_eq!(sys(&mut k, nr::BPF, &[0, 0, 64]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::BPF, &[0, UNMAPPED, 64]), Err(Errno::EFAULT));
    attr(&mut k, &[MAP_TYPE_ARRAY, 4, 8, 4]);
    assert_eq!(bpf(&mut k, 0, 4097), Err(Errno::E2BIG));
    assert_eq!(bpf(&mut k, 9999, 64), Err(Errno::EINVAL));
    assert_eq!(bpf(&mut k, -1, 64), Err(Errno::EINVAL));
    assert_eq!(bpf(&mut k, 0, 0), Err(Errno::EINVAL), "all zero: map type 0");
    attr(&mut k, &[MAP_TYPE_ARRAY, 4, 8, 4]);
    assert!(bpf(&mut k, 0, 16).is_ok(), "missing fields count as zero");
    // Bytes past the union Poly knows must be zero.
    k.poke(ATTR + 200, &[1]);
    assert_eq!(bpf(&mut k, 0, 256), Err(Errno::E2BIG));
}

#[test]
fn array_through_descriptors() {
    let mut k = kernel();
    let fd = create(&mut k, MAP_TYPE_ARRAY, 4, 8, 4, 0).unwrap();
    k.poke(KEY, &0u32.to_le_bytes());
    k.poke(VALUE, &[0xee; 8]);
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 1, 64), Ok(0));
    assert_eq!(k.peek_u64(VALUE), 0, "zeroed");
    k.poke(VALUE, &0x1122_3344u64.to_le_bytes());
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 2, 64), Ok(0));
    k.poke(VALUE, &[0; 8]);
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 1, 64), Ok(0));
    assert_eq!(k.peek_u64(VALUE), 0x1122_3344);
    elem(&mut k, fd, KEY, VALUE, 1);
    assert_eq!(bpf(&mut k, 2, 64), Err(Errno::EEXIST));
    elem(&mut k, fd, KEY, VALUE, 4);
    assert_eq!(bpf(&mut k, 2, 64), Err(Errno::EINVAL), "BPF_F_LOCK");
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 3, 64), Err(Errno::EINVAL), "no deleting in arrays");
    // The next key; NULL key gives 0.
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 4, 64), Ok(0));
    assert_eq!(k.peek_u32(VALUE), 1);
    elem(&mut k, fd, 0, VALUE, 0);
    assert_eq!(bpf(&mut k, 4, 64), Ok(0));
    assert_eq!(k.peek_u32(VALUE), 0);
    // Bad pointers.
    elem(&mut k, fd, 8, VALUE, 0);
    assert_eq!(bpf(&mut k, 1, 64), Err(Errno::EFAULT));
    elem(&mut k, fd, KEY, 8, 0);
    assert_eq!(bpf(&mut k, 1, 64), Err(Errno::EFAULT));
    assert_eq!(bpf(&mut k, 2, 64), Err(Errno::EFAULT));
}

#[test]
fn descriptors_that_are_no_maps() {
    let mut k = kernel();
    elem(&mut k, 999, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 1, 64), Err(Errno::EBADF));
    assert_eq!(bpf(&mut k, 2, 64), Err(Errno::EBADF));
    assert_eq!(bpf(&mut k, 22, 64), Err(Errno::EBADF));
    elem(&mut k, 1, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 1, 64), Err(Errno::EINVAL));
    assert_eq!(bpf(&mut k, 3, 64), Err(Errno::EINVAL));
    attr(&mut k, &[1, 104]);
    k.poke(ATTR + 8, &INFO.to_le_bytes());
    assert_eq!(bpf(&mut k, 15, 64), Err(Errno::EINVAL));
}

#[test]
fn freeze_and_info() {
    let mut k = kernel();
    let fd = create(&mut k, MAP_TYPE_HASH, 4, 8, 2, 0).unwrap();
    attr(&mut k, &[fd as u32]);
    assert_eq!(bpf(&mut k, 22, 64), Ok(0));
    assert_eq!(bpf(&mut k, 22, 64), Err(Errno::EPERM));
    k.poke(KEY, &1u32.to_le_bytes());
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 2, 64), Err(Errno::EPERM));
    // Info: the leading fields, written back, the rest of the buffer alone.
    k.poke(INFO, &[0xab; 128]);
    attr(&mut k, &[fd as u32, 128]);
    k.poke(ATTR + 8, &INFO.to_le_bytes());
    assert_eq!(bpf(&mut k, 15, 64), Err(Errno::E2BIG), "excess bytes not zero");
    k.poke(INFO + 104, &[0; 24]);
    assert_eq!(bpf(&mut k, 15, 64), Ok(0));
    assert_eq!(k.peek_u32(ATTR + 4), 104);
    assert_eq!((k.peek_u32(INFO), k.peek_u32(INFO + 8), k.peek_u32(INFO + 12), k.peek_u32(INFO + 16)), (1, 4, 8, 2));
    assert_ne!(k.peek_u32(INFO + 4), 0, "the map id");
    attr(&mut k, &[fd as u32, 16]);
    k.poke(ATTR + 8, &INFO.to_le_bytes());
    k.poke(INFO + 16, &[0xcd; 8]);
    assert_eq!(bpf(&mut k, 15, 64), Ok(0));
    assert_eq!(k.peek_u32(ATTR + 4), 16);
    assert_eq!(k.peek(INFO + 16, 8), [0xcd; 8], "beyond info_len untouched");
    attr(&mut k, &[fd as u32, 0]);
    assert_eq!(bpf(&mut k, 15, 64), Ok(0));
    assert_eq!(k.peek_u32(ATTR + 4), 0);
}

fn load(k: &mut FakeKernel, prog_type: u32, insns: u32, log_level: u32) -> Result<u64, Errno> {
    attr(k, &[prog_type, insns]);
    k.poke(ATTR + 8, &CODE.to_le_bytes());
    k.poke(ATTR + 16, &LICENSE.to_le_bytes());
    k.poke(ATTR + 24, &log_level.to_le_bytes());
    k.poke(ATTR + 28, &128u32.to_le_bytes());
    k.poke(ATTR + 32, &LOG.to_le_bytes());
    bpf(k, 5, 72)
}

#[test]
fn programs() {
    let mut k = kernel();
    // r0 = 0; exit
    k.poke(CODE, &[0xb7, 0, 0, 0, 0, 0, 0, 0, 0x95, 0, 0, 0, 0, 0, 0, 0]);
    assert!(load(&mut k, 1, 2, 0).is_ok());
    assert_eq!(load(&mut k, 1, 0, 0), Err(Errno::E2BIG));
    assert_eq!(load(&mut k, 1, 1, 0), Err(Errno::EINVAL), "does not end in exit");
    assert_eq!(load(&mut k, 9999, 2, 0), Err(Errno::EINVAL));
    // A lone exit reads r0 before it is set: refused, with the log.
    k.poke(LOG, &[0; 16]);
    k.poke(CODE + 8, &[0x95, 0, 0, 0, 0, 0, 0, 0]);
    attr(&mut k, &[1, 1]);
    k.poke(ATTR + 8, &(CODE + 8).to_le_bytes());
    k.poke(ATTR + 16, &LICENSE.to_le_bytes());
    k.poke(ATTR + 24, &1u32.to_le_bytes());
    k.poke(ATTR + 28, &128u32.to_le_bytes());
    k.poke(ATTR + 32, &LOG.to_le_bytes());
    assert_eq!(bpf(&mut k, 5, 72), Err(Errno::EACCES));
    assert!(k.peek(LOG, 16).iter().any(|&b| b != 0), "the verifier's log");
    // The license must be readable.
    attr(&mut k, &[1, 2]);
    k.poke(ATTR + 8, &CODE.to_le_bytes());
    assert_eq!(bpf(&mut k, 5, 72), Err(Errno::EFAULT));
}

#[test]
fn unprivileged_callers_only_get_what_they_hold() {
    let mut k = kernel();
    k.poke(CODE, &[0xb7, 0, 0, 0, 0, 0, 0, 0, 0x95, 0, 0, 0, 0, 0, 0, 0]);
    let fd = create(&mut k, MAP_TYPE_HASH, 4, 8, 2, 0).unwrap();
    *k.creds_mut() = Creds::user(65534, 65534);
    assert_eq!(create(&mut k, MAP_TYPE_ARRAY, 4, 8, 4, 0), Err(Errno::EPERM));
    assert_eq!(create(&mut k, MAP_TYPE_HASH, 4, 8, 4, 0), Err(Errno::EPERM));
    assert_eq!(load(&mut k, 1, 2, 0), Err(Errno::EPERM));
    // An invalid attribute is still invalid; descriptors still work.
    assert_eq!(create(&mut k, MAP_TYPE_ARRAY, 8, 8, 4, 0), Err(Errno::EINVAL));
    k.poke(KEY, &1u32.to_le_bytes());
    elem(&mut k, fd, KEY, VALUE, 0);
    assert_eq!(bpf(&mut k, 1, 64), Err(Errno::ENOENT));
}
