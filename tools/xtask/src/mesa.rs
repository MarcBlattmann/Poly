//! Mesa on Poly's DRM/KMS device (roadmap Phase 6, "Kernel GPU interface
//! compatible with Linux DRM/KMS so Mesa runs unchanged"; ADR 0115).
//!
//! The host's unmodified Mesa goes onto the persistent root disk
//! ([`add_to_root`], called by `rootdisk::collect`), the way ADR 0059 brings
//! bash and gcc: libgbm and its `dri_gbm.so` backend, glvnd's `libEGL` and
//! `libGLESv2` with Mesa's EGL vendor library (`libEGL_mesa.so.0`, named by
//! `/usr/share/glvnd/egl_vendor.d/50_mesa.json`), the DRI loader stubs of the
//! drivers Poly's GPU can lead to (`kms_swrast`, `swrast`, `virtio_gpu`,
//! all links to `libdril_dri.so`) and the Gallium library behind them with
//! LLVM (llvmpipe, softpipe), libdrm's `modetest`, `kmscube` and
//! `eglinfo`. Their libraries come with the `DT_NEEDED` closure. In the
//! poly-dev image they are Ubuntu 24.04's packages (`libgl1-mesa-dri`,
//! `libgbm1`, `libegl-mesa0`, `libegl1`, `libgles2`, `libdrm-tests`,
//! `kmscube`, `mesa-utils`); a host without them gets a note and the boot
//! test is skipped. Nothing of them is committed.
//!
//! The boot test ([`boot_test`]) boots that disk with a virtio GPU whose
//! scanout prefers [`MODE`] and types into its bash:
//!
//! 1. `modetest -M virtio_gpu`: libdrm finds the device by its driver name
//!    and lists encoders, connectors (`Virtual-1`, connected, [`MODE`]),
//!    CRTCs and planes with their properties.
//! 2. `modetest -M virtio_gpu -s <connector>@<crtc>:<mode>`: a legacy mode
//!    set with modetest's colour bars in a dumb buffer, checked by a QMP
//!    screenshot; the same with atomic modesetting (`-a`, with the primary
//!    plane named by `-P`); and with vsynced page flips (`-v`), whose
//!    screenshots must catch both the bars and modetest's plain second
//!    buffer, and whose flip rate it reports.
//! 3. `kmscube` with Mesa's software rasteriser (`LIBGL_ALWAYS_SOFTWARE=1`,
//!    GBM's DRI backend on `kms_swrast`, llvmpipe, EGL on GBM, GLES 2) for
//!    [`FRAMES`] frames of legacy page flips with events; screenshots while
//!    it runs must show a cube (pixels other than its clear colour) that
//!    moved between two of them. Then kmscube without asking for software:
//!    Mesa's loader picks its `virtio_gpu` driver by the device, which finds
//!    no 3D (Poly answers the virtio-gpu driver ioctls `EINVAL`), and GBM
//!    falls back to `kms_swrast` by itself.
//! 4. After each program the console has the display back.
//!
//! No system call may be missing (the kernel's `not implemented` lines).
//! (`kmscube -A`, atomic with fences, needs `EGL_ANDROID_native_fence_sync`,
//! which Mesa's software drivers do not offer; modetest covers atomic.)

use crate::rootdisk::{self, Collector};
use crate::screen::{Image, Qmp};
use crate::symbols::Symbols;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The programs copied from `/usr/bin`; `modetest` and `kmscube` are
/// required, `eglinfo` is a help when something fails.
const PROGRAMS: [(&str, bool); 3] = [("modetest", true), ("kmscube", true), ("eglinfo", false)];

/// The DRI drivers Mesa's loader may pick for Poly's virtio GPU: by its
/// driver name, and the software ones (each a link to `libdril_dri.so`,
/// which finds the driver in the Gallium library).
const DRI_DRIVERS: [&str; 3] = ["kms_swrast_dri.so", "swrast_dri.so", "virtio_gpu_dri.so"];

/// glvnd's EGL vendor file for Mesa, and the directory of `.conf` files
/// Mesa reads its driver options from.
const EGL_VENDOR_DIR: &str = "/usr/share/glvnd/egl_vendor.d";
const DRIRC_DIR: &str = "/usr/share/drirc.d";

/// Add Mesa, modetest and kmscube to the root disk (see the module
/// documentation); `have` gets "mesa" when everything the boot test needs
/// is there, `notes` says what is missing otherwise.
pub(crate) fn add_to_root(
    c: &mut Collector,
    have: &mut BTreeSet<&'static str>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for (name, required) in PROGRAMS {
        let path = ["/usr/bin", "/bin"].iter().map(|d| Path::new(d).join(name)).find(|p| p.exists());
        match path {
            Some(p) => c.add(&p)?,
            None if required => missing.push(name.to_string()),
            None => {}
        }
    }
    // The multiarch library directory, where libgbm is.
    let Some(libdir) = c.find_library("libgbm.so.1").and_then(|p| p.parent().map(Path::to_path_buf)) else {
        missing.push("libgbm.so.1".into());
        notes.push(format!("mesa: not on this host (missing {})", missing.join(", ")));
        return Ok(());
    };
    for name in DRI_DRIVERS {
        let p = libdir.join("dri").join(name);
        if p.exists() { c.add(&p)? } else { missing.push(format!("dri/{name}")) }
    }
    let gbm = libdir.join("gbm/dri_gbm.so");
    if gbm.exists() {
        c.add(&gbm)?;
    } else {
        missing.push("gbm/dri_gbm.so".into());
    }
    // glvnd's libEGL opens the vendor libraries its JSON files name.
    let mut vendors = 0;
    for json in dir_files(Path::new(EGL_VENDOR_DIR)) {
        let Ok(text) = std::fs::read_to_string(&json) else { continue };
        let Some(lib) = json_string(&text, "library_path") else { continue };
        let found = if lib.starts_with('/') { Some(PathBuf::from(&lib)) } else { c.find_library(&lib) };
        if let Some(found) = found.filter(|p| p.exists()) {
            c.add(&json)?;
            c.add(&found)?;
            vendors += 1;
        }
    }
    if vendors == 0 {
        missing.push(format!("an EGL vendor library in {EGL_VENDOR_DIR}"));
    }
    for name in ["libEGL.so.1", "libGLESv2.so.2"] {
        match c.find_library(name) {
            Some(p) => c.add(&p)?,
            None => missing.push(name.into()),
        }
    }
    for conf in dir_files(Path::new(DRIRC_DIR)) {
        c.add(&conf)?;
    }
    if missing.is_empty() {
        have.insert("mesa");
    } else {
        notes.push(format!("mesa: not on this host (missing {})", missing.join(", ")));
    }
    Ok(())
}

/// The regular files and links in `dir`, sorted (none if it is missing).
fn dir_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| !p.is_dir()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

/// The string value of `"key"` in a small JSON text (glvnd's vendor files:
/// `"library_path" : "libEGL_mesa.so.0"`), without escapes.
fn json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

// ---- screenshots -----------------------------------------------------------

/// The colour most pixels of `img` have (a clear colour, a background).
fn dominant(img: &Image) -> [u8; 3] {
    let mut counts = std::collections::HashMap::new();
    for p in img.rgb.chunks_exact(3) {
        *counts.entry([p[0], p[1], p[2]]).or_insert(0u32) += 1;
    }
    counts.into_iter().max_by_key(|&(c, n)| (n, c)).map_or([0; 3], |(c, _)| c)
}

/// How many pixels of `img` are not `colour`.
fn other_than(img: &Image, colour: [u8; 3]) -> usize {
    img.rgb.chunks_exact(3).filter(|p| **p != colour).count()
}

/// How many pixels differ between two images of the same size.
fn changed(a: &Image, b: &Image) -> usize {
    a.rgb.chunks_exact(3).zip(b.rgb.chunks_exact(3)).filter(|(x, y)| x != y).count()
}

/// How many distinct colours row `y` of `img` has (modetest's colour bars
/// have seven across the top two thirds).
fn row_colours(img: &Image, y: u32) -> usize {
    (0..img.width).map(|x| img.pixel(x, y)).collect::<BTreeSet<_>>().len()
}

/// What a frame of kmscube must look like: mostly its clear colour, with a
/// cube of at least `min` other pixels, and not the console's boot screen.
fn cube_frame(img: &Image, boot_bg: [u8; 3]) -> Result<([u8; 3], usize), String> {
    let bg = dominant(img);
    if bg == boot_bg {
        return Err(format!("the screen shows the console (background {bg:?}), not kmscube"));
    }
    let cube = other_than(img, bg);
    let total = (img.width * img.height) as usize;
    if cube < total / 100 || cube > total * 2 / 3 {
        return Err(format!("{cube} of {total} pixels are not the clear colour {bg:?}: no cube"));
    }
    Ok((bg, cube))
}

// ---- the boot test -------------------------------------------------------

/// The scanout's preferred mode, which modetest and kmscube set.
pub const MODE: (u32, u32) = (1024, 768);
/// The image the boot test makes afresh.
const TEST_FILE_NAME: &str = "poly-root-disk-mesa.img";
/// How long one step may take: loading Mesa and LLVM, compiling shaders
/// and rendering in software are slow under emulation.
const STEP_TIMEOUT: Duration = Duration::from_secs(600);
const SESSION_TIMEOUT: Duration = Duration::from_secs(2400);
/// The frames each kmscube run renders: 4 s at the 75 Hz the flips are paced
/// to, so that on KVM (where llvmpipe keeps up with every vblank) the
/// screenshots still catch kmscube running.
const FRAMES: u32 = 300;
/// The time between the two screenshots of a run: the cube turns visibly in
/// a few frames.
const SECOND_SHOT: Duration = Duration::from_millis(150);
/// The environment kmscube runs with: Mesa's software rasteriser.
const SOFTWARE: &str = "LIBGL_ALWAYS_SOFTWARE=1";
/// What the kernel logs when a DRM client first shows a framebuffer, and
/// when the console has the display back (ADR 0108).
const TOOK: &str = "display: virtio-gpu: a DRM client took the display";
const RESTORED: &str = "display: virtio-gpu: the console has the display again";

/// The boot test (see the module documentation).
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> super::Result {
    eprintln!("==> boot test: unmodified Mesa (kms_swrast), modetest and kmscube on /dev/dri/card0");
    if cfg!(not(unix)) {
        eprintln!("==> skipped: screenshots need a Unix host");
        return Ok(());
    }
    let (initrd, origin) = match rootdisk::write_initramfs()? {
        Ok(found) => found,
        Err(why) => {
            eprintln!("==> skipped: the root disk boots through BusyBox's init: {why}");
            return Ok(());
        }
    };
    // What the disk would hold, before spending time on writing it.
    let contents = rootdisk::collect()?;
    if !contents.have.contains("mesa") || !contents.have.contains("bash") {
        let why: Vec<&str> = contents
            .notes
            .iter()
            .map(String::as_str)
            .filter(|n| n.starts_with("mesa") || n.starts_with("bash"))
            .collect();
        eprintln!(
            "==> skipped: the Mesa boot test needs Mesa, modetest, kmscube and bash on the host ({})",
            why.join("; ")
        );
        return Ok(());
    }
    let target = super::root().join("target");
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let image = target.join(TEST_FILE_NAME);
    let how = rootdisk::write_image(&contents.tree, &image)?;
    eprintln!("==> root disk {} ({how}); BusyBox from {origin}", image.display());
    let socket = std::env::temp_dir().join(format!("poly-mesa-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let append = format!("{} {}", rootdisk::boot_args(), std::env::var("POLY_MESA_APPEND").unwrap_or_default());
    let mut cmd = super::qemu(kernel, rootdisk::MEMORY, append.trim_end(), super::Devices::None, "stdio");
    cmd.args(["-display", "none"])
        .args(rootdisk::drive_args(&image))
        .arg("-initrd")
        .arg(&initrd)
        .args(super::virtio_gpu_args(Some(MODE)))
        .arg("-qmp")
        .arg(format!("unix:{},server=on,wait=off", socket.display()));
    let mut s = crate::busybox::Session::spawn(cmd, syms)?;
    s.set_timeouts(STEP_TIMEOUT, SESSION_TIMEOUT);
    let result = session(&mut s, &socket, &target, &super::kernel_version()?);
    let _ = std::fs::remove_file(&socket);
    if result.is_err() {
        let log = target.join("poly-mesa-boot.log");
        let _ = std::fs::write(&log, s.text());
        eprintln!("==> the session's output is in {}", log.display());
    }
    result
}

/// Kernel lines in a command's output: kept apart, and a missing system
/// call fails the test.
fn split_kernel_lines(raw: &str, unimplemented: &mut BTreeSet<String>) -> String {
    let mut out = String::new();
    for line in raw.split_inclusive('\n') {
        let kernel = ["syscall: pid ", "linux: pid ", "fs: ", "strace: ", "drm: ", "display: "]
            .iter()
            .any(|p| line.starts_with(p));
        if kernel {
            if line.contains(" not implemented") {
                unimplemented.insert(line.trim_end().to_string());
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// The connector ID of `Virtual-1` in modetest's connector list
/// (`id encoder status name size modes encoders`).
fn connector_id(listing: &str) -> Option<u32> {
    listing.lines().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 4 && f[2] == "connected" && f[3] == "Virtual-1").then(|| f[0].parse().ok()).flatten()
    })
}

/// The first object ID in the section `title` of modetest's listing
/// (`CRTCs:`, `Planes:`): the first line after the section's header that
/// starts with a number.
fn first_id(listing: &str, title: &str) -> Option<u32> {
    let mut lines = listing.lines().skip_while(|l| l.trim() != title).skip(2);
    lines.next()?.split_whitespace().next()?.parse().ok()
}

fn session(s: &mut crate::busybox::Session, socket: &Path, target: &Path, version: &str) -> super::Result {
    let (w, h) = MODE;
    let boot_bg = crate::screen::boot_screen(w, h, version).pixel(0, 0);
    let mut unimplemented = BTreeSet::new();
    let mut failed: Vec<String> = Vec::new();
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    s.run("stty cols 250 rows 50")?;
    let mut qmp = Qmp::connect(socket, Duration::from_secs(10))?;
    // POLY_MESA_STEPS=<text>: only the steps whose name contains it (the
    // listing always runs), for debugging one program.
    let only = std::env::var("POLY_MESA_STEPS").ok().filter(|s| !s.is_empty());
    let skip = |name: &str| only.as_ref().is_some_and(|o| !name.contains(o.as_str()));
    let shot = |qmp: &mut Qmp, name: &str| -> super::Result<Image> {
        let file = target.join(format!("poly-mesa-{name}.ppm"));
        let _ = std::fs::remove_file(&file);
        qmp.screendump(&file)?;
        Image::read_ppm(&file)
    };

    // 1. modetest lists the device's objects.
    let out = split_kernel_lines(&s.run("modetest -M virtio_gpu; echo \"status=$?\"")?, &mut unimplemented);
    let id = connector_id(&out);
    let (crtc, plane) = (first_id(&out, "CRTCs:"), first_id(&out, "Planes:"));
    let listed = ["Encoders:", "Connectors:", "CRTCs:", "Planes:", "Virtual-1", &format!("{w}x{h}"), "status=0"]
        .iter()
        .all(|want| out.contains(want));
    match id {
        Some(id) if listed && crtc.is_some() && plane.is_some() => {
            eprintln!("==> modetest lists the device: connector Virtual-1 is {id}, {w}x{h}")
        }
        _ => failed.push(format!("modetest -M virtio_gpu printed {:?}", tail(&out))),
    }

    // 2. Mode sets with modetest's pattern: legacy (SETCRTC), atomic, and
    //    legacy with vsynced page flips (`-v`, which reports the flip rate).
    if let (Some(id), Some(crtc), Some(plane)) = (id, crtc, plane) {
        let mode = format!("-s {id}@{crtc}:{w}x{h}");
        for (name, flags, until) in [
            ("modetest", mode.clone(), "setting mode"),
            // Atomic modesetting needs the plane named too.
            ("modetest-atomic", format!("-a {mode} -P {plane}@{crtc}:{w}x{h}"), "setting mode"),
            ("modetest-flips", format!("-v {mode}"), "freq: "),
        ] {
            if skip(name) {
                continue;
            }
            s.send(&format!("modetest -M virtio_gpu {flags}; echo \"status=$?\"\r"))?;
            match s.expect_any(&[until, "\nstatus="]) {
                Ok((0, _)) => {}
                Ok((_, got)) => {
                    let out = format!("{got}{}", s.prompt()?);
                    failed.push(format!("{name} ended before {until:?}: {:?}", tail(&out)));
                    continue;
                }
                Err(e) => {
                    failed.push(format!("{name}: {e}"));
                    s.send("\r")?;
                    s.prompt()?;
                    continue;
                }
            }
            std::thread::sleep(Duration::from_secs(2));
            // With `-v` modetest flips between its pattern and a second,
            // plain buffer at every vblank: screenshots must catch both.
            let flips = flags.contains("-v");
            let (mut bars, mut plain) = (0, 0);
            for i in 0..if flips { 16 } else { 1 } {
                let img = shot(&mut qmp, &format!("{name}-{i}"))?;
                match row_colours(&img, h / 4) {
                    n if n >= 7 && img.pixel(0, 0) != boot_bg => bars += 1,
                    1 if dominant(&img) != boot_bg => plain += 1,
                    _ => {}
                }
                if bars > 0 && (plain > 0 || !flips) {
                    break;
                }
            }
            // modetest waits for Enter.
            s.send("\r")?;
            let out = split_kernel_lines(&s.prompt()?, &mut unimplemented);
            let freq = out.lines().rfind(|l| l.contains("freq: ")).unwrap_or_default().trim().to_string();
            if bars == 0 || (flips && plain == 0) {
                failed.push(format!(
                    "{name}: screenshots showed colour bars {bars} times, the plain buffer {plain} times"
                ));
            } else if !out.ends_with("status=0\n") {
                failed.push(format!("{name} printed {:?}", tail(&out)));
            } else if flips {
                eprintln!("==> {name}: the screen flips between the colour bars and a plain buffer, {freq}");
            } else {
                eprintln!("==> {name}: colour bars on the screen");
            }
        }
    }

    // 3. kmscube with Mesa's software rasteriser (legacy page flips with
    //    events), then without asking for it: Mesa's virtio_gpu (virgl)
    //    driver finds no 3D on this device and GBM falls back to kms_swrast.
    for (name, env, frames) in [("kmscube", SOFTWARE, FRAMES), ("kmscube-fallback", "", FRAMES)] {
        if skip(name) {
            continue;
        }
        let line = format!("{env} kmscube -D /dev/dri/card0 -c {frames}; echo \"status=$?\"");
        let started = Instant::now();
        s.send(&format!("{}\r", line.trim_start()))?;
        // EGL is up and the GLES context made once kmscube reports the
        // renderer; it shows its first buffer when the kernel says a DRM
        // client took the display (kmscube's SETCRTC).
        let renderer = s.expect("renderer: \"").and_then(|_| s.expect("\n"));
        let renderer = match renderer.and_then(|r| s.expect(TOOK).map(|_| r)) {
            Ok(r) => r.trim().trim_end_matches('"').to_string(),
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                break;
            }
        };
        // kmscube shows a cleared buffer first, then its frames (the first
        // one late when llvmpipe compiles the shaders): screenshots until
        // one shows the cube, then another a little later.
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let img = shot(&mut qmp, &format!("{name}-0"))?;
            if cube_frame(&img, boot_bg).is_ok() || Instant::now() > deadline {
                seen.push(img);
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        std::thread::sleep(SECOND_SHOT);
        seen.push(shot(&mut qmp, &format!("{name}-1"))?);
        let out = split_kernel_lines(&s.prompt()?, &mut unimplemented);
        let rate = out.lines().rfind(|l| l.starts_with("Rendered ")).unwrap_or_default().to_string();
        eprintln!("==> {name} ({renderer}): {rate}; {:.0?} in all", started.elapsed());
        if !renderer.starts_with("llvmpipe") && !renderer.starts_with("softpipe") {
            failed.push(format!("{name}: the renderer {renderer:?} is not Mesa's software rasteriser"));
        }
        let checked = seen.iter().map(|f| cube_frame(f, boot_bg)).collect::<Result<Vec<_>, _>>().and_then(|cubes| {
            let moved = changed(&seen[0], &seen[1]);
            if moved < 100 {
                return Err(format!("the two frames differ in only {moved} pixels: the cube does not turn"));
            }
            Ok((cubes, moved))
        });
        match checked {
            Ok((cubes, moved)) => eprintln!(
                "==> {name} renders a cube: {} and {} pixels over the clear colour {:?}, {moved} pixels moved",
                cubes[0].1, cubes[1].1, cubes[0].0
            ),
            Err(e) => failed.push(format!("{name}: {e}")),
        }
        let last = format!("Rendered {} frames", frames - 1);
        if !out.contains(&last) || !out.ends_with("status=0\n") {
            failed.push(format!("{name} printed {:?}", tail(&out)));
        }
    }
    let text = s.text();
    let restored = text.matches(RESTORED).count();
    if restored < 5 && only.is_none() {
        failed.push(format!("the console took the display back {restored} times, not after each program"));
    }
    s.send("poweroff\r")?;
    s.expect("reboot: Power down.")?;
    qmp.quit();
    let status = s.finish()?;
    if status != Some(0) {
        failed.push(format!("poweroff (QEMU status {status:?})"));
    }
    if !unimplemented.is_empty() {
        failed.push(format!("system calls Poly does not implement: {unimplemented:?}"));
    }
    if failed.is_empty() { Ok(()) } else { Err(format!("Mesa on Poly: failed: {}", failed.join("; "))) }
}

/// The last lines of `out`, for messages.
fn tail(out: &str) -> String {
    let lines: Vec<&str> = out.lines().collect();
    lines[lines.len().saturating_sub(15)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Image {
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(&f(x, y));
            }
        }
        Image { width: w, height: h, rgb }
    }

    #[test]
    fn json_strings() {
        let text = "{\n \"file_format_version\" : \"1.0.0\",\n \"ICD\" : {\n  \"library_path\" : \"libEGL_mesa.so.0\"\n }\n}\n";
        assert_eq!(json_string(text, "library_path").as_deref(), Some("libEGL_mesa.so.0"));
        assert_eq!(json_string(text, "file_format_version").as_deref(), Some("1.0.0"));
        assert_eq!(json_string(text, "missing"), None);
    }

    #[test]
    fn modetest_object_ids() {
        let listing = "CRTCs:\nid\tfb\tpos\tsize\n19\t0\t(0,0)\t(0x0)\n\nPlanes:\n\
                       id\tcrtc\tfb\tCRTC x,y\tx,y\tgamma size\tpossible crtcs\n22\t0\t0\t0,0\t\t0,0\t0\t0x00000001\n";
        assert_eq!(first_id(listing, "CRTCs:"), Some(19));
        assert_eq!(first_id(listing, "Planes:"), Some(22));
        assert_eq!(first_id(listing, "Encoders:"), None);
    }

    #[test]
    fn modetest_connector_ids() {
        let listing = "Connectors:\nid\tencoder\tstatus\t\tname\t\tsize (mm)\tmodes\tencoders\n\
                       37\t36\tconnected\tVirtual-1      \t0x0\t\t9\t36\n  modes:\n";
        assert_eq!(connector_id(listing), Some(37));
        assert_eq!(connector_id("38\t0\tdisconnected\tVirtual-2\t0x0\t0\t39\n"), None);
    }

    #[test]
    fn cube_frames() {
        let bg = [128, 128, 128];
        let cube =
            image(100, 100, |x, y| if (30..70).contains(&x) && (30..70).contains(&y) { [200, 0, 0] } else { bg });
        assert_eq!(cube_frame(&cube, [0, 0, 0]), Ok((bg, 1600)));
        // The console's background is not kmscube's clear colour.
        assert!(cube_frame(&cube, bg).unwrap_err().contains("console"));
        let empty = image(100, 100, |_, _| bg);
        assert!(cube_frame(&empty, [0, 0, 0]).unwrap_err().contains("no cube"));
        let moved =
            image(100, 100, |x, y| if (35..75).contains(&x) && (30..70).contains(&y) { [200, 0, 0] } else { bg });
        assert_eq!(changed(&cube, &moved), 400);
    }

    #[test]
    fn colour_bars() {
        let bars =
            [[192, 192, 192], [192, 192, 0], [0, 192, 192], [0, 192, 0], [192, 0, 192], [192, 0, 0], [0, 0, 192]];
        let img = image(70, 10, |x, _| bars[(x / 10) as usize]);
        assert_eq!(row_colours(&img, 5), 7);
        assert_eq!(dominant(&image(4, 1, |x, _| if x == 0 { [1, 1, 1] } else { [2, 2, 2] })), [2, 2, 2]);
    }
}
