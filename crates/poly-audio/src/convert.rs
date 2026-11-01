//! Converting a stream from one [`Format`] to another: channels, rate and
//! gain.
//!
//! **Channels.** Equal counts map one to one. A mono source feeds every
//! output channel; a mono output gets the average of the source's
//! channels. Otherwise output channel `c` takes source channel `c`, and
//! channels the source lacks are silent.
//!
//! **Rate.** Linear interpolation between neighbouring source frames. The
//! phase is 32.32 fixed point: `step = from_rate / to_rate`, and each
//! output frame lies `frac` of the way from the previous source frame to
//! the current one. The first source frame is output as is; from then on
//! output frame `k` sits at source position `k * step`, one frame behind
//! (the interpolation needs the frame after it). Equal rates skip the
//! interpolation and lose no frame. Linear interpolation is cheap and
//! keeps tones at their frequency; its images above the passband are left
//! for the device's own filtering.
//!
//! A converter consumes source frames as far as it needs them and keeps
//! its place between calls, so a stream can be converted in pieces of any
//! size; [`Converter::input_needed`] says how many source frames are
//! enough for a number of output frames.

use crate::{Format, MAX_CHANNELS, apply_gain};

const ONE: u64 = 1 << 32;

type Frame = [i32; MAX_CHANNELS];

/// A conversion in progress. See the [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Converter {
    from: Format,
    to: Format,
    /// Source frames per output frame, 32.32.
    step: u64,
    /// Position between `prev` (0) and `cur` (ONE), 32.32; at or past ONE
    /// the next source frame is due.
    frac: u64,
    prev: Frame,
    cur: Frame,
    /// No source frame taken yet.
    fresh: bool,
}

impl Converter {
    /// A converter from `from` to `to`; `None` unless both formats are
    /// valid.
    pub fn new(from: Format, to: Format) -> Option<Converter> {
        if !from.is_valid() || !to.is_valid() {
            return None;
        }
        let step = ((from.rate as u64) << 32) / to.rate as u64;
        Some(Converter { from, to, step, frac: 0, prev: [0; MAX_CHANNELS], cur: [0; MAX_CHANNELS], fresh: true })
    }

    pub const fn from(&self) -> Format {
        self.from
    }

    pub const fn to(&self) -> Format {
        self.to
    }

    /// Start over, as for a new stream.
    pub fn reset(&mut self) {
        self.frac = 0;
        self.prev = [0; MAX_CHANNELS];
        self.cur = [0; MAX_CHANNELS];
        self.fresh = true;
    }

    fn same_rate(&self) -> bool {
        self.from.rate == self.to.rate
    }

    /// Source frames that are certainly enough to make `out_frames` output
    /// frames from where the converter is (it may need fewer).
    pub fn input_needed(&self, out_frames: usize) -> usize {
        if out_frames == 0 {
            return 0;
        }
        if self.same_rate() {
            return out_frames;
        }
        let span = self.frac as u128 + (out_frames as u128 - 1) * self.step as u128;
        (span >> 32) as usize + 2
    }

    /// Source frame `i` of `input`, decoded and mapped to the output's
    /// channels.
    fn load(&self, input: &[u8], i: usize) -> Frame {
        let fb = self.from.frame_bytes();
        let ss = self.from.sample.bytes();
        let base = &input[i * fb..(i + 1) * fb];
        let (n, m) = (self.from.channels as usize, self.to.channels as usize);
        let at = |c: usize| self.from.sample.decode(&base[c * ss..]);
        let mut f = [0i32; MAX_CHANNELS];
        if n == m {
            for (c, v) in f.iter_mut().take(m).enumerate() {
                *v = at(c);
            }
        } else if n == 1 {
            let v = at(0);
            f[..m].fill(v);
        } else if m == 1 {
            let sum: i64 = (0..n).map(|c| at(c) as i64).sum();
            f[0] = (sum / n as i64) as i32;
        } else {
            for (c, v) in f.iter_mut().take(m.min(n)).enumerate() {
                *v = at(c);
            }
        }
        f
    }

    /// The next output frame from `input` starting at source frame
    /// `*pos` (moved past what was used), or `None` when `input` is used
    /// up.
    fn next(&mut self, input: &[u8], pos: &mut usize) -> Option<Frame> {
        let have = input.len() / self.from.frame_bytes();
        if self.same_rate() {
            if *pos >= have {
                return None;
            }
            let f = self.load(input, *pos);
            *pos += 1;
            return Some(f);
        }
        if self.fresh {
            if *pos >= have {
                return None;
            }
            self.cur = self.load(input, *pos);
            self.prev = self.cur;
            *pos += 1;
            self.frac = 0;
            self.fresh = false;
        }
        while self.frac >= ONE {
            if *pos >= have {
                return None;
            }
            self.prev = self.cur;
            self.cur = self.load(input, *pos);
            *pos += 1;
            self.frac -= ONE;
        }
        let mut out = [0i32; MAX_CHANNELS];
        let t = self.frac as i64; // 0..ONE
        for (c, o) in out.iter_mut().take(self.to.channels as usize).enumerate() {
            let (a, b) = (self.prev[c] as i64, self.cur[c] as i64);
            *o = (a + (((b - a) * t) >> 32)) as i32;
        }
        self.frac += self.step;
        Some(out)
    }

    /// Mix `input` (whole source frames) into `acc` (output frames of
    /// `to.channels` samples each), each channel times its gain from
    /// `gains` (16.16; missing gains are unity). Returns the source frames
    /// consumed and the output frames produced; stops when either runs
    /// out.
    pub fn mix(&mut self, input: &[u8], acc: &mut [i32], gains: &[u32]) -> (usize, usize) {
        let m = self.to.channels as usize;
        let frames = acc.len() / m;
        let mut pos = 0;
        let mut made = 0;
        while made < frames {
            let Some(f) = self.next(input, &mut pos) else { break };
            for (c, &v) in f.iter().take(m).enumerate() {
                let g = gains.get(c).copied().unwrap_or(crate::UNITY);
                let slot = &mut acc[made * m + c];
                *slot = slot.saturating_add(apply_gain(v, g));
            }
            made += 1;
        }
        (pos, made)
    }

    /// Convert `input` into `out` (whole output frames in the output
    /// format), each channel times its gain. Returns the source frames
    /// consumed and the output frames written.
    pub fn convert(&mut self, input: &[u8], out: &mut [u8], gains: &[u32]) -> (usize, usize) {
        let m = self.to.channels as usize;
        let ss = self.to.sample.bytes();
        let fb = self.to.frame_bytes();
        let frames = out.len() / fb;
        let mut pos = 0;
        let mut made = 0;
        while made < frames {
            let Some(f) = self.next(input, &mut pos) else { break };
            for (c, &v) in f.iter().take(m).enumerate() {
                let g = gains.get(c).copied().unwrap_or(crate::UNITY);
                let at = made * fb + c * ss;
                self.to.sample.encode(apply_gain(v, g), &mut out[at..at + ss]);
            }
            made += 1;
        }
        (pos, made)
    }
}

/// Clip and encode the accumulated frames `acc` as `sample`s into `out`
/// (as many as fit), each channel of `channels` times its gain.
pub fn render(acc: &[i32], channels: usize, gains: &[u32], sample: crate::Sample, out: &mut [u8]) {
    let ss = sample.bytes();
    for (i, (v, o)) in acc.iter().zip(out.chunks_exact_mut(ss)).enumerate() {
        let g = gains.get(i % channels.max(1)).copied().unwrap_or(crate::UNITY);
        sample.encode(apply_gain(*v, g), o);
    }
}
