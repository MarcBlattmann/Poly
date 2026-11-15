//! Thread and process scheduling settings as `kernel32` reports them
//! (`SetThreadPriority`, `GetThreadPriority`, `Set/GetThreadPriorityBoost`,
//! `Set/GetPriorityClass`, `SetThreadInformation`; Learn's reference pages).
//!
//! Poly's scheduler has no priorities yet, so the settings are kept per
//! process and reported back as they were set: a program that sets a level
//! reads the same level back, and every value Learn lists is accepted
//! (ADR 0155 addendum "S3 hunt"). Values from `winbase.h`/`processthreadsapi.h`.
//!
//! The table takes no lock and allocates nothing: a thread that samples another one asks
//! for its settings while that thread is suspended (Chrome's stack sampler), so nothing here
//! may wait for something the suspended thread could hold.

use crate::error::ERROR_INVALID_PARAMETER;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const THREAD_PRIORITY_IDLE: i32 = -15;
pub const THREAD_PRIORITY_LOWEST: i32 = -2;
pub const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
pub const THREAD_PRIORITY_NORMAL: i32 = 0;
pub const THREAD_PRIORITY_ABOVE_NORMAL: i32 = 1;
pub const THREAD_PRIORITY_HIGHEST: i32 = 2;
pub const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;
pub const THREAD_MODE_BACKGROUND_BEGIN: i32 = 0x0001_0000;
pub const THREAD_MODE_BACKGROUND_END: i32 = 0x0002_0000;
/// `GetThreadPriority`'s failure value.
pub const THREAD_PRIORITY_ERROR_RETURN: i32 = 0x7FFF_FFFF;

pub const NORMAL_PRIORITY_CLASS: u32 = 0x20;
pub const IDLE_PRIORITY_CLASS: u32 = 0x40;
pub const HIGH_PRIORITY_CLASS: u32 = 0x80;
pub const REALTIME_PRIORITY_CLASS: u32 = 0x100;
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
pub const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x8000;
pub const PROCESS_MODE_BACKGROUND_BEGIN: u32 = 0x0010_0000;
pub const PROCESS_MODE_BACKGROUND_END: u32 = 0x0020_0000;

/// `winerror.h`.
pub const ERROR_THREAD_MODE_ALREADY_BACKGROUND: u32 = 400;
pub const ERROR_THREAD_MODE_NOT_BACKGROUND: u32 = 401;
pub const ERROR_PROCESS_MODE_ALREADY_BACKGROUND: u32 = 402;
pub const ERROR_PROCESS_MODE_NOT_BACKGROUND: u32 = 403;

/// `THREAD_INFORMATION_CLASS` (`processthreadsapi.h`).
pub const THREAD_MEMORY_PRIORITY: u32 = 0;
pub const THREAD_ABSOLUTE_CPU_PRIORITY: u32 = 1;
pub const THREAD_DYNAMIC_CODE_POLICY: u32 = 2;
pub const THREAD_POWER_THROTTLING: u32 = 3;

/// `SetThreadInformation`'s check of `ThreadInformationClass` and
/// `ThreadInformationSize`: `MEMORY_PRIORITY_INFORMATION` is a `ULONG`,
/// `ThreadAbsoluteCpuPriority` a `LONG`, `ThreadDynamicCodePolicy` a
/// `DWORD`, `THREAD_POWER_THROTTLING_STATE` three `ULONG`s (version,
/// control mask, state mask; version 1). An error code when not accepted.
pub fn check_thread_information(class: u32, size: u32, first_word: Option<u32>) -> Result<(), u32> {
    let want = match class {
        THREAD_MEMORY_PRIORITY | THREAD_ABSOLUTE_CPU_PRIORITY | THREAD_DYNAMIC_CODE_POLICY => 4,
        THREAD_POWER_THROTTLING => 12,
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    if size != want {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // THREAD_POWER_THROTTLING_CURRENT_VERSION is 1.
    if class == THREAD_POWER_THROTTLING && first_word != Some(1) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(())
}

/// One thread's settings packed in a word: the thread id (high 32 bits), whether boosting is
/// disabled (bit 9), whether it is in background mode (bit 8), the priority level (low byte).
const BOOST_OFF: u64 = 1 << 9;
const BACKGROUND: u64 = 1 << 8;

fn pack(tid: u32, priority: i32, flags: u64) -> u64 {
    (u64::from(tid) << 32) | flags | u64::from(priority as i8 as u8)
}

fn level(word: u64) -> i32 {
    i32::from(word as u8 as i8)
}

/// The process word: the class (0: never set, `NORMAL_PRIORITY_CLASS`), background mode,
/// boosting disabled.
const PROCESS_BACKGROUND: u32 = 1 << 31;
const PROCESS_BOOST_OFF: u32 = 1 << 30;

/// One process's settings: `N` threads at most are remembered; a thread beyond them reads the
/// defaults.
#[derive(Debug)]
pub struct Priorities<const N: usize> {
    threads: [AtomicU64; N],
    process: AtomicU32,
}

impl<const N: usize> Default for Priorities<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Priorities<N> {
    pub const fn new() -> Self {
        Priorities { threads: [const { AtomicU64::new(0) }; N], process: AtomicU32::new(0) }
    }

    fn find(&self, tid: u32) -> Option<&AtomicU64> {
        self.threads.iter().find(|w| {
            let v = w.load(Ordering::Acquire);
            v != 0 && (v >> 32) as u32 == tid
        })
    }

    /// The thread's word, made with the defaults (and the process's boost setting) if new.
    fn slot(&self, tid: u32) -> Option<&AtomicU64> {
        if let Some(w) = self.find(tid) {
            return Some(w);
        }
        let boost = if self.process_boost() { BOOST_OFF } else { 0 };
        let fresh = pack(tid, THREAD_PRIORITY_NORMAL, boost);
        for w in &self.threads {
            if w.compare_exchange(0, fresh, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return Some(w);
            }
            if (w.load(Ordering::Acquire) >> 32) as u32 == tid {
                return Some(w);
            }
        }
        None
    }

    /// Change the thread's word with `f`.
    fn update(&self, tid: u32, f: impl Fn(u64) -> Result<u64, u32>) -> Result<(), u32> {
        let Some(w) = self.slot(tid) else {
            // No room: checked, not remembered.
            return f(pack(tid, THREAD_PRIORITY_NORMAL, 0)).map(|_| ());
        };
        let mut cur = w.load(Ordering::Acquire);
        loop {
            let new = f(cur)?;
            match w.compare_exchange(cur, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(()),
                Err(now) => cur = now,
            }
        }
    }

    /// `SetThreadPriority(thread tid, value)`; `current`: the thread is the
    /// caller (the background modes are only for the calling thread).
    pub fn set_thread(&self, tid: u32, value: i32, current: bool) -> Result<(), u32> {
        self.update(tid, |w| match value {
            THREAD_PRIORITY_IDLE
            | THREAD_PRIORITY_LOWEST
            | THREAD_PRIORITY_BELOW_NORMAL
            | THREAD_PRIORITY_NORMAL
            | THREAD_PRIORITY_ABOVE_NORMAL
            | THREAD_PRIORITY_HIGHEST
            | THREAD_PRIORITY_TIME_CRITICAL => Ok(pack(tid, value, w & (BOOST_OFF | BACKGROUND))),
            THREAD_MODE_BACKGROUND_BEGIN | THREAD_MODE_BACKGROUND_END if !current => Err(ERROR_INVALID_PARAMETER),
            THREAD_MODE_BACKGROUND_BEGIN if w & BACKGROUND != 0 => Err(ERROR_THREAD_MODE_ALREADY_BACKGROUND),
            THREAD_MODE_BACKGROUND_BEGIN => Ok(w | BACKGROUND),
            THREAD_MODE_BACKGROUND_END if w & BACKGROUND == 0 => Err(ERROR_THREAD_MODE_NOT_BACKGROUND),
            THREAD_MODE_BACKGROUND_END => Ok(w & !BACKGROUND),
            _ => Err(ERROR_INVALID_PARAMETER),
        })
    }

    /// `GetThreadPriority`: the level set last (`THREAD_PRIORITY_NORMAL` at first).
    pub fn thread(&self, tid: u32) -> i32 {
        self.find(tid).map_or(THREAD_PRIORITY_NORMAL, |w| level(w.load(Ordering::Acquire)))
    }

    /// `SetThreadPriorityBoost`.
    pub fn set_thread_boost(&self, tid: u32, disabled: bool) {
        let _ = self.update(tid, |w| Ok(if disabled { w | BOOST_OFF } else { w & !BOOST_OFF }));
    }

    /// `GetThreadPriorityBoost`: whether boosting is disabled (a thread never set takes its
    /// process's setting).
    pub fn thread_boost(&self, tid: u32) -> bool {
        self.find(tid).map_or(self.process_boost(), |w| w.load(Ordering::Acquire) & BOOST_OFF != 0)
    }

    pub fn set_process_boost(&self, disabled: bool) {
        if disabled {
            self.process.fetch_or(PROCESS_BOOST_OFF, Ordering::AcqRel);
        } else {
            self.process.fetch_and(!PROCESS_BOOST_OFF, Ordering::AcqRel);
        }
    }

    pub fn process_boost(&self) -> bool {
        self.process.load(Ordering::Acquire) & PROCESS_BOOST_OFF != 0
    }

    /// `SetPriorityClass(process, class)`; `current`: the process is the
    /// caller (the background modes are only for the calling process).
    pub fn set_class(&self, class: u32, current: bool) -> Result<(), u32> {
        let mut cur = self.process.load(Ordering::Acquire);
        loop {
            let flags = cur & (PROCESS_BACKGROUND | PROCESS_BOOST_OFF);
            let new = match class {
                NORMAL_PRIORITY_CLASS
                | IDLE_PRIORITY_CLASS
                | HIGH_PRIORITY_CLASS
                | REALTIME_PRIORITY_CLASS
                | BELOW_NORMAL_PRIORITY_CLASS
                | ABOVE_NORMAL_PRIORITY_CLASS => flags | class,
                PROCESS_MODE_BACKGROUND_BEGIN | PROCESS_MODE_BACKGROUND_END if !current => {
                    return Err(ERROR_INVALID_PARAMETER);
                }
                PROCESS_MODE_BACKGROUND_BEGIN if cur & PROCESS_BACKGROUND != 0 => {
                    return Err(ERROR_PROCESS_MODE_ALREADY_BACKGROUND);
                }
                PROCESS_MODE_BACKGROUND_BEGIN => cur | PROCESS_BACKGROUND,
                PROCESS_MODE_BACKGROUND_END if cur & PROCESS_BACKGROUND == 0 => {
                    return Err(ERROR_PROCESS_MODE_NOT_BACKGROUND);
                }
                PROCESS_MODE_BACKGROUND_END => cur & !PROCESS_BACKGROUND,
                _ => return Err(ERROR_INVALID_PARAMETER),
            };
            match self.process.compare_exchange(cur, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(()),
                Err(now) => cur = now,
            }
        }
    }

    /// `GetPriorityClass`: the class last set, `NORMAL_PRIORITY_CLASS` at
    /// first (the background mode is not a class).
    pub fn class(&self) -> u32 {
        match self.process.load(Ordering::Acquire) & !(PROCESS_BACKGROUND | PROCESS_BOOST_OFF) {
            0 => NORMAL_PRIORITY_CLASS,
            c => c,
        }
    }

    /// A thread ended: its settings go.
    pub fn forget(&self, tid: u32) {
        if let Some(w) = self.find(tid) {
            w.store(0, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_levels_are_kept_and_read_back() {
        let p = Priorities::<8>::new();
        assert_eq!(p.thread(7), THREAD_PRIORITY_NORMAL);
        for v in [-15, -2, -1, 0, 1, 2, 15] {
            p.set_thread(7, v, false).unwrap();
            assert_eq!(p.thread(7), v);
        }
        assert_eq!(p.thread(8), 0);
        assert_eq!(p.set_thread(7, 3, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(p.set_thread(7, -3, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(p.thread(7), 15);
    }

    #[test]
    fn background_mode_is_for_the_calling_thread_once() {
        let p = Priorities::<8>::new();
        p.set_thread(1, THREAD_PRIORITY_HIGHEST, true).unwrap();
        assert_eq!(p.set_thread(2, THREAD_MODE_BACKGROUND_BEGIN, false), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(p.set_thread(1, THREAD_MODE_BACKGROUND_END, true), Err(ERROR_THREAD_MODE_NOT_BACKGROUND));
        p.set_thread(1, THREAD_MODE_BACKGROUND_BEGIN, true).unwrap();
        assert_eq!(p.set_thread(1, THREAD_MODE_BACKGROUND_BEGIN, true), Err(ERROR_THREAD_MODE_ALREADY_BACKGROUND));
        // The level is not changed by the mode.
        assert_eq!(p.thread(1), THREAD_PRIORITY_HIGHEST);
        p.set_thread(1, THREAD_MODE_BACKGROUND_END, true).unwrap();
        assert_eq!(p.thread(1), THREAD_PRIORITY_HIGHEST);
    }

    #[test]
    fn boost_follows_the_process_until_set() {
        let p = Priorities::<8>::new();
        assert!(!p.thread_boost(3));
        p.set_process_boost(true);
        assert!(p.thread_boost(3));
        p.set_thread_boost(3, false);
        assert!(!p.thread_boost(3));
        p.forget(3);
        assert!(p.thread_boost(3));
    }

    #[test]
    fn a_full_table_still_checks_and_reads_defaults() {
        let p = Priorities::<2>::new();
        p.set_thread(1, 2, false).unwrap();
        p.set_thread(2, -2, false).unwrap();
        p.set_thread(3, 1, false).unwrap();
        assert_eq!(p.set_thread(3, 7, false), Err(ERROR_INVALID_PARAMETER));
        assert_eq!((p.thread(1), p.thread(2), p.thread(3)), (2, -2, 0));
        p.forget(1);
        p.set_thread(3, 1, false).unwrap();
        assert_eq!(p.thread(3), 1);
    }

    #[test]
    fn classes() {
        let p = Priorities::<8>::new();
        assert_eq!(p.class(), NORMAL_PRIORITY_CLASS);
        p.set_class(BELOW_NORMAL_PRIORITY_CLASS, false).unwrap();
        assert_eq!(p.class(), BELOW_NORMAL_PRIORITY_CLASS);
        assert_eq!(p.set_class(0x10, true), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(p.set_class(PROCESS_MODE_BACKGROUND_BEGIN, false), Err(ERROR_INVALID_PARAMETER));
        p.set_class(PROCESS_MODE_BACKGROUND_BEGIN, true).unwrap();
        assert_eq!(p.set_class(PROCESS_MODE_BACKGROUND_BEGIN, true), Err(ERROR_PROCESS_MODE_ALREADY_BACKGROUND));
        assert_eq!(p.class(), BELOW_NORMAL_PRIORITY_CLASS);
        p.set_class(PROCESS_MODE_BACKGROUND_END, true).unwrap();
        assert_eq!(p.set_class(PROCESS_MODE_BACKGROUND_END, true), Err(ERROR_PROCESS_MODE_NOT_BACKGROUND));
        p.set_process_boost(true);
        p.set_class(HIGH_PRIORITY_CLASS, true).unwrap();
        assert!(p.process_boost());
        assert_eq!(p.class(), HIGH_PRIORITY_CLASS);
    }

    #[test]
    fn thread_information_sizes() {
        assert_eq!(check_thread_information(THREAD_MEMORY_PRIORITY, 4, Some(5)), Ok(()));
        assert_eq!(check_thread_information(THREAD_MEMORY_PRIORITY, 8, Some(5)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_thread_information(THREAD_POWER_THROTTLING, 12, Some(1)), Ok(()));
        assert_eq!(check_thread_information(THREAD_POWER_THROTTLING, 12, Some(2)), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(check_thread_information(9, 4, Some(0)), Err(ERROR_INVALID_PARAMETER));
    }
}
