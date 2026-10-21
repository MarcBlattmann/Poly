//! Making the layout on the VFS, idempotently (ADR 0072).
//!
//! [`create`] walks every folder of [`super::FOLDERS`] from the directory
//! behind `C:` one component at a time, the way an NT open does (ADR 0067,
//! [`crate::io::walk::find_child`]): an existing entry that matches without
//! case is used, whatever its case on disk, so a `windows` directory a
//! Linux program made is the Windows directory and no `Windows` is made
//! beside it. Where two entries differ only in case, the one NT opens (the
//! exact name, else the smallest as bytes) is the one used. A missing
//! component is made (mode 0755) in the table's case; a folder marked
//! hidden gets `FILE_ATTRIBUTE_HIDDEN` when it is made, never later, so a
//! user who unhides it keeps that. Something that is not a directory where
//! a folder should be is left alone and reported ([`Report::conflicts`]),
//! with everything below it: Poly never deletes a user's file to make its
//! layout. Running it again changes nothing.
//!
//! [`install_file`] puts one of Poly's own files (a system DLL) into a
//! layout folder, rewriting it only when its bytes differ.

use super::{FOLDERS, User};
use crate::io::consts::{FILE_ATTRIBUTE_HIDDEN, STATUS_NOT_A_DIRECTORY};
use crate::io::info::{NT_XATTR, Stored};
use crate::io::names::vfs_to_wide;
use crate::io::walk::find_child;
use crate::io::{VfsRoot, status_of};
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::wide;
use poly_ntobj::status::STATUS_OBJECT_NAME_COLLISION;
use poly_vfs::{At, FileType, NodeRef, OpenFlags};

/// What [`create`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Directories it made, as DOS paths, in the order made.
    pub created: Vec<String>,
    /// Folders it could not make: the DOS path and why (a file in the way
    /// is `STATUS_NOT_A_DIRECTORY`).
    pub conflicts: Vec<(String, NtStatus)>,
}

/// The directory `name` (one component, as NT names it) in `dir`: found
/// without case, or made. `Ok((node, made))`.
pub fn ensure_dir(vr: &VfsRoot, dir: &NodeRef, name: &str) -> Result<(NodeRef, bool), NtStatus> {
    let component = wide(name);
    let made = match find_child(vr, dir, &component)? {
        Some(existing) => {
            let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), &existing, true).map_err(status_of)?;
            if node.kind().map_err(status_of)? != FileType::Directory {
                return Err(STATUS_NOT_A_DIRECTORY);
            }
            return Ok((node, false));
        }
        None => name.as_bytes(),
    };
    vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Node(dir), made, 0o755).map_err(status_of)?;
    let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), made, true).map_err(status_of)?;
    Ok((node, true))
}

/// Walk `path` (`\`-separated, below `root`), making what is missing. The
/// returned flag says whether the last component was made.
pub fn ensure_path(vr: &VfsRoot, root: &NodeRef, path: &str) -> Result<(NodeRef, bool), NtStatus> {
    let mut cur = root.clone();
    let mut made = false;
    for part in path.split('\\').filter(|p| !p.is_empty()) {
        let (next, new) = ensure_dir(vr, &cur, part)?;
        cur = next;
        made = new;
    }
    Ok((cur, made))
}

/// Make every folder of the layout for `user` below `root` (the directory
/// behind `C:`). See the module documentation.
pub fn create(vr: &VfsRoot, root: &NodeRef, user: &User) -> Report {
    let mut report = Report::default();
    for folder in FOLDERS {
        let below = folder.below_drive(user);
        // Make each component, so every directory made is reported.
        let mut cur = root.clone();
        let mut walked = String::new();
        let mut failed = None;
        for part in below.split('\\').filter(|p| !p.is_empty()) {
            if !walked.is_empty() {
                walked.push('\\');
            }
            walked.push_str(part);
            match ensure_dir(vr, &cur, part) {
                Ok((node, made)) => {
                    if made {
                        report.created.push(alloc::format!("{}\\{walked}", super::SYSTEM_DRIVE));
                        if folder.hidden && walked == below {
                            set_hidden(vr, &node);
                        }
                    }
                    cur = node;
                }
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failed {
            let path = folder.dos_path(user);
            // A conflict on a parent is reported once, for the first folder.
            if !report.conflicts.iter().any(|(p, _)| path.starts_with(p.as_str())) {
                report.conflicts.push((path, e));
            }
        }
    }
    report
}

fn set_hidden(vr: &VfsRoot, node: &NodeRef) {
    let stored = Stored { attributes: FILE_ATTRIBUTE_HIDDEN, creation: None };
    // Best effort: a file system without extended attributes keeps none
    // (ADR 0067).
    let _ = node.setxattr(&vr.cred, NT_XATTR, &stored.encode(), 0);
}

/// What [`install_file`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The file was missing or different and has been written.
    Written,
    /// It was already there with these bytes.
    Unchanged,
}

/// Put `bytes` as file `name` into the folder `dir` (`\`-separated, below
/// `root`, made if missing), unless a file of that name (without case)
/// already holds exactly these bytes. Poly's own system files are thereby
/// restored at every boot if something changed them. A directory of that
/// name is `STATUS_OBJECT_NAME_COLLISION` and left alone.
pub fn install_file(vr: &VfsRoot, root: &NodeRef, dir: &str, name: &str, bytes: &[u8]) -> Result<Installed, NtStatus> {
    let (folder, _) = ensure_path(vr, root, dir)?;
    let existing = find_child(vr, &folder, &wide(name))?;
    let on_disk: Vec<u8> = existing.clone().unwrap_or_else(|| name.as_bytes().to_vec());
    if existing.is_some() {
        let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&folder), &on_disk, true).map_err(status_of)?;
        match node.kind().map_err(status_of)? {
            FileType::Regular => {
                if same_contents(vr, &folder, &on_disk, bytes)? {
                    return Ok(Installed::Unchanged);
                }
            }
            _ => return Err(STATUS_OBJECT_NAME_COLLISION),
        }
    }
    let flags = OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::TRUNC;
    let file = vr.vfs.open(&vr.ctx, &vr.cred, At::Node(&folder), &on_disk, flags, 0o644).map_err(status_of)?;
    let mut done = 0;
    while done < bytes.len() {
        match file.write_at(done as u64, &bytes[done..]).map_err(status_of)? {
            0 => return Err(status_of(poly_vfs::Errno::EIO)),
            n => done += n,
        }
    }
    Ok(Installed::Written)
}

fn same_contents(vr: &VfsRoot, dir: &NodeRef, name: &[u8], want: &[u8]) -> Result<bool, NtStatus> {
    let file = vr.vfs.open(&vr.ctx, &vr.cred, At::Node(dir), name, OpenFlags::RDONLY, 0).map_err(status_of)?;
    let mut buf = alloc::vec![0u8; 16 * 1024];
    let mut at = 0usize;
    loop {
        let n = file.read_at(at as u64, &mut buf).map_err(status_of)?;
        if n == 0 {
            return Ok(at == want.len());
        }
        if at + n > want.len() || buf[..n] != want[at..at + n] {
            return Ok(false);
        }
        at += n;
    }
}

/// The name of every entry of `dir` as NT shows it (for tests and logs).
pub fn names(vr: &VfsRoot, dir: &NodeRef) -> Result<Vec<String>, NtStatus> {
    let list = crate::io::walk::list(vr, dir)?;
    Ok(list
        .into_iter()
        .filter(|n| n != b"." && n != b"..")
        .map(|n| String::from_utf16_lossy(&vfs_to_wide(&n)))
        .collect())
}
