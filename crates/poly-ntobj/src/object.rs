//! Objects: ids, types, the header every object has and the per-type body.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use core::fmt;

use crate::ProcessId;
use crate::access::*;
use crate::wait::{ThreadRef, WaitId};

/// Names an object. Carries a generation, so an id kept after the object
/// was deleted never reaches a new object in the same slot.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "obj{}.{}", self.index, self.generation)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// Object types. The first six have their semantics here; the others are
/// placeholders the kernel creates with [`crate::NewObject::Opaque`] and
/// fills in later (a `cookie` ties each to the kernel's own structure).
///
/// The type *names* ([`ObjectType::name`]) are what `NtQueryObject`
/// (`ObjectTypeInformation`) and `NtQueryDirectoryObject` report. To be
/// confirmed by the spec agent against real Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectType {
    Directory,
    SymbolicLink,
    Event,
    Mutant,
    Semaphore,
    Timer,
    Section,
    File,
    Process,
    Thread,
    Key,
    Device,
}

impl ObjectType {
    pub const ALL: [ObjectType; 12] = [
        ObjectType::Directory,
        ObjectType::SymbolicLink,
        ObjectType::Event,
        ObjectType::Mutant,
        ObjectType::Semaphore,
        ObjectType::Timer,
        ObjectType::Section,
        ObjectType::File,
        ObjectType::Process,
        ObjectType::Thread,
        ObjectType::Key,
        ObjectType::Device,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            ObjectType::Directory => "Directory",
            ObjectType::SymbolicLink => "SymbolicLink",
            ObjectType::Event => "Event",
            ObjectType::Mutant => "Mutant",
            ObjectType::Semaphore => "Semaphore",
            ObjectType::Timer => "Timer",
            ObjectType::Section => "Section",
            ObjectType::File => "File",
            ObjectType::Process => "Process",
            ObjectType::Thread => "Thread",
            ObjectType::Key => "Key",
            ObjectType::Device => "Device",
        }
    }

    /// Types whose behaviour the kernel provides ([`crate::NewObject::Opaque`]).
    pub const fn is_opaque(self) -> bool {
        matches!(
            self,
            ObjectType::Section
                | ObjectType::File
                | ObjectType::Process
                | ObjectType::Thread
                | ObjectType::Key
                | ObjectType::Device
        )
    }

    /// Every right a handle to this type can hold (the type's
    /// `*_ALL_ACCESS`).
    pub const fn valid_access(self) -> AccessMask {
        self.generic_mapping().all
    }

    /// The type's generic mapping.
    ///
    /// `GENERIC_ALL` maps to the type's `*_ALL_ACCESS` from `winnt.h`, and
    /// File and Key use the `FILE_GENERIC_*` and `KEY_READ`/`KEY_WRITE`/
    /// `KEY_EXECUTE` masks from `winnt.h`. The read/write/execute mappings
    /// of the other types are not documented: they follow the pattern
    /// "read = READ_CONTROL + query rights, write = READ_CONTROL + modify
    /// rights, execute = READ_CONTROL + SYNCHRONIZE (waitable types) or
    /// traverse/query (directories, links)" and are to be confirmed by the
    /// spec agent against real Windows. Process, Thread and Device are
    /// placeholders until those milestones.
    pub const fn generic_mapping(self) -> GenericMapping {
        const R: AccessMask = STANDARD_RIGHTS_READ;
        const W: AccessMask = STANDARD_RIGHTS_WRITE;
        const X: AccessMask = STANDARD_RIGHTS_EXECUTE;
        let (read, write, execute, all) = match self {
            ObjectType::Directory => (
                R | DIRECTORY_QUERY | DIRECTORY_TRAVERSE,
                W | DIRECTORY_CREATE_OBJECT | DIRECTORY_CREATE_SUBDIRECTORY,
                X | DIRECTORY_QUERY | DIRECTORY_TRAVERSE,
                DIRECTORY_ALL_ACCESS,
            ),
            ObjectType::SymbolicLink => (R | SYMBOLIC_LINK_QUERY, W, X | SYMBOLIC_LINK_QUERY, SYMBOLIC_LINK_ALL_ACCESS),
            ObjectType::Event => (R | EVENT_QUERY_STATE, W | EVENT_MODIFY_STATE, X | SYNCHRONIZE, EVENT_ALL_ACCESS),
            ObjectType::Mutant => (R | MUTANT_QUERY_STATE, W, X | SYNCHRONIZE, MUTANT_ALL_ACCESS),
            ObjectType::Semaphore => {
                (R | SEMAPHORE_QUERY_STATE, W | SEMAPHORE_MODIFY_STATE, X | SYNCHRONIZE, SEMAPHORE_ALL_ACCESS)
            }
            ObjectType::Timer => (R | TIMER_QUERY_STATE, W | TIMER_MODIFY_STATE, X | SYNCHRONIZE, TIMER_ALL_ACCESS),
            ObjectType::Section => (
                R | SECTION_QUERY | SECTION_MAP_READ,
                W | SECTION_MAP_WRITE,
                X | SECTION_MAP_EXECUTE,
                SECTION_ALL_ACCESS,
            ),
            ObjectType::File | ObjectType::Device => {
                (FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_GENERIC_EXECUTE, FILE_ALL_ACCESS)
            }
            ObjectType::Key => (KEY_READ, KEY_WRITE, KEY_EXECUTE, KEY_ALL_ACCESS),
            ObjectType::Process => (R, W, X | SYNCHRONIZE, PROCESS_ALL_ACCESS),
            ObjectType::Thread => (R, W, X | SYNCHRONIZE, THREAD_ALL_ACCESS),
        };
        GenericMapping { read, write, execute, all }
    }
}

/// `EVENT_TYPE` (`wdm.h`): `NotificationEvent` (manual reset) stays
/// signaled until reset; `SynchronizationEvent` (auto reset) resets when it
/// satisfies one wait. Timers (`TIMER_TYPE`) have the same two kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    Notification,
    Synchronization,
}

#[derive(Debug)]
pub(crate) struct Header {
    pub(crate) kind: ObjectType,
    /// The last name component, as created (case kept). `None` for unnamed
    /// objects and for named ones whose name was removed.
    pub(crate) name: Option<Vec<u16>>,
    /// The directory holding the name. Holds a pointer reference on it.
    pub(crate) parent: Option<ObjectId>,
    pub(crate) handle_count: u32,
    /// Includes one reference per handle.
    pub(crate) pointer_count: u32,
    pub(crate) permanent: bool,
    /// Created with `OBJ_EXCLUSIVE`.
    pub(crate) exclusive: bool,
    /// The process holding the exclusive handles, while there are any.
    pub(crate) exclusive_process: Option<ProcessId>,
    /// Stand-in for the security descriptor: the most a user-mode open may
    /// be granted.
    pub(crate) allowed: AccessMask,
    /// Kernel data for opaque objects (and anything else that wants one).
    pub(crate) cookie: u64,
    /// The object owns a sub-namespace: lookups that reach it go to the
    /// [`crate::ParseHook`].
    pub(crate) parse: bool,
    /// Waits blocked on this object, oldest first.
    pub(crate) waiters: VecDeque<WaitId>,
}

#[derive(Debug)]
pub(crate) enum Body {
    Directory(Directory),
    SymbolicLink {
        target: Vec<u16>,
    },
    Event {
        kind: EventType,
        signaled: bool,
    },
    Mutant(Mutant),
    Semaphore {
        count: i32,
        maximum: i32,
    },
    Timer {
        kind: EventType,
        signaled: bool,
        armed: bool,
    },
    /// `signaled` is `None` when the object cannot be waited on.
    Opaque {
        signaled: Option<bool>,
    },
}

#[derive(Debug, Default)]
pub(crate) struct Mutant {
    pub(crate) owner: Option<ThreadRef>,
    /// Recursion count; zero exactly when `owner` is `None`.
    pub(crate) count: u32,
    pub(crate) abandoned: bool,
}

/// A directory: names folded with [`crate::name::upcase`] map to the objects
/// with that folded name, oldest first, each with its exact name. Several
/// objects can share a folded name when they were created case-sensitively.
#[derive(Debug, Default)]
pub(crate) struct Directory {
    pub(crate) entries: BTreeMap<Vec<u16>, Vec<(Vec<u16>, ObjectId)>>,
}

impl Directory {
    pub(crate) fn find(&self, name: &[u16], case_insensitive: bool) -> Option<ObjectId> {
        let bucket = self.entries.get(&crate::name::fold(name))?;
        if case_insensitive {
            bucket.first().map(|(_, id)| *id)
        } else {
            bucket.iter().find(|(n, _)| n.as_slice() == name).map(|(_, id)| *id)
        }
    }

    pub(crate) fn insert(&mut self, name: &[u16], id: ObjectId) {
        self.entries.entry(crate::name::fold(name)).or_default().push((name.to_vec(), id));
    }

    pub(crate) fn remove(&mut self, name: &[u16], id: ObjectId) {
        let key = crate::name::fold(name);
        if let Some(bucket) = self.entries.get_mut(&key) {
            bucket.retain(|(_, o)| *o != id);
            if bucket.is_empty() {
                self.entries.remove(&key);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug)]
pub(crate) struct Object {
    pub(crate) header: Header,
    pub(crate) body: Body,
}

impl Object {
    pub(crate) fn new(kind: ObjectType, body: Body) -> Object {
        Object {
            header: Header {
                kind,
                name: None,
                parent: None,
                handle_count: 0,
                pointer_count: 0,
                permanent: false,
                exclusive: false,
                exclusive_process: None,
                allowed: kind.valid_access(),
                cookie: 0,
                parse: false,
                waiters: VecDeque::new(),
            },
            body,
        }
    }
}
