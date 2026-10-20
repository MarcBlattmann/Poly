//! What a network driver offers the stack: Ethernet frames in and out.
//!
//! A virtio-net driver implements [`NetDevice`] over its queues; the stack
//! never touches hardware. Frames are copied in and out of buffers the stack
//! owns, so the driver keeps full control of its DMA memory (ADR 0009).

use crate::wire::ethernet::{self, MacAddr};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// Why a frame was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxError {
    /// The transmit queue is full; try again after the device frees space.
    Busy,
    /// The frame is larger than the device takes.
    TooLong,
}

/// An Ethernet device.
///
/// [`crate::Stack::poll`] calls [`receive`](NetDevice::receive) until it
/// returns `None` (or a per-poll budget runs out), then transmits whatever
/// is due. A driver with interrupts arranges for `poll` to be called when a
/// frame arrives or transmit space frees up.
pub trait NetDevice {
    /// The station address, used as the source of every frame.
    fn mac(&self) -> MacAddr;

    /// The largest IP packet a frame can carry (1500 for standard Ethernet).
    /// Frames are at most `mtu() + 14` bytes, without the FCS.
    fn mtu(&self) -> usize;

    /// Queue one complete frame (header included, no FCS) for transmission.
    /// The device pads short frames if its hardware needs it; the stack
    /// already pads to 60 bytes.
    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError>;

    /// Copy the next received frame into `buf` (at least `mtu() + 14` bytes)
    /// and return its length, or `None` if nothing is waiting. Frames that do
    /// not fit are dropped by the device.
    fn receive(&mut self, buf: &mut [u8]) -> Option<usize>;

    /// False while [`transmit`](NetDevice::transmit) would return `Busy`.
    /// The stack holds back its own output (but not replies) while this is
    /// false, so packets wait in sockets instead of being dropped.
    fn can_transmit(&self) -> bool {
        true
    }
}

/// A device whose transmitted frames come back as received frames.
///
/// Used where there is no hardware, and by tests. Traffic to the stack's
/// own addresses and to 127.0.0.0/8 and ::1 never reaches a device at all;
/// the stack loops it back internally.
#[derive(Debug)]
pub struct Loopback {
    mac: MacAddr,
    mtu: usize,
    queue: VecDeque<Vec<u8>>,
    capacity: usize,
}

impl Loopback {
    /// A loopback device with the given address and MTU, holding up to
    /// `capacity` frames.
    pub fn new(mac: MacAddr, mtu: usize, capacity: usize) -> Loopback {
        Loopback { mac, mtu, queue: VecDeque::new(), capacity }
    }

    /// Frames waiting to be received.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }
}

impl NetDevice for Loopback {
    fn mac(&self) -> MacAddr {
        self.mac
    }

    fn mtu(&self) -> usize {
        self.mtu
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        if frame.len() > self.mtu + ethernet::HEADER_LEN {
            return Err(TxError::TooLong);
        }
        if self.queue.len() >= self.capacity {
            return Err(TxError::Busy);
        }
        self.queue.push_back(frame.to_vec());
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        loop {
            let f = self.queue.pop_front()?;
            if f.len() <= buf.len() {
                buf[..f.len()].copy_from_slice(&f);
                return Some(f.len());
            }
        }
    }

    fn can_transmit(&self) -> bool {
        self.queue.len() < self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_returns_frames_in_order() {
        let mut lo = Loopback::new(MacAddr([2, 0, 0, 0, 0, 1]), 100, 2);
        lo.transmit(&[1; 60]).unwrap();
        lo.transmit(&[2; 61]).unwrap();
        assert!(!lo.can_transmit());
        assert_eq!(lo.transmit(&[3; 60]), Err(TxError::Busy));
        assert_eq!(lo.transmit(&[3; 115]), Err(TxError::TooLong));
        let mut buf = [0u8; 114];
        assert_eq!(lo.receive(&mut buf), Some(60));
        assert_eq!(buf[0], 1);
        assert_eq!(lo.receive(&mut buf), Some(61));
        assert_eq!(buf[0], 2);
        assert_eq!(lo.receive(&mut buf), None);
        // A frame too big for the caller's buffer is dropped.
        lo.transmit(&[4; 100]).unwrap();
        assert_eq!(lo.receive(&mut buf[..50]), None);
        assert_eq!(lo.pending(), 0);
    }
}
