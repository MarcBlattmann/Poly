//! The vDSO's data page (`[vvar]`) and the time arithmetic shared by the
//! kernel, which writes the page, and the vDSO, which reads it in user
//! mode (ADR 0044).
//!
//! The kernel maps one read-only page of [`Vvar`] into every Linux process,
//! right below the vDSO image, and keeps a [`Snapshot`] of its clock in it:
//! how to turn a counter reading (the TSC) into nanoseconds of the
//! monotonic clock ([`Conversion`]), the offset of the wall clock from the
//! monotonic one, the coarse clocks' resolution and the time zone
//! `settimeofday` stored. The vDSO's `__vdso_clock_gettime` reads the
//! counter and the snapshot and computes the time without entering the
//! kernel. When the counter cannot be trusted (a TSC that is not
//! invariant) the snapshot says [`Mode::Syscall`] and the vDSO makes the
//! system call instead.
//!
//! # The seqlock
//!
//! The kernel is the only writer (it serialises its own updates); any
//! number of user threads read. [`Vvar::publish`] makes the sequence
//! number odd, writes the fields, and makes it even again;
//! [`Vvar::try_read`] reads the number, the fields, and the number again,
//! and keeps the fields only if both numbers are the same even value.
//! Every field is an atomic, so a reader racing a writer reads stale or
//! new values but never tears a field, and the fences make sure a reader
//! that saw any new field also sees the changed sequence number (the
//! pattern from H.-J. Boehm, "Can seqlocks get along with programming
//! language memory models?", 2012).
//!
//! # Layout
//!
//! [`Vvar`] is `#[repr(C)]`: the kernel and the vDSO are built from the
//! same source, so the layout is Poly's own, not an ABI. Clock ids are
//! Linux's (`linux/time.h`).
//!
//! The crate is `no_std`, allocates nothing and calls nothing that could
//! need relocations, so the vDSO (a position-independent image nobody
//! relocates) can use it.

#![no_std]

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

pub const NANOS_PER_SEC: u64 = 1_000_000_000;
pub const NANOS_PER_MICRO: u64 = 1_000;

/// The size of the data page.
pub const PAGE_SIZE: usize = 4096;

/// Clock ids (`linux/time.h`).
pub mod clock {
    pub const CLOCK_REALTIME: i32 = 0;
    pub const CLOCK_MONOTONIC: i32 = 1;
    pub const CLOCK_PROCESS_CPUTIME_ID: i32 = 2;
    pub const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
    pub const CLOCK_MONOTONIC_RAW: i32 = 4;
    pub const CLOCK_REALTIME_COARSE: i32 = 5;
    pub const CLOCK_MONOTONIC_COARSE: i32 = 6;
    pub const CLOCK_BOOTTIME: i32 = 7;
    pub const CLOCK_TAI: i32 = 11;
}

/// How the vDSO reads the time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Through the system call: the counter is not usable from user mode
    /// (or the kernel has not published a snapshot yet: a zeroed page).
    #[default]
    Syscall,
    /// From the TSC, with the snapshot's [`Conversion`].
    Tsc,
}

impl Mode {
    const fn to_raw(self) -> u32 {
        match self {
            Mode::Syscall => 0,
            Mode::Tsc => 1,
        }
    }

    const fn from_raw(raw: u32) -> Mode {
        match raw {
            1 => Mode::Tsc,
            _ => Mode::Syscall,
        }
    }
}

/// A counter reading to nanoseconds of the monotonic clock:
/// `offset_ns + ((counter - origin) * mult) >> shift`, one 64 x 64 -> 128
/// bit multiply and a shift (the kernel's clock uses the same formula, so
/// the two agree to the nanosecond).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conversion {
    /// The counter reading at which the clock read `offset_ns`.
    pub origin: u64,
    pub mult: u64,
    /// At most 63.
    pub shift: u32,
    pub offset_ns: u64,
}

impl Conversion {
    /// Nanoseconds since boot at counter reading `counter`. A reading
    /// before `origin` (which one CPU never produces) counts as `origin`;
    /// the result saturates at `u64::MAX`.
    pub const fn ns(&self, counter: u64) -> u64 {
        let ticks = counter.wrapping_sub(self.origin);
        if ticks > i64::MAX as u64 {
            return self.offset_ns;
        }
        let scaled = (ticks as u128 * self.mult as u128) >> (self.shift & 63);
        let scaled = if scaled > u64::MAX as u128 { u64::MAX } else { scaled as u64 };
        self.offset_ns.saturating_add(scaled)
    }
}

/// `struct timezone` as `settimeofday` stored it (`linux/time.h`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timezone {
    pub minuteswest: i32,
    pub dsttime: i32,
}

/// What the kernel publishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: Mode,
    /// Valid in [`Mode::Tsc`].
    pub conversion: Conversion,
    /// `CLOCK_REALTIME - CLOCK_MONOTONIC` in nanoseconds (never negative:
    /// clock_settime(2) refuses a wall clock below the monotonic clock).
    pub realtime_offset_ns: u64,
    /// The resolution of the coarse clocks (the kernel's tick).
    pub coarse_resolution_ns: u64,
    pub timezone: Timezone,
}

/// The clocks the vDSO answers itself; every other id goes to the system
/// call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VClock {
    /// Also `CLOCK_TAI` (Poly's TAI offset is 0, as the system call has it).
    Realtime,
    RealtimeCoarse,
    /// Also `CLOCK_MONOTONIC_RAW` (Poly never slews its clock) and
    /// `CLOCK_BOOTTIME` (it never suspends).
    Monotonic,
    MonotonicCoarse,
}

impl VClock {
    pub const fn from_id(id: i32) -> Option<VClock> {
        match id {
            clock::CLOCK_REALTIME | clock::CLOCK_TAI => Some(VClock::Realtime),
            clock::CLOCK_REALTIME_COARSE => Some(VClock::RealtimeCoarse),
            clock::CLOCK_MONOTONIC | clock::CLOCK_MONOTONIC_RAW | clock::CLOCK_BOOTTIME => Some(VClock::Monotonic),
            clock::CLOCK_MONOTONIC_COARSE => Some(VClock::MonotonicCoarse),
            _ => None,
        }
    }

    pub const fn is_coarse(self) -> bool {
        matches!(self, VClock::RealtimeCoarse | VClock::MonotonicCoarse)
    }
}

impl Snapshot {
    /// The monotonic clock at counter reading `counter`; `None` unless the
    /// snapshot is in [`Mode::Tsc`].
    pub const fn monotonic_ns(&self, counter: u64) -> Option<u64> {
        match self.mode {
            Mode::Tsc => Some(self.conversion.ns(counter)),
            Mode::Syscall => None,
        }
    }

    /// `clock` at counter reading `counter`, in nanoseconds (since boot or
    /// since the epoch); `None` in [`Mode::Syscall`]. The coarse clocks
    /// read the same as the precise ones (allowed: they are only promised
    /// to be no more precise than their resolution).
    pub const fn clock_ns(&self, clock: VClock, counter: u64) -> Option<u64> {
        let Some(mono) = self.monotonic_ns(counter) else { return None };
        Some(match clock {
            VClock::Monotonic | VClock::MonotonicCoarse => mono,
            VClock::Realtime | VClock::RealtimeCoarse => mono.saturating_add(self.realtime_offset_ns),
        })
    }

    /// The resolution clock_getres(2) reports: 1 ns for the precise clocks
    /// (Poly's timers are not tied to the tick), the tick for the coarse
    /// ones.
    pub const fn resolution_ns(&self, clock: VClock) -> u64 {
        if clock.is_coarse() && self.coarse_resolution_ns != 0 { self.coarse_resolution_ns } else { 1 }
    }
}

/// Nanoseconds as `(tv_sec, tv_nsec)`.
pub const fn timespec(ns: u64) -> (i64, i64) {
    ((ns / NANOS_PER_SEC) as i64, (ns % NANOS_PER_SEC) as i64)
}

/// Nanoseconds as `(tv_sec, tv_usec)`, the microseconds truncated.
pub const fn timeval(ns: u64) -> (i64, i64) {
    ((ns / NANOS_PER_SEC) as i64, (ns % NANOS_PER_SEC / NANOS_PER_MICRO) as i64)
}

/// How the vDSO's `getcpu` learns the CPU it runs on: RDPID or RDTSCP read
/// IA32_TSC_AUX, which the kernel sets to each CPU's number (SDM Vol. 2B,
/// `RDPID`, `RDTSCP`); without either, the system call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CpuNumber {
    Syscall = 0,
    Rdtscp = 1,
    Rdpid = 2,
}

/// The data page: the seqlock and the fields of a [`Snapshot`]. A zeroed
/// page is valid and reads as [`Mode::Syscall`].
#[derive(Debug, Default)]
#[repr(C)]
pub struct Vvar {
    /// Odd while the kernel writes.
    seq: AtomicU32,
    mode: AtomicU32,
    shift: AtomicU32,
    tz_minuteswest: AtomicU32,
    tz_dsttime: AtomicU32,
    /// How `getcpu` finds the CPU ([`CpuNumber`]); set once, outside the
    /// seqlock.
    cpu_number: AtomicU32,
    origin: AtomicU64,
    mult: AtomicU64,
    offset_ns: AtomicU64,
    realtime_offset_ns: AtomicU64,
    coarse_resolution_ns: AtomicU64,
}

const _: () = assert!(size_of::<Vvar>() <= PAGE_SIZE);

impl Vvar {
    pub const fn new() -> Vvar {
        Vvar {
            seq: AtomicU32::new(0),
            mode: AtomicU32::new(0),
            shift: AtomicU32::new(0),
            tz_minuteswest: AtomicU32::new(0),
            tz_dsttime: AtomicU32::new(0),
            cpu_number: AtomicU32::new(0),
            origin: AtomicU64::new(0),
            mult: AtomicU64::new(0),
            offset_ns: AtomicU64::new(0),
            realtime_offset_ns: AtomicU64::new(0),
            coarse_resolution_ns: AtomicU64::new(0),
        }
    }

    /// Replace the snapshot. One writer at a time: the caller serialises
    /// calls (readers never block it, they retry).
    pub fn publish(&self, s: &Snapshot) {
        let seq = self.seq.load(Ordering::Relaxed);
        let writing = if seq & 1 == 0 { seq.wrapping_add(1) } else { seq };
        self.seq.store(writing, Ordering::Relaxed);
        // The odd number is visible before any field changes.
        fence(Ordering::Release);
        self.mode.store(s.mode.to_raw(), Ordering::Relaxed);
        self.shift.store(s.conversion.shift, Ordering::Relaxed);
        self.tz_minuteswest.store(s.timezone.minuteswest as u32, Ordering::Relaxed);
        self.tz_dsttime.store(s.timezone.dsttime as u32, Ordering::Relaxed);
        self.origin.store(s.conversion.origin, Ordering::Relaxed);
        self.mult.store(s.conversion.mult, Ordering::Relaxed);
        self.offset_ns.store(s.conversion.offset_ns, Ordering::Relaxed);
        self.realtime_offset_ns.store(s.realtime_offset_ns, Ordering::Relaxed);
        self.coarse_resolution_ns.store(s.coarse_resolution_ns, Ordering::Relaxed);
        // Every field is visible before the even number.
        self.seq.store(writing.wrapping_add(1), Ordering::Release);
    }

    /// One attempt at a consistent read: `None` if the kernel was writing
    /// meanwhile (try again).
    pub fn try_read(&self) -> Option<Snapshot> {
        let before = self.seq.load(Ordering::Acquire);
        if before & 1 != 0 {
            return None;
        }
        let s = Snapshot {
            mode: Mode::from_raw(self.mode.load(Ordering::Relaxed)),
            conversion: Conversion {
                origin: self.origin.load(Ordering::Relaxed),
                mult: self.mult.load(Ordering::Relaxed),
                shift: self.shift.load(Ordering::Relaxed),
                offset_ns: self.offset_ns.load(Ordering::Relaxed),
            },
            realtime_offset_ns: self.realtime_offset_ns.load(Ordering::Relaxed),
            coarse_resolution_ns: self.coarse_resolution_ns.load(Ordering::Relaxed),
            timezone: Timezone {
                minuteswest: self.tz_minuteswest.load(Ordering::Relaxed) as i32,
                dsttime: self.tz_dsttime.load(Ordering::Relaxed) as i32,
            },
        };
        // The field loads complete before the second look at the number.
        fence(Ordering::Acquire);
        let after = self.seq.load(Ordering::Relaxed);
        if before == after { Some(s) } else { None }
    }

    /// A consistent snapshot, retrying while the kernel writes.
    pub fn read(&self) -> Snapshot {
        loop {
            if let Some(s) = self.try_read() {
                return s;
            }
            core::hint::spin_loop();
        }
    }

    /// Tell `getcpu` how to find the CPU it runs on.
    pub fn set_cpu_number(&self, how: CpuNumber) {
        self.cpu_number.store(how as u32, Ordering::Release);
    }

    /// How `getcpu` finds the CPU it runs on ([`CpuNumber::Syscall`] in a
    /// zeroed page).
    pub fn cpu_number(&self) -> CpuNumber {
        match self.cpu_number.load(Ordering::Acquire) {
            1 => CpuNumber::Rdtscp,
            2 => CpuNumber::Rdpid,
            _ => CpuNumber::Syscall,
        }
    }

    /// The sequence number (even when no write is in progress).
    pub fn sequence(&self) -> u32 {
        self.seq.load(Ordering::Acquire)
    }

    /// Start a write without finishing it, so tests can see readers back
    /// off. [`Vvar::publish`] finishes it.
    #[cfg(test)]
    fn begin_write(&self) {
        self.seq.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
