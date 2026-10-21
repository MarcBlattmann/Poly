//! Handle tables: one per process plus one for kernel handles.
//!
//! # Handle values
//!
//! - A handle is a multiple of 4. `wdm.h` defines `OBJ_HANDLE_TAGBITS` as
//!   `0x3`: the two low bits are ignored by the system and free for the
//!   application to use as tags, so every lookup masks them off.
//! - `0` (`NULL`) is never a handle. Handle values start at 4.
//! - A freed value is reused, lowest first. To be confirmed by the spec
//!   agent against real Windows (reuse order is not documented).
//! - Pseudo handles (`wdm.h` / Learn "GetCurrentProcess",
//!   "GetCurrentThread"): `NtCurrentProcess()` is `(HANDLE)-1` and
//!   `NtCurrentThread()` is `(HANDLE)-2`. They are not in any table and
//!   always have full access to the current process or thread.
//! - Kernel handles (`OBJ_KERNEL_HANDLE`) live in their own table. We give
//!   them the sign-extended high bits `0xFFFF_FFFF_8000_0000`, so a user-mode
//!   caller presenting one gets `STATUS_INVALID_HANDLE`. The encoding is
//!   ours; to be confirmed by the spec agent against real Windows.
//! - A table holds at most [`MAX_HANDLES`] handles; then creating one fails
//!   with `STATUS_INSUFFICIENT_RESOURCES`. The limit is to be confirmed by
//!   the spec agent against real Windows.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::fmt;

use crate::access::{self, AccessMask, Mode, PROCESS_ALL_ACCESS, THREAD_ALL_ACCESS};
use crate::object::{ObjectId, ObjectType};
use crate::status::*;
use crate::wait::ThreadRef;
use crate::{
    Caller, DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DUPLICATE_SAME_ATTRIBUTES, OBJ_INHERIT, OBJ_KERNEL_HANDLE,
    OBJ_PROTECT_CLOSE, ObjectManager, ProcessId,
};

/// A handle value as user mode sees it (`HANDLE`, pointer sized).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u64);

/// `OBJ_HANDLE_TAGBITS` (`wdm.h`).
pub const OBJ_HANDLE_TAGBITS: u64 = 0x3;

/// Most handles one table holds.
pub const MAX_HANDLES: usize = 1 << 24;

const KERNEL_HANDLE_BITS: u64 = 0xFFFF_FFFF_8000_0000;

impl Handle {
    pub const NULL: Handle = Handle(0);
    /// `NtCurrentProcess()`: `(HANDLE)-1`.
    pub const CURRENT_PROCESS: Handle = Handle(u64::MAX);
    /// `NtCurrentThread()`: `(HANDLE)-2`.
    pub const CURRENT_THREAD: Handle = Handle(u64::MAX - 1);

    pub const fn is_pseudo(self) -> bool {
        self.0 == Self::CURRENT_PROCESS.0 || self.0 == Self::CURRENT_THREAD.0
    }

    /// A handle in the kernel handle table.
    pub const fn is_kernel(self) -> bool {
        !self.is_pseudo() && self.0 & KERNEL_HANDLE_BITS == KERNEL_HANDLE_BITS
    }

    const fn from_index(index: u32, kernel: bool) -> Handle {
        let v = (index as u64 + 1) * 4;
        Handle(if kernel { v | KERNEL_HANDLE_BITS } else { v })
    }

    /// The table index, ignoring the tag bits; `None` for `NULL`.
    fn index(self) -> Option<u32> {
        let v = if self.is_kernel() { self.0 & !KERNEL_HANDLE_BITS } else { self.0 };
        let v = v & !OBJ_HANDLE_TAGBITS;
        if v == 0 || v > (MAX_HANDLES as u64) * 4 {
            return None;
        }
        Some((v / 4 - 1) as u32)
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Handle::CURRENT_PROCESS => f.write_str("NtCurrentProcess"),
            Handle::CURRENT_THREAD => f.write_str("NtCurrentThread"),
            Handle(v) => write!(f, "Handle({v:#x})"),
        }
    }
}

/// A handle's attributes (`OBJECT_HANDLE_FLAG_INFORMATION`, Learn:
/// "SetHandleInformation": `HANDLE_FLAG_INHERIT`,
/// `HANDLE_FLAG_PROTECT_FROM_CLOSE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HandleFlags {
    pub inherit: bool,
    pub protect_from_close: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HandleEntry {
    pub(crate) object: ObjectId,
    pub(crate) access: AccessMask,
    pub(crate) flags: HandleFlags,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct HandleTable {
    entries: Vec<Option<HandleEntry>>,
    /// Free indices below `entries.len()`.
    free: BTreeSet<u32>,
    count: usize,
}

impl HandleTable {
    pub(crate) const fn new() -> HandleTable {
        HandleTable { entries: Vec::new(), free: BTreeSet::new(), count: 0 }
    }

    fn insert(&mut self, entry: HandleEntry) -> Result<u32> {
        if self.count >= MAX_HANDLES {
            return Err(STATUS_INSUFFICIENT_RESOURCES);
        }
        self.count += 1;
        if let Some(index) = self.free.pop_first() {
            self.entries[index as usize] = Some(entry);
            Ok(index)
        } else {
            self.entries.push(Some(entry));
            Ok((self.entries.len() - 1) as u32)
        }
    }

    fn get(&self, index: u32) -> Option<&HandleEntry> {
        self.entries.get(index as usize)?.as_ref()
    }

    fn get_mut(&mut self, index: u32) -> Option<&mut HandleEntry> {
        self.entries.get_mut(index as usize)?.as_mut()
    }

    fn remove(&mut self, index: u32) -> Option<HandleEntry> {
        let e = self.entries.get_mut(index as usize)?.take()?;
        self.count -= 1;
        self.free.insert(index);
        Some(e)
    }

    fn iter(&self) -> impl Iterator<Item = (u32, &HandleEntry)> {
        self.entries.iter().enumerate().filter_map(|(i, e)| e.as_ref().map(|e| (i as u32, e)))
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    /// The inheritable entries at the same indices (Learn: "Handle
    /// Inheritance": an inherited handle has the same value and access).
    fn inheritable(&self) -> HandleTable {
        let entries: Vec<_> = self.entries.iter().map(|e| e.filter(|e| e.flags.inherit)).collect();
        let free = entries.iter().enumerate().filter(|(_, e)| e.is_none()).map(|(i, _)| i as u32).collect();
        let count = entries.iter().filter(|e| e.is_some()).count();
        HandleTable { entries, free, count }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ProcessRecord {
    pub(crate) table: HandleTable,
    /// The process object `NtCurrentProcess()` refers to.
    pub(crate) object: Option<ObjectId>,
}

/// Where a handle value points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resolved {
    pub(crate) object: ObjectId,
    pub(crate) access: AccessMask,
    pub(crate) flags: HandleFlags,
    /// `None` for pseudo handles; else the table (`None` = kernel) and index.
    pub(crate) slot: Option<(Option<ProcessId>, u32)>,
}

impl ObjectManager {
    /// Give a process its handle table. With `inherit_from`, the parent's
    /// inheritable handles are copied at the same values with the same
    /// access (Learn: "Handle Inheritance"). Inheriting a handle to an
    /// `OBJ_EXCLUSIVE` object is allowed; to be confirmed by the spec agent.
    pub fn create_process(&mut self, process: ProcessId, inherit_from: Option<ProcessId>) -> Result<()> {
        if self.processes.contains_key(&process) {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let table = match inherit_from {
            Some(parent) => self.processes.get(&parent).ok_or(STATUS_INVALID_PARAMETER)?.table.inheritable(),
            None => HandleTable::new(),
        };
        let objects: Vec<ObjectId> = table.iter().map(|(_, e)| e.object).collect();
        for id in objects {
            self.add_handle(id, Some(process));
        }
        self.processes.insert(process, ProcessRecord { table, object: None });
        Ok(())
    }

    /// Close every handle of an exiting process, protected ones included,
    /// and drop its table.
    pub fn destroy_process(&mut self, process: ProcessId) -> Result<()> {
        let record = self.processes.remove(&process).ok_or(STATUS_INVALID_PARAMETER)?;
        for (_, e) in record.table.iter() {
            self.release_handle(e.object);
        }
        Ok(())
    }

    /// The object `NtCurrentProcess()` means in `process`. The record holds
    /// no reference; the kernel keeps the process object alive.
    pub fn set_process_object(&mut self, process: ProcessId, object: ObjectId) -> Result<()> {
        if self.object_type(object) != Some(ObjectType::Process) {
            return Err(STATUS_OBJECT_TYPE_MISMATCH);
        }
        self.processes.get_mut(&process).ok_or(STATUS_INVALID_PARAMETER)?.object = Some(object);
        Ok(())
    }

    /// The object `NtCurrentThread()` means for `thread`, or `None` to
    /// forget it. Holds no reference.
    pub fn set_thread_object(&mut self, thread: ThreadRef, object: Option<ObjectId>) -> Result<()> {
        match object {
            Some(o) if self.object_type(o) != Some(ObjectType::Thread) => return Err(STATUS_OBJECT_TYPE_MISMATCH),
            Some(o) => self.threads.insert(thread, o),
            None => self.threads.remove(&thread),
        };
        Ok(())
    }

    /// The process object `NtCurrentProcess()` means in `process`, if it was
    /// set ([`Self::set_process_object`]). Used by `NtOpenProcess` by client
    /// id: the kernel maps the pid to its process and asks for its object.
    pub fn process_object(&self, process: ProcessId) -> Option<ObjectId> {
        self.processes.get(&process).and_then(|r| r.object).filter(|&o| self.is_alive(o))
    }

    /// The thread object of `thread`, if it was set
    /// ([`Self::set_thread_object`]). Used by `NtOpenThread` by client id.
    pub fn thread_object(&self, thread: ThreadRef) -> Option<ObjectId> {
        self.threads.get(&thread).copied().filter(|&o| self.is_alive(o))
    }

    /// Open handles in a process's table.
    pub fn process_handle_count(&self, process: ProcessId) -> Option<usize> {
        self.processes.get(&process).map(|r| r.table.len())
    }

    /// Open kernel handles.
    pub fn kernel_handle_count(&self) -> usize {
        self.kernel_handles.len()
    }

    fn table_mut(&mut self, process: Option<ProcessId>) -> Result<&mut HandleTable> {
        match process {
            None => Ok(&mut self.kernel_handles),
            Some(p) => self.processes.get_mut(&p).map(|r| &mut r.table).ok_or(STATUS_INVALID_PARAMETER),
        }
    }

    fn table(&self, process: Option<ProcessId>) -> Result<&HandleTable> {
        match process {
            None => Ok(&self.kernel_handles),
            Some(p) => self.processes.get(&p).map(|r| &r.table).ok_or(STATUS_INVALID_PARAMETER),
        }
    }

    /// Add a handle to `object` in `process`'s table, or the kernel table
    /// for `None`.
    pub(crate) fn new_handle(
        &mut self,
        process: Option<ProcessId>,
        object: ObjectId,
        access: AccessMask,
        flags: HandleFlags,
        owner: ProcessId,
    ) -> Result<Handle> {
        let index = self.table_mut(process)?.insert(HandleEntry { object, access, flags })?;
        self.add_handle(object, Some(owner));
        Ok(Handle::from_index(index, process.is_none()))
    }

    /// The table a new handle for `caller` goes to, given its `OBJ_*`
    /// attributes. `OBJ_KERNEL_HANDLE` only counts from kernel mode; from
    /// user mode it is ignored. To be confirmed by the spec agent against
    /// real Windows (it may fail with `STATUS_INVALID_PARAMETER` instead).
    pub(crate) fn target_table(caller: &Caller, attributes: u32) -> Option<ProcessId> {
        if attributes & OBJ_KERNEL_HANDLE != 0 && caller.mode == Mode::Kernel { None } else { Some(caller.process) }
    }

    /// Resolve a handle value of `process` (pseudo handles relative to it
    /// and `thread`).
    pub(crate) fn resolve_in(&self, mode: Mode, process: ProcessId, thread: ThreadRef, h: Handle) -> Result<Resolved> {
        let full = HandleFlags::default();
        match h {
            Handle::CURRENT_PROCESS => {
                let record = self.processes.get(&process).ok_or(STATUS_INVALID_HANDLE)?;
                let object = record.object.filter(|&o| self.is_alive(o)).ok_or(STATUS_INVALID_HANDLE)?;
                return Ok(Resolved { object, access: PROCESS_ALL_ACCESS, flags: full, slot: None });
            }
            Handle::CURRENT_THREAD => {
                let object =
                    self.threads.get(&thread).copied().filter(|&o| self.is_alive(o)).ok_or(STATUS_INVALID_HANDLE)?;
                return Ok(Resolved { object, access: THREAD_ALL_ACCESS, flags: full, slot: None });
            }
            _ => {}
        }
        let table = if h.is_kernel() {
            if mode == Mode::User {
                return Err(STATUS_INVALID_HANDLE);
            }
            None
        } else {
            Some(process)
        };
        let index = h.index().ok_or(STATUS_INVALID_HANDLE)?;
        let e = self.table(table).map_err(|_| STATUS_INVALID_HANDLE)?.get(index).ok_or(STATUS_INVALID_HANDLE)?;
        Ok(Resolved { object: e.object, access: e.access, flags: e.flags, slot: Some((table, index)) })
    }

    pub(crate) fn resolve(&self, caller: &Caller, h: Handle) -> Result<Resolved> {
        self.resolve_in(caller.mode, caller.process, caller.thread, h)
    }

    /// Look up a handle without taking a reference, checking its type
    /// (`STATUS_OBJECT_TYPE_MISMATCH`) and then that it was granted
    /// `required` (`STATUS_ACCESS_DENIED`, skipped for kernel-mode callers).
    /// The order of the two checks is to be confirmed by the spec agent.
    pub fn object_by_handle(
        &self,
        caller: &Caller,
        h: Handle,
        required: AccessMask,
        expected: Option<ObjectType>,
    ) -> Result<ObjectId> {
        let r = self.resolve(caller, h)?;
        if let Some(t) = expected
            && self.obj(r.object).header.kind != t
        {
            return Err(STATUS_OBJECT_TYPE_MISMATCH);
        }
        access::check(r.access, required, caller.mode)?;
        Ok(r.object)
    }

    /// `ObReferenceObjectByHandle`: [`Self::object_by_handle`] plus a pointer
    /// reference the caller later drops with [`Self::dereference`].
    pub fn reference_by_handle(
        &mut self,
        caller: &Caller,
        h: Handle,
        required: AccessMask,
        expected: Option<ObjectType>,
    ) -> Result<ObjectId> {
        let id = self.object_by_handle(caller, h, required, expected)?;
        self.add_pointer(id);
        Ok(id)
    }

    /// The access a handle was granted (`NtQueryObject`,
    /// `ObjectBasicInformation`.`GrantedAccess`).
    pub fn granted_access(&self, caller: &Caller, h: Handle) -> Result<AccessMask> {
        Ok(self.resolve(caller, h)?.access)
    }

    /// `NtClose`.
    ///
    /// - Not a handle: `STATUS_INVALID_HANDLE`.
    /// - Protected from close: `STATUS_HANDLE_NOT_CLOSABLE`, and the handle
    ///   stays open. To be confirmed by the spec agent against real Windows.
    /// - A pseudo handle: `STATUS_SUCCESS` and nothing happens ("Calling
    ///   CloseHandle with a pseudo handle has no effect", Learn:
    ///   "GetCurrentProcess"); whether `NtClose` itself returns success is
    ///   to be confirmed by the spec agent.
    pub fn close(&mut self, caller: &Caller, h: Handle) -> Result<()> {
        let r = self.resolve(caller, h)?;
        let Some((table, index)) = r.slot else { return Ok(()) };
        if r.flags.protect_from_close {
            return Err(STATUS_HANDLE_NOT_CLOSABLE);
        }
        self.close_slot(table, index);
        Ok(())
    }

    fn close_slot(&mut self, table: Option<ProcessId>, index: u32) {
        let e = self.table_mut(table).expect("table exists").remove(index).expect("handle exists");
        self.release_handle(e.object);
        self.flush_deferred();
    }

    /// `NtSetInformationObject(ObjectHandleFlagInformation)`.
    pub fn set_handle_flags(&mut self, caller: &Caller, h: Handle, flags: HandleFlags) -> Result<()> {
        let r = self.resolve(caller, h)?;
        let Some((table, index)) = r.slot else { return Err(STATUS_INVALID_HANDLE) };
        self.table_mut(table)?.get_mut(index).ok_or(STATUS_INVALID_HANDLE)?.flags = flags;
        Ok(())
    }

    /// `NtQueryObject(ObjectHandleFlagInformation)`.
    pub fn handle_flags(&self, caller: &Caller, h: Handle) -> Result<HandleFlags> {
        Ok(self.resolve(caller, h)?.flags)
    }

    /// `NtDuplicateObject`. The kernel resolves the two process handles
    /// first and checks `PROCESS_DUP_HANDLE` on them; this works on the
    /// process ids.
    ///
    /// - `source` is resolved in `source_process`; pseudo handles mean that
    ///   process (and, for `NtCurrentThread()`, the caller's thread) and give
    ///   a real handle (Learn: "GetCurrentProcess").
    /// - `target_process` `None` is only allowed with
    ///   `DUPLICATE_CLOSE_SOURCE`: the source is closed and no handle made.
    ///   Otherwise `STATUS_INVALID_PARAMETER`; to be confirmed by the spec
    ///   agent.
    /// - `DUPLICATE_SAME_ACCESS` copies the source's access; otherwise
    ///   `desired` goes through the same check as an open, so the new handle
    ///   may have more access than the source (Learn: "DuplicateHandle" says
    ///   this can happen "in some cases"; to be confirmed by the spec agent).
    /// - `DUPLICATE_SAME_ATTRIBUTES` copies the source's flags; otherwise
    ///   `attributes` gives them (`OBJ_INHERIT`, `OBJ_PROTECT_CLOSE`).
    /// - `DUPLICATE_CLOSE_SOURCE` closes the source "regardless of any error
    ///   status returned" (Learn: "DuplicateHandle"), unless the source is
    ///   invalid or protected from close; a protected source stays open (to
    ///   be confirmed by the spec agent).
    /// - An `OBJ_EXCLUSIVE` object cannot be duplicated into another process:
    ///   `STATUS_ACCESS_DENIED` (to be confirmed by the spec agent).
    /// - Unknown option bits: `STATUS_INVALID_PARAMETER` (to be confirmed).
    #[allow(clippy::too_many_arguments)]
    pub fn nt_duplicate_object(
        &mut self,
        caller: &Caller,
        source_process: ProcessId,
        source: Handle,
        target_process: Option<ProcessId>,
        desired: AccessMask,
        attributes: u32,
        options: u32,
    ) -> Result<Option<Handle>> {
        if options & !(DUPLICATE_CLOSE_SOURCE | DUPLICATE_SAME_ACCESS | DUPLICATE_SAME_ATTRIBUTES) != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        if !self.processes.contains_key(&source_process) {
            return Err(STATUS_INVALID_HANDLE);
        }
        let src = self.resolve_in(caller.mode, source_process, caller.thread, source)?;
        let result = self.duplicate_resolved(caller, &src, target_process, desired, attributes, options);
        if options & DUPLICATE_CLOSE_SOURCE != 0
            && let Some((table, index)) = src.slot
            && !src.flags.protect_from_close
        {
            self.close_slot(table, index);
        }
        result
    }

    fn duplicate_resolved(
        &mut self,
        caller: &Caller,
        src: &Resolved,
        target_process: Option<ProcessId>,
        desired: AccessMask,
        attributes: u32,
        options: u32,
    ) -> Result<Option<Handle>> {
        let Some(target) = target_process else {
            return if options & DUPLICATE_CLOSE_SOURCE != 0 { Ok(None) } else { Err(STATUS_INVALID_PARAMETER) };
        };
        if !self.processes.contains_key(&target) {
            return Err(STATUS_INVALID_HANDLE);
        }
        let header = &self.obj(src.object).header;
        if header.exclusive_process.is_some_and(|p| p != target) {
            return Err(STATUS_ACCESS_DENIED);
        }
        let access = if options & DUPLICATE_SAME_ACCESS != 0 {
            src.access
        } else {
            access::grant(header.kind, desired, header.allowed, caller.mode, false, caller.privileges)?
        };
        let flags = if options & DUPLICATE_SAME_ATTRIBUTES != 0 {
            src.flags
        } else {
            HandleFlags {
                inherit: attributes & OBJ_INHERIT != 0,
                protect_from_close: attributes & OBJ_PROTECT_CLOSE != 0,
            }
        };
        let table =
            if attributes & OBJ_KERNEL_HANDLE != 0 && caller.mode == Mode::Kernel { None } else { Some(target) };
        self.new_handle(table, src.object, access, flags, target).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_tag_bits() {
        assert_eq!(Handle::from_index(0, false), Handle(4));
        assert_eq!(Handle::from_index(1, false), Handle(8));
        assert_eq!(Handle(4).index(), Some(0));
        assert_eq!(Handle(7).index(), Some(0));
        assert_eq!(Handle(9).index(), Some(1));
        assert_eq!(Handle(0).index(), None);
        assert_eq!(Handle(3).index(), None);
        let k = Handle::from_index(0, true);
        assert!(k.is_kernel());
        assert_eq!(k.index(), Some(0));
        assert_eq!(Handle(k.0 | 2).index(), Some(0));
        assert!(!Handle::CURRENT_PROCESS.is_kernel());
        assert!(Handle::CURRENT_PROCESS.is_pseudo() && Handle::CURRENT_THREAD.is_pseudo());
        assert!(!Handle(8).is_kernel());
    }

    #[test]
    fn table_reuses_lowest_free_index() {
        let id = ObjectId { index: 0, generation: 0 };
        let e = HandleEntry { object: id, access: 0, flags: HandleFlags::default() };
        let mut t = HandleTable::new();
        assert_eq!((t.insert(e), t.insert(e), t.insert(e)), (Ok(0), Ok(1), Ok(2)));
        t.remove(2);
        t.remove(0);
        assert_eq!(t.len(), 1);
        assert_eq!(t.insert(e), Ok(0));
        assert_eq!(t.insert(e), Ok(2));
        assert_eq!(t.insert(e), Ok(3));
    }
}
