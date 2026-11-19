//! Mach-O programs on the XNU personality through Poly's own dyld (ADR 0237).
//!
//! The programs are the committed test files of `crates/poly-dyld/tests/data`
//! (built by its `generate.sh` from our own C sources; they link a stand-in
//! libSystem of ours, never Apple's), so the boot test needs no compiler.
//! The image holds:
//!
//! - `/bin/mhello`: chained fixups, `@rpath/libgreet.dylib` from
//!   `/usr/lib/poly-dyld-test`, thread-local variables, initializers, a
//!   missing weak import, `__mh_execute_header` and `apple[0]`;
//! - `/bin/mhello-classic`: the same with classic rebase, bind and lazy bind
//!   opcodes and `__mod_init_func` (`libgreetc.dylib`);
//! - `/bin/mstatic`: a static program (`LC_UNIXTHREAD`, no dyld);
//! - `/usr/lib/libSystem.B.dylib`: the stand-in, whose `exit` ends a program
//!   when `main` returns.
//!
//! Each runs as the first process and must print its lines and exit with
//! status 0.

use crate::initramfs::{self, Tree};
use crate::programs::run_as_init;
use crate::symbols::Symbols;
use crate::{Result, root};
use std::path::Path;
use std::sync::Arc;

const DATA: &str = "crates/poly-dyld/tests/data";

/// What the dynamically linked programs print when every check passed.
const HELLO_LINES: &[&str] = &[
    "libgreet: init",
    "hello: init",
    "ok initializers ran, the library's first",
    "greet: hello, mach-o",
    "ok greet binds across images",
    "ok a rebased pointer table in the library",
    "ok a missing weak import is null",
    "ok __mh_execute_header is the mapped header",
    "ok thread-local variables",
    "ok argc and argv",
    "ok apple[0] is executable_path=",
    "hello: done",
];

fn read(name: &str) -> Result<Vec<u8>> {
    let p = root().join(DATA).join(name);
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// The boot test group `macho`.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let mhello = read("hello_x64")?;
    let classic = read("helloc_x64")?;
    let mstatic = read("static_x64")?;
    let programs: Vec<(&str, &[u8])> = vec![("mhello", &mhello), ("mhello-classic", &classic), ("mstatic", &mstatic)];
    let mut tree = Tree::default();
    for (at, file) in [
        ("usr/lib/libSystem.B.dylib", "libSystem_x64.dylib"),
        ("usr/lib/poly-dyld-test/libgreet.dylib", "libgreet_x64.dylib"),
        ("usr/lib/poly-dyld-test/libgreetc.dylib", "libgreetc_x64.dylib"),
    ] {
        tree.files.push((at.into(), 0o755, read(file)?));
    }
    let image =
        initramfs::write(&root(), "poly-macho-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))?;

    eprintln!("==> boot test: a static x86-64 Mach-O program (LC_UNIXTHREAD) runs as the first process");
    run_as_init(kernel, syms, &image, "mstatic", "static: ok\n")?;

    for (program, what) in [("mhello", "chained fixups"), ("mhello-classic", "classic rebase and bind opcodes")] {
        eprintln!("==> boot test: a dynamically linked Mach-O program and its dylibs through Poly's dyld ({what})");
        let b = run_as_init(kernel, syms, &image, program, "hello: done\n")?;
        let mut at = 0;
        for line in HELLO_LINES {
            match b.output[at..].find(line) {
                Some(i) => at += i + line.len(),
                None => return Err(format!("/bin/{program}: {line:?} missing or out of order")),
            }
        }
        if let Some(line) = b.output.lines().find(|l| l.contains("not ok ") || l.contains(" not implemented")) {
            return Err(format!("/bin/{program}: {line}"));
        }
    }
    Ok(())
}
