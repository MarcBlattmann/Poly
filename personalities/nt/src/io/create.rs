//! `NtCreateFile`'s work below a device: the parse procedure that walks a
//! volume, applies the create disposition, the create options, the share
//! access and the attributes, and makes the file object.
//!
//! # Dispositions (Learn: "NtCreateFile", `CreateDisposition`)
//!
//! | Disposition | Exists | Does not exist |
//! |---|---|---|
//! | `FILE_SUPERSEDE` | replace it (`FILE_SUPERSEDED`) | create (`FILE_CREATED`) |
//! | `FILE_OPEN` | open (`FILE_OPENED`) | `STATUS_OBJECT_NAME_NOT_FOUND` |
//! | `FILE_CREATE` | `STATUS_OBJECT_NAME_COLLISION` | create |
//! | `FILE_OPEN_IF` | open | create |
//! | `FILE_OVERWRITE` | truncate (`FILE_OVERWRITTEN`) | `STATUS_OBJECT_NAME_NOT_FOUND` |
//! | `FILE_OVERWRITE_IF` | truncate | create |
//!
//! Superseding truncates the file and replaces its stored attributes rather
//! than making a new file (the VFS node stays; to be confirmed by the spec
//! agent whether Windows' file id changes). Overwriting or superseding a
//! file that is `HIDDEN` or `SYSTEM` without asking for that attribute is
//! `STATUS_ACCESS_DENIED` (Learn: "CreateFile", `CREATE_ALWAYS`), as is
//! overwriting a read-only file; a directory cannot be overwritten
//! (`STATUS_FILE_IS_A_DIRECTORY`, to be confirmed by the spec agent).
//!
//! # Order of the checks
//!
//! The name is walked first (`STATUS_OBJECT_PATH_NOT_FOUND`,
//! `STATUS_OBJECT_NAME_INVALID`), then the type options
//! (`STATUS_NOT_A_DIRECTORY`, `STATUS_FILE_IS_A_DIRECTORY`), the
//! disposition, `STATUS_DELETE_PENDING`, the read-only attribute, and the
//! share access (`STATUS_SHARING_VIOLATION`) — before anything changes on
//! disk, so a refused open never truncates. The order between the last
//! three is to be confirmed by the spec agent.

use super::consts::*;
use super::info::{self, Stored};
use super::names::{self, split_path};
use super::share::ShareRequest;
use super::walk::{self, Resolved};
use super::{Device, DiskFile, FileRecord, IoState, NodeKey, Target, VfsRoot, status_of};
use alloc::vec::Vec;
use core::result::Result;
use poly_ntobj::status::*;
use poly_ntobj::{
    NewObject, NtStatus, ObjectId, ObjectManager, ObjectType, ParseHook, ParseRequest, Parsed, ThreadRef,
};
use poly_vfs::{At, Errno, FileType, NodeRef, OpenFlags};

/// What an `NtCreateFile` asks for, after the service checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateParams {
    /// `DesiredAccess` with generic rights mapped (`MAXIMUM_ALLOWED` as
    /// `FILE_ALL_ACCESS`).
    pub access: u32,
    pub share: u32,
    pub disposition: u32,
    pub options: u32,
    /// `FileAttributes` for a new or replaced file.
    pub attributes: u32,
    /// `AllocationSize` (only a hint: ignored).
    pub allocation: u64,
}

/// The parse procedure of volume devices, the pipe device and directory
/// file objects, for one create.
#[derive(Debug)]
pub struct CreateHook<'a> {
    pub io: &'a mut IoState,
    pub params: CreateParams,
    pub thread: ThreadRef,
    /// `IO_STATUS_BLOCK.Information` once the file object is made.
    pub information: u64,
    /// The new file object's cookie.
    pub made: Option<u64>,
}

impl ParseHook for CreateHook<'_> {
    fn parse(&mut self, om: &mut ObjectManager, r: &ParseRequest<'_>) -> Parsed {
        match self.run(om, r) {
            Ok(id) => Parsed::Object(id),
            Err(s) => Parsed::Fail(s),
        }
    }
}

/// Where a walk starts.
enum Start {
    Volume(usize),
    Dir { volume: usize, node: NodeRef, path: Vec<u16> },
    Pipe,
}

impl CreateHook<'_> {
    fn run(&mut self, om: &mut ObjectManager, r: &ParseRequest<'_>) -> Result<ObjectId, NtStatus> {
        let start = match r.kind {
            ObjectType::Device => match self.io.device(r.cookie) {
                Some(Device::Volume(v)) => Start::Volume(v),
                Some(Device::Pipe) => Start::Pipe,
                Some(Device::Console) => return self.open_console(om, r, None),
                None => return Err(STATUS_OBJECT_PATH_NOT_FOUND),
            },
            ObjectType::File => {
                let rec = self.io.file(r.cookie).ok_or(STATUS_INVALID_HANDLE)?;
                if let super::Target::Console(t) = rec.target {
                    return self.open_console(om, r, Some(t));
                }
                let d = rec.disk().ok_or(STATUS_INVALID_PARAMETER)?;
                if rec.kind != FileType::Directory {
                    return Err(STATUS_NOT_A_DIRECTORY);
                }
                Start::Dir { volume: d.volume, node: d.node.clone(), path: d.path.clone() }
            }
            _ => return Err(STATUS_OBJECT_TYPE_MISMATCH),
        };
        match start {
            Start::Pipe => self.open_pipe(om, r.remaining),
            Start::Volume(v) => {
                let root = self.io.volumes[v].root.clone();
                self.open_disk(om, v, &root, &[], r.remaining)
            }
            Start::Dir { volume, node, path } => self.open_disk(om, volume, &node, &path, r.remaining),
        }
    }

    /// Below `\Device\PolyConsole`, or relative to a console handle (ADR
    /// 0080, [`crate::console`]).
    fn open_console(
        &mut self,
        om: &mut ObjectManager,
        r: &ParseRequest<'_>,
        relative: Option<crate::console::ConsoleTarget>,
    ) -> Result<ObjectId, NtStatus> {
        if self.params.options & FILE_DIRECTORY_FILE != 0 {
            return Err(STATUS_NOT_A_DIRECTORY);
        }
        let (object, cookie) =
            self.io.open_console(om, r.caller.as_ref(), self.thread, &self.params, relative, r.remaining)?;
        self.information = FILE_OPENED;
        self.made = Some(cookie);
        Ok(object)
    }

    fn open_pipe(&mut self, om: &mut ObjectManager, remaining: &[u16]) -> Result<ObjectId, NtStatus> {
        let p = &self.params;
        if p.options & FILE_DIRECTORY_FILE != 0 {
            return Err(STATUS_NOT_A_DIRECTORY);
        }
        let (parts, trailing) = split_path(remaining)?;
        if parts.len() != 1 || trailing {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        names::check_component(parts[0])?;
        let key = names::fold(parts[0]);
        let exists = self.io.pipes.contains_key(&key);
        match (p.disposition, exists) {
            (FILE_CREATE, true) => return Err(STATUS_OBJECT_NAME_COLLISION),
            (FILE_OPEN | FILE_OVERWRITE, false) => return Err(STATUS_OBJECT_NAME_NOT_FOUND),
            _ => {}
        }
        self.information = if exists { FILE_OPENED } else { FILE_CREATED };
        let cookie = self.io.cookie();
        let object = new_file_object(om, cookie, self.thread)?;
        let pipe = self.io.pipes.entry(key.clone()).or_default();
        pipe.opens += 1;
        if p.access & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0 {
            pipe.writers += 1;
            pipe.had_writer = true;
        }
        self.io.files.insert(cookie, self.record(object, Target::Pipe(key), FileType::Fifo));
        self.made = Some(cookie);
        Ok(object)
    }

    fn record(&self, object: ObjectId, target: Target, kind: FileType) -> FileRecord {
        let p = &self.params;
        FileRecord {
            object,
            target,
            kind,
            access: p.access,
            share: ShareRequest::new(p.access, p.share),
            options: p.options,
            position: 0,
            delete_on_close: p.options & FILE_DELETE_ON_CLOSE != 0,
            scan: None,
        }
    }

    fn open_disk(
        &mut self,
        om: &mut ObjectManager,
        volume: usize,
        start: &NodeRef,
        start_path: &[u16],
        remaining: &[u16],
    ) -> Result<ObjectId, NtStatus> {
        let p = self.params;
        let (parts, trailing) = split_path(remaining)?;
        // The whole name is checked before anything is looked up.
        for part in &parts {
            names::check_component(part)?;
        }
        let follow = p.options & FILE_OPEN_REPARSE_POINT == 0;
        let vr = self.io.vfs()?;
        let found = walk::resolve(vr, start, start_path, &parts, follow)?;
        let (info, node, open) = match &found.node {
            Some(node) => self.open_existing(&found, node, trailing)?,
            None => self.create_new(&found, trailing)?,
        };
        let kind = node.kind().map_err(status_of)?;
        let key = NodeKey::of(&node)?;
        let cookie = self.io.cookie();
        let object = new_file_object(om, cookie, self.thread)?;
        let share = ShareRequest::new(p.access, p.share);
        let state = self.io.nodes.entry(key).or_default();
        state.share.add(&share);
        state.opens += 1;
        if let Some(parent) = &found.parent {
            state.link = Some((parent.clone(), found.name.clone()));
        }
        let disk = DiskFile { volume, node, key, open, path: found.path, parent: found.parent, name: found.name };
        let rec = self.record(object, Target::Disk(disk), kind);
        self.io.files.insert(cookie, rec);
        self.information = info;
        self.made = Some(cookie);
        Ok(object)
    }

    /// An existing file: every check, then (if asked) the truncation, then
    /// the VFS open.
    fn open_existing(
        &mut self,
        found: &Resolved,
        node: &NodeRef,
        trailing: bool,
    ) -> Result<(u64, NodeRef, Option<alloc::sync::Arc<poly_vfs::OpenFile>>), NtStatus> {
        let p = self.params;
        let vr = self.io.vfs()?;
        let attr = node.attr().map_err(status_of)?;
        let is_dir = attr.kind == FileType::Directory;
        if p.options & FILE_DIRECTORY_FILE != 0 && !is_dir {
            return Err(STATUS_NOT_A_DIRECTORY);
        }
        if p.options & FILE_NON_DIRECTORY_FILE != 0 && is_dir {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        if trailing && !is_dir {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        let replace = matches!(p.disposition, FILE_SUPERSEDE | FILE_OVERWRITE | FILE_OVERWRITE_IF);
        if p.disposition == FILE_CREATE {
            return Err(STATUS_OBJECT_NAME_COLLISION);
        }
        if replace && is_dir {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        let key = NodeKey::of(node)?;
        if self.io.nodes.get(&key).is_some_and(|n| n.delete_pending) {
            return Err(STATUS_DELETE_PENDING);
        }
        let stored = self.io.stored(node);
        let readonly = !is_dir && attr.perm & 0o222 == 0;
        if readonly && (p.access & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0 || replace) {
            return Err(STATUS_ACCESS_DENIED);
        }
        if readonly && p.options & FILE_DELETE_ON_CLOSE != 0 {
            return Err(STATUS_CANNOT_DELETE);
        }
        if replace {
            let keep = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM;
            if stored.attributes & keep & !p.attributes != 0 {
                return Err(STATUS_ACCESS_DENIED);
            }
        }
        let share = ShareRequest::new(p.access, p.share);
        if let Some(n) = self.io.nodes.get(&key)
            && !n.share.check(&share)
        {
            return Err(STATUS_SHARING_VIOLATION);
        }
        let info = match p.disposition {
            FILE_SUPERSEDE => FILE_SUPERSEDED,
            FILE_OVERWRITE | FILE_OVERWRITE_IF => FILE_OVERWRITTEN,
            _ => FILE_OPENED,
        };
        if replace {
            node.truncate(&vr.cred, 0).map_err(status_of)?;
            let attrs = names::create_attributes(p.attributes) | FILE_ATTRIBUTE_ARCHIVE;
            self.io.set_stored(node, Stored { attributes: attrs, creation: stored.creation });
        }
        let open = open_vfs(vr, found, node, attr.kind, p.access, p.options & FILE_OPEN_REPARSE_POINT == 0)?;
        Ok((info, node.clone(), open))
    }

    fn create_new(
        &mut self,
        found: &Resolved,
        trailing: bool,
    ) -> Result<(u64, NodeRef, Option<alloc::sync::Arc<poly_vfs::OpenFile>>), NtStatus> {
        let p = self.params;
        if matches!(p.disposition, FILE_OPEN | FILE_OVERWRITE) {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        }
        let dir_wanted = p.options & FILE_DIRECTORY_FILE != 0;
        if trailing && !dir_wanted {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        let parent = found.parent.as_ref().ok_or(STATUS_OBJECT_NAME_INVALID)?;
        let vr = self.io.vfs()?;
        let readonly = p.attributes & FILE_ATTRIBUTE_READONLY != 0;
        let at = At::Node(parent);
        let (node, open) = if dir_wanted {
            vr.vfs.mkdir(&vr.ctx, &vr.cred, at, &found.name, 0o777).map_err(created_status)?;
            let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(parent), &found.name, false).map_err(status_of)?;
            let open = vr
                .vfs
                .open(&vr.ctx, &vr.cred, At::Node(&node), b".", OpenFlags::RDONLY | OpenFlags::DIRECTORY, 0)
                .map_err(status_of)?;
            (node, open)
        } else {
            let mode = if readonly { 0o444 } else { 0o666 };
            let flags = OpenFlags::CREAT | OpenFlags::EXCL | OpenFlags::RDWR | OpenFlags::NOFOLLOW;
            let open = vr.vfs.open(&vr.ctx, &vr.cred, at, &found.name, flags, mode).map_err(created_status)?;
            (open.node().clone(), open)
        };
        let creation = node.attr().ok().map(|a| info::nt_time(a.ctime));
        let mut attrs = names::create_attributes(p.attributes) & !FILE_ATTRIBUTE_READONLY;
        if !dir_wanted {
            attrs |= FILE_ATTRIBUTE_ARCHIVE;
        }
        self.io.set_stored(&node, Stored { attributes: attrs, creation });
        Ok((FILE_CREATED, node, Some(open)))
    }
}

/// `EEXIST` after our own lookup said "missing" means someone made it
/// meanwhile: a collision.
fn created_status(e: Errno) -> NtStatus {
    status_of(e)
}

/// The VFS open behind a file object on an existing node.
fn open_vfs(
    vr: &VfsRoot,
    found: &Resolved,
    node: &NodeRef,
    kind: FileType,
    access: u32,
    follow: bool,
) -> Result<Option<alloc::sync::Arc<poly_vfs::OpenFile>>, NtStatus> {
    let open = |at: At<'_>, name: &[u8], flags: OpenFlags| vr.vfs.open(&vr.ctx, &vr.cred, at, name, flags, 0);
    let r = match kind {
        FileType::Directory => open(At::Node(node), b".", OpenFlags::RDONLY | OpenFlags::DIRECTORY),
        FileType::Symlink => {
            // Only reached with FILE_OPEN_REPARSE_POINT: the link itself.
            let parent = found.parent.as_ref().ok_or(STATUS_INVALID_PARAMETER)?;
            open(At::Node(parent), &found.name, OpenFlags::PATH | OpenFlags::NOFOLLOW)
        }
        FileType::Regular | FileType::CharDevice => {
            let read = access & (FILE_READ_DATA | FILE_EXECUTE) != 0;
            let write = access & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0;
            let flags = match (read, write) {
                (_, true) if read => OpenFlags::RDWR,
                (_, true) => OpenFlags::WRONLY,
                (true, false) => OpenFlags::RDONLY,
                (false, false) => OpenFlags::PATH,
            };
            let flags = if follow { flags } else { flags | OpenFlags::NOFOLLOW };
            match &found.parent {
                Some(parent) => open(At::Node(parent), &found.name, flags | OpenFlags::NOCTTY),
                None => return Err(STATUS_INVALID_PARAMETER),
            }
        }
        // FIFOs, sockets and block devices are not NT files.
        _ => return Err(STATUS_ACCESS_DENIED),
    };
    r.map(Some).map_err(status_of)
}

/// A file object, waitable (signalled when I/O on it completes), and a
/// parse object (opens relative to it continue in its directory).
fn new_file_object(om: &mut ObjectManager, cookie: u64, thread: ThreadRef) -> Result<ObjectId, NtStatus> {
    om.new_object(NewObject::Opaque { kind: ObjectType::File, cookie, signaled: Some(false), parse: true }, thread)
}
