//! Tests against the simulated device in `sim`, and the hub fed by the
//! driver.

use super::*;
use crate::sim::{CONFIG_LEN, CommonRegs, DmaMemory, NotifyRegs, Sim};
use poly_input::codes::{abs, key, led, prop, rel, syn};
use poly_input::hub::{Event, Hub, QUEUE_EVENTS, Queue};
use poly_virtio::{DescState, Layout, PciTransport, Region, status};
use std::{vec, vec::Vec};

const VERSION_1: u64 = poly_virtio::feature::VERSION_1;
const EVENT_IDX: u64 = poly_virtio::feature::EVENT_IDX;

type Driver<'a> = VirtioInput<'a, CommonRegs, NotifyRegs>;

fn transport() -> PciTransport {
    PciTransport {
        common: Region { bar: 0, offset: 0, length: 0x38 },
        notify: Region { bar: 0, offset: 0x1000, length: 8 },
        notify_off_multiplier: 4,
        isr: Region { bar: 0, offset: 0x2000, length: 1 },
        device: Some(Region { bar: 0, offset: 0x3000, length: CONFIG_LEN }),
        pci_cfg: None,
    }
}

fn bitmap(bits: &[u16]) -> Vec<u8> {
    let len = bits.iter().map(|&b| usize::from(b) / 8 + 1).max().unwrap_or(0);
    let mut v = vec![0u8; len];
    for &b in bits {
        v[usize::from(b) / 8] |= 1 << (b % 8);
    }
    v
}

fn absinfo(min: i32, max: i32, res: i32) -> Vec<u8> {
    [min, max, 0, 0, res].iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A configuration like QEMU's virtio-tablet: absolute X and Y, buttons, a
/// wheel.
fn tablet() -> Vec<((u8, u8), Vec<u8>)> {
    vec![
        ((select::ID_NAME, 0), b"Sim Virtio Tablet".to_vec()),
        ((select::ID_SERIAL, 0), b"0001\0\0".to_vec()),
        ((select::ID_DEVIDS, 0), vec![0x06, 0, 0x27, 0x06, 0x03, 0, 0x01, 0]),
        ((select::PROP_BITS, 0), bitmap(&[prop::DIRECT])),
        ((select::EV_BITS, ev::KEY as u8), bitmap(&[key::BTN_LEFT, key::BTN_RIGHT, key::BTN_MIDDLE])),
        ((select::EV_BITS, ev::REL as u8), bitmap(&[rel::WHEEL])),
        ((select::EV_BITS, ev::ABS as u8), bitmap(&[abs::X, abs::Y])),
        ((select::ABS_INFO, abs::X as u8), absinfo(0, 32767, 0)),
        ((select::ABS_INFO, abs::Y as u8), absinfo(0, 32767, 0)),
    ]
}

/// A keyboard: letters and more, LEDs, auto-repeat.
fn keyboard() -> Vec<((u8, u8), Vec<u8>)> {
    let keys: Vec<u16> = (1..=127).collect();
    vec![
        ((select::ID_NAME, 0), b"Sim Virtio Keyboard".to_vec()),
        ((select::EV_BITS, ev::KEY as u8), bitmap(&keys)),
        ((select::EV_BITS, ev::LED as u8), bitmap(&[led::NUML, led::CAPSL, led::SCROLLL])),
        ((select::EV_BITS, ev::REP as u8), bitmap(&[0, 1])),
    ]
}

/// The driver's memory and tables, for queues of up to `entries`.
struct Harness {
    sim: Sim,
    event_ring: DmaMemory,
    event_bufs: DmaMemory,
    status_ring: DmaMemory,
    status_bufs: DmaMemory,
    event_desc: Vec<DescState>,
    status_desc: Vec<DescState>,
    vector: Option<u16>,
}

impl Harness {
    fn new(sim: &Sim, entries: u16) -> Harness {
        let ring = || DmaMemory::new(Layout::new(entries).unwrap().total());
        let bufs = || DmaMemory::new(entries as usize * EVENT_LEN);
        Harness {
            sim: sim.clone(),
            event_ring: ring(),
            event_bufs: bufs(),
            status_ring: ring(),
            status_bufs: bufs(),
            event_desc: vec![DescState::default(); entries as usize],
            status_desc: vec![DescState::default(); entries as usize],
            vector: None,
        }
    }

    fn try_init(&mut self) -> Result<Driver<'_>, InitError> {
        let setup = Setup {
            common: poly_virtio::CommonCfg::new(self.sim.common()),
            device_cfg: self.sim.config(),
            device_cfg_len: CONFIG_LEN,
            transport: transport(),
            notify: self.sim.notify(),
            event: QueueMemory { ring: self.event_ring.region(), buffers: self.event_bufs.region() },
            status: QueueMemory { ring: self.status_ring.region(), buffers: self.status_bufs.region() },
            storage: Storage { event_desc: &mut self.event_desc, status_desc: &mut self.status_desc },
            vector: self.vector,
        };
        // SAFETY: the harness owns the memory and outlives the driver; only
        // the driver and the simulated device use it.
        unsafe { init(setup) }
    }
}

fn poll_all(d: &mut Driver<'_>) -> Vec<RawEvent> {
    let mut out = [(0, 0, 0); 16];
    let mut all = Vec::new();
    loop {
        let n = d.poll(&mut out);
        all.extend_from_slice(&out[..n]);
        if n < out.len() {
            return all;
        }
    }
}

#[test]
fn reads_a_tablet_configuration() {
    let sim = Sim::new(VERSION_1 | EVENT_IDX, 64, &tablet());
    let mut h = Harness::new(&sim, 64);
    let d = h.try_init().unwrap();
    let info = d.info();
    assert_eq!(info.name, "Sim Virtio Tablet");
    assert_eq!(info.uniq, "0001");
    assert_eq!((info.id.bustype, info.id.vendor, info.id.product, info.id.version), (0x06, 0x0627, 0x0003, 0x0001));
    assert!(info.caps.prop.get(prop::DIRECT));
    assert!(info.caps.has(ev::ABS, abs::X) && info.caps.has(ev::ABS, abs::Y));
    assert!(info.caps.has(ev::KEY, key::BTN_LEFT) && info.caps.has(ev::REL, rel::WHEEL));
    assert!(!info.caps.has(ev::REL, rel::X));
    assert_eq!((info.abs[0].minimum, info.abs[0].maximum), (0, 32767));
    assert!(!info.console && !info.soft_repeat);
    assert_eq!(d.features(), VERSION_1 | EVENT_IDX);
    assert_eq!(d.event_queue_size(), 64);
    assert_eq!(d.status_queue_size(), 16);
    assert_eq!(sim.0.status.get(), status::ACKNOWLEDGE | status::DRIVER | status::FEATURES_OK | status::DRIVER_OK);
}

#[test]
fn a_keyboard_is_a_console_keyboard_with_soft_repeat() {
    let sim = Sim::new(VERSION_1, 8, &keyboard());
    let mut h = Harness::new(&sim, 8);
    let d = h.try_init().unwrap();
    let info = d.info();
    assert!(info.console && info.soft_repeat);
    assert!(info.caps.ev.get(ev::REP) && info.caps.has(ev::LED, led::CAPSL));
    assert_eq!(info.id.bustype, poly_input::codes::bus::VIRTUAL, "no ids: BUS_VIRTUAL");
    assert_eq!(info.uniq, "");
}

#[test]
fn events_come_in_and_buffers_go_back() {
    let sim = Sim::new(VERSION_1 | EVENT_IDX, 8, &tablet());
    let mut h = Harness::new(&sim, 8);
    let mut d = h.try_init().unwrap();
    assert_eq!(poll_all(&mut d), []);
    let frame: Vec<RawEvent> = vec![(ev::ABS, abs::X, 100), (ev::ABS, abs::Y, 200), (ev::SYN, syn::REPORT, 0)];
    // More events than buffers: the rest wait until buffers come back.
    let mut all = Vec::new();
    for _ in 0..10 {
        all.extend_from_slice(&frame);
    }
    sim.inject(&all);
    assert_eq!(sim.dev().inbox.len(), 30 - 8);
    let mut got = Vec::new();
    for _ in 0..10 {
        got.extend(poll_all(&mut d));
    }
    assert_eq!(got, all);
    assert_eq!(d.stats().events, 30);
    assert!(sim.dev().inbox.is_empty());
    // Into the hub, as the kernel does it.
    let mut hub = Hub::new();
    let dev = hub.add(d.info().clone());
    let c = hub.open(dev, Queue::new(QUEUE_EVENTS)).unwrap();
    sim.inject(&frame);
    for (t, c2, v) in poll_all(&mut d) {
        let _ = hub.report(dev, 5, t, c2, v);
    }
    let mut out = [Event::default(); 8];
    assert_eq!(hub.read(dev, c, &mut out), Ok(3));
    assert_eq!(out[1], Event::new(5, ev::ABS, abs::Y, 200));
}

#[test]
fn leds_go_out_on_the_status_queue() {
    let sim = Sim::new(VERSION_1 | EVENT_IDX, 8, &keyboard());
    let mut h = Harness::new(&sim, 8);
    let mut d = h.try_init().unwrap();
    let leds = [(ev::LED, led::NUML, 1), (ev::LED, led::CAPSL, 0), (ev::LED, led::SCROLLL, 0), (ev::SYN, 0, 0)];
    for _ in 0..5 {
        assert_eq!(d.send(&leds), Ok(()));
    }
    assert_eq!(sim.dev().status.len(), 20);
    assert_eq!(&sim.dev().status[..4], &leds);
    assert_eq!(d.stats().sent, 20);
    let too_many = [(ev::SYN, 0, 0); 9];
    assert_eq!(d.send(&too_many), Err(SendError::TooMany));
}

#[test]
fn interrupts_and_short_buffers() {
    let sim = Sim::new(VERSION_1, 8, &tablet());
    let mut h = Harness::new(&sim, 8);
    h.vector = Some(0);
    let mut d = h.try_init().unwrap();
    assert_eq!((sim.queue(0).msix, sim.queue(1).msix, sim.0.config_msix.get()), (0, 0, 0));
    d.disable_interrupts();
    sim.inject(&[(ev::KEY, key::BTN_LEFT, 1)]);
    assert_eq!(sim.dev().events.interrupts, 0, "suppressed");
    assert!(d.enable_interrupts(), "an event came in meanwhile");
    assert_eq!(poll_all(&mut d), [(ev::KEY, key::BTN_LEFT, 1)]);
    sim.inject(&[(ev::KEY, key::BTN_LEFT, 0)]);
    assert_eq!(sim.dev().events.interrupts, 1);
    // A buffer handed back short is not an event, and is reused.
    sim.dev().short_len = Some(4);
    sim.inject(&[(ev::KEY, key::BTN_LEFT, 1), (ev::KEY, key::BTN_LEFT, 0)]);
    assert_eq!(poll_all(&mut d), [(ev::KEY, key::BTN_LEFT, 0), (ev::KEY, key::BTN_LEFT, 0)]);
    assert_eq!(d.stats().short, 1);
}

#[test]
fn a_broken_device_is_reset_and_refused() {
    let sim = Sim::new(VERSION_1, 8, &tablet());
    let mut h = Harness::new(&sim, 8);
    let mut d = h.try_init().unwrap();
    sim.dev().bogus_id = Some(77);
    sim.inject(&[(ev::KEY, key::BTN_LEFT, 1)]);
    assert_eq!(poll_all(&mut d), []);
    assert!(d.is_failed());
    assert_eq!(sim.0.status.get(), status::FAILED);
    assert_eq!(d.send(&[(ev::SYN, 0, 0)]), Err(SendError::Failed));
    assert!(!d.enable_interrupts());
}

#[test]
fn bring_up_failures() {
    // A legacy-only device (no VERSION_1) is refused.
    let sim = Sim::new(0, 8, &tablet());
    let mut h = Harness::new(&sim, 8);
    assert!(matches!(h.try_init(), Err(InitError::Virtio(_))));
    // A configuration that never holds still.
    let sim = Sim::new(VERSION_1, 8, &tablet());
    sim.0.unstable.set(u32::MAX);
    let mut h = Harness::new(&sim, 8);
    assert_eq!(h.try_init().err(), Some(InitError::Config(ConfigError::Unstable)));
    assert_eq!(sim.0.status.get() & status::FAILED, status::FAILED);
    // Queues of one entry are too small.
    let sim = Sim::new(VERSION_1, 1, &tablet());
    let mut h = Harness::new(&sim, 8);
    assert_eq!(h.try_init().err(), Some(InitError::NoRoom));
    // A configuration region too short for its header.
    let sim = Sim::new(VERSION_1, 8, &tablet());
    let common = poly_virtio::CommonCfg::new(sim.common());
    assert_eq!(read_info(&common, &sim.config(), 4).err(), Some(ConfigError::TooShort));
}

#[test]
fn event_bytes_round_trip() {
    for e in [(0u16, 0u16, 0i32), (ev::REL, rel::X, -5), (0xffff, 0x1234, i32::MIN)] {
        assert_eq!(parse_event(&event_bytes(e)), e);
    }
    assert_eq!(event_bytes((1, 30, 1)), [1, 0, 30, 0, 1, 0, 0, 0]);
}
