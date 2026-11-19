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
///
/// `/safeseh:no`: lld marks an x86 image `NO_SEH` when it has no safe handler table, and the
/// system then refuses every frame handler in it (SafeSEH, ADR 0186). The test programs register
/// their own frames, so they are linked as legacy images (no table, no flag), as a compiler
/// that does not know `/SAFESEH` makes them; `safeseh32` asks for a table (`programs_build_rs`).
const PROGRAM_FLAGS: &str = "-Clink-arg=/ENTRY:mainCRTStartup -Clink-arg=/SUBSYSTEM:CONSOLE -Clink-arg=/safeseh:no";
/// The target.
pub const TARGET: &str = "i686-pc-windows-msvc";
/// The environment variable that gives the kernel's `build.rs` the directory.
pub const KERNEL_ENV: &str = "POLY_WOW64_DIR";
/// The 32-bit test programs' sources (each also listed in [`cache_key`]).
const PROGRAMS_DIR: &str = "tests/windows/wow64";
/// The modules (in the cache under these names).
/// `win32u`, `gdi32` and `user32` are the 32-bit GUI (ADR 0186 stage 4).
pub const CORE_MODULES: &[&str] =
    &["ntdll.dll", "kernelbase.dll", "kernel32.dll", "win32u.dll", "gdi32.dll", "user32.dll"];
/// The other system DLLs 32-bit programs import (ADR 0186 stage 3), built from the same crates
/// as the x64 ones in the Win32 workspace (the package is the file name without `.dll`).
pub const SYSTEM_MODULES: &[&str] = &[
    "advapi32.dll",
    "ole32.dll",
    "oleaut32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "comctl32.dll",
    "version.dll",
    "winmm.dll",
    "ws2_32.dll",
    "imm32.dll",
    "uxtheme.dll",
    "dwmapi.dll",
    "shcore.dll",
    "secur32.dll",
    "crypt32.dll",
    "winhttp.dll",
    "iphlpapi.dll",
    "wtsapi32.dll",
    "oleacc.dll",
    "powrprof.dll",
    "userenv.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "ncrypt.dll",
    "netapi32.dll",
    "setupapi.dll",
    "propsys.dll",
    "wininet.dll",
    "wintrust.dll",
    "cfgmgr32.dll",
    "dbghelp.dll",
    // Google's 32-bit updater imports AlphaBlend.
    "msimg32.dll",
];

/// Every module of the set, in the cache under these names.
pub fn modules() -> impl Iterator<Item = &'static str> {
    CORE_MODULES.iter().chain(SYSTEM_MODULES).copied()
}
/// The test programs (`tests/windows/wow64/<name>.rs`).
pub const PROGRAMS: &[&str] = &["hello32", "gui32", "sys32", "safeseh32", "ansi32"];
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
    modules().map(|m| m.to_string()).chain(PROGRAMS.iter().map(|p| format!("{p}.exe"))).collect()
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

/// One [`ensure`] at a time in this process. Two threads can ask at once (`desktop --chrome-win`
/// builds the kernel while `polystub.dll` reads our exports): without it both built the 32-bit modules into
/// the same target directory and the first to finish removed it under the other's build.
static ENSURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The directory with the up-to-date modules: from the cache, or built, verified and cached.
pub fn ensure() -> Result<PathBuf> {
    // The second caller waits here and then finds the first one's result in the cache.
    let _one = ENSURING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

/// The crates of the Win32 workspace that are test programs: built for x64 only.
const TEST_CRATES: &[&str] = &[
    "a32test",
    "b35test",
    "contest",
    "crtiotest",
    "crtloctest",
    "crttest",
    "cryptest",
    "httptest",
    "k32test",
    "s1test",
    "wstest",
];

/// Every `raw-dylib` link of a module that is also built for i686 must say how its names are
/// bound there (`import_name_type = "undecorated"`: a 32-bit `stdcall` import is `_Name@N`
/// otherwise, which no DLL exports), or be the x64-only half of a `cfg_attr` pair. A new link
/// without it would only fail when a 32-bit program loads the DLL.
pub fn check_links(root: &Path) -> Result {
    fn visit(dir: &Path, bad: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "target") {
                    visit(&path, bad);
                }
            } else if path.extension().is_some_and(|x| x == "rs") {
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for (i, line) in text.lines().enumerate() {
                    if line.contains("kind = \"raw-dylib\"")
                        && !line.contains("import_name_type")
                        && !line.contains("not(target_arch = \"x86\")")
                        && !line.trim_start().starts_with("//")
                    {
                        bad.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
    }
    let win32 = root.join("personalities/nt/win32");
    let mut bad = Vec::new();
    let Ok(rd) = std::fs::read_dir(&win32) else { return Ok(()) };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if e.path().is_dir() && !TEST_CRATES.contains(&name.as_str()) {
            visit(&e.path().join("src"), &mut bad);
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    bad.sort();
    Err(format!(
        "raw-dylib links without a 32-bit form (ADR 0186): write `#[cfg_attr(target_arch = \"x86\", link(name = \"dll\", kind = \"raw-dylib\", import_name_type = \"undecorated\"))]` and `#[cfg_attr(not(target_arch = \"x86\"), link(name = \"dll\", kind = \"raw-dylib\"))]`:
  {}",
        bad.join("
  ")
    ))
}

/// The hash of everything the modules are built from.
fn cache_key(root: &Path) -> Result<u64> {
    let mut start: Vec<String> =
        ["ntdll", "win32/rt", "win32/kernelbase", "win32/kernel32", "win32/win32u", "win32/gdi32", "win32/user32"]
            .iter()
            .map(|d| format!("personalities/nt/{d}"))
            .collect();
    start.extend(SYSTEM_MODULES.iter().map(|m| format!("personalities/nt/win32/{}", m.trim_end_matches(".dll"))));
    let start: Vec<&str> = start.iter().map(String::as_str).collect();
    let files = crate::ntdll::inputs_from(
        root,
        &start,
        &[
            "tests/windows/wow64/hello32.rs",
            "tests/windows/wow64/gui32.rs",
            "tests/windows/wow64/sys32.rs",
            "tests/windows/wow64/safeseh32.rs",
            "tests/windows/wow64/ansi32.rs",
            "personalities/nt/win32/Cargo.toml",
            "personalities/nt/win32/Cargo.lock",
            "personalities/nt/src/services.rs",
            "personalities/nt/src/wow64/sigs.rs",
            // The 32-bit fail stubs' argument sizes, compiled into the Win32 runtime by path
            // (ADR 0222), not through a Cargo dependency.
            "crates/poly-stubs/src/x86slots.rs",
            "crates/poly-stubs/src/x86_signatures.txt",
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

/// The build script of the test programs' package: `safeseh32` is linked with `/safeseh` after
/// the `/safeseh:no` of the others (the last of the two wins).
const PROGRAMS_BUILD_RS: &str = "// Generated by `cargo xtask wow64` (ADR 0186).
fn main() {
    println!(\"cargo:rustc-link-arg-bin=safeseh32=/safeseh\");
}
";

/// The test programs' package: one binary per source.
fn programs_manifest(root: &Path) -> String {
    let mut s = String::from(
        "# Generated by `cargo xtask wow64` (ADR 0186).\n[workspace]\n\n[package]\nname = \"poly-wow64-tests\"\nbuild = \"build.rs\"\nversion = \"0.0.1\"\nedition = \"2024\"\npublish = false\n\n[profile.release]\npanic = \"abort\"\nopt-level = \"s\"\n",
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
    check_links(root)?;
    let out = target.join("out");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let ntdll_target = target.join("ntdll");
    let flags = format!("{RUSTFLAGS} -Clink-arg=/SUBSYSTEM:CONSOLE");
    step(
        "build the i686 ntdll.dll",
        &mut cargo_build(&root.join("personalities/nt/ntdll/Cargo.toml"), &ntdll_target, &flags, &[])?,
    )?;
    let win32_target = target.join("win32");
    let packages: Vec<&str> = ["kernelbase", "kernel32", "win32u", "gdi32", "user32"]
        .into_iter()
        .chain(SYSTEM_MODULES.iter().map(|m| m.trim_end_matches(".dll")))
        .collect();
    step(
        "build the i686 Win32 modules (kernelbase, kernel32, the GUI and the other system DLLs)",
        &mut cargo_build(&root.join("personalities/nt/win32/Cargo.toml"), &win32_target, RUSTFLAGS, &packages)?,
    )?;
    let programs = target.join("programs");
    std::fs::create_dir_all(&programs).map_err(|e| format!("{}: {e}", programs.display()))?;
    let manifest = programs.join("Cargo.toml");
    std::fs::write(&manifest, programs_manifest(root)).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let build_rs = programs.join("build.rs");
    std::fs::write(&build_rs, PROGRAMS_BUILD_RS).map_err(|e| format!("{}: {e}", build_rs.display()))?;
    let programs_target = target.join("programs-target");
    step(
        "build the 32-bit test programs",
        &mut cargo_build(&manifest, &programs_target, &format!("{RUSTFLAGS} {PROGRAM_FLAGS}"), &[])?,
    )?;
    let rel = |t: &Path, f: &str| t.join(TARGET).join("release").join(f);
    let copies = std::iter::once((rel(&ntdll_target, "ntdll.dll"), "ntdll.dll".to_string()))
        .chain(modules().filter(|m| *m != "ntdll.dll").map(|m| (rel(&win32_target, m), m.to_string())))
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

/// The names `pe` exports, and its ordinals as `#<ordinal>`.
fn export_names(name: &str, pe: &poly_pe::Pe<'_>) -> Result<std::collections::BTreeSet<String>> {
    let exports = pe.exports().map_err(|e| format!("{name} exports: {e:?}"))?.ok_or(format!("{name}: no exports"))?;
    let mut out: std::collections::BTreeSet<String> =
        exports.names().filter_map(|r| r.ok()).map(|(n, _)| String::from_utf8_lossy(n).into_owned()).collect();
    // Imports by ordinal read `#<ordinal>` ([`imports`]).
    out.extend(exports.functions().filter_map(|r| r.ok()).map(|e| format!("#{}", e.ordinal)));
    Ok(out)
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
/// `kernelbase`; each test program imports only from `kernel32` and `ntdll`.
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
    let kb_exports = export_names("kernelbase.dll", &kb)?;
    for (dll, names) in imports("kernel32.dll", &k32)? {
        if dll != "kernelbase.dll" && dll != "ntdll.dll" {
            return Err(format!("the i686 kernel32.dll imports {dll}"));
        }
        // What kernel32 imports from kernelbase has to be exported under that very name (an
        // import declared without `import_name_type = "undecorated"` asks for `_Name@N`).
        if dll == "kernelbase.dll" {
            let missing: Vec<&String> = names.iter().filter(|n| !kb_exports.contains(*n)).collect();
            if !missing.is_empty() {
                return Err(format!("the i686 kernel32.dll imports what kernelbase.dll lacks: {missing:?}"));
            }
        }
    }
    // The GUI modules import only from each other and the modules above.
    let ours = ["ntdll.dll", "kernel32.dll", "win32u.dll", "gdi32.dll", "user32.dll"];
    for m in ["win32u.dll", "gdi32.dll", "user32.dll"] {
        let bytes = read(&dir.join(m))?;
        let pe = parse(m, &bytes)?;
        for dll in imports(m, &pe)?.keys() {
            if !ours.contains(&dll.as_str()) {
                return Err(format!("the i686 {m} imports {dll}"));
            }
        }
    }
    // The other system DLLs (stage 3) import only from the set (`kernelbase` too: the API-set
    // contracts forward there) and export undecorated names (no `_Name@N`).
    let set: Vec<&str> = modules().collect();
    for m in SYSTEM_MODULES {
        let bytes = read(&dir.join(m))?;
        let pe = parse(m, &bytes)?;
        for (dll, names) in imports(m, &pe)? {
            if !set.contains(&dll.as_str()) {
                return Err(format!("the i686 {m} imports {dll}, which is not one of our 32-bit modules"));
            }
            // An import by a decorated name (`_NtClose@4`) binds to nothing: raw-dylib links of
            // 32-bit code need `import_name_type = "undecorated"`.
            let bad: Vec<&String> = names.iter().filter(|n| n.contains('@')).collect();
            if !bad.is_empty() {
                return Err(format!("the i686 {m} imports decorated names from {dll}: {bad:?}"));
            }
        }
        if let Some(bad) = export_names(m, &pe)?.into_iter().find(|n| n.contains('@')) {
            return Err(format!("the i686 {m} exports the decorated name {bad}"));
        }
    }
    for p in PROGRAMS {
        let file = format!("{p}.exe");
        let bytes = read(&dir.join(&file))?;
        let pe = parse(&file, &bytes)?;
        for dll in imports(&file, &pe)?.keys() {
            if !set.contains(&dll.as_str()) {
                return Err(format!("{file} imports {dll}"));
            }
        }
    }
    verify_imports(dir)
}

/// Every name one of our 32-bit modules or programs imports from another of ours is an export of
/// it, by the name as imported: a module that imports `_FindResourceExW@16` (a `raw-dylib` block
/// without `import_name_type = "undecorated"`) or a name `kernel32.dll` does not export would
/// otherwise only fail when a program starts ("process start failed (0xc0000139)").
fn verify_imports(dir: &Path) -> Result {
    let files: Vec<String> =
        modules().map(|m| m.to_string()).chain(PROGRAMS.iter().map(|p| format!("{p}.exe"))).collect();
    let mut exports = std::collections::BTreeMap::new();
    for m in modules() {
        let bytes = read(&dir.join(m))?;
        exports.insert(m.to_string(), export_names(m, &parse(m, &bytes)?)?);
    }
    for file in &files {
        let bytes = read(&dir.join(file))?;
        let pe = parse(file, &bytes)?;
        for (dll, names) in imports(file, &pe)? {
            let Some(have) = exports.get(&dll) else { continue };
            let missing: Vec<&String> = names.iter().filter(|n| !have.contains(*n)).collect();
            if !missing.is_empty() {
                return Err(format!("{file} imports what {dll} does not export: {missing:?}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_raw_dylib_link_of_the_win32_modules_has_a_32_bit_form() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        check_links(&root).unwrap();
    }

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
