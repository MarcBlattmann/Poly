//! Comparing a target's run of a test with the reference run, and the
//! per-test report and JSON summary of a diff run.
//!
//! Compared exactly: the exit status, stdout and stderr byte for byte.
//! Compared after normalisation (`trace::normalize`): the syscall and
//! signal sequence; the first divergence is reported, since everything
//! after it usually follows from it. Time is compared only as an envelope:
//! with `--timing-envelope F` the target may take at most F times as long
//! as the reference (plus 50 ms), and only when both sides measured it.

use crate::trace::{NormCall, NormOptions, Trace, normalize};
use crate::util::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyscallMode {
    /// Names, normalised arguments and results.
    Exact,
    /// Only the sequence of names (and signals).
    Names,
    /// Not compared.
    Off,
}

impl SyscallMode {
    pub fn parse(s: &str) -> Result<SyscallMode> {
        match s {
            "exact" => Ok(SyscallMode::Exact),
            "names" => Ok(SyscallMode::Names),
            "off" => Ok(SyscallMode::Off),
            _ => Err(format!("unknown syscall mode {s} (exact, names, off)")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub syscalls: SyscallMode,
    pub norm: NormOptions,
    pub timing_envelope: Option<f64>,
}

impl Default for Options {
    fn default() -> Options {
        Options { syscalls: SyscallMode::Exact, norm: NormOptions::default(), timing_envelope: None }
    }
}

/// Where two event sequences first differ.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    /// Index of the first differing event (0-based).
    pub index: usize,
    /// The reference's event there, `None` if its sequence ended.
    pub reference: Option<String>,
    pub target: Option<String>,
    /// Up to three matching events before it.
    pub context: Vec<String>,
}

/// First divergence of `target` from `reference`, if any.
pub fn first_divergence(reference: &[NormCall], target: &[NormCall], names_only: bool) -> Option<Divergence> {
    let n = reference.len().max(target.len());
    let index = (0..n).find(|&i| match (reference.get(i), target.get(i)) {
        (Some(a), Some(b)) => !a.matches(b, names_only),
        _ => true,
    })?;
    Some(Divergence {
        index,
        reference: reference.get(index).map(ToString::to_string),
        target: target.get(index).map(ToString::to_string),
        context: reference[index.saturating_sub(3)..index].iter().map(ToString::to_string).collect(),
    })
}

/// First line where two outputs differ: (1-based line, reference, target).
fn first_line_difference(a: &str, b: &str) -> Option<(usize, String, String)> {
    if a == b {
        return None;
    }
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.split_inclusive('\n').collect(), b.split_inclusive('\n').collect());
    let i = (0..la.len().max(lb.len())).find(|&i| la.get(i) != lb.get(i)).unwrap_or(0);
    let show = |l: Option<&&str>| l.map_or("<end of output>".to_string(), |s| format!("{s:?}"));
    Some((i + 1, show(la.get(i)), show(lb.get(i))))
}

/// Everything that differs between a reference and a target run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// Human-readable, one per difference.
    pub mismatches: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_divergence: Option<Divergence>,
    /// Whether the syscall sequences were compared at all.
    pub syscalls_compared: bool,
}

pub fn compare(reference: &Trace, target: &Trace, opts: &Options) -> Comparison {
    let mut c = Comparison::default();
    if reference.exit != target.exit {
        c.mismatches.push(format!("exit: reference {}, target {}", reference.exit, target.exit));
    }
    for (stream, a, b) in [("stdout", &reference.stdout, &target.stdout), ("stderr", &reference.stderr, &target.stderr)]
    {
        if let Some((line, ra, tb)) = first_line_difference(a, b) {
            c.mismatches.push(format!("{stream} line {line}: reference {ra}, target {tb}"));
        }
    }
    let traced = reference.tracer != "none" && target.tracer != "none";
    if opts.syscalls != SyscallMode::Off && traced {
        c.syscalls_compared = true;
        let (r, t) = (normalize(reference, &opts.norm), normalize(target, &opts.norm));
        if let Some(d) = first_divergence(&r, &t, opts.syscalls == SyscallMode::Names) {
            c.mismatches.push(format!(
                "syscalls differ at event {}: reference {}, target {}",
                d.index,
                d.reference.as_deref().unwrap_or("<end>"),
                d.target.as_deref().unwrap_or("<end>")
            ));
            c.first_divergence = Some(d);
        }
    }
    if let Some(factor) = opts.timing_envelope
        && reference.duration_ns > 0
        && target.duration_ns > 0
    {
        let limit = reference.duration_ns as f64 * factor + 50e6;
        if target.duration_ns as f64 > limit {
            c.mismatches.push(format!(
                "timing: target took {:.1} ms, outside the envelope of {factor}x the reference's {:.1} ms",
                target.duration_ns as f64 / 1e6,
                reference.duration_ns as f64 / 1e6
            ));
        }
    }
    c
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    /// The target cannot run the test yet (Poly without user programs).
    Unsupported,
    /// The run itself went wrong (tool, build or boot problem).
    Error,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "FAIL",
            Verdict::Unsupported => "unsupported",
            Verdict::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestResult {
    pub test: String,
    pub verdict: Verdict,
    /// Why the verdict is unsupported or error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_exit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_exit: Option<String>,
    pub reference_syscalls: usize,
    pub target_syscalls: usize,
    #[serde(default)]
    pub comparison: Comparison,
    /// Trace files, relative to the run directory.
    #[serde(default)]
    pub files: Vec<String>,
}

impl TestResult {
    pub fn not_run(test: &str, verdict: Verdict, reason: String) -> TestResult {
        TestResult {
            test: test.into(),
            verdict,
            reason: Some(reason),
            reference_exit: None,
            target_exit: None,
            reference_syscalls: 0,
            target_syscalls: 0,
            comparison: Comparison::default(),
            files: Vec::new(),
        }
    }

    pub fn compared(test: &str, reference: &Trace, target: &Trace, opts: &Options) -> TestResult {
        let comparison = compare(reference, target, opts);
        TestResult {
            test: test.into(),
            verdict: if comparison.mismatches.is_empty() { Verdict::Pass } else { Verdict::Fail },
            reason: None,
            reference_exit: Some(reference.exit.to_string()),
            target_exit: Some(target.exit.to_string()),
            reference_syscalls: reference.syscalls().count(),
            target_syscalls: target.syscalls().count(),
            comparison,
            files: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub format: String,
    pub started: String,
    pub target: String,
    pub reference: String,
    pub tracer: String,
    pub syscalls: SyscallMode,
    pub totals: BTreeMap<String, usize>,
    pub tests: Vec<TestResult>,
    /// Tests that passed in the baseline and do not pass now.
    #[serde(default)]
    pub regressions: Vec<String>,
}

impl Summary {
    pub fn new(started: String, target: &str, reference: String, tracer: &str, syscalls: SyscallMode) -> Summary {
        Summary {
            format: "poly-diff/1".into(),
            started,
            target: target.into(),
            reference,
            tracer: tracer.into(),
            syscalls,
            totals: BTreeMap::new(),
            tests: Vec::new(),
            regressions: Vec::new(),
        }
    }

    pub fn finish(&mut self, baseline: Option<&Summary>) {
        self.totals.clear();
        for v in [Verdict::Pass, Verdict::Fail, Verdict::Unsupported, Verdict::Error] {
            let n = self.tests.iter().filter(|t| t.verdict == v).count();
            self.totals.insert(v.name().to_lowercase(), n);
        }
        self.regressions = match baseline {
            Some(b) => b
                .tests
                .iter()
                .filter(|old| old.verdict == Verdict::Pass)
                .filter(|old| self.tests.iter().any(|t| t.test == old.test && t.verdict != Verdict::Pass))
                .map(|old| old.test.clone())
                .collect(),
            None => Vec::new(),
        };
    }

    /// Whether the run should fail: regressions against a baseline, or
    /// without one any failure or error (and with `strict` also
    /// unsupported tests).
    pub fn failed(&self, has_baseline: bool, strict: bool) -> bool {
        if has_baseline {
            return !self.regressions.is_empty();
        }
        self.tests.iter().any(|t| match t.verdict {
            Verdict::Pass => false,
            Verdict::Unsupported => strict,
            Verdict::Fail | Verdict::Error => true,
        })
    }

    /// The per-test table.
    pub fn table(&self) -> String {
        let mut rows = vec![[
            "test".to_string(),
            "result".into(),
            "exit (ref / target)".into(),
            "syscalls (ref / target)".into(),
            "details".into(),
        ]];
        for t in &self.tests {
            let exit = match (&t.reference_exit, &t.target_exit) {
                (Some(a), Some(b)) => format!("{a} / {b}"),
                _ => "-".into(),
            };
            let calls = if t.comparison.syscalls_compared {
                format!("{} / {}", t.reference_syscalls, t.target_syscalls)
            } else {
                "-".into()
            };
            let details =
                t.reason.clone().or_else(|| t.comparison.mismatches.first().cloned()).unwrap_or_else(|| "same".into());
            rows.push([t.test.clone(), t.verdict.name().into(), exit, calls, details]);
        }
        let widths: Vec<usize> = (0..4).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap()).collect();
        let mut out = String::new();
        for r in &rows {
            for (cell, w) in r.iter().zip(&widths) {
                let _ = write!(out, "{cell:<w$}  ");
            }
            out.push_str(&r[4]);
            out.push('\n');
        }
        let totals: Vec<String> = self.totals.iter().map(|(k, v)| format!("{v} {k}")).collect();
        let _ = writeln!(out, "\n{} vs {} ({}): {}", self.target, self.reference, self.tracer, totals.join(", "));
        if !self.regressions.is_empty() {
            let _ = writeln!(out, "regressions against the baseline: {}", self.regressions.join(", "));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::tests::call;
    use crate::trace::{Arg, Event, Exit};

    fn hello(text: &str, written: i64) -> Trace {
        let mut t = Trace::new("hello", "linux", "ptrace");
        t.exit = Exit::Code(0);
        t.stdout = text.into();
        t.events = vec![
            call("brk", vec![Arg::Ptr(0)], Some(0x4000_0000)),
            call("write", vec![Arg::Int(1), Arg::Str(format!("{text:?}")), Arg::Int(13)], Some(written)),
            call("exit_group", vec![Arg::Int(0)], None),
        ];
        t
    }

    #[test]
    fn identical_runs_pass() {
        let a = hello("hello, world\n", 13);
        let mut b = a.clone();
        // Addresses differ between runs; normalisation hides that.
        b.events[0] = call("brk", vec![Arg::Ptr(0)], Some(0x5555_0000));
        let r = TestResult::compared("hello", &a, &b, &Options::default());
        assert_eq!(r.verdict, Verdict::Pass, "{:?}", r.comparison);
        assert_eq!((r.reference_syscalls, r.target_syscalls), (3, 3));
    }

    #[test]
    fn first_divergence_is_reported() {
        let a = hello("hello, world\n", 13);
        let b = hello("hello, world\n", -14);
        let c = compare(&a, &b, &Options::default());
        let d = c.first_divergence.unwrap();
        assert_eq!(d.index, 1);
        assert_eq!(d.target.as_deref(), Some("write(1, \"hello, world\\n\", 13) = -1 EFAULT"));
        assert_eq!(d.context, ["brk(NULL) = <addr>"]);
        // Names only: the same.
        let names = Options { syscalls: SyscallMode::Names, ..Options::default() };
        assert!(compare(&a, &b, &names).mismatches.is_empty());
    }

    #[test]
    fn missing_events_diverge_at_the_end() {
        let a = hello("x", 1);
        let mut b = a.clone();
        b.events.pop();
        let d = compare(&a, &b, &Options::default()).first_divergence.unwrap();
        assert_eq!((d.index, d.target), (2, None));
    }

    #[test]
    fn exit_and_output_are_exact() {
        let a = hello("hello, world\n", 13);
        let mut b = a.clone();
        b.exit = Exit::Signal(11);
        b.stdout = "hello, World\n".into();
        b.stderr = "oops".into();
        let c = compare(&a, &b, &Options { syscalls: SyscallMode::Off, ..Options::default() });
        assert_eq!(
            c.mismatches,
            [
                "exit: reference exit 0, target SIGSEGV",
                "stdout line 1: reference \"hello, world\\n\", target \"hello, World\\n\"",
                "stderr line 1: reference <end of output>, target \"oops\"",
            ]
        );
        assert!(!c.syscalls_compared);
    }

    #[test]
    fn untraced_runs_skip_the_syscall_comparison() {
        let a = hello("x", 1);
        let mut b = a.clone();
        b.tracer = "none".into();
        b.events.clear();
        assert!(compare(&a, &b, &Options::default()).mismatches.is_empty());
    }

    #[test]
    fn timing_envelope() {
        let mut a = hello("x", 1);
        let mut b = a.clone();
        a.duration_ns = 1_000_000;
        b.duration_ns = 40_000_000;
        let opts = Options { timing_envelope: Some(10.0), ..Options::default() };
        assert!(compare(&a, &b, &opts).mismatches.is_empty(), "within 10x + 50 ms");
        b.duration_ns = 100_000_000;
        assert!(compare(&a, &b, &opts).mismatches[0].starts_with("timing:"));
        b.duration_ns = 0;
        assert!(compare(&a, &b, &opts).mismatches.is_empty(), "not measured: not compared");
    }

    #[test]
    fn signals_are_part_of_the_sequence() {
        let mut a = hello("x", 1);
        let b = a.clone();
        a.events.insert(2, Event::Signal { pid: 0, signal: 11 });
        let d = compare(&a, &b, &Options::default()).first_divergence.unwrap();
        assert_eq!(d.reference.as_deref(), Some("--- SIGSEGV ---"));
    }

    fn summary(verdicts: &[(&str, Verdict)]) -> Summary {
        let mut s = Summary::new("now".into(), "poly", "linux".into(), "ptrace", SyscallMode::Exact);
        s.tests = verdicts.iter().map(|(n, v)| TestResult::not_run(n, *v, String::new())).collect();
        s
    }

    #[test]
    fn regressions_and_exit_policy() {
        let base = summary(&[("a", Verdict::Pass), ("b", Verdict::Fail), ("c", Verdict::Pass)]);
        let mut now = summary(&[("a", Verdict::Pass), ("b", Verdict::Fail), ("c", Verdict::Unsupported)]);
        now.finish(Some(&base));
        assert_eq!(now.regressions, ["c"]);
        assert!(now.failed(true, false));
        assert_eq!(now.totals["fail"], 1);

        let mut ok = summary(&[("a", Verdict::Pass), ("b", Verdict::Unsupported)]);
        ok.finish(None);
        assert!(!ok.failed(false, false));
        assert!(ok.failed(false, true), "strict: unsupported fails");
        let table = ok.table();
        assert!(table.contains("unsupported") && table.contains("1 pass"), "{table}");
    }

    #[test]
    fn summary_json_roundtrip() {
        let mut s = summary(&[("a", Verdict::Pass)]);
        s.finish(None);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"verdict\":\"pass\""));
        assert_eq!(serde_json::from_str::<Summary>(&json).unwrap(), s);
    }
}
