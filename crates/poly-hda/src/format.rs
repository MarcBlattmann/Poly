//! Stream formats (HDA 1.0a §3.7.1) and the PCM size / rate bits of the
//! Supported PCM Size, Rates parameter (§7.3.4.7).
//!
//! The 16-bit format word, used both in SDnFMT and by the Set Converter
//! Format verb:
//!
//! | bits  | field |
//! |-------|-------|
//! | 15    | TYPE: 0 PCM, 1 non-PCM |
//! | 14    | BASE: 0 48 kHz, 1 44.1 kHz |
//! | 13:11 | MULT: ×1 ×2 ×3 ×4 (000..011; others reserved) |
//! | 10:8  | DIV: ÷1 .. ÷8 (000..111) |
//! | 6:4   | BITS: 8, 16, 20, 24, 32 (000..100; others reserved) |
//! | 3:0   | CHAN: channels − 1 |
//!
//! In memory, 8- and 16-bit samples take one and two bytes, 20-, 24- and
//! 32-bit samples a 32-bit container each.

/// A PCM stream format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StreamFormat {
    /// Sample rate in Hz: a base of 48000 or 44100, times 1..=4, divided by
    /// 1..=8.
    pub rate: u32,
    /// Bits per sample: 8, 16, 20, 24 or 32.
    pub bits: u8,
    /// Channels per frame, 1..=16.
    pub channels: u8,
}

const BASES: [u32; 2] = [48_000, 44_100];
const TYPE_NON_PCM: u16 = 1 << 15;
const BASE_44K1: u16 = 1 << 14;

/// The sample sizes, by BITS code.
const SIZES: [u8; 5] = [8, 16, 20, 24, 32];

impl StreamFormat {
    pub const fn new(rate: u32, bits: u8, channels: u8) -> StreamFormat {
        StreamFormat { rate, bits, channels }
    }

    /// The format word, or `None` if the rate, size or channel count cannot
    /// be expressed. Of several encodings of one rate the smallest
    /// multiplier and divisor on the 48 kHz base win.
    pub fn encode(&self) -> Option<u16> {
        if !(1..=16).contains(&self.channels) {
            return None;
        }
        let bits = SIZES.iter().position(|&b| b == self.bits)? as u16;
        for (b, &base) in BASES.iter().enumerate() {
            for mult in 1..=4u64 {
                for div in 1..=8u64 {
                    if base as u64 * mult == self.rate as u64 * div {
                        let base_bit = if b == 1 { BASE_44K1 } else { 0 };
                        return Some(
                            base_bit
                                | ((mult as u16 - 1) << 11)
                                | ((div as u16 - 1) << 8)
                                | (bits << 4)
                                | (self.channels as u16 - 1),
                        );
                    }
                }
            }
        }
        None
    }

    /// The PCM format a word describes; `None` for non-PCM formats,
    /// reserved MULT or BITS codes, and rates that are not whole hertz.
    pub fn decode(v: u16) -> Option<StreamFormat> {
        if v & TYPE_NON_PCM != 0 {
            return None;
        }
        let base = if v & BASE_44K1 != 0 { BASES[1] } else { BASES[0] };
        let mult = ((v >> 11) & 7) as u32;
        if mult > 3 {
            return None;
        }
        let div = ((v >> 8) & 7) as u32 + 1;
        let scaled = base * (mult + 1);
        if !scaled.is_multiple_of(div) {
            return None;
        }
        let bits = *SIZES.get(((v >> 4) & 7) as usize)?;
        Some(StreamFormat { rate: scaled / div, bits, channels: (v & 0xF) as u8 + 1 })
    }

    /// Bytes one sample takes in memory (the container size).
    pub fn container_bytes(&self) -> Option<u32> {
        match self.bits {
            8 => Some(1),
            16 => Some(2),
            20 | 24 | 32 => Some(4),
            _ => None,
        }
    }

    /// Bytes per frame (one sample of every channel) in memory.
    pub fn frame_bytes(&self) -> Option<u32> {
        Some(self.container_bytes()? * self.channels as u32)
    }
}

/// The rates of the PCM rate bits R1..R12 (bits 11:0 of parameter 0x0A,
/// §7.3.4.7), bit 0 first.
pub const RATES: [u32; 12] = [8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000, 384000];

/// The rate bit (in bits 11:0) for `hz`.
pub fn rate_bit(hz: u32) -> Option<u32> {
    RATES.iter().position(|&r| r == hz).map(|i| 1 << i)
}

/// The rates whose bits are set in `bits` (bits 11:0), ascending.
pub fn rates_hz(bits: u32) -> impl Iterator<Item = u32> {
    RATES.iter().enumerate().filter(move |(i, _)| bits & (1 << i) != 0).map(|(_, &r)| r)
}

/// The size bit for `bits` per sample, in the layout of
/// [`crate::OutputPath::sizes`]: bit 0 8-bit, 1 16-bit, 2 20-bit, 3 24-bit,
/// 4 32-bit (parameter 0x0A bits 20:16 shifted down).
pub fn size_bit(bits: u8) -> Option<u32> {
    SIZES.iter().position(|&b| b == bits).map(|i| 1 << i)
}

/// The sample sizes whose bits are set in `sizes` (layout as
/// [`size_bit`]), ascending.
pub fn sizes_bits(sizes: u32) -> impl Iterator<Item = u8> {
    SIZES.iter().enumerate().filter(move |(i, _)| sizes & (1 << i) != 0).map(|(_, &b)| b)
}

/// Parameter 0x0A: rate bits are 11:0, size bits 20:16.
pub const PCM_RATES_MASK: u32 = 0xFFF;
pub const PCM_SIZES_SHIFT: u32 = 16;
pub const PCM_SIZES_MASK: u32 = 0x1F;

/// Parameter 0x0B (Supported Stream Formats, §7.3.4.8): bit 0 PCM.
pub const STREAM_FORMAT_PCM: u32 = 1 << 0;
