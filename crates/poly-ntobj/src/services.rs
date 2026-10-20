//! The handle-level services: what the NT system calls do once the
//! personality has copied their arguments in. Names follow the services
//! (`nt_create_event` is `NtCreateEvent`).

use alloc::vec::Vec;

use crate::access::{self, *};
use crate::handle::{Handle, HandleFlags};
use crate::namespace::{Found, Intent, NoParse, ParseHook, Walk};
use crate::object::{Body, Directory, EventType, Mutant, Object, ObjectId, ObjectType};
use crate::status::*;
use crate::wait::{EventInfo, MutantInfo, SemaphoreInfo, Timeout, WaitOutcome, WaitType};
use crate::*;

/// What `NtQueryObject(ObjectBasicInformation)` reports
/// (`PUBLIC_OBJECT_BASIC_INFORMATION`, Learn).
///
/// `attributes` holds `OBJ_INHERIT` and `OBJ_PROTECT_CLOSE` from the handle
/// and `OBJ_PERMANENT` and `OBJ_EXCLUSIVE` from the object; the counts are
/// as they stand (Windows may count the query's own reference, or report
/// the pointer count with a bias). Both points are to be confirmed by the
/// spec agent against real Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectBasicInformation {
    pub attributes: u32,
    pub granted_access: AccessMask,
    pub handle_count: u32,
    pub pointer_count: u32,
}

impl NewObject<'_> {
    /// The new object, unnamed and unreferenced; a mutant created with
    /// `initial_owner` is owned by `thread`. Validates the parameters.
    fn build(&self, thread: ThreadRef) -> Result<Object> {
        let (kind, body) = self.body()?;
        let mut object = Object::new(kind, body);
        if let NewObject::Opaque { cookie, parse, .. } = *self {
            object.header.cookie = cookie;
            object.header.parse = parse;
        }
        if let (NewObject::Mutant { initial_owner: true }, Body::Mutant(m)) = (self, &mut object.body) {
            m.owner = Some(thread);
            m.count = 1;
        }
        Ok(object)
    }

    fn body(&self) -> Result<(ObjectType, Body)> {
        Ok(match *self {
            NewObject::Directory => (ObjectType::Directory, Body::Directory(Directory::default())),
            NewObject::SymbolicLink { target } => {
                (ObjectType::SymbolicLink, Body::SymbolicLink { target: target.to_vec() })
            }
            NewObject::Event { kind, signaled } => (ObjectType::Event, Body::Event { kind, signaled }),
            NewObject::Mutant { .. } => (ObjectType::Mutant, Body::Mutant(Mutant::default())),
            NewObject::Semaphore { initial, maximum } => {
                // Learn, "CreateSemaphoreW": the maximum must be greater than
                // zero and the initial count between zero and the maximum.
                if maximum <= 0 || initial < 0 || initial > maximum {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                (ObjectType::Semaphore, Body::Semaphore { count: initial, maximum })
            }
            NewObject::Timer { kind } => (ObjectType::Timer, Body::Timer { kind, signaled: false, armed: false }),
            NewObject::Opaque { kind, signaled, .. } => {
                if !kind.is_opaque() {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                (kind, Body::Opaque { signaled })
            }
        })
    }
}

impl ObjectManager {
    fn check_attributes(attributes: u32) -> Result<()> {
        if attributes & !OBJ_VALID_ATTRIBUTES != 0 { Err(STATUS_INVALID_PARAMETER) } else { Ok(()) }
    }

    /// Resolve the root directory handle, with a reference.
    fn root_object(&mut self, caller: &Caller, attrs: &ObjectAttributes<'_>) -> Result<Option<ObjectId>> {
        attrs.root.map(|h| self.reference_by_handle(caller, h, 0, None)).transpose()
    }

    /// `OBJ_EXCLUSIVE` rules for a new handle to an existing object in
    /// `process`: another process's exclusive object is
    /// `STATUS_ACCESS_DENIED`; asking for exclusive access to an object not
    /// created exclusive is `STATUS_INVALID_PARAMETER`. Both statuses are to
    /// be confirmed by the spec agent against real Windows.
    fn check_exclusive(&self, id: ObjectId, process: ProcessId, attributes: u32) -> Result<()> {
        let h = &self.obj(id).header;
        if h.exclusive_process.is_some_and(|p| p != process) {
            return Err(STATUS_ACCESS_DENIED);
        }
        if attributes & OBJ_EXCLUSIVE != 0 && !h.exclusive {
            return Err(STATUS_INVALID_PARAMETER);
        }
        Ok(())
    }

    /// A new handle to an existing object, after the exclusive and access
    /// checks.
    fn open_existing(&mut self, caller: &Caller, id: ObjectId, desired: AccessMask, attributes: u32) -> Result<Handle> {
        self.check_exclusive(id, caller.process, attributes)?;
        let h = &self.obj(id).header;
        let force = attributes & OBJ_FORCE_ACCESS_CHECK != 0;
        let granted = access::grant(h.kind, desired, h.allowed, caller.mode, force, caller.privileges)?;
        let flags = HandleFlags { inherit: attributes & OBJ_INHERIT != 0, protect_from_close: false };
        self.new_handle(Self::target_table(caller, attributes), id, granted, flags, caller.process)
    }

    /// Create an object and a handle to it (`NtCreateEvent`,
    /// `NtCreateDirectoryObject`, ... share this).
    ///
    /// - Unnamed (no name, or an empty one without a root directory): a new
    ///   object only the handle reaches. `OBJ_PERMANENT` only applies to
    ///   named objects (Learn: "OBJECT_ATTRIBUTES") and is ignored.
    /// - Named and new: the object gets the name; `status` is
    ///   `STATUS_SUCCESS`.
    /// - Named and taken: with `OBJ_OPENIF` and the same type, the existing
    ///   object is opened (its state is not changed by the new parameters)
    ///   and `status` is `STATUS_OBJECT_NAME_EXISTS`; with `OBJ_OPENIF` and
    ///   another type, `STATUS_OBJECT_TYPE_MISMATCH` (Learn: "CreateEventW":
    ///   `ERROR_ALREADY_EXISTS`, and `ERROR_INVALID_HANDLE` for another
    ///   object type, which `STATUS_OBJECT_TYPE_MISMATCH` maps to); without
    ///   it, `STATUS_OBJECT_NAME_COLLISION`.
    /// - `OBJ_PERMANENT` from user mode needs
    ///   [`Privileges::create_permanent`], else `STATUS_PRIVILEGE_NOT_HELD`.
    /// - Unknown attribute bits: `STATUS_INVALID_PARAMETER` (to be confirmed
    ///   by the spec agent).
    /// - A relative name with a root directory handle and an empty name:
    ///   `STATUS_OBJECT_NAME_INVALID` (to be confirmed by the spec agent).
    /// - Whether the root directory handle needs `DIRECTORY_CREATE_OBJECT`
    ///   access is to be confirmed by the spec agent; it is not checked.
    pub fn create_object<H: ParseHook + ?Sized>(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        new: NewObject<'_>,
        hook: &mut H,
    ) -> Result<Created> {
        Self::check_attributes(attrs.attributes)?;
        let mut object = new.build(caller.thread)?;
        let kind = object.header.kind;
        let a = attrs.attributes;
        let permanent = a & OBJ_PERMANENT != 0;
        if permanent && caller.mode == Mode::User && !caller.privileges.create_permanent {
            return Err(STATUS_PRIVILEGE_NOT_HELD);
        }
        let name = attrs.name.unwrap_or(&[]);
        if name.is_empty() && attrs.root.is_some() {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        let granted = access::grant(kind, desired, kind.valid_access(), caller.mode, false, caller.privileges)?;
        let flags = HandleFlags { inherit: a & OBJ_INHERIT != 0, protect_from_close: false };
        let table = Self::target_table(caller, a);

        object.header.exclusive = a & OBJ_EXCLUSIVE != 0;

        if name.is_empty() {
            let id = self.insert(object);
            self.add_pointer(id);
            let handle = self.new_handle(table, id, granted, flags, caller.process);
            self.release_pointer(id);
            return handle.map(|handle| Created { handle, object: id, status: STATUS_SUCCESS });
        }

        let start = self.root_object(caller, attrs)?;
        let walk = Walk {
            start,
            path: name,
            case_insensitive: a & OBJ_CASE_INSENSITIVE != 0,
            follow_final_link: false,
            dont_reparse: a & OBJ_DONT_REPARSE != 0,
            intent: Intent::Create,
            object_type: Some(kind),
            desired_access: desired,
            caller: Some(*caller),
        };
        let found = self.walk(&walk, hook);
        if let Some(s) = start {
            self.release_pointer(s);
        }
        match found? {
            Found::Parsed(made) => {
                // A parse hook made or found it for us.
                let result = if self.obj(made).header.kind != kind {
                    Err(STATUS_OBJECT_TYPE_MISMATCH)
                } else {
                    self.open_existing(caller, made, desired, a).map(|handle| Created {
                        handle,
                        object: made,
                        status: STATUS_SUCCESS,
                    })
                };
                self.release_pointer(made);
                result
            }
            Found::Object(existing) => {
                let result = if a & OBJ_OPENIF == 0 {
                    Err(STATUS_OBJECT_NAME_COLLISION)
                } else if self.obj(existing).header.kind != kind {
                    Err(STATUS_OBJECT_TYPE_MISMATCH)
                } else {
                    self.open_existing(caller, existing, desired, a).map(|handle| Created {
                        handle,
                        object: existing,
                        status: STATUS_OBJECT_NAME_EXISTS,
                    })
                };
                self.release_pointer(existing);
                result
            }
            Found::Missing { parent, name } => {
                object.header.permanent = permanent;
                let id = self.insert(object);
                self.link(parent, &name, id);
                self.release_pointer(parent);
                self.add_pointer(id);
                let handle = self.new_handle(table, id, granted, flags, caller.process);
                self.release_pointer(id);
                handle.map(|handle| Created { handle, object: id, status: STATUS_SUCCESS })
            }
        }
    }

    /// `ObCreateObject` for kernel code (a parse hook making a file object,
    /// the kernel making a process object): an unnamed object with one
    /// pointer reference for the caller and no handle. A mutant created
    /// with `initial_owner` is owned by `thread`.
    pub fn new_object(&mut self, new: NewObject<'_>, thread: ThreadRef) -> Result<ObjectId> {
        let id = self.insert(new.build(thread)?);
        self.add_pointer(id);
        Ok(id)
    }

    /// Open a named object (`NtOpenEvent`, `NtOpenDirectoryObject`, ...).
    ///
    /// - The name is required: none (or empty without a root directory) is
    ///   `STATUS_OBJECT_PATH_SYNTAX_BAD` (to be confirmed by the spec agent);
    ///   an empty name with a root directory opens the root directory itself.
    /// - Not there: `STATUS_OBJECT_NAME_NOT_FOUND`, or
    ///   `STATUS_OBJECT_PATH_NOT_FOUND` for a missing directory on the way.
    /// - `expected` names the type the service opens; another type is
    ///   `STATUS_OBJECT_TYPE_MISMATCH`.
    /// - `OBJ_OPENIF` means nothing here.
    pub fn open_object<H: ParseHook + ?Sized>(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        expected: Option<ObjectType>,
        hook: &mut H,
    ) -> Result<Handle> {
        Self::check_attributes(attrs.attributes)?;
        let a = attrs.attributes;
        let name = attrs.name.unwrap_or(&[]);
        if name.is_empty() && attrs.root.is_none() {
            return Err(STATUS_OBJECT_PATH_SYNTAX_BAD);
        }
        let start = self.root_object(caller, attrs)?;
        let walk = Walk {
            start,
            path: name,
            case_insensitive: a & OBJ_CASE_INSENSITIVE != 0,
            follow_final_link: a & OBJ_OPENLINK == 0 && expected != Some(ObjectType::SymbolicLink),
            dont_reparse: a & OBJ_DONT_REPARSE != 0,
            intent: Intent::Open,
            object_type: expected,
            desired_access: desired,
            caller: Some(*caller),
        };
        let found = self.walk(&walk, hook);
        if let Some(s) = start {
            self.release_pointer(s);
        }
        match found? {
            Found::Missing { parent, .. } => {
                self.release_pointer(parent);
                Err(STATUS_OBJECT_NAME_NOT_FOUND)
            }
            Found::Object(id) | Found::Parsed(id) => {
                let result = if expected.is_some_and(|t| t != self.obj(id).header.kind) {
                    Err(STATUS_OBJECT_TYPE_MISMATCH)
                } else {
                    self.open_existing(caller, id, desired, a)
                };
                self.release_pointer(id);
                result
            }
        }
    }

    // ----- NtQueryObject, NtMakeTemporaryObject, NtMakePermanentObject

    /// `NtQueryObject(ObjectBasicInformation)`.
    pub fn nt_query_object_basic(&self, caller: &Caller, h: Handle) -> Result<ObjectBasicInformation> {
        let r = self.resolve(caller, h)?;
        let header = &self.obj(r.object).header;
        let mut attributes = 0;
        if r.flags.inherit {
            attributes |= OBJ_INHERIT;
        }
        if r.flags.protect_from_close {
            attributes |= OBJ_PROTECT_CLOSE;
        }
        if header.permanent {
            attributes |= OBJ_PERMANENT;
        }
        if header.exclusive {
            attributes |= OBJ_EXCLUSIVE;
        }
        Ok(ObjectBasicInformation {
            attributes,
            granted_access: r.access,
            handle_count: header.handle_count,
            pointer_count: header.pointer_count,
        })
    }

    /// `NtQueryObject(ObjectNameInformation)`: the full path, empty for an
    /// unnamed object.
    pub fn nt_query_object_name(&self, caller: &Caller, h: Handle) -> Result<Vec<u16>> {
        let r = self.resolve(caller, h)?;
        Ok(self.object_name(r.object).unwrap_or_default())
    }

    /// `NtQueryObject(ObjectTypeInformation)`: the type name.
    pub fn nt_query_object_type(&self, caller: &Caller, h: Handle) -> Result<&'static str> {
        let r = self.resolve(caller, h)?;
        Ok(self.obj(r.object).header.kind.name())
    }

    /// `NtMakeTemporaryObject`: the object loses its name once its last
    /// handle closes (Learn: "ZwMakeTemporaryObject"). Needs `DELETE`
    /// access on the handle (to be confirmed by the spec agent).
    pub fn nt_make_temporary_object(&mut self, caller: &Caller, h: Handle) -> Result<()> {
        let id = self.object_by_handle(caller, h, DELETE, None)?;
        self.make_temporary(id)
    }

    /// Make an object temporary by id; an object with no handles loses its
    /// name (and is deleted without references) at once.
    pub fn make_temporary(&mut self, id: ObjectId) -> Result<()> {
        let h = &mut self.get_mut(id).ok_or(STATUS_INVALID_HANDLE)?.header;
        h.permanent = false;
        if h.handle_count == 0 && h.parent.is_some() {
            self.unlink(id);
        }
        Ok(())
    }

    /// `NtMakePermanentObject`: needs [`Privileges::create_permanent`] from
    /// user mode (`STATUS_PRIVILEGE_NOT_HELD`).
    pub fn nt_make_permanent_object(&mut self, caller: &Caller, h: Handle) -> Result<()> {
        let id = self.object_by_handle(caller, h, 0, None)?;
        if caller.mode == Mode::User && !caller.privileges.create_permanent {
            return Err(STATUS_PRIVILEGE_NOT_HELD);
        }
        self.obj_mut(id).header.permanent = true;
        Ok(())
    }

    // ----- directories and symbolic links

    /// `NtCreateDirectoryObject`.
    pub fn nt_create_directory_object(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Created> {
        self.create_object(caller, attrs, desired, NewObject::Directory, &mut NoParse)
    }

    /// `NtOpenDirectoryObject`.
    pub fn nt_open_directory_object(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Handle> {
        self.open_object(caller, attrs, desired, Some(ObjectType::Directory), &mut NoParse)
    }

    /// `NtQueryDirectoryObject`, all entries at once: (name, type name).
    /// Needs `DIRECTORY_QUERY`.
    pub fn nt_query_directory_object(&self, caller: &Caller, h: Handle) -> Result<Vec<(Vec<u16>, &'static str)>> {
        let id = self.object_by_handle(caller, h, DIRECTORY_QUERY, Some(ObjectType::Directory))?;
        self.directory_entries(id)
    }

    /// `NtCreateSymbolicLinkObject`.
    pub fn nt_create_symbolic_link_object(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        target: &[u16],
    ) -> Result<Created> {
        self.create_object(caller, attrs, desired, NewObject::SymbolicLink { target }, &mut NoParse)
    }

    /// `NtOpenSymbolicLinkObject`: opens the link itself.
    pub fn nt_open_symbolic_link_object(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Handle> {
        self.open_object(caller, attrs, desired, Some(ObjectType::SymbolicLink), &mut NoParse)
    }

    /// `NtQuerySymbolicLinkObject`. Needs `SYMBOLIC_LINK_QUERY`.
    pub fn nt_query_symbolic_link_object(&self, caller: &Caller, h: Handle) -> Result<Vec<u16>> {
        let id = self.object_by_handle(caller, h, SYMBOLIC_LINK_QUERY, Some(ObjectType::SymbolicLink))?;
        self.link_target(id)
    }

    // ----- events

    /// `NtCreateEvent`.
    pub fn nt_create_event(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        kind: EventType,
        signaled: bool,
    ) -> Result<Created> {
        self.create_object(caller, attrs, desired, NewObject::Event { kind, signaled }, &mut NoParse)
    }

    /// `NtOpenEvent`.
    pub fn nt_open_event(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Handle> {
        self.open_object(caller, attrs, desired, Some(ObjectType::Event), &mut NoParse)
    }

    /// `NtSetEvent`: needs `EVENT_MODIFY_STATE` (Learn: "SetEvent").
    /// Returns the previous state.
    pub fn nt_set_event(&mut self, caller: &Caller, h: Handle) -> Result<bool> {
        let id = self.object_by_handle(caller, h, EVENT_MODIFY_STATE, Some(ObjectType::Event))?;
        self.set_event(id)
    }

    /// `NtResetEvent`: needs `EVENT_MODIFY_STATE` (Learn: "ResetEvent").
    /// Returns the previous state.
    pub fn nt_reset_event(&mut self, caller: &Caller, h: Handle) -> Result<bool> {
        let id = self.object_by_handle(caller, h, EVENT_MODIFY_STATE, Some(ObjectType::Event))?;
        self.reset_event(id)
    }

    /// `NtClearEvent`: `NtResetEvent` without the previous state.
    pub fn nt_clear_event(&mut self, caller: &Caller, h: Handle) -> Result<()> {
        self.nt_reset_event(caller, h).map(drop)
    }

    /// `NtPulseEvent`: needs `EVENT_MODIFY_STATE` (Learn: "PulseEvent").
    /// Returns the previous state.
    pub fn nt_pulse_event(&mut self, caller: &Caller, h: Handle) -> Result<bool> {
        let id = self.object_by_handle(caller, h, EVENT_MODIFY_STATE, Some(ObjectType::Event))?;
        self.pulse_event(id)
    }

    /// `NtQueryEvent(EventBasicInformation)`: needs `EVENT_QUERY_STATE`.
    pub fn nt_query_event(&self, caller: &Caller, h: Handle) -> Result<EventInfo> {
        let id = self.object_by_handle(caller, h, EVENT_QUERY_STATE, Some(ObjectType::Event))?;
        self.event_info(id)
    }

    // ----- mutants

    /// `NtCreateMutant`: with `initial_owner` the calling thread owns it
    /// once.
    pub fn nt_create_mutant(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        initial_owner: bool,
    ) -> Result<Created> {
        self.create_object(caller, attrs, desired, NewObject::Mutant { initial_owner }, &mut NoParse)
    }

    /// `NtOpenMutant`.
    pub fn nt_open_mutant(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Handle> {
        self.open_object(caller, attrs, desired, Some(ObjectType::Mutant), &mut NoParse)
    }

    /// `NtReleaseMutant` by the calling thread. No access right is checked:
    /// Learn documents none for releasing a mutex (`MUTEX_MODIFY_STATE` is
    /// "reserved for future use"); to be confirmed by the spec agent.
    pub fn nt_release_mutant(&mut self, caller: &Caller, h: Handle) -> Result<u32> {
        let id = self.object_by_handle(caller, h, 0, Some(ObjectType::Mutant))?;
        self.release_mutant(id, caller.thread)
    }

    /// `NtQueryMutant(MutantBasicInformation)`: needs `MUTANT_QUERY_STATE`.
    pub fn nt_query_mutant(&self, caller: &Caller, h: Handle) -> Result<MutantInfo> {
        let id = self.object_by_handle(caller, h, MUTANT_QUERY_STATE, Some(ObjectType::Mutant))?;
        self.mutant_info(id)
    }

    // ----- semaphores

    /// `NtCreateSemaphore`.
    pub fn nt_create_semaphore(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        initial: i32,
        maximum: i32,
    ) -> Result<Created> {
        self.create_object(caller, attrs, desired, NewObject::Semaphore { initial, maximum }, &mut NoParse)
    }

    /// `NtOpenSemaphore`.
    pub fn nt_open_semaphore(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Handle> {
        self.open_object(caller, attrs, desired, Some(ObjectType::Semaphore), &mut NoParse)
    }

    /// `NtReleaseSemaphore`: needs `SEMAPHORE_MODIFY_STATE` (Learn:
    /// "ReleaseSemaphore"). Returns the previous count.
    pub fn nt_release_semaphore(&mut self, caller: &Caller, h: Handle, count: i32) -> Result<i32> {
        let id = self.object_by_handle(caller, h, SEMAPHORE_MODIFY_STATE, Some(ObjectType::Semaphore))?;
        self.release_semaphore(id, count)
    }

    /// `NtQuerySemaphore(SemaphoreBasicInformation)`: needs
    /// `SEMAPHORE_QUERY_STATE`.
    pub fn nt_query_semaphore(&self, caller: &Caller, h: Handle) -> Result<SemaphoreInfo> {
        let id = self.object_by_handle(caller, h, SEMAPHORE_QUERY_STATE, Some(ObjectType::Semaphore))?;
        self.semaphore_info(id)
    }

    // ----- timers

    /// `NtCreateTimer`.
    pub fn nt_create_timer(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
        kind: EventType,
    ) -> Result<Created> {
        self.create_object(caller, attrs, desired, NewObject::Timer { kind }, &mut NoParse)
    }

    /// `NtOpenTimer`.
    pub fn nt_open_timer(
        &mut self,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
        desired: AccessMask,
    ) -> Result<Handle> {
        self.open_object(caller, attrs, desired, Some(ObjectType::Timer), &mut NoParse)
    }

    /// `NtSetTimer`, the state half: needs `TIMER_MODIFY_STATE`. Returns the
    /// previous signal state; the kernel keeps the due time.
    pub fn nt_set_timer(&mut self, caller: &Caller, h: Handle) -> Result<(ObjectId, bool)> {
        let id = self.object_by_handle(caller, h, TIMER_MODIFY_STATE, Some(ObjectType::Timer))?;
        self.set_timer(id).map(|s| (id, s))
    }

    /// `NtCancelTimer`: needs `TIMER_MODIFY_STATE`. Returns the signal
    /// state.
    pub fn nt_cancel_timer(&mut self, caller: &Caller, h: Handle) -> Result<(ObjectId, bool)> {
        let id = self.object_by_handle(caller, h, TIMER_MODIFY_STATE, Some(ObjectType::Timer))?;
        self.cancel_timer(id).map(|s| (id, s))
    }

    // ----- waits

    /// `NtWaitForSingleObject`.
    pub fn nt_wait_for_single_object(
        &mut self,
        caller: &Caller,
        h: Handle,
        alertable: bool,
        timeout: Timeout,
    ) -> Result<WaitOutcome> {
        self.nt_wait_for_multiple_objects(caller, &[h], WaitType::Any, alertable, timeout)
    }

    /// `NtWaitForMultipleObjects`: every handle needs `SYNCHRONIZE`
    /// (Learn: "WaitForMultipleObjects"). The count is checked before the
    /// handles. Two handles to the same object in a wait-all are the same
    /// object twice (`STATUS_INVALID_PARAMETER_MIX`; to be confirmed by the
    /// spec agent).
    pub fn nt_wait_for_multiple_objects(
        &mut self,
        caller: &Caller,
        handles: &[Handle],
        wait_type: WaitType,
        alertable: bool,
        timeout: Timeout,
    ) -> Result<WaitOutcome> {
        if handles.is_empty() || handles.len() > crate::MAXIMUM_WAIT_OBJECTS {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let objects =
            handles.iter().map(|&h| self.object_by_handle(caller, h, SYNCHRONIZE, None)).collect::<Result<Vec<_>>>()?;
        self.wait_objects(caller.thread, &objects, wait_type, alertable, timeout)
    }

    // ----- handles

    /// `NtClose`; see [`ObjectManager::close`].
    pub fn nt_close(&mut self, caller: &Caller, h: Handle) -> Result<()> {
        self.close(caller, h)
    }
}
