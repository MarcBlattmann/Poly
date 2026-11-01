//! Mixing clients into the device's stream, and handing captured audio to
//! clients.
//!
//! The device plays one stream in one format; every client has its own
//! ring buffer in its own format. Each device period, every playing
//! client is a [`Source`]: the server peeks at the frames it has ready,
//! converts and mixes as many as one period needs into an accumulator,
//! and tells the client how many it used ([`mix_source`]). A client with
//! too few frames contributes what it has; the rest of its share is
//! silence. The accumulator then goes out clipped, after the master gain
//! ([`crate::convert::render`]).
//!
//! Capture runs the other way: each period the device recorded goes to
//! every recording client, a [`Sink`], converted to its format as far as
//! it has room ([`deliver`]).

use crate::{Converter, MAX_CHANNELS};

/// A playing client.
pub trait Source {
    fn converter(&mut self) -> &mut Converter;
    /// Gain per output channel (16.16).
    fn gains(&self) -> [u32; MAX_CHANNELS];
    /// Copy the frames the client has ready, from where the server last
    /// stopped, into `buf` (whole frames, as many as fit). Returns how
    /// many; nothing is used up yet.
    fn peek(&mut self, buf: &mut [u8]) -> usize;
    /// The server used `frames` of them.
    fn consume(&mut self, frames: usize);
}

/// A recording client.
pub trait Sink {
    fn converter(&mut self) -> &mut Converter;
    fn gains(&self) -> [u32; MAX_CHANNELS];
    /// Frames (in the client's format) it can take now.
    fn room(&self) -> usize;
    /// Append `data`, whole frames in the client's format.
    fn push(&mut self, data: &[u8]);
}

/// What one client gave to a period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mixed {
    /// Source frames used.
    pub consumed: usize,
    /// Output frames it reached (fewer than the period: it ran dry).
    pub produced: usize,
}

/// Mix `src` into `acc` (one period of the device's frames), with
/// `scratch` for the source's bytes on the way.
pub fn mix_source(src: &mut dyn Source, acc: &mut [i32], scratch: &mut [u8]) -> Mixed {
    let conv = *src.converter();
    let ch = conv.to().channels as usize;
    let fb = conv.from().frame_bytes();
    let frames = acc.len() / ch;
    let cap = scratch.len() / fb;
    let mut total = Mixed::default();
    if cap == 0 {
        return total;
    }
    let gains = src.gains();
    while total.produced < frames {
        let need = src.converter().input_needed(frames - total.produced).min(cap);
        let got = src.peek(&mut scratch[..need * fb]);
        if got == 0 {
            break;
        }
        let (used, made) =
            src.converter().mix(&scratch[..got * fb], &mut acc[total.produced * ch..frames * ch], &gains);
        src.consume(used);
        total.consumed += used;
        total.produced += made;
        if used == 0 && made == 0 {
            break;
        }
    }
    total
}

/// Hand `captured` (whole frames in the sink converter's source format)
/// to `sink`, converted through `scratch`. Returns the frames the sink
/// got; when it runs out of room the rest is not delivered (the sink's
/// stream has overrun).
pub fn deliver(sink: &mut dyn Sink, captured: &[u8], scratch: &mut [u8]) -> usize {
    let conv = *sink.converter();
    let in_fb = conv.from().frame_bytes();
    let out_fb = conv.to().frame_bytes();
    let gains = sink.gains();
    let mut at = 0;
    let mut delivered = 0;
    while at + in_fb <= captured.len() {
        let room = sink.room().min(scratch.len() / out_fb);
        if room == 0 {
            break;
        }
        let (used, made) = sink.converter().convert(&captured[at..], &mut scratch[..room * out_fb], &gains);
        if made > 0 {
            sink.push(&scratch[..made * out_fb]);
        }
        at += used * in_fb;
        delivered += made;
        if used == 0 {
            break;
        }
    }
    delivered
}
