//! A thread's scheduling attributes (sched(7)) and the rules for changing
//! them: the nice value (getpriority(2), setpriority(2)), the policy, the
//! static (real-time) priority and the `SCHED_RESET_ON_FORK` flag
//! (sched_setscheduler(2), sched_setparam(2)).
//!
//! Every attribute is per thread ([`Sched`], kept in
//! [`LinuxThread::sched`](crate::signal::LinuxThread::sched)): a new
//! thread or process starts with its creator's ([`Sched::for_child`]
//! applies `SCHED_RESET_ON_FORK`). Poly's scheduler is round-robin without
//! priorities (ADR 0012), so the attributes are recorded and reported but
//! do not change how threads run yet (ADR 0113): real-time policies are
//! accepted under the same rules as on Linux instead of being refused.
//!
//! Who may change what ([`may_change`], [`check_policy_change`],
//! [`check_nice_change`]): a caller with `CAP_SYS_NICE` anything; others
//! only threads whose real or effective user id is their effective user
//! id and whose permitted capabilities they also hold (`EPERM`), and then
//! within the target's `RLIMIT_RTPRIO` (real-time priorities) and
//! `RLIMIT_NICE` (lower nice values: `20 - nice` may not pass it; `EACCES`
//! for setpriority, `EPERM` for leaving `SCHED_IDLE`), never clearing
//! `SCHED_RESET_ON_FORK`.

use crate::cred::CapSet;
use crate::errno::Errno;

/// `SCHED_*` (`linux/sched.h`).
pub const SCHED_OTHER: i32 = 0;
pub const SCHED_FIFO: i32 = 1;
pub const SCHED_RR: i32 = 2;
pub const SCHED_BATCH: i32 = 3;
pub const SCHED_IDLE: i32 = 5;
pub const SCHED_DEADLINE: i32 = 6;
pub const SCHED_EXT: i32 = 7;
/// `SCHED_RESET_ON_FORK`, or-ed into the policy.
pub const SCHED_RESET_ON_FORK: i32 = 0x4000_0000;

/// Nice values: getpriority(2), "a value in the range -20 to 19".
pub const NICE_MIN: i32 = -20;
pub const NICE_MAX: i32 = 19;

/// The real-time priorities (sched(7)): 1 to 99.
pub const RT_PRIO_MIN: i32 = 1;
pub const RT_PRIO_MAX: i32 = 99;

/// The `SCHED_RR` quantum sched_rr_get_interval(2) reports, in
/// nanoseconds (the default `sched_rr_timeslice_ms`, 100).
pub const RR_INTERVAL_NS: i64 = 100_000_000;

/// The time slice a fair task without one of its own reports through
/// sched_getattr(2)'s `sched_runtime`, in nanoseconds (sched_getattr's
/// spec sheet: 2.8 ms, kernel-configuration dependent).
pub const DEFAULT_SLICE_NS: u64 = 2_800_000;
/// The bounds of a custom slice (sched_setattr's spec sheet).
pub const SLICE_MIN_NS: u64 = 100_000;
pub const SLICE_MAX_NS: u64 = 100_000_000;

/// Whether `policy` (without the flag) is a real-time one.
pub const fn is_realtime(policy: i32) -> bool {
    matches!(policy, SCHED_FIFO | SCHED_RR)
}

/// One thread's scheduling attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sched {
    /// -20 (most favourable) to 19.
    pub nice: i32,
    /// `SCHED_OTHER`, `SCHED_FIFO`, `SCHED_RR`, `SCHED_BATCH` or
    /// `SCHED_IDLE`, without the flag.
    pub policy: i32,
    /// The static priority: 1 to 99 under a real-time policy, else 0.
    pub priority: i32,
    pub reset_on_fork: bool,
    /// `SCHED_DEADLINE`'s parameters in nanoseconds (period as stored:
    /// the deadline when given as 0); 0 under other policies.
    pub runtime: u64,
    pub deadline: u64,
    pub period: u64,
    /// `SCHED_FLAG_RECLAIM` / `SCHED_FLAG_DL_OVERRUN` as set with
    /// sched_setattr(2), reported for a deadline task.
    pub dl_flags: u64,
    /// A custom time slice for the fair policies set with sched_setattr(2)
    /// (100 us .. 100 ms), 0 for the default ([`DEFAULT_SLICE_NS`]).
    pub slice: u64,
}

impl Sched {
    /// The policy as sched_getscheduler(2) reports it: with
    /// `SCHED_RESET_ON_FORK` or-ed in while the flag is set.
    pub const fn reported_policy(&self) -> i32 {
        if self.reset_on_fork { self.policy | SCHED_RESET_ON_FORK } else { self.policy }
    }

    /// What a thread or process this thread creates starts with
    /// (sched(7), "Resetting scheduling policy for child processes"): the
    /// same, unless `SCHED_RESET_ON_FORK` is set: then the flag is clear,
    /// a real-time policy becomes `SCHED_OTHER` with nice 0, and a
    /// negative nice value of a normal policy becomes 0.
    pub fn for_child(&self) -> Sched {
        if !self.reset_on_fork {
            return *self;
        }
        if is_realtime(self.policy) || self.policy == SCHED_DEADLINE {
            return Sched::default();
        }
        Sched { nice: self.nice.max(0), reset_on_fork: false, ..*self }
    }
}

/// A policy argument of sched_setscheduler(2) (a 32-bit int): the policy
/// and whether `SCHED_RESET_ON_FORK` is in it. `EINVAL` for a negative
/// value, other high bits, or a policy that call does not set (`4`,
/// `SCHED_DEADLINE`, `SCHED_EXT`, ...).
pub fn parse_policy(arg: i32) -> Result<(i32, bool), Errno> {
    let policy = arg & !SCHED_RESET_ON_FORK;
    match policy {
        SCHED_OTHER | SCHED_FIFO | SCHED_RR | SCHED_BATCH | SCHED_IDLE => Ok((policy, arg & SCHED_RESET_ON_FORK != 0)),
        _ => Err(Errno::EINVAL),
    }
}

/// Whether `priority` suits `policy`: 0 for the normal policies, 1 to 99
/// for the real-time ones (`EINVAL` otherwise).
pub fn check_priority(policy: i32, priority: i32) -> Result<(), Errno> {
    let ok = if is_realtime(policy) { (RT_PRIO_MIN..=RT_PRIO_MAX).contains(&priority) } else { priority == 0 };
    if ok { Ok(()) } else { Err(Errno::EINVAL) }
}

/// sched_get_priority_max(2) and sched_get_priority_min(2) of `policy`
/// (a 32-bit int, no flag allowed): `(min, max)`.
pub fn priority_range(policy: i32) -> Result<(i32, i32), Errno> {
    match policy {
        SCHED_FIFO | SCHED_RR => Ok((RT_PRIO_MIN, RT_PRIO_MAX)),
        SCHED_OTHER | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE | SCHED_EXT => Ok((0, 0)),
        _ => Err(Errno::EINVAL),
    }
}

/// sched_rr_get_interval(2) under `policy`: the quantum in nanoseconds
/// (Poly reports 0 for the normal policies, as a thread without a slice
/// of its own does).
pub const fn rr_interval_ns(policy: i32) -> i64 {
    if policy == SCHED_RR { RR_INTERVAL_NS } else { 0 }
}

/// The caller of a change, as the rules see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changer {
    /// `CAP_SYS_NICE` (in the initial user namespace).
    pub privileged: bool,
    pub euid: u32,
    pub permitted: CapSet,
}

/// The thread being changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed {
    /// Real and effective user ids.
    pub uid: u32,
    pub euid: u32,
    pub permitted: CapSet,
    pub sched: Sched,
    /// The soft limits of its process's `RLIMIT_RTPRIO` and `RLIMIT_NICE`.
    pub rtprio_limit: u64,
    pub nice_limit: u64,
}

/// Whether `who` may change thread `t` at all: `CAP_SYS_NICE`, or its
/// effective user id is `t`'s real or effective one and `t` has no
/// permitted capability `who` lacks (`EPERM`).
pub fn may_change(who: &Changer, t: &Changed) -> Result<(), Errno> {
    if who.privileged {
        return Ok(());
    }
    if who.euid != t.uid && who.euid != t.euid {
        return Err(Errno::EPERM);
    }
    if !t.permitted.is_subset(who.permitted) {
        return Err(Errno::EPERM);
    }
    Ok(())
}

/// Whether nice value `nice` is within a `RLIMIT_NICE` soft limit of
/// `limit` (setrlimit(2): "20 - rlim_cur" is the ceiling).
fn nice_allowed(nice: i32, limit: u64) -> bool {
    (20 - nice) as u64 <= limit
}

/// The checks of sched_setscheduler(2) (and sched_setparam(2), which
/// passes the thread's own policy and flag) after the arguments were
/// found valid: `EPERM` per the module docs.
pub fn check_policy_change(
    who: &Changer,
    t: &Changed,
    policy: i32,
    priority: i32,
    reset_on_fork: bool,
) -> Result<(), Errno> {
    may_change(who, t)?;
    if who.privileged {
        return Ok(());
    }
    let cur = &t.sched;
    if is_realtime(policy) {
        if policy != cur.policy && t.rtprio_limit == 0 {
            return Err(Errno::EPERM);
        }
        let current = if is_realtime(cur.policy) { cur.priority } else { 0 };
        if priority > current && priority as u64 > t.rtprio_limit {
            return Err(Errno::EPERM);
        }
    } else if cur.policy == SCHED_IDLE && policy != SCHED_IDLE && !nice_allowed(cur.nice, t.nice_limit) {
        return Err(Errno::EPERM);
    }
    if cur.reset_on_fork && !reset_on_fork {
        return Err(Errno::EPERM);
    }
    Ok(())
}

/// The checks of setpriority(2) for one thread, with `nice` already
/// clamped: `EPERM` ([`may_change`]), then `EACCES` for a lower value
/// than the thread has beyond its `RLIMIT_NICE`.
pub fn check_nice_change(who: &Changer, t: &Changed, nice: i32) -> Result<(), Errno> {
    may_change(who, t)?;
    if !who.privileged && nice < t.sched.nice && !nice_allowed(nice, t.nice_limit) {
        return Err(Errno::EACCES);
    }
    Ok(())
}

// ---- sched_setattr(2) / sched_getattr(2) ----------------------------------

/// `SCHED_FLAG_*` (`linux/sched.h`).
pub const SCHED_FLAG_RESET_ON_FORK: u64 = 0x01;
pub const SCHED_FLAG_RECLAIM: u64 = 0x02;
pub const SCHED_FLAG_DL_OVERRUN: u64 = 0x04;
pub const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
pub const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
pub const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
pub const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;
pub const SCHED_FLAG_ALL: u64 = 0x7f;

/// `SCHED_ATTR_SIZE_VER0` and `SCHED_ATTR_SIZE_VER1` (`linux/sched/types.h`):
/// the structure without and with the utilisation clamp fields.
pub const SCHED_ATTR_SIZE_VER0: usize = 48;
pub const SCHED_ATTR_SIZE_VER1: usize = 56;
/// The largest `size` either call accepts (a page).
pub const SCHED_ATTR_SIZE_MAX: usize = 4096;

/// `struct sched_attr` (`linux/sched/types.h`), 56 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchedAttr {
    pub size: u32,
    pub policy: u32,
    pub flags: u64,
    pub nice: i32,
    pub priority: u32,
    pub runtime: u64,
    pub deadline: u64,
    pub period: u64,
    pub util_min: u32,
    pub util_max: u32,
}

impl SchedAttr {
    pub fn to_bytes(&self) -> [u8; SCHED_ATTR_SIZE_VER1] {
        let mut b = [0u8; SCHED_ATTR_SIZE_VER1];
        b[0..4].copy_from_slice(&self.size.to_ne_bytes());
        b[4..8].copy_from_slice(&self.policy.to_ne_bytes());
        b[8..16].copy_from_slice(&self.flags.to_ne_bytes());
        b[16..20].copy_from_slice(&self.nice.to_ne_bytes());
        b[20..24].copy_from_slice(&self.priority.to_ne_bytes());
        b[24..32].copy_from_slice(&self.runtime.to_ne_bytes());
        b[32..40].copy_from_slice(&self.deadline.to_ne_bytes());
        b[40..48].copy_from_slice(&self.period.to_ne_bytes());
        b[48..52].copy_from_slice(&self.util_min.to_ne_bytes());
        b[52..56].copy_from_slice(&self.util_max.to_ne_bytes());
        b
    }

    /// The structure from its first `b.len()` bytes (48 to 56; missing
    /// fields read 0).
    pub fn from_bytes(b: &[u8]) -> SchedAttr {
        let mut full = [0u8; SCHED_ATTR_SIZE_VER1];
        let n = b.len().min(SCHED_ATTR_SIZE_VER1);
        full[..n].copy_from_slice(&b[..n]);
        let u32_at = |i: usize| u32::from_ne_bytes(full[i..i + 4].try_into().expect("4 bytes"));
        let u64_at = |i: usize| u64::from_ne_bytes(full[i..i + 8].try_into().expect("8 bytes"));
        SchedAttr {
            size: u32_at(0),
            policy: u32_at(4),
            flags: u64_at(8),
            nice: u32_at(16) as i32,
            priority: u32_at(20),
            runtime: u64_at(24),
            deadline: u64_at(32),
            period: u64_at(40),
            util_min: u32_at(48),
            util_max: u32_at(52),
        }
    }

    /// What sched_getattr(2) reports for `s` (`size` set by the caller).
    pub fn of(s: &Sched) -> SchedAttr {
        let fair = !is_realtime(s.policy) && s.policy != SCHED_DEADLINE;
        let mut a = SchedAttr {
            policy: s.policy as u32,
            flags: if s.reset_on_fork { SCHED_FLAG_RESET_ON_FORK } else { 0 },
            ..SchedAttr::default()
        };
        if fair {
            a.nice = s.nice;
            a.runtime = if s.slice == 0 { DEFAULT_SLICE_NS } else { s.slice };
        } else if s.policy == SCHED_DEADLINE {
            a.flags |= s.dl_flags;
            a.runtime = s.runtime;
            a.deadline = s.deadline;
            a.period = s.period;
        } else {
            a.priority = s.priority as u32;
        }
        a
    }
}

/// The attributes a sched_setattr(2) of `attr` (read with `size` bytes)
/// gives a thread that has `cur`, after the policy, flag, priority and
/// deadline checks (`EINVAL`; `EOPNOTSUPP` for utilisation clamping,
/// which Poly does not do). The permission checks follow
/// ([`check_attr_change`]).
pub fn apply_attr(cur: &Sched, attr: &SchedAttr, size: usize) -> Result<Sched, Errno> {
    let flags = attr.flags;
    let keep_policy = flags & SCHED_FLAG_KEEP_POLICY != 0;
    let keep_params = flags & SCHED_FLAG_KEEP_PARAMS != 0;
    let policy = if keep_policy {
        cur.policy
    } else {
        match attr.policy as i32 {
            p @ (SCHED_OTHER | SCHED_FIFO | SCHED_RR | SCHED_BATCH | SCHED_IDLE | SCHED_DEADLINE) => p,
            _ => return Err(Errno::EINVAL),
        }
    };
    if flags & !SCHED_FLAG_ALL != 0 {
        return Err(Errno::EINVAL);
    }
    let clamp = SCHED_FLAG_UTIL_CLAMP_MIN | SCHED_FLAG_UTIL_CLAMP_MAX;
    if flags & clamp != 0 {
        return Err(if size < SCHED_ATTR_SIZE_VER1 { Errno::EINVAL } else { Errno::EOPNOTSUPP });
    }
    let priority = attr.priority as i32;
    if attr.priority > i32::MAX as u32 {
        return Err(Errno::EINVAL);
    }
    if policy == SCHED_DEADLINE {
        if priority != 0 {
            return Err(Errno::EINVAL);
        }
        let bit63 = 1u64 << 63;
        let period = if attr.period == 0 { attr.deadline } else { attr.period };
        if attr.runtime < 1024
            || attr.deadline == 0
            || attr.deadline & bit63 != 0
            || attr.period & bit63 != 0
            || attr.deadline > period
            || attr.runtime > attr.deadline
        {
            return Err(Errno::EINVAL);
        }
    } else {
        check_priority(policy, priority)?;
    }
    let reset_on_fork = flags & SCHED_FLAG_RESET_ON_FORK != 0;
    if keep_params {
        return Ok(Sched { reset_on_fork, ..*cur });
    }
    let mut next = Sched { policy, reset_on_fork, priority, ..Sched::default() };
    next.nice = cur.nice;
    if policy == SCHED_DEADLINE {
        next.runtime = attr.runtime;
        next.deadline = attr.deadline;
        next.period = if attr.period == 0 { attr.deadline } else { attr.period };
        next.dl_flags = flags & (SCHED_FLAG_RECLAIM | SCHED_FLAG_DL_OVERRUN);
    } else if !is_realtime(policy) {
        next.nice = attr.nice.clamp(NICE_MIN, NICE_MAX);
        next.slice = if attr.runtime == 0 { 0 } else { attr.runtime.clamp(SLICE_MIN_NS, SLICE_MAX_NS) };
    }
    Ok(next)
}

/// The permission checks of sched_setattr(2) for going from `t.sched` to
/// `next` (`EPERM`): those of sched_setscheduler(2), a lower nice value
/// within `RLIMIT_NICE`, and `SCHED_DEADLINE` only with `CAP_SYS_NICE`.
pub fn check_attr_change(who: &Changer, t: &Changed, next: &Sched) -> Result<(), Errno> {
    may_change(who, t)?;
    if who.privileged {
        return Ok(());
    }
    if next.policy == SCHED_DEADLINE {
        return Err(Errno::EPERM);
    }
    check_policy_change(who, t, next.policy, next.priority, next.reset_on_fork)?;
    if next.nice < t.sched.nice && !nice_allowed(next.nice, t.nice_limit) {
        return Err(Errno::EPERM);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(euid: u32) -> Changer {
        Changer { privileged: false, euid, permitted: CapSet::EMPTY }
    }

    fn thread(uid: u32, euid: u32, sched: Sched) -> Changed {
        Changed { uid, euid, permitted: CapSet::EMPTY, sched, rtprio_limit: 0, nice_limit: 0 }
    }

    const ROOT: Changer = Changer { privileged: true, euid: 0, permitted: CapSet::ALL };

    #[test]
    fn policies_and_priorities() {
        assert_eq!(parse_policy(SCHED_BATCH), Ok((SCHED_BATCH, false)));
        assert_eq!(parse_policy(SCHED_RR | SCHED_RESET_ON_FORK), Ok((SCHED_RR, true)));
        for bad in [4, SCHED_DEADLINE, SCHED_EXT, 8, -1, 0x2000_0000, i32::MIN] {
            assert_eq!(parse_policy(bad), Err(Errno::EINVAL), "{bad:#x}");
        }
        assert_eq!(check_priority(SCHED_OTHER, 0), Ok(()));
        assert_eq!(check_priority(SCHED_IDLE, 1), Err(Errno::EINVAL));
        assert_eq!(check_priority(SCHED_BATCH, -1), Err(Errno::EINVAL));
        assert_eq!(check_priority(SCHED_FIFO, 0), Err(Errno::EINVAL));
        assert_eq!(check_priority(SCHED_RR, 100), Err(Errno::EINVAL));
        assert_eq!(check_priority(SCHED_RR, 99), Ok(()));
        assert_eq!(priority_range(SCHED_FIFO), Ok((1, 99)));
        assert_eq!(priority_range(SCHED_EXT), Ok((0, 0)));
        assert_eq!(priority_range(SCHED_DEADLINE), Ok((0, 0)));
        assert_eq!(priority_range(4), Err(Errno::EINVAL));
        assert_eq!(priority_range(SCHED_FIFO | SCHED_RESET_ON_FORK), Err(Errno::EINVAL));
        assert_eq!(
            (rr_interval_ns(SCHED_RR), rr_interval_ns(SCHED_FIFO), rr_interval_ns(SCHED_OTHER)),
            (RR_INTERVAL_NS, 0, 0)
        );
    }

    #[test]
    fn reset_on_fork() {
        let rt = Sched { nice: -5, policy: SCHED_FIFO, priority: 30, reset_on_fork: true, ..Sched::default() };
        assert_eq!(rt.reported_policy(), SCHED_FIFO | SCHED_RESET_ON_FORK);
        assert_eq!(rt.for_child(), Sched::default());
        let batch = Sched { nice: -5, policy: SCHED_BATCH, priority: 0, reset_on_fork: true, ..Sched::default() };
        assert_eq!(
            batch.for_child(),
            Sched { nice: 0, policy: SCHED_BATCH, priority: 0, reset_on_fork: false, ..Sched::default() }
        );
        let idle = Sched { nice: 5, policy: SCHED_IDLE, priority: 0, reset_on_fork: true, ..Sched::default() };
        assert_eq!(idle.for_child(), Sched { reset_on_fork: false, ..idle });
        let plain = Sched { nice: -3, policy: SCHED_RR, priority: 7, reset_on_fork: false, ..Sched::default() };
        assert_eq!(plain.for_child(), plain);
    }

    #[test]
    fn owners_and_capabilities() {
        let t = thread(1111, 2222, Sched::default());
        assert_eq!(may_change(&user(1111), &t), Ok(()));
        assert_eq!(may_change(&user(2222), &t), Ok(()));
        assert_eq!(may_change(&user(3333), &t), Err(Errno::EPERM));
        assert_eq!(may_change(&ROOT, &t), Ok(()));
        let capable = Changed { permitted: CapSet::ALL, ..thread(1111, 1111, Sched::default()) };
        assert_eq!(may_change(&user(1111), &capable), Err(Errno::EPERM));
    }

    #[test]
    fn realtime_limits() {
        let me = user(65534);
        let mut t = Changed { rtprio_limit: 10, nice_limit: 25, ..thread(65534, 65534, Sched::default()) };
        let set = |t: &Changed, p, prio, r| check_policy_change(&me, t, p, prio, r);
        assert_eq!(set(&t, SCHED_FIFO, 11, false), Err(Errno::EPERM));
        assert_eq!(set(&t, SCHED_FIFO, 10, false), Ok(()));
        t.sched = Sched { policy: SCHED_RR, priority: 5, ..Sched::default() };
        t.rtprio_limit = 0;
        assert_eq!(set(&t, SCHED_RR, 4, false), Ok(()));
        assert_eq!(set(&t, SCHED_RR, 6, false), Err(Errno::EPERM));
        assert_eq!(set(&t, SCHED_FIFO, 4, false), Err(Errno::EPERM));
        assert_eq!(set(&t, SCHED_OTHER, 0, false), Ok(()));
        t.sched = Sched::default();
        assert_eq!(set(&t, SCHED_FIFO, 1, false), Err(Errno::EPERM));
        // Leaving SCHED_IDLE needs the nice value within RLIMIT_NICE.
        t.sched.policy = SCHED_IDLE;
        assert_eq!(set(&t, SCHED_OTHER, 0, false), Ok(()));
        t.nice_limit = 0;
        assert_eq!(set(&t, SCHED_OTHER, 0, false), Err(Errno::EPERM));
        assert_eq!(set(&t, SCHED_IDLE, 0, false), Ok(()));
        // The flag cannot be cleared without privilege.
        t.sched = Sched { reset_on_fork: true, ..Sched::default() };
        assert_eq!(set(&t, SCHED_OTHER, 0, false), Err(Errno::EPERM));
        assert_eq!(set(&t, SCHED_BATCH, 0, true), Ok(()));
        assert_eq!(check_policy_change(&ROOT, &t, SCHED_FIFO, 99, false), Ok(()));
    }

    #[test]
    fn nice_limits() {
        let me = user(1000);
        let mut t = thread(1000, 1000, Sched { nice: 5, ..Sched::default() });
        assert_eq!(check_nice_change(&me, &t, 8), Ok(()));
        assert_eq!(check_nice_change(&me, &t, 5), Ok(()));
        assert_eq!(check_nice_change(&me, &t, 3), Err(Errno::EACCES));
        t.nice_limit = 25;
        assert_eq!(check_nice_change(&me, &t, -5), Ok(()));
        assert_eq!(check_nice_change(&me, &t, -6), Err(Errno::EACCES));
        assert_eq!(check_nice_change(&user(4444), &t, 9), Err(Errno::EPERM));
        assert_eq!(check_nice_change(&user(4444), &t, -20), Err(Errno::EPERM));
        assert_eq!(check_nice_change(&ROOT, &t, -20), Ok(()));
    }

    fn attr(policy: i32, flags: u64, nice: i32, priority: u32) -> SchedAttr {
        SchedAttr { size: 48, policy: policy as u32, flags, nice, priority, ..SchedAttr::default() }
    }

    #[test]
    fn sched_attr_layout() {
        let a = SchedAttr {
            size: 56,
            policy: 6,
            flags: 1,
            nice: -3,
            priority: 0,
            runtime: 7,
            deadline: 8,
            period: 9,
            util_min: 0,
            util_max: 1024,
        };
        let b = a.to_bytes();
        assert_eq!(&b[16..20], &(-3i32).to_ne_bytes());
        assert_eq!(SchedAttr::from_bytes(&b), a);
        assert_eq!(SchedAttr::from_bytes(&b[..48]), SchedAttr { util_max: 0, ..a });
    }

    #[test]
    fn getattr_reports_by_policy() {
        let fair = Sched { nice: 7, policy: SCHED_BATCH, reset_on_fork: true, ..Sched::default() };
        let a = SchedAttr::of(&fair);
        assert_eq!((a.policy, a.flags, a.nice, a.priority, a.runtime), (3, 1, 7, 0, DEFAULT_SLICE_NS));
        let rt = Sched { nice: 5, policy: SCHED_FIFO, priority: 10, ..Sched::default() };
        let a = SchedAttr::of(&rt);
        assert_eq!((a.nice, a.priority, a.runtime), (0, 10, 0));
    }

    #[test]
    fn setattr_rules() {
        let cur = Sched { nice: 3, policy: SCHED_BATCH, ..Sched::default() };
        // KEEP_POLICY keeps the policy, applies nice; priority judged by the kept policy.
        let next = apply_attr(&cur, &attr(SCHED_OTHER, SCHED_FLAG_KEEP_POLICY, 4, 0), 48).unwrap();
        assert_eq!((next.policy, next.nice), (SCHED_BATCH, 4));
        assert_eq!(apply_attr(&cur, &attr(SCHED_RR, SCHED_FLAG_KEEP_POLICY, 6, 7), 48), Err(Errno::EINVAL));
        let both = SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS;
        assert_eq!(apply_attr(&cur, &attr(SCHED_RR, both, 6, 7), 48), Err(Errno::EINVAL));
        // KEEP_PARAMS keeps policy, nice and priority; the reset flag applies.
        let flags = SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_RESET_ON_FORK;
        let next = apply_attr(&cur, &attr(SCHED_OTHER, flags, 9, 0), 48).unwrap();
        assert_eq!(next, Sched { reset_on_fork: true, ..cur });
        for bad in [4, 7, 8, -1, SCHED_BATCH | SCHED_RESET_ON_FORK] {
            assert_eq!(apply_attr(&cur, &attr(bad, 0, 0, 0), 48), Err(Errno::EINVAL), "{bad}");
        }
        for bad in [0x80, 0x100, 1 << 63] {
            assert_eq!(apply_attr(&cur, &attr(0, bad, 0, 0), 48), Err(Errno::EINVAL));
        }
        assert_eq!(apply_attr(&cur, &attr(0, SCHED_FLAG_UTIL_CLAMP_MIN, 0, 0), 48), Err(Errno::EINVAL));
        assert_eq!(apply_attr(&cur, &attr(0, SCHED_FLAG_UTIL_CLAMP_MAX, 0, 0), 56), Err(Errno::EOPNOTSUPP));
        assert_eq!(apply_attr(&cur, &attr(0, 0, 25, 0), 48).unwrap().nice, 19, "clamped");
        assert_eq!(apply_attr(&cur, &attr(SCHED_FIFO, 0, 0, 100), 48), Err(Errno::EINVAL));
    }

    #[test]
    fn setattr_deadline() {
        let dl = |rt, d, p| SchedAttr { runtime: rt, deadline: d, period: p, ..attr(SCHED_DEADLINE, 0, 0, 0) };
        let next = apply_attr(&Sched::default(), &dl(10_000_000, 30_000_000, 0), 48).unwrap();
        assert_eq!((next.runtime, next.deadline, next.period), (10_000_000, 30_000_000, 30_000_000));
        for (rt, d, p) in [(1000, 2000, 0), (5000, 0, 0), (5000, 4000, 0), (5000, 9000, 8000), (5000, 1 << 63, 0)] {
            assert_eq!(apply_attr(&Sched::default(), &dl(rt, d, p), 48), Err(Errno::EINVAL), "{rt} {d} {p}");
        }
        let t = thread(1000, 1000, Sched::default());
        assert_eq!(check_attr_change(&user(1000), &t, &next), Err(Errno::EPERM));
        assert_eq!(check_attr_change(&ROOT, &t, &next), Ok(()));
        assert_eq!(Sched { reset_on_fork: true, ..next }.for_child(), Sched::default());
        let lower = Sched { nice: -1, ..Sched::default() };
        assert_eq!(check_attr_change(&user(1000), &t, &lower), Err(Errno::EPERM));
    }
}
