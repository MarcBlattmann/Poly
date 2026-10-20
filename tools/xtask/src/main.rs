//! Developer and CI entry point: `cargo xtask <command>`.
//!
//! Commands:
//!   build [--release]   build the kernel
//!   run   [--release] [--gdb] [--no-disk] [kernel args...]
//!                       boot the kernel in QEMU (serial on stdio, Ctrl-A X quits)
//!                       with the test disk attached (--no-disk: without);
//!                       --gdb waits for a debugger on localhost:1234 (docs/DEBUGGING.md)
//!   test                host unit tests + kernel boot tests in QEMU
//!   ci                  everything CI checks: fmt, clippy, tests, coverage
//!   symbolize [--release] [--elf <path>] [addr...]
//!                       print the kernel function at each address; with no
//!                       addresses, add symbols to the backtrace in a log on stdin
//!   coverage <args>     run the coverage tool (see tools/coverage)

mod ktest;
mod symbols;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use symbols::Symbols;

type Result<T = ()> = std::result::Result<T, String>;

const BOOT_TIMEOUT: Duration = Duration::from_secs(60);
/// Must match `selftest::PASS_MARKER` in the kernel.
const PASS_MARKER: &str = "POLY-SELFTEST: PASS";
/// QEMU exit statuses produced by the kernel's `qemu::ExitCode`.
const QEMU_SUCCESS: i32 = 33;
const QEMU_FAILURE: i32 = 35;
/// CPUs QEMU emulates; the self-test boot checks ACPI reports exactly these.
const QEMU_CPUS: u32 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let release = args.iter().any(|a| a == "--release");
    let result = match args.first().map(String::as_str) {
        Some("build") => build_kernel(release).map(|_| ()),
        Some("run") => run(&args[1..]),
        Some("test") => test(),
        Some("ci") => ci(),
        Some("symbolize") => symbolize(&args[1..]),
        Some("coverage") => coverage(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask <build|run|test|ci|symbolize|coverage> [--release]");
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
    Ok(kernel_elf(release))
}

/// Where `build_kernel` puts the kernel ELF.
fn kernel_elf(release: bool) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    kernel_dir().join("target/x86_64-unknown-none").join(profile).join("poly-kernel")
}

/// Guest RAM for `cargo xtask run` and most boot tests.
const DEFAULT_MEMORY: &str = "256M";

/// Kernel flag saying the test disk is attached; the kernel's virtio-blk
/// tests then require it (`virtio_blk::TEST_DISK_FLAG`).
const TEST_DISK_FLAG: &str = "poly.test.disk";
/// Size of the test disk in 512-byte sectors (`virtio_blk::TEST_DISK_SECTORS`).
const TEST_DISK_SECTORS: u32 = 2048;

/// Sector `sector` of the test disk: "POLY", the sector number (le32), then
/// bytes that depend on the sector and the position. Must match
/// `virtio_blk::test_pattern` in the kernel.
fn test_disk_sector(sector: u32) -> [u8; 512] {
    let mut s = [0u8; 512];
    s[..4].copy_from_slice(b"POLY");
    s[4..8].copy_from_slice(&sector.to_le_bytes());
    for (i, b) in s.iter_mut().enumerate().skip(8) {
        *b = sector.wrapping_mul(31).wrapping_add(i as u32) as u8;
    }
    s
}

/// Write a fresh test disk image (1 MiB, raw) into the target directory and
/// return its path. Rewritten on every boot so a write test that went wrong
/// cannot leak into the next run.
fn test_disk() -> Result<PathBuf> {
    let path = root().join("target").join("poly-test-disk.img");
    let image: Vec<u8> = (0..TEST_DISK_SECTORS).flat_map(test_disk_sector).collect();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&path, image).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// QEMU for `kernel`, with the serial port on `serial` (a QEMU character
/// device); `disk` is attached as a modern-only virtio-blk PCI device.
fn qemu(kernel: &Path, memory: &str, append: &str, disk: Option<&Path>, serial: &str) -> Command {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-m", memory, "-display", "none", "-serial", serial, "-no-reboot"])
        .args(["-smp", &QEMU_CPUS.to_string()])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-kernel")
        .arg(kernel)
        .args(["-append", append]);
    if let Some(disk) = disk {
        cmd.arg("-drive")
            .arg(format!("if=none,id=disk0,format=raw,file={}", disk.display()))
            .args(["-device", "virtio-blk-pci,drive=disk0,disable-legacy=on"]);
    }
    cmd
}

/// `run [--release] [--gdb] [--no-disk] [kernel args...]`
fn run(args: &[String]) -> Result {
    let (mut release, mut gdb, mut with_disk, mut cmdline) = (false, false, true, Vec::new());
    for a in args {
        match a.as_str() {
            "--release" => release = true,
            "--gdb" => gdb = true,
            "--no-disk" => with_disk = false,
            flag if flag.starts_with("--") => return Err(format!("run: unknown option {flag}")),
            arg => cmdline.push(arg),
        }
    }
    let kernel = build_kernel(release)?;
    let disk = if with_disk { Some(test_disk()?) } else { None };
    if disk.is_some() {
        cmdline.insert(0, TEST_DISK_FLAG);
    }
    // `mon:stdio` shares the terminal between the serial port and QEMU's
    // monitor: Ctrl-A C switches between them, Ctrl-A X quits.
    let mut cmd = qemu(&kernel, DEFAULT_MEMORY, &cmdline.join(" "), disk.as_deref(), "mon:stdio");
    if gdb {
        // -s: gdb stub on tcp::1234, -S: do not start the CPU until told to.
        cmd.args(["-s", "-S"]);
        eprintln!("==> QEMU is waiting for a debugger on localhost:1234 (see docs/DEBUGGING.md):");
        eprintln!("    gdb {} -ex 'target remote :1234' -ex 'hbreak kernel_entry' -ex continue", kernel.display());
    }
    eprintln!("==> boot (Ctrl-A X quits, Ctrl-A C toggles the QEMU monitor; `cargo xtask symbolize < log` symbolizes)");
    let _ = cmd.status().map_err(|e| format!("qemu: {e}"))?;
    Ok(())
}

/// `symbolize [--release] [--elf <path>] [addr...]`
fn symbolize(args: &[String]) -> Result {
    let (mut elf, mut addrs) = (None, Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--release" => elf = elf.or(Some(kernel_elf(true))),
            "--elf" => elf = Some(PathBuf::from(it.next().ok_or("--elf needs a path")?)),
            addr => addrs.push(symbols::parse_hex(addr).ok_or_else(|| format!("not a hex address: {addr}"))?),
        }
    }
    let elf = elf.unwrap_or_else(|| kernel_elf(false));
    if !elf.exists() {
        return Err(format!("{} not found; run `cargo xtask build` first or pass --elf", elf.display()));
    }
    let syms = Symbols::from_file(&elf)?;
    let mut out = std::io::stdout().lock();
    let res = if addrs.is_empty() {
        // Serial logs can contain stray non-UTF-8 bytes; keep going.
        let mut lines = std::io::stdin().lock().split(b'\n');
        lines.try_for_each(|l| writeln!(out, "{}", syms.symbolize_line(&String::from_utf8_lossy(&l?))))
    } else {
        addrs.iter().try_for_each(|&a| writeln!(out, "{a:#018x} {}", syms.describe(a)))
    };
    res.map_err(|e| e.to_string())
}

struct Boot {
    status: Option<i32>,
    output: String,
}

/// Boot headless with a timeout, echoing serial output as it arrives, with
/// symbols added to backtrace lines.
fn boot(kernel: &Path, syms: &Arc<Symbols>, memory: &str, append: &str, disk: Option<&Path>) -> Result<Boot> {
    let mut child = qemu(kernel, memory, append, disk, "stdio")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let syms = Arc::clone(syms);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            let line = syms.symbolize_line(&line);
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
    let syms = Arc::new(Symbols::from_file(&kernel)?);

    // 2 GiB puts RAM, and QEMU's ACPI tables, above the first GiB that the
    // boot stub maps: the direct map must reach them. Both boots have the
    // test disk, so the virtio-blk tests read and write it.
    for memory in [DEFAULT_MEMORY, "2G"] {
        eprintln!("==> boot test: self-test passes with {memory} of RAM and the test disk");
        let disk = test_disk()?;
        let b = boot(&kernel, &syms, memory, &format!("poly.qemu_exit {TEST_DISK_FLAG}"), Some(&disk))?;
        let report = ktest::parse(&b.output);
        if let Some(s) = report.summary {
            eprintln!("==> kernel tests: {} passed, {} failed, {} filtered out", s.passed, s.failed, s.filtered_out);
        }
        report.check()?;
        match b.status {
            Some(QEMU_SUCCESS) if b.output.contains(PASS_MARKER) => {}
            Some(s) => return Err(format!("self-test boot ({memory}) exited with status {s}")),
            None => return Err(format!("self-test boot ({memory}) timed out after {BOOT_TIMEOUT:?}")),
        }
        let cpus = format!("acpi: CPUs: {QEMU_CPUS} enabled");
        if !b.output.contains(&cpus) {
            return Err(format!(
                "ACPI did not report the {QEMU_CPUS} CPUs QEMU emulates with {memory} (expected \"{cpus}\")"
            ));
        }
        // The first sector of the test disk starts with "POLY", sector 0.
        let first_read = "virtio-blk: sector 0: 50 4f 4c 59 00 00 00 00";
        if !b.output.lines().any(|l| l.contains(" 1af4:1042 ")) || !b.output.contains(first_read) {
            return Err(format!("the virtio-blk test disk was not enumerated and read with {memory}"));
        }
    }

    // Expecting the test disk without attaching it must fail the disk
    // tests, so a boot that silently lost its disk cannot pass.
    eprintln!("==> boot test: the disk tests fail when the test disk is missing");
    let only = format!("poly.qemu_exit poly.test=virtio_blk::device_found {TEST_DISK_FLAG}");
    let b = boot(&kernel, &syms, DEFAULT_MEMORY, &only, None)?;
    let report = ktest::parse(&b.output);
    let failed = matches!(report.failed.as_slice(), [(name, _)] if name == "virtio_blk::device_found_with_sane_bars");
    if b.status != Some(QEMU_FAILURE) || !failed {
        return Err(format!("a missing test disk was not reported as a failure (status {:?})", b.status));
    }

    eprintln!("==> boot test: page fault is caught and reported with a backtrace");
    let b = expect_fault(&kernel, &syms, "poly.selftest.page_fault", "CPU exception 14 (page fault)")?;
    // Frame #0 is the faulting instruction, so it must name the function.
    match b.output.lines().find_map(symbols::parse_frame) {
        Some((0, _, sym)) if sym.starts_with("poly_kernel::selftest::trigger_page_fault+0x") => {}
        Some((_, addr, sym)) => return Err(format!("backtrace does not start in trigger_page_fault: {addr:#x} {sym}")),
        None => return Err("page fault panic printed no backtrace".into()),
    }

    // W^X. Page-fault error codes (SDM Vol. 3A §4.7): bit 0 = protection
    // violation (the page is present), bit 1 = write, bit 4 = instruction
    // fetch. Each trigger prints the address it touches first.
    eprintln!("==> boot test: writing to kernel text is a page fault");
    let b = expect_fault(&kernel, &syms, "poly.selftest.write_text", "CPU exception 14 (page fault), error 0x3,")?;
    let addr = announced(&b.output, "selftest: writing to kernel text at ")?;
    if !b.output.contains(&format!("cr2 {addr}")) {
        return Err(format!("write to kernel text faulted, but not at {addr}"));
    }
    for (flag, what) in [("poly.selftest.exec_stack", "the stack"), ("poly.selftest.exec_data", "kernel data")] {
        eprintln!("==> boot test: executing {what} is a page fault (no-execute)");
        let b = expect_fault(&kernel, &syms, flag, "CPU exception 14 (page fault), error 0x11,")?;
        let addr = announced(&b.output, &format!("selftest: executing {what} at "))?;
        if !b.output.contains(&format!("rip {addr},")) || !b.output.contains(&format!("cr2 {addr}")) {
            return Err(format!("executing {what} faulted, but not on the fetch at {addr}"));
        }
    }

    eprintln!("==> boot test: a kernel stack overflow hits the guard page and is reported");
    let b = expect_fault(&kernel, &syms, "poly.selftest.stack_overflow", "CPU exception 8 (double fault)")?;
    if !b.output.contains("kernel stack overflow: hit the guard page below the boot stack") {
        return Err("stack overflow was not attributed to the boot stack's guard page".into());
    }

    // `selftest::fails_on_request` fails or panics when asked to.
    let only = "poly.qemu_exit poly.test=selftest::fails_on_request";
    eprintln!("==> boot test: a failing kernel test is reported by name");
    let b = boot(&kernel, &syms, DEFAULT_MEMORY, &format!("{only} poly.selftest.fail"), None)?;
    let report = ktest::parse(&b.output);
    let failed_alone = matches!(report.failed.as_slice(), [(name, _)] if name == "selftest::fails_on_request");
    let filtered = report.summary.is_some_and(|s| s.passed == 0 && s.filtered_out > 0);
    if b.status != Some(QEMU_FAILURE) || !failed_alone || !filtered || report.check().is_ok() {
        return Err(format!("failing kernel test was not reported as expected (status {:?})", b.status));
    }

    eprintln!("==> boot test: a panic inside a kernel test is reported with a backtrace");
    let b = boot(&kernel, &syms, DEFAULT_MEMORY, &format!("{only} poly.selftest.panic"), None)?;
    let no_summary = ktest::parse(&b.output).summary.is_none();
    let frames: Vec<_> = b.output.lines().filter_map(symbols::parse_frame).map(|(_, _, sym)| sym).collect();
    let in_backtrace = |f: &str| frames.iter().any(|sym| sym.starts_with(f));
    let named = in_backtrace("poly_kernel::selftest::panic_on_purpose+")
        && in_backtrace("poly_kernel::selftest::fails_on_request+");
    if b.status != Some(QEMU_FAILURE) || !no_summary || !named {
        return Err(format!("panic in a kernel test was not reported as expected (status {:?})", b.status));
    }

    // `heap::out_of_memory_on_request` asks for more than the heap can give.
    eprintln!("==> boot test: running out of heap memory panics with the size and a backtrace");
    let only = "poly.qemu_exit poly.test=heap::out_of_memory_on_request poly.selftest.oom";
    let b = boot(&kernel, &syms, DEFAULT_MEMORY, only, None)?;
    let reported = b
        .output
        .lines()
        .map(str::trim_end)
        .any(|l| l.starts_with("memory allocation of ") && l.ends_with(" bytes failed"));
    let named = b
        .output
        .lines()
        .filter_map(symbols::parse_frame)
        .any(|(_, _, sym)| sym.starts_with("poly_kernel::heap::allocate_too_much+"));
    if b.status != Some(QEMU_FAILURE) || !b.output.contains("KERNEL PANIC") || !reported || !named {
        return Err(format!("out-of-memory was not reported as expected (status {:?})", b.status));
    }

    // Panics on kernel threads: the backtrace must follow the thread's own
    // stack down to the thread trampoline, where every thread starts.
    let thread_panics = [
        (
            "a panic inside a kernel thread",
            "thread::panics_in_thread_on_request poly.selftest.thread_panic",
            "poly.selftest.thread_panic",
            "poly_kernel::thread::panic_in_thread+",
        ),
        (
            "a deadlock (every thread blocked)",
            "thread::deadlock_on_request poly.selftest.deadlock",
            "deadlock: every thread is blocked",
            "poly_kernel::thread::idle_main+",
        ),
    ];
    for (what, args, message, function) in thread_panics {
        eprintln!("==> boot test: {what} is reported with a backtrace of the thread's stack");
        let b = boot(&kernel, &syms, DEFAULT_MEMORY, &format!("poly.qemu_exit poly.test={args}"), None)?;
        let frames: Vec<_> = b.output.lines().filter_map(symbols::parse_frame).map(|(_, _, sym)| sym).collect();
        let in_backtrace = |f: &str| frames.iter().any(|sym| sym.starts_with(f));
        let named = in_backtrace(function) && in_backtrace("poly_kernel::thread::thread_start+");
        let to_bottom = frames.last().is_some_and(|sym| sym.starts_with("thread_trampoline+"));
        if b.status != Some(QEMU_FAILURE) || !b.output.contains(message) || !named || !to_bottom {
            return Err(format!("{what} was not reported as expected (status {:?})", b.status));
        }
    }
    eprintln!("==> boot tests passed");
    Ok(())
}

/// Boot with a `poly.selftest.*` flag that makes the kernel fault on purpose
/// and check that the fault was reported: a panic whose message contains
/// `report`, and QEMU's failure status.
fn expect_fault(kernel: &Path, syms: &Arc<Symbols>, flag: &str, report: &str) -> Result<Boot> {
    let b = boot(kernel, syms, DEFAULT_MEMORY, &format!("poly.qemu_exit {flag}"), None)?;
    let reported = b.output.contains("KERNEL PANIC") && b.output.contains(report);
    if b.status != Some(QEMU_FAILURE) || !reported {
        return Err(format!("{flag}: expected a panic reporting \"{report}\" (status {:?})", b.status));
    }
    Ok(b)
}

/// The rest of the first output line that starts with `prefix`: the address
/// a negative test announced before touching it.
fn announced<'a>(output: &'a str, prefix: &str) -> Result<&'a str> {
    output
        .lines()
        .find_map(|l| l.trim().strip_prefix(prefix))
        .ok_or_else(|| format!("the kernel did not print \"{prefix}<address>\""))
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
