//! Parses the in-kernel test harness output (kernel/src/ktest.rs):
//!
//! ```text
//! test sync::spinlock_excludes_until_guard_dropped ... ok
//! test selftest::frame_allocator ... FAILED: out of frames
//! test result: FAILED. 4 passed; 1 failed; 0 filtered out
//! ```

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub passed: Vec<String>,
    /// (test name, reason)
    pub failed: Vec<(String, String)>,
    pub summary: Option<Summary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub passed: usize,
    pub failed: usize,
    pub filtered_out: usize,
}

pub fn parse(output: &str) -> Report {
    let mut r = Report::default();
    for line in output.lines().map(str::trim_end) {
        if let Some(rest) = line.strip_prefix("test result: ") {
            r.summary = parse_summary(rest);
        } else if let Some((name, outcome)) = line.strip_prefix("test ").and_then(|l| l.split_once(" ... ")) {
            if outcome == "ok" {
                r.passed.push(name.to_string());
            } else if let Some(why) = outcome.strip_prefix("FAILED") {
                r.failed.push((name.to_string(), why.trim_start_matches(':').trim().to_string()));
            }
        }
    }
    r
}

/// `ok. 5 passed; 0 failed; 2 filtered out`
fn parse_summary(s: &str) -> Option<Summary> {
    let (_verdict, counts) = s.split_once(". ")?;
    let mut n = [None; 3];
    for part in counts.split("; ") {
        let (count, what) = part.split_once(' ')?;
        let slot = ["passed", "failed", "filtered out"].iter().position(|w| *w == what)?;
        n[slot] = Some(count.parse().ok()?);
    }
    Some(Summary { passed: n[0]?, failed: n[1]?, filtered_out: n[2]? })
}

impl Report {
    /// Ok if the summary is there, agrees with the per-test lines, and no
    /// test failed.
    pub fn check(&self) -> Result<(), String> {
        let Some(s) = self.summary else {
            let last = self.passed.last().map_or(String::new(), |t| format!(" (last passing test: {t})"));
            return Err(format!("kernel test summary line missing: the kernel stopped during the tests{last}"));
        };
        if !self.failed.is_empty() {
            let names: Vec<_> = self.failed.iter().map(|(n, why)| format!("{n} ({why})")).collect();
            return Err(format!("{} kernel test(s) failed: {}", self.failed.len(), names.join(", ")));
        }
        if s.passed != self.passed.len() || s.failed != self.failed.len() {
            return Err(format!(
                "kernel test summary ({} passed, {} failed) disagrees with the {} per-test lines",
                s.passed,
                s.failed,
                self.passed.len() + self.failed.len()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSING: &str = "\
ktest: running 3 of 4 tests
test arch::x86_64::backtrace::walks_to_kernel_entry ... ok
breakpoint at 0xffffffff80107b18
test selftest::cpu_exceptions_return ... ok
exec: pid 1 Elf { bits: B64, arch: X86_64 } -> linux
test selftest::personality_routing ... ok
test result: ok. 3 passed; 0 failed; 1 filtered out
POLY-SELFTEST: PASS
";

    #[test]
    fn parses_a_passing_run() {
        let r = parse(PASSING);
        assert_eq!(r.passed.len(), 3);
        assert_eq!(r.passed[1], "selftest::cpu_exceptions_return");
        assert!(r.failed.is_empty());
        assert_eq!(r.summary, Some(Summary { passed: 3, failed: 0, filtered_out: 1 }));
        assert_eq!(r.check(), Ok(()));
    }

    #[test]
    fn reports_failed_tests_by_name() {
        let r = parse(
            "test a::one ... ok\ntest a::two ... FAILED: out of frames\n\
             test result: FAILED. 1 passed; 1 failed; 0 filtered out\n",
        );
        assert_eq!(r.failed, vec![("a::two".to_string(), "out of frames".to_string())]);
        let err = r.check().unwrap_err();
        assert!(err.contains("a::two (out of frames)"), "{err}");
    }

    #[test]
    fn missing_summary_is_an_error() {
        let r = parse("test a::one ... ok\n\nKERNEL PANIC: oops\n");
        assert_eq!(r.summary, None);
        let err = r.check().unwrap_err();
        assert!(err.contains("summary line missing") && err.contains("a::one"), "{err}");
    }

    #[test]
    fn summary_must_match_the_test_lines() {
        let r = parse("test a::one ... ok\ntest result: ok. 2 passed; 0 failed; 0 filtered out\n");
        assert!(r.check().unwrap_err().contains("disagrees"));
    }

    #[test]
    fn malformed_summary_is_ignored() {
        assert_eq!(parse_summary("ok. 3 passed; 0 failed"), None);
        assert_eq!(parse_summary("ok. x passed; 0 failed; 0 filtered out"), None);
        assert_eq!(parse_summary("ok. 3 passed; 0 failed; 0 skipped"), None);
    }
}
