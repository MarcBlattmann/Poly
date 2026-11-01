//! A Linux DRM/KMS device: what `/dev/dri/card0` and `/dev/dri/renderD128`
//! answer, without the hardware.
//!
//! Userspace (libdrm, Mesa's GBM and EGL platforms, Wayland compositors,
//! `modetest`) talks to a DRM device through ioctls on its device nodes,
//! `read(2)` for events and `mmap(2)` for buffers. This crate is that
//! interface as the public headers (`drm/drm.h`, `drm/drm_mode.h`,
//! `drm/drm_fourcc.h`, MIT) and the kernel's DRM documentation
//! ("Userland interfaces", "KMS Properties") describe it, on top of a
//! [`Hardware`] trait the kernel implements (Poly's: a virtio GPU).
//!
//! ```text
//!   ioctl(fd, DRM_IOCTL_*, arg) ──▶ Device::ioctl ──▶ uapi decode (min(user, kernel) size, zero-extended)
//!                                       │               permission: render node, DRM master
//!                                       ▼
//!   files: caps, GEM handles, events   KMS objects: CRTC i ── encoder i ── connector i, primary plane i
//!   GEM objects (dumb buffers) ───────▶ framebuffers ──▶ State (atomic) ──commit──▶ Hardware
//!   read(fd) ◀── vblank / flip events ◀── per-CRTC vblank clock (mode refresh)      upload / scanout / flush
//! ```
//!
//! - [`uapi`]: request numbers, structure layouts and constants.
//! - [`modes`]: modes from EDID detailed timings and VESA CVT, validation.
//! - [`Device`]: the object model (one CRTC, encoder, connector and primary
//!   plane per hardware output, all with IDs from one space; properties
//!   and property blobs), GEM objects with per-file handles, dumb buffers
//!   and their mmap offsets, framebuffers (`ADDFB`, `ADDFB2` with linear
//!   modifiers), legacy modesetting (`SETCRTC`, `SETPLANE`, `PAGE_FLIP`,
//!   `DIRTYFB`) and atomic modesetting (`MODE_ATOMIC`), which share one
//!   state and one commit path; vblank counters and the event queue
//!   (`WAIT_VBLANK`, flip-complete events, `CRTC_*_SEQUENCE`); DRM master
//!   (`SET_MASTER`, `DROP_MASTER`, the first opener) and the render-node
//!   subset; PRIME export and import through the kernel's file table
//!   ([`Fds`]).
//!
//! # Time
//!
//! Nothing here reads a clock or sleeps: every entry point takes "now" (ns
//! of CLOCK_MONOTONIC), events carry the time they become readable, and a
//! blocking `WAIT_VBLANK` (or a blocking atomic commit behind a pending
//! one) returns [`Reply::Sleep`], after which the kernel issues the same
//! ioctl again (the request has been rewritten to an absolute one, as
//! Linux does for restarted vblank waits).
//!
//! # The console
//!
//! The device takes the display from the kernel console
//! ([`Hardware::take_display`]) when it first shows a framebuffer, and gives
//! it back ([`Hardware::give_back_display`]) when the DRM master closes its
//! file (or the last file closes), after turning off every output it
//! drove. See ADR 0108.

#![no_std]

extern crate alloc;

mod device;
mod kms;
pub mod modes;
mod props;
pub mod uapi;

pub use device::{BoId, Device, DriverInfo, FileId, Mapping, Node};
pub use props::Prop;

use alloc::vec::Vec;

/// A Linux error number (asm-generic/errno-base.h, errno.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub const EPERM: Errno = Errno(1);
    pub const ENOENT: Errno = Errno(2);
    pub const EINTR: Errno = Errno(4);
    pub const EIO: Errno = Errno(5);
    pub const ENXIO: Errno = Errno(6);
    pub const EBADF: Errno = Errno(9);
    pub const EAGAIN: Errno = Errno(11);
    pub const ENOMEM: Errno = Errno(12);
    pub const EACCES: Errno = Errno(13);
    pub const EFAULT: Errno = Errno(14);
    pub const EBUSY: Errno = Errno(16);
    pub const ENODEV: Errno = Errno(19);
    pub const EINVAL: Errno = Errno(22);
    pub const ENOTTY: Errno = Errno(25);
    pub const ENOSPC: Errno = Errno(28);
    pub const ERANGE: Errno = Errno(34);
    pub const EOPNOTSUPP: Errno = Errno(95);
}

pub type Result<T, E = Errno> = core::result::Result<T, E>;

/// The calling process's memory, for the pointers inside ioctl arguments.
pub trait UserMemory {
    /// Copy `buf.len()` bytes from `addr`; `EFAULT` if they cannot be read.
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<()>;
    /// Copy `data` to `addr`; `EFAULT` if it cannot be written.
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<()>;
}

/// The calling process's file table, for PRIME (dma-buf file descriptors).
pub trait Fds<B> {
    /// Make a dma-buf file for GEM object `bo` (its memory `buffer`, `size`
    /// bytes) with `flags` (`O_CLOEXEC`, `O_RDWR`), put it in the table and
    /// return its descriptor. The file holds a reference on the object,
    /// given back with [`Device::release_dmabuf`] when it is closed.
    fn export(&mut self, bo: BoId, buffer: &B, size: u64, flags: u32) -> Result<i32>;
    /// The GEM object behind dma-buf descriptor `fd`, if it is one of this
    /// device's; `EBADF` for a closed descriptor, `EINVAL` for another file.
    fn import(&mut self, fd: i32) -> Result<BoId>;
}

/// What one ioctl call needs from the kernel.
pub struct Ctx<'a, B> {
    pub user: &'a mut dyn UserMemory,
    pub fds: &'a mut dyn Fds<B>,
    /// CLOCK_MONOTONIC, ns.
    pub now_ns: u64,
    /// Whether the caller may take DRM master from nobody
    /// (`CAP_SYS_ADMIN`); only `SET_MASTER` asks.
    pub privileged: bool,
}

impl<B> core::fmt::Debug for Ctx<'_, B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ctx").field("now_ns", &self.now_ns).field("privileged", &self.privileged).finish()
    }
}

/// How an ioctl ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Done; the ioctl returns this value (0 for every DRM ioctl).
    Done(u64),
    /// Sleep until CLOCK_MONOTONIC reads `until_ns`, then issue the same
    /// ioctl again (a blocking vblank wait; a blocking commit behind a
    /// pending one).
    Sleep { until_ns: u64 },
}

/// One output of the hardware (a virtio-gpu scanout).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// Whether a display is attached (enabled in the display info).
    pub connected: bool,
    /// Its preferred size (0 x 0 if unknown).
    pub width: u32,
    pub height: u32,
    /// Its EDID, if the hardware has one.
    pub edid: Option<Vec<u8>>,
}

/// How a framebuffer lies in its buffer: `height` lines of `pitch` bytes
/// from byte `offset`, `width` pixels of `format` (a 32-bit fourcc) used of
/// each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FbLayout {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub offset: u64,
    pub format: u32,
}

/// A rectangle in framebuffer pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    /// The part of `self` inside a `width` x `height` area.
    pub fn clip(&self, width: u32, height: u32) -> Rect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        Rect { x, y, width: self.width.min(width - x), height: self.height.min(height - y) }
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A hardware command failed (the device reported an error or was given up
/// on). The ioctl fails with `EIO`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwError;

/// What the device needs from the hardware. Framebuffers are named by
/// their DRM object ID; how the hardware calls them (virtio resource IDs)
/// is its own business.
pub trait Hardware {
    /// The memory behind a GEM object: what a mapping maps and what a
    /// framebuffer's device copy is made from. Cloned for every mapping and
    /// dma-buf, and freed by the hardware when the last clone goes.
    type Buffer: Clone;

    /// The outputs, one per CRTC. Called at creation (which fixes how many
    /// there are) and again when a client probes a connector.
    fn probe(&mut self) -> Vec<Output>;
    /// `size` bytes of zeroed memory for a dumb buffer, or `None`.
    fn alloc(&mut self, size: u64) -> Option<Self::Buffer>;
    /// Make the device's copy of framebuffer `fb` (`layout` in `buffer`).
    fn create_fb(&mut self, fb: u32, layout: &FbLayout, buffer: &Self::Buffer) -> Result<(), HwError>;
    /// Forget framebuffer `fb`, which no output shows. If this fails the
    /// device may still use `buffer`'s memory, and the hardware must keep it.
    fn destroy_fb(&mut self, fb: u32, buffer: &Self::Buffer) -> Result<(), HwError>;
    /// Copy `rect` of framebuffer `fb` from its buffer to the device.
    fn upload(&mut self, fb: u32, layout: &FbLayout, rect: Rect) -> Result<(), HwError>;
    /// Show `rect` of framebuffer `fb` on output `index`, or nothing.
    fn scanout(&mut self, index: usize, show: Option<(u32, Rect)>) -> Result<(), HwError>;
    /// Put `rect` of framebuffer `fb` on the outputs that show it.
    fn flush(&mut self, fb: u32, rect: Rect) -> Result<(), HwError>;
    /// The device is about to show its own framebuffers: the kernel
    /// console stops drawing to the outputs.
    fn take_display(&mut self);
    /// The device shows nothing any more: the console shows itself again.
    fn give_back_display(&mut self);
}

#[cfg(test)]
mod tests;
