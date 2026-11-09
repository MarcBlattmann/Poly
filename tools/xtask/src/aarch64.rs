//! `cargo xtask aarch64 build|run|test`: the aarch64 boot stub of
//! `kernel/src/arch/aarch64` (ADR 0154) on QEMU's `virt` machine.
//!
//! - `build [--release]`: build it for `aarch64-unknown-none` (pure Rust,
//!   linked by `rust-lld`; no host C toolchain).
//! - `run [--release] [kernel args...]`: boot it with the serial port on the
//!   terminal (Ctrl-A X quits QEMU); the arguments become the kernel command
//!   line (`-append`), e.g. `poly.selftest.panic`.
//! - `test [--release]`: the library's unit tests on the host, then boots
//!   under QEMU's emulator (TCG) in three configurations and checks the
//!   self-test's `ok <name>` lines, its verdict line and QEMU's exit status,
//!   which the stub sets through Arm semihosting (`-semihosting-config
//!   enable=on`; PSCI `SYSTEM_OFF` carries no status).
//!
//! Without `qemu-system-aarch64` the boots are skipped, saying so.

use crate::{Result, cargo, kernel_dir, step};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// QEMU's binary name (`qemu-system-aarch64.exe` on Windows is found by the
/// same name).
const QEMU: &str = "qemu-system-aarch64";

/// How long one boot may run before it is killed. The stub takes a second
/// under TCG; this is for a heavily loaded host.
const TIMEOUT: Duration = Duration::from_secs(90);

/// The self-test's checks, which must all appear as `ok` (so a check that
/// silently disappears from the stub fails). Keep in step with
/// `kernel/src/arch/aarch64/src/selftest.rs`.
const REQUIRED: [&str; 22] = [
    "entry-el",
    "running-at-el1",
    "vectors-installed",
    "fp-simd",
    "dtb-found",
    "dtb-memory",
    "dtb-uart",
    "dtb-clear-of-image",
    "exception-brk",
    "exception-svc",
    "timer-frequency",
    "timer-advances",
    "mmu-enable",
    "mmu-ram-readwrite",
    "mmu-translation-fault",
    "frames-available",
    "frames-allocate",
    "frames-free",
    "heap-small",
    "heap-small-freed",
    "heap-large",
    "heap-large-freed",
];

/// Must match `poly_aarch64_stub::report`.
const PASS_MARKER: &str = "POLY-A64-SELFTEST: PASS";
/// Must match `poly_aarch64_stub::semihost::status`.
const STATUS_PASS: i32 = 0;
const STATUS_PANIC: i32 = 2;
/// The kernel command line that makes the stub panic at the end.
const PANIC_FLAG: &str = "poly.selftest.panic";

fn dir() -> PathBuf {
    kernel_dir().join("src/arch/aarch64")
}

/// `cargo` run in the stub's directory, where `.cargo/config.toml` sets the
/// target and flags.
fn stub_cargo() -> Command {
    let mut cmd = cargo();
    cmd.current_dir(dir());
    cmd
}

/// The host's target triple, for the library's unit tests (the directory's
/// config makes `aarch64-unknown-none` the default target).
fn host_triple() -> Result<String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = Command::new(rustc).arg("-vV").current_dir(dir()).output().map_err(|e| format!("rustc -vV: {e}"))?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_string)
        .ok_or_else(|| "rustc -vV printed no host".to_string())
}

/// Format and lint the stub: rustfmt, clippy for the target and for the
/// library's host tests. Part of `cargo xtask ci lint`.
pub fn lint() -> Result {
    step("fmt (aarch64 stub)", stub_cargo().args(["fmt", "--check"]))?;
    step("clippy (aarch64 stub)", stub_cargo().args(["clippy", "--", "-D", "warnings"]))?;
    let host = host_triple()?;
    step(
        "clippy (aarch64 stub, host tests)",
        stub_cargo().args(["clippy", "--lib", "--tests", "--target", &host, "--", "-D", "warnings"]),
    )
}

/// The library's unit tests, on the host.
fn host_tests() -> Result {
    let host = host_triple()?;
    step("aarch64 stub unit tests", stub_cargo().args(["test", "--lib", "--target", &host]))
}

/// Build the stub; the ELF's path.
fn build(release: bool) -> Result<PathBuf> {
    let mut cmd = stub_cargo();
    cmd.arg("build");
    if release {
        cmd.arg("--release");
    }
    step("aarch64 stub", &mut cmd)?;
    let profile = if release { "release" } else { "debug" };
    let elf = dir().join("target/aarch64-unknown-none").join(profile).join("poly-kernel-aarch64");
    if elf.exists() { Ok(elf) } else { Err(format!("{} was not built", elf.display())) }
}

/// Whether QEMU's aarch64 system emulator can be run.
fn have_qemu() -> bool {
    Command::new(QEMU).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// A boot of the stub that is judged by [`judge`].
struct Case {
    name: &'static str,
    /// `-machine`.
    machine: &'static str,
    cpus: u32,
    /// The kernel command line.
    append: &'static str,
    /// The exception level the stub is entered at, or `None` for the case
    /// that ends in a panic.
    entry_el: Option<u32>,
}

const CASES: [Case; 3] = [
    // QEMU's default: the CPU starts in EL1; the second CPU is parked.
    Case { name: "EL1, 2 CPUs", machine: "virt", cpus: 2, append: "", entry_el: Some(1) },
    // The CPU starts in EL2 and the stub drops to EL1.
    Case { name: "EL2", machine: "virt,virtualization=on", cpus: 1, append: "", entry_el: Some(2) },
    // The panic path: the self-test passes, then the stub panics on purpose.
    Case { name: "panic", machine: "virt", cpus: 1, append: PANIC_FLAG, entry_el: None },
];

/// QEMU's command line for one boot of `elf`.
fn qemu(elf: &std::path::Path, machine: &str, cpus: u32, append: &str, serial: &str) -> Command {
    let mut cmd = Command::new(QEMU);
    cmd.args(["-machine", machine, "-cpu", "cortex-a72", "-smp", &cpus.to_string(), "-m", "256M"])
        .args(["-display", "none", "-monitor", "none", "-no-reboot", "-serial", serial])
        .args(["-semihosting-config", "enable=on,target=native"])
        .arg("-kernel")
        .arg(elf);
    if !append.is_empty() {
        cmd.args(["-append", append]);
    }
    cmd
}

/// What a boot produced.
struct Boot {
    /// QEMU's exit status; `None` if killed (at the timeout, or by a signal).
    status: Option<i32>,
    output: String,
    timed_out: bool,
}

/// Run `cmd` (QEMU with the serial port on its standard output), echoing the
/// serial lines as they come, for up to [`TIMEOUT`].
fn boot(mut cmd: Command) -> Result<Boot> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("{QEMU}: {e} (is QEMU installed?)"))?;
    let stdout = child.stdout.take().ok_or("no QEMU output pipe")?;
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).split(b'\n') {
            let Ok(line) = line else { break };
            if tx.send(String::from_utf8_lossy(&line).trim_end().to_string()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + TIMEOUT;
    let mut output = String::new();
    let mut timed_out = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => {
                eprintln!("  | {line}");
                output.push_str(&line);
                output.push('\n');
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!("==> QEMU timed out after {} s; killing it", TIMEOUT.as_secs());
                let _ = child.kill();
                timed_out = true;
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(|e| format!("waiting for QEMU: {e}"))?.code();
    let _ = reader.join();
    Ok(Boot { status, output, timed_out })
}

/// What is wrong with `b` as the output of `case` (nothing: it passed).
fn judge(case: &Case, b: &Boot) -> Vec<String> {
    let mut problems = Vec::new();
    if b.timed_out {
        problems.push("timed out".to_string());
    }
    let want_status = if case.entry_el.is_some() { STATUS_PASS } else { STATUS_PANIC };
    if b.status != Some(want_status) {
        problems.push(format!("QEMU status {:?}, expected {want_status}", b.status));
    }
    let lines: Vec<&str> = b.output.lines().map(str::trim_end).collect();
    for line in lines.iter().filter(|l| l.starts_with("not ok ")) {
        problems.push(format!("failed check: {line}"));
    }
    for name in REQUIRED {
        let ok = format!("ok {name}");
        if !lines.contains(&ok.as_str()) {
            problems.push(format!("no `{ok}` line"));
        }
    }
    if !lines.iter().any(|l| l.starts_with(PASS_MARKER)) {
        problems.push(format!("no `{PASS_MARKER}` line"));
    }
    match case.entry_el {
        Some(el) => {
            let entered = format!("CurrentEL: EL1 (entered at EL{el})");
            if !lines.contains(&entered.as_str()) {
                problems.push(format!("no `{entered}` line"));
            }
        }
        None => {
            if !lines.iter().any(|l| l.starts_with("KERNEL PANIC: ")) || !b.output.contains(PANIC_FLAG) {
                problems.push("no `KERNEL PANIC` line with the panic message".into());
            }
        }
    }
    problems
}

/// The boots of [`CASES`] on `elf`.
fn boot_tests(elf: &std::path::Path) -> Result {
    if !have_qemu() {
        eprintln!("==> skipped: {QEMU} is not installed (Debian and Ubuntu: qemu-system-arm; Windows: qemu.org)");
        return Ok(());
    }
    let mut failed = Vec::new();
    for case in &CASES {
        eprintln!("==> aarch64 boot: {} (-machine {})", case.name, case.machine);
        let b = boot(qemu(elf, case.machine, case.cpus, case.append, "stdio"))?;
        let problems = judge(case, &b);
        if problems.is_empty() {
            eprintln!("==> aarch64 boot {}: ok", case.name);
        } else {
            failed.push(format!("{}: {}", case.name, problems.join("; ")));
        }
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("aarch64 boot test failed: {}", failed.join(" | "))) }
}

/// The `aarch64` part of `cargo xtask ci` (CI's job of that name): the
/// library's tests, the build, the boots.
pub fn ci() -> Result {
    host_tests()?;
    let elf = build(false)?;
    boot_tests(&elf)
}

pub fn command(args: &[String]) -> Result {
    let release = args.iter().any(|a| a == "--release");
    let rest: Vec<&str> = args.iter().skip(1).filter(|a| *a != "--release").map(String::as_str).collect();
    match args.first().map(String::as_str) {
        Some("build") => build(release).map(|elf| eprintln!("==> {}", elf.display())),
        Some("run") => {
            let elf = build(release)?;
            if !have_qemu() {
                return Err(format!("{QEMU} is not installed"));
            }
            eprintln!("==> Ctrl-A X quits QEMU");
            let status =
                qemu(&elf, "virt", 1, &rest.join(" "), "mon:stdio").status().map_err(|e| format!("{QEMU}: {e}"))?;
            // 0 is the stub's pass; show any other status the stub chose.
            match status.code() {
                Some(0) => Ok(()),
                other => Err(format!("QEMU status {other:?}")),
            }
        }
        Some("test") => {
            host_tests()?;
            let elf = build(release)?;
            boot_tests(&elf)
        }
        _ => Err("usage: cargo xtask aarch64 <build|run|test> [--release] (run: [kernel args...])".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_output(el: u32) -> String {
        let mut out = format!("Poly aarch64 boot stub\nCurrentEL: EL1 (entered at EL{el})\n");
        for name in REQUIRED {
            out.push_str(&format!("ok {name}\n"));
        }
        out.push_str("POLY-A64-SELFTEST: PASS (22 passed, 0 failed)\n");
        out
    }

    fn boot_of(status: Option<i32>, output: String) -> Boot {
        Boot { status, output, timed_out: false }
    }

    #[test]
    fn a_complete_passing_boot_has_no_problems() {
        assert!(judge(&CASES[0], &boot_of(Some(0), passing_output(1))).is_empty());
        assert!(judge(&CASES[1], &boot_of(Some(0), passing_output(2))).is_empty());
    }

    #[test]
    fn the_panic_case_wants_the_panic_and_status_2() {
        let mut out = passing_output(1);
        out.push_str("KERNEL PANIC: panicked at src/main.rs:1:1:\npoly.selftest.panic: on purpose\n");
        assert!(judge(&CASES[2], &boot_of(Some(2), out.clone())).is_empty());
        assert!(!judge(&CASES[2], &boot_of(Some(0), out)).is_empty());
        assert!(!judge(&CASES[2], &boot_of(Some(2), passing_output(1))).is_empty(), "no panic message");
    }

    #[test]
    fn failures_are_reported() {
        let out = passing_output(1).replace("ok heap-large\n", "not ok heap-large # sum 1\n");
        let problems = judge(&CASES[0], &boot_of(Some(1), out));
        assert!(problems.iter().any(|p| p.contains("QEMU status Some(1)")));
        assert!(problems.iter().any(|p| p.contains("failed check: not ok heap-large")));
        assert!(problems.iter().any(|p| p.contains("no `ok heap-large` line")));
        // A check that vanished from the stub is as bad as one that failed.
        let out = passing_output(1).replace("ok mmu-enable\n", "");
        assert!(judge(&CASES[0], &boot_of(Some(0), out)).iter().any(|p| p.contains("ok mmu-enable")));
        // Entered at the wrong level, or never finished.
        assert!(!judge(&CASES[1], &boot_of(Some(0), passing_output(1))).is_empty());
        let killed = Boot { status: None, output: "Poly aarch64 boot stub\n".into(), timed_out: true };
        assert!(judge(&CASES[0], &killed).iter().any(|p| p == "timed out"));
    }
}
