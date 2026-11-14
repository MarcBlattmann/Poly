//! Build Poly's 32-bit Windows modules (ADR 0186): the i686 `ntdll.dll`, `kernelbase.dll` and
//! `kernel32.dll`, from the same crates as the x64 ones, and the 32-bit test programs of
//! `tests/windows/wow64/` — and check them.
//!
//! They are built the way the x64 modules are (ADR 0071, `ntdll.rs`, `win32.rs`): `no_std`
//! Rust for `i686-pc-windows-msvc`, `-Zbuild-std` from the toolchain's `rust-src`, linked by
//! `rust-lld` in its `lld-link` flavour; no C toolchain. Three cargo builds (the `ntdll`
//! workspace, the two modules of the Win32 workspace, and a generated package of the test
//! programs), each in a target directory of its own, cached together by content hash in
//! `<main checkout>/target/wow64-cache` beside `ntdll`'s and the Win32 modules' caches. The
//! kernel's `build.rs` embeds them from the directory `POLY_WOW64_DIR` names, which
//! [`crate::kernel_cargo`] sets; the NT layout writes them to `C:\Windows\SysWOW64`.

use crate::{Result, cargo, root, step};
use std::path::{Path, PathBuf};

/// The build's flags, part of the cache key.
const RUSTFLAGS: &str = "-Clinker=rust-lld -Clinker-flavor=lld-link -Clink-arg=/NODEFAULTLIB";
/// The test programs' extra flags.
const PROGRAM_FLAGS: &str = "-Clink-arg=/ENTRY:mainCRTStartup -Clink-arg=/SUBSYSTEM:CONSOLE";
/// The target.
pub const TARGET: &str = "i686-pc-windows-msvc";
/// The environment variable that gives the kernel's `build.rs` the directory.
pub const KERNEL_ENV: &str = "POLY_WOW64_DIR";
/// The 32-bit test programs' sources (each also listed in [`cache_key`]).
const PROGRAMS_DIR: &str = "tests/windows/wow64";
/// The modules (in the cache under these names).
pub const MODULES: &[&str] = &["ntdll.dll", "kernelbase.dll", "kernel32.dll"];
/// The test programs (`tests/windows/wow64/<name>.rs`).
pub const PROGRAMS: &[&str] = &["hello32"];
/// Cached versions kept.
const CACHE_KEEP: usize = 6;
/// Exports the kernel and every 32-bit program need from the i686 `ntdll`.
const NTDLL_REQUIRED: &[&str] = &[
    "LdrInitializeThunk",
    "KiUserApcDispatcher",
    "KiUserExceptionDispatcher",
    "RtlCaptureContext",
    "RtlAllocateHeap",
    "RtlExitUserProcess",
    "NtClose",
    "ZwClose",
    "NtWriteFile",
    "PolyDebugPrint",
];

/// Every file the cache holds.
fn outputs() -> Vec<String> {
    MODULES.iter().map(|m| m.to_string()).chain(PROGRAMS.iter().map(|p| format!("{p}.exe"))).collect()
}

/// `cargo xtask wow64 [--cache-key]`: make sure the cached modules are up to date (building
/// them if not) and verify them.
pub fn command(args: &[String]) -> Result {
    if args.iter().any(|a| a == "--cache-key") {
        println!("wow64-{:016x}", cache_key(&root())?);
        return Ok(());
    }
    let dir = ensure()?;
    verify(&dir)?;
    for f in outputs() {
        let p = dir.join(&f);
        eprintln!("{}: {} bytes", p.display(), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0));
    }
    Ok(())
}

/// The directory with the up-to-date modules: from the cache, or built, verified and cached.
pub fn ensure() -> Result<PathBuf> {
    let root = root();
    let key = cache_key(&root)?;
    let cache = cache_dir(&root);
    let cached = cache.join(format!("wow64-{key:016x}"));
    if outputs().iter().all(|f| cached.join(f).is_file()) {
        return Ok(cached);
    }
    let target = root.join("target").join("wow64");
    let started = std::time::Instant::now();
    let built = build(&root, &target)?;
    verify(&built)?;
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let tmp = cache.join(format!(".wow64-{key:016x}.{}.tmp", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    for f in outputs() {
        std::fs::copy(built.join(&f), tmp.join(&f)).map_err(|e| format!("{f}: {e}"))?;
    }
    if std::fs::rename(&tmp, &cached).is_err() {
        // Another worktree cached the same version first.
        let _ = std::fs::remove_dir_all(&tmp);
    }
    if std::env::var_os("POLY_WOW64_KEEP_BUILD").is_none() {
        let _ = std::fs::remove_dir_all(&target);
    }
    prune(&cache);
    crate::progress::note(&format!(
        "32-bit modules built in {:.0?}, cached as {}",
        started.elapsed(),
        cached.display()
    ));
    Ok(cached)
}

/// The hash of everything the modules are built from.
fn cache_key(root: &Path) -> Result<u64> {
    let files = crate::ntdll::inputs_from(
        root,
        &[
            "personalities/nt/ntdll",
            "personalities/nt/win32/rt",
            "personalities/nt/win32/kernelbase",
            "personalities/nt/win32/kernel32",
        ],
        &[
            "tests/windows/wow64/hello32.rs",
            "personalities/nt/win32/Cargo.toml",
            "personalities/nt/win32/Cargo.lock",
            "personalities/nt/src/services.rs",
            "personalities/nt/src/wow64/sigs.rs",
            "rust-toolchain.toml",
        ],
    )?;
    crate::ntdll::hash_files(root, &format!("{TARGET} {RUSTFLAGS} {PROGRAM_FLAGS}"), &files)
}

/// `wow64-cache`, beside `ntdll`'s cache.
fn cache_dir(root: &Path) -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(root);
    ntdll.parent().map_or_else(|| root.join("target"), Path::to_path_buf).join("wow64-cache")
}

/// Keep the [`CACHE_KEEP`] newest versions.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut sets: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("wow64-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    sets.sort();
    let excess = sets.len().saturating_sub(CACHE_KEEP);
    for (_, p) in sets.into_iter().take(excess) {
        let _ = std::fs::remove_dir_all(p);
    }
}

/// `cargo build` of `manifest` for the 32-bit target with `-Zbuild-std`.
fn cargo_build(manifest: &Path, target_dir: &Path, flags: &str, packages: &[&str]) -> Result<std::process::Command> {
    let vendor = crate::ntdll::vendor_dir()?;
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTFLAGS", flags)
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(manifest);
    for p in packages {
        cmd.args(["-p", p]);
    }
    cmd.args(["-Zbuild-std=core,alloc,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--target", TARGET])
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()));
    Ok(cmd)
}

/// The test programs' package: one binary per source.
fn programs_manifest(root: &Path) -> String {
    let mut s = String::from(
        "# Generated by `cargo xtask wow64` (ADR 0186).\n[workspace]\n\n[package]\nname = \"poly-wow64-tests\"\nversion = \"0.0.1\"\nedition = \"2024\"\npublish = false\n\n[profile.release]\npanic = \"abort\"\nopt-level = \"s\"\n",
    );
    // The compiler's 32-bit helper routines, which a C runtime would bring.
    let helpers = root.join("personalities/nt/msvc32");
    s.push_str(&format!(
        "
[dependencies]
poly-msvc32 = {{ path = {:?} }}
",
        helpers.to_string_lossy()
    ));
    for p in PROGRAMS {
        let path = root.join(PROGRAMS_DIR).join(format!("{p}.rs"));
        s.push_str(&format!("\n[[bin]]\nname = \"{p}\"\npath = {:?}\n", path.to_string_lossy()));
    }
    s
}

/// Build everything into `target`; the directory with the outputs.
fn build(root: &Path, target: &Path) -> Result<PathBuf> {
    let out = target.join("out");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let ntdll_target = target.join("ntdll");
    let flags = format!("{RUSTFLAGS} -Clink-arg=/SUBSYSTEM:CONSOLE");
    step(
        "build the i686 ntdll.dll",
        &mut cargo_build(&root.join("personalities/nt/ntdll/Cargo.toml"), &ntdll_target, &flags, &[])?,
    )?;
    let win32_target = target.join("win32");
    step(
        "build the i686 kernelbase.dll and kernel32.dll",
        &mut cargo_build(
            &root.join("personalities/nt/win32/Cargo.toml"),
            &win32_target,
            RUSTFLAGS,
            &["kernelbase", "kernel32"],
        )?,
    )?;
    let programs = target.join("programs");
    std::fs::create_dir_all(&programs).map_err(|e| format!("{}: {e}", programs.display()))?;
    let manifest = programs.join("Cargo.toml");
    std::fs::write(&manifest, programs_manifest(root)).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let programs_target = target.join("programs-target");
    step(
        "build the 32-bit test programs",
        &mut cargo_build(&manifest, &programs_target, &format!("{RUSTFLAGS} {PROGRAM_FLAGS}"), &[])?,
    )?;
    let rel = |t: &Path, f: &str| t.join(TARGET).join("release").join(f);
    let copies = [
        (rel(&ntdll_target, "ntdll.dll"), "ntdll.dll".to_string()),
        (rel(&win32_target, "kernelbase.dll"), "kernelbase.dll".to_string()),
        (rel(&win32_target, "kernel32.dll"), "kernel32.dll".to_string()),
    ]
    .into_iter()
    .chain(PROGRAMS.iter().map(|p| (rel(&programs_target, &format!("{p}.exe")), format!("{p}.exe"))));
    for (from, to) in copies {
        std::fs::copy(&from, out.join(&to)).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    Ok(out)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// An I386 PE32 image.
fn parse<'a>(name: &str, bytes: &'a [u8]) -> Result<poly_pe::Pe<'a>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("{name}: not a PE: {e:?}"))?;
    if pe.info().machine() != poly_pe::machine::I386 || pe.info().bits() != poly_pe::Bits::Pe32 {
        return Err(format!("{name}: not an x86 PE32 image"));
    }
    Ok(pe)
}

/// The names `pe` exports.
fn export_names(name: &str, pe: &poly_pe::Pe<'_>) -> Result<std::collections::BTreeSet<String>> {
    let exports = pe.exports().map_err(|e| format!("{name} exports: {e:?}"))?.ok_or(format!("{name}: no exports"))?;
    Ok(exports.names().filter_map(|r| r.ok()).map(|(n, _)| String::from_utf8_lossy(n).into_owned()).collect())
}

/// What `pe` imports, by DLL (lower case).
fn imports(name: &str, pe: &poly_pe::Pe<'_>) -> Result<std::collections::BTreeMap<String, Vec<String>>> {
    let mut out = std::collections::BTreeMap::new();
    let Some(dlls) = pe.imports().map_err(|e| format!("{name} imports: {e:?}"))? else { return Ok(out) };
    for dll in dlls {
        let dll = dll.map_err(|e| format!("{name} imports: {e:?}"))?;
        let names = dll
            .entries()
            .map_err(|e| format!("{name} imports: {e:?}"))?
            .filter_map(|e| e.ok())
            .map(|e| match e.symbol {
                poly_pe::ImportSymbol::Name { name, .. } => String::from_utf8_lossy(name).into_owned(),
                poly_pe::ImportSymbol::Ordinal(o) => format!("#{o}"),
            })
            .collect();
        let dll_name = dll.dll_name().map_err(|e| format!("{name} imports: {e:?}"))?;
        out.insert(String::from_utf8_lossy(dll_name).to_ascii_lowercase(), names);
    }
    Ok(out)
}

/// Check the set: every module is an x86 image; `ntdll` exports what the kernel enters and
/// what `kernelbase` imports from it, by undecorated names; `kernel32` imports only from
/// `kernelbase`; each test program imports only from `kernel32`.
fn verify(dir: &Path) -> Result {
    let ntdll = read(&dir.join("ntdll.dll"))?;
    let ntdll = parse("ntdll.dll", &ntdll)?;
    let ntdll_exports = export_names("ntdll.dll", &ntdll)?;
    let missing: Vec<&str> = NTDLL_REQUIRED.iter().copied().filter(|n| !ntdll_exports.contains(*n)).collect();
    if !missing.is_empty() {
        return Err(format!("the i686 ntdll.dll lacks {}", missing.join(", ")));
    }
    let kb = read(&dir.join("kernelbase.dll"))?;
    let kb = parse("kernelbase.dll", &kb)?;
    for (dll, names) in imports("kernelbase.dll", &kb)? {
        if dll == "ntdll.dll" {
            let missing: Vec<&String> = names.iter().filter(|n| !ntdll_exports.contains(*n)).collect();
            if !missing.is_empty() {
                return Err(format!("the i686 kernelbase.dll imports what ntdll.dll lacks: {missing:?}"));
            }
        }
    }
    let k32 = read(&dir.join("kernel32.dll"))?;
    let k32 = parse("kernel32.dll", &k32)?;
    for dll in imports("kernel32.dll", &k32)?.keys() {
        if dll != "kernelbase.dll" && dll != "ntdll.dll" {
            return Err(format!("the i686 kernel32.dll imports {dll}"));
        }
    }
    for p in PROGRAMS {
        let file = format!("{p}.exe");
        let bytes = read(&dir.join(&file))?;
        let pe = parse(&file, &bytes)?;
        for dll in imports(&file, &pe)?.keys() {
            if dll != "kernel32.dll" {
                return Err(format!("{file} imports {dll}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_programs_package_lists_every_program() {
        let m = programs_manifest(Path::new("/repo"));
        for p in PROGRAMS {
            assert!(m.contains(&format!("name = \"{p}\"")));
        }
        assert!(m.contains("[workspace]"));
        assert!(m.contains("poly-msvc32 = { path = "));
    }
}
