//! Native Poly programs (ADR 0125): building `userland/native/` and the
//! native boot test.
//!
//! The programs are `no_std` + `alloc` Rust on `poly-rt`, for Poly's own
//! target `x86_64-unknown-poly` (`userland/native/x86_64-unknown-poly.json`:
//! `x86_64-unknown-none` with SSE2 floats, ADR 0126), whose `core` and
//! `alloc` are built from the toolchain's `rust-src` (`build-std`, under
//! `RUSTC_BOOTSTRAP=1`, as for `ntdll`) and whose linker is `rust-lld`: no
//! host C toolchain, so the build is the same on Windows and Linux hosts.
//! Cargo runs inside `userland/native/` so its `.cargo/config.toml`
//! (target, build-std) applies; the
//! artifacts go to `target/native/` (the checkout and the toolchain
//! remapped to `/poly` and `/rust` in what they carry). Each built program
//! is checked to be what the kernel runs natively: an ELF whose first page
//! holds Poly's ABI note for this ABI version (`poly-binfmt`).
//!
//! The programs: the test programs `native-hello` and `native-child`, the
//! compositor and desktop `poly-compositor`, `notify-send` (ADR 0126), and
//! the desktop's Wayland test clients `wl-demo` and `wl-client` (ADR 0133;
//! `compositor.rs` and the desktop boot tests take theirs from here).
//! They are built once per xtask run ([`build`] keeps the result).
//! `cargo xtask build` and every kernel build build them; `cargo xtask run`
//! puts them into its initramfs as `/bin/<name>`, so
//! `cargo xtask run poly.init=/bin/native-hello` boots into one.
//!
//! The boot test (group `native`): `/bin/native-hello` runs as the first
//! process with the Linux nolibc hello world next to it, and must report
//! every check passed (prints, arguments, the heap, time, files, native
//! children's exit, panic and fault, a Linux child, and the calls of ABI
//! version 2: directories, pipes and `poll`, memory objects, sockets
//! passing handles, process handles) and exit 0; then
//! `/bin/native-child exit 7` as the first process must end with status 7.

use super::{Boot, DEFAULT_MEMORY, Devices, QEMU_SUCCESS, Result, boot_on, cargo, initramfs, programs, root, step};
use crate::symbols::Symbols;
use poly_personality_native::abi::ABI_VERSION;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The native programs `userland/native/` builds, as `/bin/<name>`: the
/// test programs, the compositor and desktop, `notify-send`, the
/// desktop's Wayland test clients, and Poly's Files and Terminal.
pub const PROGRAMS: [&str; 8] = [
    "native-hello",
    "native-child",
    "poly-compositor",
    "notify-send",
    "wl-demo",
    "wl-client",
    "poly-files",
    "poly-terminal",
];

/// What `/bin/native-hello` prints last when every check passed.
pub const HELLO_OUTPUT: &str = "native-hello: done, 0 failed";

fn workspace() -> PathBuf {
    root().join("userland").join("native")
}

fn target_dir() -> PathBuf {
    root().join("target").join("native")
}

/// `cargo` in `userland/native/` with the checkout's path remapped to
/// `/poly` and the toolchain's to `/rust` in what the compiler writes
/// (panic locations), so a program does not carry the host's directories.
fn native_cargo() -> std::process::Command {
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    let mut flags = vec![format!("'--remap-path-prefix={}=/poly'", root().display())];
    flags.extend(sysroot.map(|s| format!("'--remap-path-prefix={s}=/rust'")));
    let remap = format!("target.x86_64-unknown-poly.rustflags=[{}]", flags.join(", "));
    let mut cmd = cargo();
    // Poly's target builds core and alloc from rust-src (build-std).
    cmd.current_dir(workspace()).env("RUSTC_BOOTSTRAP", "1").args(["--config", &remap]);
    cmd
}

/// Build the native programs; returns `(name, ELF)` for each of
/// [`PROGRAMS`], checked to carry Poly's ABI note. The first successful
/// build of an xtask run is kept: every later call (the kernel's build, the
/// desktop's image, the boot tests) gets it without running cargo again,
/// and calls from several threads at once wait for the one build.
pub fn build() -> Result<Programs> {
    static BUILT: std::sync::Mutex<Option<Programs>> = std::sync::Mutex::new(None);
    let mut built = BUILT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(programs) = built.as_ref() {
        return Ok(programs.clone());
    }
    let programs = build_now()?;
    *built = Some(programs.clone());
    Ok(programs)
}

/// The built programs: `(name, ELF)`.
pub type Programs = Vec<(String, Vec<u8>)>;

fn build_now() -> Result<Programs> {
    let dir = target_dir().join("x86_64-unknown-poly").join("release");
    // CI's boot jobs take the build job's programs (prebuilt.rs).
    let outputs: Vec<PathBuf> = PROGRAMS.iter().map(|name| dir.join(name)).collect();
    if !crate::prebuilt::skip("the native programs", &outputs)? {
        let mut cmd = native_cargo();
        cmd.args(["build", "--release", "--target-dir"]).arg(target_dir());
        step("build native programs (userland/native)", &mut cmd)?;
    }
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

/// Build the native programs and return the one called `name`.
pub fn program(name: &str) -> Result<Vec<u8>> {
    build()?.into_iter().find(|(n, _)| n == name).map(|(_, elf)| elf).ok_or_else(|| format!("no native program {name}"))
}

/// `ci lint`'s part: formatting and clippy of `userland/native/` (for its
/// target, with `poly-rt`'s native-only code: the entry point, the
/// allocator, the panic handler).
pub fn lint() -> Result {
    step("fmt (native)", cargo().current_dir(workspace()).args(["fmt", "--all", "--check"]))?;
    let mut clippy = native_cargo();
    clippy.args(["clippy", "--release", "--target-dir"]).arg(target_dir());
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
