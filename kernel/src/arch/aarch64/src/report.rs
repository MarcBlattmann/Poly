//! The self-test's output. One line per check, `ok <name>` or
//! `not ok <name> # <why>` (in the spirit of TAP), then a verdict line that
//! `cargo xtask aarch64 test` looks for, like the x86 kernel's
//! `POLY-SELFTEST: PASS`.

use core::fmt::{self, Write};

/// Printed when every check passed (and nothing is missing).
pub const PASS_MARKER: &str = "POLY-A64-SELFTEST: PASS";
/// Printed when any check failed.
pub const FAIL_MARKER: &str = "POLY-A64-SELFTEST: FAIL";

/// Counts the checks as they are reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub passed: u32,
    pub failed: u32,
}

impl Tally {
    /// Report one check to `out` and count it. `detail` is shown after a
    /// failure.
    pub fn check(&mut self, out: &mut impl Write, name: &str, ok: bool, detail: fmt::Arguments<'_>) -> fmt::Result {
        if ok {
            self.passed += 1;
            writeln!(out, "ok {name}")
        } else {
            self.failed += 1;
            writeln!(out, "not ok {name} # {detail}")
        }
    }

    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.passed > 0
    }

    /// The verdict line.
    pub fn summary(&self, out: &mut impl Write) -> fmt::Result {
        let marker = if self.all_passed() { PASS_MARKER } else { FAIL_MARKER };
        writeln!(out, "{marker} ({} passed, {} failed)", self.passed, self.failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;

    #[test]
    fn reports_ok_and_not_ok_lines() {
        let (mut t, mut out) = (Tally::default(), String::new());
        t.check(&mut out, "alpha", true, format_args!("unused")).unwrap();
        t.check(&mut out, "beta", false, format_args!("got {}", 7)).unwrap();
        assert_eq!(out, "ok alpha\nnot ok beta # got 7\n");
        assert_eq!(t, Tally { passed: 1, failed: 1 });
        assert!(!t.all_passed());
    }

    #[test]
    fn verdict_needs_a_passing_check_and_no_failure() {
        let mut out = String::new();
        Tally::default().summary(&mut out).unwrap();
        assert!(out.starts_with(FAIL_MARKER), "no checks is not a pass");
        let mut out = String::new();
        Tally { passed: 3, failed: 0 }.summary(&mut out).unwrap();
        assert_eq!(out, "POLY-A64-SELFTEST: PASS (3 passed, 0 failed)\n");
        let mut out = String::new();
        Tally { passed: 3, failed: 1 }.summary(&mut out).unwrap();
        assert!(out.starts_with(FAIL_MARKER));
    }
}
