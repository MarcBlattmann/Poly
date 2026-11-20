//! Poly's own `libSystem.B.dylib` (ADR 0258): build, link, verify, and the
//! boot test that runs an SDK-linked C program with it.
//!
//! # The build
//!
//! `personalities/xnu/libsystem` is a `no_std` crate built for
//! `x86_64-apple-darwin` with `-Zbuild-std` (under `RUSTC_BOOTSTRAP=1`, as
//! `ntdll.dll` is built, ADR 0071), once per library with that library's
//! feature, as a static library. Each archive is then linked into a dylib by
//! the toolchain's own `rust-lld` in its Mach-O flavour (`-flavor darwin`,
//! LLVM's `ld64.lld`): the install name, compatibility and current version
//! of Apple's library (from the SDK's `.tbd` files), the exports of
//! `exports/<library>.txt`, chained fixups, dead code stripped. A library
//! that calls another links against the dylib just made, so its
//! `LC_LOAD_DYLIB` names the other's install name; `libSystem.B.dylib`
//! re-exports the six (`LC_REEXPORT_DYLIB`). No Apple tool, SDK or library
//! takes part: the same commands work on a Linux or Windows host.
//!
//! # Delivery
//!
//! As for `ntdll.dll`: the dylibs are cached under a hash of everything they
//! are built from (`target/libsystem-cache/<hash>/` of the main checkout,
//! shared by its worktrees); only a miss builds (about 20 s).
//!
//! # The check
//!
//! Every dylib is read back with `poly-macho`: a 64-bit x86-64 dylib, its
//! install name and versions, the libraries it loads (only ours), exactly the
//! exports of its list, and every exported name in the SDK's export list of
//! that library (`coverage/macos/api/<library>.dylib.json`), so Poly's
//! library never exports a name Apple's does not.

use crate::initramfs::{self, Tree};
use crate::programs::run_as_init;
use crate::symbols::Symbols;
use crate::{Result, cargo, ntdll, root, step};
use poly_macho::{DylibKind, MachO};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// One library: its feature in the crate, file name (the SDK map's
/// component without `.dylib`), install name, versions, and the libraries
/// it links against (by file name).
struct Lib {
    feature: &'static str,
    name: &'static str,
    install: &'static str,
    compatibility: &'static str,
    current: &'static str,
    links: &'static [&'static str],
}

/// The libraries in link order (each after those it links against).
/// Versions: the SDK's `.tbd` files (`current-version`; compatibility 1.0.0
/// where the file gives none, libdyld 0).
const LIBS: [Lib; 7] = [
    Lib {
        feature: "kernel",
        name: "libsystem_kernel",
        install: "/usr/lib/system/libsystem_kernel.dylib",
        compatibility: "1",
        current: "13432.41.4",
        links: &[],
    },
    Lib {
        feature: "platform",
        name: "libsystem_platform",
        install: "/usr/lib/system/libsystem_platform.dylib",
        compatibility: "1",
        current: "402.40.3",
        links: &[],
    },
    Lib {
        feature: "dyld",
        name: "libdyld",
        install: "/usr/lib/system/libdyld.dylib",
        compatibility: "0",
        current: "0",
        links: &[],
    },
    Lib {
        feature: "pthread",
        name: "libsystem_pthread",
        install: "/usr/lib/system/libsystem_pthread.dylib",
        compatibility: "1",
        current: "553.40.2",
        links: &[],
    },
    Lib {
        feature: "malloc",
        name: "libsystem_malloc",
        install: "/usr/lib/system/libsystem_malloc.dylib",
        compatibility: "1",
        current: "886.41.1",
        links: &["libsystem_kernel"],
    },
    Lib {
        feature: "c",
        name: "libsystem_c",
        install: "/usr/lib/system/libsystem_c.dylib",
        compatibility: "1",
        current: "1786.40.6",
        links: &["libsystem_kernel", "libsystem_malloc", "libdyld"],
    },
    Lib {
        feature: "system",
        name: "libSystem.B",
        install: "/usr/lib/libSystem.B.dylib",
        compatibility: "1",
        current: "1359.40.3",
        links: &[],
    },
];

/// The crate.
const CRATE: &str = "personalities/xnu/libsystem";
/// The Rust target.
const TARGET: &str = "x86_64-apple-darwin";
/// The oldest macOS the dylibs say they run on (Chrome's minimum, ADR 0157).
const MIN_MACOS: &str = "13.0";
/// The compiler flags: the baseline x86-64 instruction set (the Rust
/// target's default CPU assumes SSSE3, which QEMU's default CPU lacks).
const RUSTFLAGS: &str = "-Ctarget-cpu=x86-64";
/// The link flags every library shares (part of the cache key).
const LINK_FLAGS: &[&str] = &["-flavor", "darwin", "-arch", "x86_64", "-dylib", "-fixup_chains", "-dead_strip"];

/// Where each library goes in a Poly file tree.
fn tree_path(lib: &Lib) -> String {
    lib.install.trim_start_matches('/').to_string()
}

/// `cargo xtask libsystem [--out DIR] [--cache-key]`.
pub fn command(args: &[String]) -> Result {
    if args.iter().any(|a| a == "--cache-key") {
        println!("libsystem-{:016x}", hash_inputs(&root())?);
        return Ok(());
    }
    let dir = ensure()?;
    if let Some(out) = args.windows(2).find(|w| w[0] == "--out").map(|w| PathBuf::from(&w[1])) {
        for lib in &LIBS {
            let to = out.join(tree_path(lib));
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::copy(dir.join(format!("{}.dylib", lib.name)), &to)
                .map_err(|e| format!("{}: {e}", to.display()))?;
        }
    }
    for lib in &LIBS {
        let p = dir.join(format!("{}.dylib", lib.name));
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let exports = read_exports(lib)?.len();
        eprintln!("{:<56} {size:>7} bytes, {exports:>3} exports", lib.install);
    }
    Ok(())
}

/// The libraries as files of a Poly tree: (path without the leading `/`,
/// bytes).
pub fn tree_files() -> Result<Vec<(String, Vec<u8>)>> {
    let dir = ensure()?;
    LIBS.iter()
        .map(|lib| {
            let p = dir.join(format!("{}.dylib", lib.name));
            Ok((tree_path(lib), std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?))
        })
        .collect()
}

/// One `ensure` at a time in this process.
static ENSURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The directory with the up-to-date dylibs: from the cache, or built,
/// verified and cached.
pub fn ensure() -> Result<PathBuf> {
    let _one = ENSURING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = root();
    let key = hash_inputs(&root)?;
    let cache = ntdll::cache_dir(&root).with_file_name("libsystem-cache");
    let cached = cache.join(format!("{key:016x}"));
    if cached.join("libSystem.B.dylib").is_file() {
        return Ok(cached);
    }
    let started = std::time::Instant::now();
    let work = root.join("target").join("libsystem");
    let out = work.join("out");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    build(&work, &out)?;
    verify(&out)?;
    // Copy under a unique name and rename, so a parallel worktree never
    // sees half a directory.
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let tmp = cache.join(format!(".{key:016x}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    for lib in &LIBS {
        let f = format!("{}.dylib", lib.name);
        std::fs::copy(out.join(&f), tmp.join(&f)).map_err(|e| format!("{f}: {e}"))?;
    }
    if std::fs::rename(&tmp, &cached).is_err() {
        // Another process cached the same key first.
        let _ = std::fs::remove_dir_all(&tmp);
    }
    crate::progress::note(&format!("libSystem built in {:.0?}, cached as {}", started.elapsed(), cached.display()));
    Ok(cached)
}

/// The files the dylibs are built from: the crate with its export lists,
/// the crates it uses, the toolchain pin and this file.
fn inputs(root: &Path) -> Result<Vec<PathBuf>> {
    ntdll::inputs_from(root, &[CRATE], &["rust-toolchain.toml", "tools/xtask/src/libsystem.rs"])
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

/// Build every library's archive and link the dylibs into `out`.
fn build(work: &Path, out: &Path) -> Result {
    let manifest = root().join(CRATE).join("Cargo.toml");
    let vendor = ntdll::vendor_dir()?;
    let target_dir = work.join("cargo");
    let archives = work.join("archives");
    std::fs::create_dir_all(&archives).map_err(|e| format!("{}: {e}", archives.display()))?;
    for lib in &LIBS {
        let mut cmd = cargo();
        cmd.env("RUSTC_BOOTSTRAP", "1")
            .env("CARGO_INCREMENTAL", "0")
            .env("RUSTFLAGS", RUSTFLAGS)
            .arg("build")
            .arg("--release")
            .arg("--manifest-path")
            .arg(&manifest)
            .args(["--features", lib.feature])
            .args(["-Zbuild-std=core,compiler_builtins", "--target", TARGET])
            .arg("--target-dir")
            .arg(&target_dir)
            .arg("--offline")
            .arg("--config")
            .arg("source.crates-io.replace-with=\"rustsrc\"")
            .arg("--config")
            .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()));
        step(&format!("build {} (Rust)", lib.name), &mut cmd)?;
        let a = target_dir.join(TARGET).join("release").join("liblibsystem.a");
        let to = archives.join(format!("{}.a", lib.name));
        std::fs::copy(&a, &to).map_err(|e| format!("{}: {e}", a.display()))?;
    }
    let lld = rust_lld()?;
    for lib in &LIBS {
        let list = root().join(CRATE).join("exports").join(format!("{}.txt", lib.name));
        let mut cmd = Command::new(&lld);
        cmd.args(LINK_FLAGS)
            .args(["-platform_version", "macos", MIN_MACOS, MIN_MACOS])
            .args(["-install_name", lib.install])
            .args(["-compatibility_version", lib.compatibility, "-current_version", lib.current])
            .arg("-force_load")
            .arg(archives.join(format!("{}.a", lib.name)))
            .arg("-exported_symbols_list")
            .arg(&list)
            .arg("-o")
            .arg(out.join(format!("{}.dylib", lib.name)));
        for dep in lib.links {
            cmd.arg(out.join(format!("{dep}.dylib")));
        }
        if lib.feature == "system" {
            for other in LIBS.iter().filter(|l| l.feature != "system") {
                cmd.arg("-reexport_library").arg(out.join(format!("{}.dylib", other.name)));
            }
        }
        step(&format!("link {}.dylib (rust-lld, Mach-O)", lib.name), &mut cmd)?;
    }
    Ok(())
}

/// Clippy of the crate for its target, every library's module at once
/// (`cargo xtask ci lint`).
pub fn lint() -> Result {
    let vendor = ntdll::vendor_dir()?;
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTFLAGS", RUSTFLAGS)
        .arg("clippy")
        .arg("--manifest-path")
        .arg(root().join(CRATE).join("Cargo.toml"))
        .args(["--features", &LIBS.map(|l| l.feature).join(",")])
        .args(["-Zbuild-std=core,compiler_builtins", "--target", TARGET])
        .arg("--target-dir")
        .arg(root().join("target").join("libsystem").join("clippy"))
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()))
        .args(["--", "-D", "warnings"]);
    step("clippy (libsystem)", &mut cmd)
}

/// The names of `exports/<library>.txt`.
fn read_exports(lib: &Lib) -> Result<BTreeSet<String>> {
    let p = root().join(CRATE).join("exports").join(format!("{}.txt", lib.name));
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

/// The SDK's export names of `component` (`coverage/macos/api`), as the
/// map keeps them: the C name, a Mach-O name without its leading `_`.
fn sdk_names(component: &str) -> Result<BTreeSet<String>> {
    let p = root().join("coverage/macos/api").join(format!("{component}.json"));
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("symbol").and_then(|s| s.as_str()).map(str::to_string))
        .collect())
}

/// A Mach-O symbol's name in the SDK map.
fn map_name(symbol: &str) -> &str {
    symbol.strip_prefix('_').unwrap_or(symbol)
}

/// Read every dylib back and check it (see the module documentation).
fn verify(dir: &Path) -> Result {
    let ours: BTreeSet<&str> = LIBS.iter().map(|l| l.install).collect();
    for lib in &LIBS {
        let p = dir.join(format!("{}.dylib", lib.name));
        let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let err = |what: String| format!("{}: {what}", p.display());
        let m = MachO::parse(&bytes).map_err(|e| err(format!("not a Mach-O file: {e:?}")))?;
        let id = m.dylib_id().ok_or_else(|| err("no LC_ID_DYLIB".into()))?;
        if id.name != lib.install {
            return Err(err(format!("install name {}, expected {}", id.name, lib.install)));
        }
        let version = |v: poly_macho::Version| {
            let s = format!("{}.{}.{}", v.major(), v.minor(), v.patch());
            s.trim_end_matches(".0").trim_end_matches(".0").to_string()
        };
        if version(id.current_version) != lib.current || version(id.compatibility_version) != lib.compatibility {
            return Err(err(format!(
                "versions {} / {}, expected {} / {}",
                version(id.current_version),
                version(id.compatibility_version),
                lib.current,
                lib.compatibility
            )));
        }
        let mut reexports = BTreeSet::new();
        for d in m.dylibs() {
            if !ours.contains(d.name) {
                return Err(err(format!("loads {}, which is not one of Poly's libSystem libraries", d.name)));
            }
            if d.kind == DylibKind::Reexport {
                reexports.insert(d.name);
            }
        }
        let want_reexports: BTreeSet<&str> = if lib.feature == "system" {
            LIBS.iter().filter(|l| l.feature != "system").map(|l| l.install).collect()
        } else {
            BTreeSet::new()
        };
        if reexports != want_reexports {
            return Err(err(format!("re-exports {reexports:?}, expected {want_reexports:?}")));
        }
        let mut exported = BTreeSet::new();
        if let Some(ex) = m.exports().map_err(|e| err(format!("exports: {e:?}")))? {
            ex.walk(|name, _| {
                exported.insert(String::from_utf8_lossy(name).into_owned());
                std::ops::ControlFlow::Continue(())
            })
            .map_err(|e| err(format!("export trie: {e:?}")))?;
        }
        let listed = read_exports(lib)?;
        if exported != listed {
            let missing: Vec<_> = listed.difference(&exported).collect();
            let extra: Vec<_> = exported.difference(&listed).collect();
            return Err(err(format!("exports differ from the list: missing {missing:?}, not listed {extra:?}")));
        }
        let sdk = sdk_names(&format!("{}.dylib", lib.name))?;
        let foreign: Vec<_> = listed.iter().filter(|s| !sdk.contains(map_name(s))).collect();
        if !foreign.is_empty() {
            return Err(err(format!("exports names the SDK's {} does not: {foreign:?}", lib.name)));
        }
    }
    Ok(())
}

/// The C program of the boot test (`tests/macos/libsystem`), linked by
/// Apple's linker against the SDK's `libSystem.tbd`, and what it prints.
const HELLO: &str = "tests/macos/libsystem/hello_x64";
const HELLO_LINES: &[&str] = &[
    "hello, world",
    "ok printf: 42 -7 2a 0x2a 3.14 1.5e+03 [  pad] [left ] (null) %",
    "ok positional printf: a b 2.5",
    "ok unknown conversions print their letter: y",
    "ok puts and putchar",
    "ok malloc, calloc, realloc and free",
    "ok malloc_size and malloc_good_size",
    "ok snprintf truncates: 9 \"truncat\"",
    "ok asprintf: \"asprintf 7\"",
    "ok errno is EBADF after write(-1)",
    "ok strtol, atoi, strcmp, strlen, memcmp",
    "ok getenv",
    "ok _NSGetExecutablePath and _dyld_image_count",
    "ok _NSGetArgc and getprogname",
    "ok the stack protector's guard is set",
    "ok fprintf to stderr",
    "ok posix_spawn passes argv to the child",
    "hello: done",
    "atexit: last registered runs first",
    "atexit: then the first",
    "no newline, flushed by exit",
];

/// The boot test group `libsystem`: the SDK-linked C program runs as the
/// first process with Poly's libSystem in `/usr/lib`.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let hello = std::fs::read(root().join(HELLO)).map_err(|e| format!("{HELLO}: {e}"))?;
    let programs: Vec<(&str, &[u8])> = vec![("mhello-libsystem", &hello)];
    let mut tree = Tree::default();
    for (path, bytes) in tree_files()? {
        tree.files.push((path, 0o755, bytes));
    }
    let image =
        initramfs::write(&root(), "poly-libsystem-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))?;
    eprintln!("==> boot test: an SDK-linked x86-64 C program with Poly's libSystem.B.dylib");
    let b = run_as_init(kernel, syms, &image, "mhello-libsystem", "no newline, flushed by exit")?;
    let mut at = 0;
    for line in HELLO_LINES {
        match b.output[at..].find(line) {
            Some(i) => at += i + line.len(),
            None => return Err(format!("/bin/mhello-libsystem: {line:?} missing or out of order")),
        }
    }
    if let Some(line) = b.output.lines().find(|l| l.contains("not ok ")) {
        return Err(format!("/bin/mhello-libsystem: {line}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every listed export is a name of the SDK's library of the same name,
    /// and the libraries link only libraries before them.
    #[test]
    fn export_lists_name_only_sdk_symbols() {
        for lib in &LIBS {
            let sdk = sdk_names(&format!("{}.dylib", lib.name)).unwrap();
            for s in read_exports(lib).unwrap() {
                assert!(sdk.contains(map_name(&s)), "{}: {s} is not in the SDK's list", lib.name);
            }
        }
        for (i, lib) in LIBS.iter().enumerate() {
            for dep in lib.links {
                assert!(LIBS[..i].iter().any(|l| l.name == *dep), "{} links {dep} before it is built", lib.name);
            }
        }
    }

    #[test]
    fn inputs_cover_the_crate_and_its_logic() {
        let files = inputs(&root()).unwrap();
        for want in [
            "personalities/xnu/libsystem/src/lib.rs",
            "personalities/xnu/libsystem/exports/libsystem_c.txt",
            "crates/poly-libsystem/src/printf.rs",
            "rust-toolchain.toml",
        ] {
            assert!(files.iter().any(|f| f == Path::new(want)), "{want} not an input");
        }
    }
}
