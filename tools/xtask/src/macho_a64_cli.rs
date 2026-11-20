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
    let programs: Vec<(&str, &[u8])> = vec![("mhello-libsystem", &hello)];
    let mut tree = Tree { files: native::runtime_files()?, ..Default::default() };
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
    Ok(())
}
