//! The object namespace: walking a path through directories, symbolic
//! links and objects that own a sub-namespace.
//!
//! # Path rules
//!
//! Without a root directory a name must be fully qualified, starting with
//! `\`; with one it is relative and must not (Learn:
//! "InitializeObjectAttributes"). Either way breaks with
//! `STATUS_OBJECT_PATH_SYNTAX_BAD`. An empty component (`\a\\b`, or a
//! trailing `\`) is `STATUS_OBJECT_NAME_INVALID`. A missing last component
//! is `STATUS_OBJECT_NAME_NOT_FOUND` when opening, and a missing directory
//! on the way is `STATUS_OBJECT_PATH_NOT_FOUND` (these two are the
//! documented meanings). An object on the way that is neither a directory,
//! a symbolic link nor a parse object gives `STATUS_OBJECT_TYPE_MISMATCH`.
//! The statuses for syntax errors, empty components and non-directories on
//! the way are to be confirmed by the spec agent against real Windows.
//!
//! # Symbolic links
//!
//! A symbolic link met on the way is replaced by its target and the walk
//! starts over from the root (the target must be fully qualified, else
//! `STATUS_OBJECT_PATH_SYNTAX_BAD`). A link as the last component is
//! followed when opening, unless the caller opens a symbolic link object
//! itself or passes `OBJ_OPENLINK`; it is never followed when creating, so
//! creating over a link's name collides with the link. With
//! `OBJ_DONT_REPARSE` meeting a link fails with
//! `STATUS_REPARSE_POINT_ENCOUNTERED` (Learn: "OBJECT_ATTRIBUTES"). After
//! [`MAX_REPARSE`] link or parse-hook reparses the walk fails with
//! `STATUS_REPARSE_POINT_NOT_RESOLVED`. Whether `OBJ_DONT_REPARSE` applies
//! to object-manager links, the limit and its status are to be confirmed by
//! the spec agent against real Windows.
//!
//! # Parse hook
//!
//! An object created with `parse: true` (a device, later a file system or
//! the registry) owns the names below it. When a walk reaches one, it hands
//! the rest of the path to the [`ParseHook`] the caller passed, which opens
//! or creates an object, asks for a reparse with a new full path, or fails.

use alloc::vec::Vec;

use crate::access::AccessMask;
use crate::object::{Body, ObjectId, ObjectType};
use crate::status::*;
use crate::{Caller, ObjectManager};

/// Most symbolic-link or parse-hook reparses one walk takes.
pub const MAX_REPARSE: u32 = 32;

/// Opening an existing object or creating a new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Open,
    Create,
}

/// What a walk hands a parse object's [`ParseHook`].
#[derive(Debug)]
pub struct ParseRequest<'a> {
    pub object: ObjectId,
    pub kind: ObjectType,
    pub cookie: u64,
    /// The rest of the path after the parse object: empty, or starting with
    /// `\` (or, when the root directory handle itself is the parse object,
    /// the whole relative name).
    pub remaining: &'a [u16],
    pub case_insensitive: bool,
    pub intent: Intent,
    /// The type being opened or created, when the service knows it.
    pub object_type: Option<ObjectType>,
    pub desired_access: AccessMask,
    pub caller: Option<Caller>,
}

/// A parse hook's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    /// The object found or made. The hook passes one pointer reference
    /// (taken with [`ObjectManager::reference`] or by creating the object
    /// through the manager) to the walk.
    Object(ObjectId),
    /// Start over with this fully qualified path (counts toward
    /// [`MAX_REPARSE`]).
    Reparse(Vec<u16>),
    Fail(NtStatus),
}

/// Parse procedures for objects that own a sub-namespace.
pub trait ParseHook {
    fn parse(&mut self, om: &mut ObjectManager, request: &ParseRequest<'_>) -> Parsed;
}

/// For callers with no parse procedures: any parse object fails the walk
/// with `STATUS_OBJECT_PATH_NOT_FOUND`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoParse;

impl ParseHook for NoParse {
    fn parse(&mut self, _: &mut ObjectManager, _: &ParseRequest<'_>) -> Parsed {
        Parsed::Fail(STATUS_OBJECT_PATH_NOT_FOUND)
    }
}

/// What to walk and how.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Walk<'a> {
    /// The root directory handle's object, for relative names.
    pub(crate) start: Option<ObjectId>,
    pub(crate) path: &'a [u16],
    pub(crate) case_insensitive: bool,
    pub(crate) follow_final_link: bool,
    pub(crate) dont_reparse: bool,
    pub(crate) intent: Intent,
    pub(crate) object_type: Option<ObjectType>,
    pub(crate) desired_access: AccessMask,
    pub(crate) caller: Option<Caller>,
}

/// Where a walk ended. Every id carries a pointer reference for the caller.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// An object in the namespace.
    Object(ObjectId),
    /// An object a parse hook found or made.
    Parsed(ObjectId),
    /// The last component is free; `parent` is the directory it would go in.
    Missing { parent: ObjectId, name: Vec<u16> },
}

const SEP: u16 = crate::name::SEPARATOR;

impl ObjectManager {
    pub(crate) fn walk<H: ParseHook + ?Sized>(&mut self, w: &Walk<'_>, hook: &mut H) -> Result<Found> {
        let mut path: Vec<u16> = w.path.to_vec();
        let mut start = w.start;
        let mut reparses = 0u32;
        'restart: loop {
            let mut pos = match start {
                None if path.first() == Some(&SEP) => 1,
                Some(_) if path.first() != Some(&SEP) => 0,
                _ => return Err(STATUS_OBJECT_PATH_SYNTAX_BAD),
            };
            let mut cur = start.unwrap_or(self.root);
            let next = if self.obj(cur).header.kind == ObjectType::Directory {
                None
            } else if self.obj(cur).header.parse {
                Some(self.call_parse(cur, &path[pos..], w, hook))
            } else {
                return Err(STATUS_OBJECT_TYPE_MISMATCH);
            };
            if let Some(parsed) = next {
                match parsed? {
                    Ok(found) => return Ok(found),
                    Err(new_path) => {
                        reparses = self.count_reparse(reparses, w)?;
                        path = new_path;
                        start = None;
                        continue 'restart;
                    }
                }
            }
            if pos == path.len() {
                self.add_pointer(cur);
                return Ok(Found::Object(cur));
            }
            loop {
                let end = path[pos..].iter().position(|&c| c == SEP).map_or(path.len(), |i| pos + i);
                let component = &path[pos..end];
                if component.is_empty() {
                    return Err(STATUS_OBJECT_NAME_INVALID);
                }
                let last = end == path.len();
                let Body::Directory(dir) = &self.obj(cur).body else { unreachable!("walk only enters directories") };
                let Some(child) = dir.find(component, w.case_insensitive) else {
                    if !last {
                        return Err(STATUS_OBJECT_PATH_NOT_FOUND);
                    }
                    self.add_pointer(cur);
                    return Ok(Found::Missing { parent: cur, name: component.to_vec() });
                };
                let child_obj = self.obj(child);
                match &child_obj.body {
                    Body::SymbolicLink { target } if !last || w.follow_final_link => {
                        let mut new_path = target.clone();
                        new_path.extend_from_slice(&path[end..]);
                        reparses = self.count_reparse(reparses, w)?;
                        path = new_path;
                        start = None;
                        continue 'restart;
                    }
                    Body::Directory(_) if !last => {
                        cur = child;
                        pos = end + 1;
                    }
                    _ if child_obj.header.parse => match self.call_parse(child, &path[end..], w, hook)? {
                        Ok(found) => return Ok(found),
                        Err(new_path) => {
                            reparses = self.count_reparse(reparses, w)?;
                            path = new_path;
                            start = None;
                            continue 'restart;
                        }
                    },
                    _ if last => {
                        self.add_pointer(child);
                        return Ok(Found::Object(child));
                    }
                    _ => return Err(STATUS_OBJECT_TYPE_MISMATCH),
                }
            }
        }
    }

    fn count_reparse(&self, reparses: u32, w: &Walk<'_>) -> Result<u32> {
        if w.dont_reparse {
            return Err(STATUS_REPARSE_POINT_ENCOUNTERED);
        }
        if reparses >= MAX_REPARSE {
            return Err(STATUS_REPARSE_POINT_NOT_RESOLVED);
        }
        Ok(reparses + 1)
    }

    /// `Ok(Ok(found))`, or `Ok(Err(path))` for a reparse.
    fn call_parse<H: ParseHook + ?Sized>(
        &mut self,
        object: ObjectId,
        remaining: &[u16],
        w: &Walk<'_>,
        hook: &mut H,
    ) -> Result<core::result::Result<Found, Vec<u16>>> {
        let h = &self.obj(object).header;
        let request = ParseRequest {
            object,
            kind: h.kind,
            cookie: h.cookie,
            remaining,
            case_insensitive: w.case_insensitive,
            intent: w.intent,
            object_type: w.object_type,
            desired_access: w.desired_access,
            caller: w.caller,
        };
        match hook.parse(self, &request) {
            Parsed::Object(id) if self.is_alive(id) => Ok(Ok(Found::Parsed(id))),
            Parsed::Object(_) => Err(STATUS_OBJECT_NAME_NOT_FOUND),
            Parsed::Reparse(p) => Ok(Err(p)),
            Parsed::Fail(s) => Err(s),
        }
    }

    /// Find an object by fully qualified path, following symbolic links,
    /// without a handle or a reference (for the kernel's own lookups).
    pub fn lookup(&mut self, path: &[u16], case_insensitive: bool) -> Result<ObjectId> {
        let w = Walk {
            start: None,
            path,
            case_insensitive,
            follow_final_link: true,
            dont_reparse: false,
            intent: Intent::Open,
            object_type: None,
            desired_access: 0,
            caller: None,
        };
        match self.walk(&w, &mut NoParse)? {
            Found::Object(id) | Found::Parsed(id) => {
                self.release_pointer(id);
                Ok(id)
            }
            Found::Missing { parent, .. } => {
                self.release_pointer(parent);
                Err(STATUS_OBJECT_NAME_NOT_FOUND)
            }
        }
    }

    /// A directory's entries as (name, type name), sorted by upcased name
    /// (`NtQueryDirectoryObject`; Windows' order is to be confirmed by the
    /// spec agent).
    pub fn directory_entries(&self, dir: ObjectId) -> Result<Vec<(Vec<u16>, &'static str)>> {
        let Body::Directory(d) = &self.get(dir).ok_or(STATUS_INVALID_HANDLE)?.body else {
            return Err(STATUS_OBJECT_TYPE_MISMATCH);
        };
        Ok(d.entries.values().flatten().map(|(n, id)| (n.clone(), self.obj(*id).header.kind.name())).collect())
    }

    /// A symbolic link's target.
    pub fn link_target(&self, link: ObjectId) -> Result<Vec<u16>> {
        match &self.get(link).ok_or(STATUS_INVALID_HANDLE)?.body {
            Body::SymbolicLink { target } => Ok(target.clone()),
            _ => Err(STATUS_OBJECT_TYPE_MISMATCH),
        }
    }
}
