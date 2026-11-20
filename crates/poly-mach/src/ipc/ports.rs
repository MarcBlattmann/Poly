//! The `mach_port_*` routines beyond the basic traps (ADR 0238, addendum):
//! construct and destruct, guards and contexts, attributes, port set
//! membership, peek, extract_right, references, and notification requests
//! (no-senders, dead-name, port-destroyed).
//!
//! Each is reached through its Mach trap where one exists and through the
//! `mach_port` MIG subsystem on the task port ([`super::mig`]); both end
//! here. Results follow the `specs/macos/libsystem_kernel.dylib/mach_port_*`
//! sheets. A guard violation is [`POLY_GUARD_VIOLATION`]: the caller ends
//! the process with SIGKILL.

use super::tasks::Reply;
use super::{Carried, Entry, Guard, HEADER, Ipc, KObject, Message, NsRequest, Obj, PortId, SenderInfo, TaskId};
use crate::consts::{kern::*, msg::*, notify::*, port::*};
use crate::waitq::ThreadId;
use alloc::vec;
use alloc::vec::Vec;

/// `NDR_record`.
const NDR: [u8; 8] = [0, 0, 0, 0, 1, 0, 0, 0];

/// What mach_port_construct asks for (`mach_port_options_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub flags: u32,
    pub qlimit: u32,
}

/// A message mach_port_peek looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peeked {
    pub seqno: u32,
    /// The message's size plus the (null) trailer's.
    pub size: u32,
    pub id: u32,
}

impl Ipc {
    /// The port of the receive right `name` in `task`: `KERN_INVALID_NAME`
    /// for a name that is not there, `KERN_INVALID_RIGHT` for any other
    /// right.
    fn receive_right(&self, task: TaskId, name: u32) -> Result<PortId, u32> {
        let e = self.entry(task, name).ok_or(KERN_INVALID_NAME)?;
        match e.obj {
            Obj::Port(p) if e.receive => Ok(p),
            _ => Err(KERN_INVALID_RIGHT),
        }
    }

    /// mach_port_construct: a new receive right as `options` say.
    pub fn port_construct(&mut self, task: TaskId, options: Options, context: u64) -> Result<u32, u32> {
        if !self.spaces.contains_key(&task) {
            return Err(MACH_SEND_INVALID_DEST);
        }
        if options.flags & MPO_QLIMIT != 0 && options.qlimit > MACH_PORT_QLIMIT_MAX {
            return Err(KERN_INVALID_VALUE);
        }
        let name = self.port_allocate(task, MACH_PORT_RIGHT_RECEIVE)?;
        let p = self.receive_right(task, name).expect("just made");
        let q = self.ports.get_mut(&p).expect("just made");
        if options.flags & MPO_QLIMIT != 0 {
            q.qlimit = options.qlimit as usize;
        }
        if options.flags & MPO_CONTEXT_AS_GUARD != 0 {
            q.context = context;
            q.guard = Some(Guard { strict: options.flags & MPO_STRICT != 0 });
        }
        if options.flags & MPO_INSERT_SEND_RIGHT != 0 {
            q.mscount += 1;
            self.add_send(task, p, 1);
        }
        Ok(name)
    }

    /// Whether `guard` opens the receive right of `p`: always for a port
    /// without a guard.
    fn guard_ok(&self, p: PortId, guard: u64) -> bool {
        self.ports.get(&p).is_none_or(|q| q.guard.is_none() || q.context == guard)
    }

    fn guarded(&self, p: PortId) -> bool {
        self.ports.get(&p).is_some_and(|q| q.guard.is_some())
    }

    /// mach_port_destruct: destroy the receive right `name`, taking
    /// `-srdelta` user references off its send right at the same time.
    pub fn port_destruct(&mut self, task: TaskId, name: u32, srdelta: i32, guard: u64) -> (u32, Vec<ThreadId>) {
        let mut woken = Vec::new();
        let p = match self.receive_right(task, name) {
            Ok(p) => p,
            Err(kr) => return (kr, woken),
        };
        let send = self.entry(task, name).map_or(0, |e| e.send);
        if srdelta > 0 {
            return (KERN_INVALID_VALUE, woken);
        }
        if srdelta < 0 && send == 0 {
            return (KERN_INVALID_RIGHT, woken);
        }
        if srdelta.unsigned_abs() > send {
            return (KERN_INVALID_VALUE, woken);
        }
        if !self.guard_ok(p, guard) {
            return (POLY_GUARD_VIOLATION, woken);
        }
        if let Some(e) = self.entry_mut(task, name) {
            e.send -= srdelta.unsigned_abs();
        }
        self.destroy_port(p, &mut woken);
        if self.entry(task, name).is_some_and(Entry::is_empty) {
            self.remove_name(task, name);
        }
        (KERN_SUCCESS, woken)
    }

    /// mach_port_destroy: every right under `name` goes (a receive right's
    /// port dies; a guarded one is a violation).
    pub fn port_destroy(&mut self, task: TaskId, name: u32) -> (u32, Vec<ThreadId>) {
        let mut woken = Vec::new();
        if name == MACH_PORT_NULL || name == MACH_PORT_DEAD {
            return (KERN_SUCCESS, woken);
        }
        let Some(&e) = self.entry(task, name) else { return (KERN_INVALID_NAME, woken) };
        match e.obj {
            Obj::Port(p) if e.receive => {
                if self.guarded(p) {
                    return (POLY_GUARD_VIOLATION, woken);
                }
                self.destroy_port(p, &mut woken);
                if self.entry(task, name).is_some() {
                    self.remove_name(task, name);
                }
            }
            Obj::Port(p) => {
                self.remove_name(task, name);
                for _ in 0..e.send_once {
                    self.send_once_gone(p);
                }
            }
            Obj::Set(s) => {
                self.destroy_set(s);
                self.remove_name(task, name);
            }
            Obj::Dead => self.remove_name(task, name),
        }
        (KERN_SUCCESS, woken)
    }

    /// mach_port_guard: guard the receive right `name` with `guard`.
    pub fn port_guard(&mut self, task: TaskId, name: u32, guard: u64, strict: bool) -> u32 {
        let p = match self.receive_right(task, name) {
            Ok(p) => p,
            Err(kr) => return kr,
        };
        let q = self.ports.get_mut(&p).expect("named port exists");
        if q.guard.is_some() || q.context != 0 {
            return KERN_INVALID_ARGUMENT;
        }
        if guard != 0 {
            q.context = guard;
            q.guard = Some(Guard { strict });
        }
        KERN_SUCCESS
    }

    /// mach_port_unguard: remove the guard `guard` (anything else, or a
    /// port without a guard, is a violation).
    pub fn port_unguard(&mut self, task: TaskId, name: u32, guard: u64) -> u32 {
        let p = match self.receive_right(task, name) {
            Ok(p) => p,
            Err(kr) => return kr,
        };
        let q = self.ports.get_mut(&p).expect("named port exists");
        if q.guard.is_none() || q.context != guard {
            return POLY_GUARD_VIOLATION;
        }
        q.guard = None;
        q.context = 0;
        KERN_SUCCESS
    }

    /// mach_port_get_context (a strict guard reads as 0).
    pub fn port_get_context(&self, task: TaskId, name: u32) -> Result<u64, u32> {
        let p = self.receive_right(task, name)?;
        let q = &self.ports[&p];
        Ok(if q.guard.is_some_and(|g| g.strict) { 0 } else { q.context })
    }

    /// mach_port_set_context (on a guarded port it replaces the guard).
    pub fn port_set_context(&mut self, task: TaskId, name: u32, context: u64) -> u32 {
        match self.receive_right(task, name) {
            Ok(p) => {
                self.ports.get_mut(&p).expect("named port exists").context = context;
                KERN_SUCCESS
            }
            Err(kr) => kr,
        }
    }

    /// mach_port_set_mscount.
    pub fn port_set_mscount(&mut self, task: TaskId, name: u32, mscount: u32) -> u32 {
        match self.receive_right(task, name) {
            Ok(p) => {
                self.ports.get_mut(&p).expect("named port exists").mscount = mscount;
                KERN_SUCCESS
            }
            Err(kr) => kr,
        }
    }

    /// mach_port_get_refs: the user references of one kind of right.
    pub fn port_get_refs(&self, task: TaskId, name: u32, right: u32) -> Result<u32, u32> {
        if right > MACH_PORT_RIGHT_DEAD_NAME {
            return Err(KERN_INVALID_VALUE);
        }
        if name == MACH_PORT_NULL || name == MACH_PORT_DEAD {
            return Ok(0);
        }
        let e = self.entry(task, name).ok_or(KERN_INVALID_NAME)?;
        Ok(match right {
            MACH_PORT_RIGHT_SEND => e.send,
            MACH_PORT_RIGHT_RECEIVE => u32::from(e.receive),
            MACH_PORT_RIGHT_SEND_ONCE => e.send_once,
            MACH_PORT_RIGHT_PORT_SET => u32::from(matches!(e.obj, Obj::Set(_))),
            _ => e.dead,
        })
    }

    /// mach_port_get_attributes: `flavor`'s words, at most `count` of them.
    pub fn port_get_attributes(&self, task: TaskId, name: u32, flavor: u32, count: u32) -> Result<Vec<u32>, u32> {
        let p = self.receive_right(task, name)?;
        let q = &self.ports[&p];
        let words = match flavor {
            MACH_PORT_LIMITS_INFO => vec![q.qlimit as u32],
            MACH_PORT_RECEIVE_STATUS => vec![
                u32::from(!q.sets.is_empty()),
                q.seqno,
                q.mscount,
                q.qlimit as u32,
                q.queue.len() as u32,
                q.sorights,
                u32::from(self.has_senders(p)),
                u32::from(q.pd_request.is_some()),
                u32::from(q.ns_request.is_some()),
                0,
            ],
            MACH_PORT_DNREQUESTS_SIZE => vec![0],
            _ => return Err(KERN_INVALID_ARGUMENT),
        };
        if (count as usize) < words.len() {
            return Err(KERN_FAILURE);
        }
        Ok(words)
    }

    /// mach_port_set_attributes: only the queue limit can be set. Raising
    /// it lets waiting senders through.
    pub fn port_set_attributes(&mut self, task: TaskId, name: u32, flavor: u32, info: &[u32]) -> (u32, Vec<ThreadId>) {
        let mut woken = Vec::new();
        let p = match self.receive_right(task, name) {
            Ok(p) => p,
            Err(kr) => return (kr, woken),
        };
        if flavor != MACH_PORT_LIMITS_INFO {
            return (KERN_INVALID_ARGUMENT, woken);
        }
        let Some(&limit) = info.first() else { return (KERN_FAILURE, woken) };
        if limit > MACH_PORT_QLIMIT_MAX {
            return (KERN_INVALID_VALUE, woken);
        }
        self.ports.get_mut(&p).expect("named port exists").qlimit = limit as usize;
        self.send_waiters.wake_all(p, || super::WakeReason::Ready, &mut |t| woken.push(t));
        (KERN_SUCCESS, woken)
    }

    /// The port set `name` names (`KERN_INVALID_RIGHT` for anything else).
    fn set_of(&self, task: TaskId, name: u32) -> Result<super::SetId, u32> {
        if name == MACH_PORT_NULL {
            return Err(KERN_INVALID_RIGHT);
        }
        match self.entry(task, name).ok_or(KERN_INVALID_NAME)?.obj {
            Obj::Set(s) => Ok(s),
            _ => Err(KERN_INVALID_RIGHT),
        }
    }

    fn member_of(&self, task: TaskId, name: u32) -> Result<PortId, u32> {
        if name == MACH_PORT_NULL {
            return Err(KERN_INVALID_RIGHT);
        }
        self.receive_right(task, name)
    }

    /// mach_port_insert_member: put a receive right into a set (a port may
    /// be in several).
    pub fn port_insert_member(&mut self, task: TaskId, name: u32, set: u32) -> u32 {
        let (p, s) = match (self.member_of(task, name), self.set_of(task, set)) {
            (Ok(p), Ok(s)) => (p, s),
            (Err(kr), _) | (_, Err(kr)) => return kr,
        };
        let q = self.ports.get_mut(&p).expect("named port exists");
        if !q.sets.insert(s) {
            return KERN_ALREADY_IN_SET;
        }
        self.sets.get_mut(&s).expect("named set exists").members.insert(p);
        KERN_SUCCESS
    }

    /// mach_port_extract_member: take a receive right out of one set.
    pub fn port_extract_member(&mut self, task: TaskId, name: u32, set: u32) -> u32 {
        let (p, s) = match (self.member_of(task, name), self.set_of(task, set)) {
            (Ok(p), Ok(s)) => (p, s),
            (Err(kr), _) | (_, Err(kr)) => return kr,
        };
        if !self.ports.get_mut(&p).expect("named port exists").sets.remove(&s) {
            return KERN_NOT_IN_SET;
        }
        self.sets.get_mut(&s).expect("named set exists").members.remove(&p);
        KERN_SUCCESS
    }

    /// mach_port_get_set_status: the names of a set's members in `task`.
    pub fn port_set_members(&self, task: TaskId, set: u32) -> Result<Vec<u32>, u32> {
        let s = self.set_of(task, set)?;
        let space = &self.spaces[&task];
        Ok(self.sets[&s].members.iter().filter_map(|p| space.by_port.get(p).copied()).collect())
    }

    /// mach_port_peek: look at the queued message with sequence number
    /// `seqno` (the first one for 0) without taking it. Only the null
    /// trailer is given (`trailer_elements` must be 0, `trailer_count`
    /// room for it).
    pub fn port_peek(
        &self,
        task: TaskId,
        name: u32,
        trailer_elements: u32,
        seqno: u32,
        trailer_count: u32,
    ) -> Result<Peeked, u32> {
        let p = self.receive_right(task, name)?;
        if trailer_count == 0 || trailer_elements != 0 {
            return Err(KERN_INVALID_VALUE);
        }
        let q = &self.ports[&p];
        let index = if seqno == 0 { 0 } else { seqno.wrapping_sub(q.seqno) as usize };
        let m = q.queue.get(index).ok_or(KERN_FAILURE)?;
        Ok(Peeked { seqno: q.seqno.wrapping_add(index as u32), size: m.size + 8, id: m.id })
    }

    /// mach_port_extract_right: the right `msgt` makes of `name`, as a
    /// message would carry it, back in `task`: its name and its type.
    pub fn port_extract_right(&mut self, task: TaskId, name: u32, msgt: u32) -> Result<(u32, u32), u32> {
        // A type that is no right, and COPY_RECEIVE, are invalid values.
        if !(MACH_MSG_TYPE_MOVE_RECEIVE..=MACH_MSG_TYPE_MAKE_SEND_ONCE).contains(&msgt) {
            return Err(KERN_INVALID_VALUE);
        }
        if name == MACH_PORT_NULL {
            return Err(KERN_INVALID_RIGHT);
        }
        let e = *self.entry(task, name).ok_or(KERN_INVALID_NAME)?;
        let Obj::Port(p) = e.obj else { return Err(KERN_INVALID_RIGHT) };
        match msgt {
            MACH_MSG_TYPE_MAKE_SEND if e.receive => {
                self.ports.get_mut(&p).expect("named port exists").mscount += 1;
                Ok((self.add_send(task, p, 1), MACH_MSG_TYPE_PORT_SEND))
            }
            MACH_MSG_TYPE_COPY_SEND if e.send > 0 => Ok((self.add_send(task, p, 1), MACH_MSG_TYPE_PORT_SEND)),
            MACH_MSG_TYPE_MOVE_SEND if e.send > 0 => Ok((name, MACH_MSG_TYPE_PORT_SEND)),
            MACH_MSG_TYPE_MAKE_SEND_ONCE if e.receive => {
                self.ports.get_mut(&p).expect("named port exists").sorights += 1;
                Ok((self.copyout(task, Carried::SendOnce(p)), MACH_MSG_TYPE_PORT_SEND_ONCE))
            }
            MACH_MSG_TYPE_MOVE_SEND_ONCE if e.send_once > 0 => {
                let ent = self.entry_mut(task, name).expect("present");
                ent.send_once -= 1;
                if ent.is_empty() {
                    self.remove_name(task, name);
                }
                Ok((self.copyout(task, Carried::SendOnce(p)), MACH_MSG_TYPE_PORT_SEND_ONCE))
            }
            MACH_MSG_TYPE_MOVE_RECEIVE if e.receive => Ok((name, MACH_MSG_TYPE_PORT_RECEIVE)),
            _ => Err(KERN_INVALID_RIGHT),
        }
    }

    // ----- notifications -----

    /// Whether any send right to `p` exists: in a name space, in a queued
    /// message, or kept by the kernel (an exception handler, a special
    /// port, a bootstrap service).
    pub(super) fn has_senders(&self, p: PortId) -> bool {
        let in_space = self.spaces.values().any(|s| s.by_port.get(&p).is_some_and(|n| s.entries[n].send > 0));
        let carries = |m: &Message| {
            [Some(m.dest), m.reply]
                .into_iter()
                .chain(m.ports.iter().map(|(_, r)| *r))
                .any(|r| r == Some(Carried::Send(p)))
        };
        let in_flight = self.ports.values().any(|q| q.queue.iter().any(carries));
        let handler = |t: &crate::exc::HandlerTable| t.0.iter().flatten().any(|h| h.port == p);
        let kernel = self.exc_task.values().any(handler)
            || self.exc_thread.values().any(handler)
            || handler(&self.exc_host)
            || self.kernel_holds(p);
        in_space || in_flight || kernel
    }

    /// Send a notification: a message with id `id` to the send-once right
    /// `to`, `body` after the header.
    pub(super) fn notify(&mut self, to: PortId, id: u32, body: Vec<u8>, ports: Vec<(usize, Option<Carried>)>) {
        let complex = !ports.is_empty();
        let m = Message {
            other_bits: if complex { MACH_MSGH_BITS_COMPLEX } else { 0 },
            id,
            size: (HEADER + body.len()) as u32,
            dest: Carried::SendOnce(to),
            reply: None,
            body,
            ports,
            sender: SenderInfo::default(),
        };
        if let Some(t) = self.deliver(to, m) {
            self.woken.push(t);
        }
    }

    /// A notification with an NDR record and one 32-bit value.
    pub(super) fn notify_value(&mut self, to: PortId, id: u32, value: u32) {
        let mut body = NDR.to_vec();
        body.extend_from_slice(&value.to_le_bytes());
        self.notify(to, id, body, Vec::new());
    }

    /// Send the no-senders notifications that are due: for each request
    /// on a port that has had senders and now has none. Run after every
    /// call ([`Ipc::take_woken`]).
    pub(super) fn check_no_senders(&mut self) {
        let pending: Vec<PortId> = self.ports.iter().filter(|(_, q)| q.ns_request.is_some()).map(|(&p, _)| p).collect();
        for p in pending {
            let senders = self.has_senders(p);
            let q = self.ports.get_mut(&p).expect("listed");
            let r = q.ns_request.as_mut().expect("listed");
            if senders {
                r.armed = true;
            } else if r.armed {
                let notify = r.notify;
                q.ns_request = None;
                let mscount = q.mscount;
                self.notify_value(notify, MACH_NOTIFY_NO_SENDERS, mscount);
            }
        }
    }

    /// mach_port_request_notification: ask for notification `id` about
    /// `name`, sent to the right `disp` makes of `notify` (`MACH_PORT_NULL`
    /// cancels). Returns the previous request's send-once right, now named
    /// in `task` (`MACH_PORT_NULL` if none).
    pub fn port_request_notification(
        &mut self,
        task: TaskId,
        name: u32,
        id: u32,
        sync: u32,
        notify: u32,
        disp: u32,
    ) -> Result<u32, u32> {
        if notify != MACH_PORT_NULL && !matches!(disp, MACH_MSG_TYPE_MAKE_SEND_ONCE | MACH_MSG_TYPE_MOVE_SEND_ONCE) {
            // MIG's type check: the client asked for something that is not
            // a send-once right. MOVE_SEND still takes the right.
            if disp == MACH_MSG_TYPE_MOVE_SEND
                && let Some(e) = self.entry_mut(task, notify)
                && e.send > 0
            {
                e.send -= 1;
                if e.is_empty() {
                    self.remove_name(task, notify);
                }
            }
            return Err(MIG_TYPE_ERROR);
        }
        if !matches!(id, MACH_NOTIFY_NO_SENDERS | MACH_NOTIFY_DEAD_NAME | MACH_NOTIFY_PORT_DESTROYED) {
            return Err(KERN_INVALID_VALUE);
        }
        let e = *self.entry(task, name).ok_or(KERN_INVALID_NAME)?;
        if notify != MACH_PORT_NULL && self.entry(task, notify).is_none() {
            return Err(KERN_INVALID_NAME);
        }
        self.request(task, name, id, sync, e, |ipc| ipc.notify_right(task, notify))
    }

    /// The MIG form: the request carried the right already (`None` for
    /// `MACH_PORT_NULL`).
    fn request_with_right(
        &mut self,
        task: TaskId,
        name: u32,
        id: u32,
        sync: u32,
        r: Option<Carried>,
    ) -> Result<u32, u32> {
        let to = match r {
            None | Some(Carried::Dead) => None,
            Some(Carried::SendOnce(p)) => Some(p),
            Some(_) => return Err(MIG_TYPE_ERROR),
        };
        if !matches!(id, MACH_NOTIFY_NO_SENDERS | MACH_NOTIFY_DEAD_NAME | MACH_NOTIFY_PORT_DESTROYED) {
            return Err(KERN_INVALID_VALUE);
        }
        let e = *self.entry(task, name).ok_or(KERN_INVALID_NAME)?;
        self.request(task, name, id, sync, e, |_| Ok(to))
    }

    fn request(
        &mut self,
        task: TaskId,
        name: u32,
        id: u32,
        sync: u32,
        e: Entry,
        mut right: impl FnMut(&mut Ipc) -> Result<Option<PortId>, u32>,
    ) -> Result<u32, u32> {
        match id {
            MACH_NOTIFY_NO_SENDERS | MACH_NOTIFY_PORT_DESTROYED => {
                let p = self.receive_right(task, name)?;
                if id == MACH_NOTIFY_PORT_DESTROYED && sync != 0 {
                    return Err(KERN_INVALID_VALUE);
                }
                let to = right(self)?;
                let q = self.ports.get_mut(&p).expect("named port exists");
                let previous = if id == MACH_NOTIFY_NO_SENDERS {
                    let old = q.ns_request.take().map(|r| r.notify);
                    if let Some(to) = to {
                        q.ns_request = Some(NsRequest { notify: to, armed: true });
                        let (mscount, senders) = (q.mscount, self.has_senders(p));
                        let q = self.ports.get_mut(&p).expect("named port exists");
                        if !senders && mscount >= sync {
                            q.ns_request = None;
                            self.notify_value(to, MACH_NOTIFY_NO_SENDERS, mscount);
                        } else {
                            q.ns_request.as_mut().expect("just set").armed = senders;
                        }
                    }
                    old
                } else {
                    core::mem::replace(&mut q.pd_request, to)
                };
                Ok(previous.map_or(MACH_PORT_NULL, |o| self.copyout(task, Carried::SendOnce(o))))
            }
            _ => {
                if e.obj == Obj::Dead {
                    // The right given is used up at once.
                    if let Some(to) = right(self)? {
                        self.notify(to, MACH_NOTIFY_SEND_ONCE, Vec::new(), Vec::new());
                    }
                    return Err(KERN_INVALID_ARGUMENT);
                }
                if !matches!(e.obj, Obj::Port(_)) {
                    return Err(KERN_INVALID_RIGHT);
                }
                let to = right(self)?;
                let space = self.spaces.get_mut(&task).expect("entry found");
                let old = match to {
                    Some(to) => space.dn_requests.insert(name, to),
                    None => space.dn_requests.remove(&name),
                };
                Ok(old.map_or(MACH_PORT_NULL, |o| self.copyout(task, Carried::SendOnce(o))))
            }
        }
    }

    /// The send-once right a notification request makes of `notify`
    /// (`None` for `MACH_PORT_NULL`).
    fn notify_right(&mut self, task: TaskId, notify: u32) -> Result<Option<PortId>, u32> {
        if notify == MACH_PORT_NULL {
            return Ok(None);
        }
        let e = *self.entry(task, notify).ok_or(KERN_INVALID_NAME)?;
        let Obj::Port(p) = e.obj else { return Err(KERN_INVALID_RIGHT) };
        if e.receive {
            self.ports.get_mut(&p).expect("named port exists").sorights += 1;
            return Ok(Some(p));
        }
        if e.send_once > 0 {
            let ent = self.entry_mut(task, notify).expect("present");
            ent.send_once -= 1;
            if ent.is_empty() {
                self.remove_name(task, notify);
            }
            return Ok(Some(p));
        }
        Err(KERN_INVALID_RIGHT)
    }

    /// The receive right of `p` would be destroyed: with a port-destroyed
    /// request it goes to the notify port instead. `true` if it did.
    pub(super) fn port_destroyed_request(&mut self, p: PortId) -> bool {
        let Some(q) = self.ports.get_mut(&p) else { return false };
        if q.kobject != KObject::None {
            return false;
        }
        let Some(to) = q.pd_request else { return false };
        if to == p || !self.ports.contains_key(&to) {
            return false;
        }
        let q = self.ports.get_mut(&p).expect("checked");
        q.pd_request = None;
        q.guard = None;
        q.context = 0;
        let holder = q.receiver.take();
        self.leave_set(p);
        if let Some(t) = holder
            && let Some(space) = self.spaces.get_mut(&t)
            && let Some(&n) = space.by_port.get(&p)
        {
            let e = space.entries.get_mut(&n).expect("by_port names an entry");
            e.receive = false;
            if e.is_empty() {
                space.entries.remove(&n);
                space.by_port.remove(&p);
            }
        }
        let mut body = 1u32.to_le_bytes().to_vec();
        body.extend_from_slice(&[0; 12]);
        self.notify(to, MACH_NOTIFY_PORT_DESTROYED, body, vec![(4, Some(Carried::Receive(p)))]);
        true
    }

    /// Serve a request of the `mach_port` MIG subsystem sent to `task`'s
    /// port by `caller`. The layouts are `mach/mach_port.h`'s, except
    /// mach_port_extract_right's and mach_port_get_set_status's replies,
    /// which are Poly's (see [`super::mig`]): an NDR record, the result,
    /// then the name and the type acquired, or the count and the names.
    pub(super) fn answer_port(&mut self, caller: TaskId, task: TaskId, req: &mut Message) -> Reply {
        let id = req.id;
        let b = req.body.clone();
        let rd = |at: usize| b.get(at..at + 4).map(|x| u32::from_le_bytes(x.try_into().expect("4 bytes")));
        let rd64 = |at: usize| b.get(at..at + 8).map(|x| u64::from_le_bytes(x.try_into().expect("8 bytes")));
        let reply = |ret: u32, extra: &[u32]| -> Reply {
            let mut out = NDR.to_vec();
            out.extend_from_slice(&ret.to_le_bytes());
            for w in extra {
                out.extend_from_slice(&w.to_le_bytes());
            }
            (id + 100, out, Vec::new(), false)
        };
        let simple = |ret: u32| reply(ret, &[]);
        let bad = || simple(super::mig::MIG_BAD_ARGUMENTS);
        let violation = |ipc: &mut Ipc, kr: u32| {
            if kr == POLY_GUARD_VIOLATION {
                ipc.violations.insert(caller);
            }
            kr
        };
        let name = rd(8);
        match id {
            MIG_DESTROY => {
                let Some(name) = name else { return bad() };
                let (kr, w) = self.port_destroy(task, name);
                self.woken.extend(w);
                simple(violation(self, kr))
            }
            MIG_GET_REFS => {
                let (Some(name), Some(right)) = (name, rd(12)) else { return bad() };
                match self.port_get_refs(task, name, right) {
                    Ok(n) => reply(KERN_SUCCESS, &[n]),
                    Err(kr) => simple(kr),
                }
            }
            MIG_PEEK => {
                let (Some(name), Some(tt), Some(seqno), Some(cnt)) = (name, rd(12), rd(16), rd(20)) else {
                    return bad();
                };
                let elements = (tt >> MACH_RCV_TRAILER_ELEMENTS_SHIFT) & 0xf;
                match self.port_peek(task, name, elements, seqno, cnt) {
                    Ok(pk) => reply(KERN_SUCCESS, &[pk.seqno, pk.size, pk.id, 8, MACH_MSG_TRAILER_FORMAT_0, 8]),
                    Err(kr) => simple(kr),
                }
            }
            MIG_SET_MSCOUNT => {
                let (Some(name), Some(n)) = (name, rd(12)) else { return bad() };
                simple(self.port_set_mscount(task, name, n))
            }
            MIG_GET_SET_STATUS => {
                let Some(name) = name else { return bad() };
                match self.port_set_members(task, name) {
                    Ok(names) => {
                        let mut extra = vec![names.len() as u32];
                        extra.extend(names);
                        reply(KERN_SUCCESS, &extra)
                    }
                    Err(kr) => simple(kr),
                }
            }
            MIG_REQUEST_NOTIFICATION => {
                let (Some(name), Some(msgid), Some(sync)) = (rd(24), rd(28), rd(32)) else { return bad() };
                let right = req.ports.first().and_then(|(_, r)| *r);
                if let Some(slot) = req.ports.first_mut() {
                    // The request keeps (or uses up) the right it carried.
                    if matches!(slot.1, Some(Carried::SendOnce(_))) {
                        slot.1 = None;
                    }
                }
                match self.request_with_right(task, name, msgid, sync, right) {
                    Ok(prev) => {
                        let mut out = 1u32.to_le_bytes().to_vec();
                        out.extend_from_slice(&[0; 12]);
                        // The previous right travels back as a send-once
                        // right: take it out of the space again.
                        let carried = self.take_send_once(task, prev);
                        (id + 100, out, vec![(4, carried)], true)
                    }
                    Err(kr) => simple(kr),
                }
            }
            MIG_EXTRACT_RIGHT => {
                let (Some(name), Some(msgt)) = (name, rd(12)) else { return bad() };
                match self.port_extract_right(task, name, msgt) {
                    Ok((n, t)) => reply(KERN_SUCCESS, &[n, t]),
                    Err(kr) => simple(kr),
                }
            }
            MIG_GET_ATTRIBUTES => {
                let (Some(name), Some(flavor), Some(cnt)) = (name, rd(12), rd(16)) else { return bad() };
                match self.port_get_attributes(task, name, flavor, cnt) {
                    Ok(words) => {
                        let mut extra = vec![words.len() as u32];
                        extra.extend(words);
                        reply(KERN_SUCCESS, &extra)
                    }
                    Err(kr) => simple(kr),
                }
            }
            MIG_SET_ATTRIBUTES => {
                let (Some(name), Some(flavor), Some(cnt)) = (name, rd(12), rd(16)) else { return bad() };
                let info: Vec<u32> = (0..cnt.min(17) as usize).filter_map(|i| rd(20 + 4 * i)).collect();
                let (kr, w) = self.port_set_attributes(task, name, flavor, &info);
                self.woken.extend(w);
                simple(kr)
            }
            MIG_INSERT_MEMBER | MIG_EXTRACT_MEMBER => {
                let (Some(name), Some(set)) = (name, rd(12)) else { return bad() };
                simple(if id == MIG_INSERT_MEMBER {
                    self.port_insert_member(task, name, set)
                } else {
                    self.port_extract_member(task, name, set)
                })
            }
            MIG_GET_CONTEXT => {
                let Some(name) = name else { return bad() };
                match self.port_get_context(task, name) {
                    Ok(c) => reply(KERN_SUCCESS, &[c as u32, (c >> 32) as u32]),
                    Err(kr) => simple(kr),
                }
            }
            MIG_SET_CONTEXT => {
                let (Some(name), Some(c)) = (name, rd64(12)) else { return bad() };
                simple(self.port_set_context(task, name, c))
            }
            MIG_DESTRUCT => {
                let (Some(name), Some(delta), Some(guard)) = (name, rd(12), rd64(16)) else { return bad() };
                let (kr, w) = self.port_destruct(task, name, delta as i32, guard);
                self.woken.extend(w);
                simple(violation(self, kr))
            }
            MIG_GUARD => {
                let (Some(name), Some(guard), Some(strict)) = (name, rd64(12), rd(20)) else { return bad() };
                simple(self.port_guard(task, name, guard, strict != 0))
            }
            MIG_UNGUARD => {
                let (Some(name), Some(guard)) = (name, rd64(12)) else { return bad() };
                let kr = self.port_unguard(task, name, guard);
                simple(violation(self, kr))
            }
            _ => simple(super::mig::MIG_BAD_ID),
        }
    }

    /// Take the send-once right `name` (as a request's reply carries it).
    fn take_send_once(&mut self, task: TaskId, name: u32) -> Option<Carried> {
        let e = *self.entry(task, name)?;
        let Obj::Port(p) = e.obj else { return None };
        if e.send_once == 0 {
            return None;
        }
        let ent = self.entry_mut(task, name).expect("present");
        ent.send_once -= 1;
        if ent.is_empty() {
            self.remove_name(task, name);
        }
        Some(Carried::SendOnce(p))
    }
}

const MIG_DESTROY: u32 = 3205;
const MIG_GET_REFS: u32 = 3207;
const MIG_PEEK: u32 = 3209;
const MIG_SET_MSCOUNT: u32 = 3210;
const MIG_GET_SET_STATUS: u32 = 3211;
const MIG_REQUEST_NOTIFICATION: u32 = 3213;
const MIG_EXTRACT_RIGHT: u32 = 3215;
const MIG_GET_ATTRIBUTES: u32 = 3217;
const MIG_SET_ATTRIBUTES: u32 = 3218;
const MIG_INSERT_MEMBER: u32 = 3226;
const MIG_EXTRACT_MEMBER: u32 = 3227;
const MIG_GET_CONTEXT: u32 = 3228;
const MIG_SET_CONTEXT: u32 = 3229;
const MIG_DESTRUCT: u32 = 3232;
const MIG_GUARD: u32 = 3233;
const MIG_UNGUARD: u32 = 3234;
