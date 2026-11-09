//! 3D on the virtio GPU: what [`crate::drm`]'s hardware needs beyond 2D
//! ([ADR 0119](../../docs/adr/0119-virtio-gpu-3d-virgl-and-venus.md)).
//!
//! The protocol (3D contexts, capsets, 3D and blob resources, command
//! submission, tickets and their fences) is the host-tested
//! `poly_virtio_gpu` driver, and the `DRM_IOCTL_VIRTGPU_*` interface is the
//! host-tested `poly_drm` crate. This module gives them what only the
//! kernel has:
//!
//! - **Fences.** A submission or 3D transfer goes out asynchronously
//!   through [`display::with_gpu`]; its command stream is copied into DMA
//!   memory that lives until the device answers. The driver's fence ID is
//!   the DRM fence. A fence is done once the device answered its request
//!   ([`fence_done`] collects what came back); a `drm-fence` thread
//!   collects answers when the control queue interrupts (MSI-X entry 0) or,
//!   without MSI-X, every millisecond while fences are out, then wakes
//!   whoever waits ([`wait_fence`]) and the sync files' pollers.
//! - **Sync files** ([`SyncFile`], `linux/sync_file.h`): the descriptors
//!   `EXECBUFFER` hands out for its fence (`FENCE_FD_OUT`) and takes
//!   (`FENCE_FD_IN`): readable once every fence in them is done,
//!   `SYNC_IOC_MERGE` and `SYNC_IOC_FILE_INFO`.
//! - **The host-visible region**: mappable host blobs are mapped
//!   (`RESOURCE_MAP_BLOB`) at offsets a [`poly_virtio_gpu::hostmem::Ranges`]
//!   hands out; a mapping of the blob maps those device pages.

use crate::display;
use crate::dma::DmaBuffer;
use crate::fs::{self, OpenFile};
use crate::sync::SpinLock;
use crate::thread::{self, WaitQueue};
use crate::{kprintln, time, virtio_gpu};
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use poly_drm::{CapsetInfo, GpuCaps, HwError};
use poly_vfs::{Errno, FileObject, FileType, OpenFlags, PollEvents};
use poly_virtio_gpu::hostmem::Ranges;
use poly_virtio_gpu::{Buffer, DmaRegion, Ticket, Transfer3d};

/// A request in flight: the driver's ticket and the memory the device may
/// still read.
struct InFlight {
    ticket: Ticket,
    _data: Vec<DmaBuffer>,
    /// Memory to keep until the device answered (a destroyed resource's).
    _keep: Option<Keep>,
}

/// Memory kept alive until a request is answered.
pub type Keep = Arc<dyn core::any::Any + Send + Sync>;

/// Fences in flight, by fence ID.
static FENCES: SpinLock<BTreeMap<u64, InFlight>> = SpinLock::new(BTreeMap::new());
/// How many fences are in flight (read without the lock by the thread).
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
/// Interrupts taken; and fences completed (both only ever grow).
static IRQS: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
/// The fence thread sleeps here; waiters for fences there.
static WAKE: WaitQueue = WaitQueue::new();
static DONE: WaitQueue = WaitQueue::new();
/// Whether the control queue interrupts (else the thread polls).
static INTERRUPTS: AtomicBool = AtomicBool::new(false);
/// The open sync files' wait queues.
static FILE_WAITERS: SpinLock<Vec<Weak<poly_vfs::WaitQueue>>> = SpinLock::new(Vec::new());
/// Held while answers are collected ([`poll_all`]).
static POLLING: SpinLock<()> = SpinLock::new(());
/// Offsets in the host-visible region, once there is one.
static WINDOWS: SpinLock<Option<Ranges>> = SpinLock::new(None);

/// The largest piece of a command stream (one DMA buffer).
const PIECE: usize = 4 << 20;
/// How long a waiter waits for one fence before giving up on it.
const FENCE_TIMEOUT_NS: u64 = 60 * time::NANOS_PER_SEC;

fn gpu<R>(f: impl FnOnce(&mut virtio_gpu::Driver) -> Result<R, poly_virtio_gpu::Error>) -> Result<R, HwError> {
    match display::with_gpu(f) {
        Some(Ok(r)) => Ok(r),
        Some(Err(e)) => {
            kprintln!("drm: virtio-gpu 3D command failed: {e:?}");
            Err(HwError)
        }
        None => Err(HwError),
    }
}

/// What the device offers for 3D, its capsets listed.
pub fn caps() -> GpuCaps {
    display::with_gpu(|g| {
        if !g.has_virgl() {
            return GpuCaps::default();
        }
        let capsets = (0..g.config().num_capsets)
            .filter_map(|i| g.capset_info(i).ok())
            .map(|c| CapsetInfo { id: c.id, max_version: c.max_version, max_size: c.max_size })
            .collect();
        GpuCaps {
            virgl: true,
            blob: g.has_blob(),
            host_visible: g.has_blob() && virtio_gpu::hostmem().is_some(),
            context_init: g.has_context_init(),
            capsets,
        }
    })
    .unwrap_or_default()
}

/// Start the 3D side: connect the interrupt, start the fence thread, set up
/// the host-visible region's allocator. Call once DRM has its device.
pub fn init(caps: &GpuCaps) {
    if !caps.virgl {
        return;
    }
    if let Some(h) = virtio_gpu::hostmem() {
        *WINDOWS.lock() = Some(Ranges::new(h.len));
    }
    let interrupts = display::with_gpu(|g| g.interrupts()).unwrap_or(false)
        && match virtio_gpu::connect_interrupt(on_interrupt, 0) {
            Ok(v) => {
                kprintln!("drm: virtio-gpu fences complete on vector {v:#x} (MSI-X)");
                true
            }
            Err(e) => {
                kprintln!("drm: virtio-gpu interrupt not connected ({e:?}); fences are polled");
                false
            }
        };
    INTERRUPTS.store(interrupts, SeqCst);
    if let Err(e) = thread::spawn_named("drm-fence", fence_thread, 0) {
        kprintln!("drm: no fence thread ({e:?}); fences complete only when waited for");
    }
    let names: Vec<alloc::string::String> = caps
        .capsets
        .iter()
        .map(|c| alloc::format!("{} v{} ({} bytes)", capset_name(c.id), c.max_version, c.max_size))
        .collect();
    kprintln!(
        "drm: virtio-gpu 3D: capsets {}{}{}{}",
        names.join(", "),
        if caps.blob { ", blobs" } else { "" },
        if caps.host_visible { ", host-visible memory" } else { "" },
        if caps.context_init { ", context init" } else { "" },
    );
}

/// The name of a capset ID, for logs.
pub fn capset_name(id: u32) -> &'static str {
    use poly_virtio_gpu::protocol::capset;
    match id {
        capset::VIRGL => "virgl",
        capset::VIRGL2 => "virgl2",
        capset::GFXSTREAM_VULKAN => "gfxstream-vulkan",
        capset::VENUS => "venus",
        capset::CROSS_DOMAIN => "cross-domain",
        capset::DRM => "drm",
        _ => "unknown",
    }
}

fn on_interrupt(_: usize) {
    IRQS.fetch_add(1, SeqCst);
    WAKE.wake_all();
}

/// The data of capset `id` at `version`, `max_size` bytes at most.
pub fn capset(id: u32, version: u32, max_size: u32) -> Result<Vec<u8>, HwError> {
    let len = poly_virtio_gpu::protocol::HEADER_LEN + max_size as usize;
    let buf = DmaBuffer::new(len).ok_or(HwError)?;
    let out = DmaRegion { ptr: buf.ptr(), bus: buf.phys(), len };
    // SAFETY: the buffer is ours, device-reachable at its physical address,
    // and lives until the call returns; if the driver gave up on the device
    // meanwhile it may still write it, so it is leaked then (below).
    let r = display::with_gpu(|g| unsafe { g.capset(id, version, out) });
    match r {
        Some(Ok(n)) => {
            // SAFETY: the device wrote `n` bytes after the header, inside
            // the buffer, and is done with it (the request was answered).
            let data = unsafe { core::slice::from_raw_parts(buf.ptr().as_ptr().add(len - max_size as usize), n) };
            Ok(data.to_vec())
        }
        Some(Err(e)) => {
            kprintln!("drm: GET_CAPSET {id} v{version} failed: {e:?}");
            if display::with_gpu(|g| g.is_broken()).unwrap_or(true) {
                core::mem::forget(buf);
            }
            Err(HwError)
        }
        None => Err(HwError),
    }
}

/// Track `ticket` (and the memory its request needs) until it completes;
/// its fence ID.
fn track(ticket: Ticket, data: Vec<DmaBuffer>) -> u64 {
    track_keeping(ticket, data, None)
}

fn track_keeping(ticket: Ticket, data: Vec<DmaBuffer>, keep: Option<Keep>) -> u64 {
    FENCES.lock().insert(ticket.fence, InFlight { ticket, _data: data, _keep: keep });
    IN_FLIGHT.fetch_add(1, SeqCst);
    WAKE.wake_all();
    ticket.fence
}

/// Send a request through `send` (which gets the driver), waiting for a
/// free slot while every one is taken.
fn send_waiting(
    mut send: impl FnMut(&mut virtio_gpu::Driver) -> Result<Ticket, poly_virtio_gpu::Error>,
) -> Result<Ticket, HwError> {
    let started = time::now_ns();
    loop {
        match display::with_gpu(&mut send) {
            Some(Ok(t)) => return Ok(t),
            Some(Err(poly_virtio_gpu::Error::Busy)) if time::now_ns() - started < FENCE_TIMEOUT_NS => {
                poll_all();
                let seen = COMPLETED.load(SeqCst);
                DONE.wait_until_deadline(|| COMPLETED.load(SeqCst) != seen, time::now_ns() + time::NANOS_PER_MILLI);
            }
            Some(Err(e)) => {
                kprintln!("drm: virtio-gpu 3D request failed: {e:?}");
                return Err(HwError);
            }
            None => return Err(HwError),
        }
    }
}

/// Run `commands` in context `ctx`, fenced on `ring` if given: its fence.
pub fn submit(ctx: u32, ring: Option<u32>, commands: &[u8]) -> Result<u64, HwError> {
    let size = u32::try_from(commands.len()).map_err(|_| HwError)?;
    let ring = ring.map(|r| u8::try_from(r).map_err(|_| HwError)).transpose()?;
    let mut data = Vec::new();
    let mut pieces = [Buffer { addr: 0, len: 0 }; poly_virtio_gpu::MAX_DATA_BUFFERS];
    for (i, chunk) in commands.chunks(PIECE).enumerate() {
        let piece = pieces.get_mut(i).ok_or(HwError)?;
        let buf = DmaBuffer::new(chunk.len()).ok_or(HwError)?;
        // SAFETY: the buffer is fresh, ours alone and at least `chunk.len()`
        // bytes long; the device reads it only after the request goes out.
        unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), buf.ptr().as_ptr(), chunk.len()) };
        *piece = Buffer { addr: buf.phys(), len: chunk.len() as u32 };
        data.push(buf);
    }
    let used = &pieces[..data.len()];
    // SAFETY: the pieces stay allocated until the ticket completes: they go
    // into the fence table with it (and are leaked if the driver gives up).
    let ticket = send_waiting(|g| unsafe { g.submit_3d(ctx, ring, size, used) })?;
    Ok(track(ticket, data))
}

/// A 3D transfer in context `ctx`: its fence.
pub fn transfer(to_host: bool, ctx: u32, t: Transfer3d) -> Result<u64, HwError> {
    let ticket = send_waiting(|g| g.send_transfer_3d(to_host, ctx, None, t))?;
    Ok(track(ticket, Vec::new()))
}

/// Collect what the device answered; whether any fence completed.
fn poll_all() -> bool {
    let (completed, finished) = {
        // One collector at a time: a ticket is collected once (taken from
        // the driver and out of the table together).
        let _one = POLLING.lock();
        let tickets: Vec<(u64, Ticket)> = FENCES.lock().iter().map(|(&f, i)| (f, i.ticket)).collect();
        if tickets.is_empty() {
            return false;
        }
        let done: Vec<(u64, Result<(), poly_virtio_gpu::Error>)> =
            display::with_gpu(|g| tickets.iter().filter_map(|&(f, t)| g.poll(t).map(|r| (f, r))).collect())
                .unwrap_or_default();
        let mut finished = Vec::new();
        let mut map = FENCES.lock();
        for (f, r) in &done {
            let Some(entry) = map.remove(f) else { continue };
            IN_FLIGHT.fetch_sub(1, SeqCst);
            match r {
                Ok(()) => finished.push(entry),
                Err(e) => {
                    kprintln!("drm: virtio-gpu fence {f} completed with an error: {e:?}");
                    if matches!(
                        e,
                        poly_virtio_gpu::Error::Broken
                            | poly_virtio_gpu::Error::Queue(_)
                            | poly_virtio_gpu::Error::Protocol
                            | poly_virtio_gpu::Error::Timeout { .. }
                    ) {
                        // The device may still read the data: never reuse it.
                        core::mem::forget(entry);
                    } else {
                        finished.push(entry);
                    }
                }
            }
        }
        (done.len(), finished)
    };
    if completed == 0 {
        return false;
    }
    // The memory goes back, and waiters wake, outside the locks.
    drop(finished);
    COMPLETED.fetch_add(completed as u64, SeqCst);
    DONE.wake_all();
    notify_files();
    true
}

/// Whether `fence` has signalled (collecting answers first if it might).
pub fn fence_done(fence: u64) -> bool {
    if !FENCES.lock().contains_key(&fence) {
        return true;
    }
    poll_all();
    !FENCES.lock().contains_key(&fence)
}

/// Sleep until `fence` signals (or a minute passes: the host is gone).
pub fn wait_fence(fence: u64) {
    let started = time::now_ns();
    while !fence_done(fence) {
        let now = time::now_ns();
        if now - started > FENCE_TIMEOUT_NS {
            kprintln!("drm: virtio-gpu fence {fence} did not signal within a minute");
            return;
        }
        let seen = COMPLETED.load(SeqCst);
        // Polled without interrupts; with them, a slow tick guards against a
        // lost one.
        let tick = if INTERRUPTS.load(SeqCst) { 20 } else { 1 } * time::NANOS_PER_MILLI;
        DONE.wait_until_deadline(|| COMPLETED.load(SeqCst) != seen, now + tick);
    }
}

/// Wake the pollers of every sync file.
fn notify_files() {
    let queues: Vec<Arc<poly_vfs::WaitQueue>> = {
        let mut w = FILE_WAITERS.lock();
        w.retain(|q| q.strong_count() > 0);
        w.iter().filter_map(Weak::upgrade).collect()
    };
    for q in queues {
        q.notify(PollEvents::READABLE);
    }
}

/// Collect answers when the device interrupts (or every millisecond while
/// fences are out, without interrupts).
fn fence_thread(_: usize) -> usize {
    loop {
        let seen = IRQS.load(SeqCst);
        if IN_FLIGHT.load(SeqCst) == 0 {
            WAKE.wait_until(|| IN_FLIGHT.load(SeqCst) != 0 || IRQS.load(SeqCst) != seen);
        } else {
            let tick = if INTERRUPTS.load(SeqCst) { 50 } else { 1 } * time::NANOS_PER_MILLI;
            WAKE.wait_until_deadline(|| IRQS.load(SeqCst) != seen, time::now_ns() + tick);
        }
        poll_all();
    }
}

// --- The host-visible region --------------------------------------------------

/// A window of `size` bytes in the host-visible region for resource `res`,
/// mapped there (`RESOURCE_MAP_BLOB`): its bus address and offset.
pub fn map_window(res: u32, size: u64) -> Result<(u64, u64), HwError> {
    let base = virtio_gpu::hostmem().ok_or(HwError)?.phys;
    let offset = WINDOWS.lock().as_mut().and_then(|w| w.alloc(size)).ok_or_else(|| {
        kprintln!("drm: no room for a {size}-byte blob in the host-visible region");
        HwError
    })?;
    match gpu(|g| g.map_blob(res, offset)) {
        Ok(_cache) => Ok((base + offset, offset)),
        Err(e) => {
            release_window(offset);
            Err(e)
        }
    }
}

/// Destroy blob `res` without waiting: unmap it from the host-visible
/// region if `mapped` (`RESOURCE_UNMAP_BLOB`), then `RESOURCE_UNREF`;
/// `keep` (its memory: guest pages, or the window, which goes back to the
/// region when the memory is freed) lives until the device answered the
/// unref, which it handles after the unmap. The unmap goes without a fence:
/// QEMU 9 answers it only once its memory map changed and never signals a
/// fence on it; nobody spins on it.
pub fn retire_blob(res: u32, mapped: bool, keep: Option<Keep>) -> Result<(), HwError> {
    use poly_virtio_gpu::protocol::Command;
    if mapped {
        let ticket = send_waiting(|g| g.send_unfenced(&Command::ResourceUnmapBlob { resource_id: res }))?;
        track(ticket, Vec::new());
    }
    // SAFETY: no data buffers and no external response.
    let ticket = send_waiting(|g| unsafe { g.send(&Command::ResourceUnref { resource_id: res }, None, &[], None) })?;
    track_keeping(ticket, Vec::new(), keep);
    Ok(())
}

/// Give a window back once nothing maps it any more.
pub fn release_window(offset: u64) {
    if let Some(w) = WINDOWS.lock().as_mut() {
        w.free(offset);
    }
}

// --- Sync files -------------------------------------------------------------------

/// `SYNC_IOC_MERGE` and `SYNC_IOC_FILE_INFO` (linux/sync_file.h: `_IOWR('>',
/// 3, struct sync_merge_data)` of 48 bytes and `_IOWR('>', 4, struct
/// sync_file_info)` of 56).
const SYNC_IOC_MERGE: u32 = 0xc030_3e03;
const SYNC_IOC_FILE_INFO: u32 = 0xc038_3e04;
/// `struct sync_fence_info`: `obj_name[32]`, `driver_name[32]`, `status`,
/// `flags`, `timestamp_ns`.
const SYNC_FENCE_INFO_LEN: usize = 80;

/// A sync file: signalled once every fence in it is.
pub struct SyncFile {
    fences: Vec<u64>,
    wait: Arc<poly_vfs::WaitQueue>,
}

impl core::fmt::Debug for SyncFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SyncFile({:?})", self.fences)
    }
}

impl SyncFile {
    fn signalled(&self) -> bool {
        self.fences.iter().all(|&f| fence_done(f))
    }
}

/// Make a sync file for `fences`, in the calling process's table
/// (close-on-exec): its descriptor.
pub fn export_fence(fences: &[u64]) -> Result<i32, Errno> {
    let process = crate::user::current_process().ok_or(Errno::EBADF)?;
    let files = &process.files;
    let node = fs::vfs().anon_node(FileType::Regular, 0o400, &files.cred())?;
    let mut list = fences.to_vec();
    list.sort_unstable();
    list.dedup();
    let wait = Arc::new(poly_vfs::WaitQueue::new());
    FILE_WAITERS.lock().push(Arc::downgrade(&wait));
    let object: Arc<dyn FileObject> = Arc::new(SyncFile { fences: list, wait });
    files.install(Arc::new(OpenFile::on_object(node, object, OpenFlags::RDONLY)), true)
}

/// The fences of sync-file descriptor `fd`.
pub fn import_fence(fd: i32) -> Result<Vec<u64>, Errno> {
    let process = crate::user::current_process().ok_or(Errno::EBADF)?;
    let file = process.files.file(fd).map_err(|_| Errno::EBADF)?;
    file.object_as::<SyncFile>().map(|s| s.fences.clone()).ok_or(Errno::EINVAL)
}

impl FileObject for SyncFile {
    fn read(&self, _: &mut [u8]) -> poly_vfs::Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: &[u8]) -> poly_vfs::Result<usize> {
        Err(Errno::EINVAL)
    }

    /// Readable once signalled (what `sync_wait`'s `poll` waits for).
    fn poll(&self) -> PollEvents {
        if self.signalled() { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<poly_vfs::WaitQueue>> {
        Some(&self.wait)
    }

    fn ioctl(&self, request: u32, arg: u64) -> poly_vfs::Result<u64> {
        let mut b = [0u8; 56];
        match request {
            SYNC_IOC_MERGE => {
                crate::user::copy_from_user(&mut b[..48], arg).map_err(|_| Errno::EFAULT)?;
                let fd2 = i32::from_le_bytes(b[32..36].try_into().unwrap_or_default());
                if u32::from_le_bytes(b[40..44].try_into().unwrap_or_default()) != 0 {
                    return Err(Errno::EINVAL);
                }
                let mut fences = self.fences.clone();
                fences.extend(import_fence(fd2)?);
                let fd = export_fence(&fences)?;
                b[36..40].copy_from_slice(&fd.to_le_bytes());
                crate::user::copy_to_user(arg, &b[..48]).map_err(|_| Errno::EFAULT)?;
                Ok(0)
            }
            SYNC_IOC_FILE_INFO => {
                crate::user::copy_from_user(&mut b, arg).map_err(|_| Errno::EFAULT)?;
                let flags = u32::from_le_bytes(b[36..40].try_into().unwrap_or_default());
                let asked = u32::from_le_bytes(b[40..44].try_into().unwrap_or_default());
                let info = u64::from_le_bytes(b[48..56].try_into().unwrap_or_default());
                if flags != 0 {
                    return Err(Errno::EINVAL);
                }
                b[..32].fill(0);
                b[..10].copy_from_slice(b"virtio_gpu");
                let status = self.signalled() as i32;
                b[32..36].copy_from_slice(&status.to_le_bytes());
                if asked != 0 {
                    if asked < self.fences.len() as u32 {
                        return Err(Errno::EINVAL);
                    }
                    for (i, &f) in self.fences.iter().enumerate() {
                        let mut e = [0u8; SYNC_FENCE_INFO_LEN];
                        e[..10].copy_from_slice(b"virtio_gpu");
                        e[32..42].copy_from_slice(b"virtio_gpu");
                        e[64..68].copy_from_slice(&(fence_done(f) as i32).to_le_bytes());
                        let at = info + (i * SYNC_FENCE_INFO_LEN) as u64;
                        crate::user::copy_to_user(at, &e).map_err(|_| Errno::EFAULT)?;
                    }
                }
                b[40..44].copy_from_slice(&(self.fences.len() as u32).to_le_bytes());
                crate::user::copy_to_user(arg, &b).map_err(|_| Errno::EFAULT)?;
                Ok(0)
            }
            _ => Err(Errno::ENOTTY),
        }
    }
}

// --- Tests --------------------------------------------------------------------

use crate::kernel_test;

/// The boot flag that says the GPU has 3D, so the 3D test must run.
pub const TEST_VIRGL_FLAG: &str = "poly.test.virgl";

/// virgl's names for a 2D texture of B8G8R8A8 texels that can be sampled
/// (MIT-licensed `virgl_hw.h`: `VIRGL_FORMAT_B8G8R8A8_UNORM` = 1,
/// `VIRGL_BIND_SAMPLER_VIEW` = 1 << 3; Gallium's `p_defines.h`:
/// `PIPE_TEXTURE_2D` = 2).
const VIRGL_TEXTURE_2D: u32 = 2;
const VIRGL_FORMAT_B8G8R8A8_UNORM: u32 = 1;
const VIRGL_BIND_SAMPLER_VIEW: u32 = 1 << 3;

kernel_test! {
    /// On a virtio GPU with 3D: a context; a 64x64 texture whose guest
    /// memory goes to the host and comes back unchanged (two fenced 3D
    /// transfers, completed through the interrupt or the poller); an empty
    /// fenced submission; and everything destroyed again, without one error
    /// from the device.
    fn virgl_round_trip_through_the_host(env: &TestEnv) -> TestResult {
        let virgl = display::with_gpu(|g| g.has_virgl()).unwrap_or(false);
        if !virgl {
            if env.boot.has_flag(TEST_VIRGL_FLAG) {
                return Err("poly.test.virgl is set but the GPU has no 3D");
            }
            return Ok(());
        }
        const CTX: u32 = 0x7fff_0001;
        const RES: u32 = 0x0fff_1000;
        const SIDE: u32 = 64;
        let len = (SIDE * SIDE * 4) as usize;
        let errors = || display::with_gpu(|g| g.stats().errors).unwrap_or(u64::MAX);
        let before = errors();
        let backing = DmaBuffer::new(len).ok_or("no DMA memory for the texture")?;
        let c = poly_virtio_gpu::Create3d {
            resource_id: RES,
            target: VIRGL_TEXTURE_2D,
            format: VIRGL_FORMAT_B8G8R8A8_UNORM,
            bind: VIRGL_BIND_SAMPLER_VIEW,
            width: SIDE,
            height: SIDE,
            depth: 1,
            array_size: 1,
            ..poly_virtio_gpu::Create3d::default()
        };
        let entry = poly_virtio_gpu::MemEntry { addr: backing.phys(), length: len as u32 };
        gpu(|g| {
            g.ctx_create(CTX, 0, b"ktest")?;
            g.create_3d(&c)?;
            g.attach_backing(RES, &[entry])?;
            g.ctx_attach(CTX, RES)
        })
        .map_err(|_| "making the context and the texture failed")?;
        let pattern = |i: usize| (i * 7 + i / 256) as u8;
        for i in 0..len {
            backing.write(i, &[pattern(i)]);
        }
        let all = Transfer3d {
            resource_id: RES,
            region: poly_virtio_gpu::Box3d { x: 0, y: 0, z: 0, w: SIDE, h: SIDE, d: 1 },
            stride: SIDE * 4,
            ..Transfer3d::default()
        };
        let up = transfer(true, CTX, all).map_err(|_| "TRANSFER_TO_HOST_3D was refused")?;
        wait_fence(up);
        if !fence_done(up) {
            return Err("the upload's fence did not signal");
        }
        // SAFETY: the buffer is ours; the device is done with it (the
        // upload's fence signalled) until the next transfer.
        unsafe { core::ptr::write_bytes(backing.ptr().as_ptr(), 0, len) };
        let down = transfer(false, CTX, all).map_err(|_| "TRANSFER_FROM_HOST_3D was refused")?;
        wait_fence(down);
        // SAFETY: the download's fence signalled: the device wrote the
        // buffer and is done with it.
        let back = unsafe { core::slice::from_raw_parts(backing.ptr().as_ptr(), len) };
        let same = back.iter().enumerate().all(|(i, &b)| b == pattern(i));
        let submitted = submit(CTX, None, &[]).map(wait_fence);
        let cleaned = gpu(|g| {
            g.ctx_detach(CTX, RES)?;
            g.detach_backing(RES)?;
            g.unref(RES)?;
            g.ctx_destroy(CTX)
        });
        if !same {
            return Err("the texture did not come back from the host as it went");
        }
        if submitted.is_err() || cleaned.is_err() || errors() != before {
            return Err("the device refused a 3D command");
        }
        Ok(())
    }
}
