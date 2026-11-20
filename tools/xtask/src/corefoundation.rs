//! Poly's own `CoreFoundation.framework` (ADR 0260): build, link, verify,
//! and the boot test that runs CoreFoundation's spec tests and a C program
//! of our own with it.
//!
//! # The build
//!
//! `personalities/xnu/corefoundation` is a `no_std` crate (with `alloc`)
//! built for `x86_64-apple-darwin` with `-Zbuild-std`, like Poly's
//! libSystem (ADR 0258), as a static library, then linked into a dylib by
//! the toolchain's `rust-lld -flavor darwin`: Apple's install name and
//! versions (the SDK's `CoreFoundation.tbd`), the exports of
//! `exports/CoreFoundation.txt`, chained fixups, linked against Poly's own
//! `libSystem.B.dylib` (from a small system root holding its dylibs at
//! their install paths, where the linker finds the libraries it
//! re-exports). No Apple tool, SDK or library takes part.
//!
//! # The check
//!
//! The dylib is read back with `poly-macho`: an x86-64 dylib, its install
//! name and versions, only Poly's libSystem loaded, exactly the listed
//! exports, and every export a name of the SDK's CoreFoundation
//! (`coverage/macos/api/CoreFoundation.framework.json`).

use crate::initramfs::{self, Tree};
use crate::programs::run_as_init;
use crate::symbols::Symbols;
use crate::{Result, cargo, libsystem, ntdll, root, step};
use poly_macho::MachO;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const CRATE: &str = "personalities/xnu/corefoundation";
const TARGET: &str = "x86_64-apple-darwin";
const MIN_MACOS: &str = "13.0";
const RUSTFLAGS: &str = "-Ctarget-cpu=x86-64";
const LINK_FLAGS: &[&str] = &["-flavor", "darwin", "-arch", "x86_64", "-dylib", "-fixup_chains", "-dead_strip"];
/// The install name, compatibility and current version of the SDK's
/// `CoreFoundation.tbd`.
pub const INSTALL: &str = "/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation";
const COMPATIBILITY: &str = "150";
const CURRENT: &str = "5027.0.69";
/// The file in the build's output and the cache.
const FILE: &str = "CoreFoundation";

/// `cargo xtask corefoundation [--out DIR] [--cache-key]`.
pub fn command(args: &[String]) -> Result {
    if args.iter().any(|a| a == "--cache-key") {
        println!("corefoundation-{:016x}", hash_inputs(&root())?);
        return Ok(());
    }
    let dir = ensure()?;
    let p = dir.join(FILE);
    if let Some(out) = args.windows(2).find(|w| w[0] == "--out").map(|w| PathBuf::from(&w[1])) {
        let to = out.join(INSTALL.trim_start_matches('/'));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&p, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    eprintln!("{INSTALL} {size} bytes, {} exports", read_exports()?.len());
    Ok(())
}

/// The framework as a file of a Poly tree: (path without the leading `/`,
/// bytes).
pub fn tree_file() -> Result<(String, Vec<u8>)> {
    let p = ensure()?.join(FILE);
    Ok((INSTALL.trim_start_matches('/').to_string(), std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?))
}

static ENSURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The directory with the up-to-date framework: from the cache, or built,
/// verified and cached.
pub fn ensure() -> Result<PathBuf> {
    let _one = ENSURING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = root();
    let key = hash_inputs(&root)?;
    let cache = ntdll::cache_dir(&root).with_file_name("corefoundation-cache");
    let cached = cache.join(format!("{key:016x}"));
    if cached.join(FILE).is_file() {
        return Ok(cached);
    }
    let started = std::time::Instant::now();
    let work = root.join("target").join("corefoundation");
    let out = work.join("out");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    build(&work, &out)?;
    verify(&out.join(FILE))?;
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let tmp = cache.join(format!(".{key:016x}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::copy(out.join(FILE), tmp.join(FILE)).map_err(|e| format!("{FILE}: {e}"))?;
    if std::fs::rename(&tmp, &cached).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    crate::progress::note(&format!(
        "CoreFoundation built in {:.0?}, cached as {}",
        started.elapsed(),
        cached.display()
    ));
    Ok(cached)
}

/// The files the framework is built from: the crate with its export list,
/// the crates it uses, the toolchain pin and this file. (libSystem's own
/// inputs decide its cache; the link here only names its exports.)
fn inputs(root: &Path) -> Result<Vec<PathBuf>> {
    ntdll::inputs_from(root, &[CRATE], &["rust-toolchain.toml", "tools/xtask/src/corefoundation.rs"])
}

fn hash_inputs(root: &Path) -> Result<u64> {
    ntdll::hash_files(root, &format!("{RUSTFLAGS} {}", LINK_FLAGS.join(" ")), &inputs(root)?)
}

/// The toolchain's `rust-lld`.
fn rust_lld() -> Result<PathBuf> {
    let rustc = |arg: &str| -> Result<String> {
        let o = Command::new("rustc").args(arg.split(' ')).output().map_err(|e| format!("rustc: {e}"))?;
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let sysroot = rustc("--print sysroot")?.trim().to_string();
    let host =
        rustc("-vV")?.lines().find_map(|l| l.strip_prefix("host: ").map(str::to_string)).ok_or("rustc -vV: no host")?;
    let exe = if cfg!(windows) { "rust-lld.exe" } else { "rust-lld" };
    let lld = PathBuf::from(sysroot).join("lib/rustlib").join(host).join("bin").join(exe);
    if !lld.is_file() {
        return Err(format!("{} not found (it ships with every Rust toolchain)", lld.display()));
    }
    Ok(lld)
}

fn cargo_for_target(subcommand: &str, target_dir: &Path) -> Result<Command> {
    let vendor = ntdll::vendor_dir()?;
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTFLAGS", RUSTFLAGS)
        .arg(subcommand)
        .arg("--release")
        .arg("--manifest-path")
        .arg(root().join(CRATE).join("Cargo.toml"))
        .args(["-Zbuild-std=core,compiler_builtins,alloc", "--target", TARGET])
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()));
    Ok(cmd)
}

/// Build the archive and link the framework into `out`.
fn build(work: &Path, out: &Path) -> Result {
    let target_dir = work.join("cargo");
    step("build CoreFoundation (Rust)", &mut cargo_for_target("build", &target_dir)?)?;
    let archive = target_dir.join(TARGET).join("release").join("libcorefoundation.a");
    // A system root with Poly's libSystem at its install paths, where the
    // linker finds the libraries libSystem.B.dylib re-exports.
    let sysroot = work.join("sysroot");
    let _ = std::fs::remove_dir_all(&sysroot);
    for (path, bytes) in libsystem::tree_files()? {
        let to = sysroot.join(&path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let list = root().join(CRATE).join("exports").join("CoreFoundation.txt");
    let mut cmd = Command::new(rust_lld()?);
    cmd.args(LINK_FLAGS)
        .args(["-platform_version", "macos", MIN_MACOS, MIN_MACOS])
        .args(["-install_name", INSTALL])
        .args(["-compatibility_version", COMPATIBILITY, "-current_version", CURRENT])
        .arg("-syslibroot")
        .arg(&sysroot)
        .arg("-force_load")
        .arg(&archive)
        .arg("-exported_symbols_list")
        .arg(&list)
        .arg(sysroot.join("usr/lib/libSystem.B.dylib"))
        .arg("-o")
        .arg(out.join(FILE));
    step("link CoreFoundation (rust-lld, Mach-O)", &mut cmd)
}

/// Clippy of the crate for its target (`cargo xtask ci lint`).
pub fn lint() -> Result {
    let mut cmd = cargo_for_target("clippy", &root().join("target").join("corefoundation").join("clippy"))?;
    cmd.args(["--", "-D", "warnings"]);
    step("clippy (corefoundation)", &mut cmd)
}

/// The names of `exports/CoreFoundation.txt`.
fn read_exports() -> Result<BTreeSet<String>> {
    let p = root().join(CRATE).join("exports").join("CoreFoundation.txt");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

/// The SDK's export names of CoreFoundation (`coverage/macos/api`), without
/// the leading `_`.
fn sdk_names() -> Result<BTreeSet<String>> {
    let p = root().join("coverage/macos/api/CoreFoundation.framework.json");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("symbol").and_then(|s| s.as_str()).map(str::to_string))
        .collect())
}

/// Read the framework back and check it (see the module documentation).
fn verify(p: &Path) -> Result {
    let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let err = |what: String| format!("{}: {what}", p.display());
    let m = MachO::parse(&bytes).map_err(|e| err(format!("not a Mach-O file: {e:?}")))?;
    let id = m.dylib_id().ok_or_else(|| err("no LC_ID_DYLIB".into()))?;
    if id.name != INSTALL {
        return Err(err(format!("install name {}, expected {INSTALL}", id.name)));
    }
    let version = |v: poly_macho::Version| {
        let s = format!("{}.{}.{}", v.major(), v.minor(), v.patch());
        s.trim_end_matches(".0").trim_end_matches(".0").to_string()
    };
    if version(id.current_version) != CURRENT || version(id.compatibility_version) != COMPATIBILITY {
        return Err(err(format!(
            "versions {} / {}, expected {CURRENT} / {COMPATIBILITY}",
            version(id.current_version),
            version(id.compatibility_version)
        )));
    }
    for d in m.dylibs() {
        if d.name != "/usr/lib/libSystem.B.dylib" {
            return Err(err(format!("loads {}, which is not Poly's libSystem", d.name)));
        }
    }
    let mut exported = BTreeSet::new();
    if let Some(ex) = m.exports().map_err(|e| err(format!("exports: {e:?}")))? {
        ex.walk(|name, _| {
            exported.insert(String::from_utf8_lossy(name).into_owned());
            std::ops::ControlFlow::Continue(())
        })
        .map_err(|e| err(format!("export trie: {e:?}")))?;
    }
    let listed = read_exports()?;
    if exported != listed {
        let missing: Vec<_> = listed.difference(&exported).collect();
        let extra: Vec<_> = exported.difference(&listed).collect();
        return Err(err(format!("exports differ from the list: missing {missing:?}, not listed {extra:?}")));
    }
    let sdk = sdk_names()?;
    let foreign: Vec<_> = listed.iter().filter(|s| !sdk.contains(s.strip_prefix('_').unwrap_or(s))).collect();
    if !foreign.is_empty() {
        return Err(err(format!("exports names the SDK's CoreFoundation does not: {foreign:?}")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The boot test.

/// The committed x86-64 programs (`tests/macos/corefoundation`, built on a
/// Mac by its `generate.sh` with clang and Apple's ld against the SDK).
const DATA: &str = "tests/macos/corefoundation";

/// The `Info.plist` of the bundle tests (`CF_APP_PLIST` of
/// `tests/macos/include/poly_mcf.h`).
const APP_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
<key>CFBundleIdentifier</key><string>test.poly.probe</string>\n\
<key>CFBundleName</key><string>Probe</string>\n\
<key>CFBundleExecutable</key><string>t</string>\n\
<key>CFBundleVersion</key><string>42</string>\n\
<key>CFBundleShortVersionString</key><string>1.2.3</string>\n\
<key>CFBundlePackageType</key><string>APPL</string>\n\
<key>MyInt</key><integer>7</integer><key>MyBool</key><true/>\n\
<key>MyArr</key><array><string>a</string><string>b</string></array>\n\
</dict></plist>\n";

/// One spec test of the list: its name, the directory with its recording,
/// and whether it runs inside `T.app`.
struct SpecTest {
    name: String,
    dir: PathBuf,
    app: bool,
}

/// `tests.txt`: one source path per line, `app` after it for a test that
/// builds and enters `T.app` (the boot test lays the application out
/// beforehand and starts the copy inside it, as the test's first half
/// would with `mkdir` and `execve`).
fn spec_tests() -> Result<Vec<SpecTest>> {
    let p = root().join(DATA).join("tests.txt");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut parts = line.split_whitespace();
        let src = parts.next().ok_or("tests.txt: an empty line")?;
        let app = parts.next() == Some("app");
        let name = Path::new(src).file_stem().and_then(|s| s.to_str()).ok_or("tests.txt: a test path")?.to_string();
        out.push(SpecTest { name, dir: root().join(src).parent().ok_or("a test directory")?.to_path_buf(), app });
    }
    Ok(out)
}

fn read(name: &str) -> Result<Vec<u8>> {
    let p = root().join(DATA).join("bin").join(name);
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// The test-protocol lines of an output (`ok`, `not ok`, `value`,
/// `summary`).
fn protocol_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| ["ok ", "not ok ", "value ", "summary "].iter().any(|p| l.starts_with(p)))
        .collect()
}

/// What macOS printed for test `name` on x86-64 (`<dir>/macos.json`).
fn recorded(dir: &Path, name: &str) -> Result<String> {
    let path = dir.join("macos.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    json["tests"]
        .as_array()
        .and_then(|t| t.iter().find(|t| t["name"] == name))
        .and_then(|t| t["results"]["x86_64"]["stdout"].as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{}: no x86_64 output for {name}", path.display()))
}

/// What `cfdemo` prints when everything worked.
const DEMO_LINES: &[&str] = &[
    "cfdemo: hello from CoreFoundation on Poly",
    "cfdemo: array of 3: alpha, beta, gamma",
    "cfdemo: joined: alpha-beta-gamma",
    "cfdemo: format: 42 3.14 [0x2a] <one>",
    "cfdemo: timer fired 3 times",
    "cfdemo: run loop result 2 (stopped)",
    "cfdemo: done",
];

/// The boot test group `corefoundation`: the runner (`/bin/cf-runner`, the
/// first process) starts each spec test of `tests.txt` and `cfdemo` with
/// Poly's dyld, libSystem and CoreFoundation; each must exit 0 and print
/// what macOS printed (x86-64 recording), `cfdemo` its lines.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let tests = spec_tests()?;
    let mut bins: Vec<(String, Vec<u8>)> = Vec::new();
    for t in &tests {
        bins.push((t.name.clone(), read(&t.name)?));
    }
    let runner = read("cf-runner")?;
    let demo = read("cfdemo")?;
    let mut programs: Vec<(&str, &[u8])> = vec![("cf-runner", &runner), ("cfdemo", &demo)];
    for (n, b) in bins.iter().filter(|(n, _)| !tests.iter().any(|t| t.app && t.name == *n)) {
        programs.push((n, b));
    }
    let mut tree = Tree::default();
    for (path, bytes) in libsystem::tree_files()? {
        tree.files.push((path, 0o755, bytes));
    }
    let (cf_path, cf) = tree_file()?;
    tree.files.push((cf_path, 0o755, cf));
    let mut list = String::new();
    for t in &tests {
        if t.app {
            let app = format!("apps/{}/T.app/Contents", t.name);
            let bin = bins.iter().find(|(n, _)| *n == t.name).map(|(_, b)| b.clone()).unwrap_or_default();
            tree.files.push((format!("{app}/MacOS/t"), 0o755, bin));
            tree.files.push((format!("{app}/Info.plist"), 0o644, APP_PLIST.as_bytes().to_vec()));
            tree.files.push((format!("{app}/Resources/hello.txt"), 0o644, b"hi\n".to_vec()));
            tree.files.push((format!("{app}/Resources/sub/deep.txt"), 0o644, b"deep\n".to_vec()));
            tree.files.push((format!("{app}/Resources/en.lproj/loc.txt"), 0o644, b"loc\n".to_vec()));
            list.push_str(&format!("{} app\n", t.name));
        } else {
            list.push_str(&format!("{}\n", t.name));
        }
    }
    list.push_str("cfdemo\n");
    tree.files.push(("etc/cftests.txt".into(), 0o644, list.into_bytes()));
    let image = initramfs::write(
        &root(),
        "poly-corefoundation-initramfs.cpio",
        &initramfs::programs_image_with(&programs, &tree),
    )?;
    eprintln!(
        "==> boot test: {} CoreFoundation spec tests and cfdemo with Poly's dyld, libSystem and CoreFoundation",
        tests.len()
    );
    let b = crate::with_boot_timeout(std::time::Duration::from_secs(300), || {
        run_as_init(kernel, syms, &image, "cf-runner", "cf-runner: done")
    })?;
    let sections = split_sections(&b.output);
    let mut failures = Vec::new();
    for t in &tests {
        let Some((out, status)) = sections.iter().find(|(n, _, _)| *n == t.name).map(|(_, o, s)| (o, s)) else {
            failures.push(format!("{}: did not run", t.name));
            continue;
        };
        if *status != "0" {
            failures.push(format!("{}: exit status {status}", t.name));
        }
        let got = protocol_lines(out);
        let want_text = recorded(&t.dir, &t.name)?;
        let want: Vec<&str> = want_text.lines().collect();
        if got != want {
            failures.push(format!("{}: printed {got:#?}, macOS printed {want:#?}", t.name));
        }
    }
    match sections.iter().find(|(n, _, _)| *n == "cfdemo") {
        Some((_, out, status)) => {
            if *status != "0" {
                failures.push(format!("cfdemo: exit status {status}"));
            }
            let mut at = 0;
            for line in DEMO_LINES {
                match out[at..].find(line) {
                    Some(i) => at += i + line.len(),
                    None => {
                        failures.push(format!("cfdemo: {line:?} missing or out of order"));
                        break;
                    }
                }
            }
        }
        None => failures.push("cfdemo: did not run".into()),
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
}

/// The runner's sections: (test name, its output, its exit status), from
/// `cf-test: <name>` up to `cf-exit: <name> <status>`.
fn split_sections(output: &str) -> Vec<(&str, String, &str)> {
    let mut out = Vec::new();
    let mut current: Option<(&str, String)> = None;
    for line in output.lines().map(|l| l.trim_end_matches('\r')) {
        if let Some(name) = line.strip_prefix("cf-test: ") {
            current = Some((name, String::new()));
        } else if let Some(rest) = line.strip_prefix("cf-exit: ") {
            let mut parts = rest.split_whitespace();
            if let (Some(name), Some(status), Some((cur, text))) = (parts.next(), parts.next(), current.take())
                && cur == name
            {
                out.push((name, text, status));
            }
        } else if let Some((_, text)) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_are_sdk_names_and_tests_are_listed() {
        let sdk = sdk_names().unwrap();
        for s in read_exports().unwrap() {
            assert!(sdk.contains(s.strip_prefix('_').unwrap_or(&s)), "{s} is not in the SDK's CoreFoundation");
        }
        let tests = spec_tests().unwrap();
        assert!(tests.len() >= 40, "{} tests listed", tests.len());
        for t in &tests {
            assert!(t.dir.join("macos.json").is_file(), "{}: no recording", t.name);
        }
    }

    #[test]
    fn sections_are_split_by_the_runner_lines() {
        let s = split_sections(
            "noise\ncf-test: a\nok x\r\nsummary 1 passed, 0 failed\ncf-exit: a 0\ncf-test: b\ncf-exit: b 1\n",
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].0, "a");
        assert_eq!(s[0].1, "ok x\nsummary 1 passed, 0 failed\n");
        assert_eq!(s[1].2, "1");
    }

    #[test]
    fn inputs_cover_the_crate_and_its_logic() {
        let files = inputs(&root()).unwrap();
        for want in [
            "personalities/xnu/corefoundation/src/lib.rs",
            "personalities/xnu/corefoundation/exports/CoreFoundation.txt",
            "crates/poly-cf/src/format.rs",
            "crates/poly-libsystem/src/printf.rs",
        ] {
            assert!(files.iter().any(|f| f == Path::new(want)), "{want} not an input");
        }
    }
}
