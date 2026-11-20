//! Tests of the mach_port routines, task ports and the bootstrap server
//! against `specs/macos/{libsystem_kernel.dylib,liblaunch.dylib,mach}`.

use crate::consts::{bootstrap::*, kern::*, msg::*, notify::*, port::*, special::*};
use crate::ipc::*;
use alloc::vec;
use alloc::vec::Vec;

const T: TaskId = 1;
const U: TaskId = 2;

fn ipc() -> Ipc {
    let mut i = Ipc::new();
    i.create_task(T, 100);
    i.create_task(U, 200);
    i
}

fn recv(i: &mut Ipc, t: TaskId) -> u32 {
    i.port_allocate(t, MACH_PORT_RIGHT_RECEIVE).unwrap()
}

fn rd(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn header(bits: u32, remote: u32, local: u32, id: u32, body: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; 24];
    m.extend_from_slice(body);
    let len = m.len() as u32;
    m[0..4].copy_from_slice(&bits.to_le_bytes());
    m[4..8].copy_from_slice(&len.to_le_bytes());
    m[8..12].copy_from_slice(&remote.to_le_bytes());
    m[12..16].copy_from_slice(&local.to_le_bytes());
    m[20..24].copy_from_slice(&id.to_le_bytes());
    m
}

fn send_empty(i: &mut Ipc, t: TaskId, dst: u32, id: u32) -> Result<(), SendError> {
    i.send(t, &header(MACH_MSG_TYPE_COPY_SEND, dst, 0, id, &[]), SenderInfo::default()).map(|_| ())
}

/// The next message on `name` in `t`, or `None`.
fn take(i: &mut Ipc, t: TaskId, name: u32) -> Option<Vec<u8>> {
    match i.receive(t, name, 4096, 0).unwrap().0 {
        Received::Message(m) => Some(m),
        _ => None,
    }
}

/// A request to a kernel object; the reply, received on a reply port.
fn rpc(i: &mut Ipc, t: TaskId, msg: Vec<u8>) -> Vec<u8> {
    let reply = i.reply_port(t);
    let mut msg = msg;
    let bits = rd(&msg, 0) & !0x1f00 | (MACH_MSG_TYPE_MAKE_SEND_ONCE << 8);
    msg[0..4].copy_from_slice(&bits.to_le_bytes());
    msg[12..16].copy_from_slice(&reply.to_le_bytes());
    i.send(t, &msg, SenderInfo::default()).unwrap();
    let out = take(i, t, reply).expect("a reply");
    i.port_mod_refs(t, reply, MACH_PORT_RIGHT_RECEIVE, -1);
    out
}

#[test]
fn construct_options() {
    let mut i = ipc();
    let a = i.port_construct(T, Options::default(), 0).unwrap();
    let b = i.port_construct(T, Options::default(), 0).unwrap();
    assert_ne!(a, b);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE));
    let st = i.port_get_attributes(T, a, MACH_PORT_RECEIVE_STATUS, 10).unwrap();
    assert_eq!(st, vec![0, 0, 0, 5, 0, 0, 0, 0, 0, 0]);
    let s = i.port_construct(T, Options { flags: MPO_INSERT_SEND_RIGHT, qlimit: 0 }, 0).unwrap();
    assert_eq!(i.port_type(T, s), Ok(MACH_PORT_TYPE_SEND | MACH_PORT_TYPE_RECEIVE));
    let q = i.port_construct(T, Options { flags: MPO_QLIMIT, qlimit: 0 }, 0).unwrap();
    assert_eq!(i.port_get_attributes(T, q, MACH_PORT_LIMITS_INFO, 1), Ok(vec![0]));
    assert_eq!(i.port_construct(T, Options { flags: MPO_QLIMIT, qlimit: 1025 }, 0), Err(KERN_INVALID_VALUE));
    let g = i.port_construct(T, Options { flags: MPO_CONTEXT_AS_GUARD, qlimit: 0 }, 0x55).unwrap();
    assert_eq!(i.port_get_context(T, g), Ok(0x55));
    let st = i.port_construct(T, Options { flags: MPO_CONTEXT_AS_GUARD | MPO_STRICT, qlimit: 0 }, 0x55).unwrap();
    assert_eq!(i.port_get_context(T, st), Ok(0), "a strict guard reads as 0");
    assert_eq!(i.port_construct(99, Options::default(), 0), Err(MACH_SEND_INVALID_DEST));
}

#[test]
fn destruct_and_guards() {
    let mut i = ipc();
    let p = i.port_construct(T, Options { flags: MPO_INSERT_SEND_RIGHT, qlimit: 0 }, 0).unwrap();
    assert_eq!(i.port_destruct(T, p, 1, 0).0, KERN_INVALID_VALUE);
    assert_eq!(i.port_destruct(T, p, -2, 0).0, KERN_INVALID_VALUE);
    assert_eq!(i.port_destruct(T, p, 0, 0).0, KERN_SUCCESS);
    assert_eq!(i.port_type(T, p), Ok(MACH_PORT_TYPE_DEAD_NAME), "the send right is a dead name");
    let p = i.port_construct(T, Options { flags: MPO_INSERT_SEND_RIGHT, qlimit: 0 }, 0).unwrap();
    assert_eq!(i.port_destruct(T, p, -1, 0).0, KERN_SUCCESS);
    assert_eq!(i.port_type(T, p), Err(KERN_INVALID_NAME), "nothing is left");
    let r = recv(&mut i, T);
    assert_eq!(i.port_destruct(T, r, -1, 0).0, KERN_INVALID_RIGHT);
    assert_eq!(i.port_destruct(T, 0x1234, 0, 0).0, KERN_INVALID_NAME);
    // Guards.
    assert_eq!(i.port_guard(T, r, 7, false), KERN_SUCCESS);
    assert_eq!(i.port_guard(T, r, 8, false), KERN_INVALID_ARGUMENT, "already guarded");
    assert_eq!(i.port_destruct(T, r, 0, 1).0, POLY_GUARD_VIOLATION);
    assert_eq!(i.port_mod_refs(T, r, MACH_PORT_RIGHT_RECEIVE, -1).0, POLY_GUARD_VIOLATION);
    assert_eq!(i.port_destroy(T, r).0, POLY_GUARD_VIOLATION);
    assert_eq!(i.port_unguard(T, r, 6), POLY_GUARD_VIOLATION);
    assert_eq!(i.port_unguard(T, r, 7), KERN_SUCCESS);
    assert_eq!(i.port_get_context(T, r), Ok(0));
    assert_eq!(i.port_unguard(T, r, 0), POLY_GUARD_VIOLATION, "not guarded");
    assert_eq!(i.port_guard(T, r, 0, false), KERN_SUCCESS, "guard 0 guards nothing");
    assert_eq!(i.port_set_context(T, r, 3), KERN_SUCCESS);
    assert_eq!(i.port_guard(T, r, 9, false), KERN_INVALID_ARGUMENT, "a context is set");
    // set_context replaces a guard.
    let g = recv(&mut i, T);
    i.port_guard(T, g, 5, false);
    i.port_set_context(T, g, 6);
    assert_eq!(i.port_destruct(T, g, 0, 6).0, KERN_SUCCESS);
    let s = i.task_self(T);
    assert_eq!(i.port_guard(T, s, 1, false), KERN_INVALID_RIGHT);
}

#[test]
fn attributes_and_queue_limits() {
    let mut i = ipc();
    let p = recv(&mut i, T);
    i.port_insert_right(T, T, p, p, MACH_MSG_TYPE_MAKE_SEND);
    assert_eq!(i.port_get_attributes(T, p, 999, 10), Err(KERN_INVALID_ARGUMENT));
    assert_eq!(i.port_get_attributes(T, p, MACH_PORT_RECEIVE_STATUS, 5), Err(KERN_FAILURE));
    assert_eq!(i.port_get_attributes(T, p, MACH_PORT_LIMITS_INFO, 0), Err(KERN_FAILURE));
    assert_eq!(i.port_set_attributes(T, p, MACH_PORT_LIMITS_INFO, &[1025]).0, KERN_INVALID_VALUE);
    assert_eq!(i.port_set_attributes(T, p, MACH_PORT_LIMITS_INFO, &[]).0, KERN_FAILURE);
    assert_eq!(i.port_set_attributes(T, p, MACH_PORT_RECEIVE_STATUS, &[1]).0, KERN_INVALID_ARGUMENT);
    assert_eq!(i.port_set_attributes(T, p, MACH_PORT_LIMITS_INFO, &[1]).0, KERN_SUCCESS);
    send_empty(&mut i, T, p, 1).unwrap();
    assert!(matches!(send_empty(&mut i, T, p, 2), Err(SendError::Full(_))));
    i.port_set_attributes(T, p, MACH_PORT_LIMITS_INFO, &[3]);
    send_empty(&mut i, T, p, 2).unwrap();
    let st = i.port_get_attributes(T, p, MACH_PORT_RECEIVE_STATUS, 10).unwrap();
    assert_eq!((st[2], st[3], st[4], st[6]), (1, 3, 2, 1), "mscount, qlimit, msgcount, srights");
    take(&mut i, T, p).unwrap();
    assert_eq!(i.port_get_attributes(T, p, MACH_PORT_RECEIVE_STATUS, 10).unwrap()[1], 1, "seqno");
    let s = i.task_self(T);
    assert_eq!(i.port_get_attributes(T, s, MACH_PORT_LIMITS_INFO, 1), Err(KERN_INVALID_RIGHT));
    assert_eq!(i.port_get_attributes(T, 0x4321, MACH_PORT_LIMITS_INFO, 1), Err(KERN_INVALID_NAME));
}

#[test]
fn a_port_in_two_sets() {
    let mut i = ipc();
    let p = recv(&mut i, T);
    i.port_insert_right(T, T, p, p, MACH_MSG_TYPE_MAKE_SEND);
    let a = i.port_allocate(T, MACH_PORT_RIGHT_PORT_SET).unwrap();
    let b = i.port_allocate(T, MACH_PORT_RIGHT_PORT_SET).unwrap();
    assert_eq!(i.port_insert_member(T, p, a), KERN_SUCCESS);
    assert_eq!(i.port_insert_member(T, p, a), KERN_ALREADY_IN_SET);
    assert_eq!(i.port_insert_member(T, p, b), KERN_SUCCESS);
    assert_eq!(i.port_set_members(T, a), Ok(vec![p]));
    assert_eq!(i.port_set_members(T, b), Ok(vec![p]));
    assert_eq!(i.port_get_attributes(T, p, MACH_PORT_RECEIVE_STATUS, 10).unwrap()[0], 1);
    send_empty(&mut i, T, p, 9).unwrap();
    let m = take(&mut i, T, b).unwrap();
    assert_eq!(rd(&m, 12), p, "the member's name is the local port");
    assert_eq!(i.port_extract_member(T, p, a), KERN_SUCCESS);
    assert_eq!(i.port_extract_member(T, p, a), KERN_NOT_IN_SET);
    assert_eq!(i.port_set_members(T, a), Ok(vec![]));
    assert_eq!(i.port_insert_member(T, a, b), KERN_INVALID_RIGHT, "a set is no member");
    assert_eq!(i.port_insert_member(T, p, p), KERN_INVALID_RIGHT, "a port is no set");
    assert_eq!(i.port_insert_member(T, 0, b), KERN_INVALID_RIGHT);
    assert_eq!(i.port_insert_member(T, 0x4321, b), KERN_INVALID_NAME);
    i.port_mod_refs(T, p, MACH_PORT_RIGHT_RECEIVE, -1);
    assert_eq!(i.port_set_members(T, b), Ok(vec![]), "a destroyed member leaves");
}

#[test]
fn peek_and_extract_right() {
    let mut i = ipc();
    let p = recv(&mut i, T);
    i.port_insert_right(T, T, p, p, MACH_MSG_TYPE_MAKE_SEND);
    assert_eq!(i.port_peek(T, p, 0, 0, 8), Err(KERN_FAILURE), "empty");
    send_empty(&mut i, T, p, 10).unwrap();
    i.send(T, &header(MACH_MSG_TYPE_COPY_SEND, p, 0, 11, &[0; 40]), SenderInfo::default()).unwrap();
    take(&mut i, T, p).unwrap();
    assert_eq!(i.port_peek(T, p, 0, 0, 8), Ok(Peeked { seqno: 1, size: 72, id: 11 }));
    assert_eq!(i.port_peek(T, p, 0, 1, 8), Ok(Peeked { seqno: 1, size: 72, id: 11 }));
    assert_eq!(i.port_peek(T, p, 0, 5, 8), Err(KERN_FAILURE));
    assert_eq!(i.port_peek(T, p, 0, 0, 0), Err(KERN_INVALID_VALUE));
    assert_eq!(i.port_peek(T, p, 1, 0, 8), Err(KERN_INVALID_VALUE));
    // extract_right
    let urefs = |i: &Ipc| i.port_get_refs(T, p, MACH_PORT_RIGHT_SEND).unwrap();
    assert_eq!(i.port_extract_right(T, p, MACH_MSG_TYPE_MAKE_SEND), Ok((p, MACH_MSG_TYPE_PORT_SEND)));
    assert_eq!(urefs(&i), 2);
    assert_eq!(i.port_extract_right(T, p, MACH_MSG_TYPE_COPY_SEND), Ok((p, MACH_MSG_TYPE_PORT_SEND)));
    assert_eq!(urefs(&i), 3);
    assert_eq!(i.port_extract_right(T, p, MACH_MSG_TYPE_MOVE_SEND), Ok((p, MACH_MSG_TYPE_PORT_SEND)));
    assert_eq!(urefs(&i), 3);
    let (so, t) = i.port_extract_right(T, p, MACH_MSG_TYPE_MAKE_SEND_ONCE).unwrap();
    assert_eq!((t, i.port_type(T, so)), (MACH_MSG_TYPE_PORT_SEND_ONCE, Ok(MACH_PORT_TYPE_SEND_ONCE)));
    let (so2, _) = i.port_extract_right(T, so, MACH_MSG_TYPE_MOVE_SEND_ONCE).unwrap();
    assert_ne!(so2, so);
    assert_eq!(i.port_type(T, so), Err(KERN_INVALID_NAME));
    assert_eq!(i.port_extract_right(T, p, MACH_MSG_TYPE_MOVE_RECEIVE), Ok((p, MACH_MSG_TYPE_PORT_RECEIVE)));
    assert_eq!(i.port_extract_right(T, p, 99), Err(KERN_INVALID_VALUE));
    assert_eq!(i.port_extract_right(T, p, 22), Err(KERN_INVALID_VALUE));
    assert_eq!(i.port_extract_right(T, 0, MACH_MSG_TYPE_COPY_SEND), Err(KERN_INVALID_RIGHT));
    assert_eq!(i.port_extract_right(T, 0x4321, MACH_MSG_TYPE_COPY_SEND), Err(KERN_INVALID_NAME));
    let s = i.task_self(T);
    assert_eq!(i.port_extract_right(T, s, MACH_MSG_TYPE_MAKE_SEND), Err(KERN_INVALID_RIGHT));
}

#[test]
fn no_senders_notifications() {
    let mut i = ipc();
    let n = recv(&mut i, T);
    // No senders and a make-send count of at least sync: at once.
    let p = recv(&mut i, T);
    assert_eq!(i.port_request_notification(T, p, MACH_NOTIFY_NO_SENDERS, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE), Ok(0));
    let m = take(&mut i, T, n).expect("at once");
    assert_eq!((m.len() - 8, rd(&m, 20), rd(&m, 32)), (36, MACH_NOTIFY_NO_SENDERS, 0));
    // With a send right outstanding: when it goes.
    let q = recv(&mut i, T);
    i.port_insert_right(T, T, q, q, MACH_MSG_TYPE_MAKE_SEND);
    i.port_request_notification(T, q, MACH_NOTIFY_NO_SENDERS, 5, n, MACH_MSG_TYPE_MAKE_SEND_ONCE).unwrap();
    i.take_woken();
    assert!(take(&mut i, T, n).is_none());
    assert_eq!(i.port_get_attributes(T, q, MACH_PORT_RECEIVE_STATUS, 10).unwrap()[8], 1, "nsrequest");
    i.port_mod_refs(T, q, MACH_PORT_RIGHT_SEND, -1);
    i.take_woken();
    let m = take(&mut i, T, n).expect("when the last send right went");
    assert_eq!((rd(&m, 20), rd(&m, 32)), (MACH_NOTIFY_NO_SENDERS, 1));
    assert_eq!(i.port_get_attributes(T, q, MACH_PORT_RECEIVE_STATUS, 10).unwrap()[8], 0);
    // A second request returns the first one's right.
    let r = recv(&mut i, T);
    i.port_insert_right(T, T, r, r, MACH_MSG_TYPE_MAKE_SEND);
    assert_eq!(i.port_request_notification(T, r, MACH_NOTIFY_NO_SENDERS, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE), Ok(0));
    let prev = i.port_request_notification(T, r, MACH_NOTIFY_NO_SENDERS, 0, 0, 0).unwrap();
    assert_eq!(i.port_type(T, prev), Ok(MACH_PORT_TYPE_SEND_ONCE));
    // Refusals.
    let s = i.task_self(T);
    assert_eq!(
        i.port_request_notification(T, s, MACH_NOTIFY_NO_SENDERS, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE),
        Err(KERN_INVALID_RIGHT)
    );
    assert_eq!(i.port_request_notification(T, r, 999, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE), Err(KERN_INVALID_VALUE));
    assert_eq!(
        i.port_request_notification(T, r, MACH_NOTIFY_SEND_ONCE, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE),
        Err(KERN_INVALID_VALUE)
    );
    assert_eq!(
        i.port_request_notification(T, r, MACH_NOTIFY_NO_SENDERS, 0, n, MACH_MSG_TYPE_MAKE_SEND),
        Err(MIG_TYPE_ERROR)
    );
    assert_eq!(
        i.port_request_notification(T, 0x4321, MACH_NOTIFY_NO_SENDERS, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE),
        Err(KERN_INVALID_NAME)
    );
}

#[test]
fn dead_name_and_port_destroyed_notifications() {
    let mut i = ipc();
    let n = recv(&mut i, T);
    let p = recv(&mut i, T);
    i.port_insert_right(T, T, p, p, MACH_MSG_TYPE_MAKE_SEND);
    i.port_request_notification(T, p, MACH_NOTIFY_DEAD_NAME, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE).unwrap();
    i.port_mod_refs(T, p, MACH_PORT_RIGHT_RECEIVE, -1);
    let m = take(&mut i, T, n).expect("dead name");
    assert_eq!((rd(&m, 20), rd(&m, 32)), (MACH_NOTIFY_DEAD_NAME, p));
    assert_eq!(i.port_type(T, p), Ok(MACH_PORT_TYPE_DEAD_NAME));
    assert_eq!(i.port_get_refs(T, p, MACH_PORT_RIGHT_DEAD_NAME), Ok(2));
    // On a dead name: KERN_INVALID_ARGUMENT and a send-once notification.
    assert_eq!(
        i.port_request_notification(T, p, MACH_NOTIFY_DEAD_NAME, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE),
        Err(KERN_INVALID_ARGUMENT)
    );
    let m = take(&mut i, T, n).unwrap();
    assert_eq!((m.len() - 8, rd(&m, 20)), (24, MACH_NOTIFY_SEND_ONCE));
    // A receive-only name: port-deleted when it goes.
    let r = recv(&mut i, T);
    i.port_request_notification(T, r, MACH_NOTIFY_DEAD_NAME, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE).unwrap();
    i.port_mod_refs(T, r, MACH_PORT_RIGHT_RECEIVE, -1);
    let m = take(&mut i, T, n).unwrap();
    assert_eq!((rd(&m, 20), rd(&m, 32)), (MACH_NOTIFY_PORT_DELETED, r));
    // Port destroyed: the receive right comes back in a message.
    let d = recv(&mut i, T);
    assert_eq!(
        i.port_request_notification(T, d, MACH_NOTIFY_PORT_DESTROYED, 1, n, MACH_MSG_TYPE_MAKE_SEND_ONCE),
        Err(KERN_INVALID_VALUE)
    );
    i.port_request_notification(T, d, MACH_NOTIFY_PORT_DESTROYED, 0, n, MACH_MSG_TYPE_MAKE_SEND_ONCE).unwrap();
    assert_eq!(i.port_get_attributes(T, d, MACH_PORT_RECEIVE_STATUS, 10).unwrap()[7], 1, "pdrequest");
    i.port_mod_refs(T, d, MACH_PORT_RIGHT_RECEIVE, -1);
    let m = take(&mut i, T, n).unwrap();
    assert_eq!((m.len() - 8, rd(&m, 20), rd(&m, 24)), (40, MACH_NOTIFY_PORT_DESTROYED, 1));
    let back = rd(&m, 28);
    assert_eq!(i.port_type(T, back), Ok(MACH_PORT_TYPE_RECEIVE), "the port lives on");
}

#[test]
fn task_name_ports_and_pid_for_task() {
    let mut i = ipc();
    let n = i.task_name_for(T, U);
    assert_eq!(i.port_type(T, n), Ok(MACH_PORT_TYPE_SEND));
    assert_eq!(i.task_name_for(T, U), n, "the same name");
    assert_ne!(i.task_name_for(T, T), i.task_self(T), "not the task port");
    assert_eq!(i.pid_for_task(T, n), Ok(U));
    let s = i.task_self(T);
    assert_eq!(i.pid_for_task(T, s), Ok(T));
    let th = i.thread_self(T, 100);
    let h = i.host_self(T);
    let r = recv(&mut i, T);
    for bad in [0, 0x1234, MACH_PORT_DEAD, th, h, r] {
        assert_eq!(i.pid_for_task(T, bad), Err(KERN_FAILURE));
    }
    i.destroy_task(U);
    assert_eq!(i.port_type(T, n), Ok(MACH_PORT_TYPE_DEAD_NAME), "the name port died with the task");
    assert_eq!(i.pid_for_task(T, n), Err(KERN_FAILURE));
    // A task that never made a Mach call has a name port too, and loses it.
    let m = i.task_name_for(T, 7);
    assert_eq!(i.pid_for_task(T, m), Ok(7));
    i.destroy_task(7);
    assert_eq!(i.pid_for_task(T, m), Err(KERN_FAILURE));
}

#[test]
fn special_ports_by_message() {
    let mut i = ipc();
    let task = i.task_self(T);
    let mut b = crate::ipc::mig_ndr();
    b.extend_from_slice(&TASK_BOOTSTRAP_PORT.to_le_bytes());
    let get = header(MACH_MSG_TYPE_COPY_SEND, task, 0, 3409, &b);
    let out = rpc(&mut i, T, get.clone());
    let bp = rd(&out, 28);
    assert_eq!(i.port_type(T, bp), Ok(MACH_PORT_TYPE_SEND));
    let out = rpc(&mut i, T, get);
    assert_eq!(rd(&out, 28), bp, "the same name");
    assert_eq!(i.port_get_refs(T, bp, MACH_PORT_RIGHT_SEND), Ok(2), "one more reference each time");
    assert_eq!(i.port_extract_right(T, bp, MACH_MSG_TYPE_MAKE_SEND), Err(KERN_INVALID_RIGHT));
    assert_eq!(i.port_guard(T, bp, 1, false), KERN_INVALID_RIGHT);
    assert_eq!(i.special_port(T, 0), Err(KERN_INVALID_ARGUMENT));
    // A child inherits a port its parent set.
    let p = recv(&mut i, T);
    i.port_insert_right(T, T, p, p, MACH_MSG_TYPE_MAKE_SEND);
    assert_eq!(i.set_special_port(T, 9, Some(port_of(&i, T, p))), KERN_SUCCESS);
    i.create_task(3, 300);
    i.inherit_from(T, 3);
    assert_eq!(i.special_port(3, 9), Ok(Some(port_of(&i, T, p))));
}

fn port_of(i: &Ipc, t: TaskId, name: u32) -> PortId {
    match i.receive_key(t, name).unwrap() {
        RcvKey::Port(p) => p,
        RcvKey::Set(_) => unreachable!(),
    }
}

/// The bootstrap port's name in `t`.
fn bootstrap(i: &mut Ipc, t: TaskId) -> u32 {
    let task = i.task_self(t);
    let mut b = crate::ipc::mig_ndr();
    b.extend_from_slice(&TASK_BOOTSTRAP_PORT.to_le_bytes());
    let out = rpc(i, t, header(MACH_MSG_TYPE_COPY_SEND, task, 0, 3409, &b));
    rd(&out, 28)
}

/// A bootstrap request: (result, the port name in the reply).
fn boot(i: &mut Ipc, t: TaskId, id: u32, name: &[u8], register: Option<(u32, u32)>) -> (u32, u32) {
    let bp = bootstrap(i, t);
    let out = rpc(i, t, bootstrap_request(id, bp, 0, name, register));
    if id == BOOTSTRAP_REGISTER { (rd(&out, 32), 0) } else { (rd(&out, 48), rd(&out, 28)) }
}

#[test]
fn bootstrap_check_in_look_up_register() {
    let mut i = ipc();
    let (kr, p) = boot(&mut i, T, BOOTSTRAP_CHECK_IN, b"com.poly.a", None);
    assert_eq!(kr, KERN_SUCCESS);
    assert_eq!(i.port_type(T, p), Ok(MACH_PORT_TYPE_RECEIVE));
    assert_eq!(i.port_get_attributes(T, p, MACH_PORT_LIMITS_INFO, 1), Ok(vec![2]));
    assert_eq!(boot(&mut i, T, BOOTSTRAP_CHECK_IN, b"com.poly.a", None).0, BOOTSTRAP_SERVICE_ACTIVE);
    assert_eq!(boot(&mut i, U, BOOTSTRAP_CHECK_IN, b"com.poly.a", None).0, BOOTSTRAP_NOT_PRIVILEGED);
    // Look-ups: the holder gets its own name with a send right added.
    let (kr, own) = boot(&mut i, T, BOOTSTRAP_LOOK_UP, b"com.poly.a", None);
    assert_eq!((kr, own), (KERN_SUCCESS, p));
    assert_eq!(i.port_type(T, p), Ok(MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND));
    let (kr, other) = boot(&mut i, U, BOOTSTRAP_LOOK_UP, b"com.poly.a", None);
    assert_eq!((kr, i.port_type(U, other)), (KERN_SUCCESS, Ok(MACH_PORT_TYPE_SEND)));
    send_empty(&mut i, U, other, 5).unwrap();
    assert_eq!(rd(&take(&mut i, T, p).unwrap(), 20), 5, "a message arrives on the holder's right");
    assert_eq!(boot(&mut i, T, BOOTSTRAP_LOOK_UP, b"com.poly.none", None).0, BOOTSTRAP_UNKNOWN_SERVICE);
    // Long names are compared whole; the empty name is a name.
    let long_a = [b'a'; 199];
    let mut long_b = long_a;
    long_b[150] = b'b';
    assert_eq!(boot(&mut i, T, BOOTSTRAP_CHECK_IN, &long_a, None).0, KERN_SUCCESS);
    assert_eq!(boot(&mut i, T, BOOTSTRAP_LOOK_UP, &long_b, None).0, BOOTSTRAP_UNKNOWN_SERVICE);
    assert_eq!(boot(&mut i, T, BOOTSTRAP_CHECK_IN, b"", None).0, KERN_SUCCESS);
    // The receive right destroyed: the name is free again.
    i.port_mod_refs(T, p, MACH_PORT_RIGHT_RECEIVE, -1);
    assert_eq!(boot(&mut i, U, BOOTSTRAP_LOOK_UP, b"com.poly.a", None).0, BOOTSTRAP_UNKNOWN_SERVICE);
    assert_eq!(boot(&mut i, T, BOOTSTRAP_CHECK_IN, b"com.poly.a", None).0, KERN_SUCCESS);
    // Register a receive right (a send right made for the server).
    let r = recv(&mut i, U);
    assert_eq!(boot(&mut i, U, BOOTSTRAP_REGISTER, b"com.poly.r", Some((r, MACH_MSG_TYPE_MAKE_SEND))).0, KERN_SUCCESS);
    assert_eq!(
        boot(&mut i, U, BOOTSTRAP_REGISTER, b"com.poly.r", Some((r, MACH_MSG_TYPE_MAKE_SEND))).0,
        BOOTSTRAP_NAME_IN_USE
    );
    let s = recv(&mut i, T);
    assert_eq!(
        boot(&mut i, T, BOOTSTRAP_REGISTER, b"com.poly.r", Some((s, MACH_MSG_TYPE_MAKE_SEND))).0,
        BOOTSTRAP_NOT_PRIVILEGED
    );
    let (kr, found) = boot(&mut i, T, BOOTSTRAP_LOOK_UP, b"com.poly.r", None);
    assert_eq!(kr, KERN_SUCCESS);
    send_empty(&mut i, T, found, 6).unwrap();
    assert_eq!(rd(&take(&mut i, U, r).unwrap(), 20), 6);
    // The owner exits: its names are unknown.
    i.destroy_task(U);
    assert_eq!(boot(&mut i, T, BOOTSTRAP_LOOK_UP, b"com.poly.r", None).0, BOOTSTRAP_UNKNOWN_SERVICE);
}

#[test]
fn mach_port_requests_by_message() {
    let mut i = ipc();
    let task = i.task_self(T);
    let p = recv(&mut i, T);
    let mut b = crate::ipc::mig_ndr();
    for w in [p, MACH_PORT_LIMITS_INFO, 1, 7] {
        b.extend_from_slice(&w.to_le_bytes());
    }
    let out = rpc(&mut i, T, header(MACH_MSG_TYPE_COPY_SEND, task, 0, 3218, &b));
    assert_eq!(rd(&out, 32), KERN_SUCCESS);
    let mut b = crate::ipc::mig_ndr();
    for w in [p, MACH_PORT_RECEIVE_STATUS, 10] {
        b.extend_from_slice(&w.to_le_bytes());
    }
    let out = rpc(&mut i, T, header(MACH_MSG_TYPE_COPY_SEND, task, 0, 3217, &b));
    assert_eq!((rd(&out, 32), rd(&out, 36), rd(&out, 40 + 12)), (KERN_SUCCESS, 10, 7));
    // A guard broken through a request marks the caller.
    i.port_guard(T, p, 9, false);
    let mut b = crate::ipc::mig_ndr();
    b.extend_from_slice(&p.to_le_bytes());
    let out = rpc(&mut i, T, header(MACH_MSG_TYPE_COPY_SEND, task, 0, 3205, &b));
    assert_eq!(rd(&out, 32), POLY_GUARD_VIOLATION);
    assert!(i.take_violation(T));
    assert!(!i.take_violation(T));
}
