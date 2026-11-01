//! Host tests of the ALSA layer: structure layouts, hardware parameter
//! refinement and choice, and the playback runtime over a fake device.

use super::abi::{self, SwParams, SyncPtr, Timespec, Xferi, format, ioctl, param, state};
use super::ctl::{self, Controls, ElemId, VOLUME_MAX, id_by_name, iface};
use super::hw::{self, Caps, HwConfig, HwParams, Interval, Mask};
use super::pcm::{Dir, PcmBackend, PcmRuntime, State, boundary_for};
use crate::errno::Errno;
extern crate std;
use std::vec::Vec;
use std::{format as fmt, vec};

// ---- layouts ---------------------------------------------------------------

#[test]
fn request_numbers_encode_size_and_direction() {
    // _IOC(dir, type, nr, size): dir in bits 30-31 (1 write, 2 read), size
    // in 16-29, type in 8-15, number in 0-7.
    let ioc = |dir: u32, ty: u8, nr: u32, size: usize| (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr;
    assert_eq!(ioctl::PCM_HW_PARAMS, ioc(3, b'A', 0x11, abi::HW_PARAMS_LEN));
    assert_eq!(ioctl::PCM_HW_REFINE, ioc(3, b'A', 0x10, abi::HW_PARAMS_LEN));
    assert_eq!(ioctl::PCM_SW_PARAMS, ioc(3, b'A', 0x13, abi::SW_PARAMS_LEN));
    assert_eq!(ioctl::PCM_STATUS, ioc(2, b'A', 0x20, abi::STATUS_LEN));
    assert_eq!(ioctl::PCM_SYNC_PTR, ioc(3, b'A', 0x23, abi::SYNC_PTR_LEN));
    assert_eq!(ioctl::PCM_INFO, ioc(2, b'A', 0x01, abi::PCM_INFO_LEN));
    assert_eq!(ioctl::PCM_WRITEI_FRAMES, ioc(1, b'A', 0x50, abi::XFERI_LEN));
    assert_eq!(ioctl::CTL_CARD_INFO, ioc(2, b'U', 0x01, abi::CARD_INFO_LEN));
    assert_eq!(ioctl::CTL_PCM_INFO, ioc(3, b'U', 0x31, abi::PCM_INFO_LEN));
    assert_eq!(ioctl::CTL_ELEM_LIST, ioc(3, b'U', 0x10, abi::ELEM_LIST_LEN));
    assert_eq!(abi::PCM_VERSION, 0x0002_0011);
    assert_eq!(abi::CTL_VERSION, 0x0002_0009);
}

#[test]
fn structures_are_laid_out_like_the_header() {
    let info = abi::PcmInfo {
        device: 0,
        subdevice: 0,
        stream: abi::stream::PLAYBACK,
        card: 0,
        id: "virtio",
        name: "Output 0",
        subname: "subdevice #0",
        subdevices_count: 1,
        subdevices_avail: 1,
    };
    let b = info.encode();
    assert_eq!(&b[16..22], b"virtio");
    assert_eq!(&b[80..88], b"Output 0");
    assert_eq!(&b[160..172], b"subdevice #0");
    assert_eq!(b[200], 1);
    let card = abi::CardInfo {
        card: 0,
        id: "Virtio",
        driver: "virtio-snd",
        name: "n",
        longname: "a long name",
        mixername: "",
        components: "",
    };
    let c = card.encode();
    assert_eq!(&c[8..14], b"Virtio");
    assert_eq!(&c[24..34], b"virtio-snd");
    assert_eq!(&c[72..83], b"a long name");
    // Strings are cut to leave their terminating zero.
    let long = "x".repeat(40);
    let c = abi::CardInfo { name: &long, ..card }.encode();
    assert_eq!(c[71], 0);

    let sw = SwParams {
        avail_min: 1024,
        start_threshold: 4096,
        stop_threshold: 4096,
        boundary: 1 << 40,
        ..Default::default()
    };
    let e = sw.encode();
    assert_eq!(&e[16..24], &1024u64.to_le_bytes());
    assert_eq!(&e[64..72], &(1u64 << 40).to_le_bytes());
    assert_eq!(SwParams::decode(&e), sw);

    let st = abi::Status { state: state::RUNNING, appl_ptr: 7, hw_ptr: 5, delay: 2, avail: 9, ..Default::default() };
    let e = st.encode();
    assert_eq!(abi::Status::decode_state(&e), state::RUNNING);
    assert_eq!(&e[40..48], &7u64.to_le_bytes());
    assert_eq!(&e[48..56], &5u64.to_le_bytes());
    assert_eq!(&e[64..72], &9u64.to_le_bytes());

    let sp = SyncPtr { flags: 3, state: 2, hw_ptr: 11, appl_ptr: 13, avail_min: 17, ..Default::default() };
    let e = sp.encode();
    assert_eq!(&e[16..24], &11u64.to_le_bytes());
    assert_eq!(&e[72..80], &13u64.to_le_bytes());
    assert_eq!(SyncPtr::decode(&e), sp);

    let mut x = [0u8; abi::XFERI_LEN];
    x[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
    x[16..24].copy_from_slice(&256u64.to_le_bytes());
    assert_eq!(Xferi::decode(&x), Xferi { result: 0, buf: 0x1000, frames: 256 });
}

#[test]
fn silence_per_format() {
    assert_eq!(format::silence(format::S16_LE), ([0; 8], 2));
    assert_eq!(format::silence(format::U8).0[0], 0x80);
    assert_eq!(&format::silence(format::U16_LE).0[..2], &[0x00, 0x80]);
    assert_eq!(&format::silence(format::U24_LE).0[..4], &[0, 0, 0x80, 0]);
    assert_eq!(format::silence(format::MU_LAW).0[0], 0xff);
    assert_eq!(format::silence(format::S24_3LE).1, 3);
}

#[test]
fn next_device_walks_the_card() {
    assert_eq!(super::next_device(&[0, 2], -1), 0);
    assert_eq!(super::next_device(&[0, 2], 0), 2);
    assert_eq!(super::next_device(&[0, 2], 2), -1);
    assert_eq!(super::next_device(&[], -1), -1);
}

// ---- intervals ---------------------------------------------------------

#[test]
fn interval_arithmetic_keeps_open_ends() {
    let a = Interval::range(10, 20);
    let b = Interval::range(3, 4);
    let q = hw::div(&a, &b);
    // 10/4 = 2.5 .. 20/3 = 6.67: (2, 7) open at both ends.
    assert_eq!((q.min, q.max, q.openmin, q.openmax), (2, 7, true, true));
    let p = hw::mul(&a, &b);
    assert_eq!((p.min, p.max, p.integer), (30, 80, true));
    let t = hw::mulkdiv(&Interval::single_value(1024), 1_000_000, &Interval::single_value(44100));
    // 23219.95 us: between 23219 and 23220, open.
    assert_eq!((t.min, t.max, t.openmin, t.openmax), (23219, 23220, true, true));
    let mut i = Interval::range(0, 100);
    assert_eq!(i.refine(&q), Ok(true));
    // Integer and open at 2 and 7: 3..6.
    assert_eq!((i.min, i.max), (3, 6));
    assert!(i.refine(&Interval::range(7, 9)).is_err());
    assert_eq!(Interval::single_value(5).single(), Some(5));
    let huge = hw::mul(&Interval::range(1 << 20, 1 << 20), &Interval::range(1 << 20, 1 << 20));
    assert_eq!((huge.max, huge.openmax), (u32::MAX, false));
}

#[test]
fn hw_params_round_trip_keeps_unknown_bytes() {
    let mut raw = HwParams::any().encode();
    raw[600] = 0x5a;
    let mut p = HwParams::decode(&raw);
    assert_eq!(p.interval(param::RATE).max, u32::MAX);
    p.interval_mut(param::RATE).max = 48000;
    let out = p.encode();
    assert_eq!(out[600], 0x5a);
    assert_eq!(HwParams::decode(&out).interval(param::RATE).max, 48000);
    // Interval flags: openmin bit 0, openmax 1, integer 2, empty 3.
    let mut q = HwParams::any();
    *q.interval_mut(param::CHANNELS) =
        Interval { min: 1, max: 2, openmin: true, openmax: false, integer: true, empty: false };
    let e = q.encode();
    let at = 260 + (param::CHANNELS - param::SAMPLE_BITS) * 12;
    assert_eq!(&e[at..at + 12], &[1, 0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0]);
}

// ---- refinement --------------------------------------------------------

fn caps() -> Caps {
    let mut rates = [0u32; 16];
    rates[..4].copy_from_slice(&[8000, 22050, 44100, 48000]);
    Caps {
        access: Mask::of(&[abi::access::RW_INTERLEAVED]),
        info: hw::INFO,
        formats: Mask::of(&[format::U8, format::S16_LE, format::S32_LE]),
        rates,
        rate_count: 4,
        channels_min: 1,
        channels_max: 2,
        period_bytes_min: 64,
        period_bytes_max: 65536,
        periods_min: 2,
        periods_max: 16,
        buffer_bytes_max: 262144,
    }
}

#[test]
fn refine_from_anything_narrows_to_the_device() {
    let mut p = HwParams::any();
    hw::refine(&mut p, &caps()).unwrap();
    assert_eq!(p.mask(param::ACCESS).single(), Some(abi::access::RW_INTERLEAVED));
    assert_eq!(p.mask(param::FORMAT).values().collect::<Vec<_>>(), [format::U8, format::S16_LE, format::S32_LE]);
    assert_eq!(p.mask(param::SUBFORMAT).single(), Some(abi::SUBFORMAT_STD));
    let iv = |k| {
        let i = *p.interval(k);
        (i.min, i.max)
    };
    assert_eq!(iv(param::SAMPLE_BITS), (8, 32));
    assert_eq!(iv(param::FRAME_BITS), (8, 64));
    assert_eq!(iv(param::CHANNELS), (1, 2));
    assert_eq!(iv(param::RATE), (8000, 48000));
    assert_eq!(iv(param::PERIOD_BYTES), (64, 65536));
    assert_eq!(iv(param::PERIODS), (2, 16));
    // 64 bytes at 8 bytes a frame: 8 frames; 65536 bytes of 1-byte frames.
    assert_eq!(iv(param::PERIOD_SIZE), (8, 65536));
    // The buffer: two periods of 64 bytes at least; 262144 bytes at most.
    assert_eq!(iv(param::BUFFER_BYTES), (128, 262144));
    assert_eq!(iv(param::BUFFER_SIZE), (16, 262144));
    assert_ne!(p.cmask, 0);
    assert_eq!(p.info, hw::INFO);
    assert_eq!((p.msbits, p.rate_num), (0, 0));
}

#[test]
fn refine_with_a_format_rate_and_channels_reports_them() {
    let mut p = HwParams::any();
    p.masks[param::FORMAT] = Mask::of(&[format::S16_LE]);
    *p.interval_mut(param::CHANNELS) = Interval::single_value(2);
    *p.interval_mut(param::RATE) = Interval::single_value(44100);
    hw::refine(&mut p, &caps()).unwrap();
    assert_eq!(p.interval(param::SAMPLE_BITS).single(), Some(16));
    assert_eq!(p.interval(param::FRAME_BITS).single(), Some(32));
    assert_eq!((p.msbits, p.rate_num, p.rate_den), (16, 44100, 1));
    // 64..65536 bytes of 4-byte frames.
    assert_eq!((p.interval(param::PERIOD_SIZE).min, p.interval(param::PERIOD_SIZE).max), (16, 16384));
}

#[test]
fn refine_snaps_rates_to_the_device_list() {
    let refine_rate = |lo, hi| {
        let mut p = HwParams::any();
        *p.interval_mut(param::RATE) = Interval::range(lo, hi);
        hw::refine(&mut p, &caps()).map(|()| (p.interval(param::RATE).min, p.interval(param::RATE).max))
    };
    assert_eq!(refine_rate(44000, 46000), Ok((44100, 44100)));
    assert_eq!(refine_rate(9000, 30000), Ok((22050, 22050)));
    assert_eq!(refine_rate(44000, 44000), Err(Errno::EINVAL));
    assert_eq!(refine_rate(96000, 192000), Err(Errno::EINVAL));
    let mut p = HwParams::any();
    p.masks[param::FORMAT] = Mask::of(&[format::FLOAT_LE]);
    assert_eq!(hw::refine(&mut p, &caps()), Err(Errno::EINVAL));
    let mut p = HwParams::any();
    *p.interval_mut(param::CHANNELS) = Interval::single_value(6);
    assert_eq!(hw::refine(&mut p, &caps()), Err(Errno::EINVAL));
}

#[test]
fn refine_turns_times_into_sizes() {
    // What a "period_time near 23220 us" request looks like after the
    // library narrowed it: about 1024 frames at 44.1 kHz.
    let mut p = HwParams::any();
    p.masks[param::FORMAT] = Mask::of(&[format::S16_LE]);
    *p.interval_mut(param::CHANNELS) = Interval::single_value(2);
    *p.interval_mut(param::RATE) = Interval::single_value(44100);
    *p.interval_mut(param::PERIOD_TIME) =
        Interval { min: 23219, max: 23220, openmin: false, openmax: false, integer: false, empty: false };
    hw::refine(&mut p, &caps()).unwrap();
    assert_eq!(p.interval(param::PERIOD_SIZE).single(), Some(1024));
    assert_eq!(p.interval(param::PERIOD_BYTES).single(), Some(4096));
}

#[test]
fn choose_fills_in_what_the_caller_left_open() {
    let mut p = HwParams::any();
    let cfg = hw::choose(&mut p, &caps()).unwrap();
    assert_eq!(
        cfg,
        HwConfig {
            access: abi::access::RW_INTERLEAVED,
            format: format::S16_LE,
            channels: 2,
            rate: 48000,
            period_frames: 960,
            periods: 4,
            buffer_frames: 3840,
            frame_bytes: 4,
        }
    );
    assert_eq!(p.interval(param::BUFFER_BYTES).single(), Some(15360));
    // A caller that chose everything gets exactly that.
    let mut p = HwParams::any();
    p.masks[param::FORMAT] = Mask::of(&[format::U8]);
    *p.interval_mut(param::CHANNELS) = Interval::single_value(1);
    *p.interval_mut(param::RATE) = Interval::single_value(22050);
    *p.interval_mut(param::PERIOD_SIZE) = Interval::single_value(512);
    *p.interval_mut(param::PERIODS) = Interval::single_value(8);
    let cfg = hw::choose(&mut p, &caps()).unwrap();
    assert_eq!(
        (cfg.format, cfg.channels, cfg.rate, cfg.period_frames, cfg.buffer_frames, cfg.frame_bytes),
        (format::U8, 1, 22050, 512, 4096, 1)
    );
    // An impossible request.
    let mut p = HwParams::any();
    *p.interval_mut(param::PERIOD_BYTES) = Interval::single_value(100_000);
    assert_eq!(hw::choose(&mut p, &caps()), Err(Errno::EINVAL));
}

// ---- the runtime -------------------------------------------------------

/// Records what the runtime asked of the device; plays on request.
#[derive(Debug, Default)]
struct FakeDevice {
    calls: Vec<std::string::String>,
    ring: Vec<u8>,
    committed: usize,
    running: bool,
    fail_start: bool,
    consumed: usize,
}

impl PcmBackend for FakeDevice {
    fn configure(&mut self, cfg: &HwConfig) -> Result<(), Errno> {
        self.calls.push(fmt!("configure {} {} {}", cfg.rate, cfg.period_bytes(), cfg.buffer_bytes()));
        self.ring = vec![0; cfg.buffer_bytes() as usize];
        Ok(())
    }
    fn prepare(&mut self) -> Result<(), Errno> {
        self.calls.push("prepare".into());
        self.running = false;
        self.committed = 0;
        Ok(())
    }
    fn start(&mut self) -> Result<(), Errno> {
        if self.fail_start {
            return Err(Errno::EIO);
        }
        self.calls.push("start".into());
        self.running = true;
        Ok(())
    }
    fn stop(&mut self) -> Result<(), Errno> {
        self.calls.push("stop".into());
        self.running = false;
        Ok(())
    }
    fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Errno> {
        self.ring[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }
    fn commit(&mut self, bytes: usize, _running: bool) {
        self.committed += bytes;
    }
    fn drain(&mut self) {
        self.calls.push("drain".into());
    }
    fn release(&mut self) {
        self.calls.push("release".into());
    }
    fn read(&mut self, offset: usize, out: &mut [u8]) -> Result<(), Errno> {
        out.copy_from_slice(&self.ring[offset..offset + out.len()]);
        Ok(())
    }
    fn consumed(&mut self, bytes: usize) {
        self.consumed += bytes;
    }
}

fn cfg() -> HwConfig {
    HwConfig {
        access: abi::access::RW_INTERLEAVED,
        format: format::S16_LE,
        channels: 2,
        rate: 48000,
        period_frames: 256,
        periods: 4,
        buffer_frames: 1024,
        frame_bytes: 4,
    }
}

const T0: Timespec = Timespec { sec: 1, nsec: 0 };

fn frames(n: usize, tag: u8) -> Vec<u8> {
    (0..n * 4).map(|i| tag.wrapping_add(i as u8)).collect()
}

#[test]
fn state_rules_before_and_after_setup() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    assert_eq!(rt.state(), State::Open);
    assert_eq!(rt.prepare(&mut dev), Err(Errno::EBADFD));
    assert_eq!(rt.write(&frames(1, 0), &mut dev, T0), Err(Errno::EBADFD));
    assert_eq!(rt.set_sw_params(&SwParams::default()), Err(Errno::EBADFD));
    rt.hw_params(cfg(), &mut dev).unwrap();
    assert_eq!(rt.state(), State::Setup);
    let sw = rt.sw_params();
    assert_eq!((sw.avail_min, sw.start_threshold, sw.stop_threshold), (256, 1, 1024));
    assert_eq!(sw.boundary, boundary_for(1024));
    assert!(sw.boundary.is_power_of_two() && sw.boundary > 1 << 60);
    assert_eq!(rt.write(&frames(1, 0), &mut dev, T0), Err(Errno::EBADFD));
    assert_eq!(rt.start(&mut dev, T0), Err(Errno::EBADFD));
    rt.prepare(&mut dev).unwrap();
    assert_eq!(rt.start(&mut dev, T0), Err(Errno::EPIPE), "nothing to play");
    assert_eq!(dev.calls, ["configure 48000 1024 4096", "prepare"]);
}

#[test]
fn writes_fill_the_ring_and_start_at_the_threshold() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(cfg(), &mut dev).unwrap();
    let mut sw = rt.sw_params();
    sw.start_threshold = 1024;
    rt.set_sw_params(&sw).unwrap();
    rt.prepare(&mut dev).unwrap();
    assert_eq!(rt.write(&frames(600, 1), &mut dev, T0), Ok(600));
    assert_eq!(rt.state(), State::Prepared);
    assert!(rt.readiness().writable, "424 frames free, avail_min 256");
    // Only the room left is taken; the threshold starts the stream.
    assert_eq!(rt.write(&frames(600, 2), &mut dev, T0), Ok(424));
    assert_eq!(rt.state(), State::Running);
    assert!(dev.running);
    assert_eq!((rt.avail(), rt.hw_avail()), (0, 1024));
    assert!(!rt.readiness().writable);
    assert_eq!(rt.write(&frames(10, 3), &mut dev, T0), Ok(0));
    // The device plays a period: it wraps into the ring's start.
    rt.on_played(256, &mut dev, T0);
    assert_eq!(rt.avail(), 256);
    assert!(rt.readiness().writable);
    assert_eq!(rt.write(&frames(300, 4), &mut dev, T0), Ok(256));
    assert_eq!(&dev.ring[..4], &frames(1, 4)[..]);
    assert_eq!(dev.committed, 1280 * 4);
    let st = rt.status(T0);
    assert_eq!((st.state, st.appl_ptr, st.hw_ptr, st.avail, st.delay), (state::RUNNING, 1280, 256, 0, 1024));
    // Partial frames are not written.
    rt.on_played(512, &mut dev, T0);
    assert_eq!(rt.write(&[0u8; 7], &mut dev, T0), Ok(1));
}

#[test]
fn running_dry_is_an_underrun() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    assert_eq!(rt.write(&frames(512, 0), &mut dev, T0), Ok(512));
    assert_eq!(rt.state(), State::Running, "start_threshold 1");
    rt.on_played(256, &mut dev, T0);
    assert_eq!(rt.state(), State::Running);
    // More than was written is not believed: hw stops at appl.
    rt.on_played(9999, &mut dev, T0);
    assert_eq!(rt.state(), State::Xrun);
    assert_eq!(rt.hw(), 512);
    assert_eq!(rt.xruns, 1);
    assert_eq!(rt.write(&frames(1, 0), &mut dev, T0), Err(Errno::EPIPE));
    assert_eq!(rt.delay(), Err(Errno::EPIPE));
    let r = rt.readiness();
    assert!(r.error && r.writable);
    assert_eq!(rt.status(T0).state, state::XRUN);
    // PREPARE recovers.
    rt.prepare(&mut dev).unwrap();
    assert_eq!((rt.state(), rt.appl(), rt.hw()), (State::Prepared, 0, 0));
    assert_eq!(dev.calls.iter().filter(|c| *c == "stop").count(), 1);
}

#[test]
fn drain_waits_for_the_device_then_sets_up() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(cfg(), &mut dev).unwrap();
    let mut sw = rt.sw_params();
    sw.start_threshold = 4096;
    rt.set_sw_params(&sw).unwrap();
    rt.prepare(&mut dev).unwrap();
    rt.write(&frames(300, 0), &mut dev, T0).unwrap();
    assert_eq!(rt.state(), State::Prepared, "below the start threshold");
    // DRAIN starts a prepared stream and waits.
    assert_eq!(rt.drain(&mut dev, T0), Ok(false));
    assert_eq!(rt.state(), State::Draining);
    assert_eq!(rt.write(&frames(1, 0), &mut dev, T0), Err(Errno::EBADFD));
    assert_eq!(rt.drain(&mut dev, T0), Ok(false));
    rt.on_played(256, &mut dev, T0);
    assert_eq!(rt.state(), State::Draining);
    rt.on_played(300, &mut dev, T0);
    assert_eq!(rt.state(), State::Setup);
    assert_eq!(dev.calls, ["configure 48000 1024 4096", "prepare", "start", "drain", "stop"]);
    // Draining a stream with nothing written is immediate.
    rt.prepare(&mut dev).unwrap();
    assert_eq!(rt.drain(&mut dev, T0), Ok(true));
    assert_eq!(rt.state(), State::Setup);
    assert_eq!(rt.drain(&mut dev, T0), Ok(true));
}

#[test]
fn drop_free_and_close() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    assert_eq!(rt.drop_stream(&mut dev, T0), Err(Errno::EBADFD));
    rt.hw_params(cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    rt.write(&frames(100, 0), &mut dev, T0).unwrap();
    assert_eq!(rt.hw_free(&mut dev), Err(Errno::EBADFD), "running");
    rt.drop_stream(&mut dev, T0).unwrap();
    assert_eq!(rt.state(), State::Setup);
    rt.hw_free(&mut dev).unwrap();
    assert_eq!(rt.state(), State::Open);
    rt.hw_params(cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    rt.write(&frames(100, 0), &mut dev, T0).unwrap();
    rt.close(&mut dev);
    assert_eq!(rt.state(), State::Open);
    assert_eq!(dev.calls.last().map(|s| s.as_str()), Some("release"));
    // A device that cannot start leaves the stream prepared.
    let mut dev = FakeDevice { fail_start: true, ..Default::default() };
    rt.hw_params(cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    assert_eq!(rt.write(&frames(100, 0), &mut dev, T0), Err(Errno::EIO));
    assert_eq!(rt.state(), State::Prepared);
}

#[test]
fn sw_params_are_checked() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(cfg(), &mut dev).unwrap();
    let ok = rt.sw_params();
    assert_eq!(rt.set_sw_params(&SwParams { avail_min: 0, ..ok }), Err(Errno::EINVAL));
    assert_eq!(rt.set_sw_params(&SwParams { tstamp_mode: 2, ..ok }), Err(Errno::EINVAL));
    assert_eq!(rt.set_sw_params(&SwParams { silence_threshold: 2048, ..ok }), Err(Errno::EINVAL));
    let back = rt.set_sw_params(&SwParams { boundary: 5, avail_min: 512, ..ok }).unwrap();
    assert_eq!(back.boundary, boundary_for(1024), "the boundary is the kernel's");
    assert_eq!(rt.sw_params().avail_min, 512);
}

#[test]
fn sync_ptr_reports_pointers_and_takes_avail_min() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    rt.write(&frames(700, 0), &mut dev, T0).unwrap();
    rt.on_played(256, &mut dev, T0);
    let req = SyncPtr { flags: abi::sync_ptr::APPL, avail_min: 128, ..Default::default() };
    let r = rt.sync_ptr(&req, &mut dev, T0);
    assert_eq!((r.state, r.hw_ptr, r.appl_ptr, r.avail_min), (state::RUNNING, 256, 700, 128));
    let req = SyncPtr {
        flags: abi::sync_ptr::APPL | abi::sync_ptr::AVAIL_MIN,
        avail_min: 1,
        appl_ptr: 5,
        ..Default::default()
    };
    let r = rt.sync_ptr(&req, &mut dev, T0);
    assert_eq!((r.appl_ptr, r.avail_min), (700, 128));
}

// ---- capture ---------------------------------------------------------------

#[test]
fn capture_reads_what_the_device_captured() {
    let mut rt = PcmRuntime::capture();
    let mut dev = FakeDevice::default();
    assert_eq!(rt.dir(), Dir::Capture);
    rt.hw_params(cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    // Starting a capture stream needs nothing written.
    let mut out = vec![0u8; 100 * 4];
    assert_eq!(rt.write(&frames(1, 0), &mut dev, T0), Err(Errno::EBADFD));
    // A read of at least start_threshold (1) frames starts it; nothing yet.
    assert_eq!(rt.read(&mut out, &mut dev, T0), Ok(0));
    assert_eq!(rt.state(), State::Running);
    assert!(!rt.readiness().readable);
    // The device captured 300 frames into the ring.
    dev.ring[..300 * 4].copy_from_slice(&frames(300, 9));
    rt.on_captured(300, &mut dev, T0);
    assert_eq!((rt.avail(), rt.capture_room()), (300, 724));
    assert!(rt.readiness().readable, "300 frames, avail_min 256");
    assert_eq!(rt.read(&mut out, &mut dev, T0), Ok(100));
    assert_eq!(out, frames(100, 9));
    assert_eq!(dev.consumed, 400);
    let st = rt.status(T0);
    assert_eq!((st.state, st.appl_ptr, st.hw_ptr, st.avail, st.delay), (state::RUNNING, 100, 300, 200, 200));
    // The device may not run more than a buffer ahead of the reader.
    rt.on_captured(5000, &mut dev, T0);
    assert_eq!(rt.hw(), 100 + 1024);
    assert_eq!(rt.state(), State::Xrun, "the buffer filled up: an overrun");
    assert_eq!(rt.xruns, 1);
    assert_eq!(rt.read(&mut out, &mut dev, T0), Err(Errno::EPIPE));
    let r = rt.readiness();
    assert!(r.error && r.readable && !r.writable);
    rt.prepare(&mut dev).unwrap();
    assert_eq!((rt.appl(), rt.hw(), rt.state()), (0, 0, State::Prepared));
    // START works from PREPARED with nothing captured.
    rt.start(&mut dev, T0).unwrap();
    assert_eq!(rt.state(), State::Running);
}

#[test]
fn capture_reads_wrap_and_drain_leaves_frames_to_read() {
    let mut rt = PcmRuntime::capture();
    let mut dev = FakeDevice::default();
    rt.hw_params(cfg(), &mut dev).unwrap();
    let mut sw = rt.sw_params();
    sw.start_threshold = 2000;
    rt.set_sw_params(&sw).unwrap();
    rt.prepare(&mut dev).unwrap();
    let mut out = vec![0u8; 900 * 4];
    assert_eq!(rt.read(&mut out, &mut dev, T0), Ok(0));
    assert_eq!(rt.state(), State::Prepared, "900 frames are below the start threshold");
    rt.start(&mut dev, T0).unwrap();
    rt.on_captured(900, &mut dev, T0);
    assert_eq!(rt.read(&mut out, &mut dev, T0), Ok(900));
    // The next 300 frames wrap around the 1024-frame ring.
    for i in 0..300 {
        let at = ((900 + i) % 1024) * 4;
        dev.ring[at..at + 4].copy_from_slice(&[i as u8; 4]);
    }
    rt.on_captured(1200, &mut dev, T0);
    let mut out = vec![0u8; 300 * 4];
    assert_eq!(rt.read(&mut out[..200 * 4], &mut dev, T0), Ok(200));
    assert_eq!(&out[124 * 4..124 * 4 + 4], &[124; 4], "frames after the wrap");
    // DRAIN stops the device; the 100 frames left can still be read.
    assert_eq!(rt.drain(&mut dev, T0), Ok(true));
    assert_eq!(rt.state(), State::Draining);
    assert!(rt.readiness().readable);
    rt.on_captured(2000, &mut dev, T0);
    assert_eq!(rt.avail(), 100, "nothing is captured once stopped");
    assert_eq!(rt.read(&mut out, &mut dev, T0), Ok(100));
    assert_eq!(rt.state(), State::Setup);
    assert_eq!(rt.read(&mut out, &mut dev, T0), Err(Errno::EBADFD));
}

// ---- mmap ------------------------------------------------------------------

fn mmap_cfg() -> HwConfig {
    HwConfig { access: abi::access::MMAP_INTERLEAVED, ..cfg() }
}

#[test]
fn mmap_playback_moves_appl_through_sync_ptr() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(mmap_cfg(), &mut dev).unwrap();
    assert!(rt.is_mmap());
    rt.prepare(&mut dev).unwrap();
    // The application filled 600 frames of the mapped buffer.
    let req = SyncPtr { flags: abi::sync_ptr::AVAIL_MIN, appl_ptr: 600, ..Default::default() };
    let r = rt.sync_ptr(&req, &mut dev, T0);
    assert_eq!((r.appl_ptr, r.hw_ptr, r.state), (600, 0, state::PREPARED));
    assert_eq!(dev.committed, 600 * 4);
    assert_eq!(rt.state(), State::Prepared, "moving appl does not start the stream");
    rt.start(&mut dev, T0).unwrap();
    rt.on_played(500, &mut dev, T0);
    // More than a buffer past hw, or behind hw: refused.
    assert_eq!(rt.set_appl(500 + 1025, &mut dev), Err(Errno::EINVAL));
    assert_eq!(rt.set_appl(499, &mut dev), Err(Errno::EINVAL));
    rt.set_appl(1500, &mut dev).unwrap();
    assert_eq!(rt.appl(), 1500);
    // Rewind over what the device has not taken; forward over free room.
    assert_eq!(rt.rewind(2000), Ok(1000));
    assert_eq!(rt.appl(), 500);
    assert_eq!(rt.forward(100, &mut dev), Ok(100));
    // Read/write access does not take the caller's appl_ptr.
    let mut rw = PcmRuntime::new();
    rw.hw_params(cfg(), &mut dev).unwrap();
    rw.prepare(&mut dev).unwrap();
    rw.sync_ptr(&SyncPtr { appl_ptr: 300, ..Default::default() }, &mut dev, T0);
    assert_eq!(rw.appl(), 0);
}

#[test]
fn mmap_pointers_wrap_at_the_boundary() {
    let mut rt = PcmRuntime::new();
    let mut dev = FakeDevice::default();
    rt.hw_params(mmap_cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    let b = rt.sw_params().boundary;
    assert_eq!(rt.set_appl(b, &mut dev), Err(Errno::EINVAL), "beyond the boundary");
    rt.set_appl(1000, &mut dev).unwrap();
    rt.start(&mut dev, T0).unwrap();
    rt.on_played(1000, &mut dev, T0);
    assert_eq!(rt.state(), State::Xrun);
    let st = rt.mmap_status(T0);
    assert_eq!((st.state, st.hw_ptr), (state::XRUN, 1000));
    let e = st.encode();
    assert_eq!(&e[8..16], &1000u64.to_le_bytes());
    assert_eq!(rt.wrap(b + 5), 5);
}

#[test]
fn mmap_capture_takes_frames_through_the_control_pointer() {
    let mut rt = PcmRuntime::capture();
    let mut dev = FakeDevice::default();
    rt.hw_params(mmap_cfg(), &mut dev).unwrap();
    rt.prepare(&mut dev).unwrap();
    rt.start(&mut dev, T0).unwrap();
    rt.on_captured(700, &mut dev, T0);
    assert_eq!(rt.set_appl(701, &mut dev), Err(Errno::EINVAL), "past what was captured");
    rt.set_appl(512, &mut dev).unwrap();
    assert_eq!((rt.avail(), dev.consumed), (188, 512 * 4));
    rt.set_avail_min(64);
    assert_eq!(rt.sw_params().avail_min, 64);
    let mc = abi::MmapControl { appl_ptr: 7, avail_min: 9 };
    assert_eq!(abi::MmapControl::decode(&mc.encode()), mc);
}

// ---- mixer controls --------------------------------------------------------

#[test]
fn controls_list_info_read_and_write() {
    assert_eq!(ioctl::CTL_ELEM_INFO, 0xc110_5511);
    let mut c = Controls::new(true, true);
    assert_eq!(c.len(), 4);
    let mut list = [0u8; abi::ELEM_LIST_LEN];
    list[4..8].copy_from_slice(&3u32.to_le_bytes()); // space
    list[16..24].copy_from_slice(&0x1234u64.to_le_bytes());
    assert_eq!(c.list(&mut list), 0..3);
    assert_eq!((&list[8..12], &list[12..16]), (&3u32.to_le_bytes()[..], &4u32.to_le_bytes()[..]));
    assert_eq!(Controls::list_pointer(&list), 0x1234);
    list[0..4].copy_from_slice(&2u32.to_le_bytes()); // offset
    assert_eq!(c.list(&mut list), 2..4);
    let id = c.id(0).unwrap();
    assert_eq!((id.numid, id.iface, id.name()), (1, iface::MIXER, &b"Master Playback Volume"[..]));
    assert_eq!(ElemId::decode(&id.encode()), id);

    // INFO by name.
    let mut req = [0u8; abi::ELEM_INFO_LEN];
    req[..abi::ELEM_ID_LEN].copy_from_slice(&id_by_name(iface::MIXER, "Master Playback Volume").encode());
    let info = c.info(&req).unwrap();
    assert_eq!(&info[0..4], &1u32.to_le_bytes(), "the numid is filled in");
    assert_eq!(&info[64..68], &ctl::elem_type::INTEGER.to_le_bytes());
    assert_eq!(&info[68..72], &ctl::access::READWRITE.to_le_bytes());
    assert_eq!(&info[72..76], &2u32.to_le_bytes());
    assert_eq!(&info[88..96], &(VOLUME_MAX as u64).to_le_bytes());
    let mut sw = [0u8; abi::ELEM_INFO_LEN];
    sw[0..4].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(&c.info(&sw).unwrap()[64..68], &ctl::elem_type::BOOLEAN.to_le_bytes());
    let mut missing = [0u8; abi::ELEM_INFO_LEN];
    missing[..abi::ELEM_ID_LEN].copy_from_slice(&id_by_name(iface::MIXER, "PCM Playback Volume").encode());
    assert_eq!(c.info(&missing), Err(Errno::ENOENT));
    missing[0..4].copy_from_slice(&9u32.to_le_bytes());
    assert_eq!(c.info(&missing), Err(Errno::ENOENT));

    // READ, WRITE, and the gains they make.
    let mut v = [0u8; abi::ELEM_VALUE_LEN];
    v[0..4].copy_from_slice(&1u32.to_le_bytes());
    let r = c.read(&v).unwrap();
    assert_eq!(&r[72..80], &100u64.to_le_bytes());
    assert_eq!(c.playback_gains(), [1 << 16, 1 << 16]);
    v[72..80].copy_from_slice(&50u64.to_le_bytes());
    v[80..88].copy_from_slice(&25u64.to_le_bytes());
    let w = c.write(&v).unwrap();
    assert_eq!(w.changed.map(|i| i.numid), Some(1));
    assert_eq!(c.playback_gains(), [1 << 15, 1 << 14]);
    assert_eq!(c.write(&v).unwrap().changed, None, "the same values again: no change");
    v[72..80].copy_from_slice(&101u64.to_le_bytes());
    assert_eq!(c.write(&v), Err(Errno::EINVAL));
    assert_eq!(c.playback_gains(), [1 << 15, 1 << 14], "a refused write changes nothing");
    // Switching the master off mutes.
    let mut s = [0u8; abi::ELEM_VALUE_LEN];
    s[..abi::ELEM_ID_LEN].copy_from_slice(&id_by_name(iface::MIXER, "Master Playback Switch").encode());
    assert!(c.write(&s).unwrap().changed.is_some());
    assert_eq!(c.playback_gains(), [0, 0]);
    s[72..80].copy_from_slice(&2u64.to_le_bytes());
    assert_eq!(c.write(&s), Err(Errno::EINVAL), "a switch is 0 or 1");
    assert_eq!(c.capture_gains(), [1 << 16, 1 << 16]);
    let e = ctl::event(&c.id(1).unwrap(), ctl::EVENT_MASK_VALUE);
    assert_eq!((&e[0..4], &e[4..8], &e[8..12]), (&[0u8; 4][..], &1u32.to_le_bytes()[..], &2u32.to_le_bytes()[..]));
    // A playback-only card has two controls.
    assert_eq!(Controls::new(true, false).len(), 2);
    assert!(Controls::new(false, false).is_empty());
}
