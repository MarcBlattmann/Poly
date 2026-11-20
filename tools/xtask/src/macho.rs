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
//! - `/bin/mbsd`: the BSD layer (ADR 0246): it opens and reads
//!   `/etc/mbsd.txt`, lists a directory, maps memory and the file, asks
//!   sysctl, blocks and sends signals, and starts itself as a child with
//!   `posix_spawn` that writes through an inherited pipe and is reaped
//!   with `wait4`;
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
    let mbsd = read("bsd_x64")?;
    let programs: Vec<(&str, &[u8])> =
        vec![("mhello", &mhello), ("mhello-classic", &classic), ("mstatic", &mstatic), ("mbsd", &mbsd)];
    let mut tree = Tree::default();
    for (at, file) in [
        ("usr/lib/libSystem.B.dylib", "libSystem_x64.dylib"),
        ("usr/lib/poly-dyld-test/libgreet.dylib", "libgreet_x64.dylib"),
        ("usr/lib/poly-dyld-test/libgreetc.dylib", "libgreetc_x64.dylib"),
    ] {
        tree.files.push((at.into(), 0o755, read(file)?));
    }
    tree.files.push(("etc/mbsd.txt".into(), 0o644, b"poly reads macos io\n".to_vec()));
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

    eprintln!("==> boot test: the BSD layer: files, a directory, mmap, sysctl, signals, posix_spawn and wait4");
    let b = run_as_init(kernel, syms, &image, "mbsd", "bsd: done\n")?;
    if let Some(line) = b.output.lines().find(|l| l.contains("not ok ") || l.contains(" not implemented")) {
        return Err(format!("/bin/mbsd: {line}"));
    }
    Ok(())
}

/// The x86-64 builds of the spec tests in `macho-mach`
/// (`personalities/xnu/tests/data`, ADR 0238).
const MACH_DATA: &str = "personalities/xnu/tests/data";

/// Tests whose x86-64 build checks less than the arm64 one (a part under
/// `POLY_ARM64`): only their own checks must pass, not the recording.
const ARM64_ONLY_PARTS: &[&str] = &["ulock_wait2_timeout"];

/// The test-protocol lines of a boot's output (`ok`, `not ok`, `value`,
/// `summary`).
fn protocol_lines(output: &str) -> Vec<&str> {
    output
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| ["ok ", "not ok ", "value ", "summary "].iter().any(|p| l.starts_with(p)))
        .collect()
}

/// The arm64 output macOS recorded for test `name` (`<dir>/macos.json`).
fn recorded(dir: &Path, name: &str) -> Result<String> {
    let path = dir.join("macos.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    json["tests"]
        .as_array()
        .and_then(|t| t.iter().find(|t| t["name"] == name))
        .and_then(|t| t["results"]["arm64"]["stdout"].as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{}: no arm64 output for {name}", path.display()))
}

/// The boot test group `macho-mach`: the visible tests of the Mach core
/// entries that run on Poly today, each as the first process; their output
/// must be what macOS printed (arm64 recording).
pub fn mach_boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let list = std::fs::read_to_string(root().join(MACH_DATA).join("tests.txt")).map_err(|e| e.to_string())?;
    let sources: Vec<&str> = list.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut files = Vec::new();
    for src in &sources {
        let name = Path::new(src).file_stem().and_then(|s| s.to_str()).ok_or("a test path")?.to_string();
        let p = root().join(MACH_DATA).join(&name);
        let bytes = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        files.push((name, bytes, root().join(src).parent().ok_or("a test directory")?.to_path_buf()));
    }
    let programs: Vec<(&str, &[u8])> = files.iter().map(|(n, b, _)| (n.as_str(), b.as_slice())).collect();
    let mut tree = Tree::default();
    tree.files.push(("usr/lib/libSystem.B.dylib".into(), 0o755, read("libSystem_x64.dylib")?));
    let image =
        initramfs::write(&root(), "poly-macho-mach-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))?;
    for (name, _, dir) in &files {
        eprintln!("==> boot test: Mach core spec test {name} (x86-64) as the first process");
        let b = run_as_init(kernel, syms, &image, name, "summary ")?;
        let got = protocol_lines(&b.output);
        if let Some(line) = got.iter().find(|l| l.starts_with("not ok ")) {
            return Err(format!("/bin/{name}: {line}"));
        }
        if let Some(line) = b.output.lines().find(|l| l.contains(" not implemented")) {
            return Err(format!("/bin/{name}: {line}"));
        }
        if ARM64_ONLY_PARTS.contains(&name.as_str()) {
            continue;
        }
        let want = recorded(dir, name)?;
        let want: Vec<&str> = want.lines().collect();
        if got != want {
            return Err(format!("/bin/{name}: output {got:#?}, macOS printed {want:#?}"));
        }
    }
    Ok(())
}
