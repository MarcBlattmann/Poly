//! `coverage`: maintain coverage/coverage.json and the README progress section.
//!
//! ```text
//! coverage import wine <wine-checkout> --revision <rev>
//! coverage import linux <syscall_64.tbl> --revision <rev>
//! coverage import xnu <syscalls.master> <syscall_sw.c> --revision <rev>
//! coverage claim <id> --agent <agent-id>
//! coverage release <id>
//! coverage set-status <id> <missing|stub|green|blocked-by-design>
//! coverage check
//! coverage readme [--check]
//! ```

mod import;
mod model;
mod readme;

use model::{Coverage, Entry, Source, Status};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf()
}

fn coverage_path() -> PathBuf {
    root().join("coverage/coverage.json")
}

fn load() -> Result<Coverage, String> {
    match std::fs::read_to_string(coverage_path()) {
        Ok(text) => Coverage::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Coverage::default()),
        Err(e) => Err(format!("{}: {e}", coverage_path().display())),
    }
}

fn save(c: &Coverage) -> Result<(), String> {
    let errors = c.check();
    if let Some(first) = errors.first() {
        return Err(format!("refusing to save: {} invariant violations, first: {first}", errors.len()));
    }
    std::fs::write(coverage_path(), c.to_text()).map_err(|e| e.to_string())
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

/// Value following `--name` in `args`.
fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn positional(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with("--") {
            skip = !matches!(a.as_str(), "--check");
        } else {
            out.push(a.as_str());
        }
    }
    out
}

fn import(args: &[String]) -> Result<(), String> {
    let pos = positional(args);
    let revision = opt(args, "--revision").ok_or("--revision <upstream commit or version> is required")?;
    let mut c = load()?;
    let (sources, entries): (Vec<(&str, &str)>, Vec<Entry>) = match pos.as_slice() {
        ["wine", dir] => (
            vec![(import::WINE_SPEC, "Wine .spec export lists (wine-mirror/wine dlls/*/*.spec)")],
            import::wine_specs(Path::new(dir))?,
        ),
        ["linux", tbl] => (
            vec![(import::LINUX_TBL, "Linux arch/x86/entry/syscalls/syscall_64.tbl")],
            import::linux_syscall_tbl(&read(tbl)?),
        ),
        ["xnu", master, sw] => {
            let mut v = import::xnu_syscalls_master(&read(master)?);
            v.extend(import::xnu_mach_traps(&read(sw)?));
            (
                vec![
                    (import::XNU_SYSCALLS, "XNU bsd/kern/syscalls.master"),
                    (import::XNU_MACH_TRAPS, "XNU osfmk/kern/syscall_sw.c mach_trap_table"),
                ],
                v,
            )
        }
        _ => return Err("usage: coverage import <wine DIR | linux TBL | xnu MASTER SW> --revision REV".into()),
    };
    for (id, description) in sources {
        c.sources.insert(id.into(), Source { description: description.into(), revision: revision.into() });
    }
    let n = entries.len();
    let stats = c.merge(entries);
    save(&c)?;
    println!("imported {n} entries: {} new, {} updated; {} total", stats.added, stats.updated, c.entries.len());
    Ok(())
}

fn claim(args: &[String]) -> Result<(), String> {
    let pos = positional(args);
    let [id] = pos.as_slice() else { return Err("usage: coverage claim <id> --agent <agent-id>".into()) };
    let agent = opt(args, "--agent").ok_or("--agent is required")?;
    let mut c = load()?;
    let e = c.find_mut(id).ok_or_else(|| format!("no entry {id}"))?;
    if e.status != Status::Missing && e.status != Status::Stub {
        return Err(format!(
            "{id} is {}{}",
            e.status.name(),
            e.claimed_by.as_ref().map_or(String::new(), |a| format!(" by {a}"))
        ));
    }
    e.status = Status::Claimed;
    e.claimed_by = Some(agent.to_string());
    save(&c)
}

fn release(args: &[String]) -> Result<(), String> {
    let pos = positional(args);
    let [id] = pos.as_slice() else { return Err("usage: coverage release <id>".into()) };
    let mut c = load()?;
    let e = c.find_mut(id).ok_or_else(|| format!("no entry {id}"))?;
    if e.status != Status::Claimed {
        return Err(format!("{id} is not claimed"));
    }
    e.status = Status::Missing;
    e.claimed_by = None;
    save(&c)
}

fn set_status(args: &[String]) -> Result<(), String> {
    let pos = positional(args);
    let [id, status] = pos.as_slice() else { return Err("usage: coverage set-status <id> <status>".into()) };
    let status = Status::parse(status).ok_or_else(|| format!("unknown status {status}"))?;
    if status == Status::Claimed {
        return Err("use `coverage claim` to claim".into());
    }
    let mut c = load()?;
    let e = c.find_mut(id).ok_or_else(|| format!("no entry {id}"))?;
    e.status = status;
    e.claimed_by = None;
    save(&c)
}

fn check() -> Result<(), String> {
    let c = load()?;
    let errors = c.check();
    for e in &errors {
        eprintln!("coverage: {e}");
    }
    if !errors.is_empty() {
        return Err(format!("{} invariant violations", errors.len()));
    }
    let text = std::fs::read_to_string(coverage_path()).map_err(|e| e.to_string())?;
    if text != c.to_text() {
        return Err("coverage.json is not in canonical form; run any coverage command to rewrite it".into());
    }
    println!("coverage.json ok: {} entries", c.entries.len());
    Ok(())
}

fn readme(args: &[String]) -> Result<(), String> {
    let path = root().join("README.md");
    let current = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let updated = readme::splice(&current, &readme::render(&load()?))?;
    if args.iter().any(|a| a == "--check") {
        if updated != current {
            return Err("README coverage section is stale; run `cargo xtask coverage readme`".into());
        }
        println!("README coverage section up to date");
        return Ok(());
    }
    if updated != current {
        std::fs::write(&path, updated).map_err(|e| e.to_string())?;
        println!("README coverage section updated");
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("import") => import(rest),
        Some("claim") => claim(rest),
        Some("release") => release(rest),
        Some("set-status") => set_status(rest),
        Some("check") => check(),
        Some("readme") => readme(rest),
        _ => Err("usage: coverage <import|claim|release|set-status|check|readme> ...".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("coverage: {e}");
            ExitCode::FAILURE
        }
    }
}
