//! Behaviour tests across the modules: namespace, handles, lifetime,
//! dispatcher objects and a randomized model of waits.

extern crate std;

mod dispatch;
mod handles;
mod lifetime;
mod model;
mod namespace;

use alloc::vec::Vec;

pub(crate) use crate::access::*;
pub(crate) use crate::name::wide as w;
pub(crate) use crate::status::*;
pub(crate) use crate::*;

pub(crate) const P1: ProcessId = ProcessId(1);
pub(crate) const P2: ProcessId = ProcessId(2);
pub(crate) const T1: ThreadRef = ThreadRef(1);
pub(crate) const T2: ThreadRef = ThreadRef(2);
pub(crate) const T3: ThreadRef = ThreadRef(3);

/// A fresh manager with processes 1 and 2, and a user-mode caller in 1.
pub(crate) fn setup() -> (ObjectManager, Caller) {
    let mut om = ObjectManager::new();
    om.create_process(P1, None).unwrap();
    om.create_process(P2, None).unwrap();
    (om, Caller::user(P1, T1))
}

pub(crate) fn caller(process: ProcessId, thread: ThreadRef) -> Caller {
    Caller::user(process, thread)
}

/// Create an event by path with `attrs`.
pub(crate) fn event(om: &mut ObjectManager, c: &Caller, path: &str, attrs: u32) -> Result<Created> {
    let name = w(path);
    om.nt_create_event(c, &ObjectAttributes::named(&name, attrs), EVENT_ALL_ACCESS, EventType::Notification, false)
}

/// Open any object by path.
pub(crate) fn open(om: &mut ObjectManager, c: &Caller, path: &str, attrs: u32) -> Result<Handle> {
    let name = w(path);
    om.open_object(c, &ObjectAttributes::named(&name, attrs), MAXIMUM_ALLOWED, None, &mut NoParse)
}

pub(crate) fn lookup(om: &mut ObjectManager, path: &str) -> Result<ObjectId> {
    om.lookup(&w(path), true)
}

pub(crate) fn name_of(om: &ObjectManager, id: ObjectId) -> Option<std::string::String> {
    om.object_name(id).map(|n| std::string::String::from_utf16(&n).unwrap())
}

pub(crate) fn unnamed_event(om: &mut ObjectManager, c: &Caller, kind: EventType, signaled: bool) -> ObjectId {
    om.nt_create_event(c, &ObjectAttributes::unnamed(0), EVENT_ALL_ACCESS, kind, signaled).unwrap().object
}

/// Every wake queued so far.
pub(crate) fn wakes(om: &mut ObjectManager) -> Vec<Wake> {
    core::iter::from_fn(|| om.pop_wake()).collect()
}

/// Every woken thread with its status.
pub(crate) fn woken(om: &mut ObjectManager) -> Vec<(ThreadRef, NtStatus)> {
    wakes(om).into_iter().map(|w| (w.thread, w.status)).collect()
}

pub(crate) fn blocked(o: Result<WaitOutcome>) -> WaitId {
    match o {
        Ok(WaitOutcome::Blocked(id)) => id,
        other => panic!("expected a blocked wait, got {other:?}"),
    }
}

pub(crate) fn wait1(om: &mut ObjectManager, t: ThreadRef, o: ObjectId) -> Result<WaitOutcome> {
    om.wait_objects(t, &[o], WaitType::Any, false, Timeout::Infinite)
}

pub(crate) const fn done(s: NtStatus) -> Result<WaitOutcome> {
    Ok(WaitOutcome::Done(s))
}
