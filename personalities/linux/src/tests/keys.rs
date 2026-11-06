//! add_key(2), request_key(2), keyctl(2) and the LSM calls through the
//! dispatcher, against [`FakeKernel`].

extern crate std;

use super::fake::*;
use crate::keys::{EKEYEXPIRED, EKEYREVOKED, ENOKEY, spec};
use crate::sys::keys::op;
use crate::{Errno, nr};
use std::vec::Vec;

const TYPE: u64 = 0x100;
const DESC: u64 = 0x200;
const PAY: u64 = 0x300;
const OUT: u64 = 0x1000;
const BIG: u64 = 0x2000;

fn put(k: &mut FakeKernel, offset: u64, text: &[u8]) -> u64 {
    let mut b = text.to_vec();
    b.push(0);
    k.poke(scratch(offset), &b);
    scratch(offset)
}

fn id(v: i32) -> u64 {
    v as i64 as u64
}

fn add(k: &mut FakeKernel, ty: &[u8], desc: &[u8], payload: &[u8], ring: i32) -> Result<u64, Errno> {
    let t = put(k, TYPE, ty);
    let d = put(k, DESC, desc);
    k.poke(scratch(PAY), payload);
    let p = if payload.is_empty() { 0 } else { scratch(PAY) };
    sys(k, nr::ADD_KEY, &[t, d, p, payload.len() as u64, id(ring)])
}

fn request(k: &mut FakeKernel, ty: &[u8], desc: &[u8], callout: Option<&[u8]>, dest: i32) -> Result<u64, Errno> {
    let t = put(k, TYPE, ty);
    let d = put(k, DESC, desc);
    let c = callout.map_or(0, |c| put(k, PAY, c));
    sys(k, nr::REQUEST_KEY, &[t, d, c, id(dest)])
}

fn keyctl(k: &mut FakeKernel, args: &[u64]) -> Result<u64, Errno> {
    sys(k, nr::KEYCTL, args)
}

fn search(k: &mut FakeKernel, ring: u64, ty: &[u8], desc: &[u8]) -> Result<u64, Errno> {
    let t = put(k, TYPE, ty);
    let d = put(k, DESC, desc);
    keyctl(k, &[op::SEARCH as u64, ring, t, d, 0])
}

fn describe(k: &mut FakeKernel, key: u64) -> Result<Vec<u8>, Errno> {
    let n = keyctl(k, &[op::DESCRIBE as u64, key, scratch(OUT), 256])?;
    Ok(k.peek(scratch(OUT), n as usize - 1))
}

fn read(k: &mut FakeKernel, key: u64) -> Result<Vec<u8>, Errno> {
    let n = keyctl(k, &[op::READ as u64, key, scratch(OUT), 256])?;
    Ok(k.peek(scratch(OUT), n as usize))
}

fn fresh_session(k: &mut FakeKernel) -> u64 {
    keyctl(k, &[op::JOIN_SESSION_KEYRING as u64, 0]).unwrap()
}

#[test]
fn add_key_user_and_describe() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    assert_eq!(describe(&mut k, s).unwrap(), b"keyring;1001;2001;3f030000;_ses");
    let key = add(&mut k, b"user", b"f_k1", b"hello", spec::SESSION_KEYRING).unwrap();
    assert_eq!(describe(&mut k, key).unwrap(), b"user;1001;2001;3f010000;f_k1");
    assert_eq!(read(&mut k, key).unwrap(), b"hello");
    assert_eq!(add(&mut k, b"user", b"f_k1", b"second", spec::SESSION_KEYRING), Ok(key));
    assert_eq!(read(&mut k, key).unwrap(), b"second");
    assert_eq!(read(&mut k, s).unwrap(), (key as u32).to_le_bytes());
    // Logon keys: never readable, perm 3d010000, "<prefix>:" description.
    assert_eq!(add(&mut k, b"logon", b"nocolon", b"x", spec::SESSION_KEYRING), Err(Errno::EINVAL));
    let lg = add(&mut k, b"logon", b"svc:k", b"x", spec::SESSION_KEYRING).unwrap();
    assert_eq!(describe(&mut k, lg).unwrap(), b"logon;1001;2001;3d010000;svc:k");
    assert_eq!(read(&mut k, lg), Err(Errno::EOPNOTSUPP));
    // Payload and description limits.
    k.poke(scratch(BIG), &[b'a'; 32768]);
    let t = put(&mut k, TYPE, b"user");
    let d = put(&mut k, DESC, b"big");
    assert!(sys(&mut k, nr::ADD_KEY, &[t, d, scratch(BIG), 32767, id(spec::SESSION_KEYRING)]).is_ok());
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[t, d, scratch(BIG), 32768, id(spec::SESSION_KEYRING)]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[t, d, 0, 0, id(spec::SESSION_KEYRING)]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[t, 0, scratch(PAY), 1, id(spec::SESSION_KEYRING)]), Err(Errno::EINVAL));
    k.poke(scratch(BIG), &[b'd'; 4096]);
    k.poke(scratch(BIG + 4095), &[0]);
    assert!(sys(&mut k, nr::ADD_KEY, &[t, scratch(BIG), scratch(PAY), 1, id(spec::SESSION_KEYRING)]).is_ok());
    k.poke(scratch(BIG + 4095), b"d\0");
    assert_eq!(
        sys(&mut k, nr::ADD_KEY, &[t, scratch(BIG), scratch(PAY), 1, id(spec::SESSION_KEYRING)]),
        Err(Errno::EINVAL)
    );
}

#[test]
fn add_key_errors() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let d = put(&mut k, DESC, b"f_e");
    let p = put(&mut k, PAY, b"x");
    let ring = id(spec::SESSION_KEYRING);
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[0, d, p, 1, ring]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[UNMAPPED, d, p, 1, ring]), Err(Errno::EFAULT));
    assert_eq!(add(&mut k, b"", b"f_e", b"x", spec::SESSION_KEYRING), Err(Errno::EINVAL));
    assert_eq!(add(&mut k, b".user", b"f_e", b"x", spec::SESSION_KEYRING), Err(Errno::EPERM));
    assert_eq!(add(&mut k, b"f_nosuchtype", b"f_e", b"x", spec::SESSION_KEYRING), Err(Errno::ENODEV));
    let t = put(&mut k, TYPE, b"user");
    let d = put(&mut k, DESC, b"f_e");
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[t, d, UNMAPPED, 1, ring]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::ADD_KEY, &[t, d, UNMAPPED, (1 << 20) + 1, ring]), Err(Errno::EINVAL));
    assert_eq!(add(&mut k, b"user", b"f_e", b"x", 0), Err(Errno::EINVAL));
    assert_eq!(add(&mut k, b"user", b"f_e", b"x", spec::GROUP_KEYRING), Err(Errno::EINVAL));
    assert_eq!(add(&mut k, b"user", b"f_e", b"x", spec::REQKEY_AUTH_KEY), Err(ENOKEY));
    let plain = add(&mut k, b"user", b"f_plain", b"x", spec::SESSION_KEYRING).unwrap();
    assert_eq!(add(&mut k, b"user", b"f_e", b"x", plain as i32), Err(Errno::ENOTDIR));
    let kr = add(&mut k, b"keyring", b"f_ring", b"", spec::SESSION_KEYRING).unwrap();
    assert_eq!(add(&mut k, b"keyring", b"f_r2", b"x", spec::SESSION_KEYRING), Err(Errno::EINVAL));
    assert_eq!(add(&mut k, b"keyring", b".f_r2", b"", spec::SESSION_KEYRING), Err(Errno::EPERM));
    assert_eq!(keyctl(&mut k, &[op::REVOKE as u64, kr]), Ok(0));
    assert_eq!(add(&mut k, b"user", b"f_e", b"x", kr as i32), Err(EKEYREVOKED));
    assert_eq!(search(&mut k, s, b"user", b"f_e"), Err(ENOKEY));
}

#[test]
fn thread_and_process_keyrings() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let get = |k: &mut FakeKernel, which: i32, create: u64| keyctl(k, &[0, id(which), create]);
    assert_eq!(get(&mut k, spec::THREAD_KEYRING, 0), Err(ENOKEY));
    assert_eq!(get(&mut k, spec::PROCESS_KEYRING, 0), Err(ENOKEY));
    let kt = add(&mut k, b"user", b"f_t", b"t", spec::THREAD_KEYRING).unwrap();
    let tr = get(&mut k, spec::THREAD_KEYRING, 0).unwrap();
    assert_eq!(describe(&mut k, tr).unwrap(), b"keyring;1001;2001;3f010000;_tid");
    let kp = add(&mut k, b"user", b"f_p", b"p", spec::PROCESS_KEYRING).unwrap();
    let pr = get(&mut k, spec::PROCESS_KEYRING, 0).unwrap();
    assert_eq!(describe(&mut k, pr).unwrap(), b"keyring;1001;2001;3f010000;_pid");
    assert_eq!(search(&mut k, s, b"user", b"f_p"), Err(ENOKEY));
    assert_eq!(request(&mut k, b"user", b"f_p", None, 0), Ok(kp));
    assert_eq!(request(&mut k, b"user", b"f_t", None, 0), Ok(kt));
    assert_eq!(get(&mut k, spec::SESSION_KEYRING, 0), Ok(s));
    // Another thread shares the process keyring and session, not the
    // thread keyring.
    k.add_thread(101);
    k.as_thread(101, |k| {
        assert_eq!(get(k, spec::THREAD_KEYRING, 0), Err(ENOKEY));
        assert_eq!(get(k, spec::PROCESS_KEYRING, 0), Ok(pr));
        assert_eq!(get(k, spec::SESSION_KEYRING, 0), Ok(s));
        assert_eq!(request(k, b"user", b"f_t", None, 0), Err(ENOKEY));
    });
    // A fork child keeps only the session keyring.
    let me = k.me();
    let child = me.fork(100, 200);
    let keys = child.thread(200).keys;
    assert_eq!((keys.thread, keys.process, u64::from(keys.session)), (0, 0, s));
}

#[test]
fn join_and_keyring_ids() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let s2 = fresh_session(&mut k);
    assert_ne!(s, s2);
    assert_eq!(keyctl(&mut k, &[0, id(spec::SESSION_KEYRING), 1]), Ok(s2));
    let u = keyctl(&mut k, &[0, id(spec::USER_KEYRING), 0]).unwrap();
    let us = keyctl(&mut k, &[0, id(spec::USER_SESSION_KEYRING), 0]).unwrap();
    assert!(u > 0 && us > 0 && u != us);
    assert_eq!(keyctl(&mut k, &[0, id(spec::GROUP_KEYRING), 1]), Err(Errno::EINVAL));
    assert_eq!(keyctl(&mut k, &[0, id(-9), 1]), Err(Errno::EINVAL));
    assert_eq!(keyctl(&mut k, &[0, id(spec::REQUESTOR_KEYRING), 1]), Err(ENOKEY));
    assert_eq!(keyctl(&mut k, &[0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(keyctl(&mut k, &[0, 0x7fff_fff3, 0]), Err(ENOKEY));
    let empty = put(&mut k, DESC, b"");
    assert_eq!(keyctl(&mut k, &[op::JOIN_SESSION_KEYRING as u64, empty]), Err(Errno::EINVAL));
    let dot = put(&mut k, DESC, b".f_dot");
    assert_eq!(keyctl(&mut k, &[op::JOIN_SESSION_KEYRING as u64, dot]), Err(Errno::EPERM));
    assert_eq!(keyctl(&mut k, &[op::JOIN_SESSION_KEYRING as u64, UNMAPPED]), Err(Errno::EFAULT));
    assert_eq!(keyctl(&mut k, &[0, id(spec::SESSION_KEYRING), 0]), Ok(s2));
    let name = put(&mut k, DESC, b"f_named");
    let n = keyctl(&mut k, &[op::JOIN_SESSION_KEYRING as u64, name]).unwrap();
    assert_eq!(describe(&mut k, n).unwrap(), b"keyring;1001;2001;3f130000;f_named");
    assert_eq!(keyctl(&mut k, &[op::JOIN_SESSION_KEYRING as u64, name]), Ok(n));
}

#[test]
fn permissions() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let key = add(&mut k, b"user", b"f_k", b"x", spec::SESSION_KEYRING).unwrap();
    let setperm = |k: &mut FakeKernel, key: u64, p: u64| keyctl(k, &[op::SETPERM as u64, key, p]);
    assert_eq!(setperm(&mut k, key, 0x4000_0000), Err(Errno::EINVAL));
    assert_eq!(setperm(&mut k, key, 0x40), Err(Errno::EINVAL));
    assert_eq!(setperm(&mut k, key, 0x3f3f_3f3f), Ok(0));
    assert_eq!(setperm(&mut k, key, 0x0900_0000), Ok(0));
    assert_eq!(describe(&mut k, key).unwrap(), b"user;1001;2001;09000000;f_k");
    assert_eq!(read(&mut k, key).unwrap(), b"x", "possessed: no READ bit needed");
    let p = put(&mut k, PAY, b"y");
    assert_eq!(keyctl(&mut k, &[op::UPDATE as u64, key, p, 1]), Err(Errno::EACCES));
    assert_eq!(setperm(&mut k, key, 0x3f00_0000), Err(Errno::EACCES));
    assert_eq!(keyctl(&mut k, &[op::CHOWN as u64, key, u64::from(u32::MAX), u64::from(u32::MAX)]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::CHOWN as u64, key, 1001, u64::from(u32::MAX)]), Err(Errno::EACCES));
    assert_eq!(keyctl(&mut k, &[op::REVOKE as u64, key]), Err(Errno::EACCES));
    assert_eq!(keyctl(&mut k, &[op::LINK as u64, key, s]), Err(Errno::EACCES));
    assert_eq!(search(&mut k, s, b"user", b"f_k"), Ok(key));
    // View only: not possessed.
    let v = add(&mut k, b"user", b"f_v", b"x", spec::SESSION_KEYRING).unwrap();
    assert_eq!(setperm(&mut k, v, 0x0100_0000), Ok(0));
    assert_eq!(describe(&mut k, v), Err(Errno::EACCES));
    assert_eq!(read(&mut k, v), Err(Errno::EACCES));
    assert_eq!(search(&mut k, s, b"user", b"f_v"), Err(Errno::EACCES));
}

#[test]
fn describe_and_read_buffers() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let key = add(&mut k, b"user", b"f_k1", b"hello", spec::SESSION_KEYRING).unwrap();
    let text = b"user;1001;2001;3f010000;f_k1\0";
    let n = text.len() as u64;
    assert_eq!(keyctl(&mut k, &[op::DESCRIBE as u64, key, 0, 0]), Ok(n));
    k.poke(scratch(OUT), &[b'Z'; 64]);
    assert_eq!(keyctl(&mut k, &[op::DESCRIBE as u64, key, scratch(OUT), n - 1]), Ok(n));
    assert_eq!(k.peek(scratch(OUT), 1), b"Z");
    assert_eq!(keyctl(&mut k, &[op::DESCRIBE as u64, key, scratch(OUT), n]), Ok(n));
    assert_eq!(k.peek(scratch(OUT), text.len() + 1), [&text[..], b"Z"].concat());
    assert_eq!(keyctl(&mut k, &[op::DESCRIBE as u64, key, 8, 100]), Err(Errno::EFAULT));
    assert_eq!(keyctl(&mut k, &[op::READ as u64, key, 0, 0]), Ok(5));
    assert_eq!(keyctl(&mut k, &[op::READ as u64, key, 8, 100]), Err(Errno::EFAULT));
    let kr = add(&mut k, b"keyring", b"f_ring", b"", spec::SESSION_KEYRING).unwrap();
    assert_eq!(read(&mut k, kr).unwrap(), b"");
    let a = add(&mut k, b"user", b"f_a", b"1", kr as i32).unwrap();
    let b = add(&mut k, b"user", b"f_b", b"2", kr as i32).unwrap();
    let mut want: Vec<u8> = (a as u32).to_le_bytes().to_vec();
    want.extend_from_slice(&(b as u32).to_le_bytes());
    assert_eq!(read(&mut k, kr).unwrap(), want);
    assert_eq!(read(&mut k, s).unwrap().len(), 8);
    assert_eq!(keyctl(&mut k, &[op::READ as u64, 0x7fff_fff3, scratch(OUT), 8]), Err(ENOKEY));
    assert_eq!(keyctl(&mut k, &[op::DESCRIBE as u64, 0, scratch(OUT), 8]), Err(Errno::EINVAL));
}

#[test]
fn update_revoke_invalidate_timeout() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let key = add(&mut k, b"user", b"f_k", b"one", spec::SESSION_KEYRING).unwrap();
    let p = put(&mut k, PAY, b"second");
    assert_eq!(keyctl(&mut k, &[op::UPDATE as u64, key, p, 6]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::UPDATE as u64, key, p, 0]), Err(Errno::EINVAL));
    assert_eq!(keyctl(&mut k, &[op::UPDATE as u64, key, 8, 4]), Err(Errno::EFAULT));
    assert_eq!(read(&mut k, key).unwrap(), b"second");
    let kr = add(&mut k, b"keyring", b"f_ring", b"", spec::SESSION_KEYRING).unwrap();
    assert_eq!(keyctl(&mut k, &[op::UPDATE as u64, kr, p, 1]), Err(Errno::EOPNOTSUPP));
    assert_eq!(keyctl(&mut k, &[op::REVOKE as u64, key]), Ok(0));
    assert_eq!(read(&mut k, key), Err(EKEYREVOKED));
    assert_eq!(describe(&mut k, key), Err(EKEYREVOKED));
    assert_eq!(keyctl(&mut k, &[op::REVOKE as u64, key]), Err(EKEYREVOKED));
    assert_eq!(keyctl(&mut k, &[op::SETPERM as u64, key, 0x3f00_0000]), Err(EKEYREVOKED));
    assert_eq!(search(&mut k, s, b"user", b"f_k"), Err(EKEYREVOKED));
    assert_eq!(keyctl(&mut k, &[0, key, 0]), Err(EKEYREVOKED));
    let j = add(&mut k, b"user", b"f_j", b"j", spec::SESSION_KEYRING).unwrap();
    assert_eq!(keyctl(&mut k, &[op::INVALIDATE as u64, j]), Ok(0));
    assert_eq!(describe(&mut k, j), Err(ENOKEY));
    // Timeouts count wall-clock seconds.
    let t = add(&mut k, b"user", b"f_t", b"t", spec::SESSION_KEYRING).unwrap();
    let c = add(&mut k, b"user", b"f_c", b"c", spec::SESSION_KEYRING).unwrap();
    assert_eq!(keyctl(&mut k, &[op::SET_TIMEOUT as u64, t, 1]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::SET_TIMEOUT as u64, c, 1]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::SET_TIMEOUT as u64, c, 0]), Ok(0));
    assert_eq!(read(&mut k, t).unwrap(), b"t");
    k.realtime.sec += 2;
    assert_eq!(read(&mut k, t), Err(EKEYEXPIRED));
    assert_eq!(search(&mut k, s, b"user", b"f_t"), Err(EKEYEXPIRED));
    assert_eq!(request(&mut k, b"user", b"f_t", None, 0), Err(ENOKEY));
    assert_eq!(keyctl(&mut k, &[op::REVOKE as u64, t]), Err(EKEYEXPIRED));
    assert_eq!(keyctl(&mut k, &[op::INVALIDATE as u64, t]), Err(EKEYEXPIRED));
    assert_eq!(read(&mut k, c).unwrap(), b"c");
}

#[test]
fn keyctl_other_operations() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    let key = add(&mut k, b"user", b"f_k", b"x", spec::SESSION_KEYRING).unwrap();
    assert_eq!(keyctl(&mut k, &[9999, 0]), Err(Errno::EOPNOTSUPP));
    assert_eq!(keyctl(&mut k, &[u64::MAX, 0]), Err(Errno::EOPNOTSUPP));
    for o in [op::INSTANTIATE, op::NEGATE, op::REJECT, op::INSTANTIATE_IOV] {
        assert_eq!(keyctl(&mut k, &[o as u64, key, 0, 0, 0]), Err(Errno::EPERM));
    }
    assert_eq!(keyctl(&mut k, &[op::ASSUME_AUTHORITY as u64, 0]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::ASSUME_AUTHORITY as u64, u64::MAX]), Err(Errno::EINVAL));
    assert_eq!(keyctl(&mut k, &[op::ASSUME_AUTHORITY as u64, key]), Err(ENOKEY));
    let set = |k: &mut FakeKernel, v: i64| keyctl(k, &[op::SET_REQKEY_KEYRING as u64, v as u64]);
    assert_eq!(set(&mut k, -1), Ok(0));
    assert_eq!(set(&mut k, 3), Ok(0));
    assert_eq!(set(&mut k, -1), Ok(3));
    assert_eq!(set(&mut k, 8), Err(Errno::EINVAL));
    assert_eq!(set(&mut k, 0), Ok(3));
    assert_eq!(keyctl(&mut k, &[22, u64::MAX, id(spec::SESSION_KEYRING)]), Err(Errno::EOPNOTSUPP));
    assert_eq!(keyctl(&mut k, &[op::LINK as u64, 0x7fff_fff3, s]), Err(ENOKEY));
    // Links, cycles, clear, move.
    let r1 = add(&mut k, b"keyring", b"r1", b"", spec::SESSION_KEYRING).unwrap();
    let r2 = add(&mut k, b"keyring", b"r2", b"", r1 as i32).unwrap();
    assert_eq!(keyctl(&mut k, &[op::LINK as u64, r1, r2]), Err(crate::keys::EDEADLK));
    assert_eq!(keyctl(&mut k, &[op::LINK as u64, r1, r1]), Err(crate::keys::EDEADLK));
    assert_eq!(keyctl(&mut k, &[op::LINK as u64, key, key]), Err(Errno::ENOTDIR));
    assert_eq!(keyctl(&mut k, &[op::LINK as u64, key, r2]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::MOVE as u64, key, r2, r1, 2]), Err(Errno::EINVAL));
    assert_eq!(keyctl(&mut k, &[op::MOVE as u64, key, r2, s, 1]), Err(Errno::EEXIST));
    assert_eq!(keyctl(&mut k, &[op::MOVE as u64, key, r2, r1, 1]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::UNLINK as u64, key, r2]), Err(Errno::ENOENT));
    assert_eq!(keyctl(&mut k, &[op::UNLINK as u64, key, r1]), Ok(0));
    assert_eq!(keyctl(&mut k, &[op::CLEAR as u64, key]), Err(Errno::ENOTDIR));
    assert_eq!(keyctl(&mut k, &[op::CLEAR as u64, s]), Ok(0));
    assert_eq!(read(&mut k, s).unwrap(), b"");
}

#[test]
fn request_key_found_and_constructed() {
    let mut k = FakeKernel::new();
    let s = fresh_session(&mut k);
    assert_eq!(sys(&mut k, nr::REQUEST_KEY, &[0, scratch(DESC), 0, 0]), Err(Errno::EFAULT));
    let t = put(&mut k, TYPE, b"user");
    assert_eq!(sys(&mut k, nr::REQUEST_KEY, &[t, 0, 0, 0]), Err(Errno::EFAULT));
    assert_eq!(request(&mut k, b".user", b"f_k", None, 0), Err(Errno::EPERM));
    assert_eq!(request(&mut k, b"", b"f_k", None, 0), Err(Errno::EINVAL));
    assert_eq!(request(&mut k, b"user", b"f_k", None, spec::GROUP_KEYRING), Err(Errno::EINVAL));
    assert_eq!(request(&mut k, b"keyring", b"f_new", Some(b"info"), 0), Err(Errno::EPERM));
    let key = add(&mut k, b"user", b"f_k", b"v", spec::SESSION_KEYRING).unwrap();
    assert_eq!(request(&mut k, b"user", b"f_k", None, spec::PROCESS_KEYRING), Ok(key));
    let p = keyctl(&mut k, &[0, id(spec::PROCESS_KEYRING), 0]).unwrap();
    assert_eq!(read(&mut k, p).unwrap(), (key as u32).to_le_bytes());
    assert_eq!(request(&mut k, b"logon", b"f_k", None, 0), Err(ENOKEY));
    // A keyring reachable only by serial is not searched.
    let lone = add(&mut k, b"keyring", b"f_lone", b"", spec::SESSION_KEYRING).unwrap();
    add(&mut k, b"user", b"f_x", b"x", lone as i32).unwrap();
    assert_eq!(keyctl(&mut k, &[op::UNLINK as u64, lone, s]), Ok(0));
    assert_eq!(request(&mut k, b"user", b"f_x", None, 0), Err(ENOKEY));
    // Construction fails without /sbin/request-key and leaves a negative key.
    assert_eq!(request(&mut k, b"user", b"f_miss", Some(b"info"), 0), Err(Errno::ENOENT));
    assert_eq!(request(&mut k, b"user", b"f_miss", None, 0), Err(ENOKEY));
    assert_eq!(request(&mut k, b"user", b"f_miss", Some(b""), 0), Err(ENOKEY));
    assert_eq!(search(&mut k, s, b"user", b"f_miss"), Err(ENOKEY));
}

#[test]
fn lsm_calls() {
    let mut k = FakeKernel::new();
    let size = scratch(0x100);
    let ids = scratch(0x200);
    k.poke(size, &64u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::LSM_LIST_MODULES, &[ids, size, 0]), Ok(2));
    assert_eq!(k.peek_u32(size), 16);
    assert_eq!(k.peek_u64(ids), 100, "capability first");
    assert_eq!(k.peek_u64(ids + 8), 110);
    assert_eq!(sys(&mut k, nr::LSM_LIST_MODULES, &[ids, UNMAPPED, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LSM_LIST_MODULES, &[ids, 0, 0]), Err(Errno::EFAULT));
    k.poke(size, &15u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::LSM_LIST_MODULES, &[0, size, 0]), Err(Errno::E2BIG));
    assert_eq!(k.peek_u32(size), 16);
    k.poke(size, &64u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::LSM_LIST_MODULES, &[0, size, 0]), Err(Errno::EFAULT));
    assert_eq!(k.peek_u32(size), 16, "size is set before the ids fault");
    let ro = read_only_page(&mut k);
    assert_eq!(sys(&mut k, nr::LSM_LIST_MODULES, &[ids, ro, 0]), Err(Errno::EFAULT));

    // lsm_get_self_attr: nothing provides an attribute.
    let ctx = scratch(0x400);
    k.poke(size, &64u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, ctx, size, 2]), Err(Errno::EINVAL));
    assert_eq!(k.peek_u32(size), 64);
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, ctx, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, ctx, UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[0, ctx, size, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, ctx, size, 0]), Err(Errno::EOPNOTSUPP));
    assert_eq!(k.peek_u32(size), 0);
    k.poke(ctx, &0u64.to_le_bytes());
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, ctx, size, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, 0, size, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, 8, size, 1]), Err(Errno::EFAULT));
    k.poke(ctx, &110u64.to_le_bytes());
    k.poke(size, &64u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::LSM_GET_SELF_ATTR, &[100, ctx, size, 1]), Err(Errno::EOPNOTSUPP));
    assert_eq!(k.peek_u32(size), 0);

    // lsm_set_self_attr: header checks, then no module.
    let hdr = |k: &mut FakeKernel, id: u64, len: u64, ctx_len: u64| {
        k.poke(ctx, &[0; 64]);
        k.poke(ctx, &id.to_le_bytes());
        k.poke(ctx + 16, &len.to_le_bytes());
        k.poke(ctx + 24, &ctx_len.to_le_bytes());
    };
    hdr(&mut k, 100, 40, 7);
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, 0, 40, 1]), Err(Errno::EINVAL));
    for small in [0, 31, 32] {
        assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, small, 0]), Err(Errno::EINVAL));
    }
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 4097, 0]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, 0, 40, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 0]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 64, 0]), Err(Errno::EOPNOTSUPP));
    hdr(&mut k, 100, 48, 7);
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 0]), Err(Errno::EINVAL));
    hdr(&mut k, 100, 40, 100);
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 0]), Err(Errno::EINVAL));
    hdr(&mut k, 100, 32, 8);
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 0]), Err(Errno::EINVAL));
    hdr(&mut k, 100, 33, 1);
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 0]), Err(Errno::EOPNOTSUPP));
    hdr(&mut k, 100, 40, u64::MAX);
    assert_eq!(sys(&mut k, nr::LSM_SET_SELF_ATTR, &[100, ctx, 40, 0]), Err(Errno::EINVAL));
}
