//! `polytrace.dll`'s entry point (ADR 0090); its exports are the Rust
//! items `pt_enter` and `pt_leave`.

fn main() {
    println!("cargo:rustc-link-arg=/ENTRY:polytrace_main");
}
