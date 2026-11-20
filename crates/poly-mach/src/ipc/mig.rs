//! The kernel's side of the MIG requests a program sends to its task and
//! thread ports, and the messages the kernel sends for exceptions (ADR
//! 0238, addendum).
//!
//! The layouts are those of the public SDK headers MIG generated
//! (`mach/task.h`, `mach/thread_act.h`, `mach/exc.h`) and of
//! `mach/mach_exc.defs` (`exc` with 64-bit codes); message ids are a
//! subsystem's base plus the routine's index (`task` 3400, `thread_act`
//! 3600, `exc` 2401, `mach_exc` 2405), a reply's the request's plus 100.
//! Variable-sized inline arrays travel packed: as many elements as their
//! count says, the next field right after them.

use super::{Carried, HEADER, Ipc, KObject, Message, PortId, RcvKey, SenderInfo, TaskId, WakeReason};
use crate::consts::{kern::*, msg::*};
use crate::exc::{self, Handler, HandlerTable};
use crate::waitq::ThreadId;
use alloc::vec;
use alloc::vec::Vec;

/// `NDR_record` (`mach/ndr.h`): little-endian integers, ASCII, IEEE floats.
pub const NDR: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];
/// `MIG_BAD_ID` and `MIG_BAD_ARGUMENTS` (`mach/mig_errors.h`).
pub const MIG_BAD_ID: u32 = -303i32 as u32;
pub const MIG_BAD_ARGUMENTS: u32 = -304i32 as u32;

pub const TASK_SET_EXCEPTION_PORTS: u32 = 3413;
pub const TASK_GET_EXCEPTION_PORTS: u32 = 3414;
pub const TASK_SWAP_EXCEPTION_PORTS: u32 = 3415;
pub const SEMAPHORE_CREATE: u32 = 3418;
pub const SEMAPHORE_DESTROY: u32 = 3419;
pub const THREAD_GET_STATE: u32 = 3603;
pub const THREAD_SET_STATE: u32 = 3604;
pub const THREAD_SET_EXCEPTION_PORTS: u32 = 3613;
pub const THREAD_GET_EXCEPTION_PORTS: u32 = 3614;
pub const THREAD_SWAP_EXCEPTION_PORTS: u32 = 3615;
/// The largest number of handlers a get or swap returns
/// (`EXC_TYPES_COUNT` entries in the reply's arrays).
pub const MAX_HANDLERS: usize = 32;

/// What the kernel does for thread state requests.
pub trait ThreadServer {
    /// `thread`'s state in `flavor`, at most `max` words.
    fn get_state(&mut self, thread: ThreadId, flavor: u32, max: u32) -> Result<Vec<u32>, u32>;
    /// Set `thread`'s state in `flavor`.
    fn set_state(&mut self, thread: ThreadId, flavor: u32, state: &[u32]) -> u32;
}

/// No thread services: state requests fail with `KERN_FAILURE`.
#[derive(Debug)]
pub struct NoThreads;

impl ThreadServer for NoThreads {
    fn get_state(&mut self, _: ThreadId, _: u32, _: u32) -> Result<Vec<u32>, u32> {
        Err(KERN_FAILURE)
    }
    fn set_state(&mut self, _: ThreadId, _: u32, _: &[u32]) -> u32 {
        KERN_FAILURE
    }
}

fn rd(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn push(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A port descriptor for `out` (name and type filled in on receive).
fn descriptor(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0; 12]);
}

/// A reply the server built: id, body, port descriptors, complex.
type Reply = (u32, Vec<u8>, Vec<(usize, Option<Carried>)>, bool);

/// An exception a thread raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exception {
    pub task: TaskId,
    pub thread: ThreadId,
    /// `EXC_*`.
    pub ty: u32,
    pub codes: [i64; 2],
}

/// Whose handlers a request changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Task(TaskId),
    Thread(ThreadId),
    Host,
}

impl Ipc {
    fn table_mut(&mut self, target: Target) -> &mut HandlerTable {
        match target {
            Target::Task(t) => self.exc_task.entry(t).or_default(),
            Target::Thread(t) => self.exc_thread.entry(t).or_default(),
            Target::Host => &mut self.exc_host,
        }
    }

    fn table(&self, target: Target) -> HandlerTable {
        match target {
            Target::Task(t) => self.exc_task.get(&t).copied().unwrap_or_default(),
            Target::Thread(t) => self.exc_thread.get(&t).copied().unwrap_or_default(),
            Target::Host => self.exc_host,
        }
    }

    /// `*_set_exception_ports`: `port` (a send right the kernel now holds,
    /// `None` to clear) handles the types in `mask`.
    pub fn set_exception_ports(
        &mut self,
        target: Target,
        mask: u32,
        port: Option<PortId>,
        behavior: u32,
        flavor: u32,
    ) -> Result<(), u32> {
        exc::check_set(mask, behavior, flavor)?;
        let table = self.table_mut(target);
        for (ty, slot) in table.0.iter_mut().enumerate().skip(1) {
            if mask & (1 << ty) != 0 {
                *slot = port.map(|port| Handler { port, behavior, flavor });
            }
        }
        Ok(())
    }

    /// A new task gets its parent's task exception handlers (a child of
    /// fork or posix_spawn inherits them).
    pub fn inherit_exception_ports(&mut self, parent: TaskId, child: TaskId) {
        if let Some(t) = self.exc_task.get(&parent).copied() {
            self.exc_task.insert(child, t);
        }
    }

    /// `*_get_exception_ports`: the handlers of the types in `mask`, one
    /// entry per distinct (port, behaviour, flavour) with the mask of its
    /// types. Types without a handler are left out.
    pub fn get_exception_ports(&self, target: Target, mask: u32) -> Result<Vec<(u32, Handler)>, u32> {
        if mask & !exc::EXC_MASK_VALID != 0 {
            return Err(KERN_INVALID_ARGUMENT);
        }
        let table = self.table(target);
        let mut out: Vec<(u32, Handler)> = Vec::new();
        for (ty, slot) in table.0.iter().enumerate().skip(1) {
            let Some(h) = slot else { continue };
            if mask & (1 << ty) == 0 {
                continue;
            }
            match out.iter_mut().find(|(_, o)| o == h) {
                Some((m, _)) => *m |= 1 << ty,
                None => out.push((1 << ty, *h)),
            }
        }
        out.truncate(MAX_HANDLERS);
        Ok(out)
    }

    /// The handlers an exception of type `ty` in `thread` of `task` goes to,
    /// in order: the thread's, the task's, the host's. Dead ports are left
    /// out.
    pub fn exception_handlers(&self, task: TaskId, thread: ThreadId, ty: u32) -> Vec<Handler> {
        [Target::Thread(thread), Target::Task(task), Target::Host]
            .iter()
            .filter_map(|&t| self.table(t).0.get(ty as usize).copied().flatten())
            .filter(|h| self.ports.contains_key(&h.port))
            .collect()
    }

    /// A port whose receive right the kernel keeps, to wait for a reply.
    pub fn kernel_port(&mut self) -> PortId {
        let p = self.new_port(KObject::None, None);
        self.ports.get_mut(&p).expect("just made").kernel_reply = true;
        p
    }

    /// Give up a kernel port (its pending messages are destroyed).
    pub fn free_kernel_port(&mut self, p: PortId) -> Vec<ThreadId> {
        let mut woken = Vec::new();
        self.destroy_port(p, &mut woken);
        woken
    }

    /// The first message on a kernel port: its id and its body (after the
    /// header). Rights it carries are released.
    pub fn kernel_receive(&mut self, p: PortId) -> Option<(u32, Vec<u8>)> {
        let m = self.ports.get_mut(&p)?.queue.pop_front()?;
        let (id, body) = (m.id, m.body.clone());
        let mut woken = Vec::new();
        self.destroy_message(m, &mut woken);
        Some((id, body))
    }

    /// Send `handler` the message for exception `e`, with the thread's state in the handler's flavour
    /// (`state`, for the state behaviours); the reply goes to the kernel
    /// port `reply`. Returns the receiver woken. `Err` for a handler whose
    /// behaviour Poly does not send (the protected ones) or whose port died.
    pub fn raise(
        &mut self,
        handler: &Handler,
        e: &Exception,
        state: Option<&[u32]>,
        reply: PortId,
    ) -> Result<Option<ThreadId>, u32> {
        let Exception { task, thread, ty, codes } = *e;
        let base = handler.behavior & !exc::MACH_EXCEPTION_MASK;
        let wide = handler.behavior & exc::MACH_EXCEPTION_CODES != 0;
        let (index, identity, with_state) = match base {
            exc::EXCEPTION_DEFAULT => (0, true, false),
            exc::EXCEPTION_STATE => (1, false, true),
            exc::EXCEPTION_STATE_IDENTITY => (2, true, true),
            _ => return Err(KERN_INVALID_ARGUMENT),
        };
        if !self.ports.contains_key(&handler.port) {
            return Err(MACH_SEND_INVALID_DEST);
        }
        let id = if wide { exc::MACH_EXCEPTION_RAISE } else { exc::EXCEPTION_RAISE } + index;
        let mut body = Vec::new();
        let mut ports = Vec::new();
        if identity {
            let thread_port = self.thread_ports.get(&thread).copied();
            let task_port = self.spaces.get(&task).map(|s| s.task_port);
            push(&mut body, 2);
            for p in [thread_port, task_port] {
                ports.push((body.len(), p.map(Carried::Send)));
                descriptor(&mut body);
            }
        }
        body.extend_from_slice(&NDR);
        push(&mut body, ty);
        push(&mut body, 2);
        for c in codes {
            if wide {
                body.extend_from_slice(&c.to_le_bytes());
            } else {
                push(&mut body, c as u32);
            }
        }
        if with_state {
            let words = state.unwrap_or(&[]);
            push(&mut body, handler.flavor);
            push(&mut body, words.len() as u32);
            for w in words {
                push(&mut body, *w);
            }
        }
        let m = Message {
            other_bits: if identity { MACH_MSGH_BITS_COMPLEX } else { 0 },
            id,
            size: (HEADER + body.len()) as u32,
            dest: Carried::Send(handler.port),
            reply: Some(Carried::SendOnce(reply)),
            body,
            ports,
            sender: SenderInfo::default(),
        };
        Ok(self.deliver(handler.port, m))
    }

    /// Queue `m` on `port` (the kernel ignores the queue limit) and wake a
    /// receiver.
    fn deliver(&mut self, port: PortId, m: Message) -> Option<ThreadId> {
        let Some(q) = self.ports.get_mut(&port) else {
            let mut woken = Vec::new();
            self.destroy_message(m, &mut woken);
            return None;
        };
        q.queue.push_back(m);
        let set = q.set;
        self.rcv_waiters
            .wake_one(RcvKey::Port(port), WakeReason::Ready)
            .or_else(|| set.and_then(|s| self.rcv_waiters.wake_one(RcvKey::Set(s), WakeReason::Ready)))
    }

    /// Serve a request sent to a kernel object; queue its reply on the
    /// request's reply port. Returns the receiver woken.
    pub(super) fn serve(
        &mut self,
        caller: TaskId,
        kobject: KObject,
        mut req: Message,
        threads: &mut dyn ThreadServer,
    ) -> Option<ThreadId> {
        let reply = self.answer(caller, kobject, &mut req, threads);
        let mut woken = Vec::new();
        let to = req.reply.take();
        // The request's remaining rights (the destination, descriptors the
        // server did not keep) are released.
        self.destroy_message(req, &mut woken);
        let (id, body, ports, complex) = reply?;
        let to = to?;
        let port = match to {
            Carried::Send(p) | Carried::SendOnce(p) => p,
            _ => return None,
        };
        let m = Message {
            other_bits: if complex { MACH_MSGH_BITS_COMPLEX } else { 0 },
            id,
            size: (HEADER + body.len()) as u32,
            dest: to,
            reply: None,
            body,
            ports,
            sender: SenderInfo::default(),
        };
        self.deliver(port, m)
    }

    /// The reply to a request.
    fn answer(
        &mut self,
        caller: TaskId,
        kobject: KObject,
        req: &mut Message,
        threads: &mut dyn ThreadServer,
    ) -> Option<Reply> {
        let id = req.id;
        let b = req.body.clone();
        let simple = |ret: u32| {
            let mut out = NDR.to_vec();
            push(&mut out, ret);
            Some((id + 100, out, Vec::new(), false))
        };
        let complex = req.other_bits & MACH_MSGH_BITS_COMPLEX != 0;
        let target = match kobject {
            KObject::Task(t) => Target::Task(t),
            KObject::Thread(t) => Target::Thread(t),
            _ => return simple(MIG_BAD_ID),
        };
        match (target, id) {
            (Target::Task(_), TASK_SET_EXCEPTION_PORTS | TASK_SWAP_EXCEPTION_PORTS)
            | (Target::Thread(_), THREAD_SET_EXCEPTION_PORTS | THREAD_SWAP_EXCEPTION_PORTS) => {
                let (Some(mask), Some(behavior), Some(flavor)) = (rd(&b, 24), rd(&b, 28), rd(&b, 32)) else {
                    return simple(MIG_BAD_ARGUMENTS);
                };
                if !complex || req.ports.is_empty() {
                    return simple(MIG_BAD_ARGUMENTS);
                }
                let port = match req.ports[0].1 {
                    None | Some(Carried::Dead) => None,
                    Some(Carried::Send(p)) => Some(p),
                    _ => return simple(KERN_INVALID_RIGHT),
                };
                let swap = matches!(id, TASK_SWAP_EXCEPTION_PORTS | THREAD_SWAP_EXCEPTION_PORTS);
                let old = if swap {
                    match self.get_exception_ports(target, mask) {
                        Ok(o) => Some(o),
                        Err(kr) => return simple(kr),
                    }
                } else {
                    None
                };
                if let Err(kr) = self.set_exception_ports(target, mask, port, behavior, flavor) {
                    return simple(kr);
                }
                // The kernel keeps the send right now.
                req.ports[0].1 = None;
                match old {
                    Some(old) => Some(self.handlers_reply(id, &old)),
                    None => simple(KERN_SUCCESS),
                }
            }
            (Target::Task(_), TASK_GET_EXCEPTION_PORTS) | (Target::Thread(_), THREAD_GET_EXCEPTION_PORTS) => {
                let Some(mask) = rd(&b, 8) else { return simple(MIG_BAD_ARGUMENTS) };
                match self.get_exception_ports(target, mask) {
                    Ok(old) => Some(self.handlers_reply(id, &old)),
                    Err(kr) => simple(kr),
                }
            }
            (Target::Thread(t), THREAD_GET_STATE) => {
                let (Some(flavor), Some(max)) = (rd(&b, 8), rd(&b, 12)) else { return simple(MIG_BAD_ARGUMENTS) };
                match threads.get_state(t, flavor, max) {
                    Ok(words) => {
                        let mut out = NDR.to_vec();
                        push(&mut out, KERN_SUCCESS);
                        push(&mut out, words.len() as u32);
                        for w in words {
                            push(&mut out, w);
                        }
                        Some((id + 100, out, Vec::new(), false))
                    }
                    Err(kr) => simple(kr),
                }
            }
            (Target::Thread(t), THREAD_SET_STATE) => {
                let (Some(flavor), Some(n)) = (rd(&b, 8), rd(&b, 12)) else { return simple(MIG_BAD_ARGUMENTS) };
                let words: Option<Vec<u32>> = (0..n as usize).map(|i| rd(&b, 16 + 4 * i)).collect();
                match words {
                    Some(w) => simple(threads.set_state(t, flavor, &w)),
                    None => simple(MIG_BAD_ARGUMENTS),
                }
            }
            (Target::Task(t), SEMAPHORE_CREATE) => {
                let (Some(policy), Some(value)) = (rd(&b, 8), rd(&b, 12)) else { return simple(MIG_BAD_ARGUMENTS) };
                if policy > 3 || (value as i32) < 0 {
                    return simple(KERN_INVALID_ARGUMENT);
                }
                let _ = caller;
                let p = self.new_port(KObject::Semaphore, None);
                self.ports.get_mut(&p).expect("just made").count = value;
                let _ = t;
                let mut out = Vec::new();
                push(&mut out, 1);
                let at = out.len();
                descriptor(&mut out);
                Some((id + 100, out, vec![(at, Some(Carried::Send(p)))], true))
            }
            (Target::Task(_), SEMAPHORE_DESTROY) => {
                let sem = match req.ports.first().and_then(|(_, r)| *r) {
                    Some(Carried::Send(p)) if self.ports.get(&p).is_some_and(|q| q.kobject == KObject::Semaphore) => p,
                    Some(_) => return simple(KERN_INVALID_ARGUMENT),
                    None => return simple(KERN_INVALID_ARGUMENT),
                };
                let mut woken = Vec::new();
                self.destroy_port(sem, &mut woken);
                simple(KERN_SUCCESS)
            }
            _ => simple(MIG_BAD_ID),
        }
    }

    /// The reply of a get or swap: `MAX_HANDLERS` port descriptors (the
    /// handlers first), then the count and the packed masks, behaviours and
    /// flavours.
    fn handlers_reply(&mut self, id: u32, old: &[(u32, Handler)]) -> Reply {
        let mut out = Vec::new();
        let mut ports = Vec::new();
        push(&mut out, MAX_HANDLERS as u32);
        for i in 0..MAX_HANDLERS {
            ports.push((out.len(), old.get(i).map(|(_, h)| Carried::Send(h.port))));
            descriptor(&mut out);
        }
        out.extend_from_slice(&NDR);
        push(&mut out, old.len() as u32);
        for (m, _) in old {
            push(&mut out, *m);
        }
        for (_, h) in old {
            push(&mut out, h.behavior);
        }
        for (_, h) in old {
            push(&mut out, h.flavor);
        }
        (id + 100, out, ports, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::port::*;
    use crate::exc::*;

    const T: TaskId = 1;
    const TH: ThreadId = 100;

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

    fn port_desc(body: &mut Vec<u8>, name: u32, disp: u32) {
        push(body, name);
        push(body, 0);
        body.extend_from_slice(&[0, 0, disp as u8, 0]);
    }

    /// A MIG RPC from task T to `remote`: the reply received on a reply port.
    fn rpc(i: &mut Ipc, remote: u32, id: u32, body: &[u8], complex: bool) -> Vec<u8> {
        let reply = i.reply_port(T);
        let bits = MACH_MSG_TYPE_COPY_SEND
            | (MACH_MSG_TYPE_MAKE_SEND_ONCE << 8)
            | if complex { MACH_MSGH_BITS_COMPLEX } else { 0 };
        i.send(T, &msg(bits, remote, reply, id, body), SenderInfo::default()).unwrap();
        let (r, _) = i.receive(T, reply, 4096, 0).unwrap();
        let super::super::Received::Message(out) = r else { panic!("no reply") };
        assert_eq!(rd(&out, 20).unwrap(), id + 100);
        out
    }

    #[test]
    fn set_get_and_swap_through_messages() {
        let mut i = Ipc::new();
        i.create_task(T, TH);
        let task = i.task_self(T);
        let handler = i.port_allocate(T, MACH_PORT_RIGHT_RECEIVE).unwrap();
        // set: EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT to a send right made from `handler`.
        let mut b = Vec::new();
        push(&mut b, 1);
        port_desc(&mut b, handler, MACH_MSG_TYPE_MAKE_SEND);
        b.extend_from_slice(&NDR);
        let mask = (1 << EXC_BAD_ACCESS) | (1 << EXC_BREAKPOINT);
        push(&mut b, mask);
        push(&mut b, EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES);
        push(&mut b, X86_THREAD_STATE64);
        let out = rpc(&mut i, task, TASK_SET_EXCEPTION_PORTS, &b, true);
        assert_eq!(rd(&out, 32), Some(KERN_SUCCESS));
        // get: one entry with both bits.
        let mut g = NDR.to_vec();
        push(&mut g, EXC_MASK_ALL);
        let out = rpc(&mut i, task, TASK_GET_EXCEPTION_PORTS, &g, false);
        assert_ne!(rd(&out, 0).unwrap() & MACH_MSGH_BITS_COMPLEX, 0);
        assert_eq!(rd(&out, 24), Some(32), "descriptor count");
        let first = 28;
        assert_eq!(rd(&out, first), Some(handler), "the handler comes back under its name");
        let after = 28 + 32 * 12;
        assert_eq!(rd(&out, after + 8), Some(1), "masksCnt");
        assert_eq!(rd(&out, after + 12), Some(mask));
        assert_eq!(rd(&out, after + 16), Some(EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES));
        assert_eq!(rd(&out, after + 20), Some(X86_THREAD_STATE64));
        assert_eq!(i.exception_handlers(T, TH, EXC_BAD_ACCESS).len(), 1);
        assert!(i.exception_handlers(T, TH, EXC_ARITHMETIC).is_empty());
        // A bad behaviour is refused.
        let mut bad = b.clone();
        bad[32..36].copy_from_slice(&9u32.to_le_bytes());
        let out = rpc(&mut i, task, TASK_SET_EXCEPTION_PORTS, &bad, true);
        assert_eq!(rd(&out, 32), Some(KERN_INVALID_ARGUMENT));
        // An unknown routine: MIG_BAD_ID.
        let out = rpc(&mut i, task, 3499, &NDR, false);
        assert_eq!(rd(&out, 32), Some(MIG_BAD_ID));
    }

    #[test]
    fn an_exception_message_and_its_reply() {
        let mut i = Ipc::new();
        i.create_task(T, TH);
        let h = i.port_allocate(T, MACH_PORT_RIGHT_RECEIVE).unwrap();
        let hp = match i.receive_key(T, h).unwrap() {
            RcvKey::Port(p) => p,
            _ => unreachable!(),
        };
        i.set_exception_ports(
            Target::Task(T),
            1 << EXC_BAD_ACCESS,
            Some(hp),
            EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES,
            0,
        )
        .unwrap();
        let handler = i.exception_handlers(T, TH, EXC_BAD_ACCESS)[0];
        let kp = i.kernel_port();
        let e = Exception { task: T, thread: TH, ty: EXC_BAD_ACCESS, codes: [1, 0x1234_5678_9abc] };
        i.raise(&handler, &e, None, kp).unwrap();
        let (r, _) = i.receive(T, h, 4096, 0).unwrap();
        let super::super::Received::Message(m) = r else { panic!() };
        assert_eq!(rd(&m, 20), Some(MACH_EXCEPTION_RAISE));
        assert_ne!(rd(&m, 0).unwrap() & MACH_MSGH_BITS_COMPLEX, 0);
        assert_eq!(rd(&m, 24), Some(2));
        let thread_name = rd(&m, 28).unwrap();
        let task_name = rd(&m, 40).unwrap();
        assert_eq!(thread_name, i.thread_self(T, TH));
        assert_eq!(task_name, i.task_self(T));
        // NDR at 52, exception at 60, codeCnt 64, codes 68 (64-bit).
        assert_eq!(rd(&m, 60), Some(EXC_BAD_ACCESS));
        assert_eq!(rd(&m, 64), Some(2));
        assert_eq!(i64::from_le_bytes(m[76..84].try_into().unwrap()), 0x1234_5678_9abc);
        // The handler replies KERN_SUCCESS to the send-once right in the header.
        let reply_to = rd(&m, 8).unwrap();
        let mut rb = NDR.to_vec();
        push(&mut rb, KERN_SUCCESS);
        i.send(
            T,
            &msg(MACH_MSG_TYPE_MOVE_SEND_ONCE, reply_to, 0, MACH_EXCEPTION_RAISE + 100, &rb),
            SenderInfo::default(),
        )
        .unwrap();
        let (id, body) = i.kernel_receive(kp).unwrap();
        assert_eq!((id, rd(&body, 8)), (MACH_EXCEPTION_RAISE + 100, Some(KERN_SUCCESS)));
    }

    #[test]
    fn a_handler_that_dies_wakes_the_kernel_waiter() {
        let mut i = Ipc::new();
        i.create_task(T, TH);
        let h = i.port_allocate(T, MACH_PORT_RIGHT_RECEIVE).unwrap();
        let RcvKey::Port(hp) = i.receive_key(T, h).unwrap() else { unreachable!() };
        i.set_exception_ports(Target::Thread(TH), 1 << EXC_BREAKPOINT, Some(hp), EXCEPTION_STATE, X86_THREAD_STATE64)
            .unwrap();
        let handler = i.exception_handlers(T, TH, EXC_BREAKPOINT)[0];
        let kp = i.kernel_port();
        let state = ThreadState64::default().to_words();
        let e = Exception { task: T, thread: TH, ty: EXC_BREAKPOINT, codes: [2, 0] };
        i.raise(&handler, &e, Some(&state), kp).unwrap();
        i.wait_receive(RcvKey::Port(kp), 77);
        let (_, woken) = i.port_mod_refs(T, h, MACH_PORT_RIGHT_RECEIVE, -1);
        assert!(woken.contains(&77));
        assert_eq!(i.wait_result(77), Some(WakeReason::Died));
        assert!(i.kernel_receive(kp).is_none());
        assert!(i.exception_handlers(T, TH, EXC_BREAKPOINT).is_empty(), "a dead handler is skipped");
    }

    #[test]
    fn semaphores_by_message() {
        let mut i = Ipc::new();
        i.create_task(T, TH);
        let task = i.task_self(T);
        let mut b = NDR.to_vec();
        push(&mut b, 0);
        push(&mut b, 2);
        let out = rpc(&mut i, task, SEMAPHORE_CREATE, &b, false);
        let name = rd(&out, 28).unwrap();
        let p = i.semaphore(T, name).unwrap();
        assert_eq!(i.semaphore_count(p), Some(2));
        let mut d = Vec::new();
        push(&mut d, 1);
        port_desc(&mut d, name, MACH_MSG_TYPE_COPY_SEND);
        let out = rpc(&mut i, task, SEMAPHORE_DESTROY, &d, true);
        assert_eq!(rd(&out, 32), Some(KERN_SUCCESS));
        assert_eq!(i.semaphore_count(p), None);
    }
}
