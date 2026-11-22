//! In-process DNS resolver cache (negative NXDOMAIN and positive RR caching).
//!
//! Negative entries record definitive `NXDOMAIN` answers from a DNS server
//! across all address families (an `NXDOMAIN` means the domain name itself
//! does not exist, so a subsequent lookup for another family or with
//! `AI_DNS_ONLY` hits the same entry). Timeouts and server failures are never
//! cached. Positive entries are keyed by `(normalized_name, qtype)` and honour
//! the DNS record TTL (clamped to a safe upper bound).

use crate::dns::{self, Answer};
use alloc::string::String;
use alloc::vec::Vec;

/// Default lifetime of a negative (`NXDOMAIN`) cache entry: 900 seconds
/// (Windows' documented `MaxNegativeCacheTtl` default; specs observe >= 400 s).
pub const NEGATIVE_TTL_MS: u64 = 900_000;

/// Bounds for positive DNS cache entries.
pub const MIN_POSITIVE_TTL_MS: u64 = 1_000;
pub const MAX_POSITIVE_TTL_MS: u64 = 300_000;

const MAX_NEG_ENTRIES: usize = 256;
const MAX_POS_ENTRIES: usize = 256;

/// Result of querying the cache for `(name, qtype)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheLookup {
    Miss,
    Negative,
    Positive(Answer),
}

#[derive(Clone, Debug)]
struct NegEntry {
    name: String,
    expires_at: u64,
}

#[derive(Clone, Debug)]
struct PosEntry {
    name: String,
    qtype: u16,
    answer: Answer,
    expires_at: u64,
}

/// Bounded negative + positive DNS cache driven by monotonic millisecond timestamps.
#[derive(Debug, Default)]
pub struct DnsCache {
    neg: Vec<NegEntry>,
    pos: Vec<PosEntry>,
}

impl DnsCache {
    pub const fn new() -> DnsCache {
        DnsCache { neg: Vec::new(), pos: Vec::new() }
    }

    /// Whether `name` has an unexpired `NXDOMAIN` entry.
    pub fn is_negative(&self, name: &str, now_ms: u64) -> bool {
        let key = dns::normalize_name(name);
        self.neg.iter().any(|e| e.name == key && now_ms < e.expires_at)
    }

    /// Look up `(name, qtype)` at timestamp `now_ms`.
    pub fn lookup(&self, name: &str, qtype: u16, now_ms: u64) -> CacheLookup {
        let key = dns::normalize_name(name);
        if self.neg.iter().any(|e| e.name == key && now_ms < e.expires_at) {
            return CacheLookup::Negative;
        }
        if let Some(e) = self.pos.iter().find(|e| e.name == key && e.qtype == qtype && now_ms < e.expires_at) {
            return CacheLookup::Positive(e.answer.clone());
        }
        CacheLookup::Miss
    }

    /// Record a definitive `NXDOMAIN` response for `name`.
    pub fn insert_negative(&mut self, name: &str, now_ms: u64) {
        let key = dns::normalize_name(name);
        if key.is_empty() {
            return;
        }
        self.pos.retain(|e| e.name != key);
        self.neg.retain(|e| now_ms < e.expires_at && e.name != key);
        if self.neg.len() >= MAX_NEG_ENTRIES
            && let Some((idx, _)) = self.neg.iter().enumerate().min_by_key(|(_, e)| e.expires_at)
        {
            self.neg.swap_remove(idx);
        }
        self.neg.push(NegEntry { name: key, expires_at: now_ms.saturating_add(NEGATIVE_TTL_MS) });
    }

    /// Record a positive DNS response for `(name, qtype)`. Answers with zero
    /// TTL or no addresses are not cached.
    pub fn insert_positive(&mut self, name: &str, qtype: u16, answer: &Answer, now_ms: u64) {
        if answer.addrs.is_empty() || answer.ttl == 0 {
            return;
        }
        let key = dns::normalize_name(name);
        if key.is_empty() {
            return;
        }
        self.neg.retain(|e| e.name != key);
        self.pos.retain(|e| now_ms < e.expires_at && !(e.name == key && e.qtype == qtype));
        if self.pos.len() >= MAX_POS_ENTRIES
            && let Some((idx, _)) = self.pos.iter().enumerate().min_by_key(|(_, e)| e.expires_at)
        {
            self.pos.swap_remove(idx);
        }
        let ttl_ms = u64::from(answer.ttl).saturating_mul(1_000).clamp(MIN_POSITIVE_TTL_MS, MAX_POSITIVE_TTL_MS);
        self.pos.push(PosEntry { name: key, qtype, answer: answer.clone(), expires_at: now_ms.saturating_add(ttl_ms) });
    }

    /// Flush all cached entries.
    pub fn clear(&mut self) {
        self.neg.clear();
        self.pos.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn negative_cache_is_family_and_case_independent_and_persists_for_minutes() {
        let mut c = DnsCache::new();
        assert_eq!(c.lookup("polytest-1.invalid", dns::TYPE_A, 1_000), CacheLookup::Miss);
        c.insert_negative("PolyTest-1.INVALID.", 1_000);
        // Repeat immediately, after 1s, 3s, 10s, 30s, 400s, and for another family (AAAA).
        for dt in [0, 1_000, 3_000, 10_000, 30_000, 400_000, NEGATIVE_TTL_MS - 1] {
            let t = 1_000 + dt;
            assert!(c.is_negative("polytest-1.invalid", t));
            assert_eq!(c.lookup("polytest-1.invalid", dns::TYPE_A, t), CacheLookup::Negative);
            assert_eq!(c.lookup("POLYTEST-1.INVALID", dns::TYPE_AAAA, t), CacheLookup::Negative);
        }
        // Expires at NEGATIVE_TTL_MS.
        assert_eq!(c.lookup("polytest-1.invalid", dns::TYPE_A, 1_000 + NEGATIVE_TTL_MS), CacheLookup::Miss);
        assert!(!c.is_negative("polytest-1.invalid", 1_000 + NEGATIVE_TTL_MS));
    }

    #[test]
    fn positive_cache_is_per_qtype_clamps_ttl_and_overrides_negative() {
        let mut c = DnsCache::new();
        let a4 = Answer {
            addrs: alloc::vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))],
            canonical: Some(String::from("web.example.test")),
            ttl: 60,
        };
        let a6 = Answer {
            addrs: alloc::vec![IpAddr::V6("2001:db8::10".parse::<Ipv6Addr>().unwrap())],
            canonical: None,
            ttl: 3600,
        };
        c.insert_negative("example.test", 100);
        assert_eq!(c.lookup("example.test", dns::TYPE_A, 200), CacheLookup::Negative);

        // Positive insert clears the negative entry.
        c.insert_positive("EXAMPLE.TEST.", dns::TYPE_A, &a4, 200);
        assert_eq!(c.lookup("example.test", dns::TYPE_A, 200), CacheLookup::Positive(a4.clone()));
        assert_eq!(c.lookup("example.test", dns::TYPE_AAAA, 200), CacheLookup::Miss);

        // TTL=60s expires at 200 + 60_000.
        assert_eq!(c.lookup("example.test", dns::TYPE_A, 60_199), CacheLookup::Positive(a4));
        assert_eq!(c.lookup("example.test", dns::TYPE_A, 60_200), CacheLookup::Miss);

        // TTL=3600s is clamped to MAX_POSITIVE_TTL_MS (300_000 ms).
        c.insert_positive("example.test", dns::TYPE_AAAA, &a6, 500);
        assert_eq!(c.lookup("example.test", dns::TYPE_AAAA, 500 + MAX_POSITIVE_TTL_MS - 1), CacheLookup::Positive(a6));
        assert_eq!(c.lookup("example.test", dns::TYPE_AAAA, 500 + MAX_POSITIVE_TTL_MS), CacheLookup::Miss);

        // Zero TTL is not cached.
        let zero = Answer { addrs: alloc::vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))], canonical: None, ttl: 0 };
        c.insert_positive("zero.test", dns::TYPE_A, &zero, 1_000);
        assert_eq!(c.lookup("zero.test", dns::TYPE_A, 1_000), CacheLookup::Miss);
    }

    #[test]
    fn negative_cache_is_bounded() {
        let mut c = DnsCache::new();
        for i in 0..(MAX_NEG_ENTRIES + 20) {
            c.insert_negative(&format!("n{i}.invalid"), i as u64 * 10);
        }
        assert!(c.neg.len() <= MAX_NEG_ENTRIES);
        // The newest entry is present; the earliest was evicted.
        assert!(c.is_negative(&format!("n{}.invalid", MAX_NEG_ENTRIES + 19), 5_000));
        assert!(!c.is_negative("n0.invalid", 5_000));
        c.clear();
        assert!(!c.is_negative(&format!("n{}.invalid", MAX_NEG_ENTRIES + 19), 5_000));
    }
}
