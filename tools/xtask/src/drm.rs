//! The DRM/KMS boot test (ADR 0108), on a virtio GPU whose scanout prefers
//! [`MODE`]:
//!
//! 1. The kernel's DRM tests (`poly.test=drm::`): the device nodes, and the
//!    hardware side (a dumb buffer shown on scanout 0 instead of the
//!    console, which comes back).
//! 2. `/bin/drm-kms` (`tests/linux/drm/drm_kms.c`, linked statically with
//!    the host's C library, raw ioctls through the uapi headers, no libdrm)
//!    as init: enumeration, dumb buffers, `ADDFB2`, `SETCRTC`, `PAGE_FLIP`
//!    with an event, `DIRTYFB`, vblank waits, an atomic flip, PRIME, and the
//!    console's return when it closes. At each `drm-kms: show <name>` line
//!    the test takes a screenshot through QMP, compares it pixel for pixel
//!    with the same pattern drawn here (and, for `closed`, with the boot
//!    screen the console shows again), and answers with a line on the
//!    serial console, which the program waits for.
//!
//! Skipped, with a message, without a C compiler and static C library, or
//! on a host without Unix sockets.

use crate::screen::{self, Image, Qmp};
use crate::symbols::Symbols;
use crate::{
    BOOT_TIMEOUT, DEFAULT_MEMORY, Devices, QEMU_SUCCESS, Result, TEST_DISPLAY_FLAG, TEST_VIRTIO_GPU_FLAG, boot_with,
    initramfs, kernel_version, qemu, root, virtio_gpu_args,
};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const SOURCE: &str = "tests/linux/drm/drm_kms.c";
/// The scanout's preferred mode, which the program sets.
const MODE: (u32, u32) = (1024, 768);
/// The last line `/bin/drm-kms` writes when every check passed.
const DONE: &str = "drm-kms: done, 0 failed";
/// What the program shows, in order.
const SHOWN: [&str; 4] = ["A", "B", "C", "closed"];
/// The kernel tests that must pass.
const KERNEL_TESTS: [&str; 2] = ["drm::drm_nodes_exist", "drm::drm_hardware_shows_a_buffer_and_gives_the_display_back"];

/// Build `/bin/drm-kms`; `Ok(None)` (with the reason printed) where there is
/// no C compiler or static C library.
fn build() -> Result<Option<Vec<u8>>> {
    let out = root().join("target/xtask-programs");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let binary = out.join("drm-kms");
    let built = Command::new("cc")
        .current_dir(root())
        .args(["-static", "-fno-pie", "-no-pie", "-O2", "-Wall", "-Werror", "-Wl,--build-id=none", "-o"])
        .arg(&binary)
        .arg(SOURCE)
        .output();
    match built {
        Ok(o) if o.status.success() => {
            Ok(Some(std::fs::read(&binary).map_err(|e| format!("{}: {e}", binary.display()))?))
        }
        Ok(o) => {
            let why = String::from_utf8_lossy(&o.stderr);
            if why.contains("cannot find -lc") {
                eprintln!("==> skipped: no static C library for {SOURCE}");
                Ok(None)
            } else {
                Err(format!("building {SOURCE} failed:\n{why}"))
            }
        }
        Err(e) => {
            eprintln!("==> skipped: no C compiler for {SOURCE} ({e})");
            Ok(None)
        }
    }
}

/// The program's patterns (`pattern_a`, `pattern_b` in the C source), as
/// XRGB8888 values.
fn pattern(name: &str, x: u32, y: u32, w: u32, h: u32) -> u32 {
    match name {
        "A" => {
            let b = if (x / 64 + y / 64) & 1 == 1 { 0xe0 } else { 0x20 };
            (x * 255 / (w - 1)) << 16 | (y * 255 / (h - 1)) << 8 | b
        }
        "C" if (100..300).contains(&x) && (120..200).contains(&y) => 0x00ff_ff00,
        _ => {
            if (x / 32 + y / 32) & 1 == 1 {
                0x00c0_3030
            } else {
                0x0030_30c0
            }
        }
    }
}

/// What the screen should show at `name`.
fn expected(name: &str, version: &str) -> Image {
    let (w, h) = MODE;
    if name == "closed" {
        return screen::boot_screen(w, h, version);
    }
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = pattern(name, x, y, w, h);
            rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    Image { width: w, height: h, rgb }
}

/// The DRM boot tests.
pub fn boot_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let (w, h) = MODE;
    eprintln!("==> boot test: the kernel's DRM tests on a virtio GPU ({w}x{h})");
    if cfg!(not(unix)) {
        eprintln!("==> skipped: screenshots need a Unix host");
        return Ok(());
    }
    let append = format!("poly.qemu_exit poly.test=drm:: {TEST_DISPLAY_FLAG} {TEST_VIRTIO_GPU_FLAG} poly.terminal=off");
    let b =
        boot_with(kernel, syms, DEFAULT_MEMORY, &append, Devices::None, &virtio_gpu_args(Some(MODE)), None, |_| {})?;
    let report = b.tests(&b.output)?;
    for test in KERNEL_TESTS {
        if !report.passed.iter().any(|t| t == test) {
            return Err(format!("the kernel test {test} did not run and pass"));
        }
    }
    if !b.output.contains("drm: /dev/dri/card0 (226:0) and renderD128 (226:128) on virtio-gpu") {
        return Err("the kernel did not report its DRM device".into());
    }
    let Some(elf) = build()? else { return Ok(()) };
    let image = initramfs::write(&root(), "poly-drm-initramfs.cpio", &initramfs::programs_image(&[("drm-kms", &elf)]))?;
    eprintln!(
        "==> boot test: /bin/drm-kms sets a mode, flips, draws and closes on /dev/dri/card0 (screenshots through QMP)"
    );
    drive(kernel, syms, &image, &kernel_version()?)
}

/// Boot `/bin/drm-kms` as init and check each screen it shows.
fn drive(kernel: &Path, syms: &Arc<Symbols>, image: &Path, version: &str) -> Result {
    let target = root().join("target");
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let socket = std::env::temp_dir().join(format!("poly-drm-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let append = "poly.qemu_exit poly.init=/bin/drm-kms poly.terminal=off";
    let mut child = qemu(kernel, DEFAULT_MEMORY, append, Devices::None, "stdio")
        .args(["-display", "none"])
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", socket.display()))
        .arg("-initrd")
        .arg(image)
        .args(virtio_gpu_args(Some(MODE)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("qemu-system-x86_64: {e} (is QEMU installed?)"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = mpsc::channel();
    let syms = Arc::clone(syms);
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            let line = syms.symbolize_line(&line);
            println!("    | {line}");
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + BOOT_TIMEOUT * 2;
    let (mut log, mut errors, mut shown) = (String::new(), Vec::new(), Vec::new());
    let mut qmp: Option<Qmp> = None;
    loop {
        let line = match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(l) => l,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                errors.push(format!("timed out after {} s", (BOOT_TIMEOUT * 2).as_secs()));
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        log.push_str(&line);
        log.push('\n');
        if line.contains("KERNEL PANIC") {
            errors.push(format!("the kernel printed \"{}\"", line.trim()));
            break;
        }
        let Some(name) = line.split("drm-kms: show ").nth(1).map(str::trim) else { continue };
        if qmp.is_none() {
            qmp = Some(Qmp::connect(&socket, Duration::from_secs(10))?);
        }
        let q = qmp.as_mut().expect("connected above");
        let file = target.join(format!("poly-drm-{name}.ppm"));
        let want = expected(name, version);
        match q.screendump(&file).and_then(|()| Image::read_ppm(&file)).and_then(|got| screen::compare(&got, &want)) {
            Ok(()) => eprintln!("==> the screen shows \"{name}\" as drawn"),
            Err(e) => {
                let saved = target.join(format!("poly-drm-{name}-expected.ppm"));
                let _ = want.write_ppm(&saved);
                errors.push(format!(
                    "screen \"{name}\" ({}): {e}; expected image: {}",
                    file.display(),
                    saved.display()
                ));
            }
        }
        shown.push(name.to_string());
        // The program waits for a line before it goes on.
        stdin.write_all(b"\n").and_then(|()| stdin.flush()).map_err(|e| format!("qemu stdin: {e}"))?;
    }
    if let Some(mut q) = qmp {
        q.quit();
    }
    let status = wait_for(&mut child, Duration::from_secs(10));
    let _ = reader.join();
    let _ = std::fs::remove_file(&socket);
    if let Some(line) = log.lines().find(|l| l.contains("drm-kms: FAIL")) {
        errors.push(format!("/bin/drm-kms: {line}"));
    }
    let checks = [
        ("every screen was shown", shown == SHOWN),
        ("every check passed", log.contains(DONE)),
        ("init exited 0", log.contains("POLY-EXIT: pid=1 code=0")),
        ("the console took the display back", log.contains("display: virtio-gpu: the console has the display again")),
        ("QEMU exited with success", status == Some(QEMU_SUCCESS)),
    ];
    for (what, ok) in checks {
        if !ok {
            errors.push(format!("{what} failed (shown {shown:?}, QEMU status {status:?})"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

/// QEMU's exit status, waiting up to `timeout` (killed after that).
fn wait_for(child: &mut std::process::Child, timeout: Duration) -> Option<i32> {
    let end = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.code(),
            Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_patterns_match_the_c_program() {
        let a = expected("A", "0");
        assert_eq!(a.pixel(0, 0), [0, 0, 0x20]);
        assert_eq!(a.pixel(1023, 767), [255, 255, 0x20]);
        assert_eq!(a.pixel(64, 0), [15, 0, 0xe0]);
        let c = expected("C", "0");
        assert_eq!(c.pixel(100, 120), [0xff, 0xff, 0]);
        assert_eq!(c.pixel(300, 120), expected("B", "0").pixel(300, 120));
        assert_eq!(c.pixel(0, 0), [0x30, 0x30, 0xc0]);
        assert_eq!(c.pixel(32, 0), [0xc0, 0x30, 0x30]);
    }
}
