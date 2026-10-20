//! Linux programs for the kernel's boot tests, and those tests: a static
//! "hello world" runs as the first process (`poly.init=/bin/<name>`) and
//! exits cleanly (roadmap Phase 3, ADR 0038).
//!
//! Two variants, so the milestone never depends on the host's C library:
//!
//! - `/bin/hello`: `tests/linux/nolibc/hello.rs`, no C library, built with
//!   the pinned Rust toolchain alone (`rustc --target x86_64-unknown-none`,
//!   static, not PIE). Always there.
//! - `/bin/hello-libc`: `tests/linux/libc/hello.c` linked statically with
//!   the host's C library, and `/bin/hello-c`, the freestanding C hello of
//!   the corpus: built by the pipeline tool (`pipeline build`, the same
//!   flags and manifest as the diff runner's corpus). Skipped with a
//!   message where there is no C compiler or no static C library.
//!
//! And for processes and threads (Phase 4, ADR 0041):
//!
//! - `/bin/forkexec`: `tools/xtask/programs/forkexec.rs`, no C library, as
//!   the first process (init): fork + execve of `/bin/hello` + wait4, copy
//!   on write, a thread with TLS joined through `CLONE_CHILD_CLEARTID` and
//!   a futex, and an orphan adopted and reaped by init.
//!
//! Builds are reproducible: fixed flags, no debug info, symbols stripped,
//! no build id, so the same toolchain gives the same bytes.

use super::{Boot, DEFAULT_MEMORY, Devices, QEMU_SUCCESS, Result, boot_on, cargo, initramfs, root};
use crate::symbols::Symbols;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Where the programs are built.
const OUT: &str = "target/xtask-programs";

/// What the nolibc hello writes on stdout.
pub const HELLO_OUTPUT: &str = "Hello, world!\n";
/// What the C hellos of the corpus write.
pub const C_HELLO_OUTPUT: &str = "hello, world\n";
/// What `/bin/forkexec` writes when every check passed.
pub const FORKEXEC_OUTPUT: &str = "forkexec: ok\n";

/// `rustc` flags for the nolibc programs: a static, position-dependent
/// executable for Linux's ABI (the bare-metal target only means "no OS
/// library": the code makes Linux system calls itself).
const RUSTC_FLAGS: &[&str] = &[
    "--edition=2024",
    "--target=x86_64-unknown-none",
    "-Cpanic=abort",
    "-Copt-level=2",
    "-Crelocation-model=static",
    "-Cdebuginfo=0",
    "-Cstrip=symbols",
    "-Clink-arg=--build-id=none",
];

fn out_dir() -> PathBuf {
    root().join(OUT)
}

/// Build `tests/linux/nolibc/hello.rs`; returns the ELF.
pub fn nolibc_hello() -> Result<Vec<u8>> {
    nolibc_program("tests/linux/nolibc/hello.rs", "hello")
}

/// Build `tools/xtask/programs/forkexec.rs`; returns the ELF.
pub fn nolibc_forkexec() -> Result<Vec<u8>> {
    nolibc_program("tools/xtask/programs/forkexec.rs", "forkexec")
}

/// Build the nolibc program `source` (relative to the repository) as
/// `target/xtask-programs/<name>`; returns the ELF.
fn nolibc_program(source: &str, name: &str) -> Result<Vec<u8>> {
    let out = out_dir();
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let binary = out.join(name);
    let status = Command::new("rustc")
        .current_dir(root())
        .args(RUSTC_FLAGS)
        .arg("-o")
        .arg(&binary)
        .arg(source)
        .status()
        .map_err(|e| format!("rustc: {e}"))?;
    if !status.success() {
        return Err(format!("building {source} failed ({status})"));
    }
    std::fs::read(&binary).map_err(|e| format!("{}: {e}", binary.display()))
}

/// The C programs of the corpus, built by the pipeline tool into
/// `target/xtask-programs/corpus`: `(name, ELF)` for `hello` and
/// `libc-hello` as far as they were built. Empty, with the reason, without
/// a C compiler.
pub fn c_hellos() -> (Vec<(String, Vec<u8>)>, Option<String>) {
    let dir = out_dir().join("corpus");
    let built = cargo()
        .current_dir(root())
        .args(["run", "--quiet", "--package", "poly-pipeline", "--", "build", "--out"])
        .arg(&dir)
        .output();
    match built {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let why = String::from_utf8_lossy(&o.stderr);
            return (Vec::new(), Some(format!("the C corpus did not build: {}", why.trim())));
        }
        Err(e) => return (Vec::new(), Some(format!("pipeline build: {e}"))),
    }
    let mut out = Vec::new();
    let mut missing = None;
    for (file, name) in [("hello", "hello-c"), ("libc-hello", "hello-libc")] {
        match std::fs::read(dir.join(file)) {
            Ok(elf) => out.push((name.to_string(), elf)),
            Err(_) => missing = Some(format!("no static C library on this host: {name} was not built")),
        }
    }
    (out, missing)
}

/// The test initramfs (`initramfs::test_image_with`) with the nolibc hello
/// as `/bin/hello`, for the self-test boots (the kernel's exec tests run
/// it).
pub fn test_initramfs() -> Result<PathBuf> {
    let hello = nolibc_hello()?;
    initramfs::write(&root(), "poly-test-initramfs.cpio", &initramfs::test_image_with(&[("hello", &hello)]))
}

/// One boot that runs `/bin/<program>` as the first process and checks
/// that it printed `output` and ended with exit status 0, and that QEMU
/// then exited with the kernel's success status.
fn run_as_init(kernel: &Path, syms: &Arc<Symbols>, image: &Path, program: &str, output: &str) -> Result<Boot> {
    let append = format!("poly.qemu_exit poly.init=/bin/{program}");
    let b = boot_on(None, Some(image), kernel, syms, DEFAULT_MEMORY, &append, Devices::None)?;
    let started = b.output.contains(&format!("POLY-INIT: exec /bin/{program}"));
    let exited = b.output.contains("POLY-EXIT: pid=1 code=0");
    let printed = b.output.contains(output.trim_end());
    if b.status != Some(QEMU_SUCCESS) || !started || !printed || !exited {
        return Err(format!(
            "/bin/{program} as the first process: started {started}, printed {:?} {printed}, \
             exit status 0 {exited}, QEMU status {:?}",
            output.trim_end(),
            b.status
        ));
    }
    Ok(b)
}

/// The boot tests of the hello-world milestone.
pub fn boot_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let hello = nolibc_hello()?;
    let forkexec = nolibc_forkexec()?;
    let (c_programs, skipped) = c_hellos();
    let mut programs: Vec<(&str, &[u8])> = vec![("hello", &hello), ("forkexec", &forkexec)];
    programs.extend(c_programs.iter().map(|(n, e)| (n.as_str(), e.as_slice())));
    let image = initramfs::write(&root(), "poly-programs-initramfs.cpio", &initramfs::programs_image(&programs))?;

    eprintln!("==> boot test: a static Linux hello world without a C library runs as the first process");
    let b = run_as_init(kernel, syms, &image, "hello", HELLO_OUTPUT)?;
    if let Some(line) = b.output.lines().find(|l| l.contains(" not implemented")) {
        return Err(format!("the nolibc hello made a system call Poly does not implement: {line}"));
    }
    for name in ["hello-c", "hello-libc"] {
        if c_programs.iter().any(|(n, _)| n == name) {
            let what =
                if name == "hello-libc" { "linked with the host's static C library" } else { "in freestanding C" };
            eprintln!("==> boot test: a static Linux hello world {what} runs as the first process");
            run_as_init(kernel, syms, &image, name, C_HELLO_OUTPUT)?;
        }
    }
    if let Some(why) = skipped {
        eprintln!("==> skipped: {why}");
    }

    eprintln!("==> boot test: init forks, execs, clones a thread with TLS, waits and reaps an orphan");
    let b = run_as_init(kernel, syms, &image, "forkexec", FORKEXEC_OUTPUT)?;
    if !b.output.contains(HELLO_OUTPUT.trim_end()) {
        return Err("the child of /bin/forkexec did not run /bin/hello".into());
    }
    if let Some(line) = b.output.lines().find(|l| l.contains(" not implemented")) {
        return Err(format!("/bin/forkexec made a system call Poly does not implement: {line}"));
    }

    eprintln!("==> boot test: a program that is not there is reported as unsupported");
    let b =
        boot_on(None, Some(&image), kernel, syms, DEFAULT_MEMORY, "poly.qemu_exit poly.init=/bin/nope", Devices::None)?;
    if b.status != Some(QEMU_SUCCESS) || !b.output.contains("POLY-INIT: unsupported /bin/nope: ENOENT") {
        return Err(format!("a missing init program was not reported (status {:?})", b.status));
    }
    Ok(())
}
