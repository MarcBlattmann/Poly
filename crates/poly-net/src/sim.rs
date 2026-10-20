//! A deterministic simulated network for the tests: two stacks joined by a
//! link that can lose, duplicate, corrupt, delay and reorder frames, all
//! driven by a seeded generator and a simulated clock. Time jumps straight
//! to the next deadline, so minutes of protocol time cost nothing.

extern crate std;

use crate::device::{NetDevice, TxError};
use crate::time::Instant;
use crate::wire::ethernet::MacAddr;
use crate::{Config, Errno, Ipv4Cidr, Shutdown, SockType, SocketHandle, Stack};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddr};
use core::time::Duration;

/// xorshift64*: small, deterministic, good enough for impairments.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    /// True with probability `per_mille` / 1000.
    pub fn chance(&mut self, per_mille: u32) -> bool {
        self.below(1000) < u64::from(per_mille)
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// A device that queues what it sends and receives what the link delivers.
#[derive(Debug)]
pub struct SimDev {
    pub mac: MacAddr,
    pub mtu: usize,
    pub inbox: VecDeque<Vec<u8>>,
    pub outbox: Vec<Vec<u8>>,
    /// Frames the device holds for sending before saying Busy.
    pub tx_capacity: usize,
}

impl SimDev {
    pub fn new(mac: MacAddr, mtu: usize) -> SimDev {
        SimDev { mac, mtu, inbox: VecDeque::new(), outbox: Vec::new(), tx_capacity: usize::MAX }
    }
}

impl NetDevice for SimDev {
    fn mac(&self) -> MacAddr {
        self.mac
    }

    fn mtu(&self) -> usize {
        self.mtu
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), TxError> {
        assert!(frame.len() >= 60, "frames are padded to the Ethernet minimum");
        if frame.len() > self.mtu + 14 {
            return Err(TxError::TooLong);
        }
        if self.outbox.len() >= self.tx_capacity {
            return Err(TxError::Busy);
        }
        self.outbox.push(frame.to_vec());
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Option<usize> {
        let f = self.inbox.pop_front()?;
        let n = f.len().min(buf.len());
        buf[..n].copy_from_slice(&f[..n]);
        Some(n)
    }

    fn can_transmit(&self) -> bool {
        self.outbox.len() < self.tx_capacity
    }
}

/// Link impairments, each direction alike.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkConfig {
    /// Frames lost, per mille.
    pub loss: u32,
    /// Frames delivered twice, per mille.
    pub duplicate: u32,
    /// Frames with one byte flipped, per mille.
    pub corrupt: u32,
    /// Fixed one-way delay.
    pub delay: Duration,
    /// Extra random delay up to this much; reorders frames.
    pub jitter: Duration,
}

/// Which way a frame travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    AtoB,
    BtoA,
}

/// Decides a frame's fate before impairments: return false to drop it.
pub type Filter = Box<dyn FnMut(Dir, &mut Vec<u8>) -> bool>;

pub struct Sim {
    pub now: Instant,
    pub a: Stack,
    pub b: Stack,
    pub da: SimDev,
    pub db: SimDev,
    pub link: LinkConfig,
    pub rng: Rng,
    /// (deliver at, sequence) -> (direction, frame).
    flight: BTreeMap<(Instant, u64), (Dir, Vec<u8>)>,
    seq: u64,
    pub filter: Option<Filter>,
    /// Frames put on the wire, per direction.
    pub frames: [u64; 2],
    /// Disconnect the link entirely (everything is lost).
    pub cut: bool,
    /// When set, every frame put on the wire is recorded here.
    pub log: Option<Vec<(Dir, Vec<u8>)>>,
}

pub const MAC_A: MacAddr = MacAddr([2, 0, 0, 0, 0, 0xa]);
pub const MAC_B: MacAddr = MacAddr([2, 0, 0, 0, 0, 0xb]);
pub const IP_A: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
pub const IP_B: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

/// The configuration tests start from: a short minimum RTO so loss is
/// recovered quickly in simulated time.
pub fn config(ip: Ipv4Addr, seed: u64) -> Config {
    let mut c = Config { ipv4: Some(Ipv4Cidr::new(ip, 24)), secret: seed, ..Config::default() };
    c.tcp.min_rto = Duration::from_millis(200);
    c
}

impl Sim {
    pub fn new(seed: u64) -> Sim {
        Sim::with_configs(seed, config(IP_A, seed ^ 0xa), config(IP_B, seed ^ 0xb), 1500)
    }

    pub fn with_configs(seed: u64, ca: Config, cb: Config, mtu: usize) -> Sim {
        let da = SimDev::new(MAC_A, mtu);
        let db = SimDev::new(MAC_B, mtu);
        Sim {
            now: Instant::from_millis(1_000),
            a: Stack::for_device(&da, ca),
            b: Stack::for_device(&db, cb),
            da,
            db,
            link: LinkConfig { delay: Duration::from_millis(5), ..Default::default() },
            rng: Rng::new(seed),
            flight: BTreeMap::new(),
            seq: 0,
            filter: None,
            frames: [0; 2],
            cut: false,
            log: None,
        }
    }

    fn launch(&mut self, dir: Dir, mut frame: Vec<u8>) {
        self.frames[dir as usize] += 1;
        if std::env::var_os("POLY_NET_TRACE").is_some() {
            std::eprintln!("{:?} {:?} {}", self.now, dir, describe(&frame));
        }
        if let Some(log) = self.log.as_mut() {
            log.push((dir, frame.clone()));
        }
        if let Some(f) = self.filter.as_mut()
            && !f(dir, &mut frame)
        {
            return;
        }
        if self.cut || self.rng.chance(self.link.loss) {
            return;
        }
        let copies = if self.rng.chance(self.link.duplicate) { 2 } else { 1 };
        for _ in 0..copies {
            let mut f = frame.clone();
            if self.rng.chance(self.link.corrupt) {
                let i = self.rng.below(f.len() as u64) as usize;
                f[i] ^= 1 << self.rng.below(8);
            }
            let jitter = self.rng.below(self.link.jitter.as_millis() as u64 + 1);
            let at = self.now + self.link.delay + Duration::from_millis(jitter);
            self.seq += 1;
            self.flight.insert((at, self.seq), (dir, f));
        }
    }

    /// The next moment anything happens: a stack deadline, a frame
    /// arriving, or now if a frame waits in an inbox.
    pub fn next_event(&self) -> Option<Instant> {
        let waiting = (!self.da.inbox.is_empty() || !self.db.inbox.is_empty()).then_some(self.now);
        [self.a.poll_at(), self.b.poll_at(), self.flight.keys().next().map(|k| k.0), waiting]
            .into_iter()
            .flatten()
            .min()
    }

    /// One round: move the clock to the next event (but not past `cap`),
    /// deliver the frames due, poll both stacks and put what they send on
    /// the link. Returns false if nothing is scheduled before `cap`.
    pub fn step_until(&mut self, cap: Instant) -> bool {
        let target = match self.next_event() {
            Some(t) => t.min(cap),
            None if cap > self.now && cap != Instant::from_millis(u64::MAX) => cap,
            None => return false,
        };
        if target > self.now {
            self.now = target;
        }
        while let Some(entry) = self.flight.first_entry() {
            if entry.key().0 > self.now {
                break;
            }
            let (dir, f) = entry.remove();
            match dir {
                Dir::AtoB => self.db.inbox.push_back(f),
                Dir::BtoA => self.da.inbox.push_back(f),
            }
        }
        self.a.poll(&mut self.da, self.now);
        self.b.poll(&mut self.db, self.now);
        for f in core::mem::take(&mut self.da.outbox) {
            self.launch(Dir::AtoB, f);
        }
        for f in core::mem::take(&mut self.db.outbox) {
            self.launch(Dir::BtoA, f);
        }
        true
    }

    /// One round with no cap on the clock.
    pub fn step(&mut self) -> bool {
        self.step_until(Instant::from_millis(u64::MAX))
    }

    /// Step until `cond` holds (checked after every step) or `limit` of
    /// simulated time has passed. Returns whether `cond` held.
    pub fn run_until(&mut self, limit: Duration, mut cond: impl FnMut(&mut Sim) -> bool) -> bool {
        let end = self.now + limit;
        let mut same_time = 0u32;
        let mut last = self.now;
        loop {
            if cond(self) {
                return true;
            }
            if self.now >= end || !self.step_until(end) {
                return cond(self);
            }
            if self.now == last {
                same_time += 1;
                assert!(same_time < 100_000, "livelock: no progress in simulated time at {:?}", self.now);
            } else {
                same_time = 0;
                last = self.now;
            }
        }
    }

    /// Let exactly `d` of simulated time pass.
    pub fn run_for(&mut self, d: Duration) {
        self.run_until(d, |_| false);
    }

    /// Open a TCP connection from A to B's `port`: (client on A, listener
    /// on B, accepted socket on B).
    pub fn connect(&mut self, port: u16) -> (SocketHandle, SocketHandle, SocketHandle) {
        let l = self.b.socket(crate::Domain::Inet, SockType::Stream, None).unwrap();
        self.b.bind(l, SocketAddr::new(IP_B.into(), port)).unwrap();
        self.b.listen(l, 8).unwrap();
        let c = self.a.socket(crate::Domain::Inet, SockType::Stream, None).unwrap();
        assert_eq!(self.a.connect(c, SocketAddr::new(IP_B.into(), port)), Err(Errno::EINPROGRESS));
        let mut accepted = None;
        let ok = self.run_until(Duration::from_secs(120), |s| {
            if accepted.is_none() {
                accepted = s.b.accept(l).ok().map(|(h, _)| h);
            }
            accepted.is_some() && s.a.tcp_state(c) == Ok(crate::TcpState::Established)
        });
        assert!(ok, "connection did not open: client {:?}", self.a.tcp_state(c));
        (c, l, accepted.unwrap())
    }

    /// Send `data` from `src` on A to `dst` on B (or B to A with
    /// `a_to_b` false), close the sending side, and return what the
    /// receiver read up to end of stream.
    pub fn transfer(
        &mut self,
        a_to_b: bool,
        src: SocketHandle,
        dst: SocketHandle,
        data: &[u8],
        limit: Duration,
    ) -> Vec<u8> {
        let mut sent = 0;
        let mut got = Vec::with_capacity(data.len());
        let mut shut = false;
        let mut buf = vec![0u8; 16 * 1024];
        let done = self.run_until(limit, |s| {
            let (tx, rx) = if a_to_b { (&mut s.a, &mut s.b) } else { (&mut s.b, &mut s.a) };
            while sent < data.len() {
                match tx.send(src, &data[sent..]) {
                    Ok(n) => sent += n,
                    Err(Errno::EAGAIN) => break,
                    Err(e) => panic!("send failed: {e:?}"),
                }
            }
            if sent == data.len() && !shut {
                tx.shutdown(src, Shutdown::Write).unwrap();
                shut = true;
            }
            loop {
                match rx.recv(dst, &mut buf) {
                    Ok(0) => return true,
                    Ok(n) => got.extend_from_slice(&buf[..n]),
                    Err(Errno::EAGAIN) => return false,
                    Err(e) => panic!("recv failed: {e:?}"),
                }
            }
        });
        assert!(done, "transfer incomplete: {} of {} bytes sent, {} received", sent, data.len(), got.len());
        got
    }

    /// Like [`transfer`](Self::transfer) between two sockets of stack A.
    pub fn transfer_local(&mut self, src: SocketHandle, dst: SocketHandle, data: &[u8]) -> Vec<u8> {
        let mut sent = 0;
        let mut got = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        let done = self.run_until(Duration::from_secs(30), |s| {
            while sent < data.len() {
                match s.a.send(src, &data[sent..]) {
                    Ok(n) => sent += n,
                    Err(_) => break,
                }
            }
            while let Ok(n) = s.a.recv(dst, &mut buf) {
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            got.len() == data.len()
        });
        assert!(done, "local transfer incomplete: {} of {}", got.len(), data.len());
        got
    }
}

/// One line about a frame, for tracing.
pub fn describe(frame: &[u8]) -> std::string::String {
    use crate::wire::{arp, ethernet, ipv4, proto, tcp};
    use std::format;
    let Ok(f) = ethernet::parse(frame) else { return "bad ethernet".into() };
    if f.ethertype == ethernet::ETHERTYPE_ARP {
        return match arp::ArpPacket::parse(f.payload) {
            Ok(p) => format!("ARP op {} {} -> {}", p.op, p.sender_ip, p.target_ip),
            Err(_) => "bad ARP".into(),
        };
    }
    let Ok((h, p)) = ipv4::parse(f.payload) else { return format!("ethertype {:#x}", f.ethertype) };
    if h.protocol != proto::TCP {
        return format!("IP proto {} {} -> {} len {}", h.protocol, h.src, h.dst, p.len());
    }
    let Ok((t, d)) = tcp::parse(p) else { return "bad TCP".into() };
    let mut fl = std::string::String::new();
    for (bit, c) in [(tcp::SYN, 'S'), (tcp::ACK, 'A'), (tcp::FIN, 'F'), (tcp::RST, 'R'), (tcp::PSH, 'P')] {
        if t.flags & bit != 0 {
            fl.push(c);
        }
    }
    format!("TCP {}->{} [{}] seq {} ack {} win {} len {}", t.src_port, t.dst_port, fl, t.seq, t.ack, t.window, d.len())
}
