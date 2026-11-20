//! Poly's own `libobjc.A.dylib` (ADR 0261): build, link, verify, and the
//! boot test that runs an Objective-C program with it.
//!
//! As libSystem (ADR 0258, `libsystem.rs`): `personalities/xnu/libobjc` is
//! a `no_std` crate (with `alloc`) built for `x86_64-apple-darwin` with
//! `-Zbuild-std`, linked into a dylib by the toolchain's `rust-lld` in its
//! Mach-O flavour, against Poly's `libSystem.B.dylib`, with Apple's install
//! name and versions (`libobjc.tbd` of the SDK: current 228,
//! compatibility 1) and the exports of `exports/libobjc.A.txt`. Cached
//! under a hash of its inputs and libSystem's; read back with `poly-macho`
//! (install name, versions, only Poly's libSystem loaded, exactly the
//! listed exports, each a name the SDK's libobjc exports).

use crate::initramfs::{self, Tree};
use crate::programs::run_as_init;
use crate::symbols::Symbols;
use crate::{Result, cargo, libsystem, ntdll, root, step};
use poly_macho::MachO;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const CRATE: &str = "personalities/xnu/libobjc";
const TARGET: &str = "x86_64-apple-darwin";
const INSTALL: &str = "/usr/lib/libobjc.A.dylib";
const CURRENT: &str = "228";
const COMPATIBILITY: &str = "1";
const MIN_MACOS: &str = "13.0";
const RUSTFLAGS: &str = "-Ctarget-cpu=x86-64";
const LINK_FLAGS: &[&str] = &["-flavor", "darwin", "-arch", "x86_64", "-dylib", "-fixup_chains", "-dead_strip"];

/// `cargo xtask libobjc [--out DIR]`.
pub fn command(args: &[String]) -> Result {
    let dir = ensure()?;
    let file = dir.join("libobjc.A.dylib");
    if let Some(out) = args.windows(2).find(|w| w[0] == "--out").map(|w| PathBuf::from(&w[1])) {
        let to = out.join(INSTALL.trim_start_matches('/'));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&file, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    eprintln!("{INSTALL:<40} {size:>7} bytes, {:>3} exports", read_exports()?.len());
    Ok(())
}

/// The dylib as a file of a Poly tree: (path without the leading `/`, bytes).
pub fn tree_file() -> Result<(String, Vec<u8>)> {
    let p = ensure()?.join("libobjc.A.dylib");
    Ok((INSTALL.trim_start_matches('/').to_string(), std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?))
}

static ENSURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The directory with the up-to-date dylib: from the cache, or built,
/// verified and cached.
pub fn ensure() -> Result<PathBuf> {
    let _one = ENSURING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = root();
    let sys = libsystem::ensure()?;
    let key = hash_inputs(&root, &sys)?;
    let cache = ntdll::cache_dir(&root).with_file_name("libobjc-cache");
    let cached = cache.join(format!("{key:016x}"));
    if cached.join("libobjc.A.dylib").is_file() {
        return Ok(cached);
    }
    let started = std::time::Instant::now();
    let work = root.join("target").join("libobjc");
    let out = work.join("out");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    build(&work, &out, &sys)?;
    verify(&out.join("libobjc.A.dylib"))?;
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let tmp = cache.join(format!(".{key:016x}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::copy(out.join("libobjc.A.dylib"), tmp.join("libobjc.A.dylib")).map_err(|e| format!("libobjc: {e}"))?;
    if std::fs::rename(&tmp, &cached).is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    crate::progress::note(&format!("libobjc built in {:.0?}, cached as {}", started.elapsed(), cached.display()));
    Ok(cached)
}

fn inputs(root: &Path) -> Result<Vec<PathBuf>> {
    ntdll::inputs_from(root, &[CRATE], &["rust-toolchain.toml", "tools/xtask/src/libobjc.rs"])
}

/// The inputs and the libSystem it links against (its directory name is
/// libSystem's own content hash).
fn hash_inputs(root: &Path, sys: &Path) -> Result<u64> {
    let extra =
        format!("{RUSTFLAGS} {} {}", LINK_FLAGS.join(" "), sys.file_name().unwrap_or_default().to_string_lossy());
    ntdll::hash_files(root, &extra, &inputs(root)?)
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

fn cargo_build(target_dir: &Path, sub: &str) -> Result<Command> {
    let vendor = ntdll::vendor_dir()?;
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTFLAGS", RUSTFLAGS)
        .arg(sub)
        .arg("--manifest-path")
        .arg(root().join(CRATE).join("Cargo.toml"))
        .args(["-Zbuild-std=core,alloc,compiler_builtins", "--target", TARGET])
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()));
    Ok(cmd)
}

fn build(work: &Path, out: &Path, sys: &Path) -> Result {
    let target_dir = work.join("cargo");
    let mut cmd = cargo_build(&target_dir, "build")?;
    cmd.arg("--release");
    step("build libobjc (Rust)", &mut cmd)?;
    let archive = target_dir.join(TARGET).join("release").join("liblibobjc.a");
    // A sysroot of Poly's libSystem libraries at their install paths:
    // rust-lld looks up libSystem's re-exports by install name in it.
    let sysroot = work.join("sysroot");
    let _ = std::fs::remove_dir_all(&sysroot);
    for (path, bytes) in libsystem::tree_files()? {
        let to = sysroot.join(&path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let _ = sys;
    let mut link = Command::new(rust_lld()?);
    link.args(LINK_FLAGS)
        .args(["-platform_version", "macos", MIN_MACOS, MIN_MACOS])
        .args(["-install_name", INSTALL])
        .args(["-compatibility_version", COMPATIBILITY, "-current_version", CURRENT])
        .arg("-force_load")
        .arg(&archive)
        .arg("-exported_symbols_list")
        .arg(root().join(CRATE).join("exports").join("libobjc.A.txt"))
        .arg("-syslibroot")
        .arg(&sysroot)
        .arg(sysroot.join("usr/lib/libSystem.B.dylib"))
        .arg("-o")
        .arg(out.join("libobjc.A.dylib"));
    step("link libobjc.A.dylib (rust-lld, Mach-O)", &mut link)
}

/// Clippy of the crate for its target (`cargo xtask ci lint`).
pub fn lint() -> Result {
    let mut cmd = cargo_build(&root().join("target").join("libobjc").join("clippy"), "clippy")?;
    cmd.args(["--", "-D", "warnings"]);
    step("clippy (libobjc)", &mut cmd)
}

fn read_exports() -> Result<BTreeSet<String>> {
    let p = root().join(CRATE).join("exports").join("libobjc.A.txt");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

/// The SDK's export names of libobjc (`coverage/macos/api`).
fn sdk_names() -> Result<BTreeSet<String>> {
    let p = root().join("coverage/macos/api/libobjc.A.dylib.json");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("symbol").and_then(|s| s.as_str()).map(str::to_string))
        .collect())
}

/// A Mach-O symbol's name in the SDK map: without its leading `_`, and a
/// class's objects as `objc:Name`.
fn map_name(symbol: &str) -> String {
    let s = symbol.strip_prefix('_').unwrap_or(symbol);
    for prefix in ["OBJC_CLASS_$_", "OBJC_METACLASS_$_"] {
        if let Some(name) = s.strip_prefix(prefix) {
            return format!("objc:{name}");
        }
    }
    s.to_string()
}

fn verify(p: &Path) -> Result {
    let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let err = |what: String| format!("{}: {what}", p.display());
    let m = MachO::parse(&bytes).map_err(|e| err(format!("not a Mach-O file: {e:?}")))?;
    let id = m.dylib_id().ok_or_else(|| err("no LC_ID_DYLIB".into()))?;
    if id.name != INSTALL || id.current_version.major() != 228 || id.compatibility_version.major() != 1 {
        return Err(err(format!("identity {} {:?} {:?}", id.name, id.current_version, id.compatibility_version)));
    }
    for d in m.dylibs() {
        if d.name != "/usr/lib/libSystem.B.dylib" {
            return Err(err(format!("loads {}, not only Poly's libSystem", d.name)));
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
    let foreign: Vec<_> = listed.iter().filter(|s| !sdk.contains(&map_name(s))).collect();
    if !foreign.is_empty() {
        return Err(err(format!("exports names the SDK's libobjc does not: {foreign:?}")));
    }
    Ok(())
}

/// The Objective-C program of the boot test (`tests/macos/libobjc`), linked
/// by Apple's linker against the SDK's libobjc and libSystem, and what it
/// prints when every check passed.
const HELLO: &str = "tests/macos/libobjc/hello_x64";
const HELLO_DONE: &str = "objc: done";

/// The boot test group `objc`: the program runs as the first process with
/// Poly's libSystem and libobjc in `/usr/lib`.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let hello = std::fs::read(root().join(HELLO)).map_err(|e| format!("{HELLO}: {e}"))?;
    let programs: Vec<(&str, &[u8])> = vec![("mobjc", &hello)];
    let mut tree = Tree::default();
    for (path, bytes) in libsystem::tree_files()? {
        tree.files.push((path, 0o755, bytes));
    }
    let (path, bytes) = tree_file()?;
    tree.files.push((path, 0o755, bytes));
    let image =
        initramfs::write(&root(), "poly-objc-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))?;
    eprintln!("==> boot test: an Objective-C program (classes, a category, ARC, pools) with Poly's libobjc.A.dylib");
    let b = run_as_init(kernel, syms, &image, "mobjc", HELLO_DONE)?;
    if let Some(line) = b.output.lines().find(|l| l.contains("not ok ") || l.contains(" not implemented")) {
        return Err(format!("/bin/mobjc: {line}"));
    }
    Ok(())
}

/// The visible spec tests of libobjc built for x86-64
/// (`tests/macos/libobjc/spec`, built by its `generate.sh`).
const SPEC: &str = "tests/macos/libobjc/spec";

/// The arm64 output macOS recorded for test `name` of `specs/macos/libobjc.A.dylib`.
fn recorded(name: &str) -> Option<String> {
    let dir = root().join("specs/macos/libobjc.A.dylib");
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = entry.path().join(format!("{name}.c"));
        if !p.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(entry.path().join("macos.json")).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        return json["tests"].as_array()?.iter().find(|t| t["name"] == name)?["results"]["arm64"]["stdout"]
            .as_str()
            .map(str::to_string);
    }
    None
}

/// The outcome of one spec test on Poly.
#[derive(Debug)]
pub struct SpecResult {
    pub name: String,
    /// The wait status word.
    pub status: Option<u32>,
    /// Its protocol lines equal macOS's (arm64) recording.
    pub matches: bool,
    pub not_ok: Vec<String>,
}

/// Run every built spec test on Poly in one boot (the runner as init
/// starts each as a child) and compare each with the recording.
pub fn run_spec(kernel: &Path, syms: &Arc<Symbols>) -> Result<Vec<SpecResult>> {
    let dir = root().join(SPEC);
    let list = std::fs::read_to_string(dir.join("tests.txt")).map_err(|e| format!("{SPEC}/tests.txt: {e}"))?;
    let names: Vec<&str> = list.lines().filter(|l| !l.is_empty()).collect();
    let runner = std::fs::read(dir.join("runner")).map_err(|e| format!("{SPEC}/runner: {e}"))?;
    let programs: Vec<(&str, &[u8])> = vec![("objc-spec-runner", &runner)];
    let mut tree = Tree::default();
    for (path, bytes) in libsystem::tree_files()? {
        tree.files.push((path, 0o755, bytes));
    }
    let (path, bytes) = tree_file()?;
    tree.files.push((path, 0o755, bytes));
    for n in &names {
        let b = std::fs::read(dir.join("bin").join(n)).map_err(|e| format!("{SPEC}/bin/{n}: {e}"))?;
        tree.files.push((format!("bin/objc/{n}"), 0o755, b));
    }
    tree.files.push(("etc/objc-tests.txt".into(), 0o644, list.as_bytes().to_vec()));
    let image =
        initramfs::write(&root(), "poly-objc-spec-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))?;
    let b = run_as_init(kernel, syms, &image, "objc-spec-runner", "runner: done")?;
    let mut results = Vec::new();
    let lines: Vec<&str> = b.output.lines().map(|l| l.trim_end_matches('\r')).collect();
    for n in names {
        let start = lines.iter().position(|l| *l == format!("test {n}"));
        let end = lines.iter().position(|l| l.starts_with(&format!("exit {n} ")));
        let (Some(s), Some(e)) = (start, end) else {
            results.push(SpecResult { name: n.into(), status: None, matches: false, not_ok: Vec::new() });
            continue;
        };
        let status = lines[e].rsplit(' ').next().and_then(|v| v.parse().ok());
        // The protocol lines on both sides (children's own lines and other
        // text are not compared).
        let protocol = |l: &&str| ["ok ", "not ok ", "value ", "summary "].iter().any(|p| l.starts_with(p));
        let got: Vec<&str> = lines[s + 1..e].iter().copied().filter(protocol).collect();
        let want = recorded(n).unwrap_or_default();
        let want: Vec<&str> = want.lines().filter(protocol).collect();
        let not_ok = got.iter().filter(|l| l.starts_with("not ok ")).map(|l| l.to_string()).collect();
        results.push(SpecResult { name: n.into(), status, matches: got == want, not_ok });
    }
    Ok(results)
}

/// Spec tests that cannot pass on Poly yet, and why (they are run and
/// reported, not failed on). None of the reasons is in libobjc's own
/// behaviour except the unwinder: the others are libSystem pieces.
const SPEC_NOT_YET: &[(&str, &str)] = &[
    ("impblock_basic", "imp_implementationWithBlock and the Blocks runtime (libsystem_blocks)"),
    ("impblock_shared", "imp_implementationWithBlock and the Blocks runtime (libsystem_blocks)"),
    ("exc_begincatch_rc", "caught exceptions: an unwinder (_Unwind_Resume, libunwind)"),
    ("exc_begincatch_rethrow", "caught exceptions: an unwinder (_Unwind_Resume, libunwind)"),
    ("exc_endcatch_leave", "caught exceptions: an unwinder (_Unwind_Resume, libunwind)"),
    ("exc_endcatch_sequence", "caught exceptions: an unwinder (_Unwind_Resume, libunwind)"),
    ("exc_throw_catch", "caught exceptions: an unwinder (_Unwind_Resume, libunwind)"),
    ("arc_storeweak_arcweak", "its cleanups name _Unwind_Resume (libunwind)"),
    ("syncexit_exceptions", "caught exceptions: an unwinder (_Unwind_Resume, libunwind)"),
    ("syncenter_threads", "threads: pthread_create in Poly's libSystem"),
    ("syncexit_recursive", "threads: pthread_create in Poly's libSystem"),
    ("msgsend_tagged", "tagged pointers come from Foundation (dlopen)"),
    ("msgsend_stret_basic", "dlsym in Poly's libSystem"),
];

/// Spec tests whose x86-64 build checks other things than the arm64 one
/// the recording is of (the arm64 struct-return parts, BOOL encoded `c` on
/// x86-64, as the sheets say): their own checks must pass, the text need
/// not equal the arm64 recording.
const X86_64_PARTS: &[&str] =
    &["methodenc_nsobject", "msgsend_nil", "msgsendsuper2_args", "msgsendsuper2_stret_basic", "msgsend_stret_nil"];

/// The boot test group `objc-spec`: every visible libobjc test on Poly;
/// each must print what macOS printed, except the ones in [`SPEC_NOT_YET`].
pub fn spec_boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    eprintln!("==> boot test: the libobjc visible spec tests (x86-64 builds) with Poly's libobjc");
    let results = run_spec(kernel, syms)?;
    let mut failed = Vec::new();
    let mut passed = 0;
    for r in &results {
        let own = r.status == Some(0) && r.not_ok.is_empty();
        let ok = own && (r.matches || X86_64_PARTS.contains(&r.name.as_str()));
        if ok {
            passed += 1;
        }
        let known = SPEC_NOT_YET.iter().any(|(n, _)| *n == r.name);
        eprintln!(
            "    {} {} (status {:?}){}",
            if ok { "pass" } else { "FAIL" },
            r.name,
            r.status,
            r.not_ok.first().map(|l| format!(": {l}")).unwrap_or_default()
        );
        if !ok && !known {
            failed.push(r.name.clone());
        }
    }
    eprintln!(
        "==> {passed} of {} libobjc spec tests pass ({} of them x86-64 parts); {} wait for the unwinder, threads, \
         blocks or dlopen",
        results.len(),
        X86_64_PARTS.len(),
        SPEC_NOT_YET.len(),
    );
    if failed.is_empty() { Ok(()) } else { Err(format!("libobjc spec tests failing: {failed:?}")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_list_names_only_sdk_symbols() {
        let sdk = sdk_names().unwrap();
        for s in read_exports().unwrap() {
            assert!(sdk.contains(&map_name(&s)), "{s} is not in the SDK's libobjc list");
        }
    }

    #[test]
    fn class_symbols_map_to_the_sdk_names() {
        assert_eq!(map_name("_OBJC_CLASS_$_NSObject"), "objc:NSObject");
        assert_eq!(map_name("_objc_msgSend"), "objc_msgSend");
    }
}
