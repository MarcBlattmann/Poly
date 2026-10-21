//! The registry in the NT namespace (ADR 0066): the `\REGISTRY` key
//! object, the parse procedure that turns the rest of a path into a key
//! object, key objects' end, and the completion of change notifications.
//!
//! The registry itself is `poly-registry`'s [`Registry`]. The object
//! manager sees it through one permanent, named, opaque `Key` object,
//! `\REGISTRY`, created with `parse: true` (ADR 0030's parse objects): a
//! walk that reaches it hands the rest of the path to [`KeyParse`], which
//! resolves it in the registry and answers with a new, unnamed `Key`
//! object for the key found or created, or with a reparse for a symbolic
//! link key. Every key object is a parse object too, so a key handle works
//! as `OBJECT_ATTRIBUTES.RootDirectory`. A key object's cookie is its
//! [`OpenId`] (an open instance of the key); when the object manager
//! deletes the object (its last handle closed) the instance ends
//! ([`NtRegistry::object_deleted`]).
//!
//! Notifications: `NtNotifyChangeKey` queues a request on the open instance
//! with the caller's event and I/O status block. When a change satisfies
//! it, the event (if any) is signalled and the I/O status block written.
//! The block is in the waiting process's memory, so it is written by the
//! next system call that process makes ([`finish`], run after every
//! service); for a change made by the waiting process itself that is the
//! same call, and its event is signalled after the block is written.

pub mod services;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::result::Result;
use poly_ntobj::access::AccessMask;
use poly_ntobj::status::*;
use poly_ntobj::{
    Caller, Handle, Intent, NewObject, NtStatus, OBJ_KERNEL_HANDLE, OBJ_PERMANENT, ObjectAttributes, ObjectId,
    ObjectManager, ObjectType, ParseHook, ParseRequest, Parsed, ProcessId, ThreadRef,
};
use poly_registry::consts::*;
use poly_registry::{CreateOptions, Disposition, KeyId, OpenId, Registry, Resolved};

use crate::kernel::NtKernel;

/// The cookie of a key object made by `NtCreateKey` outside the registry
/// (a name that never reached `\REGISTRY`): no open instance has it.
pub const NO_INSTANCE: u64 = u64::MAX;

/// A queued `NtNotifyChangeKey`.
#[derive(Clone, Copy, Debug)]
struct Request {
    /// The open instance it is queued on.
    open: OpenId,
    process: ProcessId,
    /// The event to signal (a pointer reference held until then).
    event: Option<ObjectId>,
    /// The caller's `IO_STATUS_BLOCK`.
    iosb: u64,
}

/// A completed request whose I/O status block is still to be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub process: ProcessId,
    pub iosb: u64,
    pub status: NtStatus,
    /// The request it completes.
    pub request: u64,
    /// The event to signal once the block is written (`None` once it was
    /// signalled already).
    pub event: Option<ObjectId>,
}

/// The NT personality's registry state, in [`crate::NtSystem`].
#[derive(Debug)]
pub struct NtRegistry {
    pub reg: Registry,
    /// The `\REGISTRY` object.
    pub root_object: Option<ObjectId>,
    requests: BTreeMap<u64, Request>,
    next_request: u64,
    completions: Vec<Completion>,
}

impl NtRegistry {
    /// `reg` (with its hives mounted), not yet in a namespace.
    pub fn new(reg: Registry) -> NtRegistry {
        NtRegistry { reg, root_object: None, requests: BTreeMap::new(), next_request: 1, completions: Vec::new() }
    }

    /// Make `\REGISTRY` in `om`: a permanent `Key` parse object for the
    /// root key.
    pub fn install(&mut self, om: &mut ObjectManager) {
        let caller = Caller::kernel(ProcessId(0), ThreadRef(0));
        let name = poly_ntobj::name::wide("\\REGISTRY");
        let attrs = ObjectAttributes::named(&name, OBJ_PERMANENT | OBJ_KERNEL_HANDLE);
        let open = self.reg.open(self.reg.root()).expect("the root key exists");
        let new = NewObject::Opaque { kind: ObjectType::Key, cookie: open.to_bits(), signaled: None, parse: true };
        let c = om
            .create_object(&caller, &attrs, 0, new, &mut poly_ntobj::NoParse)
            .expect("\\REGISTRY goes in a fresh namespace");
        om.close(&caller, c.handle).expect("the kernel handle just made");
        self.root_object = Some(c.object);
    }

    /// A key object was deleted (the object manager's `Deleted`): its open
    /// instance ends, completing its notifications with
    /// `STATUS_NOTIFY_CLEANUP`.
    pub fn object_deleted(&mut self, cookie: u64, om: &mut ObjectManager) {
        if cookie != NO_INSTANCE {
            self.reg.close(OpenId::from_bits(cookie));
        }
        self.deliver(om, None);
    }

    /// Queue a notification request.
    fn add_request(&mut self, open: OpenId, request: Request, filter: u32, subtree: bool) -> Result<u64, NtStatus> {
        let id = self.next_request;
        self.reg.notify(open, filter, subtree, id)?;
        self.next_request += 1;
        self.requests.insert(id, request);
        Ok(id)
    }

    /// Turn completed requests into [`Completion`]s. Events of requests
    /// made by another process than `current` are signalled now; the
    /// caller's own are signalled by [`finish`] after it wrote the block.
    pub fn deliver(&mut self, om: &mut ObjectManager, current: Option<ProcessId>) {
        while let Some(f) = self.reg.pop_fired() {
            let Some(r) = self.requests.remove(&f.request) else { continue };
            let mut event = r.event;
            if Some(r.process) != current
                && let Some(e) = event.take()
            {
                signal(om, e);
            }
            let request = f.request;
            self.completions.push(Completion { process: r.process, iosb: r.iosb, status: f.status, request, event });
        }
    }

    /// Completions waiting for `process` to write them.
    pub fn take_completions(&mut self, process: ProcessId) -> Vec<Completion> {
        let (mine, rest) = core::mem::take(&mut self.completions).into_iter().partition(|c| c.process == process);
        self.completions = rest;
        mine
    }

    /// Forget a process's completions (it ended), signalling their events.
    pub fn process_ended(&mut self, process: ProcessId, om: &mut ObjectManager) {
        for c in self.take_completions(process) {
            if let Some(e) = c.event {
                signal(om, e);
            }
        }
        let gone: Vec<u64> = self.requests.iter().filter(|(_, r)| r.process == process).map(|(id, _)| *id).collect();
        for id in gone {
            let r = self.requests.remove(&id).expect("listed");
            if let Some(e) = r.event {
                let _ = om.dereference(e);
            }
        }
    }

    /// The status request `request` completed with, while its block is
    /// still to be written; `None` if it is pending or already written.
    pub fn completion_status(&self, request: u64) -> Option<NtStatus> {
        self.completions.iter().find(|c| c.request == request).map(|c| c.status)
    }

    /// Whether request `request` is still queued.
    pub fn is_pending(&self, request: u64) -> bool {
        self.requests.contains_key(&request)
    }

    /// Take back a request still queued (a synchronous call whose wait
    /// ended without it), dropping its event reference.
    pub fn cancel_request(&mut self, request: u64, om: &mut ObjectManager) {
        if let Some(r) = self.requests.remove(&request) {
            self.reg.cancel_request(r.open, request);
            if let Some(e) = r.event {
                let _ = om.dereference(e);
            }
        }
    }

    /// Requests not yet completed.
    pub fn pending_requests(&self) -> usize {
        self.requests.len()
    }
}

/// Signal an event a request held and drop the request's reference.
fn signal(om: &mut ObjectManager, event: ObjectId) {
    let _ = om.set_event(event);
    let _ = om.dereference(event);
}

/// Write the I/O status blocks of the caller's completed notifications,
/// then signal their events. Runs after every service
/// ([`crate::dispatch_with`]).
pub fn finish<K: NtKernel>(k: &mut K) {
    let caller = k.caller();
    let mine = k.with_state(|sys, _, _| {
        sys.registry.deliver(&mut sys.objects, Some(caller.process));
        sys.registry.take_completions(caller.process)
    });
    if mine.is_empty() {
        return;
    }
    for c in &mine {
        // IO_STATUS_BLOCK (wdm.h): Status (in a union with a pointer) at 0,
        // Information at 8. Nothing is returned in Information.
        let mut block = [0u8; 16];
        block[..4].copy_from_slice(&c.status.0.to_le_bytes());
        let _ = crate::abi::write(k, c.iosb, &block);
    }
    k.with_state(|sys, _, b| {
        for c in &mine {
            if let Some(e) = c.event {
                signal(&mut sys.objects, e);
            }
        }
        sys.reap(b);
    });
}

/// What `NtCreateKey` adds to a walk.
#[derive(Clone, Copy, Debug)]
pub struct CreateArgs<'a> {
    pub options: u32,
    pub class: &'a [u16],
    /// The `RootDirectory` key's instance and whether its handle has
    /// `KEY_CREATE_SUB_KEY` (Learn: "ZwCreateKey": the key RootDirectory
    /// refers to "must have been opened for KEY_CREATE_SUB_KEY access").
    pub root: Option<(OpenId, bool)>,
}

/// The parse procedure of `\REGISTRY` and of every key object.
#[derive(Debug)]
pub struct KeyParse<'a> {
    pub reg: &'a mut Registry,
    /// Keep a symbolic link key as the last component (`OBJ_OPENLINK`,
    /// `REG_OPTION_OPEN_LINK`).
    pub open_link: bool,
    pub create: Option<CreateArgs<'a>>,
    /// FILETIME for new keys.
    pub now: u64,
    /// Set when the hook made or opened a key.
    pub disposition: Option<Disposition>,
}

impl KeyParse<'_> {
    fn make_object(&mut self, om: &mut ObjectManager, key: KeyId, thread: ThreadRef) -> Parsed {
        let open = match self.reg.open(key) {
            Ok(o) => o,
            Err(e) => return Parsed::Fail(e),
        };
        let new = NewObject::Opaque { kind: ObjectType::Key, cookie: open.to_bits(), signaled: None, parse: true };
        match om.new_object(new, thread) {
            Ok(id) => Parsed::Object(id),
            Err(e) => {
                self.reg.close(open);
                Parsed::Fail(e)
            }
        }
    }

    fn create(&mut self, parent: KeyId, name: &[u16], args: CreateArgs<'_>) -> Result<KeyId, NtStatus> {
        if let Some((root, may_create)) = args.root
            && !may_create
            && self.reg.key_of(root) == Ok(parent)
        {
            return Err(STATUS_ACCESS_DENIED);
        }
        let opts = CreateOptions {
            volatile: args.options & REG_OPTION_VOLATILE != 0,
            link: args.options & REG_OPTION_CREATE_LINK != 0,
            class: args.class,
        };
        let (key, d) = self.reg.create_key(parent, name, opts, self.now)?;
        self.disposition = Some(d);
        Ok(key)
    }
}

impl ParseHook for KeyParse<'_> {
    fn parse(&mut self, om: &mut ObjectManager, request: &ParseRequest<'_>) -> Parsed {
        if request.object_type.is_some_and(|t| t != ObjectType::Key) {
            return Parsed::Fail(STATUS_OBJECT_TYPE_MISMATCH);
        }
        let thread = request.caller.map_or(ThreadRef(0), |c| c.thread);
        let start = match self.reg.key_of(OpenId::from_bits(request.cookie)) {
            Ok(k) => k,
            Err(e) => return Parsed::Fail(e),
        };
        let resolved = match self.reg.resolve(start, request.remaining, self.open_link) {
            Ok(r) => r,
            Err(e) => return Parsed::Fail(e),
        };
        let create = self.create.filter(|_| request.intent == Intent::Create);
        match resolved {
            Resolved::Reparse(path) => Parsed::Reparse(path),
            Resolved::Found(key) => {
                if create.is_some_and(|c| c.options & REG_OPTION_CREATE_LINK != 0) {
                    // A new link over an existing key (to be confirmed by
                    // the spec agent).
                    return Parsed::Fail(STATUS_OBJECT_NAME_COLLISION);
                }
                self.disposition = Some(Disposition::Opened);
                self.make_object(om, key, thread)
            }
            Resolved::Missing { parent, name } => match create {
                None => Parsed::Fail(STATUS_OBJECT_NAME_NOT_FOUND),
                Some(args) => match self.create(parent, &name, args) {
                    Ok(key) => self.make_object(om, key, thread),
                    Err(e) => Parsed::Fail(e),
                },
            },
        }
    }
}

/// The access a key service asks of `desired`: the `KEY_WOW64_*` view bits
/// are not rights (Learn: "Accessing an Alternate Registry View"); Poly has
/// one view, so they are dropped.
pub const fn key_access(desired: u64) -> AccessMask {
    desired as u32 & !KEY_WOW64_RES
}

/// A key handle's object, open instance and key, checked for `access`.
pub fn key_of_handle(
    sys: &mut crate::system::NtSystem,
    caller: &Caller,
    h: Handle,
    access: AccessMask,
) -> Result<(ObjectId, OpenId, KeyId), NtStatus> {
    let id = sys.objects.object_by_handle(caller, h, access, Some(ObjectType::Key))?;
    let open = OpenId::from_bits(sys.objects.cookie(id).expect("live object"));
    let key = sys.registry.reg.key_of(open)?;
    Ok((id, open, key))
}

/// The event a notification request signals.
#[derive(Clone, Copy, Debug)]
pub(crate) enum NotifyEvent {
    None,
    /// The caller's event handle.
    Handle(Handle),
    /// An event object the service made itself (a synchronous call), with a
    /// pointer reference the request takes over.
    Object(ObjectId),
}

/// Queue a notification for `services::notify_change_key`; returns the
/// request's id.
pub(crate) fn queue_notify(
    sys: &mut crate::system::NtSystem,
    caller: &Caller,
    h: Handle,
    event: NotifyEvent,
    iosb: u64,
    filter: u32,
    subtree: bool,
) -> Result<u64, NtStatus> {
    let (_, open, _) = key_of_handle(sys, caller, h, KEY_NOTIFY)?;
    let event = match event {
        NotifyEvent::None => None,
        NotifyEvent::Object(e) => Some(e),
        NotifyEvent::Handle(eh) => {
            let e = sys.objects.reference_by_handle(
                caller,
                eh,
                poly_ntobj::access::EVENT_MODIFY_STATE,
                Some(ObjectType::Event),
            )?;
            // The I/O manager resets a request's event when the request
            // starts (to be confirmed by the spec agent).
            let _ = sys.objects.reset_event(e);
            Some(e)
        }
    };
    let request = Request { open, process: caller.process, event, iosb };
    sys.registry.add_request(open, request, filter, subtree).inspect_err(|_| {
        if let Some(ev) = event {
            let _ = sys.objects.dereference(ev);
        }
    })
}
