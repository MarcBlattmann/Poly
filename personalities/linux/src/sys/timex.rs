//! The clock discipline: adjtimex(2) and clock_adjtime(2).
//!
//! Linux keeps a model of the system clock for a time daemon (NTP) to
//! steer: a frequency correction, a phase offset, error estimates and a
//! status word. Poly runs no daemon and does not slew its clock, so the
//! state ([`Ntp`], kept system-wide by the kernel:
//! [`LinuxKernel::ntp`]) is bookkeeping: what is set is reported back,
//! the tick is checked but does not change the clock's rate, and
//! `ADJ_SETOFFSET` steps `CLOCK_REALTIME`. Until something clears it the
//! status has `STA_UNSYNC` and the calls return `TIME_ERROR`, as on a
//! system whose clock was set and not yet synchronised (the sheets allow
//! that fixed state). Setting the clock (settimeofday(2),
//! clock_settime(2)) marks it unsynchronised again ([`clock_was_set`]).
//!
//! `struct timex` and every constant here are from `linux/timex.h`.

use crate::abi::{Timespec, clock as id};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::kernel::{Clock, LinuxKernel, UserPtr};
use crate::table::{Handler, nr};

/// The calls of this module, merged into the table (`table::GROUPS`).
pub const CALLS: &[(u64, Handler)] = &[(nr::ADJTIMEX, adjtimex), (nr::CLOCK_ADJTIME, clock_adjtime)];

// `modes` bits.
pub const ADJ_OFFSET: u32 = 0x0001;
pub const ADJ_FREQUENCY: u32 = 0x0002;
pub const ADJ_MAXERROR: u32 = 0x0004;
pub const ADJ_ESTERROR: u32 = 0x0008;
pub const ADJ_STATUS: u32 = 0x0010;
pub const ADJ_TIMECONST: u32 = 0x0020;
pub const ADJ_TAI: u32 = 0x0080;
pub const ADJ_SETOFFSET: u32 = 0x0100;
pub const ADJ_MICRO: u32 = 0x1000;
pub const ADJ_NANO: u32 = 0x2000;
pub const ADJ_TICK: u32 = 0x4000;
/// The bit of the old adjtime(3) interface (`ADJ_OFFSET_SINGLESHOT`
/// without `ADJ_OFFSET`).
pub const ADJ_ADJTIME: u32 = 0x8000;
pub const ADJ_OFFSET_SINGLESHOT: u32 = 0x8001;
pub const ADJ_OFFSET_SS_READ: u32 = 0xa001;

// `status` bits.
pub const STA_PLL: i32 = 0x0001;
pub const STA_INS: i32 = 0x0010;
pub const STA_DEL: i32 = 0x0020;
pub const STA_UNSYNC: i32 = 0x0040;
pub const STA_PPSSIGNAL: i32 = 0x0100;
pub const STA_PPSJITTER: i32 = 0x0200;
pub const STA_PPSWANDER: i32 = 0x0400;
pub const STA_PPSERROR: i32 = 0x0800;
pub const STA_CLOCKERR: i32 = 0x1000;
pub const STA_NANO: i32 = 0x2000;
pub const STA_MODE: i32 = 0x4000;
pub const STA_CLK: i32 = 0x8000;
/// The read-only status bits.
pub const STA_RONLY: i32 =
    STA_PPSSIGNAL | STA_PPSJITTER | STA_PPSWANDER | STA_PPSERROR | STA_CLOCKERR | STA_NANO | STA_MODE | STA_CLK;

// Clock states (the return values).
pub const TIME_OK: u64 = 0;
pub const TIME_INS: u64 = 1;
pub const TIME_DEL: u64 = 2;
pub const TIME_ERROR: u64 = 5;

/// `MAXFREQ_SCALED`'s value, 500 ppm scaled by 2^16: the frequency
/// tolerance reported, and the bound of `freq`.
pub const MAXFREQ_SCALED: i64 = 500 << 16;
/// Clock ticks per second as user space counts them (`USER_HZ`).
pub const USER_HZ: i64 = 100;
/// The nominal tick: microseconds per `USER_HZ` tick.
pub const NOMINAL_TICK: i64 = 1_000_000 / USER_HZ;
/// `ADJ_TICK` accepts ticks within 10% of [`NOMINAL_TICK`].
pub const MIN_TICK: i64 = NOMINAL_TICK * 9 / 10;
pub const MAX_TICK: i64 = NOMINAL_TICK * 11 / 10;
/// The error estimates Poly starts with (its choice: 16 s, an
/// unsynchronised clock's).
pub const INITIAL_ERROR: i64 = 16_000_000;

/// The size of `struct timex` on x86-64.
pub const TIMEX_SIZE: usize = 208;

/// The system's clock discipline, as adjtimex(2) reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ntp {
    /// Phase offset in nanoseconds.
    pub offset_ns: i64,
    /// Frequency offset, ppm scaled by 2^16.
    pub freq: i64,
    pub maxerror: i64,
    pub esterror: i64,
    pub status: i32,
    pub constant: i64,
    /// Microseconds per tick.
    pub tick: i64,
    /// TAI - UTC in seconds.
    pub tai: i32,
}

impl Default for Ntp {
    fn default() -> Self {
        Ntp {
            offset_ns: 0,
            freq: 0,
            maxerror: INITIAL_ERROR,
            esterror: INITIAL_ERROR,
            status: STA_UNSYNC,
            constant: 2,
            tick: NOMINAL_TICK,
            tai: 0,
        }
    }
}

impl Ntp {
    /// The clock state the calls return.
    pub fn state(&self) -> u64 {
        if self.status & STA_UNSYNC != 0 {
            TIME_ERROR
        } else if self.status & STA_INS != 0 {
            TIME_INS
        } else if self.status & STA_DEL != 0 {
            TIME_DEL
        } else {
            TIME_OK
        }
    }
}

/// `struct timex`: the fields Poly reads and writes, at their offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timex {
    pub raw: [u8; TIMEX_SIZE],
}

impl Timex {
    fn i64_at(&self, at: usize) -> i64 {
        i64::from_le_bytes(self.raw[at..at + 8].try_into().expect("8 bytes"))
    }

    fn set_i64(&mut self, at: usize, v: i64) {
        self.raw[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    fn i32_at(&self, at: usize) -> i32 {
        i32::from_le_bytes(self.raw[at..at + 4].try_into().expect("4 bytes"))
    }

    fn set_i32(&mut self, at: usize, v: i32) {
        self.raw[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn modes(&self) -> u32 {
        self.i32_at(0) as u32
    }
    pub fn offset(&self) -> i64 {
        self.i64_at(8)
    }
    pub fn freq(&self) -> i64 {
        self.i64_at(16)
    }
    pub fn maxerror(&self) -> i64 {
        self.i64_at(24)
    }
    pub fn esterror(&self) -> i64 {
        self.i64_at(32)
    }
    pub fn status(&self) -> i32 {
        self.i32_at(40)
    }
    pub fn constant(&self) -> i64 {
        self.i64_at(48)
    }
    /// `time`: seconds and the sub-second field (micro- or nanoseconds).
    pub fn time(&self) -> (i64, i64) {
        (self.i64_at(72), self.i64_at(80))
    }
    pub fn tick(&self) -> i64 {
        self.i64_at(88)
    }

    /// Fill in the state the call reports: `ntp` and the current time
    /// (`now`), `modes` and the padding left as given.
    pub fn report(&mut self, ntp: &Ntp, now: Timespec) {
        let nano = ntp.status & STA_NANO != 0;
        self.set_i64(8, if nano { ntp.offset_ns } else { ntp.offset_ns / 1000 });
        self.set_i64(16, ntp.freq);
        self.set_i64(24, ntp.maxerror);
        self.set_i64(32, ntp.esterror);
        self.set_i32(40, ntp.status);
        self.set_i64(48, ntp.constant);
        self.set_i64(56, 1); // precision: 1 microsecond
        self.set_i64(64, MAXFREQ_SCALED); // tolerance
        self.set_i64(72, now.sec);
        self.set_i64(80, if nano { now.nsec } else { now.nsec / 1000 });
        self.set_i64(88, ntp.tick);
        for at in [96, 104, 120, 128, 136, 144, 152] {
            self.set_i64(at, 0); // ppsfreq, jitter, stabil, jitcnt, calcnt, errcnt, stbcnt
        }
        self.set_i32(112, 0); // shift
        self.set_i32(160, ntp.tai);
    }
}

/// What a validated adjtimex call changes ([`check`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    /// The new state.
    pub ntp: Ntp,
    /// `ADJ_SETOFFSET`: step `CLOCK_REALTIME` by this many nanoseconds.
    pub step_ns: Option<i128>,
}

/// adjtimex(2)'s checks and the state they lead to, in the sheet's
/// order: `ADJ_ADJTIME` without `ADJ_OFFSET` (`EINVAL`, for everybody),
/// the privilege (`EPERM` for any `modes` but 0 and
/// `ADJ_OFFSET_SS_READ` without `CAP_SYS_TIME`), the values (`ADJ_TICK`
/// within 10% of the nominal tick, `ADJ_SETOFFSET`'s sub-second field
/// within a second: `EINVAL`). Bits without a meaning are ignored.
pub fn check(tx: &Timex, ntp: &Ntp, privileged: bool) -> Result<Change, Errno> {
    let modes = tx.modes();
    let mut ntp = *ntp;
    if modes & ADJ_ADJTIME != 0 {
        // The old adjtime(3): a single-shot offset, or only a read.
        if modes & ADJ_OFFSET == 0 {
            return Err(Errno::EINVAL);
        }
        if modes != ADJ_OFFSET_SS_READ && !privileged {
            return Err(Errno::EPERM);
        }
        // Poly does not slew the clock: the offset is taken and done.
        return Ok(Change { ntp, step_ns: None });
    }
    if modes != 0 && !privileged {
        return Err(Errno::EPERM);
    }
    if modes & ADJ_TICK != 0 && !(MIN_TICK..=MAX_TICK).contains(&tx.tick()) {
        return Err(Errno::EINVAL);
    }
    let mut step_ns = None;
    if modes & ADJ_SETOFFSET != 0 {
        let (sec, frac) = tx.time();
        let per_sec = if modes & ADJ_NANO != 0 { 1_000_000_000 } else { 1_000_000 };
        if !(0..per_sec).contains(&frac) {
            return Err(Errno::EINVAL);
        }
        let frac_ns = if modes & ADJ_NANO != 0 { frac } else { frac * 1000 };
        step_ns = Some(i128::from(sec) * 1_000_000_000 + i128::from(frac_ns));
    }
    if modes & ADJ_MICRO != 0 {
        ntp.status &= !STA_NANO;
    }
    if modes & ADJ_NANO != 0 {
        ntp.status |= STA_NANO;
    }
    if modes & ADJ_STATUS != 0 {
        ntp.status = (ntp.status & STA_RONLY) | (tx.status() & !STA_RONLY);
    }
    if modes & ADJ_OFFSET != 0 {
        let nano = ntp.status & STA_NANO != 0;
        ntp.offset_ns = if nano { tx.offset() } else { tx.offset().saturating_mul(1000) };
    }
    if modes & ADJ_FREQUENCY != 0 {
        ntp.freq = tx.freq().clamp(-MAXFREQ_SCALED, MAXFREQ_SCALED);
    }
    if modes & ADJ_MAXERROR != 0 {
        ntp.maxerror = tx.maxerror();
    }
    if modes & ADJ_ESTERROR != 0 {
        ntp.esterror = tx.esterror();
    }
    if modes & ADJ_TIMECONST != 0 {
        ntp.constant = tx.constant();
    }
    if modes & ADJ_TAI != 0 {
        // adjtimex(2): "ADJ_TAI: Set TAI offset from buf.constant".
        ntp.tai = tx.constant() as i32;
    }
    if modes & ADJ_TICK != 0 {
        ntp.tick = tx.tick();
    }
    Ok(Change { ntp, step_ns })
}

/// `CAP_SYS_TIME` in the initial user namespace (the clock is the
/// machine's).
fn may_set_time(k: &mut dyn LinuxKernel) -> bool {
    crate::cred::current(k).capable_initial(crate::cred::cap::SYS_TIME)
}

/// The clock was set: the discipline is no longer synchronised.
pub fn clock_was_set(k: &mut dyn LinuxKernel) {
    let mut ntp = k.ntp();
    ntp.status |= STA_UNSYNC;
    k.set_ntp(&ntp);
}

/// adjtimex on `CLOCK_REALTIME` for the struct read from `buf`.
fn adjust(k: &mut dyn LinuxKernel, mut tx: Timex, buf: UserPtr) -> SysResult {
    let privileged = may_set_time(k);
    let change = check(&tx, &k.ntp(), privileged)?;
    if let Some(step) = change.step_ns
        && step != 0
    {
        let now = k.clock(Clock::Realtime)?;
        let ns = i128::from(now.sec) * 1_000_000_000 + i128::from(now.nsec) + step;
        if ns < 0 || ns > i128::from(i64::MAX) {
            return Err(Errno::EINVAL);
        }
        k.set_realtime(Timespec::from_nanos(ns as u64))?;
    }
    if change.ntp != k.ntp() {
        k.set_ntp(&change.ntp);
    }
    let now = k.clock(Clock::Realtime)?;
    tx.report(&change.ntp, now);
    k.copy_to_user(buf, &tx.raw)?;
    Ok(change.ntp.state())
}

fn read_timex(k: &mut dyn LinuxKernel, buf: UserPtr) -> Result<Timex, Errno> {
    let mut tx = Timex { raw: [0; TIMEX_SIZE] };
    k.copy_from_user(buf, &mut tx.raw)?;
    Ok(tx)
}

/// adjtimex(buf): read and adjust the clock discipline ([`check`]);
/// returns the clock state. `EFAULT` if `buf` cannot be read or written.
pub fn adjtimex(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tx = read_timex(k, a.ptr(0))?;
    adjust(k, tx, a.ptr(0))
}

/// clock_adjtime(clockid, buf): adjtimex(2) for a clock. The struct is
/// read first (`EFAULT`), then the clock: `CLOCK_REALTIME` is adjtimex(2);
/// every other clock, a CPU-time clock too, cannot be adjusted
/// (`EOPNOTSUPP`, before any permission check); an unknown id is `EINVAL`.
pub fn clock_adjtime(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let tx = read_timex(k, a.ptr(1))?;
    let clockid = a.int(0);
    match clockid {
        id::CLOCK_REALTIME => adjust(k, tx, a.ptr(1)),
        id::CLOCK_MONOTONIC
        | id::CLOCK_PROCESS_CPUTIME_ID
        | id::CLOCK_THREAD_CPUTIME_ID
        | id::CLOCK_MONOTONIC_RAW
        | id::CLOCK_REALTIME_COARSE
        | id::CLOCK_MONOTONIC_COARSE
        | id::CLOCK_BOOTTIME
        | id::CLOCK_REALTIME_ALARM
        | id::CLOCK_BOOTTIME_ALARM
        | id::CLOCK_TAI => Err(Errno::EOPNOTSUPP),
        _ => match crate::time::cpuclock::decode(k, clockid) {
            Some(Ok(_)) => Err(Errno::EOPNOTSUPP),
            Some(Err(e)) => Err(e),
            None => Err(Errno::EINVAL),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timex(modes: u32) -> Timex {
        let mut tx = Timex { raw: [0; TIMEX_SIZE] };
        tx.raw[0..4].copy_from_slice(&modes.to_le_bytes());
        tx
    }

    #[test]
    fn reads_need_no_privilege() {
        let ntp = Ntp::default();
        assert_eq!(check(&timex(0), &ntp, false).map(|c| c.ntp), Ok(ntp));
        assert_eq!(check(&timex(ADJ_OFFSET_SS_READ), &ntp, false).map(|c| c.ntp), Ok(ntp));
        assert_eq!(ntp.state(), TIME_ERROR);
        for modes in [ADJ_TICK, ADJ_STATUS, 0x10000, ADJ_OFFSET_SINGLESHOT, ADJ_NANO] {
            assert_eq!(check(&timex(modes), &ntp, false), Err(Errno::EPERM), "{modes:#x}");
        }
        for modes in [0x8000, 0xa000] {
            assert_eq!(check(&timex(modes), &ntp, false), Err(Errno::EINVAL), "{modes:#x}");
            assert_eq!(check(&timex(modes), &ntp, true), Err(Errno::EINVAL), "{modes:#x}");
        }
    }

    #[test]
    fn values_are_checked_before_anything_changes() {
        let ntp = Ntp::default();
        for tick in [0, 1, 8999, 11001, 20000, 100000] {
            let mut tx = timex(ADJ_TICK);
            tx.set_i64(88, tick);
            assert_eq!(check(&tx, &ntp, true), Err(Errno::EINVAL), "tick {tick}");
        }
        let mut tx = timex(ADJ_TICK);
        tx.set_i64(88, 10_000);
        assert_eq!(check(&tx, &ntp, true).unwrap().ntp.tick, 10_000);
        for (modes, frac) in [(ADJ_SETOFFSET, 1_000_000), (ADJ_SETOFFSET, -1), (ADJ_SETOFFSET | ADJ_NANO, -1)] {
            let mut tx = timex(modes);
            tx.set_i64(80, frac);
            assert_eq!(check(&tx, &ntp, true), Err(Errno::EINVAL));
        }
        let mut tx = timex(ADJ_SETOFFSET | ADJ_NANO);
        tx.set_i64(72, -1);
        tx.set_i64(80, 999_999_999);
        assert_eq!(check(&tx, &ntp, true).unwrap().step_ns, Some(-1));
        assert_eq!(check(&timex(0x10000), &ntp, true).map(|c| c.ntp), Ok(ntp), "unknown bits are ignored");
    }

    #[test]
    fn status_and_report() {
        let ntp = Ntp::default();
        let mut tx = timex(ADJ_STATUS | ADJ_TAI | ADJ_NANO);
        tx.set_i32(40, STA_PLL | STA_CLK);
        tx.set_i64(48, 37);
        let got = check(&tx, &ntp, true).unwrap().ntp;
        assert_eq!((got.status, got.tai, got.state()), (STA_PLL | STA_NANO, 37, TIME_OK));
        tx.report(&got, Timespec { sec: 5, nsec: 123_456_789 });
        assert_eq!((tx.modes(), tx.time(), tx.tick()), (ADJ_STATUS | ADJ_TAI | ADJ_NANO, (5, 123_456_789), 10_000));
        assert_eq!(tx.i64_at(64), MAXFREQ_SCALED);
        assert_eq!(tx.i32_at(160), 37);
    }
}
