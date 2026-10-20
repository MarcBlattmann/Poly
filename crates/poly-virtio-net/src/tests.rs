//! Tests against the simulated device in `sim`, and a TCP/IP stack running
//! over the driver.

use super::*;
use crate::sim::{CONFIG_LEN, CommonRegs, ConfigRegs, DmaMemory, NotifyRegs, SIM_MAC, Sim};
use poly_net::{NetDevice, TxError};
use poly_virtio::{DescState, Layout, PciTransport, Region, status};
extern crate std;
use std::collections::VecDeque;
use std::{vec, vec::Vec};

const VERSION_1: u64 = poly_virtio::feature::VERSION_1;
const EVENT_IDX: u64 = poly_virtio::feature::EVENT_IDX;
/// What QEMU's virtio-net offers by default, roughly: offloads, the control
/// queue and more than this driver takes.
const QEMU_LIKE: u64 = VERSION_1
    | EVENT_IDX
    | feature::CSUM
    | feature::GUEST_CSUM
    | feature::CTRL_GUEST_OFFLOADS
    | feature::MAC
    | feature::GUEST_TSO4
    | feature::GUEST_TSO6
    | feature::GUEST_ECN
    | feature::GUEST_UFO
    | feature::HOST_TSO4
    | feature::HOST_TSO6
    | feature::HOST_ECN
    | feature::HOST_UFO
    | feature::MRG_RXBUF
    | feature::STATUS
    | feature::CTRL_VQ
    | feature::CTRL_RX
    | feature::CTRL_VLAN
    | feature::GUEST_ANNOUNCE
    | feature::CTRL_MAC_ADDR;

type Driver<'a> = VirtioNet<'a, CommonRegs, ConfigRegs, NotifyRegs>;

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

/// The driver's memory and tables, for queues of up to `entries`.
struct Harness {
    sim: Sim,
    rx_ring: DmaMemory,
    rx_bufs: DmaMemory,
    tx_ring: DmaMemory,
    tx_bufs: DmaMemory,
    rx_desc: Vec<DescState>,
    tx_desc: Vec<DescState>,
    rx_owner: Vec<u16>,
    tx_owner: Vec<u16>,
    tx_free: Vec<u16>,
    vectors: (Option<u16>, Option<u16>, Option<u16>),
}

impl Harness {
    fn new(sim: &Sim, entries: u16) -> Harness {
        let ring = || DmaMemory::new(Layout::new(entries).unwrap().total());
        let bufs = || DmaMemory::new(entries as usize * BUF_LEN);
        let n = entries as usize;
        Harness {
            sim: sim.clone(),
            rx_ring: ring(),
            rx_bufs: bufs(),
            tx_ring: ring(),
            tx_bufs: bufs(),
            rx_desc: vec![DescState::default(); n],
            tx_desc: vec![DescState::default(); n],
            rx_owner: vec![0; n],
            tx_owner: vec![0; n],
            tx_free: vec![0; n],
            vectors: (None, None, None),
        }
    }

    fn try_init(&mut self) -> Result<Driver<'_>, InitError> {
        let setup = Setup {
            common: poly_virtio::CommonCfg::new(self.sim.common()),
            device_cfg: self.sim.config(),
            device_cfg_len: CONFIG_LEN,
            transport: transport(),
            notify: self.sim.notify(),
            rx: PacketMemory { ring: self.rx_ring.region(), buffers: self.rx_bufs.region() },
            tx: PacketMemory { ring: self.tx_ring.region(), buffers: self.tx_bufs.region() },
            storage: Storage {
                rx_desc: &mut self.rx_desc,
                tx_desc: &mut self.tx_desc,
                rx_owner: &mut self.rx_owner,
                tx_owner: &mut self.tx_owner,
                tx_free: &mut self.tx_free,
            },
            rx_vector: self.vectors.0,
            tx_vector: self.vectors.1,
            config_vector: self.vectors.2,
            mac_seed: [0xff, 1, 2, 3, 4, 5],
        };
        // SAFETY: the harness owns the memory and outlives the driver; only
        // the driver and the simulated device use it.
        unsafe { init(setup) }
    }

    fn init(&mut self) -> Driver<'_> {
        self.try_init().unwrap()
    }
}

/// An Ethernet frame of `len` bytes: broadcast, from SIM_MAC, type 0x88b5
/// (IEEE local experimental), payload a pattern of `tag`.
fn frame(len: usize, tag: u8) -> Vec<u8> {
    let mut f = vec![0u8; len];
    f[..6].fill(0xff);
    f[6..12].copy_from_slice(&SIM_MAC);
    f[12..14].copy_from_slice(&0x88b5u16.to_be_bytes());
    for (i, b) in f[14..].iter_mut().enumerate() {
        *b = tag ^ (i as u8).wrapping_mul(7) ^ (i >> 8) as u8;
    }
    f
}

#[test]
fn feature_selection() {
    let ours = select_features(QEMU_LIKE | feature::MTU, Some(1500));
    assert_eq!(ours, feature::MAC | feature::STATUS | feature::MTU | feature::MRG_RXBUF | EVENT_IDX);
    // Offloads, the control queue and multiqueue are never taken.
    let never = feature::CSUM
        | feature::GUEST_CSUM
        | feature::GUEST_TSO4
        | feature::HOST_TSO4
        | feature::CTRL_VQ
        | feature::MQ
        | feature::RSS
        | feature::HASH_REPORT
        | feature::GUEST_ANNOUNCE;
    assert_eq!(select_features(never, Some(1500)), 0);
    // The MTU only when the driver can receive a packet that size.
    assert_eq!(select_features(feature::MTU, Some(2022)), feature::MTU);
    assert_eq!(select_features(feature::MTU, Some(2023)), 0, "does not fit one buffer");
    assert_eq!(select_features(feature::MTU | feature::MRG_RXBUF, Some(9000)), feature::MTU | feature::MRG_RXBUF);
    assert_eq!(select_features(feature::MTU | feature::MRG_RXBUF, Some(67)), feature::MRG_RXBUF, "below IPv4's 68");
    assert_eq!(select_features(feature::MTU, None), 0);
    assert_eq!(effective_mtu(None), 1500);
    assert_eq!(effective_mtu(Some(1400)), 1400);
    assert_eq!(effective_mtu(Some(9000)), 1500);
}

#[test]
fn header_and_mac_helpers() {
    let h = NetHeader {
        flags: hdr_flags::DATA_VALID,
        gso_type: gso::TCPV4,
        hdr_len: 0x1234,
        gso_size: 0x5678,
        csum_start: 34,
        csum_offset: 16,
        num_buffers: 3,
    };
    let b = h.to_bytes();
    assert_eq!(b, [2, 1, 0x34, 0x12, 0x78, 0x56, 34, 0, 16, 0, 3, 0]);
    assert_eq!(NetHeader::from_bytes(&b), h);
    assert_eq!(NetHeader::TRANSMIT.to_bytes(), [0; HEADER_LEN]);
    // Locally administered, never multicast.
    assert_eq!(local_mac([0xff, 1, 2, 3, 4, 5]), [0xfe, 1, 2, 3, 4, 5]);
    assert_eq!(local_mac([0, 0, 0, 0, 0, 0]), [2, 0, 0, 0, 0, 0]);
}

#[test]
fn initialisation_negotiates_and_fills_the_receive_queue() {
    let sim = Sim::new(QEMU_LIKE | feature::MTU, 64, 1500);
    let mut h = Harness::new(&sim, 32);
    h.vectors = (Some(0), Some(1), Some(2));
    let d = h.init();
    let want = VERSION_1 | feature::MAC | feature::STATUS | feature::MTU | feature::MRG_RXBUF | EVENT_IDX;
    assert_eq!(sim.0.driver_features.get(), want);
    assert_eq!(d.features(), want);
    use status::*;
    assert_eq!(
        *sim.0.status_writes.borrow(),
        [0, ACKNOWLEDGE, ACKNOWLEDGE | DRIVER, ACKNOWLEDGE | DRIVER | FEATURES_OK, 15]
    );
    assert_eq!(d.mac(), poly_net::MacAddr(SIM_MAC));
    assert_eq!(d.mtu(), 1500);
    assert!(d.link_up());
    // Both queues as big as the tables allow, on their vectors.
    assert_eq!((d.rx_queue_size(), d.tx_queue_size()), (32, 32));
    let (rx, tx) = (sim.queue(0), sim.queue(1));
    assert_eq!((rx.enable, rx.size, rx.msix), (1, 32, 0));
    assert_eq!((tx.enable, tx.size, tx.msix), (1, 32, 1));
    assert_eq!(sim.0.config_msix.get(), 2);
    assert_eq!(sim.queue(2).enable, 0, "no control queue");
    // Every receive buffer is with the device, which heard about them once
    // DRIVER_OK was set.
    assert_eq!(d.rx_posted(), 32);
    assert_eq!(sim.dev().rx.notifies, 1);
    assert_eq!(d.tx_in_flight(), 0);
    assert!(d.can_transmit());
}

#[test]
fn queues_shrink_to_the_device_and_the_memory() {
    let sim = Sim::new(VERSION_1, 16, 1500);
    let mut h = Harness::new(&sim, 64);
    let d = h.init();
    assert_eq!((d.rx_queue_size(), d.tx_queue_size()), (16, 16));
    drop(d);
    // Room for only 4 transmit buffers.
    let sim = Sim::new(VERSION_1, 256, 1500);
    let mut h = Harness::new(&sim, 64);
    h.tx_bufs = DmaMemory::new(4 * BUF_LEN + 100);
    let d = h.init();
    assert_eq!((d.rx_queue_size(), d.tx_queue_size()), (64, 4));
    drop(d);
    // Less than two buffers: refused, and the device is told.
    let sim = Sim::new(VERSION_1, 256, 1500);
    let mut h = Harness::new(&sim, 64);
    h.rx_bufs = DmaMemory::new(BUF_LEN);
    assert_eq!(h.try_init().err(), Some(InitError::NoRoom));
    assert_eq!(sim.0.status.get() & status::FAILED, status::FAILED);
}

#[test]
fn devices_without_version_1_or_queues_are_refused() {
    let sim = Sim::new(feature::MAC, 16, 1500);
    let mut h = Harness::new(&sim, 16);
    assert_eq!(h.try_init().err(), Some(InitError::Virtio(poly_virtio::InitError::LegacyOnly)));
    let sim = Sim::new(VERSION_1, 16, 1500);
    sim.0.queues.borrow_mut().truncate(1);
    let mut h = Harness::new(&sim, 16);
    assert_eq!(h.try_init().err(), Some(InitError::Virtio(poly_virtio::InitError::NoSuchQueue)));
    assert_ne!(sim.0.status.get() & status::FAILED, 0);
}

#[test]
fn without_the_mac_feature_a_local_address_is_made_up() {
    let sim = Sim::new(VERSION_1 | feature::STATUS, 16, 1500);
    let mut h = Harness::new(&sim, 16);
    let d = h.init();
    assert_eq!(d.mac(), poly_net::MacAddr([0xfe, 1, 2, 3, 4, 5]));
}

#[test]
fn link_status_follows_the_configuration() {
    let sim = Sim::new(VERSION_1 | feature::STATUS, 16, 1500);
    let mut h = Harness::new(&sim, 16);
    let d = h.init();
    assert!(d.link_up());
    sim.set_link(false);
    assert!(!d.link_up());
    sim.set_link(true);
    assert!(d.link_up());
    drop(d);
    // Without VIRTIO_NET_F_STATUS the link counts as up, whatever the
    // field says (§5.1.4.2).
    let sim = Sim::new(VERSION_1, 16, 1500);
    sim.set_link(false);
    let mut h = Harness::new(&sim, 16);
    assert!(h.init().link_up());
}

#[test]
fn mtu_from_the_device() {
    for (offered, mtu, negotiated, used) in [
        (VERSION_1 | feature::MTU, 1400, true, 1400),
        (VERSION_1 | feature::MTU | feature::MRG_RXBUF, 9000, true, 1500),
        (VERSION_1 | feature::MTU, 9000, false, 1500),
        (VERSION_1 | feature::MTU, 40, false, 1500),
        (VERSION_1, 1400, false, 1500),
    ] {
        let sim = Sim::new(offered, 16, mtu);
        let mut h = Harness::new(&sim, 16);
        let d = h.init();
        assert_eq!(sim.negotiated(feature::MTU), negotiated, "device MTU {mtu}");
        assert_eq!(d.mtu(), used, "device MTU {mtu}");
    }
}

#[test]
fn frames_go_out_behind_a_zero_header() {
    let sim = Sim::new(VERSION_1 | EVENT_IDX | feature::MAC, 16, 1500);
    let mut h = Harness::new(&sim, 16);
    let mut d = h.init();
    let frames: Vec<Vec<u8>> = (0..40).map(|i| frame(60 + i * 37, i as u8)).collect();
    for f in &frames {
        d.transmit(f).unwrap();
    }
    assert_eq!(sim.dev().sent, frames);
    assert!(sim.dev().sent_headers.iter().all(|h| *h == NetHeader::TRANSMIT));
    let s = d.stats();
    assert_eq!(s.tx_packets, 40);
    assert_eq!(s.tx_bytes, frames.iter().map(|f| f.len() as u64).sum::<u64>());
    // The largest frame the MTU allows goes; one byte more does not.
    d.transmit(&frame(1514, 9)).unwrap();
    assert_eq!(d.transmit(&frame(1515, 9)), Err(TxError::TooLong));
    assert_eq!(sim.dev().sent.len(), 41);
}

#[test]
fn a_full_transmit_queue_is_busy_until_the_device_catches_up() {
    let sim = Sim::new(VERSION_1, 8, 1500);
    let mut h = Harness::new(&sim, 8);
    let mut d = h.init();
    sim.dev().tx_auto = false;
    for i in 0..8 {
        assert!(d.can_transmit());
        d.transmit(&frame(100, i)).unwrap();
    }
    assert!(!d.can_transmit());
    assert_eq!(d.transmit(&frame(100, 8)), Err(TxError::Busy));
    assert_eq!(d.stats().tx_busy, 1);
    assert_eq!(d.tx_in_flight(), 8);
    // Interrupts for transmit completions only while it is full.
    assert!(!d.enable_interrupts());
    sim.take_tx();
    assert_eq!(sim.dev().tx.interrupts, 8, "every completion interrupts while the queue is full");
    assert!(d.can_transmit());
    d.transmit(&frame(100, 8)).unwrap();
    assert_eq!(d.tx_in_flight(), 1);
    assert!(!d.enable_interrupts());
    sim.take_tx();
    assert_eq!(sim.dev().tx.interrupts, 8, "no transmit interrupts with buffers to spare");
    let sent = sim.dev().sent.clone();
    assert_eq!(sent.len(), 9);
    assert_eq!(sent[8], frame(100, 8));
}

#[test]
fn received_frames_come_out_and_buffers_go_back() {
    for offered in [VERSION_1, VERSION_1 | EVENT_IDX, VERSION_1 | feature::MRG_RXBUF | EVENT_IDX] {
        let sim = Sim::new(offered, 8, 1500);
        let mut h = Harness::new(&sim, 8);
        let mut d = h.init();
        let mut buf = vec![0u8; 1514];
        assert_eq!(d.receive(&mut buf), None);
        // Many times around the 8-entry queue, past the 16-bit index wrap,
        // in bursts of up to a full queue.
        let mut n = 0u32;
        while n < 70_000 {
            let burst = 1 + n % 8;
            for i in 0..burst {
                sim.inject(&frame(60 + ((n + i) % 1455) as usize, (n + i) as u8));
            }
            for i in 0..burst {
                let len = d.receive(&mut buf).expect("a frame");
                assert_eq!(buf[..len], frame(60 + ((n + i) % 1455) as usize, (n + i) as u8)[..]);
            }
            assert_eq!(d.receive(&mut buf), None);
            assert_eq!(d.rx_posted(), 8);
            n += burst;
        }
        assert_eq!(d.stats().rx_packets, n as u64);
        assert_eq!((d.stats().rx_errors, d.stats().rx_dropped), (0, 0));
    }
}

#[test]
fn merged_buffers_make_one_frame() {
    let sim = Sim::new(VERSION_1 | feature::MRG_RXBUF, 8, 1500);
    let mut h = Harness::new(&sim, 8);
    let mut d = h.init();
    // 5000 bytes plus the header take three 2048-byte buffers.
    let big = frame(5000, 3);
    sim.inject(&big);
    let mut buf = vec![0u8; 6000];
    assert_eq!(d.receive(&mut buf), Some(5000));
    assert_eq!(buf[..5000], big[..]);
    assert_eq!(d.rx_posted(), 8);
    // Too big for the caller: dropped, and every buffer goes back.
    sim.inject(&big);
    sim.inject(&frame(100, 4));
    let mut small = vec![0u8; 1514];
    assert_eq!(d.receive(&mut small), Some(100));
    assert_eq!(small[..100], frame(100, 4)[..]);
    assert_eq!(d.stats().rx_dropped, 1);
    assert_eq!(d.rx_posted(), 8);
    // A packet that needs more buffers than the queue has waits for the
    // device, which cannot place it: nothing arrives, nothing breaks.
    sim.inject(&frame(8 * 2048, 5));
    assert_eq!(d.receive(&mut buf), None);
    assert!(!d.is_failed());
}

#[test]
fn a_merged_frame_waits_for_all_its_buffers() {
    let sim = Sim::new(VERSION_1 | feature::MRG_RXBUF, 8, 1500);
    let mut h = Harness::new(&sim, 8);
    let mut d = h.init();
    sim.dev().split_publish = true;
    let big = frame(3000, 6);
    sim.inject(&big);
    let mut buf = vec![0u8; 4000];
    assert_eq!(d.receive(&mut buf), None, "only the first buffer is back");
    assert_eq!(d.receive(&mut buf), None);
    sim.dev().split_publish = false;
    sim.deliver();
    assert_eq!(d.receive(&mut buf), Some(3000));
    assert_eq!(buf[..3000], big[..]);
    assert_eq!(d.rx_posted(), 8);
}

#[test]
fn bad_lengths_and_counts_are_dropped() {
    let sim = Sim::new(VERSION_1 | feature::MRG_RXBUF, 8, 1500);
    let mut h = Harness::new(&sim, 8);
    let mut d = h.init();
    let mut buf = vec![0u8; 1514];
    // Shorter than a header; a buffer count of 0; one beyond the queue; a
    // frame shorter than an Ethernet header.
    sim.dev().bogus_len = Some(5);
    sim.inject(&frame(64, 1));
    sim.dev().bogus_count = Some(0);
    sim.inject(&frame(64, 2));
    sim.dev().bogus_count = Some(9);
    sim.inject(&frame(64, 3));
    sim.inject(&frame(64, 4)[..10]);
    sim.inject(&frame(64, 5));
    assert_eq!(d.receive(&mut buf), Some(64));
    assert_eq!(buf[..64], frame(64, 5)[..]);
    assert_eq!(d.stats().rx_errors, 4);
    assert_eq!(d.rx_posted(), 8);
    assert!(!d.is_failed());
}

#[test]
fn a_device_that_breaks_the_queue_protocol_is_reset_for_good() {
    let sim = Sim::new(VERSION_1 | feature::STATUS, 8, 1500);
    let mut h = Harness::new(&sim, 8);
    let mut d = h.init();
    sim.dev().bogus_id = Some(77);
    sim.inject(&frame(64, 1));
    let mut buf = vec![0u8; 1514];
    assert_eq!(d.receive(&mut buf), None);
    assert!(d.is_failed());
    let writes = sim.0.status_writes.borrow().clone();
    assert_eq!(writes[writes.len() - 2..], [0, status::FAILED]);
    assert!(!d.link_up());
    assert!(!d.can_transmit());
    assert_eq!(d.transmit(&frame(64, 2)), Err(TxError::Busy));
    assert!(!d.enable_interrupts());
    sim.inject(&frame(64, 3));
    assert_eq!(d.receive(&mut buf), None);
    // The same on the transmit side.
    let sim = Sim::new(VERSION_1, 8, 1500);
    let mut h = Harness::new(&sim, 8);
    let mut d = h.init();
    sim.dev().bogus_id = Some(3000);
    d.transmit(&frame(64, 1)).unwrap();
    assert_eq!(d.transmit(&frame(64, 2)), Err(TxError::Busy));
    assert!(d.is_failed());
}

#[test]
fn receive_interrupts_can_be_suppressed() {
    for offered in [VERSION_1, VERSION_1 | EVENT_IDX] {
        let sim = Sim::new(offered, 8, 1500);
        let mut h = Harness::new(&sim, 8);
        let mut d = h.init();
        let mut buf = vec![0u8; 1514];
        d.disable_interrupts();
        sim.inject(&frame(64, 1));
        let quiet = sim.dev().rx.interrupts;
        assert!(quiet <= 1, "at most one more interrupt after disabling ({offered:#x})");
        // Re-enabling reports the frame that arrived meanwhile.
        assert!(d.enable_interrupts());
        assert_eq!(d.receive(&mut buf), Some(64));
        assert!(!d.enable_interrupts());
        let before = sim.dev().rx.interrupts;
        sim.inject(&frame(64, 2));
        assert_eq!(sim.dev().rx.interrupts, before + 1);
        assert_eq!(d.receive(&mut buf), Some(64));
    }
}

/// The other end of the link: a device whose frames the test moves by hand.
struct Pipe {
    mac: poly_net::MacAddr,
    inbox: VecDeque<Vec<u8>>,
    outbox: VecDeque<Vec<u8>>,
}

impl NetDevice for Pipe {
    fn mac(&self) -> poly_net::MacAddr {
        self.mac
    }

    fn mtu(&self) -> usize {
        1500
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        self.outbox.push_back(frame.to_vec());
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        let f = self.inbox.pop_front()?;
        buf[..f.len()].copy_from_slice(&f);
        Some(f.len())
    }
}

#[test]
fn a_tcp_ip_stack_runs_over_the_driver() {
    use core::net::{Ipv4Addr, SocketAddr};
    use poly_net::{Config, Domain, Instant, Ipv4Cidr, Protocol, Readiness, RecvFlags, SockType, Stack};

    let sim = Sim::new(QEMU_LIKE, 64, 1500);
    let mut h = Harness::new(&sim, 64);
    let mut dev = h.init();
    let (ip_a, ip_b) = (Ipv4Addr::new(10, 0, 2, 15), Ipv4Addr::new(10, 0, 2, 2));
    let cfg = |ip, secret| Config { ipv4: Some(Ipv4Cidr::new(ip, 24)), secret, ..Config::default() };
    let mut a = Stack::for_device(&dev, cfg(ip_a, 1));
    let mut peer = Pipe { mac: poly_net::MacAddr([2, 0, 0, 0, 0, 2]), inbox: VecDeque::new(), outbox: VecDeque::new() };
    let mut b = Stack::for_device(&peer, cfg(ip_b, 2));
    let mut now = 0u64;
    // One millisecond of simulated time: both stacks run, frames cross.
    let mut step = |a: &mut Stack, b: &mut Stack, dev: &mut Driver<'_>, peer: &mut Pipe| {
        a.poll(dev, Instant::from_millis(now));
        sim.take_tx();
        peer.inbox.extend(sim.dev().sent.drain(..));
        b.poll(peer, Instant::from_millis(now));
        for f in peer.outbox.drain(..) {
            sim.inject(&f);
        }
        now += 1;
    };

    // Ping: ARP first, then the echo.
    let p = a.socket(Domain::Inet, SockType::Datagram, Some(Protocol::Icmp)).unwrap();
    let mut req = vec![8u8, 0, 0, 0, 0, 0, 0, 1];
    req.extend_from_slice(b"poly-virtio-net");
    a.send_to(p, &req, Some(SocketAddr::new(ip_b.into(), 0))).unwrap();
    for _ in 0..100 {
        step(&mut a, &mut b, &mut dev, &mut peer);
    }
    let mut reply = [0u8; 64];
    let info = a.recv_from(p, &mut reply, RecvFlags::default()).unwrap();
    assert_eq!(info.from.ip(), ip_b);
    assert_eq!((reply[0], &reply[8..info.len]), (0, &b"poly-virtio-net"[..]));
    assert_eq!(a.neighbor(ip_b.into()), Some(peer.mac));

    // TCP: 300 KB from a to b, echoed back, byte for byte.
    let l = b.socket(Domain::Inet, SockType::Stream, None).unwrap();
    b.bind(l, SocketAddr::new(ip_b.into(), 7)).unwrap();
    b.listen(l, 4).unwrap();
    let c = a.socket(Domain::Inet, SockType::Stream, None).unwrap();
    assert_eq!(a.connect(c, SocketAddr::new(ip_b.into(), 7)), Err(poly_net::Errno::EINPROGRESS));
    let data: Vec<u8> = (0..300_000u32).map(|i| (i ^ (i >> 9)) as u8).collect();
    let (mut sent, mut echoed, mut back) = (0, 0, Vec::new());
    let mut server = None;
    let mut chunk = vec![0u8; 8192];
    for _ in 0..200_000 {
        if server.is_none() {
            server = b.accept(l).ok().map(|(s, _)| s);
        }
        if a.readiness(c).unwrap().contains(Readiness::WRITABLE) && sent < data.len() {
            sent += a.send(c, &data[sent..]).unwrap_or(0);
        }
        if let Some(s) = server {
            while let Ok(n @ 1..) = b.recv(s, &mut chunk) {
                let mut off = 0;
                while off < n {
                    off += b.send(s, &chunk[off..n]).unwrap();
                }
                echoed += n;
            }
        }
        while let Ok(n @ 1..) = a.recv(c, &mut chunk) {
            back.extend_from_slice(&chunk[..n]);
        }
        if back.len() == data.len() {
            break;
        }
        step(&mut a, &mut b, &mut dev, &mut peer);
    }
    assert_eq!(echoed, data.len());
    assert!(back == data, "the echo differs from what was sent");
    let s = dev.stats();
    assert!(s.rx_packets > 200 && s.tx_packets > 200);
    assert_eq!((s.rx_errors, s.rx_dropped, dev.is_failed()), (0, 0, false));
}
