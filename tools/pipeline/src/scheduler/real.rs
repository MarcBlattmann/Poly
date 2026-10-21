//! [`Ops`] by running commands: git in the runner's checkout and its
//! worktrees, `cargo xtask ...` there, the role commands (`sudo -u <role
//! user> /srv/poly/bin/poly-role ...`), `gh`, `date`, the notify hook.
//! Every process goes through a [`Shell`], so `--dry-run` swaps in
//! [`DryShell`], which prints the commands and pretends they succeed.

use super::config::Config;
use super::ops::{DiffResult, Merge, Mergeable, Ops, Out, Pr, Push, Remote, SpecFiles};
use super::safety::{self, Change};
use super::state;
use crate::diff::{Summary, Verdict};
use crate::roles::Role;
use crate::store::{self, EntryId, HeldOutManifest};
use crate::util::{self, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One process to run (no shell).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    /// Send stdout and stderr to this file (long commands); else capture them.
    pub log: Option<PathBuf>,
}

impl Cmd {
    pub fn new(program: &str, args: &[&str]) -> Cmd {
        Cmd { program: program.into(), args: args.iter().map(|s| s.to_string()).collect(), ..Cmd::default() }
    }

    pub fn line(&self) -> String {
        let mut s = String::new();
        if let Some(d) = &self.cwd {
            let _ = write!(s, "(cd {}) ", util::shell_quote(&d.display().to_string()));
        }
        for (k, v) in &self.env {
            let _ = write!(s, "{k}={} ", util::shell_quote(v));
        }
        s.push_str(&util::shell_quote(&self.program));
        for a in &self.args {
            s.push(' ');
            s.push_str(&util::shell_quote(a));
        }
        s
    }
}

/// Processes, files and the clock.
pub trait Shell: Send {
    /// Captured commands: `text` is stdout, plus stderr when it failed.
    /// Logged commands: `text` is the whole log.
    fn run(&mut self, cmd: &Cmd) -> Result<Out>;
    fn read(&mut self, path: &Path) -> Result<String>;
    /// Files under `dir`, relative to it, sorted (empty if it does not exist).
    fn list(&mut self, dir: &Path) -> Result<Vec<String>>;
    fn exists(&mut self, path: &Path) -> bool;
    fn write(&mut self, path: &Path, text: &str) -> Result;
    fn append(&mut self, path: &Path, text: &str) -> Result;
    fn remove_dir(&mut self, path: &Path) -> Result;
    fn now(&mut self) -> u64;
    fn sleep(&mut self, secs: u64);
}

#[derive(Debug, Default)]
pub struct RealShell;

impl Shell for RealShell {
    fn run(&mut self, cmd: &Cmd) -> Result<Out> {
        let mut c = Command::new(&cmd.program);
        c.args(&cmd.args).stdin(Stdio::null());
        if let Some(d) = &cmd.cwd {
            c.current_dir(d);
        }
        for (k, v) in &cmd.env {
            c.env(k, v);
        }
        for k in &cmd.env_remove {
            c.env_remove(k);
        }
        if let Some(log) = &cmd.log {
            util::write(log, format!("$ {}\n", cmd.line()))?;
            let file =
                std::fs::OpenOptions::new().append(true).open(log).map_err(|e| format!("{}: {e}", log.display()))?;
            let err = file.try_clone().map_err(|e| e.to_string())?;
            let status = c.stdout(file).stderr(err).status().map_err(|e| format!("{}: {e}", cmd.program))?;
            let text = String::from_utf8_lossy(&util::read(log)?).into_owned();
            return Ok(Out { code: status.code().unwrap_or(-1), text, log: Some(log.clone()) });
        }
        let o = c.output().map_err(|e| format!("{}: {e}", cmd.program))?;
        let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
        if !o.status.success() {
            text.push_str(&String::from_utf8_lossy(&o.stderr));
        }
        Ok(Out { code: o.status.code().unwrap_or(-1), text, log: None })
    }
    fn read(&mut self, path: &Path) -> Result<String> {
        util::read_string(path)
    }
    fn list(&mut self, dir: &Path) -> Result<Vec<String>> {
        Ok(util::files_under(dir)?.into_iter().map(|p| p.display().to_string()).collect())
    }
    fn exists(&mut self, path: &Path) -> bool {
        path.exists()
    }
    fn write(&mut self, path: &Path, text: &str) -> Result {
        util::write(path, text)
    }
    fn append(&mut self, path: &Path, text: &str) -> Result {
        use std::io::Write;
        if let Some(d) = path.parent() {
            util::create_dir(d)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        f.write_all(text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
    }
    fn remove_dir(&mut self, path: &Path) -> Result {
        match std::fs::remove_dir_all(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
            _ => Ok(()),
        }
    }
    fn now(&mut self) -> u64 {
        util::now_secs()
    }
    fn sleep(&mut self, secs: u64) {
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

/// Prints every command and pretends: git keeps a small model of heads so
/// the cycle sees progress, the agents "commit", the diff is green.
#[derive(Debug, Default)]
pub struct DryShell {
    heads: BTreeMap<String, String>,
    refs: BTreeMap<String, String>,
    counter: u32,
    /// Every command, in order (tests read it).
    pub lines: Vec<String>,
    pub quiet: bool,
}

impl DryShell {
    pub fn new(quiet: bool) -> DryShell {
        DryShell { quiet, ..DryShell::default() }
    }

    fn sha(&mut self) -> String {
        self.counter += 1;
        format!("{:040x}", 0xd000 + self.counter)
    }

    fn resolve(&self, rev: &str) -> String {
        if let Some(b) = rev.strip_prefix("hub/") {
            return self.refs.get(&format!("refs/heads/{b}")).cloned().unwrap_or_else(|| "0".repeat(40));
        }
        if rev.contains('/') || rev == "HEAD" {
            return self.refs.get(rev).cloned().unwrap_or_else(|| format!("{:040x}", 0xd000));
        }
        rev.to_string()
    }

    fn say(&mut self, line: String) {
        if !self.quiet {
            println!("[dry-run] {line}");
        }
        self.lines.push(line);
    }
}

pub const DRY_VISIBLE: [&str; 2] = ["dry_visible_1.c", "dry_visible_2.c"];
pub const DRY_HELD_OUT: [&str; 1] = ["dry_heldout_1.c"];

impl Shell for DryShell {
    fn run(&mut self, cmd: &Cmd) -> Result<Out> {
        self.say(cmd.line());
        let a: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
        let ok = |text: &str| Ok(Out { code: 0, text: text.to_string(), log: cmd.log.clone() });
        match cmd.program.as_str() {
            "git" => {
                let dir = a.iter().position(|x| *x == "-C").and_then(|i| a.get(i + 1)).copied().unwrap_or(".");
                // The subcommand: the first argument that is not `-C DIR` or `-c KEY=VALUE`.
                let mut i = 0;
                while i < a.len() && (a[i] == "-C" || a[i] == "-c") {
                    i += 2;
                }
                let sub = a.get(i).copied().unwrap_or("");
                let rest = a.get(i + 1..).unwrap_or(&[]);
                match sub {
                    "rev-parse" => {
                        let rev = rest.last().copied().unwrap_or("HEAD");
                        let sha = if rev == "HEAD" {
                            self.heads.get(dir).cloned().unwrap_or_else(|| self.resolve("HEAD"))
                        } else {
                            self.resolve(rev)
                        };
                        ok(&format!("{sha}\n"))
                    }
                    "checkout" => {
                        let start = rest.last().copied().unwrap_or("HEAD");
                        let sha = self.resolve(start);
                        self.heads.insert(dir.to_string(), sha);
                        ok("")
                    }
                    "commit" | "merge" if !rest.contains(&"--abort") => {
                        if sub == "merge" {
                            return ok("Already up to date.\n");
                        }
                        let sha = self.sha();
                        self.heads.insert(dir.to_string(), sha);
                        ok("")
                    }
                    "push" => {
                        if let Some(spec) = rest.iter().find(|x| x.contains(":refs/heads/")) {
                            let (rev, dst) = spec.split_once(':').unwrap();
                            let sha = self.resolve(rev);
                            if rest.iter().find(|r| !r.starts_with('-')).is_some_and(|r| r.starts_with('/')) {
                                self.refs.insert(dst.to_string(), sha);
                            }
                        }
                        ok("")
                    }
                    "ls-remote" => {
                        let r = rest.last().copied().unwrap_or("");
                        match self.refs.get(r) {
                            Some(sha) => ok(&format!("{sha}\t{r}\n")),
                            None => ok(""),
                        }
                    }
                    "fetch" => {
                        // A role checkout: the agent "committed".
                        if let Some(spec) =
                            rest.iter().find(|x| x.starts_with("+refs/heads/") && x.contains(":refs/pipeline/"))
                        {
                            let dst = spec.split_once(':').unwrap().1.to_string();
                            let sha = self.sha();
                            self.refs.insert(dst, sha);
                        }
                        ok("")
                    }
                    "log" => ok("\n"),
                    "merge-base" => ok(&format!("{:040x}\n", 0xd000)),
                    "show" => Ok(Out { code: 128, text: "fatal: path not in the dry run".into(), log: None }),
                    _ => ok(""),
                }
            }
            "gh" => match (a.first().copied(), a.get(1).copied()) {
                // No PR yet; once created, mergeable.
                (Some("api"), Some(path)) if path.contains("/pulls?") => ok("[]"),
                (Some("api"), Some(path)) if path.contains("/pulls/") => {
                    ok("{\"html_url\":\"https://github.com/OWNER/REPO/pull/0\",\"number\":0,\"state\":\"open\",\
                     \"merged\":false,\"mergeable\":true,\"mergeable_state\":\"clean\"}")
                }
                (Some("pr"), Some("create")) => ok("https://github.com/OWNER/REPO/pull/0\n"),
                _ => ok(""),
            },
            "date" => ok(&format!("{}\n", util::now_secs() + 3600)),
            _ => ok(""),
        }
    }
    fn read(&mut self, path: &Path) -> Result<String> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name == "summary.json" {
            let mut s =
                Summary::new(util::now_utc(), "poly", "dry run".into(), "ptrace", crate::diff::SyscallMode::Exact);
            for t in DRY_VISIBLE.iter().chain(&DRY_HELD_OUT) {
                let mut r = crate::diff::TestResult::not_run(t.trim_end_matches(".c"), Verdict::Pass, String::new());
                r.reason = None;
                s.tests.push(r);
            }
            s.finish(None);
            return serde_json::to_string(&s).map_err(|e| e.to_string());
        }
        if name == store::MANIFEST {
            return Ok(format!(
                "{{\"format\":\"dry\",\"entry\":\"dry\",\"created\":\"\",\"spec_agent\":\"dry\",\"spec_revision\":\"\",\
                 \"ratio\":0.3,\"visible\":[{}],\"held_out\":[{}]}}",
                DRY_VISIBLE
                    .iter()
                    .map(|f| format!("{{\"path\":\"{f}\",\"sha256\":\"0\"}}"))
                    .collect::<Vec<_>>()
                    .join(","),
                DRY_HELD_OUT
                    .iter()
                    .map(|f| format!("{{\"path\":\"{f}\",\"sha256\":\"0\"}}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        Err(format!("{}: not read in a dry run", path.display()))
    }
    fn list(&mut self, dir: &Path) -> Result<Vec<String>> {
        if dir.to_string_lossy().ends_with(".tests") {
            Ok(DRY_VISIBLE.iter().map(|s| s.to_string()).collect())
        } else {
            Ok(std::iter::once(store::MANIFEST).chain(DRY_HELD_OUT).map(String::from).collect())
        }
    }
    fn exists(&mut self, _path: &Path) -> bool {
        true
    }
    fn write(&mut self, path: &Path, text: &str) -> Result {
        self.say(format!("write {} ({} bytes)", path.display(), text.len()));
        Ok(())
    }
    fn append(&mut self, path: &Path, text: &str) -> Result {
        self.say(format!("append to {} ({} bytes)", path.display(), text.len()));
        Ok(())
    }
    fn remove_dir(&mut self, path: &Path) -> Result {
        self.say(format!("rm -rf {}", path.display()));
        Ok(())
    }
    fn now(&mut self) -> u64 {
        util::now_secs()
    }
    fn sleep(&mut self, _secs: u64) {}
}

#[derive(Debug)]
pub struct RealOps<S: Shell> {
    pub cfg: Config,
    pub shell: S,
    /// Where `next` reads the coverage map (the runner checkout; in a dry
    /// run, the checkout running the command).
    pub queue_repo: PathBuf,
    pub logs: PathBuf,
    entry: Option<String>,
    n: u32,
}

impl<S: Shell> RealOps<S> {
    pub fn new(cfg: Config, shell: S, logs: &Path) -> RealOps<S> {
        let queue_repo = cfg.repo.clone();
        RealOps { cfg, shell, queue_repo, logs: logs.to_path_buf(), entry: None, n: 0 }
    }

    fn slot_dir(&self, slot: usize) -> PathBuf {
        self.cfg.worktrees.join(format!("w{slot}"))
    }

    fn log_file(&mut self, what: &str) -> PathBuf {
        self.n += 1;
        let dir = match &self.entry {
            Some(e) => self.logs.join(state::key(e)),
            None => self.logs.join("loop"),
        };
        dir.join(format!("{}-{}-{:03}-{what}.log", util::now_stamp(), std::process::id(), self.n))
    }

    fn git_cmd(&self, dir: &Path, args: &[&str]) -> Cmd {
        let mut all = vec!["-C".to_string(), dir.display().to_string()];
        if let Some((name, email)) = self.cfg.author.as_deref().and_then(parse_author) {
            all.extend(["-c".into(), format!("user.name={name}"), "-c".into(), format!("user.email={email}")]);
        }
        all.extend(args.iter().map(|s| s.to_string()));
        Cmd { program: "git".into(), args: all, ..Cmd::default() }
    }

    fn git(&mut self, dir: &Path, args: &[&str]) -> Result<Out> {
        let c = self.git_cmd(dir, args);
        self.shell.run(&c)
    }

    /// Stdout of a git command that must succeed.
    fn git_ok(&mut self, dir: &Path, args: &[&str]) -> Result<String> {
        let o = self.git(dir, args)?;
        if !o.ok() {
            return Err(format!("git {}: {}", args.join(" "), o.text.trim()));
        }
        Ok(o.text.trim().to_string())
    }

    /// `cargo xtask <args>` in a worktree, logged.
    fn xtask(&mut self, slot: usize, args: &[String], what: &str, heldout: bool) -> Result<Out> {
        let mut c = Cmd {
            program: self.cfg.cargo.clone(),
            args: std::iter::once("xtask".to_string()).chain(args.iter().cloned()).collect(),
            cwd: Some(self.slot_dir(slot)),
            ..Cmd::default()
        };
        if heldout {
            c.env.push((store::HELDOUT_ENV.into(), self.cfg.heldout.display().to_string()));
        } else {
            c.env_remove.push(store::HELDOUT_ENV.into());
        }
        c.log = Some(self.log_file(what));
        self.shell.run(&c)
    }

    fn role_cfg(&self, role: Role) -> &super::config::RoleConfig {
        match role {
            Role::Spec => &self.cfg.spec,
            Role::Implementer => &self.cfg.implementer,
        }
    }

    fn role_cmd(&mut self, role: Role, extra: &[String], what: &str) -> Result<Out> {
        let rc = self.role_cfg(role).clone();
        let mut c = Cmd {
            program: rc.command[0].clone(),
            args: rc.command[1..].iter().cloned().chain(extra.iter().cloned()).collect(),
            ..Cmd::default()
        };
        c.env_remove.push(store::HELDOUT_ENV.into());
        c.log = Some(self.log_file(what));
        let out = self.shell.run(&c)?;
        // sudo itself failing is the server's problem, not the agent's.
        if !out.ok() && out.text.lines().skip(1).take(3).any(|l| l.starts_with("sudo: ")) {
            return Err(format!("{}: {}", c.program, out.tail(3)));
        }
        Ok(out)
    }

    /// `git add -A` of those `paths` that exist (a pathspec matching
    /// nothing is an error to git).
    fn add(&mut self, dir: &Path, paths: &[String]) -> Result {
        let mut add = vec!["add", "-A", "--"];
        add.extend(paths.iter().filter(|p| self.shell.exists(&dir.join(p))).map(String::as_str));
        if add.len() > 3 {
            self.git_ok(dir, &add)?;
        }
        Ok(())
    }

    fn heldout_names(&mut self, entry: &EntryId) -> Result<Vec<String>> {
        let dir = entry.heldout_dir(&self.cfg.heldout);
        Ok(self.shell.list(&dir)?.into_iter().filter(|f| f != store::MANIFEST).collect())
    }

    /// The rest of [`Ops::merge_main`] once `git merge --no-commit` ran
    /// (`o`) and did not find the branch up to date: resolve the conflicts
    /// the loop may resolve, make the README progress section again, commit.
    /// Anything but `Merged` leaves the merge to be aborted by the caller.
    fn finish_merge(&mut self, slot: usize, entry: &EntryId, main: &str, o: &Out) -> Result<Merge> {
        let dir = self.slot_dir(slot);
        let conflicted = self.git_ok(&dir, &["diff", "--name-only", "--diff-filter=U"])?;
        let files: Vec<String> = conflicted.lines().filter(|l| !l.is_empty()).map(String::from).collect();
        if !o.ok() && files.is_empty() {
            return Err(format!("git merge {main}: {}", o.tail(3)));
        }
        let coverage = safety::coverage_file(entry);
        let mut refused = Vec::new();
        let mut plan = Vec::new();
        for f in &files {
            match conflict_rule(f, &coverage) {
                None => refused.push(f.clone()),
                Some(Resolve::Coverage) => {
                    // Main's version is taken only if main left this entry alone.
                    let base = self.git(&dir, &["show", &format!(":1:{f}")])?;
                    let theirs = self.git(&dir, &["show", &format!(":3:{f}")])?;
                    if !base.ok() || !theirs.ok() || entry_changed(&base.text, &theirs.text, &entry.symbol) {
                        refused.push(format!("{f} (main changed {} itself)", entry.id()));
                    } else {
                        plan.push((f.clone(), Resolve::Coverage));
                    }
                }
                Some(rule) => plan.push((f.clone(), rule)),
            }
        }
        if !refused.is_empty() {
            return Ok(Merge::Conflicts(refused));
        }
        for (f, rule) in &plan {
            match rule {
                Resolve::Generated | Resolve::Coverage => {
                    if self.git(&dir, &["show", &format!(":3:{f}")])?.ok() {
                        self.git_ok(&dir, &["checkout", "--theirs", "--", f])?;
                        self.git_ok(&dir, &["add", "--", f])?;
                    } else {
                        // Main removed it: `coverage readme` makes what is needed.
                        self.git_ok(&dir, &["rm", "-q", "--", f])?;
                    }
                }
                Resolve::Log => {
                    let ours = self.git(&dir, &["show", &format!(":2:{f}")])?;
                    let theirs = self.git(&dir, &["show", &format!(":3:{f}")])?;
                    if !ours.ok() || !theirs.ok() {
                        return Ok(Merge::Conflicts(vec![format!("{f} (removed on one side)")]));
                    }
                    self.shell.write(&dir.join(f), &union_log(&ours.text, &theirs.text))?;
                    self.git_ok(&dir, &["add", "--", f])?;
                }
            }
        }
        // Even a merge without conflicts: a textual merge of a generated
        // section is not what the generator makes of the merged coverage map.
        let r = self.xtask(slot, &["coverage".into(), "readme".into()], "readme", false)?;
        if !r.ok() {
            return Err(format!("coverage readme after merging {main}: {}", r.tail(3)));
        }
        self.add(&dir, &["README.md".into(), "docs/progress".into()])?;
        let message = merge_message(main, &super::cycle::impl_branch(entry), &plan);
        self.git_ok(&dir, &["commit", "-q", "-m", &message])?;
        Ok(Merge::Merged(self.git_ok(&dir, &["rev-parse", "HEAD"])?))
    }

    /// `gh api <path>` in the runner's checkout (`{owner}` and `{repo}` are
    /// its GitHub repository), as JSON. REST, not GraphQL: GraphQL is not
    /// available to every token.
    fn gh_api(&mut self, path: &str) -> Result<serde_json::Value> {
        let mut c = Cmd::new(&self.cfg.gh, &["api", path]);
        c.cwd = Some(self.cfg.repo.clone());
        let o = self.shell.run(&c)?;
        if !o.ok() {
            return Err(format!("gh api {path}: {}", o.text.trim()));
        }
        serde_json::from_str(&o.text).map_err(|e| format!("gh api {path}: {e}"))
    }
}

/// How the loop resolves a conflict in one file when it merges main.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    /// README.md, `docs/progress/`: main's version, then `coverage readme`.
    Generated,
    /// The entry's coverage file: main's version (the publish step sets the
    /// entry's status again with `coverage set-status`).
    Coverage,
    /// The append-only hand-off log: main's rows, then the branch's.
    Log,
}

/// The rule for a conflict in `path`; `None`: the owner's.
pub fn conflict_rule(path: &str, coverage: &str) -> Option<Resolve> {
    if path == "README.md" || path.starts_with("docs/progress/") {
        Some(Resolve::Generated)
    } else if path == coverage {
        Some(Resolve::Coverage)
    } else if path == "specs/HANDOFF_LOG.md" {
        Some(Resolve::Log)
    } else {
        None
    }
}

/// Whether `symbol`'s object differs between two versions of a coverage
/// file (or either is not a coverage file).
pub fn entry_changed(before: &str, after: &str, symbol: &str) -> bool {
    let find = |text: &str| -> Option<serde_json::Value> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        Some(v.as_array()?.iter().find(|e| e.get("symbol").and_then(|s| s.as_str()) == Some(symbol))?.clone())
    };
    match (find(before), find(after)) {
        (Some(b), Some(a)) => a != b,
        _ => true,
    }
}

/// The hand-off log after a merge: main's, then the branch's rows main lacks.
pub fn union_log(ours: &str, theirs: &str) -> String {
    let mut merged = theirs.to_string();
    if !merged.ends_with('\n') {
        merged.push('\n');
    }
    for row in ours.lines().filter(|l| l.starts_with("| ") && !theirs.lines().any(|t| t == *l)) {
        merged.push_str(row);
        merged.push('\n');
    }
    merged
}

/// The message of the loop's merge commit.
pub fn merge_message(main: &str, branch: &str, resolved: &[(String, Resolve)]) -> String {
    let mut m = format!(
        "Merge {main} into {branch}\n\nMerged by the agent pipeline loop to keep the PR mergeable \
         (docs/AGENT_PIPELINE.md, ADR 0086).\n"
    );
    if resolved.is_empty() {
        m.push_str("No conflicts; `cargo xtask coverage readme` made the README progress section again.\n");
        return m;
    }
    m.push_str("\nConflicts, all in generated or append-only files:\n");
    for (f, rule) in resolved {
        let how = match rule {
            Resolve::Generated => "main's version, then `cargo xtask coverage readme`",
            Resolve::Coverage => "main's version; the loop sets this entry's status again",
            Resolve::Log => "main's rows, then this branch's (the log is append-only)",
        };
        let _ = writeln!(m, "- {f}: {how}");
    }
    m
}

/// `text` for a URL query value.
fn query_value(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A pull request object of GitHub's REST API. A list item has no
/// `mergeable` (it reads as unknown); the single-PR object has it, `null`
/// while GitHub computes it.
pub fn parse_pr(v: &serde_json::Value) -> Result<Pr> {
    let url = v.get("html_url").and_then(|u| u.as_str()).ok_or("a PR without html_url")?.to_string();
    let merged =
        v.get("merged").and_then(|m| m.as_bool()) == Some(true) || v.get("merged_at").is_some_and(|m| !m.is_null());
    let state = match (merged, v.get("state").and_then(|s| s.as_str())) {
        (true, _) => "MERGED",
        (false, Some("open")) => "OPEN",
        (false, Some("closed")) => "CLOSED",
        (false, other) => return Err(format!("{url}: unknown PR state {other:?}")),
    };
    let mergeable = if v.get("mergeable_state").and_then(|s| s.as_str()) == Some("dirty") {
        Mergeable::Conflicting
    } else if v.get("mergeable").and_then(|m| m.as_bool()) == Some(true) {
        Mergeable::Yes
    } else {
        Mergeable::Unknown
    };
    Ok(Pr { url, state: state.into(), mergeable })
}

fn parse_author(a: &str) -> Option<(String, String)> {
    let (name, rest) = a.split_once('<')?;
    Some((name.trim().to_string(), rest.trim_end_matches('>').trim().to_string()))
}

fn stem(file: &str) -> Option<String> {
    let name = file.rsplit('/').next()?;
    name.strip_suffix(".c").map(String::from)
}

/// The result of a diff run for the cycle, and what the implementer may
/// see of it: visible tests in full, held-out tests only as a count.
pub fn diff_result(summary: &Summary, visible: &[String], held_out: &[String]) -> DiffResult {
    let passed = |name: &String| summary.tests.iter().any(|t| &t.test == name && t.verdict == Verdict::Pass);
    let vp = visible.iter().filter(|n| passed(n)).count();
    let hp = held_out.iter().filter(|n| passed(n)).count();
    let green = !visible.is_empty() && vp == visible.len() && hp == held_out.len();
    let mut shown = summary.clone();
    shown.tests.retain(|t| visible.contains(&t.test));
    shown.finish(None);
    let mut feedback = String::from("Visible tests of the entry on Poly against the real OS:\n\n");
    feedback.push_str(&shown.table());
    for t in shown.tests.iter().filter(|t| t.verdict != Verdict::Pass) {
        let _ = writeln!(feedback, "\n{} ({}):", t.test, t.verdict.name());
        if let Some(r) = &t.reason {
            let _ = writeln!(feedback, "  {r}");
        }
        for m in &t.comparison.mismatches {
            let _ = writeln!(feedback, "  {m}");
        }
        if let Some(d) = &t.comparison.first_divergence {
            let _ = writeln!(
                feedback,
                "  first syscall divergence at #{}: reference {} / target {}",
                d.index,
                d.reference.as_deref().unwrap_or("(end)"),
                d.target.as_deref().unwrap_or("(end)")
            );
            for c in &d.context {
                let _ = writeln!(feedback, "    {c}");
            }
        }
    }
    let missing: Vec<&String> = visible.iter().filter(|n| !summary.tests.iter().any(|t| &t.test == *n)).collect();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
        let _ = writeln!(feedback, "\nNot run (build or tool problem): {}", names.join(", "));
    }
    let _ = writeln!(
        feedback,
        "\nHeld-out tests: {hp} of {} pass. Which ones fail, and how, stays with the diff runner \
         (CLEAN_ROOM.md): make the behaviour match the spec sheet, not just the visible tests.",
        held_out.len()
    );
    DiffResult { green, summary: format!("visible {vp}/{}, held-out {hp}/{}", visible.len(), held_out.len()), feedback }
}

/// `text` with every line that names a held-out test or contains a line
/// of one replaced (CI output goes back to the implementer).
pub fn redact_lines(text: &str, names: &[String], contents: &BTreeSet<String>) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let leaks = names.iter().any(|n| n.len() >= 3 && line.contains(n.as_str()))
            || contents.iter().any(|c| line.contains(c.as_str()));
        out.push_str(if leaks { "[line withheld: it refers to a held-out test]" } else { line });
        out.push('\n');
    }
    out
}

impl<S: Shell> Ops for RealOps<S> {
    fn now(&mut self) -> u64 {
        self.shell.now()
    }

    fn sleep(&mut self, secs: u64) {
        self.shell.sleep(secs)
    }

    fn set_entry(&mut self, entry: Option<&str>) {
        self.entry = entry.map(String::from);
    }

    fn refresh_main(&mut self) -> Result<String> {
        let repo = self.cfg.repo.clone();
        let (remote, base) = (self.cfg.remote.clone(), self.cfg.base_branch.clone());
        self.git_ok(&repo, &["fetch", "--prune", "--no-tags", &remote])?;
        self.git_ok(&repo, &["merge", "--ff-only", "-q", &format!("{remote}/{base}")])?;
        self.git_ok(&repo, &["rev-parse", "HEAD"])
    }

    fn changed_paths(&mut self, from: &str, to: &str) -> Result<Vec<String>> {
        let repo = self.cfg.repo.clone();
        let out = self.git_ok(&repo, &["diff", "--name-only", "--no-renames", from, to])?;
        Ok(out.lines().filter(|l| !l.is_empty()).map(String::from).collect())
    }

    fn build_self(&mut self) -> Result<Out> {
        // One package at a time, as `cargo xtask` (xtask) and its `cargo run
        // --package poly-pipeline` build them: built together, shared
        // dependencies could get other features, and the start after the
        // restart would build everything again.
        let mut last = Out::default();
        for package in ["xtask", "poly-pipeline"] {
            let mut c = Cmd {
                program: self.cfg.cargo.clone(),
                args: ["build", "--quiet", "--package", package].iter().map(|s| s.to_string()).collect(),
                cwd: Some(self.cfg.repo.clone()),
                ..Cmd::default()
            };
            // rustup names the toolchain it runs us with in the environment;
            // without that, the checkout's rust-toolchain.toml picks it, as it
            // will when systemd starts the loop again.
            c.env_remove.extend(["RUSTUP_TOOLCHAIN".to_string(), store::HELDOUT_ENV.to_string()]);
            c.log = Some(self.log_file(&format!("build-{package}")));
            last = self.shell.run(&c)?;
            if !last.ok() {
                break;
            }
        }
        Ok(last)
    }

    fn queue(&mut self) -> Result<Vec<String>> {
        let traces: Vec<crate::trace::Trace> =
            crate::queue::default_traces(&self.queue_repo).iter().filter_map(|f| util::read_json(f).ok()).collect();
        let items = crate::queue::next(&self.queue_repo, &self.cfg.os, &self.cfg.layer, &traces)?;
        Ok(items.into_iter().map(|i| i.id).collect())
    }

    fn hub_head(&mut self, branch: &str) -> Result<Option<String>> {
        let (repo, hub) = (self.cfg.repo.clone(), self.cfg.hub.display().to_string());
        let r = format!("refs/heads/{branch}");
        let out = self.git_ok(&repo, &["ls-remote", &hub, &r])?;
        Ok(out.lines().find(|l| l.ends_with(&r)).and_then(|l| l.split_whitespace().next()).map(String::from))
    }

    fn prepare(&mut self, slot: usize, start: &str) -> Result<String> {
        let dir = self.slot_dir(slot);
        let (repo, remote, hub) = (self.cfg.repo.clone(), self.cfg.remote.clone(), self.cfg.hub.display().to_string());
        if !self.shell.exists(&dir.join(".git")) {
            let base = format!("{remote}/{}", self.cfg.base_branch);
            self.git_ok(&repo, &["worktree", "add", "--detach", &dir.display().to_string(), &base])?;
        }
        self.git_ok(&dir, &["fetch", "--prune", "--no-tags", "--no-write-fetch-head", &remote])?;
        self.git_ok(
            &dir,
            &[
                "fetch",
                "--prune",
                "--no-tags",
                "--no-write-fetch-head",
                &hub,
                "+refs/heads/pipeline/*:refs/remotes/hub/pipeline/*",
            ],
        )?;
        self.git_ok(&dir, &["checkout", "-q", "-f", "--detach", start])?;
        self.git_ok(&dir, &["clean", "-fdq"])?;
        self.git_ok(&dir, &["rev-parse", "HEAD"])
    }

    fn subject(&mut self, slot: usize, rev: &str) -> Result<String> {
        let dir = self.slot_dir(slot);
        self.git_ok(&dir, &["log", "-1", "--format=%s", rev])
    }

    fn commit(&mut self, slot: usize, message: &str, paths: &[String]) -> Result<String> {
        let dir = self.slot_dir(slot);
        self.add(&dir, paths)?;
        self.git_ok(&dir, &["commit", "-q", "-m", message])?;
        self.git_ok(&dir, &["rev-parse", "HEAD"])
    }

    fn push(&mut self, slot: usize, remote: Remote, rev: &str, branch: &str) -> Result<Push> {
        let dir = self.slot_dir(slot);
        let target = match remote {
            Remote::Hub => self.cfg.hub.display().to_string(),
            Remote::Origin => self.cfg.remote.clone(),
        };
        // Never --force: a rewritten branch is refused, and that is a red flag.
        let o = self.git(&dir, &["push", "-q", &target, &format!("{rev}:refs/heads/{branch}")])?;
        if o.ok() {
            return Ok(Push::Done);
        }
        let t = o.text.to_lowercase();
        if t.contains("rejected") || t.contains("non-fast-forward") || t.contains("fetch first") {
            return Ok(Push::Rejected(o.text.trim().to_string()));
        }
        Err(format!("git push {target} {branch}: {}", o.text.trim()))
    }

    fn changes(&mut self, slot: usize, from: &str, to: &str) -> Result<Vec<Change>> {
        let dir = self.slot_dir(slot);
        let o = self.git(&dir, &["diff", "--no-renames", "--no-color", "--no-ext-diff", "-U0", from, to])?;
        if !o.ok() {
            return Err(format!("git diff {from} {to}: {}", o.text.trim()));
        }
        Ok(safety::parse_diff(&o.text))
    }

    fn merge_base(&mut self, slot: usize, a: &str, b: &str) -> Result<String> {
        let dir = self.slot_dir(slot);
        self.git_ok(&dir, &["merge-base", a, b])
    }

    fn file_at(&mut self, slot: usize, rev: &str, path: &str) -> Result<Option<String>> {
        let dir = self.slot_dir(slot);
        let o = self.git(&dir, &["show", &format!("{rev}:{path}")])?;
        Ok(o.ok().then_some(o.text))
    }

    fn claim(&mut self, slot: usize, entry: &EntryId) -> Result<Out> {
        let args = ["coverage", "claim", &entry.id(), "--agent", &self.cfg.spec.agent].map(String::from);
        let o = self.xtask(slot, &args, "claim", false)?;
        if !o.ok() {
            return Ok(o);
        }
        // The claim changes the counts the README progress section and
        // graphics are generated from; `cargo xtask ci` fails while they are
        // stale, and the spec agent may not regenerate them.
        self.xtask(slot, &["coverage".into(), "readme".into()], "readme", false)
    }

    fn ci(&mut self, slot: usize) -> Result<Out> {
        self.xtask(slot, &["ci".into()], "ci", false)
    }

    fn handoff_record(&mut self, slot: usize, entry: &EntryId) -> Result<Out> {
        let args = [
            "pipeline",
            "handoff",
            "record",
            &entry.id(),
            "--spec-agent",
            &self.cfg.spec.agent,
            "--implementer",
            &self.cfg.implementer.agent,
        ]
        .map(String::from);
        self.xtask(slot, &args, "handoff-record", true)
    }

    fn handoff_bundle(&mut self, slot: usize, entry: &EntryId) -> Result<String> {
        let dir = self.cfg.inbox.join(state::key(&entry.id())).join("bundle");
        self.shell.remove_dir(&dir)?;
        let d = dir.display().to_string();
        let args = ["pipeline", "handoff", "bundle", &entry.id(), "--out", &d].map(String::from);
        let o = self.xtask(slot, &args, "handoff-bundle", true)?;
        if !o.ok() {
            return Err(format!("handoff bundle: {}", o.tail(3)));
        }
        Ok(d)
    }

    fn handoff_verify(&mut self, slot: usize, entry: &EntryId, received: &str) -> Result<Out> {
        let args = ["pipeline", "handoff", "verify", &entry.id(), "--received", received].map(String::from);
        self.xtask(slot, &args, "handoff-verify", true)
    }

    fn spec_files(&mut self, slot: usize, entry: &EntryId) -> Result<SpecFiles> {
        let dir = self.slot_dir(slot);
        let spec = self.shell.exists(&dir.join(entry.spec_path()));
        let visible = self.shell.list(&dir.join(entry.tests_dir()))?.len() as u32;
        let manifest = entry.heldout_dir(&self.cfg.heldout).join(store::MANIFEST);
        let held_out = match self.shell.read(&manifest) {
            Ok(text) => serde_json::from_str::<HeldOutManifest>(&text)
                .map_err(|e| format!("{}: {e}", manifest.display()))?
                .held_out
                .len() as u32,
            Err(_) => 0,
        };
        Ok(SpecFiles { spec, visible, held_out })
    }

    fn diff(&mut self, slot: usize, entry: &EntryId) -> Result<DiffResult> {
        let visible_dir = self.slot_dir(slot).join(entry.tests_dir());
        let held_dir = entry.heldout_dir(&self.cfg.heldout);
        let visible: Vec<String> = self.shell.list(&visible_dir)?.iter().filter_map(|f| stem(f)).collect();
        let held: Vec<String> = self.heldout_names(entry)?.iter().filter_map(|f| stem(f)).collect();
        if visible.is_empty() {
            return Err(format!("no visible .c tests in {}", visible_dir.display()));
        }
        let run_dir = self.log_file("diff").with_extension("run");
        let names: Vec<&str> = visible.iter().chain(&held).map(String::as_str).collect();
        let args = [
            "pipeline",
            "diff",
            "--target",
            "poly",
            "--build-kernel",
            "--tests-dir",
            &visible_dir.display().to_string(),
            "--tests-dir",
            &held_dir.display().to_string(),
            "--tests",
            &names.join(","),
            "--run-dir",
            &run_dir.display().to_string(),
        ]
        .map(String::from);
        let o = self.xtask(slot, &args, "diff", true)?;
        let summary: Summary = match self.shell.read(&run_dir.join("summary.json")) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("summary.json: {e}"))?,
            Err(_) => return Err(format!("the diff runner wrote no summary ({})", o.describe())),
        };
        Ok(diff_result(&summary, &visible, &held))
    }

    fn set_green(&mut self, slot: usize, entry: &EntryId, visible: u32, held_out: u32) -> Result<Out> {
        let args = [
            "coverage",
            "set-status",
            &entry.id(),
            "green",
            "--tests-visible",
            &visible.to_string(),
            "--tests-held-out",
            &held_out.to_string(),
        ]
        .map(String::from);
        let o = self.xtask(slot, &args, "set-status", false)?;
        if !o.ok() {
            return Ok(o);
        }
        self.xtask(slot, &["coverage".into(), "readme".into()], "readme", false)
    }

    fn coverage_check(&mut self, slot: usize) -> Result<Out> {
        let o = self.xtask(slot, &["coverage".into(), "check".into()], "coverage-check", false)?;
        if !o.ok() {
            return Ok(o);
        }
        self.xtask(slot, &["coverage".into(), "readme".into(), "--check".into()], "readme-check", false)
    }

    fn merge_main(&mut self, slot: usize, entry: &EntryId) -> Result<Merge> {
        let dir = self.slot_dir(slot);
        let main = format!("{}/{}", self.cfg.remote, self.cfg.base_branch);
        let up = self.git(&dir, &["merge-base", "--is-ancestor", &main, "HEAD"])?;
        match up.code {
            0 => return Ok(Merge::UpToDate),
            1 => {}
            _ => return Err(format!("git merge-base --is-ancestor {main} HEAD: {}", up.text.trim())),
        }
        // Not committed yet: the README progress section is made again first.
        let o = self.git(&dir, &["merge", "--no-ff", "--no-commit", &main])?;
        // Whatever goes wrong from here, the checkout is left as it was.
        let r = self.finish_merge(slot, entry, &main, &o);
        if !matches!(r, Ok(Merge::Merged(_))) {
            let _ = self.git(&dir, &["merge", "--abort"]);
        }
        r
    }

    fn role_sync(&mut self, role: Role, _entry: &EntryId, branch: &str, base: &str) -> Result<Out> {
        self.role_cmd(role, &["sync".into(), branch.into(), base.into()], &format!("{}-sync", role.name()))
    }

    fn role_run(&mut self, role: Role, entry: &EntryId, feedback: Option<&str>) -> Result<Out> {
        let rc = self.role_cfg(role).clone();
        let mut args = vec!["run".to_string(), entry.id(), rc.agent];
        if let Some(m) = rc.model {
            args.extend(["--model".into(), m]);
        }
        if let Some(f) = feedback {
            args.extend(["--feedback".into(), f.into()]);
        }
        self.role_cmd(role, &args, role.name())
    }

    fn role_fetch(&mut self, role: Role, branch: &str) -> Result<Option<String>> {
        let rc = self.role_cfg(role).clone();
        let repo = self.cfg.repo.clone();
        let src = rc.repo.display().to_string();
        let dst = format!("refs/pipeline/{}/{}", role.name(), branch.rsplit('/').next().unwrap_or(branch));
        // The role checkout belongs to another user: name it safe for this fetch only.
        let o = self.git(
            &repo,
            &[
                "-c",
                &format!("safe.directory={src}"),
                "-c",
                &format!("safe.directory={src}/.git"),
                "fetch",
                "-q",
                "--no-tags",
                "--no-write-fetch-head",
                &src,
                &format!("+refs/heads/{branch}:{dst}"),
            ],
        )?;
        if !o.ok() {
            if o.text.contains("couldn't find remote ref") {
                return Ok(None);
            }
            return Err(format!("fetching {branch} from {src}: {}", o.text.trim()));
        }
        self.git_ok(&repo, &["rev-parse", &dst]).map(Some)
    }

    fn redact(&mut self, entry: &EntryId, text: &str) -> String {
        let dir = entry.heldout_dir(&self.cfg.heldout);
        let files = self.heldout_names(entry).unwrap_or_default();
        let names: Vec<String> = files.iter().filter_map(|f| stem(f)).collect();
        let mut contents = BTreeSet::new();
        for f in &files {
            if let Ok(t) = self.shell.read(&dir.join(f)) {
                contents.extend(t.lines().map(str::trim).filter(|l| l.len() >= 12).map(String::from));
            }
        }
        redact_lines(text, &names, &contents)
    }

    fn write_feedback(&mut self, role: Role, entry: &EntryId, cycle: u32, text: &str) -> Result<String> {
        let inbox = match role {
            Role::Spec => &self.cfg.spec_inbox,
            Role::Implementer => &self.cfg.inbox,
        };
        let path = inbox.join(state::key(&entry.id())).join(format!("feedback-{cycle}.txt"));
        self.shell.write(&path, text)?;
        Ok(path.display().to_string())
    }

    fn resolve_clock(&mut self, time: &str, tz: Option<&str>) -> Option<u64> {
        let now = self.shell.now();
        for expr in [time.to_string(), format!("tomorrow {time}")] {
            let mut c = Cmd::new("date", &["-d", &expr, "+%s"]);
            if let Some(tz) = tz {
                c.env.push(("TZ".into(), tz.into()));
            }
            let t = self.shell.run(&c).ok().filter(Out::ok).and_then(|o| o.text.trim().parse::<u64>().ok())?;
            if t > now {
                return Some(t);
            }
        }
        None
    }

    fn pr_find(&mut self, branch: &str) -> Result<Option<Pr>> {
        // Newest first (GitHub's default order), closed and merged ones too.
        let list = self.gh_api(&format!(
            "repos/{{owner}}/{{repo}}/pulls?head={{owner}}:{}&state=all&per_page=100",
            query_value(branch)
        ))?;
        let Some(newest) = list.as_array().and_then(|a| a.first()).cloned() else { return Ok(None) };
        let pr = parse_pr(&newest)?;
        if pr.state != "OPEN" {
            return Ok(Some(pr));
        }
        // Only the single-PR object has `mergeable`; asking for it also
        // makes GitHub compute it when it has not yet.
        let number = newest.get("number").and_then(|n| n.as_u64()).ok_or(format!("{}: no PR number", pr.url))?;
        parse_pr(&self.gh_api(&format!("repos/{{owner}}/{{repo}}/pulls/{number}"))?).map(Some)
    }

    fn pr_create(&mut self, branch: &str, title: &str, body: &str) -> Result<String> {
        let body_file = self.log_file("pr-body").with_extension("md");
        self.shell.write(&body_file, body)?;
        let base = self.cfg.base_branch.clone();
        let mut c = Cmd::new(
            &self.cfg.gh,
            &[
                "pr",
                "create",
                "--base",
                &base,
                "--head",
                branch,
                "--title",
                title,
                "--body-file",
                &body_file.display().to_string(),
            ],
        );
        c.cwd = Some(self.cfg.repo.clone());
        let o = self.shell.run(&c)?;
        if !o.ok() {
            return Err(format!("gh pr create: {}", o.text.trim()));
        }
        o.text
            .lines()
            .rev()
            .find(|l| l.contains("://"))
            .map(|l| l.trim().to_string())
            .ok_or("gh pr create printed no URL".into())
    }

    fn pr_auto_merge(&mut self, url: &str) -> Result {
        // A merge commit, never squash: the hand-off log cites the spec commit.
        let mut c = Cmd::new(&self.cfg.gh, &["pr", "merge", url, "--auto", "--merge"]);
        c.cwd = Some(self.cfg.repo.clone());
        let o = self.shell.run(&c)?;
        if o.ok() { Ok(()) } else { Err(o.text.trim().to_string()) }
    }

    fn notify(&mut self, event: &str, entry: &str, message: &str) -> Result {
        let Some(command) = self.cfg.notify_command.clone() else { return Ok(()) };
        let mut c = Cmd::new("sh", &["-c", &command]);
        c.env = vec![
            ("POLY_EVENT".into(), event.into()),
            ("POLY_ENTRY".into(), entry.into()),
            ("POLY_MESSAGE".into(), message.into()),
        ];
        c.env_remove.push(store::HELDOUT_ENV.into());
        let o = self.shell.run(&c)?;
        if o.ok() { Ok(()) } else { Err(format!("notify command: {}", o.text.trim())) }
    }

    fn append_human_queue(&mut self, text: &str) -> Result {
        let path = self.cfg.human_queue.clone();
        if !self.shell.exists(&path) {
            self.shell.write(
                &path,
                "# Human queue\n\nEntries the agent pipeline stopped on (docs/AGENT_PIPELINE.md). Newest last.\n",
            )?;
        }
        self.shell.append(&path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{Comparison, Divergence, TestResult};

    fn result(name: &str, verdict: Verdict, mismatch: Option<&str>) -> TestResult {
        let mut r = TestResult::not_run(name, verdict, String::new());
        r.reason = None;
        if let Some(m) = mismatch {
            r.comparison = Comparison {
                mismatches: vec![m.into()],
                first_divergence: Some(Divergence {
                    index: 3,
                    reference: Some("brk(0x1000) = 0x1000".into()),
                    target: Some("brk(0x1000) = -1 ENOMEM".into()),
                    context: vec!["brk(NULL) = <addr>".into()],
                }),
                syscalls_compared: true,
            };
        }
        r
    }

    #[test]
    fn diff_feedback_withholds_held_out_details() {
        let mut s = Summary::new("t".into(), "poly", "linux".into(), "ptrace", crate::diff::SyscallMode::Exact);
        s.tests = vec![
            result("brk_grow", Verdict::Pass, None),
            result("brk_shrink", Verdict::Fail, Some("syscalls: differ at #3")),
            result("brk_secret_case", Verdict::Fail, Some("stdout: SECRET held-out output")),
            result("brk_other_secret", Verdict::Pass, None),
        ];
        s.finish(None);
        let v = vec!["brk_grow".to_string(), "brk_shrink".to_string()];
        let h = vec!["brk_secret_case".to_string(), "brk_other_secret".to_string()];
        let d = diff_result(&s, &v, &h);
        assert!(!d.green);
        assert_eq!(d.summary, "visible 1/2, held-out 1/2");
        assert!(d.feedback.contains("brk_shrink") && d.feedback.contains("brk(0x1000) = -1 ENOMEM"));
        assert!(!d.feedback.contains("secret") && !d.feedback.contains("SECRET"), "{}", d.feedback);
        assert!(d.feedback.contains("Held-out tests: 1 of 2 pass"));

        s.tests[1].verdict = Verdict::Pass;
        s.tests[2].verdict = Verdict::Pass;
        assert!(diff_result(&s, &v, &h).green);
        // A visible test that did not run is not green.
        assert!(!diff_result(&s, &[v[0].clone(), "brk_missing".into()], &h).green);
    }

    #[test]
    fn redaction() {
        let names = vec!["brk_secret_case".to_string()];
        let contents: BTreeSet<String> = ["check(brk(p) == p + 4096);".to_string()].into();
        let text = "ok 1\nFAILED brk_secret_case\nassert: check(brk(p) == p + 4096); failed\nok 2\n";
        assert_eq!(
            redact_lines(text, &names, &contents),
            "ok 1\n[line withheld: it refers to a held-out test]\n[line withheld: it refers to a held-out test]\nok 2\n"
        );
    }

    #[test]
    fn commands_are_quoted_for_display() {
        let mut c = Cmd::new("git", &["commit", "-m", "a message"]);
        c.cwd = Some("/srv/poly/runner/work/w0".into());
        assert_eq!(c.line(), "(cd /srv/poly/runner/work/w0) git commit -m 'a message'");
        assert_eq!(
            parse_author("Poly pipeline <p@example.com>"),
            Some(("Poly pipeline".into(), "p@example.com".into()))
        );
    }

    #[test]
    fn the_real_shell_captures_and_logs() {
        let dir = std::env::temp_dir().join(format!("poly-loop-shell-{}", std::process::id()));
        let mut sh = RealShell;
        let o = sh.run(&Cmd::new("sh", &["-c", "echo out; echo err >&2; exit 3"])).unwrap();
        assert_eq!((o.code, o.text.as_str()), (3, "out\nerr\n"));
        let mut c = Cmd::new("sh", &["-c", "echo $POLY_X; echo ${POLY_HELDOUT_DIR:-unset}"]);
        c.env.push(("POLY_X".into(), "x".into()));
        c.env_remove.push(store::HELDOUT_ENV.into());
        c.log = Some(dir.join("a.log"));
        let o = sh.run(&c).unwrap();
        assert!(o.ok() && o.text.ends_with("x\nunset\n"), "{}", o.text);
        assert!(o.text.starts_with("$ POLY_X=x sh -c"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Run git for the test's own setup.
    fn vcs(dir: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    const COVERAGE: &str = "coverage/linux/syscall/x86_64.json";
    const LOG_HEADER: &str = "# Hand-off log\n\n| Date (UTC) | Entry id | Spec revision (git) | Spec agent | Implementer agent | Files handed over |\n|---|---|---|---|---|---|\n";

    /// The git side of `RealOps` on real repositories: an "origin" (GitHub),
    /// the runner's clone with worktrees, the hub, a role checkout.
    #[test]
    fn real_git_operations() {
        if util::which("git").is_none() {
            eprintln!("skipped: no git");
            return;
        }
        let base = std::env::temp_dir().join(format!("poly-loop-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (origin, seed, runner, hub, role) =
            (base.join("origin.git"), base.join("seed"), base.join("runner"), base.join("hub.git"), base.join("spec"));
        for d in [&origin, &seed, &hub] {
            util::create_dir(d).unwrap();
        }
        vcs(&origin, &["init", "-q", "--bare", "-b", "main"]);
        vcs(&hub, &["init", "-q", "--bare"]);
        vcs(&seed, &["init", "-q", "-b", "main"]);
        util::write(
            &seed.join(COVERAGE),
            "[\n{\"symbol\":\"brk\",\"src\":[\"t\"]},\n{\"symbol\":\"mmap\",\"src\":[\"t\"]}\n]\n",
        )
        .unwrap();
        util::write(&seed.join("README.md"), "progress 0\n").unwrap();
        util::write(&seed.join("specs/HANDOFF_LOG.md"), LOG_HEADER).unwrap();
        util::write(&seed.join("src.rs"), "fn a() {}\n").unwrap();
        vcs(&seed, &["add", "."]);
        vcs(&seed, &["commit", "-q", "-m", "seed"]);
        let o = origin.display().to_string();
        vcs(&seed, &["push", "-q", &o, "main"]);
        vcs(&base, &["clone", "-q", &o, "runner"]);
        let main0 = vcs(&runner, &["rev-parse", "HEAD"]);

        let cfg = Config {
            repo: runner.clone(),
            worktrees: base.join("work"),
            hub: hub.clone(),
            heldout: base.join("heldout"),
            inbox: base.join("inbox"),
            author: Some("Poly pipeline <p@example.com>".into()),
            cargo: "true".into(),
            spec: crate::scheduler::config::RoleConfig { repo: role.clone(), ..Config::default().spec },
            ..Config::default()
        };
        let mut ops = RealOps::new(cfg, RealShell, &base.join("logs"));
        ops.set_entry(Some("linux/syscall/x86_64/brk"));

        // A worktree on main, a commit, pushed to the hub.
        assert_eq!(ops.prepare(0, "origin/main").unwrap(), main0);
        util::write(&base.join("work/w0/specs/linux/x86_64/brk.yaml"), "id: brk\n").unwrap();
        let c1 = ops.commit(0, "spec", &["specs/linux/x86_64/brk.yaml".into()]).unwrap();
        assert_eq!(ops.subject(0, &c1).unwrap(), "spec");
        assert_eq!(ops.hub_head("pipeline/spec/brk").unwrap(), None);
        assert_eq!(ops.push(0, Remote::Hub, &c1, "pipeline/spec/brk").unwrap(), Push::Done);
        assert_eq!(ops.hub_head("pipeline/spec/brk").unwrap(), Some(c1.clone()));
        let ch = ops.changes(0, &main0, &c1).unwrap();
        assert_eq!(
            (ch.len(), ch[0].path.as_str(), ch[0].added.as_slice()),
            (1, "specs/linux/x86_64/brk.yaml", &["id: brk".to_string()][..])
        );
        assert!(ops.file_at(0, &c1, COVERAGE).unwrap().unwrap().contains("\"brk\""));
        assert_eq!(ops.file_at(0, &c1, "nope").unwrap(), None);
        assert_eq!(ops.merge_base(0, "origin/main", &c1).unwrap(), main0);

        // A second worktree sees the hub branch; a diverging push is refused.
        assert_eq!(ops.prepare(1, "hub/pipeline/spec/brk").unwrap(), c1);
        ops.prepare(0, "origin/main").unwrap();
        util::write(&base.join("work/w0/other.txt"), "x\n").unwrap();
        let c2 = ops.commit(0, "other", &["other.txt".into()]).unwrap();
        assert!(matches!(ops.push(0, Remote::Hub, &c2, "pipeline/spec/brk").unwrap(), Push::Rejected(_)));

        // The role's checkout: cloned from the hub, the agent commits, the loop fetches.
        vcs(&base, &["clone", "-q", "-o", "hub", &hub.display().to_string(), "spec"]);
        vcs(&role, &["checkout", "-q", "-b", "pipeline/spec/brk", "hub/pipeline/spec/brk"]);
        util::write(&role.join("specs/linux/x86_64/brk.tests/t1.c"), "int x;\n").unwrap();
        vcs(&role, &["add", "."]);
        vcs(&role, &["commit", "-q", "-m", "tests"]);
        let agent = vcs(&role, &["rev-parse", "HEAD"]);
        assert_eq!(ops.role_fetch(Role::Spec, "pipeline/spec/brk").unwrap(), Some(agent.clone()));
        assert_eq!(ops.role_fetch(Role::Spec, "pipeline/spec/none").unwrap(), None);
        assert_eq!(ops.push(1, Remote::Hub, &agent, "pipeline/spec/brk").unwrap(), Push::Done);

        // Main moves on: README, the coverage line next to ours, the hand-off log.
        let e = EntryId::parse("linux/syscall/x86_64/brk").unwrap();
        ops.prepare(0, "hub/pipeline/spec/brk").unwrap();
        let w0 = base.join("work/w0");
        util::write(&w0.join("README.md"), "progress 1\n").unwrap();
        util::write(&w0.join(COVERAGE), "[\n{\"symbol\":\"brk\",\"status\":\"green\",\"tests_visible\":1,\"src\":[\"t\"]},\n{\"symbol\":\"mmap\",\"src\":[\"t\"]}\n]\n").unwrap();
        util::write(
            &w0.join("specs/HANDOFF_LOG.md"),
            format!("{LOG_HEADER}| d | linux/syscall/x86_64/brk | r | s | i | f |\n"),
        )
        .unwrap();
        let ours =
            ops.commit(0, "green", &["README.md".into(), COVERAGE.into(), "specs/HANDOFF_LOG.md".into()]).unwrap();
        util::write(&seed.join("README.md"), "progress 2\n").unwrap();
        util::write(&seed.join(COVERAGE), "[\n{\"symbol\":\"brk\",\"src\":[\"t\"]},\n{\"symbol\":\"mmap\",\"status\":\"green\",\"tests_visible\":1,\"src\":[\"t\"]}\n]\n").unwrap();
        util::write(
            &seed.join("specs/HANDOFF_LOG.md"),
            format!("{LOG_HEADER}| d | linux/syscall/x86_64/mmap | r | s | i | f |\n"),
        )
        .unwrap();
        vcs(&seed, &["commit", "-q", "-am", "mmap green"]);
        vcs(&seed, &["push", "-q", &o, "main"]);
        ops.prepare(0, &ours).unwrap();
        let merged = match ops.merge_main(0, &e).unwrap() {
            Merge::Merged(h) => h,
            m => panic!("{m:?}"),
        };
        let log = ops.file_at(0, &merged, "specs/HANDOFF_LOG.md").unwrap().unwrap();
        assert!(log.contains("x86_64/mmap |") && log.contains("x86_64/brk |"), "{log}");
        assert_eq!(
            ops.file_at(0, &merged, "README.md").unwrap().unwrap(),
            "progress 2\n",
            "main's, made again by `coverage readme`"
        );
        assert!(ops.file_at(0, &merged, COVERAGE).unwrap().unwrap().contains("\"mmap\",\"status\":\"green\""));
        // A merge commit of both heads, saying what the loop resolved.
        assert_eq!(vcs(&w0, &["rev-list", "--parents", "-n", "1", &merged]).split(' ').count(), 3);
        let message = vcs(&w0, &["log", "-1", "--format=%B", &merged]);
        assert!(message.starts_with("Merge origin/main into pipeline/impl/brk\n"), "{message}");
        for f in
            ["README.md: main's version", &format!("{COVERAGE}: main's version"), "specs/HANDOFF_LOG.md: main's rows"]
        {
            assert!(message.contains(f), "{f}: {message}");
        }
        assert_eq!(ops.merge_main(0, &e).unwrap(), Merge::UpToDate);

        // Main changed this entry's own coverage status: the owner's.
        util::write(&w0.join(COVERAGE), "[\n{\"symbol\":\"brk\",\"status\":\"green\",\"tests_visible\":2,\"src\":[\"t\"]},\n{\"symbol\":\"mmap\",\"status\":\"green\",\"tests_visible\":1,\"src\":[\"t\"]}\n]\n").unwrap();
        let before = ops.commit(0, "ours", &[COVERAGE.into()]).unwrap();
        util::write(&seed.join(COVERAGE), "[\n{\"symbol\":\"brk\",\"status\":\"partial\",\"src\":[\"t\"]},\n{\"symbol\":\"mmap\",\"status\":\"green\",\"tests_visible\":1,\"src\":[\"t\"]}\n]\n").unwrap();
        vcs(&seed, &["commit", "-q", "-am", "brk partial"]);
        vcs(&seed, &["push", "-q", &o, "main"]);
        ops.prepare(0, "HEAD").unwrap();
        assert_eq!(
            ops.merge_main(0, &e).unwrap(),
            Merge::Conflicts(vec![format!("{COVERAGE} (main changed linux/syscall/x86_64/brk itself)")])
        );
        assert_eq!(vcs(&w0, &["status", "--porcelain"]), "", "the merge was aborted");
        assert_eq!(vcs(&w0, &["rev-parse", "HEAD"]), before);

        // A merge without conflicts is committed too.
        ops.prepare(0, "origin/main").unwrap();
        util::write(&w0.join("ours.txt"), "x\n").unwrap();
        ops.commit(0, "ours", &["ours.txt".into()]).unwrap();
        util::write(&seed.join("theirs.txt"), "y\n").unwrap();
        vcs(&seed, &["add", "theirs.txt"]);
        vcs(&seed, &["commit", "-q", "-m", "theirs"]);
        vcs(&seed, &["push", "-q", &o, "main"]);
        ops.prepare(0, "HEAD").unwrap();
        let clean = match ops.merge_main(0, &e).unwrap() {
            Merge::Merged(h) => h,
            m => panic!("{m:?}"),
        };
        assert!(vcs(&w0, &["log", "-1", "--format=%B", &clean]).contains("No conflicts"));
        assert!(ops.file_at(0, &clean, "theirs.txt").unwrap().is_some());

        // A conflict in code is the owner's.
        util::write(&w0.join("src.rs"), "fn ours() {}\n").unwrap();
        ops.commit(0, "code", &["src.rs".into()]).unwrap();
        util::write(&seed.join("src.rs"), "fn theirs() {}\n").unwrap();
        vcs(&seed, &["commit", "-q", "-am", "code"]);
        vcs(&seed, &["push", "-q", &o, "main"]);
        ops.prepare(0, "HEAD").unwrap();
        assert_eq!(ops.merge_main(0, &e).unwrap(), Merge::Conflicts(vec!["src.rs".into()]));
        assert_eq!(vcs(&w0, &["status", "--porcelain"]), "", "the merge was aborted");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rest_pull_requests() {
        let pr = |json: &str| parse_pr(&serde_json::from_str(json).unwrap()).unwrap();
        let url = "\"html_url\":\"https://github.com/o/r/pull/28\"";
        assert_eq!(
            pr(&format!(
                "{{{url},\"state\":\"open\",\"merged\":false,\"mergeable\":false,\"mergeable_state\":\"dirty\"}}"
            )),
            Pr {
                url: "https://github.com/o/r/pull/28".into(),
                state: "OPEN".into(),
                mergeable: Mergeable::Conflicting
            }
        );
        let p = pr(&format!("{{{url},\"state\":\"open\",\"mergeable\":true,\"mergeable_state\":\"blocked\"}}"));
        assert_eq!(p.mergeable, Mergeable::Yes, "waiting for CI is mergeable");
        let p = pr(&format!("{{{url},\"state\":\"open\",\"mergeable\":null,\"mergeable_state\":\"unknown\"}}"));
        assert_eq!(p.mergeable, Mergeable::Unknown, "still being computed");
        // A list item: no `mergeable`, `merged_at` for merged ones.
        let p = pr(&format!("{{{url},\"state\":\"closed\",\"merged_at\":\"2026-10-21T14:00:00Z\"}}"));
        assert_eq!((p.state.as_str(), p.mergeable), ("MERGED", Mergeable::Unknown));
        assert_eq!(pr(&format!("{{{url},\"state\":\"closed\",\"merged_at\":null}}")).state, "CLOSED");
        assert!(parse_pr(&serde_json::json!({"state": "open"})).is_err());
        assert_eq!(query_value("pipeline/impl/a+b c"), "pipeline/impl/a%2Bb%20c");
    }

    #[test]
    fn conflict_rules() {
        let cov = "coverage/linux/syscall/x86_64.json";
        assert_eq!(conflict_rule("README.md", cov), Some(Resolve::Generated));
        assert_eq!(conflict_rule("docs/progress/linux.svg", cov), Some(Resolve::Generated));
        assert_eq!(conflict_rule(cov, cov), Some(Resolve::Coverage));
        assert_eq!(conflict_rule("specs/HANDOFF_LOG.md", cov), Some(Resolve::Log));
        for other in
            ["coverage/linux/syscall/arm64.json", "docs/ROADMAP.md", "personalities/linux/src/a.rs", "x/README.md"]
        {
            assert_eq!(conflict_rule(other, cov), None, "{other}");
        }
        let file = |brk: &str| format!("[\n{{\"symbol\":\"access\",\"src\":[\"t\"]}},\n{brk}\n]\n");
        let base = file("{\"symbol\":\"brk\",\"src\":[\"t\"]}");
        let neighbour = base.replace("\"access\",", "\"access\",\"status\":\"green\",");
        assert!(!entry_changed(&base, &neighbour, "brk"));
        assert!(entry_changed(&base, &file("{\"symbol\":\"brk\",\"status\":\"partial\",\"src\":[\"t\"]}"), "brk"));
        assert!(entry_changed(&base, "not json", "brk") && entry_changed(&base, &base, "mmap"));
        assert_eq!(union_log("h\n| a |\n| b |\n", "h\n| a |\n| c |"), "h\n| a |\n| c |\n| b |\n");
    }
}
