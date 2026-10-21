//! Everything the cycle does outside its own state, as one trait: git,
//! the repository's tools (`cargo xtask ...`), the role commands, `gh`, the
//! clock. [`super::real::RealOps`] runs the commands; the unit tests drive
//! the cycle with a fake that models the branches and scripts the agents'
//! and tools' results.

use super::safety::Change;
use crate::roles::Role;
use crate::store::EntryId;
use crate::util::Result;
use std::path::PathBuf;

/// What a command did. `text` is its output (stdout and stderr).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub code: i32,
    pub text: String,
    /// Where the full output is kept.
    pub log: Option<PathBuf>,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    #[cfg(test)]
    pub fn with(code: i32, text: impl Into<String>) -> Out {
        Out { code, text: text.into(), log: None }
    }

    /// The last `n` lines of the output.
    pub fn tail(&self, n: usize) -> String {
        let lines: Vec<&str> = self.text.lines().collect();
        lines[lines.len().saturating_sub(n)..].join("\n")
    }

    /// `exit N (log FILE)` for messages.
    pub fn describe(&self) -> String {
        match &self.log {
            Some(l) => format!("exit {} (log {})", self.code, l.display()),
            None => format!("exit {}", self.code),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    /// The local bare repository the role checkouts fetch from.
    Hub,
    /// GitHub (`git.remote`), where PRs are opened.
    Origin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Push {
    Done,
    /// Not a fast-forward (or refused): the loop never forces.
    Rejected(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pr {
    pub url: String,
    /// OPEN, MERGED or CLOSED.
    pub state: String,
    /// MERGEABLE, CONFLICTING or UNKNOWN.
    pub mergeable: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecFiles {
    pub spec: bool,
    pub visible: u32,
    pub held_out: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffResult {
    /// Every visible and held-out test of the entry passes on Poly.
    pub green: bool,
    /// One line: `visible 7/7, held-out 2/3`.
    pub summary: String,
    /// What the implementer gets back: visible tests in full, held-out
    /// tests only as a count (CLEAN_ROOM.md: it never sees them).
    pub feedback: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Merge {
    UpToDate,
    /// Main merged in (conflicts in generated files resolved); the new head.
    Merged(String),
    /// Conflicts the loop cannot resolve: these files.
    Conflicts(Vec<String>),
}

pub trait Ops {
    /// Seconds since the epoch.
    fn now(&mut self) -> u64;
    fn sleep(&mut self, secs: u64);
    /// The entry the following calls are for (log files), if any.
    fn set_entry(&mut self, entry: Option<&str>);

    /// Bring the runner's checkout of the base branch up to date.
    fn refresh_main(&mut self) -> Result;
    /// Unclaimed entries, best first (`pipeline next`).
    fn queue(&mut self) -> Result<Vec<String>>;
    /// The hub's head of `branch`.
    fn hub_head(&mut self, branch: &str) -> Result<Option<String>>;

    /// Fetch, then check out `start` (`origin/main`, `hub/<branch>`)
    /// detached and clean in runner worktree `slot`; the head.
    fn prepare(&mut self, slot: usize, start: &str) -> Result<String>;
    fn subject(&mut self, slot: usize, rev: &str) -> Result<String>;
    /// Commit `paths` (files or directories); the new head.
    fn commit(&mut self, slot: usize, message: &str, paths: &[String]) -> Result<String>;
    fn push(&mut self, slot: usize, remote: Remote, rev: &str, branch: &str) -> Result<Push>;
    fn changes(&mut self, slot: usize, from: &str, to: &str) -> Result<Vec<Change>>;
    fn merge_base(&mut self, slot: usize, a: &str, b: &str) -> Result<String>;
    fn file_at(&mut self, slot: usize, rev: &str, path: &str) -> Result<Option<String>>;

    /// `coverage claim <entry> --agent <spec agent>`.
    fn claim(&mut self, slot: usize, entry: &EntryId) -> Result<Out>;
    /// `cargo xtask ci`.
    fn ci(&mut self, slot: usize) -> Result<Out>;
    fn handoff_record(&mut self, slot: usize, entry: &EntryId) -> Result<Out>;
    /// `handoff bundle` into the implementer's inbox; the directory.
    fn handoff_bundle(&mut self, slot: usize, entry: &EntryId) -> Result<String>;
    fn handoff_verify(&mut self, slot: usize, entry: &EntryId, received: &str) -> Result<Out>;
    fn spec_files(&mut self, slot: usize, entry: &EntryId) -> Result<SpecFiles>;
    /// `pipeline diff --target poly` with the entry's visible and held-out tests.
    fn diff(&mut self, slot: usize, entry: &EntryId) -> Result<DiffResult>;
    /// `coverage set-status <entry> green` with the test counts, then `coverage readme`.
    fn set_green(&mut self, slot: usize, entry: &EntryId, visible: u32, held_out: u32) -> Result<Out>;
    /// `coverage check` and `coverage readme --check`.
    fn coverage_check(&mut self, slot: usize) -> Result<Out>;
    /// Merge the base branch into the checked-out head.
    fn merge_main(&mut self, slot: usize, entry: &EntryId) -> Result<Merge>;

    /// The role's checkout: `branch` at the hub's `base`, clean.
    fn role_sync(&mut self, role: Role, entry: &EntryId, branch: &str, base: &str) -> Result<Out>;
    /// One agent session (`pipeline role`); exit 75 means a usage limit.
    fn role_run(&mut self, role: Role, entry: &EntryId, feedback: Option<&str>) -> Result<Out>;
    /// The head of `branch` in the role's checkout, fetched into the runner.
    fn role_fetch(&mut self, role: Role, branch: &str) -> Result<Option<String>>;

    /// `text` without anything that could reveal a held-out test.
    fn redact(&mut self, entry: &EntryId, text: &str) -> String;
    /// Write feedback for the role's next session into that role's inbox
    /// (the implementer's, or the spec agent's own); the file.
    fn write_feedback(&mut self, role: Role, entry: &EntryId, cycle: u32, text: &str) -> Result<String>;
    /// A clock time from a usage-limit message (`3pm`, in `tz`) as epoch seconds.
    fn resolve_clock(&mut self, time: &str, tz: Option<&str>) -> Option<u64>;

    fn pr_find(&mut self, branch: &str) -> Result<Option<Pr>>;
    fn pr_create(&mut self, branch: &str, title: &str, body: &str) -> Result<String>;
    fn pr_auto_merge(&mut self, url: &str) -> Result;
    fn notify(&mut self, event: &str, entry: &str, message: &str) -> Result;
    fn append_human_queue(&mut self, text: &str) -> Result;
}
