//! Mach exceptions on the host model (ADR 0238, addendum): exception ports
//! set and read through MIG messages sent with mach_msg_trap, a fault that
//! a handler thread answers (with a new state, through thread_get_state and
//! thread_set_state too), declined and missing handlers.
//!
//! The messages follow the request and reply layouts of the public SDK
//! headers (`mach/task.h`, `mach/thread_act.h`, `mach/exc.h`).

mod common;

use common::*;
use poly_mach::consts::{kern::*, msg::*, port::*, trap::*};
use poly_mach::exc::*;
use poly_personality_xnu::exc::Outcome;

const NDR: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];

struct Msg(Vec<u8>);

impl Msg {
    fn new(bits: u32, remote: u64, local: u64, id: u32) -> Msg {
        let mut b = vec![0u8; 24];
        b[0..4].copy_from_slice(&bits.to_le_bytes());
        b[8..12].copy_from_slice(&(remote as u32).to_le_bytes());
        b[12..16].copy_from_slice(&(local as u32).to_le_bytes());
        b[20..24].copy_from_slice(&id.to_le_bytes());
        Msg(b)
    }
    fn u32(mut self, v: u32) -> Msg {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(mut self, v: &[u8]) -> Msg {
        self.0.extend_from_slice(v);
        self
    }
    fn port(self, name: u64, disp: u32) -> Msg {
        self.u32(name as u32).u32(0).bytes(&[0, 0, disp as u8, 0])
    }
}

fn rd(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Write `m` to the process, send it with mach_msg_trap and receive the
/// reply on a new reply port (MIG RPC). Returns (kern_return, reply bytes).
fn rpc(p: &mut P, m: Msg) -> (u32, Vec<u8>) {
    let reply = p.mach(MACH_REPLY_PORT, &[]);
    let mut b = m.0;
    b[12..16].copy_from_slice(&(reply as u32).to_le_bytes());
    let bits = rd(&b, 0) | (MACH_MSG_TYPE_MAKE_SEND_ONCE << 8);
    b[0..4].copy_from_slice(&bits.to_le_bytes());
    let buf = p.var(4096);
    for (i, c) in b.iter().enumerate() {
        p.poke(buf + i as u64, *c);
    }
    let kr = p.kr(MACH_MSG, &[buf, u64::from(MACH_SEND_MSG | MACH_RCV_MSG), b.len() as u64, 4096, reply, 0, 0]);
    let n = p.peek32(buf + 4) as usize;
    let out = (0..n.min(4096)).map(|i| p.peek(buf + i as u64)).collect();
    (kr, out)
}

fn receive(p: &mut P, name: u64) -> Vec<u8> {
    let buf = p.var(8192);
    let kr = p.kr(MACH_MSG, &[buf, u64::from(MACH_RCV_MSG), 0, 8192, name, 0, 0]);
    assert_eq!(kr, MACH_MSG_SUCCESS);
    let n = p.peek32(buf + 4) as usize;
    (0..n).map(|i| p.peek(buf + i as u64)).collect()
}

fn send(p: &mut P, m: Msg) -> u32 {
    let b = m.0;
    let buf = p.var(4096);
    for (i, c) in b.iter().enumerate() {
        p.poke(buf + i as u64, *c);
    }
    p.kr(MACH_MSG, &[buf, u64::from(MACH_SEND_MSG), b.len() as u64, 0, 0, 0, 0])
}

fn new_receive(p: &mut P) -> u64 {
    let v = p.var(4);
    let task = p.mach(TASK_SELF, &[]);
    p.kr(MACH_PORT_ALLOCATE, &[task, u64::from(MACH_PORT_RIGHT_RECEIVE), v]);
    u64::from(p.peek32(v))
}

/// task_set_exception_ports(task, mask, handler (a send right made from a
/// receive right), behavior, flavor).
fn set_ports(p: &mut P, task: u64, mask: u32, handler: u64, behavior: u32, flavor: u32) -> u32 {
    let m = Msg::new(MACH_MSG_TYPE_COPY_SEND | MACH_MSGH_BITS_COMPLEX, task, 0, 3413)
        .u32(1)
        .port(handler, if handler == 0 { MACH_MSG_TYPE_COPY_SEND } else { MACH_MSG_TYPE_MAKE_SEND })
        .bytes(&NDR)
        .u32(mask)
        .u32(behavior)
        .u32(flavor);
    let (kr, out) = rpc(p, m);
    assert_eq!(kr, MACH_MSG_SUCCESS);
    rd(&out, 32)
}

#[test]
fn exception_ports_by_message() {
    run_x86(|p| {
        let task = p.mach(TASK_SELF, &[]);
        let h = new_receive(p);
        let kr =
            set_ports(p, task, 1 << EXC_BAD_ACCESS, h, EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES, THREAD_STATE_NONE);
        p.check("set: KERN_SUCCESS", kr == KERN_SUCCESS, i64::from(kr));
        let kr = set_ports(p, task, 1 << 30, h, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
        p.check("a mask bit that is no exception: KERN_INVALID_ARGUMENT", kr == KERN_INVALID_ARGUMENT, i64::from(kr));
        let kr = set_ports(p, task, 1 << EXC_ARITHMETIC, h, EXCEPTION_STATE, 999);
        p.check("a state behaviour with an unknown flavor: KERN_INVALID_ARGUMENT", kr == KERN_INVALID_ARGUMENT, 0);
        // get: one handler for EXC_MASK_BAD_ACCESS, under its own name.
        let (kr, out) = rpc(p, Msg::new(MACH_MSG_TYPE_COPY_SEND, task, 0, 3414).bytes(&NDR).u32(EXC_MASK_ALL));
        p.check("get: sent and received", kr == MACH_MSG_SUCCESS, i64::from(kr));
        let after = 28 + 32 * 12;
        p.check("get: one handler", rd(&out, after + 8) == 1, i64::from(rd(&out, after + 8)));
        p.check("its mask", rd(&out, after + 12) == 1 << EXC_BAD_ACCESS, 0);
        p.check("its behaviour", rd(&out, after + 16) == EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES, 0);
        p.check("its port is the handler's name", u64::from(rd(&out, 28)) == h, 0);
        // An unknown routine.
        let (_, out) = rpc(p, Msg::new(MACH_MSG_TYPE_COPY_SEND, task, 0, 3499).bytes(&NDR));
        p.check("an unknown routine: MIG_BAD_ID", rd(&out, 32) == -303i32 as u32, 0);
        // Clear it again (MACH_PORT_NULL).
        let kr = set_ports(p, task, 1 << EXC_BAD_ACCESS, 0, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
        p.check("clear: KERN_SUCCESS", kr == KERN_SUCCESS, 0);
        let (_, out) = rpc(p, Msg::new(MACH_MSG_TYPE_COPY_SEND, task, 0, 3414).bytes(&NDR).u32(EXC_MASK_ALL));
        p.check("no handler afterwards", rd(&out, after + 8) == 0, 0);
    });
}

#[test]
fn a_fault_without_a_handler_takes_the_default_action() {
    run_x86(|p| {
        let r = p.fault(14, Some(0x10), false, ThreadState64::default());
        p.check("no handler: the default action", r == Outcome::Default, 0);
    });
}

/// The faulting thread's registers in the tests.
fn regs() -> ThreadState64 {
    let mut r = ThreadState64::default();
    r.0[ThreadState64::RIP] = 0x1_0000_4000;
    r.0[ThreadState64::RSP] = 0x7000_0000;
    r.0[ThreadState64::RAX] = 42;
    r
}

#[test]
fn a_state_identity_handler_moves_the_thread_on() {
    run_x86(|p| {
        let task = p.mach(TASK_SELF, &[]);
        let h = new_receive(p);
        let kr = set_ports(
            p,
            task,
            (1 << EXC_BAD_ACCESS) | (1 << EXC_BAD_INSTRUCTION),
            h,
            EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES,
            X86_THREAD_STATE64,
        );
        p.check("set: KERN_SUCCESS", kr == KERN_SUCCESS, 0);
        let faulting = p.thread(|t| t.fault(14, Some(0xdead_0000), true, regs()));
        // The handler (this thread) gets mach_exception_raise_state_identity.
        let m = receive(p, h);
        p.check("message id 2407", rd(&m, 20) == 2407, i64::from(rd(&m, 20)));
        p.check("complex, with the thread and the task", rd(&m, 0) & MACH_MSGH_BITS_COMPLEX != 0 && rd(&m, 24) == 2, 0);
        let thread_port = u64::from(rd(&m, 28));
        let exc_at = 24 + 4 + 2 * 12 + 8;
        p.check("EXC_BAD_ACCESS", rd(&m, exc_at) == EXC_BAD_ACCESS, i64::from(rd(&m, exc_at)));
        p.check("two codes", rd(&m, exc_at + 4) == 2, 0);
        let code0 = i64::from_le_bytes(m[exc_at + 8..exc_at + 16].try_into().unwrap());
        let code1 = i64::from_le_bytes(m[exc_at + 16..exc_at + 24].try_into().unwrap());
        p.check("KERN_PROTECTION_FAILURE and the address (64-bit codes)", code0 == 2 && code1 == 0xdead_0000, code0);
        let flavor_at = exc_at + 24;
        p.check("the flavor", rd(&m, flavor_at) == X86_THREAD_STATE64, 0);
        p.check("42 words of state", rd(&m, flavor_at + 4) == 42, 0);
        let rip_at = flavor_at + 8 + 4 * 2 * ThreadState64::RIP;
        p.check("the faulting rip", rd(&m, rip_at) == 0x4000 && rd(&m, rip_at + 4) == 1, 0);
        // thread_get_state of the stopped thread, by message to its port.
        let (kr, out) = rpc(p, Msg::new(MACH_MSG_TYPE_COPY_SEND, thread_port, 0, 3603).bytes(&NDR).u32(4).u32(42));
        p.check("thread_get_state: sent", kr == MACH_MSG_SUCCESS, i64::from(kr));
        p.check("thread_get_state: KERN_SUCCESS, 42 words", rd(&out, 32) == 0 && rd(&out, 36) == 42, 0);
        p.check("the stopped thread's rax", rd(&out, 40) == 42, i64::from(rd(&out, 40)));
        // The reply: KERN_SUCCESS, the state with rip moved past the access.
        let mut state = regs();
        state.0[ThreadState64::RIP] += 3;
        let mut r = Msg::new(MACH_MSG_TYPE_MOVE_SEND_ONCE, u64::from(rd(&m, 8)), 0, 2507)
            .bytes(&NDR)
            .u32(KERN_SUCCESS)
            .u32(X86_THREAD_STATE64)
            .u32(42);
        for w in state.to_words() {
            r = r.u32(w);
        }
        let sent = send(p, r);
        p.check("the reply is sent", sent == MACH_MSG_SUCCESS, i64::from(sent));
        let outcome = faulting.join().unwrap();
        let want = Outcome::Resume(state);
        p.check("the thread goes on at the new rip", outcome == want, 0);
    });
}

#[test]
fn thread_handlers_come_first_and_a_declining_one_passes_on() {
    run_x86(|p| {
        let task = p.mach(TASK_SELF, &[]);
        let task_handler = new_receive(p);
        let thread_handler = new_receive(p);
        set_ports(p, task, 1 << EXC_BREAKPOINT, task_handler, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
        let faulting = p.thread(move |t| {
            // thread_set_exception_ports on the thread's own port.
            let me = t.mach(THREAD_SELF, &[]);
            let m = Msg::new(MACH_MSG_TYPE_COPY_SEND | MACH_MSGH_BITS_COMPLEX, me, 0, 3613)
                .u32(1)
                .port(thread_handler, MACH_MSG_TYPE_MAKE_SEND)
                .bytes(&NDR)
                .u32(1 << EXC_BREAKPOINT)
                .u32(EXCEPTION_DEFAULT)
                .u32(THREAD_STATE_NONE);
            let (_, out) = rpc(t, m);
            let set = rd(&out, 32);
            (set, t.fault(3, None, false, regs()))
        });
        // The thread's handler gets exception_raise (2401, 32-bit codes) and declines.
        let m = receive(p, thread_handler);
        p.check("the thread's handler first: exception_raise", rd(&m, 20) == 2401, i64::from(rd(&m, 20)));
        let exc_at = 24 + 4 + 2 * 12 + 8;
        p.check("EXC_BREAKPOINT, EXC_I386_BPT", rd(&m, exc_at) == EXC_BREAKPOINT && rd(&m, exc_at + 8) == 2, 0);
        let decline =
            Msg::new(MACH_MSG_TYPE_MOVE_SEND_ONCE, u64::from(rd(&m, 8)), 0, 2501).bytes(&NDR).u32(KERN_FAILURE);
        send(p, decline);
        // Then the task's handler, which takes it.
        let m = receive(p, task_handler);
        p.check("then the task's handler", rd(&m, 20) == 2401, 0);
        let take = Msg::new(MACH_MSG_TYPE_MOVE_SEND_ONCE, u64::from(rd(&m, 8)), 0, 2501).bytes(&NDR).u32(KERN_SUCCESS);
        send(p, take);
        let (set, outcome) = faulting.join().unwrap();
        p.check("thread_set_exception_ports: KERN_SUCCESS", set == KERN_SUCCESS, i64::from(set));
        p.check("resumed with its registers", outcome == Outcome::Resume(regs()), 0);
    });
}

#[test]
fn a_handler_whose_port_dies_leaves_the_default_action() {
    run_x86(|p| {
        let task = p.mach(TASK_SELF, &[]);
        let h = new_receive(p);
        set_ports(p, task, 1 << EXC_ARITHMETIC, h, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
        let faulting = p.thread(|t| t.fault(0, None, false, regs()));
        let m = receive(p, h);
        let exc_at = 24 + 4 + 2 * 12 + 8;
        p.check("EXC_ARITHMETIC", rd(&m, exc_at) == EXC_ARITHMETIC, 0);
        // The handler goes away without replying: its send-once right dies.
        let reply = u64::from(rd(&m, 8));
        let kr = p.kr(MACH_PORT_MOD_REFS, &[task, reply, u64::from(MACH_PORT_RIGHT_SEND_ONCE), (-1i64) as u64]);
        p.check("the reply right is destroyed", kr == KERN_SUCCESS, i64::from(kr));
        p.check("the thread gets the default action", faulting.join().unwrap() == Outcome::Default, 0);
    });
}

#[test]
fn semaphores_made_by_message_count() {
    run_x86(|p| {
        let task = p.mach(TASK_SELF, &[]);
        let (kr, out) = rpc(p, Msg::new(MACH_MSG_TYPE_COPY_SEND, task, 0, 3418).bytes(&NDR).u32(0).u32(1));
        p.check("semaphore_create: sent", kr == MACH_MSG_SUCCESS, 0);
        let sem = u64::from(rd(&out, 28));
        p.check("a semaphore name", sem != 0, 0);
        let kr = p.kr(SEMAPHORE_WAIT, &[sem]);
        p.check("the count of 1 is taken at once", kr == KERN_SUCCESS, i64::from(kr));
        let kr = p.kr(SEMAPHORE_TIMEDWAIT, &[sem, 0, 1_000_000]);
        p.check("then a timed wait times out", kr == KERN_OPERATION_TIMED_OUT, i64::from(kr));
        let kr = p.kr(SEMAPHORE_SIGNAL, &[sem]);
        p.check("a signal", kr == KERN_SUCCESS, 0);
        let kr = p.kr(SEMAPHORE_WAIT, &[sem]);
        p.check("and a wait takes it", kr == KERN_SUCCESS, 0);
    });
}
