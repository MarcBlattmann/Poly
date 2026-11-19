//! When a task's triggers fire (ADR 0226, section 4; Learn: "Trigger Types", "Repetition
//! (repetitionType)", "ITrigger::StartBoundary").
//!
//! Each trigger that fires has a series of **starts**: a time trigger its start boundary, a daily
//! trigger the start boundary and every `DaysInterval` days after it, a weekly, monthly or monthly
//! day-of-week trigger the days its schedule names at the start boundary's time of day (Learn:
//! "IWeeklyTrigger", "IMonthlyTrigger", "IMonthlyDOWTrigger"), a logon, boot or registration
//! trigger the time of that event (after the trigger's `Delay`). Each start repeats
//! every `Repetition.Interval` for `Repetition.Duration` (for ever when there is an interval and no
//! duration). `StartBoundary` (when set) is the earliest and `EndBoundary` the latest time a trigger
//! fires. Idle, event and session triggers do not fire (yet).

use crate::def::{Definition, Trigger, trigger};
use crate::time::{civil_from_days, days_in_month, parse_datetime, parse_duration};

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

/// The day of the week of a day number (0 Sunday; 1970-01-01 was a Thursday).
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

/// Whether a calendar trigger whose first day is `first` runs on the day `day`.
fn runs_on(t: &Trigger, first: i64, day: i64) -> bool {
    if day < first {
        return false;
    }
    let wd = weekday(day);
    let (_, m, d) = civil_from_days(day);
    let in_month = t.months_of_year & (1 << (m - 1)) != 0;
    match t.kind {
        trigger::WEEKLY => {
            // Weeks count from the week (Sunday to Saturday) of the first day.
            let week = (day - wd - (first - weekday(first))) / 7;
            t.days_of_week & (1 << wd) != 0 && week % i64::from(t.weeks_interval.max(1)) == 0
        }
        trigger::MONTHLY => {
            let (y, _, _) = civil_from_days(day);
            in_month
                && (t.days_of_month & (1 << (d - 1)) != 0
                    || (t.run_on_last_day_of_month != 0 && d == days_in_month(y, m)))
        }
        trigger::MONTHLYDOW => {
            let (y, _, _) = civil_from_days(day);
            let week = (d - 1) / 7;
            let last = d + 7 > days_in_month(y, m);
            in_month
                && t.days_of_week & (1 << wd) != 0
                && ((week < 4 && t.weeks_of_month & (1 << week) != 0) || (t.run_on_last_week_of_month != 0 && last))
        }
        _ => false,
    }
}

/// The first time after `after` a weekly or monthly trigger starting at `s0` fires (the days are
/// looked at up to 28 years ahead, a whole cycle of the calendar).
fn calendar_after(t: &Trigger, s0: i64, interval: Option<i64>, duration: Option<i64>, after: i64) -> Option<i64> {
    let first = s0.div_euclid(86_400);
    let time = s0.rem_euclid(86_400);
    let from = first.max(after.div_euclid(86_400) - 1);
    let mut best: Option<i64> = None;
    // The repetitions of the last day that ran before `after` may still be going on.
    if interval.is_some() {
        for day in (first..from).rev().take(400) {
            if runs_on(t, first, day) {
                best = repeat_after(day * 86_400 + time, interval, duration, after);
                break;
            }
        }
    }
    for day in from..from + 28 * 366 {
        let s = day * 86_400 + time;
        if best.is_some_and(|b| s >= b) {
            break;
        }
        if runs_on(t, first, day)
            && let Some(r) = repeat_after(s, interval, duration, after)
        {
            best = Some(best.map_or(r, |b| b.min(r)));
        }
    }
    best
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
        trigger::WEEKLY | trigger::MONTHLY | trigger::MONTHLYDOW => {
            calendar_after(t, start?, interval, duration, after)
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
