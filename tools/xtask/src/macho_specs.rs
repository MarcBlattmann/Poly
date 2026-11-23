//! The boot test groups `macho-specs` and `macho-specs-a64` (ADR 0246,
//! addendum): every visible test of the macOS system call entries
//! (`specs/macos/{bsd,mach}/*.tests/*.c`) built for x86-64 (`macho-specs`)
//! and for arm64 (`macho-specs-a64`, through the translator, ADR 0257), each
//! run on Poly as the first process, and compared with what macOS printed
//! (the arm64 recording in the test's `macos.json`). One group per
//! architecture, so CI runs them in two jobs (each about six minutes).
//!
//! - **Building.** The tests include only the raw test header
//!   (`tests/macos/include/poly_mtest.h`), so any clang builds them for a
//!   Darwin target (`-target <arch>-apple-macos13.0`, no SDK) and rust-lld
//!   links them against Poly's stand-in libSystem
//!   (`crates/poly-dyld/tests/data`). The programs go to
//!   `target/macho-specs/<arch>/` and are rebuilt when a source is newer.
//!   Without clang the group says so and skips.
//! - **Comparing.** An arm64 run must print exactly what macOS printed and
//!   end with its exit code. An x86-64 run must print the same check lines
//!   (`ok`, `not ok`, `summary`) and end the same: `value` lines may depend
//!   on the architecture (the page size), and a test whose checks are
//!   arm64-only prints only its summary on x86-64, which counts as
//!   checking nothing.
//! - **The ratchet.** `personalities/xnu/tests/data/specs-passing.txt` lists
//!   the runs (`<arch> <test>`) that pass: one of them failing fails the
//!   group; a run that passes but is not listed is reported, to be added.
//!   `POLY_MACHO_SPECS=<substring>[,<substring>...]` runs only the tests
//!   whose name or entry contains one of them.

use crate::initramfs::{self, Tree};
use crate::symbols::Symbols;
use crate::{Devices, QEMU_SUCCESS, Result, boot_on, native, root, with_boot_timeout};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// The guest's memory: enough that `hw.memsize` is what a Mac's is above
/// (a test checks for more than 1 GiB).
const MEMORY: &str = "2G";
const PASSING: &str = "personalities/xnu/tests/data/specs-passing.txt";
const LIBSYSTEM: &str = "crates/poly-dyld/tests/data";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Arch {
    X64,
    A64,
}

impl Arch {
    fn tag(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::A64 => "a64",
        }
    }
    fn target(self) -> &'static str {
        match self {
            Arch::X64 => "x86_64-apple-macos13.0",
            Arch::A64 => "arm64-apple-macos13.0",
        }
    }
    fn lld_arch(self) -> &'static str {
        match self {
            Arch::X64 => "x86_64",
            Arch::A64 => "arm64",
        }
    }
}

/// One visible test.
struct Test {
    name: String,
    /// `<component>/<symbol>` of its entry.
    entry: String,
    source: PathBuf,
    dir: PathBuf,
}

fn tests() -> Result<Vec<Test>> {
    let mut out = Vec::new();
    for component in ["bsd", "mach"] {
        let base = root().join("specs/macos").join(component);
        let Ok(dirs) = std::fs::read_dir(&base) else { continue };
        let mut dirs: Vec<PathBuf> = dirs.filter_map(|d| d.ok().map(|d| d.path())).collect();
        dirs.sort();
        for dir in dirs.into_iter().filter(|d| d.extension().is_some_and(|e| e == "tests")) {
            let symbol = dir.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .filter_map(|f| f.ok().map(|f| f.path()))
                .filter(|f| f.extension().is_some_and(|e| e == "c"))
                .collect();
            files.sort();
            for source in files {
                let name = source.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
                out.push(Test { name, entry: format!("{component}/{symbol}"), source, dir: dir.clone() });
            }
        }
    }
    Ok(out)
}

fn rust_lld() -> Result<PathBuf> {
    let out = Command::new("rustc").args(["--print", "sysroot"]).output().map_err(|e| format!("rustc: {e}"))?;
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let vv = Command::new("rustc").arg("-vV").output().map_err(|e| format!("rustc: {e}"))?;
    let host = String::from_utf8_lossy(&vv.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        .ok_or("rustc -vV: no host")?;
    let exe = if cfg!(windows) { "rust-lld.exe" } else { "rust-lld" };
    Ok(Path::new(&sysroot).join("lib/rustlib").join(host).join("bin").join(exe))
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Build `t` for `arch`; the program's path.
fn build(t: &Test, arch: Arch, lld: &Path) -> Result<PathBuf> {
    let out_dir = root().join("target/macho-specs").join(arch.tag());
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let program = out_dir.join(&t.name);
    let header = root().join("tests/macos/include/poly_mtest.h");
    let fresh =
        modified(&program).is_some_and(|p| [&t.source, &header].iter().all(|s| modified(s).is_some_and(|s| s <= p)));
    if fresh {
        return Ok(program);
    }
    let object = out_dir.join(format!("{}.o", t.name));
    let st = Command::new("clang")
        .args(["-target", arch.target()])
        .args([
            "-ffreestanding",
            "-nostdlib",
            "-fno-stack-protector",
            "-fno-builtin",
            "-fno-ident",
            "-O1",
            "-fno-asynchronous-unwind-tables",
            "-fno-unwind-tables",
            "-w",
        ])
        .arg("-I")
        .arg(root().join("tests/macos/include"))
        .arg("-c")
        .arg(&t.source)
        .arg("-o")
        .arg(&object)
        .status()
        .map_err(|e| format!("clang: {e}"))?;
    if !st.success() {
        return Err(format!("{}: clang failed", t.source.display()));
    }
    let libsystem = root().join(LIBSYSTEM).join(format!("libSystem_{}.dylib", arch.tag()));
    let st = Command::new(lld)
        .args(["-flavor", "darwin", "-arch", arch.lld_arch(), "-platform_version", "macos", "13.0", "13.0"])
        .arg("-fixup_chains")
        .arg("-o")
        .arg(&program)
        .arg(&object)
        .arg(&libsystem)
        .output()
        .map_err(|e| format!("rust-lld: {e}"))?;
    if !st.status.success() {
        return Err(format!("{}: rust-lld failed: {}", t.name, String::from_utf8_lossy(&st.stderr)));
    }
    Ok(program)
}

/// What macOS recorded for test `name`: its arm64 output and exit code.
fn recorded(dir: &Path, name: &str) -> Result<(String, i64)> {
    let path = dir.join("macos.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let t = json["tests"]
        .as_array()
        .and_then(|t| t.iter().find(|t| t["name"] == name))
        .ok_or_else(|| format!("{}: no {name}", path.display()))?;
    let r = &t["results"]["arm64"];
    Ok((r["stdout"].as_str().unwrap_or_default().to_string(), r["exit_code"].as_i64().unwrap_or(-1)))
}

/// The program's standard output, from the kernel's `POLY-OUT: fd=1
/// hex=...` records (`poly.test.capture`, ADR 0027), as lines.
fn stdout_lines(output: &str) -> Vec<String> {
    let mut bytes = Vec::new();
    let mut rest = output;
    while let Some(at) = rest.find("POLY-OUT: fd=1 hex=") {
        let hex = &rest[at + "POLY-OUT: fd=1 hex=".len()..];
        let end = hex.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(hex.len());
        let h = &hex[..end];
        for i in (0..h.len() / 2).map(|i| 2 * i) {
            if let Ok(b) = u8::from_str_radix(&h[i..i + 2], 16) {
                bytes.push(b);
            }
        }
        rest = &hex[end..];
    }
    String::from_utf8_lossy(&bytes).lines().map(str::to_string).collect()
}

/// The program's exit code from the first `POLY-EXIT` record (the program
/// runs as the child of the kernel's test init), or a signal as
/// `128 + signal`.
fn exit_code(output: &str) -> Option<i64> {
    let line = output.lines().find(|l| l.contains("POLY-EXIT: pid="))?;
    let rest = &line[line.find("POLY-EXIT: pid=")? + "POLY-EXIT: pid=".len()..];
    let rest = rest.split_once(' ').map(|(_, r)| r)?;
    if let Some(c) = rest.strip_prefix("code=") {
        return c.trim().parse().ok();
    }
    rest.strip_prefix("signal=").and_then(|s| s.trim().parse::<i64>().ok()).map(|s| 128 + s)
}

/// How a run compared with the recording.
#[derive(PartialEq, Eq, Debug)]
enum Verdict {
    Pass,
    /// The x86-64 build checks nothing (arm64-only parts).
    Nothing,
    Fail(String),
}

fn judge(arch: Arch, got: &[String], code: Option<i64>, want: &str, want_code: i64) -> Verdict {
    let want: Vec<String> = want.lines().map(str::to_string).collect();
    let checks = |l: &Vec<String>| -> Vec<String> { l.iter().filter(|l| !l.starts_with("value ")).cloned().collect() };
    if arch == Arch::X64 && got.len() == 1 && got[0] == "summary 0 passed, 0 failed" && want.len() > 1 {
        return Verdict::Nothing;
    }
    let same = match arch {
        Arch::A64 => *got == want,
        Arch::X64 => checks(&got.to_vec()) == checks(&want),
    };
    if !same {
        let first = got
            .iter()
            .zip(want.iter())
            .find(|(g, w)| g != w)
            .map(|(g, w)| format!("got {g:?}, macOS {w:?}"))
            .unwrap_or_else(|| format!("{} lines, macOS {}", got.len(), want.len()));
        return Verdict::Fail(first);
    }
    if code != Some(want_code) {
        return Verdict::Fail(format!("exit {code:?}, macOS {want_code}"));
    }
    Verdict::Pass
}

/// The boot test group `macho-specs`: the x86-64 builds.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    run(kernel, syms, Arch::X64)
}

/// The boot test group `macho-specs-a64`: the arm64 builds, through the translator.
pub fn a64_boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    run(kernel, syms, Arch::A64)
}

/// Every chosen test built for `arch`, run on Poly and judged; the runs of
/// `arch` listed as passing must pass.
fn run(kernel: &Path, syms: &Arc<Symbols>, arch: Arch) -> Result {
    if Command::new("clang").arg("--version").output().is_err() {
        eprintln!("==> macho-specs: no clang on this host to build the tests; skipped");
        return Ok(());
    }
    let lld = rust_lld()?;
    let filter = std::env::var("POLY_MACHO_SPECS").ok();
    let all = tests()?;
    let chosen: Vec<&Test> = all
        .iter()
        .filter(|t| filter.as_deref().is_none_or(|f| f.split(',').any(|f| t.name.contains(f) || t.entry.contains(f))))
        .collect();
    let passing: BTreeSet<String> = std::fs::read_to_string(root().join(PASSING))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| l.split_whitespace().next() == Some(arch.tag()))
        .map(str::to_string)
        .collect();
    let mut regressions = Vec::new();
    let mut new_passes = Vec::new();
    let (mut pass, mut fail, mut nothing) = (0, 0, 0);
    let mut programs = Vec::new();
    for t in &chosen {
        programs.push((t.name.clone(), std::fs::read(build(t, arch, &lld)?).map_err(|e| e.to_string())?));
    }
    let progs: Vec<(&str, &[u8])> = programs.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let mut tree = Tree::default();
    if arch == Arch::A64 {
        tree.files.extend(native::runtime_files()?);
    }
    let lib = root().join(LIBSYSTEM).join(format!("libSystem_{}.dylib", arch.tag()));
    tree.files.push(("usr/lib/libSystem.B.dylib".into(), 0o755, std::fs::read(&lib).map_err(|e| e.to_string())?));
    let image = initramfs::write(
        &root(),
        &format!("poly-macho-specs-{}.cpio", arch.tag()),
        &initramfs::programs_image_with(&progs, &tree),
    )?;
    for t in &chosen {
        let key = format!("{} {}", arch.tag(), t.name);
        // As an ordinary user in an empty scratch directory of its own, as the
        // oracle ran them (uid 501, group staff).
        let append = format!(
            "poly.qemu_exit poly.test.capture poly.test.uid=501 poly.test.gid=20 poly.test.cwd=/scratch poly.init=/bin/{}",
            t.name
        );
        let b = with_boot_timeout(Duration::from_secs(20), || {
            boot_on(None, Some(&image), kernel, syms, MEMORY, &append, Devices::None)
        });
        let verdict = match b {
            Err(e) => Verdict::Fail(format!("boot: {e}")),
            Ok(b) if b.status != Some(QEMU_SUCCESS) && exit_code(&b.output).is_none() => {
                let last = stdout_lines(&b.output).last().cloned().unwrap_or_default();
                Verdict::Fail(format!("did not end (QEMU {:?}; last line {last:?})", b.status))
            }
            Ok(b) => {
                let (want, want_code) = recorded(&t.dir, &t.name)?;
                judge(arch, &stdout_lines(&b.output), exit_code(&b.output), &want, want_code)
            }
        };
        match &verdict {
            Verdict::Pass => {
                pass += 1;
                if !passing.contains(&key) {
                    new_passes.push(key.clone());
                }
            }
            Verdict::Nothing => nothing += 1,
            Verdict::Fail(why) => {
                fail += 1;
                if passing.contains(&key) {
                    regressions.push(format!("{key} ({}): {why}", t.entry));
                }
            }
        }
        let mark = match &verdict {
            Verdict::Pass => "PASS".to_string(),
            Verdict::Nothing => "-   (checks nothing on x86-64)".to_string(),
            Verdict::Fail(why) => format!("FAIL {why}"),
        };
        eprintln!("    {:<4} {:<40} {:<28} {mark}", arch.tag(), t.name, t.entry);
    }
    eprintln!("==> macho-specs ({}): {pass} pass, {fail} fail, {nothing} check nothing on x86-64", arch.tag());
    if !new_passes.is_empty() {
        eprintln!("==> macho-specs: passing but not in {PASSING}:\n    {}", new_passes.join("\n    "));
    }
    if regressions.is_empty() {
        Ok(())
    } else {
        Err(format!("macho-specs: runs listed as passing fail:\n  {}", regressions.join("\n  ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judging() {
        let want = "ok a\nvalue page 16384\nsummary 1 passed, 0 failed\n";
        let got = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(judge(Arch::A64, &got(want), Some(0), want, 0), Verdict::Pass);
        assert_eq!(
            judge(Arch::X64, &got("ok a\nvalue page 4096\nsummary 1 passed, 0 failed"), Some(0), want, 0),
            Verdict::Pass
        );
        assert!(matches!(
            judge(Arch::A64, &got("ok a\nvalue page 4096\nsummary 1 passed, 0 failed"), Some(0), want, 0),
            Verdict::Fail(_)
        ));
        assert_eq!(judge(Arch::X64, &got("summary 0 passed, 0 failed"), Some(0), want, 0), Verdict::Nothing);
        assert!(matches!(judge(Arch::A64, &got(want), Some(1), want, 0), Verdict::Fail(_)));
        assert_eq!(exit_code("x\nPOLY-EXIT: pid=2 code=3\n"), Some(3));
        assert_eq!(exit_code("POLY-EXIT: pid=2 signal=9"), Some(137));
        let out = "a POLY-OUT: fd=1 hex=6f6b20\nx\nPOLY-OUT: fd=2 hex=41\nPOLY-OUT: fd=1 hex=610a62\n";
        assert_eq!(stdout_lines(out), ["ok a", "b"]);
    }
}
