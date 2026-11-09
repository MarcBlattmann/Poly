//! The virtio GPU device (VIRTIO 1.2 §5.7): modesetting and a framebuffer
//! in guest memory (2D), and the 3D interface Mesa's virgl and Venus
//! drivers use (contexts, capsets, 3D resources and transfers, command
//! submission, blob resources, fences).
//!
//! - [`protocol`] is the wire format of the control queue (§5.7.6.7):
//!   request encoding for `GET_DISPLAY_INFO`, `GET_EDID`,
//!   `RESOURCE_CREATE_2D`, `RESOURCE_UNREF`, `SET_SCANOUT`,
//!   `RESOURCE_FLUSH`, `TRANSFER_TO_HOST_2D`, `RESOURCE_ATTACH_BACKING` and
//!   `RESOURCE_DETACH_BACKING`, and response decoding (display info, EDID
//!   blobs, error codes).
//! - [`edid`] reads the parts of an EDID base block a driver wants: the
//!   vendor, the monitor name and the preferred timing.
//! - [`modes`] decides what to show where: the size of the framebuffer
//!   (the command line's, else the preferred mode of the first enabled
//!   scanout) and which scanouts show it, and what changes when the
//!   device reports a new display configuration.
//! - [`VirtioGpu`] is the driver: device initialisation (§3.1.1, §5.7.5),
//!   synchronous, fenced commands on the control queue, the display-change
//!   event in the device configuration (§5.7.4), and the steps of
//!   §5.7.6.1-§5.7.6.2 (create a resource, attach guest memory, set the
//!   scanouts; transfer and flush damaged rectangles).
//!
//! Like the other virtio crates it allocates nothing: the kernel provides
//! the queue memory, the command buffers and the framebuffer, and gets
//! everything here tested on the host against a simulated device written
//! from the spec's device requirements.
//!
//! # Layering
//!
//! The driver deals in resources and scanouts, which is what a DRM/KMS
//! layer needs later (framebuffers / dumb buffers are resources, CRTCs and
//! connectors are scanouts, page flips are `SET_SCANOUT`, damage clips are
//! `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH`). The kernel's console display
//! is one user of it: one resource, mirrored on every enabled scanout.
//!
//! # 3D
//!
//! With VIRTIO_GPU_F_VIRGL (and RESOURCE_BLOB, CONTEXT_INIT) negotiated,
//! the driver also creates 3D contexts for a capset (virgl, Venus),
//! fetches capsets, creates 3D and blob resources, maps host blobs into the
//! host-visible shared memory region, and submits command streams. The
//! command streams themselves are the client's (Mesa's): the driver
//! carries them, as the kernel's DRM layer carries them from user space
//! (ADR 0119). Submissions and 3D transfers complete asynchronously
//! ([`Ticket`]s), optionally with an interrupt.
//!
//! # Not here
//!
//! Resource UUIDs, `SET_SCANOUT_BLOB` and the cursor queue: those features
//! are not accepted or not used (§5.7.3).
//!
//! What the device reports is untrusted: response types, lengths, fence
//! IDs, scanout counts and rectangles are checked, and a device that
//! breaks the queue protocol or does not answer is given up on.

#![no_std]

mod driver;
pub mod edid;
pub mod hostmem;
pub mod modes;
pub mod protocol;

pub use driver::{
    COMMAND_MEMORY_LEN, Clock, DmaRegion, Error, Framebuffer, InitError, MAX_BACKING_ENTRIES, MAX_BLOB_ENTRIES,
    MAX_DATA_BUFFERS, SLOTS, Setup, Stats, Ticket, VirtioGpu, init,
};
pub use poly_virtio::Buffer;
pub use protocol::{
    Box3d, CapsetInfo, Create3d, CreateBlob, DisplayInfo, EdidBlob, MemEntry, Rect, ScanoutInfo, Transfer3d,
};

use poly_pci::Mmio;
use poly_virtio::CommonCfg;

/// Device feature bits (§5.7.3).
pub mod feature {
    /// 3D mode: contexts, 3D resources, command submission (virgl and,
    /// with context init, other capsets).
    pub const VIRGL: u64 = 1 << 0;
    /// `GET_EDID` is supported.
    pub const EDID: u64 = 1 << 1;
    /// `RESOURCE_ASSIGN_UUID` is supported. Not accepted.
    pub const RESOURCE_UUID: u64 = 1 << 2;
    /// Blob resources (and the host-visible shared memory region).
    pub const RESOURCE_BLOB: u64 = 1 << 3;
    /// Contexts of a chosen capset, fences on per-context rings.
    pub const CONTEXT_INIT: u64 = 1 << 4;
}

/// Features this driver uses when offered: EDID, 3D (VIRGL), blob
/// resources and context init, plus EVENT_IDX and ACCESS_PLATFORM (a
/// device offering the latter may refuse to work without it, §6.1; with no
/// IOMMU programmed, bus addresses are physical addresses anyway). The 2D
/// commands work the same with 3D negotiated. Resource UUIDs are not
/// taken. [`poly_virtio::FeatureNegotiation::accept`] adds
/// VIRTIO_F_VERSION_1.
pub const DRIVER_FEATURES: u64 = feature::EDID
    | feature::VIRGL
    | feature::RESOURCE_BLOB
    | feature::CONTEXT_INIT
    | poly_virtio::feature::EVENT_IDX
    | poly_virtio::feature::ACCESS_PLATFORM;

/// The features to accept of those `offered`.
pub const fn select_features(offered: u64) -> u64 {
    offered & DRIVER_FEATURES
}

/// Bits of `events_read` / `events_clear` (§5.7.4).
pub mod event {
    /// The display configuration changed: the driver should fetch it again
    /// with `GET_DISPLAY_INFO` (and `GET_EDID` with VIRTIO_GPU_F_EDID).
    pub const DISPLAY: u32 = 1 << 0;
}

/// Virtqueue numbers (§5.7.2).
pub const CONTROL_QUEUE: u16 = 0;
pub const CURSOR_QUEUE: u16 = 1;

/// Device configuration offsets (§5.7.4, `struct virtio_gpu_config`).
pub mod cfg {
    /// Pending events (read-only).
    pub const EVENTS_READ: usize = 0;
    /// Write 1s to clear events (write-only).
    pub const EVENTS_CLEAR: usize = 4;
    /// Scanouts the device supports, at most 16 (§5.7.6.7).
    pub const NUM_SCANOUTS: usize = 8;
    /// Capability sets (3D only).
    pub const NUM_CAPSETS: usize = 12;
    /// Length of the structure.
    pub const LEN: u32 = 16;
}

/// Tries at a consistent configuration read before giving up.
const CONFIG_TRIES: u32 = 16;

/// The device configuration (§5.7.4), without the write-only field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuConfig {
    pub events_read: u32,
    pub num_scanouts: u32,
    pub num_capsets: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The device configuration region is shorter than the structure.
    TooShort,
    /// The configuration kept changing while being read.
    Unstable,
    /// `num_scanouts` is 0 or above [`protocol::MAX_SCANOUTS`].
    BadScanoutCount(u32),
}

impl GpuConfig {
    /// Read the configuration through `device` (the device configuration
    /// region, `len` bytes long) consistently (§2.5.1), and check it.
    pub fn read<M: Mmio, C: Mmio + ?Sized>(
        common: &CommonCfg<M>,
        device: &C,
        len: u32,
    ) -> Result<GpuConfig, ConfigError> {
        if len < cfg::LEN {
            return Err(ConfigError::TooShort);
        }
        let c = common
            .read_config(CONFIG_TRIES, || GpuConfig {
                events_read: device.read32(cfg::EVENTS_READ),
                num_scanouts: device.read32(cfg::NUM_SCANOUTS),
                num_capsets: device.read32(cfg::NUM_CAPSETS),
            })
            .ok_or(ConfigError::Unstable)?;
        // §5.7.4: "num_scanouts specifies the maximum number of scanouts
        // supported by the device. Minimum value is 1, maximum value is 16."
        if c.num_scanouts == 0 || c.num_scanouts as usize > protocol::MAX_SCANOUTS {
            return Err(ConfigError::BadScanoutCount(c.num_scanouts));
        }
        Ok(c)
    }
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
