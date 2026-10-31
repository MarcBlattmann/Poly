//! `bcrypt.dll`'s entry point is its `DllMain` (`bcrypt_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:bcrypt_main");
}
