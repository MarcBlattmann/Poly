//! System V message queues: msgget(2), msgsnd(2), msgrcv(2), msgctl(2)
//! (`specs/linux/x86_64/msg*.yaml` and their visible tests). The queue
//! rules are tested on the table (`crate::ipc::msg`); these tests cover
//! the handlers: argument widths and checks, user memory, the order of
//! the errors.
//!
//! Each test runs in an IPC namespace of its own.

use super::fake::*;
use crate::ipc::msg::*;
use crate::ipc::shm::{IPC_RMID, IPC_SET, IPC_STAT};
use crate::ns::IpcNs;
use crate::{Errno, nr};

/// A fake process (pid 100, euid 1001) in an IPC namespace of its own.
fn fresh() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.me().ns.ipc = Some(IpcNs::create(&None));
    k
}

const MSG: u64 = DATA + 0x800;
const BUF: u64 = DATA + 0x1000;

fn msgget(k: &mut FakeKernel) -> u64 {
    sys(k, nr::MSGGET, &[0, 0o600]).unwrap()
}

/// Send `text` with `mtype` from [`MSG`].
fn send(k: &mut FakeKernel, id: u64, mtype: i64, text: &[u8], flags: u32) -> Result<u64, Errno> {
    k.poke(MSG, &mtype.to_ne_bytes());
    k.poke(MSG + 8, text);
    sys(k, nr::MSGSND, &[id, MSG, text.len() as u64, u64::from(flags)])
}

fn recv(k: &mut FakeKernel, id: u64, buf: u64, size: u64, msgtyp: i64, flags: u32) -> Result<u64, Errno> {
    sys(k, nr::MSGRCV, &[id, buf, size, msgtyp as u64, u64::from(flags)])
}

fn msgctl(k: &mut FakeKernel, id: u64, cmd: u32, buf: u64) -> Result<u64, Errno> {
    sys(k, nr::MSGCTL, &[id, u64::from(cmd), buf])
}

#[test]
fn send_checks_in_order() {
    // c_msgsnd_basic; spec: msgsnd order.
    let mut k = fresh();
    let id = msgget(&mut k);
    assert_eq!(sys(&mut k, nr::MSGSND, &[u64::MAX, 0, 1 << 20, 0]), Err(Errno::EFAULT), "mtype read first");
    k.poke(MSG, &5i64.to_ne_bytes());
    assert_eq!(sys(&mut k, nr::MSGSND, &[u64::MAX, MSG, 1, 0]), Err(Errno::EINVAL), "negative id");
    assert_eq!(sys(&mut k, nr::MSGSND, &[id, MSG, MSGMAX + 1, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MSGSND, &[id, MSG, 1 << 32 | 1, 0]), Err(Errno::EINVAL), "msgsz is 64-bit");
    assert_eq!(send(&mut k, id, 0, b"a", 0), Err(Errno::EINVAL), "mtype 0");
    assert_eq!(send(&mut k, id, i64::MIN, b"a", 0), Err(Errno::EINVAL));
    assert_eq!(send(&mut k, 0x7fff_fff0, 1, b"a", 0), Err(Errno::EINVAL));
    k.poke(MSG, &7i64.to_ne_bytes());
    k.poke(MSG + 8, b"hello");
    assert_eq!(sys(&mut k, nr::MSGSND, &[id | 1 << 32, MSG, 5, 1 << 32]), Ok(0), "upper halves ignored");
    assert_eq!(send(&mut k, id, 1, b"", IPC_NOWAIT), Ok(0), "zero-length message");
    assert_eq!(msgctl(&mut k, id, IPC_STAT, BUF), Ok(0));
    assert_eq!((k.peek_u64(BUF + 72), k.peek_u64(BUF + 80), k.peek_u64(BUF + 88)), (5, 2, MSGMNB));
    assert_eq!(k.peek_u64(BUF + 96) as u32, 100, "msg_lspid");
}

#[test]
fn receive_copies_type_then_text() {
    // c_msgrcv_errors, c_msgrcv_sizes, c_msgrcv_copy.
    let mut k = fresh();
    let id = msgget(&mut k);
    send(&mut k, id, 2, b"0123456789", 0).unwrap();
    send(&mut k, id, 3, b"lost", 0).unwrap();
    send(&mut k, id, 4, b"kept", 0).unwrap();
    assert_eq!(recv(&mut k, u64::MAX, 0, 10, 0, IPC_NOWAIT), Err(Errno::EINVAL));
    assert_eq!(recv(&mut k, id, BUF, 1 << 63, 0, IPC_NOWAIT), Err(Errno::EINVAL), "negative size");
    assert_eq!(recv(&mut k, 0x7fff_fff0, BUF, 10, 0, MSG_COPY), Err(Errno::EINVAL), "MSG_COPY needs IPC_NOWAIT");
    assert_eq!(recv(&mut k, id, BUF, 9, 0, IPC_NOWAIT), Err(Errno::E2BIG));
    k.poke(BUF, &[b'#'; 24]);
    assert_eq!(recv(&mut k, id, BUF, 10, 1, MSG_COPY | IPC_NOWAIT), Ok(4), "copy of position 1");
    assert_eq!(recv(&mut k, id, BUF, 4, 0, IPC_NOWAIT | MSG_NOERROR), Ok(4));
    assert_eq!(k.peek_u64(BUF), 2, "mtype");
    assert_eq!(k.peek(BUF + 8, 5), b"0123#", "text cut, nothing written past it");
    assert_eq!(recv(&mut k, id, 0, 10, 0, IPC_NOWAIT), Err(Errno::EFAULT), "the message is lost");
    assert_eq!(recv(&mut k, id, 0, 10, 99, IPC_NOWAIT), Err(Errno::ENOMSG));
    assert_eq!(recv(&mut k, id, BUF, 10, 0, IPC_NOWAIT), Ok(4));
    assert_eq!(k.peek_u64(BUF), 4);
    assert_eq!(msgctl(&mut k, id, IPC_STAT, BUF), Ok(0));
    assert_eq!((k.peek_u64(BUF + 80), k.peek_u64(BUF + 100) as u32), (0, 100), "empty, msg_lrpid");
}

#[test]
fn control_commands() {
    // c_msgctl_stat_set, c_msgctl_owner_perms; spec: msgctl order_set, ipc_info.
    let mut k = fresh();
    let id = msgget(&mut k);
    let index = crate::ipc::index_of(id as i32) as u64;
    assert_eq!(msgctl(&mut k, u64::MAX, IPC_INFO, BUF), Err(Errno::EINVAL));
    assert_eq!(msgctl(&mut k, 0, u32::MAX, BUF), Err(Errno::EINVAL), "negative command");
    assert_eq!(msgctl(&mut k, id, IPC_STAT | 0x100, BUF), Err(Errno::EINVAL));
    assert_eq!(msgctl(&mut k, 0x7fff_fff0, IPC_SET, 0), Err(Errno::EFAULT), "buffer before the id");
    assert_eq!(msgctl(&mut k, 0, IPC_INFO, BUF), Ok(index));
    assert_eq!(k.peek_u64(BUF + 8) as u32, MSGMAX as u32);
    send(&mut k, id, 1, b"abc", 0).unwrap();
    assert_eq!(msgctl(&mut k, 0, MSG_INFO, BUF), Ok(index));
    assert_eq!((k.peek_u64(BUF) as u32, (k.peek_u64(BUF) >> 32) as u32), (1, 1), "queues, messages");
    assert_eq!(msgctl(&mut k, index, MSG_STAT, BUF), Ok(id));
    assert_eq!(msgctl(&mut k, 32767, MSG_STAT_ANY, BUF), Err(Errno::EINVAL));

    // IPC_SET: qbytes above MSGMNB needs CAP_SYS_RESOURCE, before the ids.
    msgctl(&mut k, id, IPC_STAT, BUF).unwrap();
    k.poke(BUF + 88, &(MSGMNB + 1).to_ne_bytes());
    k.poke(BUF + 4, &u32::MAX.to_ne_bytes());
    assert_eq!(msgctl(&mut k, id, IPC_SET, BUF), Err(Errno::EPERM));
    k.poke(BUF + 88, &2u64.to_ne_bytes());
    assert_eq!(msgctl(&mut k, id, IPC_SET, BUF), Err(Errno::EINVAL), "uid -1");
    k.poke(BUF + 4, &1001u32.to_ne_bytes());
    k.poke(BUF + 20, &0o177640u16.to_ne_bytes());
    assert_eq!(msgctl(&mut k, id, IPC_SET, BUF), Ok(0));
    msgctl(&mut k, id, IPC_STAT, BUF).unwrap();
    assert_eq!((k.peek_u64(BUF + 20) as u16, k.peek_u64(BUF + 88)), (0o640, 2));
    assert_eq!(msgctl(&mut k, id, IPC_RMID, 1), Ok(0), "buffer ignored");
    assert_eq!(msgctl(&mut k, id, IPC_STAT, BUF), Err(Errno::EINVAL));
}
