//! Loop devices and the file systems mounted from them (ADR 0168):
//! `poly-loop` over the running process's memory and descriptors,
//! `/dev/loop-control` (10, 237), the block nodes `/dev/loop<n>` made when a
//! device is added (what devtmpfs does on Linux), and squashfs mounts.

use crate::fs;
use crate::sync::{RawMutex, SetOnce, SpinLock};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::sync::{Arc, Weak};
use poly_loop::{Env, Loops, MountClaim, Registry};
use poly_squashfs::{IoFailed, Source, Squashfs};
use poly_vfs::{At, CharDevice, Cred, DeviceId, Errno, FileSystem, FileType, OpenFile, Result, Vfs};

type KLoops = Loops<RawMutex>;

static LOOPS: SetOnce<Arc<KLoops>> = SetOnce::new();

/// The squashfs file systems mounted from loop devices, by device number:
/// mounting a device that is mounted already gives the same file system.
static MOUNTED: SpinLock<BTreeMap<u32, Weak<dyn FileSystem>>> = SpinLock::new(BTreeMap::new());

/// The caller's memory and descriptors.
struct KernelEnv;

impl Env for KernelEnv {
    fn read_user(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        crate::user::copy_from_user(buf, addr).map_err(|_| Errno::EFAULT)
    }

    fn write_user(&self, addr: u64, data: &[u8]) -> Result<()> {
        crate::user::copy_to_user(addr, data).map_err(|_| Errno::EFAULT)
    }

    fn file(&self, fd: i32) -> Result<Arc<OpenFile>> {
        crate::user::current_process().ok_or(Errno::EBADF)?.files.file(fd)
    }
}

/// Block devices go to the VFS, with a `/dev/loop<n>` node each.
struct KernelRegistry;

impl Registry for KernelRegistry {
    fn register(&self, id: DeviceId, dev: Arc<dyn CharDevice>) -> Result<()> {
        let vfs = fs::vfs();
        vfs.register_block_device(id, dev)?;
        if let Some(ctx) = node_context(vfs) {
            let path = format!("/dev/loop{}", id.minor);
            let _ = vfs.mknod(&ctx, &Cred::root(), At::Cwd, path.as_bytes(), FileType::BlockDevice, 0o660, id);
        }
        Ok(())
    }

    fn unregister(&self, id: DeviceId) {
        let vfs = fs::vfs();
        vfs.unregister_block_device(id);
        if let Some(ctx) = node_context(vfs) {
            let path = format!("/dev/loop{}", id.minor);
            let _ = vfs.unlink(&ctx, &Cred::root(), At::Cwd, path.as_bytes());
        }
    }
}

fn node_context(vfs: &Vfs) -> Option<poly_vfs::FsContext> {
    let mut ctx = vfs.root_context().ok()?;
    ctx.set_umask(0);
    Some(ctx)
}

/// Register `/dev/loop-control`; called once from `fs::devices::install`.
pub fn install(vfs: &Vfs) {
    let loops = KLoops::new(Arc::new(KernelEnv), Arc::new(KernelRegistry));
    if vfs.register_char_device(poly_loop::CONTROL, loops.control()).is_err() || LOOPS.set(loops).is_err() {
        crate::kprintln!("fs: loop-control registered twice");
    }
}

/// The squashfs image a loop device holds, read through the device.
struct LoopSource {
    claim: MountClaim<RawMutex>,
}

impl Source for LoopSource {
    fn size(&self) -> u64 {
        self.claim.device().size_bytes()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<(), IoFailed> {
        let mut done = 0;
        while done < buf.len() {
            match self.claim.device().read_at(offset + done as u64, &mut buf[done..]) {
                Ok(0) | Err(_) => return Err(IoFailed),
                Ok(n) => done += n,
            }
        }
        Ok(())
    }
}

/// What a mapping of the loop device `id` shows (`poly_loop::Loop::mapped`):
/// its backing file, where the device starts in it and how long it is.
/// `None` for another device or one that is not bound.
pub fn mapped(id: DeviceId) -> Result<Option<poly_loop::Mapped>> {
    if !is_loop(id) {
        return Ok(None);
    }
    match LOOPS.get().and_then(|l| l.device_of(id)) {
        Some(dev) => dev.mapped(),
        None => Ok(None),
    }
}

/// Whether `id` names a loop device.
pub fn is_loop(id: DeviceId) -> bool {
    id.major == poly_loop::LOOP_MAJOR
}

/// The squashfs file system the loop device `id` holds (the same one when
/// it is mounted already).
pub fn mount_squashfs(id: DeviceId) -> Result<Arc<dyn FileSystem>> {
    let loops = LOOPS.get().ok_or(Errno::ENXIO)?;
    let dev = loops.device_of(id).ok_or(Errno::ENXIO)?;
    let mut mounted = MOUNTED.lock();
    if let Some(fs) = mounted.get(&id.minor).and_then(Weak::upgrade) {
        return Ok(fs);
    }
    let claim = dev.claim_mount()?;
    let sq = Squashfs::<LoopSource, RawMutex>::mount(LoopSource { claim }).map_err(|e| e.errno())?;
    // Behind the page cache, so files can be mapped.
    let fs: Arc<dyn FileSystem> = fs::memfs::cached(Arc::new(sq));
    mounted.retain(|_, w| w.strong_count() > 0);
    mounted.insert(id.minor, Arc::downgrade(&fs));
    Ok(fs)
}
