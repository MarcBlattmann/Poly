//! The virtio input device (VIRTIO 1.2 §5.8): keyboards, mice and tablets
//! (QEMU's `virtio-keyboard-pci`, `virtio-mouse-pci`, `virtio-tablet-pci`).
//!
//! The device speaks evdev: its events are `struct virtio_input_event`
//! (`le16 type`, `le16 code`, `le32 value`, §5.8.6) with the Linux input
//! layer's types and codes, and its configuration describes it the way
//! the evdev ioctls do. So this driver is thin:
//!
//! - [`read_info`] queries the configuration (§5.8.4) into a
//!   [`poly_input::hub::DeviceInfo`]: the name, serial, ids, properties,
//!   the codes of every event type and each absolute axis's range.
//! - [`VirtioInput`] keeps the event queue (0) full of 8-byte buffers and
//!   hands back the events the device put in them ([`VirtioInput::poll`]),
//!   and sends events to the device on the status queue (1), which is how
//!   the keyboard LEDs are set ([`VirtioInput::send`]).
//! - [`init`] runs device initialisation (§3.1.1, §5.8.5) with
//!   [`poly_virtio`].
//!
//! The device has no feature bits of its own; VERSION_1, EVENT_IDX and
//! ACCESS_PLATFORM are taken when offered. As for the other virtio
//! drivers (ADR 0009), the kernel provides the DMA memory; what the device
//! reports is untrusted (a device that breaks the queue protocol is reset
//! and refused from then on).

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod driver;

pub use driver::{
    DmaRegion, InitError, MAX_EVENT_ENTRIES, MAX_STATUS_ENTRIES, QueueMemory, SendError, Setup, Stats, Storage,
    VirtioInput, init,
};

use alloc::string::String;
use alloc::vec::Vec;
use poly_input::codes::{bus, ev};
use poly_input::hub::{AbsInfo, DeviceInfo, InputId};
use poly_pci::Mmio;
use poly_virtio::CommonCfg;

/// Virtqueue numbers (§5.8.2).
pub const EVENT_QUEUE: u16 = 0;
pub const STATUS_QUEUE: u16 = 1;

/// Bytes of `struct virtio_input_event`.
pub const EVENT_LEN: usize = 8;

/// `virtio_input_config.select` values (§5.8.4).
pub mod select {
    pub const UNSET: u8 = 0x00;
    pub const ID_NAME: u8 = 0x01;
    pub const ID_SERIAL: u8 = 0x02;
    pub const ID_DEVIDS: u8 = 0x03;
    pub const PROP_BITS: u8 = 0x10;
    /// `subsel` is an event type; the bitmap of its codes.
    pub const EV_BITS: u8 = 0x11;
    /// `subsel` is an absolute axis; its `virtio_input_absinfo`.
    pub const ABS_INFO: u8 = 0x12;
}

/// Offsets in `struct virtio_input_config` (§5.8.4).
pub mod cfg {
    pub const SELECT: usize = 0;
    pub const SUBSEL: usize = 1;
    pub const SIZE: usize = 2;
    /// The union: a string, a bitmap, an absinfo or the device ids.
    pub const U: usize = 8;
    /// The union's size.
    pub const U_LEN: usize = 128;
}

/// Features this driver uses when offered (§6: the input device defines
/// none of its own).
pub const DRIVER_FEATURES: u64 = poly_virtio::feature::EVENT_IDX | poly_virtio::feature::ACCESS_PLATFORM;

/// One `struct virtio_input_event`: type, code, value.
pub type RawEvent = (u16, u16, i32);

/// The event in 8 bytes of an event buffer.
pub fn parse_event(b: &[u8; EVENT_LEN]) -> RawEvent {
    (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]), i32::from_le_bytes([b[4], b[5], b[6], b[7]]))
}

pub fn event_bytes(e: RawEvent) -> [u8; EVENT_LEN] {
    let mut b = [0u8; EVENT_LEN];
    b[0..2].copy_from_slice(&e.0.to_le_bytes());
    b[2..4].copy_from_slice(&e.1.to_le_bytes());
    b[4..8].copy_from_slice(&e.2.to_le_bytes());
    b
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The configuration region is shorter than its header.
    TooShort,
    /// The configuration kept changing while being read.
    Unstable,
}

/// Tries at a consistent configuration read before giving up.
const CONFIG_TRIES: u32 = 16;

/// Ask the configuration for `select`/`subsel`: the bytes the device
/// returns (`size` of them, at most the union and the region).
pub fn query<M: Mmio, C: Mmio + ?Sized>(
    common: &CommonCfg<M>,
    device: &C,
    len: u32,
    select: u8,
    subsel: u8,
) -> Result<Vec<u8>, ConfigError> {
    let len = len as usize;
    if len < cfg::U {
        return Err(ConfigError::TooShort);
    }
    common
        .read_config(CONFIG_TRIES, || {
            device.write8(cfg::SELECT, select);
            device.write8(cfg::SUBSEL, subsel);
            let size = usize::from(device.read8(cfg::SIZE)).min(cfg::U_LEN).min(len - cfg::U);
            (0..size).map(|i| device.read8(cfg::U + i)).collect::<Vec<u8>>()
        })
        .ok_or(ConfigError::Unstable)
}

fn le32(b: &[u8], i: usize) -> i32 {
    b.get(i..i + 4).map_or(0, |w| i32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

fn le16(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2).map_or(0, |w| u16::from_le_bytes([w[0], w[1]]))
}

/// A configuration string: up to its first NUL, as UTF-8 (invalid bytes
/// replaced).
fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The device as the event layer describes devices, from its
/// configuration (§5.8.4): name, serial (`uniq`), ids (`BUS_VIRTUAL` if it
/// has none), properties, the codes of each event type it supports and
/// each absolute axis's range. A device that supports `EV_REP` gets soft
/// repeat (§5.8.5.1 leaves auto-repeat to the driver); one with the letter
/// keys is a console keyboard.
pub fn read_info<M: Mmio, C: Mmio + ?Sized>(
    common: &CommonCfg<M>,
    device: &C,
    len: u32,
) -> Result<DeviceInfo, ConfigError> {
    let q = |select, subsel| query(common, device, len, select, subsel);
    let name = text(&q(select::ID_NAME, 0)?);
    let ids = q(select::ID_DEVIDS, 0)?;
    let id = if ids.len() >= 8 {
        InputId { bustype: le16(&ids, 0), vendor: le16(&ids, 2), product: le16(&ids, 4), version: le16(&ids, 6) }
    } else {
        InputId { bustype: bus::VIRTUAL, ..InputId::default() }
    };
    let mut info = DeviceInfo::new(if name.is_empty() { "virtio input device" } else { &name }, id);
    info.uniq = text(&q(select::ID_SERIAL, 0)?);
    for (i, byte) in q(select::PROP_BITS, 0)?.iter().enumerate() {
        for bit in 0..8 {
            if byte & (1 << bit) != 0 {
                info.caps.prop.set((i * 8 + bit) as u16, true);
            }
        }
    }
    for kind in 1..=ev::MAX {
        let bitmap = q(select::EV_BITS, kind as u8)?;
        if bitmap.is_empty() {
            continue;
        }
        info.caps.ev.set(kind, true);
        for (i, byte) in bitmap.iter().enumerate() {
            for bit in 0..8 {
                if byte & (1 << bit) != 0 {
                    info.declare(kind, (i * 8 + bit) as u16);
                }
            }
        }
    }
    let axes: Vec<u16> = info.caps.abs.iter().collect();
    for code in axes {
        let a = q(select::ABS_INFO, code as u8)?;
        info.abs[usize::from(code)] = AbsInfo {
            value: 0,
            minimum: le32(&a, 0),
            maximum: le32(&a, 4),
            fuzz: le32(&a, 8),
            flat: le32(&a, 12),
            resolution: le32(&a, 16),
        };
    }
    info.soft_repeat = info.caps.ev.get(ev::REP);
    info.console = info.has_letter_keys();
    Ok(info)
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
