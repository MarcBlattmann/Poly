//! Simulated hardware for the unit tests, written from the register
//! descriptions in the SDM, the 82093AA and 8259A datasheets and the HPET
//! specification.
//!
//! All the simulated counters share one [`SimClock`]. Every counter read
//! takes simulated time (a fixed cost plus random jitter, and now and then a
//! long stall), so busy-wait loops make progress and calibration sees the
//! same kind of noise it sees on real machines and under hypervisors. The
//! devices panic when the driver does something the hardware forbids
//! (x2APIC accesses that would raise #GP, writing the HPET main counter
//! while it runs, an 8259 command in the middle of initialisation).

extern crate std;

use crate::lapic::{Divide, reg};
use crate::time::Counter;
use crate::{Mmio, Msr, PortIo};
use core::cell::{Cell, RefCell};
use std::vec::Vec;

const PS_PER_SEC: u128 = 1_000_000_000_000;

/// A small deterministic random number generator (xorshift64).
#[derive(Clone, Copy, Debug)]
pub struct Xorshift(pub u64);

impl Xorshift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Simulated time, in picoseconds.
#[derive(Debug)]
pub struct SimClock {
    now_ps: Cell<u64>,
    rng: Cell<Xorshift>,
    cost_ps: u64,
    jitter_ps: u64,
    stall_one_in: u64,
    stall_ps: u64,
}

impl SimClock {
    /// Reads are free until [`SimClock::read_cost`] says otherwise.
    pub fn new(seed: u64) -> SimClock {
        SimClock {
            now_ps: Cell::new(1_000_000),
            rng: Cell::new(Xorshift(seed | 1)),
            cost_ps: 0,
            jitter_ps: 0,
            stall_one_in: 0,
            stall_ps: 0,
        }
    }

    /// Each counter read takes `base_ps` plus up to `jitter_ps`.
    pub fn read_cost(mut self, base_ps: u64, jitter_ps: u64) -> SimClock {
        self.cost_ps = base_ps;
        self.jitter_ps = jitter_ps;
        self
    }

    /// One read in `one_in` (on average) also stalls for `ps`.
    pub fn stalls(mut self, one_in: u64, ps: u64) -> SimClock {
        self.stall_one_in = one_in;
        self.stall_ps = ps;
        self
    }

    fn random(&self) -> u64 {
        let mut r = self.rng.get();
        let v = r.next();
        self.rng.set(r);
        v
    }

    pub fn now_ps(&self) -> u64 {
        self.now_ps.get()
    }

    pub fn advance_ps(&self, ps: u64) {
        self.now_ps.set(self.now_ps.get() + ps);
    }

    /// Called by every simulated counter read after it sampled the time.
    pub fn read_done(&self) {
        let mut d = self.cost_ps;
        if self.jitter_ps > 0 {
            d += self.random() % self.jitter_ps;
        }
        if self.stall_one_in > 0 && self.random().is_multiple_of(self.stall_one_in) {
            d += self.stall_ps;
        }
        self.advance_ps(d);
    }
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A TSC counting at `hz`, or with [`FakeTsc::wobble`] at a rate that jumps
/// around like a TSC that follows the core clock.
#[derive(Debug)]
pub struct FakeTsc<'a> {
    clock: &'a SimClock,
    hz: u64,
    wobble: Option<(u64, u64)>,
    /// Ticks times 10^12, to integrate without rounding.
    acc: Cell<u128>,
    last_ps: Cell<u64>,
}

impl<'a> FakeTsc<'a> {
    pub fn new(clock: &'a SimClock, hz: u64) -> FakeTsc<'a> {
        FakeTsc { clock, hz, wobble: None, acc: Cell::new(0), last_ps: Cell::new(clock.now_ps()) }
    }

    /// Each `segment_ps` of time runs at `hz` plus up to `spread_hz`.
    pub fn wobble(mut self, segment_ps: u64, spread_hz: u64) -> FakeTsc<'a> {
        self.wobble = Some((segment_ps, spread_hz));
        self
    }

    fn rate(&self, segment: u64) -> u64 {
        match self.wobble {
            Some((_, spread)) => self.hz + splitmix(segment) % spread,
            None => self.hz,
        }
    }
}

impl Counter for FakeTsc<'_> {
    fn read(&self) -> u64 {
        let now = self.clock.now_ps();
        let mut last = self.last_ps.get();
        let mut acc = self.acc.get();
        let seg_ps = self.wobble.map_or(u64::MAX, |(s, _)| s);
        while last < now {
            let segment = last / seg_ps;
            let end = now.min((segment + 1).saturating_mul(seg_ps));
            acc += (end - last) as u128 * self.rate(segment) as u128;
            last = end;
        }
        self.acc.set(acc);
        self.last_ps.set(now);
        self.clock.read_done();
        (acc / PS_PER_SEC) as u64
    }
}

/// A counter that never moves.
#[derive(Debug)]
pub struct Stuck(pub u64);

impl Counter for Stuck {
    fn read(&self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// HPET

const HPET_PERIOD_FS: u64 = 69_841_279;

#[derive(Debug)]
struct SimHpetTimer {
    config: Cell<u64>,
    comparator: Cell<u64>,
    period: Cell<u64>,
    fsb: Cell<u64>,
    val_set: Cell<bool>,
}

/// An HPET block with three timers: 0 periodic-capable and routable to
/// I/O APIC input 2 or 16-23, 1 one-shot only and routable to 8 or 16-23, 2
/// with FSB delivery and a 32-bit comparator. `wide` gives a 64-bit main
/// counter (and 64-bit comparators on timers 0 and 1) plus legacy
/// replacement; otherwise everything is 32 bits.
#[derive(Debug)]
pub struct SimHpet<'a> {
    clock: &'a SimClock,
    caps: Cell<u64>,
    config: Cell<u64>,
    status: Cell<u64>,
    /// Main counter when last enabled (or now, while halted).
    base: Cell<u64>,
    started_ps: Cell<u64>,
    timers: [SimHpetTimer; 3],
    read_cost_ps: Cell<u64>,
    live_comparator_writes: Cell<u32>,
}

impl<'a> SimHpet<'a> {
    pub fn new(clock: &'a SimClock, wide: bool) -> SimHpet<'a> {
        use crate::hpet::timer::*;
        let size = if wide { SIZE_64 } else { 0 };
        let caps = HPET_PERIOD_FS << 32 | 0x8086 << 16 | 2 << 8 | 1 | if wide { 1 << 13 | 1 << 15 } else { 0 };
        let timer = |config: u64| SimHpetTimer {
            config: Cell::new(config),
            comparator: Cell::new(if config & SIZE_64 != 0 { u64::MAX } else { u32::MAX as u64 }),
            period: Cell::new(0),
            fsb: Cell::new(0),
            val_set: Cell::new(false),
        };
        SimHpet {
            clock,
            caps: Cell::new(caps),
            config: Cell::new(0),
            status: Cell::new(0),
            base: Cell::new(0),
            started_ps: Cell::new(0),
            timers: [
                timer(PERIODIC_CAPABLE | size | 0x00ff_0004 << 32),
                timer(size | 0x00ff_0100 << 32),
                timer(FSB_CAPABLE | 0x00ff_0000 << 32),
            ],
            read_cost_ps: Cell::new(0),
            live_comparator_writes: Cell::new(0),
        }
    }

    fn wide(&self) -> bool {
        self.caps.get() & (1 << 13) != 0
    }

    fn enabled(&self) -> bool {
        self.config.get() & crate::hpet::config::ENABLE != 0
    }

    fn counter_now(&self) -> u64 {
        let mut v = self.base.get();
        if self.enabled() {
            let fs = (self.clock.now_ps() - self.started_ps.get()) as u128 * 1000;
            v = v.wrapping_add((fs / HPET_PERIOD_FS as u128) as u64);
        }
        if self.wide() { v } else { v & 0xffff_ffff }
    }

    pub fn set_caps(&self, caps: u64) {
        self.caps.set(caps);
    }

    pub fn set_read_cost_ps(&self, ps: u64) {
        self.read_cost_ps.set(ps);
    }

    pub fn write_raw(&self, offset: usize, value: u32) {
        Mmio::write32(&self, offset, value);
    }

    pub fn configuration(&self) -> u64 {
        self.config.get()
    }

    pub fn timer_config(&self, n: usize) -> u64 {
        self.timers[n].config.get()
    }

    pub fn fsb_route(&self, n: usize) -> u64 {
        self.timers[n].fsb.get()
    }

    pub fn periodic_state(&self, n: usize) -> (u64, u64) {
        (self.timers[n].comparator.get(), self.timers[n].period.get())
    }

    pub fn comparator_writes_while_enabled(&self) -> u32 {
        self.live_comparator_writes.get()
    }

    pub fn raise(&self, n: u32) {
        self.status.set(self.status.get() | 1 << n);
    }

    fn timer_write(&self, n: usize, reg: usize, high: bool, value: u32) {
        use crate::hpet::timer::*;
        let t = &self.timers[n];
        let config = t.config.get();
        let set_half = |old: u64| {
            if high { old & 0xffff_ffff | (value as u64) << 32 } else { old & !0xffff_ffff | value as u64 }
        };
        match reg {
            0x00 => {
                if high {
                    return; // route capability: read-only
                }
                let mut writable = LEVEL | INT_ENABLE | ROUTE_MASK;
                if config & PERIODIC_CAPABLE != 0 {
                    writable |= PERIODIC;
                }
                if config & SIZE_64 != 0 {
                    writable |= MODE_32;
                }
                if config & FSB_CAPABLE != 0 {
                    writable |= FSB_ENABLE;
                }
                let new = config & !writable | value as u64 & writable;
                let route = (new & ROUTE_MASK) >> ROUTE_SHIFT;
                if new & ROUTE_MASK != config & ROUTE_MASK {
                    assert!(config >> 32 & 1 << route != 0, "timer {n} routed to a disallowed input {route}");
                }
                if value as u64 & VAL_SET != 0 {
                    assert!(new & PERIODIC != 0, "VAL_SET on a one-shot timer");
                    t.val_set.set(true);
                }
                t.config.set(new);
            }
            0x08 => {
                if config & INT_ENABLE != 0 {
                    self.live_comparator_writes.set(self.live_comparator_writes.get() + 1);
                }
                let narrow = config & SIZE_64 == 0 || config & MODE_32 != 0;
                if narrow && high {
                    return;
                }
                if config & PERIODIC != 0 && !t.val_set.get() {
                    t.period.set(if narrow { value as u64 } else { set_half(t.period.get()) });
                } else {
                    t.val_set.set(false);
                    t.comparator.set(if narrow { value as u64 } else { set_half(t.comparator.get()) });
                }
            }
            0x10 => {
                assert!(config & FSB_CAPABLE != 0, "FSB route of a timer without FSB delivery");
                t.fsb.set(set_half(t.fsb.get()));
            }
            _ => panic!("write to reserved HPET offset"),
        }
    }
}

impl Mmio for &SimHpet<'_> {
    fn read32(&self, offset: usize) -> u32 {
        assert!(offset.is_multiple_of(4) && offset < 0x400, "bad HPET read at {offset:#x}");
        let high = offset & 4 != 0;
        let v = match offset & !7 {
            0x000 => self.caps.get(),
            0x010 => self.config.get(),
            0x020 => self.status.get(),
            0x0f0 => {
                let v = self.counter_now();
                self.clock.read_done();
                self.clock.advance_ps(self.read_cost_ps.get());
                v
            }
            o if o >= 0x100 && (o - 0x100) / 0x20 < 3 => {
                let t = &self.timers[(o - 0x100) / 0x20];
                match (o - 0x100) % 0x20 {
                    0x00 => t.config.get(),
                    0x08 => t.comparator.get(),
                    _ => t.fsb.get(),
                }
            }
            _ => 0,
        };
        if high { (v >> 32) as u32 } else { v as u32 }
    }

    fn write32(&self, offset: usize, value: u32) {
        assert!(offset.is_multiple_of(4) && offset < 0x400, "bad HPET write at {offset:#x}");
        let high = offset & 4 != 0;
        match offset & !7 {
            0x000 => {}
            0x010 => {
                if high {
                    return;
                }
                let mut writable = crate::hpet::config::ENABLE;
                if self.caps.get() & (1 << 15) != 0 {
                    writable |= crate::hpet::config::LEGACY_REPLACEMENT;
                }
                let was = self.enabled();
                let now = self.counter_now();
                self.config.set(self.config.get() & !writable | value as u64 & writable);
                if was != self.enabled() {
                    self.base.set(now);
                    self.started_ps.set(self.clock.now_ps());
                }
            }
            0x020 => {
                if !high {
                    self.status.set(self.status.get() & !(value as u64));
                }
            }
            0x0f0 => {
                assert!(!self.enabled(), "HPET main counter written while running");
                let b = self.base.get();
                if high {
                    if self.wide() {
                        self.base.set(b & 0xffff_ffff | (value as u64) << 32);
                    }
                } else {
                    self.base.set(b & !0xffff_ffff | value as u64);
                }
            }
            o if o >= 0x100 && (o - 0x100) / 0x20 < 3 => {
                self.timer_write((o - 0x100) / 0x20, (o - 0x100) % 0x20, high, value)
            }
            o => panic!("write to reserved HPET offset {o:#x}"),
        }
    }

    fn read8(&self, _: usize) -> u8 {
        panic!("HPET accessed with 8 bits")
    }

    fn read16(&self, _: usize) -> u16 {
        panic!("HPET accessed with 16 bits")
    }

    fn write8(&self, _: usize, _: u8) {
        panic!("HPET accessed with 8 bits")
    }

    fn write16(&self, _: usize, _: u16) {
        panic!("HPET accessed with 16 bits")
    }
}

// ---------------------------------------------------------------------------
// Local APIC

/// The simulated APIC timer runs at 1 GHz before the divider, as QEMU's does.
const APIC_BUS_HZ: u128 = 1_000_000_000;

/// One local APIC, reachable as an xAPIC page (`Mmio`) or through MSRs
/// (`Msr`, which also has `IA32_APIC_BASE` and `IA32_TSC_DEADLINE`). Which
/// one is legal follows `IA32_APIC_BASE`, as on hardware.
#[derive(Debug)]
pub struct SimApic<'a> {
    clock: &'a SimClock,
    id: u32,
    version: u32,
    regs: RefCell<[u32; 64]>,
    apic_base: Cell<u64>,
    /// Destination and low ICR dword of every IPI sent.
    ipis: RefCell<Vec<(u32, u32)>>,
    /// Every write that reached a register common to both modes.
    writes: RefCell<Vec<(u16, u32)>>,
    eois: Cell<u32>,
    busy: Cell<u32>,
    busy_reads: Cell<u32>,
    icr_reads: Cell<u32>,
    pending_errors: Cell<u32>,
    timer_start_ps: Cell<u64>,
    deadline: Cell<u64>,
}

fn idx(offset: u16) -> usize {
    offset as usize >> 4
}

impl<'a> SimApic<'a> {
    /// Starts out globally enabled in xAPIC mode, software-disabled, every
    /// LVT entry masked: the state after reset.
    pub fn new(clock: &'a SimClock, id: u32, version: u32) -> SimApic<'a> {
        let mut regs = [0u32; 64];
        for r in [
            reg::LVT_CMCI,
            reg::LVT_TIMER,
            reg::LVT_THERMAL,
            reg::LVT_PERF,
            reg::LVT_LINT0,
            reg::LVT_LINT1,
            reg::LVT_ERROR,
        ] {
            regs[idx(r)] = 1 << 16;
        }
        regs[idx(reg::SVR)] = 0xff;
        regs[idx(reg::DFR)] = u32::MAX;
        SimApic {
            clock,
            id,
            version,
            regs: RefCell::new(regs),
            apic_base: Cell::new(0xfee0_0000 | 1 << 11 | 1 << 8),
            ipis: RefCell::new(Vec::new()),
            writes: RefCell::new(Vec::new()),
            eois: Cell::new(0),
            busy: Cell::new(0),
            busy_reads: Cell::new(0),
            icr_reads: Cell::new(0),
            pending_errors: Cell::new(0),
            timer_start_ps: Cell::new(0),
            deadline: Cell::new(0),
        }
    }

    pub fn set_apic_base(&self, v: u64) {
        self.apic_base.set(v);
    }

    /// Reads of the ICR show delivery pending for `n` reads after each send
    /// (`u32::MAX`: forever).
    pub fn set_busy_reads(&self, n: u32) {
        self.busy_reads.set(n);
        self.busy.set(n.min(self.busy.get()));
    }

    pub fn poke(&self, offset: u16, v: u32) {
        self.regs.borrow_mut()[idx(offset)] = v;
    }

    pub fn last_write(&self, offset: u16) -> Option<u32> {
        self.writes.borrow().iter().rev().find(|w| w.0 == offset).map(|w| w.1)
    }

    pub fn register_writes(&self) -> Vec<(u16, u32)> {
        self.writes.borrow().clone()
    }

    pub fn ipis(&self) -> Vec<(u32, u32)> {
        self.ipis.borrow().clone()
    }

    pub fn eois(&self) -> u32 {
        self.eois.get()
    }

    pub fn icr_reads(&self) -> u32 {
        self.icr_reads.get()
    }

    pub fn tsc_deadline(&self) -> u64 {
        self.deadline.get()
    }

    fn xapic_mode(&self) -> bool {
        self.apic_base.get() & (1 << 11 | 1 << 10) == 1 << 11
    }

    fn x2apic_mode(&self) -> bool {
        self.apic_base.get() & (1 << 11 | 1 << 10) == 1 << 11 | 1 << 10
    }

    fn lvt_present(&self, offset: u16) -> bool {
        let max = (self.version >> 16) as u8;
        match offset {
            reg::LVT_PERF => max >= 4,
            reg::LVT_THERMAL => max >= 5,
            reg::LVT_CMCI => max >= 6,
            _ => true,
        }
    }

    fn get(&self, offset: u16) -> u32 {
        self.regs.borrow()[idx(offset)]
    }

    fn set(&self, offset: u16, v: u32) {
        self.regs.borrow_mut()[idx(offset)] = v;
    }

    fn current_count(&self) -> u32 {
        let initial = self.get(reg::TIMER_INITIAL);
        let mode = (self.get(reg::LVT_TIMER) >> 17) & 0b11;
        if initial == 0 || mode == 0b10 {
            return 0;
        }
        let divisor = Divide::from_bits(self.get(reg::TIMER_DIVIDE)).divisor() as u128;
        let elapsed_ps = (self.clock.now_ps() - self.timer_start_ps.get()) as u128;
        let ticks = elapsed_ps * APIC_BUS_HZ / PS_PER_SEC / divisor;
        if mode == 0b01 {
            initial - (ticks % initial as u128) as u32
        } else {
            (initial as u128).saturating_sub(ticks) as u32
        }
    }

    fn core_read(&self, offset: u16) -> u32 {
        assert!(self.lvt_present(offset), "read of missing LVT entry {offset:#x}");
        match offset {
            reg::TIMER_CURRENT => {
                let v = self.current_count();
                self.clock.read_done();
                v
            }
            reg::PPR => self.get(reg::TPR),
            reg::VERSION => self.version,
            _ => self.get(offset),
        }
    }

    fn core_write(&self, offset: u16, v: u32) {
        assert!(self.lvt_present(offset), "write to missing LVT entry {offset:#x}");
        self.writes.borrow_mut().push((offset, v));
        let lvt_writable = match offset {
            reg::LVT_TIMER => 0x700ff,
            reg::LVT_LINT0 | reg::LVT_LINT1 => 0x1a7ff,
            reg::LVT_ERROR => 0x100ff,
            reg::LVT_THERMAL | reg::LVT_PERF | reg::LVT_CMCI => 0x107ff,
            _ => 0,
        };
        match offset {
            reg::TPR => self.set(offset, v & 0xff),
            reg::EOI => self.eois.set(self.eois.get() + 1),
            reg::SVR => {
                self.set(offset, v & 0x13ff);
                if v & 0x100 == 0 {
                    for r in [reg::LVT_CMCI, reg::LVT_TIMER, reg::LVT_THERMAL, reg::LVT_PERF, reg::LVT_LINT0]
                        .into_iter()
                        .chain([reg::LVT_LINT1, reg::LVT_ERROR])
                    {
                        self.set(r, self.get(r) | 1 << 16);
                    }
                }
            }
            reg::ESR => {
                self.set(offset, self.pending_errors.get());
                self.pending_errors.set(0);
            }
            _ if lvt_writable != 0 => {
                let mut new = self.get(offset) & !lvt_writable | v & lvt_writable;
                if self.get(reg::SVR) & 0x100 == 0 {
                    new |= 1 << 16;
                }
                self.set(offset, new);
            }
            reg::TIMER_INITIAL => {
                self.set(offset, v);
                self.timer_start_ps.set(self.clock.now_ps());
            }
            reg::TIMER_DIVIDE => self.set(offset, v & 0b1011),
            reg::LDR => self.set(offset, v & 0xff00_0000),
            reg::DFR => self.set(offset, v | 0x0fff_ffff),
            reg::ID | reg::VERSION | reg::PPR | reg::TIMER_CURRENT => {}
            o if (reg::ISR..reg::ESR).contains(&o) => {}
            o => panic!("write to unknown APIC register {o:#x}"),
        }
    }

    fn send(&self, destination: u32, low: u32) {
        self.ipis.borrow_mut().push((destination, low & !(1 << 12)));
        self.busy.set(self.busy_reads.get());
        let delivery = (low >> 8) & 0b111;
        if delivery <= 1 && (low & 0xff) < 16 {
            self.pending_errors.set(self.pending_errors.get() | crate::lapic::ErrorStatus::SEND_ILLEGAL_VECTOR);
        }
    }
}

impl Mmio for &SimApic<'_> {
    fn read32(&self, offset: usize) -> u32 {
        assert!(self.xapic_mode(), "xAPIC page read outside xAPIC mode");
        assert!(offset.is_multiple_of(16) && offset < 0x400, "bad APIC read at {offset:#x}");
        match offset as u16 {
            reg::ID => self.id << 24,
            reg::EOI => panic!("read of the write-only EOI register"),
            reg::SELF_IPI => panic!("SELF IPI is x2APIC only"),
            reg::ICR_LOW => {
                self.icr_reads.set(self.icr_reads.get() + 1);
                let mut v = self.get(reg::ICR_LOW);
                let b = self.busy.get();
                if b > 0 {
                    v |= 1 << 12;
                    if b != u32::MAX {
                        self.busy.set(b - 1);
                    }
                }
                v
            }
            o => self.core_read(o),
        }
    }

    fn write32(&self, offset: usize, value: u32) {
        assert!(self.xapic_mode(), "xAPIC page write outside xAPIC mode");
        assert!(offset.is_multiple_of(16) && offset < 0x400, "bad APIC write at {offset:#x}");
        match offset as u16 {
            reg::ID => {}
            reg::ICR_HIGH => self.set(reg::ICR_HIGH, value & 0xff00_0000),
            reg::ICR_LOW => {
                self.set(reg::ICR_LOW, value & !(1 << 12));
                self.send(self.get(reg::ICR_HIGH) >> 24, value);
            }
            reg::SELF_IPI => panic!("SELF IPI is x2APIC only"),
            o => self.core_write(o, value),
        }
    }

    fn read8(&self, _: usize) -> u8 {
        panic!("APIC registers are 32-bit only")
    }

    fn read16(&self, _: usize) -> u16 {
        panic!("APIC registers are 32-bit only")
    }

    fn write8(&self, _: usize, _: u8) {
        panic!("APIC registers are 32-bit only")
    }

    fn write16(&self, _: usize, _: u16) {
        panic!("APIC registers are 32-bit only")
    }
}

impl Msr for &SimApic<'_> {
    fn read(&self, msr: u32) -> u64 {
        match msr {
            0x1b => self.apic_base.get(),
            0x6e0 => self.deadline.get(),
            0x800..=0x8ff => {
                assert!(self.x2apic_mode(), "#GP: x2APIC MSR {msr:#x} read outside x2APIC mode");
                match ((msr - 0x800) << 4) as u16 {
                    reg::ID => self.id as u64,
                    reg::EOI => panic!("#GP: read of the EOI MSR"),
                    reg::ICR_LOW => self.get(reg::ICR_LOW) as u64 | (self.get(reg::ICR_HIGH) as u64) << 32,
                    o @ (reg::ICR_HIGH | reg::DFR | reg::APR | reg::SELF_IPI) => panic!("#GP: x2APIC has no {o:#x}"),
                    o => self.core_read(o) as u64,
                }
            }
            _ => panic!("#GP: unknown MSR {msr:#x}"),
        }
    }

    fn write(&self, msr: u32, value: u64) {
        match msr {
            0x1b => {
                let (en, extd) = (value & 1 << 11 != 0, value & 1 << 10 != 0);
                assert!(!(extd && !en), "#GP: EXTD without EN is invalid");
                assert!(!(self.x2apic_mode() && en && !extd), "#GP: x2APIC to xAPIC directly");
                self.apic_base.set(value);
            }
            0x6e0 => self.deadline.set(value),
            0x800..=0x8ff => {
                assert!(self.x2apic_mode(), "#GP: x2APIC MSR {msr:#x} written outside x2APIC mode");
                let offset = ((msr - 0x800) << 4) as u16;
                if offset != reg::ICR_LOW {
                    assert_eq!(value >> 32, 0, "#GP: reserved bits of {msr:#x}");
                }
                match offset {
                    reg::ICR_LOW => {
                        self.set(reg::ICR_LOW, value as u32);
                        self.set(reg::ICR_HIGH, (value >> 32) as u32);
                        self.send((value >> 32) as u32, value as u32);
                    }
                    reg::EOI | reg::ESR => {
                        assert_eq!(value, 0, "#GP: non-zero write to {msr:#x}");
                        self.core_write(offset, 0);
                    }
                    reg::SELF_IPI => self.send(self.id, 1 << 18 | 1 << 14 | value as u32 & 0xff),
                    o @ (reg::ID | reg::VERSION | reg::PPR | reg::LDR | reg::TIMER_CURRENT) => {
                        panic!("#GP: write to read-only {o:#x}")
                    }
                    o if (reg::ISR..reg::ESR).contains(&o) => panic!("#GP: write to read-only {o:#x}"),
                    o @ (reg::ICR_HIGH | reg::DFR | reg::APR) => panic!("#GP: x2APIC has no {o:#x}"),
                    o => self.core_write(o, value as u32),
                }
            }
            _ => panic!("#GP: unknown MSR {msr:#x}"),
        }
    }
}

// ---------------------------------------------------------------------------
// I/O APIC

#[derive(Debug)]
pub struct SimIoApic {
    select: Cell<u32>,
    regs: RefCell<[u32; 256]>,
    log: RefCell<Vec<(u8, u32)>>,
    /// Inputs whose high dword was written while they were unmasked.
    live_half_written: RefCell<[bool; 128]>,
}

impl SimIoApic {
    /// `inputs` redirection entries exist, all masked; `version` is
    /// whatever the version register should say.
    pub fn new(version: u32, inputs: u8) -> SimIoApic {
        let mut regs = [0u32; 256];
        regs[1] = version;
        for i in 0..inputs as usize {
            regs[0x10 + 2 * i] = 1 << 16;
        }
        SimIoApic {
            select: Cell::new(0),
            regs: RefCell::new(regs),
            log: RefCell::new(Vec::new()),
            live_half_written: RefCell::new([false; 128]),
        }
    }

    pub fn poke(&self, index: u8, v: u32) {
        self.regs.borrow_mut()[index as usize] = v;
    }

    pub fn log(&self) -> Vec<(u8, u32)> {
        self.log.borrow().clone()
    }

    pub fn clear_log(&self) {
        self.log.borrow_mut().clear();
    }

    pub fn set_remote_irr(&self, input: u8, on: bool) {
        let mut regs = self.regs.borrow_mut();
        let r = &mut regs[0x10 + 2 * input as usize];
        *r = if on { *r | 1 << 14 } else { *r & !(1 << 14) };
    }

    pub fn was_live_half_written(&self, input: u8) -> bool {
        self.live_half_written.borrow()[input as usize]
    }
}

impl Mmio for &SimIoApic {
    fn read32(&self, offset: usize) -> u32 {
        match offset {
            0x00 => self.select.get(),
            0x10 => self.regs.borrow()[self.select.get() as usize],
            _ => panic!("I/O APIC read at {offset:#x}"),
        }
    }

    fn write32(&self, offset: usize, value: u32) {
        match offset {
            0x00 => {
                assert!(value <= 0xff, "IOREGSEL is 8 bits");
                self.select.set(value);
            }
            0x10 => {
                let index = self.select.get() as usize;
                self.log.borrow_mut().push((index as u8, value));
                let mut regs = self.regs.borrow_mut();
                match index {
                    0x00 => regs[0] = value & 0x0f00_0000,
                    0x01 | 0x02 => {}
                    i if i >= 0x10 && i % 2 == 1 => {
                        if regs[i - 1] & (1 << 16) == 0 {
                            self.live_half_written.borrow_mut()[(i - 0x10) / 2] = true;
                        }
                        regs[i] = value & 0xff00_0000;
                    }
                    i if i >= 0x10 => {
                        const READ_ONLY: u32 = 1 << 12 | 1 << 14;
                        regs[i] = regs[i] & READ_ONLY | value & 0x1_afff;
                    }
                    _ => {}
                }
            }
            _ => panic!("I/O APIC write at {offset:#x}"),
        }
    }

    fn read8(&self, _: usize) -> u8 {
        panic!("I/O APIC registers are 32-bit only")
    }

    fn read16(&self, _: usize) -> u16 {
        panic!("I/O APIC registers are 32-bit only")
    }

    fn write8(&self, _: usize, _: u8) {
        panic!("I/O APIC registers are 32-bit only")
    }

    fn write16(&self, _: usize, _: u16) {
        panic!("I/O APIC registers are 32-bit only")
    }
}

// ---------------------------------------------------------------------------
// 8259 PIC

#[derive(Debug, Default)]
struct Chip {
    /// 0 when operational, else the ICW expected next (2, 3 or 4).
    expect: Cell<u8>,
    initialised: Cell<bool>,
    base: Cell<u8>,
    icw3: Cell<u8>,
    icw4: Cell<u8>,
    imr: Cell<u8>,
    isr: Cell<u8>,
    irr: Cell<u8>,
    read_isr: Cell<bool>,
    eois: Cell<u32>,
}

impl Chip {
    fn command(&self, v: u8) {
        if v & 0x10 != 0 {
            assert!(v & 0x01 != 0, "ICW1 without ICW4");
            assert!(v & 0x02 == 0, "ICW1 in single mode");
            self.expect.set(2);
            self.initialised.set(false);
            self.imr.set(0);
            self.isr.set(0);
            self.read_isr.set(false);
            return;
        }
        assert_eq!(self.expect.get(), 0, "operation command word during initialisation");
        if v & 0x08 != 0 {
            if v & 0x02 != 0 {
                self.read_isr.set(v & 0x01 != 0);
            }
        } else if v & 0xe0 == 0x20 {
            self.eois.set(self.eois.get() + 1);
            let isr = self.isr.get();
            self.isr.set(isr & isr.wrapping_sub(1));
        }
    }

    fn data(&self, v: u8) {
        match self.expect.get() {
            2 => {
                self.base.set(v & 0xf8);
                self.expect.set(3);
            }
            3 => {
                self.icw3.set(v);
                self.expect.set(4);
            }
            4 => {
                self.icw4.set(v);
                self.expect.set(0);
                self.initialised.set(true);
            }
            _ => self.imr.set(v),
        }
    }
}

#[derive(Debug, Default)]
pub struct SimPic {
    chips: [Chip; 2],
    log: RefCell<Vec<(u16, u8)>>,
}

impl SimPic {
    pub fn new() -> SimPic {
        SimPic::default()
    }

    pub fn log(&self) -> Vec<(u16, u8)> {
        self.log.borrow().clone()
    }

    pub fn set_masks(&self, m: u16) {
        self.chips[0].imr.set(m as u8);
        self.chips[1].imr.set((m >> 8) as u8);
    }

    /// `(vector base, ICW3, ICW4, initialised)` of master and slave.
    pub fn state(&self) -> [(u8, u8, u8, bool); 2] {
        self.chips.each_ref().map(|c| (c.base.get(), c.icw3.get(), c.icw4.get(), c.initialised.get()))
    }

    /// The CPU acknowledged IRQ `irq`: it is now in service (an IRQ from the
    /// slave also puts the master's cascade input in service).
    pub fn raise(&self, irq: u8) {
        let c = &self.chips[irq as usize / 8];
        c.isr.set(c.isr.get() | 1 << (irq % 8));
        if irq >= 8 {
            self.chips[0].isr.set(self.chips[0].isr.get() | 1 << crate::pic::CASCADE_IRQ);
        }
    }

    pub fn request(&self, irq: u8) {
        let c = &self.chips[irq as usize / 8];
        c.irr.set(c.irr.get() | 1 << (irq % 8));
    }

    pub fn eois(&self) -> (u32, u32) {
        (self.chips[0].eois.get(), self.chips[1].eois.get())
    }
}

impl PortIo for &SimPic {
    fn read8(&self, port: u16) -> u8 {
        let chip = &self.chips[(port >= 0xa0) as usize];
        match port {
            0x20 | 0xa0 => {
                if chip.read_isr.get() {
                    chip.isr.get()
                } else {
                    chip.irr.get()
                }
            }
            0x21 | 0xa1 => {
                assert_eq!(chip.expect.get(), 0, "mask read during initialisation");
                chip.imr.get()
            }
            _ => panic!("read of port {port:#x}"),
        }
    }

    fn write8(&self, port: u16, value: u8) {
        self.log.borrow_mut().push((port, value));
        let chip = &self.chips[(port >= 0xa0) as usize];
        match port {
            0x20 | 0xa0 => chip.command(value),
            0x21 | 0xa1 => chip.data(value),
            0x80 => {}
            _ => panic!("write of port {port:#x}"),
        }
    }
}
