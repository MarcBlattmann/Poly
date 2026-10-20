//! IPv4 fragment reassembly (RFC 791 section 3.2, RFC 1122 3.3.2).
//!
//! Fragments are keyed by (source, destination, protocol, identification).
//! Bounded on every axis: datagrams in progress, bytes buffered in total,
//! fragments per datagram, and time (RFC 1122 asks for a timeout of 60 to
//! 120 seconds as a fixed value or a TTL-based one; the default here is 30
//! seconds, configurable).
//!
//! Overlaps: a fragment that overlaps data already received is accepted only
//! if the overlapping bytes are identical (a plain duplicate). Anything else
//! poisons the datagram: it is dropped, and so are its later fragments until
//! the timeout, following the reasoning of RFC 1858 and RFC 5722 (which
//! makes the same rule mandatory for IPv6).

use crate::time::Instant;
use crate::wire::ipv4::{Ipv4Header, MAX_PACKET_LEN, MIN_HEADER_LEN};
use alloc::vec::Vec;
use core::net::Ipv4Addr;
use core::time::Duration;

/// Tunables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReassemblyConfig {
    /// Datagrams reassembled at once; fragments of further ones are dropped.
    pub max_datagrams: usize,
    /// Bytes buffered across all datagrams.
    pub max_bytes: usize,
    /// Distinct pieces kept per datagram.
    pub max_fragments: usize,
    /// Time from the first fragment until an incomplete datagram is dropped.
    pub timeout: Duration,
}

impl Default for ReassemblyConfig {
    fn default() -> Self {
        ReassemblyConfig {
            max_datagrams: 16,
            max_bytes: 256 * 1024,
            max_fragments: 64,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    src: Ipv4Addr,
    dst: Ipv4Addr,
    protocol: u8,
    ident: u16,
}

#[derive(Debug)]
struct Datagram {
    key: Key,
    deadline: Instant,
    data: Vec<u8>,
    /// Received byte ranges, sorted, non-overlapping, merged when adjacent.
    ranges: Vec<(usize, usize)>,
    /// Payload length, once the last fragment (MF clear) has arrived.
    total: Option<usize>,
    /// Header and first 8 data bytes of the first fragment, for ICMP Time
    /// Exceeded (RFC 792).
    first: Option<(Ipv4Header, [u8; 8], usize)>,
    poisoned: bool,
}

/// A datagram whose reassembly timed out after its first fragment arrived:
/// the caller sends ICMP Time Exceeded (code 1) quoting this.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Expired {
    pub header: Ipv4Header,
    pub data: [u8; 8],
    pub data_len: usize,
}

/// Counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReassemblyStats {
    pub completed: u64,
    pub timed_out: u64,
    pub overlaps: u64,
    pub dropped: u64,
}

#[derive(Debug)]
pub(crate) struct Reassembler {
    cfg: ReassemblyConfig,
    slots: Vec<Datagram>,
    bytes: usize,
    pub stats: ReassemblyStats,
}

impl Reassembler {
    pub fn new(cfg: ReassemblyConfig) -> Self {
        Reassembler { cfg, slots: Vec::new(), bytes: 0, stats: ReassemblyStats::default() }
    }

    /// Add a fragment. Returns the reassembled datagram's header (fragment
    /// fields cleared, lengths fixed) and payload when it is complete.
    pub fn insert(&mut self, now: Instant, h: &Ipv4Header, payload: &[u8]) -> Option<(Ipv4Header, Vec<u8>)> {
        let key = Key { src: h.src, dst: h.dst, protocol: h.protocol, ident: h.ident };
        let start = h.frag_offset;
        let end = start + payload.len();
        // Non-final fragments carry a multiple of 8 bytes (the offset unit);
        // nothing may end past the largest datagram.
        let bad = (h.more_fragments && (payload.is_empty() || !payload.len().is_multiple_of(8)))
            || end + MIN_HEADER_LEN > MAX_PACKET_LEN;
        let idx = match self.slots.iter().position(|d| d.key == key) {
            Some(i) => i,
            None => {
                if bad || self.slots.len() >= self.cfg.max_datagrams {
                    self.stats.dropped += 1;
                    return None;
                }
                self.slots.push(Datagram {
                    key,
                    deadline: now + self.cfg.timeout,
                    data: Vec::new(),
                    ranges: Vec::new(),
                    total: None,
                    first: None,
                    poisoned: false,
                });
                self.slots.len() - 1
            }
        };
        if bad {
            self.poison(idx);
            return None;
        }
        let d = &mut self.slots[idx];
        if d.poisoned {
            self.stats.dropped += 1;
            return None;
        }
        // A second "last" fragment that disagrees, or data past the end.
        let conflict = match d.total {
            Some(t) => end > t || (!h.more_fragments && end != t),
            None => !h.more_fragments && d.ranges.last().is_some_and(|r| r.1 > end),
        };
        if conflict {
            self.poison(idx);
            return None;
        }
        // Overlap check: overlapping bytes must match what is there.
        let d = &mut self.slots[idx];
        for &(a, b) in &d.ranges {
            let (lo, hi) = (a.max(start), b.min(end));
            if lo < hi && d.data[lo..hi] != payload[lo - start..hi - start] {
                self.stats.overlaps += 1;
                self.poison(idx);
                return None;
            }
        }
        let d = &mut self.slots[idx];
        let grow = end.saturating_sub(d.data.len());
        if self.bytes + grow > self.cfg.max_bytes {
            self.stats.dropped += 1;
            return None;
        }
        // Merge the new range; refuse if it would exceed the piece limit.
        let mut ranges = d.ranges.clone();
        ranges.push((start, end));
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match merged.last_mut() {
                Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
                _ => merged.push(r),
            }
        }
        if merged.len() > self.cfg.max_fragments {
            self.stats.dropped += 1;
            return None;
        }
        if grow > 0 {
            d.data.resize(end, 0);
            self.bytes += grow;
        }
        d.data[start..end].copy_from_slice(payload);
        d.ranges = merged;
        if !h.more_fragments {
            d.total = Some(end);
        }
        if start == 0 {
            let mut first8 = [0u8; 8];
            let n = payload.len().min(8);
            first8[..n].copy_from_slice(&payload[..n]);
            d.first = Some((*h, first8, n));
        }
        let complete = matches!((d.total, d.ranges.as_slice()), (Some(t), [(0, e)]) if *e == t);
        if !complete {
            return None;
        }
        let d = self.slots.swap_remove(idx);
        self.bytes -= d.data.len();
        self.stats.completed += 1;
        let (mut header, _, _) = d.first?;
        header.more_fragments = false;
        header.frag_offset = 0;
        header.header_len = MIN_HEADER_LEN;
        header.total_len = MIN_HEADER_LEN + d.data.len();
        Some((header, d.data))
    }

    fn poison(&mut self, idx: usize) {
        let d = &mut self.slots[idx];
        self.bytes -= d.data.len();
        d.data = Vec::new();
        d.ranges = Vec::new();
        d.first = None;
        d.poisoned = true;
        self.stats.dropped += 1;
    }

    /// Drop datagrams whose time is up; `expired` gets those whose first
    /// fragment had arrived.
    pub fn poll(&mut self, now: Instant, mut expired: impl FnMut(Expired)) {
        let mut i = 0;
        while i < self.slots.len() {
            if now >= self.slots[i].deadline {
                let d = self.slots.swap_remove(i);
                self.bytes -= d.data.len();
                if !d.poisoned {
                    self.stats.timed_out += 1;
                    if let Some((header, data, data_len)) = d.first {
                        expired(Expired { header, data, data_len });
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.slots.iter().map(|d| d.deadline).min()
    }

    /// Datagrams in progress.
    #[cfg(test)]
    pub fn in_progress(&self) -> usize {
        self.slots.len()
    }

    /// Bytes buffered.
    #[cfg(test)]
    pub fn buffered(&self) -> usize {
        self.bytes
    }
}

/// Build the payload of a datagram split at `offsets` (helper for tests in
/// other modules): returns `(offset, more_fragments, bytes)` triples.
#[cfg(test)]
pub(crate) fn split(payload: &[u8], piece: usize) -> Vec<(usize, bool, Vec<u8>)> {
    let mut out = Vec::new();
    let mut off = 0;
    while off < payload.len() {
        let end = (off + piece).min(payload.len());
        out.push((off, end < payload.len(), payload[off..end].to_vec()));
        off = end;
    }
    if out.is_empty() {
        out.push((0, false, Vec::new()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(off: usize, mf: bool, len: usize, ident: u16) -> Ipv4Header {
        let mut h = Ipv4Header::new(Ipv4Addr::new(10, 0, 0, 2), Ipv4Addr::new(10, 0, 0, 1), 17, 64, len);
        h.ident = ident;
        h.frag_offset = off;
        h.more_fragments = mf;
        h
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 + 3) as u8).collect()
    }

    fn t(ms: u64) -> Instant {
        Instant::from_millis(ms)
    }

    #[test]
    fn in_order_and_out_of_order() {
        let payload = data(3000);
        for order in [[0usize, 1, 2], [2, 1, 0], [1, 2, 0], [2, 0, 1]] {
            let mut r = Reassembler::new(ReassemblyConfig::default());
            let pieces = split(&payload, 1480);
            let mut done = None;
            for &i in &order {
                let (off, mf, ref b) = pieces[i];
                done = r.insert(t(0), &hdr(off, mf, b.len(), 7), b);
            }
            let (h, d) = done.expect("complete");
            assert_eq!(d, payload);
            assert_eq!(h.total_len, 3020);
            assert!(!h.is_fragment());
            assert_eq!(r.in_progress(), 0);
            assert_eq!(r.buffered(), 0);
        }
    }

    #[test]
    fn duplicates_and_identical_overlaps_are_fine() {
        let payload = data(64);
        let mut r = Reassembler::new(ReassemblyConfig::default());
        assert!(r.insert(t(0), &hdr(0, true, 32, 1), &payload[..32]).is_none());
        assert!(r.insert(t(0), &hdr(0, true, 32, 1), &payload[..32]).is_none());
        assert!(r.insert(t(0), &hdr(16, true, 32, 1), &payload[16..48]).is_none());
        let (_, d) = r.insert(t(0), &hdr(48, false, 16, 1), &payload[48..]).unwrap();
        assert_eq!(d, payload);
    }

    #[test]
    fn conflicting_overlap_poisons_until_timeout() {
        let payload = data(64);
        let mut r = Reassembler::new(ReassemblyConfig::default());
        r.insert(t(0), &hdr(0, true, 32, 1), &payload[..32]);
        let mut evil = payload[16..48].to_vec();
        evil[0] ^= 0xff;
        assert!(r.insert(t(0), &hdr(16, true, 32, 1), &evil).is_none());
        assert_eq!(r.stats.overlaps, 1);
        // The rest does not complete it.
        assert!(r.insert(t(1), &hdr(32, false, 32, 1), &payload[32..]).is_none());
        assert!(r.insert(t(1), &hdr(0, true, 32, 1), &payload[..32]).is_none());
        assert_eq!(r.buffered(), 0);
        let mut n = 0;
        r.poll(t(30_000), |_| n += 1);
        assert_eq!(n, 0, "no time exceeded for a poisoned datagram");
        assert_eq!(r.in_progress(), 0);
        // After the timeout the same id works again.
        r.insert(t(30_001), &hdr(0, true, 32, 1), &payload[..32]);
        assert!(r.insert(t(30_001), &hdr(32, false, 32, 1), &payload[32..]).is_some());
    }

    #[test]
    fn conflicting_ends_poison() {
        let payload = data(64);
        let mut r = Reassembler::new(ReassemblyConfig::default());
        r.insert(t(0), &hdr(32, false, 32, 1), &payload[32..]);
        assert!(r.insert(t(0), &hdr(40, false, 8, 1), &payload[40..48]).is_none());
        assert!(r.insert(t(0), &hdr(0, true, 32, 1), &payload[..32]).is_none());
        // Data past a known end.
        let mut r = Reassembler::new(ReassemblyConfig::default());
        r.insert(t(0), &hdr(0, false, 16, 2), &payload[..16]);
        let _ = r.insert(t(0), &hdr(0, true, 8, 3), &payload[..8]);
        r.insert(t(0), &hdr(24, true, 8, 3), &payload[..8]);
        assert!(r.insert(t(0), &hdr(8, false, 8, 3), &payload[..8]).is_none(), "ends before data already seen");
    }

    #[test]
    fn timeout_reports_first_fragment() {
        let payload = data(64);
        let mut r = Reassembler::new(ReassemblyConfig::default());
        r.insert(t(0), &hdr(0, true, 32, 1), &payload[..32]);
        r.insert(t(0), &hdr(32, true, 8, 9), &payload[32..40]);
        assert_eq!(r.next_deadline(), Some(t(30_000)));
        let mut got = Vec::new();
        r.poll(t(29_999), |e| got.push(e));
        assert!(got.is_empty());
        r.poll(t(30_000), |e| got.push(e));
        assert_eq!(got.len(), 1, "only the datagram whose first fragment arrived");
        assert_eq!(got[0].header.ident, 1);
        assert_eq!(&got[0].data, &payload[..8]);
        assert_eq!(r.stats.timed_out, 2);
        assert_eq!(r.buffered(), 0);
    }

    #[test]
    fn limits() {
        let cfg = ReassemblyConfig { max_datagrams: 2, max_bytes: 100, max_fragments: 2, ..Default::default() };
        let mut r = Reassembler::new(cfg);
        let p = data(200);
        r.insert(t(0), &hdr(0, true, 8, 1), &p[..8]);
        r.insert(t(0), &hdr(0, true, 8, 2), &p[..8]);
        r.insert(t(0), &hdr(0, true, 8, 3), &p[..8]);
        assert_eq!(r.in_progress(), 2);
        // Byte budget.
        assert!(r.insert(t(0), &hdr(96, true, 8, 1), &p[..8]).is_none());
        assert!(r.buffered() <= 100);
        // Piece limit: 0..8 and 16..24 are two pieces; a third is refused.
        r.insert(t(0), &hdr(16, true, 8, 2), &p[16..24]);
        let dropped = r.stats.dropped;
        assert!(r.insert(t(0), &hdr(32, true, 8, 2), &p[32..40]).is_none());
        assert_eq!(r.stats.dropped, dropped + 1);
        // The refused piece did no harm: the datagram still completes.
        assert!(r.insert(t(0), &hdr(24, false, 8, 2), &p[24..32]).is_none());
        let (_, d) = r.insert(t(0), &hdr(8, true, 8, 2), &p[8..16]).unwrap();
        assert_eq!(d, &p[..32]);
    }

    #[test]
    fn rejects_bad_fragments() {
        let p = data(100);
        let mut r = Reassembler::new(ReassemblyConfig::default());
        // Not a multiple of 8 with MF set.
        assert!(r.insert(t(0), &hdr(0, true, 7, 1), &p[..7]).is_none());
        assert_eq!(r.in_progress(), 0);
        // Past 65535.
        assert!(r.insert(t(0), &hdr(65528, false, 16, 2), &p[..16]).is_none());
        assert_eq!(r.in_progress(), 0);
        // Empty non-final fragment.
        assert!(r.insert(t(0), &hdr(0, true, 0, 3), &[]).is_none());
    }
}
