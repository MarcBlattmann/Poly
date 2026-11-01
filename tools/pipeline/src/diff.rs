//! Comparing a target's run of a test with the reference run, and the
//! per-test report and JSON summary of a diff run.
//!
//! Compared exactly: the exit status, stdout and stderr byte for byte.
//! Compared after normalisation (`trace::normalize`): the syscall and
//! signal sequence; the first divergence is reported, since everything
//! after it usually follows from it. Time is compared only as an envelope:
//! with `--timing-envelope F` the target may take at most F times as long
//! as the reference (plus 50 ms), and only when both sides measured it.
//!
//! App tests (`apps.rs`) compare their syscalls in the looser [`App`]
//! mode ([`app_events`]): an unchanged program's C library start-up and
//! its reads of host files (`/proc/mounts`, `/proc/self/maps`) depend on
//! the system in ways that do not change what the program does, so only
//! the shape of each process's sequence is compared.
//!
//! [`App`]: SyscallMode::App

use crate::identity::Identity;
use crate::trace::{Exit, NormCall, NormOptions, Trace, normalize};
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
    /// Per process: names and result classes, repeats collapsed; for app
    /// tests ([`app_events`]).
    App,
}

impl SyscallMode {
    pub fn parse(s: &str) -> Result<SyscallMode> {
        match s {
            "exact" => Ok(SyscallMode::Exact),
            "names" => Ok(SyscallMode::Names),
            "off" => Ok(SyscallMode::Off),
            "app" => Ok(SyscallMode::App),
            _ => Err(format!("unknown syscall mode {s} (exact, names, off, app)")),
        }
    }

    /// The weaker of two modes: a test's own directive can relax the run's
    /// mode, never tighten it.
    pub fn weaker(self, other: SyscallMode) -> SyscallMode {
        // App and Names are equally strong for this purpose (neither
        // compares less than the other in every respect); a tie keeps the
        // run's mode.
        let rank = |m: SyscallMode| match m {
            SyscallMode::Exact => 0,
            SyscallMode::Names | SyscallMode::App => 1,
            SyscallMode::Off => 2,
        };
        if rank(other) > rank(self) { other } else { self }
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
    first_divergence_by(reference, target, |a, b| a.matches(b, names_only))
}

/// First divergence where `same` says two events differ.
fn first_divergence_by(
    reference: &[NormCall],
    target: &[NormCall],
    same: impl Fn(&NormCall, &NormCall) -> bool,
) -> Option<Divergence> {
    let n = reference.len().max(target.len());
    let index = (0..n).find(|&i| match (reference.get(i), target.get(i)) {
        (Some(a), Some(b)) => !same(a, b),
        _ => true,
    })?;
    Some(Divergence {
        index,
        reference: reference.get(index).map(ToString::to_string),
        target: target.get(index).map(ToString::to_string),
        context: reference[index.saturating_sub(3)..index].iter().map(ToString::to_string).collect(),
    })
}

/// Calls whose result is a file descriptor: app mode compares it exactly
/// (which descriptor a call gets is the lowest free one, open(2)).
const FD_RESULTS: &[&str] = &[
    "open",
    "openat",
    "openat2",
    "creat",
    "dup",
    "dup2",
    "dup3",
    "socket",
    "accept",
    "accept4",
    "eventfd2",
    "epoll_create1",
    "memfd_create",
    "inotify_init1",
    "signalfd4",
    "timerfd_create",
    "pidfd_open",
];

/// Calls app mode leaves out: `rseq`, which Poly answered with `ENOSYS`
/// until ADR 0111 (ADR 0043, ADR 0049) and after which a C library behaves
/// the same either way; still left out so app tests do not depend on it.
pub const APP_DROPPED_CALLS: &[&str] = &["rseq"];

/// Signals app mode leaves out: `SIGCHLD`, whose place in the parent's
/// sequence depends on when the child ends.
pub const APP_DROPPED_SIGNALS: &[&str] = &["--- SIGCHLD ---"];

/// Longest argument shown in app mode (arguments are not compared there).
const APP_ARG_WIDTH: usize = 48;

/// The event sequence app mode compares:
///
/// 1. each process's events in its own order, process 0 first (so the
///    interleaving of a parent waiting for a child does not matter);
/// 2. without [`APP_DROPPED_CALLS`] and [`APP_DROPPED_SIGNALS`];
/// 3. each call reduced to its name and result class: `ok`, `-1 ERRNO`,
///    `?` (did not return) or, for calls returning a descriptor, the
///    descriptor; arguments are kept for the report only;
/// 4. consecutive events of the same process, name and class collapsed
///    into one (a read loop over a file of another size, the same signal
///    action set for many signals).
pub fn app_events(trace: &Trace, norm: &NormOptions) -> Vec<NormCall> {
    let mut events: Vec<NormCall> = normalize(trace, norm)
        .into_iter()
        .filter(|e| !APP_DROPPED_CALLS.contains(&e.name.as_str()) && !APP_DROPPED_SIGNALS.contains(&e.name.as_str()))
        .map(|mut e| {
            if !e.ret.is_empty() && e.ret != "?" && !e.ret.starts_with("-1 ") && !FD_RESULTS.contains(&e.name.as_str())
            {
                e.ret = "ok".into();
            }
            for a in &mut e.args {
                if a.chars().count() > APP_ARG_WIDTH {
                    *a = format!("{}...", a.chars().take(APP_ARG_WIDTH - 3).collect::<String>());
                }
            }
            e
        })
        .collect();
    events.sort_by_key(|e| e.pid);
    events.dedup_by(|b, a| app_same(a, b));
    events
}

/// Equal in app mode: process, name and result class.
fn app_same(a: &NormCall, b: &NormCall) -> bool {
    a.pid == b.pid && a.name == b.name && a.ret == b.ret
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
        let divergence = if opts.syscalls == SyscallMode::App {
            first_divergence_by(&app_events(reference, &opts.norm), &app_events(target, &opts.norm), app_same)
        } else {
            let (r, t) = (normalize(reference, &opts.norm), normalize(target, &opts.norm));
            first_divergence(&r, &t, opts.syscalls == SyscallMode::Names)
        };
        if let Some(d) = divergence {
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
    /// The syscall mode of this test, where it is not the run's (app tests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syscall_mode: Option<SyscallMode>,
    /// Trace files, relative to the run directory.
    #[serde(default)]
    pub files: Vec<String>,
    /// Not compared with the reference but checked on the target as root
    /// alone ([`TestResult::self_checked`]): the test needs root and the
    /// reference did not run as root.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub self_checked: bool,
    /// Reference runs that timed out: each was run again, up to
    /// `runner::TIMEOUT_RERUNS` times (ADR 0085). Above that many, the
    /// last one timed out too.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reference_timeouts: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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
            syscall_mode: None,
            files: Vec::new(),
            self_checked: false,
            reference_timeouts: 0,
        }
    }

    /// A test that needs root, where the reference is not root: run on
    /// the target as root alone, it passes if it passes its own checks
    /// (exit status 0; the corpus format's tests print `ok` / `not ok` and
    /// exit 0 when every check passed). `why` says why it was not compared.
    pub fn self_checked(test: &str, target: &Trace, why: &str) -> TestResult {
        let passed = target.exit == Exit::Code(0);
        let verdict = if passed { Verdict::Pass } else { Verdict::Fail };
        let outcome = if passed { "passed its own checks" } else { "FAILED its own checks" };
        let mut r = TestResult::not_run(test, verdict, format!("{why}; not compared: {outcome} on the target as root"));
        r.target_exit = Some(target.exit.to_string());
        r.target_syscalls = target.syscalls().count();
        r.self_checked = true;
        r
    }

    pub fn compared(test: &str, reference: &Trace, target: &Trace, opts: &Options) -> TestResult {
        let comparison = compare(reference, target, opts);
        // The reference always finishes: if it did not, the test or the
        // tracer is broken, whatever the target did.
        let (verdict, reason) = if reference.exit == Exit::Timeout {
            (Verdict::Error, Some("the reference run timed out (a broken test, or the tracer)".to_string()))
        } else if comparison.mismatches.is_empty() {
            (Verdict::Pass, None)
        } else {
            (Verdict::Fail, None)
        };
        TestResult {
            test: test.into(),
            verdict,
            reason,
            reference_exit: Some(reference.exit.to_string()),
            target_exit: Some(target.exit.to_string()),
            reference_syscalls: reference.syscalls().count(),
            target_syscalls: target.syscalls().count(),
            comparison,
            syscall_mode: None,
            files: Vec::new(),
            self_checked: false,
            reference_timeouts: 0,
        }
    }

    /// Record that `n` reference runs timed out: before the one compared
    /// here, or, where that one timed out too, including it.
    pub fn with_reference_timeouts(mut self, n: usize) -> TestResult {
        self.reference_timeouts = n;
        if n > 1 && self.verdict == Verdict::Error && self.reference_exit.as_deref() == Some("timeout") {
            self.reason =
                Some(format!("the reference run timed out {n} times in a row (a broken test, or the tracer)"));
        }
        self
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
    /// Who the reference ran the test programs as, and the target too
    /// (`identity.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<Identity>,
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
            identity: None,
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
        if let Some(id) = &self.identity {
            let _ = writeln!(out, "test programs ran as {} on both sides", id.describe());
        }
        if !self.regressions.is_empty() {
            let _ = writeln!(out, "regressions against the baseline: {}", self.regressions.join(", "));
        }
        let rerun: Vec<&str> = self
            .tests
            .iter()
            .filter(|t| t.reference_timeouts > 0 && t.reference_exit.as_deref() != Some("timeout"))
            .map(|t| t.test.as_str())
            .collect();
        if !rerun.is_empty() {
            let _ = writeln!(
                out,
                "the reference run timed out and was run again (a test that can hang, ADR 0085): {}",
                rerun.join(", ")
            );
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
    fn a_directive_only_weakens_the_mode() {
        use SyscallMode::*;
        assert_eq!(Exact.weaker(Off), Off);
        assert_eq!(Off.weaker(Exact), Off);
        assert_eq!(Names.weaker(Exact), Names);
        assert_eq!(Exact.weaker(Names), Names);
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

    /// A test checked on the target alone passes on exit 0 and fails
    /// otherwise, says why it was not compared, and the summary shows who
    /// the programs ran as.
    #[test]
    fn self_checked_tests_pass_on_their_own_checks() {
        let mut t = hello("hello, world\n", 13);
        let r = TestResult::self_checked("mount_root", &t, "needs root, the reference runs as uid 1000");
        assert_eq!((r.verdict, r.self_checked, r.target_exit.as_deref()), (Verdict::Pass, true, Some("exit 0")));
        assert!(r.reason.as_deref().unwrap().contains("passed its own checks on the target as root"), "{r:?}");
        t.exit = Exit::Code(1);
        let r = TestResult::self_checked("mount_root", &t, "needs root");
        assert_eq!(r.verdict, Verdict::Fail);
        let mut s = Summary::new("now".into(), "poly", "linux".into(), "ptrace", SyscallMode::Exact);
        s.identity = Some(Identity { uid: [1000; 3], gid: [1000; 3], groups: vec![1000], umask: 0o022 });
        s.tests.push(r);
        s.finish(None);
        assert!(s.failed(false, false));
        assert!(s.table().contains("ran as uid 1000 gid 1000 groups 1000 umask 0022 on both sides"), "{}", s.table());
        let back: Summary = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
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
    fn a_reference_timeout_is_an_error_even_when_the_target_agrees() {
        let mut a = hello("x", 1);
        a.exit = Exit::Timeout;
        let b = a.clone();
        let r = TestResult::compared("t", &a, &b, &Options::default());
        assert_eq!(r.verdict, Verdict::Error);
        assert!(r.reason.unwrap().contains("timed out"));
        let mut c = hello("x", 1);
        c.exit = Exit::Timeout;
        assert_eq!(TestResult::compared("t", &hello("x", 1), &c, &Options::default()).verdict, Verdict::Fail);
    }

    /// A reference that timed out and passed when run again is compared as
    /// usual, and the report names it; one that timed out every time is an
    /// error that says so (ADR 0085).
    #[test]
    fn reference_reruns_after_a_timeout_are_reported() {
        let a = hello("x", 1);
        let rerun = TestResult::compared("ctty", &a, &a.clone(), &Options::default()).with_reference_timeouts(1);
        assert_eq!((rerun.verdict, rerun.reason.as_deref(), rerun.reference_timeouts), (Verdict::Pass, None, 1));
        let mut hung = a.clone();
        hung.exit = Exit::Timeout;
        let twice = TestResult::compared("hangs", &hung, &a, &Options::default()).with_reference_timeouts(2);
        assert_eq!(twice.verdict, Verdict::Error);
        assert!(twice.reason.as_deref().unwrap().contains("timed out 2 times in a row"), "{twice:?}");
        let mut s = Summary::new("now".into(), "poly", "linux".into(), "ptrace", SyscallMode::Exact);
        s.tests = vec![rerun, twice, TestResult::compared("plain", &a, &a.clone(), &Options::default())];
        s.finish(None);
        let table = s.table();
        assert!(table.contains("timed out and was run again (a test that can hang, ADR 0085): ctty\n"), "{table}");
        let back: Summary = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Not written when zero, so older summaries read the same.
        assert!(!serde_json::to_string(&s.tests[2]).unwrap().contains("reference_timeouts"));
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

    fn app_trace(pid1_first: bool, reads: usize, rseq: i64) -> Trace {
        let mut t = Trace::new("app", "linux", "ptrace");
        t.exit = Exit::Code(0);
        let mut ev = vec![
            call("rseq", vec![Arg::Ptr(1), Arg::Int(32), Arg::Hex(0), Arg::Hex(0x5305_3053)], Some(rseq)),
            call(
                "openat",
                vec![Arg::Int(-100), Arg::Str("\"/proc/mounts\"".into()), Arg::Hex(0), Arg::Hex(0)],
                Some(3),
            ),
        ];
        for i in 0..reads {
            ev.push(call("read", vec![Arg::Int(3), Arg::Ptr(1), Arg::Int(1024 - i as i64)], Some(100)));
        }
        ev.push(call("read", vec![Arg::Int(3), Arg::Ptr(1), Arg::Int(1024)], Some(0)));
        let mut child = call("execve", vec![Arg::Str("\"/bin/cat\"".into()), Arg::Ptr(1), Arg::Ptr(1)], Some(0));
        if let Event::Syscall(c) = &mut child {
            c.pid = 1;
        }
        let wait = call("wait4", vec![Arg::Pid(5), Arg::Ptr(0), Arg::Hex(0), Arg::Ptr(0)], Some(5));
        if pid1_first {
            ev.extend([child, Event::Signal { pid: 0, signal: 17 }, wait]);
        } else {
            ev.extend([wait, child]);
        }
        ev.push(call("exit_group", vec![Arg::Int(0)], None));
        t.events = ev;
        t
    }

    #[test]
    fn app_mode_compares_the_shape_per_process() {
        let a = app_trace(true, 3, 0);
        let b = app_trace(false, 1, -38);
        let app = Options { syscalls: SyscallMode::App, ..Options::default() };
        assert!(compare(&a, &b, &app).mismatches.is_empty(), "{:?}", compare(&a, &b, &app));
        assert!(!compare(&a, &b, &Options::default()).mismatches.is_empty(), "exact mode sees it");
        let lines: Vec<String> = app_events(&a, &NormOptions::default()).iter().map(ToString::to_string).collect();
        assert_eq!(
            lines,
            [
                "openat(-100, \"/proc/mounts\", 0x0, 0x0) = 3",
                "read(3, <ptr>, 1024) = ok",
                "wait4(<pid>, NULL, 0x0, NULL) = ok",
                "exit_group(0) = ?",
                "[p1] execve(\"/bin/cat\", <ptr>, <ptr>) = ok",
            ]
        );
        // Another descriptor, or an error instead of success, is a divergence.
        let mut c = app_trace(true, 1, 0);
        if let Event::Syscall(s) = &mut c.events[1] {
            s.ret = Some(4);
        }
        let d = compare(&a, &c, &app).first_divergence.unwrap();
        assert_eq!(d.target.as_deref(), Some("openat(-100, \"/proc/mounts\", 0x0, 0x0) = 4"));
        if let Event::Syscall(s) = &mut c.events[1] {
            s.ret = Some(3);
        }
        if let Event::Syscall(s) = &mut c.events[2] {
            s.ret = Some(-2);
        }
        let d = compare(&a, &c, &app).first_divergence.unwrap();
        assert_eq!((d.index, d.target.as_deref()), (1, Some("read(3, <ptr>, 1024) = -1 ENOENT")));
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
