//! The virtio block device (VIRTIO 1.2 §5.2) as a [`poly_block::BlockDevice`].
//!
//! - [`select_features`] picks what the driver understands, and
//!   [`BlkConfig::read`] reads the device configuration that goes with it.
//! - [`VirtioBlk`] is the driver: it turns block requests into descriptor
//!   chains on the device's request queue and their status bytes back into
//!   results. It implements [`poly_block::Abort`] (reset the device, fail
//!   what is in flight), so a [`poly_block::Watchdog`] can give up on a
//!   device that stops answering.
//! - [`init`] runs the whole of device initialisation (§3.1.1) with
//!   [`poly_virtio`] and hands back a running driver.
//!
//! This is its own crate, not part of `poly-block`, so the block layer stays
//! free of any transport (RAM disks and other drivers use it too) and each
//! device class keeps its protocol to itself (ADR 0009).
//!
//! # Requests (§5.2.6)
//!
//! Every request is one descriptor chain:
//!
//! ```text
//! le32 type; le32 reserved; le64 sector;   16 bytes, device-readable
//! u8 data[];                               readable for OUT, writable for IN and GET_ID
//! u8 status;                               1 byte, device-writable
//! ```
//!
//! Sectors are always 512 bytes, whatever block size the device suggests
//! with `blk_size`. Headers and status bytes live in a driver-owned DMA
//! area, one slot per queue entry; data goes straight from and to the
//! caller's buffers.
//!
//! Only the modern interface and a single request queue are used (no
//! VIRTIO_BLK_F_MQ); discard, write-zeroes, secure erase, lifetime and zoned
//! commands are not used.
//!
//! As everywhere in the driver crates, what the device reports is untrusted:
//! configuration values are checked before use and odd status bytes fail
//! the request rather than the kernel.

#![no_std]

mod driver;

pub use driver::{
    DmaRegion, IdentityDma, InitError, Parts, ReqSlot, SLOT_LEN, Setup, Storage, VirtioBlk, init, request_area_len,
};

use poly_block::{DeviceInfo, MIN_BLOCK_SIZE};
use poly_pci::Mmio;
use poly_virtio::CommonCfg;

/// Turns the CPU address of memory handed to the driver into the address
/// the device uses for it. The kernel implements this for its direct map.
pub trait Dma {
    /// The bus address of `buf`, or `None` if the device cannot reach all of
    /// it at consecutive bus addresses (the request then fails with
    /// [`poly_block::IoError::BadBuffer`]).
    fn bus_address(&self, buf: poly_block::IoBuf) -> Option<u64>;
}

/// Device feature bits (§5.2.3).
pub mod feature {
    /// `size_max` is valid: the largest segment the device takes.
    pub const SIZE_MAX: u64 = 1 << 1;
    /// `seg_max` is valid: the most segments in a request.
    pub const SEG_MAX: u64 = 1 << 2;
    pub const GEOMETRY: u64 = 1 << 4;
    /// The device is read-only.
    pub const RO: u64 = 1 << 5;
    /// `blk_size` is valid: the block size to use for best performance.
    pub const BLK_SIZE: u64 = 1 << 6;
    /// The device has a write cache and takes flush requests.
    pub const FLUSH: u64 = 1 << 9;
    pub const TOPOLOGY: u64 = 1 << 10;
    /// The write cache can be switched with the `writeback` field.
    pub const CONFIG_WCE: u64 = 1 << 11;
    pub const MQ: u64 = 1 << 12;
    pub const DISCARD: u64 = 1 << 13;
    pub const WRITE_ZEROES: u64 = 1 << 14;
    pub const LIFETIME: u64 = 1 << 15;
    pub const SECURE_ERASE: u64 = 1 << 16;
}

/// Request types (§5.2.6).
pub mod req_type {
    pub const IN: u32 = 0;
    pub const OUT: u32 = 1;
    pub const FLUSH: u32 = 4;
    pub const GET_ID: u32 = 8;
    pub const GET_LIFETIME: u32 = 10;
    pub const DISCARD: u32 = 11;
    pub const WRITE_ZEROES: u32 = 13;
    pub const SECURE_ERASE: u32 = 14;
}

/// Request status values (§5.2.6).
pub mod req_status {
    pub const OK: u8 = 0;
    pub const IOERR: u8 = 1;
    pub const UNSUPP: u8 = 2;
}

/// The unit of the `sector` field and of `capacity`.
pub const SECTOR_SIZE: u32 = 512;
/// Length of the request header.
pub const HEADER_LEN: usize = 16;
/// Length of the device ID string GET_ID returns (NUL-padded, not
/// necessarily terminated).
pub const ID_LEN: usize = 20;
/// The largest `blk_size` taken as the block size; above it the driver
/// uses 512-byte blocks.
pub const MAX_BLOCK_SIZE: u32 = 64 * 1024;

/// Features this driver uses when offered. VIRTIO_F_ACCESS_PLATFORM is
/// accepted because a device that offers it may refuse to work without it
/// (§6.1); with no IOMMU programmed, bus addresses are physical addresses,
/// which is what [`Dma`] provides anyway.
pub const DRIVER_FEATURES: u64 = feature::SIZE_MAX
    | feature::SEG_MAX
    | feature::RO
    | feature::BLK_SIZE
    | feature::FLUSH
    | poly_virtio::feature::EVENT_IDX
    | poly_virtio::feature::ACCESS_PLATFORM;

/// The features to accept of those `offered`.
/// [`poly_virtio::FeatureNegotiation::accept`] adds VIRTIO_F_VERSION_1.
pub const fn select_features(offered: u64) -> u64 {
    offered & DRIVER_FEATURES
}

/// Device configuration offsets (§5.2.4, `struct virtio_blk_config`).
mod cfg {
    pub const CAPACITY: usize = 0;
    pub const SIZE_MAX: usize = 8;
    pub const SEG_MAX: usize = 12;
    pub const BLK_SIZE: usize = 20;
}

/// Tries at a consistent configuration read before giving up.
const CONFIG_TRIES: u32 = 16;

/// The parts of the device configuration the driver uses. Optional fields
/// are `None` when their feature was not negotiated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlkConfig {
    /// In 512-byte sectors.
    pub capacity: u64,
    pub size_max: Option<u32>,
    pub seg_max: Option<u32>,
    pub blk_size: Option<u32>,
    pub read_only: bool,
    /// The device has a write cache that needs flushes.
    pub flush: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The device configuration region is too short for the negotiated
    /// features.
    TooShort,
    /// The configuration kept changing while being read.
    Unstable,
    /// The limits leave no way to transfer a single sector, or the queue is
    /// too small for a request (it needs at least 4 entries).
    Unusable,
}

impl BlkConfig {
    /// Read the configuration through `device` (the device configuration
    /// region, `len` bytes long) for the negotiated `features`, consistently
    /// (§2.5.1: `capacity` is wider than 32 bits).
    pub fn read<M: Mmio, C: Mmio + ?Sized>(
        common: &CommonCfg<M>,
        device: &C,
        len: u32,
        features: u64,
    ) -> Result<BlkConfig, ConfigError> {
        let has = |f: u64| features & f != 0;
        let need = if has(feature::BLK_SIZE) {
            cfg::BLK_SIZE + 4
        } else if has(feature::SEG_MAX) {
            cfg::SEG_MAX + 4
        } else if has(feature::SIZE_MAX) {
            cfg::SIZE_MAX + 4
        } else {
            cfg::CAPACITY + 8
        };
        if (len as usize) < need {
            return Err(ConfigError::TooShort);
        }
        common
            .read_config(CONFIG_TRIES, || BlkConfig {
                capacity: device.read32(cfg::CAPACITY) as u64 | (device.read32(cfg::CAPACITY + 4) as u64) << 32,
                size_max: has(feature::SIZE_MAX).then(|| device.read32(cfg::SIZE_MAX)),
                seg_max: has(feature::SEG_MAX).then(|| device.read32(cfg::SEG_MAX)),
                blk_size: has(feature::BLK_SIZE).then(|| device.read32(cfg::BLK_SIZE)),
                read_only: has(feature::RO),
                flush: has(feature::FLUSH),
            })
            .ok_or(ConfigError::Unstable)
    }

    /// The device as the block layer sees it, on a request queue of
    /// `queue_size` entries.
    ///
    /// - Block size: `blk_size` if it is a power of two from 512 to
    ///   [`MAX_BLOCK_SIZE`] and the segment limits can carry such a block,
    ///   else 512. The capacity is rounded down to whole blocks.
    /// - Segments: a request needs a descriptor each for the header and the
    ///   status besides its data, and without indirect descriptors the chain
    ///   must fit in the queue. §5.2.4 does not say whether `seg_max` counts
    ///   those two, so the driver assumes it does (at least one data segment
    ///   is always allowed: a device saying otherwise could not work at
    ///   all). That costs two segments on devices that count only data.
    /// - Segment length: `size_max`, unless it is 0 (no limit then).
    pub fn info(&self, queue_size: u16) -> Result<DeviceInfo, ConfigError> {
        if queue_size < 4 {
            return Err(ConfigError::Unusable);
        }
        let mut max_segments = queue_size as u32 - 2;
        if let Some(s) = self.seg_max {
            max_segments = max_segments.min(s.saturating_sub(2).max(1));
        }
        let max_segment_len = self.size_max.filter(|&s| s > 0).unwrap_or(u32::MAX);
        let fits = |bs: u32| max_segments as u64 * max_segment_len as u64 >= bs as u64;
        let block_size = match self.blk_size {
            Some(bs) if bs.is_power_of_two() && (MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&bs) && fits(bs) => bs,
            _ => SECTOR_SIZE,
        };
        let info = DeviceInfo {
            block_size,
            blocks: self.capacity / (block_size / SECTOR_SIZE) as u64,
            read_only: self.read_only,
            write_cache: self.flush,
            // The device reports the bytes it wrote (data and status) in
            // 32 bits.
            max_blocks: (u32::MAX - 1) / block_size,
            max_segments: max_segments.min(u16::MAX as u32) as u16,
            max_segment_len,
        };
        if !info.is_valid() {
            return Err(ConfigError::Unusable);
        }
        Ok(info)
    }
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
