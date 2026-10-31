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
//! ```

mod import;
mod model;
mod readme;
mod roadmap;
mod svg;

use model::{Coverage, Entry, Source, Status};
use roadmap::Roadmap;
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
