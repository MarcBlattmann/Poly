//! Case-insensitive, case-preserving lookup over the case-sensitive VFS.
//!
//! An NT path below a volume is walked one component at a time from the
//! volume's root (or from the directory a relative open starts at). Each
//! component is looked up in its directory:
//!
//! 1. the name exactly as given (UTF-8), which is one VFS lookup;
//! 2. otherwise every entry of the directory whose name equals the
//!    component without case ([`super::names::eq_fold`]).
//!
//! **Names that differ only in case.** The VFS can hold `Readme` and
//! `README` side by side (a Linux program made them); NT cannot tell them
//! apart. Poly decides: an exact match wins; otherwise the entry whose name
//! is smallest as bytes (`README` before `Readme`) is the one a lookup
//! finds, so the choice is stable whatever order the directory lists in.
//! Directory enumeration lists both, each with its own case. Creating a
//! name that matches an existing one without case finds that one (a
//! `FILE_CREATE` is `STATUS_OBJECT_NAME_COLLISION`), as on Windows.
//!
//! Intermediate components follow symbolic links (with the VFS's own rules:
//! an absolute target starts at the VFS root, the host-visible `Z:`); the
//! last one does unless the open asks for `FILE_OPEN_REPARSE_POINT`. Below
//! a symbolic link's target, names are case-sensitive again (the VFS
//! resolves the target). A missing directory on the way is
//! `STATUS_OBJECT_PATH_NOT_FOUND`, a file on the way too (to be confirmed by
//! the spec agent: Windows may say `STATUS_OBJECT_PATH_NOT_FOUND` or
//! `STATUS_NOT_A_DIRECTORY`).

use super::names::{self, component_to_vfs, eq_fold, fold, vfs_to_wide};
use super::{NodeKey, VfsRoot, status_of};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::STATUS_OBJECT_PATH_NOT_FOUND;
use poly_vfs::{At, Errno, FileType, NodeRef, OpenFlags};

/// Where a walk ended.
#[derive(Debug)]
pub struct Resolved {
    /// The directory the last component is in; `None` when the path named
    /// the starting directory itself.
    pub parent: Option<NodeRef>,
    /// The last component's name on disk (as found, or as it would be
    /// created). Empty when `parent` is `None`.
    pub name: Vec<u8>,
    /// The path below the volume root, `\`-separated, with each existing
    /// component in its on-disk case.
    pub path: Vec<u16>,
    /// The node the path names, if it exists.
    pub node: Option<NodeRef>,
}

/// The name in `dir` that `component` finds, without case, if any.
///
/// The exact name is one VFS lookup. Without it, the directory's entries
/// are compared without case — from a [`FoldCache`] index where the
/// directory has one, so that making thousands of files in one directory
/// (each a lookup that finds nothing first) is not quadratic (ADR 0091).
pub fn find_child(vr: &VfsRoot, dir: &NodeRef, component: &[u16]) -> Result<Option<Vec<u8>>, NtStatus> {
    let exact = component_to_vfs(component)?;
    if exists(vr, dir, &exact)? {
        return Ok(Some(exact));
    }
    if !FoldCache::may_index(dir) {
        return scan(vr, dir, component);
    }
    let key = NodeKey::of(dir)?;
    let mut cache = vr.fold.borrow_mut();
    let found = cache.spellings(vr, dir, key, component)?.into_iter().next();
    if found.is_none() {
        cache.pending = Some((dir.clone(), key, exact));
    }
    Ok(found)
}

/// Every name in `dir` equal to `component` without case, smallest (as
/// bytes) first: what a directory query for a pattern without wildcards
/// lists, without reading the whole directory where it has an index.
pub fn find_all(vr: &VfsRoot, dir: &NodeRef, component: &[u16]) -> Result<Vec<Vec<u8>>, NtStatus> {
    if !FoldCache::may_index(dir) {
        let mut all: Vec<Vec<u8>> = list(vr, dir)?
            .into_iter()
            .filter(|n| n != b"." && n != b".." && eq_fold(&vfs_to_wide(n), component))
            .collect();
        all.sort();
        return Ok(all);
    }
    let key = NodeKey::of(dir)?;
    vr.fold.borrow_mut().spellings(vr, dir, key, component)
}

/// Whether `dir` has an entry spelled exactly `name`.
fn exists(vr: &VfsRoot, dir: &NodeRef, name: &[u8]) -> Result<bool, NtStatus> {
    match vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), name, false) {
        Ok(_) => Ok(true),
        Err(Errno::ENOENT) => Ok(false),
        Err(e) => Err(status_of(e)),
    }
}

/// The smallest name (as bytes) in `dir` equal to `component` without case,
/// by reading the whole directory.
fn scan(vr: &VfsRoot, dir: &NodeRef, component: &[u16]) -> Result<Option<Vec<u8>>, NtStatus> {
    let mut best: Option<Vec<u8>> = None;
    for name in list(vr, dir)? {
        if name == b"." || name == b".." {
            continue;
        }
        if eq_fold(&vfs_to_wide(&name), component) && best.as_ref().is_none_or(|b| name < *b) {
            best = Some(name);
        }
    }
    Ok(best)
}

/// The names of one directory by their upcased form, each with every
/// spelling the directory has (smallest first).
type DirIndex = BTreeMap<Vec<u16>, BTreeSet<Vec<u8>>>;

/// How many directories a [`FoldCache`] keeps.
const FOLD_CACHE_DIRS: usize = 8;

/// Case-folded indexes of the directories case-insensitive lookups missed
/// in last (ADR 0091).
///
/// They are right while the VFS's [`namespace_generation`] has not moved
/// since they were read. One step more is accepted when it is the name
/// the last lookup missed now being there (a file made where nothing of
/// that name was: the usual next step after such a lookup), which is added
/// to its directory's index; any other change drops every index. A name
/// the index has is checked to exist before it is used, so a rename seen
/// only as its new name cannot find the old one.
///
/// [`namespace_generation`]: poly_vfs::Vfs::namespace_generation
#[derive(Debug, Default)]
pub struct FoldCache {
    /// The generation every index is right at.
    generation: u64,
    /// Most recently used last.
    dirs: Vec<(NodeKey, DirIndex)>,
    /// The last lookup that found nothing: the directory and the name.
    pending: Option<(NodeRef, NodeKey, Vec<u8>)>,
}

impl FoldCache {
    /// Only directories whose names change through the VFS alone are
    /// indexed: the file systems that let the VFS cache lookups, and tmpfs
    /// (which does not, lookups being as cheap there). A procfs or devpts
    /// makes and removes entries by itself.
    fn may_index(dir: &NodeRef) -> bool {
        let info = dir.mount().fs().info();
        info.cache_lookups || info.name == "tmpfs"
    }

    /// Bring the indexes to the VFS's generation (see the type's docs).
    fn sync(&mut self, vr: &VfsRoot) {
        let now = vr.vfs.namespace_generation();
        if now == self.generation {
            return;
        }
        let pending = self.pending.take();
        if now == self.generation + 1
            && let Some((dir, key, name)) = pending
            && exists(vr, &dir, &name).unwrap_or(false)
        {
            if let Some((_, index)) = self.dirs.iter_mut().find(|(k, _)| *k == key) {
                index.entry(fold(&vfs_to_wide(&name))).or_default().insert(name);
            }
            self.generation = now;
            return;
        }
        self.dirs.clear();
        self.generation = now;
    }

    /// The names `component` finds in `dir` (key `key`), smallest first,
    /// from its index, read now if it has none.
    fn lookup(
        &mut self,
        vr: &VfsRoot,
        dir: &NodeRef,
        key: NodeKey,
        component: &[u16],
    ) -> Result<Vec<Vec<u8>>, NtStatus> {
        let at = match self.dirs.iter().position(|(k, _)| *k == key) {
            Some(i) => {
                let entry = self.dirs.remove(i);
                self.dirs.push(entry);
                self.dirs.len() - 1
            }
            None => {
                let mut index = DirIndex::new();
                for name in list(vr, dir)? {
                    if name != b"." && name != b".." {
                        index.entry(fold(&vfs_to_wide(&name))).or_default().insert(name);
                    }
                }
                if self.dirs.len() == FOLD_CACHE_DIRS {
                    self.dirs.remove(0);
                }
                self.dirs.push((key, index));
                self.dirs.len() - 1
            }
        };
        Ok(self.dirs[at].1.get(&fold(component)).map(|names| names.iter().cloned().collect()).unwrap_or_default())
    }

    /// The spellings of `component` in `dir` (key `key`), smallest first,
    /// each checked to exist: the index may hold a name that is gone (a
    /// rename it saw only as its new name), and then it is read again.
    fn spellings(
        &mut self,
        vr: &VfsRoot,
        dir: &NodeRef,
        key: NodeKey,
        component: &[u16],
    ) -> Result<Vec<Vec<u8>>, NtStatus> {
        self.sync(vr);
        let names = self.lookup(vr, dir, key, component)?;
        let mut all = true;
        for n in &names {
            all &= exists(vr, dir, n)?;
        }
        if all {
            return Ok(names);
        }
        self.forget(key);
        self.lookup(vr, dir, key, component)
    }

    /// Drop `key`'s index.
    fn forget(&mut self, key: NodeKey) {
        self.dirs.retain(|(k, _)| *k != key);
    }
}

/// Every name in directory `dir`, in the order the file system lists them
/// (`.` and `..` included when it has them).
pub fn list(vr: &VfsRoot, dir: &NodeRef) -> Result<Vec<Vec<u8>>, NtStatus> {
    let file = vr
        .vfs
        .open(&vr.ctx, &vr.cred, At::Node(dir), b".", OpenFlags::RDONLY | OpenFlags::DIRECTORY, 0)
        .map_err(status_of)?;
    let mut out = Vec::new();
    file.read_dir(&mut |e| {
        out.push(e.name.to_vec());
        true
    })
    .map_err(status_of)?;
    Ok(out)
}

/// Walk `components` from `start` (whose own path below the volume root is
/// `start_path`).
pub fn resolve(
    vr: &VfsRoot,
    start: &NodeRef,
    start_path: &[u16],
    components: &[&[u16]],
    follow_last: bool,
) -> Result<Resolved, NtStatus> {
    let mut path: Vec<u16> = start_path.to_vec();
    let Some((last, middle)) = components.split_last() else {
        return Ok(Resolved { parent: None, name: Vec::new(), path, node: Some(start.clone()) });
    };
    let mut dir = start.clone();
    for comp in middle {
        let name = find_child(vr, &dir, comp)?.ok_or(STATUS_OBJECT_PATH_NOT_FOUND)?;
        let next = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&dir), &name, true).map_err(|e| match e {
            Errno::ENOENT | Errno::ENOTDIR => STATUS_OBJECT_PATH_NOT_FOUND,
            e => status_of(e),
        })?;
        if next.kind().map_err(status_of)? != FileType::Directory {
            return Err(STATUS_OBJECT_PATH_NOT_FOUND);
        }
        push_component(&mut path, &vfs_to_wide(&name));
        dir = next;
    }
    let found = find_child(vr, &dir, last)?;
    let (name, node) = match found {
        Some(name) => {
            let node = match vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&dir), &name, follow_last) {
                Ok(n) => Some(n),
                // A dangling symbolic link, followed: the name exists but
                // there is nothing to open.
                Err(Errno::ENOENT) => None,
                Err(e) => return Err(status_of(e)),
            };
            push_component(&mut path, &vfs_to_wide(&name));
            (name, node)
        }
        None => {
            push_component(&mut path, last);
            (component_to_vfs(last)?, None)
        }
    };
    Ok(Resolved { parent: Some(dir), name, path, node })
}

/// `path` + `\` + `component`.
pub fn push_component(path: &mut Vec<u16>, component: &[u16]) {
    path.push(b'\\' as u16);
    path.extend_from_slice(component);
}

/// For messages: the path as a string.
pub fn show(path: &[u16]) -> alloc::string::String {
    names::display(path)
}
