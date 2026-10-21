//! Build our `ntdll.dll` (ADR 0071) and check it is a real PE32+ DLL
//! exporting the names applications import.
//!
//! `ntdll` is a `no_std` + `alloc` crate compiled for
//! `x86_64-pc-windows-msvc` and linked by `rust-lld` (the `lld-link`
//! flavour). The pinned toolchain has no prebuilt standard library for that
//! target, so the build uses `-Zbuild-std` (under `RUSTC_BOOTSTRAP=1`, the
//! only unstable use in the repo, confined here) and vendors `core`/`alloc`
//! dependencies from the toolchain's own `rust-src`. `_fltused` and
//! `__chkstk` are provided by the crate.
//!
//! # Delivery: built once per source version, cached by content hash
//!
//! The kernel embeds the DLL (ADR 0071). Every kernel build `xtask` runs
//! first calls [`ensure`], which hashes everything the DLL is built from
//! ([`inputs`]: the `ntdll` crate, the repo crates it reaches through
//! `Cargo.toml` path dependencies, the service table its stubs come from,
//! the toolchain pin and the build flags) and looks for `ntdll-<hash>.dll`
//! in the cache. Only a miss builds (about 30 s and 140 MB of build-std
//! artifacts); the artifacts are then deleted (`POLY_NTDLL_KEEP_BUILD=1`
//! keeps them for incremental work on `ntdll`) and only the ~100 KB DLL
//! stays. The cache is shared by every worktree of the checkout (`<main
//! checkout>/target/ntdll-cache`, found through `git rev-parse
//! --git-common-dir`), so parallel agents build each version once. The
//! kernel's `build.rs` reads the DLL from `POLY_NTDLL_DLL`, which
//! [`crate::kernel_cargo`] sets.

use crate::{Result, cargo, root, step};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The flags of the build, part of the cache key.
const RUSTFLAGS: &str =
    "-Clinker=rust-lld -Clinker-flavor=lld-link -Clink-arg=/NODEFAULTLIB -Clink-arg=/SUBSYSTEM:CONSOLE";
/// Cached DLLs kept (the newest), so switching branches rarely rebuilds.
const CACHE_KEEP: usize = 8;
/// The environment variable that gives the kernel's `build.rs` the DLL.
pub const KERNEL_ENV: &str = "POLY_NTDLL_DLL";

/// Exports every modern Windows program expects from `ntdll`; the check
/// fails if the built DLL is missing one.
const REQUIRED: &[&str] = &[
    // The loader and process start.
    "LdrInitializeThunk",
    "LdrLoadDll",
    "LdrUnloadDll",
    "LdrGetProcedureAddress",
    "LdrGetDllHandle",
    "LdrResolveDelayLoadedAPI",
    "RtlUserThreadStart",
    "RtlExitUserThread",
    "RtlExitUserProcess",
    // The heap.
    "RtlCreateHeap",
    "RtlAllocateHeap",
    "RtlFreeHeap",
    "RtlReAllocateHeap",
    "RtlSizeHeap",
    // Strings, status, version, critical sections.
    "RtlInitUnicodeString",
    "RtlCompareUnicodeString",
    "RtlAnsiStringToUnicodeString",
    "RtlNtStatusToDosError",
    "RtlGetVersion",
    "RtlEnterCriticalSection",
    "RtlLeaveCriticalSection",
    // Structured exception handling (ADR 0068's exports).
    "KiUserExceptionDispatcher",
    "KiUserApcDispatcher",
    "RtlAddVectoredExceptionHandler",
    "RtlRemoveVectoredExceptionHandler",
    "RtlLookupFunctionEntry",
    "RtlVirtualUnwind",
    "RtlUnwindEx",
    "RtlCaptureContext",
    "RtlRaiseException",
    "__C_specific_handler",
    // System-call stubs (both decorations).
    "NtClose",
    "NtAllocateVirtualMemory",
    "NtCreateFile",
    "NtContinue",
    "ZwClose",
    "ZwAllocateVirtualMemory",
];

/// `cargo xtask ntdll [--out <path>]`: make sure the cached DLL is up to
/// date (building it if not), verify it, and optionally copy it.
pub fn command(args: &[String]) -> Result {
    // The name the DLL is cached under, a hash of its inputs (CI keys its
    // cache of `target/ntdll-cache` with it).
    if args.iter().any(|a| a == "--cache-key") {
        println!("ntdll-{:016x}", hash_inputs(&root())?);
        return Ok(());
    }
    let out = opt(args, "--out");
    let dll = ensure()?;
    if let Some(dest) = out {
        std::fs::copy(&dll, &dest).map_err(|e| format!("copy to {dest}: {e}"))?;
    }
    verify(&dll)?;
    eprintln!(
        "ntdll.dll: {} ({} bytes), every required export present",
        dll.display(),
        std::fs::metadata(&dll).map(|m| m.len()).unwrap_or(0)
    );
    Ok(())
}

/// The up-to-date `ntdll.dll`: from the cache, or built, verified and
/// cached.
pub fn ensure() -> Result<PathBuf> {
    let root = root();
    let key = hash_inputs(&root)?;
    let dir = cache_dir(&root);
    let cached = dir.join(format!("ntdll-{key:016x}.dll"));
    if cached.is_file() {
        return Ok(cached);
    }
    let target = root.join("target").join("ntdll");
    let started = std::time::Instant::now();
    let dll = build(target.clone())?;
    verify(&dll)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Write under a unique name and rename, so a parallel worktree never
    // sees half a file.
    let tmp = dir.join(format!(".ntdll-{key:016x}.{}.tmp", std::process::id()));
    std::fs::copy(&dll, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &cached).map_err(|e| format!("{}: {e}", cached.display()))?;
    if std::env::var_os("POLY_NTDLL_KEEP_BUILD").is_none() {
        let _ = std::fs::remove_dir_all(&target);
    }
    prune(&dir);
    eprintln!("==> ntdll.dll built in {:.0?}, cached as {}", started.elapsed(), cached.display());
    Ok(cached)
}

/// The cache directory: in the main checkout's `target/` (shared by its
/// worktrees), else in this checkout's.
pub(crate) fn cache_dir(root: &Path) -> PathBuf {
    let common = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    let main = common.and_then(|c| c.parent().map(Path::to_path_buf)).filter(|m| m.join("Cargo.toml").is_file());
    main.unwrap_or_else(|| root.to_path_buf()).join("target").join("ntdll-cache")
}

/// Keep the [`CACHE_KEEP`] newest DLLs.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut dlls: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".dll"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    dlls.sort();
    let excess = dlls.len().saturating_sub(CACHE_KEEP);
    for (_, p) in dlls.into_iter().take(excess) {
        let _ = std::fs::remove_file(p);
    }
}

/// The files the DLL is built from (relative to the repo root, sorted): the
/// `ntdll` crate, every crate it reaches through path dependencies (direct
/// ones, or `workspace = true` ones the root manifest maps to a path), the
/// service table and the toolchain pin.
pub fn inputs(root: &Path) -> Result<Vec<PathBuf>> {
    inputs_from(root, &["personalities/nt/ntdll"], &["personalities/nt/src/services.rs", "rust-toolchain.toml"])
}

/// The files of the crates `start` and every crate they reach through path
/// dependencies, plus the `extra` files that exist (relative to `root`,
/// sorted). Also used for our other Windows modules (`win32.rs`, ADR 0079).
pub(crate) fn inputs_from(root: &Path, start: &[&str], extra: &[&str]) -> Result<Vec<PathBuf>> {
    let workspace = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let mut crates: Vec<PathBuf> = Vec::new();
    let mut todo: Vec<PathBuf> = start.iter().map(PathBuf::from).collect();
    while let Some(dir) = todo.pop() {
        if crates.contains(&dir) {
            continue;
        }
        let manifest = std::fs::read_to_string(root.join(&dir).join("Cargo.toml"))
            .map_err(|e| format!("{}/Cargo.toml: {e}", dir.display()))?;
        for (path, from_root) in path_dependencies(&manifest, &workspace) {
            let base = if from_root { PathBuf::new() } else { dir.clone() };
            todo.push(normalize(&base.join(path)));
        }
        crates.push(dir);
    }
    let mut files: Vec<PathBuf> = extra.iter().map(PathBuf::from).filter(|f| root.join(f).is_file()).collect();
    for c in &crates {
        walk(root, c, &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The `[dependencies]` of a manifest that are paths: (path, whether it is
/// relative to the repo root rather than to the crate).
fn path_dependencies(manifest: &str, workspace: &str) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = line == "[dependencies]";
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some(p) = quoted_after(line, "path = \"") {
            out.push((PathBuf::from(p), false));
        } else if line.contains("workspace = true")
            && let Some(name) = line.split('=').next().map(str::trim)
        {
            let prefix = format!("{name} = ");
            let entry = workspace.lines().map(str::trim).find(|l| l.starts_with(&prefix));
            if let Some(p) = entry.and_then(|l| quoted_after(l, "path = \"")) {
                out.push((PathBuf::from(p), true));
            }
        }
    }
    out
}

fn quoted_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(key)? + key.len()..];
    Some(&rest[..rest.find('"')?])
}

/// `a/b/../c` as `a/c`.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Every file below `dir` (relative to `root`), skipping `target/`.
fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result {
    let rd = std::fs::read_dir(root.join(dir)).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in rd.flatten() {
        let rel = dir.join(e.file_name());
        let ty = e.file_type().map_err(|e| format!("{}: {e}", rel.display()))?;
        if ty.is_dir() {
            if e.file_name() != "target" {
                walk(root, &rel, out)?;
            }
        } else if ty.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}

/// FNV-1a over the build flags and every input's path and bytes.
fn hash_inputs(root: &Path) -> Result<u64> {
    hash_files(root, RUSTFLAGS, &inputs(root)?)
}

/// FNV-1a over `flags` and every file's path and bytes.
pub(crate) fn hash_files(root: &Path, flags: &str, files: &[PathBuf]) -> Result<u64> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes.iter().chain(&[0xFF]) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(flags.as_bytes());
    for f in files {
        eat(f.to_string_lossy().as_bytes());
        eat(&std::fs::read(root.join(f)).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    Ok(h)
}

/// The toolchain's `rust-src` vendored-dependency directory
/// (`<sysroot>/lib/rustlib/src/rust/library/vendor`).
pub(crate) fn vendor_dir() -> Result<PathBuf> {
    let out = Command::new("rustc").args(["--print", "sysroot"]).output().map_err(|e| format!("rustc: {e}"))?;
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let dir = PathBuf::from(sysroot).join("lib/rustlib/src/rust/library/vendor");
    if !dir.is_dir() {
        return Err(format!("{} not found: install the rust-src component", dir.display()));
    }
    Ok(dir)
}

/// Build `ntdll.dll` into `target_dir` and return its path.
pub fn build(target_dir: PathBuf) -> Result<PathBuf> {
    let manifest = root().join("personalities/nt/ntdll/Cargo.toml");
    let vendor = vendor_dir()?;
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTFLAGS", RUSTFLAGS)
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&manifest)
        .args(["-Zbuild-std=core,alloc,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--target", "x86_64-pc-windows-msvc"])
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()));
    step("build ntdll.dll", &mut cmd)?;
    let dll = target_dir.join("x86_64-pc-windows-msvc/release/ntdll.dll");
    if !dll.is_file() {
        return Err(format!("{} was not produced", dll.display()));
    }
    Ok(dll)
}

/// Parse the DLL with `poly-pe` and check every [`REQUIRED`] name is
/// exported.
fn verify(dll: &Path) -> Result {
    let bytes = std::fs::read(dll).map_err(|e| format!("read {}: {e}", dll.display()))?;
    let pe = poly_pe::Pe::parse(&bytes).map_err(|e| format!("not a PE: {e:?}"))?;
    if pe.info().machine() != poly_pe::machine::AMD64 || pe.info().bits() != poly_pe::Bits::Pe32Plus {
        return Err("not an x64 PE32+ image".into());
    }
    if !pe.info().is_dll() {
        return Err("not a DLL".into());
    }
    let exports = pe.exports().map_err(|e| format!("exports: {e:?}"))?.ok_or("no export directory")?;
    let names: std::collections::BTreeSet<Vec<u8>> =
        exports.names().filter_map(|r| r.ok()).map(|(n, _)| n.to_vec()).collect();
    let missing: Vec<&str> = REQUIRED.iter().copied().filter(|n| !names.contains(n.as_bytes())).collect();
    if !missing.is_empty() {
        return Err(format!("ntdll.dll is missing exports: {}", missing.join(", ")));
    }
    // A forwarder- and ordinal-free sanity check: RtlAllocateHeap resolves
    // to code inside the image.
    match exports.by_name(b"RtlAllocateHeap").map_err(|e| format!("{e:?}"))? {
        Some(poly_pe::ExportTarget::Rva(rva)) if rva < pe.info().size_of_image() => Ok(()),
        other => Err(format!("RtlAllocateHeap export is wrong: {other:?}")),
    }
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inputs cover the `ntdll` crate and every repo crate it uses, and
    /// nothing of the kernel.
    #[test]
    fn inputs_follow_path_dependencies() {
        let files = inputs(&root()).unwrap();
        for want in [
            "personalities/nt/ntdll/src/lib.rs",
            "personalities/nt/ntdll/build.rs",
            "crates/poly-ldr/src/loader.rs",
            "crates/poly-rtl/Cargo.toml",
            "crates/poly-seh/src/lib.rs",
            "crates/poly-pe/src/lib.rs",
            "crates/poly-ntobj/src/lib.rs",
            "personalities/nt/src/services.rs",
            "rust-toolchain.toml",
        ] {
            assert!(files.contains(&PathBuf::from(want)), "{want} is not an input");
        }
        assert!(!files.iter().any(|f| f.starts_with("kernel")));
    }
}
