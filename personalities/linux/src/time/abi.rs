//! Time structures and constants of the x86-64 Linux ABI, from the UAPI
//! headers cited at each definition. `struct timespec` is
//! [`crate::abi::Timespec`].

use crate::abi::{Timespec, le_u64};

pub const NANOS_PER_SEC: u64 = 1_000_000_000;
pub const NANOS_PER_MICRO: u64 = 1_000;

/// `TIMER_ABSTIME` (`linux/time.h`): the time is absolute, not relative.
pub const TIMER_ABSTIME: u32 = 0x01;

/// Interval timers (`linux/time.h`).
pub const ITIMER_REAL: i32 = 0;
pub const ITIMER_VIRTUAL: i32 = 1;
pub const ITIMER_PROF: i32 = 2;

/// `sigev_notify` values (`asm-generic/siginfo.h`).
pub const SIGEV_SIGNAL: i32 = 0;
pub const SIGEV_NONE: i32 = 1;
pub const SIGEV_THREAD: i32 = 2;
pub const SIGEV_THREAD_ID: i32 = 4;

/// `si_code` of a POSIX timer's signal (`asm-generic/siginfo.h`).
pub const SI_TIMER: i32 = -2;

/// getrusage(2)'s `who` (`linux/resource.h`).
pub const RUSAGE_SELF: i32 = 0;
pub const RUSAGE_CHILDREN: i32 = -1;
pub const RUSAGE_THREAD: i32 = 1;

/// The unit of `clock_t` in times(2): `sysconf(_SC_CLK_TCK)`, which the
/// C libraries take from `AT_CLKTCK` (100 in Poly's auxiliary vector).
pub const USER_HZ: u64 = 100;

/// The largest overrun count timer_getoverrun(2) reports
/// (`DELAYTIMER_MAX`: "the maximum value of a signed int").
pub const DELAYTIMER_MAX: u32 = i32::MAX as u32;

/// A timespec as nanoseconds, saturating; only for valid (non-negative)
/// values.
pub const fn timespec_ns(t: Timespec) -> u64 {
    if t.sec < 0 || t.nsec < 0 {
        return 0;
    }
    (t.sec as u64).saturating_mul(NANOS_PER_SEC).saturating_add(t.nsec as u64)
}

/// `struct __kernel_old_timeval` (`linux/time_types.h`): seconds and
/// microseconds, 64 bits each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timeval {
    pub sec: i64,
    pub usec: i64,
}

impl Timeval {
    pub const SIZE: usize = 16;

    /// Nanoseconds as a timeval, the microseconds rounded down.
    pub const fn from_nanos(ns: u64) -> Timeval {
        Timeval { sec: (ns / NANOS_PER_SEC) as i64, usec: (ns % NANOS_PER_SEC / NANOS_PER_MICRO) as i64 }
    }

    /// Nanoseconds as a timeval, the microseconds rounded up (a time left
    /// that is not quite zero does not read as zero).
    pub const fn from_nanos_up(ns: u64) -> Timeval {
        Timeval::from_nanos(ns.saturating_add(NANOS_PER_MICRO - 1))
    }

    /// `sec` not negative and `usec` in `0..1_000_000`.
    pub const fn is_valid(self) -> bool {
        self.sec >= 0 && self.usec >= 0 && self.usec < 1_000_000
    }

    /// As nanoseconds, saturating (valid values only).
    pub const fn to_nanos(self) -> u64 {
        if !self.is_valid() {
            return 0;
        }
        (self.sec as u64).saturating_mul(NANOS_PER_SEC).saturating_add(self.usec as u64 * NANOS_PER_MICRO)
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..8].copy_from_slice(&self.sec.to_le_bytes());
        b[8..16].copy_from_slice(&self.usec.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Timeval {
        Timeval { sec: le_u64(&b[0..8]) as i64, usec: le_u64(&b[8..16]) as i64 }
    }
}

/// `struct __kernel_old_itimerval` (`linux/time_types.h`): the interval,
/// then the value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Itimerval {
    pub interval: Timeval,
    pub value: Timeval,
}

impl Itimerval {
    pub const SIZE: usize = 32;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..16].copy_from_slice(&self.interval.to_bytes());
        b[16..32].copy_from_slice(&self.value.to_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Itimerval {
        let half = |o: usize| Timeval::from_bytes(b[o..o + 16].try_into().expect("16 bytes"));
        Itimerval { interval: half(0), value: half(16) }
    }
}

/// `struct __kernel_itimerspec` (`linux/time_types.h`): the interval,
/// then the value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Itimerspec {
    pub interval: Timespec,
    pub value: Timespec,
}

impl Itimerspec {
    pub const SIZE: usize = 32;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..16].copy_from_slice(&self.interval.to_bytes());
        b[16..32].copy_from_slice(&self.value.to_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Itimerspec {
        let half = |o: usize| Timespec::from_bytes(b[o..o + 16].try_into().expect("16 bytes"));
        Itimerspec { interval: half(0), value: half(16) }
    }
}

/// `struct sigevent` (`asm-generic/siginfo.h`): `sigev_value` (8 bytes) at
/// 0, `sigev_signo` at 8, `sigev_notify` at 12, then the union whose
/// `_tid` (`sigev_notify_thread_id`) is at 16; 64 bytes
/// (`SIGEV_MAX_SIZE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sigevent {
    pub value: u64,
    pub signo: i32,
    pub notify: i32,
    pub tid: i32,
}

impl Sigevent {
    pub const SIZE: usize = 64;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Sigevent {
        let int = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Sigevent { value: le_u64(&b[0..8]), signo: int(8), notify: int(12), tid: int(16) }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..8].copy_from_slice(&self.value.to_le_bytes());
        b[8..12].copy_from_slice(&self.signo.to_le_bytes());
        b[12..16].copy_from_slice(&self.notify.to_le_bytes());
        b[16..20].copy_from_slice(&self.tid.to_le_bytes());
        b
    }
}

/// `struct tms` (`linux/times.h`): four `clock_t` (64-bit `long`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tms {
    pub utime: i64,
    pub stime: i64,
    pub cutime: i64,
    pub cstime: i64,
}

impl Tms {
    pub const SIZE: usize = 32;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        for (i, v) in [self.utime, self.stime, self.cutime, self.cstime].into_iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
}

/// `struct rusage` (`linux/resource.h`): `ru_utime` and `ru_stime` (two
/// timevals), then fourteen `long`s from `ru_maxrss` to `ru_nivcsw`; 144
/// bytes. Poly fills in the times, `ru_maxrss` (KiB), `ru_minflt` and
/// `ru_majflt`; the other counters stay 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rusage {
    pub utime: Timeval,
    pub stime: Timeval,
    pub maxrss: i64,
    pub minflt: i64,
    pub majflt: i64,
}

impl Rusage {
    pub const SIZE: usize = 144;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..16].copy_from_slice(&self.utime.to_bytes());
        b[16..32].copy_from_slice(&self.stime.to_bytes());
        b[32..40].copy_from_slice(&self.maxrss.to_le_bytes());
        b[64..72].copy_from_slice(&self.minflt.to_le_bytes());
        b[72..80].copy_from_slice(&self.majflt.to_le_bytes());
        b
    }
}

/// `struct timezone` (`linux/time.h`): two `int`s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timezone {
    pub minuteswest: i32,
    pub dsttime: i32,
}

impl Timezone {
    pub const SIZE: usize = 8;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0; Self::SIZE];
        b[0..4].copy_from_slice(&self.minuteswest.to_le_bytes());
        b[4..8].copy_from_slice(&self.dsttime.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> Timezone {
        Timezone {
            minuteswest: i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            dsttime: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_round_trip() {
        let tv = Timeval { sec: 7, usec: 999_999 };
        assert_eq!(Timeval::from_bytes(&tv.to_bytes()), tv);
        let it = Itimerval { interval: Timeval { sec: 1, usec: 2 }, value: Timeval { sec: 3, usec: 4 } };
        let b = it.to_bytes();
        assert_eq!(&b[0..8], &1i64.to_le_bytes(), "the interval comes first");
        assert_eq!(Itimerval::from_bytes(&b), it);
        let is = Itimerspec { interval: Timespec { sec: 5, nsec: 6 }, value: Timespec { sec: 7, nsec: 8 } };
        let b = is.to_bytes();
        assert_eq!(&b[16..24], &7i64.to_le_bytes(), "the value comes second");
        assert_eq!(Itimerspec::from_bytes(&b), is);
        let ev = Sigevent { value: 0x1122_3344_5566_7788, signo: 34, notify: SIGEV_THREAD_ID, tid: 1234 };
        let b = ev.to_bytes();
        assert_eq!((b[8], b[12], b[16]), (34, 4, 0xd2));
        assert_eq!(Sigevent::from_bytes(&b), ev);
        let tz = Timezone { minuteswest: -120, dsttime: 1 };
        assert_eq!(Timezone::from_bytes(&tz.to_bytes()), tz);
        let tms = Tms { utime: 1, stime: 2, cutime: 3, cstime: 4 }.to_bytes();
        assert_eq!(&tms[24..32], &4i64.to_le_bytes());
        let ru = Rusage { utime: Timeval { sec: 1, usec: 5 }, stime: Timeval { sec: 2, usec: 6 }, ..Rusage::default() }
            .to_bytes();
        assert_eq!((&ru[0..8], &ru[24..32]), (&1i64.to_le_bytes()[..], &6i64.to_le_bytes()[..]));
        assert!(ru[32..].iter().all(|&x| x == 0));
    }

    #[test]
    fn conversions() {
        assert_eq!(Timeval::from_nanos(1_234_567_891), Timeval { sec: 1, usec: 234_567 });
        assert_eq!(Timeval::from_nanos_up(1_234_567_891), Timeval { sec: 1, usec: 234_568 });
        assert_eq!(Timeval::from_nanos_up(1), Timeval { sec: 0, usec: 1 });
        assert_eq!(Timeval::from_nanos_up(0), Timeval::default());
        assert_eq!(Timeval { sec: 2, usec: 500_000 }.to_nanos(), 2_500_000_000);
        assert!(!Timeval { sec: 0, usec: 1_000_000 }.is_valid());
        assert!(!Timeval { sec: -1, usec: 0 }.is_valid());
        assert!(!Timeval { sec: 0, usec: -1 }.is_valid());
        assert_eq!(Timeval { sec: i64::MAX, usec: 0 }.to_nanos(), u64::MAX);
        assert_eq!(timespec_ns(Timespec { sec: 3, nsec: 4 }), 3_000_000_004);
        assert_eq!(timespec_ns(Timespec { sec: -3, nsec: 4 }), 0);
        assert_eq!(timespec_ns(Timespec { sec: i64::MAX, nsec: 4 }), u64::MAX);
    }
}
