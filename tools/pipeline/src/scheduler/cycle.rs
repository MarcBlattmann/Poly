//! One entry's way through the pipeline, as a state machine whose state
//! is saved after every step (docs/AGENT_PIPELINE.md, "A home server
//! running 24/7"):
//!
//! ```text
//! claim ─► spec ─► spec-check ─► handoff ─► implement ─► verify ──green──► publish ─► awaiting-merge ─► merged
//!            ▲  (exit 75: pause) │            ▲  (exit 75: pause) │                         │ conflicts
//!            └─ red: feedback ───┘            └──── red: feedback ┘ (up to max_cycles)   refresh ─► verify
//!               (up to spec_cycles)
//!   any red flag (forbidden path, unsafe, hand-off problem, history rewritten): human
//! ```
//!
//! Every step is idempotent on restart: steps whose result is a pushed
//! branch look at the hub first, a fetched head waiting to be pushed is
//! kept in the state (`pending_head`), and an agent session is recorded in
//! the state (`session`) before it starts and when it ends, so an error or
//! a crash after it never pays for it again: the retry fetches the role's
//! branch first, and the agent runs again (its checkout reset first) only
//! when a session cut short left no commit.

use super::config::Config;
use super::limits::{self, Reset};
use super::ops::{Merge, Ops, Out, Push, Remote};
use super::safety::{self, PrCheck};
use super::state::{self, EntryState, Lock, Pause, Session, Step, Store};
use crate::roles::Role;
use crate::store::EntryId;
use crate::util::{self, Result};
use std::io::Write;
use std::sync::Mutex;

/// Log lines go to stdout (journald, with priority prefixes when run by
/// systemd) and to the log file.
#[derive(Debug)]
pub struct Logger {
    file: Mutex<Option<std::fs::File>>,
    journal: bool,
    quiet: bool,
}

impl Logger {
    pub fn new(file: Option<&std::path::Path>, quiet: bool) -> Result<Logger> {
        let file = match file {
            Some(p) => {
                if let Some(dir) = p.parent() {
                    util::create_dir(dir)?;
                }
                Some(
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                        .map_err(|e| format!("{}: {e}", p.display()))?,
                )
            }
            None => None,
        };
        Ok(Logger { file: Mutex::new(file), journal: std::env::var_os("JOURNAL_STREAM").is_some(), quiet })
    }

    /// `level`: 3 error, 4 warning, 6 info (syslog priorities).
    pub fn log(&self, level: u8, who: &str, message: &str) {
        let line = if who.is_empty() { message.to_string() } else { format!("{who}: {message}") };
        if !self.quiet {
            if self.journal {
                println!("<{level}>{line}");
            } else {
                println!("{line}");
            }
        }
        if let Ok(mut f) = self.file.lock()
            && let Some(f) = f.as_mut()
        {
            let _ = writeln!(f, "{} {line}", util::now_utc());
        }
    }
}

/// How a drive of an entry ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Yielded {
    /// The role hit a usage limit and is paused until then.
    Paused(&'static str, u64),
    /// Waiting for something outside (the PR) until about then.
    Waiting(u64),
    /// An infrastructure error; retried after then.
    Retry(u64),
    /// Merged or in the human queue.
    Done(Step),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pass {
    Worked(String, Yielded),
    /// Nothing to do before then.
    Idle(u64),
}

enum Next {
    Continue,
    Yield(Yielded),
}

/// What an agent session left on its branch.
enum Ended {
    /// A new head, fetched into the runner.
    Committed(String),
    /// No new commit; how the session ended.
    Nothing(Out),
    /// A usage limit: the role is paused.
    Limit(Next),
}

/// Whether the entry's agent session has ended and only its branch is
/// left to fetch (no session to start, so no reason to wait for the role).
fn fetch_only(st: &EntryState) -> bool {
    st.pending_head.is_some() || st.session.as_ref().is_some_and(|s| s.exit.is_some())
}

pub fn spec_branch(e: &EntryId) -> String {
    format!("pipeline/spec/{}", e.symbol)
}

pub fn impl_branch(e: &EntryId) -> String {
    format!("pipeline/impl/{}", e.symbol)
}

/// Symbols that make a good branch name component.
pub fn valid_symbol(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(['.', '-'])
        && !s.ends_with(".lock")
        && !s.contains("..")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
}

pub fn claim_message(e: &EntryId, agent: &str) -> String {
    format!("Pipeline: claim {} for {agent}", e.id())
}

pub fn handoff_message(e: &EntryId, spec: &str, implementer: &str) -> String {
    format!("Pipeline: hand off {} from {spec} to {implementer}", e.id())
}

pub fn green_message(e: &EntryId, visible: u32, held_out: u32) -> String {
    format!("Pipeline: {} is green ({visible} visible, {held_out} held-out tests)", e.id())
}

/// Whether `symbol` is green in a coverage file's text.
pub fn green_in(coverage: Option<&str>, symbol: &str) -> bool {
    let Some(text) = coverage else { return false };
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str::<serde_json::Value>(text) else { return false };
    entries.iter().any(|e| {
        e.get("symbol").and_then(|s| s.as_str()) == Some(symbol)
            && e.get("status").and_then(|s| s.as_str()) == Some("green")
    })
}

#[derive(Debug)]
pub struct Driver<'a, O: Ops> {
    pub cfg: &'a Config,
    pub store: &'a Store,
    pub log: &'a Logger,
    pub ops: O,
    /// Worker name for the log (`w0`, ...).
    pub name: String,
}

impl<O: Ops> Driver<'_, O> {
    fn who(&self, entry: &str) -> String {
        format!("{} {entry}", self.name)
    }

    fn note(&mut self, st: &mut EntryState, message: impl Into<String>) {
        let message = message.into();
        let now = self.ops.now();
        self.log.log(6, &self.who(&st.entry), &format!("[{}] {message}", st.step.name()));
        st.note(now, message);
    }

    /// When `st` can make progress, if not now.
    fn ready_at(&self, st: &EntryState, now: u64) -> Result<u64> {
        let mut at = st.retry_at;
        if let Some(role) = st.step.role()
            && !fetch_only(st)
            && let Some(until) = self.store.paused_until(role, now)?
        {
            at = at.max(until);
        }
        if st.step == Step::AwaitingMerge {
            at = at.max(st.last_poll + self.cfg.poll_secs);
        }
        Ok(at)
    }

    fn wanted(&self, id: &str) -> bool {
        (self.cfg.entries.is_empty() || self.cfg.entries.iter().any(|p| id.starts_with(p.as_str())))
            && !self.cfg.exclude.iter().any(|p| id.starts_with(p.as_str()))
    }

    /// Choose an entry to drive: one in progress that can move, else a new
    /// one from the queue (within `max_parallel` and `max_open_prs`).
    fn pick(&mut self, only: Option<&str>) -> Result<std::result::Result<(EntryState, Lock), u64>> {
        let _pick = self.store.lock("pick")?;
        let now = self.ops.now();
        let mut states = self.store.load_all()?;
        states.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.entry.cmp(&b.entry)));
        // The earliest time an entry in progress can move.
        let mut ready = u64::MAX;
        for st in states.iter().filter(|s| !s.step.terminal() && only.is_none_or(|o| o == s.entry)) {
            let at = self.ready_at(st, now)?;
            if at > now {
                ready = ready.min(at);
                continue;
            }
            if let Some(lock) = self.store.try_lock(&format!("entry-{}", state::key(&st.entry)))? {
                // Another worker may have released it just now: its state
                // as saved, not as read before the lock.
                match self.store.load(&st.entry)? {
                    Some(fresh) if !fresh.step.terminal() && self.ready_at(&fresh, now)? <= now => {
                        return Ok(Ok((fresh, lock)));
                    }
                    _ => continue,
                }
            }
        }
        let idle = now + self.cfg.idle_secs;
        let in_flight = states.iter().filter(|s| !s.step.terminal() && s.step != Step::AwaitingMerge).count();
        let open_prs = states.iter().filter(|s| s.step == Step::AwaitingMerge).count();
        if in_flight >= self.cfg.max_parallel || open_prs >= self.cfg.max_open_prs {
            // No new entries: wake when one in progress can move.
            return Ok(Err(if ready == u64::MAX { idle } else { ready }));
        }
        self.ops.set_entry(None);
        self.ops.refresh_main()?;
        for id in self.ops.queue()? {
            if only.is_some_and(|o| o != id) || !self.wanted(&id) || states.iter().any(|s| s.entry == id) {
                continue;
            }
            let Ok(entry) = EntryId::parse(&id) else { continue };
            // Branch names use the symbol: skip unusual ones and clashes.
            if !valid_symbol(&entry.symbol) || states.iter().any(|s| s.symbol() == entry.symbol) {
                continue;
            }
            let Some(lock) = self.store.try_lock(&format!("entry-{}", state::key(&id)))? else { continue };
            let mut st = EntryState::new(&id, now);
            self.note(&mut st, "picked from the queue");
            self.store.save(&st)?;
            return Ok(Ok((st, lock)));
        }
        Ok(Err(ready.min(idle)))
    }

    /// One scheduling decision: drive an entry until it yields, or say
    /// when there may be work again.
    pub fn pass(&mut self, only: Option<&str>) -> Result<Pass> {
        match self.pick(only)? {
            Ok((mut st, lock)) => {
                self.ops.set_entry(Some(&st.entry));
                let y = self.drive(&mut st);
                drop(lock);
                Ok(Pass::Worked(st.entry, y))
            }
            Err(wake) => Ok(Pass::Idle(wake)),
        }
    }

    fn save(&self, st: &EntryState) {
        if let Err(e) = self.store.save(st) {
            self.log.log(3, &self.who(&st.entry), &format!("cannot save the state: {e}"));
        }
    }

    pub fn drive(&mut self, st: &mut EntryState) -> Yielded {
        loop {
            match self.step(st) {
                Ok(Next::Continue) => {
                    st.errors = 0;
                    self.save(st);
                }
                Ok(Next::Yield(y)) => {
                    st.errors = 0;
                    self.save(st);
                    return y;
                }
                Err(e) => {
                    st.errors += 1;
                    let now = self.ops.now();
                    self.log.log(3, &self.who(&st.entry), &format!("[{}] error: {e}", st.step.name()));
                    st.note(now, format!("error: {e}"));
                    if st.errors >= self.cfg.max_errors {
                        let reason = format!("{} infrastructure errors in a row; last: {e}", st.errors);
                        let y = match self.human(st, reason) {
                            Next::Yield(y) => y,
                            Next::Continue => Yielded::Done(Step::Human),
                        };
                        self.save(st);
                        return y;
                    }
                    // 2, 4, 8, ... minutes, at most an hour.
                    st.retry_at = now + (60u64 << st.errors.min(6)).min(3600);
                    self.save(st);
                    return Yielded::Retry(st.retry_at);
                }
            }
        }
    }

    /// A runner worktree for one step.
    fn slot(&self) -> Result<(usize, Lock)> {
        let n = self.cfg.max_parallel;
        for i in 0..n {
            if let Some(l) = self.store.try_lock(&format!("slot-{i}"))? {
                return Ok((i, l));
            }
        }
        let i = self.name.trim_start_matches('w').parse::<usize>().unwrap_or(0) % n;
        Ok((i, self.store.lock(&format!("slot-{i}"))?))
    }

    /// Stop the entry and hand it to the owner.
    fn human(&mut self, st: &mut EntryState, reason: String) -> Next {
        st.step = Step::Human;
        st.reason = Some(reason.clone());
        self.log.log(4, &self.who(&st.entry), &format!("to the human queue: {reason}"));
        let now = self.ops.now();
        st.note(now, format!("to the human queue: {reason}"));
        let Ok(entry) = EntryId::parse(&st.entry) else { return Next::Yield(Yielded::Done(Step::Human)) };
        let mut pushed = Vec::new();
        if self.cfg.push_human_branches {
            // Every hub head was pushed from the runner's object store, which
            // the worktrees share: no checkout is needed to push it on (and
            // no slot lock, which the calling step may hold).
            for b in [spec_branch(&entry), impl_branch(&entry)] {
                let Ok(Some(sha)) = self.ops.hub_head(&b) else { continue };
                match self.ops.push(0, Remote::Origin, &sha, &b) {
                    Ok(Push::Done) => pushed.push(b),
                    Ok(Push::Rejected(m)) | Err(m) => {
                        self.log.log(4, &self.who(&st.entry), &format!("could not push {b}: {m}"))
                    }
                }
            }
        }
        let logs = self.store.logs(&st.entry);
        let text = format!(
            "\n## {} {}\n\n- reason: {}\n- step reached: {}, red cycles: {}, last result: {}\n- branches (hub {}): `{}`, `{}`{}\n\
             - state: `{}`, logs: `{}`\n- to retry from scratch: `cargo xtask pipeline loop reset {}`\n",
            util::format_utc(now),
            st.entry,
            reason,
            st.history.iter().rev().find(|e| e.step != Step::Human).map_or("claim", |e| e.step.name()),
            st.cycle,
            st.last_result.as_deref().unwrap_or("-"),
            self.cfg.hub.display(),
            spec_branch(&entry),
            impl_branch(&entry),
            if pushed.is_empty() { String::new() } else { format!(" (pushed to {})", self.cfg.remote) },
            self.store.dir.join("entries").join(format!("{}.json", state::key(&st.entry))).display(),
            logs.display(),
            st.entry,
        );
        if let Err(e) = self.ops.append_human_queue(&text) {
            self.log.log(3, &self.who(&st.entry), &format!("cannot write the human queue: {e}"));
        }
        if let Err(e) = self.ops.notify("human", &st.entry, &reason) {
            self.log.log(4, &self.who(&st.entry), &format!("notify: {e}"));
        }
        Next::Yield(Yielded::Done(Step::Human))
    }

    /// Exit status 75: pause the role, keep the step.
    fn limit(&mut self, st: &mut EntryState, role: &'static str, out: &Out) -> Result<Next> {
        let now = self.ops.now();
        let resolved = match limits::parse(&out.text) {
            Some(Reset::At(t)) => Some(t),
            Some(Reset::In(s)) => Some(now + s),
            Some(Reset::Clock { time, tz }) => self.ops.resolve_clock(&time, tz.as_deref()),
            None => None,
        };
        let until = limits::retry_time(now, resolved, self.cfg.limit_pause_secs);
        let reason = format!("usage limit ({})", out.tail(1).trim());
        self.store.set_pause(role, &Pause { until, reason })?;
        self.note(st, format!("the {role} role hit a usage limit; paused until {} UTC", util::format_utc(until)));
        Ok(Next::Yield(Yielded::Paused(role, until)))
    }

    /// A red cycle: feedback to the implementer, or the human queue.
    fn red(&mut self, st: &mut EntryState, entry: &EntryId, result: String, feedback: &str) -> Result<Next> {
        st.last_result = Some(result.clone());
        let cycle = st.cycle + 1;
        if cycle >= self.cfg.max_cycles {
            st.cycle = cycle;
            return Ok(self.human(st, format!("{cycle} red cycles; last: {result}")));
        }
        // Counted once the feedback is written: a failed write is retried
        // as an error, not counted twice.
        let file = self.ops.write_feedback(Role::Implementer, entry, cycle, feedback)?;
        st.cycle = cycle;
        st.feedback_file = Some(file);
        self.note(st, format!("red ({}/{}): {result}; back to the implementer", st.cycle, self.cfg.max_cycles));
        st.step = Step::Implement;
        Ok(Next::Continue)
    }

    /// A failed spec check: feedback to the spec agent, which continues
    /// its work on its branch, or the human queue after `spec_cycles`.
    ///
    /// The feedback is not redacted: the spec agent wrote the held-out
    /// tests and reads the held-out store, and CI of a spec branch runs
    /// without it (CLEAN_ROOM.md hides the held-out tests from the
    /// implementer only). It goes to the spec role's own inbox, which the
    /// implementer cannot read.
    fn spec_red(&mut self, st: &mut EntryState, entry: &EntryId, result: String, feedback: &str) -> Result<Next> {
        st.last_result = Some(result.clone());
        if st.spec_cycle >= self.cfg.spec_cycles {
            let times = match st.spec_cycle + 1 {
                1 => "once".to_string(),
                n => format!("{n} times"),
            };
            return Ok(self.human(st, format!("the spec check failed {times}; last: {result}")));
        }
        let cycle = st.spec_cycle + 1;
        let file = self.ops.write_feedback(Role::Spec, entry, cycle, feedback)?;
        st.spec_cycle = cycle;
        st.spec_feedback = Some(file);
        self.note(st, format!("spec check red ({cycle}/{}): {result}; back to the spec agent", self.cfg.spec_cycles));
        st.step = Step::Spec;
        Ok(Next::Continue)
    }

    /// The agent session of this step, and the fetch of its branch.
    ///
    /// The session is recorded in the state once the role's checkout is on
    /// the branch (at `base`) and again when it ends, so it is never paid
    /// for twice: when a session is recorded (a fetch failed after it, or
    /// the loop stopped), the role's branch is fetched first, without
    /// touching the checkout; a commit beyond `base` is the session's
    /// result. Only a session that did not end and left no commit runs
    /// again, from a clean checkout. (For the spec role, a new session that
    /// is not a feedback cycle also starts without the generated and
    /// held-out tests of an earlier one: `poly-role spec run` removes them.)
    #[allow(clippy::too_many_arguments)]
    fn session(
        &mut self,
        st: &mut EntryState,
        role: Role,
        entry: &EntryId,
        sync_base: &str,
        base: &str,
        feedback: Option<&str>,
        start: String,
    ) -> Result<Ended> {
        let branch = match role {
            Role::Spec => spec_branch(entry),
            Role::Implementer => impl_branch(entry),
        };
        if let Some(s) = st.session.clone() {
            let ended = s.exit.map(|code| Out { code, text: String::new(), log: s.log.clone() });
            if let Some(out) = ended.as_ref().filter(|o| !o.ok()) {
                st.session = None;
                return Ok(Ended::Nothing(out.clone()));
            }
            let head = self.ops.role_fetch(role, &branch)?;
            st.session = None;
            match (head, ended) {
                (Some(h), _) if h != s.base => {
                    self.note(st, format!("fetched {branch} of the {} session that ran before", role.name()));
                    return Ok(Ended::Committed(h));
                }
                (_, Some(out)) => return Ok(Ended::Nothing(out)),
                (_, None) => {
                    self.note(st, format!("the {} session cut short left no commit; it runs again", role.name()))
                }
            }
        }
        let _role = self.store.lock(&format!("role-{}", role.name()))?;
        let sync = self.ops.role_sync(role, entry, &branch, sync_base)?;
        if !sync.ok() {
            return Err(format!("preparing the {} checkout failed: {}", role.name(), sync.tail(5)));
        }
        st.session = Some(Session { base: base.to_string(), exit: None, log: None });
        self.save(st);
        self.note(st, start);
        let out = self.ops.role_run(role, entry, feedback)?;
        if out.code == i32::from(crate::roles::LIMIT_EXIT) {
            st.session = None;
            return Ok(Ended::Limit(self.limit(st, role.name(), &out)?));
        }
        st.session = Some(Session { base: base.to_string(), exit: Some(out.code), log: out.log.clone() });
        self.save(st);
        if !out.ok() {
            st.session = None;
            return Ok(Ended::Nothing(out));
        }
        // An error here (git, permissions) leaves the ended session in the
        // state: the retry fetches again and does not run the agent.
        let head = self.ops.role_fetch(role, &branch)?;
        st.session = None;
        Ok(match head {
            Some(h) if h != base => Ended::Committed(h),
            _ => Ended::Nothing(out),
        })
    }

    fn push_hub(&mut self, slot: usize, rev: &str, branch: &str) -> Result<Option<String>> {
        Ok(match self.ops.push(slot, Remote::Hub, rev, branch)? {
            Push::Done => None,
            Push::Rejected(m) => Some(format!("pushing {branch} to the hub was refused (history rewritten?): {m}")),
        })
    }

    fn base(&self) -> String {
        format!("{}/{}", self.cfg.remote, self.cfg.base_branch)
    }

    fn step(&mut self, st: &mut EntryState) -> Result<Next> {
        let entry = EntryId::parse(&st.entry)?;
        let (spec_b, impl_b) = (spec_branch(&entry), impl_branch(&entry));
        let now = self.ops.now();
        match st.step {
            Step::Claim => {
                let (slot, _l) = self.slot()?;
                let message = claim_message(&entry, &self.cfg.spec.agent);
                if let Some(head) = self.ops.hub_head(&spec_b)? {
                    // Pushed before a crash; anything else is an old attempt.
                    self.ops.prepare(slot, &format!("hub/{spec_b}"))?;
                    if self.ops.subject(slot, &head)? != message {
                        return Ok(self.human(
                            st,
                            format!(
                                "{spec_b} is in the hub from an earlier attempt (`pipeline loop reset` deletes it)"
                            ),
                        ));
                    }
                    st.claim_sha = Some(head);
                } else {
                    self.ops.prepare(slot, &self.base())?;
                    let out = self.ops.claim(slot, &entry)?;
                    if !out.ok() {
                        return Ok(self.human(st, format!("coverage claim failed: {}", out.tail(3))));
                    }
                    let sha = self.ops.commit(slot, &message, &[safety::coverage_file(&entry)])?;
                    if let Some(r) = self.push_hub(slot, &sha, &spec_b)? {
                        return Ok(self.human(st, r));
                    }
                    st.claim_sha = Some(sha);
                }
                self.note(st, format!("claimed for {} on {spec_b}", self.cfg.spec.agent));
                st.step = Step::Spec;
                Ok(Next::Continue)
            }
            Step::Spec => {
                if !fetch_only(st)
                    && let Some(until) = self.store.paused_until("spec", now)?
                {
                    return Ok(Next::Yield(Yielded::Paused("spec", until)));
                }
                let claim = st.claim_sha.clone().ok_or("no claim commit in the state")?;
                if st.pending_head.is_none() {
                    // After a failed spec check the agent continues from its own head.
                    let base = st.spec_head.clone().unwrap_or(claim);
                    let start = match st.spec_cycle {
                        0 => format!("spec agent {} starts", self.cfg.spec.agent),
                        n => format!(
                            "spec agent {} starts (feedback {n}/{} on the spec check)",
                            self.cfg.spec.agent, self.cfg.spec_cycles
                        ),
                    };
                    let feedback = st.spec_feedback.clone();
                    match self.session(st, Role::Spec, &entry, &spec_b, &base, feedback.as_deref(), start)? {
                        Ended::Committed(h) => {
                            st.pending_head = Some(h);
                            self.save(st);
                        }
                        Ended::Limit(next) => return Ok(next),
                        Ended::Nothing(out) => {
                            st.spec_attempts += 1;
                            let why = if out.ok() {
                                "the spec agent committed nothing".to_string()
                            } else {
                                format!("the spec agent failed: {}", out.describe())
                            };
                            if st.spec_attempts >= self.cfg.spec_attempts {
                                return Ok(self.human(st, format!("{why} ({} attempts)", st.spec_attempts)));
                            }
                            self.note(st, format!("{why}; trying again"));
                            return Ok(Next::Continue);
                        }
                    }
                }
                let head = st.pending_head.clone().unwrap();
                let (slot, _l) = self.slot()?;
                if let Some(r) = self.push_hub(slot, &head, &spec_b)? {
                    return Ok(self.human(st, r));
                }
                st.spec_head = Some(head);
                st.pending_head = None;
                self.note(st, "spec agent done");
                st.step = Step::SpecCheck;
                Ok(Next::Continue)
            }
            Step::SpecCheck => {
                let (slot, _l) = self.slot()?;
                let head = self.ops.prepare(slot, &format!("hub/{spec_b}"))?;
                let (claim, spec_head) = (st.claim_sha.clone().unwrap_or_default(), st.spec_head.clone());
                if spec_head.as_deref() != Some(head.as_str()) {
                    return Ok(self.human(st, format!("{spec_b} in the hub is not what the spec agent committed")));
                }
                let changes = self.ops.changes(slot, &claim, &head)?;
                let problems =
                    safety::check_role("spec agent", &changes, &safety::spec_paths(&entry), &self.cfg.extra_forbidden);
                if !problems.is_empty() {
                    return Ok(self.human(st, format!("red flag in the spec agent's changes: {}", problems.join("; "))));
                }
                let files = self.ops.spec_files(slot, &entry)?;
                if !files.spec || files.visible == 0 || files.held_out == 0 {
                    let result = format!(
                        "incomplete spec: sheet {}, {} visible and {} held-out tests",
                        if files.spec { "present" } else { "missing" },
                        files.visible,
                        files.held_out
                    );
                    let feedback = format!(
                        "The runner's spec check found your spec incomplete: the spec sheet {} is {}, {} has {} \
                         visible tests and the held-out store has {} held-out tests for {}. All three are needed: \
                         write the missing parts, run `cargo xtask pipeline tests split` (with `--replace` if it \
                         split before) and commit the spec sheet and the visible tests.\n",
                        entry.spec_path().display(),
                        if files.spec { "present" } else { "missing" },
                        entry.tests_dir().display(),
                        files.visible,
                        files.held_out,
                        entry.id()
                    );
                    return self.spec_red(st, &entry, result, &feedback);
                }
                st.tests_visible = files.visible;
                st.tests_held_out = files.held_out;
                if self.cfg.spec_ci {
                    let ci = self.ops.ci(slot)?;
                    if !ci.ok() {
                        let feedback = format!(
                            "`cargo xtask ci` failed on your spec branch on the runner (the end of its output; the \
                             full log stays on the runner):\n\n{}\n",
                            ci.tail(80)
                        );
                        let result = format!("the spec branch fails `cargo xtask ci`: {}", ci.describe());
                        return self.spec_red(st, &entry, result, &feedback);
                    }
                }
                self.note(st, format!("spec checked: {} visible, {} held-out tests", files.visible, files.held_out));
                st.step = Step::Handoff;
                Ok(Next::Continue)
            }
            Step::Handoff => {
                let (slot, _l) = self.slot()?;
                let head = self.ops.prepare(slot, &format!("hub/{spec_b}"))?;
                let message = handoff_message(&entry, &self.cfg.spec.agent, &self.cfg.implementer.agent);
                if st.handoff_sha.is_none() {
                    if Some(&head) != st.spec_head.as_ref() {
                        // The record was pushed before a crash.
                        if self.ops.subject(slot, &head)? != message {
                            return Ok(self.human(st, format!("{spec_b} moved in the hub after the spec check")));
                        }
                        st.handoff_sha = Some(head);
                    } else {
                        let out = self.ops.handoff_record(slot, &entry)?;
                        if !out.ok() {
                            return Ok(self.human(st, format!("`handoff record` refused: {}", out.tail(3))));
                        }
                        let sha = self.ops.commit(slot, &message, &["specs/HANDOFF_LOG.md".to_string()])?;
                        if let Some(r) = self.push_hub(slot, &sha, &spec_b)? {
                            return Ok(self.human(st, r));
                        }
                        st.handoff_sha = Some(sha);
                    }
                    self.save(st);
                }
                st.bundle_dir = Some(self.ops.handoff_bundle(slot, &entry)?);
                self.note(st, format!("handed off to {}", self.cfg.implementer.agent));
                st.step = Step::Implement;
                Ok(Next::Continue)
            }
            Step::Implement => {
                if !fetch_only(st)
                    && let Some(until) = self.store.paused_until("implementer", now)?
                {
                    return Ok(Next::Yield(Yielded::Paused("implementer", until)));
                }
                if st.pending_head.is_none() {
                    let base_branch = if self.ops.hub_head(&impl_b)?.is_some() { &impl_b } else { &spec_b };
                    let base = match &st.session {
                        // A session that ran before: its start, as recorded.
                        Some(s) => s.base.clone(),
                        None => {
                            let base =
                                self.ops.hub_head(base_branch)?.ok_or(format!("{base_branch} is not in the hub"))?;
                            st.impl_base = Some(base.clone());
                            self.save(st);
                            base
                        }
                    };
                    let start = format!("implementer {} starts (cycle {})", self.cfg.implementer.agent, st.cycle + 1);
                    let feedback = st.feedback_file.clone();
                    match self.session(st, Role::Implementer, &entry, base_branch, &base, feedback.as_deref(), start)? {
                        Ended::Committed(h) => {
                            st.pending_head = Some(h);
                            self.save(st);
                        }
                        Ended::Limit(next) => return Ok(next),
                        Ended::Nothing(out) => {
                            let (result, feedback) = if out.ok() {
                                (
                                    "the implementer committed nothing".to_string(),
                                    "Your previous session ended without a commit on your branch. Commit your work."
                                        .to_string(),
                                )
                            } else {
                                (
                                    format!("the implementer session failed: {}", out.describe()),
                                    format!(
                                        "Your previous session failed (exit status {}). Start from the state of your branch.",
                                        out.code
                                    ),
                                )
                            };
                            return self.red(st, &entry, result, &feedback);
                        }
                    }
                }
                let head = st.pending_head.clone().unwrap();
                let (slot, _l) = self.slot()?;
                if let Some(r) = self.push_hub(slot, &head, &impl_b)? {
                    return Ok(self.human(st, r));
                }
                st.impl_head = Some(head);
                st.pending_head = None;
                self.note(st, "implementer done");
                st.step = Step::Verify;
                Ok(Next::Continue)
            }
            Step::Verify => {
                let (slot, _l) = self.slot()?;
                let head = self.ops.prepare(slot, &format!("hub/{impl_b}"))?;
                if st.impl_head.as_deref() != Some(head.as_str()) {
                    return Ok(self.human(st, format!("{impl_b} in the hub is not what the loop pushed")));
                }
                // Before running anything from the branch: what did the agent change?
                if let Some(base) = st.impl_base.clone().filter(|b| *b != head) {
                    let changes = self.ops.changes(slot, &base, &head)?;
                    let problems = safety::check_role(
                        "implementer",
                        &changes,
                        &self.cfg.implementer_paths,
                        &self.cfg.extra_forbidden,
                    );
                    if !problems.is_empty() {
                        return Ok(
                            self.human(st, format!("red flag in the implementer's changes: {}", problems.join("; ")))
                        );
                    }
                }
                let received = st.bundle_dir.clone().ok_or("no bundle directory in the state")?;
                let v = self.ops.handoff_verify(slot, &entry, &received)?;
                if !v.ok() {
                    return Ok(self.human(st, format!("`handoff verify` failed: {}", v.tail(5))));
                }
                let ci = self.ops.ci(slot)?;
                if !ci.ok() {
                    let feedback = format!(
                        "`cargo xtask ci` failed on the runner (the end of its output):\n\n{}\n",
                        self.ops.redact(&entry, &ci.tail(80))
                    );
                    return self.red(st, &entry, format!("cargo xtask ci failed ({})", ci.describe()), &feedback);
                }
                let d = self.ops.diff(slot, &entry)?;
                if !d.green {
                    return self.red(st, &entry, format!("diff red: {}", d.summary), &d.feedback);
                }
                st.last_result = Some(format!("green: {}", d.summary));
                self.note(st, format!("green: {}", d.summary));
                st.step = Step::Publish;
                Ok(Next::Continue)
            }
            Step::Publish => {
                let (slot, _l) = self.slot()?;
                let mut head = self.ops.prepare(slot, &format!("hub/{impl_b}"))?;
                let coverage = safety::coverage_file(&entry);
                if !green_in(self.ops.file_at(slot, &head, &coverage)?.as_deref(), &entry.symbol) {
                    let out = self.ops.set_green(slot, &entry, st.tests_visible, st.tests_held_out)?;
                    if !out.ok() {
                        return Ok(self.human(st, format!("setting the coverage status failed: {}", out.tail(3))));
                    }
                    let paths = [coverage.clone(), "README.md".into(), "docs/progress".into()];
                    head =
                        self.ops.commit(slot, &green_message(&entry, st.tests_visible, st.tests_held_out), &paths)?;
                }
                let base = self.ops.merge_base(slot, &self.base(), &head)?;
                let changes = self.ops.changes(slot, &base, &head)?;
                let before = self.ops.file_at(slot, &base, &coverage)?;
                let after = self.ops.file_at(slot, &head, &coverage)?;
                let problems = safety::check_pr(&PrCheck {
                    entry: &entry,
                    changes: &changes,
                    implementer_paths: &self.cfg.implementer_paths,
                    extra_forbidden: &self.cfg.extra_forbidden,
                    coverage_before: before.as_deref(),
                    coverage_after: after.as_deref(),
                });
                if !problems.is_empty() {
                    return Ok(self.human(st, format!("red flag in the PR: {}", problems.join("; "))));
                }
                let check = self.ops.coverage_check(slot)?;
                if !check.ok() {
                    return Ok(self.human(st, format!("coverage check failed: {}", check.tail(3))));
                }
                if let Some(r) = self.push_hub(slot, &head, &impl_b)? {
                    return Ok(self.human(st, r));
                }
                if let Push::Rejected(m) = self.ops.push(slot, Remote::Origin, &head, &impl_b)? {
                    return Ok(self.human(st, format!("pushing {impl_b} to {} was refused: {m}", self.cfg.remote)));
                }
                st.impl_head = Some(head.clone());
                if !self.cfg.open_prs {
                    self.note(st, format!("pushed {impl_b} to {} (PRs are off)", self.cfg.remote));
                    st.step = Step::Merged;
                    return Ok(Next::Yield(Yielded::Done(Step::Merged)));
                }
                let url = match self.ops.pr_find(&impl_b)? {
                    Some(pr) if pr.state == "OPEN" => pr.url,
                    _ => {
                        let title = format!("{}: spec, tests and implementation (agent pipeline)", entry.id());
                        let url = self.ops.pr_create(&impl_b, &title, &self.pr_body(st, &entry))?;
                        st.pr_url = Some(url.clone());
                        self.save(st);
                        if let Err(e) = self.ops.notify("pr", &st.entry, &format!("{title}: {url}")) {
                            self.log.log(4, &self.who(&st.entry), &format!("notify: {e}"));
                        }
                        url
                    }
                };
                st.pr_url = Some(url.clone());
                if self.cfg.auto_merge {
                    match self.ops.pr_auto_merge(&url) {
                        Ok(()) => self.note(st, format!("auto-merge enabled on {url}")),
                        Err(e) => self.note(st, format!("auto-merge not enabled ({e}); {url} waits for the owner")),
                    }
                }
                self.note(st, format!("PR {url}"));
                st.step = Step::AwaitingMerge;
                st.last_poll = now;
                Ok(Next::Yield(Yielded::Waiting(now + self.cfg.poll_secs)))
            }
            Step::AwaitingMerge => {
                if now < st.last_poll + self.cfg.poll_secs {
                    return Ok(Next::Yield(Yielded::Waiting(st.last_poll + self.cfg.poll_secs)));
                }
                st.last_poll = now;
                match self.ops.pr_find(&impl_b)? {
                    None => Ok(self.human(st, format!("the PR of {impl_b} is gone"))),
                    Some(pr) if pr.state == "MERGED" => {
                        self.note(st, format!("merged: {}", pr.url));
                        st.step = Step::Merged;
                        Ok(Next::Yield(Yielded::Done(Step::Merged)))
                    }
                    Some(pr) if pr.state == "CLOSED" => {
                        Ok(self.human(st, format!("{} was closed without merging", pr.url)))
                    }
                    Some(pr) if pr.mergeable == "CONFLICTING" => {
                        self.note(st, format!("{} conflicts with {}", pr.url, self.cfg.base_branch));
                        st.step = Step::Refresh;
                        Ok(Next::Continue)
                    }
                    Some(_) => Ok(Next::Yield(Yielded::Waiting(now + self.cfg.poll_secs))),
                }
            }
            Step::Refresh => {
                st.refreshes += 1;
                if st.refreshes > self.cfg.max_refresh {
                    return Ok(self.human(st, format!("the PR conflicted with main {} times", st.refreshes - 1)));
                }
                let (slot, _l) = self.slot()?;
                self.ops.prepare(slot, &format!("hub/{impl_b}"))?;
                let head = match self.ops.merge_main(slot, &entry)? {
                    Merge::Conflicts(files) => {
                        return Ok(self.human(st, format!("conflicts with main in {}", files.join(", "))));
                    }
                    Merge::UpToDate => {
                        self.note(st, "already up to date with main");
                        st.step = Step::AwaitingMerge;
                        return Ok(Next::Continue);
                    }
                    Merge::Merged(head) => head,
                };
                if let Some(r) = self.push_hub(slot, &head, &impl_b)? {
                    return Ok(self.human(st, r));
                }
                st.impl_base = Some(head.clone());
                st.impl_head = Some(head);
                self.note(st, format!("merged {} in; verifying again", self.cfg.base_branch));
                st.step = Step::Verify;
                Ok(Next::Continue)
            }
            Step::Merged | Step::Human => Ok(Next::Yield(Yielded::Done(st.step))),
        }
    }

    fn pr_body(&self, st: &EntryState, entry: &EntryId) -> String {
        format!(
            "Coverage entry `{id}`, through the agent pipeline (docs/AGENT_PIPELINE.md).\n\n\
             - Spec agent: `{spec}`; implementer: `{implementer}` (hand-off in `specs/HANDOFF_LOG.md`, \
             spec revision `{rev}`)\n\
             - Tests: {v} visible (`{tests}`), {h} held out (in the runner's held-out store)\n\
             - Result on the runner: {result}; red cycles before: {cycles}\n\
             - Checked by the loop: `cargo xtask ci`, `pipeline diff --target poly` with visible and held-out \
             tests, `pipeline handoff verify`, no `unsafe` added, only allowed paths changed\n\n\
             Merge with a **merge commit** (never squash or rebase): the hand-off log cites the spec commit \
             `{rev}` by its hash.\n",
            id = entry.id(),
            spec = self.cfg.spec.agent,
            implementer = self.cfg.implementer.agent,
            rev = st.spec_head.as_deref().unwrap_or("?"),
            v = st.tests_visible,
            h = st.tests_held_out,
            tests = entry.tests_dir().display(),
            result = st.last_result.as_deref().unwrap_or("-"),
            cycles = st.cycle,
        )
    }
}
