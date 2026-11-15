//! `kernelbase.dll`'s entry point is its `DllMain` (`kernelbase_main`):
//! there is no C runtime to provide `_DllMainCRTStartup` (ADR 0079).

fn main() {
    // BEGIN generated: the names the Windows applications of the survey import (ADR 0155, step 10).
    // The 32-bit modules (ADR 0186) carry none of these: a fail-only stub takes no arguments and `stdcall` would leave them.
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        for (name, target) in APPS_FORWARDS {
            println!("cargo:rustc-link-arg=/EXPORT:{name}={target}");
        }
    }
    // END generated
    println!("cargo:rustc-link-arg=/ENTRY:kernelbase_main");
    // The version resource (ADR 0155, step 9): programs read the system's version from this
    // file (Learn, "Getting the System Version"). lld-link takes a `.res` file as an input.
    println!("cargo:rerun-if-changed=../../../../crates/poly-kernelbase/src/verinfo.rs");
    let version = poly_kernelbase::verinfo::system_dll_version_info("kernelbase.dll", "Poly Windows base API DLL");
    let res = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("version.res");
    std::fs::write(&res, poly_kernelbase::verinfo::res_file(&version)).expect("write version.res");
    println!("cargo:rustc-link-arg={}", res.display());
}

// BEGIN generated consts
#[rustfmt::skip]
const APPS_FORWARDS: &[(&str, &str)] = &[
    ("AllocateLocallyUniqueId", "advapi32.AllocateLocallyUniqueId"),
    ("EventRegister", "advapi32.EventRegister"),
    ("EventSetInformation", "advapi32.EventSetInformation"),
    ("EventUnregister", "advapi32.EventUnregister"),
    ("EventWriteTransfer", "advapi32.EventWriteTransfer"),
    ("GetFileVersionInfoSizeExW", "version.GetFileVersionInfoSizeExW"),
    ("GetFileVersionInfoSizeW", "version.GetFileVersionInfoSizeW"),
    ("GetFileVersionInfoW", "version.GetFileVersionInfoW"),
    ("GlobalAlloc", "kernel32.GlobalAlloc"),
    ("GlobalFree", "kernel32.GlobalFree"),
    ("GlobalLock", "kernel32.GlobalLock"),
    ("GlobalSize", "kernel32.GlobalSize"),
    ("GlobalUnlock", "kernel32.GlobalUnlock"),
    ("RegDeleteKeyW", "advapi32.RegDeleteKeyW"),
    ("VerQueryValueW", "version.VerQueryValueW"),
    ("lstrcmpW", "kernel32.lstrcmpW"),
    ("lstrcmpiW", "kernel32.lstrcmpiW"),
];
// END generated consts
