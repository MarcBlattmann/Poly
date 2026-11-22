//! Making the layout on the VFS, idempotently (ADR 0073).
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
//! [`install_own_copy`] does the same for a file the user may replace with
//! someone else's (ADR 0199: Microsoft's Visual C++ runtime installed by
//! `vc_redist`): a file there that is not Poly's (it lacks
//! [`OWN_COPY_MARKER`]) is left as it is.

use super::{Base, FOLDERS, User};
use crate::io::consts::{FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, STATUS_NOT_A_DIRECTORY};
use crate::io::info::{NT_XATTR, Stored};
use crate::io::names::vfs_to_wide;
use crate::io::walk::find_child;
use crate::io::{NodeKey, VfsRoot, status_of};
use alloc::string::String;
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::name::wide;
use poly_ntobj::status::STATUS_OBJECT_NAME_COLLISION;
use poly_vfs::{At, Errno, FileType, NodeRef, OpenFlags};

/// How [`unify_home`] unified the POSIX home directory (`/home/<user>`) with the
/// Windows user profile (`C:\Users\<user>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeUnified {
    /// Both paths already name the same directory (same device and inode).
    AlreadyUnified,
    /// The POSIX home path was created (or replaced from an empty directory) as a symbolic
    /// link pointing to the Windows profile directory on `C:`.
    LinkedPosixToProfile,
    /// The existing POSIX home directory was bind-mounted onto the Windows profile directory
    /// so both paths expose the same directory tree without disturbing existing files.
    BoundProfileToPosix,
    /// Both paths already exist as separate non-empty directories, or a non-directory blocks
    /// one of them; neither was modified.
    Conflict,
}

fn is_empty_dir(vr: &VfsRoot, dir: &NodeRef) -> Result<bool, NtStatus> {
    Ok(crate::io::walk::list(vr, dir)?.into_iter().all(|n| n == b"." || n == b".."))
}

fn is_tmpfs(node: &NodeRef) -> bool {
    node.mount().fs().info().name == "tmpfs"
}

fn mkdir_all_posix(vr: &VfsRoot, path: &str) -> Result<(), NtStatus> {
    let mut at = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        at.push('/');
        at.push_str(part);
        match vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Cwd, at.as_bytes(), 0o755) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => return Err(status_of(e)),
        }
    }
    Ok(())
}

/// Unify `posix_home` (e.g. `/home/poly`) and the Windows profile `C:\Users\<user>`
/// below `c_root` (reached at `c_path`, e.g. `/nt/c`) so both worlds share one home
/// directory tree (Phase 8).
///
/// - If both paths already resolve to the same directory inode: [`HomeUnified::AlreadyUnified`].
/// - If `posix_home` is missing, or is an empty directory on tmpfs, or is empty while the
///   Windows profile already holds files: ensures the profile directory exists on `C:`,
///   removes the empty `posix_home` if present, and makes `posix_home` a symbolic link to
///   `<c_path>/<Users>/<user>` ([`HomeUnified::LinkedPosixToProfile`]).
/// - If `posix_home` is an existing directory (non-empty, or empty on a persistent disk) and
///   the Windows profile is missing or empty: creates the profile mountpoint if missing and
///   bind-mounts `posix_home` onto it ([`HomeUnified::BoundProfileToPosix`]).
/// - If both are non-empty directories on different inodes, or something other than a
///   directory blocks either path: leaves both untouched and returns [`HomeUnified::Conflict`].
///
/// Call before [`create`] so that missing profile folders (`Desktop`, `Documents`,
/// `Downloads`, `Music`, `Pictures`, `Videos`, `AppData`, ...) are created inside the
/// unified directory. Idempotent.
pub fn unify_home(
    vr: &VfsRoot,
    c_root: &NodeRef,
    c_path: &str,
    user: &User,
    posix_home: &str,
) -> Result<HomeUnified, NtStatus> {
    let posix_home = posix_home.trim_end_matches('/');
    if !posix_home.starts_with('/') || posix_home.is_empty() {
        return Ok(HomeUnified::Conflict);
    }
    // 1. Ensure `C:\Users` exists (case-insensitively).
    let (users_dir, users_name) = match find_child(vr, c_root, &wide("Users"))? {
        Some(existing) => {
            let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(c_root), &existing, true).map_err(status_of)?;
            if node.kind().map_err(status_of)? != FileType::Directory {
                return Ok(HomeUnified::Conflict);
            }
            (node, existing)
        }
        None => {
            vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Node(c_root), b"Users", 0o755).map_err(status_of)?;
            let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(c_root), b"Users", true).map_err(status_of)?;
            (node, b"Users".to_vec())
        }
    };
    // 2. Inspect `C:\Users\<user>` (case-insensitively).
    let (nt_node, nt_name, nt_empty) = match find_child(vr, &users_dir, &wide(user.name))? {
        Some(existing) => {
            let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&users_dir), &existing, false).map_err(status_of)?;
            if node.kind().map_err(status_of)? != FileType::Directory {
                return Ok(HomeUnified::Conflict);
            }
            let empty = is_empty_dir(vr, &node)?;
            (Some(node), existing, empty)
        }
        None => (None, user.name.as_bytes().to_vec(), true),
    };
    let profile_target = alloc::format!(
        "{}/{}/{}",
        c_path.trim_end_matches('/'),
        String::from_utf8_lossy(&users_name),
        String::from_utf8_lossy(&nt_name)
    );
    let ensure_nt_profile = || -> Result<(), NtStatus> {
        if nt_node.is_none() {
            vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Node(&users_dir), &nt_name, 0o755).map_err(status_of)?;
        }
        Ok(())
    };
    // 3. Inspect `posix_home`.
    match vr.vfs.lookup(&vr.ctx, &vr.cred, At::Cwd, posix_home.as_bytes(), false) {
        Err(Errno::ENOENT) => {
            ensure_nt_profile()?;
            if let Some(cut) = posix_home.rfind('/')
                && cut > 0
            {
                mkdir_all_posix(vr, &posix_home[..cut])?;
            }
            vr.vfs
                .symlink(&vr.ctx, &vr.cred, profile_target.as_bytes(), At::Cwd, posix_home.as_bytes())
                .map_err(status_of)?;
            Ok(HomeUnified::LinkedPosixToProfile)
        }
        Err(e) => Err(status_of(e)),
        Ok(posix_raw) => {
            if let Some(nt) = &nt_node
                && let Ok(posix_followed) = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Cwd, posix_home.as_bytes(), true)
                && NodeKey::of(&posix_followed)? == NodeKey::of(nt)?
            {
                return Ok(HomeUnified::AlreadyUnified);
            }
            match posix_raw.kind().map_err(status_of)? {
                FileType::Symlink => {
                    if posix_raw.readlink().map_err(status_of)? == profile_target.as_bytes() {
                        ensure_nt_profile()?;
                        Ok(HomeUnified::AlreadyUnified)
                    } else {
                        Ok(HomeUnified::Conflict)
                    }
                }
                FileType::Directory => {
                    let posix_empty = is_empty_dir(vr, &posix_raw)?;
                    let prefer_link_to_profile = posix_empty && (!nt_empty || is_tmpfs(&posix_raw));
                    if prefer_link_to_profile {
                        ensure_nt_profile()?;
                        vr.vfs.rmdir(&vr.ctx, &vr.cred, At::Cwd, posix_home.as_bytes()).map_err(status_of)?;
                        vr.vfs
                            .symlink(&vr.ctx, &vr.cred, profile_target.as_bytes(), At::Cwd, posix_home.as_bytes())
                            .map_err(status_of)?;
                        Ok(HomeUnified::LinkedPosixToProfile)
                    } else if nt_empty {
                        ensure_nt_profile()?;
                        vr.vfs
                            .bind(
                                &vr.ctx,
                                &vr.cred,
                                At::Cwd,
                                posix_home.as_bytes(),
                                At::Node(&users_dir),
                                &nt_name,
                                false,
                            )
                            .map_err(status_of)?;
                        Ok(HomeUnified::BoundProfileToPosix)
                    } else {
                        Ok(HomeUnified::Conflict)
                    }
                }
                _ => Ok(HomeUnified::Conflict),
            }
        }
    }
}

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
    // If the user's profile directory is owned by a non-root POSIX user (e.g. uid 1000
    // for `/home/poly` unified with `C:\Users\poly`), newly created folders inside it
    // inherit that owner so Linux apps running as that user can write into `Downloads`,
    // `Desktop`, `Documents`, etc.
    let mut profile_owner: Option<(u32, u32)> = None;
    for folder in FOLDERS {
        let below = folder.below_drive(user);
        let profile_root = folder.base == Base::Profile && folder.path.is_empty();
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
                        if folder.base == Base::Profile
                            && !profile_root
                            && let Some((uid, gid)) = profile_owner
                        {
                            let _ = node.chown(&vr.cred, Some(uid), Some(gid));
                        }
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
        if failed.is_none()
            && profile_root
            && let Ok(attr) = cur.attr()
            && (attr.uid != 0 || attr.gid != 0)
        {
            profile_owner = Some((attr.uid, attr.gid));
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

/// Give an installed system file `FILE_ATTRIBUTE_ARCHIVE`, as Windows' own system files have it
/// (the sheets of spec-win-chrome-setup read `C:\Windows\System32\kernel32.dll` as 0x20, ADR
/// 0227), unless NT attributes are stored already (a program may have changed them).
fn mark_archive(vr: &VfsRoot, folder: &NodeRef, on_disk: &[u8]) {
    let Ok(node) = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(folder), on_disk, true) else { return };
    if node.getxattr(&vr.cred, NT_XATTR).is_ok() {
        return;
    }
    let stored = Stored { attributes: FILE_ATTRIBUTE_ARCHIVE, creation: None };
    // Best effort, as for `set_hidden`.
    let _ = node.setxattr(&vr.cred, NT_XATTR, &stored.encode(), 0);
}

/// What [`install_file`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The file was missing or different and has been written.
    Written,
    /// It was already there with these bytes.
    Unchanged,
    /// Another file of that name, not Poly's, is there and stays
    /// ([`install_own_copy`]).
    Yielded,
}

/// The bytes every Poly DLL that a user's installer may replace carries
/// (ADR 0199): `vcruntime140.dll`, `vcruntime140_1.dll` and `msvcp140.dll`. The DLLs
/// hold the same text (`personalities/nt/win32/{vcruntime140*,msvcp140}/src/lib.rs`)
/// and the kernel's `build.rs` refuses to embed one without it.
pub const OWN_COPY_MARKER: &[u8] = b"Poly's own copy of a Visual C++ runtime DLL (ADR 0199)";

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
                    mark_archive(vr, &folder, &on_disk);
                    return Ok(Installed::Unchanged);
                }
            }
            _ => return Err(STATUS_OBJECT_NAME_COLLISION),
        }
    }
    let flags = OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::TRUNC;
    // A program the kernel itself starts (`services.exe`, ADR 0221) needs the execute bits.
    let mode = if name.ends_with(".exe") { 0o755 } else { 0o644 };
    let file = vr.vfs.open(&vr.ctx, &vr.cred, At::Node(&folder), &on_disk, flags, mode).map_err(status_of)?;
    let mut done = 0;
    while done < bytes.len() {
        match file.write_at(done as u64, &bytes[done..]).map_err(status_of)? {
            0 => return Err(status_of(poly_vfs::Errno::EIO)),
            n => done += n,
        }
    }
    drop(file);
    mark_archive(vr, &folder, &on_disk);
    Ok(Installed::Written)
}

/// [`install_file`] for a DLL the user may replace with another vendor's
/// copy (ADR 0199, owner decision 5: Microsoft's `vc_redist` replaces
/// Poly's Visual C++ runtime): written when missing, rewritten when it is
/// an older Poly copy (it holds `marker`) with other bytes, and left alone
/// ([`Installed::Yielded`]) when the file there is someone else's.
pub fn install_own_copy(
    vr: &VfsRoot,
    root: &NodeRef,
    dir: &str,
    name: &str,
    bytes: &[u8],
    marker: &[u8],
) -> Result<Installed, NtStatus> {
    let (folder, _) = ensure_path(vr, root, dir)?;
    if let Some(on_disk) = find_child(vr, &folder, &wide(name))? {
        let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&folder), &on_disk, true).map_err(status_of)?;
        if node.kind().map_err(status_of)? == FileType::Regular && !contains(vr, &folder, &on_disk, marker)? {
            return Ok(Installed::Yielded);
        }
    }
    install_file(vr, root, dir, name, bytes)
}

/// Whether the file `name` in `dir` holds `needle` anywhere.
fn contains(vr: &VfsRoot, dir: &NodeRef, name: &[u8], needle: &[u8]) -> Result<bool, NtStatus> {
    if needle.is_empty() {
        return Ok(true);
    }
    let file = vr.vfs.open(&vr.ctx, &vr.cred, At::Node(dir), name, OpenFlags::RDONLY, 0).map_err(status_of)?;
    // Each read is appended to the last `needle.len() - 1` bytes of the one
    // before, so a match across two reads is found.
    let keep = needle.len() - 1;
    let mut window: Vec<u8> = Vec::new();
    let mut buf = alloc::vec![0u8; 16 * 1024];
    let mut at = 0u64;
    loop {
        let n = file.read_at(at, &mut buf).map_err(status_of)?;
        if n == 0 {
            return Ok(false);
        }
        at += n as u64;
        window.extend_from_slice(&buf[..n]);
        if window.windows(needle.len()).any(|w| w == needle) {
            return Ok(true);
        }
        let drop = window.len().saturating_sub(keep);
        window.drain(..drop);
    }
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
