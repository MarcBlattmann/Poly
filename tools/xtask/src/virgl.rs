//! 3D acceleration on Poly (roadmap Phase 6, "3D acceleration: virtio-gpu
//! with Venus (Vulkan) / virgl (OpenGL) through Mesa"; ADR 0119).
//!
//! QEMU's `virtio-gpu-gl-pci` runs the guest's virgl command streams on the
//! host's OpenGL through virglrenderer. Without a GPU on the host, that is
//! Mesa's llvmpipe behind QEMU's `egl-headless` display, which needs a DRM
//! render node to make its EGL display on (in the poly-dev container: the
//! WSL kernel's `vgem`, `/dev/dri/renderD128`, passed in by `polyd`). The
//! host's unmodified Mesa goes onto the persistent root disk
//! ([`add_to_root`], called by `rootdisk::collect`): EGL (glvnd's `libEGL`
//! and Mesa's vendor library), GLES, GBM with its DRI backend, the DRI
//! loader stub `virtio_gpu_dri.so` (Mesa's virgl driver, picked by the DRM
//! driver name `virtio_gpu`) and the software ones beside it, the Gallium
//! library behind them, `eglinfo`, `kmscube`, the Vulkan loader with
//! Mesa's Venus ICD (`libvulkan_virtio.so`) and lavapipe, and
//! `vulkaninfo`; and two test programs built here against the Khronos
//! headers, `/usr/local/bin/gl-readback` (`tests/linux/virgl/gl_readback.c`)
//! and `/usr/local/bin/vk-clear` (`tests/linux/virgl/vk_clear.c`). Nothing
//! of them is committed.
//!
//! The boot test ([`boot_test`]) first runs the kernel's 3D test on the
//! device (`gpu3d::virgl_round_trip_through_the_host`: a context, a texture
//! sent to the host and read back, fenced transfers and a fenced
//! submission completed through MSI-X), then boots that disk on a virtio
//! GPU with 3D whose scanout prefers [`MODE`] and types into its bash:
//!
//! 1. `eglinfo -B -p gbm` and `-p surfaceless`: EGL on Poly's render node
//!    with Mesa's virgl driver (the renderer string names virgl).
//! 2. `gl-readback`: GLES clears, a scissored clear and a shader-drawn
//!    rectangle in a framebuffer object, read back with `glReadPixels` and
//!    checked pixel for pixel, on virgl.
//! 3. `kmscube` with hardware OpenGL: GBM allocates its buffers as virgl
//!    resources, kmscube shows them with legacy page flips, the host draws
//!    the cube; screenshots (through QEMU's VNC server: QEMU 8.2 cannot
//!    `screendump` a GL scanout, it has no display surface) must show a
//!    cube that moves.
//! 4. `vulkaninfo --summary` on this device, which has no Venus capset
//!    (Ubuntu's QEMU 8.2): reported only.
//!
//! Then, where QEMU 9.2 is installed ([`QEMU_VENUS`]: the poly-dev image
//! builds it), the disk boots again on its `virtio-gpu-gl-pci` with
//! `venus=on`, blob resources and a host-visible region, and Venus must
//! work: the kernel's 3D test lists the Venus capset, `vulkaninfo
//! --summary` lists the `Virtio-GPU Venus` device and `vk-clear Venus`
//! clears an image on it, copies it to host-visible memory (a mapped host
//! blob) and checks every pixel.
//!
//! No system call may be missing (the kernel's `not implemented` lines).
//! Skipped, with a message, where the host cannot run 3D (no render node,
//! no QEMU OpenGL modules) or has no Mesa.

use crate::screen::Image;
use crate::symbols::Symbols;
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The scanout's preferred mode, which kmscube sets.
pub const MODE: (u32, u32) = (1024, 768);
/// The render node QEMU's EGL display is made on.
const RENDER_NODE: &str = "/dev/dri/renderD128";
/// The image the boot test makes afresh.
const TEST_FILE_NAME: &str = "poly-root-disk-virgl.img";
/// How long one step may take: loading Mesa under emulation is slow.
const STEP_TIMEOUT: Duration = Duration::from_secs(600);
const SESSION_TIMEOUT: Duration = Duration::from_secs(2400);
/// The frames kmscube renders.
const FRAMES: u32 = 120;
/// The kernel's boot flag for a GPU that must have 3D (`gpu3d::TEST_VIRGL_FLAG`).
const TEST_VIRGL_FLAG: &str = "poly.test.virgl";
/// The kernel test of the 3D path on the device.
const KERNEL_TEST: &str = "gpu3d::virgl_round_trip_through_the_host";
/// A QEMU whose virtio GPU has Venus (9.2 or later), where the poly-dev
/// image puts it; `POLY_QEMU_VENUS` overrides.
pub const QEMU_VENUS: &str = "/opt/qemu-venus/bin/qemu-system-x86_64";
/// The test programs built for the disk: source, name, libraries.
const TEST_PROGRAMS: [(&str, &str, &[&str]); 2] = [
    ("tests/linux/virgl/gl_readback.c", "gl-readback", &["-lEGL", "-lGLESv2"]),
    ("tests/linux/virgl/vk_clear.c", "vk-clear", &["-lvulkan"]),
];

/// Programs copied from `/usr/bin`: `eglinfo` and `kmscube` are required,
/// `vulkaninfo` is reported on.
const PROGRAMS: [(&str, bool); 3] = [("eglinfo", true), ("kmscube", true), ("vulkaninfo", false)];
/// The DRI loader stubs Mesa may pick for Poly's virtio GPU (each a link to
/// `libdril_dri.so`).
const DRI_DRIVERS: [&str; 3] = ["virtio_gpu_dri.so", "kms_swrast_dri.so", "swrast_dri.so"];
const EGL_VENDOR_DIR: &str = "/usr/share/glvnd/egl_vendor.d";
const DRIRC_DIR: &str = "/usr/share/drirc.d";
const VULKAN_ICD_DIR: &str = "/usr/share/vulkan/icd.d";
/// The Vulkan drivers: Venus (virtio-gpu) and lavapipe (software).
const VULKAN_ICDS: [&str; 2] = ["virtio_icd.json", "lvp_icd.json"];

/// The regular files and links in `dir`, sorted (none if it is missing).
fn dir_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| !p.is_dir()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

/// The string value of `"key"` in a small JSON text (glvnd's and the
/// Vulkan loader's files: `"library_path": "libEGL_mesa.so.0"`).
fn json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = text[at..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// A shared library on the host, found the way ld.so does.
fn library(name: &str) -> Option<PathBuf> {
    crate::libsearch::SearchPath::host().find(name, &[]).map(|(p, _)| p)
}

/// Add Mesa's EGL, GLES, GBM, virgl and Venus drivers, `eglinfo`,
/// `kmscube` and `vulkaninfo` to the root disk (see the module
/// documentation); `have` gets "virgl" when everything the boot test
/// needs is there, `notes` says what is missing otherwise.
pub(crate) fn add_to_root(
    c: &mut crate::rootdisk::Collector,
    have: &mut BTreeSet<&'static str>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for (name, required) in PROGRAMS {
        match ["/usr/bin", "/bin"].iter().map(|d| Path::new(d).join(name)).find(|p| p.exists()) {
            Some(p) => c.add(&p)?,
            None if required => missing.push(name.to_string()),
            None => notes.push(format!("virgl: {name} not on this host")),
        }
    }
    let Some(libdir) = library("libgbm.so.1").and_then(|p| p.parent().map(Path::to_path_buf)) else {
        notes.push("virgl: Mesa is not on this host (no libgbm.so.1)".into());
        return Ok(());
    };
    for name in DRI_DRIVERS {
        let p = libdir.join("dri").join(name);
        if p.exists() { c.add(&p)? } else { missing.push(format!("dri/{name}")) }
    }
    // The Gallium library the stubs load.
    for p in dir_files(&libdir)
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("libgallium")))
    {
        c.add(&p)?;
    }
    let gbm = libdir.join("gbm/dri_gbm.so");
    if gbm.exists() {
        c.add(&gbm)?
    } else {
        missing.push("gbm/dri_gbm.so".into())
    }
    let mut vendors = 0;
    for json in dir_files(Path::new(EGL_VENDOR_DIR)) {
        let Some(lib) = std::fs::read_to_string(&json).ok().and_then(|t| json_string(&t, "library_path")) else {
            continue;
        };
        if let Some(found) = if lib.starts_with('/') { Some(PathBuf::from(&lib)) } else { library(&lib) } {
            c.add(&json)?;
            c.add(&found)?;
            vendors += 1;
        }
    }
    if vendors == 0 {
        missing.push(format!("an EGL vendor library in {EGL_VENDOR_DIR}"));
    }
    for name in ["libEGL.so.1", "libGLESv2.so.2", "libgbm.so.1"] {
        match library(name) {
            Some(p) => c.add(&p)?,
            None => missing.push(name.into()),
        }
    }
    for conf in dir_files(Path::new(DRIRC_DIR)) {
        c.add(&conf)?;
    }
    // Vulkan: the loader, the ICD files and the drivers they name.
    let mut icds = 0;
    for name in VULKAN_ICDS {
        let json = Path::new(VULKAN_ICD_DIR).join(name);
        let Some(lib) = std::fs::read_to_string(&json).ok().and_then(|t| json_string(&t, "library_path")) else {
            continue;
        };
        if let Some(found) = if lib.starts_with('/') { Some(PathBuf::from(&lib)) } else { library(&lib) } {
            c.add(&json)?;
            c.add(&found)?;
            icds += 1;
        }
    }
    match library("libvulkan.so.1") {
        Some(p) if icds > 0 => c.add(&p)?,
        _ => notes.push("virgl: no Vulkan loader or ICDs (Venus) on this host".into()),
    }
    for (source, name, libs) in TEST_PROGRAMS {
        match build_program(source, name, libs) {
            Ok(elf) => c.tree.file(&format!("usr/local/bin/{name}"), 0o755, &elf),
            Err(why) => notes.push(format!("virgl: {name} not built: {why}")),
        }
    }
    if missing.is_empty() {
        have.insert("virgl");
    } else {
        notes.push(format!("virgl: not on this host (missing {})", missing.join(", ")));
    }
    Ok(())
}

/// Build test program `name` from `source` (dynamically linked against
/// the host's libraries, which the disk carries).
fn build_program(source: &str, name: &str, libs: &[&str]) -> Result<Vec<u8>, String> {
    let out = super::root().join("target/xtask-programs");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let binary = out.join(name);
    let built = std::process::Command::new("cc")
        .current_dir(super::root())
        .args(["-O2", "-Wall", "-Werror", "-o"])
        .arg(&binary)
        .arg(source)
        .args(libs)
        .output()
        .map_err(|e| format!("no C compiler ({e})"))?;
    if !built.status.success() {
        return Err(String::from_utf8_lossy(&built.stderr).lines().take(3).collect::<Vec<_>>().join(" "));
    }
    std::fs::read(&binary).map_err(|e| format!("{}: {e}", binary.display()))
}

/// The QEMU with Venus, if this host has one.
fn qemu_venus() -> Option<PathBuf> {
    let path = std::env::var_os("POLY_QEMU_VENUS").map_or_else(|| PathBuf::from(QEMU_VENUS), PathBuf::from);
    path.exists().then_some(path)
}

/// Whether this host can run QEMU's virtio GPU with 3D: a render node to
/// open and QEMU's OpenGL modules (`virtio-gpu-gl-pci`, `egl-headless`).
/// `Err(why)` if not.
pub fn host_3d() -> Result<(), String> {
    if std::fs::OpenOptions::new().read(true).write(true).open(RENDER_NODE).is_err() {
        return Err(format!("no DRM render node {RENDER_NODE} for QEMU's OpenGL display"));
    }
    let out = std::process::Command::new("qemu-system-x86_64")
        .args(["-display", "help"])
        .output()
        .map_err(|e| format!("qemu-system-x86_64: {e}"))?;
    if !String::from_utf8_lossy(&out.stdout).lines().any(|l| l.trim() == "egl-headless") {
        return Err("QEMU has no egl-headless display (its OpenGL modules are not installed)".into());
    }
    let out = std::process::Command::new("qemu-system-x86_64")
        .args(["-device", "help"])
        .output()
        .map_err(|e| format!("qemu-system-x86_64: {e}"))?;
    if !String::from_utf8_lossy(&out.stdout).contains("\"virtio-gpu-gl-pci\"") {
        return Err("QEMU has no virtio-gpu-gl-pci device".into());
    }
    Ok(())
}

/// QEMU arguments for a virtio GPU with 3D (virgl) whose scanout prefers
/// `size`, shown on the headless EGL display and served over VNC at
/// `vnc` (a Unix socket).
pub fn qemu_args(size: (u32, u32), vnc: &Path) -> Vec<String> {
    gpu_args(size, vnc, "")
}

/// The device options that give QEMU 9.2's virtio GPU Venus: the capset,
/// blob resources and a host-visible region of 1 GiB.
const VENUS_OPTIONS: &str = ",venus=on,blob=on,hostmem=1G";

/// [`qemu_args`] with more device options.
fn gpu_args(size: (u32, u32), vnc: &Path, options: &str) -> Vec<String> {
    let (w, h) = size;
    vec![
        "-vga".into(),
        "none".into(),
        "-device".into(),
        format!("virtio-gpu-gl-pci,xres={w},yres={h}{options}"),
        "-display".into(),
        format!("egl-headless,rendernode={RENDER_NODE}"),
        "-vnc".into(),
        format!("unix:{}", vnc.display()),
    ]
}

// ---- screenshots through VNC -------------------------------------------------

/// A minimal RFB 3.8 client (RFC 6143) for QEMU's VNC server: no
/// authentication, raw encoding, 32-bit true colour. QEMU's egl-headless
/// display reads the GL scanout back into the surface VNC serves.
pub struct Vnc {
    sock: crate::hostfs::UnixStream,
    width: u32,
    height: u32,
}

impl Vnc {
    fn read_exact(&mut self, n: usize) -> Result<Vec<u8>, String> {
        let mut b = vec![0u8; n];
        self.sock.read_exact(&mut b).map_err(|e| format!("VNC: {e}"))?;
        Ok(b)
    }

    /// Connect to QEMU's VNC server at `path`, waiting up to `timeout` for
    /// it to come up.
    pub fn connect(path: &Path, timeout: Duration) -> Result<Vnc, String> {
        let deadline = Instant::now() + timeout;
        let sock = loop {
            match crate::hostfs::UnixStream::connect(path) {
                Ok(s) => break s,
                Err(e) if Instant::now() > deadline => return Err(format!("VNC {}: {e}", path.display())),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        sock.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
        let mut v = Vnc { sock, width: 0, height: 0 };
        let version = v.read_exact(12)?;
        if !version.starts_with(b"RFB 003.") {
            return Err(format!("VNC: not an RFB server ({version:?})"));
        }
        v.sock.write_all(b"RFB 003.008\n").map_err(|e| e.to_string())?;
        let n = v.read_exact(1)?[0] as usize;
        let types = v.read_exact(n)?;
        if !types.contains(&1) {
            return Err(format!("VNC: no unauthenticated access (security types {types:?})"));
        }
        v.sock.write_all(&[1]).map_err(|e| e.to_string())?;
        if v.read_exact(4)? != [0, 0, 0, 0] {
            return Err("VNC: the security handshake failed".into());
        }
        // ClientInit (shared), ServerInit: size, pixel format, name.
        v.sock.write_all(&[1]).map_err(|e| e.to_string())?;
        let init = v.read_exact(24)?;
        v.width = u16::from_be_bytes([init[0], init[1]]) as u32;
        v.height = u16::from_be_bytes([init[2], init[3]]) as u32;
        let name_len = u32::from_be_bytes([init[20], init[21], init[22], init[23]]) as usize;
        v.read_exact(name_len)?;
        // SetPixelFormat: 32 bpp, depth 24, little-endian true colour,
        // red at bit 16, green 8, blue 0. SetEncodings: raw, and the
        // DesktopSize pseudo-encoding (-223) so a mode change is reported.
        let mut pf = vec![0u8, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];
        pf.extend_from_slice(&[2, 0, 0, 2]);
        pf.extend_from_slice(&0i32.to_be_bytes());
        pf.extend_from_slice(&(-223i32).to_be_bytes());
        v.sock.write_all(&pf).map_err(|e| e.to_string())?;
        Ok(v)
    }

    /// The whole screen as it is now.
    pub fn screenshot(&mut self) -> Result<Image, String> {
        let (w, h) = (self.width as u16, self.height as u16);
        let mut req = vec![3u8, 0];
        for v in [0u16, 0, w, h] {
            req.extend_from_slice(&v.to_be_bytes());
        }
        self.sock.write_all(&req).map_err(|e| e.to_string())?;
        loop {
            let ty = self.read_exact(1)?[0];
            match ty {
                0 => break,
                // Bell; server cut text.
                2 => continue,
                3 => {
                    let head = self.read_exact(7)?;
                    let n = u32::from_be_bytes([head[3], head[4], head[5], head[6]]) as usize;
                    self.read_exact(n)?;
                }
                1 => {
                    let head = self.read_exact(5)?;
                    let n = u16::from_be_bytes([head[3], head[4]]) as usize * 6;
                    self.read_exact(n)?;
                }
                other => return Err(format!("VNC: unexpected message {other}")),
            }
        }
        let head = self.read_exact(3)?;
        let rects = u16::from_be_bytes([head[1], head[2]]);
        let mut img =
            Image { width: self.width, height: self.height, rgb: vec![0; (self.width * self.height * 3) as usize] };
        let mut resized = false;
        for _ in 0..rects {
            let r = self.read_exact(12)?;
            let (x, y) = (u16::from_be_bytes([r[0], r[1]]) as u32, u16::from_be_bytes([r[2], r[3]]) as u32);
            let (rw, rh) = (u16::from_be_bytes([r[4], r[5]]) as u32, u16::from_be_bytes([r[6], r[7]]) as u32);
            let enc = i32::from_be_bytes([r[8], r[9], r[10], r[11]]);
            match enc {
                0 => {
                    let data = self.read_exact((rw * rh * 4) as usize)?;
                    for row in 0..rh {
                        for col in 0..rw {
                            let (px, py) = (x + col, y + row);
                            if px >= img.width || py >= img.height {
                                continue;
                            }
                            let s = ((row * rw + col) * 4) as usize;
                            let d = ((py * img.width + px) * 3) as usize;
                            img.rgb[d..d + 3].copy_from_slice(&[data[s + 2], data[s + 1], data[s]]);
                        }
                    }
                }
                -223 => {
                    (self.width, self.height) = (rw, rh);
                    resized = true;
                }
                other => return Err(format!("VNC: unexpected encoding {other}")),
            }
        }
        if resized {
            return self.screenshot();
        }
        Ok(img)
    }
}

// ---- the boot test -------------------------------------------------------

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

/// What a frame of kmscube looks like: mostly its clear colour, with a
/// cube of at least 1% of the pixels, and not the console's boot screen.
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

/// Kernel lines in a command's output: kept apart; a missing system call
/// is noted.
fn split_kernel_lines(raw: &str, unimplemented: &mut BTreeSet<String>) -> String {
    let mut out = String::new();
    for line in raw.split_inclusive('\n') {
        let kernel = ["syscall: pid ", "linux: pid ", "fs: ", "strace: ", "drm: ", "display: ", "virtio-gpu: "]
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

/// The last few lines of `s`, for messages.
fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(12)..].join("\n")
}

/// The boot test (see the module documentation).
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> super::Result {
    eprintln!("==> boot test: Mesa's virgl driver (OpenGL) and Venus (Vulkan) on Poly's virtio GPU with 3D");
    if cfg!(not(unix)) {
        eprintln!("==> skipped: screenshots need a Unix host");
        return Ok(());
    }
    if let Err(why) = host_3d() {
        eprintln!("==> skipped: this host cannot run QEMU's 3D virtio GPU: {why}");
        return Ok(());
    }
    // The kernel's 3D test on the device first: a context, a texture there
    // and back through the host, a fenced submission.
    let vnc = std::env::temp_dir().join(format!("poly-virgl-ktest-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&vnc);
    let append = format!("poly.qemu_exit poly.test=gpu3d:: {TEST_VIRGL_FLAG} poly.terminal=off");
    let b = super::boot_with(
        kernel,
        syms,
        super::DEFAULT_MEMORY,
        &append,
        super::Devices::None,
        &qemu_args(MODE, &vnc),
        None,
        |_| {},
    )?;
    let _ = std::fs::remove_file(&vnc);
    let report = b.tests(&b.output)?;
    if !report.passed.iter().any(|t| t == KERNEL_TEST) {
        return Err(format!("the kernel test {KERNEL_TEST} did not run and pass"));
    }
    if !b.output.contains("drm: virtio-gpu 3D: capsets virgl") {
        return Err("the kernel did not report the device's 3D capsets".into());
    }
    eprintln!("==> {KERNEL_TEST}: a texture went to the host and came back, fences signalled");
    let (initrd, origin) = match crate::rootdisk::write_initramfs()? {
        Ok(found) => found,
        Err(why) => {
            eprintln!("==> skipped: the root disk boots through BusyBox's init: {why}");
            return Ok(());
        }
    };
    let (image, contents) = crate::rootdisk::make(TEST_FILE_NAME)?;
    if !contents.have.contains("virgl") || !contents.have.contains("bash") {
        eprintln!("==> skipped: the virgl boot test needs Mesa, eglinfo, kmscube and bash on the host");
        return Ok(());
    }
    eprintln!("==> BusyBox from {origin}");
    let version = super::kernel_version()?;
    // POLY_VIRGL_ONLY=venus: the Venus boot alone (for debugging it).
    let only_venus = std::env::var("POLY_VIRGL_ONLY").is_ok_and(|v| v == "venus");
    let virgl = if only_venus {
        Ok(())
    } else {
        disk_boot(kernel, syms, &image, &initrd, None, "", "virgl", |s, vnc, target| session(s, vnc, target, &version))
    };
    let venus = match qemu_venus() {
        Some(qemu) => {
            eprintln!(
                "==> boot test: Venus (Vulkan) on {} with venus=on, blobs, a host-visible region",
                qemu.display()
            );
            disk_boot(kernel, syms, &image, &initrd, Some(&qemu), VENUS_OPTIONS, "venus", |s, _, _| venus_session(s))
        }
        None => {
            eprintln!("==> Venus not tried: no QEMU with Venus at {QEMU_VENUS} (POLY_QEMU_VENUS)");
            Ok(())
        }
    };
    match (virgl, venus) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(a), Ok(())) => Err(a),
        (Ok(()), Err(b)) => Err(format!("Venus: {b}")),
        (Err(a), Err(b)) => Err(format!("{a}\nVenus: {b}")),
    }
}

/// Boot the disk `image` (with `initrd`) on a virtio GPU with 3D (more
/// device `options`), with `qemu` instead of the system's if given, and
/// run `steps` in its bash. The session's output goes to
/// `target/poly-<name>-boot.log`.
#[allow(clippy::too_many_arguments)]
fn disk_boot(
    kernel: &Path,
    syms: &Arc<Symbols>,
    image: &Path,
    initrd: &Path,
    qemu: Option<&Path>,
    options: &str,
    name: &str,
    steps: impl FnOnce(&mut crate::busybox::Session, &Path, &Path) -> super::Result,
) -> super::Result {
    let target = super::root().join("target");
    let vnc = std::env::temp_dir().join(format!("poly-{name}-vnc-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&vnc);
    let append = format!("{} {}", crate::rootdisk::boot_args(), std::env::var("POLY_VIRGL_APPEND").unwrap_or_default());
    let mut cmd = super::qemu(kernel, crate::rootdisk::MEMORY, append.trim_end(), super::Devices::None, "stdio");
    cmd.args(crate::rootdisk::drive_args(image)).arg("-initrd").arg(initrd).args(gpu_args(MODE, &vnc, options));
    if let Some(qemu) = qemu {
        // The same arguments to another QEMU, with its own firmware.
        let args: Vec<std::ffi::OsString> = cmd.get_args().map(|a| a.to_os_string()).collect();
        let share = qemu.parent().and_then(Path::parent).map(|p| p.join("share/qemu")).unwrap_or_default();
        cmd = std::process::Command::new(qemu);
        cmd.arg("-L").arg(share).args(args);
    }
    let mut s = crate::busybox::Session::spawn(cmd, syms)?;
    s.set_timeouts(STEP_TIMEOUT, SESSION_TIMEOUT);
    let result = steps(&mut s, &vnc, &target);
    let _ = std::fs::remove_file(&vnc);
    let log = target.join(format!("poly-{name}-boot.log"));
    let _ = std::fs::write(&log, s.text());
    if result.is_err() {
        eprintln!("==> the session's output is in {}", log.display());
    }
    result
}

/// The Venus steps: the device's capsets, `vulkaninfo --summary`,
/// `vk-clear Venus`.
fn venus_session(s: &mut crate::busybox::Session) -> super::Result {
    let mut unimplemented = BTreeSet::new();
    let mut failed: Vec<String> = Vec::new();
    let boot = s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    if !boot.contains("drm: virtio-gpu 3D: capsets") || !boot.contains("venus v") {
        failed.push("the kernel did not list a Venus capset".into());
    }
    s.run("stty cols 250 rows 50")?;
    let out = split_kernel_lines(&s.run("vulkaninfo --summary 2>&1; echo \"status=$?\"")?, &mut unimplemented);
    let devices: Vec<String> =
        out.lines().filter(|l| l.trim_start().starts_with("deviceName")).map(|l| l.trim().to_string()).collect();
    if devices.iter().any(|d| d.contains("Virtio-GPU Venus")) {
        eprintln!("==> vulkaninfo: {}", devices.join("; "));
    } else {
        failed.push(format!("vulkaninfo --summary listed no Venus device:\n{}", tail(&out)));
    }
    let out = split_kernel_lines(&s.run("vk-clear Venus; echo \"status=$?\"")?, &mut unimplemented);
    if out.contains("vk-clear: ok") && out.contains("status=0") {
        let line = out.lines().find(|l| l.contains("wrong")).unwrap_or_default();
        eprintln!("==> {}", line.trim());
    } else {
        failed.push(format!("vk-clear Venus failed:\n{}", tail(&out)));
    }
    if !unimplemented.is_empty() {
        failed.push(format!(
            "system calls not implemented: {}",
            unimplemented.into_iter().collect::<Vec<_>>().join("; ")
        ));
    }
    if failed.is_empty() { Ok(()) } else { Err(failed.join("\n")) }
}

fn session(s: &mut crate::busybox::Session, vnc: &Path, target: &Path, version: &str) -> super::Result {
    let (w, h) = MODE;
    let boot_bg = crate::screen::boot_screen(w, h, version).pixel(0, 0);
    let mut unimplemented = BTreeSet::new();
    let mut failed: Vec<String> = Vec::new();
    s.expect("rootdisk: /dev/vda mounted on /mnt/root")?;
    s.prompt()?;
    s.run("stty cols 250 rows 50")?;
    let mut screen = Vnc::connect(vnc, Duration::from_secs(10))?;
    let shot = |screen: &mut Vnc, name: &str| -> super::Result<Image> {
        let img = screen.screenshot()?;
        img.write_ppm(&target.join(format!("poly-virgl-{name}.ppm")))?;
        Ok(img)
    };
    // POLY_VIRGL_RUN=<command>: run it first and print what it printed
    // (for debugging a program on the disk).
    if let Ok(extra) = std::env::var("POLY_VIRGL_RUN")
        && !extra.is_empty()
    {
        let out = s.run(&format!("{extra}; echo \"status=$?\""))?;
        eprintln!("==> {extra}:\n{out}");
    }

    // 1. EGL on the render node with Mesa's virgl driver, for the GBM and
    //    surfaceless platforms.
    for platform in ["gbm", "surfaceless"] {
        let out =
            split_kernel_lines(&s.run(&format!("eglinfo -B -p {platform}; echo \"status=$?\""))?, &mut unimplemented);
        let renderer = out
            .lines()
            .find(|l| {
                l.trim_start().starts_with("OpenGL ES profile renderer:")
                    || l.trim_start().starts_with("OpenGL core profile renderer:")
                    || l.trim_start().starts_with("OpenGL compatibility profile renderer:")
            })
            .map(|l| l.split_once(':').map_or("", |(_, r)| r).trim().to_string());
        match renderer {
            Some(r) if r.starts_with("virgl") => eprintln!("==> eglinfo -p {platform}: renderer {r:?}"),
            _ => failed.push(format!("eglinfo -B -p {platform} did not report virgl:\n{}", tail(&out))),
        }
    }

    // 2. GLES rendering read back and checked pixel for pixel.
    let out = split_kernel_lines(&s.run("gl-readback; echo \"status=$?\"")?, &mut unimplemented);
    let renderer = out.lines().find(|l| l.contains("renderer \"")).unwrap_or_default().to_string();
    if out.contains("gl-readback: ok") && out.contains("status=0") && renderer.contains("\"virgl") {
        eprintln!("==> gl-readback: the framebuffer read back is exactly what was drawn ({})", renderer.trim());
    } else {
        failed.push(format!("gl-readback failed:\n{}", tail(&out)));
    }

    // 3. kmscube with hardware OpenGL: virgl renders on the host, kmscube
    //    shows its GBM buffers (3D resources) with legacy page flips.
    let started = Instant::now();
    s.send(&format!("kmscube -D /dev/dri/card0 -c {FRAMES}; echo \"status=$?\"\r"))?;
    let renderer = s.expect("renderer: \"").and_then(|_| s.expect("\n"));
    match renderer {
        Ok(r) => {
            let r = r.trim().trim_end_matches('"').to_string();
            let mut seen = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                let img = shot(&mut screen, "kmscube-0")?;
                if cube_frame(&img, boot_bg).is_ok() || Instant::now() > deadline {
                    seen.push(img);
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            std::thread::sleep(Duration::from_millis(700));
            seen.push(shot(&mut screen, "kmscube-1")?);
            let out = split_kernel_lines(&s.prompt()?, &mut unimplemented);
            let rate = out.lines().rfind(|l| l.starts_with("Rendered ")).unwrap_or_default().to_string();
            eprintln!("==> kmscube ({r}): {rate}; {:.0?} in all", started.elapsed());
            if !r.starts_with("virgl") {
                failed.push(format!("kmscube rendered with {r:?}, not virgl"));
            }
            match (cube_frame(&seen[0], boot_bg), cube_frame(&seen[1], boot_bg)) {
                (Ok((bg, n)), Ok(_)) => {
                    let moved = changed(&seen[0], &seen[1]);
                    eprintln!(
                        "==> kmscube: a cube of {n} pixels on {bg:?}; {moved} pixels changed between two screenshots"
                    );
                    if moved == 0 {
                        failed.push("kmscube: the cube did not move between two screenshots".into());
                    }
                }
                (a, b) => failed.push(format!("kmscube: screenshots: {:?} / {:?}", a.err(), b.err())),
            }
            if !out.contains("status=0") {
                failed.push(format!("kmscube failed:\n{}", tail(&out)));
            }
            // The console has the display back.
            std::thread::sleep(Duration::from_millis(500));
            let img = shot(&mut screen, "after")?;
            if dominant(&img) != boot_bg {
                failed.push("the console did not get the display back after kmscube".into());
            }
        }
        Err(e) => {
            failed.push(format!("kmscube: {e}"));
            let out = s.prompt().unwrap_or_default();
            failed.push(tail(&out));
        }
    }

    // 4. Vulkan on this device (no Venus capset): reported, not required.
    let out = split_kernel_lines(&s.run("vulkaninfo --summary 2>&1; echo \"status=$?\"")?, &mut unimplemented);
    let devices: Vec<&str> = out.lines().filter(|l| l.trim_start().starts_with("deviceName")).collect();
    if devices.iter().any(|d| d.contains("Virtio-GPU Venus")) {
        eprintln!("==> vulkaninfo: Venus: {}", devices.join("; "));
    } else {
        eprintln!(
            "==> vulkaninfo: no Venus device (this virtio GPU has no Venus capset); devices: {}",
            if devices.is_empty() { "none".to_string() } else { devices.join("; ") }
        );
    }

    if !unimplemented.is_empty() {
        failed.push(format!(
            "system calls not implemented: {}",
            unimplemented.into_iter().collect::<Vec<_>>().join("; ")
        ));
    }
    if failed.is_empty() { Ok(()) } else { Err(failed.join("\n")) }
}
