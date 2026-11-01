//! The Wayland compositor on Poly (roadmap Phase 6, the compositor's
//! groundwork; ADR 0106): `poly-compositor` and the C test client
//! `tests/linux/wayland/wl-client.c` in one boot.
//!
//! - `/bin/poly-compositor`: `userland/poly-compositor`, built for
//!   `x86_64-unknown-linux-gnu` with the host's static glibc
//!   (`-C target-feature=+crt-static`, not PIE), into its own target
//!   directory so the host build keeps its flags.
//! - `/bin/wl-client`: the client, `cc -static`. Without a C compiler or a
//!   static C library the test is skipped with the reason.
//!
//! The compositor runs as the first process (`poly.init`, its arguments
//! `poly.init.arg=` words): it listens on `/run/user/0/wayland-0`, starts
//! the client with `--exec`, and ends with the client's status. The client
//! goes through the whole protocol path (registry, SHM pool from a file in
//! `/dev/shm` passed with `SCM_RIGHTS`, xdg_toplevel with a server-side
//! decoration, configure and ack, commit, frame callback, release), asks
//! the compositor for a picture with `SIGUSR1` and finds its pixels in it.
//! The boot passes when the client printed `wl-client: ok`, the compositor
//! reported the window and the probed pixel inside it, and both exited 0.

use super::{Boot, DEFAULT_MEMORY, Devices, QEMU_SUCCESS, Result, boot_on, cargo, initramfs, root};
use crate::symbols::Symbols;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

/// Where the programs are built.
const OUT: &str = "target/xtask-programs";
/// The compositor's own target directory.
const TARGET_DIR: &str = "target/xtask-compositor";
const TRIPLE: &str = "x86_64-unknown-linux-gnu";
/// The client's source.
const CLIENT: &str = "tests/linux/wayland/wl-client.c";

/// What the client prints when every check passed.
pub const CLIENT_OK: &str = "wl-client: ok";
/// The output the boot test composes for.
const SIZE: &str = "640x480";
/// A pixel inside the client's window: its first window is placed at
/// (33, 57) (the scene's cascade, after the frame), and the left half of
/// its buffer is `0x2060c0`.
const PROBE: (&str, &str) = ("40,60", "#2060c0");

/// Build `poly-compositor` as a static Linux program; returns the ELF.
pub fn build_compositor() -> Result<Vec<u8>> {
    let status = cargo()
        .current_dir(root())
        .args(["build", "--release", "--quiet", "--package", "poly-compositor", "--target", TRIPLE, "--target-dir"])
        .arg(root().join(TARGET_DIR))
        .env("RUSTFLAGS", "-C target-feature=+crt-static -C relocation-model=static -C strip=symbols")
        .status()
        .map_err(|e| format!("cargo build poly-compositor: {e}"))?;
    if !status.success() {
        return Err(format!("building poly-compositor failed ({status})"));
    }
    let elf = root().join(TARGET_DIR).join(TRIPLE).join("release/poly-compositor");
    std::fs::read(&elf).map_err(|e| format!("{}: {e}", elf.display()))
}

/// Build `notify-send` (`userland/poly-notify`, ADR 0117) as a static Linux
/// program, like the compositor; returns the ELF.
pub fn build_notify_send() -> Result<Vec<u8>> {
    let status = cargo()
        .current_dir(root())
        .args(["build", "--release", "--quiet", "--package", "poly-notify", "--target", TRIPLE, "--target-dir"])
        .arg(root().join(TARGET_DIR))
        .env("RUSTFLAGS", "-C target-feature=+crt-static -C relocation-model=static -C strip=symbols")
        .status()
        .map_err(|e| format!("cargo build poly-notify: {e}"))?;
    if !status.success() {
        return Err(format!("building notify-send failed ({status})"));
    }
    let elf = root().join(TARGET_DIR).join(TRIPLE).join("release/notify-send");
    std::fs::read(&elf).map_err(|e| format!("{}: {e}", elf.display()))
}

/// Build the C client statically: the ELF, or why not.
pub fn build_client() -> std::result::Result<Vec<u8>, String> {
    build_c(CLIENT, "wl-client")
}

/// Build the C program `source` statically as `name`: the ELF, or why not.
pub fn build_c(source: &str, name: &str) -> std::result::Result<Vec<u8>, String> {
    let out = root().join(OUT);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let binary = out.join(name);
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let built = Command::new(&cc)
        .current_dir(root())
        .args(["-static", "-O2", "-Wall", "-Wextra", "-Werror", "-Wl,--build-id=none", "-o"])
        .arg(&binary)
        .arg(source)
        .output()
        .map_err(|e| format!("no C compiler ({cc}: {e})"))?;
    if !built.status.success() {
        return Err(format!("{source} did not build statically: {}", String::from_utf8_lossy(&built.stderr).trim()));
    }
    std::fs::read(&binary).map_err(|e| format!("{}: {e}", binary.display()))
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let client = match build_client() {
        Ok(elf) => elf,
        Err(why) => {
            eprintln!("==> skipped: the Wayland boot test: {why}");
            return Ok(());
        }
    };
    let compositor = build_compositor()?;
    let image = initramfs::programs_image(&[("poly-compositor", &compositor), ("wl-client", &client)]);
    let image = initramfs::write(&root(), "poly-wayland-initramfs.cpio", &image)?;
    eprintln!(
        "==> boot test: the Wayland compositor and a C client on Poly (SHM buffer, xdg_toplevel, frame callback)"
    );
    let args = ["--size", SIZE, "--probe", PROBE.0, "--exec", "/bin/wl-client"];
    let append = args
        .iter()
        .fold("poly.qemu_exit poly.init=/bin/poly-compositor".to_string(), |a, w| format!("{a} poly.init.arg={w}"));
    let b = boot_on(None, Some(&image), kernel, syms, DEFAULT_MEMORY, &append, Devices::None)?;
    check(&b)
}

fn check(b: &Boot) -> Result {
    let out = &b.output;
    for line in out.lines().filter(|l| l.starts_with("not ok ") || l.contains(" not implemented")) {
        eprintln!("    {line}");
    }
    let probe = format!("poly-compositor: pixel {} = {}", PROBE.0, PROBE.1);
    let checks = [
        ("the compositor listened on its socket", out.contains("poly-compositor: listening on /run/user/0/wayland-0")),
        ("the client passed every check", out.contains(CLIENT_OK)),
        ("the compositor showed the client's window", out.contains("poly-compositor: window \"wl-client\"")),
        ("the window's pixels were composed", out.contains(&probe)),
        ("the client exited 0", out.contains("poly-compositor: client exited with status 0")),
        ("the compositor exited 0", out.contains("POLY-EXIT: pid=1 code=0")),
    ];
    for (what, ok) in checks {
        if !ok {
            return Err(format!("the Wayland boot test: {what} failed (QEMU status {:?})", b.status));
        }
    }
    if b.status != Some(QEMU_SUCCESS) {
        return Err(format!("the Wayland boot test: QEMU status {:?}", b.status));
    }
    Ok(())
}
