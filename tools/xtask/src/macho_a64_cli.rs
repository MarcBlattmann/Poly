//! arm64 command-line programs linked against the macOS SDK, on x86-64 Poly
//! through Poly's dyld, Poly's own libSystem (its arm64 slices, ADR 0258)
//! and the translator (ADR 0257).
//!
//! The image holds the translator's runtime, Poly's libSystem dylibs in
//! `/usr/lib` (universal files: x86-64 and arm64 slices) and the programs:
//!
//! - `/bin/mhello-libsystem` (the name the program checks): libSystem's own test program
//!   (`tests/macos/libsystem/hello.c`) built for arm64.

use crate::initramfs::{self, Tree};
use crate::macho_a64::{in_order, run_ending};
use crate::symbols::Symbols;
use crate::{Result, libsystem, native, root};
use std::path::Path;
use std::sync::Arc;

fn read(rel: &str) -> Result<Vec<u8>> {
    let p = root().join(rel);
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// The boot test group `macho-a64-cli`.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let hello = read("tests/macos/libsystem/hello_a64")?;
    let lua = read("tests/a64/cli/bin/lua")?;
    let sqlite = read("tests/a64/cli/bin/sqlite3")?;
    let threads = read("tests/a64/cli/bin/threads")?;
    let programs: Vec<(&str, &[u8])> =
        vec![("mhello-libsystem", &hello), ("lua", &lua), ("sqlite3", &sqlite), ("a64-threads", &threads)];
    let mut tree = Tree { files: native::runtime_files()?, ..Default::default() };
    tree.files.push(("usr/share/poly-a64/hello.lua".into(), 0o644, read("tests/a64/cli/hello.lua")?));
    tree.files.push(("usr/share/poly-a64/session.sql".into(), 0o644, read("tests/a64/cli/session.sql")?));
    for (path, bytes) in libsystem::tree_files()? {
        tree.files.push((path, 0o755, bytes));
    }
    let image = initramfs::write(
        &root(),
        "poly-macho-a64-cli-initramfs.cpio",
        &initramfs::programs_image_with(&programs, &tree),
    )?;

    eprintln!("==> boot test: an SDK-linked arm64 C program with Poly's libSystem, through the translator");
    let b = run_ending(kernel, syms, &image, "mhello-libsystem", &[], &[], "code=0")?;
    let lines: Vec<String> = libsystem::HELLO_LINES.iter().map(|l| l.to_string()).collect();
    in_order("mhello-libsystem", &b.output, &lines)?;

    eprintln!("==> boot test: Lua 5.4 (built from source for arm64) runs a script on Poly through the translator");
    let b = run_ending(kernel, syms, &image, "lua", &["/usr/share/poly-a64/hello.lua"], &["POLY_LUA=yes"], "code=0")?;
    in_order("lua", &b.output, &["lua: done, 0 failed".to_string()])?;

    eprintln!("==> boot test: the SQLite shell (built from source for arm64) runs a session on Poly");
    let b = run_ending(
        kernel,
        syms,
        &image,
        "sqlite3",
        &["-init", "/usr/share/poly-a64/session.sql", ":memory:", ".quit"],
        &[],
        "code=0",
    )?;
    let want = String::from_utf8(read("tests/a64/cli/session.out")?).map_err(|e| e.to_string())?;
    let lines: Vec<String> = want.lines().map(String::from).collect();
    in_order("sqlite3", &b.output, &lines)?;

    eprintln!("==> boot test: an arm64 C program with threads, signals and memory mappings");
    let b = run_ending(kernel, syms, &image, "a64-threads", &[], &[], "code=0")?;
    in_order("a64-threads", &b.output, &["threads: done, 0 failed".to_string()])?;
    Ok(())
}
