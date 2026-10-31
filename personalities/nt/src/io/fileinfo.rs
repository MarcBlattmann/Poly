//! `NtQueryInformationFile`, `NtSetInformationFile` and
//! `NtQueryVolumeInformationFile` on a file object, below the ABI.
//!
//! The access each class needs (Learn: "ZwQueryInformationFile",
//! "ZwSetInformationFile"): `FileBasicInformation`, `FileAllInformation`,
//! `FileNetworkOpenInformation` and `FileAttributeTagInformation` need
//! `FILE_READ_ATTRIBUTES` to query; setting basic information needs
//! `FILE_WRITE_ATTRIBUTES`, the end of file and allocation `FILE_WRITE_DATA`,
//! disposition and rename `DELETE`; the rest need nothing.

use super::consts::*;
use super::info::{self, HandleFacts, Stored};
use super::names::{self, split_path};
use super::{IoState, NodeKey, Target, status_of, walk};
use alloc::vec::Vec;
use core::result::Result;
use poly_ntobj::NtStatus;
use poly_ntobj::name::wide;
use poly_ntobj::status::*;
use poly_vfs::{At, FileType, NodeRef, PathArg, RenameFlags, SetTime};

/// What a query class returns: the bytes and the size of their fixed part
/// (a shorter buffer is `STATUS_INFO_LENGTH_MISMATCH`; a buffer that holds
/// the fixed part but not the name gets `STATUS_BUFFER_OVERFLOW`).
#[derive(Debug)]
pub struct InfoBytes {
    pub bytes: Vec<u8>,
    pub fixed: usize,
}

/// Where a rename goes, once the service has resolved its handles.
#[derive(Debug)]
pub enum RenameTo {
    /// A name or relative path in the file's own directory
    /// (`RootDirectory` NULL and no leading `\`).
    SameDirectory(Vec<u16>),
    /// A path relative to a directory file object (`RootDirectory`).
    Relative { dir: u64, path: Vec<u16> },
    /// A path below a volume, from a fully qualified name.
    Volume { volume: usize, path: Vec<u16> },
}

/// Where a new name for a file goes ([`IoState::rename`],
/// [`IoState::link`]).
struct NewName {
    /// The file's node.
    key: NodeKey,
    src_parent: NodeRef,
    src_name: Vec<u8>,
    dst_parent: NodeRef,
    /// The new last component, as the VFS spells it, and the new path
    /// below the volume in the caller's case.
    new_bytes: Vec<u8>,
    new_path: Vec<u16>,
    /// What already has that name (and its name as found).
    existing: Option<(NodeRef, Vec<u8>)>,
}

impl IoState {
    /// The access a query of `class` needs, or `None` for a class we do not
    /// answer.
    pub fn query_access(class: u32) -> Option<u32> {
        Some(match class {
            FILE_BASIC_INFORMATION
            | FILE_ALL_INFORMATION
            | FILE_NETWORK_OPEN_INFORMATION
            | FILE_ATTRIBUTE_TAG_INFORMATION => FILE_READ_ATTRIBUTES,
            FILE_STANDARD_INFORMATION
            | FILE_INTERNAL_INFORMATION
            | FILE_EA_INFORMATION
            | FILE_ACCESS_INFORMATION
            | FILE_NAME_INFORMATION
            | FILE_POSITION_INFORMATION
            | FILE_MODE_INFORMATION
            | FILE_ALIGNMENT_INFORMATION
            | FILE_STREAM_INFORMATION => 0,
            _ => return None,
        })
    }

    /// The access a set of `class` needs and the size of its structure.
    pub fn set_access(class: u32) -> Option<(u32, usize)> {
        Some(match class {
            FILE_BASIC_INFORMATION => (FILE_WRITE_ATTRIBUTES, BASIC_INFO_SIZE),
            FILE_POSITION_INFORMATION => (0, 8),
            FILE_END_OF_FILE_INFORMATION | FILE_ALLOCATION_INFORMATION => (FILE_WRITE_DATA, 8),
            FILE_DISPOSITION_INFORMATION => (DELETE, 1),
            FILE_DISPOSITION_INFORMATION_EX => (DELETE, 4),
            FILE_RENAME_INFORMATION => (DELETE, RENAME_INFO_FIXED),
            FILE_LINK_INFORMATION => (0, RENAME_INFO_FIXED),
            _ => return None,
        })
    }

    /// The path `FileNameInformation` reports: below the volume root, `\`
    /// for the root; `\name` for a pipe.
    pub fn file_name(&self, cookie: u64) -> Vec<u16> {
        match self.files.get(&cookie).map(|f| &f.target) {
            Some(Target::Disk(d)) if d.path.is_empty() => wide("\\"),
            Some(Target::Disk(d)) => d.path.clone(),
            Some(Target::Pipe(name)) => {
                let mut p = wide("\\");
                p.extend_from_slice(name);
                p
            }
            _ => Vec::new(),
        }
    }

    /// Answer `NtQueryInformationFile(class)` for file object `cookie`
    /// whose handle was granted `granted`.
    pub fn query_info(&self, cookie: u64, class: u32, granted: u32) -> Result<InfoBytes, NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let facts = match &rec.target {
            Target::Disk(d) => self.facts(&d.node)?,
            Target::Pipe(key) => info::Facts {
                size: self.pipe_buffered(key).unwrap_or(0) as u64,
                links: 1,
                attributes: FILE_ATTRIBUTE_NORMAL,
                ..Default::default()
            },
            // A console or an inherited file: nothing on a volume (ADR 0080).
            Target::Console(_) | Target::Host(_) => {
                info::Facts { links: 1, attributes: FILE_ATTRIBUTE_NORMAL, ..Default::default() }
            }
        };
        let handle = HandleFacts { access: granted, position: rec.position, mode: info::mode_bits(rec.options) };
        let fixed = |bytes: Vec<u8>| {
            let n = bytes.len();
            InfoBytes { bytes, fixed: n }
        };
        Ok(match class {
            FILE_BASIC_INFORMATION => fixed(info::basic(&facts).to_vec()),
            FILE_STANDARD_INFORMATION => fixed(info::standard(&facts).to_vec()),
            FILE_INTERNAL_INFORMATION => fixed(facts.id.to_le_bytes().to_vec()),
            FILE_EA_INFORMATION | FILE_ALIGNMENT_INFORMATION => fixed(0u32.to_le_bytes().to_vec()),
            FILE_ACCESS_INFORMATION => fixed(granted.to_le_bytes().to_vec()),
            FILE_POSITION_INFORMATION => fixed(rec.position.to_le_bytes().to_vec()),
            FILE_MODE_INFORMATION => fixed(handle.mode.to_le_bytes().to_vec()),
            FILE_NETWORK_OPEN_INFORMATION => fixed(info::network_open(&facts).to_vec()),
            FILE_ATTRIBUTE_TAG_INFORMATION => fixed(info::attribute_tag(&facts).to_vec()),
            FILE_STREAM_INFORMATION => {
                let on_disk = matches!(rec.target, Target::Disk(_));
                InfoBytes { bytes: info::streams(&facts, on_disk), fixed: STREAM_INFO_FIXED }
            }
            FILE_NAME_INFORMATION => InfoBytes { bytes: info::name(&self.file_name(cookie)), fixed: NAME_INFO_FIXED },
            FILE_ALL_INFORMATION => {
                InfoBytes { bytes: info::all(&facts, &handle, &self.file_name(cookie)), fixed: ALL_INFO_FIXED }
            }
            _ => return Err(STATUS_INVALID_INFO_CLASS),
        })
    }

    fn disk_node(&self, cookie: u64) -> Result<(NodeRef, NodeKey), NtStatus> {
        let d = self.files.get(&cookie).and_then(|f| f.disk()).ok_or(STATUS_INVALID_PARAMETER)?;
        Ok((d.node.clone(), d.key))
    }

    /// `FileBasicInformation`: a time of 0 leaves it alone, -1 too (it
    /// would stop the system updating that time for this handle, which Poly
    /// does not model; to be confirmed by the spec agent); attributes 0
    /// leave them alone. `ChangeTime` cannot be set on the VFS and is
    /// ignored.
    pub fn set_basic(&mut self, cookie: u64, b: &info::SetBasic) -> Result<(), NtStatus> {
        let (node, _) = self.disk_node(cookie)?;
        let vr = self.vfs()?;
        if b.attributes & !FILE_ATTRIBUTE_VALID_FLAGS != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let set = |t: i64| (t != 0 && t != -1).then(|| SetTime::At(info::vfs_time(t)));
        let (atime, mtime) = (set(b.access), set(b.write));
        if atime.is_some() || mtime.is_some() {
            node.set_times(&vr.cred, atime, mtime).map_err(status_of)?;
        }
        let mut stored = self.stored(&node);
        let mut changed = false;
        if b.creation != 0 && b.creation != -1 {
            stored.creation = Some(b.creation);
            changed = true;
        }
        if b.attributes != 0 {
            let attr = node.attr().map_err(status_of)?;
            if attr.kind != FileType::Directory {
                let readonly = b.attributes & FILE_ATTRIBUTE_READONLY != 0;
                let perm = info::perm_for_readonly(attr.perm, readonly);
                if perm != attr.perm {
                    node.chmod(&vr.cred, perm).map_err(status_of)?;
                }
            }
            stored.attributes = b.attributes & info::STORED_ATTRIBUTES;
            changed = true;
        }
        if changed {
            self.set_stored(&node, stored);
        }
        Ok(())
    }

    /// `FileEndOfFileInformation` (`FileAllocationInformation` only ever
    /// shrinks: allocation is the file system's business).
    pub fn set_end_of_file(&mut self, cookie: u64, size: u64, allocation: bool) -> Result<(), NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        if rec.kind == FileType::Directory {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let d = rec.disk().ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        let open = d.open.clone().ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        let now = open.stat().map_err(status_of)?.size;
        if allocation && size >= now {
            return Ok(());
        }
        open.truncate(size).map_err(status_of)
    }

    /// `FileDispositionInformation`: mark the file to be deleted when its
    /// last file object closes (or unmark it). A read-only file is
    /// `STATUS_CANNOT_DELETE` (unless `ignore_readonly`), a directory that
    /// is not empty `STATUS_DIRECTORY_NOT_EMPTY`, a volume's root
    /// `STATUS_CANNOT_DELETE` (Learn: "FILE_DISPOSITION_INFORMATION").
    pub fn set_disposition(&mut self, cookie: u64, delete: bool, ignore_readonly: bool) -> Result<(), NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let d = rec.disk().ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        let (node, key) = (d.node.clone(), d.key);
        if delete {
            if d.parent.is_none() {
                return Err(STATUS_CANNOT_DELETE);
            }
            let attr = node.attr().map_err(status_of)?;
            if attr.kind == FileType::Directory {
                let vr = self.vfs()?;
                let entries = walk::list(vr, &node)?;
                if entries.iter().any(|n| n != b"." && n != b"..") {
                    return Err(STATUS_DIRECTORY_NOT_EMPTY);
                }
            } else if attr.perm & 0o222 == 0 && !ignore_readonly {
                return Err(STATUS_CANNOT_DELETE);
            }
        }
        self.node_mut(key).delete_pending = delete;
        Ok(())
    }

    /// `FileRenameInformation`. The target must be on the same volume
    /// (`STATUS_NOT_SAME_DEVICE`). An existing target is
    /// `STATUS_OBJECT_NAME_COLLISION` without `ReplaceIfExists`, and
    /// `STATUS_ACCESS_DENIED` when it is a directory, read-only or open
    /// (Learn: "FILE_RENAME_INFORMATION"). Renaming a file to its own name
    /// in another case changes the case.
    pub fn rename(&mut self, cookie: u64, to: RenameTo, replace: bool) -> Result<(), NtStatus> {
        let n = self.new_name(cookie, to)?;
        let NewName { key, src_parent, src_name, dst_parent, new_bytes, new_path, existing } = n;
        if let Some((target, name)) = &existing {
            let tkey = NodeKey::of(target)?;
            if tkey != key {
                self.make_room(target, name, &dst_parent, replace)?;
            } else if *name == new_bytes && src_name == new_bytes {
                // Its own name, same case: nothing to do.
                return Ok(());
            }
        }
        let vr = self.vfs()?;
        let flags = RenameFlags::default();
        vr.vfs
            .rename(&vr.ctx, &vr.cred, At::Node(&src_parent), &src_name, At::Node(&dst_parent), &new_bytes, flags)
            .map_err(status_of)?;
        let rec = self.files.get_mut(&cookie).expect("checked");
        let d = rec.disk_mut().expect("disk");
        d.parent = Some(dst_parent.clone());
        d.name = new_bytes.clone();
        d.path = new_path;
        if let Some(n) = self.nodes.get_mut(&key) {
            n.link = Some((dst_parent, new_bytes));
        }
        Ok(())
    }

    /// `FSCTL_GET_REPARSE_POINT` on file object `cookie`: a symbolic link's
    /// `REPARSE_DATA_BUFFER` ([`info::symlink_reparse`]); anything else on
    /// a volume is `STATUS_NOT_A_REPARSE_POINT` (Learn:
    /// "FSCTL_GET_REPARSE_POINT"), anything not on one
    /// `STATUS_INVALID_DEVICE_REQUEST`.
    pub fn reparse_point(&self, cookie: u64) -> Result<Vec<u8>, NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let d = rec.disk().ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        if d.node.kind().map_err(status_of)? != FileType::Symlink {
            return Err(STATUS_NOT_A_REPARSE_POINT);
        }
        Ok(info::symlink_reparse(&d.node.readlink().map_err(status_of)?))
    }

    /// A device or file-system control code `code` on file object `cookie`
    /// (a file or directory on a volume) with `outlen` bytes for the answer:
    /// the status and the bytes to return. `FSCTL_GET_REPARSE_POINT` is the
    /// one Poly's volumes answer: a buffer smaller than the header is
    /// `STATUS_BUFFER_TOO_SMALL`, one that holds part of the answer gets
    /// that part and `STATUS_BUFFER_OVERFLOW`. Every other code is
    /// `STATUS_INVALID_DEVICE_REQUEST` (`ERROR_INVALID_FUNCTION`).
    pub fn fs_control(
        &mut self,
        cookie: u64,
        code: u32,
        input: &[u8],
        outlen: usize,
    ) -> Result<(NtStatus, Vec<u8>), NtStatus> {
        match code {
            FSCTL_SET_REPARSE_POINT => self.set_reparse_point(cookie, input).map(|()| (STATUS_SUCCESS, Vec::new())),
            FSCTL_GET_REPARSE_POINT => {
                let mut data = self.reparse_point(cookie)?;
                if outlen < REPARSE_HEADER_SIZE {
                    return Err(STATUS_BUFFER_TOO_SMALL);
                }
                if data.len() > outlen {
                    data.truncate(outlen);
                    return Ok((STATUS_BUFFER_OVERFLOW, data));
                }
                Ok((STATUS_SUCCESS, data))
            }
            _ => Err(STATUS_INVALID_DEVICE_REQUEST),
        }
    }

    /// `FSCTL_SET_REPARSE_POINT` with a symbolic link's
    /// `REPARSE_DATA_BUFFER` ([`info::parse_symlink_reparse`]) on an empty
    /// file or directory: its name becomes a VFS symbolic link to the
    /// target (what Poly's volumes keep of a reparse point). A file with
    /// data, another tag, or a target on no volume is `STATUS_NOT_SUPPORTED`;
    /// a directory with entries `STATUS_DIRECTORY_NOT_EMPTY`. The file object
    /// stays on what it was opened on, which no longer has that name (a
    /// program closes it next; to be confirmed what else Windows allows).
    pub fn set_reparse_point(&mut self, cookie: u64, input: &[u8]) -> Result<(), NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let d = rec.disk().ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        let (node, parent, name) = (d.node.clone(), d.parent.clone().ok_or(STATUS_ACCESS_DENIED)?, d.name.clone());
        let link = info::parse_symlink_reparse(input)?;
        let target = match link.volume {
            None => link.path,
            Some(letter) => {
                let v = self
                    .volumes
                    .iter()
                    .find(|v| v.letter.is_some_and(|l| l.eq_ignore_ascii_case(&letter)))
                    .ok_or(STATUS_NOT_SUPPORTED)?;
                let mut t = v.path.clone();
                if t.len() > 1 && t.last() == Some(&b'/') {
                    t.pop();
                }
                if t == b"/" {
                    t.clear();
                }
                t.extend_from_slice(&link.path);
                t
            }
        };
        let attr = node.attr().map_err(status_of)?;
        let vr = self.vfs()?;
        match attr.kind {
            FileType::Directory => {
                if walk::list(vr, &node)?.iter().any(|n| n != b"." && n != b"..") {
                    return Err(STATUS_DIRECTORY_NOT_EMPTY);
                }
                vr.vfs.rmdir(&vr.ctx, &vr.cred, At::Node(&parent), &name).map_err(status_of)?;
            }
            FileType::Regular if attr.size == 0 => {
                vr.vfs.unlink(&vr.ctx, &vr.cred, At::Node(&parent), &name).map_err(status_of)?;
            }
            _ => return Err(STATUS_NOT_SUPPORTED),
        }
        vr.vfs.symlink(&vr.ctx, &vr.cred, &target, At::Node(&parent), &name).map_err(status_of)
    }

    /// Whether file object `cookie` is a file or directory on a volume
    /// (not a console, pipe or inherited file).
    pub fn is_disk(&self, cookie: u64) -> bool {
        self.files.get(&cookie).is_some_and(|f| f.disk().is_some())
    }

    /// `FileLinkInformation` (`FILE_LINK_INFORMATION` has
    /// `FILE_RENAME_INFORMATION`'s layout, `ntifs.h`): another name for the
    /// file, on the same volume (`STATUS_NOT_SAME_DEVICE`). A directory
    /// cannot get one (`STATUS_FILE_IS_A_DIRECTORY`); an existing name is
    /// `STATUS_OBJECT_NAME_COLLISION` without `ReplaceIfExists`, and is
    /// replaced as by a rename otherwise (Learn: "FILE_LINK_INFORMATION";
    /// what Windows does when the name already is one of the file's own is
    /// to be confirmed: Poly leaves it, or reports the collision).
    pub fn link(&mut self, cookie: u64, to: RenameTo, replace: bool) -> Result<(), NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let node = rec.disk().ok_or(STATUS_INVALID_DEVICE_REQUEST)?.node.clone();
        if node.attr().map_err(status_of)?.kind == FileType::Directory {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        let NewName { key, dst_parent, new_bytes, existing, .. } = self.new_name(cookie, to)?;
        if let Some((target, name)) = &existing {
            if NodeKey::of(target)? == key {
                return if replace { Ok(()) } else { Err(STATUS_OBJECT_NAME_COLLISION) };
            }
            self.make_room(target, name, &dst_parent, replace)?;
        }
        let vr = self.vfs()?;
        vr.vfs.link_node(&vr.ctx, &vr.cred, &node, PathArg::new(At::Node(&dst_parent), &new_bytes)).map_err(status_of)
    }

    /// Take the name `name` in `dir` that a rename or a new link wants, held
    /// by `target`: `STATUS_OBJECT_NAME_COLLISION` without `replace`,
    /// `STATUS_ACCESS_DENIED` when `target` is a directory, read-only or
    /// open (Learn: "FILE_RENAME_INFORMATION").
    fn make_room(&mut self, target: &NodeRef, name: &[u8], dir: &NodeRef, replace: bool) -> Result<(), NtStatus> {
        if !replace {
            return Err(STATUS_OBJECT_NAME_COLLISION);
        }
        let tkey = NodeKey::of(target)?;
        let tattr = target.attr().map_err(status_of)?;
        let open = self.nodes.get(&tkey).is_some_and(|n| n.opens > 0);
        if tattr.kind == FileType::Directory || tattr.perm & 0o222 == 0 || open {
            return Err(STATUS_ACCESS_DENIED);
        }
        let vr = self.vfs()?;
        vr.vfs.unlink(&vr.ctx, &vr.cred, At::Node(dir), name).map_err(status_of)
    }

    /// Resolve where a new name (`to`) for file object `cookie`'s file goes.
    fn new_name(&self, cookie: u64, to: RenameTo) -> Result<NewName, NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let d = rec.disk().ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        let (volume, key) = (d.volume, d.key);
        let src_parent = d.parent.clone().ok_or(STATUS_ACCESS_DENIED)?;
        let src_name = d.name.clone();
        let parent_path: Vec<u16> = match d.path.iter().rposition(|&c| c == b'\\' as u16) {
            Some(i) => d.path[..i].to_vec(),
            None => Vec::new(),
        };
        let (start, start_path, path) = match to {
            RenameTo::SameDirectory(p) => (src_parent.clone(), parent_path, p),
            RenameTo::Relative { dir, path } => {
                let dr = self.files.get(&dir).and_then(|f| f.disk()).ok_or(STATUS_INVALID_HANDLE)?;
                if dr.volume != volume {
                    return Err(STATUS_NOT_SAME_DEVICE);
                }
                (dr.node.clone(), dr.path.clone(), path)
            }
            RenameTo::Volume { volume: v, path } => {
                if v != volume {
                    return Err(STATUS_NOT_SAME_DEVICE);
                }
                (self.volumes[v].root.clone(), Vec::new(), path)
            }
        };
        let (parts, trailing) = split_path(&path)?;
        let last = *parts.last().ok_or(STATUS_OBJECT_NAME_INVALID)?;
        if trailing {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        let new_bytes = names::component_to_vfs(last)?;
        let vr = self.vfs()?;
        let found = walk::resolve(vr, &start, &start_path, &parts, false)?;
        let dst_parent = found.parent.clone().ok_or(STATUS_OBJECT_NAME_INVALID)?;
        let mut new_path = found.path.clone();
        // The new name in the caller's case.
        if let Some(i) = new_path.iter().rposition(|&c| c == b'\\' as u16) {
            new_path.truncate(i);
        }
        walk::push_component(&mut new_path, last);
        let existing = found.node.clone().map(|n| (n, found.name.clone()));
        Ok(NewName { key, src_parent, src_name, dst_parent, new_bytes, new_path, existing })
    }

    /// `NtQueryVolumeInformationFile(class)`.
    pub fn query_volume(&self, cookie: u64, class: u32) -> Result<InfoBytes, NtStatus> {
        let rec = self.files.get(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        let fixed = |bytes: Vec<u8>, fixed: usize| InfoBytes { bytes, fixed };
        let disk = match &rec.target {
            Target::Disk(d) => Some(d),
            _ => None,
        };
        if class == FILE_FS_DEVICE_INFORMATION {
            // A console is FILE_DEVICE_CONSOLE (GetFileType: FILE_TYPE_CHAR);
            // an inherited file is what its node is (ADR 0080).
            let kind = match &rec.target {
                Target::Disk(_) => FILE_DEVICE_DISK,
                Target::Pipe(_) => FILE_DEVICE_NAMED_PIPE,
                Target::Console(_) => poly_console::ioctl::FILE_DEVICE_CONSOLE,
                Target::Host(open) => match open.kind() {
                    poly_vfs::FileType::Regular | poly_vfs::FileType::Directory => FILE_DEVICE_DISK,
                    poly_vfs::FileType::Fifo | poly_vfs::FileType::Socket => FILE_DEVICE_NAMED_PIPE,
                    _ => FILE_DEVICE_UNKNOWN,
                },
            };
            let mut b = kind.to_le_bytes().to_vec();
            b.extend_from_slice(&0x20u32.to_le_bytes()); // FILE_DEVICE_IS_MOUNTED
            return Ok(fixed(b, 8));
        }
        let d = disk.ok_or(STATUS_INVALID_DEVICE_REQUEST)?;
        let vol = &self.volumes[d.volume];
        let stats = d.node.statfs().map_err(status_of)?;
        let unit = stats.block_size.max(512);
        let sectors = (unit / 512) as u32;
        let to_units = |blocks: u64| blocks.saturating_mul(stats.block_size.max(1)) / unit;
        Ok(match class {
            FILE_FS_VOLUME_INFORMATION => {
                // VolumeCreationTime 0, VolumeSerialNumber 8, VolumeLabelLength
                // 12, SupportsObjects 16, VolumeLabel 18 (24 bytes).
                let mut b = alloc::vec![0u8; 18];
                b[8..12].copy_from_slice(&vol.serial.to_le_bytes());
                b[12..16].copy_from_slice(&((2 * vol.label.len()) as u32).to_le_bytes());
                for c in &vol.label {
                    b.extend_from_slice(&c.to_le_bytes());
                }
                b.resize(b.len().max(24), 0);
                fixed(b, 24)
            }
            FILE_FS_SIZE_INFORMATION => {
                // TotalAllocationUnits, AvailableAllocationUnits,
                // SectorsPerAllocationUnit, BytesPerSector (24 bytes).
                let mut b = Vec::with_capacity(24);
                b.extend_from_slice(&to_units(stats.blocks).to_le_bytes());
                b.extend_from_slice(&to_units(stats.blocks_avail).to_le_bytes());
                b.extend_from_slice(&sectors.to_le_bytes());
                b.extend_from_slice(&512u32.to_le_bytes());
                fixed(b, 24)
            }
            FILE_FS_FULL_SIZE_INFORMATION => {
                // Total, CallerAvailable, ActualAvailable, Sectors..., Bytes...
                let mut b = Vec::with_capacity(32);
                b.extend_from_slice(&to_units(stats.blocks).to_le_bytes());
                b.extend_from_slice(&to_units(stats.blocks_avail).to_le_bytes());
                b.extend_from_slice(&to_units(stats.blocks_free).to_le_bytes());
                b.extend_from_slice(&sectors.to_le_bytes());
                b.extend_from_slice(&512u32.to_le_bytes());
                fixed(b, 32)
            }
            FILE_FS_ATTRIBUTE_INFORMATION => {
                // FileSystemAttributes, MaximumComponentNameLength,
                // FileSystemNameLength, FileSystemName (16 bytes fixed).
                let mut attrs = FILE_CASE_PRESERVED_NAMES
                    | FILE_UNICODE_ON_DISK
                    | FILE_SUPPORTS_REPARSE_POINTS
                    | FILE_SUPPORTS_HARD_LINKS;
                if d.node.mount().read_only() {
                    attrs |= FILE_READ_ONLY_VOLUME;
                }
                let name = wide(FS_NAME);
                let mut b = Vec::with_capacity(12 + 2 * name.len());
                b.extend_from_slice(&attrs.to_le_bytes());
                b.extend_from_slice(&(names::MAX_COMPONENT as u32).to_le_bytes());
                b.extend_from_slice(&((2 * name.len()) as u32).to_le_bytes());
                for c in &name {
                    b.extend_from_slice(&c.to_le_bytes());
                }
                b.resize(b.len().max(16), 0);
                fixed(b, 16)
            }
            _ => return Err(STATUS_INVALID_INFO_CLASS),
        })
    }

    /// Store new stored attributes (tests).
    pub fn stored_of(&self, cookie: u64) -> Option<Stored> {
        let d = self.files.get(&cookie)?.disk()?;
        Some(self.stored(&d.node))
    }
}

/// The file system name our volumes report (`FileFsAttributeInformation`).
/// Poly's own: applications that look for `NTFS` features should probe
/// them, as Learn advises; to be revisited when an app needs otherwise.
pub const FS_NAME: &str = "PolyFS";
