//! `coverage`: maintain the coverage map in `coverage/` and the README progress section.
//!
//! ```text
//! coverage import wine <wine-checkout> --revision <rev>
//! coverage import linux <syscall_64.tbl> --revision <rev>
//! coverage import xnu <syscalls.master> <syscall_sw.c> --revision <rev>
//! coverage import macos-sdk <MacOSX.sdk> --revision <sdk version>   (run on a Mac)
//! coverage claim <id> --agent <agent-id>
//! coverage release <id>
//! coverage set-status <id> <missing|stub|green|blocked-by-design|app-provided>
//!                     [--tests-visible N] [--tests-held-out N]
//! coverage set-status --component <os>/<layer>/<component> [--component ...] <status>
//!                                                    (every entry of those components)
//! coverage check
//! coverage readme [--check]      README progress section + docs/progress/*.svg
//! coverage seed-wine <wine-checkout> --revision <rev>
//!                                where Wine's conformance tests exercise each Windows entry (names only, ADR 0097)
//! coverage show <id>...          one entry: status, sources, tests, seeds (seeds hidden from implementers)
//! ```

mod import;
mod model;
mod readme;
mod roadmap;
mod seeds;
mod svg;

use model::{Coverage, Entry, Source, Status};
use roadmap::Roadmap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf()
}

fn coverage_dir() -> PathBuf {
    root().join("coverage")
}

fn load() -> Result<Coverage, String> {
    Coverage::load(&coverage_dir())
}

fn save(c: &Coverage) -> Result<(), String> {
    let errors = c.check();
    if let Some(first) = errors.first() {
        return Err(format!("refusing to save: {} invariant violations, first: {first}", errors.len()));
    }
    c.save(&coverage_dir())
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
        ["macos-sdk", sdk] => (
            vec![(import::MACOS_SDK, "macOS SDK .tbd stubs (exported symbols and Objective-C classes)")],
            import::macos_sdk(Path::new(sdk))?,
        ),
        _ => {
            return Err(
                "usage: coverage import <wine DIR | linux TBL | xnu MASTER SW | macos-sdk SDK> --revision REV".into()
            );
        }
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

/// Every value following a `--name` in `args` (the option may repeat).
fn opts<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].as_str()).collect()
}

fn parse_status(s: &str) -> Result<Status, String> {
    let names: Vec<&str> = Status::ALL.iter().map(|s| s.name()).collect();
    Status::parse(s).ok_or_else(|| format!("unknown status {s} (one of {})", names.join(", ")))
}

fn set_status(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: coverage set-status <id> <status> [--tests-visible N] [--tests-held-out N] | coverage set-status --component <os>/<layer>/<component> ... <status>";
    let pos = positional(args);
    let components = opts(args, "--component");
    if !components.is_empty() {
        let [status] = pos.as_slice() else { return Err(USAGE.into()) };
        let status = parse_status(status)?;
        let mut c = load()?;
        for component in components {
            let n = c.set_component_status(component, status)?;
            println!("{component}: {n} entries set to {}", status.name());
        }
        return save(&c);
    }
    let [id, status] = pos.as_slice() else { return Err(USAGE.into()) };
    let status = parse_status(status)?;
    if status == Status::Claimed {
        return Err("use `coverage claim` to claim".into());
    }
    let count = |name: &str| -> Result<Option<u32>, String> {
        opt(args, name).map(|n| n.parse::<u32>().map_err(|_| format!("{name} {n}: a number"))).transpose()
    };
    let (visible, held_out) = (count("--tests-visible")?, count("--tests-held-out")?);
    let mut c = load()?;
    let e = c.find_mut(id).ok_or_else(|| format!("no entry {id}"))?;
    e.status = status;
    e.claimed_by = None;
    if let Some(n) = visible {
        e.tests_visible = n;
    }
    if let Some(n) = held_out {
        e.tests_held_out = n;
    }
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
    let stale = c.stale_files(&coverage_dir())?;
    if !stale.is_empty() {
        let names: Vec<String> = stale.iter().take(5).map(|p| p.display().to_string()).collect();
        return Err(format!(
            "{} coverage files not in canonical form (e.g. {}); run `cargo xtask coverage readme` or any write command",
            stale.len(),
            names.join(", ")
        ));
    }
    println!("coverage map ok: {} entries", c.entries.len());
    Ok(())
}

fn readme(args: &[String]) -> Result<(), String> {
    let roadmap_path = root().join("docs/ROADMAP.md");
    let roadmap = Roadmap::parse(
        &std::fs::read_to_string(&roadmap_path).map_err(|e| format!("{}: {e}", roadmap_path.display()))?,
    )?;
    let coverage = load()?;
    let readme_path = root().join("README.md");
    let current = std::fs::read_to_string(&readme_path).map_err(|e| format!("{}: {e}", readme_path.display()))?;

    let mut files = readme::assets(&coverage, &roadmap);
    let updated = readme::splice(&current, &readme::render(&coverage, &roadmap))?;
    files.push(("README.md".into(), readme::splice_phases(&updated, &readme::render_phases(&roadmap))?));

    let check = args.iter().any(|a| a == "--check");
    let mut stale = Vec::new();
    for (rel, contents) in &files {
        let path = root().join(rel);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()) {
            continue;
        }
        if check {
            stale.push(rel.clone());
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
            println!("updated {rel}");
        }
    }
    if !stale.is_empty() {
        return Err(format!("stale: {}; run `cargo xtask coverage readme`", stale.join(", ")));
    }
    if check {
        println!("README progress section, phase table and graphics up to date");
    }
    Ok(())
}

/// `seed-wine <checkout> --revision <rev>`: replace every entry's `wine:`
/// seeds with what the checkout's conformance tests call (ADR 0097).
fn seed_wine(args: &[String]) -> Result<(), String> {
    let pos = positional(args);
    let [dir] = pos.as_slice() else {
        return Err("usage: coverage seed-wine <wine checkout> --revision <wine commit>".into());
    };
    let revision = opt(args, "--revision").ok_or("--revision <wine commit> is required")?;
    // A git checkout must be at the revision it is recorded as (only read,
    // so a checkout owned by another user, as in a container, is fine).
    let safe = format!("safe.directory={dir}");
    if let Ok(out) = std::process::Command::new("git")
        .args(["-c", &safe, "-C", dir, "rev-parse", "HEAD"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        && out.status.success()
    {
        let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !head.starts_with(&revision.to_ascii_lowercase()) {
            return Err(format!("{dir} is at {head}, not {revision}"));
        }
    }
    let files = seeds::wine_test_files(Path::new(dir))?;
    if files.is_empty() {
        return Err(format!("{dir}: no dlls/*/tests/*.c; is it a Wine checkout?"));
    }
    let mut c = load()?;
    let (found, stats) = seeds::seeds(&c, &files);
    let source = Source { description: seeds::WINE_DESCRIPTION.into(), revision: revision.into() };
    let seeded = c.set_seeds(seeds::WINE, source, &found)?;
    save(&c)?;
    println!(
        "{} test files, {} functions ({} that check something), {} entry/test pairs: {seeded} entries seeded (at most {} each)",
        stats.files,
        stats.functions,
        stats.checking,
        stats.pairs,
        seeds::MAX_SEEDS
    );
    Ok(())
}

/// `show <id>...`: what the map knows of entries. Seeds point into other
/// projects' tests, which implementers may not read (CLEAN_ROOM.md), so
/// under `POLY_ROLE=implementer` only their number is shown.
fn show(args: &[String]) -> Result<(), String> {
    let ids = positional(args);
    if ids.is_empty() {
        return Err("usage: coverage show <id>...".into());
    }
    let implementer = std::env::var("POLY_ROLE").is_ok_and(|r| r == "implementer");
    let c = load()?;
    for id in ids {
        let e = c.find(id).ok_or_else(|| format!("no entry {id}"))?;
        println!("{}", e.id);
        let claim = e.claimed_by.as_ref().map(|a| format!(" by {a}")).unwrap_or_default();
        println!("  status   {}{claim}", e.status.name());
        if let Some(n) = e.number {
            println!("  number   {n}");
        }
        if let Some(f) = &e.forward {
            println!("  forward  {f}");
        }
        if !e.arch.is_empty() {
            println!("  arch     {}", e.arch.join(" "));
        }
        for s in &e.sources {
            let rev = c.sources.get(s).map(|s| s.revision.as_str()).unwrap_or("?");
            println!("  source   {s} @ {rev}");
        }
        println!("  tests    {} visible, {} held out", e.tests_visible, e.tests_held_out);
        if e.weight > 0.0 {
            println!("  weight   {}", e.weight);
        }
        if e.seeds.is_empty() {
            println!("  seeds    none");
        } else if implementer {
            println!("  seeds    {} (not shown to implementers: CLEAN_ROOM.md)", e.seeds.len());
        } else {
            let prefixes: BTreeSet<&str> = e.seeds.iter().filter_map(|s| s.split_once(':')).map(|(p, _)| p).collect();
            for p in prefixes {
                let rev = c.seeds.get(p).map(|s| s.revision.as_str()).unwrap_or("?");
                println!("  seeds    {p} @ {rev} (spec agents only: read for behaviour, never copy)");
            }
            for s in &e.seeds {
                println!("    {s}");
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("import") => import(rest),
        Some("seed-wine") => seed_wine(rest),
        Some("show") => show(rest),
        Some("claim") => claim(rest),
        Some("release") => release(rest),
        Some("set-status") => set_status(rest),
        Some("check") => check(),
        Some("readme") => readme(rest),
        _ => Err("usage: coverage <import|seed-wine|show|claim|release|set-status|check|readme> ...".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("coverage: {e}");
            ExitCode::FAILURE
        }
    }
}
