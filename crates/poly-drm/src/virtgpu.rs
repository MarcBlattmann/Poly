//! The virtio-gpu driver's ioctls (`DRM_IOCTL_VIRTGPU_*`, the MIT-licensed
//! `drm/virtgpu_drm.h`), which Mesa's virgl (OpenGL) and Venus (Vulkan)
//! drivers use on a device whose `DRM_IOCTL_VERSION` name is `virtio_gpu`
//! (ADR 0119).
//!
//! # Objects
//!
//! - **Contexts.** Each file has at most one 3D context, its ID the file's.
//!   `CONTEXT_INIT` makes it for a capset (virgl, Venus, ...) with a number
//!   of fence rings; otherwise the first ioctl that needs one makes a
//!   default (virgl) context, as Linux does. It goes when the file closes.
//! - **Resources.** `RESOURCE_CREATE` and `RESOURCE_CREATE_BLOB` make a GEM
//!   object whose hardware resource is [`RESOURCE_BASE`] + the object's ID:
//!   a 3D resource with zeroed guest memory of the requested size (a 2D
//!   resource on hardware without 3D), or a blob (guest memory for the
//!   `GUEST` kinds; a host blob, mapped into the host-visible region when
//!   mappable). The object is attached to a file's context when the file
//!   gets a handle for it while it has a context, and before a submission
//!   or transfer that uses it; it is detached from every context and
//!   destroyed with its last reference. `MAP` gives the same fake mmap
//!   offset as a dumb buffer; mapping it maps the guest memory (or the
//!   host-visible window).
//! - **Fences.** A submission (`EXECBUFFER`) or 3D transfer returns at once
//!   with a hardware fence, recorded on every object it uses. `WAIT` on an
//!   object sleeps ([`Reply::WaitFence`]) until its fences signal (`EBUSY`
//!   with `VIRTGPU_WAIT_NOWAIT`); `EXECBUFFER` with `FENCE_FD_OUT` hands out
//!   a sync file for its fence and with `FENCE_FD_IN` first waits for the
//!   one it is given. Sync objects are not offered (`DRM_CAP_SYNCOBJ` 0).
//!
//! # Not here
//!
//! Cross-device sharing (resource UUIDs), `POLL_RINGS_MASK` fence events
//! on the file (the mask is accepted and kept, nothing is queued), sync
//! objects in `EXECBUFFER`.

use crate::device::{BoId, Device, FileId};
use crate::uapi::virtgpu::{
    ContextInit, ContextSetParam, Execbuffer, GetCaps, GetParam, Map, ResourceCreate, ResourceCreateBlob, ResourceInfo,
    Transfer as UTransfer, WAIT_NOWAIT, Wait, blob, context_param, execbuf, nr, param,
};
use crate::{Create3d, Ctx, Errno, Hardware, Reply, ResourceKind, Result, Transfer, UserMemory};
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

/// A GEM object's hardware resource is this plus the object's ID (below:
/// the kernel's own resources, the console's and its framebuffers').
pub const RESOURCE_BASE: u32 = 0x0010_0000;
/// The longest command stream one `EXECBUFFER` (or blob command) carries.
const MAX_COMMANDS: u32 = 16 << 20;
/// The most buffers one `EXECBUFFER` lists.
const MAX_BO_HANDLES: u32 = 1 << 16;
/// The largest resource's guest memory.
const MAX_RESOURCE: u64 = 1 << 30;
/// Fence rings a context may have (Linux's `VIRTGPU_MAX_RINGS`... 64).
const MAX_RINGS: u64 = 64;
/// The longest context debug name (`virtio_gpu_ctx_create.debug_name`).
const MAX_NAME: usize = 64;
/// Parameters one `CONTEXT_INIT` may set.
const MAX_PARAMS: u32 = 16;
const PAGE: u64 = 4096;

/// The kernel's argument size of virtio-gpu request `n`.
pub(crate) fn arg_size(n: u8) -> Option<usize> {
    Some(match n {
        nr::MAP => Map::SIZE,
        nr::EXECBUFFER => Execbuffer::SIZE,
        nr::GETPARAM => GetParam::SIZE,
        nr::RESOURCE_CREATE => ResourceCreate::SIZE,
        nr::RESOURCE_INFO => ResourceInfo::SIZE,
        nr::TRANSFER_FROM_HOST | nr::TRANSFER_TO_HOST => UTransfer::SIZE,
        nr::WAIT => Wait::SIZE,
        nr::GET_CAPS => GetCaps::SIZE,
        nr::RESOURCE_CREATE_BLOB => ResourceCreateBlob::SIZE,
        nr::CONTEXT_INIT => ContextInit::SIZE,
        _ => return None,
    })
}

/// The hardware resource of a GEM object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuRes {
    pub id: u32,
    /// `VIRTGPU_BLOB_MEM_*`, 0 for a non-blob resource.
    pub blob_mem: u32,
    /// Fences of work that uses it and may not have signalled.
    pub fences: Vec<u64>,
}

/// A file's 3D context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GpuCtx {
    pub id: u32,
    pub capset: u32,
    pub num_rings: u32,
    pub poll_rings_mask: u64,
    /// Resources attached to it.
    pub attached: BTreeSet<u32>,
}

/// A file's 3D state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileGpu {
    pub ctx: Option<GpuCtx>,
}

fn hw<T>(r: core::result::Result<T, crate::HwError>) -> Result<T> {
    r.map_err(|_| Errno::EIO)
}

/// Read `len` bytes from the caller.
fn read_user(user: &mut dyn UserMemory, addr: u64, len: usize) -> Result<Vec<u8>> {
    let mut v = vec![0u8; len];
    if len > 0 {
        user.read(addr, &mut v)?;
    }
    Ok(v)
}

impl<H: Hardware> Device<H> {
    pub(crate) fn virtgpu_ioctl(
        &mut self,
        file: FileId,
        n: u8,
        b: &mut [u8],
        ctx: &mut Ctx<'_, H::Buffer>,
    ) -> Result<Reply> {
        let done = |r: Result<()>| r.map(|()| Reply::Done(0));
        match n {
            nr::GETPARAM => done(self.vg_getparam(b, ctx.user)),
            nr::GET_CAPS => done(self.vg_get_caps(b, ctx.user)),
            nr::CONTEXT_INIT => done(self.vg_context_init(file, b, ctx.user)),
            nr::RESOURCE_CREATE => done(self.vg_resource_create(file, b)),
            nr::RESOURCE_CREATE_BLOB => done(self.vg_resource_create_blob(file, b, ctx.user)),
            nr::RESOURCE_INFO => done(self.vg_resource_info(file, b)),
            nr::MAP => {
                let mut m = Map::decode(b);
                let bo = self.lookup(file, m.handle)?;
                if self.bos.get(&bo).is_none_or(|o| o.buffer.is_none()) {
                    return Err(Errno::EINVAL);
                }
                m.offset = (bo.0 as u64) << 32;
                m.encode(b);
                Ok(Reply::Done(0))
            }
            nr::TRANSFER_TO_HOST => done(self.vg_transfer(file, true, b)),
            nr::TRANSFER_FROM_HOST => done(self.vg_transfer(file, false, b)),
            nr::WAIT => self.vg_wait(file, b),
            nr::EXECBUFFER => self.vg_execbuffer(file, b, ctx),
            _ => Err(Errno::EINVAL),
        }
    }

    /// Capset IDs the hardware supports, a bit each.
    fn capset_mask(&self) -> u64 {
        self.gpu_caps.capsets.iter().filter(|c| c.id < 64).fold(0, |m, c| m | 1 << c.id)
    }

    /// `GETPARAM`: the value goes to the `int` that `value` points at.
    fn vg_getparam(&mut self, b: &[u8], user: &mut dyn UserMemory) -> Result<()> {
        let p = GetParam::decode(b);
        let c = &self.gpu_caps;
        let v: u64 = match p.param {
            param::THREE_D_FEATURES => c.virgl as u64,
            param::CAPSET_QUERY_FIX => 1,
            param::RESOURCE_BLOB => c.blob as u64,
            param::HOST_VISIBLE => (c.blob && c.host_visible) as u64,
            param::CROSS_DEVICE => 0,
            param::CONTEXT_INIT => c.context_init as u64,
            param::SUPPORTED_CAPSET_IDS => self.capset_mask(),
            param::EXPLICIT_DEBUG_NAME => 1,
            _ => return Err(Errno::EINVAL),
        };
        user.write(p.value, &(v as u32).to_le_bytes())
    }

    /// `GET_CAPS`: capset `cap_set_id` at `cap_set_ver`, at most `size`
    /// bytes of it to `addr`.
    fn vg_get_caps(&mut self, b: &[u8], user: &mut dyn UserMemory) -> Result<()> {
        let g = GetCaps::decode(b);
        if !self.gpu_caps.virgl || self.gpu_caps.capsets.is_empty() {
            return Err(Errno::ENOSYS);
        }
        let info = self.gpu_caps.capsets.iter().find(|c| c.id == g.cap_set_id).copied().ok_or(Errno::EINVAL)?;
        if g.cap_set_ver > info.max_version {
            return Err(Errno::EINVAL);
        }
        let key = (g.cap_set_id, g.cap_set_ver);
        if !self.capsets.contains_key(&key) {
            let data = hw(self.hw.capset(g.cap_set_id, g.cap_set_ver))?;
            self.capsets.insert(key, data);
        }
        let data = &self.capsets[&key];
        let n = (g.size as usize).min(data.len());
        if n > 0 {
            user.write(g.addr, &data[..n])?;
        }
        Ok(())
    }

    /// The file's 3D context, made (virgl, no rings) if it has none.
    fn gpu_ctx(&mut self, file: FileId) -> Result<u32> {
        if !self.gpu_caps.virgl {
            return Err(Errno::ENOSYS);
        }
        if let Some(c) = &self.file(file)?.gpu.ctx {
            return Ok(c.id);
        }
        hw(self.hw.ctx_create(file, 0, b"poly"))?;
        self.file_mut(file)?.gpu.ctx = Some(GpuCtx { id: file, ..GpuCtx::default() });
        Ok(file)
    }

    /// `CONTEXT_INIT`: the file's context, of a capset, with fence rings.
    fn vg_context_init(&mut self, file: FileId, b: &[u8], user: &mut dyn UserMemory) -> Result<()> {
        let ci = ContextInit::decode(b);
        if !self.gpu_caps.virgl || !self.gpu_caps.context_init {
            return Err(Errno::EINVAL);
        }
        if self.file(file)?.gpu.ctx.is_some() {
            return Err(Errno::EEXIST);
        }
        if ci.num_params > MAX_PARAMS {
            return Err(Errno::EINVAL);
        }
        let raw = read_user(user, ci.ctx_set_params, ci.num_params as usize * ContextSetParam::SIZE)?;
        let mut capset = None;
        let mut rings = None;
        let mut mask = None;
        let mut name: Vec<u8> = b"poly".to_vec();
        for chunk in raw.chunks_exact(ContextSetParam::SIZE) {
            let p = ContextSetParam::decode(chunk);
            match p.param {
                context_param::CAPSET_ID if capset.is_none() => {
                    if p.value >= 64 || self.capset_mask() & 1 << p.value == 0 {
                        return Err(Errno::EINVAL);
                    }
                    capset = Some(p.value as u32);
                }
                context_param::NUM_RINGS if rings.is_none() => {
                    if p.value > MAX_RINGS {
                        return Err(Errno::EINVAL);
                    }
                    rings = Some(p.value as u32);
                }
                context_param::POLL_RINGS_MASK if mask.is_none() => mask = Some(p.value),
                context_param::DEBUG_NAME => {
                    name.clear();
                    let mut c = [0u8; 1];
                    while name.len() < MAX_NAME {
                        user.read(p.value + name.len() as u64, &mut c)?;
                        if c[0] == 0 {
                            break;
                        }
                        name.push(c[0]);
                    }
                }
                _ => return Err(Errno::EINVAL),
            }
        }
        let num_rings = rings.unwrap_or(0);
        let poll_rings_mask = mask.unwrap_or(0);
        if num_rings < 64 && poll_rings_mask >> num_rings != 0 {
            return Err(Errno::EINVAL);
        }
        let capset = capset.unwrap_or(0);
        hw(self.hw.ctx_create(file, capset, &name))?;
        self.file_mut(file)?.gpu.ctx =
            Some(GpuCtx { id: file, capset, num_rings, poll_rings_mask, attached: BTreeSet::new() });
        Ok(())
    }

    /// The hardware resource of `bo`.
    fn gpu_res(&self, bo: BoId) -> Result<u32> {
        self.bos.get(&bo).and_then(|o| o.gpu.as_ref()).map(|g| g.id).ok_or(Errno::EINVAL)
    }

    /// Attach `bo`'s resource to the file's context `ctx` unless it is.
    fn gpu_attach(&mut self, file: FileId, ctx: u32, bo: BoId) -> Result<()> {
        let res = self.gpu_res(bo)?;
        let attached = self.file(file)?.gpu.ctx.as_ref().is_some_and(|c| c.attached.contains(&res));
        if !attached {
            hw(self.hw.ctx_attach(ctx, res))?;
            if let Some(c) = self.file_mut(file)?.gpu.ctx.as_mut() {
                c.attached.insert(res);
            }
        }
        Ok(())
    }

    /// Give `file` a handle for the new resource object `bo` (attached to
    /// the file's context if it has one). On failure the object goes.
    fn gpu_handle(&mut self, file: FileId, bo: BoId) -> Result<u32> {
        let handle = self.handle_for(file, bo)?;
        if let Some(ctx) = self.file(file)?.gpu.ctx.as_ref().map(|c| c.id)
            && let Err(e) = self.gpu_attach(file, ctx, bo)
        {
            if let Ok(f) = self.file_mut(file) {
                f.handles.remove(&handle);
            }
            self.unref(bo);
            return Err(e);
        }
        Ok(handle)
    }

    /// The last reference to a resource object went: detach it from every
    /// context and destroy it.
    pub(crate) fn destroy_gpu_resource(&mut self, res: &GpuRes, memory: Option<&H::Buffer>) {
        let id = res.id;
        let users: Vec<u32> = self
            .files
            .values_mut()
            .filter_map(|f| f.gpu.ctx.as_mut())
            .filter_map(|c| c.attached.remove(&id).then_some(c.id))
            .collect();
        for ctx in users {
            self.hw.ctx_detach(ctx, id);
        }
        self.hw.destroy_resource(id, memory);
    }

    /// Whole pages of guest memory for a resource of `size` bytes (at least
    /// one page).
    fn resource_pages(size: u64) -> Result<u64> {
        let pages = size.max(1).next_multiple_of(PAGE);
        if pages > MAX_RESOURCE { Err(Errno::ENOMEM) } else { Ok(pages) }
    }

    /// `RESOURCE_CREATE`: a 3D resource with guest memory of `size` bytes
    /// (a 2D one on hardware without 3D).
    fn vg_resource_create(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut rc = ResourceCreate::decode(b);
        if rc.width == 0 || rc.height == 0 {
            return Err(Errno::EINVAL);
        }
        let pages = Self::resource_pages(rc.size as u64)?;
        let (kind, ctx) = if self.gpu_caps.virgl {
            let c = Create3d {
                target: rc.target,
                format: rc.format,
                bind: rc.bind,
                width: rc.width,
                height: rc.height,
                depth: rc.depth,
                array_size: rc.array_size,
                last_level: rc.last_level,
                nr_samples: rc.nr_samples,
                flags: rc.flags,
            };
            (ResourceKind::ThreeD(c), Some(self.gpu_ctx(file)?))
        } else {
            (ResourceKind::Simple { format: rc.format, width: rc.width, height: rc.height }, None)
        };
        let buffer = self.hw.alloc(pages).ok_or(Errno::ENOMEM)?;
        let res = RESOURCE_BASE + self.next_bo_id();
        hw(self.hw.create_resource(res, ctx, &kind, Some(&buffer)))?;
        let bo = self.new_bo(pages, Some(buffer), Some(GpuRes { id: res, blob_mem: 0, fences: Vec::new() }));
        rc.bo_handle = self.gpu_handle(file, bo)?;
        rc.res_handle = res;
        rc.encode(b);
        Ok(())
    }

    /// `RESOURCE_CREATE_BLOB`.
    fn vg_resource_create_blob(&mut self, file: FileId, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut rb = ResourceCreateBlob::decode(b);
        let caps = &self.gpu_caps;
        if !caps.blob {
            return Err(Errno::EINVAL);
        }
        let host3d = match rb.blob_mem {
            blob::MEM_GUEST => false,
            blob::MEM_HOST3D | blob::MEM_HOST3D_GUEST => true,
            _ => return Err(Errno::EINVAL),
        };
        let mappable = rb.blob_flags & blob::FLAG_USE_MAPPABLE != 0;
        if rb.blob_flags & !blob::FLAGS != 0 || rb.size == 0 || rb.cmd_size > MAX_COMMANDS {
            return Err(Errno::EINVAL);
        }
        if (host3d || rb.cmd_size != 0) && !caps.virgl {
            return Err(Errno::EINVAL);
        }
        if host3d && mappable && !caps.host_visible {
            return Err(Errno::EINVAL);
        }
        let pages = Self::resource_pages(rb.size)?;
        let ctx = if host3d || rb.cmd_size != 0 { Some(self.gpu_ctx(file)?) } else { None };
        // The commands that make the host memory go first.
        if rb.cmd_size != 0 {
            let cmds = read_user(user, rb.cmd, rb.cmd_size as usize)?;
            let fence = hw(self.hw.submit(ctx.unwrap_or(file), None, &cmds))?;
            let _ = fence;
        }
        let backing =
            if rb.blob_mem == blob::MEM_HOST3D { None } else { Some(self.hw.alloc(pages).ok_or(Errno::ENOMEM)?) };
        let res = RESOURCE_BASE + self.next_bo_id();
        let kind = ResourceKind::Blob { mem: rb.blob_mem, flags: rb.blob_flags, blob_id: rb.blob_id, size: rb.size };
        let mapping = hw(self.hw.create_resource(res, if host3d { ctx } else { None }, &kind, backing.as_ref()))?;
        let memory = backing.or(mapping);
        let bo = self.new_bo(pages, memory, Some(GpuRes { id: res, blob_mem: rb.blob_mem, fences: Vec::new() }));
        rb.bo_handle = self.gpu_handle(file, bo)?;
        rb.res_handle = res;
        rb.encode(b);
        Ok(())
    }

    /// `RESOURCE_INFO`: the resource, size and blob kind of a buffer (a dumb
    /// buffer has no resource of its own: 0).
    fn vg_resource_info(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut ri = ResourceInfo::decode(b);
        let bo = self.lookup(file, ri.bo_handle)?;
        let o = self.bos.get(&bo).ok_or(Errno::ENOENT)?;
        ri.res_handle = o.gpu.as_ref().map_or(0, |g| g.id);
        ri.blob_mem = o.gpu.as_ref().map_or(0, |g| g.blob_mem);
        ri.size = o.size.min(u32::MAX as u64) as u32;
        ri.encode(b);
        Ok(())
    }

    /// Note that `fence` uses `bo` (forgetting fences that have signalled).
    fn add_fence(&mut self, bo: BoId, fence: u64) {
        if fence == 0 {
            return;
        }
        let Some(g) = self.bos.get_mut(&bo).and_then(|o| o.gpu.as_mut()) else { return };
        let mut fences = core::mem::take(&mut g.fences);
        fences.retain(|&f| !self.hw.fence_done(f));
        fences.push(fence);
        if let Some(g) = self.bos.get_mut(&bo).and_then(|o| o.gpu.as_mut()) {
            g.fences = fences;
        }
    }

    /// The first fence of `bo` that has not signalled.
    fn busy_fence(&mut self, bo: BoId) -> Option<u64> {
        let g = self.bos.get_mut(&bo).and_then(|o| o.gpu.as_mut())?;
        let mut fences = core::mem::take(&mut g.fences);
        fences.retain(|&f| !self.hw.fence_done(f));
        let first = fences.first().copied();
        if let Some(g) = self.bos.get_mut(&bo).and_then(|o| o.gpu.as_mut()) {
            g.fences = fences;
        }
        first
    }

    /// `TRANSFER_TO_HOST` / `TRANSFER_FROM_HOST`.
    fn vg_transfer(&mut self, file: FileId, to_host: bool, b: &[u8]) -> Result<()> {
        let t = UTransfer::decode(b);
        let bo = self.lookup(file, t.bo_handle)?;
        let res = self.gpu_res(bo)?;
        let region = Transfer {
            x: t.x,
            y: t.y,
            z: t.z,
            w: t.w,
            h: t.h,
            d: t.d,
            level: t.level,
            offset: t.offset as u64,
            stride: t.stride,
            layer_stride: t.layer_stride,
        };
        let fence = if self.gpu_caps.virgl {
            let ctx = self.gpu_ctx(file)?;
            self.gpu_attach(file, ctx, bo)?;
            hw(self.hw.transfer(to_host, Some(ctx), res, &region))?
        } else if to_host {
            hw(self.hw.transfer(true, None, res, &region))?
        } else {
            return Err(Errno::ENOSYS);
        };
        self.add_fence(bo, fence);
        Ok(())
    }

    /// `WAIT`: until the work using a buffer is done.
    fn vg_wait(&mut self, file: FileId, b: &[u8]) -> Result<Reply> {
        let w = Wait::decode(b);
        let bo = self.lookup(file, w.handle)?;
        match self.busy_fence(bo) {
            None => Ok(Reply::Done(0)),
            Some(_) if w.flags & WAIT_NOWAIT != 0 => Err(Errno::EBUSY),
            Some(fence) => Ok(Reply::WaitFence { fence }),
        }
    }

    /// `EXECBUFFER`: run a command stream in the file's context.
    fn vg_execbuffer(&mut self, file: FileId, b: &mut [u8], ctx: &mut Ctx<'_, H::Buffer>) -> Result<Reply> {
        let mut e = Execbuffer::decode(b);
        if e.flags & !execbuf::FLAGS != 0 || e.num_in_syncobjs != 0 || e.num_out_syncobjs != 0 {
            return Err(Errno::EINVAL);
        }
        if e.size > MAX_COMMANDS || e.num_bo_handles > MAX_BO_HANDLES {
            return Err(Errno::EINVAL);
        }
        let gctx = self.gpu_ctx(file)?;
        let ring = if e.flags & execbuf::RING_IDX != 0 {
            let rings = self.file(file)?.gpu.ctx.as_ref().map_or(0, |c| c.num_rings);
            if e.ring_idx >= rings {
                return Err(Errno::EINVAL);
            }
            Some(e.ring_idx)
        } else {
            None
        };
        if e.flags & execbuf::FENCE_FD_IN != 0 {
            let fences = ctx.fds.import_fence(e.fence_fd)?;
            if let Some(&fence) = fences.iter().find(|&&f| !self.hw.fence_done(f)) {
                return Ok(Reply::WaitFence { fence });
            }
        }
        let raw = read_user(ctx.user, e.bo_handles, e.num_bo_handles as usize * 4)?;
        let mut bos = Vec::with_capacity(raw.len() / 4);
        for h in raw.chunks_exact(4) {
            let bo = self.lookup(file, u32::from_le_bytes([h[0], h[1], h[2], h[3]]))?;
            self.gpu_res(bo)?;
            bos.push(bo);
        }
        let cmds = read_user(ctx.user, e.command, e.size as usize)?;
        for &bo in &bos {
            self.gpu_attach(file, gctx, bo)?;
        }
        let fence = hw(self.hw.submit(gctx, ring, &cmds))?;
        for &bo in &bos {
            self.add_fence(bo, fence);
        }
        if e.flags & execbuf::FENCE_FD_OUT != 0 {
            e.fence_fd = ctx.fds.export_fence(&[fence])?;
            e.encode(b);
        }
        Ok(Reply::Done(0))
    }
}
