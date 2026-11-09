//! The launch timer (ADR 0148): how long an app launched from the
//! launcher takes from the Enter key to its first window. A permanent,
//! cheap debug feature: two lines on the console per launch, so every
//! change that claims to make a program start faster has a number.
//!
//! ```text
//! poly-compositor: launched Chromium (pid 32) at 14231 ms
//! poly-compositor: launch-to-window Chromium: 5120 ms
//! ```
//!
//! The first window to show after a launch is the oldest pending launch's
//! (apps open one window first); a launch whose process ends before it
//! showed a window is dropped.

use crate::prelude::*;

/// Launches that have not shown a window yet, oldest first.
#[derive(Debug, Default)]
pub struct LaunchTimer {
    pending: Vec<(u32, String, u64)>,
}

impl LaunchTimer {
    /// `name` (process `pid`) was started at `now_ms`.
    pub fn started(&mut self, pid: u32, name: &str, now_ms: u64) {
        self.pending.push((pid, name.into(), now_ms));
    }

    /// A new window was shown at `now_ms`: the oldest pending launch's name
    /// and the milliseconds since it started.
    pub fn window_shown(&mut self, now_ms: u64) -> Option<(String, u64)> {
        if self.pending.is_empty() {
            return None;
        }
        let (_, name, at) = self.pending.remove(0);
        Some((name, now_ms.saturating_sub(at)))
    }

    /// Process `pid` ended: it will not show a window.
    pub fn ended(&mut self, pid: u32) {
        self.pending.retain(|p| p.0 != pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_window_is_the_oldest_launchs() {
        let mut t = LaunchTimer::default();
        assert_eq!(t.window_shown(5), None, "a window nobody launched");
        t.started(7, "Chromium", 1_000);
        t.started(9, "Terminal", 1_500);
        assert_eq!(t.window_shown(6_000), Some(("Chromium".into(), 5_000)));
        assert_eq!(t.window_shown(6_100), Some(("Terminal".into(), 4_600)));
        assert_eq!(t.window_shown(6_200), None);
    }

    #[test]
    fn a_launch_that_ended_shows_no_window() {
        let mut t = LaunchTimer::default();
        t.started(7, "Chromium", 10);
        t.started(9, "Terminal", 20);
        t.ended(7);
        assert_eq!(t.window_shown(100), Some(("Terminal".into(), 80)));
    }

    #[test]
    fn a_clock_that_went_back_counts_zero() {
        let mut t = LaunchTimer::default();
        t.started(1, "A", 50);
        assert_eq!(t.window_shown(40), Some(("A".into(), 0)));
    }
}
