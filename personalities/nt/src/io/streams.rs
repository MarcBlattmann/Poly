//! Alternate data streams (ADR 0223; Learn: "File Streams (Local File
//! Systems)"): `file:stream` names a second sequence of bytes of `file`.
//!
//! # Where the bytes are
//!
//! The VFS has one data stream per inode, so each named stream is a
//! regular file of its own, kept outside the volume where no NT or Linux
//! program lists it: in `.poly-streams-<device>` beside the volume's root
//! directory (the root's parent), in a directory per file. The file's
//! extended attribute [`STREAMS_XATTR`] names that directory; the name
//! starts with the file's inode number, and an attribute whose number is
//! not the file's own (a copy made with its extended attributes) is no
//! stream directory of this file.
//!
//! What follows from that:
//! - a rename, on the same volume, keeps the streams (the inode and its
//!   attribute move); hard links share them;
//! - a stream handle is a VFS open file like any other, so reading,
//!   writing, the size, the end of file, locks and share access work as
//!   for a file (share access per stream, as on NTFS);
//! - replacing a file's data (`FILE_SUPERSEDE`, `FILE_OVERWRITE[_IF]`)
//!   drops its streams, and deleting its last name removes them;
//! - a volume whose root is the VFS's root (`Z:`) has no place beside it
//!   and so no named streams (`FILE_NAMED_STREAMS` is not reported there).

use super::consts::STATUS_DISK_FULL;
use super::info;
use super::names::{eq_fold, vfs_to_wide};
use super::walk;
use super::{IoState, NodeKey, VfsRoot, status_of};
use alloc::vec::Vec;
use core::result::Result;
use poly_ntobj::NtStatus;
use poly_ntobj::name::upcase;
use poly_ntobj::status::*;
use poly_vfs::{At, Errno, FileType, NodeRef};

/// The extended attribute that names a file's stream directory.
pub const STREAMS_XATTR: &[u8] = b"user.poly.streams";

/// The name of a volume's stream store, before the device name.
pub const STORE_PREFIX: &str = ".poly-streams-";

/// One named stream of a file: its name as created, its backing file's
/// name in the file's stream directory, and that file.
#[derive(Debug)]
pub struct Stream {
    pub name: Vec<u16>,
    pub backing: Vec<u8>,
    pub node: NodeRef,
}

impl IoState {
    /// The directory beside volume `volume`'s root that holds its streams,
    /// made when `create` asks; `None` when the volume has no place for one.
    fn stream_store(&self, volume: usize, create: bool) -> Result<Option<NodeRef>, NtStatus> {
        let vr = self.vfs()?;
        let Some(v) = self.volumes.get(volume) else { return Ok(None) };
        let Ok(parent) = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&v.root), b"..", true) else { return Ok(None) };
        if NodeKey::of(&parent)? == NodeKey::of(&v.root)? {
            return Ok(None);
        }
        let mut name = STORE_PREFIX.as_bytes().to_vec();
        name.extend(
            char::decode_utf16(v.device.iter().copied())
                .map(|c| c.unwrap_or('_'))
                .collect::<alloc::string::String>()
                .bytes(),
        );
        lookup_or_make(vr, &parent, &name, create)
    }

    /// The volume file object `cookie` is on (0 for one on none).
    pub fn volume_of(&self, cookie: u64) -> usize {
        self.files.get(&cookie).and_then(|f| f.disk()).map_or(0, |d| d.volume)
    }

    /// Whether volume `volume` can hold named streams.
    pub fn has_streams(&self, volume: usize) -> bool {
        let (Ok(vr), Some(v)) = (self.vfs(), self.volumes.get(volume)) else { return false };
        vr.vfs
            .lookup(&vr.ctx, &vr.cred, At::Node(&v.root), b"..", true)
            .is_ok_and(|p| NodeKey::of(&p).ok() != NodeKey::of(&v.root).ok())
    }

    /// The stream directory of `file` (on volume `volume`), made (with the
    /// store) when `create` asks.
    pub fn stream_dir(&self, volume: usize, file: &NodeRef, create: bool) -> Result<Option<NodeRef>, NtStatus> {
        let vr = self.vfs()?;
        let Some(store) = self.stream_store(volume, create)? else {
            return if create { Err(STATUS_NOT_SUPPORTED) } else { Ok(None) };
        };
        let ino = file.ino();
        if let Ok(id) = file.getxattr(&vr.cred, STREAMS_XATTR)
            && owns(&id, ino)
            && let Some(dir) = lookup_or_make(vr, &store, &id, false)?
        {
            return Ok(Some(dir));
        }
        if !create {
            return Ok(None);
        }
        // A new directory: the inode number and the first free count, so a
        // directory left by a deleted file of the same number is never
        // taken over.
        for n in 0u32.. {
            let id = alloc::format!("{ino:x}-{n}").into_bytes();
            match vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Node(&store), &id, 0o700) {
                Ok(()) => {
                    file.setxattr(&vr.cred, STREAMS_XATTR, &id, 0).map_err(status_of)?;
                    return lookup_or_make(vr, &store, &id, false);
                }
                Err(Errno::EEXIST) => continue,
                Err(e) => return Err(status_of(e)),
            }
        }
        Err(STATUS_DISK_FULL)
    }

    /// The named streams of `file`, by name without case.
    pub fn named_streams(&self, volume: usize, file: &NodeRef) -> Vec<Stream> {
        let Ok(vr) = self.vfs() else { return Vec::new() };
        let Ok(Some(dir)) = self.stream_dir(volume, file, false) else { return Vec::new() };
        let mut out: Vec<Stream> = walk::list(vr, &dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n != b"." && n != b"..")
            .filter_map(|backing| {
                let node = vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(&dir), &backing, false).ok()?;
                Some(Stream { name: vfs_to_wide(&backing), backing, node })
            })
            .collect();
        out.sort_by_key(|s| s.name.iter().map(|&c| upcase(c)).collect::<Vec<u16>>());
        out
    }

    /// The backing name of `file`'s stream `name` (compared without case),
    /// if it has one.
    pub fn find_stream(&self, volume: usize, file: &NodeRef, name: &[u16]) -> Option<Vec<u8>> {
        self.named_streams(volume, file).into_iter().find(|s| eq_fold(&s.name, name)).map(|s| s.backing)
    }

    /// Drop every named stream of `file` (its data was replaced, or its last
    /// name went).
    pub fn drop_streams(&self, volume: usize, file: &NodeRef) {
        let Ok(vr) = self.vfs() else { return };
        let Ok(Some(dir)) = self.stream_dir(volume, file, false) else {
            let _ = file.removexattr(&vr.cred, STREAMS_XATTR);
            return;
        };
        for s in self.named_streams(volume, file) {
            let _ = vr.vfs.unlink(&vr.ctx, &vr.cred, At::Node(&dir), &s.backing);
        }
        if let (Ok(store), Ok(id)) = (self.stream_store(volume, false), file.getxattr(&vr.cred, STREAMS_XATTR))
            && let Some(store) = store
        {
            let _ = vr.vfs.rmdir(&vr.ctx, &vr.cred, At::Node(&store), &id);
        }
        let _ = file.removexattr(&vr.cred, STREAMS_XATTR);
    }

    /// Drop the streams of `file` if it has no name left (after an unlink).
    pub fn drop_streams_if_gone(&self, volume: usize, file: &NodeRef) {
        if file.attr().is_ok_and(|a| a.nlink == 0) {
            self.drop_streams(volume, file);
        }
    }

    /// `FILE_STREAM_INFORMATION` of `file` (`facts` its own): the default
    /// stream of a file, then the named streams by name without case.
    pub fn stream_information(&self, volume: Option<usize>, file: Option<&NodeRef>, facts: &info::Facts) -> Vec<u8> {
        let mut entries: Vec<(Vec<u16>, u64, u64)> = Vec::new();
        if file.is_none() {
            return Vec::new();
        }
        if !facts.directory {
            entries.push((info::DEFAULT_STREAM.encode_utf16().collect(), facts.size, facts.allocation));
        }
        if let (Some(v), Some(f)) = (volume, file) {
            for s in self.named_streams(v, f) {
                let Ok(attr) = s.node.attr() else { continue };
                let mut name: Vec<u16> = ":".encode_utf16().collect();
                name.extend_from_slice(&s.name);
                name.extend(":$DATA".encode_utf16());
                entries.push((name, attr.size, info::allocation(attr.size, attr.blocks)));
            }
        }
        info::stream_entries(&entries)
    }
}

/// Whether stream directory name `id` belongs to inode `ino`.
fn owns(id: &[u8], ino: u64) -> bool {
    let want = alloc::format!("{ino:x}-");
    id.starts_with(want.as_bytes())
}

/// The directory `name` in `dir`, made when `create` asks.
fn lookup_or_make(vr: &VfsRoot, dir: &NodeRef, name: &[u8], create: bool) -> Result<Option<NodeRef>, NtStatus> {
    match vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), name, false) {
        Ok(n) if matches!(n.kind(), Ok(FileType::Directory)) => return Ok(Some(n)),
        Ok(_) => return Err(STATUS_OBJECT_NAME_COLLISION),
        Err(Errno::ENOENT) if create => {}
        Err(Errno::ENOENT) => return Ok(None),
        Err(e) => return Err(status_of(e)),
    }
    match vr.vfs.mkdir(&vr.ctx, &vr.cred, At::Node(dir), name, 0o700) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(e) => return Err(status_of(e)),
    }
    vr.vfs.lookup(&vr.ctx, &vr.cred, At::Node(dir), name, false).map(Some).map_err(status_of)
}

/// A stream name as the VFS stores it (UTF-8; a name that cannot be
/// encoded is `STATUS_OBJECT_NAME_INVALID`).
pub fn stream_to_vfs(name: &[u16]) -> Result<Vec<u8>, NtStatus> {
    char::decode_utf16(name.iter().copied())
        .collect::<Result<alloc::string::String, _>>()
        .map(alloc::string::String::into_bytes)
        .map_err(|_| STATUS_OBJECT_NAME_INVALID)
}
