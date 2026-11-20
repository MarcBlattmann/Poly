//! Tests of the IPC model against the trap spec sheets
//! (`specs/macos/mach/*.yaml`) and the public Mach semantics.

use crate::consts::{kern::*, msg::*, port::*};
use crate::ipc::*;
use alloc::vec;
use alloc::vec::Vec;

const T: TaskId = 1;
const TH: u64 = 100;

fn ipc() -> Ipc {
    let mut i = Ipc::new();
    i.create_task(T, TH);
    i
}

fn recv(i: &mut Ipc) -> u32 {
    i.port_allocate(T, MACH_PORT_RIGHT_RECEIVE).unwrap()
}

fn msg(bits: u32, remote: u32, local: u32, id: u32, body: &[u8]) -> Vec<u8> {
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

fn rd(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn self_ports_are_send_rights_with_distinct_stable_names() {
    let mut i = ipc();
    let (t, th, h) = (i.task_self(T), i.thread_self(T, TH), i.host_self(T));
    assert!(t != 0 && th != 0 && h != 0 && t != th && th != h && t != h);
    assert!(t != MACH_PORT_DEAD);
    assert_eq!(i.task_self(T), t);
    for n in [t, th, h] {
        assert_eq!(i.port_type(T, n), Ok(MACH_PORT_TYPE_SEND));
    }
    assert_eq!(i.resolve_task(T, t), Ok(T));
    assert_eq!(i.resolve_task(T, 12345), Err(MACH_SEND_INVALID_DEST));
    assert_eq!(i.resolve_task(T, 0), Err(MACH_SEND_INVALID_DEST));
    assert_eq!(i.resolve_task(T, th), Err(MACH_SEND_INVALID_DEST));
    // A deallocate gives one of the references back; the name stays.
    assert_eq!(i.port_deallocate(T, t), KERN_SUCCESS);
    assert_eq!(i.port_type(T, t), Ok(MACH_PORT_TYPE_SEND));
}

#[test]
fn a_forked_child_has_its_own_names() {
    let mut i = ipc();
    i.create_task(2, 200);
    assert_ne!(i.task_self(2), i.task_self(T));
    assert_ne!(i.thread_self(2, 200), i.thread_self(T, TH));
    let h = i.host_self(2);
    assert_eq!(i.port_type(2, h), Ok(MACH_PORT_TYPE_SEND));
    // No rights of the parent.
    let a = recv(&mut i);
    assert_eq!(i.port_type(2, a), Err(KERN_INVALID_NAME));
}

#[test]
fn allocate_rights_and_values() {
    let mut i = ipc();
    let a = recv(&mut i);
    let b = recv(&mut i);
    assert!(a != b && a != 0 && b != 0);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE));
    let s = i.port_allocate(T, MACH_PORT_RIGHT_PORT_SET).unwrap();
    assert_eq!(i.port_type(T, s), Ok(MACH_PORT_TYPE_PORT_SET));
    let d = i.port_allocate(T, MACH_PORT_RIGHT_DEAD_NAME).unwrap();
    assert_eq!(i.port_type(T, d), Ok(MACH_PORT_TYPE_DEAD_NAME));
    for r in [MACH_PORT_RIGHT_SEND, MACH_PORT_RIGHT_SEND_ONCE, 99] {
        assert_eq!(i.port_allocate(T, r), Err(KERN_INVALID_VALUE));
    }
    assert_eq!(i.port_allocate(77, MACH_PORT_RIGHT_RECEIVE), Err(MACH_SEND_INVALID_DEST));
}

#[test]
fn send_rights_count_references() {
    let mut i = ipc();
    let a = recv(&mut i);
    assert_eq!(i.port_insert_right(T, T, a, a, MACH_MSG_TYPE_MAKE_SEND), KERN_SUCCESS);
    assert_eq!(i.port_insert_right(T, T, a, a, MACH_MSG_TYPE_MAKE_SEND), KERN_SUCCESS);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND));
    assert_eq!(i.port_deallocate(T, a), KERN_SUCCESS);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND));
    assert_eq!(i.port_deallocate(T, a), KERN_SUCCESS);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE));
    // A receive right cannot be deallocated.
    assert_eq!(i.port_deallocate(T, a), KERN_INVALID_RIGHT);
    assert_eq!(i.port_deallocate(T, MACH_PORT_NULL), KERN_SUCCESS);
    assert_eq!(i.port_deallocate(T, MACH_PORT_DEAD), KERN_SUCCESS);
    assert_eq!(i.port_deallocate(T, 0x12345), KERN_INVALID_NAME);
}

#[test]
fn mod_refs_rules() {
    let mut i = ipc();
    let a = recv(&mut i);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_SEND, -1).0, KERN_INVALID_RIGHT);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_PORT_SET, -1).0, KERN_INVALID_RIGHT);
    assert_eq!(i.port_mod_refs(T, a, 99, -1).0, KERN_INVALID_VALUE);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_RECEIVE, -2).0, KERN_INVALID_VALUE);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_RECEIVE, 1).0, KERN_INVALID_VALUE);
    i.port_insert_right(T, T, a, a, MACH_MSG_TYPE_MAKE_SEND);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_SEND, 1).0, KERN_SUCCESS);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_SEND, -1).0, KERN_SUCCESS);
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_SEND, -1).0, KERN_SUCCESS);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE));
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_RECEIVE, -1).0, KERN_SUCCESS);
    assert_eq!(i.port_type(T, a), Err(KERN_INVALID_NAME));
    assert_eq!(i.port_mod_refs(T, a, MACH_PORT_RIGHT_RECEIVE, -1).0, KERN_INVALID_NAME);
}

#[test]
fn destroying_a_receive_right_leaves_dead_names() {
    let mut i = ipc();
    let a = recv(&mut i);
    i.port_insert_right(T, T, a, a, MACH_MSG_TYPE_MAKE_SEND);
    // Another task holds a send right too.
    i.create_task(2, 200);
    assert_eq!(i.port_insert_right(T, 2, 0x5003, a, MACH_MSG_TYPE_MAKE_SEND), KERN_SUCCESS);
    assert_eq!(i.port_type(2, 0x5003), Ok(MACH_PORT_TYPE_SEND));
    i.port_mod_refs(T, a, MACH_PORT_RIGHT_RECEIVE, -1);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_DEAD_NAME));
    assert_eq!(i.port_type(2, 0x5003), Ok(MACH_PORT_TYPE_DEAD_NAME));
    assert_eq!(i.port_deallocate(T, a), KERN_SUCCESS);
    assert_eq!(i.port_type(T, a), Err(KERN_INVALID_NAME));
    assert_eq!(i.port_mod_refs(2, 0x5003, MACH_PORT_RIGHT_DEAD_NAME, -1).0, KERN_SUCCESS);
    assert_eq!(i.port_type(2, 0x5003), Err(KERN_INVALID_NAME));
}

#[test]
fn insert_right_errors() {
    let mut i = ipc();
    let a = recv(&mut i);
    let b = recv(&mut i);
    let set = i.port_allocate(T, MACH_PORT_RIGHT_PORT_SET).unwrap();
    assert_eq!(i.port_insert_right(T, T, a, 0x12345, MACH_MSG_TYPE_MAKE_SEND), KERN_INVALID_NAME);
    assert_eq!(i.port_insert_right(T, T, a, a, 99), KERN_INVALID_RIGHT);
    assert_eq!(i.port_insert_right(T, T, set, set, MACH_MSG_TYPE_MAKE_SEND), KERN_INVALID_RIGHT);
    assert_eq!(i.port_insert_right(T, T, b, a, MACH_MSG_TYPE_MAKE_SEND), KERN_RIGHT_EXISTS);
    assert_eq!(i.port_type(T, a), Ok(MACH_PORT_TYPE_RECEIVE));
    // A port has one name for its send and receive rights in a space.
    assert_eq!(i.port_insert_right(T, T, 0x7003, a, MACH_MSG_TYPE_MAKE_SEND), KERN_RIGHT_EXISTS);
}

#[test]
fn move_member_and_sets() {
    let mut i = ipc();
    let a = recv(&mut i);
    let set = i.port_allocate(T, MACH_PORT_RIGHT_PORT_SET).unwrap();
    assert_eq!(i.port_move_member(T, a, set), KERN_SUCCESS);
    assert!(i.receive_key(T, a).is_ok());
    assert!(matches!(i.receive_key(T, set), Ok(RcvKey::Set(_))));
    assert_eq!(i.port_move_member(T, set, set), KERN_INVALID_RIGHT);
    assert_eq!(i.port_move_member(T, 0x12345, set), KERN_INVALID_NAME);
    assert_eq!(i.port_move_member(T, a, MACH_PORT_NULL), KERN_SUCCESS);
    assert_eq!(i.port_move_member(T, a, MACH_PORT_NULL), KERN_NOT_IN_SET);
}

#[test]
fn receive_names() {
    let mut i = ipc();
    let a = recv(&mut i);
    let d = i.port_allocate(T, MACH_PORT_RIGHT_DEAD_NAME).unwrap();
    let t = i.task_self(T);
    for n in [0x12345, MACH_PORT_NULL, MACH_PORT_DEAD, t, d] {
        assert_eq!(i.receive(T, n, 128, 0).err(), Some(MACH_RCV_INVALID_NAME), "{n:#x}");
    }
    assert!(matches!(i.receive(T, a, 128, 0), Ok((Received::Empty(RcvKey::Port(_)), _))));
}

#[test]
fn a_message_carries_a_reply_right() {
    let mut i = ipc();
    let server = recv(&mut i);
    let reply = recv(&mut i);
    // A client in another task holds a send right to the server port.
    i.create_task(2, 200);
    i.port_insert_right(T, 2, 0x9003, server, MACH_MSG_TYPE_MAKE_SEND);
    let creply = i.port_allocate(2, MACH_PORT_RIGHT_RECEIVE).unwrap();
    let m = msg(MACH_MSG_TYPE_COPY_SEND | (MACH_MSG_TYPE_MAKE_SEND_ONCE << 8), 0x9003, creply, 42, b"hello!!\0");
    assert_eq!(i.send(2, &m, SenderInfo { uid: 501, gid: 20, audit: [0; 8] }), Ok(None));
    // Too small a buffer: MACH_RCV_LARGE keeps the message.
    match i.receive(T, server, 32, MACH_RCV_LARGE).unwrap().0 {
        Received::TooLarge { needed, header } => {
            assert_eq!(needed, 32 + 8);
            assert_eq!(rd(&header, 4), 40);
        }
        r => panic!("{r:?}"),
    }
    let (r, _) = i.receive(T, server, 256, 2 << MACH_RCV_TRAILER_ELEMENTS_SHIFT).unwrap();
    let Received::Message(out) = r else { panic!() };
    assert_eq!(out.len(), 32 + 20);
    assert_eq!(rd(&out, 0), MACH_MSG_TYPE_PORT_SEND_ONCE | (MACH_MSG_TYPE_PORT_SEND << 8));
    assert_eq!(rd(&out, 4), 32);
    assert_eq!(rd(&out, 12), server);
    assert_eq!(rd(&out, 20), 42);
    assert_eq!(&out[24..32], b"hello!!\0");
    assert_eq!(rd(&out, 32 + 4), 20);
    assert_eq!(rd(&out, 32 + 12), 501);
    let sonce = rd(&out, 8);
    assert_eq!(i.port_type(T, sonce), Ok(MACH_PORT_TYPE_SEND_ONCE));
    // The reply goes back on the send-once right, which is used up.
    let back = msg(MACH_MSG_TYPE_MOVE_SEND_ONCE, sonce, 0, 43, &[]);
    assert_eq!(i.send(T, &back, SenderInfo::default()), Ok(None));
    assert_eq!(i.port_type(T, sonce), Err(KERN_INVALID_NAME));
    let (r, _) = i.receive(2, creply, 64, 0).unwrap();
    let Received::Message(out) = r else { panic!() };
    assert_eq!(rd(&out, 20), 43);
    assert_eq!(rd(&out, 0) >> 8, MACH_MSG_TYPE_PORT_SEND_ONCE);
    let _ = reply;
}

#[test]
fn port_descriptors_move_receive_rights() {
    let mut i = ipc();
    let dest = recv(&mut i);
    let moved = recv(&mut i);
    let mut body = Vec::new();
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(&moved.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&[0, 0, MACH_MSG_TYPE_MOVE_RECEIVE as u8, MACH_MSG_PORT_DESCRIPTOR]);
    let m = msg(MACH_MSGH_BITS_COMPLEX | MACH_MSG_TYPE_MAKE_SEND, dest, 0, 1, &body);
    assert_eq!(i.send(T, &m, SenderInfo::default()), Ok(None));
    assert_eq!(i.port_type(T, moved), Err(KERN_INVALID_NAME));
    let Received::Message(out) = i.receive(T, dest, 256, 0).unwrap().0 else { panic!() };
    let name = rd(&out, 28);
    assert_eq!(out[38], MACH_MSG_TYPE_PORT_RECEIVE as u8);
    assert_eq!(i.port_type(T, name), Ok(MACH_PORT_TYPE_RECEIVE));
    // A bad descriptor type is refused before anything moves.
    let mut bad = body.clone();
    bad[15] = 1;
    let m = msg(MACH_MSGH_BITS_COMPLEX | MACH_MSG_TYPE_MAKE_SEND, dest, 0, 1, &bad);
    assert_eq!(i.send(T, &m, SenderInfo::default()), Err(SendError::Return(MACH_SEND_INVALID_TYPE)));
}

#[test]
fn send_errors_and_full_queues() {
    let mut i = ipc();
    let p = recv(&mut i);
    assert_eq!(i.send(T, &[0; 20], SenderInfo::default()), Err(SendError::Return(MACH_SEND_MSG_TOO_SMALL)));
    let to = |n: u32| msg(MACH_MSG_TYPE_MAKE_SEND, n, 0, 0, &[]);
    assert_eq!(i.send(T, &to(0x12345), SenderInfo::default()), Err(SendError::Return(MACH_SEND_INVALID_DEST)));
    assert_eq!(
        i.send(T, &msg(5, p, 0, 0, &[]), SenderInfo::default()),
        Err(SendError::Return(MACH_SEND_INVALID_HEADER))
    );
    // The task port answers MIG requests (a request without a reply port
    // gets none); the host port has no server yet.
    let t = i.task_self(T);
    assert_eq!(i.send(T, &msg(MACH_MSG_TYPE_COPY_SEND, t, 0, 0, &[]), SenderInfo::default()), Ok(None));
    let h = i.host_self(T);
    assert!(matches!(
        i.send(T, &msg(MACH_MSG_TYPE_COPY_SEND, h, 0, 0, &[]), SenderInfo::default()),
        Err(SendError::KernelObject(KObject::Host))
    ));
    for _ in 0..MACH_PORT_QLIMIT_DEFAULT {
        assert_eq!(i.send(T, &to(p), SenderInfo::default()), Ok(None));
    }
    assert!(matches!(i.send(T, &to(p), SenderInfo::default()), Err(SendError::Full(_))));
}

#[test]
fn receivers_wake_on_send_and_on_death() {
    let mut i = ipc();
    let p = recv(&mut i);
    let set = i.port_allocate(T, MACH_PORT_RIGHT_PORT_SET).unwrap();
    i.port_move_member(T, p, set);
    let key = i.receive_key(T, set).unwrap();
    i.wait_receive(key, 7);
    assert_eq!(i.send(T, &msg(MACH_MSG_TYPE_MAKE_SEND, p, 0, 0, &[]), SenderInfo::default()), Ok(Some(7)));
    assert_eq!(i.wait_result(7), Some(WakeReason::Ready));
    assert!(matches!(i.receive(T, set, 64, 0).unwrap().0, Received::Message(_)));
    let key = i.receive_key(T, p).unwrap();
    i.wait_receive(key, 8);
    let (kr, woken) = i.port_mod_refs(T, p, MACH_PORT_RIGHT_RECEIVE, -1);
    assert_eq!((kr, woken), (KERN_SUCCESS, vec![8]));
    assert_eq!(i.wait_result(8), Some(WakeReason::Died));
}

#[test]
fn semaphore_lookup_errors() {
    let mut i = ipc();
    let rp = i.reply_port(T);
    let (t, th) = (i.task_self(T), i.thread_self(T, TH));
    let d = i.port_allocate(T, MACH_PORT_RIGHT_DEAD_NAME).unwrap();
    assert_eq!(i.semaphore(T, 0), Err(KERN_INVALID_NAME));
    assert_eq!(i.semaphore(T, MACH_PORT_DEAD), Err(KERN_INVALID_NAME));
    assert_eq!(i.semaphore(T, 0x12345), Err(KERN_INVALID_NAME));
    assert_eq!(i.semaphore(T, t), Err(KERN_INVALID_CAPABILITY));
    assert_eq!(i.semaphore(T, th), Err(KERN_INVALID_CAPABILITY));
    assert_eq!(i.semaphore(T, rp), Err(KERN_INVALID_RIGHT));
    assert_eq!(i.semaphore(T, d), Err(KERN_INVALID_RIGHT));
    assert_eq!(i.thread_of(T, 0), Ok(None));
    assert_eq!(i.thread_of(T, th), Ok(Some(TH)));
    for n in [0x12345, t, rp] {
        assert_eq!(i.thread_of(T, n), Err(KERN_INVALID_ARGUMENT));
    }
}

#[test]
fn semaphores_count_and_wake() {
    let mut i = ipc();
    let s = i.semaphore_create(T, 1).unwrap();
    let p = i.semaphore(T, s).unwrap();
    assert_eq!(i.semaphore_take(p, 1), Ok(true));
    assert_eq!(i.semaphore_take(p, 1), Ok(false));
    assert_eq!(i.semaphore_take(p, 2), Ok(false));
    assert_eq!(i.semaphore_signal(p, Signal::Thread(Some(2))), Ok(vec![2]));
    assert_eq!(i.wait_result(2), Some(WakeReason::Ready));
    assert_eq!(i.semaphore_signal(p, Signal::Thread(Some(5))), Err(KERN_NOT_WAITING));
    assert_eq!(i.semaphore_signal(p, Signal::All), Ok(vec![1]));
    assert_eq!(i.semaphore_signal(p, Signal::Thread(None)), Err(KERN_NOT_WAITING));
    assert_eq!(i.semaphore_signal(p, Signal::One), Ok(vec![]));
    assert_eq!(i.semaphore_count(p), Some(1));
    assert_eq!(i.semaphore_take(p, 3), Ok(true));
    assert_eq!(i.semaphore_take(p, 3), Ok(false));
    assert_eq!(i.semaphore_destroy(T, s), Ok(vec![3]));
    assert_eq!(i.wait_result(3), Some(WakeReason::Died));
    assert_eq!(i.port_type(T, s), Ok(MACH_PORT_TYPE_DEAD_NAME));
}

#[test]
fn a_task_that_ends_kills_its_ports() {
    let mut i = ipc();
    i.create_task(2, 200);
    let p = i.port_allocate(2, MACH_PORT_RIGHT_RECEIVE).unwrap();
    assert_eq!(i.port_insert_right(2, T, 0x8803, p, MACH_MSG_TYPE_MAKE_SEND), KERN_SUCCESS);
    let child_task = i.task_self(2);
    i.port_insert_right(2, T, 0x8903, child_task, MACH_MSG_TYPE_COPY_SEND);
    assert_eq!(i.resolve_task(T, 0x8903), Ok(2));
    i.destroy_task(2);
    assert_eq!(i.port_type(T, 0x8803), Ok(MACH_PORT_TYPE_DEAD_NAME));
    assert_eq!(i.port_type(T, 0x8903), Ok(MACH_PORT_TYPE_DEAD_NAME));
    assert_eq!(i.resolve_task(T, 0x8903), Err(MACH_SEND_INVALID_DEST));
    assert!(!i.has_task(2));
}
