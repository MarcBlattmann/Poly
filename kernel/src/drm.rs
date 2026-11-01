//! The DRM device: `/dev/dri/card0` and `/dev/dri/renderD128` on the
//! virtio GPU ([ADR 0108](../../docs/adr/0108-drm-kms-device-on-virtio-gpu.md)).
//!
//! The device itself (objects, GEM handles, modesetting, events, the ioctl
//! encoding) is the host-tested `poly_drm` crate. This module gives it what
//! only the kernel has:
//!
//! - [`KernelHw`], its `Hardware`: dumb buffers are physically contiguous
//!   frames ([`DumbMemory`]), so a framebuffer's device copy is a virtio-gpu
//!   resource (ID `RESOURCE_BASE` + the framebuffer's ID) with one
//!   scatter-list entry (`RESOURCE_CREATE_2D` + `RESOURCE_ATTACH_BACKING`,
//!   lines of `pitch / 4` pixels); an upload is `TRANSFER_TO_HOST_2D`, a
//!   scanout `SET_SCANOUT`, a flush `RESOURCE_FLUSH`, all through
//!   [`display::with_gpu`]. Taking the display and giving it back are
//!   [`display::console_yield`] and [`display::console_restore`].
//! - the device nodes: char 226:0 and 226:128 (Linux's DRM major and
//!   minors), whose every open is a file of the device ([`DrmFile`]):
//!   ioctls (copying through the user's memory; a blocking vblank wait
//!   sleeps and asks again), `read` of events (`EAGAIN`, so the file layer
//!   waits on the file's queue), `poll`.
//! - mappings ([`mapping`]): a mapping of the card file at a dumb buffer's
//!   offset, or of a dma-buf, maps the buffer's frames themselves
//!   ([`SharedFrame::device`]: never copied, never freed while mapped).
//! - PRIME: a dma-buf is a file object ([`DmaBuf`]) holding the buffer;
//!   `lseek(SEEK_END)` gives its size and `DMA_BUF_IOCTL_SYNC` is accepted.
//! - the vblank thread, which wakes readers when their events come due.
//!
//! Files that close (and dma-bufs) while the device's lock is taken, or
//! where sleeping is not allowed, are handed to the device later
//! ([`DEFERRED`]).

use crate::display;
use crate::fs::{self, OpenFile};
use crate::sync::{Mutex, SpinLock};
use crate::thread::{self, WaitQueue};
use crate::user::memory::{IoError, PageSource, SharedFrame};
use crate::{arch, frames, kprintln, time};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use poly_drm::uapi::fourcc;
use poly_drm::{BoId, Ctx, Device, DriverInfo, FbLayout, FileId, Hardware, HwError, Node, Output, Rect, Reply};
use poly_mem::PAGE_SIZE;
use poly_vfs::device::CharDevice;
use poly_vfs::{At, Cred, DeviceId, Errno, FileObject, FileType, OpenFlags, PollEvents};
use poly_virtio_gpu::MemEntry;
use poly_virtio_gpu::protocol::format;

/// Linux's DRM character device major.
pub const DRM_MAJOR: u32 = 226;
/// `card0` and `renderD128`.
pub const CARD: DeviceId = DeviceId::new(DRM_MAJOR, 0);
pub const RENDER: DeviceId = DeviceId::new(DRM_MAJOR, 128);
/// A framebuffer's virtio resource is this plus its object ID (the
/// console's are 1 and 2).
const RESOURCE_BASE: u32 = 0x1000;
/// `DMA_BUF_IOCTL_SYNC` (linux/dma-buf.h: `_IOW('b', 0, struct
/// dma_buf_sync)`, one `__u64 flags`) and its flags.
const DMA_BUF_IOCTL_SYNC: u32 = 0x4008_6200;
const DMA_BUF_SYNC_FLAGS: u64 = 0x7;

/// The device, once [`init`] found a virtio GPU display.
static DRM: Mutex<Option<Device<KernelHw>>> = Mutex::new(None);

/// Work for the device that could not take its lock where it came up.
enum Deferred {
    Close(FileId),
    Release(BoId),
}

static DEFERRED: SpinLock<Vec<Deferred>> = SpinLock::new(Vec::new());

/// The open files' wait queues, for the vblank thread.
static WAITERS: SpinLock<BTreeMap<FileId, Weak<poly_vfs::WaitQueue>>> = SpinLock::new(BTreeMap::new());

/// When the vblank thread should look next (`u64::MAX`: no event queued).
static NEXT: AtomicU64 = AtomicU64::new(u64::MAX);
static WAKE: WaitQueue = WaitQueue::new();

/// The memory of a dumb buffer: physically contiguous frames, zeroed,
/// freed with the last reference (a GEM object, a mapping, a dma-buf).
#[derive(Debug)]
pub struct DumbMemory {
    phys: u64,
    frames: u64,
    len: u64,
}

impl DumbMemory {
    fn bytes(&self) -> &[u8] {
        let virt = arch::phys_to_virt(self.phys).expect("dumb buffer outside the direct map");
        // SAFETY: the frames stay allocated while `self` lives and the
        // direct map maps them, contiguously. Users write them through
        // their mappings meanwhile; bytes have no invalid values, so a read
        // racing a write sees old or new bytes.
        unsafe { core::slice::from_raw_parts(virt as *const u8, self.len as usize) }
    }
}

impl Drop for DumbMemory {
    fn drop(&mut self) {
        if let Err(e) = frames::free_contiguous(self.phys, self.frames) {
            kprintln!("drm: cannot free a dumb buffer at {:#x}: {e:?}", self.phys);
        }
    }
}

/// A mapping's view of a dumb buffer: its frames, shared.
struct DumbPages {
    memory: Arc<DumbMemory>,
    label: &'static str,
}

impl PageSource for DumbPages {
    fn len(&self) -> u64 {
        self.memory.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        let bytes = self.memory.bytes();
        let start = (offset as usize).min(bytes.len());
        let n = buf.len().min(bytes.len() - start);
        buf[..n].copy_from_slice(&bytes[start..start + n]);
        Ok(n)
    }

    fn shareable(&self) -> bool {
        true
    }

    fn shared_frame(&self, offset: u64) -> Option<Result<Arc<SharedFrame>, IoError>> {
        let page = offset / PAGE_SIZE * PAGE_SIZE;
        if page >= self.memory.frames * PAGE_SIZE {
            return Some(Err(IoError));
        }
        let keep: Arc<dyn core::any::Any + Send + Sync> = self.memory.clone();
        Some(Ok(Arc::new(SharedFrame::device(self.memory.phys + page, keep))))
    }

    fn label(&self) -> Option<&str> {
        Some(self.label)
    }
}

/// The virtio GPU as the device's hardware.
#[derive(Debug, Default)]
pub struct KernelHw;

fn resource(fb: u32) -> u32 {
    RESOURCE_BASE + fb
}

fn gpu_rect(r: Rect) -> poly_virtio_gpu::Rect {
    poly_virtio_gpu::Rect::new(r.x, r.y, r.width, r.height)
}

/// The virtio-gpu format with the same bytes in memory as a DRM fourcc
/// (VIRTIO 1.2 §5.7.6.8: `B8G8R8X8` is blue, green, red, unused in that
/// byte order, which is DRM's little-endian `XRGB8888`).
fn virtio_format(fourcc: u32) -> Option<u32> {
    Some(match fourcc {
        fourcc::XRGB8888 => format::B8G8R8X8_UNORM,
        fourcc::ARGB8888 => format::B8G8R8A8_UNORM,
        fourcc::XBGR8888 => format::R8G8B8X8_UNORM,
        fourcc::ABGR8888 => format::R8G8B8A8_UNORM,
        _ => return None,
    })
}

fn gpu<R>(f: impl FnOnce(&mut crate::virtio_gpu::Driver) -> Result<R, poly_virtio_gpu::Error>) -> Result<R, HwError> {
    match display::with_gpu(f) {
        Some(Ok(r)) => Ok(r),
        Some(Err(e)) => {
            kprintln!("drm: virtio-gpu command failed: {e:?}");
            Err(HwError)
        }
        None => Err(HwError),
    }
}

impl Hardware for KernelHw {
    type Buffer = Arc<DumbMemory>;

    fn probe(&mut self) -> Vec<Output> {
        display::with_gpu(|g| {
            let info = g.display_info().ok();
            (0..g.num_scanouts() as usize)
                .map(|i| {
                    let s = info.map(|info| info.scanouts[i]).unwrap_or_default();
                    let edid = if g.has_edid() && s.enabled {
                        g.edid(i as u32).ok().map(|b| b.bytes().to_vec())
                    } else {
                        None
                    };
                    Output { connected: s.enabled, width: s.rect.width, height: s.rect.height, edid }
                })
                .collect()
        })
        .unwrap_or_default()
    }

    fn alloc(&mut self, size: u64) -> Option<Arc<DumbMemory>> {
        let frames = size.div_ceil(PAGE_SIZE);
        let phys = frames::allocate_contiguous(frames)?;
        let Some(virt) = arch::phys_range_to_virt(phys, frames * PAGE_SIZE) else {
            let _ = frames::free_contiguous(phys, frames);
            return None;
        };
        // SAFETY: the frames were just allocated for this buffer alone and
        // are mapped writable and contiguous by the direct map.
        unsafe { core::ptr::write_bytes(virt as *mut u8, 0, (frames * PAGE_SIZE) as usize) };
        Some(Arc::new(DumbMemory { phys, frames, len: size }))
    }

    fn create_fb(&mut self, fb: u32, layout: &FbLayout, buffer: &Arc<DumbMemory>) -> Result<(), HwError> {
        let format = virtio_format(layout.format).ok_or(HwError)?;
        let id = resource(fb);
        let entry = MemEntry { addr: buffer.phys + layout.offset, length: layout.pitch * layout.height };
        gpu(|g| {
            g.create_2d(id, format, layout.pitch / 4, layout.height)?;
            if let Err(e) = g.attach_backing(id, &[entry]) {
                let _ = g.unref(id);
                return Err(e);
            }
            Ok(())
        })
    }

    fn destroy_fb(&mut self, fb: u32, buffer: &Arc<DumbMemory>) -> Result<(), HwError> {
        let id = resource(fb);
        let r = gpu(|g| {
            g.detach_backing(id)?;
            g.unref(id)
        });
        if r.is_err() {
            // The device may still read the memory: never free it.
            core::mem::forget(buffer.clone());
        }
        r
    }

    fn upload(&mut self, fb: u32, layout: &FbLayout, rect: Rect) -> Result<(), HwError> {
        let offset = rect.y as u64 * layout.pitch as u64 + rect.x as u64 * 4;
        gpu(|g| g.transfer_to_host_2d(resource(fb), gpu_rect(rect), offset))
    }

    fn scanout(&mut self, index: usize, show: Option<(u32, Rect)>) -> Result<(), HwError> {
        let (id, rect) = show.map_or((0, poly_virtio_gpu::Rect::ZERO), |(fb, r)| (resource(fb), gpu_rect(r)));
        gpu(|g| g.set_scanout(index as u32, id, rect))
    }

    fn flush(&mut self, fb: u32, rect: Rect) -> Result<(), HwError> {
        gpu(|g| g.flush(resource(fb), gpu_rect(rect)))
    }

    fn take_display(&mut self) {
        display::console_yield();
    }

    fn give_back_display(&mut self) {
        display::console_restore();
    }
}

/// Run `f` on the device after the work deferred to it. `None` without one.
fn with_device<R>(f: impl FnOnce(&mut Device<KernelHw>) -> R) -> Option<R> {
    let mut guard = DRM.lock();
    let d = guard.as_mut()?;
    drain(d);
    Some(f(d))
}

fn drain(d: &mut Device<KernelHw>) {
    let work = core::mem::take(&mut *DEFERRED.lock());
    for w in work {
        match w {
            Deferred::Close(file) => d.close(file, time::now_ns()),
            Deferred::Release(bo) => d.release_dmabuf(bo),
        }
    }
}

/// Hand `w` to the device now if its lock is free, else later.
fn defer(w: Deferred) {
    DEFERRED.lock().push(w);
    if let Some(mut guard) = DRM.try_lock()
        && let Some(d) = guard.as_mut()
    {
        drain(d);
    } else {
        // The vblank thread drains it on its next round.
        NEXT.store(0, SeqCst);
        WAKE.wake_all();
    }
}

fn errno(e: poly_drm::Errno) -> Errno {
    Errno::from_code(e.0).unwrap_or(Errno::EINVAL)
}

/// The caller's memory.
struct UserMem;

impl poly_drm::UserMemory for UserMem {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> poly_drm::Result<()> {
        crate::user::copy_from_user(buf, addr).map_err(|_| poly_drm::Errno::EFAULT)
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> poly_drm::Result<()> {
        crate::user::copy_to_user(addr, data).map_err(|_| poly_drm::Errno::EFAULT)
    }
}

/// The caller's file table, for PRIME.
struct ProcessFds;

impl poly_drm::Fds<Arc<DumbMemory>> for ProcessFds {
    fn export(&mut self, bo: BoId, buffer: &Arc<DumbMemory>, size: u64, flags: u32) -> poly_drm::Result<i32> {
        let process = crate::user::current_process().ok_or(poly_drm::Errno::EBADF)?;
        let files = &process.files;
        let node =
            fs::vfs().anon_node(FileType::Regular, 0o600, &files.cred()).map_err(|e| poly_drm::Errno(e.code()))?;
        let buf = Arc::new(DmaBuf { bo, memory: buffer.clone(), size, holds: AtomicBool::new(true) });
        let object: Arc<dyn FileObject> = buf.clone();
        let mode = if flags & poly_drm::uapi::prime::RDWR != 0 { OpenFlags::RDWR } else { OpenFlags::RDONLY };
        let file = Arc::new(OpenFile::on_object(node, object, mode));
        let cloexec = flags & poly_drm::uapi::prime::CLOEXEC != 0;
        files.install(file, cloexec).map_err(|e| {
            // The device takes its reference back itself on an error.
            buf.holds.store(false, SeqCst);
            poly_drm::Errno(e.code())
        })
    }

    fn import(&mut self, fd: i32) -> poly_drm::Result<BoId> {
        let process = crate::user::current_process().ok_or(poly_drm::Errno::EBADF)?;
        let file = process.files.file(fd).map_err(|_| poly_drm::Errno::EBADF)?;
        file.object_as::<DmaBuf>().map(|b| b.bo).ok_or(poly_drm::Errno::EINVAL)
    }
}

/// A device node: every open is a new file of the device.
struct DrmNode(Node);

impl core::fmt::Debug for DrmNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DrmNode({:?})", self.0)
    }
}

impl CharDevice for DrmNode {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::ENODEV)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::ENODEV)
    }

    fn open(&self, _: OpenFlags) -> poly_vfs::Result<Option<Arc<dyn CharDevice>>> {
        let id = with_device(|d| d.open(self.0)).ok_or(Errno::ENODEV)?;
        let wait = Arc::new(poly_vfs::WaitQueue::new());
        WAITERS.lock().insert(id, Arc::downgrade(&wait));
        Ok(Some(Arc::new(DrmFile { id, node: self.0, wait })))
    }
}

/// One open of a DRM node.
pub struct DrmFile {
    id: FileId,
    node: Node,
    wait: Arc<poly_vfs::WaitQueue>,
}

impl core::fmt::Debug for DrmFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DrmFile({}, {:?})", self.id, self.node)
    }
}

impl Drop for DrmFile {
    fn drop(&mut self) {
        WAITERS.lock().remove(&self.id);
        defer(Deferred::Close(self.id));
    }
}

/// Look again when the next event comes due (the vblank thread).
fn schedule(next: Option<u64>) {
    let next = next.unwrap_or(u64::MAX);
    if next < NEXT.fetch_min(next, SeqCst) {
        WAKE.wake_all();
    }
}

impl CharDevice for DrmFile {
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        let now = time::now_ns();
        let bytes = with_device(|d| d.read(self.id, buf.len(), now)).ok_or(Errno::ENODEV)?.map_err(errno)?;
        buf[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::EINVAL)
    }

    fn ioctl(&self, request: u32, arg: u64) -> poly_vfs::Result<u64> {
        let privileged = crate::linux::cred::current_vfs_cred().has(poly_vfs::cap::SYS_ADMIN);
        loop {
            let now = time::now_ns();
            let (reply, readable, next) = with_device(|d| {
                let mut ctx = Ctx { user: &mut UserMem, fds: &mut ProcessFds, now_ns: now, privileged };
                let reply = d.ioctl(self.id, request, arg, &mut ctx);
                (reply, d.readable(self.id, now), d.next_event_after(now))
            })
            .ok_or(Errno::ENODEV)?;
            schedule(next);
            if readable {
                self.wait.notify(PollEvents::READABLE);
            }
            match reply.map_err(errno)? {
                Reply::Done(v) => return Ok(v),
                // A blocking vblank wait (rewritten to an absolute one), or
                // a blocking commit behind a pending flip: ask again then.
                Reply::Sleep { until_ns } => thread::sleep_until(until_ns.max(now + 1)),
            }
        }
    }

    fn poll(&self) -> PollEvents {
        let now = time::now_ns();
        match with_device(|d| d.readable(self.id, now)) {
            Some(true) => PollEvents::READABLE,
            Some(false) => PollEvents::NONE,
            None => PollEvents::ERR | PollEvents::HUP,
        }
    }

    fn wait_queue(&self) -> Option<&Arc<poly_vfs::WaitQueue>> {
        Some(&self.wait)
    }

    fn as_any(&self) -> Option<&(dyn core::any::Any + Send + Sync)> {
        Some(self)
    }
}

/// A dma-buf exported with `PRIME_HANDLE_TO_FD`: it holds the GEM object
/// (given back when the last descriptor closes) and its memory.
pub struct DmaBuf {
    bo: BoId,
    memory: Arc<DumbMemory>,
    size: u64,
    /// Whether it holds a reference on the GEM object (not if making its
    /// descriptor failed).
    holds: AtomicBool,
}

impl core::fmt::Debug for DmaBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DmaBuf({:?}, {} bytes)", self.bo, self.size)
    }
}

impl Drop for DmaBuf {
    fn drop(&mut self) {
        if self.holds.load(SeqCst) {
            defer(Deferred::Release(self.bo));
        }
    }
}

impl FileObject for DmaBuf {
    fn read(&self, _: &mut [u8]) -> poly_vfs::Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: &[u8]) -> poly_vfs::Result<usize> {
        Err(Errno::EINVAL)
    }

    /// No fences: always ready.
    fn poll(&self) -> PollEvents {
        PollEvents::READABLE | PollEvents::WRITABLE
    }

    fn wait_queue(&self) -> Option<&Arc<poly_vfs::WaitQueue>> {
        None
    }

    fn ioctl(&self, request: u32, arg: u64) -> poly_vfs::Result<u64> {
        if request != DMA_BUF_IOCTL_SYNC {
            return Err(Errno::ENOTTY);
        }
        // CPU access needs no bracketing on a coherent buffer; the flags
        // must still be valid.
        let mut flags = [0u8; 8];
        crate::user::copy_from_user(&mut flags, arg).map_err(|_| Errno::EFAULT)?;
        if u64::from_le_bytes(flags) & !DMA_BUF_SYNC_FLAGS != 0 {
            return Err(Errno::EINVAL);
        }
        Ok(0)
    }

    fn seek_size(&self) -> Option<u64> {
        Some(self.size)
    }
}

/// The pages a mapping maps and where in them it starts.
pub type Mapped = (Arc<dyn PageSource>, u64);

/// What a mapping of `file` at `offset` (`len` bytes) maps, if `file` is a
/// DRM device file (a dumb buffer at its fake offset) or a dma-buf (its
/// buffer from `offset`): the pages and where in them the mapping starts.
/// `None` for every other file.
pub fn mapping(file: &OpenFile, offset: u64, len: u64) -> Option<Result<Mapped, Errno>> {
    if let Some(any) = file.device().and_then(|d| d.as_any())
        && let Some(f) = any.downcast_ref::<DrmFile>()
    {
        let found = with_device(|d| d.map(f.id, offset, len)).unwrap_or(Err(poly_drm::Errno::ENODEV));
        return Some(found.map_err(errno).map(|m| {
            let label = if f.node == Node::Render { "/dev/dri/renderD128" } else { "/dev/dri/card0" };
            let pages: Arc<dyn PageSource> = Arc::new(DumbPages { memory: m.buffer, label });
            (pages, m.start)
        }));
    }
    let buf = file.object_as::<DmaBuf>()?;
    if len == 0 || offset.checked_add(len).is_none_or(|end| end > buf.size.next_multiple_of(PAGE_SIZE)) {
        return Some(Err(Errno::EINVAL));
    }
    let pages: Arc<dyn PageSource> = Arc::new(DumbPages { memory: buf.memory.clone(), label: "/dmabuf:" });
    Some(Ok((pages, offset)))
}

/// Wake the readers whose events came due, at those times.
fn vblank_thread(_: usize) -> usize {
    loop {
        let next = NEXT.load(SeqCst);
        if next == u64::MAX {
            WAKE.wait_until(|| NEXT.load(SeqCst) != u64::MAX);
        } else {
            WAKE.wait_until_deadline(|| NEXT.load(SeqCst) < next, next);
        }
        let now = time::now_ns();
        NEXT.store(u64::MAX, SeqCst);
        let Some((ready, next)) = with_device(|d| {
            d.tick(now);
            (d.ready_files(now), d.next_event_after(now))
        }) else {
            continue;
        };
        schedule(next);
        let queues: Vec<Arc<poly_vfs::WaitQueue>> = {
            let w = WAITERS.lock();
            ready.iter().filter_map(|f| w.get(f).and_then(Weak::upgrade)).collect()
        };
        for q in queues {
            q.notify(PollEvents::READABLE);
        }
    }
}

/// The bus ID `GET_UNIQUE` reports: `pci:<domain>:<bus>:<device>.<function>`.
fn unique() -> alloc::string::String {
    display::gpu_bdf().map_or_else(|| "virtio".into(), |b| format!("pci:0000:{b}"))
}

/// Make the device if the display is on a virtio GPU, its nodes in
/// `/dev/dri`, and the vblank thread. Call after `fs::init`.
pub fn init() {
    if display::gpu_bdf().is_none() {
        kprintln!("drm: no virtio GPU display; no /dev/dri");
        return;
    }
    let info = DriverInfo {
        name: "virtio_gpu",
        desc: "virtio GPU (Poly, 2D)",
        date: "0",
        version: (0, 1, 0),
        unique: unique(),
    };
    let device = Device::new(KernelHw, info);
    let crtcs = device.crtc_count();
    *DRM.lock() = Some(device);
    let vfs = fs::vfs();
    let Ok(mut ctx) = vfs.root_context() else { return };
    ctx.set_umask(0);
    let root = Cred::root();
    match vfs.mkdir(&ctx, &root, At::Cwd, b"/dev/dri", 0o755) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(e) => {
            kprintln!("drm: /dev/dri: {}", e.name());
            return;
        }
    }
    for (path, id, node) in
        [(&b"/dev/dri/card0"[..], CARD, Node::Primary), (b"/dev/dri/renderD128", RENDER, Node::Render)]
    {
        if let Err(e) = vfs.register_char_device(id, Arc::new(DrmNode(node))) {
            kprintln!("drm: device {}:{} not registered: {}", id.major, id.minor, e.name());
            continue;
        }
        // Linux systems give 0660 and the video / render groups; there is
        // one user of the machine's display.
        match vfs.mknod(&ctx, &root, At::Cwd, path, FileType::CharDevice, 0o666, id) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => kprintln!("drm: {}: {}", alloc::string::String::from_utf8_lossy(path), e.name()),
        }
    }
    if let Err(e) = thread::spawn_named("drm-vblank", vblank_thread, 0) {
        kprintln!("drm: no vblank thread ({e:?}); events wake readers only when they look");
    }
    kprintln!(
        "drm: /dev/dri/card0 (226:0) and renderD128 (226:128) on virtio-gpu {}: {crtcs} CRTC(s), dumb buffers, PRIME, atomic",
        unique()
    );
}

/// The sysfs entries of the DRM device (`fs::sysfs`): the PCI function's
/// identity files and the `drm/card0`, `drm/renderD128` minors under it,
/// as the sysfs ABI documents them (`Documentation/ABI/testing/sysfs-bus-pci`,
/// `sysfs-class-drm`... in the kernel's public documentation), so that
/// libdrm and Mesa can find the device and its PCI ID.
pub fn sysfs(b: &mut poly_procfs::StaticFsBuilder) {
    let Some(bdf) = display::gpu_bdf() else { return };
    let Some(dev) = crate::pci::devices().iter().find(|d| d.bdf() == bdf) else { return };
    let h = &dev.header;
    let slot = format!("0000:{bdf}");
    let pci = format!("devices/pci0000:00/{slot}");
    let class = (h.class.class as u32) << 16 | (h.class.subclass as u32) << 8 | h.class.prog_if as u32;
    b.text(&format!("{pci}/vendor"), &format!("{:#06x}\n", h.vendor_id));
    b.text(&format!("{pci}/device"), &format!("{:#06x}\n", h.device_id));
    b.text(&format!("{pci}/subsystem_vendor"), &format!("{:#06x}\n", h.subsystem_vendor_id));
    b.text(&format!("{pci}/subsystem_device"), &format!("{:#06x}\n", h.subsystem_id));
    b.text(&format!("{pci}/revision"), &format!("{:#04x}\n", h.revision));
    b.text(&format!("{pci}/class"), &format!("{class:#08x}\n"));
    b.text(
        &format!("{pci}/uevent"),
        &format!(
            "DRIVER=virtio-pci\nPCI_CLASS={class:X}\nPCI_ID={:04X}:{:04X}\nPCI_SUBSYS_ID={:04X}:{:04X}\nPCI_SLOT_NAME={slot}\n",
            h.vendor_id, h.device_id, h.subsystem_vendor_id, h.subsystem_id
        ),
    );
    b.link(&format!("{pci}/subsystem"), "../../../bus/pci");
    b.link(&format!("bus/pci/devices/{slot}"), &format!("../../../{pci}"));
    for (name, id) in [("card0", CARD), ("renderD128", RENDER)] {
        let dir = format!("{pci}/drm/{name}");
        b.text(&format!("{dir}/dev"), &format!("{}:{}\n", id.major, id.minor));
        b.text(
            &format!("{dir}/uevent"),
            &format!("MAJOR={}\nMINOR={}\nDEVNAME=dri/{name}\nDEVTYPE=drm_minor\n", id.major, id.minor),
        );
        b.link(&format!("{dir}/device"), "../..");
        b.link(&format!("{dir}/subsystem"), "../../../../../class/drm");
        b.link(&format!("dev/char/{}:{}", id.major, id.minor), &format!("../../{dir}"));
        b.link(&format!("class/drm/{name}"), &format!("../../{dir}"));
    }
}

// --- Tests --------------------------------------------------------------------

use crate::kernel_test;

/// The display tests' flag: a virtio GPU was attached, so the device must
/// be there.
fn require(env: &crate::ktest::TestEnv) -> Result<bool, &'static str> {
    let present = DRM.lock().is_some();
    if env.boot.has_flag(display::TEST_VIRTIO_GPU_FLAG) && !present {
        return Err("poly.test.virtio_gpu is set but there is no DRM device");
    }
    Ok(present)
}

kernel_test! {
    /// `/dev/dri/card0` and `renderD128` are char 226:0 and 226:128, and
    /// the device has a CRTC per scanout.
    fn drm_nodes_exist(env: &TestEnv) -> TestResult {
        if !require(env)? {
            return Ok(());
        }
        let files = fs::Files::new();
        for (path, id) in [(&b"/dev/dri/card0"[..], CARD), (b"/dev/dri/renderD128", RENDER)] {
            let st = files.stat(path).map_err(|_| "a DRM node is missing")?;
            if st.mode & 0o170_000 != 0o020_000 || st.rdev != id {
                return Err("a DRM node is not the right character device");
            }
        }
        // sysfs leads from the device number to the PCI function's ID.
        let fd = files
            .open(b"/sys/dev/char/226:0/device/vendor", OpenFlags::RDONLY, 0)
            .map_err(|_| "no /sys/dev/char/226:0/device/vendor")?;
        let mut buf = [0u8; 16];
        let n = files.read(fd, &mut buf).map_err(|_| "cannot read the vendor file")?;
        let _ = files.close(fd);
        if &buf[..n] != b"0x1af4\n" {
            return Err("/sys/dev/char/226:0/device/vendor is not the virtio vendor 0x1af4");
        }
        let crtcs = with_device(|d| d.crtc_count()).unwrap_or(0);
        let scanouts = display::with_gpu(|g| g.num_scanouts() as usize).unwrap_or(0);
        if crtcs == 0 || crtcs != scanouts { Err("the DRM device does not have a CRTC per scanout") } else { Ok(()) }
    }
}

kernel_test! {
    /// The hardware side on the real device: a dumb buffer becomes a
    /// resource, shown on scanout 0 instead of the console (which yields),
    /// uploaded and flushed without errors; turned off and destroyed, the
    /// console shows itself again.
    fn drm_hardware_shows_a_buffer_and_gives_the_display_back(env: &TestEnv) -> TestResult {
        if !require(env)? {
            return Ok(());
        }
        let mut hw = KernelHw;
        let out = hw.probe();
        let first = out.first().ok_or("no outputs")?;
        let (w, h) = (first.width.max(64), first.height.max(64));
        let layout = FbLayout { width: w, height: h, pitch: w * 4, offset: 0, format: fourcc::XRGB8888 };
        let size = (layout.pitch as u64 * h as u64).next_multiple_of(PAGE_SIZE);
        let buf = hw.alloc(size).ok_or("no memory for a dumb buffer")?;
        let errors_before = display::with_gpu(|g| g.stats().errors).unwrap_or(0);
        const FB: u32 = 0x0fff_0000;
        hw.create_fb(FB, &layout, &buf).map_err(|_| "RESOURCE_CREATE_2D / ATTACH_BACKING failed")?;
        hw.take_display();
        let all = Rect::new(0, 0, w, h);
        let shown = hw.upload(FB, &layout, all).and_then(|()| hw.scanout(0, Some((FB, all)))).and_then(|()| hw.flush(FB, all));
        let yielded = display::console_yielded();
        let off = hw.scanout(0, None);
        hw.give_back_display();
        let destroyed = hw.destroy_fb(FB, &buf);
        let errors = display::with_gpu(|g| g.stats().errors).unwrap_or(0) - errors_before;
        if shown.is_err() || off.is_err() || destroyed.is_err() || errors != 0 {
            return Err("the virtio GPU refused a DRM command");
        }
        if !yielded || display::console_yielded() {
            return Err("the console did not yield to DRM and come back");
        }
        Ok(())
    }
}
