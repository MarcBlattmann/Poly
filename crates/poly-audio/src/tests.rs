//! Host tests: sample conversions, channel maps, resampling (by the
//! frequency and level of a converted tone), mixing and capture delivery.

use crate::convert::render;
use crate::server::{Sink, Source, deliver, mix_source};
use crate::{Converter, FULL_SCALE, Format, MAX_CHANNELS, Sample, UNITY, gain_of};
extern crate std;
use std::vec;
use std::vec::Vec;

const ALL: [Sample; 12] = [
    Sample::U8,
    Sample::S8,
    Sample::S16,
    Sample::U16,
    Sample::S24,
    Sample::U24,
    Sample::S24_3,
    Sample::U24_3,
    Sample::S32,
    Sample::U32,
    Sample::Float,
    Sample::Float64,
];

fn enc(s: Sample, v: i32) -> Vec<u8> {
    let mut b = vec![0u8; s.bytes()];
    s.encode(v, &mut b);
    b
}

#[test]
fn integer_formats_map_onto_the_internal_scale() {
    assert_eq!(Sample::S16.decode(&0x4000i16.to_le_bytes()), FULL_SCALE / 2);
    assert_eq!(Sample::S16.decode(&i16::MIN.to_le_bytes()), -FULL_SCALE);
    assert_eq!(Sample::U8.decode(&[0x80]), 0);
    assert_eq!(Sample::U8.decode(&[0]), -FULL_SCALE);
    assert_eq!(Sample::U16.decode(&0xc000u16.to_le_bytes()), FULL_SCALE / 2);
    assert_eq!(Sample::S32.decode(&(1i32 << 30).to_le_bytes()), FULL_SCALE / 2);
    assert_eq!(Sample::U32.decode(&0u32.to_le_bytes()), -FULL_SCALE);
    assert_eq!(Sample::S24.decode(&0x00c0_0000u32.to_le_bytes()), -FULL_SCALE / 2);
    assert_eq!(Sample::S24_3.decode(&[0x00, 0x00, 0x40]), FULL_SCALE / 2);
    assert_eq!(Sample::U24_3.decode(&[0x00, 0x00, 0x80]), 0);
    assert_eq!(enc(Sample::S16, FULL_SCALE / 2), 0x4000i16.to_le_bytes());
    assert_eq!(enc(Sample::U8, 0), [0x80]);
    assert_eq!(enc(Sample::S32, -FULL_SCALE), i32::MIN.to_le_bytes());
    // Out of range values are clipped.
    assert_eq!(enc(Sample::S16, 4 * FULL_SCALE), i16::MAX.to_le_bytes());
    assert_eq!(enc(Sample::S16, -4 * FULL_SCALE), i16::MIN.to_le_bytes());
    assert_eq!(enc(Sample::U8, i32::MAX), [0xff]);
    // Silence.
    assert_eq!(Sample::U8.silence()[0], 0x80);
    assert_eq!(&Sample::U16.silence()[..2], &[0x00, 0x80]);
    assert_eq!(Sample::S16.silence(), [0; 8]);
}

#[test]
fn every_format_round_trips_at_its_precision() {
    for s in ALL {
        for v in [0, 1 << 16, -(1 << 16), FULL_SCALE / 3, -FULL_SCALE / 3, FULL_SCALE - (1 << 16), -FULL_SCALE] {
            let back = s.decode(&enc(s, v));
            let step = match s.bytes() {
                1 => 1 << 16,
                2 => 1 << 8,
                _ => 1,
            };
            assert!((back - v).abs() < step, "{s:?}: {v} came back as {back}");
        }
    }
}

#[test]
fn floats_are_built_and_taken_apart_exactly() {
    let f = |v: f32| Sample::Float.decode(&v.to_le_bytes());
    assert_eq!(f(0.5), FULL_SCALE / 2);
    assert_eq!(f(-0.25), -FULL_SCALE / 4);
    assert_eq!(f(0.0), 0);
    assert_eq!(f(-0.0), 0);
    assert_eq!(f(1.0), FULL_SCALE - 1);
    assert_eq!(f(-1.0), -FULL_SCALE);
    assert_eq!(f(7.5), FULL_SCALE - 1);
    assert_eq!(f(f32::INFINITY), FULL_SCALE - 1);
    assert_eq!(f(f32::NEG_INFINITY), -FULL_SCALE);
    assert_eq!(f(f32::NAN), 0);
    assert_eq!(f(1e-30), 0);
    let d = |v: f64| Sample::Float64.decode(&v.to_le_bytes());
    assert_eq!(d(0.75), 3 * FULL_SCALE / 4);
    assert_eq!(d(-0.5), -FULL_SCALE / 2);
    for v in [1, -1, 12345, -FULL_SCALE, FULL_SCALE - 1, 1 << 20] {
        let b = enc(Sample::Float, v);
        assert_eq!(f32::from_le_bytes(b.try_into().unwrap()), v as f32 / FULL_SCALE as f32);
        let b = enc(Sample::Float64, v);
        assert_eq!(f64::from_le_bytes(b.try_into().unwrap()), v as f64 / FULL_SCALE as f64);
    }
}

fn s16_frames(frames: &[[i16; 2]]) -> Vec<u8> {
    frames.iter().flat_map(|f| f.iter().flat_map(|s| s.to_le_bytes())).collect()
}

#[test]
fn channels_are_mapped() {
    let stereo = Format::new(Sample::S16, 2, 48000);
    let mono = Format::new(Sample::S16, 1, 48000);
    // Mono to stereo: both channels.
    let mut c = Converter::new(mono, stereo).unwrap();
    let mut acc = [0i32; 4];
    let input: Vec<u8> = [100i16, -200].iter().flat_map(|s| s.to_le_bytes()).collect();
    assert_eq!(c.mix(&input, &mut acc, &[]), (2, 2));
    assert_eq!(acc, [100 << 8, 100 << 8, -200 << 8, -200 << 8]);
    // Stereo to mono: the average.
    let mut c = Converter::new(stereo, mono).unwrap();
    let mut acc = [0i32; 1];
    assert_eq!(c.mix(&s16_frames(&[[100, 300]]), &mut acc, &[]), (1, 1));
    assert_eq!(acc, [200 << 8]);
    // Six channels to stereo: the first two.
    let six = Format::new(Sample::S16, 6, 48000);
    let mut c = Converter::new(six, stereo).unwrap();
    let input: Vec<u8> = [1i16, 2, 3, 4, 5, 6].iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut acc = [0i32; 2];
    c.mix(&input, &mut acc, &[]);
    assert_eq!(acc, [1 << 8, 2 << 8]);
    assert!(Converter::new(Format::new(Sample::S16, 0, 48000), stereo).is_none());
    assert!(Converter::new(Format::new(Sample::S16, 9, 48000), stereo).is_none());
    assert!(Converter::new(stereo, Format::new(Sample::S16, 2, 0)).is_none());
}

/// A sine of `hz` at `rate`, `frames` long, amplitude `amp` (of 1.0), as
/// stereo S16.
fn sine(rate: u32, hz: f64, frames: usize, amp: f64) -> Vec<u8> {
    (0..frames)
        .flat_map(|i| {
            let v = (amp * 32767.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()).round() as i16;
            [v, v].into_iter().flat_map(|s| s.to_le_bytes())
        })
        .collect()
}

/// Frequency (upward zero crossings) and RMS of channel 0 of `acc`
/// (stereo frames in the internal domain) at `rate`.
fn measure(acc: &[i32], rate: u32) -> (f64, f64) {
    let ch0: Vec<f64> = acc.chunks(2).map(|f| f[0] as f64).collect();
    let rms = (ch0.iter().map(|v| v * v).sum::<f64>() / ch0.len() as f64).sqrt();
    let mut first = None;
    let mut last = 0.0;
    let mut n = 0;
    for i in 1..ch0.len() {
        let (a, b) = (ch0[i - 1], ch0[i]);
        if a < 0.0 && b >= 0.0 {
            let t = (i - 1) as f64 + a / (a - b);
            first.get_or_insert(t);
            last = t;
            n += 1;
        }
    }
    let hz = (n - 1) as f64 * rate as f64 / (last - first.unwrap());
    (hz, rms)
}

#[test]
fn resampling_keeps_a_tone_at_its_frequency_and_level() {
    for (from, to, hz) in [(44100, 48000, 1000.0), (22050, 48000, 660.0), (48000, 44100, 440.0), (8000, 48000, 300.0)] {
        let input = sine(from, hz, from as usize, 0.5);
        let mut c = Converter::new(Format::new(Sample::S16, 2, from), Format::new(Sample::S16, 2, to)).unwrap();
        let mut acc = vec![0i32; to as usize * 2];
        // In pieces of 1000 output frames, as the server would ask.
        let mut at = 0;
        let mut made = 0;
        while made < to as usize - 1000 {
            let need = c.input_needed(1000);
            let end = (at + need * 4).min(input.len());
            let (used, m) = c.mix(&input[at..end], &mut acc[made * 2..(made + 1000) * 2], &[]);
            assert!(used <= need, "used {used} of the {need} it asked for");
            at += used * 4;
            made += m;
            assert_eq!(m, 1000, "{from} -> {to}: a full piece from enough input");
        }
        let (f, rms) = measure(&acc[..made * 2], to);
        let want_rms = 0.5 * FULL_SCALE as f64 / std::f64::consts::SQRT_2;
        assert!((f - hz).abs() < hz * 0.002, "{from} -> {to}: {f} Hz, wanted {hz}");
        assert!((rms - want_rms).abs() < want_rms * 0.02, "{from} -> {to}: rms {rms}, wanted {want_rms}");
    }
}

#[test]
fn equal_rates_pass_frames_through_without_delay() {
    let f = Format::new(Sample::S16, 2, 48000);
    let mut c = Converter::new(f, f).unwrap();
    let input = s16_frames(&[[1, 2], [3, 4], [5, 6]]);
    let mut out = [0u8; 8];
    assert_eq!(c.convert(&input, &mut out, &[]), (2, 2));
    assert_eq!(&out[..], &input[..8]);
    assert_eq!(c.input_needed(5), 5);
}

#[test]
fn a_stream_converted_in_pieces_equals_one_converted_whole() {
    let input = sine(44100, 523.0, 4410, 0.7);
    let fmt_in = Format::new(Sample::S16, 2, 44100);
    let fmt_out = Format::new(Sample::Float, 2, 48000);
    let mut whole = Converter::new(fmt_in, fmt_out).unwrap();
    let mut out_whole = vec![0u8; 4800 * 8];
    let (_, n) = whole.convert(&input, &mut out_whole, &[]);
    let mut parts = Converter::new(fmt_in, fmt_out).unwrap();
    let mut out_parts = vec![0u8; 4800 * 8];
    let (mut at, mut made) = (0, 0);
    for piece in [1usize, 7, 100, 333, 1000, 5000].iter().cycle().take(40) {
        let end = (at + piece * 4).min(input.len());
        let (u, m) = parts.convert(&input[at..end], &mut out_parts[made * 8..], &[]);
        at += u * 4;
        made += m;
    }
    assert_eq!(made, n);
    assert_eq!(out_parts[..made * 8], out_whole[..n * 8]);
    // After a reset it starts over.
    parts.reset();
    let mut again = vec![0u8; 16];
    parts.convert(&input, &mut again, &[]);
    assert_eq!(again[..], out_whole[..16]);
}

#[test]
fn gains_and_volume_values() {
    assert_eq!(gain_of(100, 100), UNITY);
    assert_eq!(gain_of(50, 100), UNITY / 2);
    assert_eq!(gain_of(0, 100), 0);
    assert_eq!(gain_of(500, 100), UNITY);
    let f = Format::new(Sample::S16, 2, 48000);
    let mut c = Converter::new(f, f).unwrap();
    let mut acc = [0i32; 2];
    c.mix(&s16_frames(&[[1000, 1000]]), &mut acc, &[UNITY / 2, 0]);
    assert_eq!(acc, [500 << 8, 0]);
    let mut out = [0u8; 4];
    render(&[FULL_SCALE / 2, FULL_SCALE / 2], 2, &[UNITY, UNITY / 4], Sample::S16, &mut out);
    assert_eq!(out, [0x00, 0x40, 0x00, 0x10]);
}

/// A client ring for the server tests.
struct Client {
    conv: Converter,
    data: Vec<u8>,
    at: usize,
    gain: u32,
    room: usize,
}

impl Source for Client {
    fn converter(&mut self) -> &mut Converter {
        &mut self.conv
    }
    fn gains(&self) -> [u32; MAX_CHANNELS] {
        [self.gain; MAX_CHANNELS]
    }
    fn peek(&mut self, buf: &mut [u8]) -> usize {
        let fb = self.conv.from().frame_bytes();
        let n = (buf.len() / fb).min((self.data.len() - self.at) / fb);
        buf[..n * fb].copy_from_slice(&self.data[self.at..self.at + n * fb]);
        n
    }
    fn consume(&mut self, frames: usize) {
        self.at += frames * self.conv.from().frame_bytes();
    }
}

impl Sink for Client {
    fn converter(&mut self) -> &mut Converter {
        &mut self.conv
    }
    fn gains(&self) -> [u32; MAX_CHANNELS] {
        [self.gain; MAX_CHANNELS]
    }
    fn room(&self) -> usize {
        self.room
    }
    fn push(&mut self, data: &[u8]) {
        self.room -= data.len() / self.conv.to().frame_bytes();
        self.data.extend_from_slice(data);
    }
}

#[test]
fn two_clients_mix_into_one_period() {
    let hw = Format::new(Sample::S16, 2, 48000);
    let a = Format::new(Sample::S16, 2, 48000);
    let b = Format::new(Sample::U8, 1, 48000);
    let mut ca = Client {
        conv: Converter::new(a, hw).unwrap(),
        data: s16_frames(&[[1000, -1000]; 480]),
        at: 0,
        gain: UNITY,
        room: 0,
    };
    // b has only 100 frames: the rest of its share is silence.
    let mut cb =
        Client { conv: Converter::new(b, hw).unwrap(), data: vec![0x90; 100], at: 0, gain: UNITY / 2, room: 0 };
    let mut acc = vec![0i32; 240 * 2];
    let mut scratch = [0u8; 64]; // smaller than a period: the server loops
    let ma = mix_source(&mut ca, &mut acc, &mut scratch);
    let mb = mix_source(&mut cb, &mut acc, &mut scratch);
    assert_eq!((ma.consumed, ma.produced), (240, 240));
    assert_eq!((mb.consumed, mb.produced), (100, 100));
    assert_eq!(ca.at, 240 * 4);
    let b_level = (0x10 << 16) / 2;
    assert_eq!(acc[0], (1000 << 8) + b_level);
    assert_eq!(acc[1], (-1000 << 8) + b_level);
    assert_eq!(acc[2 * 150], 1000 << 8);
    let mut out = vec![0u8; 240 * 4];
    render(&acc, 2, &[UNITY, UNITY], Sample::S16, &mut out);
    assert_eq!(i16::from_le_bytes([out[0], out[1]]), 1000 + 0x10 * 128);
    // A client with nothing to give.
    let mut empty = Client { conv: Converter::new(a, hw).unwrap(), data: Vec::new(), at: 0, gain: UNITY, room: 0 };
    assert_eq!(mix_source(&mut empty, &mut acc, &mut scratch), Default::default());
}

#[test]
fn captured_periods_reach_each_client_in_its_format() {
    let hw = Format::new(Sample::S16, 2, 48000);
    let captured = sine(48000, 1000.0, 960, 0.5);
    let mut mono = Client {
        conv: Converter::new(hw, Format::new(Sample::S32, 1, 48000)).unwrap(),
        data: Vec::new(),
        at: 0,
        gain: UNITY,
        room: 10_000,
    };
    let mut scratch = [0u8; 256];
    assert_eq!(deliver(&mut mono, &captured, &mut scratch), 960);
    assert_eq!(mono.data.len(), 960 * 4);
    let first = i32::from_le_bytes(mono.data[4..8].try_into().unwrap());
    assert_eq!(first, (i16::from_le_bytes([captured[4], captured[5]]) as i32) << 16);
    // A client with room for 100 frames gets 100.
    let mut small = Client { conv: Converter::new(hw, hw).unwrap(), data: Vec::new(), at: 0, gain: UNITY, room: 100 };
    assert_eq!(deliver(&mut small, &captured, &mut scratch), 100);
    assert_eq!(small.data[..], captured[..400]);
    // Resampled to 22.05 kHz: about 441 frames.
    let mut slow = Client {
        conv: Converter::new(hw, Format::new(Sample::S16, 2, 22050)).unwrap(),
        data: Vec::new(),
        at: 0,
        gain: UNITY,
        room: 10_000,
    };
    let n = deliver(&mut slow, &captured, &mut scratch);
    assert!((440..=442).contains(&n), "{n}");
}
