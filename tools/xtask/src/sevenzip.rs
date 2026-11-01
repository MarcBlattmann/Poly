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
//! `7zip` (shard `userland`): without `7za.exe` (or the Linux `7zz`) in the
//! cache it is skipped with the reason — the boot tests never touch the
//! network. CI fetches both in the job that runs that shard
//! (`.github/workflows/ci.yml`). The test puts `7za.exe` and its inputs in
//! an initramfs ([`fixtures`]: text and random files of up to 3 MiB, a tree
//! of small files, 3,000 files in one directory, Unicode names, a path past
//! `MAX_PATH`, a read-only file, a symbolic link, and archives of every
//! format `7za` reads made by the same release's Linux `7zz`), boots once
//! per step of [`STEPS`] (`poly.init=/7zip/7za.exe`, its arguments
//! `poly.init.arg=` words) with ADR 0027's test-run records, and reports
//! for each step how far it got: the exit code and `NTSTATUS`, what it
//! printed, the loader's or the program's last word, and how long the
//! program ran. Every step must pass but those of [`NOT_YET`], which are
//! reported only, until Poly runs them (ADR 0091). `POLY_7ZIP_STEPS=a,b`
//! runs only the steps named.
//!
//! # The scenario (ADR 0088)
//!
//! After those boots the test runs the scenario driver
//! (`tests/windows/sevenzip_scenario.rs`) on Poly: one boot in which the
//! driver runs fifteen 7-Zip steps itself and records each one, compared
//! step by step with the recording made on Windows
//! ([`SCENARIO_REFERENCE`]) by `pipeline win scenario diff`. Poly passes
//! every step, so a difference fails the test ([`SCENARIO_REQUIRED`], ADR 0101).
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

/// The same release's Linux `7zz`, kept in the cache: it unpacks the Extra
/// package and makes the boot test's archives on the host ([`fixtures`]).
fn tool_path() -> PathBuf {
    cache_dir().join("linux").join("7zz")
}

/// `cargo xtask win fetch-7zip`: put the pinned `7za.exe` and the Linux
/// `7zz` in the cache.
pub fn fetch() -> Result<PathBuf> {
    let have_exe = cached().is_some() && cache_dir().join(SAMPLE_ARCHIVE).is_file();
    if have_exe && tool_path().is_file() {
        eprintln!("==> {} is the pinned 7-Zip {VERSION} 7za.exe (x64)", cache_dir().join(EXE).display());
        return Ok(cache_dir().join(EXE));
    }
    for tool in ["curl", "sha256sum", "tar", "xz"] {
        if Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
            return Err(format!("fetching 7-Zip needs `{tool}`"));
        }
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let linux = download(&dir, &FILES[1])?;
    // Each archive into a fresh, empty directory of its own.
    let tool_dir = dir.join("linux");
    let _ = std::fs::remove_dir_all(&tool_dir);
    std::fs::create_dir_all(&tool_dir).map_err(|e| format!("{}: {e}", tool_dir.display()))?;
    let unpacked = Command::new("tar").arg("-xJf").arg(&linux).arg("-C").arg(&tool_dir).arg("7zz").status();
    let _ = std::fs::remove_file(&linux);
    if !unpacked.is_ok_and(|s| s.success()) {
        return Err(format!("could not unpack 7zz from {}", linux.display()));
    }
    let exe = dir.join(EXE);
    if have_exe {
        eprintln!("==> 7-Zip {VERSION}'s Linux 7zz cached as {}", tool_path().display());
        return Ok(exe);
    }
    let extra = download(&dir, &FILES[0])?;
    let out_dir = dir.join("extra");
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let extracted = Command::new(tool_path())
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
    let made = Command::new(tool_path())
        .current_dir(&out_dir)
        .args(["a", "-bd", "-y", SAMPLE_ARCHIVE, SAMPLE_FILE])
        .stdout(Stdio::null())
        .status();
    if !made.is_ok_and(|s| s.success()) {
        return Err(format!("7zz could not make {SAMPLE_ARCHIVE}"));
    }
    std::fs::rename(out_dir.join(SAMPLE_ARCHIVE), dir.join(SAMPLE_ARCHIVE)).map_err(|e| e.to_string())?;
    std::fs::rename(&got, &exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let _ = std::fs::remove_dir_all(&out_dir);
    let _ = std::fs::remove_file(&extra);
    eprintln!("==> 7-Zip {VERSION} 7za.exe (x64) cached as {}", exe.display());
    Ok(exe)
}

/// `cargo xtask win <fetch-7zip | imports ... | 7zip-diff ...>`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("fetch-7zip") => fetch().map(|_| ()),
        Some("imports") => crate::winimports::command(&args[1..]),
        Some("7zip-diff") => scenario_diff(&args[1..]),
        // The Windows API tracer (wintrace.rs, ADR 0090).
        Some("trace-kit") => crate::wintrace::trace_kit_command(&args[1..]),
        Some("trace-diff") => crate::wintrace::trace_diff_command(&args[1..]),
        // The traced scenario (sevenzip_trace.rs, ADR 0110).
        Some("7zip-trace") => crate::sevenzip_trace::command(&args[1..]),
        _ => Err("usage: cargo xtask win <fetch-7zip | imports <exe | 7zip> [--markdown <file>] [--check] | \
                  7zip-diff [--kernel <elf>] [--build-kernel] [--markdown <file>] [--target <poly.json>] | \
                  trace-kit <exe | 7zip> [--out <dir>] | trace-diff <a.jsonl> <b.jsonl> | \
                  7zip-trace <stage | record <windows.json>... | diff [--kernel <elf>] [--markdown <file>] [--check]>>"
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
/// Whether a difference fails the `7zip` boot test: `true` since Poly
/// passes every step (ADR 0101; ADR 0088 made it report-only until then).
const SCENARIO_REQUIRED: bool = true;

/// `cargo run --package poly-pipeline -- <args>`; its exit status.
pub(crate) fn pipeline(args: &[&str]) -> Result<std::process::ExitStatus> {
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
    /// The exit code it must end with: 7-Zip's documented codes are 0 (no
    /// error), 1 (warning), 2 (fatal error), 7 (command line error), 8 (not
    /// enough memory) and 255 (stopped by the user).
    code: u32,
    /// Virtual CPUs for the boot (0: QEMU's default for the boot tests).
    cpus: u32,
    /// Seconds the boot may take (0: the boot tests' timeout).
    secs: u64,
}

/// What 7-Zip prints when a command did all it was asked.
const OK: &str = "Everything is Ok";

const fn step(name: &'static str, args: &'static [&'static str], expect: &'static str) -> Step {
    Step { name, args, expect, code: 0, cpus: 0, secs: 0 }
}

/// A step that must end with [`OK`].
const fn ok(name: &'static str, args: &'static [&'static str]) -> Step {
    step(name, args, OK)
}

impl Step {
    const fn code(self, code: u32) -> Step {
        Step { code, ..self }
    }
    const fn cpus(self, cpus: u32) -> Step {
        Step { cpus, ..self }
    }
    const fn secs(self, secs: u64) -> Step {
        Step { secs, ..self }
    }
}

/// Every step, each its own boot of the same initramfs ([`fixtures`] says
/// what is in it): nothing a step writes is there for the next one.
const STEPS: &[Step] = &[
    // ADR 0087's slice.
    step("usage", &[], "Usage: 7za"),
    step("info", &["i"], "Formats:"),
    ok("add", &["a", "-bd", "new.7z", SAMPLE_FILE]),
    ok("test", &["t", "-bd", SAMPLE_ARCHIVE]),
    ok("extract", &["x", "-bd", "-y", "-oout", SAMPLE_ARCHIVE]),
    // Listings: names, sizes, times and attributes.
    step("list", &["l", SAMPLE_ARCHIVE], SAMPLE_FILE),
    step("list-slt", &["l", "-slt", "tree.7z"], "Modified = "),
    step("list-zip", &["l", "f.zip"], "mid.txt"),
    step("list-attr", &["l", "-slt", "ro.7z"], "Attributes = "),
    // The 7z methods: archives the Linux 7zz made, tested, and new ones.
    ok("t-lzma", &["t", "-bd", "m-lzma.7z"]),
    ok("t-lzma2", &["t", "-bd", "m-lzma2.7z"]),
    ok("t-ppmd", &["t", "-bd", "m-ppmd.7z"]),
    ok("t-bzip2", &["t", "-bd", "m-bzip2.7z"]),
    ok("t-deflate", &["t", "-bd", "m-deflate.7z"]),
    ok("t-deflate64", &["t", "-bd", "m-deflate64.7z"]),
    ok("t-copy", &["t", "-bd", "m-copy.7z"]),
    ok("a-lzma", &["a", "-bd", "-m0=LZMA", "c-lzma.7z", "mid.txt"]),
    ok("a-lzma2", &["a", "-bd", "-m0=LZMA2", "c-lzma2.7z", "mid.txt"]),
    ok("a-ppmd", &["a", "-bd", "-m0=PPMd", "c-ppmd.7z", "mid.txt"]),
    ok("a-bzip2", &["a", "-bd", "-m0=BZip2", "c-bzip2.7z", "mid.txt"]),
    ok("a-deflate", &["a", "-bd", "-m0=Deflate", "c-deflate.7z", "mid.txt"]),
    ok("a-copy", &["a", "-bd", "-m0=Copy", "c-copy.7z", "mid.txt"]),
    // Other archive formats.
    ok("x-zip", &["x", "-bd", "-y", "-ozip", "f.zip"]),
    ok("x-tar", &["x", "-bd", "-y", "-otar", "f.tar"]),
    ok("x-gzip", &["x", "-bd", "-y", "-ogz", "mid.txt.gz"]),
    ok("x-bzip2", &["x", "-bd", "-y", "-obz2", "mid.txt.bz2"]),
    ok("x-xz", &["x", "-bd", "-y", "-oxz", "mid.txt.xz"]),
    ok("a-zip", &["a", "-bd", "-tzip", "c.zip", "mid.txt", "tree"]),
    ok("a-tar", &["a", "-bd", "-ttar", "c.tar", "tree"]),
    ok("a-gzip", &["a", "-bd", "-tgzip", "c.gz", "mid.txt"]),
    ok("a-bzip2f", &["a", "-bd", "-tbzip2", "c.bz2", "mid.txt"]),
    ok("a-xz", &["a", "-bd", "-txz", "c.xz", "mid.txt"]),
    // Changing an archive, and extracting without paths.
    ok("update", &["u", "-bd", "upd.7z", SAMPLE_FILE, "ro.txt"]),
    ok("delete", &["d", "-bd", "upd.7z", "mid.txt"]),
    ok("rename", &["rn", "-bd", "upd.7z", SAMPLE_FILE, "renamed.txt"]),
    ok("e-flat", &["e", "-bd", "-y", "-oflat", "tree.7z"]),
    ok("add-to", &["a", "-bd", "upd.7z", "tree"]),
    ok("zip-delete", &["d", "-bd", "f.zip", "tree/logs/l0.log"]),
    ok("zip-rename", &["rn", "-bd", "f.zip", "mid.txt", "m2.txt"]),
    ok("a-sdel", &["a", "-bd", "-sdel", "del.7z", "mid.txt"]),
    ok("a-times", &["a", "-bd", "-mtc=on", "-mta=on", "-stl", "tm.7z", SAMPLE_FILE]),
    ok("a-zip-methods", &["a", "-bd", "-tzip", "-mm=BZip2", "zb.zip", "mid.txt"]),
    ok("a-exe-bcj2", &["a", "-bd", "-mf=BCJ2", "exe.7z", "7za.exe"]).secs(300),
    // Extracting over files that exist: overwrite, skip, rename, and a
    // read-only one.
    ok("x-overwrite", &["x", "-bd", "-aoa", SAMPLE_ARCHIVE]),
    ok("x-skip", &["x", "-bd", "-aos", SAMPLE_ARCHIVE]),
    ok("x-rename", &["x", "-bd", "-aou", SAMPLE_ARCHIVE]),
    ok("x-over-ro", &["x", "-bd", "-y", "ro.7z"]),
    // Directories, recursion, wildcards, exclusions; many small files.
    ok("a-dir", &["a", "-bd", "d.7z", "tree"]),
    ok("a-r-wild", &["a", "-bd", "-r", "w.7z", "f*.txt"]),
    ok("a-exclude", &["a", "-bd", "-r", "x.7z", "tree", "-x!*.log"]),
    ok("x-dir", &["x", "-bd", "-y", "-otreeout", "tree.7z"]),
    ok("a-many", &["a", "-bd", "mn.7z", "many"]).secs(300),
    ok("x-many", &["x", "-bd", "-y", "-omanyout", "many.7z"]).secs(300),
    step("l-many", &["l", "many.7z"], "3000 files").secs(300),
    // A multi-megabyte file.
    ok("a-big-mx1", &["a", "-bd", "-mx1", "-mmt1", "b1.7z", "big.txt"]).secs(300),
    ok("a-big", &["a", "-bd", "-mmt1", "b5.7z", "big.txt"]).secs(600),
    ok("x-big", &["x", "-bd", "-y", "-obig", "big.7z"]).secs(300),
    // Threads.
    ok("a-mmt2", &["a", "-bd", "-mmt2", "-mx1", "mt2.7z", "big.txt"]).cpus(2).secs(300),
    ok("a-mmt4", &["a", "-bd", "-mmt4", "-mx1", "mt4.7z", "big.txt"]).cpus(4).secs(300),
    ok("a-lzma2-mt4", &["a", "-bd", "-m0=LZMA2:d=256k", "-mmt4", "l2mt.7z", "big.txt"]).cpus(4).secs(300),
    ok("a-bzip2-mt4", &["a", "-bd", "-tbzip2", "-mmt4", "mt.bz2", "big.txt"]).cpus(4).secs(300),
    ok("x-mt4", &["x", "-bd", "-y", "-mmt4", "-omt", "big.7z"]).cpus(4).secs(300),
    step("bench", &["b", "-mmt1", "-md18"], "Tot:").secs(600),
    // Passwords and AES.
    ok("a-pw", &["a", "-bd", "-pSecret", "-mhe=on", "p.7z", SAMPLE_FILE]),
    ok("x-pw", &["x", "-bd", "-y", "-pSecret", "-opw", "pw.7z"]),
    step("l-pw", &["l", "-pSecret", "pw.7z"], "mid.txt"),
    ok("a-zip-aes", &["a", "-bd", "-tzip", "-pSecret", "-mem=AES256", "p.zip", SAMPLE_FILE]),
    ok("x-zip-aes", &["x", "-bd", "-y", "-pSecret", "-opz", "pw.zip"]),
    step("x-pw-wrong", &["x", "-bd", "-y", "-pWrong", "-opw", "pw.7z"], "Wrong password").code(2),
    // Volumes.
    ok("a-vol", &["a", "-bd", "-v100k", "v.7z", "rand.bin"]),
    ok("x-vol", &["x", "-bd", "-y", "-ovol", "vol.7z.001"]),
    // Standard output.
    step("x-so", &["x", "-so", SAMPLE_ARCHIVE], "Hello from 7-Zip on Poly."),
    // Unicode names, long paths, attributes.
    ok("a-uni", &["a", "-bd", "u.7z", "uni"]),
    ok("x-uni", &["x", "-bd", "-y", "-ouniout", "uni.7z"]),
    // 7-Zip prints names in the console's OEM code page (Poly's: 437, which
    // has `ü` and `ß`); with `-sccUTF-8` it writes UTF-8 bytes, which a
    // console on 437 shows as other characters, on Windows as here.
    step("l-uni", &["l", "uni.7z"], "gr\u{fc}\u{df}e.txt"),
    step("l-uni-utf8", &["l", "-sccUTF-8", "uni.7z"], "with space.txt"),
    ok("a-long", &["a", "-bd", "lp.7z", "long"]),
    ok("x-long", &["x", "-bd", "-y", "-olongout", "long.7z"]),
    ok("a-ro", &["a", "-bd", "r.7z", "ro.txt"]),
    ok("x-ro", &["x", "-bd", "-y", "-oroout", "ro.7z"]),
    // Hashes.
    step("hash", &["h", SAMPLE_FILE], "CRC32"),
    step("hash-sha256", &["h", "-scrcSHA256", SAMPLE_FILE], "SHA256"),
    step("hash-all", &["h", "-scrc*", "mid.txt"], "SHA256"),
    step("hash-dir", &["h", "tree"], OK),
    // Character sets of the console and of list files; the progress line.
    ok("a-scc", &["a", "-bd", "-sccUTF-8", "c8.7z", SAMPLE_FILE]),
    ok("a-scs", &["a", "-bd", "-scsUTF-8", "ls.7z", "@list.txt"]),
    ok("add-progress", &["a", "pr.7z", "mid.txt"]),
    // NTFS features: alternate streams, security descriptors, symbolic
    // and hard links.
    ok("a-streams", &["a", "-bd", "-sns", "s.7z", SAMPLE_FILE]),
    ok("a-security", &["a", "-bd", "-sni", "sec.7z", SAMPLE_FILE]),
    ok("a-symlink", &["a", "-bd", "-snl", "sl.7z", "sl"]),
    ok("a-symlink-follow", &["a", "-bd", "slf.7z", "sl"]),
    ok("x-symlink", &["x", "-bd", "-y", "-snl", "-oslout", "sl.tar"]),
    ok("x-hardlink", &["x", "-bd", "-y", "-snh", "-ohlout", "hl.tar"]),
    // Errors.
    step("err-missing", &["x", "-bd", "-y", "missing.7z"], "ERROR").code(2),
    step("err-corrupt", &["t", "-bd", "corrupt.7z"], "ERROR").code(2),
    step("err-truncated", &["t", "-bd", "trunc.7z"], "ERROR").code(2),
    step("err-not-archive", &["t", "-bd", SAMPLE_FILE], "Cannot open the file as archive").code(2),
    step("err-add-missing", &["a", "-bd", "n.7z", "nosuch.txt"], "WARNING").code(1),
    step("err-command", &["q"], "ERROR").code(7),
];

/// The steps that are run and reported only, with what stops them: every
/// other step must pass (ADR 0091). 7-Zip reports these errors by throwing
/// a C++ exception (code `0xE06D7363`), which ends the process until Poly
/// has C++ exception handling (`__CxxFrameHandler`, ADR 0087), so their
/// message and exit code are not seen yet.
const NOT_YET: &[(&str, &str)] = &[
    ("a-streams", "C++ exception after `Creating archive` (what 7-Zip throws is not seen yet)"),
    ("a-security", "C++ exception after GetFileSecurityW's ERROR_NOT_SUPPORTED warnings"),
    ("err-missing", "C++ exception: 7-Zip's error for a missing archive"),
    ("err-command", "C++ exception: 7-Zip's command line error"),
];

/// Whether step `name` must pass.
fn required(name: &str) -> bool {
    !NOT_YET.iter().any(|(n, _)| *n == name)
}

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
    /// It started, ended with `code` and printed the expected text.
    fn passed(&self, code: u32) -> bool {
        self.started && self.code == Some(code) && self.printed
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

// ----- the files the steps work on ---------------------------------------------------------

/// A small deterministic generator (xorshift64*): the test's inputs are the
/// same bytes on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// `len` bytes of text: words in lines of about 70 characters (CRLF), which
/// compress like prose.
fn text(seed: u64, len: usize) -> Vec<u8> {
    const WORDS: [&str; 32] = [
        "poly",
        "kernel",
        "seven",
        "zip",
        "archive",
        "the",
        "of",
        "and",
        "a",
        "to",
        "in",
        "is",
        "file",
        "data",
        "stream",
        "block",
        "window",
        "thread",
        "volume",
        "header",
        "method",
        "dictionary",
        "match",
        "literal",
        "length",
        "offset",
        "range",
        "coder",
        "table",
        "entry",
        "name",
        "time",
    ];
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut out = Vec::with_capacity(len + 16);
    let mut line = 0;
    while out.len() < len {
        let w = WORDS[rng.below(WORDS.len() as u64) as usize];
        out.extend_from_slice(w.as_bytes());
        line += w.len() + 1;
        if line > 70 {
            out.extend_from_slice(b"\r\n");
            line = 0;
        } else {
            out.push(b' ');
        }
    }
    out.truncate(len);
    out
}

/// `len` bytes that do not compress.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut rng = Rng(seed | 1);
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        out.extend_from_slice(&rng.next().to_le_bytes());
    }
    out.truncate(len);
    out
}

/// How many files `many/` holds (`l-many` expects the count).
const MANY: usize = 3000;

/// A directory name of 40 characters for the long path (`n` in its name).
fn long_component(n: usize) -> String {
    format!("long-directory-name-number-{n}-abcdefghijkl")
}

/// The files the test makes itself, relative to [`DIR`], with their modes.
fn sources() -> Vec<(String, u16, Vec<u8>)> {
    let mut f: Vec<(String, u16, Vec<u8>)> = vec![
        (SAMPLE_FILE.into(), 0o644, SAMPLE_TEXT.as_bytes().to_vec()),
        ("mid.txt".into(), 0o644, text(1, 64 * 1024)),
        ("big.txt".into(), 0o644, text(2, 3 * 1024 * 1024)),
        ("rand.bin".into(), 0o644, noise(3, 300 * 1024)),
        ("ro.txt".into(), 0o444, b"read-only\r\n".to_vec()),
        // What the symbolic link `sl/to-target.txt` ([`SYMLINKS`]) names.
        ("sl/target.txt".into(), 0o644, b"the target\r\n".to_vec()),
        // A list file in UTF-8 (`-scsUTF-8 @list.txt`).
        ("list.txt".into(), 0o644, "hello.txt\r\nuni/gr\u{fc}\u{df}e.txt\r\n".as_bytes().to_vec()),
    ];
    // Many small files in a few directories, and some to exclude.
    for d in 0..9u64 {
        for i in 0..25u64 {
            let len = 100 + ((d * 25 + i) * 37 % 2900) as usize;
            f.push((format!("tree/d{d}/f{i:02}.txt"), 0o644, text(100 + d * 25 + i, len)));
        }
    }
    for i in 0..5u64 {
        f.push((format!("tree/logs/l{i}.log"), 0o644, text(1000 + i, 500)));
    }
    // Thousands of tiny files in one directory (directory listing at scale).
    for i in 0..MANY {
        f.push((format!("many/n{i:04}.txt"), 0o644, format!("{i}\r\n").into_bytes()));
    }
    // Names beyond ASCII: Latin, CJK, Greek, a character outside the BMP
    // (a surrogate pair in UTF-16) and a space.
    for name in [
        "gr\u{fc}\u{df}e.txt",
        "\u{65e5}\u{672c}\u{8a9e}.txt",
        "\u{395}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac}.txt",
        "emoji-\u{1f600}.txt",
        "with space.txt",
    ] {
        f.push((format!("uni/{name}"), 0o644, format!("{name}\r\n").into_bytes()));
    }
    // A path longer than MAX_PATH (260) below the current directory.
    let deep: Vec<String> = (0..7).map(long_component).collect();
    f.push((format!("long/{}/file.txt", deep.join("/")), 0o644, b"deep\r\n".to_vec()));
    f
}

/// The archives the Linux `7zz` makes of [`sources`] (its own arguments,
/// run in the directory that holds them).
const MADE_BY_7ZZ: &[&[&str]] = &[
    &["a", "-m0=LZMA", "m-lzma.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-m0=LZMA2", "m-lzma2.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-m0=PPMd", "m-ppmd.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-m0=BZip2", "m-bzip2.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-m0=Deflate", "m-deflate.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-m0=Deflate64", "m-deflate64.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-m0=Copy", "m-copy.7z", "mid.txt", SAMPLE_FILE],
    &["a", "-tzip", "f.zip", "mid.txt", "tree"],
    &["a", "-ttar", "f.tar", "tree"],
    &["a", "-tgzip", "mid.txt.gz", "mid.txt"],
    &["a", "-tbzip2", "mid.txt.bz2", "mid.txt"],
    &["a", "-txz", "mid.txt.xz", "mid.txt"],
    &["a", "tree.7z", "tree"],
    &["a", "many.7z", "many"],
    &["a", "-mx1", "big.7z", "big.txt"],
    &["a", "-pSecret", "-mhe=on", "pw.7z", SAMPLE_FILE, "mid.txt"],
    &["a", "-tzip", "-pSecret", "-mem=AES256", "pw.zip", SAMPLE_FILE, "mid.txt"],
    &["a", "-v100k", "-mx0", "vol.7z", "rand.bin"],
    &["a", "upd.7z", SAMPLE_FILE, "mid.txt"],
    &["a", "uni.7z", "uni"],
    &["a", "long.7z", "long"],
    &["a", "ro.7z", "ro.txt"],
    &["a", "-ttar", "-snh", "hl.tar", "hl"],
    &["a", "-ttar", "-snl", "sl.tar", "sl"],
];

/// Symbolic links of the test's directory (in the initramfs, and for
/// `sl.tar`): a relative link to a file beside it.
const SYMLINKS: &[(&str, &str)] = &[("sl/to-target.txt", "target.txt")];

/// Every file of the test's directory, relative to [`DIR`]: [`sources`],
/// the archives of [`MADE_BY_7ZZ`] (made in `work`, which is emptied
/// first) and two damaged copies of one (`corrupt.7z`, `trunc.7z`). An
/// archive `7zz` could not make is left out with a warning (its steps
/// then fail).
fn fixtures(tool: &Path, work: &Path) -> Result<Vec<(String, u16, Vec<u8>)>> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    // Every file and directory gets the same time (2024-01-02 03:04:05 UTC),
    // so the archives store it.
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_704_164_645);
    let mut files = sources();
    let mut dirs = std::collections::BTreeSet::new();
    for (rel, mode, bytes) in &files {
        let path = work.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let mut d = Path::new(rel).parent();
            while let Some(dir) = d.filter(|d| !d.as_os_str().is_empty()) {
                dirs.insert(work.join(dir));
                d = dir.parent();
            }
        }
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = std::fs::File::options().write(true).open(&path).map_err(|e| e.to_string())?;
        file.set_modified(when).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(u32::from(*mode)))
            .map_err(|e| e.to_string())?;
    }
    for d in dirs.iter().rev() {
        if let Ok(dir) = std::fs::File::open(d) {
            let _ = dir.set_modified(when);
        }
    }
    // Two names of one file and a symbolic link, only for 7zz's `hl.tar`
    // and `sl.tar` (the initramfs gets the link itself: [`SYMLINKS`]).
    let hl = work.join("hl");
    std::fs::create_dir_all(&hl).map_err(|e| format!("{}: {e}", hl.display()))?;
    std::fs::write(hl.join("a.txt"), b"one file, two names\r\n").map_err(|e| e.to_string())?;
    std::fs::hard_link(hl.join("a.txt"), hl.join("b.txt")).map_err(|e| format!("hard link: {e}"))?;
    for (rel, target) in SYMLINKS {
        let path = work.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        std::os::unix::fs::symlink(target, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    for args in MADE_BY_7ZZ {
        let made = Command::new(tool)
            .current_dir(work)
            .args(["-bd", "-y", "-bso0", "-bsp0"])
            .args(*args)
            .stdout(Stdio::null())
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("==> warning: 7zz {} failed; its steps will fail", args.join(" "));
        }
    }
    let mut made = Vec::new();
    for entry in std::fs::read_dir(work).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() && !files.iter().any(|(rel, _, _)| *rel == name) {
            made.push((name, 0o644, std::fs::read(entry.path()).map_err(|e| e.to_string())?));
        }
    }
    made.sort();
    if let Some((_, _, lzma)) = made.iter().find(|(n, _, _)| n == "m-lzma.7z") {
        // A flipped byte in the packed data (after the 32-byte start header)
        // and the first half of the archive.
        let mut corrupt = lzma.clone();
        if let Some(b) = corrupt.get_mut(32 + 200) {
            *b ^= 0x55;
        }
        let half = lzma[..lzma.len() / 2].to_vec();
        made.push(("corrupt.7z".into(), 0o644, corrupt));
        made.push(("trunc.7z".into(), 0o644, half));
    }
    files.extend(made);
    Ok(files)
}

// ----- the boot test -----------------------------------------------------------------------

/// The boot test (see the module docs).
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    eprintln!("==> boot test: 7-Zip's command line (7za.exe, x64) runs on the NT personality (ADR 0087, 0091)");
    let Some(exe) = cached() else {
        eprintln!(
            "==> skipped: 7-Zip {VERSION}'s 7za.exe is not in the cache ({}); `cargo xtask win fetch-7zip` downloads it",
            cache_dir().display()
        );
        return Ok(());
    };
    if !tool_path().is_file() {
        eprintln!(
            "==> skipped: 7-Zip {VERSION}'s Linux 7zz is not in the cache ({}); `cargo xtask win fetch-7zip` downloads it",
            tool_path().display()
        );
        return Ok(());
    }
    let bytes = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let mut tree = crate::initramfs::Tree::default();
    tree.files.push((format!("{DIR}/7za.exe"), 0o755, bytes));
    match std::fs::read(cache_dir().join(SAMPLE_ARCHIVE)) {
        Ok(a) => tree.files.push((format!("{DIR}/{SAMPLE_ARCHIVE}"), 0o644, a)),
        Err(_) => eprintln!("==> no {SAMPLE_ARCHIVE} in the cache (`cargo xtask win fetch-7zip` makes it)"),
    }
    for (rel, mode, bytes) in fixtures(&tool_path(), &root().join("target").join("poly-7zip-files"))? {
        tree.files.push((format!("{DIR}/{rel}"), mode, bytes));
    }
    for (rel, target) in SYMLINKS {
        tree.symlinks.push((format!("{DIR}/{rel}"), (*target).to_string()));
    }
    let image = crate::initramfs::programs_image_with(&[], &tree);
    let image = crate::initramfs::write(&root(), "poly-7zip-initramfs.cpio", &image)?;
    // `POLY_7ZIP_STEPS=name,name`: only those steps.
    let only: Option<Vec<String>> =
        std::env::var("POLY_7ZIP_STEPS").ok().map(|v| v.split(',').map(str::to_string).collect());
    let mut failed = Vec::new();
    let mut summary = Vec::new();
    for step in STEPS {
        if only.as_ref().is_some_and(|o| !o.iter().any(|n| n == step.name)) {
            continue;
        }
        let mut append = format!("poly.qemu_exit poly.init=/{DIR}/7za.exe");
        for a in step.args {
            append.push_str(" poly.init.arg=");
            append.push_str(a);
        }
        let mut qemu_args = vec!["-initrd".to_string(), image.display().to_string()];
        if step.cpus != 0 {
            qemu_args.extend(["-smp".into(), step.cpus.to_string()]);
        }
        let timeout = if step.secs == 0 { crate::BOOT_TIMEOUT } else { std::time::Duration::from_secs(step.secs) };
        let started = std::time::Instant::now();
        // When the program started and ended, as the serial lines arrive:
        // its own run time, without the boot's.
        let marks = Arc::new(std::sync::Mutex::new(RunMarks::default()));
        let on_line = {
            let marks = Arc::clone(&marks);
            move |line: &str| marks.lock().unwrap_or_else(|e| e.into_inner()).see(line, std::time::Instant::now())
        };
        let b = crate::with_boot_timeout(timeout, || {
            crate::boot_with(kernel, syms, MEMORY, &append, crate::Devices::None, &qemu_args, None, on_line)
        })?;
        let took = started.elapsed();
        let ran = marks.lock().unwrap_or_else(|e| e.into_inner()).ran();
        let ran = ran.map_or_else(|| "-".to_string(), |r| format!("{:.2} s", r.as_secs_f64()));
        let o = outcome(&b.output, step.expect);
        let passed = o.passed(step.code);
        let verdict = if passed { "passed" } else { "did not pass" };
        let status = o.ntstatus.map_or_else(|| format!("{:?}", o.code), |s| format!("{s:#010x}"));
        eprintln!(
            "==> 7za {} ({}): {verdict}, ran {ran} (boot and run {:.1} s): started {}, exit code {:?} (want {}), \
             status {status}, printed {:?}: {}",
            step.args.join(" "),
            step.name,
            took.as_secs_f64(),
            o.started,
            o.code,
            step.code,
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
        let required = required(step.name);
        if required && !passed {
            failed.push(step.name);
        }
        summary.push(format!(
            "{:<16} {:<8} ran {ran:>8} (boot and run {:>5.1} s)  code {:<4} {}",
            step.name,
            if passed {
                "passed"
            } else if required {
                "FAILED"
            } else {
                "not yet"
            },
            took.as_secs_f64(),
            o.code.map_or_else(|| "-".to_string(), |c| c.to_string()),
            if b.timed_out { "timed out" } else { "" }
        ));
    }
    eprintln!("==> 7za.exe steps ({} required, {} reported only):", STEPS.len() - NOT_YET.len(), NOT_YET.len());
    for l in &summary {
        eprintln!("      {l}");
    }
    let scenario = scenario_boot(kernel, &exe);
    if !failed.is_empty() {
        return Err(format!("7za.exe steps that must pass did not: {}", failed.join(", ")));
    }
    scenario
}

/// When a boot's program started (`POLY-INIT: exec`) and ended
/// (`POLY-EXIT`), by the arrival of those serial lines.
#[derive(Default)]
struct RunMarks {
    exec: Option<std::time::Instant>,
    exit: Option<std::time::Instant>,
}

impl RunMarks {
    fn see(&mut self, line: &str, now: std::time::Instant) {
        if self.exec.is_none() && line.contains("POLY-INIT: exec") {
            self.exec = Some(now);
        }
        if self.exit.is_none() && line.contains("POLY-EXIT:") {
            self.exit = Some(now);
        }
    }

    /// The program's run time, if it started and ended.
    fn ran(&self) -> Option<std::time::Duration> {
        Some(self.exit?.saturating_duration_since(self.exec?))
    }
}

/// Memory for the 7-Zip boots: the initramfs holds a few megabytes of
/// inputs and 7-Zip's larger dictionaries need room.
const MEMORY: &str = "512M";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_boot() {
        let log = "fs: proc on /proc\nPOLY-INIT: exec /7zip/7za.exe\n\n7-Zip (a) 26.05 (x64)\nnt: pid 2 unhandled exception 0xe06d7363 at 0x1\nUsage: 7za <command>\nPOLY-EXIT: pid=2 code=0 ntstatus=0x0\n";
        let o = outcome(log, "Usage: 7za");
        assert!(o.passed(0), "{o:?}");
        assert_eq!(o.output, ["7-Zip (a) 26.05 (x64)", "Usage: 7za <command>"]);
        assert_eq!(o.notes.len(), 1);
        let o = outcome("POLY-INIT: exec /x\nPOLY-EXIT: pid=2 code=53 ntstatus=0xc0000135\n", "Usage");
        assert_eq!((o.code, o.ntstatus, o.printed), (Some(53), Some(0xC000_0135), false));
        assert!(!o.passed(0) && o.passed(53) == o.printed);
        // Output on the same line as the record.
        let o = outcome("POLY-INIT: exec /x\nEverything is OkPOLY-EXIT: pid=2 code=0\n", "Everything is Ok");
        assert!(o.passed(0));
        assert!(!outcome("", "x").started);
        // The console host's screen: cursor movements are line breaks.
        let o = outcome(
            "POLY-INIT: exec /x\n\x1b[2J\x1b[2;1H7-Zip\x1b[3;1HUsage: 7za\x1b[1mx\nPOLY-EXIT: pid=2 code=0\n",
            "",
        );
        assert_eq!(o.output, ["7-Zip", "Usage: 7zax"]);
    }

    #[test]
    fn every_reported_only_step_exists() {
        for (r, _) in NOT_YET {
            assert!(STEPS.iter().any(|s| s.name == *r), "{r}");
        }
        // Each step's command line fits Poly's (512 bytes), its words have
        // no spaces (one `poly.init.arg=` each), and names are unique.
        for (i, s) in STEPS.iter().enumerate() {
            let len: usize = s.args.iter().map(|a| a.len() + 15).sum();
            assert!(len + 64 < 512);
            assert!(s.args.iter().all(|a| !a.contains(' ') && a.is_ascii()), "{}", s.name);
            assert!(STEPS[..i].iter().all(|t| t.name != s.name), "{}", s.name);
        }
    }

    #[test]
    fn the_inputs_are_the_same_every_time() {
        assert_eq!(text(7, 1000), text(7, 1000));
        assert_ne!(text(7, 1000), text(8, 1000));
        assert_eq!(text(1, 64 * 1024).len(), 64 * 1024);
        assert!(text(1, 4096).iter().all(|b| b.is_ascii()));
        assert_eq!(noise(3, 1001).len(), 1001);
        let files = sources();
        assert_eq!(files, sources());
        // Every name is unique, and the long one is past MAX_PATH below
        // the test's directory (`Z:\7zip\`).
        for (i, (rel, _, _)) in files.iter().enumerate() {
            assert!(files[..i].iter().all(|(r, _, _)| r != rel), "{rel}");
        }
        assert!(files.iter().any(|(rel, _, _)| rel.starts_with("long/") && rel.len() + 8 > 260));
        assert!(files.iter().filter(|(rel, _, _)| rel.starts_with("tree/")).count() > 200);
        assert!(files.iter().any(|(rel, mode, _)| rel == "ro.txt" && mode & 0o222 == 0));
    }

    #[test]
    fn run_time_is_from_exec_to_exit() {
        let t0 = std::time::Instant::now();
        let mut m = RunMarks::default();
        m.see("boot: cmdline", t0);
        assert_eq!(m.ran(), None);
        m.see("    | POLY-INIT: exec /7zip/7za.exe", t0 + std::time::Duration::from_secs(1));
        m.see("POLY-EXIT: pid=1 code=0", t0 + std::time::Duration::from_secs(3));
        m.see("POLY-EXIT: again", t0 + std::time::Duration::from_secs(9));
        assert_eq!(m.ran(), Some(std::time::Duration::from_secs(2)));
    }
}
