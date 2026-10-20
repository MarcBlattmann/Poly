//! The object manager: object storage, reference counts and lifetime.
//!
//! # Reference counts
//!
//! Every object has a *handle count* (open handles in any table) and a
//! *pointer count* (references held by code, one of which every handle
//! also holds), as `NtQueryObject`'s `PUBLIC_OBJECT_BASIC_INFORMATION`
//! reports them (Learn: "PUBLIC_OBJECT_BASIC_INFORMATION").
//!
//! - When the handle count drops to zero, a named object that is not
//!   permanent loses its name: it leaves its directory and can no longer be
//!   opened (Learn: "Life Cycle of an Object"; `OBJ_PERMANENT` in
//!   "OBJECT_ATTRIBUTES"; `ZwMakeTemporaryObject`).
//! - When the pointer count drops to zero and the object has no name, it is
//!   deleted. A permanent object keeps its name, so it lives on with no
//!   handles and no references until it is made temporary.
//! - A named object holds a pointer reference on its directory, so a
//!   directory outlives the names in it.
//! - A blocked wait holds a pointer reference on each object it waits on,
//!   so closing the last handle during a wait does not delete the object
//!   (what the wait then does is undefined per Learn, "WaitForMultipleObjects";
//!   we keep waiting).
//!
//! Deleting an opaque object queues a [`Deleted`] notice so the kernel can
//! free what the object's cookie names.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use crate::ProcessId;
use crate::access::AccessMask;
use crate::handle::{HandleTable, ProcessRecord};
use crate::name::{SEPARATOR, wide};
use crate::object::{Body, Directory, Object, ObjectId, ObjectType};
use crate::slab::Slab;
use crate::status::*;
use crate::wait::{ThreadRef, WaitBlock, Wake};

/// An opaque object was deleted: the kernel frees what `cookie` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deleted {
    pub id: ObjectId,
    pub kind: ObjectType,
    pub cookie: u64,
}

/// The NT object manager's state: every object, the namespace, the handle
/// tables and the dispatcher's waits. The kernel keeps one behind a lock.
#[derive(Debug)]
pub struct ObjectManager {
    pub(crate) objects: Slab<Object>,
    pub(crate) root: ObjectId,
    pub(crate) processes: BTreeMap<ProcessId, ProcessRecord>,
    pub(crate) kernel_handles: HandleTable,
    pub(crate) threads: BTreeMap<ThreadRef, ObjectId>,
    pub(crate) waits: Slab<WaitBlock>,
    pub(crate) wakes: VecDeque<Wake>,
    pub(crate) deleted: VecDeque<Deleted>,
    /// Pointer references to drop once the current operation is done, so
    /// nothing is deleted while a loop still looks at it.
    pub(crate) deferred: Vec<ObjectId>,
}

/// The directories every namespace starts with.
///
/// - `\` the root, `\Device`, `\BaseNamedObjects`, `\Sessions` (empty, a
///   placeholder for per-session namespaces), `\GLOBAL??`, `\KernelObjects`
///   (Learn: "CreateMemoryResourceNotification" names objects there).
/// - `\??` is a plain directory for now. On Windows it is the caller's
///   DOS-devices directory, searched before `\GLOBAL??` (Learn: "Local and
///   Global MS-DOS Device Names"); that per-session lookup comes with
///   sessions and is to be confirmed by the spec agent against real Windows.
/// - Symbolic links: `\DosDevices` → `\??`, and `\BaseNamedObjects\Global`
///   and `\BaseNamedObjects\Local` → `\BaseNamedObjects`, so the `Global\`
///   and `Local\` prefixes of "Kernel object namespaces" (Learn) work while
///   there is one session. To be confirmed by the spec agent.
pub const WELL_KNOWN_DIRECTORIES: [&str; 6] =
    ["\\??", "\\GLOBAL??", "\\Device", "\\BaseNamedObjects", "\\Sessions", "\\KernelObjects"];

/// The symbolic links every namespace starts with: (link, target).
pub const WELL_KNOWN_LINKS: [(&str, &str); 3] = [
    ("\\DosDevices", "\\??"),
    ("\\BaseNamedObjects\\Global", "\\BaseNamedObjects"),
    ("\\BaseNamedObjects\\Local", "\\BaseNamedObjects"),
];

impl Default for ObjectManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectManager {
    /// A namespace with the root, the well-known directories and links,
    /// all permanent.
    pub fn new() -> ObjectManager {
        let mut objects = Slab::new();
        let mut root = Object::new(ObjectType::Directory, Body::Directory(Directory::default()));
        root.header.permanent = true;
        // The manager's own reference: the root has no name to keep it.
        root.header.pointer_count = 1;
        let (index, generation) = objects.insert(root);
        let mut om = ObjectManager {
            objects,
            root: ObjectId { index, generation },
            processes: BTreeMap::new(),
            kernel_handles: HandleTable::new(),
            threads: BTreeMap::new(),
            waits: Slab::new(),
            wakes: VecDeque::new(),
            deleted: VecDeque::new(),
            deferred: Vec::new(),
        };
        for path in WELL_KNOWN_DIRECTORIES {
            om.insert_permanent(path, Object::new(ObjectType::Directory, Body::Directory(Directory::default())));
        }
        for (path, target) in WELL_KNOWN_LINKS {
            om.insert_permanent(
                path,
                Object::new(ObjectType::SymbolicLink, Body::SymbolicLink { target: wide(target) }),
            );
        }
        om
    }

    /// Put a permanent object at an absolute path whose parent exists.
    fn insert_permanent(&mut self, path: &str, mut object: Object) -> ObjectId {
        let path = wide(path);
        let cut = path.iter().rposition(|&c| c == SEPARATOR).expect("absolute path");
        let parent = if cut == 0 { self.root } else { self.lookup(&path[..cut], true).expect("parent exists") };
        object.header.permanent = true;
        let id = self.insert(object);
        self.link(parent, &path[cut + 1..], id);
        id
    }

    /// The root directory `\`.
    pub fn root(&self) -> ObjectId {
        self.root
    }

    pub(crate) fn insert(&mut self, object: Object) -> ObjectId {
        let (index, generation) = self.objects.insert(object);
        ObjectId { index, generation }
    }

    pub(crate) fn get(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(id.index, id.generation)
    }

    pub(crate) fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.objects.get_mut(id.index, id.generation)
    }

    /// The object, which the caller knows is alive.
    pub(crate) fn obj(&self, id: ObjectId) -> &Object {
        self.get(id).expect("live object")
    }

    pub(crate) fn obj_mut(&mut self, id: ObjectId) -> &mut Object {
        self.get_mut(id).expect("live object")
    }

    /// Name `id` as `name` in directory `parent`.
    pub(crate) fn link(&mut self, parent: ObjectId, name: &[u16], id: ObjectId) {
        let Body::Directory(dir) = &mut self.obj_mut(parent).body else { panic!("parent is not a directory") };
        dir.insert(name, id);
        self.obj_mut(parent).header.pointer_count += 1;
        let h = &mut self.obj_mut(id).header;
        h.name = Some(name.to_vec());
        h.parent = Some(parent);
    }

    /// Remove `id`'s name from its directory.
    pub(crate) fn unlink(&mut self, id: ObjectId) {
        let h = &mut self.obj_mut(id).header;
        let (Some(parent), Some(name)) = (h.parent.take(), h.name.take()) else { return };
        if let Body::Directory(dir) = &mut self.obj_mut(parent).body {
            dir.remove(&name, id);
        }
        self.release_pointer(parent);
        self.delete_if_unused(id);
    }

    /// Take a pointer reference.
    pub(crate) fn add_pointer(&mut self, id: ObjectId) {
        self.obj_mut(id).header.pointer_count += 1;
    }

    /// Drop a pointer reference; delete the object if that was the last and
    /// it has no name.
    pub(crate) fn release_pointer(&mut self, id: ObjectId) {
        let h = &mut self.obj_mut(id).header;
        h.pointer_count = h.pointer_count.checked_sub(1).expect("pointer count underflow");
        self.delete_if_unused(id);
    }

    fn delete_if_unused(&mut self, id: ObjectId) {
        let h = &self.obj(id).header;
        if h.pointer_count == 0 && h.parent.is_none() {
            debug_assert_eq!(h.handle_count, 0);
            debug_assert!(h.waiters.is_empty());
            let object = self.objects.remove(id.index, id.generation).expect("live object");
            match object.body {
                Body::Directory(dir) => debug_assert!(dir.is_empty()),
                Body::Opaque { .. } => {
                    self.deleted.push_back(Deleted { id, kind: object.header.kind, cookie: object.header.cookie })
                }
                _ => {}
            }
        }
    }

    /// A new handle refers to `id`.
    pub(crate) fn add_handle(&mut self, id: ObjectId, process: Option<ProcessId>) {
        let h = &mut self.obj_mut(id).header;
        h.handle_count += 1;
        h.pointer_count += 1;
        if h.exclusive && h.exclusive_process.is_none() {
            h.exclusive_process = process;
        }
    }

    /// A handle to `id` was closed.
    pub(crate) fn release_handle(&mut self, id: ObjectId) {
        let h = &mut self.obj_mut(id).header;
        h.handle_count = h.handle_count.checked_sub(1).expect("handle count underflow");
        if h.handle_count == 0 {
            h.exclusive_process = None;
            if !h.permanent && h.parent.is_some() {
                self.unlink(id);
            }
        }
        self.release_pointer(id);
    }

    /// Drop the references deferred during an operation.
    pub(crate) fn flush_deferred(&mut self) {
        while let Some(id) = self.deferred.pop() {
            self.release_pointer(id);
        }
    }

    // ----- kernel-facing references (ObReferenceObject / ObDereferenceObject)

    /// Take a pointer reference on a live object (`ObReferenceObject`).
    pub fn reference(&mut self, id: ObjectId) -> Result<()> {
        let o = self.get_mut(id).ok_or(STATUS_INVALID_PARAMETER)?;
        o.header.pointer_count += 1;
        Ok(())
    }

    /// Drop a pointer reference (`ObDereferenceObject`). The object is
    /// deleted if it was the last one and the object has no name.
    pub fn dereference(&mut self, id: ObjectId) -> Result<()> {
        let o = self.get(id).ok_or(STATUS_INVALID_PARAMETER)?;
        if o.header.pointer_count <= o.header.handle_count {
            // Would leave fewer references than handles: not one of ours.
            return Err(STATUS_INVALID_PARAMETER);
        }
        self.release_pointer(id);
        Ok(())
    }

    // ----- inspection

    pub fn is_alive(&self, id: ObjectId) -> bool {
        self.get(id).is_some()
    }

    pub fn object_type(&self, id: ObjectId) -> Option<ObjectType> {
        self.get(id).map(|o| o.header.kind)
    }

    pub fn handle_count(&self, id: ObjectId) -> Option<u32> {
        self.get(id).map(|o| o.header.handle_count)
    }

    pub fn pointer_count(&self, id: ObjectId) -> Option<u32> {
        self.get(id).map(|o| o.header.pointer_count)
    }

    pub fn is_permanent(&self, id: ObjectId) -> Option<bool> {
        self.get(id).map(|o| o.header.permanent)
    }

    /// The kernel data given at creation.
    pub fn cookie(&self, id: ObjectId) -> Option<u64> {
        self.get(id).map(|o| o.header.cookie)
    }

    /// Live objects, the root included.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Narrow (or widen) what user-mode opens may be granted on an object:
    /// the stand-in for its DACL until security descriptors exist.
    pub fn set_allowed_access(&mut self, id: ObjectId, allowed: AccessMask) -> Result<()> {
        let o = self.get_mut(id).ok_or(STATUS_INVALID_PARAMETER)?;
        o.header.allowed = allowed & o.header.kind.valid_access();
        Ok(())
    }

    /// The object's full path (`\BaseNamedObjects\Foo`), `\` for the root,
    /// and `None` when it has no name (`NtQueryObject`,
    /// `ObjectNameInformation`). An object whose directory lost its own
    /// name reports the part of the path that is left; to be confirmed by
    /// the spec agent against real Windows.
    pub fn object_name(&self, id: ObjectId) -> Option<Vec<u16>> {
        if id == self.root {
            return Some(alloc::vec![SEPARATOR]);
        }
        let mut parts: Vec<&[u16]> = Vec::new();
        let mut at = id;
        loop {
            let h = &self.get(at)?.header;
            let (Some(name), Some(parent)) = (&h.name, h.parent) else { break };
            parts.push(name);
            at = parent;
        }
        if parts.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for p in parts.iter().rev() {
            out.push(SEPARATOR);
            out.extend_from_slice(p);
        }
        Some(out)
    }

    /// Next deleted opaque object, oldest first.
    pub fn pop_deleted(&mut self) -> Option<Deleted> {
        self.deleted.pop_front()
    }
}
