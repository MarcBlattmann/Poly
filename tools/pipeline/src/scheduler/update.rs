//! The loop keeps itself up to date (docs/AGENT_PIPELINE.md, "Updates").
//!
//! Every fast-forward of the runner's checkout is compared with the commit
//! the running binaries were built from. When it changes the loop's own
//! code ([`Effect::Restart`]: `tools/pipeline/`, `tools/xtask/`, the
//! workspace manifest and lock file, the toolchain pin), the loop builds
//! the new code first; once that build passes, it lets every step in
//! progress end (each is resumable from its state), starts nothing new,
//! and exits with [`RESTART_EXIT`]. systemd (`Restart=always`) starts
//! `cargo xtask pipeline loop` again, which runs the new code. A build
//! that fails never exits: the old loop keeps working, the owner is told
//! once, and the build is tried again when main moves.
//!
//! The deploy files ([`Effect::Deploy`]: the role wrapper and `setup.sh`,
//! which writes the sudoers rule and the systemd unit) need root, so they
//! are never applied by the loop. `setup.sh` records a fingerprint of what
//! it installed ([`deploy_hash`]) in `paths.deploy_stamp`; while the
//! runner's checkout has other deploy files, `loop status` says so, and the
//! loop tells the owner once (log, human queue, notify command).

use super::config::Config;
use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The loop's exit status when it stops to run its new code. The systemd
/// unit lists it in `SuccessExitStatus=` (tools/pipeline/deploy/setup.sh).
pub const RESTART_EXIT: u8 = 10;

/// The deploy files, relative to the repository.
pub const DEPLOY_DIR: &str = "tools/pipeline/deploy";

/// In [`DEPLOY_DIR`] but not installed by `setup.sh` (it never overwrites
/// the configuration; every key has a default), so not a deploy change.
pub const EXAMPLE_CONFIG: &str = "pipeline.example.toml";

/// What a change to a path means for the running loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The loop's own binaries (`pipeline`, `xtask`) change: rebuild and restart.
    Restart,
    /// Installed by `setup.sh` as root: the owner runs it again.
    Deploy,
    /// Nothing the running loop depends on.
    None,
}

/// What a change to `path` (relative to the repository) means.
///
/// The crates the binaries link (`poly-elf`, `poly-pe`, ...) and the files
/// they embed (the syscall table from the coverage map) are not here: the
/// loop process never runs that code. The tools that do (`pipeline diff`,
/// `pipeline handoff`, CI) run as `cargo xtask ...` in the runner's
/// worktrees and are built fresh from the branch every time.
pub fn effect(path: &str) -> Effect {
    if let Some(rest) = path.strip_prefix(DEPLOY_DIR).and_then(|r| r.strip_prefix('/')) {
        return if rest == EXAMPLE_CONFIG { Effect::None } else { Effect::Deploy };
    }
    let restart = path.starts_with("tools/pipeline/")
        || path.starts_with("tools/xtask/")
        || matches!(path, "Cargo.toml" | "Cargo.lock" | "rust-toolchain.toml" | ".cargo/config.toml");
    if restart { Effect::Restart } else { Effect::None }
}

/// Changed paths sorted by what they mean.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub restart: Vec<String>,
    pub deploy: Vec<String>,
}

pub fn classify(paths: &[String]) -> Changes {
    let mut c = Changes::default();
    for p in paths {
        match effect(p) {
            Effect::Restart => c.restart.push(p.clone()),
            Effect::Deploy => c.deploy.push(p.clone()),
            Effect::None => {}
        }
    }
    c
}

/// `a, b, c and 4 more` for log lines.
pub fn list(paths: &[String]) -> String {
    let shown: Vec<&str> = paths.iter().take(3).map(String::as_str).collect();
    match paths.len().saturating_sub(3) {
        0 => shown.join(", "),
        n => format!("{} and {n} more", shown.join(", ")),
    }
}

/// The fingerprint of the deploy files in `dir`: SHA-256 of `sha256sum`'s
/// listing of every file but [`EXAMPLE_CONFIG`], by path in byte order.
/// `deploy_hash` in setup.sh computes the same with coreutils.
pub fn deploy_hash(dir: &Path) -> Result<String> {
    let mut files: Vec<String> =
        util::files_under(dir)?.into_iter().map(|p| p.display().to_string()).filter(|p| p != EXAMPLE_CONFIG).collect();
    files.sort();
    let mut listing = String::new();
    for f in &files {
        let data = util::read(&dir.join(f))?;
        listing.push_str(&format!("{}  {f}\n", crate::sha256::hex(&data)));
    }
    Ok(crate::sha256::hex(listing.as_bytes()))
}

/// The deploy files of the runner's checkout, if `setup.sh` has not
/// installed exactly these: their fingerprint and a one-line notice.
pub fn deploy_pending(cfg: &Config) -> Result<Option<(String, String)>> {
    let dir = cfg.repo.join(DEPLOY_DIR);
    if !dir.is_dir() {
        return Ok(None);
    }
    let want = deploy_hash(&dir)?;
    let have = std::fs::read_to_string(&cfg.deploy_stamp).ok().map(|s| s.trim().to_string());
    if have.as_deref() == Some(want.as_str()) {
        return Ok(None);
    }
    let why = match have {
        Some(_) => {
            "the role wrapper, sudoers rule or systemd unit in the runner's checkout differ from what setup.sh installed"
        }
        None => "setup.sh has not recorded an install yet",
    };
    Ok(Some((want, format!("deploy files changed: run sudo {}/setup.sh ({why})", dir.display()))))
}

/// The last fingerprint the owner was told about (`<state>/deploy-notice.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployNotice {
    pub hash: String,
    pub time: String,
}

/// A new version of the loop that did not build (`<state>/self-update.json`),
/// for `loop status`; removed by the next build that passes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFailure {
    pub head: String,
    pub time: String,
    pub message: String,
}

pub fn notice_file(state: &Path) -> PathBuf {
    state.join("deploy-notice.json")
}

pub fn failure_file(state: &Path) -> PathBuf {
    state.join("self-update.json")
}

/// Shared by the workers of one loop: the commit the running binaries
/// were built from, the last head looked at, and the request to stop.
#[derive(Debug)]
pub struct SelfUpdate {
    enabled: bool,
    built: String,
    /// Look at main at least this often, also while no new entry can start.
    every: u64,
    inner: Mutex<Inner>,
    stop: AtomicBool,
}

#[derive(Debug)]
struct Inner {
    /// The last head whose changes were looked at.
    seen: Option<String>,
    /// When main was last fetched (or tried).
    refreshed: u64,
}

/// No self-update: a dry run, `--once`, tests, a loop not run from the runner's checkout.
pub static OFF: SelfUpdate = SelfUpdate::off();

impl SelfUpdate {
    pub const fn off() -> SelfUpdate {
        SelfUpdate {
            enabled: false,
            built: String::new(),
            every: u64::MAX,
            inner: Mutex::new(Inner { seen: None, refreshed: 0 }),
            stop: AtomicBool::new(false),
        }
    }

    /// The loop's binaries were built from `built`; look at main every `every` seconds.
    pub fn new(built: String, every: u64) -> SelfUpdate {
        SelfUpdate { enabled: true, built, every, ..SelfUpdate::off() }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn built(&self) -> &str {
        &self.built
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Whether main is due for a look while no new entry can start.
    pub fn due(&self, now: u64) -> bool {
        self.enabled && self.inner.lock().is_ok_and(|i| now >= i.refreshed.saturating_add(self.every))
    }

    pub fn mark_refreshed(&self, now: u64) {
        if let Ok(mut i) = self.inner.lock() {
            i.refreshed = now;
        }
    }

    /// Whether `head` is new to look at (and remember it).
    pub fn first_look(&self, head: &str) -> bool {
        let Ok(mut i) = self.inner.lock() else { return false };
        if i.seen.as_deref() == Some(head) {
            return false;
        }
        i.seen = Some(head.to_string());
        true
    }
}

/// Whether `a` and `b` are the same directory.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// `4h`, `30m`, `90s`, `2d`, `1h30m` as seconds.
pub fn parse_duration(s: &str) -> Result<u64> {
    let bad = || format!("--for {s:?}: a duration like 30m, 4h, 2d or 1h30m");
    let mut total: u64 = 0;
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            _ => return Err(bad()),
        };
        let n: u64 = digits.parse().map_err(|_| bad())?;
        total = n.checked_mul(unit).and_then(|v| total.checked_add(v)).ok_or_else(bad)?;
        digits.clear();
    }
    if !digits.is_empty() || total == 0 {
        return Err(bad());
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn which_paths_restart_the_loop_and_which_need_setup() {
        for p in [
            "tools/pipeline/src/scheduler/cycle.rs",
            "tools/pipeline/Cargo.toml",
            "tools/pipeline/windows/poly-oracle.ps1",
            "tools/xtask/src/main.rs",
            "tools/xtask/Cargo.toml",
            "Cargo.lock",
            "Cargo.toml",
            "rust-toolchain.toml",
            ".cargo/config.toml",
        ] {
            assert_eq!(effect(p), Effect::Restart, "{p}");
        }
        for p in ["tools/pipeline/deploy/setup.sh", "tools/pipeline/deploy/poly-role", "tools/pipeline/deploy/new-file"]
        {
            assert_eq!(effect(p), Effect::Deploy, "{p}");
        }
        for p in [
            "tools/pipeline/deploy/pipeline.example.toml",
            "tools/pipelines/x.rs",
            "tools/coverage/src/main.rs",
            "crates/poly-elf/src/lib.rs",
            "coverage/linux/syscall/x86_64.json",
            "kernel/Cargo.lock",
            "personalities/linux/Cargo.toml",
            "docs/AGENT_PIPELINE.md",
            "README.md",
        ] {
            assert_eq!(effect(p), Effect::None, "{p}");
        }
        let c = classify(&strings(&[
            "README.md",
            "tools/pipeline/deploy/poly-role",
            "tools/pipeline/src/main.rs",
            "Cargo.lock",
        ]));
        assert_eq!(c.restart, ["tools/pipeline/src/main.rs", "Cargo.lock"]);
        assert_eq!(c.deploy, ["tools/pipeline/deploy/poly-role"]);
        assert_eq!(classify(&strings(&["docs/x.md"])), Changes::default());
        assert_eq!(list(&strings(&["a", "b"])), "a, b");
        assert_eq!(list(&strings(&["a", "b", "c", "d", "e"])), "a, b, c and 2 more");
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("4h"), Ok(4 * 3600));
        assert_eq!(parse_duration("30m"), Ok(1800));
        assert_eq!(parse_duration("1h30m"), Ok(5400));
        assert_eq!(parse_duration("2d"), Ok(172_800));
        assert_eq!(parse_duration("90s"), Ok(90));
        for bad in ["", "4", "h", "4x", "0h", "-1h", "4 h", "99999999999999999999d"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    /// setup.sh records the same fingerprint the loop computes.
    #[test]
    fn the_deploy_fingerprint_matches_setup_sh() {
        let dir = std::env::temp_dir().join(format!("poly-deploy-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        util::write(&dir.join("poly-role"), "#!/bin/bash\n").unwrap();
        util::write(&dir.join("setup.sh"), "echo setup\n").unwrap();
        util::write(&dir.join("sub/x-y"), "x").unwrap();
        util::write(&dir.join("sub-a"), "y").unwrap();
        let h = deploy_hash(&dir).unwrap();
        util::write(&dir.join(EXAMPLE_CONFIG), "[loop]\n").unwrap();
        assert_eq!(deploy_hash(&dir).unwrap(), h, "the example configuration does not count");
        util::write(&dir.join("poly-role"), "#!/bin/bash\nexit 0\n").unwrap();
        let h2 = deploy_hash(&dir).unwrap();
        assert_ne!(h2, h);

        let script = util::read_string(&crate::repo_root().join(DEPLOY_DIR).join("setup.sh")).unwrap();
        let start = script.find("\ndeploy_hash() {").expect("setup.sh defines deploy_hash") + 1;
        let end = start + script[start..].find("\n}\n").unwrap() + 3;
        let function = &script[start..end];
        let have_tools = ["bash", "sha256sum", "find", "sort", "xargs"].iter().all(|t| util::which(t).is_some());
        if have_tools {
            let o = std::process::Command::new("bash")
                .arg("-c")
                .arg(format!("set -euo pipefail\n{function}deploy_hash \"$1\""))
                .arg("bash")
                .arg(&dir)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), h2);
        } else {
            eprintln!("skipped the shell half: no bash or coreutils");
        }

        // deploy_pending against a stamp.
        let repo = dir.join("repo");
        let deploy = repo.join(DEPLOY_DIR);
        util::write(&deploy.join("setup.sh"), "echo setup\n").unwrap();
        let cfg = Config { repo: repo.clone(), deploy_stamp: dir.join("stamp"), ..Config::default() };
        let (hash, msg) = deploy_pending(&cfg).unwrap().unwrap();
        assert!(msg.starts_with("deploy files changed: run sudo ") && msg.contains("not recorded"), "{msg}");
        util::write(&cfg.deploy_stamp, "0000\n").unwrap();
        assert!(deploy_pending(&cfg).unwrap().unwrap().1.contains("differ"));
        util::write(&cfg.deploy_stamp, format!("{hash}\n")).unwrap();
        assert_eq!(deploy_pending(&cfg).unwrap(), None);
        let none = Config { repo: dir.join("nothing"), ..cfg };
        assert_eq!(deploy_pending(&none).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
