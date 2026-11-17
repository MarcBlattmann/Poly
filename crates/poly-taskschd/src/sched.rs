//! When a task's triggers fire (ADR 0226, section 4; Learn: "Trigger Types", "Repetition
//! (repetitionType)", "ITrigger::StartBoundary").
//!
//! Each trigger that fires has a series of **starts**: a time trigger its start boundary, a daily
//! trigger the start boundary and every `DaysInterval` days after it, a logon, boot or
//! registration trigger the time of that event (after the trigger's `Delay`). Each start repeats
//! every `Repetition.Interval` for `Repetition.Duration` (for ever when there is an interval and no
//! duration). `StartBoundary` (when set) is the earliest and `EndBoundary` the latest time a trigger
//! fires. Weekly, monthly, idle, event and session triggers do not fire (yet).

use crate::def::{Definition, Trigger, trigger};
use crate::time::{parse_datetime, parse_duration};

/// The events logon, boot and registration triggers count from (seconds since 1970).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Events {
    pub boot: Option<i64>,
    pub logon: Option<i64>,
    pub registered: Option<i64>,
}

fn secs(t: &Option<alloc::string::String>) -> Option<i64> {
    t.as_deref().filter(|s| !s.is_empty()).and_then(parse_duration).map(|d| d as i64)
}

fn at(t: &Option<alloc::string::String>) -> Option<i64> {
    t.as_deref().filter(|s| !s.is_empty()).and_then(parse_datetime)
}

/// The first time after `after` that the repetitions of the start `s` reach, if any.
fn repeat_after(s: i64, interval: Option<i64>, duration: Option<i64>, after: i64) -> Option<i64> {
    if s > after {
        return Some(s);
    }
    let i = interval.filter(|&i| i > 0)?;
    let j = (after - s) / i + 1;
    let t = s + j * i;
    match duration {
        Some(d) if j * i >= d => None,
        _ => Some(t),
    }
}

/// The first time after `after` that one trigger fires.
fn trigger_after(t: &Trigger, ev: &Events, after: i64) -> Option<i64> {
    if t.enabled == 0 {
        return None;
    }
    let interval = secs(&t.repetition.interval);
    let duration = secs(&t.repetition.duration).filter(|&d| d > 0);
    let start = at(&t.start_boundary);
    let end = at(&t.end_boundary);
    let delay = secs(&t.delay).unwrap_or(0);
    let floor = start.unwrap_or(i64::MIN);
    let after = after.max(floor.saturating_sub(1));
    let next = match t.kind {
        trigger::TIME => repeat_after(start?, interval, duration, after),
        trigger::DAILY => {
            let s0 = start?;
            let step = i64::from(t.days_interval.max(1)) * 86_400;
            if after < s0 {
                Some(s0)
            } else {
                let k = (after - s0) / step;
                let mut best: Option<i64> = Some(s0 + (k + 1) * step);
                for kk in [k - 1, k] {
                    if kk < 0 {
                        continue;
                    }
                    if let Some(r) = repeat_after(s0 + kk * step, interval, duration, after) {
                        best = Some(best.map_or(r, |b| b.min(r)));
                    }
                }
                best
            }
        }
        trigger::LOGON => repeat_after(ev.logon? + delay, interval, duration, after),
        trigger::BOOT => repeat_after(ev.boot? + delay, interval, duration, after),
        trigger::REGISTRATION => repeat_after(ev.registered? + delay, interval, duration, after),
        _ => None,
    }?;
    match end {
        Some(e) if next > e => None,
        _ => Some(next),
    }
}

/// The first time after `after` that any trigger of `d` fires.
pub fn next_fire(d: &Definition, ev: &Events, after: i64) -> Option<i64> {
    d.triggers.iter().filter_map(|t| trigger_after(t, ev, after)).min()
}

/// How many times the triggers of `d` fire in `(from, to]` (at most 1000 are counted).
pub fn fires_between(d: &Definition, ev: &Events, from: i64, to: i64) -> u32 {
    let mut n = 0;
    let mut at = from;
    while n < 1000 {
        match next_fire(d, ev, at) {
            Some(t) if t <= to => {
                n += 1;
                at = t;
            }
            _ => break,
        }
    }
    n
}
