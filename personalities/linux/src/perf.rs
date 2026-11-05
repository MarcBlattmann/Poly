//! perf_event_open(2) counters (ADR 0134): the attribute structure, its
//! checks, and software counters with their read formats, groups,
//! inheritance and ring buffer mappings.
//!
//! Poly has no hardware performance monitoring unit, so the software
//! events (`PERF_TYPE_SOFTWARE`) are the counters there are: task-clock
//! and cpu-clock (CPU time of the counted task), page-faults (and
//! -min; -maj stays 0, nothing is paged in), and events that open but do
//! not count (context switches, migrations, alignment and emulation
//! faults, dummy, bpf-output, cgroup switches). A counter is a running
//! difference of what the kernel accounts per task anyway
//! ([`PerfSource`]): enabling it notes the task's total, reading adds
//! what came since. Sampling is accepted where the checks allow it, but
//! no samples are written; the ring buffer has its header page only
//! filled in.
//!
//! Constants and layouts are those of `linux/perf_event.h`.

use crate::errno::Errno;
use crate::ns::lock::Mutex;
use crate::uring::RingMemory;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// `PERF_ATTR_SIZE_VER0` and the size of `struct perf_event_attr` Poly
/// knows (`PERF_ATTR_SIZE_VER8`).
pub const ATTR_SIZE_VER0: usize = 64;
pub const ATTR_SIZE: usize = 136;
/// The largest attribute size accepted: a page.
pub const ATTR_SIZE_MAX: usize = 4096;

/// `perf_event_open` flags (`PERF_FLAG_*`).
pub const FLAG_FD_NO_GROUP: u64 = 1 << 0;
pub const FLAG_FD_OUTPUT: u64 = 1 << 1;
pub const FLAG_PID_CGROUP: u64 = 1 << 2;
pub const FLAG_FD_CLOEXEC: u64 = 1 << 3;
pub const FLAGS_ALL: u64 = FLAG_FD_NO_GROUP | FLAG_FD_OUTPUT | FLAG_PID_CGROUP | FLAG_FD_CLOEXEC;

/// `enum perf_type_id`.
pub const TYPE_HARDWARE: u32 = 0;
pub const TYPE_SOFTWARE: u32 = 1;
pub const TYPE_TRACEPOINT: u32 = 2;
pub const TYPE_HW_CACHE: u32 = 3;
pub const TYPE_RAW: u32 = 4;
pub const TYPE_BREAKPOINT: u32 = 5;

/// `enum perf_sw_ids`.
pub mod sw {
    pub const CPU_CLOCK: u64 = 0;
    pub const TASK_CLOCK: u64 = 1;
    pub const PAGE_FAULTS: u64 = 2;
    pub const CONTEXT_SWITCHES: u64 = 3;
    pub const CPU_MIGRATIONS: u64 = 4;
    pub const PAGE_FAULTS_MIN: u64 = 5;
    pub const PAGE_FAULTS_MAJ: u64 = 6;
    pub const ALIGNMENT_FAULTS: u64 = 7;
    pub const EMULATION_FAULTS: u64 = 8;
    pub const DUMMY: u64 = 9;
    pub const BPF_OUTPUT: u64 = 10;
    pub const CGROUP_SWITCHES: u64 = 11;
    /// `PERF_COUNT_SW_MAX`.
    pub const MAX: u64 = 12;
}

/// `enum perf_event_read_format`.
pub const FORMAT_TOTAL_TIME_ENABLED: u64 = 1 << 0;
pub const FORMAT_TOTAL_TIME_RUNNING: u64 = 1 << 1;
pub const FORMAT_ID: u64 = 1 << 2;
pub const FORMAT_GROUP: u64 = 1 << 3;
pub const FORMAT_LOST: u64 = 1 << 4;
const FORMAT_MAX: u64 = 1 << 5;

/// `enum perf_event_sample_format`: the bits used here, and
/// `PERF_SAMPLE_MAX`.
pub const SAMPLE_BRANCH_STACK: u64 = 1 << 11;
pub const SAMPLE_REGS_USER: u64 = 1 << 12;
pub const SAMPLE_STACK_USER: u64 = 1 << 13;
pub const SAMPLE_REGS_INTR: u64 = 1 << 18;
const SAMPLE_MAX: u64 = 1 << 25;

/// The bit fields of `perf_event_attr` at offset 40.
pub mod bit {
    pub const DISABLED: u64 = 1 << 0;
    pub const INHERIT: u64 = 1 << 1;
    pub const EXCLUDE_USER: u64 = 1 << 4;
    pub const EXCLUDE_KERNEL: u64 = 1 << 5;
    pub const EXCLUDE_HV: u64 = 1 << 6;
    pub const FREQ: u64 = 1 << 10;
    pub const USE_CLOCKID: u64 = 1 << 25;
    pub const REMOVE_ON_EXEC: u64 = 1 << 36;
    pub const SIGTRAP: u64 = 1 << 37;
    /// `__reserved_1`: bits 38 to 63.
    pub const RESERVED: u64 = !((1 << 38) - 1);
}

/// The x86-64 user registers `sample_regs_user` may name (`enum
/// perf_event_x86_regs`: AX .. R15, 24 of them).
const REGS_VALID: u64 = (1 << 24) - 1;

/// The highest `sample_freq` accepted (`kernel.perf_event_max_sample_rate`
/// of the oracle).
pub const MAX_SAMPLE_RATE: u64 = 100_000;

/// The clocks `use_clockid` may name (`linux/time.h`: REALTIME, MONOTONIC,
/// MONOTONIC_RAW, REALTIME_COARSE... BOOTTIME, TAI).
const CLOCKS: &[i32] = &[0, 1, 4, 7, 11];

/// `struct perf_event_attr`, the fields Poly reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    pub kind: u32,
    pub size: u32,
    pub config: u64,
    /// `sample_period` or, with `freq`, `sample_freq`.
    pub sample_period: u64,
    pub sample_type: u64,
    pub read_format: u64,
    pub flags: u64,
    pub sample_regs_user: u64,
    pub sample_stack_user: u32,
    pub clockid: i32,
}

impl Attr {
    /// From the attribute's bytes (shorter ones are zero-extended).
    pub fn parse(b: &[u8]) -> Attr {
        let mut buf = [0u8; ATTR_SIZE];
        let n = b.len().min(ATTR_SIZE);
        buf[..n].copy_from_slice(&b[..n]);
        let u32_at = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().expect("4 bytes"));
        let u64_at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().expect("8 bytes"));
        Attr {
            kind: u32_at(0),
            size: u32_at(4),
            config: u64_at(8),
            sample_period: u64_at(16),
            sample_type: u64_at(24),
            read_format: u64_at(32),
            flags: u64_at(40),
            sample_regs_user: u64_at(80),
            sample_stack_user: u32_at(88),
            clockid: u32_at(92) as i32,
        }
    }

    pub const fn has(&self, b: u64) -> bool {
        self.flags & b != 0
    }

    /// The checks of the attribute itself, in the spec sheet's order:
    /// reserved bits, unknown `read_format` and `sample_type` bits,
    /// sampling settings (`EINVAL`), unknown sampled registers
    /// (`EOPNOTSUPP`).
    pub fn check(&self) -> Result<(), Errno> {
        if self.flags & bit::RESERVED != 0 || self.read_format & !(FORMAT_MAX - 1) != 0 {
            return Err(Errno::EINVAL);
        }
        if self.sample_type & !(SAMPLE_MAX - 1) != 0 {
            return Err(Errno::EINVAL);
        }
        if self.has(bit::SIGTRAP) && !self.has(bit::REMOVE_ON_EXEC) {
            return Err(Errno::EINVAL);
        }
        if self.has(bit::USE_CLOCKID) && !CLOCKS.contains(&self.clockid) {
            return Err(Errno::EINVAL);
        }
        if self.sample_type & SAMPLE_STACK_USER != 0 && !self.sample_stack_user.is_multiple_of(8) {
            return Err(Errno::EINVAL);
        }
        if self.has(bit::FREQ) {
            if self.sample_period > MAX_SAMPLE_RATE {
                return Err(Errno::EINVAL);
            }
        } else if self.sample_period & (1 << 63) != 0 {
            return Err(Errno::EINVAL);
        }
        if self.sample_type & SAMPLE_REGS_USER != 0 && self.sample_regs_user & !REGS_VALID != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        Ok(())
    }

    /// Whether the event exists here: software events below
    /// `PERF_COUNT_SW_MAX` (`ENOENT` for other ones and for types no PMU
    /// provides; a tracepoint or breakpoint without a configuration is
    /// `EINVAL`), and no branch stack on a software event (`EINVAL`).
    pub fn check_event(&self) -> Result<(), Errno> {
        match self.kind {
            TYPE_SOFTWARE if self.config < sw::MAX => {
                if self.sample_type & SAMPLE_BRANCH_STACK != 0 {
                    return Err(Errno::EINVAL);
                }
                Ok(())
            }
            TYPE_TRACEPOINT | TYPE_BREAKPOINT if self.config == 0 => Err(Errno::EINVAL),
            _ => Err(Errno::ENOENT),
        }
    }

    /// A sampling event (as opposed to a counting one).
    pub const fn sampling(&self) -> bool {
        self.sample_period != 0
    }
}

/// How large the attribute at the caller's address is, from its `size`
/// field: 0 is the first version's 64 bytes. `E2BIG` (and the size the
/// kernel knows, to be written back) for sizes below 64 or above a page.
pub fn attr_len(size: u32) -> Result<usize, Errno> {
    match size as usize {
        0 => Ok(ATTR_SIZE_VER0),
        s if !(ATTR_SIZE_VER0..=ATTR_SIZE_MAX).contains(&s) => Err(Errno::E2BIG),
        s => Ok(s),
    }
}

/// What a counter counts from: the kernel's accounting of one task.
pub trait PerfSource: Send + Sync + core::fmt::Debug {
    /// The task's total of software event `config` so far (nanoseconds
    /// for the clocks, a count otherwise).
    fn total(&self, config: u64) -> u64;
    /// The time now (monotonic nanoseconds).
    fn now_ns(&self) -> u64;
}

/// Counter ids (`PERF_FORMAT_ID`, `PERF_EVENT_IOC_ID`): unique, never 0.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default)]
struct State {
    enabled: bool,
    /// Counted while enabled before the last enable.
    base: u64,
    /// The source's total at the last enable.
    at_enable: u64,
    /// Ended inherited children's counts.
    children: u64,
    /// Nanoseconds enabled before the last enable, and its time.
    enabled_ns: u64,
    since: u64,
    /// `PERF_EVENT_IOC_PERIOD`.
    period: u64,
    /// `PERF_EVENT_IOC_SET_OUTPUT`: events go to another counter's ring.
    output: Option<Weak<Counter>>,
    paused: bool,
}

/// A software counter: what a perf event descriptor is open on.
pub struct Counter {
    pub attr: Attr,
    pub id: u64,
    source: Arc<dyn PerfSource>,
    state: Mutex<State>,
    /// A group leader's members, in the order they were opened.
    members: Mutex<Vec<Arc<Counter>>>,
    /// A member's leader.
    leader: Option<Weak<Counter>>,
    ring: Mutex<Option<Arc<dyn RingMemory>>>,
}

impl core::fmt::Debug for Counter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Counter").field("attr", &self.attr).field("id", &self.id).finish()
    }
}

impl Counter {
    /// A new counter of `attr` on `source`, a member of `leader`'s group if
    /// given; enabled unless `attr` says `disabled`.
    pub fn new(attr: Attr, source: Arc<dyn PerfSource>, leader: Option<&Arc<Counter>>) -> Arc<Counter> {
        let c = Arc::new(Counter {
            attr,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            source,
            state: Mutex::new(State { period: attr.sample_period, ..State::default() }),
            members: Mutex::new(Vec::new()),
            leader: leader.map(Arc::downgrade),
            ring: Mutex::new(None),
        });
        if let Some(l) = leader {
            l.members.lock().push(Arc::clone(&c));
        }
        if !attr.has(bit::DISABLED) {
            c.enable();
        }
        c
    }

    /// Whether it is a member of another counter's group.
    pub fn is_member(&self) -> bool {
        self.leader.is_some()
    }

    /// The source's total, for the events Poly counts (0 for the others).
    fn total(&self) -> u64 {
        match self.attr.config {
            sw::CPU_CLOCK | sw::TASK_CLOCK | sw::PAGE_FAULTS | sw::PAGE_FAULTS_MIN => {
                self.source.total(self.attr.config)
            }
            _ => 0,
        }
    }

    pub fn enable(&self) {
        let (total, now) = (self.total(), self.source.now_ns());
        let mut s = self.state.lock();
        if !s.enabled {
            s.enabled = true;
            s.at_enable = total;
            s.since = now;
        }
    }

    pub fn disable(&self) {
        let (total, now) = (self.total(), self.source.now_ns());
        let mut s = self.state.lock();
        if s.enabled {
            s.enabled = false;
            s.base += total.saturating_sub(s.at_enable);
            s.enabled_ns += now.saturating_sub(s.since);
        }
    }

    pub fn reset(&self) {
        let total = self.total();
        let mut s = self.state.lock();
        s.base = 0;
        s.children = 0;
        s.at_enable = total;
    }

    /// An inherited child of the counted task ended, having counted
    /// `total` of the event.
    pub fn add_child(&self, total: u64) {
        let mut s = self.state.lock();
        if s.enabled {
            s.children += total;
        }
    }

    /// The value and the times enabled and running.
    pub fn value(&self) -> (u64, u64, u64) {
        let (total, now) = (self.total(), self.source.now_ns());
        let s = self.state.lock();
        let (running, time) = if s.enabled {
            (total.saturating_sub(s.at_enable), now.saturating_sub(s.since))
        } else {
            (0, 0)
        };
        let enabled = s.enabled_ns + time;
        (s.base + running + s.children, enabled, enabled)
    }

    /// The group this counter reads with `PERF_FORMAT_GROUP`: its leader
    /// (or itself) first, then the members in the order they were opened.
    pub fn group(self: &Arc<Self>) -> Vec<Arc<Counter>> {
        let leader = self.leader.as_ref().and_then(Weak::upgrade).unwrap_or_else(|| Arc::clone(self));
        let mut all = alloc::vec![Arc::clone(&leader)];
        all.extend(leader.members.lock().iter().cloned());
        all
    }

    /// read(2) of the descriptor: the values `read_format` asks for, as
    /// `u64`s (`ENOSPC` if `len` is too small for them).
    pub fn read(self: &Arc<Self>, len: usize) -> Result<Vec<u8>, Errno> {
        let f = self.attr.read_format;
        let mut out: Vec<u64> = Vec::new();
        if f & FORMAT_GROUP != 0 {
            let group = self.group();
            let (_, enabled, running) = group[0].value();
            out.push(group.len() as u64);
            if f & FORMAT_TOTAL_TIME_ENABLED != 0 {
                out.push(enabled);
            }
            if f & FORMAT_TOTAL_TIME_RUNNING != 0 {
                out.push(running);
            }
            for c in &group {
                out.push(c.value().0);
                if f & FORMAT_ID != 0 {
                    out.push(c.id);
                }
                if f & FORMAT_LOST != 0 {
                    out.push(0);
                }
            }
        } else {
            let (value, enabled, running) = self.value();
            out.push(value);
            if f & FORMAT_TOTAL_TIME_ENABLED != 0 {
                out.push(enabled);
            }
            if f & FORMAT_TOTAL_TIME_RUNNING != 0 {
                out.push(running);
            }
            if f & FORMAT_ID != 0 {
                out.push(self.id);
            }
            if f & FORMAT_LOST != 0 {
                out.push(0);
            }
        }
        if len < out.len() * 8 {
            return Err(Errno::ENOSPC);
        }
        Ok(out.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// The counters an ioctl with `PERF_IOC_FLAG_GROUP` acts on.
    pub fn targets(self: &Arc<Self>, group: bool) -> Vec<Arc<Counter>> {
        if group { self.group() } else { alloc::vec![Arc::clone(self)] }
    }

    pub fn set_period(&self, period: u64) {
        self.state.lock().period = period;
    }

    pub fn has_ring(&self) -> bool {
        self.ring.lock().is_some()
    }

    pub fn set_output(&self, to: Option<&Arc<Counter>>) {
        self.state.lock().output = to.map(Arc::downgrade);
    }

    pub fn pause(&self, on: bool) {
        self.state.lock().paused = on;
    }

    /// mmap(2) of the descriptor, `len` bytes at file offset `offset`:
    /// shared, at offset 0, one page (the header) or one plus a power of
    /// two (the data); a ring in use (`mapped`) keeps its size (`EINVAL`
    /// otherwise). `make` gives new ring memory. Returns the ring.
    pub fn map(
        &self,
        len: u64,
        offset: u64,
        shared: bool,
        mapped: impl Fn(&Arc<dyn RingMemory>) -> bool,
        make: impl FnOnce(usize) -> Result<Arc<dyn RingMemory>, Errno>,
    ) -> Result<Arc<dyn RingMemory>, Errno> {
        const PAGE: u64 = 4096;
        if !shared || offset != 0 || len == 0 || !len.is_multiple_of(PAGE) {
            return Err(Errno::EINVAL);
        }
        let data = len / PAGE - 1;
        if data != 0 && !data.is_power_of_two() {
            return Err(Errno::EINVAL);
        }
        let mut ring = self.ring.lock();
        if let Some(r) = ring.as_ref() {
            if r.len() as u64 == len {
                return Ok(Arc::clone(r));
            }
            if mapped(r) {
                return Err(Errno::EINVAL);
            }
        }
        let r = make(len as usize)?;
        // struct perf_event_mmap_page: data_offset at 1040, data_size at
        // 1048 (data_head and data_tail at 1024 and 1032 stay 0).
        if data != 0 {
            r.write(1040, &PAGE.to_le_bytes());
            r.write(1048, &(data * PAGE).to_le_bytes());
        }
        *ring = Some(Arc::clone(&r));
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicU64;

    #[derive(Debug, Default)]
    struct Fake {
        faults: AtomicU64,
        now: AtomicU64,
    }

    impl PerfSource for Fake {
        fn total(&self, _config: u64) -> u64 {
            self.faults.load(Ordering::Relaxed)
        }
        fn now_ns(&self) -> u64 {
            self.now.load(Ordering::Relaxed)
        }
    }

    fn attr(config: u64, flags: u64, read_format: u64) -> Attr {
        Attr { kind: TYPE_SOFTWARE, size: 136, config, flags, read_format, ..Attr::default() }
    }

    #[test]
    fn sizes() {
        assert_eq!(attr_len(0), Ok(64));
        assert_eq!(attr_len(63), Err(Errno::E2BIG));
        assert_eq!(attr_len(136), Ok(136));
        assert_eq!(attr_len(4096), Ok(4096));
        assert_eq!(attr_len(4097), Err(Errno::E2BIG));
    }

    #[test]
    fn attribute_checks() {
        let ok = attr(sw::TASK_CLOCK, bit::EXCLUDE_KERNEL, 0);
        assert_eq!(ok.check(), Ok(()));
        assert_eq!(Attr { flags: 1 << 63, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { read_format: 1 << 10, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { sample_type: 1 << 40, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { flags: bit::SIGTRAP, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { flags: bit::USE_CLOCKID, clockid: 99, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { flags: bit::USE_CLOCKID, clockid: 1, ..ok }.check(), Ok(()));
        assert_eq!(Attr { flags: bit::FREQ, sample_period: 1_000_000_000, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { sample_period: 1 << 63, ..ok }.check(), Err(Errno::EINVAL));
        assert_eq!(Attr { sample_type: SAMPLE_STACK_USER, sample_stack_user: 3, ..ok }.check(), Err(Errno::EINVAL));
        let regs = Attr { sample_type: SAMPLE_REGS_USER, sample_regs_user: 1 << 60, ..ok };
        assert_eq!(regs.check(), Err(Errno::EOPNOTSUPP));
        assert_eq!(Attr { sample_regs_user: 1, ..regs }.check(), Ok(()));
        assert_eq!(Attr { config: 12, ..ok }.check_event(), Err(Errno::ENOENT));
        assert_eq!(Attr { kind: 1000, ..ok }.check_event(), Err(Errno::ENOENT));
        assert_eq!(Attr { kind: TYPE_HARDWARE, ..ok }.check_event(), Err(Errno::ENOENT));
        let branch = Attr { sample_type: SAMPLE_BRANCH_STACK, sample_period: 1, ..ok };
        assert_eq!(branch.check_event(), Err(Errno::EINVAL));
    }

    #[test]
    fn counting_enable_disable_reset() {
        let src = Arc::new(Fake::default());
        let c = Counter::new(attr(sw::PAGE_FAULTS, bit::DISABLED, 0), src.clone(), None);
        src.faults.store(10, Ordering::Relaxed);
        assert_eq!(c.read(8).unwrap(), 0u64.to_le_bytes());
        assert_eq!(c.read(7), Err(Errno::ENOSPC));
        c.enable();
        src.faults.store(110, Ordering::Relaxed);
        c.disable();
        src.faults.store(200, Ordering::Relaxed);
        assert_eq!(c.value().0, 100);
        c.reset();
        assert_eq!(c.value().0, 0);
    }

    #[test]
    fn group_read_format() {
        let src = Arc::new(Fake::default());
        let f = FORMAT_GROUP | FORMAT_ID | FORMAT_TOTAL_TIME_ENABLED | FORMAT_TOTAL_TIME_RUNNING;
        let leader = Counter::new(attr(sw::PAGE_FAULTS, 0, f), src.clone(), None);
        let member = Counter::new(attr(sw::DUMMY, 0, f), src, Some(&leader));
        assert!(member.is_member());
        let bytes = member.read(56).unwrap();
        assert_eq!(bytes.len(), 56, "nr, two times, two values with ids");
        assert_eq!(u64::from_le_bytes(bytes[0..8].try_into().unwrap()), 2);
        assert_eq!(u64::from_le_bytes(bytes[32..40].try_into().unwrap()), leader.id, "leader first");
        assert_eq!(member.read(55), Err(Errno::ENOSPC));
    }
}

/// The counters a process opened (for prctl(2)
/// `PR_TASK_PERF_EVENTS_DISABLE` / `_ENABLE`) and those it counts into as
/// an inherited child (`inherit`), in its [`crate::LinuxProcess`].
#[derive(Clone, Default)]
pub struct Links {
    owned: Vec<Weak<Counter>>,
    inherit: Vec<Arc<Counter>>,
    inherited: Vec<Arc<Counter>>,
}

impl Links {
    /// The process opened `c`; with `inherit`, its later children count
    /// into it.
    pub fn own(&mut self, c: &Arc<Counter>, inherit: bool) {
        self.owned.retain(|w| w.strong_count() > 0);
        self.owned.push(Arc::downgrade(c));
        if inherit {
            self.inherit.push(Arc::clone(c));
        }
    }

    pub fn owned(&self) -> Vec<Arc<Counter>> {
        self.owned.iter().filter_map(Weak::upgrade).collect()
    }

    pub fn inherited(&self) -> Vec<Arc<Counter>> {
        self.inherited.clone()
    }

    /// A child's links: it counts into the inheritable counters (and those
    /// its parent inherited), and owns none.
    pub fn for_child(&self) -> Links {
        let mut inherited = self.inherit.clone();
        inherited.extend(self.inherited.iter().cloned());
        Links { owned: Vec::new(), inherit: Vec::new(), inherited }
    }
}

impl core::fmt::Debug for Links {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Links").field("owned", &self.owned.len()).field("inherited", &self.inherited.len()).finish()
    }
}

impl PartialEq for Links {
    fn eq(&self, other: &Links) -> bool {
        self.owned.len() == other.owned.len()
            && self.inherit.iter().zip(&other.inherit).all(|(a, b)| Arc::ptr_eq(a, b))
            && self.inherited.iter().zip(&other.inherited).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

impl Eq for Links {}
