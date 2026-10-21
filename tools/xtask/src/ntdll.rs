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
//! This is a separate, on-demand command (not part of `cargo xtask ci`) so
//! the build-std compile and its disk use do not weigh on every kernel
//! build and boot test.

use crate::{Result, cargo, root, step};
use std::path::PathBuf;
use std::process::Command;

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
    "RtlLookupFunctionEntry",
    "RtlVirtualUnwind",
    "RtlUnwindEx",
    "RtlCaptureContext",
    "RtlRaiseException",
    // System-call stubs (both decorations).
    "NtClose",
    "NtAllocateVirtualMemory",
    "NtCreateFile",
    "NtContinue",
    "ZwClose",
    "ZwAllocateVirtualMemory",
];

/// `cargo xtask ntdll [--out <path>]`: build the DLL and verify it.
pub fn command(args: &[String]) -> Result {
    let out = opt(args, "--out");
    let dll = build(root().join("target").join("ntdll"))?;
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

/// The toolchain's `rust-src` vendored-dependency directory
/// (`<sysroot>/lib/rustlib/src/rust/library/vendor`).
fn vendor_dir() -> Result<PathBuf> {
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
    let rustflags = "-Clinker=rust-lld -Clinker-flavor=lld-link \
         -Clink-arg=/NODEFAULTLIB -Clink-arg=/ENTRY:LdrInitializeThunk -Clink-arg=/SUBSYSTEM:CONSOLE";
    let mut cmd = cargo();
    cmd.env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTFLAGS", rustflags)
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
fn verify(dll: &std::path::Path) -> Result {
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
