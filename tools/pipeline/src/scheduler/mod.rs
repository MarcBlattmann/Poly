//! `pipeline loop`: the scheduler that runs the agent pipeline unattended
//! (docs/AGENT_PIPELINE.md, "A home server running 24/7"; ADR 0056).
//!
//! ```text
//! loop [--config FILE] [--once] [--entry ID] [--dry-run] [--quiet]
//! loop status [--config FILE] [--json]
//! loop watch [--config FILE] [--interval SECS] [--once] [--json] [--width N] [--lines N]
//! loop pause [--role spec|implementer|all] [--for 4h] [--reason TEXT] [--config FILE]
//! loop resume [--role spec|implementer|all] [--config FILE]
//! loop reset <entry> [--config FILE] [--keep-branches]
//! ```
//!
//! `max_parallel` workers each pick an entry that can move (or a new one
//! from `pipeline next`), drive it through [`cycle`] until it has to wait
//! (a role's usage limit, a PR waiting for its merge, an error back-off)
//! or ends (merged, or in the human queue), and pick again. `--once` drives
//! one entry until it waits or ends, then exits; `--entry` restricts the
//! loop to one entry; `--dry-run` prints every command the cycle would run
//! for the next entry, pretending each succeeds, and changes nothing.
//! When main changes the loop's own code, the loop builds it and exits
//! with [`update::RESTART_EXIT`] once the steps in progress end, for
//! systemd to start the new code ([`update`]). `pause` and `resume` write
//! and remove the pause files the cycle reads before an agent session.
//! `watch` is a live, read-only dashboard of all of it ([`watch`]).

pub mod config;
pub mod cycle;
pub mod limits;
pub mod ops;
pub mod real;
pub mod safety;
pub mod state;
pub mod update;
pub mod watch;

#[cfg(test)]
mod tests;

use crate::util::{self, Args, Result};
use config::Config;
use cycle::{Driver, Logger, Pass, Yielded};
use real::{DryShell, RealOps, RealShell, Shell};
use state::{EntryState, Pause, Step, Store};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use update::SelfUpdate;

pub const USAGE: &str = "usage: pipeline loop [--config FILE] [--once] [--entry ID] [--dry-run] [--quiet] | \
                         loop status [--config FILE] [--json] | \
                         loop watch [--config FILE] [--interval SECS] [--once] [--json] [--width N] [--lines N] | \
                         loop pause [--role spec|implementer|all] [--for 4h] [--reason TEXT] [--config FILE] | \
                         loop resume [--role spec|implementer|all] [--config FILE] | \
                         loop reset <entry> [--config FILE] [--keep-branches]";

/// The LLM roles, by the names of their pause files.
pub const ROLES: [&str; 2] = ["spec", "implementer"];

fn load_config(a: &Args, dry_run: bool) -> Result<(Config, Option<PathBuf>)> {
    let path = a.opt("config").map(PathBuf::from).or_else(Config::default_path);
    match path {
        Some(p) => Ok((Config::load(&p)?, Some(p))),
        None if dry_run => Ok((Config::default(), None)),
        None => Err(format!(
            "no configuration: pass --config, set POLY_PIPELINE_CONFIG or write /etc/poly-pipeline.toml or {} \
             (example: tools/pipeline/deploy/pipeline.example.toml)",
            config::INSTALLED
        )),
    }
}

pub fn command(repo: &Path, args: &[String]) -> Result<u8> {
    let a = Args::parse(args, &["once", "dry-run", "quiet", "json", "keep-branches"])?;
    match a.positional.first().map(String::as_str) {
        None => {
            a.only(&["config", "once", "entry", "dry-run", "quiet"])?;
            run_loop(repo, &a)
        }
        Some("status") => {
            a.only(&["config", "json"])?;
            status(&a)
        }
        Some("watch") => {
            a.only(watch::OPTIONS)?;
            watch::command(&a)
        }
        Some("pause") => {
            a.only(&["config", "role", "for", "reason"])?;
            print!("{}", pause(&a, util::now_secs())?);
            Ok(0)
        }
        Some("resume") => {
            a.only(&["config", "role"])?;
            print!("{}", resume(&a, util::now_secs())?);
            Ok(0)
        }
        Some("reset") => {
            a.only(&["config", "keep-branches"])?;
            reset(&a)
        }
        Some(other) => Err(format!("unknown loop command {other}\n{USAGE}")),
    }
}

fn run_loop(repo: &Path, a: &Args) -> Result<u8> {
    let dry_run = a.has("dry-run");
    let (mut cfg, path) = load_config(a, dry_run)?;
    let only = a.opt("entry").map(String::from);
    if let Some(e) = &only {
        crate::store::EntryId::parse(e)?;
    }
    if dry_run {
        // Nothing real is touched: state and logs in a scratch directory,
        // the queue from this checkout, every command printed.
        let scratch = repo.join("target/pipeline/loop-dry-run");
        let _ = std::fs::remove_dir_all(&scratch);
        cfg.state_dir = scratch.join("state");
        cfg.log_file = Some(scratch.join("pipeline.log"));
        cfg.max_parallel = 1;
        let store = Store::new(&cfg.state_dir)?;
        let log = Logger::new(cfg.log_file.as_deref(), false)?;
        log.log(
            6,
            "",
            &format!("dry run with {}", path.map_or("the default configuration".into(), |p| p.display().to_string())),
        );
        let mut ops = RealOps::new(cfg.clone(), DryShell::new(a.has("quiet")), &store.dir.join("logs"));
        ops.queue_repo = repo.to_path_buf();
        let mut d = Driver { cfg: &cfg, store: &store, log: &log, ops, name: "w0".into(), update: &update::OFF };
        return match d.pass(only.as_deref())? {
            Pass::Worked(entry, y) => {
                log.log(6, "", &format!("dry run: {entry} would reach {y:?}"));
                Ok(0)
            }
            Pass::Idle(_) => {
                log.log(6, "", "dry run: nothing to do");
                Ok(0)
            }
        };
    }
    if std::env::var("POLY_ROLE").is_ok_and(|r| r == "implementer") {
        return Err("the loop reads the held-out store; implementer agents may not run it".into());
    }
    let store = Store::new(&cfg.state_dir)?;
    let log = Logger::new(cfg.log_file.as_deref(), a.has("quiet"))?;
    log.log(
        6,
        "",
        &format!(
            "pipeline loop started ({}): {}",
            path.map_or("defaults".into(), |p| p.display().to_string()),
            cfg.describe()
        ),
    );
    let once = a.has("once");
    let update = self_update(repo, &cfg, once, &log);
    let workers = if once { 1 } else { cfg.max_parallel };
    let failed = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        for w in 0..workers {
            let (cfg, store, log, only, failed, update) = (&cfg, &store, &log, only.as_deref(), &failed, &update);
            s.spawn(move || {
                let ops = RealOps::new(cfg.clone(), RealShell, &store.dir.join("logs"));
                let mut d = Driver { cfg, store, log, ops, name: format!("w{w}"), update };
                if let Err(e) = worker(&mut d, once, only) {
                    log.log(3, &d.name, &format!("stopped: {e}"));
                    failed.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
            // Do not let all workers hit git and the queue in the same second.
            if !once {
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    });
    if update.stopping() {
        log.log(
            6,
            "",
            &format!(
                "pipeline loop stopped to run its new code (exit {}); systemd starts it again",
                update::RESTART_EXIT
            ),
        );
        return Ok(update::RESTART_EXIT);
    }
    Ok(u8::from(failed.load(std::sync::atomic::Ordering::Relaxed)))
}

/// Self-update is on when the loop runs from the runner's checkout (as
/// the systemd unit runs it), forever (not `--once`), and not turned off.
fn self_update(repo: &Path, cfg: &Config, once: bool, log: &Logger) -> SelfUpdate {
    let off = |why: String| {
        log.log(6, "", &format!("self-update off: {why}"));
        SelfUpdate::off()
    };
    if !cfg.self_update {
        return off("loop.self_update = false".into());
    }
    if once {
        return off("--once".into());
    }
    if !update::same_dir(repo, &cfg.repo) {
        return off(format!(
            "this loop runs from {}, not the runner's checkout {}",
            repo.display(),
            cfg.repo.display()
        ));
    }
    let head = RealShell.run(&real::Cmd::new("git", &["-C", &cfg.repo.display().to_string(), "rev-parse", "HEAD"]));
    match head {
        Ok(o) if o.ok() => {
            let head = o.text.trim().to_string();
            log.log(
                6,
                "",
                &format!(
                    "self-update on: running the code of {}; main is looked at every {} s at least",
                    &head[..head.len().min(12)],
                    cfg.poll_secs
                ),
            );
            SelfUpdate::new(head, cfg.poll_secs)
        }
        Ok(o) => off(format!("git rev-parse HEAD in {}: {}", cfg.repo.display(), o.text.trim())),
        Err(e) => off(e),
    }
}

/// One worker: pass after pass; sleep when there is nothing to do.
pub fn worker<O: ops::Ops>(d: &mut Driver<'_, O>, once: bool, only: Option<&str>) -> Result {
    loop {
        if d.update.stopping() {
            return Ok(());
        }
        match d.pass(only) {
            Ok(Pass::Worked(entry, y)) => {
                d.log.log(6, &d.name, &format!("{entry}: {}", describe(&y)));
                if once {
                    return Ok(());
                }
            }
            Ok(Pass::Idle(until)) => {
                if once {
                    d.log.log(6, &d.name, "nothing to do now");
                    return Ok(());
                }
                let now = d.ops.now();
                let secs = until.saturating_sub(now).clamp(30, d.cfg.idle_secs.max(30));
                nap(d, secs);
            }
            Err(e) if once => return Err(e),
            Err(e) => {
                d.log.log(3, &d.name, &format!("{e}; trying again in a minute"));
                nap(d, 60);
            }
        }
    }
}

/// Sleep `secs`, waking within half a minute when the loop is to stop.
fn nap<O: ops::Ops>(d: &mut Driver<'_, O>, secs: u64) {
    let mut left = secs;
    while left > 0 && !d.update.stopping() {
        let s = left.min(30);
        d.ops.sleep(s);
        left -= s;
    }
}

fn describe(y: &Yielded) -> String {
    match y {
        Yielded::Paused(role, until) => format!("{role} paused {}", state::until_text(*until)),
        Yielded::Waiting(until) => format!("waiting (next look at {} UTC)", util::format_utc(*until)),
        Yielded::Retry(until) => format!("error; retry after {} UTC", util::format_utc(*until)),
        Yielded::Done(step) => format!("done: {}", step.name()),
        Yielded::Stopping(step) => format!("the loop stops for its update; resumes at {}", step.name()),
    }
}

fn status(a: &Args) -> Result<u8> {
    let (cfg, path) = load_config(a, false)?;
    if !cfg.state_dir.is_dir() {
        println!("no state in {} yet (the loop has not run)", cfg.state_dir.display());
        return Ok(0);
    }
    let store = Store::new(&cfg.state_dir)?;
    let mut states = store.load_all()?;
    states.sort_by(|a, b| a.created.cmp(&b.created));
    if a.has("json") {
        println!("{}", serde_json::to_string_pretty(&states).map_err(|e| e.to_string())?);
        return Ok(0);
    }
    print!("{}", status_text(&cfg, path.as_deref(), &store, &states, util::now_secs())?);
    Ok(0)
}

/// What `loop status` prints: the configuration, the loop's own notices
/// (deploy files, a new version that does not build), the roles and
/// their pauses, every entry.
pub fn status_text(
    cfg: &Config,
    path: Option<&Path>,
    store: &Store,
    states: &[EntryState],
    now: u64,
) -> Result<String> {
    let mut out = String::new();
    let _ = writeln!(out, "config {}: {}", path.map_or("-".into(), |p| p.display().to_string()), cfg.describe());
    match update::deploy_pending(cfg) {
        Ok(Some((_, message))) => {
            let _ = writeln!(out, "{message}");
        }
        Ok(None) => {}
        Err(e) => {
            let _ = writeln!(out, "deploy files: cannot check them: {e}");
        }
    }
    if let Ok(f) = util::read_json::<update::BuildFailure>(&update::failure_file(&store.dir)) {
        let _ = writeln!(out, "self-update: {} (since {} UTC)", f.message, f.time);
    }
    for role in ROLES {
        match store.pause(role)? {
            Some(p) if p.until > now => {
                let _ = writeln!(out, "{role} role: paused {}: {}", state::until_text(p.until), p.reason);
            }
            _ => {
                let running = if store.locked(&format!("role-{role}")) { ", running" } else { "" };
                let _ = writeln!(out, "{role} role: ready{running}");
            }
        }
    }
    if states.is_empty() {
        out.push_str("no entries yet\n");
    }
    for s in states {
        let running = store.locked(&format!("entry-{}", state::key(&s.entry)));
        let detail = match s.step {
            Step::Human => s.reason.clone().unwrap_or_default(),
            Step::AwaitingMerge | Step::Merged => s.pr_url.clone().unwrap_or_default(),
            _ => s.last_result.clone().unwrap_or_default(),
        };
        let _ = writeln!(
            out,
            "{:<40} {:<15} cycle {}/{}{} {}{}  {}",
            s.entry,
            s.step.name(),
            s.cycle,
            cfg.max_cycles,
            if s.spec_cycle > 0 {
                format!(" (spec check {}/{})", s.spec_cycle, cfg.spec_cycles)
            } else {
                String::new()
            },
            s.updated,
            if running { " (running)" } else { "" },
            detail
        );
    }
    let human = states.iter().filter(|s| s.step == Step::Human).count();
    if human > 0 {
        let _ = writeln!(out, "{human} entries in the human queue: {}", cfg.human_queue.display());
    }
    Ok(out)
}

/// The roles `--role` names (`all` when it is not given).
fn roles(a: &Args) -> Result<Vec<&'static str>> {
    match a.opt("role").unwrap_or("all") {
        "all" => Ok(ROLES.to_vec()),
        r => ROLES
            .iter()
            .copied()
            .find(|x| *x == r)
            .map(|x| vec![x])
            .ok_or(format!("--role {r}: spec, implementer or all")),
    }
}

/// `loop pause`: no new session of the role(s) until then (or `loop resume`).
fn pause(a: &Args, now: u64) -> Result<String> {
    if a.positional.len() > 1 {
        return Err(USAGE.into());
    }
    let roles = roles(a)?;
    let until = match a.opt("for") {
        Some(d) => now.saturating_add(update::parse_duration(d)?),
        None => state::FOREVER,
    };
    let reason = a.opt("reason").unwrap_or("paused with `loop pause`").to_string();
    let (cfg, _) = load_config(a, false)?;
    let store = Store::new(&cfg.state_dir)?;
    let mut out = String::new();
    for role in roles {
        store.set_pause(role, &Pause { until, reason: reason.clone() })?;
        let _ = writeln!(out, "{role} role paused {}: {reason}", state::until_text(until));
    }
    out.push_str(
        "A session that is running finishes; no new one starts. The runner's own steps (CI, diff, PRs) go on.\n",
    );
    Ok(out)
}

/// `loop resume`: remove the pause file(s), usage-limit pauses too.
fn resume(a: &Args, now: u64) -> Result<String> {
    if a.positional.len() > 1 {
        return Err(USAGE.into());
    }
    let roles = roles(a)?;
    let (cfg, _) = load_config(a, false)?;
    let store = Store::new(&cfg.state_dir)?;
    let mut out = String::new();
    for role in roles {
        let was = store.pause(role)?.filter(|p| p.until > now);
        store.clear_pause(role)?;
        let _ = match was {
            Some(p) => {
                writeln!(out, "{role} role resumed (it was paused {}: {})", state::until_text(p.until), p.reason)
            }
            None => writeln!(out, "{role} role was not paused"),
        };
    }
    let _ = writeln!(out, "The loop starts the next session within {} s (loop.idle_secs).", cfg.idle_secs);
    Ok(out)
}

fn reset(a: &Args) -> Result<u8> {
    let [_, id] = a.positional.as_slice() else { return Err(USAGE.into()) };
    let entry = crate::store::EntryId::parse(id)?;
    let (cfg, _) = load_config(a, false)?;
    let store = Store::new(&cfg.state_dir)?;
    let Some(_lock) = store.try_lock(&format!("entry-{}", state::key(id)))? else {
        return Err(format!("{id} is being worked on right now; stop the loop first"));
    };
    if store.load(id)?.is_none() {
        return Err(format!("the loop has no state for {id}"));
    }
    if !a.has("keep-branches") {
        let mut sh = RealShell;
        for b in [cycle::spec_branch(&entry), cycle::impl_branch(&entry)] {
            let c = real::Cmd::new(
                "git",
                &["-C", &cfg.repo.display().to_string(), "push", "-q", &cfg.hub.display().to_string(), "--delete", &b],
            );
            match sh.run(&c) {
                Ok(o) if o.ok() => println!("deleted {b} in the hub"),
                _ => println!("{b}: not in the hub"),
            }
        }
    }
    store.remove(id)?;
    println!("{id}: state removed; the loop starts it again from the queue.");
    println!(
        "Branches and PRs on {} are left as they are. The loop never forces a push, so delete old branches \
         there before the new attempt reaches them:\n  git push {} --delete {} {}",
        cfg.remote,
        cfg.remote,
        cycle::spec_branch(&entry),
        cycle::impl_branch(&entry)
    );
    Ok(0)
}
