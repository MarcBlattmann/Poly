//! CPU time by mode: how long a thread ran in user mode and in the kernel.
//!
//! The kernel measures a thread's run time with its monotonic clock at
//! every context switch. To split that into user and system time it also
//! notes, on the CPU running the thread, every change of mode: each entry
//! into the kernel from user mode (a system call, an interrupt or an
//! exception in ring 3) and each return to user mode, with a cheap
//! free-running counter (the time-stamp counter on x86-64). A [`Stint`] is
//! one CPU's record of its running thread since it was switched in; at the
//! next switch the stint's [`ModeCycles`] are added to the thread's.
//!
//! [`Split`] then divides the thread's measured run time in the ratio of
//! its counted cycles. Both are precise, so time is attributed to the mode
//! it was spent in, whatever happened before: sampling the mode at timer
//! ticks instead (ADR 0044 as first written) attributes new time in the
//! ratio of the thread's whole history of samples, and the samples are few
//! and biased where ticks coalesce or arrive late (an emulator delivers a
//! tick that came due during a slow device read right after that read, in
//! the kernel).

/// Counter units a thread spent in each mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeCycles {
    pub user: u64,
    pub system: u64,
}

impl ModeCycles {
    /// `cycles` more in user mode if `user`, else in the kernel.
    pub fn add(&mut self, cycles: u64, user: bool) {
        if user {
            self.user = self.user.saturating_add(cycles);
        } else {
            self.system = self.system.saturating_add(cycles);
        }
    }

    pub fn plus(self, other: ModeCycles) -> ModeCycles {
        ModeCycles { user: self.user.saturating_add(other.user), system: self.system.saturating_add(other.system) }
    }

    pub fn total(self) -> u64 {
        self.user.saturating_add(self.system)
    }
}

/// One CPU's record of the thread it runs, since that thread was switched
/// in: which mode the thread is in since `mark` (a counter reading), and
/// what it used before `mark`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stint {
    pub mark: u64,
    pub user: bool,
    pub used: ModeCycles,
}

impl Stint {
    /// A thread is switched in at `now`. Switches happen in the kernel, so
    /// it resumes there.
    pub fn start(now: u64) -> Stint {
        Stint { mark: now, user: false, used: ModeCycles::default() }
    }

    /// The thread enters the kernel (`user` false) or returns to user mode
    /// (`user` true) at `now`: the span since the last change counts in
    /// the mode it was in. A second change to the same mode only moves the
    /// mark.
    pub fn enter(&mut self, now: u64, user: bool) {
        self.used = self.used_at(now);
        self.mark = now.max(self.mark);
        self.user = user;
    }

    /// What the stint has used by `now`, the span still open counted in
    /// the current mode. A counter reading below `mark` (another CPU's
    /// counter, a little behind) counts as no time.
    pub fn used_at(&self, now: u64) -> ModeCycles {
        let mut used = self.used;
        used.add(now.saturating_sub(self.mark), self.user);
        used
    }
}

/// A thread's user / system split as last reported, which later reports
/// never go below (getrusage(2) and times(2) callers subtract readings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub user_ns: u64,
    pub system_ns: u64,
}

impl Split {
    /// Split `total_ns` (the run time measured so far, which only grows)
    /// in the ratio of `cycles`: no cycles at all is all system time. The
    /// parts add up to `total_ns` unless an earlier report forces one of
    /// them up; then the other takes what remains, and the sum is
    /// `total_ns` again once the ratio and the total have caught up.
    pub fn update(&mut self, total_ns: u64, cycles: ModeCycles) -> Split {
        let ideal_user = match cycles.total() {
            0 => 0,
            all => (u128::from(total_ns) * u128::from(cycles.user) / u128::from(all)) as u64,
        };
        let floor = self.user_ns;
        let ceiling = total_ns.saturating_sub(self.system_ns).max(floor);
        let user_ns = ideal_user.clamp(floor, ceiling);
        let system_ns = total_ns.saturating_sub(user_ns).max(self.system_ns);
        *self = Split { user_ns, system_ns };
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stint_counts_each_span_in_its_mode() {
        let mut s = Stint::start(100);
        s.enter(130, true); // 30 in the kernel, then user mode
        s.enter(1130, false); // 1000 in user mode: a system call
        s.enter(1140, true); // 10 in the kernel
        assert_eq!(s.used_at(1640), ModeCycles { user: 1500, system: 40 });
        assert_eq!(s.used, ModeCycles { user: 1000, system: 40 });
    }

    #[test]
    fn a_repeated_return_to_user_mode_counts_once() {
        let mut s = Stint::start(0);
        s.enter(10, true);
        s.enter(12, true);
        assert_eq!(s.used_at(20), ModeCycles { user: 10, system: 10 });
    }

    #[test]
    fn a_counter_behind_the_mark_counts_as_no_time() {
        let mut s = Stint::start(1000);
        assert_eq!(s.used_at(990), ModeCycles::default());
        s.enter(990, true);
        assert_eq!((s.mark, s.used), (1000, ModeCycles::default()));
        assert_eq!(s.used_at(1100), ModeCycles { user: 100, system: 0 });
    }

    #[test]
    fn the_split_follows_the_cycles_of_each_new_span() {
        // A thread that spent its first second in the kernel, then 200 ms
        // in a busy loop (the counter runs at 3 per ns): the 200 ms are
        // user time, however much kernel time came before.
        let mut split = Split::default();
        let ns = 1_000_000_000;
        let before = split.update(ns, ModeCycles { user: 0, system: 3 * ns });
        let after = split.update(ns + ns / 5, ModeCycles { user: 3 * ns / 5, system: 3 * ns });
        assert_eq!(before, Split { user_ns: 0, system_ns: ns });
        assert_eq!(after, Split { user_ns: ns / 5, system_ns: ns });
    }

    #[test]
    fn with_the_counter_in_step_with_the_clock_the_split_is_exact() {
        // 1 cycle per ns: user time is exactly the user cycles.
        let mut split = Split::default();
        let mut cycles = ModeCycles::default();
        let mut total = 0;
        for (span, user) in [(500, false), (2000, true), (30, false), (7000, true), (900, false)] {
            cycles.add(span, user);
            total += span;
            let s = split.update(total, cycles);
            assert_eq!((s.user_ns, s.system_ns), (cycles.user, cycles.system));
        }
    }

    #[test]
    fn no_cycles_is_all_system_time() {
        assert_eq!(Split::default().update(70, ModeCycles::default()), Split { user_ns: 0, system_ns: 70 });
    }

    #[test]
    fn neither_part_goes_down() {
        // The clock total and the cycles are read a little apart: a ratio
        // that dips must not take back reported time.
        let mut split = Split::default();
        let mut last = Split::default();
        let steps = [(1000, 900, 100), (1010, 900, 200), (1020, 1000, 200), (1500, 1000, 1000), (1500, 5000, 10)];
        for (total, user, system) in steps {
            let s = split.update(total, ModeCycles { user, system });
            assert!(s.user_ns >= last.user_ns && s.system_ns >= last.system_ns, "{s:?} after {last:?}");
            assert!(s.user_ns + s.system_ns >= total);
            last = s;
        }
    }
}
