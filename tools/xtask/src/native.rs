//! Native Poly programs (ADR 0125): building `userland/native/` and the
//! native boot test.
//!
//! The programs are `no_std` + `alloc` Rust on `poly-rt`, for the
//! `x86_64-unknown-none` target, whose standard library (core, alloc) the
//! pinned toolchain ships and whose linker is `rust-lld`: no host C
//! toolchain and no unstable flags, so the build is the same on Windows
//! and Linux hosts. Cargo runs inside `userland/native/` so its
//! `.cargo/config.toml` (target, static non-PIE flags) applies; the
//! artifacts go to `target/native/`. Each built program is checked to be
//! what the kernel runs natively: an ELF whose first page holds Poly's ABI
//! note for this ABI version (`poly-binfmt`).
//!
//! `cargo xtask build` and every kernel build build them; `cargo xtask run`
//! puts them into its initramfs as `/bin/<name>`, so
//! `cargo xtask run poly.init=/bin/native-hello` boots into one.
//!
//! The boot test (group `native`): `/bin/native-hello` runs as the first
//! process with the Linux nolibc hello world next to it, and must report
//! every check passed (prints, arguments, the heap, time, files, native
//! children's exit, panic and fault, a Linux child) and exit 0; then
//! `/bin/native-child exit 7` as the first process must end with status 7.

use super::{Boot, DEFAULT_MEMORY, Devices, QEMU_SUCCESS, Result, boot_on, cargo, initramfs, programs, root, step};
use crate::symbols::Symbols;
use poly_personality_native::abi::ABI_VERSION;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The native programs `userland/native/` builds, as `/bin/<name>`.
pub const PROGRAMS: [&str; 2] = ["native-hello", "native-child"];

/// What `/bin/native-hello` prints last when every check passed.
pub const HELLO_OUTPUT: &str = "native-hello: done, 0 failed";

fn workspace() -> PathBuf {
    root().join("userland").join("native")
}

fn target_dir() -> PathBuf {
    root().join("target").join("native")
}

/// Build the native programs; returns `(name, ELF)` for each of
/// [`PROGRAMS`], checked to carry Poly's ABI note.
pub fn build() -> Result<Vec<(String, Vec<u8>)>> {
    let mut cmd = cargo();
    cmd.current_dir(workspace()).args(["build", "--release", "--target-dir"]).arg(target_dir());
    step("build native programs (userland/native)", &mut cmd)?;
    let dir = target_dir().join("x86_64-unknown-none").join("release");
    PROGRAMS
        .iter()
        .map(|name| {
            let path = dir.join(name);
            let elf = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            check(name, &elf)?;
            Ok((name.to_string(), elf))
        })
        .collect()
}

/// `ci lint`'s part: formatting and clippy of `userland/native/` (for its
/// target, with `poly-rt`'s native-only code: the entry point, the
/// allocator, the panic handler).
pub fn lint() -> Result {
    step("fmt (native)", cargo().current_dir(workspace()).args(["fmt", "--all", "--check"]))?;
    let mut clippy = cargo();
    clippy.current_dir(workspace()).args(["clippy", "--release", "--target-dir"]).arg(target_dir());
    step("clippy (native)", clippy.args(["--", "-D", "warnings"]))
}

/// Check that `elf` is a native program of this ABI version, as the
/// kernel decides from its first page.
fn check(name: &str, elf: &[u8]) -> Result {
    let first_page = &elf[..elf.len().min(4096)];
    match poly_binfmt::detect(first_page) {
        Ok(poly_binfmt::Format::Native { arch: poly_binfmt::Arch::X86_64, abi: ABI_VERSION }) => Ok(()),
        other => Err(format!(
            "{name}: not a native x86-64 Poly program of ABI version {ABI_VERSION} in its first page ({other:?}); \
             is poly-rt's ABI note linked in?"
        )),
    }
}

/// The initramfs of `cargo xtask run`: the default image with the native
/// programs in `/bin`.
pub fn run_image() -> Result<PathBuf> {
    let built = build()?;
    let programs: Vec<(&str, &[u8])> = built.iter().map(|(n, e)| (n.as_str(), e.as_slice())).collect();
    initramfs::write(&root(), "poly-initramfs.cpio", &initramfs::programs_image(&programs))
}

/// One boot running `/bin/<program> <args>` as the first process.
fn run_first(kernel: &Path, syms: &Arc<Symbols>, image: &Path, program: &str, args: &[&str]) -> Result<Boot> {
    let mut append = format!("poly.qemu_exit poly.init=/bin/{program}");
    for a in args {
        append.push_str(&format!(" poly.init.arg={a}"));
    }
    boot_on(None, Some(image), kernel, syms, DEFAULT_MEMORY, &append, Devices::None)
}

/// The native boot test (see the module docs).
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let built = build()?;
    let hello = programs::nolibc_hello()?;
    let mut list: Vec<(&str, &[u8])> = built.iter().map(|(n, e)| (n.as_str(), e.as_slice())).collect();
    list.push(("hello", &hello));
    let image = initramfs::write(&root(), "poly-native-initramfs.cpio", &initramfs::programs_image(&list))?;

    eprintln!(
        "==> boot test: a native Poly program runs as the first process: prints, allocates, keeps time, \
         reads and writes files, spawns native and Linux children and checks how they end"
    );
    let b = run_first(kernel, syms, &image, "native-hello", &["--greeting"])?;
    let checks = [
        ("it started in the native personality", b.output.contains("POLY-INIT: exec /bin/native-hello")),
        ("it saw its arguments", b.output.contains(r#"native-hello: arguments ["/bin/native-hello", "--greeting"]"#)),
        ("its native child exited with 42", b.output.contains("native-child: exiting with 42")),
        ("its native child's panic was printed", b.output.contains("panic: ") && b.output.contains("on purpose")),
        ("its Linux child printed on the shared console", b.output.contains(programs::HELLO_OUTPUT.trim_end())),
        ("every check passed", b.output.contains(HELLO_OUTPUT)),
        ("it exited with status 0", b.output.contains("POLY-EXIT: pid=1 code=0")),
        ("QEMU exited with the kernel's success status", b.status == Some(QEMU_SUCCESS)),
    ];
    for (what, ok) in checks {
        if !ok {
            let fails: Vec<&str> = b.output.lines().filter(|l| l.contains("native-hello: FAIL")).collect();
            return Err(format!("/bin/native-hello: {what} failed ({})", fails.join("; ")));
        }
    }
    if let Some(line) = b.output.lines().find(|l| l.contains(" not implemented")) {
        return Err(format!("/bin/native-hello made a call Poly does not implement: {line}"));
    }

    eprintln!("==> boot test: a native program's exit status is the first process's status");
    let b = run_first(kernel, syms, &image, "native-child", &["exit", "7"])?;
    if b.status != Some(QEMU_SUCCESS) || !b.output.contains("POLY-EXIT: pid=1 code=7") {
        return Err(format!("/bin/native-child exit 7 did not end with status 7 (QEMU status {:?})", b.status));
    }
    Ok(())
}
