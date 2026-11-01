//! The audio server's arithmetic (ADR 0109): what turns several client
//! streams, each in its own format, into the one stream a sound device
//! plays, and the device's captured stream back into each client's format.
//!
//! - [`sample`]: sample formats and their conversion to and from one
//!   integer domain, signed 24-bit precision in an `i32`
//!   ([`FULL_SCALE`]), with clipping on the way out. Floating-point
//!   samples are taken apart and built with integer operations (the kernel
//!   has no FPU state to spare).
//! - [`convert`]: a [`Converter`] from one [`Format`] to another: channel
//!   mapping (mono to all channels, many down to mono by averaging),
//!   linear-interpolation resampling with a 32.32 fixed-point phase, and a
//!   gain per output channel. It mixes into an accumulator (playback) or
//!   writes encoded frames (capture).
//! - [`server`]: one period of the device's stream mixed from every
//!   playing [`server::Source`], and one captured period handed to every
//!   recording [`server::Sink`]. Clients that cannot keep up are mixed
//!   with what they have; the rest of their share is silence.
//! - [`gain_of`]: a volume control's value as a gain.
//!
//! Nothing here knows about ALSA or a device; the kernel's sound module
//! puts client ring buffers behind the traits. Nothing allocates.

#![no_std]

pub mod convert;
pub mod sample;
pub mod server;

pub use convert::Converter;
pub use sample::Sample;

#[cfg(test)]
mod tests;

/// Full scale of the internal sample domain: samples lie in
/// `-FULL_SCALE..FULL_SCALE` (24 significant bits). Sums of several
/// streams may go beyond until they are clipped on output.
pub const FULL_SCALE: i32 = 1 << 23;

/// The most channels a stream may have.
pub const MAX_CHANNELS: usize = 8;

/// A gain of 1.0 (gains are 16.16 fixed point).
pub const UNITY: u32 = 1 << 16;

/// A stream's layout: the sample format, channels per frame and frames
/// per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub sample: Sample,
    pub channels: u8,
    pub rate: u32,
}

impl Format {
    pub const fn new(sample: Sample, channels: u8, rate: u32) -> Format {
        Format { sample, channels, rate }
    }

    /// Bytes per frame.
    pub const fn frame_bytes(&self) -> usize {
        self.sample.bytes() * self.channels as usize
    }

    /// Usable: 1 to [`MAX_CHANNELS`] channels and a rate.
    pub const fn is_valid(&self) -> bool {
        self.channels >= 1 && self.channels as usize <= MAX_CHANNELS && self.rate > 0
    }
}

/// The gain of a volume control at `value` out of `0..=max`, linear in
/// amplitude: 0 is silence, `max` is unity.
pub fn gain_of(value: u32, max: u32) -> u32 {
    if max == 0 {
        return UNITY;
    }
    ((value.min(max) as u64 * UNITY as u64) / max as u64) as u32
}

/// `v` times `gain` (16.16).
#[inline]
pub fn apply_gain(v: i32, gain: u32) -> i32 {
    if gain == UNITY {
        return v;
    }
    ((v as i64 * gain as i64) >> 16).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}
