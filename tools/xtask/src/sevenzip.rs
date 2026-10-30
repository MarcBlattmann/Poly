//! 7-Zip's command line on Poly (roadmap Phase 5, "proof of concept b";
//! ADR 0087): the official x64 `7za.exe`, fetched into a cache, and a boot
//! test that runs it.
//!
//! 7-Zip is LGPL-2.1 software by Igor Pavlov. Like BusyBox (ADR 0051) it is
//! never committed or copied into the repository: it is a test program,
//! downloaded at the developer's request by `cargo xtask win fetch-7zip`
//! and kept under the main checkout's `target/` (shared by its worktrees).
//! Nothing of it is read to learn how Windows behaves (CLEAN_ROOM.md): it
//! is run on Poly, and its PE import table — names only — says what it
//! needs (`cargo xtask win imports 7zip`, `winimports.rs`).
//!
//! # Which binary, and how it is checked
//!
//! 7-zip.org's download page (2026-10-21) offers 7-Zip 26.05; its
//! standalone `7zr.exe` is an x86 (32-bit) program, which the NT
//! personality does not run, so the x64 console program is `x64/7za.exe`
//! from the "7-Zip Extra" package (`7z2605-extra.7z`). The page links both
//! to the project's GitHub releases (`ip7z/7zip`), fetched over HTTPS.
//! The site publishes no hashes; the SHA-1 and MD5 sums SourceForge's
//! `sevenzip` project lists for the same release matched the downloads
//! when [`FILES`] was pinned, and the SHA-256 sums here are of those
//! bytes. The Extra package is a `.7z` archive, which no host tool here
//! reads, so the same release's Linux console version (`7zz`, from
//! `7z2605-linux-x64.tar.xz`, also pinned) unpacks it; both archives are
//! deleted afterwards and only `7za.exe` (1.3 MB) stays.
//!
//! # The boot test
//!
//! `7zip` (shard `userland`): without `7za.exe` in the cache it is skipped
//! with the reason — the boot tests never touch the network. CI fetches it
//! in the job that runs that shard (`.github/workflows/ci.yml`). The test
//! puts `7za.exe` and a small text file in an initramfs, starts it as the
//! first program (`poly.init=/7zip/7za.exe`, its arguments `poly.init.arg=`
//! words) with ADR 0027's test-run records, and reports for each step how
//! far it got: the exit code and `NTSTATUS`, what it printed, and the
//! loader's or the program's last word. Which steps must pass is
//! [`REQUIRED`]; the others are reported only, until Poly runs them.
//!
//! # The scenario (ADR 0088)
//!
//! After those boots the test runs the scenario driver
//! (`tests/windows/sevenzip_scenario.rs`) on Poly: one boot in which the
//! driver runs fifteen 7-Zip steps itself and records each one, compared
//! step by step with the recording made on Windows
//! ([`SCENARIO_REFERENCE`]) by `pipeline win scenario diff`. It is
//! reported only ([`SCENARIO_REQUIRED`]) until Poly passes it.
//! `cargo xtask win 7zip-diff` runs the same comparison by itself.

use crate::symbols::Symbols;
use crate::{Result, root};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

/// The 7-Zip release.
pub const VERSION: &str = "26.05";

/// A pinned download.
struct Pinned {
    file: &'static str,
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

/// The two archives: the Extra package with `x64/7za.exe`, and the Linux
/// console version that unpacks it. (SourceForge's SHA-1 sums: `16554e53…`
/// and `b5b38abb…`.)
const FILES: [Pinned; 2] = [
    Pinned {
        file: "7z2605-extra.7z",
        url: "https://github.com/ip7z/7zip/releases/download/26.05/7z2605-extra.7z",
        size: 1_763_900,
        sha256: "f27a8f103c84785026edcba1f67faf1357225df9d19ddb0b7760d55947141bc7",
    },
    Pinned {
        file: "7z2605-linux-x64.tar.xz",
        url: "https://github.com/ip7z/7zip/releases/download/26.05/7z2605-linux-x64.tar.xz",
        size: 1_579_976,
        sha256: "22cf82fcc6eb7474f1d5bb4f02c0f8ebd573a718089d6833dc30f8350a3b232e",
    },
];

/// `x64/7za.exe` of the Extra package.
const EXE: &str = "7za.exe";
const EXE_IN_ARCHIVE: &str = "x64/7za.exe";
const EXE_SIZE: u64 = 1_334_784;
const EXE_SHA256: &str = "7ddf0a3bc451e166679296be630d6f0573eb386159c04184761c43020c484291";

/// `<main checkout>/target/7zip/<version>/`, beside the `ntdll` cache.
fn cache_dir() -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(&root());
    ntdll.parent().map_or_else(|| root().join("target"), Path::to_path_buf).join("7zip").join(VERSION)
}

/// The SHA-256 of a file (`sha256sum`), or `None`.
fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

/// Whether `path` is the pinned file (size first, then the hash).
fn matches(path: &Path, size: u64, sum: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == size) && sha256(path).as_deref() == Some(sum)
}

/// The cached `7za.exe`, if it is there and is the pinned file.
pub fn cached() -> Option<PathBuf> {
    let exe = cache_dir().join(EXE);
    matches(&exe, EXE_SIZE, EXE_SHA256).then_some(exe)
}

/// Download `p` into `dir` unless it is there; check it.
fn download(dir: &Path, p: &Pinned) -> Result<PathBuf> {
    let path = dir.join(p.file);
    if !matches(&path, p.size, p.sha256) {
        eprintln!("==> downloading {}", p.url);
        let partial = path.with_extension("part");
        let fetched = Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(p.url)
            .status();
        if !fetched.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("downloading {} failed", p.url));
        }
        std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    }
    if !matches(&path, p.size, p.sha256) {
        let got = sha256(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        return Err(format!("{} is not the pinned file (SHA-256 {}, got {got}); removed", p.file, p.sha256));
    }
    Ok(path)
}

/// `cargo xtask win fetch-7zip`: put the pinned `7za.exe` in the cache.
pub fn fetch() -> Result<PathBuf> {
    if let Some(exe) = cached()
        && cache_dir().join(SAMPLE_ARCHIVE).is_file()
    {
        eprintln!("==> {} is the pinned 7-Zip {VERSION} 7za.exe (x64)", exe.display());
        return Ok(exe);
    }
    for tool in ["curl", "sha256sum", "tar", "xz"] {
        if Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
            return Err(format!("fetching 7-Zip needs `{tool}`"));
        }
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let extra = download(&dir, &FILES[0])?;
    let linux = download(&dir, &FILES[1])?;
    // Each archive into a fresh, empty directory of its own.
    let tool_dir = dir.join("linux-7zz");
    let out_dir = dir.join("extra");
    for d in [&tool_dir, &out_dir] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let unpacked = Command::new("tar").arg("-xJf").arg(&linux).arg("-C").arg(&tool_dir).arg("7zz").status();
    if !unpacked.is_ok_and(|s| s.success()) {
        return Err(format!("could not unpack 7zz from {}", linux.display()));
    }
    let extracted = Command::new(tool_dir.join("7zz"))
        .arg("e")
        .arg(format!("-o{}", out_dir.display()))
        .arg("-y")
        .arg(&extra)
        .arg(EXE_IN_ARCHIVE)
        .stdout(Stdio::null())
        .status();
    if !extracted.is_ok_and(|s| s.success()) {
        return Err(format!("7zz could not extract {EXE_IN_ARCHIVE} from {}", extra.display()));
    }
    let got = out_dir.join(EXE);
    if !matches(&got, EXE_SIZE, EXE_SHA256) {
        return Err(format!("{EXE_IN_ARCHIVE} of {} is not the pinned file", extra.display()));
    }
    // A small archive for the boot test's extraction steps, made by the
    // same release's Linux version.
    std::fs::write(out_dir.join(SAMPLE_FILE), SAMPLE_TEXT).map_err(|e| format!("{SAMPLE_FILE}: {e}"))?;
    let made = Command::new(tool_dir.join("7zz"))
        .current_dir(&out_dir)
        .args(["a", "-bd", "-y", SAMPLE_ARCHIVE, SAMPLE_FILE])
        .stdout(Stdio::null())
        .status();
    if !made.is_ok_and(|s| s.success()) {
        return Err(format!("7zz could not make {SAMPLE_ARCHIVE}"));
    }
    std::fs::rename(out_dir.join(SAMPLE_ARCHIVE), dir.join(SAMPLE_ARCHIVE)).map_err(|e| e.to_string())?;
    let exe = dir.join(EXE);
    std::fs::rename(&got, &exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    for d in [&tool_dir, &out_dir] {
        let _ = std::fs::remove_dir_all(d);
    }
    for f in [&extra, &linux] {
        let _ = std::fs::remove_file(f);
    }
    eprintln!("==> 7-Zip {VERSION} 7za.exe (x64) cached as {}", exe.display());
    Ok(exe)
}

/// `cargo xtask win <fetch-7zip | imports ... | 7zip-diff ...>`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("fetch-7zip") => fetch().map(|_| ()),
        Some("imports") => crate::winimports::command(&args[1..]),
        Some("7zip-diff") => scenario_diff(&args[1..]),
        _ => Err("usage: cargo xtask win <fetch-7zip | imports <exe | 7zip> [--markdown <file>] [--check] | \
                  7zip-diff [--kernel <elf>] [--build-kernel] [--markdown <file>] [--target <poly.json>]>"
            .into()),
    }
}

// ----- the scenario (ADR 0088) -----------------------------------------------------------

/// The scenario driver (`tests/windows/<name>.rs`).
const SCENARIO_TEST: &str = "sevenzip_scenario";
/// Its recording on the oracle machine (`poly-oracle.ps1`, ADR 0075).
pub const SCENARIO_REFERENCE: &str = "tests/windows/oracle/sevenzip_scenario.windows.json";
/// Where the driver is built (apart from `win build`'s own directory, so
/// building it does not remove a developer's other test programs).
const SCENARIO_BIN: &str = "target/pipeline/windows/scenario-bin";
/// Its data files: `<dir>/<test>/7za.exe` (`pipeline`'s `scenario::DATA_DIR`).
const SCENARIO_DATA: &str = "target/pipeline/windows/data";
/// Whether a difference fails the `7zip` boot test. `false` until Poly
/// passes every step; then `true` makes the scenario required (ADR 0088).
const SCENARIO_REQUIRED: bool = false;

/// `cargo run --package poly-pipeline -- <args>`; its exit status.
fn pipeline(args: &[&str]) -> Result<std::process::ExitStatus> {
    crate::cargo()
        .current_dir(root())
        .args(["run", "--quiet", "--package", "poly-pipeline", "--"])
        .args(args)
        .status()
        .map_err(|e| format!("pipeline: {e}"))
}

/// Put `7za.exe` where the driver's data files go and build the driver.
fn stage_scenario(exe: &Path) -> Result {
    let data = root().join(SCENARIO_DATA).join(SCENARIO_TEST);
    std::fs::create_dir_all(&data).map_err(|e| format!("{}: {e}", data.display()))?;
    std::fs::copy(exe, data.join(EXE)).map_err(|e| format!("{}: {e}", data.display()))?;
    let built = pipeline(&["win", "build", "--tests", SCENARIO_TEST, "--out", SCENARIO_BIN])?;
    if !built.success() {
        return Err(format!("building {SCENARIO_TEST}.exe failed ({built})"));
    }
    Ok(())
}

/// `pipeline win scenario diff` against the recording, with `extra`
/// arguments; whether every step passed.
fn run_scenario_diff(extra: &[&str]) -> Result<bool> {
    let mut args = vec!["win", "scenario", "diff", "--reference", SCENARIO_REFERENCE, "--bin-dir", SCENARIO_BIN];
    args.extend_from_slice(extra);
    let status = pipeline(&args)?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!("pipeline win scenario diff failed ({status})")),
    }
}

/// `cargo xtask win 7zip-diff [pipeline options]`: build the driver, run
/// it on Poly (the default kernel, or `--kernel`, `--build-kernel`) and
/// compare it with the Windows recording step by step.
fn scenario_diff(args: &[String]) -> Result {
    let exe = cached().ok_or_else(|| {
        format!("7-Zip {VERSION}'s 7za.exe is not in the cache; `cargo xtask win fetch-7zip` downloads it")
    })?;
    stage_scenario(&exe)?;
    let given = |a: &str| args.iter().any(|x| x == a);
    // The kernel the boot tests run (our DLLs embedded), built unless one
    // is named or a result file stands in for Poly.
    let kernel = if given("--build-kernel") || !given("--kernel") && !given("--target") {
        Some(crate::build_kernel(false)?.to_string_lossy().into_owned())
    } else {
        None
    };
    let mut extra: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--build-kernel").collect();
    if let Some(k) = &kernel {
        extra.extend(["--kernel", k.as_str()]);
    }
    if run_scenario_diff(&extra)? {
        Ok(())
    } else {
        Err("the 7-Zip scenario on Poly differs from the Windows recording (report above)".into())
    }
}

/// The scenario on the boot test's kernel: reported, and failing the boot
/// test only when [`SCENARIO_REQUIRED`].
fn scenario_boot(kernel: &Path, exe: &Path) -> Result {
    eprintln!("==> 7-Zip scenario (ADR 0088): the driver runs every step in one boot, compared with Windows");
    let outcome = stage_scenario(exe).and_then(|()| run_scenario_diff(&["--kernel", &kernel.to_string_lossy()]));
    match outcome {
        Ok(true) => {
            eprintln!("==> 7-Zip scenario: every step matches the Windows recording");
            Ok(())
        }
        Ok(false) if SCENARIO_REQUIRED => Err("the 7-Zip scenario differs from the Windows recording".into()),
        Ok(false) => {
            eprintln!("==> 7-Zip scenario: differs from the Windows recording (report only, ADR 0088)");
            Ok(())
        }
        Err(e) if SCENARIO_REQUIRED => Err(format!("7-Zip scenario: {e}")),
        Err(e) => {
            eprintln!("==> 7-Zip scenario: could not run ({e}; report only, ADR 0088)");
            Ok(())
        }
    }
}

// ----- the boot test ---------------------------------------------------------------------

/// The text file the test archives, and the archive `fetch` makes of it.
const SAMPLE_FILE: &str = "hello.txt";
const SAMPLE_TEXT: &str = "Hello from 7-Zip on Poly.\r\n";
const SAMPLE_ARCHIVE: &str = "sample.7z";
/// Where the test puts everything in the initramfs: `Z:\7zip` to the
/// program, its own directory and so its current one (ADR 0067, 0079).
const DIR: &str = "7zip";

/// One run of `7za.exe`: what it is called with and what shows it worked.
struct Step {
    name: &'static str,
    args: &'static [&'static str],
    /// Text its output must contain.
    expect: &'static str,
}

const STEPS: [Step; 5] = [
    Step { name: "usage", args: &[], expect: "Usage: 7za" },
    Step { name: "info", args: &["i"], expect: "Formats:" },
    Step { name: "add", args: &["a", "-bd", "new.7z", SAMPLE_FILE], expect: "Everything is Ok" },
    Step { name: "test", args: &["t", "-bd", SAMPLE_ARCHIVE], expect: "Everything is Ok" },
    Step { name: "extract", args: &["x", "-bd", "-y", "-oout", SAMPLE_ARCHIVE], expect: "Everything is Ok" },
];

/// The steps that must pass (every one, since ADR 0087's slice); a step
/// added for something Poly cannot do yet is run and reported only until
/// it passes.
const REQUIRED: &[&str] = &["usage", "info", "add", "test", "extract"];

/// How far one step got.
#[derive(Debug, Default, PartialEq, Eq)]
struct Outcome {
    started: bool,
    /// `POLY-EXIT`'s `code=` and `ntstatus=`.
    code: Option<u32>,
    ntstatus: Option<u32>,
    /// The expected text was printed.
    printed: bool,
    /// What the program printed (the lines between `POLY-INIT` and
    /// `POLY-EXIT` that are not kernel log lines), and the kernel's notes
    /// about it (unimplemented calls, unhandled exceptions).
    output: Vec<String>,
    notes: Vec<String>,
}

impl Outcome {
    fn passed(&self) -> bool {
        self.started && self.code == Some(0) && self.printed
    }
}

/// A kernel log line (`subsystem: ...`) rather than program output.
fn is_kernel_line(l: &str) -> bool {
    let head = l.split(':').next().unwrap_or("");
    ["nt", "init", "fs", "syscall", "linux", "pci", "smp", "tty", "console"].contains(&head)
}

/// The console host draws the program's output as a screen (ADR 0080):
/// each cursor movement (`ESC [ row ; col H`) starts a new line here.
fn screen_lines(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    let mut rest = log;
    while let Some(at) = rest.find("\x1b[") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 2..];
        let len = tail.find(|c: char| !(c.is_ascii_digit() || c == ';')).unwrap_or(tail.len());
        if tail[len..].starts_with('H') {
            out.push('\n');
            rest = &tail[len + 1..];
        } else {
            out.push_str("\x1b[");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// Read one boot's output.
fn outcome(log: &str, expect: &str) -> Outcome {
    let mut o = Outcome::default();
    let mut inside = false;
    for line in screen_lines(log).lines() {
        let line = crate::console::strip_ansi(line);
        let line = line.trim_end_matches('\r');
        if line.contains("POLY-INIT: exec") {
            o.started = true;
            inside = true;
            continue;
        }
        if let Some(at) = line.find("POLY-EXIT:") {
            for field in line[at..].split_whitespace() {
                if let Some(c) = field.strip_prefix("code=") {
                    o.code = c.parse().ok();
                }
                if let Some(s) = field.strip_prefix("ntstatus=0x") {
                    o.ntstatus = u32::from_str_radix(s, 16).ok();
                }
            }
            let before = line[..at].trim();
            if inside && !before.is_empty() {
                o.output.push(before.to_string());
            }
            inside = false;
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("unimplemented") || lower.contains("not implemented") || lower.contains("unhandled exception")
        {
            o.notes.push(line.to_string());
        }
        if line.contains("POLY-INIT: unsupported") {
            o.notes.push(line.to_string());
        }
        if inside && !line.trim().is_empty() && !is_kernel_line(line) {
            o.output.push(line.to_string());
        }
    }
    o.printed = !expect.is_empty() && o.output.iter().any(|l| l.contains(expect));
    o
}

/// The boot test (see the module docs).
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    eprintln!("==> boot test: 7-Zip's command line (7za.exe, x64) runs on the NT personality (ADR 0087)");
    let Some(exe) = cached() else {
        eprintln!(
            "==> skipped: 7-Zip {VERSION}'s 7za.exe is not in the cache ({}); `cargo xtask win fetch-7zip` downloads it",
            cache_dir().display()
        );
        return Ok(());
    };
    let bytes = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let mut tree = crate::initramfs::Tree::default();
    tree.files.push((format!("{DIR}/7za.exe"), 0o755, bytes));
    tree.files.push((format!("{DIR}/{SAMPLE_FILE}"), 0o644, SAMPLE_TEXT.as_bytes().to_vec()));
    match std::fs::read(cache_dir().join(SAMPLE_ARCHIVE)) {
        Ok(a) => tree.files.push((format!("{DIR}/{SAMPLE_ARCHIVE}"), 0o644, a)),
        Err(_) => eprintln!("==> no {SAMPLE_ARCHIVE} in the cache (`cargo xtask win fetch-7zip` makes it)"),
    }
    let image = crate::initramfs::programs_image_with(&[], &tree);
    let image = crate::initramfs::write(&root(), "poly-7zip-initramfs.cpio", &image)?;
    let mut failed = Vec::new();
    for step in &STEPS {
        let mut append = format!("poly.qemu_exit poly.init=/{DIR}/7za.exe");
        for a in step.args {
            append.push_str(" poly.init.arg=");
            append.push_str(a);
        }
        let b = crate::boot_on(None, Some(&image), kernel, syms, crate::DEFAULT_MEMORY, &append, crate::Devices::None)?;
        let o = outcome(&b.output, step.expect);
        let verdict = if o.passed() { "passed" } else { "did not pass" };
        let status = o.ntstatus.map_or_else(|| format!("{:?}", o.code), |s| format!("{s:#010x}"));
        eprintln!(
            "==> 7za {} ({}): {verdict}: started {}, exit status {status}, printed {:?}: {}",
            step.args.join(" "),
            step.name,
            o.started,
            step.expect,
            o.printed
        );
        for l in o.output.iter().take(12) {
            eprintln!("      | {l}");
        }
        if o.output.len() > 12 {
            eprintln!("      | ... ({} lines)", o.output.len());
        }
        for n in o.notes.iter().take(5) {
            eprintln!("      ! {n}");
        }
        if b.status != Some(crate::QEMU_SUCCESS) && !b.timed_out {
            eprintln!("      ! QEMU ended with {:?}", b.status);
        }
        if b.timed_out {
            eprintln!("      ! the boot timed out");
        }
        if REQUIRED.contains(&step.name) && !o.passed() {
            failed.push(step.name);
        }
    }
    let scenario = scenario_boot(kernel, &exe);
    if !failed.is_empty() {
        return Err(format!("7za.exe steps that must pass did not: {}", failed.join(", ")));
    }
    scenario
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_boot() {
        let log = "fs: proc on /proc\nPOLY-INIT: exec /7zip/7za.exe\n\n7-Zip (a) 26.05 (x64)\nnt: pid 2 unhandled exception 0xe06d7363 at 0x1\nUsage: 7za <command>\nPOLY-EXIT: pid=2 code=0 ntstatus=0x0\n";
        let o = outcome(log, "Usage: 7za");
        assert!(o.passed(), "{o:?}");
        assert_eq!(o.output, ["7-Zip (a) 26.05 (x64)", "Usage: 7za <command>"]);
        assert_eq!(o.notes.len(), 1);
        let o = outcome("POLY-INIT: exec /x\nPOLY-EXIT: pid=2 code=53 ntstatus=0xc0000135\n", "Usage");
        assert_eq!((o.code, o.ntstatus, o.printed), (Some(53), Some(0xC000_0135), false));
        assert!(!o.passed());
        // Output on the same line as the record.
        let o = outcome("POLY-INIT: exec /x\nEverything is OkPOLY-EXIT: pid=2 code=0\n", "Everything is Ok");
        assert!(o.passed());
        assert!(!outcome("", "x").started);
        // The console host's screen: cursor movements are line breaks.
        let o = outcome(
            "POLY-INIT: exec /x\n\x1b[2J\x1b[2;1H7-Zip\x1b[3;1HUsage: 7za\x1b[1mx\nPOLY-EXIT: pid=2 code=0\n",
            "",
        );
        assert_eq!(o.output, ["7-Zip", "Usage: 7zax"]);
    }

    #[test]
    fn every_required_step_exists() {
        for r in REQUIRED {
            assert!(STEPS.iter().any(|s| s.name == *r), "{r}");
        }
        // Each step's command line fits Poly's (512 bytes).
        for s in &STEPS {
            let len: usize = s.args.iter().map(|a| a.len() + 15).sum();
            assert!(len + 64 < 512);
        }
    }
}
