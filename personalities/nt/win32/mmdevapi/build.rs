//! `mmdevapi.dll`'s entry point is its `DllMain` (`mmdevapi_main`), as for the other Win32
//! modules (ADR 0079). Chrome's `chrome.dll` delay-loads the export with ordinal 17 by its number
//! (the audio plan's survey): it is exported as `ord_17`, a fail-only stub until W2. The 32-bit
//! module carries none of the fail-only stubs (they take no arguments, and `stdcall` would leave
//! them).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:mmdevapi_main");
    if std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "x86_64") {
        println!("cargo:rustc-link-arg=/EXPORT:ord_17,@17");
    }
}
