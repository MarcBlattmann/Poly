//! The virtio network device (VIRTIO 1.2 §5.1) as a [`poly_net::NetDevice`].
//!
//! - [`select_features`] picks what the driver understands, and
//!   [`NetConfig::read`] reads the device configuration that goes with it.
//! - [`VirtioNet`] is the driver: Ethernet frames are copied into transmit
//!   buffers behind a [`NetHeader`] and out of receive buffers the device
//!   filled, and the receive queue is refilled as frames are taken. It
//!   implements [`poly_net::NetDevice`], so a [`poly_net::Stack`] runs on it.
//! - [`init`] runs device initialisation (§3.1.1, §5.1.5) with
//!   [`poly_virtio`] and hands back a running driver.
//!
//! Like `poly-virtio-blk`, this is a crate of its own so each device class
//! keeps its protocol to itself (ADR 0009), and it allocates nothing: the
//! kernel provides the queue memory, the packet buffers and the tables.
//!
//! # Frames (§5.1.6)
//!
//! Every frame on either queue is preceded by a 12-byte `virtio_net_hdr`
//! (with VIRTIO_F_VERSION_1 the header always has `num_buffers`, §5.1.6):
//!
//! ```text
//! u8 flags; u8 gso_type; le16 hdr_len; le16 gso_size; le16 csum_start; le16 csum_offset; le16 num_buffers;
//! ```
//!
//! Each packet buffer is [`BUF_LEN`] bytes of driver-owned DMA memory, one
//! descriptor each. On transmit the header (all zeros: no checksum offload,
//! no segmentation, §5.1.6.2.1) and the frame go into one buffer as a single
//! device-readable descriptor, which VERSION_1 allows (it implies
//! VIRTIO_F_ANY_LAYOUT, §2.6.4). On receive the device writes header and
//! frame into device-writable buffers; with VIRTIO_NET_F_MRG_RXBUF a large
//! packet may continue over `num_buffers` buffers (§5.1.6.4).
//!
//! # Features
//!
//! Taken when offered: MAC (the station address from the configuration;
//! without it the caller's locally administered address is used, §5.1.4.2),
//! STATUS (link state), MTU (when the buffers can hold a frame of that
//! size), MRG_RXBUF, EVENT_IDX and ACCESS_PLATFORM. Not taken: checksum
//! and segmentation offloads (CSUM, GUEST_CSUM, TSO, UFO, USO), so every
//! packet in both directions carries complete checksums (§5.1.6.2.1,
//! §5.1.6.4.1); the control queue (CTRL_VQ and what needs it); several
//! queue pairs (MQ, RSS); hash reports.
//!
//! What the device reports is untrusted: lengths, buffer counts and used
//! ring entries are checked, and a device that breaks the queue protocol is
//! reset and refused from then on.

#![no_std]

mod driver;

pub use driver::{DmaRegion, InitError, PacketMemory, Setup, Stats, Storage, VirtioNet, init};

use poly_pci::Mmio;
use poly_virtio::CommonCfg;

/// Device feature bits (§5.1.3).
pub mod feature {
    /// The device takes packets with a partial checksum (checksum offload).
    pub const CSUM: u64 = 1 << 0;
    /// The driver takes packets with a partial checksum.
    pub const GUEST_CSUM: u64 = 1 << 1;
    pub const CTRL_GUEST_OFFLOADS: u64 = 1 << 2;
    /// `mtu` in the configuration is valid: the largest packet the device
    /// handles.
    pub const MTU: u64 = 1 << 3;
    /// `mac` in the configuration is valid.
    pub const MAC: u64 = 1 << 5;
    pub const GUEST_TSO4: u64 = 1 << 7;
    pub const GUEST_TSO6: u64 = 1 << 8;
    pub const GUEST_ECN: u64 = 1 << 9;
    pub const GUEST_UFO: u64 = 1 << 10;
    pub const HOST_TSO4: u64 = 1 << 11;
    pub const HOST_TSO6: u64 = 1 << 12;
    pub const HOST_ECN: u64 = 1 << 13;
    pub const HOST_UFO: u64 = 1 << 14;
    /// Received packets may span several buffers (`num_buffers`).
    pub const MRG_RXBUF: u64 = 1 << 15;
    /// `status` in the configuration is valid: link up or down.
    pub const STATUS: u64 = 1 << 16;
    pub const CTRL_VQ: u64 = 1 << 17;
    pub const CTRL_RX: u64 = 1 << 18;
    pub const CTRL_VLAN: u64 = 1 << 19;
    pub const GUEST_ANNOUNCE: u64 = 1 << 21;
    pub const MQ: u64 = 1 << 22;
    pub const CTRL_MAC_ADDR: u64 = 1 << 23;
    pub const HOST_USO: u64 = 1 << 56;
    pub const HASH_REPORT: u64 = 1 << 57;
    pub const RSS: u64 = 1 << 60;
    pub const RSC_EXT: u64 = 1 << 61;
    pub const STANDBY: u64 = 1 << 62;
    pub const SPEED_DUPLEX: u64 = 1 << 63;
}

/// `virtio_net_hdr.flags` (§5.1.6).
pub mod hdr_flags {
    /// The checksum at `csum_start` + `csum_offset` still has to be filled
    /// in (only with CSUM / GUEST_CSUM, which this driver never takes).
    pub const NEEDS_CSUM: u8 = 1;
    /// The device validated the packet's checksums.
    pub const DATA_VALID: u8 = 2;
    pub const RSC_INFO: u8 = 4;
}

/// `virtio_net_hdr.gso_type` (§5.1.6).
pub mod gso {
    pub const NONE: u8 = 0;
    pub const TCPV4: u8 = 1;
    pub const UDP: u8 = 3;
    pub const TCPV6: u8 = 4;
    pub const UDP_L4: u8 = 5;
    pub const ECN: u8 = 0x80;
}

/// Bits of the configuration's `status` field (§5.1.4).
pub mod link_status {
    pub const LINK_UP: u16 = 1;
    pub const ANNOUNCE: u16 = 2;
}

/// Virtqueue numbers (§5.1.2): one queue pair, no control queue.
pub const RX_QUEUE: u16 = 0;
pub const TX_QUEUE: u16 = 1;

/// Length of `virtio_net_hdr` without hash reporting (§5.1.6).
pub const HEADER_LEN: usize = 12;
/// Bytes of each packet buffer: room for the header and a frame of up to
/// 2036 bytes, more than the 1526 bytes §5.1.6.3.1 asks of receive buffers
/// without MRG_RXBUF and segmentation offloads.
pub const BUF_LEN: usize = 2048;
/// Ethernet header: destination, source, type.
pub const ETHERNET_HEADER_LEN: usize = 14;
/// The MTU without VIRTIO_NET_F_MTU, and the most the driver uses with it.
pub const DEFAULT_MTU: usize = 1500;
/// The smallest MTU IPv4 allows (RFC 791: every host takes 68-byte
/// datagrams); a device reporting less is not taken at its word.
pub const MIN_MTU: usize = 68;

/// Features this driver uses when offered (MTU only under the conditions
/// of [`select_features`]). VIRTIO_F_ACCESS_PLATFORM is accepted because a
/// device that offers it may refuse to work without it (§6.1); with no
/// IOMMU programmed, bus addresses are physical addresses anyway.
pub const DRIVER_FEATURES: u64 = feature::MAC
    | feature::STATUS
    | feature::MTU
    | feature::MRG_RXBUF
    | poly_virtio::feature::EVENT_IDX
    | poly_virtio::feature::ACCESS_PLATFORM;

/// Can the driver honour a device MTU of `mtu`? §5.1.4.2: with
/// VIRTIO_NET_F_MTU negotiated, the driver must be able to receive a packet
/// of that size, which without MRG_RXBUF has to fit in one buffer.
pub const fn mtu_usable(mtu: u16, mrg_rxbuf: bool) -> bool {
    let mtu = mtu as usize;
    mtu >= MIN_MTU && (mrg_rxbuf || HEADER_LEN + ETHERNET_HEADER_LEN + mtu <= BUF_LEN)
}

/// The features to accept of those `offered`; `device_mtu` is the
/// configuration's `mtu`, read when VIRTIO_NET_F_MTU is offered (§3.1.1
/// step 4 lets the driver read the configuration before accepting).
/// [`poly_virtio::FeatureNegotiation::accept`] adds VIRTIO_F_VERSION_1.
pub const fn select_features(offered: u64, device_mtu: Option<u16>) -> u64 {
    let mut f = offered & DRIVER_FEATURES;
    let usable = match device_mtu {
        Some(m) => mtu_usable(m, f & feature::MRG_RXBUF != 0),
        None => false,
    };
    if !usable {
        f &= !feature::MTU;
    }
    f
}

/// The MTU the driver gives the stack: the device's when VIRTIO_NET_F_MTU
/// was negotiated (a device MTU above [`DEFAULT_MTU`] is not used: the
/// stack's frames stay standard-sized), else [`DEFAULT_MTU`].
pub fn effective_mtu(negotiated_mtu: Option<u16>) -> usize {
    match negotiated_mtu {
        Some(m) => (m as usize).clamp(MIN_MTU, DEFAULT_MTU),
        None => DEFAULT_MTU,
    }
}

/// `struct virtio_net_hdr` (§5.1.6), without the hash report fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
    pub num_buffers: u16,
}

impl NetHeader {
    /// The header of every frame this driver sends: complete checksums, no
    /// segmentation, `num_buffers` 0 (§5.1.6.2.1).
    pub const TRANSMIT: NetHeader = NetHeader {
        flags: 0,
        gso_type: gso::NONE,
        hdr_len: 0,
        gso_size: 0,
        csum_start: 0,
        csum_offset: 0,
        num_buffers: 0,
    };

    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0] = self.flags;
        b[1] = self.gso_type;
        b[2..4].copy_from_slice(&self.hdr_len.to_le_bytes());
        b[4..6].copy_from_slice(&self.gso_size.to_le_bytes());
        b[6..8].copy_from_slice(&self.csum_start.to_le_bytes());
        b[8..10].copy_from_slice(&self.csum_offset.to_le_bytes());
        b[10..12].copy_from_slice(&self.num_buffers.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; HEADER_LEN]) -> NetHeader {
        let le = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        NetHeader {
            flags: b[0],
            gso_type: b[1],
            hdr_len: le(2),
            gso_size: le(4),
            csum_start: le(6),
            csum_offset: le(8),
            num_buffers: le(10),
        }
    }
}

/// Device configuration offsets (§5.1.4, `struct virtio_net_config`).
mod cfg {
    pub const MAC: usize = 0;
    pub const STATUS: usize = 6;
    pub const MTU: usize = 10;
}

/// Tries at a consistent configuration read before giving up.
const CONFIG_TRIES: u32 = 16;

/// The parts of the device configuration the driver uses; each is `None`
/// when its feature is not in the feature set it was read for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetConfig {
    pub mac: Option<[u8; 6]>,
    pub status: Option<u16>,
    pub mtu: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The device configuration region is too short for the features.
    TooShort,
    /// The configuration kept changing while being read.
    Unstable,
}

impl NetConfig {
    /// Read the configuration through `device` (the device configuration
    /// region, `len` bytes long) for `features`, consistently (§2.5.1:
    /// the MAC address spans several reads).
    pub fn read<M: Mmio, C: Mmio + ?Sized>(
        common: &CommonCfg<M>,
        device: &C,
        len: u32,
        features: u64,
    ) -> Result<NetConfig, ConfigError> {
        let has = |f: u64| features & f != 0;
        let need = if has(feature::MTU) {
            cfg::MTU + 2
        } else if has(feature::STATUS) {
            cfg::STATUS + 2
        } else if has(feature::MAC) {
            cfg::MAC + 6
        } else {
            0
        };
        if (len as usize) < need {
            return Err(ConfigError::TooShort);
        }
        common
            .read_config(CONFIG_TRIES, || NetConfig {
                mac: has(feature::MAC).then(|| core::array::from_fn(|i| device.read8(cfg::MAC + i))),
                status: has(feature::STATUS).then(|| device.read16(cfg::STATUS)),
                mtu: has(feature::MTU).then(|| device.read16(cfg::MTU)),
            })
            .ok_or(ConfigError::Unstable)
    }
}

/// A locally administered, unicast MAC address made from `seed` (random
/// bytes, say), for devices without VIRTIO_NET_F_MAC (§5.1.4.2). IEEE 802
/// puts both flags in the first octet: bit 0 set means group (multicast),
/// bit 1 set means locally administered.
pub fn local_mac(seed: [u8; 6]) -> [u8; 6] {
    let mut mac = seed;
    mac[0] = (mac[0] & !0x01) | 0x02;
    mac
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
