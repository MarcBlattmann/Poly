//! Intel High Definition Audio: the controller and its codecs.
//!
//! Written from Intel's public *High Definition Audio Specification*
//! revision 1.0a ("HDA 1.0a" below).
//!
//! - [`regs`]: the controller registers (§3.3) and their bits.
//! - [`format`]: the 16-bit stream format of SDnFMT and of the converter
//!   format verb (§3.7.1), and the PCM rate / size bits of parameter 0x0A.
//! - [`codec`]: verbs (§7.3.3), parameters (§7.3.4) and the decoded
//!   capability words: audio widget capabilities, pin capabilities, the
//!   configuration default, amplifier capabilities.
//! - [`graph`]: the widget table of one audio function group and the
//!   search for an output path through it (pin back to a DAC).
//! - [`Hda`] is the driver: [`init`] resets the controller (§4.2), finds the
//!   codecs (§4.3), sets up the command and response rings (§4.4), walks
//!   every codec's widget graph, picks one output path and powers and
//!   unmutes it. One output stream descriptor (§3.3, §3.6) then plays a
//!   cyclic buffer described by a buffer descriptor list.
//!
//! # Memory
//!
//! Nothing here allocates. The caller lends four DMA regions in a
//! [`Memory`]: the CORB (command ring), the RIRB (response ring), the
//! buffer descriptor list and the output ring buffer. Each must start on a
//! 128-byte boundary (§3.3, CORBLBASE / RIRBLBASE / SDnBDPL bits 6:0 are
//! zero; §3.6.2, BDL entries' buffers). The driver touches them only through
//! raw pointers; no Rust references to device-visible memory are made.
//!
//! # Commands
//!
//! Verbs go out one at a time: write the command at CORBWP + 1, move
//! CORBWP, poll RIRBWP until the solicited response from the addressed
//! codec arrives (bounded; see [`Delay`]). Unsolicited responses, answers
//! from other codecs and leftovers of an earlier timed-out verb are skipped
//! and counted. The rings are polled: no command or response interrupts.
//!
//! # Untrusted device
//!
//! Every value read from the controller or a codec is untrusted. Loops over
//! device data are bounded (function groups, widgets, connection list
//! entries, path depth, register polls), table indices come from the
//! driver's own counters, and nothing panics on device data.

#![no_std]

pub mod codec;
mod driver;
pub mod format;
pub mod graph;
pub mod regs;

pub use driver::{
    CODEC_WAKE_US, DmaRegion, Error, Gcap, Hda, InitError, Interrupts, MAX_BDL_ENTRIES, Memory, OUTPUT_STREAM_TAG,
    Stats, init,
};
pub use format::StreamFormat;
pub use graph::{MAX_CONNECTIONS, MAX_PATH, MAX_WIDGETS, OutputPath, Widget};

/// Waits. The driver calls this to let `us` microseconds pass: between
/// register polls, during and after controller reset, while waiting for a
/// codec response. The kernel spins on a timer or yields; tests do nothing.
///
/// Every wait loop in this crate is bounded by a poll count, so a `Delay`
/// that returns immediately only shortens the timeouts, it never hangs.
pub trait Delay {
    fn delay_us(&mut self, us: u32);
}

impl<F: FnMut(u32)> Delay for F {
    fn delay_us(&mut self, us: u32) {
        self(us)
    }
}

#[cfg(test)]
mod sim;

#[cfg(test)]
mod tests;
