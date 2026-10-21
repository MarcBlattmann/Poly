//! The scheduler's state on disk, so a restart resumes every entry at the
//! step it was at:
//!
//! ```text
//! <state>/entries/<os>__<layer>__<component>__<symbol>.json   one file per entry (atomic rename)
//! <state>/locks/entry-<key>.lock    held while a worker drives the entry
//! <state>/locks/role-<role>.lock    one agent session per role (one checkout per role)
//! <state>/locks/slot-<n>.lock       a runner worktree in use
//! <state>/locks/pick.lock           choosing and creating a new entry
//! <state>/pause-<role>.json         usage limit or `loop pause`: the role waits until then
//! <state>/deploy-notice.json        the deploy files the owner was told to install (update.rs)
//! <state>/self-update.json          the loop's new code did not build (update.rs)
//! <state>/logs/<key>/...            output of every command the loop ran for the entry
//! ```
//!
//! Locks are `flock` locks on the files (`std::fs::File::lock`): they go
//! away with the process, so a crash never leaves a stale lock, and two
//! loops (or two workers of one loop) never drive the same entry.

use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "poly-loop-state/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Step {
    /// Claim the entry on a new spec branch.
    Claim,
    /// The spec agent writes the spec sheet and tests.
    Spec,
    /// The runner checks the spec branch (paths, files, CI).
    SpecCheck,
    /// Hand-off record (committed) and bundle into the implementer's inbox.
    Handoff,
    /// The implementer works on its branch.
    Implement,
    /// The runner checks the implementer's branch: safety, hand-off, CI, diff.
    Verify,
    /// Green: main merged in, status update, final checks, push, PR.
    Publish,
    /// The PR waits for CI and the merge.
    AwaitingMerge,
    /// GitHub reports the PR `dirty` (it conflicts with main): counted
    /// against `max_refresh`, then publish again, which merges main in.
    Refresh,
    /// Done: the PR was merged (or, without PRs, the branch was pushed).
    Merged,
    /// Stopped: in the human queue.
    Human,
}

impl Step {
    pub fn name(self) -> &'static str {
        match self {
            Step::Claim => "claim",
            Step::Spec => "spec",
            Step::SpecCheck => "spec-check",
            Step::Handoff => "handoff",
            Step::Implement => "implement",
            Step::Verify => "verify",
            Step::Publish => "publish",
            Step::AwaitingMerge => "awaiting-merge",
            Step::Refresh => "refresh",
            Step::Merged => "merged",
            Step::Human => "human",
        }
    }

    pub fn terminal(self) -> bool {
        matches!(self, Step::Merged | Step::Human)
    }

    /// The LLM role this step runs, if any.
    pub fn role(self) -> Option<&'static str> {
        match self {
            Step::Spec => Some("spec"),
            Step::Implement => Some("implementer"),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub time: String,
    pub step: Step,
    pub message: String,
}

/// An agent session of the entry's current step (spec or implement),
/// recorded once its role checkout is on the branch, before the session
/// starts, so the steps after it can resume without paying for the
/// session again: a retry after a failed fetch, or after a crash, first
/// fetches the role's branch and runs the agent again only when the branch
/// has no commit beyond `base`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// The branch head the session started from.
    pub base: String,
    /// Its exit status once it ended; `None` while it runs (or if the loop
    /// stopped before it ended).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// Its log, for messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntryState {
    pub format: String,
    pub entry: String,
    pub step: Step,
    /// Red cycles so far (diff or CI red, or an implementer session that
    /// failed or committed nothing). Usage limits never count.
    #[serde(default)]
    pub cycle: u32,
    #[serde(default)]
    pub spec_attempts: u32,
    /// Spec checks that failed and went back to the spec agent with
    /// feedback (CI red, an incomplete spec); at most `spec_cycles`.
    #[serde(default)]
    pub spec_cycle: u32,
    /// Consecutive infrastructure errors (reset by every step that succeeds).
    #[serde(default)]
    pub errors: u32,
    #[serde(default)]
    pub refreshes: u32,
    /// Not before this time (seconds since the epoch): error back-off.
    #[serde(default)]
    pub retry_at: u64,
    #[serde(default)]
    pub last_poll: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_sha: Option<String>,
    /// The implementer branch before the latest session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impl_base: Option<String>,
    /// The implementer branch after the latest session (checked by Verify).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impl_head: Option<String>,
    /// Fetched from a role's checkout but not yet pushed to the hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_head: Option<String>,
    /// The agent session of this step that ran, or was running, and
    /// whose branch has not been fetched yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Session>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback_file: Option<String>,
    /// Feedback for the spec agent's next session (a failed spec check).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_feedback: Option<String>,
    /// What the implementer received (`handoff bundle`), for `handoff verify --received`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_dir: Option<String>,
    #[serde(default)]
    pub tests_visible: u32,
    #[serde(default)]
    pub tests_held_out: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created: String,
    pub updated: String,
    #[serde(default)]
    pub history: Vec<Event>,
}

impl EntryState {
    pub fn new(entry: &str, now: u64) -> EntryState {
        EntryState {
            format: FORMAT.into(),
            entry: entry.into(),
            step: Step::Claim,
            cycle: 0,
            spec_attempts: 0,
            spec_cycle: 0,
            errors: 0,
            refreshes: 0,
            retry_at: 0,
            last_poll: 0,
            claim_sha: None,
            spec_head: None,
            handoff_sha: None,
            impl_base: None,
            impl_head: None,
            pending_head: None,
            session: None,
            feedback_file: None,
            spec_feedback: None,
            bundle_dir: None,
            tests_visible: 0,
            tests_held_out: 0,
            last_result: None,
            pr_url: None,
            reason: None,
            created: util::format_utc(now),
            updated: util::format_utc(now),
            history: Vec::new(),
        }
    }

    pub fn note(&mut self, now: u64, message: impl Into<String>) {
        self.updated = util::format_utc(now);
        self.history.push(Event { time: self.updated.clone(), step: self.step, message: message.into() });
        // Keep the file small: the logs have everything.
        if self.history.len() > 200 {
            self.history.drain(..self.history.len() - 200);
        }
    }

    /// The entry's symbol (the last part of the id).
    pub fn symbol(&self) -> &str {
        self.entry.rsplit('/').next().unwrap_or(&self.entry)
    }
}

/// File-name form of an entry id.
pub fn key(entry: &str) -> String {
    entry.replace('/', "__")
}

/// `<state>/pause-<role>.json`: no new session of the role before `until`
/// (a usage limit, or `loop pause`; [`FOREVER`] until `loop resume`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pause {
    pub until: u64,
    pub reason: String,
}

/// A pause without an end: until `loop resume`.
pub const FOREVER: u64 = u64::MAX;

/// `until 2026-10-21 18:00:00 UTC`, or `until \`loop resume\``.
pub fn until_text(until: u64) -> String {
    if until == FOREVER { "until `loop resume`".into() } else { format!("until {} UTC", util::format_utc(until)) }
}

/// A held lock; dropping it unlocks.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Result<Store> {
        for sub in ["entries", "locks", "logs"] {
            util::create_dir(&dir.join(sub))?;
        }
        Ok(Store { dir: dir.to_path_buf() })
    }

    fn entry_path(&self, entry: &str) -> PathBuf {
        self.dir.join("entries").join(format!("{}.json", key(entry)))
    }

    pub fn logs(&self, entry: &str) -> PathBuf {
        self.dir.join("logs").join(key(entry))
    }

    pub fn load(&self, entry: &str) -> Result<Option<EntryState>> {
        let p = self.entry_path(entry);
        if !p.is_file() {
            return Ok(None);
        }
        util::read_json(&p).map(Some)
    }

    pub fn load_all(&self) -> Result<Vec<EntryState>> {
        let mut out = Vec::new();
        let dir = self.dir.join("entries");
        for rel in util::files_under(&dir)? {
            if rel.extension().is_some_and(|x| x == "json") && rel.components().count() == 1 {
                out.push(util::read_json(&dir.join(rel))?);
            }
        }
        Ok(out)
    }

    /// Write atomically: a crash leaves the old or the new state, never half.
    pub fn save(&self, s: &EntryState) -> Result {
        write_atomic(&self.entry_path(&s.entry), &(serde_json::to_string_pretty(s).map_err(|e| e.to_string())? + "\n"))
    }

    pub fn remove(&self, entry: &str) -> Result {
        let p = self.entry_path(entry);
        std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))
    }

    fn lock_file(&self, name: &str) -> Result<(File, PathBuf)> {
        let path = self.dir.join("locks").join(format!("{name}.lock"));
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok((f, path))
    }

    /// The lock `name` if nobody holds it.
    ///
    /// A lock just released can look held for a moment: a child process
    /// another thread is starting holds a copy of every descriptor between
    /// its fork and its exec, and the `flock` lasts until the last copy is
    /// closed. So "held" is believed only after retrying for a second: on
    /// a loaded host the window between a fork and its exec can last much
    /// longer than a few milliseconds.
    pub fn try_lock(&self, name: &str) -> Result<Option<Lock>> {
        let (f, path) = self.lock_file(name)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match f.try_lock() {
                Ok(()) => return Ok(Some(Lock { _file: f })),
                Err(TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
            }
        }
    }

    /// Wait for the lock `name`.
    pub fn lock(&self, name: &str) -> Result<Lock> {
        let (f, path) = self.lock_file(name)?;
        f.lock().map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Lock { _file: f })
    }

    /// Whether someone holds the lock `name` (for `status`).
    pub fn locked(&self, name: &str) -> bool {
        matches!(self.try_lock(name), Ok(None))
    }

    pub fn pause(&self, role: &str) -> Result<Option<Pause>> {
        let p = self.dir.join(format!("pause-{role}.json"));
        if !p.is_file() {
            return Ok(None);
        }
        util::read_json(&p).map(Some)
    }

    pub fn set_pause(&self, role: &str, pause: &Pause) -> Result {
        write_atomic(
            &self.dir.join(format!("pause-{role}.json")),
            &(serde_json::to_string_pretty(pause).map_err(|e| e.to_string())? + "\n"),
        )
    }

    /// Remove `role`'s pause; whether there was one.
    pub fn clear_pause(&self, role: &str) -> Result<bool> {
        let p = self.dir.join(format!("pause-{role}.json"));
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("{}: {e}", p.display())),
        }
    }

    /// The time `role` may run again, if it is paused at `now`.
    pub fn paused_until(&self, role: &str, now: u64) -> Result<Option<u64>> {
        Ok(self.pause(role)?.map(|p| p.until).filter(|&u| u > now))
    }
}

pub fn write_atomic(path: &Path, text: &str) -> Result {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    util::write(&tmp, text)?;
    if let Ok(f) = File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("poly-loop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn states_roundtrip_and_locks_exclude() {
        let dir = temp("store");
        let store = Store::new(&dir).unwrap();
        let mut s = EntryState::new("linux/syscall/x86_64/brk", 1_792_454_400);
        s.step = Step::Implement;
        s.cycle = 2;
        s.note(1_792_454_401, "red: 1 of 3 held-out tests fail");
        store.save(&s).unwrap();
        assert_eq!(store.load("linux/syscall/x86_64/brk").unwrap(), Some(s.clone()));
        assert_eq!(store.load_all().unwrap(), [s.clone()]);
        assert_eq!(s.symbol(), "brk");
        assert_eq!(key(&s.entry), "linux__syscall__x86_64__brk");

        let held = store.try_lock("entry-x").unwrap().unwrap();
        assert!(store.try_lock("entry-x").unwrap().is_none(), "a second open file description must not get it");
        assert!(store.locked("entry-x"));
        drop(held);
        assert!(store.try_lock("entry-x").unwrap().is_some());

        assert_eq!(store.paused_until("spec", 10).unwrap(), None);
        store.set_pause("spec", &Pause { until: 100, reason: "limit".into() }).unwrap();
        assert_eq!(store.paused_until("spec", 10).unwrap(), Some(100));
        assert_eq!(store.paused_until("spec", 100).unwrap(), None);
        store.remove(&s.entry).unwrap();
        assert!(store.load_all().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
