//! Tasks seen from other tasks, and the bootstrap server (ADR 0238,
//! addendum): task name ports, the task special ports, what the
//! task_for_pid family of traps hands out, and the services
//! `bootstrap_check_in`, `bootstrap_register` and `bootstrap_look_up`
//! reach.
//!
//! The bootstrap server is a kernel object (`KObject::Bootstrap`), one
//! for the machine: every task's `TASK_BOOTSTRAP_PORT` is a send right to
//! it, so a child reaches the same services as its parent. A service
//! belongs to the task that checked it in or registered it and lives as
//! long as that task and the port do (`specs/macos/liblaunch.dylib/*`).

use super::{Carried, HEADER, Ipc, KObject, Message, PortId, TaskId};
use crate::consts::{bootstrap::*, kern::*, msg::*, port::*, special::*};
use alloc::vec;
use alloc::vec::Vec;

/// A name the bootstrap server knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Service {
    pub port: PortId,
    pub owner: TaskId,
    /// Checked in (the owner got the receive right) rather than
    /// registered.
    pub checked_in: bool,
}

/// A reply the server built: id, body, port descriptors, complex.
pub(super) type Reply = (u32, Vec<u8>, Vec<(usize, Option<Carried>)>, bool);

fn rd(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn push(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The name in a bootstrap request's body at `at`: length, then bytes.
fn name_at(b: &[u8], at: usize) -> Option<Vec<u8>> {
    let len = rd(b, at)? as usize;
    if len > NAME_MAX {
        return None;
    }
    Some(b.get(at + 4..at + 4 + len)?.to_vec())
}

impl Ipc {
    // ----- task ports seen from other tasks -----

    /// The name port of `target`, made on first use. `None` once the task
    /// is gone.
    fn name_port(&mut self, target: TaskId) -> PortId {
        if let Some(&p) = self.name_ports.get(&target) {
            return p;
        }
        let p = self.new_port(KObject::TaskName(target), None);
        self.name_ports.insert(target, p);
        p
    }

    /// task_name_for_pid's right: a send right to `target`'s name port in
    /// `caller` (the caller has checked that `target` runs).
    pub fn task_name_for(&mut self, caller: TaskId, target: TaskId) -> u32 {
        if !self.spaces.contains_key(&caller) {
            return MACH_PORT_NULL;
        }
        let p = self.name_port(target);
        self.add_send(caller, p, 1)
    }

    /// pid_for_task: the task a task port or name port stands for.
    /// Anything else (a dead name too) is `KERN_FAILURE`.
    pub fn pid_for_task(&self, caller: TaskId, name: u32) -> Result<TaskId, u32> {
        let e = self.entry(caller, name).ok_or(KERN_FAILURE)?;
        match e.obj {
            super::Obj::Port(p) if e.send > 0 => match self.ports.get(&p).map(|q| q.kobject) {
                Some(KObject::Task(t) | KObject::TaskName(t)) => Ok(t),
                _ => Err(KERN_FAILURE),
            },
            _ => Err(KERN_FAILURE),
        }
    }

    /// Whether `name` in `caller` is a send right to a live task port.
    pub fn is_task_port(&self, caller: TaskId, name: u32) -> bool {
        self.resolve_task(caller, name).is_ok()
    }

    /// The task `target` ended (whether or not it ever made a Mach call):
    /// its name port dies, its special ports go.
    pub(super) fn task_gone(&mut self, target: TaskId, woken: &mut Vec<crate::waitq::ThreadId>) {
        if let Some(p) = self.name_ports.remove(&target) {
            self.destroy_port(p, woken);
        }
        self.special.retain(|&(t, _), _| t != target);
    }

    // ----- special ports -----

    /// task_get_special_port: the port `which` of `task` (`None` for
    /// `MACH_PORT_NULL`).
    pub fn special_port(&mut self, task: TaskId, which: u32) -> Result<Option<PortId>, u32> {
        if which == 0 || which > TASK_MAX_SPECIAL_PORT {
            return Err(KERN_INVALID_ARGUMENT);
        }
        if let Some(&p) = self.special.get(&(task, which)) {
            return Ok(p.filter(|p| self.ports.contains_key(p)));
        }
        Ok(match which {
            TASK_KERNEL_PORT => self.spaces.get(&task).map(|s| s.task_port),
            TASK_HOST_PORT => Some(self.host_port),
            TASK_NAME_PORT => Some(self.name_port(task)),
            TASK_BOOTSTRAP_PORT => Some(self.bootstrap_port),
            _ => None,
        })
    }

    /// task_set_special_port: the kernel keeps `port` (a send right) as
    /// `task`'s port `which`. The task's own port and name port cannot be
    /// replaced.
    pub fn set_special_port(&mut self, task: TaskId, which: u32, port: Option<PortId>) -> u32 {
        if which == 0 || which > TASK_MAX_SPECIAL_PORT || which == TASK_KERNEL_PORT || which == TASK_NAME_PORT {
            return KERN_INVALID_ARGUMENT;
        }
        self.special.insert((task, which), port);
        KERN_SUCCESS
    }

    /// A new task gets its parent's task exception handlers and special
    /// ports (a child of fork or posix_spawn inherits them).
    pub fn inherit_from(&mut self, parent: TaskId, child: TaskId) {
        self.inherit_exception_ports(parent, child);
        let from: Vec<((TaskId, u32), Option<PortId>)> =
            self.special.range((parent, 0)..=(parent, u32::MAX)).map(|(&k, &v)| (k, v)).collect();
        for ((_, which), p) in from {
            self.special.entry((child, which)).or_insert(p);
        }
    }

    /// Whether the kernel keeps a send right to `p` (a special port, a
    /// bootstrap service).
    pub(super) fn kernel_holds(&self, p: PortId) -> bool {
        self.special.values().any(|&v| v == Some(p)) || self.services.values().any(|s| s.port == p)
    }

    // ----- the bootstrap server -----

    /// The service under `name` if it still lives: its port is alive and
    /// its owner still runs (the server learns of an owner's death when it
    /// next looks).
    pub fn service(&mut self, name: &[u8]) -> Option<Service> {
        let s = *self.services.get(name)?;
        let alive = self.ports.contains_key(&s.port) && self.spaces.contains_key(&s.owner);
        if alive {
            Some(s)
        } else {
            self.services.remove(name);
            None
        }
    }

    /// bootstrap_check_in: a new receive right for `name` (queue limit 2)
    /// that `caller` will hold.
    pub fn bootstrap_check_in(&mut self, caller: TaskId, name: &[u8]) -> Result<PortId, u32> {
        if let Some(s) = self.service(name) {
            return Err(if s.owner == caller { BOOTSTRAP_SERVICE_ACTIVE } else { BOOTSTRAP_NOT_PRIVILEGED });
        }
        let p = self.new_port(KObject::None, None);
        self.ports.get_mut(&p).expect("just made").qlimit = CHECK_IN_QLIMIT;
        self.services.insert(name.to_vec(), Service { port: p, owner: caller, checked_in: true });
        Ok(p)
    }

    /// bootstrap_register: `caller` registers the send right `port` under
    /// `name`.
    pub fn bootstrap_register(&mut self, caller: TaskId, name: &[u8], port: PortId) -> u32 {
        if let Some(s) = self.service(name) {
            return if s.owner == caller { BOOTSTRAP_NAME_IN_USE } else { BOOTSTRAP_NOT_PRIVILEGED };
        }
        if !self.ports.contains_key(&port) {
            return KERN_INVALID_RIGHT;
        }
        self.services.insert(name.to_vec(), Service { port, owner: caller, checked_in: false });
        KERN_SUCCESS
    }

    /// bootstrap_look_up: the port registered under `name`.
    pub fn bootstrap_look_up(&mut self, name: &[u8]) -> Result<PortId, u32> {
        self.service(name).map(|s| s.port).ok_or(BOOTSTRAP_UNKNOWN_SERVICE)
    }

    /// Serve a request sent to the bootstrap port.
    pub(super) fn answer_bootstrap(&mut self, caller: TaskId, req: &mut Message) -> Reply {
        let id = req.id;
        let b = req.body.clone();
        let with_port = |ret: u32, right: Option<Carried>| -> Reply {
            let mut out = Vec::new();
            push(&mut out, 1);
            out.extend_from_slice(&[0; 12]);
            out.extend_from_slice(&super::mig::NDR);
            push(&mut out, ret);
            (id + 100, out, vec![(4, right)], true)
        };
        match id {
            BOOTSTRAP_CHECK_IN | BOOTSTRAP_LOOK_UP => {
                let Some(name) = name_at(&b, 8) else { return with_port(BOOTSTRAP_BAD_COUNT, None) };
                let got = if id == BOOTSTRAP_CHECK_IN {
                    self.bootstrap_check_in(caller, &name).map(Carried::Receive)
                } else {
                    self.bootstrap_look_up(&name).map(Carried::Send)
                };
                match got {
                    Ok(r) => with_port(KERN_SUCCESS, Some(r)),
                    Err(kr) => with_port(kr, None),
                }
            }
            BOOTSTRAP_REGISTER => {
                let simple = |ret: u32| -> Reply {
                    let mut out = super::mig::NDR.to_vec();
                    push(&mut out, ret);
                    (id + 100, out, Vec::new(), false)
                };
                let port = match req.ports.first().and_then(|(_, r)| *r) {
                    Some(Carried::Send(p)) => p,
                    _ => return simple(KERN_INVALID_RIGHT),
                };
                let Some(name) = name_at(&b, 4 + 12 + 8) else { return simple(BOOTSTRAP_BAD_COUNT) };
                let kr = self.bootstrap_register(caller, &name, port);
                if kr == KERN_SUCCESS {
                    // The server keeps the send right now.
                    req.ports[0].1 = None;
                }
                simple(kr)
            }
            _ => {
                let mut out = super::mig::NDR.to_vec();
                push(&mut out, super::mig::MIG_BAD_ID);
                (id + 100, out, Vec::new(), false)
            }
        }
    }

    /// Serve task_get_special_port and task_set_special_port.
    pub(super) fn answer_special(&mut self, task: TaskId, req: &mut Message) -> Reply {
        let id = req.id;
        let b = &req.body;
        let simple = |ret: u32| -> Reply {
            let mut out = super::mig::NDR.to_vec();
            push(&mut out, ret);
            (id + 100, out, Vec::new(), false)
        };
        if id == super::mig::TASK_GET_SPECIAL_PORT {
            let Some(which) = rd(b, 8) else { return simple(super::mig::MIG_BAD_ARGUMENTS) };
            match self.special_port(task, which) {
                Ok(p) => {
                    let mut out = Vec::new();
                    push(&mut out, 1);
                    out.extend_from_slice(&[0; 12]);
                    (id + 100, out, vec![(4, p.map(Carried::Send))], true)
                }
                Err(kr) => simple(kr),
            }
        } else {
            let Some(which) = rd(b, 4 + 12 + 8) else { return simple(super::mig::MIG_BAD_ARGUMENTS) };
            let port = match req.ports.first().and_then(|(_, r)| *r) {
                None | Some(Carried::Dead) => None,
                Some(Carried::Send(p)) => Some(p),
                Some(_) => return simple(KERN_INVALID_RIGHT),
            };
            let kr = self.set_special_port(task, which, port);
            if kr == KERN_SUCCESS && port.is_some() {
                req.ports[0].1 = None;
            }
            simple(kr)
        }
    }
}

/// The request message a client sends to the bootstrap port, header
/// first (for tests and for Poly's libSystem): `id`, `name`, and for
/// `BOOTSTRAP_REGISTER` the port `register` with disposition `disp`.
pub fn bootstrap_request(id: u32, dest: u32, reply: u32, name: &[u8], register: Option<(u32, u32)>) -> Vec<u8> {
    let mut m = vec![0u8; HEADER];
    let mut bits = MACH_MSG_TYPE_COPY_SEND | (MACH_MSG_TYPE_MAKE_SEND_ONCE << 8);
    if let Some((port, disp)) = register {
        bits |= MACH_MSGH_BITS_COMPLEX;
        push(&mut m, 1);
        push(&mut m, port);
        push(&mut m, 0);
        m.extend_from_slice(&[0, 0, disp as u8, 0]);
    }
    m.extend_from_slice(&super::mig::NDR);
    push(&mut m, name.len() as u32);
    m.extend_from_slice(name);
    while !m.len().is_multiple_of(4) {
        m.push(0);
    }
    let len = m.len() as u32;
    m[0..4].copy_from_slice(&bits.to_le_bytes());
    m[4..8].copy_from_slice(&len.to_le_bytes());
    m[8..12].copy_from_slice(&dest.to_le_bytes());
    m[12..16].copy_from_slice(&reply.to_le_bytes());
    m[20..24].copy_from_slice(&id.to_le_bytes());
    m
}
