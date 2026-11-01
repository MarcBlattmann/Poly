//! The virtio sound device (VIRTIO 1.2 §5.14).
//!
//! - [`proto`]: the messages: configuration, item queries (jacks, PCM
//!   streams, channel maps), SET PARAMETERS and the other PCM commands,
//!   events, and the header and status around every I/O message.
//! - [`stream`]: the PCM command lifecycle (§5.14.6.6.1) as a state
//!   machine, and the checks a stream's parameters must pass.
//! - [`ring`]: period and buffer bookkeeping of an output stream: what the
//!   writer may fill, what goes to the device next, what it has played.
//! - [`VirtioSound`] is the driver: [`init`] runs device initialisation
//!   (§3.1.1, §5.14.5) with [`poly_virtio`], control requests go out one at
//!   a time and are polled for, output messages point into the PCM memory,
//!   event buffers are kept posted.
//!
//! Like the other virtio device crates it allocates nothing (ADR 0009):
//! the kernel provides the queue memory, the control, event and message
//! buffers, the PCM ring buffer and the tables. ADR 0104 has the design.
//!
//! # Queues (§5.14.2)
//!
//! 0 control, 1 event, 2 tx (output), 3 rx (input). All four are set up.
//!
//! # Input messages (§5.14.6.8)
//!
//! Capture runs the other way round: each message is the header
//! (device-readable), a data buffer in the PCM memory and the status (both
//! device-writable). The device returns it once it filled the buffer with
//! captured frames ([`VirtioSound::submit_rx`], [`VirtioSound::poll_rx`]);
//! ADR 0109 has the capture design.
//!
//! # Output messages (§5.14.6.8)
//!
//! Each message is a `virtio_snd_pcm_xfer` header (the stream id), the
//! frames, and a device-writable `virtio_snd_pcm_status`; the device
//! returns the message once it has consumed the frames, which is the
//! driver's "period elapsed". Headers and status live in per-slot message
//! memory, the frames in the PCM ring buffer, so nothing is copied twice.
//!
//! What the device reports is untrusted: response lengths and statuses are
//! checked, and a device that breaks the queue protocol or does not answer
//! a control request in time is reset and refused from then on.

#![no_std]

mod driver;
pub mod proto;
pub mod ring;
pub mod stream;

pub use driver::{
    CONTROL_MIN, CONTROL_REQUEST_MAX, CtlError, DRIVER_FEATURES, DmaRegion, InitError, Memory, RX_DESCRIPTORS,
    RX_SLOT_LEN, RxDone, Setup, Stats, Storage, TX_DESCRIPTORS, TX_SLOT_LEN, TxDone, TxError, VirtioSound, init,
};

/// Virtqueue numbers (§5.14.2).
pub const CONTROL_QUEUE: u16 = 0;
pub const EVENT_QUEUE: u16 = 1;
pub const TX_QUEUE: u16 = 2;
pub const RX_QUEUE: u16 = 3;

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
