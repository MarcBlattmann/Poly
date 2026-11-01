//! What mount(2) mounts (ADR 0051): new instances of the memory file
//! systems by type, and the disks as block device nodes in `/dev`, so a
//! mount names its disk the way Linux programs do (`mount /dev/vdc /mnt`).
//!
//! | Type | What a mount gets |
//! |---|---|
//! | `proc` | a procfs over the kernel's process table, like `/proc` |
//! | `sysfs` | a sysfs, like `/sys` |
//! | `devpts` | a view of the one set of pseudo-terminals (`/dev/pts`) |
//! | `tmpfs`, `ramfs` | a new, empty tmpfs (root `mode`, `uid`, `gid` from the options) |
//! | `ext2` | the ext2 volume on the disk the source node names, behind the page cache |
//!
//! Disks: `/dev/vda`, `/dev/vdb`, ... are the virtio-blk functions in PCI
//! order (`virtio0`, `virtio1`, ...: block devices, major
//! [`VIRTIO_BLOCK_MAJOR`], minor 16 per disk as Linux numbers `vd`
//! partitions); `/dev/ram0` is the RAM disk (block major 1, minor 0, the
//! RAM disk's number in Linux's device list). A disk comes up when it is
//! first mounted (`block::open`).

use super::{proc, sysfs};
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_personality_linux::admin::{FsType, RootOptions};
use poly_vfs::{At, Cred, DeviceId, Errno, FileSystem, FileType, FsContext, MountId, SetAttr, Vfs};

/// The block major of the virtio-blk disks. Linux gives virtio-blk a
/// dynamic major; this is Poly's fixed choice.
pub const VIRTIO_BLOCK_MAJOR: u32 = 254;
/// Minors per virtio-blk disk (the whole disk and its partitions).
const MINORS_PER_DISK: u32 = 16;
/// The RAM disk's block device number (Linux's "RAM disk" major 1).
pub const RAM_DISK: DeviceId = DeviceId::new(1, 0);

/// The most virtio-blk disks that get nodes (`vda` to `vdz`).
const MAX_DISKS: usize = 26;

/// `/dev/<name>` of the `n`-th virtio-blk disk: `vda`, `vdb`, ...
fn virtio_node(n: usize) -> String {
    format!("vd{}", (b'a' + n as u8) as char)
}

/// Make `/dev/vd*` for every virtio-blk function and `/dev/ram0` (each
/// unless the root has it already). Called once at boot by `fs::init`.
pub fn make_block_nodes(vfs: &Vfs, ctx: &FsContext) -> usize {
    let mut ctx = ctx.clone();
    ctx.set_umask(0);
    let root = Cred::root();
    let mut nodes: alloc::vec::Vec<(String, DeviceId)> = (0..MAX_DISKS)
        .take_while(|&n| crate::virtio_blk::nth_function(n).is_some())
        .map(|n| (virtio_node(n), DeviceId::new(VIRTIO_BLOCK_MAJOR, n as u32 * MINORS_PER_DISK)))
        .collect();
    nodes.push((String::from("ram0"), RAM_DISK));
    let mut made = 0;
    for (name, id) in &nodes {
        let path = format!("/dev/{name}");
        match vfs.mknod(&ctx, &root, At::Cwd, path.as_bytes(), FileType::BlockDevice, 0o660, *id) {
            Ok(()) => made += 1,
            Err(Errno::EEXIST) => {}
            Err(e) => crate::kprintln!("fs: {path}: {}", e.name()),
        }
    }
    made
}

/// The kernel's name of the disk block device `dev` is (`virtio2` for
/// `/dev/vdc`'s number), if it is one.
pub fn disk_of(dev: DeviceId) -> Option<String> {
    if dev == RAM_DISK {
        return Some(String::from(crate::block::RAM_NAME));
    }
    (dev.major == VIRTIO_BLOCK_MAJOR && dev.minor.is_multiple_of(MINORS_PER_DISK))
        .then(|| format!("virtio{}", dev.minor / MINORS_PER_DISK))
}

/// A new instance of the memory file system `fs` (not ext2), with the
/// root options applied.
pub fn new_virtual(fs: FsType, root: RootOptions) -> Result<Arc<dyn FileSystem>, Errno> {
    Ok(match fs {
        FsType::Proc => proc::new_procfs(),
        FsType::Sysfs => sysfs::build(),
        FsType::Devpts => crate::tty::new_devpts().ok_or(Errno::ENODEV)?,
        FsType::Tmpfs => {
            let tmpfs = super::memfs::new_tmpfs_sized(super::clock(), root.size, root.inodes);
            // tmpfs(5): the root is 01777 unless `mode` says otherwise.
            let set =
                SetAttr { perm: Some(root.mode.unwrap_or(0o1777)), uid: root.uid, gid: root.gid, ..SetAttr::default() };
            tmpfs.setattr(tmpfs.root(), &set)?;
            super::memfs::cached(tmpfs)
        }
        FsType::Ext2 => return Err(Errno::ENOTBLK),
    })
}

/// The sources mount(2) was given, by mount: what `/proc/mounts` shows in
/// its first field (`tmpfs`, `none`, `/dev/vdc`).
static SOURCES: SpinLock<BTreeMap<MountId, Vec<u8>>> = SpinLock::new(BTreeMap::new());

/// Remember `source` as what mount `id` was mounted from.
pub fn record_source(id: MountId, source: &[u8]) {
    SOURCES.lock().insert(id, source.to_vec());
}

/// What mount `id` was mounted from, if mount(2) made it.
pub fn source(id: MountId) -> Option<Vec<u8>> {
    SOURCES.lock().get(&id).cloned()
}

/// Mount `id` is gone.
pub fn forget(id: MountId) {
    SOURCES.lock().remove(&id);
}
