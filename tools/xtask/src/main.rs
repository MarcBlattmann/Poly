//! Developer and CI entry point: `cargo xtask <command>`.
//!
//! Commands:
//!   build [--release]   build the kernel
//!   run   [--release] [--gdb] [--no-disk] [--headless] [--busybox | --disk-root [--fresh-disk] | --initrd=<file> | --no-initrd] [kernel args...]
//!                       boot the kernel into its shell in QEMU (serial on stdio, Ctrl-A X quits)
//!                       with the test devices and a virtio-net NIC on QEMU's user
//!                       networking attached (--no-disk: without them),
//!                       the screen in a window when there is a desktop (--headless:
//!                       never); --gdb waits for a debugger on localhost:1234
//!                       (docs/DEBUGGING.md); the root file system is a small
//!                       initramfs (--initrd=<file>: that cpio archive instead,
//!                       --no-initrd: none; --busybox: BusyBox's image, booting into
//!                       BusyBox's init and a shell on the console, see busybox.rs;
//!                       add console=tty1 for the screen; --disk-root [--fresh-disk]: BusyBox's
//!                       init mounts the root disk (made if missing) and runs its bash, see
//!                       rootdisk.rs)
//!   screenshot [--release] [--out <file>] [--wait <text>] [kernel args...]
//!                       boot headless, wait for a serial line containing <text>
//!                       (default: the self-test's verdict), save the screen to <file>
//!                       (default target/poly-screen.png; .ppm for PPM) and quit
//!   test                host unit tests + kernel boot tests in QEMU
//!   boot-tests [--list] [--shard <name>]... [group...]
//!                       the boot tests alone: all, the groups named (`selftest` is
//!                       every `selftest-*`), or a CI shard's; --list lists them
//!   ci [part...]        everything CI checks: fmt, clippy, tests, coverage; or the
//!                       parts named (lint, host-tests, boot-tests, coverage,
//!                       pipeline), as CI's parallel jobs run them
//!                       QEMU runs on KVM when /dev/kvm is usable, else on TCG;
//!                       POLY_QEMU_ACCEL=tcg|kvm|auto chooses (accel.rs)
//!   symbolize [--release] [--elf <path>] [addr...]
//!                       print the kernel function at each address; with no
//!                       addresses, add symbols to the backtrace in a log on stdin
//!   coverage <args>     run the coverage tool (see tools/coverage)
//!   pipeline <args>     run the agent pipeline tool (see tools/pipeline, docs/AGENT_PIPELINE.md);
//!                       exits with the tool's own status
//!   dynamic-initramfs   build the dynamically linked test programs and an initramfs
//!                       with them and the host's ld.so and libraries (docs/DEBUGGING.md)
//!   busybox-initramfs   write the BusyBox image (BusyBox from the host or built; busybox.rs)
//!   busybox-test        only the BusyBox boot test of `test` (busybox.rs)
//!   root-disk           write the persistent root disk image target/poly-root-disk.img
//!                       (the host's bash, coreutils, Python, gcc; rootdisk.rs)
//!   root-disk-test      only the root disk boot test of `test` (rootdisk.rs)
//!   ntdll [--out <file>] build our ntdll.dll (or take it from the content-hash
//!                       cache every kernel build uses) and verify its exports (ntdll.rs);
//!                       --cache-key: print the name it is cached under
//!   win32               the same for kernelbase.dll, kernel32.dll, ucrtbase.dll, msvcrt.dll and
//!                       the test programs (win32.rs)
//!   win fetch-7zip      download the pinned 7-Zip 7za.exe (x64) into the cache, checked (sevenzip.rs)
//!   win imports <exe | 7zip> [--markdown <file>] [--check]
//!                       what a Windows program imports and what of it our DLLs lack
//!                       (winimports.rs, ADR 0087); --check fails unless every import binds
//!   win 7zip-diff [--kernel <elf>] [--build-kernel] [--target <poly.json>] [--markdown <file>]
//!                       run the 7-Zip scenario driver on Poly and compare it step by step
//!                       with its Windows recording (sevenzip.rs, ADR 0088)
//!   font [--check]      regenerate poly-gfx's glyph tables from the BDF fonts
//!                       in third_party/ (--check: fail if they are out of date)

mod accel;
mod asmlabels;
mod busybox;
mod conflicts;
mod console;
mod dynamic;
mod ext2disk;
mod ext4disk;
mod font;
mod initramfs;
mod ktest;
mod libsearch;
mod net;
mod nls;
mod ntdll;
mod programs;
mod rootdisk;
mod screen;
mod sevenzip;
mod symbols;
mod terminal;
mod win32;
mod winimports;
mod wintrace;

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use symbols::Symbols;

type Result<T = ()> = std::result::Result<T, String>;

const BOOT_TIMEOUT: Duration = Duration::from_secs(60);

thread_local! {
    /// This thread's boot timeout ([`with_boot_timeout`]; [`BOOT_TIMEOUT`]
    /// unless a test asks for more).
    static THREAD_BOOT_TIMEOUT: std::cell::Cell<Duration> = const { std::cell::Cell::new(BOOT_TIMEOUT) };
}

/// How long a boot of this thread may run before it is killed.
fn boot_timeout() -> Duration {
    THREAD_BOOT_TIMEOUT.with(std::cell::Cell::get)
}

/// Run `f` with boots of this thread allowed `timeout` instead of
/// [`BOOT_TIMEOUT`] (a test whose program runs for longer, such as a
/// benchmark under TCG).
fn with_boot_timeout<T>(timeout: Duration, f: impl FnOnce() -> T) -> T {
    let old = THREAD_BOOT_TIMEOUT.with(|t| t.replace(timeout));
    let r = f();
    THREAD_BOOT_TIMEOUT.with(|t| t.set(old));
    r
}
/// Must match `selftest::PASS_MARKER` in the kernel.
const PASS_MARKER: &str = "POLY-SELFTEST: PASS";
/// QEMU exit statuses produced by the kernel's `qemu::ExitCode`.
const QEMU_SUCCESS: i32 = 33;
const QEMU_FAILURE: i32 = 35;
/// CPUs QEMU emulates unless a boot asks for another number (`-smp`); the
/// self-test boots check that ACPI reports exactly as many and that the
/// kernel brings them all up.
const QEMU_CPUS: u32 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let release = args.iter().any(|a| a == "--release");
    if let Err(e) = accel::init() {
        eprintln!("xtask: error: {e}");
        return ExitCode::FAILURE;
    }
    let result = match args.first().map(String::as_str) {
        Some("build") => build_kernel(release).map(|_| ()),
        Some("run") => run(&args[1..]),
        Some("screenshot") => screenshot(&args[1..]),
        Some("test") => test(),
        Some("ci") => ci(&args[1..]),
        Some("boot-tests") => boot_tests_command(&args[1..]),
        Some("symbolize") => symbolize(&args[1..]),
        Some("coverage") => coverage(&args[1..]),
        Some("pipeline") => return pipeline_command(&args[1..]),
        Some("font") => font::run(&root(), &args[1..]),
        Some("nls-tables") => nls::run(&root(), &args[1..]),
        Some("ntdll") => ntdll::command(&args[1..]),
        Some("win32") => win32::command(&args[1..]),
        Some("win") => sevenzip::command(&args[1..]),
        Some("dynamic-initramfs") => dynamic::command(),
        Some("busybox-initramfs") => busybox::command(),
        Some("busybox-test") => busybox_test(),
        Some("root-disk") => rootdisk::command(),
        Some("root-disk-test") => root_disk_test(),
        _ => {
            eprintln!(
                "usage: cargo xtask <build|run|screenshot|test|boot-tests|ci|symbolize|coverage|pipeline|font|nls-tables|ntdll|win32|win|dynamic-initramfs|busybox-initramfs|busybox-test|root-disk|root-disk-test> [--release]"
            );
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

/// `cargo` run inside kernel/ (so kernel/.cargo/config.toml, target and
/// flags, applies), with our `ntdll.dll` up to date and named in the
/// environment for the kernel's `build.rs` to embed (ADR 0071).
fn kernel_cargo() -> Result<Command> {
    let dll = ntdll::ensure()?;
    let win32 = win32::ensure()?;
    let mut cmd = cargo();
    cmd.current_dir(kernel_dir()).env(ntdll::KERNEL_ENV, dll).env(win32::KERNEL_ENV, win32);
    Ok(cmd)
}

/// Builds from inside kernel/ so kernel/.cargo/config.toml (target, flags) applies.
fn build_kernel(release: bool) -> Result<PathBuf> {
    let mut cmd = kernel_cargo()?;
    cmd.arg("build");
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

/// Kernel flag saying the test disk is attached; the kernel's block and
/// virtio-blk tests then require it (`block::TEST_DISK_FLAG`).
const TEST_DISK_FLAG: &str = "poly.test.disk";
/// Size of the test disk in 512-byte sectors (`block::TEST_DISK_SECTORS`).
const TEST_DISK_SECTORS: u32 = 2048;

/// Sector `sector` of the test disk: "POLY", the sector number (le32), then
/// bytes that depend on the sector and the position. Must match
/// `block::test_pattern` in the kernel.
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
    // The ext2 and ext4 test disks go next to it (`qemu` attaches them
    // from there).
    ext2disk::write_next_to(&path)?;
    ext4disk::write_next_to(&path)?;
    Ok(path)
}

/// Kernel flag that starts the shell after the tests even with
/// `poly.qemu_exit` (`shell::SHELL_FLAG`).
const SHELL_FLAG: &str = "poly.shell";

/// Kernel flag saying QEMU's `edu` device is attached, a function with MSI
/// and no MSI-X; the kernel's MSI test then requires it
/// (`pci::TEST_MSI_FLAG`).
const TEST_MSI_FLAG: &str = "poly.test.msi";

/// Kernel flag saying the boot has a display, which QEMU's `q35` machine
/// always has unless told otherwise (`-vga none`); the kernel's display
/// tests then require it (`display::TEST_DISPLAY_FLAG`).
const TEST_DISPLAY_FLAG: &str = "poly.test.display";
/// The mode the kernel sets without `poly.display=` (`display::DEFAULT_MODE`).
const DEFAULT_DISPLAY: (u32, u32) = (1280, 800);
/// The mode the screenshot boot test asks for, so the command line option
/// is tested too.
const SCREENSHOT_DISPLAY: (u32, u32) = (1024, 768);

/// The test devices a boot gets.
#[derive(Clone, Copy, Debug)]
enum Devices<'a> {
    None,
    /// The test disk image as a modern-only virtio-blk device with MSI-X;
    /// a spare virtio-blk device on QEMU's null block driver (1 MiB of
    /// zeros), which the kernel's watchdog test breaks on purpose; and
    /// QEMU's `edu` device, for MSI.
    Test(&'a Path),
    /// The test disk image and the spare device, both without MSI-X
    /// (`vectors=0`), so they are polled.
    PolledDisk(&'a Path),
}

impl Devices<'_> {
    /// The kernel flags that say what is attached.
    fn flags(self) -> String {
        match self {
            Devices::None => String::new(),
            Devices::Test(_) => {
                format!("{TEST_DISK_FLAG} {TEST_MSI_FLAG} {TEST_DISPLAY_FLAG} {}", ext2disk::KERNEL_FLAGS)
            }
            Devices::PolledDisk(_) => format!("{TEST_DISK_FLAG} {TEST_DISPLAY_FLAG} {}", ext2disk::KERNEL_FLAGS),
        }
    }
}

/// QEMU for `kernel`, on KVM when it is usable and TCG otherwise
/// ([`accel`]: `POLY_QEMU_ACCEL`), with the serial port on `serial` (a QEMU
/// character device) and `devices` attached. The machine has QEMU's standard VGA; add
/// `-display none` for no window.
fn qemu(kernel: &Path, memory: &str, append: &str, devices: Devices<'_>, serial: &str) -> Command {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(accel::get().qemu_args())
        .args(["-machine", "q35", "-m", memory, "-serial", serial, "-no-reboot"])
        .args(["-smp", &QEMU_CPUS.to_string()])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-kernel")
        .arg(kernel)
        .args(["-append", append]);
    let (Devices::Test(disk) | Devices::PolledDisk(disk)) = devices else { return cmd };
    let vectors = if let Devices::PolledDisk(_) = devices { ",vectors=0" } else { "" };
    // The test disk comes first, so it is the first virtio-blk function the
    // kernel finds (`virtio0`).
    cmd.arg("-drive")
        .arg(format!("if=none,id=disk0,format=raw,file={}", disk.display()))
        .args(["-device", &format!("virtio-blk-pci,drive=disk0,disable-legacy=on{vectors}")]);
    cmd.args(["-blockdev", "driver=null-co,node-name=spare,size=1048576,read-zeroes=on"])
        .args(["-device", &format!("virtio-blk-pci,drive=spare,disable-legacy=on{vectors}")]);
    if let Devices::Test(_) = devices {
        cmd.args(["-device", "edu"]);
    }
    // The ext2 test disk (`test_disk` writes it): the third virtio-blk
    // function, `virtio2`, which the kernel mounts on /mnt.
    let ext2 = disk.with_file_name(ext2disk::FILE_NAME);
    cmd.arg("-drive")
        .arg(format!("if=none,id=ext2,format=raw,file={}", ext2.display()))
        .args(["-device", &format!("virtio-blk-pci,drive=ext2,disable-legacy=on{vectors}")]);
    // The ext4 test disk: the fourth, `virtio3`, mounted on /ext4.
    let ext4 = disk.with_file_name(ext4disk::FILE_NAME);
    cmd.arg("-drive")
        .arg(format!("if=none,id=ext4,format=raw,file={}", ext4.display()))
        .args(["-device", &format!("virtio-blk-pci,drive=ext4,disable-legacy=on{vectors}")]);
    cmd
}

/// Whether QEMU can open a window: on macOS (Cocoa) and Windows always,
/// elsewhere when an X11 or Wayland display is set. Without one, QEMU's
/// default display (GTK or SDL) would fail to start.
fn has_desktop() -> bool {
    let set = |var: &str| std::env::var_os(var).is_some_and(|v| !v.is_empty());
    cfg!(any(target_os = "macos", windows)) || set("DISPLAY") || set("WAYLAND_DISPLAY")
}

/// `run [--release] [--gdb] [--no-disk] [--headless] [--busybox | --initrd=<file> | --no-initrd] [kernel args...]`
fn run(args: &[String]) -> Result {
    let (mut release, mut gdb, mut with_disk, mut window, mut cmdline) = (false, false, true, true, Vec::new());
    let (mut with_busybox, mut disk_root, mut fresh_disk) = (false, false, false);
    // The default initramfs: an empty path until it is written below.
    let mut initrd = Some(PathBuf::new());
    for a in args {
        match a.as_str() {
            "--release" => release = true,
            "--gdb" => gdb = true,
            "--no-disk" => with_disk = false,
            "--headless" => window = false,
            "--no-initrd" => initrd = None,
            "--busybox" => with_busybox = true,
            "--disk-root" => disk_root = true,
            "--fresh-disk" => fresh_disk = true,
            flag if flag.starts_with("--initrd=") => initrd = Some(PathBuf::from(&flag["--initrd=".len()..])),
            flag if flag.starts_with("--") => return Err(format!("run: unknown option {flag}")),
            arg => cmdline.push(arg),
        }
    }
    let kernel = build_kernel(release)?;
    if disk_root {
        return run_disk_root(&kernel, fresh_disk, gdb, &cmdline);
    }
    let disk = if with_disk { Some(test_disk()?) } else { None };
    let devices = disk.as_deref().map_or(Devices::None, Devices::Test);
    let flags = devices.flags();
    if !flags.is_empty() {
        cmdline.insert(0, &flags);
    }
    if with_busybox {
        let (image, binary) = busybox::write_image()?.map_err(|why| format!("--busybox: {why}"))?;
        eprintln!("==> BusyBox from {}; init is /sbin/init (exit QEMU with `poweroff`)", binary.origin);
        initrd = Some(image);
        if !cmdline.iter().any(|a| a.starts_with("poly.init=")) {
            cmdline.push(busybox::INIT_ARGS);
        }
    } else if initrd.as_deref() == Some(Path::new("")) {
        initrd = Some(initramfs::write(&root(), "poly-initramfs.cpio", &initramfs::default_image())?);
    }
    // A virtio-net device on QEMU's user networking with its addresses
    // (an earlier poly.net= on the command line wins).
    if with_disk {
        cmdline.push("poly.net=qemu");
    }
    // `mon:stdio` shares the terminal between the serial port and QEMU's
    // monitor: Ctrl-A C switches between them, Ctrl-A X quits.
    let mut cmd = qemu(&kernel, DEFAULT_MEMORY, &cmdline.join(" "), devices, "mon:stdio");
    if with_disk {
        cmd.args(net::nic_args(None, false));
    }
    if let Some(file) = &initrd {
        cmd.arg("-initrd").arg(file);
    }
    if window && !has_desktop() {
        eprintln!(
            "==> no desktop (DISPLAY / WAYLAND_DISPLAY unset): no window; `cargo xtask screenshot` shows the screen"
        );
        window = false;
    }
    if window {
        eprintln!(
            "==> the screen terminal (a shell on the keyboard) is in QEMU's window; serial output, the serial shell and the monitor stay in this terminal"
        );
    } else {
        cmd.args(["-display", "none"]);
    }
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

/// `run --disk-root [--fresh-disk]`: BusyBox's init mounts the root disk
/// (`target/poly-root-disk.img`, made if missing or with `--fresh-disk`)
/// and runs its bash on the serial console (rootdisk.rs). No window, no
/// test devices; the network on QEMU's user networking.
fn run_disk_root(kernel: &Path, fresh: bool, gdb: bool, cmdline: &[&str]) -> Result {
    let image = root().join("target").join(rootdisk::FILE_NAME);
    if fresh || !image.exists() {
        rootdisk::make(rootdisk::FILE_NAME)?;
    } else {
        eprintln!("==> the root disk {} is kept from before (--fresh-disk makes it again)", image.display());
    }
    let (initrd, origin) = rootdisk::write_initramfs()?.map_err(|why| format!("--disk-root: {why}"))?;
    eprintln!(
        "==> BusyBox from {origin}; its init mounts /dev/vda on /mnt/root and runs bash there (`poweroff` quits)"
    );
    let mut append = format!("{} poly.net=qemu", rootdisk::boot_args());
    for a in cmdline {
        append.push(' ');
        append.push_str(a);
    }
    let mut cmd = qemu(kernel, rootdisk::MEMORY, &append, Devices::None, "mon:stdio");
    cmd.args(rootdisk::drive_args(&image)).args(net::nic_args(None, false));
    cmd.arg("-initrd").arg(&initrd).args(["-display", "none"]);
    if gdb {
        cmd.args(["-s", "-S"]);
        eprintln!("==> QEMU is waiting for a debugger on localhost:1234 (see docs/DEBUGGING.md)");
    }
    let _ = cmd.status().map_err(|e| format!("qemu: {e}"))?;
    Ok(())
}

/// `cargo xtask root-disk-test`: the root disk boot test alone.
fn root_disk_test() -> Result {
    run_boot_groups(&["rootdisk".into()])
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
    /// QEMU's exit status; `None` if it was killed (at [`BOOT_TIMEOUT`], or
    /// on purpose, e.g. once a screenshot was taken).
    status: Option<i32>,
    output: String,
    /// Whether QEMU was still running at [`BOOT_TIMEOUT`] and was killed.
    timed_out: bool,
}

impl Boot {
    /// The kernel tests' report in `log` (the boot's output, or the part
    /// of it before the shell), checked. A boot that ran into
    /// [`BOOT_TIMEOUT`] before the summary is reported as a timeout, with
    /// the last line the kernel printed; one whose QEMU exited early, as a
    /// kernel that stopped during the tests.
    fn tests(&self, log: &str) -> Result<ktest::Report> {
        let report = ktest::parse(log);
        let end = if self.timed_out {
            ktest::End::TimedOut { secs: boot_timeout().as_secs(), last_line: ktest::last_line(&self.output) }
        } else {
            ktest::End::Exited
        };
        report.check_end(end)?;
        Ok(report)
    }
}

/// Say that a boot ran into [`BOOT_TIMEOUT`] (and is being killed), with the
/// last line it printed, before whatever check then fails on it.
fn report_timeout(last_line: &str) {
    eprintln!("==> QEMU timed out after {} s (last line: {last_line:?}); killing it", boot_timeout().as_secs());
}

/// Boot headless with a timeout, echoing serial output as it arrives, with
/// symbols added to backtrace lines.
fn boot(kernel: &Path, syms: &Arc<Symbols>, memory: &str, append: &str, devices: Devices<'_>) -> Result<Boot> {
    boot_with(kernel, syms, memory, append, devices, &[], None, |_| {})
}

/// [`boot`] on a given QEMU CPU model (`-cpu`), or QEMU's default one, with
/// an initramfs (`-initrd`) or without.
#[allow(clippy::too_many_arguments)]
fn boot_on(
    cpu: Option<&str>,
    initrd: Option<&Path>,
    kernel: &Path,
    syms: &Arc<Symbols>,
    memory: &str,
    append: &str,
    devices: Devices<'_>,
) -> Result<Boot> {
    let mut args: Vec<String> = cpu.map(|c| vec!["-cpu".into(), c.into()]).unwrap_or_default();
    if let Some(file) = initrd {
        args.extend(["-initrd".into(), file.display().to_string()]);
    }
    boot_with(kernel, syms, memory, append, devices, &args, None, |_| {})
}

/// A screenshot to take during a boot: once the kernel prints a line
/// containing `marker`, the screen goes to `file` and QEMU quits. A failed
/// self-test or a panic ends the boot without one.
#[derive(Clone, Copy, Debug)]
struct Screenshot<'a> {
    marker: &'a str,
    file: &'a Path,
}

/// The kernel's verdicts that end a boot taken for a screenshot when the
/// marker has not come (the kernel does not exit QEMU in those boots).
const STOP_MARKERS: [&str; 2] = ["POLY-SELFTEST: FAIL", "KERNEL PANIC"];

/// How often a boot is retried when QEMU itself crashed before the kernel
/// touched any device ([`crashed_before_devices`]).
const QEMU_CRASH_RETRIES: usize = 2;

/// QEMU 8.2 under TCG on a loaded host sometimes dies with SIGSEGV while
/// the kernel scans PCI configuration space, before the first device line.
/// A guest cannot make the emulator take a signal (a panic or triple fault
/// ends QEMU with a status), and nothing on a disk image or the network
/// has been touched yet, so booting again is safe. QEMU killed by a
/// signal shows as no status without a timeout.
fn crashed_before_devices(b: &Boot) -> bool {
    b.status.is_none() && !b.timed_out && !b.output.lines().any(|l| l.contains("pci: 00:"))
}

/// Boot again (up to [`QEMU_CRASH_RETRIES`] times) while
/// [`crashed_before_devices`] says QEMU died early.
fn retry_qemu_crash(mut boot: impl FnMut() -> Result<Boot>) -> Result<Boot> {
    let mut attempt = 0;
    loop {
        let b = boot()?;
        if attempt == QEMU_CRASH_RETRIES || !crashed_before_devices(&b) {
            return Ok(b);
        }
        attempt += 1;
        eprintln!(
            "==> QEMU died of a signal before the kernel reached any device (an emulator crash); booting again \
             ({attempt} of {QEMU_CRASH_RETRIES})"
        );
    }
}

/// [`boot`] with more QEMU arguments and, optionally, a screenshot, calling
/// `on_line` with every line of serial output (from the thread that reads it)
/// as it arrives. A boot QEMU crashed early is retried ([`retry_qemu_crash`]).
#[allow(clippy::too_many_arguments)]
fn boot_with(
    kernel: &Path,
    syms: &Arc<Symbols>,
    memory: &str,
    append: &str,
    devices: Devices<'_>,
    qemu_args: &[String],
    shot: Option<Screenshot<'_>>,
    on_line: impl FnMut(&str) + Send + 'static,
) -> Result<Boot> {
    let on_line = Arc::new(std::sync::Mutex::new(on_line));
    retry_qemu_crash(|| {
        let f = Arc::clone(&on_line);
        let call = move |line: &str| (f.lock().unwrap_or_else(|e| e.into_inner()))(line);
        boot_with_once(kernel, syms, memory, append, devices, qemu_args, shot, call)
    })
}

/// `POLY-OUT` lines a boot echoes before it stops showing them.
const ECHOED_OUT_LINES: usize = 64;

#[allow(clippy::too_many_arguments)]
fn boot_with_once(
    kernel: &Path,
    syms: &Arc<Symbols>,
    memory: &str,
    append: &str,
    devices: Devices<'_>,
    qemu_args: &[String],
    shot: Option<Screenshot<'_>>,
    mut on_line: impl FnMut(&str) + Send + 'static,
) -> Result<Boot> {
    let socket = std::env::temp_dir().join(format!("poly-qmp-{}.sock", std::process::id()));
    let mut cmd = qemu(kernel, memory, append, devices, "stdio");
    cmd.args(["-display", "none"]).args(qemu_args);
    if shot.is_some() {
        let _ = std::fs::remove_file(&socket);
        cmd.arg("-qmp").arg(format!("unix:{},server=on,wait=off", socket.display()));
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let syms = Arc::clone(syms);
    let reader = thread::spawn(move || {
        // Output files come back as many `POLY-OUT` lines (a trace of the
        // Windows API tracer is hundreds of KB): only the first are echoed.
        let mut out_lines = 0usize;
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            let line = syms.symbolize_line(&line);
            if line.contains("POLY-OUT:") {
                out_lines += 1;
            }
            if out_lines <= ECHOED_OUT_LINES || !line.contains("POLY-OUT:") {
                println!("    | {line}");
            }
            on_line(&line);
            let _ = tx.send(line);
        }
        if out_lines > ECHOED_OUT_LINES {
            println!("    | ({} more POLY-OUT lines not shown)", out_lines - ECHOED_OUT_LINES);
        }
    });

    let deadline = Instant::now() + boot_timeout();
    let (mut lines, mut taken, mut timed_out) = (Vec::new(), None, false);
    let status = loop {
        for line in rx.try_iter() {
            if let Some(s) = shot.filter(|_| taken.is_none()) {
                if line.contains(s.marker) {
                    taken = Some(screen::screenshot(&socket, s.file));
                } else if STOP_MARKERS.iter().any(|m| line.contains(m)) {
                    taken = Some(Err(format!("the kernel printed \"{}\" first", line.trim())));
                    let _ = child.kill();
                }
            }
            lines.push(line);
        }
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s.code();
        }
        if Instant::now() > deadline {
            timed_out = true;
            report_timeout(lines.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or_default());
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let _ = reader.join();
    lines.extend(rx.try_iter());
    if let Some(s) = shot {
        let _ = std::fs::remove_file(&socket);
        let what = format!("screenshot after \"{}\"", s.marker);
        taken.unwrap_or_else(|| Err("the kernel never printed it".into())).map_err(|e| format!("{what}: {e}"))?;
    }
    Ok(Boot { status, output: lines.join("\n"), timed_out })
}

/// The kernel's version, from `kernel/Cargo.toml`, as the boot screen shows it.
fn kernel_version() -> Result<String> {
    let path = kernel_dir().join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .find_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"'))
        .map(String::from)
        .ok_or_else(|| format!("{}: no version", path.display()))
}

/// What the self-test prints before PASS or FAIL.
const SELFTEST_VERDICT: &str = "POLY-SELFTEST: ";

/// `screenshot [--release] [--out <file>] [--wait <text>] [kernel args...]`
fn screenshot(args: &[String]) -> Result {
    let (mut release, mut out, mut wait, mut cmdline) = (false, None, SELFTEST_VERDICT.to_string(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--release" => release = true,
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a file")?)),
            "--wait" => wait = it.next().ok_or("--wait needs a text")?.clone(),
            flag if flag.starts_with("--") => return Err(format!("screenshot: unknown option {flag}")),
            arg => cmdline.push(arg),
        }
    }
    let kernel = build_kernel(release)?;
    let syms = Arc::new(Symbols::from_file(&kernel)?);
    let disk = test_disk()?;
    let devices = Devices::Test(&disk);
    let flags = devices.flags();
    cmdline.insert(0, &flags);
    let out = out.unwrap_or_else(|| root().join("target").join("poly-screen.png"));
    // QEMU writes the file itself, so give it a full path.
    let out = std::path::absolute(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let shot = Screenshot { marker: &wait, file: &out };
    boot_with(&kernel, &syms, DEFAULT_MEMORY, &cmdline.join(" "), devices, &[], Some(shot), |_| {})?;
    eprintln!("==> screenshot saved to {}", out.display());
    Ok(())
}

/// Must match `keyboard::TEST_READY` in the kernel.
const KEYBOARD_READY: &str = "keyboard: ready for test input";

/// Press `keys` (QEMU key names in `sendkey` syntax, e.g. `shift-p`,
/// `alt_r-2`) on the guest's PS/2 keyboard through the human monitor on
/// the Unix socket `monitor`, one at a time.
fn send_keys(monitor: &Path, keys: &[&str]) -> Result {
    let mut sock = UnixStream::connect(monitor).map_err(|e| format!("{}: {e}", monitor.display()))?;
    // Read (and drop) the monitor's banner and echo so it never blocks.
    let mut echo = sock.try_clone().map_err(|e| e.to_string())?;
    thread::spawn(move || std::io::copy(&mut echo, &mut std::io::sink()));
    for key in keys {
        writeln!(sock, "sendkey {key}").map_err(|e| e.to_string())?;
        // sendkey holds the keys for 100 ms; let them go before the next.
        thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// Boot with `poly.test.keyboard=<expected>` (and `poly.keymap=<keymap>`),
/// press `keys` once the kernel waits for them, and check the kernel typed
/// `expected` with that layout.
fn keyboard_boot_test(
    kernel: &Path,
    syms: &Arc<Symbols>,
    keymap: Option<&str>,
    keys: &'static [&'static str],
    expected: &str,
) -> Result {
    let monitor = std::env::temp_dir().join(format!("poly-monitor-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&monitor);
    let qemu_args = ["-monitor".to_string(), format!("unix:{},server=on,wait=off", monitor.display())];
    let mut append = format!("poly.qemu_exit poly.test=keyboard::typed poly.test.keyboard={expected}");
    if let Some(k) = keymap {
        append.push_str(&format!(" poly.keymap={k}"));
    }
    let socket = monitor.clone();
    let mut sent = false;
    let on_line = move |line: &str| {
        if !sent && line.contains(KEYBOARD_READY) {
            sent = true;
            let socket = socket.clone();
            thread::spawn(move || {
                if let Err(e) = send_keys(&socket, keys) {
                    eprintln!("xtask: sending keys through the QEMU monitor: {e}");
                }
            });
        }
    };
    let b = boot_with(kernel, syms, DEFAULT_MEMORY, &append, Devices::None, &qemu_args, None, on_line);
    let _ = std::fs::remove_file(&monitor);
    let b = b?;
    b.tests(&b.output)?;
    let layout = keymap.unwrap_or("us");
    if !b.output.contains("keyboard: ISA IRQ 1 -> GSI ") || !b.output.contains(&format!("; layout {layout} (")) {
        return Err(format!("the keyboard was not brought up on IRQ 1 with layout {layout}"));
    }
    let typed = format!("keyboard: typed {expected:?}");
    if b.status != Some(QEMU_SUCCESS) || !b.output.lines().any(|l| l.trim_end() == typed) {
        return Err(format!(
            "keys sent through the QEMU monitor did not type {expected:?} with layout {layout} (status {:?})",
            b.status
        ));
    }
    Ok(())
}

/// Boot headless with the serial port on QEMU's stdin and stdout, and type
/// into it: for each `(expect, input)` step, wait until the output (escape
/// sequences removed) shows `expect` after what the previous step matched,
/// then write `input`. Returns when QEMU exits, with the output stripped of
/// escape sequences (or after [`BOOT_TIMEOUT`], with no status). A boot QEMU
/// crashed early is retried ([`retry_qemu_crash`]).
fn boot_typing(
    kernel: &Path,
    syms: &Arc<Symbols>,
    append: &str,
    devices: Devices<'_>,
    qemu_args: &[String],
    steps: &[(&str, &str)],
) -> Result<Boot> {
    retry_qemu_crash(|| boot_typing_once(kernel, syms, append, devices, qemu_args, steps))
}

fn boot_typing_once(
    kernel: &Path,
    syms: &Arc<Symbols>,
    append: &str,
    devices: Devices<'_>,
    qemu_args: &[String],
    steps: &[(&str, &str)],
) -> Result<Boot> {
    let mut child = qemu(kernel, DEFAULT_MEMORY, append, devices, "stdio")
        .args(["-display", "none"])
        .args(qemu_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    // Raw bytes, not lines: the prompt has no newline after it.
    let reader = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n @ 1..) = stdout.read(&mut buf) {
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + BOOT_TIMEOUT;
    let (mut raw, mut printed, mut matched, mut timed_out) = (Vec::new(), 0, 0, false);
    let mut steps = steps.iter();
    let mut step = steps.next();
    let status = loop {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(20)) {
            raw.extend(chunk);
            raw.extend(rx.try_iter().flatten());
        }
        // Up to the last whole character, so the text only ever grows.
        let upto = match std::str::from_utf8(&raw) {
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            _ => raw.len(),
        };
        let text = console::strip_ansi(&String::from_utf8_lossy(&raw[..upto]));
        for line in text.split_inclusive('\n').skip(printed).filter(|l| l.ends_with('\n')) {
            println!("    | {}", syms.symbolize_line(line.trim_end()));
            printed += 1;
        }
        if let Some((expect, input)) = step
            && let Some(at) = text[matched..].find(expect)
        {
            matched += at + expect.len();
            stdin.write_all(input.as_bytes()).and_then(|()| stdin.flush()).map_err(|e| format!("qemu stdin: {e}"))?;
            step = steps.next();
        }
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s.code();
        }
        if Instant::now() > deadline {
            timed_out = true;
            report_timeout(ktest::last_line(&text));
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
    };
    drop(stdin);
    let _ = reader.join();
    raw.extend(rx.try_iter().flatten());
    let rest = console::strip_ansi(&String::from_utf8_lossy(&raw));
    for line in rest.lines().skip(printed) {
        println!("    | {}", syms.symbolize_line(line));
    }
    Ok(Boot { status, output: rest, timed_out })
}

/// Type commands into the kernel shell on the serial port and check what
/// they print; `reboot` ends the boot (QEMU runs with `-no-reboot`).
fn shell_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let disk = test_disk()?;
    let devices = Devices::Test(&disk);
    let append = format!("poly.qemu_exit {SHELL_FLAG} poly.test=shell:: {}", devices.flags());
    let initrd = programs::test_initramfs()?;
    let qemu_args = ["-initrd".to_string(), initrd.display().to_string()];
    let commands = [
        "help\r",
        "uptime\r",
        "lspci\r",
        "hlep\r",
        // Tab completes "echo"; Left moves back over the "c" to insert "b".
        "ech\t'over  serial'\r",
        "echo ac\x1b[Db\r",
        "test shell::\r",
        "ls /test\r",
        "cat /etc/motd\r",
        "run /bin/hello\r",
        "reboot\r",
    ];
    let steps: Vec<_> = commands.iter().map(|&c| (console::PROMPT, c)).collect();
    let b = boot_typing(kernel, syms, &append, devices, &qemu_args, &steps)?;
    let boot_log = b.output.split(console::PROMPT).next().unwrap_or_default();
    b.tests(boot_log)?;
    let ex = console::exchanges(&b.output);
    if ex.len() < commands.len() {
        return Err(format!(
            "the shell answered {} of {} command lines (status {:?})",
            ex.len(),
            commands.len(),
            b.status
        ));
    }
    let out = |i: usize| ex[i].output;
    let checks: [(&str, bool); 11] = [
        (
            "help lists the commands",
            ["uptime ", "lspci ", "reboot ", "list the commands"].iter().all(|s| out(0).contains(s)),
        ),
        ("uptime", out(1).starts_with("up ") && out(1).contains(" ticks at 1000 Hz")),
        ("lspci lists the test disk", out(2).contains(" 1af4:1042 SCSI storage") && out(2).contains(" functions\n")),
        ("an unknown command gets a suggestion", out(3) == "unknown command 'hlep'; did you mean 'help'?\n"),
        ("Tab completes over the serial line", out(4) == "over  serial\n"),
        ("cursor keys edit over the serial line", out(5) == "abc\n"),
        ("test runs the kernel tests at runtime", out(6).contains("test result: ok.") && !out(6).contains("FAILED")),
        (
            "ls lists the initramfs",
            [
                "-rw-r--r--        26 hello.txt\n",
                "lrwxrwxrwx         9 link -> hello.txt\n",
                "crw-rw-rw-      1, 5 zero-node\n",
            ]
            .iter()
            .all(|s| out(7).contains(s)),
        ),
        ("cat prints /etc/motd", out(8) == initramfs::MOTD),
        (
            "run starts /bin/hello and reports its exit",
            out(9).contains(programs::HELLO_OUTPUT) && out(9).ends_with("/bin/hello: exit status 0\n"),
        ),
        ("reboot uses the ACPI reset register", out(10).contains("reboot: ACPI reset register")),
    ];
    for (what, ok) in checks {
        if !ok {
            return Err(format!("shell over the serial port: {what} failed"));
        }
    }
    // QEMU turns the reset into an exit with -no-reboot.
    if b.status != Some(0) {
        return Err(format!("reboot did not reset the machine (status {:?})", b.status));
    }
    Ok(())
}

/// Boot with the ext2 test disk mounted on /mnt, write a file there through
/// the shell, `sync` and `reboot` (which unmounts it clean); check the
/// image with `e2fsck -fn` if e2fsprogs are installed; then boot again from
/// the same image and read the file back.
fn ext2_persistence_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    const TEXT: &str = "persisted across boots";
    let disk = test_disk()?;
    let image = disk.with_file_name(ext2disk::FILE_NAME);
    let devices = Devices::Test(&disk);
    let append = format!("poly.qemu_exit {SHELL_FLAG} poly.test=ext2_test_disk {}", devices.flags());
    let first = [
        (console::PROMPT, format!("write /mnt/persist.txt {TEXT}\r")),
        (console::PROMPT, "sync\r".to_string()),
        (console::PROMPT, "reboot\r".to_string()),
    ];
    let steps: Vec<_> = first.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let b = boot_typing(kernel, syms, &append, devices, &[], &steps)?;
    b.tests(b.output.split(console::PROMPT).next().unwrap_or_default())?;
    let ex = console::exchanges(&b.output);
    let out = |i: usize| ex.get(i).map_or("", |e| e.output);
    if ex.len() < 3 || !out(0).is_empty() || !out(1).contains("every disk synced") {
        return Err(format!("writing to the ext2 test disk failed (status {:?}): {:?}", b.status, ex));
    }
    if !out(2).contains("fs: /mnt (virtio2) unmounted clean") || b.status != Some(0) {
        return Err(format!("reboot did not unmount the ext2 test disk clean (status {:?})", b.status));
    }
    match ext2disk::e2fsck(&image)? {
        Some(_) => eprintln!("==> e2fsck -fn passes on the image after the writing boot"),
        None => eprintln!("==> e2fsck not installed; skipped checking the image"),
    }
    let second = ["cat /mnt/persist.txt\r", "mount\r", "reboot\r"];
    let steps: Vec<_> = second.iter().map(|c| (console::PROMPT, *c)).collect();
    let b = boot_typing(kernel, syms, &append, devices, &[], &steps)?;
    b.tests(b.output.split(console::PROMPT).next().unwrap_or_default())?;
    let ex = console::exchanges(&b.output);
    let out = |i: usize| ex.get(i).map_or("", |e| e.output);
    if out(0) != format!("{TEXT}\n") {
        return Err(format!("the file written in the first boot reads {:?} in the second", out(0)));
    }
    if !out(1).contains("virtio2 on /mnt type ext2 (rw)") || b.status != Some(0) {
        return Err(format!("the second boot did not mount the ext2 test disk (status {:?})", b.status));
    }
    Ok(())
}

/// The boot options that limit the CPUs: `poly.cpus=<n>` starts that many
/// (the boot CPU included) and `poly.smp=off` only the boot CPU. The SMP
/// tests run in each boot, against the CPUs that run.
fn smp_option_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    for (option, cpus, online) in [("poly.cpus=2", 4, 2), ("poly.smp=off", QEMU_CPUS, 1)] {
        eprintln!("==> boot test: {option} on {cpus} CPUs runs the kernel on {online}");
        let append = format!("poly.qemu_exit poly.test=smp:: {option}");
        let args = ["-smp".to_string(), cpus.to_string()];
        let b = boot_with(kernel, syms, DEFAULT_MEMORY, &append, Devices::None, &args, None, |_| {})?;
        b.tests(&b.output)?;
        let left_out = cpus - online;
        let line = format!(
            "smp: {online} of {cpus} CPUs online (0{}), {left_out} left out by poly.smp / poly.cpus",
            if online > 1 { format!("-{}", online - 1) } else { String::new() }
        );
        if b.status != Some(QEMU_SUCCESS) || !b.output.contains(&line) {
            return Err(format!(
                "{option} on {cpus} CPUs did not run on {online} (expected \"{line}\", status {:?})",
                b.status
            ));
        }
    }
    Ok(())
}

/// `cargo xtask busybox-test`: the BusyBox boot test alone.
fn busybox_test() -> Result {
    run_boot_groups(&["busybox".into()])
}

/// A self-test boot: QEMU's RAM, the test disk polled (without MSI-X) or
/// not, a `-cpu` model (QEMU's default one if none), and the CPUs.
#[derive(Clone, Copy, Debug)]
struct SelfTest {
    memory: &'static str,
    polled: bool,
    cpu: Option<&'static str>,
    cpus: u32,
}

// 2 GiB puts RAM, and QEMU's ACPI tables, above the first GiB that the boot
// stub maps: the direct map must reach them. Every self-test boot has the
// test disk, so the block layer tests read and write it; the polled one has
// it without MSI-X, so the kernel must poll it. `max` runs on QEMU's most
// capable CPU model, whose SMEP, SMAP and XSAVE (with AVX) the default one
// lacks, so user mode is tested with and without them. All but `max` boot
// with the test initramfs (QEMU puts it at the top of RAM, so with 2 GiB
// above the first GiB); `max` has none, so the kernel must come up with an
// empty root as well. They run on 2, 4, 1 and 4 CPUs: every test passes
// with one CPU and with several.
const SELFTEST_DEFAULT: SelfTest = SelfTest { memory: DEFAULT_MEMORY, polled: false, cpu: None, cpus: QEMU_CPUS };
const SELFTEST_2G: SelfTest = SelfTest { memory: "2G", polled: false, cpu: None, cpus: 4 };
const SELFTEST_POLLED: SelfTest = SelfTest { memory: DEFAULT_MEMORY, polled: true, cpu: None, cpus: 1 };
const SELFTEST_MAX: SelfTest = SelfTest { memory: DEFAULT_MEMORY, polled: false, cpu: Some("max"), cpus: 4 };

/// One self-test boot ([`SelfTest`]): every kernel test passes, and the
/// boot log shows the CPUs, disks, display, user mode and root file system
/// set up as the configuration asks.
fn selftest_boot(kernel: &Path, syms: &Arc<Symbols>, config: SelfTest) -> Result {
    let SelfTest { memory, polled, cpu, cpus } = config;
    let how = if polled { "the test disk without MSI-X (polled)" } else { "the test devices" };
    let on = cpu.map_or(String::new(), |c| format!(" on -cpu {c}"));
    let initrd = if cpu.is_none() { Some(programs::test_initramfs()?) } else { None };
    let initrd = initrd.as_deref();
    let with = if initrd.is_some() { "the test initramfs" } else { "no initramfs" };
    eprintln!("==> boot test: self-test passes on {cpus} CPU(s) with {memory} of RAM, {how} and {with}{on}");
    let disk = test_disk()?;
    let devices = if polled { Devices::PolledDisk(&disk) } else { Devices::Test(&disk) };
    let mut append = format!("poly.qemu_exit {}", devices.flags());
    if initrd.is_some() {
        append = format!("{append} {}", initramfs::TEST_INITRD_FLAG);
    }
    let mut args = vec!["-smp".to_string(), cpus.to_string()];
    if let Some(c) = cpu {
        args.extend(["-cpu".into(), c.into()]);
    }
    if let Some(file) = initrd {
        args.extend(["-initrd".into(), file.display().to_string()]);
    }
    let b = boot_with(kernel, syms, memory, &append, devices, &args, None, |_| {})?;
    if let Some(s) = ktest::parse(&b.output).summary {
        eprintln!("==> kernel tests: {} passed, {} failed, {} filtered out", s.passed, s.failed, s.filtered_out);
    }
    b.tests(&b.output)?;
    match b.status {
        Some(QEMU_SUCCESS) if b.output.contains(PASS_MARKER) => {}
        Some(s) => return Err(format!("self-test boot ({memory}) exited with status {s}")),
        None => return Err(format!("self-test boot ({memory}) timed out after {BOOT_TIMEOUT:?}")),
    }
    let acpi = format!("acpi: CPUs: {cpus} enabled");
    if !b.output.contains(&acpi) {
        return Err(format!("ACPI did not report the {cpus} CPUs QEMU emulates with {memory} (expected \"{acpi}\")"));
    }
    let online = format!("smp: {cpus} of {cpus} CPUs online");
    if !b.output.contains(&online) {
        return Err(format!("not all of the {cpus} CPUs came up with {memory}{on} (expected \"{online}\")"));
    }
    // The first sector of the test disk starts with "POLY", sector 0,
    // read through the block layer.
    let first_read = "block: virtio0: block 0: 50 4f 4c 59 00 00 00 00";
    if !b.output.lines().any(|l| l.contains(" 1af4:1042 ")) || !b.output.contains(first_read) {
        return Err(format!("the virtio-blk test disk was not enumerated and read with {memory}"));
    }
    if !b.output.contains("block: ram0: RAM disk") {
        return Err(format!("the RAM disk was not registered with {memory}"));
    }
    let (w, h) = DEFAULT_DISPLAY;
    let display = b.output.lines().find(|l| l.contains("display: ") && l.contains(" bytes per line;"));
    if !display.is_some_and(|l| l.contains(&format!(" {w}x{h}x32, ")) && l.contains(" write-combining;")) {
        return Err(format!("the display did not come up {w}x{h}, write-combining, with {memory}"));
    }
    // How the disk waits, as the boot log says.
    let virtio0 = b.output.lines().find(|l| l.contains("block: virtio0: virtio-blk")).unwrap_or_default();
    let mode_ok = if polled {
        b.output.contains(": no MSI-X; polling") && virtio0.ends_with("; polled")
    } else {
        b.output.contains(": request queue on MSI-X entry 0 -> vector 0x")
            && virtio0.contains("; interrupt-driven (vector 0x")
    };
    // User mode: set up (with the CPU's protections when it has them),
    // and user threads that fault are killed with a log line while the
    // kernel goes on.
    let user = b.output.lines().find(|l| l.starts_with("user: syscall/sysret on;")).unwrap_or_default();
    let protected = user.contains("SMEP: yes, SMAP: yes") && user.contains("FPU state: XSAVE");
    if user.is_empty() || (cpu == Some("max") && !protected) {
        return Err(format!("user mode was not set up as expected{on}: \"{user}\""));
    }
    if !b.output.lines().any(|l| l.starts_with("user: pid ") && l.contains(" killed: page fault (vector 14)")) {
        return Err(format!("no user thread was reported killed by a page fault{on}"));
    }
    if !mode_ok {
        let want = if polled { "polled" } else { "interrupt-driven through MSI-X" };
        return Err(format!("the test disk was not {want} with {memory}"));
    }
    // The root file system: the initramfs unpacked and its pages freed,
    // or an empty tmpfs.
    let root_ok = if initrd.is_some() {
        b.output.contains("fs: initramfs unpacked into /: ")
            && b.output
                .lines()
                .any(|l| l.starts_with("fs: initramfs: ") && l.contains(" returned to the frame allocator"))
    } else {
        b.output.contains("fs: no initramfs (no boot module); / is an empty tmpfs")
    };
    if !root_ok {
        return Err(format!("the root file system was not set up from {with} with {memory}{on}"));
    }
    Ok(())
}

/// Keys pressed on the host reach the kernel through the i8042 and IRQ 1.
fn keyboard_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    eprintln!("==> boot test: keys sent through the QEMU monitor type text (US layout)");
    keyboard_boot_test(kernel, syms, None, &["shift-p", "o", "l", "y", "ret"], "Poly")?;
    // QEMU names keys by their US position: "z" is the Swiss German y, "equal"
    // the ^ dead key, "bracket_left" ü, "bracket_right" the ¨ dead key.
    eprintln!("==> boot test: keys sent through the QEMU monitor type text (Swiss German: AltGr, dead keys)");
    const DE_CH_KEYS: [&str; 14] = [
        "p",
        "o",
        "l",
        "z",
        "alt_r-2",
        "equal",
        "a",
        "alt_r-equal",
        "n",
        "bracket_left",
        "shift-bracket_left",
        "bracket_right",
        "spc",
        "ret",
    ];
    keyboard_boot_test(kernel, syms, Some("de-ch"), &DE_CH_KEYS, "poly@\u{e2}\u{f1}\u{fc}\u{e8}\u{a8}")?;
    Ok(())
}

/// Expecting a device without attaching it must fail its tests, so a boot
/// that silently lost it cannot pass.
fn missing_device_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    eprintln!("==> boot test: the disk tests fail when the test disk is missing");
    let only = format!("poly.qemu_exit poly.test=virtio_blk::device_found {TEST_DISK_FLAG}");
    let b = boot(kernel, syms, DEFAULT_MEMORY, &only, Devices::None)?;
    let report = ktest::parse(&b.output);
    let failed = matches!(report.failed.as_slice(), [(name, _)] if name == "virtio_blk::device_found_with_sane_bars");
    if b.status != Some(QEMU_FAILURE) || !failed {
        return Err(format!("a missing test disk was not reported as a failure (status {:?})", b.status));
    }

    // The same for the display: no VGA (`-vga none`), but the flag.
    eprintln!("==> boot test: the display tests fail when there is no display");
    let only = format!("poly.qemu_exit poly.test=display::device_found {TEST_DISPLAY_FLAG}");
    let b =
        boot_with(kernel, syms, DEFAULT_MEMORY, &only, Devices::None, &["-vga".into(), "none".into()], None, |_| {})?;
    let report = ktest::parse(&b.output);
    let failed = matches!(report.failed.as_slice(), [(name, _)] if name == "display::device_found_and_mode_set");
    if b.status != Some(QEMU_FAILURE) || !failed {
        return Err(format!("a missing display was not reported as a failure (status {:?})", b.status));
    }
    Ok(())
}

/// Faults the kernel provokes on purpose are caught and reported: page
/// faults (W^X among them) and stack overflows, with backtraces.
fn fault_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    eprintln!("==> boot test: page fault is caught and reported with a backtrace");
    let b = expect_fault(kernel, syms, "poly.selftest.page_fault", "CPU exception 14 (page fault)")?;
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
    let b = expect_fault(kernel, syms, "poly.selftest.write_text", "CPU exception 14 (page fault), error 0x3,")?;
    let addr = announced(&b.output, "selftest: writing to kernel text at ")?;
    if !b.output.contains(&format!("cr2 {addr}")) {
        return Err(format!("write to kernel text faulted, but not at {addr}"));
    }
    for (flag, what) in [("poly.selftest.exec_stack", "the stack"), ("poly.selftest.exec_data", "kernel data")] {
        eprintln!("==> boot test: executing {what} is a page fault (no-execute)");
        let b = expect_fault(kernel, syms, flag, "CPU exception 14 (page fault), error 0x11,")?;
        let addr = announced(&b.output, &format!("selftest: executing {what} at "))?;
        if !b.output.contains(&format!("rip {addr},")) || !b.output.contains(&format!("cr2 {addr}")) {
            return Err(format!("executing {what} faulted, but not on the fetch at {addr}"));
        }
    }

    eprintln!("==> boot test: a kernel stack overflow hits the guard page and is reported");
    let b = expect_fault(kernel, syms, "poly.selftest.stack_overflow", "CPU exception 8 (double fault)")?;
    if !b.output.contains("kernel stack overflow: hit the guard page below the boot stack") {
        return Err("stack overflow was not attributed to the boot stack's guard page".into());
    }
    eprintln!("==> boot test: a kernel thread's stack overflow hits its guard and is reported with the thread's name");
    let flag = "poly.selftest.thread_stack_overflow";
    let b = expect_fault(kernel, syms, flag, "CPU exception 8 (double fault)")?;
    if !b.output.contains("kernel stack overflow in thread overflow (slot ") {
        return Err("a thread stack overflow was not attributed to the thread's guard".into());
    }
    let recursed = b
        .output
        .lines()
        .filter_map(symbols::parse_frame)
        .any(|(_, _, s)| s.starts_with("poly_kernel::selftest::recurse+"));
    if !recursed {
        return Err("the thread stack overflow's backtrace does not show the recursion".into());
    }
    Ok(())
}

/// Failing kernel tests and panics (in a test, out of memory, on kernel
/// threads, a deadlock) are reported by name and with backtraces.
fn failure_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    // `selftest::fails_on_request` fails or panics when asked to.
    let only = "poly.qemu_exit poly.test=selftest::fails_on_request";
    eprintln!("==> boot test: a failing kernel test is reported by name");
    let b = boot(kernel, syms, DEFAULT_MEMORY, &format!("{only} poly.selftest.fail"), Devices::None)?;
    let report = ktest::parse(&b.output);
    let failed_alone = matches!(report.failed.as_slice(), [(name, _)] if name == "selftest::fails_on_request");
    let filtered = report.summary.is_some_and(|s| s.passed == 0 && s.filtered_out > 0);
    if b.status != Some(QEMU_FAILURE) || !failed_alone || !filtered || report.check().is_ok() {
        return Err(format!("failing kernel test was not reported as expected (status {:?})", b.status));
    }

    eprintln!("==> boot test: a panic inside a kernel test is reported with a backtrace");
    let b = boot(kernel, syms, DEFAULT_MEMORY, &format!("{only} poly.selftest.panic"), Devices::None)?;
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
    let b = boot(kernel, syms, DEFAULT_MEMORY, only, Devices::None)?;
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
            "a deadlock (a thread locks a mutex it holds)",
            "sync::deadlock_on_request poly.selftest.deadlock",
            "deadlock: thread ",
            "poly_kernel::sync::lock_twice+",
        ),
    ];
    for (what, args, message, function) in thread_panics {
        eprintln!("==> boot test: {what} is reported with a backtrace of the thread's stack");
        let b = boot(kernel, syms, DEFAULT_MEMORY, &format!("poly.qemu_exit poly.test={args}"), Devices::None)?;
        let frames: Vec<_> = b.output.lines().filter_map(symbols::parse_frame).map(|(_, _, sym)| sym).collect();
        let in_backtrace = |f: &str| frames.iter().any(|sym| sym.starts_with(f));
        let named = in_backtrace(function) && in_backtrace("poly_kernel::thread::thread_start+");
        let to_bottom = frames.last().is_some_and(|sym| sym.starts_with("thread_trampoline+"));
        if b.status != Some(QEMU_FAILURE) || !b.output.contains(message) || !named || !to_bottom {
            return Err(format!("{what} was not reported as expected (status {:?})", b.status));
        }
    }
    Ok(())
}

/// A group of boot tests: its name (`cargo xtask boot-tests <name>`) and
/// what it runs, given the kernel and its symbols.
type BootGroup = (&'static str, fn(&Path, &Arc<Symbols>) -> Result);

/// Every boot test, in groups, in the order `test` runs them. CI runs them
/// in parallel jobs of a few groups each ([`CI_BOOT_SHARDS`]).
const BOOT_GROUPS: [BootGroup; 21] = [
    ("selftest-default", |k, s| selftest_boot(k, s, SELFTEST_DEFAULT)),
    ("selftest-2g", |k, s| selftest_boot(k, s, SELFTEST_2G)),
    ("selftest-polled", |k, s| selftest_boot(k, s, SELFTEST_POLLED)),
    ("selftest-max", |k, s| selftest_boot(k, s, SELFTEST_MAX)),
    ("smp", smp_option_tests),
    ("keyboard", keyboard_tests),
    ("shell", |k, s| {
        eprintln!("==> boot test: the shell on the serial port answers typed commands");
        shell_test(k, s)
    }),
    // Static Linux hello worlds as the first process (poly.init).
    ("programs", programs::boot_tests),
    // Dynamically linked glibc programs through glibc's own ld.so (dynamic.rs).
    ("dynamic", dynamic::boot_tests),
    // BusyBox as init, its shell typed into on the serial console (busybox.rs).
    ("busybox", busybox::boot_test),
    // bash, coreutils, Python and gcc from the persistent ext4 root disk
    // (rootdisk.rs).
    ("rootdisk", rootdisk::boot_test),
    // 7-Zip's command line on the NT personality, skipped without the
    // cached 7za.exe (sevenzip.rs, ADR 0087).
    ("7zip", sevenzip::boot_test),
    // 7za.exe under the Windows API tracer, its traces read back and
    // compared with Windows reference traces if there are any (report only;
    // wintrace.rs, ADR 0090).
    ("wintrace", wintrace::boot_test),
    // Networking: virtio-net on QEMU's user networking, against servers on
    // the host (see net.rs).
    ("net", net::boot_tests),
    ("ext2", |k, s| {
        eprintln!("==> boot test: a file written to the ext2 test disk in one boot is there in the next");
        ext2_persistence_test(k, s)
    }),
    ("ext4", ext4disk::boot_tests),
    ("missing-devices", missing_device_tests),
    // What the screen shows: a screenshot through QMP, compared pixel for
    // pixel with the boot screen poly-gfx draws for the same size, after the
    // display tests (which draw and must put the screen back).
    ("screen", screenshot_test),
    // The screen terminal: a command typed on the keyboard runs in the shell
    // on the screen, whose output and status bar show up.
    ("terminal", |k, s| {
        eprintln!(
            "==> boot test: a command typed on the keyboard runs in the screen terminal (screenshots through QMP)"
        );
        terminal::boot_test(k, s, &kernel_version()?)
    }),
    ("faults", fault_tests),
    ("failures", failure_tests),
];

/// The boot test jobs of CI: `.github/workflows/ci.yml` runs one job per
/// shard (`cargo xtask boot-tests --shard <name>`). Every group is in
/// exactly one. Each job builds the kernel first (a few minutes), so a few
/// shards of about a minute of boots each (on TCG) keep the slowest job
/// short without many builds. `cargo xtask ci lint` checks that the
/// workflow's matrix lists exactly these.
const CI_BOOT_SHARDS: [(&str, &[&str]); 4] = [
    ("selftest", &["selftest-default", "selftest-2g", "selftest-polled", "selftest-max", "smp", "keyboard", "shell"]),
    ("userland", &["programs", "dynamic", "busybox", "rootdisk", "7zip", "wintrace"]),
    ("devices", &["net", "ext2", "ext4", "missing-devices", "screen", "terminal"]),
    ("faults", &["faults", "failures"]),
];

/// The groups `selectors` name, in [`BOOT_GROUPS`] order: a selector is a
/// group's name or the part of names before a `-` (`selftest` is every
/// `selftest-*` group).
fn select_boot_groups(selectors: &[String]) -> Result<Vec<BootGroup>> {
    let matches = |sel: &str, name: &str| name == sel || name.strip_prefix(sel).is_some_and(|r| r.starts_with('-'));
    if let Some(bad) = selectors.iter().find(|sel| !BOOT_GROUPS.iter().any(|(n, _)| matches(sel, n))) {
        let names: Vec<_> = BOOT_GROUPS.iter().map(|(n, _)| *n).collect();
        return Err(format!("no boot test group {bad:?}; the groups are {}", names.join(", ")));
    }
    Ok(BOOT_GROUPS.into_iter().filter(|(n, _)| selectors.iter().any(|sel| matches(sel, n))).collect())
}

/// Build the kernel and run the boot test groups `selectors` name
/// ([`select_boot_groups`]), saying how long each took.
fn run_boot_groups(selectors: &[String]) -> Result {
    let groups = select_boot_groups(selectors)?;
    let kernel = build_kernel(false)?;
    let syms = Arc::new(Symbols::from_file(&kernel)?);
    let started = Instant::now();
    let mut took = Vec::new();
    for (name, run) in groups {
        eprintln!("==> boot test group {name} (QEMU on {})", accel::get().name());
        let t = Instant::now();
        run(&kernel, &syms).map_err(|e| format!("boot test group {name}: {e}"))?;
        took.push(format!("{name} {:.0?}", t.elapsed()));
    }
    eprintln!("==> boot tests passed in {:.0?} ({})", started.elapsed(), took.join(", "));
    Ok(())
}

/// The groups of the CI shard `name`.
fn shard_groups(name: &str) -> Result<&'static [&'static str]> {
    CI_BOOT_SHARDS.iter().find(|(s, _)| *s == name).map(|(_, g)| *g).ok_or_else(|| {
        let names: Vec<_> = CI_BOOT_SHARDS.iter().map(|(s, _)| *s).collect();
        format!("no CI shard {name:?}; the shards are {}", names.join(", "))
    })
}

/// `boot-tests [--list] [--shard <name>]... [group...]`: every boot test,
/// or the groups named, or those of CI shards ([`CI_BOOT_SHARDS`]).
fn boot_tests_command(args: &[String]) -> Result {
    let mut selectors = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--list" => {
                for (name, _) in BOOT_GROUPS {
                    let shard = CI_BOOT_SHARDS.iter().find(|(_, g)| g.contains(&name)).map_or("-", |(s, _)| s);
                    println!("{name:<18} CI shard {shard}");
                }
                return Ok(());
            }
            "--shard" => {
                let groups = shard_groups(it.next().ok_or("--shard needs a name")?)?;
                selectors.extend(groups.iter().map(|g| g.to_string()));
            }
            flag if flag.starts_with("--") => return Err(format!("boot-tests: unknown option {flag}")),
            group => selectors.push(group.to_string()),
        }
    }
    if selectors.is_empty() {
        return boot_tests();
    }
    run_boot_groups(&selectors)
}

/// Every boot test group.
fn boot_tests() -> Result {
    let all: Vec<String> = BOOT_GROUPS.iter().map(|(n, _)| n.to_string()).collect();
    run_boot_groups(&all)
}

/// Boot in a mode other than the default, take a screenshot once the
/// display tests have passed, and compare it with the boot screen.
fn screenshot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let (w, h) = SCREENSHOT_DISPLAY;
    eprintln!("==> boot test: the screen shows the boot screen at {w}x{h} (screenshot through QMP)");
    if cfg!(not(unix)) {
        eprintln!("==> skipped: screenshots need a Unix host");
        return Ok(());
    }
    let target = root().join("target");
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let file = target.join("poly-screen-test.ppm");
    // No screen terminal: it would draw over the boot screen once the
    // tests pass.
    let append = format!("poly.test=display {TEST_DISPLAY_FLAG} poly.display={w}x{h} poly.terminal=off");
    let shot = Screenshot { marker: PASS_MARKER, file: &file };
    let b = boot_with(kernel, syms, DEFAULT_MEMORY, &append, Devices::None, &[], Some(shot), |_| {})?;
    b.tests(&b.output)?;
    let got = screen::Image::read_ppm(&file)?;
    let want = screen::boot_screen(w, h, &kernel_version()?);
    screen::compare(&got, &want).map_err(|e| {
        let expected = target.join("poly-screen-expected.ppm");
        let saved =
            want.write_ppm(&expected).map_or_else(|e| e, |()| format!("expected image: {}", expected.display()));
        format!("the screenshot ({}) is not the boot screen: {e}; {saved}", file.display())
    })
}

/// Boot with a `poly.selftest.*` flag that makes the kernel fault on purpose
/// and check that the fault was reported: a panic whose message contains
/// `report`, and QEMU's failure status.
fn expect_fault(kernel: &Path, syms: &Arc<Symbols>, flag: &str, report: &str) -> Result<Boot> {
    let b = boot(kernel, syms, DEFAULT_MEMORY, &format!("poly.qemu_exit {flag}"), Devices::None)?;
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

fn host_tests() -> Result {
    step("host tests", cargo().current_dir(root()).args(["test", "--workspace"]))
}

fn test() -> Result {
    host_tests()?;
    boot_tests()
}

/// Formatting, clippy and the source checks.
fn lint() -> Result {
    step("fmt (host)", cargo().current_dir(root()).args(["fmt", "--all", "--check"]))?;
    step("fmt (kernel)", cargo().current_dir(kernel_dir()).args(["fmt", "--check"]))?;
    step(
        "clippy (host)",
        cargo().current_dir(root()).args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    )?;
    step("clippy (kernel)", kernel_cargo()?.args(["clippy", "--", "-D", "warnings"]))?;
    step(
        "fmt (ntdll)",
        cargo().args(["fmt", "--check", "--manifest-path"]).arg(root().join("personalities/nt/ntdll/Cargo.toml")),
    )?;
    step(
        "fmt (win32)",
        cargo()
            .args(["fmt", "--all", "--check", "--manifest-path"])
            .arg(root().join("personalities/nt/win32/Cargo.toml")),
    )?;
    // The Windows API tracer's DLL sources (no manifest of their own: the
    // kit's generated workspace builds them, wintrace.rs).
    step(
        "fmt (wintrace)",
        Command::new("rustfmt").args(["--check", "--edition", "2024"]).args(
            [
                "tools/wintrace/polytrace/src/lib.rs",
                "tools/wintrace/polytrace/build.rs",
                "tools/wintrace/proxy/common.rs",
            ]
            .map(|f| root().join(f)),
        ),
    )?;
    eprintln!("==> font tables");
    font::run(&root(), &["--check".into()])?;
    eprintln!("==> global_asm! labels");
    asmlabels::check(&kernel_dir().join("src"))?;
    eprintln!("==> merge conflict markers");
    conflicts::check(&root())?;
    eprintln!("==> the CI workflow runs every part and boot test shard");
    check_workflow(&root().join(WORKFLOW))
}

/// The boot tests a second time on QEMU's emulator, when the first ran on
/// KVM: the self-test boots, so code paths only TCG takes (and TCG's own
/// quirks) stay tested. A separate xtask, since the accelerator is chosen
/// once per process.
fn tcg_boot_tests() -> Result {
    if accel::get() != accel::Accel::Kvm {
        return Ok(());
    }
    let xtask = std::env::current_exe().map_err(|e| format!("the xtask binary: {e}"))?;
    let mut cmd = Command::new(xtask);
    cmd.args(["boot-tests", "--shard", TCG_SHARD]).env(accel::ENV, "tcg");
    step(&format!("boot tests on TCG (shard {TCG_SHARD})"), &mut cmd)
}

/// The boot test shard CI also runs on TCG ([`tcg_boot_tests`]).
const TCG_SHARD: &str = "selftest";

/// The coverage map's checks.
fn coverage_checks() -> Result {
    coverage(&["check".into()])?;
    coverage(&["readme".into(), "--check".into()])
}

/// The agent pipeline's checks against the reference OS.
fn pipeline_checks() -> Result {
    // The test corpus on Linux against itself: the tracer and the diff
    // runner must find no differences. Takes a few seconds; skips itself
    // without a C compiler or where ptrace is not permitted.
    pipeline(&["selfcheck".into()])?;
    // The corpus and the visible spec tests of every green coverage entry
    // on Poly against Linux (one QEMU boot per test, several at a time):
    // the tests of tests/linux/poly-pass.txt and of the green entries must
    // pass. Skips itself like selfcheck, and without QEMU.
    pipeline(&["polycheck".into()])
}

/// A part of CI: its name (`cargo xtask ci <name>`) and what it runs.
type CiPart = (&'static str, fn() -> Result);

/// The parts of `cargo xtask ci`, in the order it runs them. CI on GitHub
/// runs each as a job of its own (`cargo xtask ci <part>`), and the boot
/// tests as one job per shard ([`CI_BOOT_SHARDS`], `boot-tests --shard`)
/// plus the TCG run ([`tcg_boot_tests`]).
const CI_PARTS: [CiPart; 5] = [
    ("lint", lint),
    ("host-tests", host_tests),
    ("boot-tests", || boot_tests().and_then(|()| tcg_boot_tests())),
    ("coverage", coverage_checks),
    ("pipeline", pipeline_checks),
];

/// The GitHub Actions workflow that runs CI.
const WORKFLOW: &str = ".github/workflows/ci.yml";

/// Check that the workflow at `path` runs every part of `cargo xtask ci`
/// (but the boot tests, which run by shard), and exactly the boot test
/// shards of [`CI_BOOT_SHARDS`] in its matrix, a line `shard: [a, b, ...]`.
fn check_workflow(path: &Path) -> Result {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    workflow_problems(&text).map_err(|p| format!("{}: {}", path.display(), p.join("; ")))
}

fn workflow_problems(text: &str) -> std::result::Result<(), Vec<String>> {
    let mut problems = Vec::new();
    for (part, _) in CI_PARTS.iter().filter(|(p, _)| *p != "boot-tests") {
        if !text.contains(&format!("cargo xtask ci {part}")) {
            problems.push(format!("no job runs `cargo xtask ci {part}`"));
        }
    }
    let line = text.lines().map(str::trim).find_map(|l| l.strip_prefix("shard: [")?.strip_suffix(']'));
    match line {
        None => problems.push("no boot test matrix line `shard: [...]`".into()),
        Some(list) => {
            let listed: Vec<&str> = list.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
            let want: Vec<&str> = CI_BOOT_SHARDS.iter().map(|(s, _)| *s).collect();
            if listed != want {
                problems.push(format!("the boot test matrix lists shards {listed:?}, xtask has {want:?}"));
            }
        }
    }
    if !text.contains("cargo xtask boot-tests --shard ${{ matrix.shard }}") {
        problems.push("no job runs `cargo xtask boot-tests --shard ${{ matrix.shard }}`".into());
    }
    // The 7zip boot test's binary is fetched and cached by version (ADR 0087).
    if !text.contains("cargo xtask win fetch-7zip") || !text.contains(&format!("key: 7zip-{}", sevenzip::VERSION)) {
        problems.push(format!("no job fetches 7-Zip {} into a cache keyed 7zip-{0}", sevenzip::VERSION));
    }
    if !text.contains(&format!("shard: {TCG_SHARD}")) || !text.contains("accel: tcg") {
        problems.push(format!("the matrix does not run shard {TCG_SHARD} with `accel: tcg`"));
    }
    if problems.is_empty() { Ok(()) } else { Err(problems) }
}

/// `ci [part...]`: every part of CI ([`CI_PARTS`]), or the ones named.
fn ci(args: &[String]) -> Result {
    let names: Vec<&str> = CI_PARTS.iter().map(|(n, _)| *n).collect();
    if let Some(bad) = args.iter().find(|a| !names.contains(&a.as_str())) {
        return Err(format!("ci: no part {bad:?}; the parts are {}", names.join(", ")));
    }
    let started = Instant::now();
    let mut took = Vec::new();
    for (name, run) in CI_PARTS.iter().filter(|(n, _)| args.is_empty() || args.iter().any(|a| a == n)) {
        let t = Instant::now();
        run().map_err(|e| format!("ci {name}: {e}"))?;
        took.push(format!("{name} {:.0?}", t.elapsed()));
    }
    eprintln!("==> ci passed in {:.0?} ({})", started.elapsed(), took.join(", "));
    Ok(())
}

fn pipeline(args: &[String]) -> Result {
    let mut cmd = cargo();
    cmd.current_dir(root()).args(["run", "--quiet", "--package", "poly-pipeline", "--"]).args(args);
    // The commands that build the kernel (`--build-kernel`, or a missing
    // one) build it as `kernel_cargo` does, with our ntdll.dll: the same
    // kernel as the boot tests', so neither build undoes the other.
    if matches!(args.first().map(String::as_str), Some("polycheck" | "diff" | "polyrun")) {
        cmd.env(ntdll::KERNEL_ENV, ntdll::ensure()?);
        cmd.env(win32::KERNEL_ENV, win32::ensure()?);
    }
    step("pipeline", &mut cmd)
}

/// `cargo xtask pipeline ...` exits with the tool's own status: callers
/// act on it (`diff` exits 1 when a test fails).
fn pipeline_command(args: &[String]) -> ExitCode {
    let status =
        cargo().current_dir(root()).args(["run", "--quiet", "--package", "poly-pipeline", "--"]).args(args).status();
    match status {
        Ok(s) => exit_code(s.code()),
        Err(e) => {
            eprintln!("xtask: error: pipeline: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A child's exit status as ours (killed by a signal: failure).
fn exit_code(code: Option<i32>) -> ExitCode {
    code.and_then(|c| u8::try_from(c).ok()).map_or(ExitCode::FAILURE, ExitCode::from)
}

fn coverage(args: &[String]) -> Result {
    step(
        "coverage",
        cargo().current_dir(root()).args(["run", "--quiet", "--package", "poly-coverage", "--"]).args(args),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot(status: Option<i32>, timed_out: bool, output: &str) -> Boot {
        Boot { status, output: output.into(), timed_out }
    }

    /// Only an emulator crash before any device line is retried: not a
    /// timeout, not an exit status, not a crash after devices were set up.
    #[test]
    fn retries_only_early_qemu_crashes() {
        let early = "smp: 2 of 2 CPUs online (0-1)\npci: segment 0 buses 00-ff: ECAM at 0xb0000000";
        assert!(crashed_before_devices(&boot(None, false, early)));
        assert!(!crashed_before_devices(&boot(None, true, early)));
        assert!(!crashed_before_devices(&boot(Some(33), false, early)));
        let late = format!("{early}\npci: 00:00.0 8086:29c0 class 06.00.00");
        assert!(!crashed_before_devices(&boot(None, false, &late)));

        let mut boots = 0;
        let b = retry_qemu_crash(|| {
            boots += 1;
            Ok(boot(if boots < 2 { None } else { Some(33) }, false, early))
        })
        .unwrap();
        assert_eq!((boots, b.status), (2, Some(33)));
        let mut boots = 0;
        let b = retry_qemu_crash(|| {
            boots += 1;
            Ok(boot(None, false, early))
        })
        .unwrap();
        assert_eq!((boots, b.status), (1 + QEMU_CRASH_RETRIES, None));
    }

    /// CI runs every boot test group exactly once, in some shard, and the
    /// shards are named once each.
    #[test]
    fn every_boot_group_is_in_one_ci_shard() {
        for (name, _) in BOOT_GROUPS {
            let n = CI_BOOT_SHARDS.iter().filter(|(_, g)| g.contains(&name)).count();
            assert_eq!(n, 1, "boot test group {name} is in {n} CI shards");
        }
        for (shard, groups) in CI_BOOT_SHARDS {
            assert_eq!(CI_BOOT_SHARDS.iter().filter(|(s, _)| *s == shard).count(), 1, "{shard}");
            for g in groups {
                assert!(BOOT_GROUPS.iter().any(|(n, _)| n == g), "shard {shard} names no group {g}");
            }
        }
        assert!(shard_groups(TCG_SHARD).is_ok());
        let mut names: Vec<_> = BOOT_GROUPS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BOOT_GROUPS.len(), "boot test group names repeat");
    }

    #[test]
    fn selects_boot_groups_by_name_or_prefix() {
        let names = |sel: &[&str]| -> Vec<&str> {
            let sel: Vec<String> = sel.iter().map(|s| s.to_string()).collect();
            select_boot_groups(&sel).unwrap().iter().map(|(n, _)| *n).collect()
        };
        assert_eq!(names(&["selftest"]), ["selftest-default", "selftest-2g", "selftest-polled", "selftest-max"]);
        // In BOOT_GROUPS order, whatever order they were asked for in.
        assert_eq!(names(&["shell", "selftest-max", "smp"]), ["selftest-max", "smp", "shell"]);
        assert!(select_boot_groups(&["self".into()]).is_err());
        assert!(select_boot_groups(&["nope".into()]).is_err());
    }

    /// The workflow in the repository runs every part and shard
    /// (`cargo xtask lint` checks it too); a matrix that lost a shard does not.
    #[test]
    fn the_ci_workflow_runs_every_part_and_shard() {
        let text = std::fs::read_to_string(root().join(WORKFLOW)).unwrap();
        workflow_problems(&text).unwrap();
        let shards: Vec<&str> = CI_BOOT_SHARDS.iter().map(|(s, _)| *s).collect();
        let lost = text.replace(&format!("shard: [{}]", shards.join(", ")), "shard: [selftest]");
        assert!(workflow_problems(&lost).unwrap_err()[0].contains("the boot test matrix lists shards"));
        let no_lint = text.replace("cargo xtask ci lint", "cargo xtask lint");
        assert!(workflow_problems(&no_lint).unwrap_err()[0].contains("ci lint"));
    }
}
