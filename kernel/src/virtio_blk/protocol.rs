//! The virtio-blk device type (VIRTIO 1.2 §5.2): feature bits, the device
//! configuration layout and the request format.
//!
//! Everything block-specific the driver needs to put on the wire is here and
//! nowhere else, so a shared virtio-blk library can replace this module
//! without touching the transport and queue handling in the driver.
//!
//! A request is one descriptor chain (§5.2.6):
//!
//! ```text
//! device-readable  le32 type; le32 reserved; le64 sector;   the header, 16 bytes
//! either           u8 data[];                               OUT: readable, IN: writable, FLUSH: none
//! device-writable  u8 status;
//! ```
//!
//! `sector` always counts 512-byte units, whatever the device's block size,
//! and data lengths are multiples of 512.

/// Device feature bits (§5.2.3) the driver asks for.
pub mod feature {
    /// The device is read-only.
    pub const RO: u64 = 1 << 5;
    /// `blk_size` in the configuration is valid.
    pub const BLK_SIZE: u64 = 1 << 6;
    /// The device takes FLUSH requests.
    pub const FLUSH: u64 = 1 << 9;
}

/// Offsets in the device configuration structure (§5.2.4,
/// `struct virtio_blk_config`).
pub mod config {
    /// le64: size of the device in 512-byte sectors.
    pub const CAPACITY: usize = 0x00;
    /// le32: the optimal block size, valid with [`super::feature::BLK_SIZE`].
    pub const BLK_SIZE: usize = 0x14;
    /// Bytes the driver reads: up to and including `blk_size`.
    pub const LEN: usize = 0x18;
}

/// The unit of `sector` and of data lengths.
pub const SECTOR_SIZE: usize = 512;

/// Request types (§5.2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RequestType {
    /// Read: the device writes the data.
    In = 0,
    /// Write: the device reads the data.
    Out = 1,
    /// Write back the device's cache. No data; `sector` must be 0.
    Flush = 4,
}

pub const HEADER_LEN: usize = 16;

/// The request header for a request of `kind` starting at `sector`.
pub fn header(kind: RequestType, sector: u64) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..4].copy_from_slice(&(kind as u32).to_le_bytes());
    // Bytes 4..8 are reserved and stay zero.
    h[8..16].copy_from_slice(&sector.to_le_bytes());
    h
}

/// The status byte the device writes last (§5.2.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    IoError,
    Unsupported,
    /// Anything else: the device did not complete the request properly.
    Invalid(u8),
}

impl Status {
    /// The status byte is set to this before a request goes out, so a
    /// device that never writes it is caught.
    pub const UNWRITTEN: u8 = 0xff;

    pub const fn from_byte(b: u8) -> Status {
        match b {
            0 => Status::Ok,
            1 => Status::IoError,
            2 => Status::Unsupported,
            other => Status::Invalid(other),
        }
    }
}
