//! Tests of the messages, the state machine, the ring bookkeeping, and the
//! driver against the simulated device in `sim`.

use super::*;
use crate::proto::{ChmapInfo, Event, JackInfo, PcmInfo, SetParams, chmap, code, direction, format, rate, status};
use crate::ring::{Chunk, PeriodRing};
use crate::sim::{CONFIG_LEN, CommonRegs, DevState, DmaMemory, NotifyRegs, Sim, output_stream};
use crate::stream::{Command, ParamError, StreamState, Wanted, check_params};
use poly_virtio::{DescState, Layout, PciTransport, Region};
extern crate std;
use std::{vec, vec::Vec};

const VERSION_1: u64 = poly_virtio::feature::VERSION_1;
const EVENT_IDX: u64 = poly_virtio::feature::EVENT_IDX;

// ---- messages ------------------------------------------------------------

#[test]
fn messages_have_the_spec_layouts() {
    // §5.14.6.1: virtio_snd_query_info is hdr, start_id, count, size.
    assert_eq!(proto::query_info(code::PCM_INFO, 1, 2, 32), [0x00, 0x01, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 32, 0, 0, 0]);
    assert_eq!(proto::id_request(code::PCM_START, 3), [0x04, 0x01, 0, 0, 3, 0, 0, 0]);
    let p = SetParams {
        buffer_bytes: 0x10000,
        period_bytes: 0x1000,
        features: 0,
        channels: 2,
        format: format::S16,
        rate: rate::R48000,
    };
    let req = p.request(0);
    assert_eq!(req.len(), 24);
    assert_eq!(&req[0..4], &code::PCM_SET_PARAMS.to_le_bytes());
    assert_eq!(&req[8..12], &0x10000u32.to_le_bytes());
    assert_eq!(&req[12..16], &0x1000u32.to_le_bytes());
    assert_eq!(req[20..23], [2, format::S16, rate::R48000]);
    assert_eq!(SetParams::parse(&req), Some((0, p)));
    assert_eq!(p.frame_bytes(), Some(4));

    let info = output_stream();
    assert_eq!(PcmInfo::decode(&info.encode()), info);
    let enc = info.encode();
    assert_eq!(enc[24], direction::OUTPUT);
    assert_eq!(&enc[8..16], &info.formats.to_le_bytes());
    let j = JackInfo { hda_fn_nid: 1, features: 0, hda_reg_defconf: 0x1234, hda_reg_caps: 7, connected: true };
    assert_eq!(JackInfo::decode(&j.encode()), j);
    let mut positions = [0u8; chmap::MAX_SIZE];
    positions[..2].copy_from_slice(&[chmap::FL, chmap::FR]);
    let m = ChmapInfo { hda_fn_nid: 0, direction: direction::OUTPUT, channels: 2, positions };
    assert_eq!(ChmapInfo::decode(&m.encode()), m);
    assert_eq!(m.used_positions(), &[chmap::FL, chmap::FR]);
    let e = Event { code: code::EVT_PCM_XRUN, data: 0 };
    assert_eq!(Event::decode(&e.encode()), Some(e));
}

#[test]
fn formats_and_rates() {
    assert_eq!(format::physical_bits(format::S16), Some(16));
    assert_eq!(format::physical_bits(format::S24_3), Some(24));
    assert_eq!(format::physical_bits(format::S24), Some(32));
    assert_eq!(format::width_bits(format::S24), Some(24));
    assert_eq!(format::physical_bits(format::IMA_ADPCM), None);
    assert!(format::is_unsigned(format::U8) && !format::is_unsigned(format::S16));
    assert_eq!(rate::code_of(44100), Some(rate::R44100));
    assert_eq!(rate::code_of(48000), Some(rate::R48000));
    assert_eq!(rate::code_of(44000), None);
    let info = output_stream();
    assert_eq!(info.rates_hz().collect::<Vec<_>>(), [8000, 22050, 44100, 48000]);
    assert_eq!(info.usable_formats().collect::<Vec<_>>(), [format::U8, format::S16, format::S32, format::FLOAT]);
    assert!(info.is_usable());
    assert!(!PcmInfo { channels_min: 3, channels_max: 2, ..info }.is_usable());
    assert!(!PcmInfo { formats: 1 << format::IMA_ADPCM, ..info }.is_usable());
}

// ---- the state machine ---------------------------------------------------

#[test]
fn lifecycle_paths_follow_the_spec() {
    use Command::*;
    use StreamState::*;
    let path = |s: StreamState, c| s.path_to(c).as_slice().to_vec();
    assert_eq!(path(Initial, SetParams), [SetParams]);
    assert_eq!(path(ParamsSet, Start), [Prepare, Start]);
    assert_eq!(path(Prepared, Start), [Start]);
    assert_eq!(path(Started, Prepare), [Stop, Release, Prepare]);
    assert_eq!(path(Started, SetParams), [Stop, Release, SetParams]);
    assert_eq!(path(Stopped, Start), [Start]);
    assert_eq!(path(Started, Release), [Stop, Release]);
    assert_eq!(path(Released, Prepare), [Prepare]);
    assert!(path(Initial, Start).is_empty());
    assert!(path(Initial, Prepare).is_empty());
    assert!(path(ParamsSet, Stop).is_empty());
    // Every path only sends commands the state allows.
    for s in [Initial, ParamsSet, Prepared, Started, Stopped, Released] {
        for c in [SetParams, Prepare, Start, Stop, Release] {
            let mut st = s;
            for &cmd in s.path_to(c).as_slice() {
                assert!(st.allows(cmd), "{st:?} does not allow {cmd:?}");
                st = st.after(cmd);
            }
        }
    }
}

#[test]
fn parameters_are_checked_against_the_stream() {
    let info = output_stream();
    let want = Wanted { format: format::S16, rate_hz: 48000, channels: 2, period_bytes: 4096, buffer_bytes: 16384 };
    let p = check_params(&info, &want).unwrap();
    assert_eq!((p.rate, p.format, p.channels, p.features), (rate::R48000, format::S16, 2, 0));
    let bad = |w: Wanted| check_params(&info, &w).unwrap_err();
    assert_eq!(bad(Wanted { format: format::S24, ..want }), ParamError::Format);
    assert_eq!(bad(Wanted { rate_hz: 96000, ..want }), ParamError::Rate);
    assert_eq!(bad(Wanted { rate_hz: 44000, ..want }), ParamError::Rate);
    assert_eq!(bad(Wanted { channels: 3, ..want }), ParamError::Channels);
    assert_eq!(bad(Wanted { period_bytes: 4094, ..want }), ParamError::Period);
    assert_eq!(bad(Wanted { buffer_bytes: 6000, ..want }), ParamError::Buffer);
    assert_eq!(bad(Wanted { buffer_bytes: 2048, ..want }), ParamError::Buffer);
}

// ---- the ring --------------------------------------------------------------

#[test]
fn ring_hands_out_whole_periods_then_waits() {
    let mut r = PeriodRing::new(4096, 1024).unwrap();
    assert!(PeriodRing::new(4000, 1024).is_none() && PeriodRing::new(512, 1024).is_none());
    assert_eq!(r.room(), 4096);
    assert_eq!(r.write_spans(1500), [Chunk { offset: 0, len: 1500 }, Chunk { offset: 0, len: 0 }]);
    r.commit(1500);
    // Nothing in flight: the device would run dry, so the first chunk is the
    // whole first period, and the rest waits for its period to fill.
    assert_eq!(r.next_chunk(false), Some(Chunk { offset: 0, len: 1024 }));
    r.mark_submitted(Chunk { offset: 0, len: 1024 });
    assert_eq!(r.next_chunk(false), None);
    assert_eq!(r.next_chunk(true), Some(Chunk { offset: 1024, len: 476 }));
    r.commit(548);
    assert_eq!(r.next_chunk(false), Some(Chunk { offset: 1024, len: 1024 }));
    r.mark_submitted(Chunk { offset: 1024, len: 1024 });
    assert_eq!((r.in_flight(), r.queued(), r.room()), (2048, 2048, 2048));
    r.complete(1024);
    assert_eq!((r.played(), r.room()), (1024, 3072));
    // Writes wrap at the end of the ring.
    r.commit(2048);
    assert_eq!(r.write_spans(2000), [Chunk { offset: 0, len: 1024 }, Chunk { offset: 0, len: 0 }]);
    r.commit(1024);
    assert_eq!(r.room(), 0);
    assert_eq!(r.write_spans(10)[0].len, 0);
}

#[test]
fn ring_starving_device_gets_partial_periods_and_realigns() {
    let mut r = PeriodRing::new(4096, 1024).unwrap();
    r.commit(100);
    let c = r.next_chunk(false).unwrap();
    assert_eq!(c, Chunk { offset: 0, len: 100 });
    r.mark_submitted(c);
    r.commit(2000);
    // The rest of the first period, then a whole one.
    assert_eq!(r.next_chunk(false), Some(Chunk { offset: 100, len: 924 }));
    r.mark_submitted(Chunk { offset: 100, len: 924 });
    assert_eq!(r.next_chunk(false), Some(Chunk { offset: 1024, len: 1024 }));
    r.mark_submitted(Chunk { offset: 1024, len: 1024 });
    assert_eq!(r.next_chunk(false), None);
    r.complete(100);
    r.complete(924);
    r.complete(1024);
    r.complete(5000);
    assert_eq!(r.played(), 2048);
    assert!(!r.is_drained());
}

#[test]
fn ring_drain_pads_the_last_period_with_silence() {
    let mut r = PeriodRing::new(4096, 1024).unwrap();
    r.commit(1500);
    let pad = r.pad_to_period().unwrap();
    assert_eq!(pad, Chunk { offset: 1500, len: 548 });
    assert!(r.pad_to_period().is_none());
    for _ in 0..2 {
        let c = r.next_chunk(false).unwrap();
        assert_eq!(c.len, 1024);
        r.mark_submitted(c);
    }
    r.complete(1024);
    r.complete(1024);
    assert!(r.is_drained());
    // The pad is not the writer's.
    assert_eq!(r.played(), 1500);
    r.reset();
    assert_eq!((r.written(), r.room(), r.played()), (0, 4096, 0));
}

// ---- the driver ------------------------------------------------------------

fn transport() -> PciTransport {
    PciTransport {
        common: Region { bar: 0, offset: 0, length: 0x38 },
        notify: Region { bar: 0, offset: 0x1000, length: 16 },
        notify_off_multiplier: 4,
        isr: Region { bar: 0, offset: 0x2000, length: 1 },
        device: Some(Region { bar: 0, offset: 0x3000, length: CONFIG_LEN }),
        pci_cfg: None,
    }
}

type Driver<'a> = VirtioSound<'a, CommonRegs, NotifyRegs>;

const PCM_LEN: usize = 16384;

struct Harness {
    sim: Sim,
    rings: [DmaMemory; 4],
    control: DmaMemory,
    events: DmaMemory,
    tx_meta: DmaMemory,
    rx_meta: DmaMemory,
    pcm: DmaMemory,
    desc: [Vec<DescState>; 4],
    event_owner: Vec<u16>,
    tx_owner: Vec<u16>,
    tx_free: Vec<u16>,
    tx_len: Vec<u32>,
    rx_owner: Vec<u16>,
    rx_free: Vec<u16>,
    rx_len: Vec<u32>,
    vector: Option<u16>,
}

impl Harness {
    fn new(sim: &Sim, entries: u16) -> Harness {
        let n = entries as usize;
        let ring = || DmaMemory::new(Layout::new(entries).unwrap().total());
        Harness {
            sim: sim.clone(),
            rings: [ring(), ring(), ring(), ring()],
            control: DmaMemory::new(1024),
            events: DmaMemory::new(n * Event::LEN),
            tx_meta: DmaMemory::new(n * TX_SLOT_LEN),
            rx_meta: DmaMemory::new(n * RX_SLOT_LEN),
            pcm: DmaMemory::new(PCM_LEN),
            desc: core::array::from_fn(|_| vec![DescState::default(); n]),
            event_owner: vec![0; n],
            tx_owner: vec![0; n],
            tx_free: vec![0; n],
            tx_len: vec![0; n],
            rx_owner: vec![0; n],
            rx_free: vec![0; n],
            rx_len: vec![0; n],
            vector: None,
        }
    }

    fn try_init(&mut self) -> Result<Driver<'_>, InitError> {
        let [d0, d1, d2, d3] = &mut self.desc;
        let setup = Setup {
            common: poly_virtio::CommonCfg::new(self.sim.common()),
            device_cfg: self.sim.config(),
            device_cfg_len: CONFIG_LEN,
            transport: transport(),
            notify: self.sim.notify(),
            memory: Memory {
                rings: core::array::from_fn(|i| self.rings[i].region()),
                control: self.control.region(),
                events: self.events.region(),
                tx_meta: self.tx_meta.region(),
                rx_meta: self.rx_meta.region(),
                pcm: self.pcm.region(),
            },
            storage: Storage {
                desc: [d0, d1, d2, d3],
                event_owner: &mut self.event_owner,
                tx_owner: &mut self.tx_owner,
                tx_free: &mut self.tx_free,
                tx_len: &mut self.tx_len,
                rx_owner: &mut self.rx_owner,
                rx_free: &mut self.rx_free,
                rx_len: &mut self.rx_len,
            },
            vector: self.vector,
        };
        // SAFETY: the harness owns the memory and outlives the driver; only
        // the driver and the simulated device use it.
        unsafe { init(setup) }
    }

    fn init(&mut self) -> Driver<'_> {
        self.try_init().unwrap()
    }
}

fn chmap_stereo() -> ChmapInfo {
    let mut positions = [0u8; chmap::MAX_SIZE];
    positions[..2].copy_from_slice(&[chmap::FL, chmap::FR]);
    ChmapInfo { hda_fn_nid: 0, direction: direction::OUTPUT, channels: 2, positions }
}

fn qemu_like() -> Sim {
    let mut input = output_stream();
    input.direction = direction::INPUT;
    Sim::new(VERSION_1 | EVENT_IDX, 64, vec![output_stream(), input], Vec::new(), vec![chmap_stereo()])
}

/// A `wait` that allows `n` more looks at the used ring.
fn patience(mut n: u32) -> impl FnMut() -> bool {
    move || {
        n = n.saturating_sub(1);
        n > 0
    }
}

#[test]
fn init_sets_up_four_queues_and_posts_event_buffers() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let snd = h.init();
    assert_eq!(snd.config(), proto::DeviceConfig { jacks: 0, streams: 2, chmaps: 1 });
    assert_eq!(snd.queue_sizes(), [32, 32, 32, 32]);
    // Three descriptors per output message.
    assert_eq!(snd.tx_slots(), 10);
    assert!(sim.negotiated(VERSION_1) && sim.negotiated(EVENT_IDX));
    assert_ne!(sim.0.status.get() & poly_virtio::status::DRIVER_OK, 0);
    assert_eq!(sim.event_buffers(), 32);
    for q in 0..4 {
        assert_eq!(sim.queue(q).enable, 1, "queue {q}");
    }
}

#[test]
fn init_refuses_a_short_configuration_and_too_little_memory() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    h.control = DmaMemory::new(64);
    assert_eq!(h.try_init().unwrap_err(), InitError::NoRoom);
    assert_ne!(sim.0.status.get() & poly_virtio::status::FAILED, 0);
    let mut h = Harness::new(&sim, 4);
    assert_eq!(h.try_init().unwrap_err(), InitError::NoRoom, "4 entries hold one output message");
}

#[test]
fn queries_return_streams_jacks_and_channel_maps() {
    let jack = JackInfo { hda_fn_nid: 0, features: 0, hda_reg_defconf: 0x0101_0010, hda_reg_caps: 0, connected: true };
    let streams: Vec<PcmInfo> = (0..40).map(|_| output_stream()).collect();
    let sim = Sim::new(VERSION_1, 32, streams, vec![jack], vec![chmap_stereo()]);
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    let mut infos = vec![PcmInfo::default(); 64];
    // 40 streams of 32 bytes do not fit one 960-byte response: two requests.
    assert_eq!(snd.query_streams(&mut infos, &mut patience(10)), Ok(40));
    assert!(infos[..40].iter().all(|i| *i == output_stream()));
    assert_eq!(sim.dev().requests, [code::PCM_INFO, code::PCM_INFO]);
    let mut jacks = [JackInfo::default(); 4];
    assert_eq!(snd.query_jacks(&mut jacks, &mut patience(10)), Ok(1));
    assert_eq!(jacks[0], jack);
    let mut maps = [ChmapInfo::default(); 2];
    assert_eq!(snd.query_chmaps(&mut maps, &mut patience(10)), Ok(1));
    assert_eq!(maps[0].used_positions(), &[chmap::FL, chmap::FR]);
    // A device that refuses.
    sim.dev().force_status = Some(status::NOT_SUPP);
    assert_eq!(snd.query_jacks(&mut jacks, &mut patience(10)), Err(CtlError::Status(status::NOT_SUPP)));
    assert!(!snd.is_failed());
}

fn s16_stereo(period: u32, buffer: u32) -> SetParams {
    check_params(
        &output_stream(),
        &Wanted { format: format::S16, rate_hz: 48000, channels: 2, period_bytes: period, buffer_bytes: buffer },
    )
    .unwrap()
}

#[test]
fn stream_commands_walk_the_lifecycle() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    let mut st = StreamState::default();
    let p = s16_stereo(1024, 4096);
    assert_eq!(snd.pcm_goto(&mut st, 0, Command::Start, None, &mut patience(10)), Err(CtlError::State));
    snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&p), &mut patience(10)).unwrap();
    assert_eq!(sim.dev().streams[0].params, Some(p));
    snd.pcm_goto(&mut st, 0, Command::Start, None, &mut patience(10)).unwrap();
    assert_eq!((st, sim.dev().streams[0].state), (StreamState::Started, DevState::Running));
    snd.pcm_goto(&mut st, 0, Command::Prepare, None, &mut patience(10)).unwrap();
    assert_eq!(st, StreamState::Prepared);
    let r = sim.dev().requests.clone();
    assert_eq!(
        r,
        [
            code::PCM_SET_PARAMS,
            code::PCM_PREPARE,
            code::PCM_START,
            code::PCM_STOP,
            code::PCM_RELEASE,
            code::PCM_PREPARE
        ]
    );
    // Parameters the device does not take.
    let mut bad = p;
    bad.rate = 12;
    assert_eq!(
        snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&bad), &mut patience(10)),
        Err(CtlError::Status(status::NOT_SUPP))
    );
    assert_eq!(st, StreamState::Prepared, "a refused command leaves the state");
}

#[test]
fn a_response_with_a_short_used_length_is_taken_by_its_status() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    sim.dev().short_control = true;
    let mut infos = [PcmInfo::default(); 2];
    assert_eq!(snd.query_streams(&mut infos, &mut patience(10)), Ok(2));
    assert_eq!(infos[0], output_stream());
    let mut st = StreamState::default();
    snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&s16_stereo(1024, 4096)), &mut patience(10)).unwrap();
    assert_eq!(snd.stats().short_responses, 1);
    // A response whose status was never written is still an error.
    sim.dev().force_status = Some(0);
    assert_eq!(snd.query_streams(&mut infos, &mut patience(10)), Err(CtlError::Short));
}

#[test]
fn a_device_that_does_not_answer_is_reset() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    sim.dev().answer = false;
    let mut infos = [PcmInfo::default(); 2];
    assert_eq!(snd.query_streams(&mut infos, &mut patience(5)), Err(CtlError::Timeout));
    assert!(snd.is_failed());
    assert_eq!(sim.0.status.get(), poly_virtio::status::FAILED);
    assert_eq!(snd.query_streams(&mut infos, &mut patience(5)), Err(CtlError::Failed));
    assert_eq!(snd.submit(0, Chunk { offset: 0, len: 4 }), Err(TxError::Failed));
}

/// Play a 1 kHz-ish pattern through the ring and the driver, the way the
/// kernel does: write, submit what the ring hands out, collect returns.
#[test]
fn playback_feeds_periods_and_gets_them_back_in_order() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    h.vector = Some(0);
    let mut snd = h.init();
    let mut st = StreamState::default();
    let p = s16_stereo(1024, 4096);
    snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&p), &mut patience(10)).unwrap();
    snd.pcm_goto(&mut st, 0, Command::Prepare, None, &mut patience(10)).unwrap();
    let mut ring = PeriodRing::new(p.buffer_bytes, p.period_bytes).unwrap();
    let data: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + i / 300) as u8).collect();
    let mut fed = 0;
    let mut started = false;
    let mut returned = 0u64;
    let mut rounds = 0;
    while (ring.played() as usize) < data.len() {
        rounds += 1;
        assert!(rounds < 1000, "stuck");
        // Write what fits.
        for span in ring.write_spans((data.len() - fed) as u32) {
            assert!(snd.pcm_write(span.offset as usize, &data[fed..fed + span.len as usize]));
            fed += span.len as usize;
            ring.commit(span.len);
        }
        if !started && ring.room() == 0 {
            snd.pcm_goto(&mut st, 0, Command::Start, None, &mut patience(10)).unwrap();
            started = true;
        }
        if started {
            let drain = fed == data.len();
            while snd.tx_in_flight() < snd.tx_slots()
                && let Some(c) = ring.next_chunk(drain)
            {
                snd.submit(0, c).unwrap();
                ring.mark_submitted(c);
            }
            sim.play(2);
            snd.poll_tx(|d| {
                assert_eq!((d.stream, d.status.status), (0, status::OK));
                ring.complete(d.len);
                returned += d.len as u64;
            });
        }
    }
    let dev = sim.dev();
    assert_eq!(dev.streams[0].played, data, "the device got every byte, in order");
    assert!(dev.streams[0].messages.iter().all(|&m| m <= 1024));
    assert_eq!(returned, data.len() as u64);
    assert!(dev.queues[2].interrupts > 0, "returned messages interrupt");
    drop(dev);
    assert_eq!(snd.stats().tx_bytes, data.len() as u64);
    assert_eq!(snd.tx_in_flight(), 0);
}

#[test]
fn release_returns_messages_still_with_the_device() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    let mut st = StreamState::default();
    let p = s16_stereo(1024, 4096);
    snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&p), &mut patience(10)).unwrap();
    snd.pcm_goto(&mut st, 0, Command::Start, None, &mut patience(10)).unwrap();
    for i in 0..4 {
        snd.submit(0, Chunk { offset: i * 1024, len: 1024 }).unwrap();
    }
    assert_eq!(snd.tx_in_flight(), 4);
    assert_eq!(sim.play(1), 1);
    snd.pcm_goto(&mut st, 0, Command::Release, None, &mut patience(10)).unwrap();
    let mut back = 0;
    snd.poll_tx(|_| back += 1);
    assert_eq!((back, snd.tx_in_flight(), sim.pending()), (4, 0, 0));
}

#[test]
fn submit_checks_range_and_slots() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 8);
    let mut snd = h.init();
    assert_eq!(snd.tx_slots(), 2);
    assert_eq!(snd.submit(0, Chunk { offset: 0, len: 0 }), Err(TxError::Range));
    assert_eq!(snd.submit(0, Chunk { offset: PCM_LEN as u32 - 2, len: 4 }), Err(TxError::Range));
    snd.submit(0, Chunk { offset: 0, len: 4 }).unwrap();
    snd.submit(0, Chunk { offset: 4, len: 4 }).unwrap();
    assert_eq!(snd.submit(0, Chunk { offset: 8, len: 4 }), Err(TxError::Busy));
    assert!(snd.pcm_fill(0, 6, &[0x80]));
    let mut b = [0u8; 8];
    assert!(snd.pcm_read(0, &mut b));
    assert_eq!(b, [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0, 0]);
    assert!(!snd.pcm_write(PCM_LEN - 1, &[1, 2]));
}

#[test]
fn a_short_status_is_an_io_error() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    let mut st = StreamState::default();
    snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&s16_stereo(1024, 4096)), &mut patience(10)).unwrap();
    snd.pcm_goto(&mut st, 0, Command::Start, None, &mut patience(10)).unwrap();
    snd.submit(0, Chunk { offset: 0, len: 1024 }).unwrap();
    sim.dev().short_status = Some(4);
    sim.play(1);
    let mut got = None;
    snd.poll_tx(|d| got = Some(d));
    assert_eq!(got.unwrap().status.status, status::IO_ERR);
    assert_eq!(snd.stats().tx_errors, 1);
}

#[test]
fn events_are_decoded_and_buffers_reposted() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 8);
    let mut snd = h.init();
    for _ in 0..20 {
        assert!(sim.send_event(Event { code: code::EVT_PCM_XRUN, data: 0 }));
        assert!(sim.send_event(Event { code: code::EVT_JACK_CONNECTED, data: 1 }));
        let mut got = Vec::new();
        snd.poll_events(|e| got.push(e));
        assert_eq!(
            got,
            [Event { code: code::EVT_PCM_XRUN, data: 0 }, Event { code: code::EVT_JACK_CONNECTED, data: 1 }]
        );
    }
    assert_eq!(sim.event_buffers(), 8);
    let s = snd.stats();
    assert_eq!((s.events, s.xruns, s.jack_events), (40, 20, 20));
}

#[test]
fn a_used_element_with_a_bogus_id_fails_the_device() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    let mut st = StreamState::default();
    snd.pcm_goto(&mut st, 0, Command::SetParams, Some(&s16_stereo(1024, 4096)), &mut patience(10)).unwrap();
    snd.pcm_goto(&mut st, 0, Command::Start, None, &mut patience(10)).unwrap();
    snd.submit(0, Chunk { offset: 0, len: 1024 }).unwrap();
    sim.dev().bogus_id = Some(31);
    sim.play(1);
    snd.poll_tx(|_| panic!("nothing valid came back"));
    assert!(snd.is_failed());
    assert_eq!(sim.0.status.get(), poly_virtio::status::FAILED);
}

#[test]
fn interrupts_are_asked_for_on_the_event_and_transmit_queues() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    let mut snd = h.init();
    snd.disable_interrupts();
    assert!(!snd.enable_interrupts());
    assert!(sim.send_event(Event { code: code::EVT_PCM_PERIOD_ELAPSED, data: 0 }));
    assert!(snd.has_work());
    assert!(snd.enable_interrupts(), "work that arrived is reported");
    snd.poll_events(|_| {});
    assert!(!snd.has_work());
}

fn s16_input(period: u32, buffer: u32) -> SetParams {
    let mut info = output_stream();
    info.direction = direction::INPUT;
    check_params(
        &info,
        &Wanted { format: format::S16, rate_hz: 48000, channels: 2, period_bytes: period, buffer_bytes: buffer },
    )
    .unwrap()
}

/// Capture the way the kernel does: lend periods to the device, collect
/// them filled, read the data, lend them again.
#[test]
fn capture_lends_periods_and_gets_them_back_filled() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 32);
    h.vector = Some(0);
    let mut snd = h.init();
    assert_eq!(snd.rx_slots(), 10);
    let mut st = StreamState::default();
    let p = s16_input(1024, 4096);
    snd.pcm_goto(&mut st, 1, Command::SetParams, Some(&p), &mut patience(10)).unwrap();
    snd.pcm_goto(&mut st, 1, Command::Prepare, None, &mut patience(10)).unwrap();
    // Capture lives in the upper half of the PCM memory.
    let base = 8192u32;
    for i in 0..4 {
        snd.submit_rx(1, Chunk { offset: base + i * 1024, len: 1024 }).unwrap();
    }
    assert_eq!((snd.rx_in_flight(), sim.rx_pending()), (4, 4));
    // Not running yet: nothing is recorded.
    assert_eq!(sim.record(4, |_, _| {}), 0);
    snd.pcm_goto(&mut st, 1, Command::Start, None, &mut patience(10)).unwrap();
    let mut counter = 0u8;
    let mut source = |stream: u32, buf: &mut [u8]| {
        assert_eq!(stream, 1);
        for b in buf {
            *b = counter;
            counter = counter.wrapping_add(1);
        }
    };
    let mut got = Vec::new();
    for _ in 0..10 {
        assert_eq!(sim.record(2, &mut source), 2);
        let mut back = Vec::new();
        snd.poll_rx(|d| back.push(d));
        assert_eq!(back.len(), 2);
        for d in back {
            assert_eq!((d.stream, d.len, d.status.status), (1, 1024, status::OK));
            let mut b = vec![0u8; d.len as usize];
            assert!(snd.pcm_read(d.offset as usize, &mut b));
            got.extend_from_slice(&b);
            snd.submit_rx(1, Chunk { offset: d.offset, len: d.len }).unwrap();
        }
    }
    let want: Vec<u8> = (0..got.len()).map(|i| i as u8).collect();
    assert_eq!(got, want, "every captured byte, in order");
    assert!(sim.dev().queues[3].interrupts > 0, "filled input messages interrupt");
    let s = snd.stats();
    assert_eq!((s.rx_messages, s.rx_bytes), (20, 20 * 1024));
    // RELEASE hands back what the device still holds, empty.
    snd.pcm_goto(&mut st, 1, Command::Release, None, &mut patience(10)).unwrap();
    let mut empty = 0;
    snd.poll_rx(|d| {
        assert_eq!(d.len, 0);
        empty += 1;
    });
    assert_eq!((empty, snd.rx_in_flight()), (4, 0));
}

#[test]
fn input_messages_check_range_and_take_a_short_used_length_as_full() {
    let sim = qemu_like();
    let mut h = Harness::new(&sim, 8);
    let mut snd = h.init();
    assert_eq!(snd.rx_slots(), 2);
    assert_eq!(snd.submit_rx(1, Chunk { offset: 0, len: 0 }), Err(TxError::Range));
    assert_eq!(snd.submit_rx(1, Chunk { offset: PCM_LEN as u32 - 2, len: 4 }), Err(TxError::Range));
    let mut st = StreamState::default();
    snd.pcm_goto(&mut st, 1, Command::SetParams, Some(&s16_input(1024, 4096)), &mut patience(10)).unwrap();
    snd.pcm_goto(&mut st, 1, Command::Start, None, &mut patience(10)).unwrap();
    snd.submit_rx(1, Chunk { offset: 0, len: 512 }).unwrap();
    snd.submit_rx(1, Chunk { offset: 512, len: 512 }).unwrap();
    assert_eq!(snd.submit_rx(1, Chunk { offset: 1024, len: 512 }), Err(TxError::Busy));
    // A used length of 0 with an OK status: the whole buffer.
    sim.dev().rx_used_len = Some(0);
    sim.record(1, |_, b| b.fill(7));
    // One that covers 100 bytes of data and the status.
    sim.dev().rx_used_len = Some(108);
    sim.record(1, |_, b| b.fill(9));
    let mut lens = Vec::new();
    snd.poll_rx(|d| lens.push((d.offset, d.len)));
    assert_eq!(lens, [(0, 512), (512, 100)]);
    assert_eq!(snd.stats().rx_short, 1);
}
