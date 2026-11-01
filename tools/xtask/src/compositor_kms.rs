//! The compositor on DRM/KMS (ADR 0116): boot group `wayland-kms`.
//!
//! `poly-compositor --kms` runs as init on a virtio GPU of [`SIZE`] and
//! starts `/bin/wl-proto` (`tests/linux/wayland/wl-proto.c`), which goes
//! through the protocols one window at a time:
//!
//! 1. **vsync**: 60 frames paced by page flips; every frame's
//!    presentation feedback has the vsync, hardware clock and completion
//!    flags, times a whole number of refresh periods apart and vblank
//!    sequences counting them;
//! 2. **subsurface**: a synchronized subsurface shown with its parent;
//! 3. **popup**: a popup the constraint adjustment keeps on the screen;
//! 4. **dmabuf**: a dumb buffer of `/dev/dri/card0` shared as a PRIME
//!    dma-buf (`zwp_linux_dmabuf_v1`) and composed;
//! 5. **clipboard**: a second client pastes the first one's selection.
//!
//! The client checks each picture the compositor writes (`SIGUSR1`); at each
//! `wl-proto: show <name>` this test takes a screenshot of the scanout
//! through QMP (PNG in `$POLY_KMS_SHOTS`, default `target/wayland-kms/`),
//! counts the scenario's colours in it, and answers on the serial console,
//! which the client waits for. At the end the compositor reports its flips
//! (`N flips in T ms (R per second)`): R must not exceed the mode's
//! refresh rate.
//!
//! Skipped without a C compiler and static C library, or on a host
//! without Unix sockets.

use crate::screen::{Image, Qmp};
use crate::symbols::Symbols;
use crate::{BOOT_TIMEOUT, Devices, QEMU_SUCCESS, Result, compositor, initramfs, qemu, root, virtio_gpu_args};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// The scanout's size.
const SIZE: (u32, u32) = (1280, 800);
/// Guest memory: two 4 MB scanout buffers, the clients' buffers.
const MEMORY: &str = "512M";
/// What the client prints when every check passed.
const CLIENT_OK: &str = "wl-proto: ok";
/// What it shows, in order, and a colour (`0xRRGGBB`) with how many
/// pixels of it the screenshot must have (`None`: at least 10 000).
const SHOWN: [(&str, u32, Option<u32>); 5] = [
    ("vsync", 0x3a5f8a, None),
    ("subsurface", 0xd04a4a, Some(60 * 40)),
    ("popup", 0xe0c040, Some(120 * 80)),
    ("dmabuf", 0x40b0e0, Some(128 * 96)),
    ("clipboard", 0x404040, Some(80 * 60)),
];

fn shots_dir() -> PathBuf {
    std::env::var_os("POLY_KMS_SHOTS").map_or_else(|| root().join("target").join("wayland-kms"), PathBuf::from)
}

/// How many pixels of `rgb` the image has.
fn count(img: &Image, rgb: u32) -> u32 {
    let want = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
    img.rgb.chunks_exact(3).filter(|p| *p == want).count() as u32
}

/// The flips the compositor reported: count, and rate in millihertz.
fn flips(log: &str) -> Option<(u64, u64)> {
    let line = log.lines().find(|l| l.contains("poly-compositor: ") && l.contains(" flips in "))?;
    let rest = line.split("poly-compositor: ").nth(1)?;
    let n = rest.split(' ').next()?.parse().ok()?;
    let rate = rest.split('(').nth(1)?.split(' ').next()?;
    let (int, frac) = rate.split_once('.')?;
    Some((n, int.parse::<u64>().ok()? * 1000 + frac.parse::<u64>().ok()?))
}

/// The mode's refresh rate the compositor reported, in millihertz.
fn refresh(log: &str) -> Option<u64> {
    let line = log.lines().find(|l| l.contains("poly-compositor: KMS on "))?;
    let hz = line.split(" at ").nth(1)?.split(' ').next()?;
    let (int, frac) = hz.split_once('.')?;
    Some(int.parse::<u64>().ok()? * 1000 + frac.parse::<u64>().ok()?)
}

/// The boot test.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    if cfg!(not(unix)) {
        eprintln!("==> skipped: the KMS compositor boot test needs a Unix host (QMP)");
        return Ok(());
    }
    let client = match compositor::build_c("tests/linux/wayland/wl-proto.c", "wl-proto") {
        Ok(elf) => elf,
        Err(why) => {
            eprintln!("==> skipped: the KMS compositor boot test: {why}");
            return Ok(());
        }
    };
    let comp = compositor::build_compositor()?;
    let image = initramfs::programs_image(&[("poly-compositor", &comp), ("wl-proto", &client)]);
    let image = initramfs::write(&root(), "poly-wayland-kms-initramfs.cpio", &image)?;
    eprintln!(
        "==> boot test: the compositor on DRM/KMS (page flips, vsync-paced frames, a subsurface, a popup, a dma-buf \
         client buffer, the clipboard between two clients; screenshots through QMP)"
    );
    drive(kernel, syms, &image)
}

fn drive(kernel: &Path, syms: &Arc<Symbols>, image: &Path) -> Result {
    let shots = shots_dir();
    std::fs::create_dir_all(&shots).map_err(|e| format!("{}: {e}", shots.display()))?;
    let socket = std::env::temp_dir().join(format!("poly-kms-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let args = ["--kms", "--exec", "/bin/wl-proto"];
    let append =
        args.iter().fold("poly.qemu_exit poly.terminal=off poly.init=/bin/poly-compositor".to_string(), |a, w| {
            format!("{a} poly.init.arg={w}")
        });
    let mut child = qemu(kernel, MEMORY, &append, Devices::None, "stdio")
        .args(["-display", "none"])
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", socket.display()))
        .arg("-initrd")
        .arg(image)
        .args(virtio_gpu_args(Some(SIZE)))
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
    let timeout = BOOT_TIMEOUT * 6;
    let deadline = Instant::now() + timeout;
    let (mut log, mut errors, mut shown) = (String::new(), Vec::new(), Vec::new());
    let mut qmp: Option<Qmp> = None;
    loop {
        let line = match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(l) => l,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                errors.push(format!("timed out after {} s", timeout.as_secs()));
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
        let Some(name) = line.split("wl-proto: show ").nth(1).map(str::trim) else { continue };
        if qmp.is_none() {
            qmp = Some(Qmp::connect(&socket, Duration::from_secs(10))?);
        }
        let q = qmp.as_mut().expect("connected above");
        let png = shots.join(format!("wayland-kms-{}-{name}.png", shown.len() + 1));
        let ppm = shots.join(format!("wayland-kms-{name}.ppm"));
        let checked = q.screendump(&png).and_then(|()| q.screendump(&ppm)).and_then(|()| Image::read_ppm(&ppm));
        let _ = std::fs::remove_file(&ppm);
        match (checked, SHOWN.iter().find(|s| s.0 == name)) {
            (Ok(img), Some(&(_, colour, want))) => {
                let n = count(&img, colour);
                let ok = (img.width, img.height) == SIZE && want.map_or(n >= 10_000, |w| n == w);
                if ok {
                    eprintln!("==> the screen shows \"{name}\" ({n} pixels of #{colour:06x}): {}", png.display());
                } else {
                    errors.push(format!(
                        "screen \"{name}\": {}x{}, {n} pixels of #{colour:06x}, want {want:?} ({})",
                        img.width,
                        img.height,
                        png.display()
                    ));
                }
            }
            (Ok(_), None) => errors.push(format!("an unknown screen \"{name}\"")),
            (Err(e), _) => errors.push(format!("screen \"{name}\": {e}")),
        }
        shown.push(name.to_string());
        stdin.write_all(b"\n").and_then(|()| stdin.flush()).map_err(|e| format!("qemu stdin: {e}"))?;
    }
    if let Some(mut q) = qmp {
        q.quit();
    }
    let status = wait_for(&mut child, Duration::from_secs(10));
    let _ = reader.join();
    let _ = std::fs::remove_file(&socket);
    for line in log.lines().filter(|l| l.contains("not ok ") || l.contains("cut off")) {
        errors.push(line.trim().to_string());
    }
    let expected: Vec<&str> = SHOWN.iter().map(|s| s.0).collect();
    let (n, rate) = flips(&log).unwrap_or((0, 0));
    let hz = refresh(&log).unwrap_or(0);
    eprintln!(
        "==> {n} flips at {}.{:03} per second on a {}.{:03} Hz mode",
        rate / 1000,
        rate % 1000,
        hz / 1000,
        hz % 1000
    );
    let checks = [
        ("the compositor took the screen through KMS", log.contains("poly-compositor: KMS on /dev/dri/card0")),
        ("it took dma-bufs", log.contains("dma-bufs taken")),
        ("every scenario was shown", shown == expected),
        ("the client passed every check", log.contains(CLIENT_OK)),
        ("the compositor flipped at least once a frame", n >= 60),
        ("flips came no faster than the display refreshes", hz > 0 && rate <= hz + hz / 100),
        ("the client exited 0", log.contains("poly-compositor: client exited with status 0")),
        ("the compositor exited 0", log.contains("POLY-EXIT: pid=1 code=0")),
        ("QEMU exited with success", status == Some(QEMU_SUCCESS)),
    ];
    for (what, ok) in checks {
        if !ok {
            errors.push(format!("{what} failed (shown {shown:?}, QEMU status {status:?})"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(format!("the KMS compositor boot test:\n{}", errors.join("\n"))) }
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
    fn reads_the_compositor_s_reports() {
        let log = "    | poly-compositor: KMS on /dev/dri/card0: connector 35, CRTC 33, 1280x800 at 74.971 Hz, \
                   325x203 mm, scale 1, dma-bufs taken\n    | poly-compositor: 123 flips in 4567 ms (26.718 per second)\n";
        assert_eq!(refresh(log), Some(74_971));
        assert_eq!(flips(log), Some((123, 26_718)));
        assert_eq!(flips("nothing"), None);
        let img = Image { width: 2, height: 1, rgb: vec![1, 2, 3, 4, 5, 6] };
        assert_eq!((count(&img, 0x010203), count(&img, 0x040506), count(&img, 0)), (1, 1, 0));
    }
}
