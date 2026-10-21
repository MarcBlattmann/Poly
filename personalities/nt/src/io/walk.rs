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

use super::names::{self, component_to_vfs, eq_fold, vfs_to_wide};
use super::{VfsRoot, status_of};
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
pub fn find_child(vr: &VfsRoot, dir: &NodeRef, component: &[u16]) -> Result<Option<Vec<u8>>, NtStatus> {
    let exact = component_to_vfs(component)?;
    match vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), &exact, false) {
        Ok(_) => return Ok(Some(exact)),
        Err(Errno::ENOENT) => {}
        Err(e) => return Err(status_of(e)),
    }
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
