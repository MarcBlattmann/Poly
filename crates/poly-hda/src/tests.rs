//! Tests of the format word, the graph search and the driver against the
//! simulated controller in `sim`.

use super::*;
use crate::codec::{WidgetType, command, long, short};
use crate::regs::sd;
use crate::sim::{Codec, DmaMemory, Regs, Sim, caps, config, pincaps};
use poly_pci::Mmio;
extern crate std;
use std::{vec, vec::Vec};

const BUF: usize = 16 * 1024;

/// The DMA memory of one controller.
struct Rig {
    sim: Sim,
    corb: DmaMemory,
    rirb: DmaMemory,
    bdl: DmaMemory,
    buf: DmaMemory,
}

impl Rig {
    fn new(codecs: Vec<Codec>) -> Rig {
        Rig {
            sim: Sim::new(codecs),
            corb: DmaMemory::new(1024),
            rirb: DmaMemory::new(2048),
            bdl: DmaMemory::new(16 * MAX_BDL_ENTRIES),
            buf: DmaMemory::new(BUF),
        }
    }

    fn memory(&self) -> Memory {
        Memory { corb: self.corb.region(), rirb: self.rirb.region(), bdl: self.bdl.region(), buffer: self.buf.region() }
    }

    fn init(&self) -> Result<Hda<Regs>, InitError> {
        // SAFETY: the regions live in `self`, which outlives the driver in
        // every test; the simulated device uses host addresses.
        unsafe { init(self.sim.regs(), self.memory(), &mut nodelay()) }
    }
}

fn nodelay() -> impl FnMut(u32) {
    |_| {}
}

/// DAC 2 → mixer 3 → line-out pin 4 (with EAPD), plus a mic pin 5.
fn simple_codec(addr: u8) -> Codec {
    let mut c = Codec::new(addr, 1);
    c.dac(2).mixer(3, &[2]).pin(4, &[3], pincaps::OUT | pincaps::EAPD, config::LINE_OUT).pin(
        5,
        &[],
        pincaps::IN,
        config::MIC,
    );
    c
}

// ---- formats ---------------------------------------------------------------

#[test]
fn format_words() {
    let cases: [(u32, u8, u8, u16); 8] = [
        (48000, 16, 2, 0x0011),
        (44100, 16, 2, 0x4011),
        (22050, 16, 2, 0x4111),
        (96000, 24, 2, 0x0831),
        (8000, 8, 1, 0x0500),
        (192000, 32, 8, 0x1847),
        (32000, 20, 2, 0x0A21),
        (11025, 16, 1, 0x4310),
    ];
    for (rate, bits, ch, word) in cases {
        let f = StreamFormat::new(rate, bits, ch);
        assert_eq!(f.encode(), Some(word), "{f:?}");
        assert_eq!(StreamFormat::decode(word), Some(f));
    }
    assert_eq!(StreamFormat::new(48000, 16, 0).encode(), None);
    assert_eq!(StreamFormat::new(48000, 16, 17).encode(), None);
    assert_eq!(StreamFormat::new(48000, 12, 2).encode(), None);
    assert_eq!(StreamFormat::new(12345, 16, 2).encode(), None);
    assert_eq!(StreamFormat::new(48000, 16, 16).encode(), Some(0x001F));
    // Non-PCM, reserved MULT, reserved BITS, fractional rate.
    assert_eq!(StreamFormat::decode(0x8011), None);
    assert_eq!(StreamFormat::decode(0x2011), None);
    assert_eq!(StreamFormat::decode(0x0051), None);
    assert_eq!(StreamFormat::decode(0x0611), None); // 48000 / 7
    assert_eq!(StreamFormat::new(48000, 24, 2).frame_bytes(), Some(8));
    assert_eq!(StreamFormat::new(48000, 16, 2).frame_bytes(), Some(4));
    assert_eq!(format::rates_hz(0x070).collect::<Vec<_>>(), [32000, 44100, 48000]);
    assert_eq!(format::sizes_bits(0b00110).collect::<Vec<_>>(), [16, 20]);
}

#[test]
fn command_words() {
    assert_eq!(short(codec::verb::GET_PARAMETER, 0x09), 0xF0009);
    assert_eq!(long(codec::verb::SET_AMP_GAIN_MUTE, 0xB027), 0x3B027);
    assert_eq!(command(2, 0x14, 0xF0009), 0x214F_0009);
    assert_eq!(codec::amp::set(true, 0, false, 0x27), 0xB027);
    assert_eq!(codec::amp::set(false, 3, true, 0), 0x7380);
    let w = codec::WidgetCaps(caps::MIXER);
    assert_eq!(w.kind(), WidgetType::Mixer);
    assert_eq!(w.channels(), 2);
    assert!(w.in_amp() && w.out_amp() && w.conn_list() && !w.power_control());
    assert_eq!(codec::WidgetCaps(0x0000_E001).channels(), 16);
    let cfg = codec::ConfigDefault(config::disconnected(2));
    assert_eq!(cfg.connectivity(), codec::Connectivity::None);
    assert_eq!(cfg.device(), codec::device::HP_OUT);
    let a = codec::AmpCaps(crate::sim::OUT_AMP_CAPS);
    assert_eq!((a.offset(), a.steps(), a.step_size(), a.mute_capable()), (0x27, 0x3F, 5, true));
    assert_eq!(Gcap::decode(0x4401).output_streams, 4);
    assert_eq!(Gcap::decode(0x4401).input_streams, 4);
    assert!(Gcap::decode(0x4401).addr64);
}

// ---- init and the output path ----------------------------------------------

#[test]
fn init_sets_up_dac_mixer_pin() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.codecs(), 1);
    assert_eq!(rig.sim.dev().resets, 1);
    let p = *hda.output();
    assert_eq!((p.codec, p.afg, p.dac, p.pin), (0, 1, 2, 4));
    assert_eq!(p.nodes(), &[2, 3, 4]);
    assert_eq!(p.inputs[..3], [0, 0, 0]);
    assert_eq!(p.max_channels, 2);
    assert_eq!(p.rates_hz().collect::<Vec<_>>(), [32000, 44100, 48000]);
    assert_eq!(p.sample_bits().collect::<Vec<_>>(), [16, 20]);
    assert!(p.supports(&StreamFormat::new(48000, 16, 2)));
    assert!(!p.supports(&StreamFormat::new(96000, 16, 2)));
    assert!(!p.supports(&StreamFormat::new(48000, 24, 2)));
    assert!(!p.supports(&StreamFormat::new(48000, 16, 3)));
    assert_eq!(hda.widgets().len(), 4);
    assert_eq!(hda.vendor_id(), 0x1AF4_0022);
    let mut seen = Vec::new();
    hda.each_widget(|c, nid, t, _| seen.push((c, nid, t)));
    assert_eq!(seen[1], (0, 3, WidgetType::Mixer));

    let c = rig.sim.codec(0);
    assert_eq!(c.node(1).power, 0, "AFG in D0");
    assert_eq!(c.node(2).power, 0, "DAC in D0");
    assert_eq!(c.node(4).power, 0, "pin in D0");
    assert!(c.node(2).amp_is(true, 0, 0x27), "DAC output amp at 0 dB, unmuted");
    assert!(c.node(3).amp_is(true, 0, 0x27), "mixer output amp");
    assert!(c.node(3).amp_is(false, 0, 0x17), "mixer input amp of the DAC input");
    assert!(c.node(4).amp_is(true, 0, 0x27), "pin output amp");
    assert_eq!(c.node(4).pin_ctl, codec::pinctl::OUT_EN);
    assert_eq!(c.node(4).eapd, codec::eapd::EAPD);
    assert_eq!(c.node(5).pin_ctl, 0, "mic pin untouched");
    // Rings and interrupts.
    assert_eq!(rig.sim.reg8(regs::CORBCTL) & regs::corbctl::RUN, regs::corbctl::RUN);
    assert_eq!(rig.sim.reg8(regs::RIRBCTL), regs::rirbctl::DMAEN | regs::rirbctl::RINTCTL);
    assert_eq!(rig.sim.reg8(regs::CORBSIZE) & 3, regs::ringsize::SIZE_256);
    assert_eq!(rig.sim.reg32(regs::INTCTL), 0);
    assert_eq!(rig.sim.reg32(regs::CORBLBASE) as u64, rig.corb.region().bus & 0xFFFF_FFFF);
    assert!(hda.stats().verbs > 10);
    assert_eq!(hda.stats().timeouts, 0);
}

#[test]
fn init_waits_for_codecs_after_reset() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut total = 0u64;
    let mut d = |us: u32| total += us as u64;
    // SAFETY: as in `Rig::init`.
    let hda = unsafe { init(rig.sim.regs(), rig.memory(), &mut d) };
    assert!(hda.is_ok());
    assert!(total >= 521 + 100, "waited {total} µs");
}

#[test]
fn selector_and_headphone_pin() {
    // DAC 2; selector 3 picks from [mic 6, DAC 2]; line-out pin 4 has no
    // connection; HP pin 5 picks from [mic 6, selector 3]; mic pin 6.
    let mut c = Codec::new(0, 1);
    c.dac(2)
        .selector(3, &[6, 2])
        .pin(4, &[2], pincaps::OUT, config::disconnected(0))
        .pin(5, &[6, 3], pincaps::OUT | pincaps::HP, config::HP_OUT)
        .pin(6, &[], pincaps::IN, config::MIC);
    let rig = Rig::new(vec![c]);
    let hda = rig.init().unwrap();
    let p = *hda.output();
    assert_eq!(p.pin, 5);
    assert_eq!(p.nodes(), &[2, 3, 5]);
    assert_eq!(p.inputs[1..3], [1, 1]);
    assert_eq!(p.rank, 2);
    let c = rig.sim.codec(0);
    assert_eq!(c.node(3).select, 1);
    assert_eq!(c.node(5).select, 1);
    assert_eq!(c.node(5).pin_ctl, codec::pinctl::OUT_EN | codec::pinctl::HP_EN);
    assert_eq!(c.node(4).pin_ctl, 0, "disconnected pin untouched");
    assert_eq!(c.node(5).eapd, 0, "no EAPD capability");
}

#[test]
fn pin_straight_to_dac() {
    let mut c = Codec::new(0, 1);
    c.dac(2).pin(3, &[2], pincaps::OUT, config::SPEAKER);
    let rig = Rig::new(vec![c]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.output().nodes(), &[2, 3]);
    assert_eq!(hda.output().rank, 1);
    assert_eq!(rig.sim.codec(0).node(3).pin_ctl, codec::pinctl::OUT_EN);
}

#[test]
fn line_out_beats_speaker_and_headphone() {
    let mut c = Codec::new(0, 1);
    c.dac(2).pin(3, &[2], pincaps::OUT | pincaps::HP, config::HP_OUT).pin(4, &[2], pincaps::OUT, config::SPEAKER).pin(
        5,
        &[2],
        pincaps::OUT,
        config::LINE_OUT,
    );
    let rig = Rig::new(vec![c]);
    assert_eq!(rig.init().unwrap().output().pin, 5);
}

#[test]
fn no_output_pin() {
    let mut c = Codec::new(0, 1);
    c.dac(2).pin(3, &[2], pincaps::IN, config::MIC).pin(4, &[2], pincaps::OUT, config::disconnected(0));
    let rig = Rig::new(vec![c]);
    assert_eq!(rig.init().err(), Some(InitError::NoOutputPath));
    // An output pin with no way to a DAC.
    let mut c = Codec::new(0, 1);
    c.dac(2).mixer(3, &[]).pin(4, &[3], pincaps::OUT, config::LINE_OUT);
    let rig = Rig::new(vec![c]);
    assert_eq!(rig.init().err(), Some(InitError::NoOutputPath));
}

#[test]
fn no_codec() {
    let rig = Rig::new(vec![]);
    assert_eq!(rig.init().err(), Some(InitError::NoCodec));
}

#[test]
fn silent_codec_times_out_bounded() {
    let mut c = simple_codec(0);
    c.responsive = false;
    let rig = Rig::new(vec![c]);
    let mut total = 0u64;
    let mut d = |us: u32| total += us as u64;
    // SAFETY: as in `Rig::init`.
    let r = unsafe { init(rig.sim.regs(), rig.memory(), &mut d) };
    assert_eq!(r.err(), Some(InitError::Timeout));
    assert!(total < 100_000, "waited {total} µs");
    assert_eq!(rig.sim.codec(0).log.len(), 1, "gave up after the first command");
}

#[test]
fn silent_second_codec_is_skipped() {
    let mut dead = simple_codec(3);
    dead.responsive = false;
    let rig = Rig::new(vec![simple_codec(0), dead]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.codecs(), 0b1001);
    assert_eq!(hda.output().codec, 0);
    assert_eq!(hda.stats().timeouts, 1);
}

#[test]
fn stuck_reset_times_out() {
    let rig = Rig::new(vec![simple_codec(0)]);
    rig.sim.dev().stuck_in_reset = true;
    assert_eq!(rig.init().err(), Some(InitError::Timeout));
}

#[test]
fn bad_memory_is_refused() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut m = rig.memory();
    m.rirb.bus += 64;
    // SAFETY: refused before any DMA is set up.
    assert_eq!(unsafe { init(rig.sim.regs(), m, &mut nodelay()) }.err(), Some(InitError::Misaligned));
    let mut m = rig.memory();
    m.corb.len = 4;
    // SAFETY: as above.
    assert_eq!(unsafe { init(rig.sim.regs(), m, &mut nodelay()) }.err(), Some(InitError::NoRoom));
    // A controller without 64-bit addressing (GCAP is latched at reset)
    // and a buffer above 4 GiB.
    let rig = Rig::new(vec![simple_codec(0)]);
    rig.sim.dev().gcap = 0x4400;
    let r = rig.sim.regs();
    r.write32(regs::GCTL, 0);
    r.write32(regs::GCTL, 1);
    let mut m = rig.memory();
    m.buffer.bus = 1 << 33;
    // SAFETY: as above.
    assert_eq!(unsafe { init(r, m, &mut nodelay()) }.err(), Some(InitError::Address));
}

#[test]
fn looping_connection_lists_end() {
    // Mixers 3 and 4 feed each other; nothing reaches a DAC.
    let mut c = Codec::new(0, 1);
    c.dac(2).mixer(3, &[4]).mixer(4, &[3]).pin(5, &[3, 4], pincaps::OUT, config::LINE_OUT);
    let rig = Rig::new(vec![c]);
    assert_eq!(rig.init().err(), Some(InitError::NoOutputPath));
    // The same loop with a way out.
    let mut c = Codec::new(0, 1);
    c.dac(2).mixer(3, &[4]).mixer(4, &[3, 2]).pin(5, &[3], pincaps::OUT, config::LINE_OUT);
    let rig = Rig::new(vec![c]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.output().nodes(), &[2, 4, 3, 5]);
    assert_eq!(hda.output().inputs[..4], [0, 1, 0, 0]);
    assert!(rig.sim.codec(0).node(4).amp_is(false, 1, 0x17));
}

#[test]
fn deep_chains_are_cut_off() {
    // pin 20 → mixers 19, 18, ... 3 → DAC 2: longer than MAX_PATH.
    let mut c = Codec::new(0, 1);
    c.dac(2);
    for nid in 3..20u8 {
        c.mixer(nid, &[nid - 1]);
    }
    c.pin(20, &[19], pincaps::OUT, config::LINE_OUT);
    let rig = Rig::new(vec![c]);
    assert_eq!(rig.init().err(), Some(InitError::NoOutputPath));
}

#[test]
fn absurd_counts_are_bounded() {
    let mut c = simple_codec(0);
    // 255 function groups from 1 (only 1 is audio), 255 widgets from 2.
    c.node_mut(0).params.insert(0x04, (1 << 16) | 0xFF);
    c.node_mut(1).params.insert(0x04, (2 << 16) | 0xFF);
    // A long-form connection list of 127 entries: DAC 2 then a range up to
    // 0x7FFF.
    let m = c.node_mut(3);
    m.long_form = true;
    m.conn = vec![0x0002, 0xFFFF];
    m.conn.resize(127, 0xFFFF);
    m.params.insert(0x0E, 0x80 | 127);
    let rig = Rig::new(vec![c]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.widgets().len(), MAX_WIDGETS);
    let mixer = hda.widgets()[1];
    assert_eq!(mixer.nid, 3);
    assert_eq!(mixer.connections().len(), MAX_CONNECTIONS);
    assert!(mixer.conn_truncated);
    assert_eq!(mixer.connections()[..3], [2, 3, 4]);
    assert_eq!(hda.output().nodes(), &[2, 3, 4]);
    // Function groups looked at: at most 8.
    let fg_queries = rig
        .sim
        .codec(0)
        .log
        .iter()
        .filter(|&&cmd| cmd & 0xF_FFFF == short(codec::verb::GET_PARAMETER, codec::param::FUNCTION_GROUP_TYPE))
        .count();
    assert_eq!(fg_queries, 8);
}

#[test]
fn short_form_ranges_expand() {
    let mut c = Codec::new(0, 1);
    c.dac(2).dac(3).dac(4).selector(5, &[]).pin(6, &[5], pincaps::OUT, config::LINE_OUT);
    let s = c.node_mut(5);
    s.conn = vec![0x02, 0x84]; // 2, then the range 3..=4
    s.params.insert(0x0E, 2);
    let rig = Rig::new(vec![c]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.widgets()[3].connections(), &[2, 3, 4]);
    assert_eq!(hda.output().nodes(), &[2, 5, 6]);
}

#[test]
fn best_path_across_codecs() {
    // Codec 0 has a line out, codec 1 only a headphone: the table must end
    // up holding codec 0's widgets again.
    let mut hp = Codec::new(1, 1);
    hp.dac(2).pin(3, &[2], pincaps::OUT | pincaps::HP, config::HP_OUT);
    let rig = Rig::new(vec![simple_codec(0), hp]);
    let hda = rig.init().unwrap();
    assert_eq!(hda.codecs(), 0b11);
    assert_eq!(hda.output().codec, 0);
    assert_eq!(hda.output().pin, 4);
    assert_eq!(hda.widgets().len(), 4);
    assert_eq!(rig.sim.codec(0).node(4).pin_ctl, codec::pinctl::OUT_EN);
    assert_eq!(rig.sim.codec(1).node(3).pin_ctl, 0);
}

#[test]
fn unsolicited_responses_are_skipped() {
    let mut c = simple_codec(0);
    c.unsolicited_before_each = 2;
    let rig = Rig::new(vec![c]);
    let mut hda = rig.init().unwrap();
    assert_eq!(hda.output().nodes(), &[2, 3, 4]);
    let s = hda.stats();
    assert_eq!(s.unsolicited, 2 * s.verbs);
    let vendor = hda.verb(0, 0, short(codec::verb::GET_PARAMETER, 0), &mut nodelay()).unwrap();
    assert_eq!(vendor, 0x1AF4_0022);
}

#[test]
fn small_rings_wrap() {
    let rig = Rig::new(vec![simple_codec(0)]);
    {
        let mut d = rig.sim.dev();
        d.corb_size_caps = 0x20;
        d.rirb_size_caps = 0x10;
    }
    let mut hda = rig.init().unwrap();
    assert_eq!(rig.sim.reg8(regs::CORBSIZE) & 3, regs::ringsize::SIZE_16);
    assert_eq!(rig.sim.reg8(regs::RIRBSIZE) & 3, regs::ringsize::SIZE_2);
    for _ in 0..40 {
        assert_eq!(hda.verb(0, 2, short(codec::verb::GET_POWER_STATE, 0), &mut nodelay()), Ok(0));
    }
    assert_eq!(hda.stats().timeouts, 0);
}

#[test]
fn invisible_corbrp_reset_is_tolerated() {
    let rig = Rig::new(vec![simple_codec(0)]);
    rig.sim.dev().corbrp_reset_invisible = true;
    assert!(rig.init().is_ok());
}

#[test]
fn verb_checks_and_recovers() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    let d = &mut nodelay();
    assert_eq!(hda.verb(15, 0, 0xF0000, d), Err(Error::Range));
    assert_eq!(hda.verb(0, 0, 0x10_0000, d), Err(Error::Range));
    // A codec that is not there.
    assert_eq!(hda.verb(5, 0, 0xF0000, d), Err(Error::Timeout));
    rig.sim.dev().codecs[0].responsive = false;
    assert_eq!(hda.verb(0, 0, 0xF0000, d), Err(Error::Timeout));
    rig.sim.dev().codecs[0].responsive = true;
    assert_eq!(hda.verb(0, 2, short(codec::verb::GET_PARAMETER, 0x09), d), Ok(caps::DAC));
    assert_eq!(hda.stats().timeouts, 2);
}

// ---- the output stream -----------------------------------------------------

#[test]
fn configure_output_programs_the_stream() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    let f = StreamFormat::new(44100, 16, 2);
    hda.configure_output(f, 1024, 4, &mut nodelay()).unwrap();
    let base = regs::sd(4); // after four input streams
    assert_eq!(rig.sim.reg32(base + sd::CBL), 4096);
    assert_eq!(rig.sim.reg16(base + sd::LVI), 3);
    assert_eq!(rig.sim.reg16(base + sd::FMT), 0x4011);
    assert_eq!(rig.sim.reg8(base + sd::CTL_HI) >> 4, OUTPUT_STREAM_TAG);
    assert_eq!(rig.sim.reg8(base + sd::CTL) & 3, 0, "not running, out of reset");
    let bdl = rig.bdl.region().bus;
    assert_eq!(rig.sim.reg32(base + sd::BDPL) as u64 | ((rig.sim.reg32(base + sd::BDPU) as u64) << 32), bdl);
    let mem = rig.bdl.bytes();
    for i in 0..4 {
        let e = &mem[i * 16..i * 16 + 16];
        assert_eq!(u64::from_le_bytes(e[0..8].try_into().unwrap()), rig.buf.region().bus + i as u64 * 1024);
        assert_eq!(u32::from_le_bytes(e[8..12].try_into().unwrap()), 1024);
        assert_eq!(u32::from_le_bytes(e[12..16].try_into().unwrap()), 1);
    }
    let c = rig.sim.codec(0);
    assert_eq!(c.node(2).format, 0x4011);
    assert_eq!(c.node(2).stream_chan, 0x10);
    assert_eq!(hda.output_format(), Some(f));
    assert_eq!(hda.cyclic_len(), 4096);
}

#[test]
fn configure_output_rejects_bad_sizes() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    let d = &mut nodelay();
    let f = StreamFormat::new(48000, 16, 2);
    assert_eq!(hda.configure_output(f, 1024, 1, d), Err(Error::Range));
    assert_eq!(hda.configure_output(f, 256, MAX_BDL_ENTRIES as u32 + 1, d), Err(Error::Range));
    assert_eq!(hda.configure_output(f, 100, 4, d), Err(Error::Range));
    assert_eq!(hda.configure_output(f, 0, 4, d), Err(Error::Range));
    assert_eq!(hda.configure_output(f, 8192, 4, d), Err(Error::Range), "beyond the buffer");
    assert_eq!(hda.configure_output(f, u32::MAX - 127, 4, d), Err(Error::Range));
    // 3 channels × 2 bytes = 6-byte frames do not divide 128-byte periods,
    // but the DAC has only 2 channels anyway.
    assert_eq!(hda.configure_output(StreamFormat::new(48000, 16, 3), 384, 4, d), Err(Error::Unsupported));
    assert_eq!(hda.configure_output(StreamFormat::new(96000, 16, 2), 1024, 4, d), Err(Error::Unsupported));
    assert_eq!(hda.configure_output(StreamFormat::new(48000, 24, 2), 1024, 4, d), Err(Error::Unsupported));
    assert_eq!(hda.output_format(), None);
    assert_eq!(hda.configure_output(f, 2048, 8, d), Ok(()));
    let mut small = rig.memory();
    small.bdl.len = 16 * 3;
    let rig2 = Rig::new(vec![simple_codec(0)]);
    // SAFETY: as in `Rig::init`; `small` points into `rig`, alive here.
    let mut hda2 = unsafe { init(rig2.sim.regs(), small, &mut nodelay()) }.unwrap();
    assert_eq!(hda2.configure_output(f, 1024, 4, d), Err(Error::Range), "BDL holds three entries");
    assert_eq!(hda2.configure_output(f, 1024, 3, d), Ok(()));
}

#[test]
fn playback_reports_periods_and_plays_the_buffer() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    let d = &mut nodelay();
    hda.configure_output(StreamFormat::new(48000, 16, 2), 512, 4, d).unwrap();
    let data: Vec<u8> = (0..2048u32).map(|i| (i * 7 + 3) as u8).collect();
    assert!(hda.buffer_write(0, &data));
    hda.enable_interrupts(true);
    assert_eq!(rig.sim.reg32(regs::INTCTL), regs::int::GLOBAL | (1 << 4));
    assert!(!hda.take_interrupt().any(), "nothing yet");

    hda.start_output();
    assert!(hda.is_running());
    assert_eq!(rig.sim.run_output(300), 300);
    assert_eq!(hda.position(), 300);
    assert!(!rig.sim.irq());
    assert!(!hda.take_interrupt().buffer_complete);
    assert_eq!(rig.sim.run_output(212), 212);
    assert_eq!(hda.position(), 512);
    assert!(rig.sim.irq());
    let i = hda.take_interrupt();
    assert!(i.buffer_complete && !i.fifo_error && !i.descriptor_error);
    assert_eq!(i.intsts & (1 << 4), 1 << 4);
    assert!(!rig.sim.irq(), "cleared");
    assert!(!hda.take_interrupt().buffer_complete);

    // The rest of the buffer and once around again.
    assert_eq!(rig.sim.run_output(1536 + 1024), 2560);
    assert_eq!(hda.position(), 1024);
    assert!(hda.take_interrupt().buffer_complete);
    let played = rig.sim.dev().played.clone();
    assert_eq!(played.len(), 512 + 2560);
    assert_eq!(&played[..2048], &data[..]);
    assert_eq!(&played[2048..], &data[..1024]);
    assert_eq!(rig.sim.dev().completions, 6);
    assert_eq!(hda.stats().buffer_completions, 2);

    hda.stop_output(d);
    assert!(!hda.is_running());
    assert_eq!(rig.sim.run_output(512), 0);
    hda.start_output();
    assert_eq!(rig.sim.run_output(128), 128);
    hda.enable_interrupts(false);
    assert_eq!(rig.sim.reg32(regs::INTCTL), 0);
}

#[test]
fn buffer_bounds() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    assert_eq!(hda.buffer_len(), BUF);
    assert!(hda.buffer_write(BUF - 4, &[1, 2, 3, 4]));
    assert!(!hda.buffer_write(BUF - 3, &[1, 2, 3, 4]));
    assert!(!hda.buffer_write(usize::MAX, &[1]));
    assert!(hda.buffer_fill(0, 16, 0xAA));
    assert!(!hda.buffer_fill(1, BUF, 0));
    assert!(!hda.buffer_fill(usize::MAX, 2, 0));
    let b = rig.buf.bytes();
    assert_eq!(&b[..16], &[0xAA; 16]);
    assert_eq!(&b[BUF - 4..], &[1, 2, 3, 4]);
}

#[test]
fn start_without_configure_does_nothing() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    hda.start_output();
    assert!(!hda.is_running());
    assert_eq!(hda.position(), 0);
}

#[test]
fn shutdown_resets_the_controller() {
    let rig = Rig::new(vec![simple_codec(0)]);
    let mut hda = rig.init().unwrap();
    hda.configure_output(StreamFormat::new(48000, 16, 2), 512, 4, &mut nodelay()).unwrap();
    hda.start_output();
    let r = hda.shutdown(&mut nodelay());
    assert_eq!(r.read32(regs::GCTL) & regs::gctl::CRST, 0);
    assert_eq!(rig.sim.dev().resets, 2);
}

#[test]
fn widget_overrides_win_over_the_afg() {
    // The DAC has its own formats (96 kHz, 24-bit, 6 channels); the pin its
    // own output amplifier (0 dB at step 0x10).
    let mut c = Codec::new(0, 1);
    c.widget(2, caps::DAC | caps::FORMAT_OVERRIDE | (2 << 13), &[]).pin(3, &[2], pincaps::OUT, config::LINE_OUT);
    c.node_mut(3).params.insert(0x09, caps::PIN | caps::AMP_OVERRIDE);
    c.node_mut(3).params.insert(0x12, 0x8002_2010);
    c.node_mut(2).params.insert(0x0A, (0b01000 << 16) | 0x100);
    c.node_mut(2).params.insert(0x0B, 1);
    let rig = Rig::new(vec![c]);
    let mut hda = rig.init().unwrap();
    let p = *hda.output();
    assert_eq!(p.rates_hz().collect::<Vec<_>>(), [96000]);
    assert_eq!(p.sample_bits().collect::<Vec<_>>(), [24]);
    assert_eq!(p.max_channels, 6);
    let f = StreamFormat::new(96000, 24, 6);
    assert!(p.supports(&f));
    assert!(!p.supports(&StreamFormat::new(48000, 16, 2)));
    assert!(rig.sim.codec(0).node(3).amp_is(true, 0, 0x10));
    assert!(rig.sim.codec(0).node(2).amp_is(true, 0, 0x27), "DAC uses the AFG's amp caps");
    // 6 × 4 bytes = 24-byte frames: 1024-byte periods are not whole frames.
    assert_eq!(hda.configure_output(f, 1024, 4, &mut nodelay()), Err(Error::Range));
    assert_eq!(hda.configure_output(f, 3072, 4, &mut nodelay()), Ok(()));
    assert_eq!(rig.sim.codec(0).node(2).format, f.encode().unwrap());
}

/// QEMU's controller waits for RINTFL to be cleared before it takes the
/// next command (and sets it only with RINTCTL on): the driver turns
/// RINTCTL on and clears the flag with every response, so a long run of
/// verbs goes through.
#[test]
fn commands_flow_past_the_response_interrupt_flag() {
    let rig = Rig::new(vec![simple_codec(0)]);
    assert!(rig.sim.0.borrow().qemu_rirb);
    let mut hda = rig.init().expect("init with a QEMU-like RIRB");
    for i in 0..600u32 {
        let v = hda.verb(0, 0, short(codec::verb::GET_PARAMETER, codec::param::VENDOR_ID), &mut nodelay());
        assert!(v.is_ok(), "verb {i} answered");
    }
    assert_eq!(hda.stats().timeouts, 0);
}
