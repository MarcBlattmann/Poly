//! Developer and CI entry point: `cargo xtask <command>`.
//!
//! Commands:
//!   build [--release]   build the kernel
//!   run   [--release]   boot the kernel in QEMU (serial on stdio, Ctrl-A X quits)
//!   test                host unit tests + kernel boot tests in QEMU
//!   ci                  everything CI checks: fmt, clippy, tests, coverage
//!   coverage <args>     run the coverage tool (see tools/coverage)

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

type Result<T = ()> = std::result::Result<T, String>;

const BOOT_TIMEOUT: Duration = Duration::from_secs(60);
/// Must match `selftest::PASS_MARKER` in the kernel.
const PASS_MARKER: &str = "POLY-SELFTEST: PASS";
/// QEMU exit statuses produced by the kernel's `qemu::ExitCode`.
const QEMU_SUCCESS: i32 = 33;
const QEMU_FAILURE: i32 = 35;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let release = args.iter().any(|a| a == "--release");
    let result = match args.first().map(String::as_str) {
        Some("build") => build_kernel(release).map(|_| ()),
        Some("run") => run(release),
        Some("test") => test(),
        Some("ci") => ci(),
        Some("coverage") => coverage(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask <build|run|test|ci|coverage> [--release]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf()
}

fn kernel_dir() -> PathBuf {
    root().join("kernel")
}

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

fn step(name: &str, cmd: &mut Command) -> Result {
    eprintln!("==> {name}");
    let status = cmd.status().map_err(|e| format!("{name}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{name} failed ({status})")) }
}

/// Builds from inside kernel/ so kernel/.cargo/config.toml (target, flags) applies.
fn build_kernel(release: bool) -> Result<PathBuf> {
    let mut cmd = cargo();
    cmd.current_dir(kernel_dir()).arg("build");
    if release {
        cmd.arg("--release");
    }
    step("build kernel", &mut cmd)?;
    let profile = if release { "release" } else { "debug" };
    Ok(kernel_dir().join("target/x86_64-unknown-none").join(profile).join("poly-kernel"))
}

/// Guest RAM for `cargo xtask run` and most boot tests.
const DEFAULT_MEMORY: &str = "256M";

fn qemu(kernel: &Path, memory: &str, append: &str) -> Command {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-m", memory, "-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-kernel")
        .arg(kernel)
        .args(["-append", append]);
    cmd
}

fn run(release: bool) -> Result {
    let kernel = build_kernel(release)?;
    eprintln!("==> boot (Ctrl-A X to quit)");
    let _ = qemu(&kernel, DEFAULT_MEMORY, "").status().map_err(|e| format!("qemu: {e}"))?;
    Ok(())
}

struct Boot {
    status: Option<i32>,
    output: String,
}

/// Boot headless with a timeout, echoing serial output as it arrives.
fn boot(kernel: &Path, memory: &str, append: &str) -> Result<Boot> {
    let mut child = qemu(kernel, memory, append)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            println!("    | {line}");
            let _ = tx.send(line);
        }
    });

    let deadline = Instant::now() + BOOT_TIMEOUT;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s.code();
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let _ = reader.join();
    let output = rx.try_iter().collect::<Vec<_>>().join("\n");
    Ok(Boot { status, output })
}

fn boot_tests() -> Result {
    let kernel = build_kernel(false)?;

    // 2 GiB puts RAM above the 1 GiB boot mapping: the frame allocator must
    // manage it without touching it.
    for memory in [DEFAULT_MEMORY, "2G"] {
        eprintln!("==> boot test: self-test passes with {memory} of RAM");
        let b = boot(&kernel, memory, "poly.qemu_exit")?;
        match b.status {
            Some(QEMU_SUCCESS) if b.output.contains(PASS_MARKER) => {}
            Some(s) => return Err(format!("self-test boot ({memory}) exited with status {s}")),
            None => return Err(format!("self-test boot ({memory}) timed out after {BOOT_TIMEOUT:?}")),
        }
    }

    eprintln!("==> boot test: page fault is caught and reported");
    let b = boot(&kernel, DEFAULT_MEMORY, "poly.qemu_exit poly.selftest.page_fault")?;
    let reported = b.output.contains("KERNEL PANIC") && b.output.contains("CPU exception 14 (page fault)");
    if b.status != Some(QEMU_FAILURE) || !reported {
        return Err(format!("page fault was not reported as expected (status {:?})", b.status));
    }
    eprintln!("==> boot tests passed");
    Ok(())
}

fn test() -> Result {
    step("host tests", cargo().current_dir(root()).args(["test", "--workspace"]))?;
    boot_tests()
}

fn ci() -> Result {
    step("fmt (host)", cargo().current_dir(root()).args(["fmt", "--all", "--check"]))?;
    step("fmt (kernel)", cargo().current_dir(kernel_dir()).args(["fmt", "--check"]))?;
    step(
        "clippy (host)",
        cargo().current_dir(root()).args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    )?;
    step("clippy (kernel)", cargo().current_dir(kernel_dir()).args(["clippy", "--", "-D", "warnings"]))?;
    test()?;
    coverage(&["check".into()])?;
    coverage(&["readme".into(), "--check".into()])
}

fn coverage(args: &[String]) -> Result {
    step(
        "coverage",
        cargo().current_dir(root()).args(["run", "--quiet", "--package", "poly-coverage", "--"]).args(args),
    )
}
