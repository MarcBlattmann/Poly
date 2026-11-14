//! Window stations and desktops as objects (ADR 0186).
//!
//! `user32` asks for them through `PolyUserCall` (`Request::Winsta`, the records of
//! [`poly_win32k::winsta`]); this module makes the objects, hands out the handles and keeps what
//! a process and a thread are attached to. The session's one window manager does not model
//! several desktops: a desktop here is a name with an object behind it (its security descriptor,
//! its handles, its lifetime), and the windows of every process live on the one real desktop.
//!
//! - A station or desktop is an opaque object of its own type. The registry
//!   ([`poly_win32k::winsta::Registry`]) finds it by name; the object manager deletes it with its
//!   last reference (handles, the processes on a station, the threads on a desktop, and a
//!   desktop's hold on its station), and [`WinSta::deleted`] forgets the name then.
//! - `WinSta0` and its `Default` desktop are made when first needed and never go. Their
//!   descriptor lets Everyone in and carries a Low mandatory label (no write up).
//! - A new station or desktop gets the descriptor of its creator's token (with the creator's
//!   integrity as the label) unless the call gave one; the creator of a station gets the access
//!   it asked for whatever the descriptor says, the creator of a desktop has to be granted it by
//!   the descriptor (the `CreateDesktopW` sheet: a descriptor that names only SYSTEM makes the
//!   call fail for the caller).
//! - A process is on one station (its handle, `GetProcessWindowStation`) and each thread on one
//!   desktop (`GetThreadDesktop`); a process started with `lpDesktop` names a station and
//!   desktop that must exist and be open to its token, or `user32` does not initialise in it
//!   (`STATUS_DLL_INIT_FAILED`). A process that was not told inherits its creator's.

use crate::kernel::NtKernel;
use crate::system::{NtProcess, NtSystem};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_ntobj::access::MAXIMUM_ALLOWED;
use poly_ntobj::{Caller, Handle, NewObject, OBJ_INHERIT, ObjectId, ObjectType, ProcessId, ThreadRef};
use poly_win32k::winsta::*;

/// What a process was started with: its `lpDesktop` and what it inherits (a pointer reference is
/// held on each object until the process connects or ends).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub spec: Vec<u16>,
    pub station: Option<ObjectId>,
    pub desktop: Option<ObjectId>,
}

#[derive(Debug, Default)]
struct Proc {
    /// The station the process is on, and the handle it has to it (0: none made yet). A pointer
    /// reference is held on the object.
    station: Option<ObjectId>,
    handle: u64,
    /// The desktop its threads start on (a pointer reference is held).
    desktop: Option<ObjectId>,
    start: Option<Start>,
    connected: bool,
}

#[derive(Debug)]
struct Thread {
    desktop: ObjectId,
    handle: u64,
}

/// The window stations and desktops of the session and who is attached to them.
#[derive(Debug, Default)]
pub struct WinSta {
    registry: Registry<ObjectId>,
    procs: BTreeMap<ProcessId, Proc>,
    threads: BTreeMap<ThreadRef, Thread>,
    next_cookie: u64,
    defaults: Option<(ObjectId, ObjectId)>,
}

/// The descriptor of `WinSta0` and `Default`: Everyone full, a Low label with no write up.
const DEFAULT_SDDL: &str = "O:BAG:SYD:(A;;GA;;;WD)S:(ML;;NW;;;LW)";

impl WinSta {
    /// What a process created now by `creator`'s thread starts with: `spec` (its `lpDesktop`) and
    /// the creator's station and desktop to inherit.
    pub fn child_start(
        &mut self,
        objects: &mut poly_ntobj::ObjectManager,
        creator: ProcessId,
        thread: ThreadRef,
        spec: Option<Vec<u16>>,
    ) -> Start {
        let p = self.procs.get(&creator);
        let station = p.and_then(|p| p.station);
        let desktop = self.threads.get(&thread).map(|t| t.desktop).or_else(|| p.and_then(|p| p.desktop));
        for id in station.iter().chain(desktop.iter()) {
            let _ = objects.reference(*id);
        }
        Start { spec: spec.unwrap_or_default(), station, desktop }
    }

    /// The kernel made process `pid` with `start`.
    pub fn begin(&mut self, pid: ProcessId, start: Start) {
        self.procs.entry(pid).or_default().start = Some(start);
    }

    /// Process `pid` has ended.
    pub fn process_exited(&mut self, objects: &mut poly_ntobj::ObjectManager, pid: ProcessId) {
        let Some(p) = self.procs.remove(&pid) else { return };
        for id in p.station.into_iter().chain(p.desktop) {
            let _ = objects.dereference(id);
        }
        if let Some(s) = p.start {
            for id in s.station.into_iter().chain(s.desktop) {
                let _ = objects.dereference(id);
            }
        }
    }

    /// A thread has ended: it leaves its desktop.
    pub fn thread_exited(&mut self, objects: &mut poly_ntobj::ObjectManager, thread: ThreadRef) {
        if let Some(t) = self.threads.remove(&thread) {
            let _ = objects.dereference(t.desktop);
        }
    }

    /// The object manager deleted station or desktop `id`.
    pub fn deleted(&mut self, objects: &mut poly_ntobj::ObjectManager, id: ObjectId) {
        let parent = self.registry.station_of_desktop(id);
        self.registry.forget(id);
        if let Some(s) = parent {
            // A desktop held its station.
            let _ = objects.dereference(s);
        }
    }

    fn new_station(
        &mut self,
        objects: &mut poly_ntobj::ObjectManager,
        creator: Option<u64>,
        thread: ThreadRef,
        name: &[u16],
        sd: Option<&[u8]>,
    ) -> Option<ObjectId> {
        self.next_cookie += 1;
        let new = NewObject::Opaque {
            kind: ObjectType::WindowStation,
            cookie: self.next_cookie,
            signaled: None,
            parse: false,
        };
        let id = objects.new_object(new, thread).ok()?;
        objects.assign_security(id, ObjectType::WindowStation, creator, sd);
        self.registry.add_station(id, name);
        Some(id)
    }

    fn new_desktop(
        &mut self,
        objects: &mut poly_ntobj::ObjectManager,
        creator: Option<u64>,
        thread: ThreadRef,
        station: ObjectId,
        name: &[u16],
        sd: Option<&[u8]>,
    ) -> Option<ObjectId> {
        self.next_cookie += 1;
        let new =
            NewObject::Opaque { kind: ObjectType::Desktop, cookie: self.next_cookie, signaled: None, parse: false };
        let id = objects.new_object(new, thread).ok()?;
        objects.assign_security(id, ObjectType::Desktop, creator, sd);
        // The desktop keeps its station alive.
        let _ = objects.reference(station);
        self.registry.add_desktop(station, id, name);
        Some(id)
    }

    /// `WinSta0` and `Default`, made on first use and kept for good.
    fn defaults(&mut self, sys_objects: &mut poly_ntobj::ObjectManager, caller: &Caller) -> (ObjectId, ObjectId) {
        if let Some(d) = self.defaults {
            return d;
        }
        let creator = sys_objects.security.effective(caller.process, caller.thread);
        let sd = poly_security::sdstr::parse(&wide(DEFAULT_SDDL)).ok();
        let station = self.new_station(sys_objects, creator, caller.thread, &wide(DEFAULT_STATION), sd.as_deref());
        let station = station.expect("the first station of a session");
        let desktop = self
            .new_desktop(sys_objects, creator, caller.thread, station, &wide(DEFAULT_DESKTOP), sd.as_deref())
            .expect("the first desktop of a session");
        // The creation references stay: the defaults never go.
        self.defaults = Some((station, desktop));
        (station, desktop)
    }

    /// The station process `pid` is on: the one it was started on or inherited, else `WinSta0`
    /// (a pointer reference is taken for the process the first time).
    fn station_of(&mut self, objects: &mut poly_ntobj::ObjectManager, caller: &Caller) -> ObjectId {
        if let Some(s) = self.procs.get(&caller.process).and_then(|p| p.station) {
            return s;
        }
        let inherited = self
            .procs
            .get(&caller.process)
            .and_then(|p| p.start.as_ref())
            .and_then(|s| s.station.filter(|&id| objects.is_alive(id)));
        let s = match inherited {
            Some(s) => s,
            None => self.defaults(objects, caller).0,
        };
        let _ = objects.reference(s);
        self.procs.entry(caller.process).or_default().station = Some(s);
        s
    }

    /// The desktop the threads of the caller's process start on.
    fn initial_desktop(&mut self, objects: &mut poly_ntobj::ObjectManager, caller: &Caller) -> ObjectId {
        if let Some(d) = self.procs.get(&caller.process).and_then(|p| p.desktop) {
            return d;
        }
        let station = self.station_of(objects, caller);
        let inherited = self
            .procs
            .get(&caller.process)
            .and_then(|p| p.start.as_ref())
            .and_then(|s| s.desktop.filter(|&id| objects.is_alive(id)))
            .filter(|&d| self.registry.station_of_desktop(d) == Some(station));
        let d = match inherited {
            Some(d) => d,
            None => {
                let named = self.registry.desktop_by_name(station, &wide(DEFAULT_DESKTOP)).map(|d| d.object);
                named.unwrap_or_else(|| self.defaults(objects, caller).1)
            }
        };
        let _ = objects.reference(d);
        self.procs.entry(caller.process).or_default().desktop = Some(d);
        d
    }

    /// `user32` attaches: a process started on a named station and desktop needs both to exist
    /// and to be open to its token, with the rights a process needs there.
    fn connect(&mut self, objects: &mut poly_ntobj::ObjectManager, caller: &Caller) -> Result<(), ()> {
        let proc = self.procs.entry(caller.process).or_default();
        if proc.connected {
            return Ok(());
        }
        let start = proc.start.clone();
        let spec = start.as_ref().map(|s| s.spec.clone()).unwrap_or_default();
        let parsed = parse_spec(&spec);
        if !parsed.desktop.is_empty() || parsed.station.is_some() {
            // Named: the station (its own when the text only names a desktop) and the desktop.
            let station = match parsed.station {
                Some(n) => self.registry.station_by_name(n).map(|s| s.object),
                None => Some(self.station_of(objects, caller)),
            }
            .ok_or(())?;
            let desk_name: Vec<u16> =
                if parsed.desktop.is_empty() { wide(DEFAULT_DESKTOP) } else { parsed.desktop.to_vec() };
            let desktop = self.registry.desktop_by_name(station, &desk_name).map(|d| d.object).ok_or(())?;
            objects.access_for(caller, station, START_STATION_RIGHTS).map_err(|_| ())?;
            objects.access_for(caller, desktop, START_DESKTOP_RIGHTS).map_err(|_| ())?;
            let p = self.procs.entry(caller.process).or_default();
            for (slot, new) in [(&mut p.station, station), (&mut p.desktop, desktop)] {
                let _ = objects.reference(new);
                if let Some(old) = slot.replace(new) {
                    let _ = objects.dereference(old);
                }
            }
        } else {
            self.station_of(objects, caller);
            self.initial_desktop(objects, caller);
        }
        let p = self.procs.entry(caller.process).or_default();
        p.connected = true;
        if let Some(s) = p.start.take() {
            for id in s.station.into_iter().chain(s.desktop) {
                let _ = objects.dereference(id);
            }
        }
        Ok(())
    }
}

type Reply = WinstaRes;

fn err(code: u32) -> Reply {
    WinstaRes::Err { code }
}

/// A handle's object if it is of type `ty` (else the answer is `ERROR_INVALID_HANDLE`).
fn object_of(sys: &NtSystem, caller: &Caller, h: u64, ty: ObjectType) -> Option<ObjectId> {
    sys.objects.object_by_handle(caller, Handle(h), 0, Some(ty)).ok()
}

/// Make a handle to the existing or just made object `id`.
fn handle_with(sys: &mut NtSystem, caller: &Caller, id: ObjectId, access: u32, inherit: bool) -> Option<u64> {
    sys.objects.new_handle_exact(caller, id, access, inherit).ok().map(|h| h.0)
}

/// Run one operation for `caller` (a thread of `proc`). `sd` is the descriptor the call carried,
/// already read from the caller's memory.
pub fn run(sys: &mut NtSystem, caller: &Caller, proc: &NtProcess, op: &WinstaOp, sd: Option<Vec<u8>>) -> Reply {
    // A process that has not used its token yet gets it now: new objects take their descriptor from it.
    let _ = crate::security::process_token(sys, caller.process, caller.thread);
    match op {
        WinstaOp::CreateStation { name, flags, access, .. } => {
            if let Err(e) = check_station_name(name) {
                return err(e);
            }
            let existing = sys.winsta.registry.station_by_name(name).map(|s| s.object);
            if let Some(id) = existing {
                if flags & CWF_CREATE_ONLY != 0 {
                    return err(ERROR_ALREADY_EXISTS);
                }
                return handle_with(sys, caller, id, *access, false)
                    .map_or(err(ERROR_NOT_ENOUGH_MEMORY), |handle| WinstaRes::HandleClear { handle });
            }
            let creator = sys.objects.security.effective(caller.process, caller.thread);
            let Some(id) = sys.winsta.new_station(&mut sys.objects, creator, caller.thread, name, sd.as_deref()) else {
                return err(ERROR_NOT_ENOUGH_MEMORY);
            };
            let h = handle_with(sys, caller, id, *access, false);
            // The creation reference goes: the handle holds the station.
            let _ = sys.objects.dereference(id);
            h.map_or(err(ERROR_NOT_ENOUGH_MEMORY), |handle| WinstaRes::HandleClear { handle })
        }
        WinstaOp::OpenStation { name, inherit, access } => {
            if name.is_empty() {
                return err(ERROR_FILE_NOT_FOUND);
            }
            let Some(id) = sys.winsta.registry.station_by_name(name).map(|s| s.object) else {
                return err(ERROR_FILE_NOT_FOUND);
            };
            match sys.objects.open_by_pointer(caller, id, *access, if *inherit { OBJ_INHERIT } else { 0 }) {
                Ok(h) => WinstaRes::Handle { handle: h.0 },
                Err(_) => err(ERROR_ACCESS_DENIED),
            }
        }
        WinstaOp::CloseStation { handle } => {
            if object_of(sys, caller, *handle, ObjectType::WindowStation).is_none() {
                return err(ERROR_INVALID_HANDLE);
            }
            if sys.winsta.procs.get(&caller.process).is_some_and(|p| p.handle == *handle) {
                return err(ERROR_BUSY);
            }
            match sys.objects.close(caller, Handle(*handle)) {
                Ok(()) => WinstaRes::Done {},
                Err(_) => err(ERROR_INVALID_HANDLE),
            }
        }
        WinstaOp::ProcessStation {} => {
            let station = sys.winsta.station_of(&mut sys.objects, caller);
            let have = sys.winsta.procs.get(&caller.process).map_or(0, |p| p.handle);
            if have != 0 {
                return WinstaRes::Handle { handle: have };
            }
            match sys.objects.open_by_pointer(caller, station, MAXIMUM_ALLOWED, 0) {
                Ok(h) => {
                    sys.winsta.procs.entry(caller.process).or_default().handle = h.0;
                    WinstaRes::Handle { handle: h.0 }
                }
                Err(_) => err(ERROR_ACCESS_DENIED),
            }
        }
        WinstaOp::SetProcessStation { handle } => {
            let Some(id) = object_of(sys, caller, *handle, ObjectType::WindowStation) else {
                return err(ERROR_INVALID_HANDLE);
            };
            let _ = sys.objects.reference(id);
            let p = sys.winsta.procs.entry(caller.process).or_default();
            let old = p.station.replace(id);
            p.handle = *handle;
            if let Some(old) = old {
                let _ = sys.objects.dereference(old);
            }
            WinstaRes::Done {}
        }
        WinstaOp::CreateDesktop { name, has_device, flags, access, .. } => {
            if name.is_empty() {
                return err(ERROR_INVALID_HANDLE);
            }
            if *has_device || flags & !DF_ALLOWOTHERACCOUNTHOOK != 0 {
                return err(ERROR_INVALID_PARAMETER);
            }
            if *access == 0 {
                return err(ERROR_NOT_ENOUGH_MEMORY);
            }
            let station = sys.winsta.station_of(&mut sys.objects, caller);
            let existing = sys.winsta.registry.desktop_by_name(station, name).map(|d| d.object);
            if let Some(id) = existing {
                if sys.objects.access_for(caller, id, *access).is_err() {
                    return err(ERROR_ACCESS_DENIED);
                }
                return handle_with(sys, caller, id, *access, false)
                    .map_or(err(ERROR_NOT_ENOUGH_MEMORY), |handle| WinstaRes::Handle { handle });
            }
            let creator = sys.objects.security.effective(caller.process, caller.thread);
            let Some(id) =
                sys.winsta.new_desktop(&mut sys.objects, creator, caller.thread, station, name, sd.as_deref())
            else {
                return err(ERROR_NOT_ENOUGH_MEMORY);
            };
            // The creator must be granted what it asked for by the descriptor it gave.
            let granted = sys.objects.access_for(caller, id, *access).is_ok();
            let h = if granted { handle_with(sys, caller, id, *access, false) } else { None };
            let _ = sys.objects.dereference(id);
            match h {
                Some(handle) => WinstaRes::Handle { handle },
                None => err(if granted { ERROR_NOT_ENOUGH_MEMORY } else { ERROR_ACCESS_DENIED }),
            }
        }
        WinstaOp::OpenDesktop { name, inherit, access, .. } => {
            if name.is_empty() {
                return err(ERROR_FILE_NOT_FOUND);
            }
            let station = sys.winsta.station_of(&mut sys.objects, caller);
            let Some(id) = sys.winsta.registry.desktop_by_name(station, name).map(|d| d.object) else {
                return err(ERROR_FILE_NOT_FOUND);
            };
            match sys.objects.open_by_pointer(caller, id, *access, if *inherit { OBJ_INHERIT } else { 0 }) {
                Ok(h) => WinstaRes::Handle { handle: h.0 },
                Err(_) => err(ERROR_ACCESS_DENIED),
            }
        }
        WinstaOp::CloseDesktop { handle } => {
            if object_of(sys, caller, *handle, ObjectType::Desktop).is_none() {
                return err(ERROR_INVALID_HANDLE);
            }
            // A desktop a thread of the process is on cannot be closed through that handle.
            let in_use =
                proc.threads.iter().any(|t| sys.winsta.threads.get(&t.thread).is_some_and(|d| d.handle == *handle));
            if in_use {
                return err(ERROR_BUSY);
            }
            match sys.objects.close(caller, Handle(*handle)) {
                Ok(()) => WinstaRes::Done {},
                Err(_) => err(ERROR_INVALID_HANDLE),
            }
        }
        WinstaOp::ThreadDesktop { tid } => {
            let thread = ThreadRef(u64::from(*tid));
            if *tid == 0 || !proc.has_live_thread(thread) {
                return WinstaRes::Silent {};
            }
            if let Some(t) = sys.winsta.threads.get(&thread) {
                return WinstaRes::Handle { handle: t.handle };
            }
            let desktop = sys.winsta.initial_desktop(&mut sys.objects, caller);
            let Ok(h) = sys.objects.open_by_pointer(caller, desktop, MAXIMUM_ALLOWED, 0) else {
                return err(ERROR_ACCESS_DENIED);
            };
            let _ = sys.objects.reference(desktop);
            sys.winsta.threads.insert(thread, Thread { desktop, handle: h.0 });
            WinstaRes::Handle { handle: h.0 }
        }
        WinstaOp::SetThreadDesktop { handle } => {
            let Some(id) = object_of(sys, caller, *handle, ObjectType::Desktop) else {
                return err(ERROR_INVALID_HANDLE);
            };
            let _ = sys.objects.reference(id);
            let old = sys.winsta.threads.insert(caller.thread, Thread { desktop: id, handle: *handle });
            if let Some(old) = old {
                let _ = sys.objects.dereference(old.desktop);
            }
            WinstaRes::Done {}
        }
        WinstaOp::GetInfo { handle, index, capacity } => {
            let Some((kind, place, name, flags)) = info_target(sys, caller, *handle) else {
                // A NULL handle fails without touching the last error; anything else is no
                // station or desktop (`ERROR_INVALID_FUNCTION`).
                return if *handle == 0 { WinstaRes::Silent {} } else { err(ERROR_INVALID_FUNCTION) };
            };
            let _ = place;
            let granted = sys.objects.granted_access(caller, Handle(*handle)).unwrap_or(0);
            match object_info(kind, &name, flags, granted, *index) {
                Ok(bytes) if bytes.len() as u32 <= *capacity => WinstaRes::Info { data: bytes },
                Ok(bytes) => WinstaRes::Short { needed: bytes.len() as u32 },
                Err(code) => err(code),
            }
        }
        WinstaOp::SetInfo { handle, index, data } => {
            let Some((kind, place, _, _)) = info_target(sys, caller, *handle) else {
                return if *handle == 0 { WinstaRes::Silent {} } else { err(ERROR_INVALID_FUNCTION) };
            };
            if kind == Kind::Station {
                let granted = sys.objects.granted_access(caller, Handle(*handle)).unwrap_or(0);
                if granted & WINSTA_WRITEATTRIBUTES == 0 {
                    return err(ERROR_ACCESS_DENIED);
                }
            }
            let slot = match place {
                Place::Station(i) => sys.winsta.registry.station_mut(i).map(|s| &mut s.flags),
                Place::Desktop(i, j) => {
                    sys.winsta.registry.station_mut(i).and_then(|s| s.desktops.get_mut(j)).map(|d| &mut d.flags)
                }
            };
            let Some(flags) = slot else { return err(ERROR_INVALID_HANDLE) };
            match set_info(flags, *index, data) {
                Ok(()) => WinstaRes::Done {},
                Err(code) => err(code),
            }
        }
        WinstaOp::Connect {} => match sys.winsta.connect(&mut sys.objects, caller) {
            Ok(()) => WinstaRes::Done {},
            Err(()) => err(STATUS_DLL_INIT_FAILED),
        },
    }
}

/// What a handle of `GetUserObjectInformationW` names: its kind, where it is, its name and flags.
fn info_target(sys: &NtSystem, caller: &Caller, h: u64) -> Option<(Kind, Place, Vec<u16>, [u32; 3])> {
    let id = sys.objects.object_by_handle(caller, Handle(h), 0, None).ok()?;
    let place = sys.winsta.registry.place(id)?;
    match place {
        Place::Station(i) => {
            let s = sys.winsta.registry.station(i)?;
            Some((Kind::Station, place, s.name.clone(), s.flags))
        }
        Place::Desktop(i, j) => {
            let d = sys.winsta.registry.station(i)?.desktops.get(j)?;
            Some((Kind::Desktop, place, d.name.clone(), d.flags))
        }
    }
}

/// Run `op` for the calling thread (the `Request::Winsta` of `PolyUserCall`), reading the
/// descriptor it carries from the caller's memory first.
pub fn handle_call<K: NtKernel>(k: &mut K, op: &WinstaOp) -> Reply {
    let sd_addr = match op {
        WinstaOp::CreateStation { sd, .. } | WinstaOp::CreateDesktop { sd, .. } => *sd,
        _ => 0,
    };
    let sd = if sd_addr == 0 {
        None
    } else {
        match crate::abi::read_security_descriptor(k, sd_addr) {
            Ok(b) => Some(b),
            // `ERROR_INVALID_PARAMETER`: the caller's descriptor is not one.
            Err(_) => return err(ERROR_INVALID_PARAMETER),
        }
    };
    let caller = k.caller();
    k.with_state(|sys, proc, b| {
        let r = run(sys, &caller, proc, op, sd);
        sys.reap(b);
        r
    })
}
