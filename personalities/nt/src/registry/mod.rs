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

pub mod hivefile;
pub mod services;
pub mod wow64;

pub use wow64::{View, view_of, wow64_view};

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
    /// The hives programs loaded from their own files ([`hivefile`]).
    pub external: Vec<hivefile::External>,
    /// The registry view each open key (by its instance's bits) was opened or made in
    /// ([`apply_view`]): keys below it with no view bit are in that view
    /// (`RegCreateKeyExW.yaml` `inheritance`). Keys opened otherwise (the `\REGISTRY` object)
    /// have the caller's default view.
    views: BTreeMap<u64, View>,
}

impl NtRegistry {
    /// `reg` (with its hives mounted), not yet in a namespace.
    pub fn new(reg: Registry) -> NtRegistry {
        NtRegistry {
            reg,
            root_object: None,
            requests: BTreeMap::new(),
            next_request: 1,
            completions: Vec::new(),
            external: Vec::new(),
            views: BTreeMap::new(),
        }
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
        // The user's hive is the user's (ADR 0222): a program without administrator rights
        // writes its settings under `HKEY_CURRENT_USER`, as on Windows.
        let u = poly_registry::defaults::POLY_USER_SID;
        let key = poly_ntobj::path_key("K", &poly_ntobj::name::wide(&alloc::format!("\\REGISTRY\\USER\\{u}")));
        om.seed_path_security(
            &key,
            &alloc::format!("O:{u}G:{u}D:P(A;CI;KA;;;SY)(A;CI;KA;;;BA)(A;CI;KA;;;{u})(A;CI;KR;;;RC)"),
        );
    }

    /// A key object was deleted (the object manager's `Deleted`): its open
    /// instance ends, completing its notifications with
    /// `STATUS_NOTIFY_CLEANUP`.
    pub fn object_deleted(&mut self, cookie: u64, om: &mut ObjectManager) {
        if cookie != NO_INSTANCE {
            self.views.remove(&cookie);
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
#[inline(never)]
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
        // Information at 8 (4 for a 32-bit caller). Nothing is returned in Information.
        let _ = crate::io::services::write_iosb(k, c.iosb, c.status, 0);
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
            Ok(id) => {
                let path = self.reg.full_path(key).unwrap_or_default();
                om.assign_path_security(id, poly_ntobj::DefaultSd::Key, &poly_ntobj::path_key("K", &path), true);
                Parsed::Object(id)
            }
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

/// Apply the registry view of the call (ADR 0186, stage 2; `RegCreateKeyExW.yaml` `views`,
/// `inheritance`) to the key name `attrs` gives, and return the view chosen for the key it
/// reaches (for [`remember_view`]; `None`: no bit chose one, the caller's default view).
///
/// The view: `KEY_WOW64_32KEY` in `access` is the 32-bit one; `KEY_WOW64_64KEY` the native one,
/// except below an open key of the 32-bit view that physically lies below a `Wow6432Node` (the view is part of the
/// open key's place: a vendor key below `Software` opened in the 32-bit view stays there); no
/// bit, the view of the open key the name is relative to, or the caller's default for an
/// absolute name (a 32-bit caller's is the 32-bit view). In the 32-bit view the full path (the
/// open key's physical path and the relative name, or the absolute name) becomes the physical
/// key of that view ([`wow64::x86_target`]): when that is another path, the name becomes it,
/// absolute, and the root handle is dropped. A root that is no key, or whose name does not
/// resolve, is left to the object manager as it is.
pub fn apply_view(
    sys: &mut crate::system::NtSystem,
    caller: &Caller,
    wow64: bool,
    access: u64,
    attrs: &mut crate::abi::Attributes,
) -> Option<View> {
    let default = view_of(wow64, 0);
    let name = attrs.name.clone().unwrap_or_default();
    let sep = u16::from(b'\\');
    let (base, base_view) = match attrs.root {
        Some(h) => {
            if name.first() == Some(&sep) {
                return None;
            }
            let Ok((_, open, key)) = key_of_handle(sys, caller, h, 0) else { return None };
            let Ok(path) = sys.registry.reg.full_path(key) else { return None };
            (Some(path), sys.registry.views.get(&open.to_bits()).copied())
        }
        None => (None, None),
    };
    if base.is_some() && name.iter().all(|&c| c == sep) {
        // The open key itself again, in its view (`RegCreateKeyExW.yaml` `empty_subkey`).
        return base_view;
    }
    // An open key of the 32-bit view that lies below a `Wow6432Node` keeps its keys there; one
    // opened in the native view at a path naming the node is a plain physical key
    // (`RegistryDefaults.Wow6432Node.yaml` `one_key_64`: `KEY_WOW64_64KEY` below
    // `Classes\Wow6432Node` opened with it makes the key there).
    let inside = base.as_deref().is_some_and(wow64::has_node) && base_view.unwrap_or(default) == View::X86;
    let chosen = match access as u32 & KEY_WOW64_RES {
        KEY_WOW64_32KEY => Some(View::X86),
        KEY_WOW64_64KEY if inside => Some(View::X86),
        KEY_WOW64_64KEY => Some(View::Native),
        _ => base_view,
    };
    if chosen.unwrap_or(default) == View::Native {
        return chosen;
    }
    let mut full = Vec::new();
    if let Some(b) = &base {
        full.extend_from_slice(b);
        full.push(sep);
    }
    full.extend_from_slice(&name);
    let target = wow64::x86_target(&full);
    if target != full {
        attrs.root = None;
        attrs.name = Some(target);
    }
    chosen
}

/// Before a create whose name [`apply_view`] moved into the 32-bit view: the `Wow6432Node` key
/// the new name goes through (right below `\REGISTRY\MACHINE\SOFTWARE` or below a `Classes` key),
/// made when its parent exists and it does not. A 32-bit program makes
/// `HKCU\Software\Classes\CLSID\{id}` one level at a time on a machine whose user classes have no
/// `Wow6432Node` yet, and finds it there (`RegistryDefaults.ClassesRoots.yaml` `hkcu_clsid`).
pub fn make_view_node(sys: &mut crate::system::NtSystem, name: &[u16], now: u64) {
    let sep = u16::from(b'\\');
    let parts: Vec<&[u16]> = name.split(|&c| c == sep).filter(|c| !c.is_empty()).collect();
    let is =
        |c: &[u16], s: &str| c.len() == s.len() && c.iter().zip(s.bytes()).all(|(&a, b)| fold(a) == fold(u16::from(b)));
    let ends = |c: &[u16], s: &str| c.len() >= s.len() && is(&c[c.len() - s.len()..], s);
    if parts.first().is_none_or(|c| !is(c, "REGISTRY")) {
        return;
    }
    let Some(at) = (1..parts.len()).find(|&i| {
        is(parts[i], WOW6432_NODE)
            && (i == 3 && is(parts[1], "MACHINE") && is(parts[2], "SOFTWARE") || ends(parts[i - 1], "Classes"))
    }) else {
        return;
    };
    let reg = &mut sys.registry.reg;
    let mut parent: Vec<u16> = Vec::new();
    for (n, c) in parts[1..at].iter().enumerate() {
        if n > 0 {
            parent.push(sep);
        }
        parent.extend_from_slice(c);
    }
    if let Ok(Resolved::Found(key)) = reg.resolve(reg.root(), &parent, false)
        && reg.child(key, parts[at]).is_none()
    {
        let _ = reg.create_key(key, parts[at], CreateOptions::default(), now);
    }
}

/// ASCII case folding, enough for the registry's own names.
fn fold(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

/// Keep the view [`apply_view`] chose for the key behind the new handle `h` (`None`: the
/// caller's default view, which is not kept: the key follows the view of whoever uses it).
pub fn remember_view(sys: &mut crate::system::NtSystem, caller: &Caller, h: Handle, view: Option<View>) {
    let Some(view) = view else { return };
    if let Ok((_, open, _)) = key_of_handle(sys, caller, h, 0) {
        sys.registry.views.insert(open.to_bits(), view);
    }
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
