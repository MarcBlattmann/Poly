//! Prebuilt CI artifacts (ADR 0156): build once, boot many times.
//!
//! The boot and pipeline jobs of CI used to build the same kernel, native
//! programs and Windows test programs each (about two minutes of a four-core
//! runner, seven times over). Now two `build` jobs make them once
//! (`cargo xtask prebuild kernel|tools`), pack what they made into an
//! artifact, and the other jobs unpack it and run with `POLY_PREBUILT=1`:
//! every build step whose output is among the prebuilt files is skipped (the
//! step checks the file is there and says so), and the pipeline tool is
//! run from its binary instead of through `cargo run`. Without the
//! variable nothing changes: local runs build as they always did.
//!
//! What goes in which artifact:
//!
//! - `kernel`: the kernel ELF (with our ntdll.dll and Win32 modules
//!   embedded);
//! - `tools`: the native programs (`userland/native`), the xtask binary itself, the pipeline tool's binary, the
//!   traced 7-Zip scenario's kit and driver, the 7-Zip scenario's driver
//!   and the desktop's Windows test program.
//!
//! Each artifact carries a stamp file holding the commit it was built from;
//! a consumer of another commit's artifact refuses to run ([`check_stamps`]).

use crate::{Result, root};
use std::path::{Path, PathBuf};

/// The environment variable that turns prebuilt mode on (`1`).
pub const ENV: &str = "POLY_PREBUILT";

/// Whether this run takes the CI build job's outputs instead of building.
pub fn active() -> bool {
    std::env::var_os(ENV).is_some_and(|v| v == "1")
}

/// The parts of the prebuild, each one artifact.
const PARTS: [&str; 2] = ["kernel", "tools"];

/// `target/prebuilt-<part>.stamp`: the commit the part was built from.
fn stamp_path(part: &str) -> PathBuf {
    root().join("target").join(format!("prebuilt-{part}.stamp"))
}

/// The checked-out commit.
fn head() -> Result<String> {
    let out = std::process::Command::new("git")
        .current_dir(root())
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("git rev-parse HEAD: {e}"))?;
    if !out.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Refuse prebuilt outputs of another commit than the checked-out one (a
/// job that downloaded an artifact of a different run, or a stale local
/// directory). Does nothing unless [`active`]; `parts` names the artifacts
/// the job needs.
pub fn check_stamps(parts: &[&str]) -> Result {
    if !active() {
        return Ok(());
    }
    let head = head()?;
    for part in parts {
        let path = stamp_path(part);
        let stamp = std::fs::read_to_string(&path)
            .map_err(|e| format!("{ENV}=1 but the prebuilt `{part}` files are not here ({}: {e})", path.display()))?;
        if stamp.trim() != head {
            return Err(format!(
                "the prebuilt `{part}` files are from commit {}, the checkout is {head}",
                stamp.trim()
            ));
        }
    }
    eprintln!("==> prebuilt: {} from commit {}", parts.join(" and "), &head[..head.len().min(12)]);
    Ok(())
}

/// In prebuilt mode: check that the outputs of the build step `what` are
/// there (it is skipped) and return `true`; `false` when the step has to
/// run (not in prebuilt mode).
pub fn skip(what: &str, outputs: &[PathBuf]) -> Result<bool> {
    if !active() {
        return Ok(false);
    }
    if let Some(missing) = outputs.iter().find(|p| !p.is_file()) {
        return Err(format!("{ENV}=1 but {} is missing, so `{what}` cannot be skipped", missing.display()));
    }
    eprintln!("==> prebuilt: {what}");
    Ok(true)
}

/// The command that runs the pipeline tool with the arguments to come: its
/// prebuilt binary, or `cargo run` (which builds it first).
pub fn pipeline_tool() -> std::process::Command {
    if active() {
        let mut cmd = std::process::Command::new(root().join("target/debug/pipeline"));
        cmd.current_dir(root());
        cmd
    } else {
        let mut cmd = crate::cargo();
        cmd.current_dir(root()).args(["run", "--quiet", "--package", "poly-pipeline", "--"]);
        cmd
    }
}

/// `prebuild <kernel|tools> [--list <file>]`: build the part and write the
/// paths of its files (relative to the checkout, one per line) to the list
/// the workflow packs.
pub fn command(args: &[String]) -> Result {
    let usage = "usage: cargo xtask prebuild <kernel|tools> [--list <file>]";
    let part = args.first().map(String::as_str).ok_or(usage)?;
    let mut list = None;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--list" => list = Some(PathBuf::from(it.next().ok_or(usage)?)),
            _ => return Err(usage.into()),
        }
    }
    if active() {
        return Err(format!("prebuild builds: run it without {ENV}=1"));
    }
    let mut files = match part {
        "kernel" => kernel_files()?,
        "tools" => tools_files()?,
        _ => return Err(format!("no prebuild part {part:?}; the parts are {}", PARTS.join(", "))),
    };
    let stamp = stamp_path(part);
    std::fs::write(&stamp, head()? + "\n").map_err(|e| format!("{}: {e}", stamp.display()))?;
    files.push(stamp);
    let rel: Vec<String> = files
        .iter()
        .map(|p| {
            let r = p.strip_prefix(root()).unwrap_or(p);
            r.to_string_lossy().replace('\\', "/")
        })
        .collect();
    let missing: Vec<&String> = files.iter().zip(&rel).filter(|(p, _)| !p.is_file()).map(|(_, r)| r).collect();
    if !missing.is_empty() {
        return Err(format!("prebuild {part}: expected files are missing: {missing:?}"));
    }
    if let Some(list) = list {
        std::fs::write(&list, rel.join("\n") + "\n").map_err(|e| format!("{}: {e}", list.display()))?;
    }
    eprintln!("==> prebuild {part}: {} files", rel.len());
    Ok(())
}

/// The native programs' files (`native::PROGRAMS`).
fn native_files() -> Vec<PathBuf> {
    let dir = root().join("target/native/x86_64-unknown-poly/release");
    crate::native::PROGRAMS.iter().map(|n| dir.join(n)).collect()
}

/// The kernel part: the kernel alone (the native programs are the tools
/// part's, which has time to spare).
fn kernel_files() -> Result<Vec<PathBuf>> {
    crate::build_kernel_only(false)?;
    Ok(vec![crate::kernel_elf(false)])
}

/// Every file below `dir`, in order (nothing for a missing directory).
pub fn dir_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { todo.push(p) } else { files.push(p) }
        }
    }
    files.sort();
    files
}

/// The tools part: xtask, the pipeline tool, and the Windows programs the
/// boot groups would build.
fn tools_files() -> Result<Vec<PathBuf>> {
    let mut pipeline = crate::cargo();
    pipeline.current_dir(root()).args(["build", "--package", "poly-pipeline"]);
    crate::step("build the pipeline tool", &mut pipeline)?;
    let xtask = std::env::current_exe().map_err(|e| format!("the xtask binary: {e}"))?;
    let mut files = vec![xtask, root().join("target/debug/pipeline")];
    crate::native::build()?;
    files.extend(native_files());
    // The Linux hello worlds and test programs of the boot groups.
    files.extend(crate::programs::prebuild()?);
    // The traced scenario's kit and driver, the 7-Zip scenario's driver and
    // the desktop's Windows test program.
    crate::sevenzip_trace::stage()?;
    files.extend(crate::sevenzip_trace::staged_files());
    files.extend(crate::sevenzip::stage_prebuild()?);
    files.extend(crate::desktop::prebuild_windows_app()?);
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_part_lists_the_kernel_and_every_native_program() {
        let native = native_files();
        assert_eq!(native.len(), crate::native::PROGRAMS.len());
        assert!(native.iter().all(|p| p.starts_with(root().join("target/native"))));
    }

    #[test]
    fn without_the_variable_nothing_is_skipped() {
        if !active() {
            assert_eq!(skip("x", &[PathBuf::from("/nonexistent")]), Ok(false));
        }
    }
}
