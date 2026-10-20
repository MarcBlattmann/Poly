//! The neighbour cache: IP address to MAC address, for ARP (RFC 826,
//! RFC 1122 2.3.2.1-2) and IPv6 Neighbor Discovery (RFC 4861 section 7.3,
//! simplified).
//!
//! An entry is *incomplete* while a request is out (packets for it wait in a
//! short queue), *reachable* for a while after a reply, and *probing* once
//! that time has run out but it is still used: it keeps working while new
//! requests go out, and is dropped if none is answered. Requests are
//! retransmitted at a fixed interval and give up after a few tries; packets
//! that waited for a resolution that failed are handed back so the stack can
//! tell their senders (`EHOSTUNREACH`).

use crate::time::Instant;
use crate::wire::ethernet::MacAddr;
use alloc::vec::Vec;
use core::time::Duration;

/// Tunables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeighborConfig {
    /// How long a reply keeps an entry usable without asking again.
    pub reachable_time: Duration,
    /// Time between requests while resolving.
    pub retransmit_interval: Duration,
    /// Requests sent before giving up.
    pub max_probes: u8,
    /// Packets kept per unresolved address; later ones are dropped.
    pub max_pending: usize,
    /// Entries in the cache.
    pub max_entries: usize,
}

impl Default for NeighborConfig {
    fn default() -> Self {
        NeighborConfig {
            reachable_time: Duration::from_secs(60),
            retransmit_interval: Duration::from_secs(1),
            max_probes: 3,
            max_pending: 4,
            max_entries: 128,
        }
    }
}

/// A packet waiting for its next hop's address.
#[derive(Debug)]
pub(crate) struct Pending {
    pub ethertype: u16,
    pub packet: Vec<u8>,
}

#[derive(Debug)]
enum State {
    Incomplete,
    Reachable { until: Instant },
    Probing,
}

#[derive(Debug)]
struct Entry<A> {
    addr: A,
    mac: MacAddr,
    state: State,
    probes: u8,
    next_probe: Instant,
    last_used: Instant,
    pending: Vec<Pending>,
}

/// What a lookup found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lookup {
    /// The address to send to, if known.
    pub mac: Option<MacAddr>,
    /// Send a request for the address now.
    pub request: bool,
}

#[derive(Debug)]
pub(crate) struct NeighborCache<A> {
    entries: Vec<Entry<A>>,
    cfg: NeighborConfig,
}

impl<A: Copy + Eq> NeighborCache<A> {
    pub fn new(cfg: NeighborConfig) -> Self {
        NeighborCache { entries: Vec::new(), cfg }
    }

    fn find(&mut self, addr: A) -> Option<&mut Entry<A>> {
        self.entries.iter_mut().find(|e| e.addr == addr)
    }

    /// Look up `addr` for sending. An entry past its reachable time starts
    /// probing (and asks for a request) but keeps its address meanwhile.
    pub fn lookup(&mut self, addr: A, now: Instant) -> Lookup {
        let interval = self.cfg.retransmit_interval;
        match self.find(addr) {
            None => Lookup { mac: None, request: false },
            Some(e) => {
                e.last_used = now;
                match e.state {
                    State::Incomplete => Lookup { mac: None, request: false },
                    State::Reachable { until } if now < until => Lookup { mac: Some(e.mac), request: false },
                    State::Reachable { .. } => {
                        e.state = State::Probing;
                        e.probes = 1;
                        e.next_probe = now + interval;
                        Lookup { mac: Some(e.mac), request: true }
                    }
                    State::Probing => Lookup { mac: Some(e.mac), request: false },
                }
            }
        }
    }

    /// True if packets are already waiting for `addr`.
    pub fn has_pending(&self, addr: A) -> bool {
        self.entries.iter().any(|e| e.addr == addr && !e.pending.is_empty())
    }

    /// Queue a packet for an unresolved `addr`, creating the entry. Returns
    /// true if a request should go out now (the first packet). A full
    /// queue or cache drops the packet.
    pub fn enqueue(&mut self, addr: A, p: Pending, now: Instant) -> bool {
        let (max_pending, interval) = (self.cfg.max_pending, self.cfg.retransmit_interval);
        if let Some(e) = self.find(addr) {
            if e.pending.len() < max_pending {
                e.pending.push(p);
            }
            return false;
        }
        if !self.make_room() {
            return false;
        }
        let mut pending = Vec::new();
        if max_pending > 0 {
            pending.push(p);
        }
        self.entries.push(Entry {
            addr,
            mac: MacAddr::ZERO,
            state: State::Incomplete,
            probes: 1,
            next_probe: now + interval,
            last_used: now,
            pending,
        });
        true
    }

    /// Free a slot if the cache is full by dropping the least recently used
    /// entry that has nothing waiting. False if every entry has packets
    /// waiting.
    fn make_room(&mut self) -> bool {
        if self.entries.len() < self.cfg.max_entries {
            return true;
        }
        let victim = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.pending.is_empty())
            .min_by_key(|(_, e)| e.last_used)
            .map(|(i, _)| i);
        match victim {
            Some(i) => {
                self.entries.swap_remove(i);
                true
            }
            None => false,
        }
    }

    /// Record that `addr` is at `mac`. Existing entries are updated (the
    /// RFC 826 merge rule); a new one is made only if `create`. Returns the
    /// packets that were waiting, to be sent now.
    pub fn update(&mut self, addr: A, mac: MacAddr, now: Instant, create: bool) -> Vec<Pending> {
        let until = now + self.cfg.reachable_time;
        if let Some(e) = self.find(addr) {
            e.mac = mac;
            e.state = State::Reachable { until };
            e.probes = 0;
            return core::mem::take(&mut e.pending);
        }
        if create && self.make_room() {
            self.entries.push(Entry {
                addr,
                mac,
                state: State::Reachable { until },
                probes: 0,
                next_probe: now,
                last_used: now,
                pending: Vec::new(),
            });
        }
        Vec::new()
    }

    /// Run timers: `request` is called for each address to ask for again,
    /// `fail` for each resolution that gave up, with its waiting packets.
    /// Entries unused for a whole reachable time after expiring are dropped.
    pub fn poll(&mut self, now: Instant, mut request: impl FnMut(A), mut fail: impl FnMut(A, Vec<Pending>)) {
        let cfg = self.cfg;
        let mut i = 0;
        while i < self.entries.len() {
            let e = &mut self.entries[i];
            let remove = match e.state {
                State::Reachable { until } => now >= until + cfg.reachable_time,
                State::Incomplete | State::Probing if now >= e.next_probe => {
                    if e.probes < cfg.max_probes {
                        e.probes += 1;
                        e.next_probe = now + cfg.retransmit_interval;
                        request(e.addr);
                        false
                    } else {
                        true
                    }
                }
                _ => false,
            };
            if remove {
                let e = self.entries.swap_remove(i);
                if !e.pending.is_empty() || matches!(e.state, State::Incomplete) {
                    fail(e.addr, e.pending);
                }
            } else {
                i += 1;
            }
        }
    }

    /// When [`poll`](Self::poll) next has work.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .iter()
            .map(|e| match e.state {
                State::Reachable { until } => until + self.cfg.reachable_time,
                State::Incomplete | State::Probing => e.next_probe,
            })
            .min()
    }

    /// Number of entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The address `addr` resolves to right now, without side effects.
    pub fn peek(&self, addr: A) -> Option<MacAddr> {
        self.entries.iter().find(|e| e.addr == addr && !matches!(e.state, State::Incomplete)).map(|e| e.mac)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const M: MacAddr = MacAddr([2, 0, 0, 0, 0, 7]);

    fn t(ms: u64) -> Instant {
        Instant::from_millis(ms)
    }

    fn pkt(n: u8) -> Pending {
        Pending { ethertype: 0x800, packet: vec![n] }
    }

    #[test]
    fn resolve_releases_pending_in_order() {
        let mut c = NeighborCache::<u32>::new(NeighborConfig::default());
        assert_eq!(c.lookup(1, t(0)), Lookup { mac: None, request: false });
        assert!(c.enqueue(1, pkt(1), t(0)));
        assert!(!c.enqueue(1, pkt(2), t(0)));
        assert!(c.has_pending(1));
        let out = c.update(1, M, t(5), false);
        assert_eq!(out.iter().map(|p| p.packet[0]).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(c.lookup(1, t(6)).mac, Some(M));
    }

    #[test]
    fn pending_queue_is_bounded() {
        let cfg = NeighborConfig { max_pending: 2, ..Default::default() };
        let mut c = NeighborCache::<u32>::new(cfg);
        for i in 0..5 {
            c.enqueue(1, pkt(i), t(0));
        }
        assert_eq!(c.update(1, M, t(1), false).len(), 2);
    }

    #[test]
    fn retries_then_fails() {
        let mut c = NeighborCache::<u32>::new(NeighborConfig::default());
        c.enqueue(9, pkt(1), t(0));
        let mut requests = 0;
        let mut failed = Vec::new();
        let mut now = 0;
        while let Some(d) = c.next_deadline() {
            now = d.millis();
            c.poll(d, |_| requests += 1, |a, p| failed.push((a, p.len())));
        }
        assert_eq!(requests, 2, "the first request is sent by the caller");
        assert_eq!(failed, vec![(9, 1)]);
        assert_eq!(now, 3000);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn expiry_probes_and_keeps_working() {
        let cfg = NeighborConfig::default();
        let mut c = NeighborCache::<u32>::new(cfg);
        c.update(1, M, t(0), true);
        assert_eq!(c.lookup(1, t(59_999)), Lookup { mac: Some(M), request: false });
        assert_eq!(c.lookup(1, t(60_000)), Lookup { mac: Some(M), request: true });
        // A reply confirms it again.
        c.update(1, M, t(60_010), false);
        assert_eq!(c.lookup(1, t(60_020)), Lookup { mac: Some(M), request: false });
        // Unanswered probes drop the entry.
        let _ = c.lookup(1, t(200_000));
        let mut now = t(200_000);
        while c.len() > 0 {
            now = c.next_deadline().unwrap();
            c.poll(now, |_| {}, |_, _| panic!("no packets waited"));
        }
        assert_eq!(now, t(203_000));
    }

    #[test]
    fn unused_entries_age_out() {
        let mut c = NeighborCache::<u32>::new(NeighborConfig::default());
        c.update(1, M, t(0), true);
        assert_eq!(c.next_deadline(), Some(t(120_000)));
        c.poll(t(120_000), |_| {}, |_, _| {});
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn update_without_create_is_ignored() {
        let mut c = NeighborCache::<u32>::new(NeighborConfig::default());
        c.update(1, M, t(0), false);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn full_cache_evicts_idle_entries_only() {
        let cfg = NeighborConfig { max_entries: 2, ..Default::default() };
        let mut c = NeighborCache::<u32>::new(cfg);
        c.enqueue(1, pkt(1), t(0));
        c.enqueue(2, pkt(2), t(0));
        assert!(!c.enqueue(3, pkt(3), t(0)), "both entries have packets waiting");
        c.update(1, M, t(1), false);
        assert!(c.enqueue(3, pkt(3), t(2)));
        assert_eq!(c.peek(1), None);
        assert!(c.has_pending(2) && c.has_pending(3));
    }
}
