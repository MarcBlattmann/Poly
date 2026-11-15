//! Git for Windows on the NT personality (ADR 0194): MinGit's `git.exe`, and
//! the MSYS2 runtime behind its `sh.exe` (git-bash's runtime).
//!
//! `cargo xtask win fetch-git` downloads the pinned MinGit zip into a cache
//! beside 7-Zip's (shared by worktrees, kept by CI by version), checks its size
//! and SHA-256, and unpacks it with the pure-Rust ZIP reader. The `git-win`
//! boot group (skipped without the cache) boots once and runs, one after
//! another: `sh.exe -c "echo hi && /usr/bin/ls /cmd"`, `git --version`, and
//! `git init`, `config`, `add`, `commit` and `log --stat` in a scratch
//! repository. MinGit is a third-party app run as a black box: downloaded at
//! build time, never committed (CLEAN_ROOM.md).

use crate::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const VERSION: &str = "2.56.1";
const URL: &str = "https://github.com/git-for-windows/git/releases/download/v2.56.1.windows.1/MinGit-2.56.1-64-bit.zip";
const FILE: &str = "MinGit-2.56.1-64-bit.zip";
const SIZE: u64 = 39_849_768;
const SHA256: &str = "4a92850b6e917da4fd010669b614b55dccf5b0ec333a534cc4be4700dfefdafd";

/// Where the zip and the unpacked MinGit are kept: `target/mingit/<version>`
/// beside the 7-Zip and Chrome caches.
pub fn cache_dir() -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(&crate::root());
    ntdll.parent().map_or_else(|| crate::root().join("target"), Path::to_path_buf).join("mingit").join(VERSION)
}

/// The unpacked MinGit (`cmd/git.exe`, `usr/bin/sh.exe`, ...).
pub fn app_dir() -> PathBuf {
    cache_dir().join("MinGit")
}

fn matches(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == SIZE) && crate::sha256::file(path).as_deref() == Some(SHA256)
}

/// Whether MinGit is fetched and unpacked.
pub fn fetched() -> bool {
    cache_dir().join(".unpacked").is_file() && app_dir().join("cmd/git.exe").is_file()
}

/// Whether a member name of the zip is safe to write below the output directory.
fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.split('/').all(|c| !c.is_empty() && c != ".." && c != "." && !c.contains([':', '\\']))
}

/// `cargo xtask win fetch-git`: the pinned zip (checked) and its contents.
pub fn fetch() -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if fetched() {
        eprintln!("==> {} is the pinned MinGit {VERSION}", app_dir().display());
        return Ok(app_dir());
    }
    let zip = dir.join(FILE);
    if !matches(&zip) {
        eprintln!("==> downloading {URL}");
        let partial = dir.join(format!("{FILE}.part"));
        let got = Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(URL)
            .status();
        if !got.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("downloading {URL} failed"));
        }
        std::fs::rename(&partial, &zip).map_err(|e| e.to_string())?;
    }
    if !matches(&zip) {
        let got = crate::sha256::file(&zip).unwrap_or_default();
        let _ = std::fs::remove_file(&zip);
        return Err(format!("{FILE} is not the pinned file (SHA-256 {SHA256}, got {got}); removed"));
    }
    let bytes = std::fs::read(&zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let out = app_dir();
    let _ = std::fs::remove_dir_all(&out);
    let mut count = 0;
    crate::unzip::for_each(&bytes, |name, data| {
        if name.ends_with('/') {
            return Ok(());
        }
        if !safe_name(name) {
            return Err(format!("{name}: an unsafe name"));
        }
        let path = out.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        count += 1;
        Ok(())
    })?;
    std::fs::write(dir.join(".unpacked"), format!("{count} files\n")).map_err(|e| e.to_string())?;
    eprintln!("==> {count} files in {}", out.display());
    Ok(out)
}

/// The runs of the boot test, in order: the program (relative to MinGit's
/// directory), its arguments, and lines its standard output must have.
const RUNS: &[(&str, &str, &[&str])] = &[
    ("usr/bin/sh.exe", r#"-c "echo hi && /usr/bin/ls /cmd""#, &["hi", "git.exe"]),
    ("cmd/git.exe", "--version", &["git version 2.56.1.windows.1"]),
    ("cmd/git.exe", "init", &["Initialized empty Git repository in C:/apps/git/.git/"]),
    ("cmd/git.exe", "config user.name Poly", &[]),
    ("cmd/git.exe", "config user.email poly@example.invalid", &[]),
    ("cmd/git.exe", "add LICENSE.txt", &[]),
    ("cmd/git.exe", "commit -m first", &[" 1 file changed, 360 insertions(+)", " create mode 100644 LICENSE.txt"]),
    ("cmd/git.exe", "log --stat", &["Author: Poly <poly@example.invalid>", "    first", " LICENSE.txt | 360 +"]),
];

/// What each run of a serial log printed: `(standard output lines, LAUNCH-EXIT line)`.
fn runs_of(text: &str) -> Vec<(Vec<String>, String)> {
    let mut out = Vec::new();
    let (mut lines, mut inside) = (Vec::new(), false);
    for l in text.lines() {
        if l.contains("LAUNCH-END stdout") {
            inside = false;
        }
        if inside && !l.trim().is_empty() {
            lines.push(l.trim_end().to_string());
        }
        if l.contains("LAUNCH-BEGIN stdout") {
            inside = true;
        }
        if let Some(i) = l.find("LAUNCH-EXIT") {
            out.push((std::mem::take(&mut lines), l[i..].trim().to_string()));
        }
    }
    out
}

/// What is wrong with the runs of a log, if anything.
fn check(text: &str) -> Option<String> {
    let got = runs_of(text);
    for (i, (exe, args, want)) in RUNS.iter().enumerate() {
        let Some((lines, exit)) = got.get(i) else {
            return Some(format!("run {} ({exe} {args}) did not end", i + 1));
        };
        if exit != "LAUNCH-EXIT code=0x0" {
            return Some(format!("run {} ({exe} {args}): {exit}", i + 1));
        }
        for w in *want {
            if !lines.iter().any(|l| l.starts_with(w)) {
                return Some(format!("run {} ({exe} {args}): no line {w:?} in {lines:?}", i + 1));
            }
        }
    }
    None
}

/// The `git-win` boot group.
pub fn boot_test(kernel: &Path, _syms: &std::sync::Arc<crate::symbols::Symbols>) -> Result {
    eprintln!(
        "==> boot test: Git for Windows {VERSION} (MinGit) and its MSYS2 sh.exe on the NT personality (ADR 0194)"
    );
    if !fetched() {
        eprintln!(
            "==> skipped: MinGit {VERSION} is not in the cache ({}); `cargo xtask win fetch-git` downloads it",
            cache_dir().display()
        );
        return Ok(());
    }
    let runs: Vec<(&str, &str)> = RUNS.iter().map(|&(e, a, _)| (e, a)).collect();
    let log = crate::root().join("target/git-win.log");
    let text = crate::winapps::run_sequence(kernel, &app_dir(), "git", "C:\\apps\\git", &runs, 60, &log)?;
    match check(&text) {
        None => {
            eprintln!("==> sh.exe and git (init, add, commit, log) ran as expected");
            Ok(())
        }
        Some(e) => Err(format!("git-win: {e} (log {})", log.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_outside_the_directory_are_refused() {
        assert!(safe_name("cmd/git.exe"));
        assert!(!safe_name("../x"));
        assert!(!safe_name("a//b"));
        assert!(!safe_name("C:/x"));
        assert!(!safe_name("a\\b"));
    }

    #[test]
    fn each_run_has_its_output_and_exit() {
        let log = "LAUNCH-START a\nLAUNCH-BEGIN stdout\nhi\ngit.exe\nLAUNCH-END stdout\nLAUNCH-EXIT code=0x0\n\
                   LAUNCH-START b\nLAUNCH-BEGIN stdout\nLAUNCH-END stdout\nx LAUNCH-EXIT code=0x80\n";
        let r = runs_of(log);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], (vec!["hi".to_string(), "git.exe".to_string()], "LAUNCH-EXIT code=0x0".to_string()));
        assert_eq!(r[1].1, "LAUNCH-EXIT code=0x80");
        assert_eq!(check(log), Some("run 2 (cmd/git.exe --version): LAUNCH-EXIT code=0x80".into()));
    }
}
