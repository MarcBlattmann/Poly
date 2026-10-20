//! Disk file systems: ext2 volumes on the kernel's disks, behind the page
//! cache, mounted into the VFS (ADR 0040).
//!
//! ```text
//!   fs::vfs() ── mount at <path> ──▶ CachedDiskFs (page cache) ──▶ Ext2<DiskIo, RawMutex> ──▶ block::Disk
//! ```
//!
//! - [`DiskIo`] is a `block::Disk` as `poly_ext2::BlockIo`: reads and
//!   writes through the disk's buffer cache, `flush` is `Disk::sync`.
//! - [`mount`] brings the disk up if needed (`block::open`: `virtio1` and
//!   later virtio-blk disks come up on demand), mounts ext2 on it with the
//!   kernel's sleeping mutex as the volume lock, registers it with the page
//!   cache and mounts the result on a directory (made if missing).
//!   [`unmount`] undoes it: the VFS mount must not be busy; the dirty pages
//!   are written back, the volume synced and marked clean.
//! - [`mount_from_cmdline`]: `poly.mount=<disk>:<path>[:ro]`, several
//!   separated by commas, mounted at boot (`poly.mount=virtio2:/mnt`).
//! - [`sync_all`] (the shell's `sync`), [`writeback_pass`] (the writeback
//!   thread) and [`shutdown`] (before a reboot: every volume unmounted
//!   clean, though files may still be open).

use super::cache::{self, CachedDiskFs};
use super::vfs;
use crate::block::{self, Disk};
use crate::kprintln;
use crate::sync::{Mutex, RawMutex};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};
use poly_block::{CacheError, IoError};
use poly_ext2::{BlockIo, Ext2, MountOptions};
use poly_vfs::{At, Cred, Errno, FileSystem, MountFlags, MountId};

/// Kernel flag: disks to mount at boot, `<disk>:<path>[:ro]`, comma-separated.
pub const MOUNT_ARG: &str = "poly.mount";

/// A `block::Disk` as ext2's block device.
#[derive(Debug, Clone, Copy)]
pub struct DiskIo(pub &'static Disk);

fn io_error(e: CacheError) -> IoError {
    match e {
        CacheError::Io(e) => e,
        CacheError::AllPinned => IoError::Busy,
    }
}

impl BlockIo for DiskIo {
    fn block_size(&self) -> u32 {
        self.0.info().block_size
    }

    fn blocks(&self) -> u64 {
        self.0.info().blocks
    }

    fn read_only(&self) -> bool {
        self.0.info().read_only
    }

    fn read(&self, first: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.0.read(first, buf).map_err(io_error)
    }

    fn write(&self, first: u64, buf: &[u8]) -> Result<(), IoError> {
        let bs = self.0.info().block_size as usize;
        if !buf.len().is_multiple_of(bs) {
            return Err(IoError::BadRequest);
        }
        let mut cache = self.0.lock();
        for (block, data) in (first..).zip(buf.chunks(bs)) {
            cache.write_block(block, data).map_err(io_error)?;
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), IoError> {
        self.0.sync().map_err(io_error)
    }
}

/// An ext2 volume as the kernel mounts it.
pub type Ext2Fs = Ext2<DiskIo, RawMutex>;

/// A mounted disk.
pub struct DiskMount {
    pub disk: &'static Disk,
    /// Where it is mounted, as given (absolute).
    pub path: String,
    pub ext2: Arc<Ext2Fs>,
    /// What the VFS has: the volume behind the page cache.
    pub cached: Arc<CachedDiskFs>,
    pub mount_id: MountId,
    pub read_only: bool,
}

impl fmt::Debug for DiskMount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskMount").field("disk", &self.disk.name()).field("path", &self.path).finish()
    }
}

static MOUNTS: Mutex<Vec<Arc<DiskMount>>> = Mutex::new(Vec::new());
/// Set by [`shutdown`]: the volumes are read-only from then on.
static SHUT_DOWN: AtomicBool = AtomicBool::new(false);

/// Every mounted disk, in mount order.
pub fn mounts() -> Vec<Arc<DiskMount>> {
    MOUNTS.lock().clone()
}

/// The cached disk file system `fs` is, if it is one (`fs` as the VFS
/// holds it).
pub fn cached_fs_of(fs: &Arc<dyn FileSystem>) -> Option<Arc<CachedDiskFs>> {
    let want = Arc::as_ptr(fs) as *const ();
    MOUNTS.lock().iter().find(|m| Arc::as_ptr(&m.cached) as *const () == want).map(|m| Arc::clone(&m.cached))
}

/// The mounted disk whose VFS mount is `id`.
pub fn mount_of(id: MountId) -> Option<Arc<DiskMount>> {
    MOUNTS.lock().iter().find(|m| m.mount_id == id).cloned()
}

/// Why a disk did not mount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountError {
    /// No disk of that name, or it did not come up.
    NoDisk,
    /// The disk is mounted already.
    Busy,
    Ext2(poly_ext2::MountError),
    /// The mount point: missing and cannot be made, not a directory, ...
    Vfs(Errno),
}

impl MountError {
    pub fn errno(&self) -> Errno {
        match self {
            MountError::NoDisk => Errno::ENODEV,
            MountError::Busy => Errno::EBUSY,
            MountError::Ext2(e) => e.errno(),
            MountError::Vfs(e) => *e,
        }
    }
}

impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MountError::NoDisk => write!(f, "no such disk"),
            MountError::Busy => write!(f, "the disk is mounted already"),
            MountError::Ext2(e) => write!(f, "{e}"),
            MountError::Vfs(e) => write!(f, "{}", e.name()),
        }
    }
}

/// Make the directory `path` and any missing parents, as root, 0755.
fn make_dirs(path: &str) -> Result<(), Errno> {
    let v = vfs();
    let ctx = v.root_context()?;
    let cred = Cred::root();
    let mut at = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        at.push('/');
        at.push_str(part);
        match v.mkdir(&ctx, &cred, At::Cwd, at.as_bytes(), 0o755) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Mount the ext2 volume on disk `name` at `path` (absolute; made if
/// missing), read-only if asked or if the volume or disk says so.
pub fn mount(name: &str, path: &str, read_only: bool) -> Result<Arc<DiskMount>, MountError> {
    if !path.starts_with('/') {
        return Err(MountError::Vfs(Errno::EINVAL));
    }
    let disk = block::open(name).ok_or(MountError::NoDisk)?;
    let mut mounts = MOUNTS.lock();
    if mounts.iter().any(|m| core::ptr::eq(m.disk, disk)) {
        return Err(MountError::Busy);
    }
    let opts = MountOptions { read_only, ..MountOptions::default() };
    let ext2 = Ext2::mount_with_lock(DiskIo(disk), super::clock(), opts, RawMutex::new()).map_err(MountError::Ext2)?;
    let ext2 = Arc::new(ext2);
    let read_only = ext2.read_only();
    let cached = cache::cache().register(Arc::clone(&ext2) as Arc<dyn FileSystem>);
    let v = vfs();
    let flags = MountFlags { read_only, ..MountFlags::default() };
    let mounted = make_dirs(path).and_then(|()| {
        let ctx = v.root_context()?;
        v.mount(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), Arc::clone(&cached) as Arc<dyn FileSystem>, flags)
    });
    let mount_id = match mounted {
        Ok(id) => id,
        Err(e) => {
            let _ = ext2.unmount();
            return Err(MountError::Vfs(e));
        }
    };
    let s = ext2.stats();
    kprintln!(
        "fs: {} mounted on {path}: ext2, {} blocks of {} bytes ({} free), {} inodes ({} free), {}{}",
        disk.name(),
        s.blocks,
        s.block_size,
        s.free_blocks,
        s.inodes,
        s.free_inodes,
        if read_only { "read-only" } else { "read-write" },
        if s.was_clean { "" } else { "; was not cleanly unmounted (run e2fsck)" },
    );
    let m = Arc::new(DiskMount { disk, path: String::from(path), ext2, cached, mount_id, read_only });
    mounts.push(Arc::clone(&m));
    Ok(m)
}

/// Unmount the disk mounted at `path`: `EBUSY` while anything uses the
/// mount (open files, mappings, working directories, mounts below it),
/// `EINVAL` if no disk is mounted there. Writes everything back and marks
/// the volume clean.
pub fn unmount(path: &str) -> Result<(), Errno> {
    let path = path.trim_end_matches('/');
    let mut mounts = MOUNTS.lock();
    let at = mounts.iter().position(|m| m.path.trim_end_matches('/') == path).ok_or(Errno::EINVAL)?;
    let m = Arc::clone(&mounts[at]);
    let v = vfs();
    let ctx = v.root_context()?;
    // Data first, while the mount is still there to come back to.
    m.cached.sync()?;
    v.umount(&ctx, &Cred::root(), At::Cwd, m.path.as_bytes(), false)?;
    let synced = m.cached.sync();
    let unmounted = m.ext2.unmount();
    mounts.remove(at);
    kprintln!("fs: {} unmounted from {}", m.disk.name(), m.path);
    synced.and(unmounted)
}

/// `sync`: write back every dirty page and sync every disk file system
/// and every disk. Returns the first error.
pub fn sync_all() -> Result<(), Errno> {
    let mut result = Ok(());
    for m in mounts() {
        if let Err(e) = m.cached.sync() {
            kprintln!("fs: sync of {} ({}) failed: {}", m.path, m.disk.name(), e.name());
            result = result.and(Err(e));
        }
    }
    for d in block::disks() {
        if d.sync().is_err() {
            result = result.and(Err(Errno::EIO));
        }
    }
    result
}

/// One pass of the writeback thread: sync each disk file system that
/// changed since the last pass or has dirty pages.
pub fn writeback_pass() {
    if SHUT_DOWN.load(Ordering::SeqCst) {
        return;
    }
    for m in mounts() {
        if m.read_only {
            continue;
        }
        let changed = m.cached.take_changed();
        if (changed || m.cached.page_counts().1 > 0)
            && let Err(e) = m.cached.sync()
        {
            kprintln!("fs: writeback to {} ({}) failed: {}", m.path, m.disk.name(), e.name());
        }
    }
}

/// Before the machine goes down: write everything back and mark every
/// volume clean. They refuse changes afterwards. Reports to `out`.
pub fn shutdown(out: &mut dyn Write) {
    SHUT_DOWN.store(true, Ordering::SeqCst);
    for m in mounts() {
        let synced = m.cached.sync();
        let unmounted = m.ext2.unmount();
        match synced.and(unmounted) {
            Ok(()) => {
                let _ = writeln!(out, "fs: {} ({}) unmounted clean", m.path, m.disk.name());
            }
            Err(e) => {
                let _ = writeln!(out, "fs: {} ({}): unmounting failed: {}", m.path, m.disk.name(), e.name());
            }
        }
    }
}

/// Mount what `poly.mount=` asks for: `<disk>:<path>[:ro]`, comma-separated.
pub fn mount_from_cmdline(arg: &str) {
    for spec in arg.split(',').filter(|s| !s.is_empty()) {
        let mut parts = spec.split(':');
        let (Some(disk), Some(path)) = (parts.next(), parts.next()) else {
            kprintln!("fs: {MOUNT_ARG}: '{spec}' is not <disk>:<path>[:ro]");
            continue;
        };
        let read_only = match parts.next() {
            None | Some("rw") => false,
            Some("ro") => true,
            Some(other) => {
                kprintln!("fs: {MOUNT_ARG}: unknown option '{other}' in '{spec}'");
                continue;
            }
        };
        if let Err(e) = mount(disk, path, read_only) {
            kprintln!("fs: mounting {disk} on {path} failed: {e}");
        }
    }
}

/// What the shell's `mount` shows as the source of mount `id`: the disk's
/// name if it is a disk mount, else the file system's name.
pub fn source_of(id: MountId, fs_name: &str) -> String {
    mount_of(id).map_or_else(|| String::from(fs_name), |m| String::from(m.disk.name()))
}
