//! System V message queues: msgget(2), msgsnd(2), msgrcv(2), msgctl(2)
//! (`specs/linux/x86_64/msgget.yaml` and its visible tests). The rules of
//! msgget itself are tested on the table (`crate::ipc::msg`); these tests
//! cover the handlers: argument widths, user memory, sleeping, waking and
//! signals.
//!
//! Each test runs in an IPC namespace of its own.

use super::fake::*;
use crate::abi::signal::SIGUSR1;
use crate::ipc::msg::*;
use crate::ipc::shm::{IPC_CREAT, IPC_EXCL, IPC_PRIVATE, IPC_RMID, IPC_SET, IPC_STAT};
use crate::ns::IpcNs;
use crate::{Errno, nr};

/// A fake process (pid 100, euid 1001) in an IPC namespace of its own.
fn fresh() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.me().ns.ipc = Some(IpcNs::create(&None));
    k
}

/// Another process `pid` of the same user in the same IPC namespace.
fn other(k: &mut FakeKernel, pid: i32) {
    let ns = k.me().ns.ipc.clone();
    k.add_process(pid, 1001, pid, pid);
    k.proc_state(pid).ns.ipc = ns;
}

const MSG: u64 = DATA + 0x800;
const BUF: u64 = DATA + 0x1000;
const KEY: u64 = 0x4000_0064;

fn msgget(k: &mut FakeKernel, key: u64, flags: u64) -> Result<u64, Errno> {
    sys(k, nr::MSGGET, &[key, flags])
}

fn send(k: &mut FakeKernel, id: u64, mtype: i64, text: &[u8], flags: u32) -> Result<u64, Errno> {
    k.poke(MSG, &mtype.to_ne_bytes());
    k.poke(MSG + 8, text);
    sys(k, nr::MSGSND, &[id, MSG, text.len() as u64, u64::from(flags)])
}

/// Receive at most 16 bytes; the result and what was written.
fn recv(k: &mut FakeKernel, id: u64, msgtyp: i64, flags: u32) -> Result<(i64, alloc::vec::Vec<u8>), Errno> {
    let n = sys(k, nr::MSGRCV, &[id, MSG, 16, msgtyp as u64, u64::from(flags)])?;
    Ok((k.peek_u64(MSG) as i64, k.peek(MSG + 8, n as usize)))
}

fn stat(k: &mut FakeKernel, id: u64) -> Result<u64, Errno> {
    sys(k, nr::MSGCTL, &[id, u64::from(IPC_STAT), BUF])
}

fn rm(k: &mut FakeKernel, id: u64) -> Result<u64, Errno> {
    sys(k, nr::MSGCTL, &[id, u64::from(IPC_RMID), 0])
}

const C: u64 = IPC_CREAT as u64;
const X: u64 = IPC_EXCL as u64;
const NOWAIT: u32 = IPC_NOWAIT;

#[test]
fn create_lookup_and_stat() {
    // c_msgget_create_lookup; spec: new_queue_stat
    let mut k = fresh();
    assert_eq!(msgget(&mut k, KEY, 0o600), Err(Errno::ENOENT));
    assert_eq!(msgget(&mut k, KEY, 0o600 | X), Err(Errno::ENOENT));
    let id = msgget(&mut k, KEY, 0o600 | C).unwrap();
    assert_eq!(msgget(&mut k, KEY, 0), Ok(id));
    assert_eq!(msgget(&mut k, KEY, X), Ok(id));
    assert_eq!(msgget(&mut k, KEY, 0o600 | C | X), Err(Errno::EEXIST));
    assert_eq!(stat(&mut k, id), Ok(0));
    assert_eq!(k.peek_u32(BUF), KEY as u32, "key");
    assert_eq!(k.peek_u32(BUF + 4), 1001, "uid");
    assert_eq!(k.peek_u32(BUF + 12), 1001, "cuid");
    assert_eq!(k.peek_u32(BUF + 20) & 0xffff, 0o600, "mode");
    assert_eq!(k.peek_u64(BUF + 48), 0, "stime");
    assert_eq!(k.peek_u64(BUF + 56), 0, "rtime");
    assert_eq!(k.peek_u64(BUF + 72), 0, "cbytes");
    assert_eq!(k.peek_u64(BUF + 80), 0, "qnum");
    assert_eq!(k.peek_u64(BUF + 88), MSGMNB, "qbytes");
    assert_eq!(k.peek_u64(BUF + 96), 0, "lspid, lrpid");
    assert_eq!(rm(&mut k, id), Ok(0));
}

#[test]
fn argument_widths() {
    // c_msgget_key_width
    let mut k = fresh();
    let id = msgget(&mut k, KEY, 0o600 | C | X).unwrap();
    assert_eq!(msgget(&mut k, 1 << 32 | KEY, 0), Ok(id));
    assert_eq!(msgget(&mut k, 0xffff_ffff_0000_0000 | KEY, 0), Ok(id));
    assert_eq!(msgget(&mut k, KEY, 1 << 32 | C | X | 0o600), Err(Errno::EEXIST));
    assert_eq!(msgget(&mut k, KEY + 1, 1 << 32 | 0o600), Err(Errno::ENOENT));
    // msgflg -1 is IPC_CREAT | IPC_EXCL | 0777.
    let full = msgget(&mut k, KEY + 1, u64::MAX).unwrap();
    assert_eq!(stat(&mut k, full), Ok(0));
    assert_eq!(k.peek_u32(BUF + 20) & 0xffff, 0o777);
    assert_eq!(msgget(&mut k, KEY + 1, u64::MAX), Err(Errno::EEXIST));
    // IPC_PRIVATE is any key whose low half is 0.
    let a = msgget(&mut k, 1 << 32, 0).unwrap();
    let b = msgget(&mut k, IPC_PRIVATE as u64, 0).unwrap();
    assert_ne!(a, b);
    // A negative key, zero- or sign-extended.
    let neg = (-7i64 - 100) as u64;
    let id = msgget(&mut k, neg, 0o600 | C | X).unwrap();
    assert_eq!(stat(&mut k, id), Ok(0));
    assert_eq!(k.peek_u32(BUF) as i32, -107);
    assert_eq!(msgget(&mut k, neg & 0xffff_ffff, 0), Ok(id));
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id | 1 << 32, u64::from(IPC_STAT), BUF]), Ok(0));
}

#[test]
fn send_receive_and_removal() {
    // c_msgget_ids, c_msgget_removed_key
    let mut k = fresh();
    let id = msgget(&mut k, KEY, 0o600 | C | X).unwrap();
    let other_q = msgget(&mut k, 0, 0o600).unwrap();
    assert_eq!(send(&mut k, id, 1, b"abc", NOWAIT), Ok(0));
    assert_eq!(stat(&mut k, other_q), Ok(0));
    assert_eq!(k.peek_u64(BUF + 80), 0, "only on its own queue");
    assert_eq!(stat(&mut k, id), Ok(0));
    assert_eq!((k.peek_u64(BUF + 72), k.peek_u64(BUF + 80)), (3, 1));
    assert_eq!(k.peek_u32(BUF + 96), 100, "lspid");
    assert_eq!(rm(&mut k, id), Ok(0));
    assert_eq!(msgget(&mut k, KEY, 0o600), Err(Errno::ENOENT));
    assert_eq!(stat(&mut k, id), Err(Errno::EINVAL));
    assert_eq!(send(&mut k, id, 1, b"x", NOWAIT), Err(Errno::EINVAL));
    assert_eq!(rm(&mut k, id), Err(Errno::EINVAL));
    let id2 = msgget(&mut k, KEY, 0o600 | C | X).unwrap();
    assert_ne!(id2, id);
    assert_eq!(recv(&mut k, id2, 0, NOWAIT), Err(Errno::ENOMSG));
    assert_eq!(send(&mut k, id2, 7, b"child", NOWAIT), Ok(0));
    assert_eq!(recv(&mut k, id2, 0, NOWAIT), Ok((7, b"child".to_vec())));
    assert_eq!(stat(&mut k, id2), Ok(0));
    assert_eq!(k.peek_u32(BUF + 100), 100, "lrpid");
}

#[test]
fn send_and_receive_errors() {
    let mut k = fresh();
    let id = msgget(&mut k, 0, 0o600).unwrap();
    assert_eq!(send(&mut k, id, 0, b"a", NOWAIT), Err(Errno::EINVAL), "mtype 0");
    assert_eq!(send(&mut k, id, -1, b"a", NOWAIT), Err(Errno::EINVAL), "negative mtype");
    assert_eq!(sys(&mut k, nr::MSGSND, &[id, MSG, MSGMAX + 1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MSGSND, &[id, MSG, u64::MAX, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MSGSND, &[id, UNMAPPED, 1, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::MSGRCV, &[id, MSG, u64::MAX, 0, 0]), Err(Errno::EINVAL), "negative msgsz");
    assert_eq!(sys(&mut k, nr::MSGRCV, &[id, MSG, 16, 0, u64::from(MSG_COPY)]), Err(Errno::EINVAL));
    assert_eq!(send(&mut k, id, 2, b"too long", NOWAIT), Ok(0));
    assert_eq!(sys(&mut k, nr::MSGRCV, &[id, MSG, 3, 0, u64::from(NOWAIT)]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::MSGRCV, &[id, MSG, 3, 0, u64::from(NOWAIT | MSG_NOERROR)]), Ok(3));
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id, 99, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MSGCTL, &[u64::MAX, u64::from(IPC_STAT), BUF]), Err(Errno::EINVAL));
}

#[test]
fn ipc_set_changes_the_mode_for_the_next_lookup() {
    // c_msgget_ipc_set
    let mut k = fresh();
    let id = msgget(&mut k, KEY, 0o600 | C).unwrap();
    assert_eq!(stat(&mut k, id), Ok(0));
    k.poke(BUF + 20, &0o400u16.to_ne_bytes());
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id, u64::from(IPC_SET), BUF]), Ok(0));
    assert_eq!(msgget(&mut k, KEY, 0o600), Err(Errno::EACCES));
    assert_eq!(msgget(&mut k, KEY, 0o400), Ok(id));
    assert_eq!(msgget(&mut k, KEY, 0o600 | C), Err(Errno::EACCES));
    assert_eq!(send(&mut k, id, 1, b"a", NOWAIT), Err(Errno::EACCES));
    k.poke(BUF + 20, &0o600u16.to_ne_bytes());
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id, u64::from(IPC_SET), BUF]), Ok(0));
    assert_eq!(msgget(&mut k, KEY, 0o600), Ok(id));
    assert_eq!(stat(&mut k, id), Ok(0));
    assert_eq!(k.peek_u32(BUF), KEY as u32, "key kept");
    // Raising msg_qbytes needs CAP_SYS_RESOURCE.
    k.poke(BUF + 88, &(MSGMNB + 1).to_ne_bytes());
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id, u64::from(IPC_SET), BUF]), Err(Errno::EPERM));
    // A mode 0 queue cannot be read by IPC_STAT.
    k.poke(BUF + 88, &MSGMNB.to_ne_bytes());
    k.poke(BUF + 20, &0u16.to_ne_bytes());
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id, u64::from(IPC_SET), BUF]), Ok(0));
    assert_eq!(stat(&mut k, id), Err(Errno::EACCES));
}

#[test]
fn queues_are_shared_by_the_namespace() {
    // c_msgget_fork_persist
    let mut k = fresh();
    let pre = msgget(&mut k, KEY, 0o600 | C | X).unwrap();
    other(&mut k, 200);
    k.as_thread(200, |k| {
        let id = msgget(k, KEY + 1, 0o600 | C | X).unwrap();
        assert_eq!(send(k, id, 7, b"child", NOWAIT), Ok(0));
        assert_eq!(msgget(k, KEY, 0o600), Ok(pre));
        assert_eq!(send(k, pre, 3, b"hi", NOWAIT), Ok(0));
    });
    let id = msgget(&mut k, KEY + 1, 0o600).unwrap();
    assert_eq!(stat(&mut k, id), Ok(0));
    assert_eq!(k.peek_u32(BUF + 96), 200, "lspid is the child");
    assert_eq!(recv(&mut k, id, 0, NOWAIT), Ok((7, b"child".to_vec())));
    assert_eq!(recv(&mut k, pre, 0, NOWAIT), Ok((3, b"hi".to_vec())));
}

#[test]
fn a_receiver_is_woken_by_a_sender() {
    let mut k = fresh();
    let id = msgget(&mut k, 0, 0o600).unwrap();
    other(&mut k, 200);
    k.meanwhile(move |k| {
        k.as_thread(200, |k| assert_eq!(send(k, id, 4, b"late", 0), Ok(0)));
    });
    assert_eq!(recv(&mut k, id, 4, 0), Ok((4, b"late".to_vec())));
}

#[test]
fn a_sender_is_woken_by_a_receiver() {
    let mut k = fresh();
    let id = msgget(&mut k, 0, 0o600).unwrap();
    assert_eq!(stat(&mut k, id), Ok(0));
    k.poke(BUF + 88, &4u64.to_ne_bytes());
    assert_eq!(sys(&mut k, nr::MSGCTL, &[id, u64::from(IPC_SET), BUF]), Ok(0));
    assert_eq!(send(&mut k, id, 1, b"abc", NOWAIT), Ok(0));
    assert_eq!(send(&mut k, id, 1, b"de", NOWAIT), Err(Errno::EAGAIN));
    other(&mut k, 200);
    k.meanwhile(move |k| {
        k.as_thread(200, |k| assert_eq!(recv(k, id, 0, NOWAIT), Ok((1, b"abc".to_vec()))));
    });
    assert_eq!(send(&mut k, id, 1, b"de", 0), Ok(0));
}

#[test]
fn a_signal_ends_the_wait_with_eintr() {
    let mut k = fresh();
    let id = msgget(&mut k, 0, 0o600).unwrap();
    k.add_thread(101);
    k.meanwhile(move |k| {
        k.as_thread(101, |k| {
            sys(k, nr::TGKILL, &[100, 100, u64::from(SIGUSR1)]).unwrap();
        });
    });
    assert_eq!(recv(&mut k, id, 0, 0), Err(Errno::EINTR));
}

#[test]
fn removal_ends_the_wait_with_eidrm() {
    let mut k = fresh();
    let id = msgget(&mut k, KEY, 0o600 | C).unwrap();
    other(&mut k, 200);
    k.meanwhile(move |k| {
        k.as_thread(200, |k| {
            assert_eq!(rm(k, id), Ok(0));
            // The key comes back with a new queue at once.
            assert!(msgget(k, KEY, 0o600 | C | X).is_ok());
        });
    });
    assert_eq!(recv(&mut k, id, 0, 0), Err(Errno::EIDRM));
}
