//! The cycle with fake commands: a small model of the hub, the role
//! checkouts and the PRs, with scripted agents, CI and diff results, and
//! simulated crashes (a panic at the n-th call of an operation; the next
//! "process" is a new driver over the same state directory and world).

use super::config::Config;
use super::cycle::{Driver, Logger, Pass, Yielded};
use super::ops::{DiffResult, Merge, Ops, Out, Pr, Push, Remote, SpecFiles};
use super::real::{DryShell, RealOps};
use super::safety::{Change, coverage_file};
use super::state::{Step, Store};
use crate::roles::Role;
use crate::store::EntryId;
use crate::util::Result;
use std::collections::{BTreeMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

const BRK: &str = "linux/syscall/x86_64/brk";
const T0: u64 = 1_792_454_400;

/// What an agent session does.
#[derive(Clone, Debug)]
enum Run {
    Commit(Vec<Change>),
    /// Commits on top of the parent of its base: a rewritten history.
    Rewrite,
    Limit(String),
    Fail,
    Nothing,
}

#[derive(Clone, Debug)]
struct Commit {
    parent: Option<String>,
    subject: String,
    changes: Vec<Change>,
    green: bool,
}

#[derive(Debug, Default)]
struct World {
    now: u64,
    n: u32,
    commits: BTreeMap<String, Commit>,
    hub: BTreeMap<String, String>,
    origin: BTreeMap<String, String>,
    slots: BTreeMap<usize, String>,
    staged: Vec<Change>,
    roles: BTreeMap<String, String>,
    queue: Vec<String>,
    spec_runs: VecDeque<Run>,
    impl_runs: VecDeque<Run>,
    ci: VecDeque<Result<i32>>,
    diffs: VecDeque<bool>,
    merges: VecDeque<Merge>,
    verify_ok: bool,
    spec_files: SpecFiles,
    prs: BTreeMap<String, Pr>,
    prs_created: u32,
    feedback: Vec<String>,
    feedback_given: Vec<Option<String>>,
    human: String,
    notified: Vec<String>,
    calls: Vec<String>,
    crash: Option<(String, u32)>,
    entry: Option<String>,
}

fn change(path: &str, added: &[&str]) -> Change {
    Change { path: path.into(), added: added.iter().map(|s| s.to_string()).collect(), ..Change::default() }
}

fn good_impl() -> Run {
    Run::Commit(vec![change("personalities/linux/src/brk.rs", &["pub fn brk() -> usize { 0 }"])])
}

fn good_spec() -> Run {
    Run::Commit(vec![
        change("specs/linux/x86_64/brk.yaml", &["id: linux/syscall/x86_64/brk"]),
        change("specs/linux/x86_64/brk.tests/brk_grow.c", &["int test_main(void) { return 0; }"]),
    ])
}

impl World {
    fn new() -> World {
        let mut w = World {
            now: T0,
            queue: vec![BRK.into()],
            verify_ok: true,
            spec_files: SpecFiles { spec: true, visible: 7, held_out: 3 },
            ..World::default()
        };
        w.commits.insert("main0".into(), Commit { parent: None, subject: "main".into(), changes: vec![], green: false });
        w
    }

    /// Record a call; panic at the configured one (a crash).
    fn call(&mut self, name: &str) {
        self.calls.push(name.to_string());
        if let Some((at, n)) = &mut self.crash
            && at == name
        {
            *n -= 1;
            if *n == 0 {
                self.crash = None;
                panic!("simulated crash at {name}");
            }
        }
    }

    fn count(&self, name: &str) -> usize {
        self.calls.iter().filter(|c| *c == name).count()
    }

    fn new_commit(&mut self, parent: Option<String>, subject: &str, changes: Vec<Change>) -> String {
        self.n += 1;
        let sha = format!("c{:03}", self.n);
        let green = subject.contains(" is green ");
        self.commits.insert(sha.clone(), Commit { parent, subject: subject.into(), changes, green });
        sha
    }

    fn resolve(&self, rev: &str) -> Result<String> {
        if rev == "origin/main" {
            return Ok("main0".into());
        }
        if let Some(b) = rev.strip_prefix("hub/") {
            return self.hub.get(b).cloned().ok_or(format!("no {b} in the hub"));
        }
        if self.commits.contains_key(rev) { Ok(rev.into()) } else { Err(format!("no commit {rev}")) }
    }

    fn ancestors(&self, sha: &str) -> Vec<String> {
        let mut out = vec![];
        let mut cur = Some(sha.to_string());
        while let Some(c) = cur {
            cur = self.commits.get(&c).and_then(|c| c.parent.clone());
            out.push(c);
        }
        out
    }

    fn run_role(&mut self, role: Role, run: Run) -> Out {
        let key = format!("{}:{}", role.name(), self.entry.clone().unwrap_or_default());
        let base = self.roles.get(&key).cloned();
        match run {
            Run::Commit(changes) => {
                let sha = self.new_commit(base, "agent work", changes);
                self.roles.insert(key, sha);
                Out::with(0, "done")
            }
            Run::Rewrite => {
                let parent = base.and_then(|b| self.commits[&b].parent.clone());
                let sha = self.new_commit(parent, "rewritten", vec![]);
                self.roles.insert(key, sha);
                Out::with(0, "done")
            }
            Run::Limit(msg) => Out::with(75, msg),
            Run::Fail => Out::with(1, "agent crashed"),
            Run::Nothing => Out::with(0, "nothing to do"),
        }
    }
}

impl Ops for World {
    fn now(&mut self) -> u64 {
        self.now
    }
    fn sleep(&mut self, secs: u64) {
        self.now += secs;
    }
    fn set_entry(&mut self, entry: Option<&str>) {
        self.entry = entry.map(String::from);
    }
    fn refresh_main(&mut self) -> Result {
        self.call("refresh_main");
        Ok(())
    }
    fn queue(&mut self) -> Result<Vec<String>> {
        Ok(self.queue.clone())
    }
    fn hub_head(&mut self, branch: &str) -> Result<Option<String>> {
        Ok(self.hub.get(branch).cloned())
    }
    fn prepare(&mut self, slot: usize, start: &str) -> Result<String> {
        self.call("prepare");
        let sha = self.resolve(start)?;
        self.slots.insert(slot, sha.clone());
        Ok(sha)
    }
    fn subject(&mut self, _slot: usize, rev: &str) -> Result<String> {
        Ok(self.commits.get(rev).map(|c| c.subject.clone()).unwrap_or_default())
    }
    fn commit(&mut self, slot: usize, message: &str, _paths: &[String]) -> Result<String> {
        self.call("commit");
        let changes = std::mem::take(&mut self.staged);
        let sha = self.new_commit(self.slots.get(&slot).cloned(), message, changes);
        self.slots.insert(slot, sha.clone());
        Ok(sha)
    }
    fn push(&mut self, _slot: usize, remote: Remote, rev: &str, branch: &str) -> Result<Push> {
        self.call("push");
        let sha = self.resolve(rev)?;
        let map = match remote {
            Remote::Hub => &self.hub,
            Remote::Origin => &self.origin,
        };
        if let Some(old) = map.get(branch)
            && !self.ancestors(&sha).contains(old)
        {
            return Ok(Push::Rejected("! [rejected] (non-fast-forward)".into()));
        }
        match remote {
            Remote::Hub => self.hub.insert(branch.into(), sha),
            Remote::Origin => self.origin.insert(branch.into(), sha),
        };
        Ok(Push::Done)
    }
    fn changes(&mut self, _slot: usize, from: &str, to: &str) -> Result<Vec<Change>> {
        let mut out = vec![];
        for c in self.ancestors(to) {
            if c == from {
                out.reverse();
                return Ok(out.into_iter().flatten().collect());
            }
            out.push(self.commits[&c].changes.clone());
        }
        Err(format!("{from} is not an ancestor of {to}"))
    }
    fn merge_base(&mut self, _slot: usize, _a: &str, _b: &str) -> Result<String> {
        Ok("main0".into())
    }
    fn file_at(&mut self, _slot: usize, rev: &str, path: &str) -> Result<Option<String>> {
        if path != "coverage/linux/syscall/x86_64.json" {
            return Ok(None);
        }
        let ancestry = self.ancestors(rev);
        let sym = self.entry.clone().unwrap_or_default().rsplit('/').next().unwrap_or_default().to_string();
        let green = ancestry.iter().any(|c| self.commits[c].green);
        let claimed = ancestry.iter().any(|c| self.commits[c].subject.starts_with("Pipeline: claim"));
        let brk: String = if green {
            "{\"symbol\":\"brk\",\"number\":12,\"status\":\"green\",\"tests_visible\":7,\"tests_held_out\":3,\"src\":[\"t\"]}".replace("brk", &sym)
        } else if claimed {
            "{\"symbol\":\"brk\",\"number\":12,\"status\":\"claimed\",\"claimed_by\":\"spec-claude-1\",\"src\":[\"t\"]}".replace("brk", &sym)
        } else {
            "{\"symbol\":\"brk\",\"number\":12,\"src\":[\"t\"]}".replace("brk", &sym)
        };
        Ok(Some(format!("[\n{{\"symbol\":\"access\",\"number\":21,\"src\":[\"t\"]}},\n{brk}\n]\n")))
    }
    fn claim(&mut self, _slot: usize, entry: &EntryId) -> Result<Out> {
        self.call("claim");
        self.staged.push(change(&coverage_file(entry), &["claimed"]));
        Ok(Out::with(0, ""))
    }
    fn ci(&mut self, _slot: usize) -> Result<Out> {
        self.call("ci");
        let code = self.ci.pop_front().unwrap_or(Ok(0))?;
        Ok(Out::with(code, "test result: FAILED\nbrk_secret: SECRET held-out line\nerror: 1 test failed"))
    }
    fn handoff_record(&mut self, _slot: usize, entry: &EntryId) -> Result<Out> {
        self.call("handoff_record");
        let row = format!("| 2026-10-20 10:00:00 | {} | c002 | spec-claude-1 | impl-claude-1 | `f` sha256:0 |", entry.id());
        self.staged.push(change("specs/HANDOFF_LOG.md", &[&row]));
        Ok(Out::with(0, ""))
    }
    fn handoff_bundle(&mut self, _slot: usize, _entry: &EntryId) -> Result<String> {
        self.call("handoff_bundle");
        Ok("/srv/poly/inbox/brk/bundle".into())
    }
    fn handoff_verify(&mut self, _slot: usize, _entry: &EntryId, received: &str) -> Result<Out> {
        self.call("handoff_verify");
        assert_eq!(received, "/srv/poly/inbox/brk/bundle");
        Ok(Out::with(if self.verify_ok { 0 } else { 1 }, "problem: received x.c is held-out test y: LEAK"))
    }
    fn spec_files(&mut self, _slot: usize, _entry: &EntryId) -> Result<SpecFiles> {
        Ok(self.spec_files.clone())
    }
    fn diff(&mut self, _slot: usize, _entry: &EntryId) -> Result<DiffResult> {
        self.call("diff");
        let green = self.diffs.pop_front().unwrap_or(true);
        let summary = if green { "visible 7/7, held-out 3/3" } else { "visible 6/7, held-out 2/3" };
        Ok(DiffResult { green, summary: summary.into(), feedback: format!("feedback after {}", self.count("diff")) })
    }
    fn set_green(&mut self, _slot: usize, entry: &EntryId, visible: u32, held_out: u32) -> Result<Out> {
        self.call("set_green");
        assert_eq!((visible, held_out), (7, 3));
        self.staged.push(change(&coverage_file(entry), &["green"]));
        self.staged.push(change("README.md", &["progress"]));
        self.staged.push(change("docs/progress/linux.svg", &["<svg/>"]));
        Ok(Out::with(0, ""))
    }
    fn coverage_check(&mut self, _slot: usize) -> Result<Out> {
        Ok(Out::with(0, "coverage map ok"))
    }
    fn merge_main(&mut self, slot: usize, _entry: &EntryId) -> Result<Merge> {
        self.call("merge_main");
        let m = self.merges.pop_front().unwrap_or(Merge::UpToDate);
        if let Merge::Merged(_) = m {
            let sha = self.new_commit(self.slots.get(&slot).cloned(), "Merge main", vec![]);
            return Ok(Merge::Merged(sha));
        }
        Ok(m)
    }
    fn role_sync(&mut self, role: Role, entry: &EntryId, _branch: &str, base: &str) -> Result<Out> {
        self.call(&format!("{}_sync", role.name()));
        let sha = self.hub.get(base).cloned().ok_or("no base")?;
        self.roles.insert(format!("{}:{}", role.name(), entry.id()), sha);
        Ok(Out::with(0, ""))
    }
    fn role_run(&mut self, role: Role, _entry: &EntryId, feedback: Option<&str>) -> Result<Out> {
        self.call(&format!("{}_run", role.name()));
        let run = match role {
            Role::Spec => self.spec_runs.pop_front().unwrap_or_else(good_spec),
            Role::Implementer => {
                self.feedback_given.push(feedback.map(String::from));
                self.impl_runs.pop_front().unwrap_or_else(good_impl)
            }
        };
        Ok(self.run_role(role, run))
    }
    fn role_fetch(&mut self, role: Role, _branch: &str) -> Result<Option<String>> {
        self.call(&format!("{}_fetch", role.name()));
        Ok(self.roles.get(&format!("{}:{}", role.name(), self.entry.clone().unwrap_or_default())).cloned())
    }
    fn redact(&mut self, _entry: &EntryId, text: &str) -> String {
        text.lines().filter(|l| !l.contains("SECRET")).collect::<Vec<_>>().join("\n")
    }
    fn write_feedback(&mut self, _entry: &EntryId, cycle: u32, text: &str) -> Result<String> {
        self.feedback.push(text.to_string());
        Ok(format!("/srv/poly/inbox/brk/feedback-{cycle}.txt"))
    }
    fn resolve_clock(&mut self, time: &str, _tz: Option<&str>) -> Option<u64> {
        assert_eq!(time, "3pm");
        Some(self.now + 7200)
    }
    fn pr_find(&mut self, branch: &str) -> Result<Option<Pr>> {
        self.call("pr_find");
        Ok(self.prs.get(branch).cloned())
    }
    fn pr_create(&mut self, branch: &str, title: &str, body: &str) -> Result<String> {
        self.call("pr_create");
        assert!(title.starts_with(BRK) && body.contains("merge commit"));
        self.prs_created += 1;
        let url = format!("https://github.com/o/r/pull/{}", self.prs_created);
        self.prs.insert(branch.into(), Pr { url: url.clone(), state: "OPEN".into(), mergeable: "MERGEABLE".into() });
        Ok(url)
    }
    fn pr_auto_merge(&mut self, _url: &str) -> Result {
        self.call("pr_auto_merge");
        Ok(())
    }
    fn notify(&mut self, event: &str, entry: &str, _message: &str) -> Result {
        self.notified.push(format!("{event} {entry}"));
        Ok(())
    }
    fn append_human_queue(&mut self, text: &str) -> Result {
        self.human.push_str(text);
        Ok(())
    }
}

struct Setup {
    dir: PathBuf,
    cfg: Config,
    store: Store,
    log: Logger,
}

fn setup(name: &str) -> Setup {
    let dir = std::env::temp_dir().join(format!("poly-loop-cycle-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = Config { state_dir: dir.clone(), log_file: None, ..Config::default() };
    let store = Store::new(&dir).unwrap();
    Setup { dir, cfg, store, log: Logger::new(None, true).unwrap() }
}

impl Setup {
    fn driver(&self, w: World) -> Driver<'_, World> {
        Driver { cfg: &self.cfg, store: &self.store, log: &self.log, ops: w, name: "w0".into() }
    }

    fn step(&self, entry: &str) -> Step {
        self.store.load(entry).unwrap().unwrap().step
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn worked(p: Pass) -> Yielded {
    match p {
        Pass::Worked(e, y) => {
            assert_eq!(e, BRK);
            y
        }
        p => panic!("expected work, got {p:?}"),
    }
}

/// The order of the main operations in `calls`.
fn order(calls: &[String]) -> Vec<&str> {
    let main = [
        "claim",
        "spec_run",
        "ci",
        "handoff_record",
        "handoff_bundle",
        "implementer_run",
        "handoff_verify",
        "diff",
        "set_green",
        "pr_create",
        "pr_auto_merge",
    ];
    calls.iter().map(String::as_str).filter(|c| main.contains(c)).collect()
}

#[test]
fn green_path_opens_a_pr_and_finishes_when_merged() {
    let s = setup("green");
    let mut d = s.driver(World::new());
    let y = worked(d.pass(None).unwrap());
    assert_eq!(y, Yielded::Waiting(T0 + s.cfg.poll_secs));
    assert_eq!(
        order(&d.ops.calls),
        [
            "claim",
            "spec_run",
            "ci",
            "handoff_record",
            "handoff_bundle",
            "implementer_run",
            "handoff_verify",
            "ci",
            "diff",
            "set_green",
            "pr_create",
            "pr_auto_merge"
        ]
    );
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!(st.step, Step::AwaitingMerge);
    assert_eq!((st.cycle, st.tests_visible, st.tests_held_out), (0, 7, 3));
    assert_eq!(st.pr_url.as_deref(), Some("https://github.com/o/r/pull/1"));
    // The PR branch carries the claim, the spec, the hand-off, the
    // implementation and the green status, in that order.
    let head = d.ops.origin["pipeline/impl/brk"].clone();
    let subjects: Vec<String> =
        d.ops.ancestors(&head).iter().rev().map(|c| d.ops.commits[c].subject.clone()).collect();
    assert_eq!(
        subjects,
        [
            "main",
            "Pipeline: claim linux/syscall/x86_64/brk for spec-claude-1",
            "agent work",
            "Pipeline: hand off linux/syscall/x86_64/brk from spec-claude-1 to impl-claude-1",
            "agent work",
            "Pipeline: linux/syscall/x86_64/brk is green (7 visible, 3 held-out tests)"
        ]
    );
    assert_eq!(d.ops.notified, ["pr linux/syscall/x86_64/brk"]);

    // Nothing new to start (the queue has only brk), and the PR is not due.
    assert_eq!(d.pass(None).unwrap(), Pass::Idle(T0 + s.cfg.poll_secs));
    d.ops.prs.get_mut("pipeline/impl/brk").unwrap().state = "MERGED".into();
    d.ops.sleep(s.cfg.poll_secs);
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Done(Step::Merged));
    assert_eq!(d.ops.prs_created, 1);
}

#[test]
fn red_goes_back_with_feedback_then_green() {
    let s = setup("red");
    let mut w = World::new();
    w.diffs = [false].into();
    w.ci = [Ok(0), Ok(0), Ok(1)].into(); // spec check, cycle 1, cycle 2 (red CI)
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!((st.step, st.cycle), (Step::AwaitingMerge, 2));
    assert_eq!(d.ops.count("implementer_run"), 3);
    assert_eq!(
        d.ops.feedback_given,
        [None, Some("/srv/poly/inbox/brk/feedback-1.txt".into()), Some("/srv/poly/inbox/brk/feedback-2.txt".into())]
    );
    assert_eq!(d.ops.feedback[0], "feedback after 1");
    // CI output goes back redacted: nothing of the held-out test.
    assert!(d.ops.feedback[1].contains("cargo xtask ci") && d.ops.feedback[1].contains("1 test failed"));
    assert!(!d.ops.feedback[1].contains("SECRET"));
}

#[test]
fn five_reds_go_to_the_human_queue() {
    let s = setup("five");
    let mut w = World::new();
    w.diffs = [false; 5].into();
    let mut d = s.driver(w);
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Done(Step::Human));
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!((st.step, st.cycle), (Step::Human, 5));
    assert_eq!(d.ops.count("implementer_run"), 5);
    assert!(st.reason.as_deref().unwrap().starts_with("5 red cycles; last: diff red"), "{:?}", st.reason);
    assert!(d.ops.human.contains("## 2026-10-20") && d.ops.human.contains(BRK) && d.ops.human.contains("loop reset"));
    assert_eq!(d.ops.notified, ["human linux/syscall/x86_64/brk"]);
    // Both branches are on GitHub for the owner; no PR.
    assert!(d.ops.origin.contains_key("pipeline/spec/brk") && d.ops.origin.contains_key("pipeline/impl/brk"));
    assert_eq!(d.ops.prs_created, 0);
    // The loop does not take it again.
    assert!(matches!(d.pass(None).unwrap(), Pass::Idle(_)));
}

#[test]
fn a_usage_limit_pauses_the_role_and_resumes_the_same_step() {
    let s = setup("limit");
    let mut w = World::new();
    w.impl_runs = [Run::Limit(format!("Claude AI usage limit reached|{}", T0 + 5000))].into();
    w.spec_runs = [Run::Limit("5-hour limit reached ∙ resets 3pm".into()), good_spec()].into();
    let mut d = s.driver(w);
    // The spec agent hits the limit first: a clock time, resolved with `date`.
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Paused("spec", T0 + 7200 + 120));
    assert_eq!(s.step(BRK), Step::Spec);
    assert_eq!(d.pass(None).unwrap(), Pass::Idle(T0 + 7320));
    d.ops.sleep(7320);
    let t1 = d.ops.now;
    // Then the implementer: a Unix time in the message (already passed: default pause).
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Paused("implementer", t1 + s.cfg.limit_pause_secs));
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!((st.step, st.cycle), (Step::Implement, 0), "a limit is never a red cycle");
    assert_eq!(s.store.paused_until("spec", t1).unwrap(), None);
    d.ops.sleep(s.cfg.limit_pause_secs);
    worked(d.pass(None).unwrap());
    assert_eq!(s.step(BRK), Step::AwaitingMerge);
    assert_eq!((d.ops.count("spec_run"), d.ops.count("implementer_run")), (2, 2));
}

#[test]
fn a_reset_time_in_the_message_is_used() {
    let s = setup("limit-at");
    let mut w = World::new();
    w.impl_runs = [Run::Limit(format!("Claude AI usage limit reached|{}", T0 + 5000))].into();
    let mut d = s.driver(w);
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Paused("implementer", T0 + 5000 + 120));
}

/// Drive until the simulated crash; the world survives, the process does not.
fn crash(d: &mut Driver<'_, World>) {
    let r = catch_unwind(AssertUnwindSafe(|| d.pass(None)));
    assert!(r.is_err(), "expected the simulated crash");
}

#[test]
fn a_crash_resumes_at_the_right_step() {
    // After the implementer session, before its branch was fetched: run it again.
    let s = setup("crash1");
    let mut w = World::new();
    w.crash = Some(("implementer_fetch".into(), 1));
    let mut d = s.driver(w);
    crash(&mut d);
    assert_eq!(s.step(BRK), Step::Implement);
    let mut d = s.driver(d.ops);
    worked(d.pass(None).unwrap());
    assert_eq!(s.step(BRK), Step::AwaitingMerge);
    assert_eq!((d.ops.count("spec_run"), d.ops.count("implementer_run"), d.ops.count("claim")), (1, 2, 1));

    // After the spec was fetched (the 2nd push is the spec branch): no new spec session.
    let s = setup("crash2");
    let mut w = World::new();
    w.crash = Some(("push".into(), 2));
    let mut d = s.driver(w);
    crash(&mut d);
    let st = s.store.load(BRK).unwrap().unwrap();
    assert!(st.step == Step::Spec && st.pending_head.is_some());
    let mut d = s.driver(d.ops);
    worked(d.pass(None).unwrap());
    assert_eq!((d.ops.count("spec_run"), d.ops.count("handoff_record")), (1, 1));

    // After the PR was created: no second PR.
    let s = setup("crash3");
    let mut w = World::new();
    w.crash = Some(("pr_auto_merge".into(), 1));
    let mut d = s.driver(w);
    crash(&mut d);
    assert_eq!(s.step(BRK), Step::Publish);
    let mut d = s.driver(d.ops);
    worked(d.pass(None).unwrap());
    assert_eq!((d.ops.prs_created, d.ops.count("set_green"), d.ops.count("pr_auto_merge")), (1, 1, 2));

    // After the hand-off commit was pushed: the record is not made twice.
    let s = setup("crash4");
    let mut w = World::new();
    w.crash = Some(("handoff_bundle".into(), 1));
    let mut d = s.driver(w);
    crash(&mut d);
    let mut d = s.driver(d.ops);
    worked(d.pass(None).unwrap());
    assert_eq!((d.ops.count("handoff_record"), d.ops.count("handoff_bundle")), (1, 2));
    assert_eq!(s.step(BRK), Step::AwaitingMerge);
}

fn human_reason(s: &Setup) -> String {
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!(st.step, Step::Human);
    st.reason.unwrap()
}

#[test]
fn unsafe_code_is_refused_before_anything_runs() {
    let s = setup("unsafe");
    let mut w = World::new();
    w.impl_runs = [Run::Commit(vec![change("crates/poly-vm/src/lib.rs", &["    let v = unsafe { *p };"])])].into();
    let mut d = s.driver(w);
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Done(Step::Human));
    let r = human_reason(&s);
    assert!(r.contains("red flag in the implementer's changes") && r.contains("adds unsafe code"), "{r}");
    assert_eq!(d.ops.count("ci"), 1, "only the spec check's CI ran; nothing of the implementer's branch");
    assert_eq!(d.ops.count("handoff_verify"), 0);
    assert_eq!(d.ops.prs_created, 0);
}

#[test]
fn forbidden_paths_are_refused() {
    let s = setup("forbidden");
    let mut w = World::new();
    w.impl_runs =
        [Run::Commit(vec![good_change(), change("docs/ROADMAP.md", &["- [x] brk"]), change("README.md", &["x"])])]
            .into();
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    let r = human_reason(&s);
    assert!(r.contains("may not change docs/ROADMAP.md") && r.contains("may not change README.md"), "{r}");

    let s = setup("forbidden-spec");
    let mut w = World::new();
    w.spec_runs = [Run::Commit(vec![change("coverage/linux/syscall/x86_64.json", &["x"])])].into();
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    assert!(human_reason(&s).contains("red flag in the spec agent's changes"));
    assert_eq!(d.ops.count("handoff_record"), 0);

    let s = setup("outside");
    let mut w = World::new();
    w.impl_runs = [Run::Commit(vec![change("tools/pipeline/src/roles.rs", &["x"])])].into();
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    assert!(human_reason(&s).contains("may not change tools/"));
}

fn good_change() -> Change {
    change("personalities/linux/src/brk.rs", &["pub fn brk() {}"])
}

#[test]
fn a_failed_handoff_verify_stops_the_entry() {
    let s = setup("verify");
    let mut w = World::new();
    w.verify_ok = false;
    let mut d = s.driver(w);
    assert_eq!(worked(d.pass(None).unwrap()), Yielded::Done(Step::Human));
    assert!(human_reason(&s).contains("`handoff verify` failed") && human_reason(&s).contains("LEAK"));
    assert_eq!((d.ops.count("ci"), d.ops.count("diff")), (1, 0));
}

#[test]
fn a_rewritten_history_is_refused() {
    let s = setup("rewrite");
    let mut w = World::new();
    w.diffs = [false].into();
    w.impl_runs = [good_impl(), Run::Rewrite].into();
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    assert!(human_reason(&s).contains("history rewritten"));
}

#[test]
fn agent_failures_are_bounded() {
    // The spec agent fails once, then works.
    let s = setup("spec-fail");
    let mut w = World::new();
    w.spec_runs = [Run::Fail, good_spec()].into();
    w.impl_runs = [Run::Nothing, Run::Fail, good_impl()].into();
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!((st.step, st.spec_attempts, st.cycle), (Step::AwaitingMerge, 1, 2));
    assert!(d.ops.feedback[0].contains("without a commit") && d.ops.feedback[1].contains("failed"));

    // A spec agent that never commits goes to the human after spec_attempts.
    let s = setup("spec-nothing");
    let mut w = World::new();
    w.spec_runs = [Run::Nothing, Run::Nothing].into();
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    assert!(human_reason(&s).contains("committed nothing (2 attempts)"));

    // An incomplete spec (no held-out tests) too.
    let s = setup("spec-incomplete");
    let mut w = World::new();
    w.spec_files.held_out = 0;
    let mut d = s.driver(w);
    worked(d.pass(None).unwrap());
    assert!(human_reason(&s).contains("incomplete spec"));
}

#[test]
fn infrastructure_errors_back_off_and_retry() {
    let s = setup("errors");
    let mut w = World::new();
    w.ci = [Ok(0), Err("cargo: not found".into())].into();
    let mut d = s.driver(w);
    let y = worked(d.pass(None).unwrap());
    assert_eq!(y, Yielded::Retry(T0 + 120));
    let st = s.store.load(BRK).unwrap().unwrap();
    assert_eq!((st.step, st.errors, st.cycle), (Step::Verify, 1, 0), "an error is not a red cycle");
    assert_eq!(d.pass(None).unwrap(), Pass::Idle(T0 + 120));
    d.ops.sleep(120);
    worked(d.pass(None).unwrap());
    assert_eq!(s.step(BRK), Step::AwaitingMerge);
    assert_eq!(s.store.load(BRK).unwrap().unwrap().errors, 0);
}

#[test]
fn conflicts_with_main_are_merged_and_verified_again() {
    let s = setup("refresh");
    let mut d = s.driver(World::new());
    worked(d.pass(None).unwrap());
    d.ops.prs.get_mut("pipeline/impl/brk").unwrap().mergeable = "CONFLICTING".into();
    d.ops.merges = [Merge::Merged(String::new())].into();
    d.ops.sleep(s.cfg.poll_secs);
    worked(d.pass(None).unwrap());
    assert_eq!(s.step(BRK), Step::AwaitingMerge);
    assert_eq!((d.ops.count("diff"), d.ops.count("set_green"), d.ops.prs_created), (2, 1, 1));
    assert_eq!(d.ops.count("implementer_run"), 1, "a merge of main is not an implementer session");

    // Conflicts in real code go to the owner.
    d.ops.merges = [Merge::Conflicts(vec!["personalities/linux/src/mm.rs".into()])].into();
    d.ops.sleep(s.cfg.poll_secs);
    worked(d.pass(None).unwrap());
    assert!(human_reason(&s).contains("conflicts with main in personalities/linux/src/mm.rs"));
}

#[test]
fn two_workers_never_take_the_same_entry() {
    let mut s = setup("parallel");
    s.cfg.max_parallel = 2;
    let mut w = World::new();
    w.queue = vec![BRK.into(), "linux/syscall/x86_64/mmap".into()];
    let mut a = s.driver(w);
    // Worker a pauses brk on a limit; while a "holds" brk, b gets mmap.
    a.ops.impl_runs = [Run::Limit("usage limit reached".into())].into();
    assert!(matches!(worked(a.pass(None).unwrap()), Yielded::Paused("implementer", _)));
    let lock = s.store.try_lock("entry-linux__syscall__x86_64__brk").unwrap().unwrap();
    let mut b = s.driver(a.ops);
    b.ops.sleep(s.cfg.limit_pause_secs);
    match b.pass(None).unwrap() {
        Pass::Worked(e, _) => assert_eq!(e, "linux/syscall/x86_64/mmap"),
        p => panic!("{p:?}"),
    }
    drop(lock);
    // With both in flight, nothing new starts; brk can move again.
    let mut c = s.driver(b.ops);
    assert_eq!(worked(c.pass(None).unwrap()), Yielded::Waiting(c.ops.now + s.cfg.poll_secs));
    // --entry restricts the loop to one entry.
    assert!(matches!(c.pass(Some("linux/syscall/x86_64/read")).unwrap(), Pass::Idle(_)));
}

#[test]
fn the_dry_run_walks_the_whole_cycle_with_real_commands() {
    let s = setup("dry");
    let cfg = s.cfg.clone();
    let mut ops = RealOps::new(cfg.clone(), DryShell::new(true), &s.dir.join("logs"));
    ops.queue_repo = crate::repo_root();
    let mut d = Driver { cfg: &cfg, store: &s.store, log: &s.log, ops, name: "w0".into() };
    let (entry, y) = match d.pass(None).unwrap() {
        Pass::Worked(e, y) => (e, y),
        p => panic!("{p:?}"),
    };
    assert!(matches!(y, Yielded::Waiting(_)), "{y:?}");
    let lines = &d.ops.shell.lines;
    let find = |needle: &str| {
        lines.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("no {needle:?} in {lines:#?}"))
    };
    let steps = [
        format!("cargo xtask coverage claim {entry} --agent spec-claude-1"),
        "/srv/poly/bin/poly-role spec sync pipeline/spec/".into(),
        format!("/srv/poly/bin/poly-role spec run {entry} spec-claude-1"),
        "cargo xtask ci".into(),
        format!("cargo xtask pipeline handoff record {entry} --spec-agent spec-claude-1 --implementer impl-claude-1"),
        format!("cargo xtask pipeline handoff bundle {entry} --out /srv/poly/inbox/"),
        format!("sudo -n -u polyimpl /srv/poly/bin/poly-role implementer run {entry} impl-claude-1"),
        format!("cargo xtask pipeline handoff verify {entry} --received /srv/poly/inbox/"),
        "cargo xtask pipeline diff --target poly --build-kernel --tests-dir".into(),
        format!("cargo xtask coverage set-status {entry} green --tests-visible 2 --tests-held-out 1"),
        "cargo xtask coverage readme --check".into(),
        "push -q origin".into(),
        "gh pr create --base main --head pipeline/impl/".into(),
        "gh pr merge https://github.com/OWNER/REPO/pull/0 --auto --merge".into(),
    ];
    let at: Vec<usize> = steps.iter().map(|n| find(n)).collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "out of order: {at:?}");
    assert!(lines.iter().all(|l| !l.contains("--force") && !l.contains(" -f ") || l.contains("checkout")));
    // The implementer's commands never carry the held-out store.
    for l in lines.iter().filter(|l| l.contains("polyimpl")) {
        assert!(!l.contains("POLY_HELDOUT_DIR") && !l.contains("/srv/poly/heldout"), "{l}");
    }
    // The CLI dry run (what `cargo xtask ci` runs) works too.
    assert_eq!(super::command(&crate::repo_root(), &["--dry-run".into(), "--quiet".into()]), Ok(0));
}
