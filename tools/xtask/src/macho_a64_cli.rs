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
    let jit = read("tests/a64/cli/bin/jit")?;
    let programs: Vec<(&str, &[u8])> = vec![
        ("mhello-libsystem", &hello),
        ("lua", &lua),
        ("sqlite3", &sqlite),
        ("a64-threads", &threads),
        ("a64-jit", &jit),
    ];
    let mut tree = Tree { files: native::runtime_files()?, ..Default::default() };
    tree.files.push(("usr/share/poly-a64/hello.lua".into(), 0o644, read("tests/a64/cli/hello.lua")?));
    tree.files.push(("usr/share/poly-a64/session.sql".into(), 0o644, read("tests/a64/cli/session.sql")?));
    tree.files.push(("usr/share/poly-a64/bench.lua".into(), 0o644, read("tests/a64/cli/bench.lua")?));
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

    eprintln!("==> boot test: an arm64 program that writes its own code (MAP_JIT, mprotect flips)");
    let b = run_ending(kernel, syms, &image, "a64-jit", &[], &[], "code=0")?;
    in_order("a64-jit", &b.output, &["jit: done, 0 failed".to_string()])?;

    // Speed: Lua's benchmark under the JIT with the interpreter profile
    // (the whole script), then a fifth of it under the JIT and under the
    // interpreter alone; those two must print the same.
    eprintln!("==> boot test: Lua's benchmark script under the JIT, with the profile");
    let b =
        run_ending(kernel, syms, &image, "lua", &["/usr/share/poly-a64/bench.lua"], &["POLY_A64_PROFILE=1"], "code=0")?;
    in_order("lua", &b.output, &["bench: done".to_string()])?;
    for l in b.output.lines().filter(|l| l.contains("poly-a64: ")) {
        eprintln!("    {}", &l[l.find("poly-a64: ").unwrap_or(0)..]);
    }
    let mut runs = Vec::new();
    for (engine, env) in [("JIT", "POLY_A64_ENGINE=jit"), ("interpreter", "POLY_A64_ENGINE=interp")] {
        eprintln!("==> boot test: a fifth of Lua's benchmark script under the {engine}");
        let b = run_ending(
            kernel,
            syms,
            &image,
            "lua",
            &["/usr/share/poly-a64/bench.lua", "0.2"],
            &[env, "POLY_A64_STATS=1"],
            "code=0",
        )?;
        let lines: Vec<String> =
            b.output.lines().filter_map(|l| l.find("bench: ").map(|i| l[i..].trim_end().to_string())).collect();
        let (line, cycles) = crate::macho_a64::stats("lua", &b)?;
        eprintln!("    {line}");
        runs.push((lines, cycles));
    }
    if runs[0].0 != runs[1].0 || !runs[0].0.iter().any(|l| l == "bench: done") {
        return Err(format!("lua bench: the JIT printed {:?}, the interpreter {:?}", runs[0].0, runs[1].0));
    }
    eprintln!(
        "    lua bench: the JIT took {:.2}x fewer cycles than the interpreter",
        runs[1].1 as f64 / runs[0].1.max(1) as f64
    );
    Ok(())
}
