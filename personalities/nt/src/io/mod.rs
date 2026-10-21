//! NT file I/O (ADR 0067): file and device objects over the VFS, drive
//! letters, case-insensitive lookup, share modes, byte-range locks, and
//! IRP-style asynchronous I/O.
//!
//! # Objects and names
//!
//! Each volume is a device object `\Device\PolyFs<N>` in the object
//! manager, created with a parse procedure (ADR 0030's [`ParseHook`]): a
//! lookup that reaches it hands the rest of the path to
//! [`create::CreateHook`], which walks the VFS below the volume's root
//! directory ([`walk`], case-insensitive and case-preserving) and makes a
//! *file object*. Drive letters are symbolic links in `\??`:
//!
//! | Name | Target | VFS directory |
//! |---|---|---|
//! | `\??\C:` | `\Device\PolyFs1` | [`SYSTEM_ROOT`] (`/nt/c`), the NT system root |
//! | `\??\Z:` | `\Device\PolyFs0` | `/`, the whole host-visible VFS |
//! | (none) | `\Device\PolyFs2`... | every other VFS mount, by mount point |
//! | (none) | `\Device\PolyPipe` | Poly's loopback byte pipes (below) |
//!
//! So `\??\C:\Windows\win.ini` reaches `\Device\PolyFs1` through the link
//! and becomes `/nt/c/Windows/win.ini` (in whatever case it has on disk).
//! The table of mounts is taken when the NT personality first starts;
//! later mounts appear below `Z:` but get no device of their own (a later
//! milestone can watch the mount table). A file object is itself a parse
//! object, so an open relative to a directory handle (`RootDirectory`)
//! continues from that directory.
//!
//! # State
//!
//! [`IoState`] lives in [`crate::NtSystem`] behind the NT lock: the volumes,
//! each file object's [`FileRecord`] (keyed by the object's cookie), each
//! open node's share access and delete-pending state ([`NodeState`]), the
//! byte-range locks ([`locks`]), the pipes, and the I/O request packets
//! ([`irp`]). A file object's last reference going (`NtSystem::reap`) is its
//! cleanup *and* close: its share access and locks go, delete-on-close
//! happens, and its pending requests are cancelled. Poly does not separate
//! the two the way `IRP_MJ_CLEANUP` and `IRP_MJ_CLOSE` do, so a section
//! mapping a file keeps its share access until the section goes (to be
//! confirmed by the spec agent).
//!
//! # `\Device\PolyPipe`
//!
//! A stand-in for named pipes until they exist: `\Device\PolyPipe\<name>`
//! opens (creating on first open) a byte stream every opener of the name
//! shares. A write appends and completes waiting reads; a read takes what
//! is there or waits, which is what the asynchronous-I/O and cancellation
//! tests need from a "slow device". It is Poly's own device, not a Windows
//! interface.

pub mod consts;
pub mod create;
pub mod dir;
pub mod fileinfo;
pub mod info;
pub mod irp;
pub mod locks;
pub mod names;
pub mod services;
pub mod share;
pub mod walk;

#[cfg(test)]
mod tests;

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use consts::*;
use core::result::Result;
use info::{Facts, NT_XATTR, Stored};
pub use irp::{Completion, Irp, IrpOp, IrpState, Work, WorkOp};
use locks::LockTable;
use poly_ntobj::name::wide;
use poly_ntobj::status::*;
use poly_ntobj::{
    Caller, NewObject, NoParse, NtStatus, OBJ_CASE_INSENSITIVE, OBJ_KERNEL_HANDLE, OBJ_PERMANENT, ObjectAttributes,
    ObjectId, ObjectManager, ObjectType, ProcessId, ThreadRef,
};
use poly_vfs::{At, Cred, Errno, FileType, FsContext, NodeRef, OpenFile, Vfs};
use share::{ShareAccess, ShareRequest};

/// The VFS directory of the NT system root, `C:`.
pub const SYSTEM_ROOT: &[u8] = b"/nt/c";

/// File and device object cookies start here, far from the keys the
/// kernel's minimal file objects of ADR 0063 use.
const COOKIE_BASE: u64 = 1 << 40;

/// The VFS as the NT personality uses it: one context (root at `/`) and one
/// set of credentials (root) for every NT process until NT processes have
/// tokens (single user, Phase 5).
#[derive(Debug)]
pub struct VfsRoot {
    pub vfs: &'static Vfs,
    pub ctx: FsContext,
    pub cred: Cred,
}

/// A file as the share-access, delete and lock tables name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeKey {
    pub dev: u64,
    pub ino: u64,
}

impl NodeKey {
    pub fn of(node: &NodeRef) -> Result<NodeKey, NtStatus> {
        let st = node.stat().map_err(status_of)?;
        Ok(NodeKey { dev: (u64::from(st.dev.major) << 32) | u64::from(st.dev.minor), ino: st.ino })
    }
}

/// A volume to create: its device name, its root directory in the VFS and
/// its drive letter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VolumeConfig {
    pub device: String,
    pub path: Vec<u8>,
    pub letter: Option<char>,
    pub label: String,
}

/// One volume device.
#[derive(Debug)]
pub struct Volume {
    /// `PolyFs<N>`.
    pub device: Vec<u16>,
    pub root: NodeRef,
    pub path: Vec<u8>,
    pub label: Vec<u16>,
    pub letter: Option<char>,
    pub serial: u32,
}

/// What a device object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Volume(usize),
    Pipe,
}

/// A file object's target on a volume.
#[derive(Debug)]
pub struct DiskFile {
    pub volume: usize,
    pub node: NodeRef,
    pub key: NodeKey,
    /// The VFS open file (`None` for nothing but a symbolic link itself).
    pub open: Option<Arc<OpenFile>>,
    /// The path below the volume root (`FileNameInformation`).
    pub path: Vec<u16>,
    /// The directory and name it was opened by (for rename and delete).
    pub parent: Option<NodeRef>,
    pub name: Vec<u8>,
}

/// What a file object is open on.
#[derive(Debug)]
pub enum Target {
    Disk(DiskFile),
    /// A `\Device\PolyPipe` stream, by its upcased name.
    Pipe(Vec<u16>),
}

/// The kernel's data for one file object.
#[derive(Debug)]
pub struct FileRecord {
    pub object: ObjectId,
    pub target: Target,
    pub kind: FileType,
    /// The access the open asked for, generic rights mapped.
    pub access: u32,
    pub share: ShareRequest,
    /// The create options (`FILE_SYNCHRONOUS_IO_*`, `FILE_DELETE_ON_CLOSE`,
    /// ...).
    pub options: u32,
    /// `CurrentByteOffset`, used by synchronous file objects.
    pub position: u64,
    pub delete_on_close: bool,
    /// The directory scan of `NtQueryDirectoryFile`.
    pub scan: Option<dir::Scan>,
}

impl FileRecord {
    /// Opened for synchronous I/O (`FILE_SYNCHRONOUS_IO_ALERT` or
    /// `_NONALERT`): the I/O manager keeps a file position and every call
    /// waits for its request to finish.
    pub fn is_sync(&self) -> bool {
        self.options & (FILE_SYNCHRONOUS_IO_ALERT | FILE_SYNCHRONOUS_IO_NONALERT) != 0
    }

    pub fn disk(&self) -> Option<&DiskFile> {
        match &self.target {
            Target::Disk(d) => Some(d),
            Target::Pipe(_) => None,
        }
    }

    pub fn disk_mut(&mut self) -> Option<&mut DiskFile> {
        match &mut self.target {
            Target::Disk(d) => Some(d),
            Target::Pipe(_) => None,
        }
    }
}

/// Per open file (node): what every file object on it shares.
#[derive(Debug, Default)]
pub struct NodeState {
    pub share: ShareAccess,
    /// File objects open on it.
    pub opens: u32,
    /// `FileDispositionInformation` or a closed `FILE_DELETE_ON_CLOSE`
    /// handle asked for deletion; it happens when the last file object
    /// goes, and new opens fail with `STATUS_DELETE_PENDING`.
    pub delete_pending: bool,
    /// Where it was last opened (directory and name), to delete it.
    pub link: Option<(NodeRef, Vec<u8>)>,
}

/// A `\Device\PolyPipe` stream.
#[derive(Debug, Default)]
pub struct Pipe {
    pub data: VecDeque<u8>,
    pub opens: u32,
    /// Reads waiting for data, oldest first (IRP ids).
    pub readers: VecDeque<u64>,
}

/// The machine-wide file I/O state (in [`crate::NtSystem`]).
pub struct IoState {
    pub(crate) root: Option<VfsRoot>,
    pub volumes: Vec<Volume>,
    pub(crate) devices: BTreeMap<u64, Device>,
    pub(crate) files: BTreeMap<u64, FileRecord>,
    pub(crate) nodes: BTreeMap<NodeKey, NodeState>,
    pub locks: LockTable,
    pub(crate) pipes: BTreeMap<Vec<u16>, Pipe>,
    pub(crate) irps: BTreeMap<u64, Irp>,
    pub(crate) queues: BTreeMap<u32, VecDeque<u64>>,
    pub(crate) kicks: BTreeSet<u32>,
    pub(crate) completions: VecDeque<Completion>,
    pub(crate) sync_done: BTreeMap<u64, (NtStatus, u64)>,
    next_cookie: u64,
    pub(crate) next_irp: u64,
}

impl core::fmt::Debug for IoState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IoState")
            .field("attached", &self.root.is_some())
            .field("volumes", &self.volumes.len())
            .field("files", &self.files.len())
            .field("irps", &self.irps.len())
            .finish()
    }
}

impl Default for IoState {
    fn default() -> Self {
        Self::new()
    }
}

/// Map a VFS error to the `NTSTATUS` a file service returns (the pairs are
/// Poly's: the nearest documented meaning; to be confirmed by the spec
/// agent where Windows has a choice).
pub fn status_of(e: Errno) -> NtStatus {
    match e {
        Errno::ENOENT => STATUS_OBJECT_NAME_NOT_FOUND,
        Errno::ENOTDIR => STATUS_OBJECT_PATH_NOT_FOUND,
        Errno::EEXIST => STATUS_OBJECT_NAME_COLLISION,
        Errno::EACCES | Errno::EPERM => STATUS_ACCESS_DENIED,
        Errno::EISDIR => STATUS_FILE_IS_A_DIRECTORY,
        Errno::ENOTEMPTY => STATUS_DIRECTORY_NOT_EMPTY,
        Errno::ENOSPC | Errno::EFBIG | Errno::EDQUOT => STATUS_DISK_FULL,
        Errno::EROFS => STATUS_MEDIA_WRITE_PROTECTED,
        Errno::ENAMETOOLONG => STATUS_OBJECT_NAME_INVALID,
        Errno::ELOOP => STATUS_REPARSE_POINT_NOT_RESOLVED,
        Errno::EXDEV => STATUS_NOT_SAME_DEVICE,
        Errno::EBUSY => STATUS_SHARING_VIOLATION,
        Errno::EIO => STATUS_IO_DEVICE_ERROR,
        Errno::EMFILE | Errno::ENFILE => STATUS_TOO_MANY_OPENED_FILES,
        Errno::ENOMEM => STATUS_NO_MEMORY,
        Errno::EINVAL => STATUS_INVALID_PARAMETER,
        Errno::EBADF => STATUS_INVALID_HANDLE,
        Errno::ENXIO | Errno::ENODEV | Errno::EOPNOTSUPP => STATUS_INVALID_DEVICE_REQUEST,
        _ => STATUS_UNSUCCESSFUL,
    }
}

/// The volumes Poly creates on `vfs`: `\Device\PolyFs0` on `/` as `Z:`,
/// `\Device\PolyFs1` on [`SYSTEM_ROOT`] as `C:`, then one device per other
/// mount (by mount point, no letter).
pub fn default_volumes(vfs: &Vfs, ctx: &FsContext) -> Vec<VolumeConfig> {
    let mut v = alloc::vec![
        VolumeConfig { device: "PolyFs0".into(), path: b"/".to_vec(), letter: Some('Z'), label: "Poly".into() },
        VolumeConfig {
            device: "PolyFs1".into(),
            path: SYSTEM_ROOT.to_vec(),
            letter: Some('C'),
            label: "System".into()
        },
    ];
    let mut paths: Vec<Vec<u8>> = vfs
        .mounts()
        .iter()
        .filter(|m| m.parent.is_some())
        .filter_map(|m| vfs.mount_path(ctx, m.id).ok())
        .filter(|p| p.as_slice() != b"/")
        .collect();
    paths.sort();
    paths.dedup();
    for (i, path) in paths.into_iter().enumerate() {
        v.push(VolumeConfig { device: alloc::format!("PolyFs{}", i + 2), path, letter: None, label: String::new() });
    }
    v
}

impl IoState {
    pub fn new() -> IoState {
        IoState {
            root: None,
            volumes: Vec::new(),
            devices: BTreeMap::new(),
            files: BTreeMap::new(),
            nodes: BTreeMap::new(),
            locks: LockTable::default(),
            pipes: BTreeMap::new(),
            irps: BTreeMap::new(),
            queues: BTreeMap::new(),
            kicks: BTreeSet::new(),
            completions: VecDeque::new(),
            sync_done: BTreeMap::new(),
            next_cookie: COOKIE_BASE,
            next_irp: 1,
        }
    }

    pub fn is_attached(&self) -> bool {
        self.root.is_some()
    }

    pub(crate) fn cookie(&mut self) -> u64 {
        let c = self.next_cookie;
        self.next_cookie += 1;
        c
    }

    /// Put the file system into the object namespace: a device per volume
    /// (its root directory made if missing), the drive letters in `\??`,
    /// and `\Device\PolyPipe`. Called once, when the NT personality starts.
    pub fn attach(
        &mut self,
        om: &mut ObjectManager,
        vfs: &'static Vfs,
        volumes: &[VolumeConfig],
    ) -> Result<(), NtStatus> {
        if self.root.is_some() {
            return Ok(());
        }
        let ctx = vfs.root_context().map_err(status_of)?;
        let cred = Cred::root();
        let kernel = Caller::kernel(ProcessId(0), ThreadRef(0));
        for (index, config) in volumes.iter().enumerate() {
            make_dirs(vfs, &ctx, &cred, &config.path);
            let Ok(root) = vfs.lookup(&ctx, &cred, At::Cwd, &config.path, true) else { continue };
            let cookie = self.cookie();
            let device = wide(&config.device);
            let mut path = wide("\\Device\\");
            path.extend_from_slice(&device);
            add_named(
                om,
                &kernel,
                &path,
                NewObject::Opaque { kind: ObjectType::Device, cookie, signaled: None, parse: true },
            )?;
            if let Some(letter) = config.letter {
                let link = wide(&alloc::format!("\\??\\{letter}:"));
                add_named(om, &kernel, &link, NewObject::SymbolicLink { target: &path })?;
            }
            self.devices.insert(cookie, Device::Volume(self.volumes.len()));
            self.volumes.push(Volume {
                device,
                root,
                path: config.path.clone(),
                label: wide(&config.label),
                letter: config.letter,
                serial: 0x5059_0000 | index as u32,
            });
        }
        let cookie = self.cookie();
        add_named(
            om,
            &kernel,
            &wide("\\Device\\PolyPipe"),
            NewObject::Opaque { kind: ObjectType::Device, cookie, signaled: None, parse: true },
        )?;
        self.devices.insert(cookie, Device::Pipe);
        self.root = Some(VfsRoot { vfs, ctx, cred });
        Ok(())
    }

    pub fn vfs(&self) -> Result<&VfsRoot, NtStatus> {
        self.root.as_ref().ok_or(STATUS_OBJECT_PATH_NOT_FOUND)
    }

    pub fn device(&self, cookie: u64) -> Option<Device> {
        self.devices.get(&cookie).copied()
    }

    pub fn file(&self, cookie: u64) -> Option<&FileRecord> {
        self.files.get(&cookie)
    }

    pub fn file_mut(&mut self, cookie: u64) -> Option<&mut FileRecord> {
        self.files.get_mut(&cookie)
    }

    /// File objects open.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn node(&self, key: NodeKey) -> Option<&NodeState> {
        self.nodes.get(&key)
    }

    pub fn node_mut(&mut self, key: NodeKey) -> &mut NodeState {
        self.nodes.entry(key).or_default()
    }

    /// The NT facts of a node: times, size, attributes.
    pub fn facts(&self, node: &NodeRef) -> Result<Facts, NtStatus> {
        let attr = node.attr().map_err(status_of)?;
        let stored = self.stored(node);
        let key = NodeKey::of(node)?;
        let directory = attr.kind == FileType::Directory;
        let size = if directory { 0 } else { attr.size };
        Ok(Facts {
            creation: stored.creation.unwrap_or_else(|| info::nt_time(attr.mtime)),
            access: info::nt_time(attr.atime),
            write: info::nt_time(attr.mtime),
            change: info::nt_time(attr.ctime),
            size,
            allocation: if directory { 0 } else { info::allocation(attr.size, attr.blocks) },
            links: attr.nlink,
            attributes: info::attributes_of(attr.kind, attr.perm, stored.attributes),
            directory,
            delete_pending: self.nodes.get(&key).is_some_and(|n| n.delete_pending),
            id: attr.ino,
            reparse_tag: if attr.kind == FileType::Symlink { IO_REPARSE_TAG_SYMLINK } else { 0 },
        })
    }

    /// The attributes stored in [`NT_XATTR`].
    pub fn stored(&self, node: &NodeRef) -> Stored {
        let Some(root) = &self.root else { return Stored::default() };
        node.getxattr(&root.cred, NT_XATTR).map(|v| Stored::parse(&v)).unwrap_or_default()
    }

    /// Store attributes (best effort: a file system without extended
    /// attributes keeps none).
    pub fn set_stored(&self, node: &NodeRef, stored: Stored) {
        let Some(root) = &self.root else { return };
        if matches!(node.kind(), Ok(FileType::Regular | FileType::Directory)) {
            let _ = node.setxattr(&root.cred, NT_XATTR, &stored.encode(), 0);
        }
    }

    /// The last reference to file object `cookie` went: its cleanup and
    /// close. Pending requests are cancelled, its locks and share access
    /// go, `FILE_DELETE_ON_CLOSE` marks the file for deletion, and the file
    /// is deleted if it was the last open of a file marked so.
    pub fn file_closed(&mut self, cookie: u64) {
        let Some(rec) = self.files.get(&cookie) else { return };
        let _ = rec;
        self.cancel_where(|irp| irp.file == cookie, true);
        let rec = self.files.remove(&cookie).expect("checked");
        match rec.target {
            Target::Pipe(key) => {
                if let Some(p) = self.pipes.get_mut(&key) {
                    p.opens -= 1;
                    if p.opens == 0 {
                        self.pipes.remove(&key);
                    }
                }
            }
            Target::Disk(d) => {
                if self.locks.release_file(d.key, cookie) {
                    self.retry_locks(d.key);
                }
                let state = self.nodes.entry(d.key).or_default();
                state.share.remove(&rec.share);
                state.opens = state.opens.saturating_sub(1);
                if rec.delete_on_close {
                    state.delete_pending = true;
                }
                if state.opens == 0 {
                    let state = self.nodes.remove(&d.key).expect("there");
                    if state.delete_pending {
                        self.delete_node(d.key, &d.node, state.link);
                    }
                }
            }
        }
    }

    /// Remove a file whose deletion was pending, by the name it was last
    /// opened by, if that name still names it.
    fn delete_node(&self, key: NodeKey, node: &NodeRef, link: Option<(NodeRef, Vec<u8>)>) {
        let (Some(root), Some((dir, name))) = (&self.root, link) else { return };
        let Ok(now) = root.vfs.lookup(&root.ctx, &root.cred, At::Node(&dir), &name, false) else { return };
        if NodeKey::of(&now) != Ok(key) {
            return;
        }
        let _ = if matches!(node.kind(), Ok(FileType::Directory)) {
            root.vfs.rmdir(&root.ctx, &root.cred, At::Node(&dir), &name)
        } else {
            root.vfs.unlink(&root.ctx, &root.cred, At::Node(&dir), &name)
        };
    }

    /// The volume device and the path below it that a fully qualified NT
    /// path names, following the namespace's symbolic links (for
    /// `FileRenameInformation` with a full path): a walk with a hook that
    /// stops at the device.
    pub fn resolve_device_path(
        &self,
        om: &mut ObjectManager,
        caller: &Caller,
        attrs: &ObjectAttributes<'_>,
    ) -> Result<(usize, Vec<u16>), NtStatus> {
        struct Stop<'a> {
            io: &'a IoState,
            found: Option<(usize, Vec<u16>)>,
        }
        impl poly_ntobj::ParseHook for Stop<'_> {
            fn parse(&mut self, _: &mut ObjectManager, r: &poly_ntobj::ParseRequest<'_>) -> poly_ntobj::Parsed {
                let found = match r.kind {
                    ObjectType::Device => match self.io.device(r.cookie) {
                        Some(Device::Volume(v)) => Some((v, r.remaining.to_vec())),
                        _ => None,
                    },
                    ObjectType::File => self.io.file(r.cookie).and_then(|f| f.disk()).map(|d| {
                        let mut p = d.path.clone();
                        if !r.remaining.is_empty() {
                            walk::push_component(&mut p, r.remaining);
                        }
                        (d.volume, p)
                    }),
                    _ => None,
                };
                match found {
                    Some(f) => {
                        self.found = Some(f);
                        poly_ntobj::Parsed::Fail(STATUS_REPARSE_POINT_ENCOUNTERED)
                    }
                    None => poly_ntobj::Parsed::Fail(STATUS_NOT_SAME_DEVICE),
                }
            }
        }
        let mut hook = Stop { io: self, found: None };
        let r = om.open_object(caller, attrs, 0, Some(ObjectType::File), &mut hook);
        match (hook.found, r) {
            (Some(found), _) => Ok(found),
            (None, Ok(h)) => {
                let _ = om.close(caller, h);
                Err(STATUS_OBJECT_TYPE_MISMATCH)
            }
            (None, Err(e)) => Err(e),
        }
    }
}

/// `mkdir -p` (as root), ignoring what exists.
fn make_dirs(vfs: &Vfs, ctx: &FsContext, cred: &Cred, path: &[u8]) {
    let mut at = Vec::new();
    for part in path.split(|&b| b == b'/').filter(|p| !p.is_empty()) {
        at.push(b'/');
        at.extend_from_slice(part);
        let _ = vfs.mkdir(ctx, cred, At::Cwd, &at, 0o755);
    }
}

/// Create a permanent named object as the kernel and drop the handle.
fn add_named(om: &mut ObjectManager, kernel: &Caller, path: &[u16], new: NewObject<'_>) -> Result<(), NtStatus> {
    let attrs = ObjectAttributes::named(path, OBJ_PERMANENT | OBJ_KERNEL_HANDLE | OBJ_CASE_INSENSITIVE);
    let created = om.create_object(kernel, &attrs, 0, new, &mut NoParse)?;
    om.close(kernel, created.handle)
}
