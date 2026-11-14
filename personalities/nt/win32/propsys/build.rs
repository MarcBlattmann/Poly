//! `propsys.dll`'s entry point is its `DllMain` (`propsys_main`), as for the
//! other Win32 modules (ADR 0079).

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:propsys_main");
}
