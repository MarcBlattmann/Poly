//! `pipeline loop`: the scheduler that runs the agent pipeline unattended
//! (docs/AGENT_PIPELINE.md, "A home server running 24/7"; ADR 0056).
//!
//! ```text
//! loop [--config FILE] [--once] [--entry ID] [--dry-run] [--quiet]
//! loop status [--config FILE] [--json]
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

pub mod config;
pub mod cycle;
pub mod limits;
pub mod ops;
pub mod real;
pub mod safety;
pub mod state;

#[cfg(test)]
mod tests;

use crate::util::{self, Args, Result};
use config::Config;
use cycle::{Driver, Logger, Pass, Yielded};
use real::{DryShell, RealOps, RealShell, Shell};
use state::{Step, Store};
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: pipeline loop [--config FILE] [--once] [--entry ID] [--dry-run] [--quiet] | \
                         loop status [--config FILE] [--json] | loop reset <entry> [--config FILE] [--keep-branches]";

fn load_config(a: &Args, dry_run: bool) -> Result<(Config, Option<PathBuf>)> {
    let path = a.opt("config").map(PathBuf::from).or_else(Config::default_path);
    match path {
        Some(p) => Ok((Config::load(&p)?, Some(p))),
        None if dry_run => Ok((Config::default(), None)),
        None => Err("no configuration: pass --config, set POLY_PIPELINE_CONFIG or write /etc/poly-pipeline.toml \
             (example: tools/pipeline/deploy/pipeline.example.toml)"
            .into()),
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
        let mut d = Driver { cfg: &cfg, store: &store, log: &log, ops, name: "w0".into() };
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
    let workers = if once { 1 } else { cfg.max_parallel };
    let failed = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        for w in 0..workers {
            let (cfg, store, log, only, failed) = (&cfg, &store, &log, only.as_deref(), &failed);
            s.spawn(move || {
                let ops = RealOps::new(cfg.clone(), RealShell, &store.dir.join("logs"));
                let mut d = Driver { cfg, store, log, ops, name: format!("w{w}") };
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
    Ok(u8::from(failed.load(std::sync::atomic::Ordering::Relaxed)))
}

/// One worker: pass after pass; sleep when there is nothing to do.
pub fn worker<O: ops::Ops>(d: &mut Driver<'_, O>, once: bool, only: Option<&str>) -> Result {
    loop {
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
                d.ops.sleep(secs);
            }
            Err(e) if once => return Err(e),
            Err(e) => {
                d.log.log(3, &d.name, &format!("{e}; trying again in a minute"));
                d.ops.sleep(60);
            }
        }
    }
}

fn describe(y: &Yielded) -> String {
    match y {
        Yielded::Paused(role, until) => format!("{role} paused until {} UTC", util::format_utc(*until)),
        Yielded::Waiting(until) => format!("waiting (next look at {} UTC)", util::format_utc(*until)),
        Yielded::Retry(until) => format!("error; retry after {} UTC", util::format_utc(*until)),
        Yielded::Done(step) => format!("done: {}", step.name()),
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
    let now = util::now_secs();
    println!("config {}: {}", path.map_or("-".into(), |p| p.display().to_string()), cfg.describe());
    for role in ["spec", "implementer"] {
        match store.pause(role)? {
            Some(p) if p.until > now => {
                println!("{role} role paused until {} UTC: {}", util::format_utc(p.until), p.reason)
            }
            _ => println!("{role} role: ready{}", if store.locked(&format!("role-{role}")) { ", running" } else { "" }),
        }
    }
    if states.is_empty() {
        println!("no entries yet");
    }
    for s in &states {
        let running = store.locked(&format!("entry-{}", state::key(&s.entry)));
        let detail = match s.step {
            Step::Human => s.reason.clone().unwrap_or_default(),
            Step::AwaitingMerge | Step::Merged => s.pr_url.clone().unwrap_or_default(),
            _ => s.last_result.clone().unwrap_or_default(),
        };
        println!(
            "{:<40} {:<15} cycle {}/{} {}{}  {}",
            s.entry,
            s.step.name(),
            s.cycle,
            cfg.max_cycles,
            s.updated,
            if running { " (running)" } else { "" },
            detail
        );
    }
    let human = states.iter().filter(|s| s.step == Step::Human).count();
    if human > 0 {
        println!("{human} entries in the human queue: {}", cfg.human_queue.display());
    }
    Ok(0)
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
    println!(
        "{id}: state removed; the loop starts it again from the queue. Branches and PRs on {} are left as they are.",
        cfg.remote
    );
    Ok(0)
}
