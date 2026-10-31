//! Build our Win32 modules above `ntdll` (ADR 0079) — `kernelbase.dll`,
//! `kernel32.dll` and the test program `k32test.exe`, plus `advapi32.dll`
//! and its test program `a32test.exe` (ADR 0083), the C runtime (ADR 0084),
//! `user32.dll` and `oleaut32.dll` (ADR 0087), `bcrypt.dll`, `crypt32.dll`
//! and `cryptest.exe` (ADR 0096) — and check them.
//!
//! They are one Cargo workspace, `personalities/nt/win32`, built the way
//! `ntdll` is (ADR 0071, `ntdll.rs`): `no_std` Rust for
//! `x86_64-pc-windows-msvc`, `-Zbuild-std` from the toolchain's `rust-src`,
//! linked by `rust-lld` in its `lld-link` flavour. One `cargo build`
//! compiles `core`/`alloc` for the target once for all three.
//!
//! # Delivery: built once per source version, cached by content hash
//!
//! As for `ntdll`: [`ensure`] hashes everything the modules are built from
//! (the four crates, the repo crates they reach through path dependencies,
//! the workspace manifest and lock file, the toolchain pin and the flags)
//! and looks for `win32-<hash>/` in the cache next to `ntdll`'s (`<main
//! checkout>/target/win32-cache`, shared by every worktree). Only a miss
//! builds (about 40 s and 150 MB of build-std artifacts, deleted
//! afterwards unless `POLY_WIN32_KEEP_BUILD=1`); the three files (~150 KB)
//! stay. The kernel's `build.rs` embeds them from the directory
//! `POLY_WIN32_DIR` names, which [`crate::kernel_cargo`] sets.

use crate::{Result, cargo, root, step};
use std::path::{Path, PathBuf};

/// The build's flags, part of the cache key. Entry points and subsystems
/// come from each crate's `build.rs`.
const RUSTFLAGS: &str = "-Clinker=rust-lld -Clinker-flavor=lld-link -Clink-arg=/NODEFAULTLIB";
/// The environment variable that gives the kernel's `build.rs` the
/// directory with the three modules.
pub const KERNEL_ENV: &str = "POLY_WIN32_DIR";
/// The workspace.
const WORKSPACE: &str = "personalities/nt/win32";
/// What the build produces (in the target's release directory).
pub const OUTPUTS: [&str; 16] = [
    "kernelbase.dll",
    "kernel32.dll",
    "k32test.exe",
    "contest.exe",
    "advapi32.dll",
    "a32test.exe",
    "ucrtbase.dll",
    "msvcrt.dll",
    "crttest.exe",
    "crtloctest.exe",
    "user32.dll",
    "oleaut32.dll",
    "bcrypt.dll",
    "crypt32.dll",
    "cryptest.exe",
    // ADR 0093.
    "crtiotest.exe",
];
/// Cached versions kept.
const CACHE_KEEP: usize = 6;

/// `cargo xtask win32 [--clippy]`: make sure the cached modules are up to
/// date (building them if not) and verify them; `--clippy` lints the
/// workspace for the Windows target instead (into `target/win32-clippy`,
/// deleted afterwards).
pub fn command(args: &[String]) -> Result {
    if args.iter().any(|a| a == "--clippy") {
        let target = root().join("target").join("win32-clippy");
        let mut cmd = cargo_for(&target, "clippy")?;
        cmd.args(["--", "-D", "warnings"]);
        let r = step("clippy (win32)", &mut cmd);
        let _ = std::fs::remove_dir_all(&target);
        return r;
    }
    let dir = ensure()?;
    verify(&dir)?;
    for f in OUTPUTS {
        let p = dir.join(f);
        eprintln!("{}: {} bytes", p.display(), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0));
    }
    Ok(())
}

/// The directory with the up-to-date modules: from the cache, or built,
/// verified and cached.
pub fn ensure() -> Result<PathBuf> {
    let root = root();
    let files = crate::ntdll::inputs_from(
        &root,
        &[
            "personalities/nt/win32/rt",
            "personalities/nt/win32/kernelbase",
            "personalities/nt/win32/kernel32",
            "personalities/nt/win32/k32test",
            "personalities/nt/win32/contest",
            "personalities/nt/win32/advapi32",
            "personalities/nt/win32/a32test",
            "personalities/nt/win32/crt",
            "personalities/nt/win32/ucrtbase",
            "personalities/nt/win32/msvcrt",
            "personalities/nt/win32/crttest",
            "personalities/nt/win32/crtloctest",
            "personalities/nt/win32/user32",
            "personalities/nt/win32/oleaut32",
            "personalities/nt/win32/cryptrt",
            "personalities/nt/win32/bcrypt",
            "personalities/nt/win32/crypt32",
            "personalities/nt/win32/cryptest",
            "personalities/nt/win32/crtiotest",
        ],
        &[
            "personalities/nt/win32/Cargo.toml",
            "personalities/nt/win32/Cargo.lock",
            "personalities/nt/src/services.rs",
            "rust-toolchain.toml",
        ],
    )?;
    let key = crate::ntdll::hash_files(&root, RUSTFLAGS, &files)?;
    let cache = cache_dir(&root);
    let cached = cache.join(format!("win32-{key:016x}"));
    if OUTPUTS.iter().all(|f| cached.join(f).is_file()) {
        return Ok(cached);
    }
    let target = root.join("target").join("win32");
    let started = std::time::Instant::now();
    let built = build(&target)?;
    verify(&built)?;
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    // Fill a private directory and rename it, so a parallel worktree never
    // sees half a set.
    let tmp = cache.join(format!(".win32-{key:016x}.{}.tmp", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    for f in OUTPUTS {
        std::fs::copy(built.join(f), tmp.join(f)).map_err(|e| format!("{f}: {e}"))?;
    }
    if std::fs::rename(&tmp, &cached).is_err() {
        // Another worktree cached the same version first.
        let _ = std::fs::remove_dir_all(&tmp);
    }
    if std::env::var_os("POLY_WIN32_KEEP_BUILD").is_none() {
        let _ = std::fs::remove_dir_all(&target);
    }
    prune(&cache);
    eprintln!("==> Win32 modules built in {:.0?}, cached as {}", started.elapsed(), cached.display());
    Ok(cached)
}

/// `win32-cache`, beside `ntdll`'s cache.
fn cache_dir(root: &Path) -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(root);
    ntdll.parent().map_or_else(|| root.join("target"), Path::to_path_buf).join("win32-cache")
}

/// Keep the [`CACHE_KEEP`] newest versions.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut sets: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("win32-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    sets.sort();
    let excess = sets.len().saturating_sub(CACHE_KEEP);
    for (_, p) in sets.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// Build the workspace into `target_dir`; the directory with the outputs.
fn build(target_dir: &Path) -> Result<PathBuf> {
    let mut cmd = cargo_for(target_dir, "build")?;
    step("build kernelbase.dll, kernel32.dll, advapi32.dll, ucrtbase.dll, msvcrt.dll and the test programs", &mut cmd)?;
    let out = target_dir.join("x86_64-pc-windows-msvc/release");
    for f in OUTPUTS {
        if !out.join(f).is_file() {
            return Err(format!("{} was not produced", out.join(f).display()));
        }
    }
    Ok(out)
}

/// `cargo <verb>` for the workspace and the Windows target, with
/// `-Zbuild-std` from the toolchain's `rust-src` (ADR 0071).
fn cargo_for(target_dir: &Path, verb: &str) -> Result<std::process::Command> {
    let manifest = root().join(WORKSPACE).join("Cargo.toml");
    let vendor = crate::ntdll::vendor_dir()?;
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTFLAGS", RUSTFLAGS)
        .arg(verb)
        .arg("--release")
        .arg("--workspace")
        .arg("--manifest-path")
        .arg(&manifest)
        .args(["-Zbuild-std=core,alloc,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--target", "x86_64-pc-windows-msvc"])
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()));
    Ok(cmd)
}

fn parse(bytes: &[u8]) -> Result<poly_pe::Pe<'_>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE: {e:?}"))?;
    if pe.info().machine() != poly_pe::machine::AMD64 || pe.info().bits() != poly_pe::Bits::Pe32Plus {
        return Err("not an x64 PE32+ image".into());
    }
    Ok(pe)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Check the three modules: `kernelbase` exports every function `kernel32`
/// forwards, as code; `kernel32` forwards each to its API-set contract and
/// exports its own functions as code; `k32test.exe` is an executable that
/// imports from `kernel32.dll` only.
fn verify(dir: &Path) -> Result {
    use poly_kernelbase::exports::{KERNEL32_FORWARDS, KERNEL32_OWN, forwarder, kernelbase_exports};
    use poly_pe::ExportTarget;
    let kb = read(&dir.join("kernelbase.dll"))?;
    let kb = parse(&kb)?;
    let kb_exports = kb.exports().map_err(|e| format!("kernelbase exports: {e:?}"))?.ok_or("kernelbase: no exports")?;
    let mut missing = Vec::new();
    for name in kernelbase_exports() {
        match kb_exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))? {
            Some(ExportTarget::Rva(rva)) if rva < kb.info().size_of_image() => {}
            _ => missing.push(name),
        }
    }
    if !missing.is_empty() {
        return Err(format!("kernelbase.dll lacks exports: {}", missing.join(", ")));
    }
    let k32 = read(&dir.join("kernel32.dll"))?;
    let k32 = parse(&k32)?;
    let k32_exports = k32.exports().map_err(|e| format!("kernel32 exports: {e:?}"))?.ok_or("kernel32: no exports")?;
    for &(name, _) in KERNEL32_FORWARDS.iter().chain(poly_kernelbase::exports::KERNEL32_NTDLL_FORWARDS) {
        let want = forwarder(name).unwrap_or_default();
        match k32_exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))? {
            Some(ExportTarget::Forwarder(f)) if f.raw == want.as_bytes() => {}
            other => return Err(format!("kernel32!{name} should forward to {want}, is {other:?}")),
        }
    }
    for name in KERNEL32_OWN {
        match k32_exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))? {
            Some(ExportTarget::Rva(_)) => {}
            other => return Err(format!("kernel32!{name} should be code, is {other:?}")),
        }
    }
    // The test programs (ADR 0079, 0080, 0083).
    verify_exe(dir, "k32test.exe", &["kernel32.dll"])?;
    verify_exe(dir, "contest.exe", &["kernel32.dll"])?;
    verify_advapi32(dir)?;
    verify_exe(dir, "a32test.exe", &["advapi32.dll", "kernel32.dll"])?;
    verify_crt(dir)?;
    verify_user32_oleaut32(dir)?;
    verify_crypto(dir)
}

/// `bcrypt` and `crypt32` export their functions by name; `cryptest.exe`
/// imports from them and `kernel32` only (ADR 0096).
fn verify_crypto(dir: &Path) -> Result {
    use poly_pe::ExportTarget;
    for (file, names) in
        [("bcrypt.dll", poly_wincrypt::BCRYPT_EXPORTS), ("crypt32.dll", poly_wincrypt::CRYPT32_EXPORTS)]
    {
        let bytes = read(&dir.join(file))?;
        let dll = parse(&bytes)?;
        let exports =
            dll.exports().map_err(|e| format!("{file} exports: {e:?}"))?.ok_or(format!("{file}: no exports"))?;
        for name in names {
            if !matches!(exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))?, Some(ExportTarget::Rva(_))) {
                return Err(format!("{file} lacks {name}"));
            }
        }
    }
    verify_exe(dir, "cryptest.exe", &["bcrypt.dll", "crypt32.dll", "kernel32.dll"])
}

/// `user32` exports its functions by name; `oleaut32` exports each of its
/// functions at its Windows ordinal, the way programs import them (ADR
/// 0087).
fn verify_user32_oleaut32(dir: &Path) -> Result {
    use poly_kernelbase::exports::{OLEAUT32_EXPORTS, USER32_EXPORTS};
    use poly_pe::ExportTarget;
    let bytes = read(&dir.join("user32.dll"))?;
    let dll = parse(&bytes)?;
    let exports = dll.exports().map_err(|e| format!("user32 exports: {e:?}"))?.ok_or("user32: no exports")?;
    for name in USER32_EXPORTS {
        if !matches!(exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))?, Some(ExportTarget::Rva(_))) {
            return Err(format!("user32.dll lacks {name}"));
        }
    }
    let bytes = read(&dir.join("oleaut32.dll"))?;
    let dll = parse(&bytes)?;
    let exports = dll.exports().map_err(|e| format!("oleaut32 exports: {e:?}"))?.ok_or("oleaut32: no exports")?;
    for &(name, ordinal) in OLEAUT32_EXPORTS {
        let by_name = exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))?;
        let by_ordinal = exports.by_ordinal(u32::from(ordinal)).map_err(|e| format!("{e:?}"))?;
        if by_name.is_none() || by_name != by_ordinal {
            return Err(format!("oleaut32.dll does not export {name} at ordinal {ordinal}"));
        }
    }
    Ok(())
}

/// An executable that imports only from `allowed`.
fn verify_exe(dir: &Path, file: &str, allowed: &[&str]) -> Result {
    let exe = read(&dir.join(file))?;
    let exe = parse(&exe)?;
    if exe.info().is_dll() {
        return Err(format!("{file} is a DLL"));
    }
    let imports =
        exe.imports().map_err(|e| format!("{file} imports: {e:?}"))?.ok_or(format!("{file} imports nothing"))?;
    for d in imports {
        let d = d.map_err(|e| format!("{e:?}"))?;
        let name = d.dll_name().map_err(|e| format!("{e:?}"))?;
        if !allowed.iter().any(|a| name.eq_ignore_ascii_case(a.as_bytes())) {
            return Err(format!("{file} imports {}", String::from_utf8_lossy(name)));
        }
    }
    Ok(())
}

/// `advapi32` forwards each of its `kernelbase` functions to its API-set
/// contract and exports its own as code (ADR 0083).
fn verify_advapi32(dir: &Path) -> Result {
    use poly_kernelbase::advapi::{ADVAPI32_FORWARDS, ADVAPI32_OWN, forwarder};
    use poly_pe::ExportTarget;
    let dll = read(&dir.join("advapi32.dll"))?;
    let dll = parse(&dll)?;
    let exports = dll.exports().map_err(|e| format!("advapi32 exports: {e:?}"))?.ok_or("advapi32: no exports")?;
    for &(name, _) in ADVAPI32_FORWARDS {
        let want = forwarder(name).unwrap_or_default();
        match exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))? {
            Some(ExportTarget::Forwarder(f)) if f.raw == want.as_bytes() => {}
            other => return Err(format!("advapi32!{name} should forward to {want}, is {other:?}")),
        }
    }
    for name in ADVAPI32_OWN {
        match exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))? {
            Some(ExportTarget::Rva(_)) => {}
            other => return Err(format!("advapi32!{name} should be code, is {other:?}")),
        }
    }
    Ok(())
}

/// The C runtime (ADR 0084): `ucrtbase.dll` exports what MSVC's start-up
/// code imports, `msvcrt.dll` its classic start-up interface; every export
/// of each is a name the coverage map lists for that DLL (so nothing of
/// one leaks into the other); `crttest.exe` imports the UCRT through
/// `api-ms-win-crt-*` names (and `kernel32.dll`) only.
fn verify_crt(dir: &Path) -> Result {
    use poly_crt::exports::{MSVCRT_STARTUP, UCRT_STARTUP};
    for (dll, required) in [("ucrtbase.dll", UCRT_STARTUP), ("msvcrt.dll", MSVCRT_STARTUP)] {
        let bytes = read(&dir.join(dll))?;
        let pe = parse(&bytes)?;
        let exports = pe.exports().map_err(|e| format!("{dll} exports: {e:?}"))?.ok_or(format!("{dll}: no exports"))?;
        for name in required {
            if exports.by_name(name.as_bytes()).map_err(|e| format!("{e:?}"))?.is_none() {
                return Err(format!("{dll} lacks {name}"));
            }
        }
        let listed = std::fs::read_to_string(root().join("coverage/windows/api").join(format!("{dll}.json")))
            .map_err(|e| format!("the coverage list of {dll}: {e}"))?;
        let mut stray = Vec::new();
        for e in exports.names() {
            let (name, _) = e.map_err(|e| format!("{dll}: {e:?}"))?;
            let name = String::from_utf8_lossy(name);
            // The entry point and `_fltused` are Poly's own (ADR 0071, 0079).
            if name == "_fltused" || name.ends_with("_main") {
                continue;
            }
            if !listed.contains(&format!("\"symbol\":\"{name}\"")) {
                stray.push(name.into_owned());
            }
        }
        if !stray.is_empty() {
            return Err(format!("{dll} exports names it does not have on Windows: {}", stray.join(", ")));
        }
    }
    let exe = read(&dir.join("crttest.exe"))?;
    let exe = parse(&exe)?;
    let imports =
        exe.imports().map_err(|e| format!("crttest.exe imports: {e:?}"))?.ok_or("crttest.exe imports nothing")?;
    for d in imports {
        let d = d.map_err(|e| format!("{e:?}"))?;
        let name = d.dll_name().map_err(|e| format!("{e:?}"))?.to_ascii_lowercase();
        if !(name.starts_with(b"api-ms-win-crt-") || name == b"kernel32.dll") {
            return Err(format!("crttest.exe imports {}", String::from_utf8_lossy(&name)));
        }
    }
    Ok(())
}
