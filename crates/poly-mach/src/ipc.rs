//! Port name spaces, rights, port sets, message queues and semaphores.
//!
//! One [`Ipc`] holds the Mach state of the whole machine: every port, every
//! task's name space and the threads waiting on ports and semaphores. The
//! kernel keeps it behind one lock. Each call names the calling task; a task
//! port name in the arguments is resolved with [`Ipc::resolve_task`] first.
//!
//! # Model
//!
//! - A **port** is a kernel queue. Its receive right lives in at most one
//!   name space (or travels in a message). Kernel objects (a task, a thread,
//!   the host, a semaphore) are ports whose receive right the kernel keeps.
//! - A **name space** maps 32-bit names to rights: one name per port for its
//!   receive and send rights (send rights counted in user references), a name
//!   of its own for each send-once right, port sets, and dead names.
//! - When a port's receive right is destroyed, every send and send-once
//!   right to it, in every space, becomes a **dead name** keeping its user
//!   references; queued messages are destroyed; receivers waiting on it are
//!   woken with `MACH_RCV_PORT_DIED`.
//!
//! The results the trap spec sheets record (`specs/macos/mach/*.yaml`) are
//! kept: for example `MACH_PORT_NULL` and `MACH_PORT_DEAD` are no-ops for
//! deallocate, a receive right's delta must be -1 or 0, and a send right made
//! under the name of another port's receive right is `KERN_RIGHT_EXISTS`.

use crate::consts::{kern::*, msg::*, port::*};
use crate::waitq::{ThreadId, WaitQueue};
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;

/// A kernel task id (one per process).
pub type TaskId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SetId(u64);

/// What a port stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KObject {
    /// A port a program made: messages queue on it.
    None,
    Task(TaskId),
    Thread(ThreadId),
    Host,
    Semaphore,
}

/// Where a receiver waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RcvKey {
    Port(PortId),
    Set(SetId),
}

/// A right travelling in a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Carried {
    Send(PortId),
    SendOnce(PortId),
    Receive(PortId),
    /// A right to a port that died (or `MACH_PORT_DEAD` sent as is).
    Dead,
}

impl Carried {
    /// The `MACH_MSG_TYPE_PORT_*` value a receiver finds.
    fn received_type(self) -> u32 {
        match self {
            Carried::Send(_) | Carried::Dead => MACH_MSG_TYPE_PORT_SEND,
            Carried::SendOnce(_) => MACH_MSG_TYPE_PORT_SEND_ONCE,
            Carried::Receive(_) => MACH_MSG_TYPE_PORT_RECEIVE,
        }
    }
}

#[derive(Debug)]
struct Message {
    /// The header's bits without the port dispositions.
    other_bits: u32,
    id: u32,
    /// The size the sender gave (header included).
    size: u32,
    dest: Carried,
    reply: Option<Carried>,
    /// Everything after the header, as sent; port descriptors' names are
    /// rewritten on receive.
    body: Vec<u8>,
    /// Port descriptors: byte offset in `body`, and the right.
    ports: Vec<(usize, Option<Carried>)>,
    sender: SenderInfo,
}

/// Who sent a message, for its trailer: `security_token_t` (uid, gid) and
/// `audit_token_t` (eight words), as the kernel fills them in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SenderInfo {
    pub uid: u32,
    pub gid: u32,
    pub audit: [u32; 8],
}

#[derive(Debug)]
struct Port {
    kobject: KObject,
    /// The space holding the receive right; `None` for kernel objects and
    /// for a receive right in transit.
    receiver: Option<TaskId>,
    set: Option<SetId>,
    queue: VecDeque<Message>,
    seqno: u32,
    qlimit: usize,
    /// A semaphore's count.
    count: u32,
}

#[derive(Debug)]
struct PortSet {
    members: BTreeSet<PortId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Obj {
    Port(PortId),
    Set(SetId),
    Dead,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    obj: Obj,
    receive: bool,
    send: u32,
    send_once: u32,
    dead: u32,
}

impl Entry {
    fn new(obj: Obj) -> Entry {
        Entry { obj, receive: false, send: 0, send_once: 0, dead: 0 }
    }

    fn type_bits(&self) -> u32 {
        let mut t = 0;
        if self.send > 0 {
            t |= MACH_PORT_TYPE_SEND;
        }
        if self.receive {
            t |= MACH_PORT_TYPE_RECEIVE;
        }
        if self.send_once > 0 {
            t |= MACH_PORT_TYPE_SEND_ONCE;
        }
        if matches!(self.obj, Obj::Set(_)) {
            t |= MACH_PORT_TYPE_PORT_SET;
        }
        if matches!(self.obj, Obj::Dead) {
            t |= MACH_PORT_TYPE_DEAD_NAME;
        }
        t
    }

    fn is_empty(&self) -> bool {
        match self.obj {
            Obj::Port(_) => !self.receive && self.send == 0 && self.send_once == 0,
            Obj::Set(_) => false,
            Obj::Dead => self.dead == 0,
        }
    }
}

#[derive(Debug)]
struct Space {
    entries: BTreeMap<u32, Entry>,
    /// The name of a port's receive / send right (not of send-once rights,
    /// which get names of their own).
    by_port: BTreeMap<PortId, u32>,
    next_index: u32,
    task_port: PortId,
}

impl Space {
    fn alloc_name(&mut self) -> u32 {
        loop {
            self.next_index = (self.next_index + 1) & 0x00ff_ffff;
            if self.next_index == 0 {
                continue;
            }
            // The low byte is a generation number in Mach names; Poly keeps
            // it fixed, so a name is never MACH_PORT_DEAD.
            let name = (self.next_index << 8) | 3;
            if !self.entries.contains_key(&name) {
                return name;
            }
        }
    }
}

/// What a receive found.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// A message, laid out for the receiver (header, body, trailer).
    Message(Vec<u8>),
    /// The message (header plus asked-for trailer) needs `needed` bytes, more
    /// than the receiver offered. `header` is what to write to the
    /// receiver's buffer (`msgh_size` holds the size needed). With
    /// `MACH_RCV_LARGE` the message stays queued; without it it was
    /// destroyed.
    TooLarge { header: [u8; 24], needed: u32 },
    /// Nothing queued: wait on this key, or time out.
    Empty(RcvKey),
}

/// Why a send did not queue its message.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// A `mach_msg_return_t`.
    Return(u32),
    /// The destination's queue is full: wait on the port, or time out.
    Full(PortId),
    /// The destination is a kernel object (task, thread, host, semaphore):
    /// a MIG request for the kernel's servers, which Poly does not have yet.
    KernelObject(KObject),
}

/// How a thread's wait on a port, a set or a semaphore ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeReason {
    /// A message arrived (receive), a count was given (semaphore), room
    /// was made (send).
    Ready,
    /// The port or semaphore was destroyed.
    Died,
}

#[derive(Debug)]
pub struct Ipc {
    next_id: u64,
    ports: BTreeMap<PortId, Port>,
    sets: BTreeMap<SetId, PortSet>,
    spaces: BTreeMap<TaskId, Space>,
    thread_ports: BTreeMap<ThreadId, PortId>,
    host_port: PortId,
    rcv_waiters: WaitQueue<RcvKey, WakeReason>,
    send_waiters: WaitQueue<PortId, WakeReason>,
    sem_waiters: WaitQueue<PortId, WakeReason>,
}

impl Default for Ipc {
    fn default() -> Self {
        Self::new()
    }
}

const HEADER: usize = 24;
const PORT_DESCRIPTOR: usize = 12;

fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn wr32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

impl Ipc {
    pub fn new() -> Ipc {
        let mut ipc = Ipc {
            next_id: 1,
            ports: BTreeMap::new(),
            sets: BTreeMap::new(),
            spaces: BTreeMap::new(),
            thread_ports: BTreeMap::new(),
            host_port: PortId(0),
            rcv_waiters: WaitQueue::new(),
            send_waiters: WaitQueue::new(),
            sem_waiters: WaitQueue::new(),
        };
        ipc.host_port = ipc.new_port(KObject::Host, None);
        ipc
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn new_port(&mut self, kobject: KObject, receiver: Option<TaskId>) -> PortId {
        let id = PortId(self.new_id());
        self.ports.insert(
            id,
            Port {
                kobject,
                receiver,
                set: None,
                queue: VecDeque::new(),
                seqno: 0,
                qlimit: MACH_PORT_QLIMIT_DEFAULT,
                count: 0,
            },
        );
        id
    }

    // ----- tasks and threads -----

    /// Make the name space of a new task (a new process: at start, or the
    /// child of fork, which inherits no rights) with send rights to its own
    /// task port, its first thread's port and the host port.
    pub fn create_task(&mut self, task: TaskId, thread: ThreadId) {
        if self.spaces.contains_key(&task) {
            return;
        }
        let task_port = self.new_port(KObject::Task(task), None);
        // Names start at a different index in each task, so the names of
        // the self ports of a parent and its child differ.
        let next_index = 0x100 + ((task as u32).wrapping_mul(0x40) & 0xffff);
        self.spaces.insert(task, Space { entries: BTreeMap::new(), by_port: BTreeMap::new(), next_index, task_port });
        self.add_send(task, task_port, 1);
        let host = self.host_port;
        self.add_send(task, host, 1);
        self.create_thread(task, thread);
    }

    /// A new thread of `task`, with a send right to its port in the task.
    pub fn create_thread(&mut self, task: TaskId, thread: ThreadId) {
        if self.thread_ports.contains_key(&thread) || !self.spaces.contains_key(&task) {
            return;
        }
        let p = self.new_port(KObject::Thread(thread), None);
        self.thread_ports.insert(thread, p);
        self.add_send(task, p, 1);
    }

    /// A thread ended: its port dies (send rights to it become dead names).
    pub fn destroy_thread(&mut self, thread: ThreadId) -> Vec<ThreadId> {
        let mut woken = Vec::new();
        self.cancel_waits(thread);
        if let Some(p) = self.thread_ports.remove(&thread) {
            self.destroy_port(p, &mut woken);
        }
        woken
    }

    /// A task ended (exit or exec): every right in its space goes, its
    /// receive rights' ports die, its task port dies. Returns the threads
    /// woken by that (receivers whose ports died).
    pub fn destroy_task(&mut self, task: TaskId) -> Vec<ThreadId> {
        let mut woken = Vec::new();
        let Some(space) = self.spaces.get(&task) else { return woken };
        let task_port = space.task_port;
        let entries: Vec<(u32, Entry)> = space.entries.iter().map(|(&n, &e)| (n, e)).collect();
        for (_, e) in &entries {
            match e.obj {
                Obj::Port(p) if e.receive => self.destroy_port(p, &mut woken),
                Obj::Set(s) => self.destroy_set(s),
                _ => {}
            }
        }
        // Semaphores belong to the task that made them.
        let sems: Vec<PortId> = entries
            .iter()
            .filter_map(|(_, e)| match e.obj {
                Obj::Port(p) if self.ports.get(&p).is_some_and(|q| q.kobject == KObject::Semaphore) => Some(p),
                _ => None,
            })
            .collect();
        for p in sems {
            self.destroy_port(p, &mut woken);
        }
        self.spaces.remove(&task);
        self.destroy_port(task_port, &mut woken);
        woken
    }

    /// Whether `task` has a name space.
    pub fn has_task(&self, task: TaskId) -> bool {
        self.spaces.contains_key(&task)
    }

    /// The task a task port name in `caller`'s space stands for: a send
    /// right to a task's port. Anything else is `MACH_SEND_INVALID_DEST`
    /// (what the traps return for a task argument that is not a task).
    pub fn resolve_task(&self, caller: TaskId, name: u32) -> Result<TaskId, u32> {
        let e = self.entry(caller, name).ok_or(MACH_SEND_INVALID_DEST)?;
        match e.obj {
            Obj::Port(p) if e.send > 0 => match self.ports.get(&p).map(|p| p.kobject) {
                Some(KObject::Task(t)) if self.spaces.contains_key(&t) => Ok(t),
                _ => Err(MACH_SEND_INVALID_DEST),
            },
            _ => Err(MACH_SEND_INVALID_DEST),
        }
    }

    fn entry(&self, task: TaskId, name: u32) -> Option<&Entry> {
        self.spaces.get(&task)?.entries.get(&name)
    }

    fn entry_mut(&mut self, task: TaskId, name: u32) -> Option<&mut Entry> {
        self.spaces.get_mut(&task)?.entries.get_mut(&name)
    }

    /// Add `refs` send references to `port` in `task`'s space; its name.
    fn add_send(&mut self, task: TaskId, port: PortId, refs: u32) -> u32 {
        let space = self.spaces.get_mut(&task).expect("space exists");
        if let Some(&name) = space.by_port.get(&port) {
            let e = space.entries.get_mut(&name).expect("by_port names an entry");
            e.send = (e.send + refs).min(MACH_PORT_UREFS_MAX);
            return name;
        }
        let name = space.alloc_name();
        let mut e = Entry::new(Obj::Port(port));
        e.send = refs;
        space.entries.insert(name, e);
        space.by_port.insert(port, name);
        name
    }

    fn remove_name(&mut self, task: TaskId, name: u32) {
        if let Some(space) = self.spaces.get_mut(&task)
            && let Some(e) = space.entries.remove(&name)
            && let Obj::Port(p) = e.obj
            && space.by_port.get(&p) == Some(&name)
        {
            space.by_port.remove(&p);
        }
    }

    // ----- the self traps -----

    fn self_name(&mut self, task: TaskId, port: PortId) -> u32 {
        if !self.spaces.contains_key(&task) || !self.ports.contains_key(&port) {
            return MACH_PORT_NULL;
        }
        // Each call hands out a reference, as mach_task_self() does.
        self.add_send(task, port, 1)
    }

    /// task_self_trap: the name of the task's own port.
    pub fn task_self(&mut self, task: TaskId) -> u32 {
        let Some(p) = self.spaces.get(&task).map(|s| s.task_port) else { return MACH_PORT_NULL };
        self.self_name(task, p)
    }

    /// thread_self_trap: the name of the calling thread's port.
    pub fn thread_self(&mut self, task: TaskId, thread: ThreadId) -> u32 {
        let Some(&p) = self.thread_ports.get(&thread) else { return MACH_PORT_NULL };
        self.self_name(task, p)
    }

    /// host_self_trap: the name of the host port.
    pub fn host_self(&mut self, task: TaskId) -> u32 {
        let p = self.host_port;
        self.self_name(task, p)
    }

    /// mach_reply_port: a new receive right.
    pub fn reply_port(&mut self, task: TaskId) -> u32 {
        self.port_allocate(task, MACH_PORT_RIGHT_RECEIVE).unwrap_or(MACH_PORT_NULL)
    }

    // ----- port traps -----

    /// port_allocate: a new receive right, port set or dead name.
    pub fn port_allocate(&mut self, task: TaskId, right: u32) -> Result<u32, u32> {
        if !self.spaces.contains_key(&task) {
            return Err(MACH_SEND_INVALID_DEST);
        }
        let obj = match right {
            MACH_PORT_RIGHT_RECEIVE => Obj::Port(self.new_port(KObject::None, Some(task))),
            MACH_PORT_RIGHT_PORT_SET => {
                let s = SetId(self.new_id());
                self.sets.insert(s, PortSet { members: BTreeSet::new() });
                Obj::Set(s)
            }
            MACH_PORT_RIGHT_DEAD_NAME => Obj::Dead,
            _ => return Err(KERN_INVALID_VALUE),
        };
        let space = self.spaces.get_mut(&task).expect("checked");
        let name = space.alloc_name();
        let mut e = Entry::new(obj);
        match obj {
            Obj::Port(p) => {
                e.receive = true;
                space.by_port.insert(p, name);
            }
            Obj::Dead => e.dead = 1,
            Obj::Set(_) => {}
        }
        space.entries.insert(name, e);
        Ok(name)
    }

    /// port_deallocate: give up one user reference of a send right, a
    /// send-once right or a dead name.
    pub fn port_deallocate(&mut self, task: TaskId, name: u32) -> u32 {
        if name == MACH_PORT_NULL || name == MACH_PORT_DEAD {
            return KERN_SUCCESS;
        }
        let Some(e) = self.entry_mut(task, name) else { return KERN_INVALID_NAME };
        if e.send > 0 {
            e.send -= 1;
        } else if e.send_once > 0 {
            e.send_once -= 1;
        } else if e.obj == Obj::Dead {
            e.dead -= 1;
        } else {
            return KERN_INVALID_RIGHT;
        }
        if e.is_empty() {
            self.remove_name(task, name);
        }
        KERN_SUCCESS
    }

    /// port_mod_refs: change the user references of one kind of right.
    pub fn port_mod_refs(&mut self, task: TaskId, name: u32, right: u32, delta: i32) -> (u32, Vec<ThreadId>) {
        let mut woken = Vec::new();
        let kr = self.mod_refs(task, name, right, delta, &mut woken);
        (kr, woken)
    }

    fn mod_refs(&mut self, task: TaskId, name: u32, right: u32, delta: i32, woken: &mut Vec<ThreadId>) -> u32 {
        if right > MACH_PORT_RIGHT_DEAD_NAME {
            return KERN_INVALID_VALUE;
        }
        if name == MACH_PORT_NULL || name == MACH_PORT_DEAD {
            return KERN_SUCCESS;
        }
        let Some(&e) = self.entry(task, name) else { return KERN_INVALID_NAME };
        let urefs = |have: u32| -> Result<u32, u32> {
            let n = i64::from(have) + i64::from(delta);
            if n < 0 {
                Err(KERN_INVALID_VALUE)
            } else if n > i64::from(MACH_PORT_UREFS_MAX) {
                Err(KERN_UREFS_OVERFLOW)
            } else {
                Ok(n as u32)
            }
        };
        match right {
            MACH_PORT_RIGHT_RECEIVE => {
                let Obj::Port(p) = e.obj else { return KERN_INVALID_RIGHT };
                if !e.receive {
                    return KERN_INVALID_RIGHT;
                }
                match delta {
                    0 => KERN_SUCCESS,
                    -1 => {
                        self.destroy_port(p, woken);
                        KERN_SUCCESS
                    }
                    _ => KERN_INVALID_VALUE,
                }
            }
            MACH_PORT_RIGHT_SEND => {
                if e.send == 0 || !matches!(e.obj, Obj::Port(_)) {
                    return KERN_INVALID_RIGHT;
                }
                match urefs(e.send) {
                    Ok(n) => {
                        let ent = self.entry_mut(task, name).expect("present");
                        ent.send = n;
                        if ent.is_empty() {
                            self.remove_name(task, name);
                        }
                        KERN_SUCCESS
                    }
                    Err(kr) => kr,
                }
            }
            MACH_PORT_RIGHT_SEND_ONCE => {
                if e.send_once == 0 {
                    return KERN_INVALID_RIGHT;
                }
                match delta {
                    0 => KERN_SUCCESS,
                    -1 => {
                        let ent = self.entry_mut(task, name).expect("present");
                        ent.send_once -= 1;
                        if ent.is_empty() {
                            self.remove_name(task, name);
                        }
                        KERN_SUCCESS
                    }
                    _ => KERN_INVALID_VALUE,
                }
            }
            MACH_PORT_RIGHT_PORT_SET => {
                let Obj::Set(s) = e.obj else { return KERN_INVALID_RIGHT };
                match delta {
                    0 => KERN_SUCCESS,
                    -1 => {
                        self.destroy_set(s);
                        self.remove_name(task, name);
                        KERN_SUCCESS
                    }
                    _ => KERN_INVALID_VALUE,
                }
            }
            _ => {
                if e.obj != Obj::Dead {
                    return KERN_INVALID_RIGHT;
                }
                match urefs(e.dead) {
                    Ok(0) => {
                        self.remove_name(task, name);
                        KERN_SUCCESS
                    }
                    Ok(n) => {
                        self.entry_mut(task, name).expect("present").dead = n;
                        KERN_SUCCESS
                    }
                    Err(kr) => kr,
                }
            }
        }
    }

    /// port_type: the rights `name` holds, as `MACH_PORT_TYPE_*` bits.
    pub fn port_type(&self, task: TaskId, name: u32) -> Result<u32, u32> {
        if name == MACH_PORT_DEAD {
            return Ok(MACH_PORT_TYPE_DEAD_NAME);
        }
        if name == MACH_PORT_NULL {
            return Err(KERN_INVALID_NAME);
        }
        self.entry(task, name).map(Entry::type_bits).ok_or(KERN_INVALID_NAME)
    }

    /// port_insert_right: give `task` the right `msgt` makes of `port_name`
    /// (a name in the caller's space `caller`), under `name`.
    pub fn port_insert_right(&mut self, caller: TaskId, task: TaskId, name: u32, port_name: u32, msgt: u32) -> u32 {
        if !(MACH_MSG_TYPE_MOVE_RECEIVE..=MACH_MSG_TYPE_MAKE_SEND_ONCE).contains(&msgt) {
            return KERN_INVALID_RIGHT;
        }
        if name == MACH_PORT_NULL || name == MACH_PORT_DEAD {
            return KERN_INVALID_VALUE;
        }
        let Some(&src) = self.entry(caller, port_name) else { return KERN_INVALID_NAME };
        let port = match src.obj {
            Obj::Port(p) => p,
            _ => return KERN_INVALID_RIGHT,
        };
        let held = match msgt {
            MACH_MSG_TYPE_MAKE_SEND | MACH_MSG_TYPE_MAKE_SEND_ONCE | MACH_MSG_TYPE_MOVE_RECEIVE => src.receive,
            MACH_MSG_TYPE_COPY_SEND | MACH_MSG_TYPE_MOVE_SEND => src.send > 0,
            _ => src.send_once > 0,
        };
        if !held {
            return KERN_INVALID_RIGHT;
        }
        // Where the right goes: the name must be free, or already name this
        // port (send and receive rights share one name per port).
        let space = self.spaces.get(&task).expect("resolved task");
        let once = matches!(msgt, MACH_MSG_TYPE_MAKE_SEND_ONCE | MACH_MSG_TYPE_MOVE_SEND_ONCE);
        match space.entries.get(&name) {
            Some(e) if once || e.obj != Obj::Port(port) => return KERN_RIGHT_EXISTS,
            Some(e) if msgt == MACH_MSG_TYPE_MOVE_RECEIVE && e.receive => return KERN_RIGHT_EXISTS,
            None if !once && space.by_port.get(&port).is_some_and(|&n| n != name) => return KERN_RIGHT_EXISTS,
            _ => {}
        }
        // Take the right from the source.
        let carried = match msgt {
            MACH_MSG_TYPE_MOVE_SEND => {
                let e = self.entry_mut(caller, port_name).expect("present");
                e.send -= 1;
                if e.is_empty() {
                    self.remove_name(caller, port_name);
                }
                Carried::Send(port)
            }
            MACH_MSG_TYPE_MOVE_SEND_ONCE => {
                let e = self.entry_mut(caller, port_name).expect("present");
                e.send_once -= 1;
                if e.is_empty() {
                    self.remove_name(caller, port_name);
                }
                Carried::SendOnce(port)
            }
            MACH_MSG_TYPE_MOVE_RECEIVE => {
                let e = self.entry_mut(caller, port_name).expect("present");
                e.receive = false;
                if e.is_empty() {
                    self.remove_name(caller, port_name);
                }
                self.leave_set(port);
                Carried::Receive(port)
            }
            MACH_MSG_TYPE_MAKE_SEND_ONCE => Carried::SendOnce(port),
            _ => Carried::Send(port),
        };
        let space = self.spaces.get_mut(&task).expect("resolved task");
        let e = space.entries.entry(name).or_insert_with(|| Entry::new(Obj::Port(port)));
        match carried {
            Carried::Send(_) => e.send = (e.send + 1).min(MACH_PORT_UREFS_MAX),
            Carried::SendOnce(_) => e.send_once += 1,
            Carried::Receive(_) => e.receive = true,
            Carried::Dead => {}
        }
        if !once {
            space.by_port.insert(port, name);
        }
        if let Carried::Receive(p) = carried
            && let Some(q) = self.ports.get_mut(&p)
        {
            q.receiver = Some(task);
        }
        KERN_SUCCESS
    }

    /// port_move_member (trap 20): put the receive right `member` into the
    /// port set `after`, or out of any set when `after` is `MACH_PORT_NULL`.
    pub fn port_move_member(&mut self, task: TaskId, member: u32, after: u32) -> u32 {
        let Some(&m) = self.entry(task, member) else { return KERN_INVALID_NAME };
        let (Obj::Port(p), true) = (m.obj, m.receive) else { return KERN_INVALID_RIGHT };
        let target = if after == MACH_PORT_NULL {
            None
        } else {
            let Some(&s) = self.entry(task, after) else { return KERN_INVALID_NAME };
            let Obj::Set(s) = s.obj else { return KERN_INVALID_RIGHT };
            Some(s)
        };
        let before = self.ports.get(&p).and_then(|q| q.set);
        if target.is_none() && before.is_none() {
            return KERN_NOT_IN_SET;
        }
        self.leave_set(p);
        if let Some(s) = target {
            self.sets.get_mut(&s).expect("named set exists").members.insert(p);
            self.ports.get_mut(&p).expect("named port exists").set = Some(s);
        }
        KERN_SUCCESS
    }

    fn leave_set(&mut self, p: PortId) {
        if let Some(q) = self.ports.get_mut(&p)
            && let Some(s) = q.set.take()
            && let Some(set) = self.sets.get_mut(&s)
        {
            set.members.remove(&p);
        }
    }

    fn destroy_set(&mut self, s: SetId) {
        if let Some(set) = self.sets.remove(&s) {
            for p in set.members {
                if let Some(q) = self.ports.get_mut(&p) {
                    q.set = None;
                }
            }
        }
        // Receivers waiting on the set: their names are gone.
        self.rcv_waiters.wake_all(RcvKey::Set(s), || WakeReason::Died, &mut |_| {});
    }

    /// The port is destroyed: rights to it become dead names everywhere,
    /// its messages are destroyed, its waiters are woken.
    fn destroy_port(&mut self, p: PortId, woken: &mut Vec<ThreadId>) {
        self.leave_set(p);
        let Some(port) = self.ports.remove(&p) else { return };
        for space in self.spaces.values_mut() {
            if let Some(name) = space.by_port.remove(&p) {
                let e = space.entries.get_mut(&name).expect("by_port names an entry");
                e.receive = false;
                if e.send > 0 {
                    *e = Entry { obj: Obj::Dead, receive: false, send: 0, send_once: 0, dead: e.send };
                } else if e.send_once == 0 {
                    space.entries.remove(&name);
                }
            }
            for e in space.entries.values_mut() {
                if e.obj == Obj::Port(p) && e.send_once > 0 {
                    *e = Entry { obj: Obj::Dead, receive: false, send: 0, send_once: 0, dead: e.send_once };
                }
            }
        }
        for m in port.queue {
            self.destroy_message(m, woken);
        }
        self.rcv_waiters.wake_all(RcvKey::Port(p), || WakeReason::Died, &mut |t| woken.push(t));
        self.send_waiters.wake_all(p, || WakeReason::Died, &mut |t| woken.push(t));
        self.sem_waiters.wake_all(p, || WakeReason::Died, &mut |t| woken.push(t));
    }

    fn destroy_message(&mut self, m: Message, woken: &mut Vec<ThreadId>) {
        let rights = [Some(m.dest), m.reply].into_iter().chain(m.ports.into_iter().map(|(_, r)| r));
        for r in rights.flatten() {
            if let Carried::Receive(p) = r {
                self.destroy_port(p, woken);
            }
        }
    }

    // ----- messages -----

    /// Copy a right named in a message into the message: `name` with
    /// disposition `disp` in `task`'s space.
    fn copyin(&mut self, task: TaskId, name: u32, disp: u32) -> Result<Carried, ()> {
        if name == MACH_PORT_DEAD {
            return Ok(Carried::Dead);
        }
        let e = *self.entry(task, name).ok_or(())?;
        if e.obj == Obj::Dead && matches!(disp, MACH_MSG_TYPE_COPY_SEND | MACH_MSG_TYPE_MOVE_SEND) {
            if disp == MACH_MSG_TYPE_MOVE_SEND {
                let ent = self.entry_mut(task, name).expect("present");
                ent.dead -= 1;
                if ent.is_empty() {
                    self.remove_name(task, name);
                }
            }
            return Ok(Carried::Dead);
        }
        let Obj::Port(p) = e.obj else { return Err(()) };
        let carried = match disp {
            MACH_MSG_TYPE_COPY_SEND if e.send > 0 => Carried::Send(p),
            MACH_MSG_TYPE_MOVE_SEND if e.send > 0 => {
                self.entry_mut(task, name).expect("present").send -= 1;
                Carried::Send(p)
            }
            MACH_MSG_TYPE_MAKE_SEND if e.receive => Carried::Send(p),
            MACH_MSG_TYPE_MAKE_SEND_ONCE if e.receive => Carried::SendOnce(p),
            MACH_MSG_TYPE_MOVE_SEND_ONCE if e.send_once > 0 => {
                self.entry_mut(task, name).expect("present").send_once -= 1;
                Carried::SendOnce(p)
            }
            MACH_MSG_TYPE_MOVE_RECEIVE if e.receive => {
                self.entry_mut(task, name).expect("present").receive = false;
                self.leave_set(p);
                if let Some(q) = self.ports.get_mut(&p) {
                    q.receiver = None;
                }
                Carried::Receive(p)
            }
            _ => return Err(()),
        };
        if self.entry(task, name).is_some_and(Entry::is_empty) {
            self.remove_name(task, name);
        }
        Ok(carried)
    }

    /// Put a carried right into `task`'s space; its name there.
    fn copyout(&mut self, task: TaskId, r: Carried) -> u32 {
        let alive = |ipc: &Ipc, p: PortId| ipc.ports.contains_key(&p);
        match r {
            Carried::Dead => MACH_PORT_DEAD,
            Carried::Send(p) if !alive(self, p) => MACH_PORT_DEAD,
            Carried::Send(p) => self.add_send(task, p, 1),
            Carried::SendOnce(p) if !alive(self, p) => MACH_PORT_DEAD,
            Carried::SendOnce(p) => {
                let space = self.spaces.get_mut(&task).expect("space exists");
                let name = space.alloc_name();
                let mut e = Entry::new(Obj::Port(p));
                e.send_once = 1;
                space.entries.insert(name, e);
                name
            }
            Carried::Receive(p) => {
                let space = self.spaces.get_mut(&task).expect("space exists");
                let name = match space.by_port.get(&p) {
                    Some(&n) => n,
                    None => {
                        let n = space.alloc_name();
                        space.entries.insert(n, Entry::new(Obj::Port(p)));
                        space.by_port.insert(p, n);
                        n
                    }
                };
                space.entries.get_mut(&name).expect("just made").receive = true;
                if let Some(q) = self.ports.get_mut(&p) {
                    q.receiver = Some(task);
                }
                name
            }
        }
    }

    /// Send the message `msg` (header first, `msgh_size` = its length) from
    /// `task`. On success returns the receiver woken, if any. A message to
    /// a full queue is not queued ([`SendError::Full`]); the caller waits
    /// for room (or times out) and sends again.
    pub fn send(&mut self, task: TaskId, msg: &[u8], sender: SenderInfo) -> Result<Option<ThreadId>, SendError> {
        use SendError::Return;
        if msg.len() < HEADER || !msg.len().is_multiple_of(4) {
            return Err(Return(MACH_SEND_MSG_TOO_SMALL));
        }
        let bits = rd32(msg, 0);
        let (remote, local) = (rd32(msg, 8), rd32(msg, 12));
        let rdisp = bits & MACH_MSGH_BITS_REMOTE_MASK;
        let ldisp = (bits & MACH_MSGH_BITS_LOCAL_MASK) >> 8;
        if !(MACH_MSG_TYPE_MOVE_SEND..=MACH_MSG_TYPE_MAKE_SEND_ONCE).contains(&rdisp) {
            return Err(Return(MACH_SEND_INVALID_HEADER));
        }
        if ldisp != 0 && !(MACH_MSG_TYPE_MOVE_SEND..=MACH_MSG_TYPE_MAKE_SEND_ONCE).contains(&ldisp) {
            return Err(Return(MACH_SEND_INVALID_HEADER));
        }
        // The destination.
        let dest_port = match self.entry(task, remote) {
            Some(e) => match e.obj {
                Obj::Port(p) => p,
                _ => return Err(Return(MACH_SEND_INVALID_DEST)),
            },
            None => return Err(Return(MACH_SEND_INVALID_DEST)),
        };
        let kobject = self.ports.get(&dest_port).map(|p| p.kobject).ok_or(Return(MACH_SEND_INVALID_DEST))?;
        if kobject != KObject::None {
            return Err(SendError::KernelObject(kobject));
        }
        // Check every right before moving any.
        let check = |ipc: &Ipc, name: u32, disp: u32| -> bool {
            if name == MACH_PORT_DEAD {
                return true;
            }
            let Some(e) = ipc.entry(task, name) else { return false };
            match disp {
                MACH_MSG_TYPE_COPY_SEND | MACH_MSG_TYPE_MOVE_SEND => e.send > 0 || e.obj == Obj::Dead,
                MACH_MSG_TYPE_MAKE_SEND | MACH_MSG_TYPE_MAKE_SEND_ONCE | MACH_MSG_TYPE_MOVE_RECEIVE => e.receive,
                MACH_MSG_TYPE_MOVE_SEND_ONCE => e.send_once > 0,
                _ => false,
            }
        };
        if !check(self, remote, rdisp) || remote == MACH_PORT_DEAD {
            return Err(Return(MACH_SEND_INVALID_DEST));
        }
        if ldisp != 0 && local != MACH_PORT_NULL && !check(self, local, ldisp) {
            return Err(Return(MACH_SEND_INVALID_REPLY));
        }
        let mut descriptors = Vec::new();
        if bits & MACH_MSGH_BITS_COMPLEX != 0 {
            if msg.len() < HEADER + 4 {
                return Err(Return(MACH_SEND_MSG_TOO_SMALL));
            }
            let count = rd32(msg, HEADER) as usize;
            for i in 0..count {
                let at = HEADER + 4 + i * PORT_DESCRIPTOR;
                if at + PORT_DESCRIPTOR > msg.len() {
                    return Err(Return(MACH_SEND_MSG_TOO_SMALL));
                }
                if msg[at + 11] != MACH_MSG_PORT_DESCRIPTOR {
                    return Err(Return(MACH_SEND_INVALID_TYPE));
                }
                let (name, disp) = (rd32(msg, at), u32::from(msg[at + 10]));
                if !(MACH_MSG_TYPE_MOVE_RECEIVE..=MACH_MSG_TYPE_MAKE_SEND_ONCE).contains(&disp) {
                    return Err(Return(MACH_SEND_INVALID_TYPE));
                }
                if name != MACH_PORT_NULL && !check(self, name, disp) {
                    return Err(Return(MACH_SEND_INVALID_RIGHT));
                }
                descriptors.push((at - HEADER, name, disp));
            }
        }
        let dest = self.ports.get(&dest_port).expect("checked");
        if dest.queue.len() >= dest.qlimit {
            return Err(SendError::Full(dest_port));
        }
        // Move the rights into the message.
        let dest_right = self.copyin(task, remote, rdisp).map_err(|_| Return(MACH_SEND_INVALID_DEST))?;
        let reply = if ldisp != 0 && local != MACH_PORT_NULL {
            Some(self.copyin(task, local, ldisp).map_err(|_| Return(MACH_SEND_INVALID_REPLY))?)
        } else {
            None
        };
        let mut ports = Vec::with_capacity(descriptors.len());
        for (off, name, disp) in descriptors {
            let r = if name == MACH_PORT_NULL {
                None
            } else {
                Some(self.copyin(task, name, disp).map_err(|_| Return(MACH_SEND_INVALID_RIGHT))?)
            };
            ports.push((off, r));
        }
        let m = Message {
            other_bits: bits & !(MACH_MSGH_BITS_REMOTE_MASK | MACH_MSGH_BITS_LOCAL_MASK | MACH_MSGH_BITS_VOUCHER_MASK),
            id: rd32(msg, 20),
            size: msg.len() as u32,
            dest: dest_right,
            reply,
            body: msg[HEADER..].to_vec(),
            ports,
            sender,
        };
        let port = self.ports.get_mut(&dest_port).expect("checked");
        port.queue.push_back(m);
        let set = port.set;
        let woken = self
            .rcv_waiters
            .wake_one(RcvKey::Port(dest_port), WakeReason::Ready)
            .or_else(|| set.and_then(|s| self.rcv_waiters.wake_one(RcvKey::Set(s), WakeReason::Ready)));
        Ok(woken)
    }

    /// What `name` receives from: a receive right's port or a port set.
    pub fn receive_key(&self, task: TaskId, name: u32) -> Result<RcvKey, u32> {
        let e = self.entry(task, name).ok_or(MACH_RCV_INVALID_NAME)?;
        match e.obj {
            Obj::Port(p) if e.receive => Ok(RcvKey::Port(p)),
            Obj::Set(s) => Ok(RcvKey::Set(s)),
            _ => Err(MACH_RCV_INVALID_NAME),
        }
    }

    /// Receive on `name` into a buffer of `rcv_size` bytes with `option`
    /// (`MACH_RCV_LARGE`, the trailer elements).
    pub fn receive(
        &mut self,
        task: TaskId,
        name: u32,
        rcv_size: u32,
        option: u32,
    ) -> Result<(Received, Vec<ThreadId>), u32> {
        let key = self.receive_key(task, name)?;
        let port = match key {
            RcvKey::Port(p) => self.ports.get(&p).filter(|q| !q.queue.is_empty()).map(|_| p),
            RcvKey::Set(s) => {
                let set = self.sets.get(&s).ok_or(MACH_RCV_INVALID_NAME)?;
                set.members.iter().copied().find(|p| self.ports.get(p).is_some_and(|q| !q.queue.is_empty()))
            }
        };
        let Some(p) = port else { return Ok((Received::Empty(key), Vec::new())) };
        let rcv_name = self.spaces.get(&task).and_then(|s| s.by_port.get(&p).copied()).unwrap_or(MACH_PORT_NULL);
        let trailer = trailer_size(option);
        let q = self.ports.get_mut(&p).expect("found above");
        let m = q.queue.front().expect("non-empty");
        let needed = m.size + trailer;
        let mut woken = Vec::new();
        if needed > rcv_size {
            let mut header = [0u8; 24];
            wr32(&mut header, 4, needed);
            wr32(&mut header, 20, m.id);
            if option & MACH_RCV_LARGE == 0 {
                let m = q.queue.pop_front().expect("non-empty");
                self.destroy_message(m, &mut woken);
                if let Some(t) = self.send_waiters.wake_one(p, WakeReason::Ready) {
                    woken.push(t);
                }
            }
            return Ok((Received::TooLarge { header, needed }, woken));
        }
        let m = q.queue.pop_front().expect("non-empty");
        q.seqno = q.seqno.wrapping_add(1);
        let seqno = q.seqno.wrapping_sub(1);
        if let Some(t) = self.send_waiters.wake_one(p, WakeReason::Ready) {
            woken.push(t);
        }
        // The destination right is used up; the reply right and the
        // descriptors' rights go into the receiver's space.
        let local_type = m.dest.received_type();
        let (reply_name, reply_type) = match m.reply {
            Some(r) => (self.copyout(task, r), r.received_type()),
            None => (MACH_PORT_NULL, 0),
        };
        let mut out = Vec::with_capacity(needed as usize);
        out.resize(HEADER, 0);
        wr32(&mut out, 0, m.other_bits | reply_type | (local_type << 8));
        wr32(&mut out, 4, m.size);
        wr32(&mut out, 8, reply_name);
        wr32(&mut out, 12, rcv_name);
        wr32(&mut out, 16, MACH_PORT_NULL);
        wr32(&mut out, 20, m.id);
        let body_at = out.len();
        out.extend_from_slice(&m.body);
        for (off, r) in m.ports {
            let at = body_at + off;
            let (name, t) = match r {
                Some(r) => (self.copyout(task, r), r.received_type()),
                None => (MACH_PORT_NULL, 0),
            };
            wr32(&mut out, at, name);
            out[at + 10] = t as u8;
        }
        // The trailer (format 0): type, size, seqno, sender, audit token,
        // context; elements past what was asked for are cut off.
        let mut t = [0u8; 68];
        wr32(&mut t, 0, MACH_MSG_TRAILER_FORMAT_0);
        wr32(&mut t, 4, trailer);
        wr32(&mut t, 8, seqno);
        wr32(&mut t, 12, m.sender.uid);
        wr32(&mut t, 16, m.sender.gid);
        for (i, w) in m.sender.audit.iter().enumerate() {
            wr32(&mut t, 20 + 4 * i, *w);
        }
        out.extend_from_slice(&t[..trailer as usize]);
        Ok((Received::Message(out), woken))
    }

    /// Queue `thread` as a receiver waiting on `key`.
    pub fn wait_receive(&mut self, key: RcvKey, thread: ThreadId) {
        self.rcv_waiters.enqueue(key, thread);
    }

    /// Queue `thread` as a sender waiting for room on `port`.
    pub fn wait_send(&mut self, port: PortId, thread: ThreadId) {
        self.send_waiters.enqueue(port, thread);
    }

    /// How the wait of `thread` ended, if a waker ended it.
    pub fn wait_result(&mut self, thread: ThreadId) -> Option<WakeReason> {
        self.rcv_waiters
            .take(thread)
            .or_else(|| self.send_waiters.take(thread))
            .or_else(|| self.sem_waiters.take(thread))
    }

    /// Take `thread` off every wait queue (it timed out or was interrupted).
    pub fn cancel_waits(&mut self, thread: ThreadId) {
        self.rcv_waiters.cancel(thread);
        self.send_waiters.cancel(thread);
        self.sem_waiters.cancel(thread);
        self.wait_result(thread);
    }

    // ----- semaphores -----

    /// semaphore_create: a semaphore with `value`, as a send right in `task`.
    pub fn semaphore_create(&mut self, task: TaskId, value: u32) -> Result<u32, u32> {
        if !self.spaces.contains_key(&task) {
            return Err(KERN_INVALID_ARGUMENT);
        }
        let p = self.new_port(KObject::Semaphore, None);
        self.ports.get_mut(&p).expect("just made").count = value;
        Ok(self.add_send(task, p, 1))
    }

    /// semaphore_destroy: the semaphore dies; its waiters return
    /// `KERN_TERMINATED`.
    pub fn semaphore_destroy(&mut self, task: TaskId, name: u32) -> Result<Vec<ThreadId>, u32> {
        let p = self.semaphore(task, name)?;
        let mut woken = Vec::new();
        self.destroy_port(p, &mut woken);
        Ok(woken)
    }

    /// The semaphore `name` names in `task`: `KERN_INVALID_NAME` for a name
    /// that is not there (and `MACH_PORT_NULL`, `MACH_PORT_DEAD`),
    /// `KERN_INVALID_RIGHT` for a name without a send right,
    /// `KERN_INVALID_CAPABILITY` for a send right to something else.
    pub fn semaphore(&self, task: TaskId, name: u32) -> Result<PortId, u32> {
        let e = self.entry(task, name).ok_or(KERN_INVALID_NAME)?;
        let Obj::Port(p) = e.obj else { return Err(KERN_INVALID_RIGHT) };
        if e.send == 0 {
            return Err(KERN_INVALID_RIGHT);
        }
        match self.ports.get(&p).map(|q| q.kobject) {
            Some(KObject::Semaphore) => Ok(p),
            _ => Err(KERN_INVALID_CAPABILITY),
        }
    }

    /// The thread a thread port name stands for; `None` for 0. Anything else
    /// is `KERN_INVALID_ARGUMENT` (semaphore_signal_thread's thread name).
    pub fn thread_of(&self, task: TaskId, name: u32) -> Result<Option<ThreadId>, u32> {
        if name == MACH_PORT_NULL {
            return Ok(None);
        }
        let e = self.entry(task, name).ok_or(KERN_INVALID_ARGUMENT)?;
        match e.obj {
            Obj::Port(p) if e.send > 0 => match self.ports.get(&p).map(|q| q.kobject) {
                Some(KObject::Thread(t)) => Ok(Some(t)),
                _ => Err(KERN_INVALID_ARGUMENT),
            },
            _ => Err(KERN_INVALID_ARGUMENT),
        }
    }

    /// Signal a semaphore: wake one waiter (or `thread`, or all of them), or
    /// count one up when nobody waits. Returns the threads woken.
    /// `KERN_NOT_WAITING` when a specific thread (or, for
    /// semaphore_signal_thread with no thread, any thread) was asked for and
    /// none waits.
    pub fn semaphore_signal(&mut self, p: PortId, how: Signal) -> Result<Vec<ThreadId>, u32> {
        let mut woken = Vec::new();
        match how {
            Signal::One => match self.sem_waiters.wake_one(p, WakeReason::Ready) {
                Some(t) => woken.push(t),
                None => {
                    let q = self.ports.get_mut(&p).ok_or(KERN_TERMINATED)?;
                    q.count = q.count.saturating_add(1);
                }
            },
            Signal::All => {
                self.sem_waiters.wake_all(p, || WakeReason::Ready, &mut |t| woken.push(t));
            }
            Signal::Thread(None) => {
                woken.push(self.sem_waiters.wake_one(p, WakeReason::Ready).ok_or(KERN_NOT_WAITING)?)
            }
            Signal::Thread(Some(t)) => {
                if !self.sem_waiters.wake_thread(p, t, WakeReason::Ready) {
                    return Err(KERN_NOT_WAITING);
                }
                woken.push(t);
            }
        }
        Ok(woken)
    }

    /// Take one count of a semaphore: `true` if there was one, otherwise
    /// `thread` is queued and must block.
    pub fn semaphore_take(&mut self, p: PortId, thread: ThreadId) -> Result<bool, u32> {
        let q = self.ports.get_mut(&p).ok_or(KERN_TERMINATED)?;
        if q.count > 0 {
            q.count -= 1;
            return Ok(true);
        }
        self.sem_waiters.enqueue(p, thread);
        Ok(false)
    }

    /// A semaphore's count (for tests and diagnostics).
    pub fn semaphore_count(&self, p: PortId) -> Option<u32> {
        self.ports.get(&p).map(|q| q.count)
    }
}

/// How semaphore_signal wakes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    One,
    All,
    /// semaphore_signal_thread: that thread, or any one waiter for `None`.
    Thread(Option<ThreadId>),
}
