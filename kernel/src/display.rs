//! The display: a back buffer in RAM, drawn on with `poly-gfx`, and an
//! output that puts it on the screen: QEMU's standard VGA or
//! `bochs-display` (a linear framebuffer,
//! [ADR 0019](../../docs/adr/0019-display-framebuffer-and-drawing.md)), or a
//! virtio GPU (a 2D resource in guest memory,
//! [ADR 0102](../../docs/adr/0102-virtio-gpu-2d-display.md)).
//!
//! # The interface
//!
//! Whatever the device, drawing works the same: [`with_canvas`] draws on
//! the back buffer, [`flush`] puts a rectangle of it on the screen, [`draw`]
//! does both under one lock, [`info`] says how big the screen is and what
//! device it is on. The screen terminal (`terminal.rs`) and the boot screen
//! use nothing else. What differs is the output behind the back buffer: a
//! framebuffer mapping that `flush` copies into, or a virtio-gpu resource
//! whose guest memory *is* the back buffer, which `flush` transfers and
//! flushes.
//!
//! # Which device
//!
//! A virtio GPU wins when there is one (`-device virtio-gpu-pci`); the
//! Bochs VBE device is the fallback, also when the virtio GPU cannot be
//! brought up. `poly.display=off` skips both and leaves the devices as the
//! firmware set them up.
//!
//! # Bring-up on Bochs VBE ([`init`])
//!
//! 1. Find PCI function 1234:1111 ([`poly_bochs_vbe::PCI_VENDOR`]) in the
//!    list `pci::init` built. Its BAR 0 is the framebuffer (16 MiB on
//!    QEMU), BAR 2 a 4 KiB register block with the DISPI registers at
//!    0x500 (QEMU `docs/specs/standard-vga.rst`). Without BAR 2, a VGA-class
//!    device is driven through I/O ports 0x1ce / 0x1cf instead
//!    (`arch::dispi`).
//! 2. Turn on memory (and I/O) decoding, check the interface ID, read the
//!    largest mode, and set the mode: `poly.display=<width>x<height>` from
//!    the kernel command line, else [`DEFAULT_MODE`], at 32 bits per pixel
//!    (XRGB8888, little-endian, which QEMU's extended register makes sure
//!    of). A mode the device refuses falls back to the default; the
//!    registers are read back to check what it shows.
//! 3. Map all of BAR 0 write-combining (`mm::map_mmio_wc`; uncached if the
//!    CPU has no PAT). This is the BAR's only mapping: [`pci::Device::map_bar`]
//!    is never called for it, so no uncached alias exists.
//! 4. Take a back buffer of `width * height * 4` bytes from the frame
//!    allocator (`frames::allocate_contiguous`, as it may exceed the 4 MiB
//!    a heap allocation can have), zeroed, with lines exactly `width`
//!    pixels long.
//! 5. Draw the boot screen (`poly_gfx::boot_screen`) and flush it.
//!
//! # Bring-up on a virtio GPU
//!
//! `virtio_gpu::probe` initialises the device; then the size is planned
//! (`poly_virtio_gpu::modes`, VIRTIO 1.2 §5.7.6.1, §5.7.6.4):
//! `poly.display=<width>x<height>` if given, else the preferred mode of
//! the first enabled scanout, else [`DEFAULT_MODE`]. The back buffer is
//! allocated as above and becomes the guest memory of resource 1
//! (B8G8R8X8, which is XRGB8888), shown on every enabled scanout. A flush
//! is `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` of the damaged rectangle
//! (§5.7.6.2). Each flush also looks at the device's display-change event
//! (§5.7.4.2): when the planned size changes, a new back buffer and
//! resource of the new size take over (what was drawn is kept, top-left)
//! and the old ones are destroyed and freed.
//!
//! # Drawing
//!
//! Everything draws into the back buffer ([`with_canvas`]), which is
//! ordinary write-back RAM, so reading pixels back (scrolling a terminal
//! with `Canvas::copy_within`, blending) is fast. [`flush`] then puts a
//! rectangle of it on the screen. On Bochs VBE that copies it to the
//! framebuffer, row by row: the framebuffer is only ever written, in long
//! runs, which is what write-combining is good at (and what an uncached
//! mapping is least bad at). [`draw`] does both under one lock. The lock is
//! a spinlock (interrupts off while drawing), so any context may draw, as a
//! panic screen will want to; keep the work done under it to what changed.
//! (That is also why the virtio GPU's commands are polled, not waited for
//! with an interrupt.)

use crate::arch::dispi::DispiPorts;
use crate::boot_info::BootInfo;
use crate::frames;
use crate::pci::{self, Device, MmioRegion, Size};
use crate::sync::SpinLock;
use crate::virtio_gpu;
use crate::{arch, kprintln, mm};
use core::fmt;
use poly_bochs_vbe::{self as vbe, Mode, ModeInfo, Registers};
use poly_gfx::{Canvas, Color, PixelFormat, Rect};
use poly_mem::PAGE_SIZE;
use poly_pci::{Bdf, Mmio, command};
use poly_virtio_gpu::modes::{Plan, Request};
use poly_virtio_gpu::{Framebuffer, MemEntry};

/// The mode without `poly.display=` on the command line (and without a
/// preferred mode from a virtio GPU): 16:10 and small enough for a laptop
/// screen at 100 % scaling. Its back buffer (4000 KiB) fits in one 4 MiB
/// block of frames.
pub const DEFAULT_MODE: Mode = Mode::new(1280, 800);

/// Kernel argument choosing the mode: `poly.display=<width>x<height>`, or
/// `poly.display=off` for no display at all.
pub const MODE_ARG: &str = "poly.display";

/// Kernel flag saying the boot test runs with a display; the display tests
/// then require one (`cargo xtask` always passes it: QEMU's `q35` machine
/// comes with a standard VGA unless told otherwise).
pub const TEST_DISPLAY_FLAG: &str = "poly.test.display";

/// Kernel flag saying the boot test attached a virtio GPU; the display
/// tests then require the display to be on it.
pub const TEST_VIRTIO_GPU_FLAG: &str = "poly.test.virtio_gpu";

/// The pixel format every mode set here uses.
const FORMAT: PixelFormat = PixelFormat::Xrgb8888;

/// The largest framebuffer a virtio GPU gets (a bigger preferred mode
/// falls back to [`DEFAULT_MODE`]): 2560x1600, a 16 MiB back buffer.
const MAX_GPU_PIXELS: u64 = 2560 * 1600;

/// Failed virtio-gpu updates logged before the display goes quiet.
const LOGGED_FAILURES: u32 = 4;

/// The device the display is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// QEMU's standard VGA or `bochs-display`: a linear framebuffer.
    BochsVbe,
    /// A virtio GPU: a 2D resource in guest memory.
    VirtioGpu,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DeviceKind::BochsVbe => "Bochs VBE",
            DeviceKind::VirtioGpu => "virtio-gpu",
        })
    }
}

/// What the display shows and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    /// Bytes per framebuffer line. (The back buffer's lines are exactly
    /// `width` pixels long.)
    pub stride: usize,
    pub format: PixelFormat,
    /// Physical address and size of the framebuffer: BAR 0 on Bochs VBE,
    /// the back buffer (the resource's guest memory) on a virtio GPU.
    pub framebuffer: u64,
    pub vram: u64,
    /// Whether the framebuffer is mapped write-combining (else uncached;
    /// false on a virtio GPU, whose framebuffer is ordinary RAM).
    pub write_combining: bool,
    pub device: DeviceKind,
}

impl DisplayInfo {
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
}

/// The DISPI registers, wherever the device has them.
#[derive(Clone, Copy, Debug)]
enum Regs {
    /// The MMIO BAR (BAR 2), all of it.
    Mmio(MmioRegion),
    Ports(DispiPorts),
}

impl Registers for Regs {
    fn read(&self, index: u16) -> u16 {
        match self {
            Regs::Mmio(m) => m.read16(vbe::MMIO_DISPI + 2 * index as usize),
            Regs::Ports(p) => p.read(index),
        }
    }

    fn write(&self, index: u16, value: u16) {
        match self {
            Regs::Mmio(m) => m.write16(vbe::MMIO_DISPI + 2 * index as usize, value),
            Regs::Ports(p) => p.write(index, value),
        }
    }
}

/// Why the display could not be brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    /// BAR 0 is missing, not memory, or unassigned.
    NoFramebuffer,
    /// No MMIO BAR, and not a VGA device with the I/O ports.
    NoRegisters,
    Vbe(vbe::Error),
    /// Neither a write-combining nor an uncached mapping could be made.
    MapFailed,
    /// No physically contiguous memory for the back buffer.
    NoBackBuffer(u64),
    Gpu(poly_virtio_gpu::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoFramebuffer => write!(f, "BAR 0 (the framebuffer) is missing or unassigned"),
            Error::NoRegisters => write!(f, "no MMIO register BAR and no VGA I/O ports"),
            Error::Vbe(e) => write!(f, "{e}"),
            Error::MapFailed => write!(f, "cannot map the framebuffer"),
            Error::NoBackBuffer(bytes) => write!(f, "no {} of contiguous memory for the back buffer", Size(*bytes)),
            Error::Gpu(e) => write!(f, "{e:?}"),
        }
    }
}

/// Where the back buffer goes on [`flush`].
enum Output {
    Vbe(VbeOutput),
    /// Boxed: the driver and the plan are much larger than the VBE side.
    VirtioGpu(alloc::boxed::Box<GpuOutput>),
}

struct VbeOutput {
    /// The mode asked for, when the device refused it and got
    /// [`DEFAULT_MODE`] instead.
    refused: Option<Mode>,
    regs: Regs,
    /// The framebuffer, as mapped (all of BAR 0 behind it).
    front: Canvas<'static>,
}

struct GpuOutput {
    gpu: virtio_gpu::Driver,
    /// The resource the scanouts show; its guest memory is the back buffer.
    fb: Framebuffer,
    /// Which scanouts show it, and where its size came from.
    plan: Plan,
    /// What the size is planned from (the command line's size, limits).
    request: Request,
    /// The rectangle of the last update sent (for the tests).
    last_update: Option<poly_virtio_gpu::Rect>,
    /// Failed updates, to log only the first few.
    failures: u32,
}

/// A zeroed back buffer from the frame allocator: its frames and a canvas
/// on it.
struct BackBuffer {
    phys: u64,
    frames: u64,
    canvas: Canvas<'static>,
}

struct Display {
    info: DisplayInfo,
    bdf: Bdf,
    /// The back buffer in RAM, where all drawing happens.
    back: Canvas<'static>,
    /// Its frames (`frames::allocate_contiguous`).
    back_phys: u64,
    back_frames: u64,
    output: Output,
}

impl Display {
    /// Put `r` (clipped) of the back buffer on the screen.
    fn flush(&mut self, r: Rect) {
        if let Output::VirtioGpu(_) = self.output {
            self.poll_gpu_events();
        }
        let r = r.intersect(&self.info.bounds());
        if r.is_empty() {
            return;
        }
        match &mut self.output {
            Output::Vbe(v) => v.front.blit(r.x, r.y, &self.back, r),
            Output::VirtioGpu(g) => {
                if g.gpu.is_broken() {
                    return;
                }
                // Clipped to the screen above, so the corner is not negative.
                let rect = poly_virtio_gpu::Rect::new(r.x as u32, r.y as u32, r.width, r.height);
                g.last_update = Some(rect);
                if let Err(e) = g.gpu.update(&g.fb, rect) {
                    g.failures += 1;
                    if g.failures <= LOGGED_FAILURES {
                        kprintln!("display: virtio-gpu: update of {rect:?} failed: {e:?}");
                    }
                }
            }
        }
    }

    /// Take a virtio GPU's display-change event, if one is pending, and
    /// follow the new display configuration (§5.7.4.2).
    fn poll_gpu_events(&mut self) {
        let Output::VirtioGpu(g) = &mut self.output else { return };
        if g.gpu.is_broken() || g.gpu.take_events() & poly_virtio_gpu::event::DISPLAY == 0 {
            return;
        }
        let size = g.request.size;
        match self.replan(size) {
            Ok(()) => kprintln!(
                "display: virtio-gpu: the display configuration changed; now {}x{} on scanout(s) {}",
                self.info.width,
                self.info.height,
                ScanoutList(self.gpu_plan()),
            ),
            Err(e) => kprintln!("display: virtio-gpu: the display configuration changed; cannot follow: {e}"),
        }
    }

    fn gpu_plan(&self) -> Option<&Plan> {
        match &self.output {
            Output::VirtioGpu(g) => Some(&g.plan),
            Output::Vbe(_) => None,
        }
    }

    /// Plan the virtio GPU's display again from its display info, with
    /// `size` (if any) wanted, and switch to that plan: with the same size,
    /// just set the scanouts; with another, a new back buffer and resource
    /// (keeping what was drawn, top-left), the whole of it shown, the old
    /// resource destroyed and its memory freed. Nothing to do on Bochs VBE.
    fn replan(&mut self, size: Option<(u32, u32)>) -> Result<(), Error> {
        let Output::VirtioGpu(g) = &mut self.output else { return Ok(()) };
        let info = g.gpu.display_info().map_err(Error::Gpu)?;
        let plan = Plan::new(&info, g.gpu.num_scanouts(), &Request { size, ..g.request });
        if !plan.resized(&g.plan) {
            g.gpu.set_scanouts(&g.fb, &plan, Some(&g.plan)).map_err(Error::Gpu)?;
            g.plan = plan;
            return Ok(());
        }
        let mut back = back_buffer(plan.width, plan.height)?;
        back.canvas.blit(0, 0, &self.back, self.back.bounds());
        let id = if g.fb.resource_id == 1 { 2 } else { 1 };
        let fb = match g.gpu.show(id, &plan, Some(&g.plan), &backing(&back)) {
            Ok(fb) => fb,
            Err(e) => {
                // A device that was given up on may still hold the memory.
                if !g.gpu.is_broken() {
                    free_back_buffer(back.phys, back.frames);
                }
                return Err(Error::Gpu(e));
            }
        };
        let old = core::mem::replace(&mut g.fb, fb);
        g.plan = plan;
        // No scanout shows the old resource now (`show` moved or turned off
        // every one); once the device has let go of its memory, free it.
        let destroyed = g.gpu.destroy(&old);
        let (old_phys, old_frames) = (self.back_phys, self.back_frames);
        self.back = back.canvas;
        (self.back_phys, self.back_frames) = (back.phys, back.frames);
        match destroyed {
            Ok(()) => free_back_buffer(old_phys, old_frames),
            Err(e) => kprintln!(
                "display: virtio-gpu: resource {} not destroyed ({e:?}); its memory stays with the device",
                old.resource_id
            ),
        }
        let (width, height) = (plan.width, plan.height);
        self.info = DisplayInfo {
            width,
            height,
            stride: width as usize * FORMAT.bytes_per_pixel(),
            framebuffer: self.back_phys,
            vram: self.back_frames * PAGE_SIZE,
            ..self.info
        };
        self.flush(self.info.bounds());
        Ok(())
    }
}

/// The scanouts of a plan, for messages: "0, 2".
struct ScanoutList<'a>(Option<&'a Plan>);

impl fmt::Display for ScanoutList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(plan) = self.0 else { return f.write_str("-") };
        for (n, (i, _)) in plan.active().enumerate() {
            if n > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{i}")?;
        }
        Ok(())
    }
}

/// A zeroed back buffer of `width` × `height` from the frame allocator,
/// lines exactly `width` pixels long.
fn back_buffer(width: u32, height: u32) -> Result<BackBuffer, Error> {
    let stride = width as usize * FORMAT.bytes_per_pixel();
    let len = stride * height as usize;
    let frames = (len as u64).div_ceil(PAGE_SIZE);
    let phys = frames::allocate_contiguous(frames).ok_or(Error::NoBackBuffer(len as u64))?;
    let Some(virt) = arch::phys_range_to_virt(phys, frames * PAGE_SIZE) else {
        free_back_buffer(phys, frames);
        return Err(Error::NoBackBuffer(len as u64));
    };
    // SAFETY: the frames were just allocated for the display, which keeps
    // them until it frees them, after dropping this canvas (or for the
    // kernel's lifetime); the direct map makes them writable at `virt`,
    // physically contiguous, so contiguous here too. They are zeroed before
    // the slice exists.
    let bytes = unsafe {
        core::ptr::write_bytes(virt as *mut u8, 0, len);
        core::slice::from_raw_parts_mut(virt as *mut u8, len)
    };
    let canvas = Canvas::new(bytes, width, height, stride, FORMAT).map_err(|_| Error::MapFailed)?;
    Ok(BackBuffer { phys, frames, canvas })
}

fn free_back_buffer(phys: u64, frames: u64) {
    if let Err(e) = frames::free_contiguous(phys, frames) {
        kprintln!("display: cannot free the back buffer at {phys:#x}: {e:?}");
    }
}

/// The scatter list of a back buffer: one piece, as it is contiguous.
fn backing(b: &BackBuffer) -> [MemEntry; 1] {
    [MemEntry { addr: b.phys, length: b.canvas.as_bytes().len() as u32 }]
}

static DISPLAY: SpinLock<Option<Display>> = SpinLock::new(None);

/// The display, if [`init`] brought one up.
pub fn info() -> Option<DisplayInfo> {
    DISPLAY.lock().as_ref().map(|d| d.info)
}

/// Run `f` on the back buffer. Nothing reaches the screen until it is
/// [`flush`]ed. `None` without a display.
pub fn with_canvas<R>(f: impl FnOnce(&mut Canvas<'_>) -> R) -> Option<R> {
    let mut guard = DISPLAY.lock();
    let d = guard.as_mut()?;
    Some(f(&mut d.back.reborrow()))
}

/// Draw with `f` on the back buffer and flush the rectangle it returns
/// (`TextGrid::render` returns just that), under one lock. Returns that
/// rectangle.
pub fn draw(f: impl FnOnce(&mut Canvas<'_>) -> Option<Rect>) -> Option<Rect> {
    let mut guard = DISPLAY.lock();
    let d = guard.as_mut()?;
    let r = f(&mut d.back.reborrow())?;
    d.flush(r);
    Some(r)
}

/// Put rectangle `r` of the back buffer on the screen.
pub fn flush(r: Rect) {
    if let Some(d) = DISPLAY.lock().as_mut() {
        d.flush(r);
    }
}

/// Put the whole back buffer on the screen.
pub fn flush_all() {
    if let Some(d) = DISPLAY.lock().as_mut() {
        d.flush(d.info.bounds());
    }
}

/// QEMU's standard VGA, `secondary-vga` or `bochs-display`, the first one.
fn find() -> Option<&'static Device> {
    pci::devices().iter().find(|d| (d.header.vendor_id, d.header.device_id) == (vbe::PCI_VENDOR, vbe::PCI_DEVICE))
}

/// Whether the function is a VGA-compatible controller (class 03.00),
/// which has the legacy VGA ports, as the standard VGA has and
/// `bochs-display` (class 03.80) has not.
fn is_vga(d: &Device) -> bool {
    (d.header.class.class, d.header.class.subclass) == (0x03, 0x00)
}

/// Bring up the display (see the module documentation). Call once, after
/// `pci::init`.
pub fn init(boot: &BootInfo) {
    assert!(DISPLAY.lock().is_none(), "display::init called twice");
    let arg = boot.arg(MODE_ARG);
    if arg == Some("off") {
        kprintln!("display: off ({MODE_ARG}=off)");
        return;
    }
    let asked = match arg.map(|a| (a, Mode::parse(a))) {
        None => None,
        Some((_, Some(mode))) => Some(mode),
        Some((a, None)) => {
            kprintln!("display: {MODE_ARG}={a} is not <width>x<height>; using {}", DEFAULT_MODE);
            None
        }
    };
    if let Some(gpu) = virtio_gpu::probe() {
        let bdf = gpu.bdf;
        match bring_up_gpu(gpu, asked) {
            Ok(display) => return finish(display),
            Err(e) => kprintln!("display: virtio-gpu {bdf}: {e}; trying Bochs VBE"),
        }
    }
    let mode = asked.unwrap_or(DEFAULT_MODE);
    let Some(dev) = find() else {
        kprintln!("display: no virtio GPU, QEMU standard VGA or bochs-display (PCI 1234:1111)");
        return;
    };
    let display = match bring_up(dev, mode) {
        Ok(d) => d,
        Err(e @ Error::Vbe(vbe::Error::Unsupported { .. } | vbe::Error::TooLarge { .. })) if mode != DEFAULT_MODE => {
            kprintln!("display: {}: {mode}: {e}; trying {DEFAULT_MODE}", dev.bdf());
            match bring_up(dev, DEFAULT_MODE) {
                Ok(mut d) => {
                    if let Output::Vbe(v) = &mut d.output {
                        v.refused = Some(mode);
                    }
                    d
                }
                Err(e) => return kprintln!("display: {}: {e}", dev.bdf()),
            }
        }
        Err(e) => return kprintln!("display: {}: {e}", dev.bdf()),
    };
    finish(display);
}

/// Draw the boot screen on a display just brought up, show it, say what
/// it is, and make it the display.
fn finish(mut display: Display) {
    let i = display.info;
    poly_gfx::boot_screen::draw(&mut display.back, env!("CARGO_PKG_VERSION"));
    display.flush(i.bounds());
    match &display.output {
        Output::Vbe(v) => kprintln!(
            "display: {} {}x{}x32, {} bytes per line; framebuffer {:#x} ({}) mapped {}; registers through {}",
            display.bdf,
            i.width,
            i.height,
            i.stride,
            i.framebuffer,
            Size(i.vram),
            if i.write_combining { "write-combining" } else { "uncached" },
            match v.regs {
                Regs::Mmio(_) => "MMIO (BAR 2)",
                Regs::Ports(_) => "I/O ports 0x1ce/0x1cf",
            },
        ),
        Output::VirtioGpu(g) => kprintln!(
            "display: virtio-gpu {} {}x{}x32 ({:?} size), resource {} on scanout(s) {}; back buffer {:#x} ({}){}",
            display.bdf,
            i.width,
            i.height,
            g.plan.source,
            g.fb.resource_id,
            ScanoutList(Some(&g.plan)),
            i.framebuffer,
            Size(i.vram),
            if g.failures > 0 { "; showing it failed" } else { "" },
        ),
    }
    *DISPLAY.lock() = Some(display);
}

/// Plan the size on a virtio GPU, take a back buffer of it, and show it as
/// resource 1 on the planned scanouts.
fn bring_up_gpu(gpu: virtio_gpu::Gpu, asked: Option<Mode>) -> Result<Display, Error> {
    let virtio_gpu::Gpu { bdf, mut driver } = gpu;
    let request = Request {
        size: asked.map(|m| (m.width as u32, m.height as u32)),
        fallback: (DEFAULT_MODE.width as u32, DEFAULT_MODE.height as u32),
        max_pixels: MAX_GPU_PIXELS,
    };
    let info = driver.display_info().map_err(Error::Gpu)?;
    let mut plan = Plan::new(&info, driver.num_scanouts(), &request);
    let back = match back_buffer(plan.width, plan.height) {
        Ok(b) => b,
        // Not enough memory for the planned size: the default one.
        Err(Error::NoBackBuffer(_)) if (plan.width, plan.height) != request.fallback => {
            plan = Plan::new(&info, driver.num_scanouts(), &Request { size: Some(request.fallback), ..request });
            back_buffer(plan.width, plan.height)?
        }
        Err(e) => return Err(e),
    };
    let fb = match driver.show(1, &plan, None, &backing(&back)) {
        Ok(fb) => fb,
        Err(e) => {
            // The device lets go of the memory once reset.
            if driver.reset() {
                free_back_buffer(back.phys, back.frames);
            }
            return Err(Error::Gpu(e));
        }
    };
    let (width, height) = (plan.width, plan.height);
    let info = DisplayInfo {
        width,
        height,
        stride: fb.stride as usize,
        format: FORMAT,
        framebuffer: back.phys,
        vram: back.frames * PAGE_SIZE,
        write_combining: false,
        device: DeviceKind::VirtioGpu,
    };
    let output = GpuOutput { gpu: driver, fb, plan, request, last_update: None, failures: 0 };
    Ok(Display {
        info,
        bdf,
        back: back.canvas,
        back_phys: back.phys,
        back_frames: back.frames,
        output: Output::VirtioGpu(alloc::boxed::Box::new(output)),
    })
}

fn bring_up(dev: &Device, mode: Mode) -> Result<Display, Error> {
    let (cfg, bdf) = (dev.config(), dev.bdf());
    let fb = dev.bars[0].filter(|b| b.is_memory() && b.address != 0).ok_or(Error::NoFramebuffer)?;
    let mmio =
        dev.bars[2].filter(|b| b.is_memory() && b.size as usize > vbe::MMIO_DISPI).and_then(|_| dev.map_bar(2).ok());
    let regs = match mmio {
        Some(m) => Regs::Mmio(m),
        None if is_vga(dev) => Regs::Ports(DispiPorts),
        None => return Err(Error::NoRegisters),
    };
    let io = if let Regs::Ports(_) = regs { command::IO_SPACE } else { 0 };
    poly_pci::enable(cfg, bdf, command::MEMORY_SPACE | io);

    let caps = vbe::probe(&regs).map_err(Error::Vbe)?;
    if let Some(m) = mmio {
        // QEMU's extended registers, when the block is big enough to hold
        // the byte-order register: little-endian pixels.
        let ext = m.read32(vbe::MMIO_QEMU_EXT);
        if ext != u32::MAX && ext as usize >= vbe::QEMU_EXT_ENDIANNESS + 4 {
            m.write32(vbe::MMIO_QEMU_EXT + vbe::QEMU_EXT_ENDIANNESS, vbe::QEMU_EXT_LITTLE_ENDIAN);
        }
    }
    let set = vbe::set_mode(&regs, &caps, mode, fb.size).map_err(Error::Vbe)?;
    if let (Some(m), true) = (mmio, is_vga(dev)) {
        unblank(m);
    }

    let (virt, write_combining) = match mm::map_mmio_wc(fb.address, fb.size) {
        Some(v) => (v, true),
        None => (mm::map_mmio(fb.address, fb.size).ok_or(Error::MapFailed)?, false),
    };
    let (width, height) = (set.mode.width as u32, set.mode.height as u32);
    let stride = set.stride();
    let front_len = Canvas::required_len(width, height, stride, FORMAT).ok_or(Error::MapFailed)?;
    // SAFETY: `virt` maps all of BAR 0 (`fb.size` bytes), and `set_mode`
    // checked that `height` lines of `stride` bytes fit in it. The mapping
    // is never removed, and only this display writes the framebuffer
    // (through this canvas, under DISPLAY's lock). Framebuffer memory has
    // no side effects on access, so a byte slice over it behaves like RAM:
    // the device only reads it.
    let front = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, front_len) };
    let front = Canvas::new(front, width, height, stride, FORMAT).map_err(|_| Error::MapFailed)?;

    let back = back_buffer(width, height)?;
    let info = DisplayInfo {
        width,
        height,
        stride,
        format: FORMAT,
        framebuffer: fb.address,
        vram: fb.size,
        write_combining,
        device: DeviceKind::BochsVbe,
    };
    Ok(Display {
        info,
        bdf,
        back: back.canvas,
        back_phys: back.phys,
        back_frames: back.frames,
        output: Output::Vbe(VbeOutput { refused: None, regs, front }),
    })
}

/// Make sure a VGA device's attribute controller passes the picture
/// through. The attribute address register (port 0x3c0) has a "palette
/// address source" bit (0x20) which, when clear, blanks the screen.
/// SeaBIOS, which QEMU runs before a PVH boot, leaves it set, but other
/// firmware (a UEFI boot, later) need not. Reading input status 1 (port
/// 0x3da) first makes the next write to 0x3c0 an address write. Both ports
/// appear in the MMIO BAR at 0x400 + (port - 0x3c0).
fn unblank(m: MmioRegion) {
    let port = |p: usize| vbe::MMIO_VGA_PORTS + (p - 0x3c0);
    let _ = m.read8(port(0x3da));
    m.write8(port(0x3c0), 0x20);
}

// --- Tests --------------------------------------------------------------------

/// The size the command line asked for, if any.
fn asked_mode(env: &crate::ktest::TestEnv) -> Option<Mode> {
    env.boot.arg(MODE_ARG).and_then(Mode::parse)
}

/// The mode the boot asked for on Bochs VBE.
fn expected_mode(env: &crate::ktest::TestEnv) -> Mode {
    asked_mode(env).unwrap_or(DEFAULT_MODE)
}

/// The display, or why the test cannot run: an error if the boot said
/// there is one (on a virtio GPU, if it said so).
fn require(env: &crate::ktest::TestEnv) -> Result<Option<DisplayInfo>, &'static str> {
    match info() {
        Some(i) if env.boot.has_flag(TEST_VIRTIO_GPU_FLAG) && i.device != DeviceKind::VirtioGpu => {
            Err("poly.test.virtio_gpu is set but the display is not on the virtio GPU")
        }
        Some(i) => Ok(Some(i)),
        None if env.boot.has_flag(TEST_DISPLAY_FLAG) => Err("poly.test.display is set but no display came up"),
        None => Ok(None),
    }
}

/// The pixels of `r` the screen shows, where the kernel can read them back
/// (a Bochs VBE framebuffer; a virtio GPU's host resource cannot be read).
fn screen(d: &Display, r: Rect) -> Option<alloc::vec::Vec<Option<Color>>> {
    match &d.output {
        Output::Vbe(v) => Some(save(&v.front, r)),
        Output::VirtioGpu(_) => None,
    }
}

/// The virtio GPU's last update and update count; `None` on Bochs VBE.
fn gpu_updates(d: &Display) -> Option<(Option<poly_virtio_gpu::Rect>, u64)> {
    match &d.output {
        Output::VirtioGpu(g) => Some((g.last_update, g.gpu.stats().updates)),
        Output::Vbe(_) => None,
    }
}

fn gpu_rect(r: Rect) -> poly_virtio_gpu::Rect {
    poly_virtio_gpu::Rect::new(r.x as u32, r.y as u32, r.width, r.height)
}

crate::kernel_test! {
    /// The device is found, and its registers show the mode that was asked
    /// for (or the default, if the device refused that), the linear
    /// framebuffer on, starting at the framebuffer's start. (On a virtio
    /// GPU, `virtio_gpu_mode_set` checks the mode.)
    fn device_found_and_mode_set(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let guard = DISPLAY.lock();
        let d = guard.as_ref().ok_or("the display went away")?;
        let Output::Vbe(v) = &d.output else { return Ok(()) };
        let want = match v.refused {
            Some(refused) if refused == expected_mode(env) => DEFAULT_MODE,
            Some(_) => return Err("the display fell back from a mode the command line did not ask for"),
            None => expected_mode(env),
        };
        if (i.width, i.height) != (want.width as u32, want.height as u32) {
            return Err("the display does not have the mode the command line asked for");
        }
        let id = v.regs.read(vbe::index::ID);
        let shown: Option<ModeInfo> = vbe::current(&v.regs);
        let ok = shown.is_some_and(|s| s.mode == want && s.stride() == i.stride && s.offset() == 0);
        kprintln!("display: ID {id:#06x}, registers say {shown:?}");
        if !(vbe::ID_MIN..=vbe::ID_MAX).contains(&id) || !ok {
            return Err("the DISPI registers do not read back the mode that was set");
        }
        if i.stride < i.width as usize * 4 || i.stride as u64 * i.height as u64 > i.vram {
            return Err("framebuffer lines too short, or the screen does not fit the framebuffer");
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// Through the I/O ports, a VGA device's DISPI registers read the same
    /// as through the MMIO BAR.
    fn ports_and_mmio_agree(env: &TestEnv) -> TestResult {
        if require(env)?.is_none() {
            return Ok(());
        }
        let guard = DISPLAY.lock();
        let d = guard.as_ref().ok_or("the display went away")?;
        let Output::Vbe(v) = &d.output else { return Ok(()) };
        let dev = pci::devices().iter().find(|x| x.bdf() == d.bdf).ok_or("the display's function is gone")?;
        if !is_vga(dev) || !matches!(v.regs, Regs::Mmio(_)) {
            return Ok(());
        }
        // The ports need I/O decoding, which MMIO-driven bring-up left off.
        poly_pci::enable(dev.config(), dev.bdf(), command::IO_SPACE);
        for index in [vbe::index::ID, vbe::index::XRES, vbe::index::YRES, vbe::index::BPP, vbe::index::VIRT_WIDTH] {
            if DispiPorts.read(index) != v.regs.read(index) {
                return Err("a DISPI register reads differently through the I/O ports");
            }
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// The framebuffer is mapped write-combining (when the CPU has a PAT),
    /// and a pixel written through the mapping reads back.
    fn framebuffer_is_mapped_write_combining(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let Output::Vbe(v) = &mut d.output else { return Ok(()) };
        let virt = v.front.as_bytes().as_ptr() as u64;
        let want = if arch::cpu::features().pat { mm::Caching::WriteCombining } else { mm::Caching::Uncached };
        if mm::caching(virt) != Some(want) || i.write_combining != (want == mm::Caching::WriteCombining) {
            return Err("the framebuffer is not mapped with the expected memory type");
        }
        let last = (i.width as i32 - 1, i.height as i32 - 1);
        if mm::caching(virt + (i.stride * (i.height as usize - 1)) as u64) != Some(want) {
            return Err("the end of the framebuffer is not mapped like its start");
        }
        let old = v.front.pixel(last.0, last.1).ok_or("the last pixel is outside the framebuffer")?;
        let probe = Color::rgb(0x12, 0x34, 0x56);
        v.front.set_pixel(last.0, last.1, probe);
        let back = v.front.pixel(last.0, last.1);
        v.front.set_pixel(last.0, last.1, old);
        if back != Some(probe) { Err("a pixel written to the framebuffer did not read back") } else { Ok(()) }
    }
}

/// A test pattern in `r`: a dark fill, an outline, a cross and some text.
fn pattern(c: &mut Canvas<'_>, r: Rect) {
    c.fill_rect(r, Color::rgb(0x10, 0x20, 0x30));
    c.stroke_rect(r, Color::rgb(0xff, 0, 0));
    c.hline(r.x, r.y + r.height as i32 / 2, r.width, Color::rgb(0, 0xff, 0));
    c.vline(r.x + r.width as i32 / 2, r.y, r.height, Color::rgb(0, 0, 0xff));
    c.draw_str(&poly_gfx::SPLEEN_8X16, r.x + 2, r.y + 2, "Ök", Color::WHITE, None);
}

/// The pixels of `r` on `c`, row by row.
fn save(c: &Canvas<'_>, r: Rect) -> alloc::vec::Vec<Option<Color>> {
    (r.y..r.bottom() as i32)
        .flat_map(|y| (r.x..r.right() as i32).map(move |x| (x, y)))
        .map(|(x, y)| c.pixel(x, y))
        .collect()
}

fn restore(c: &mut Canvas<'_>, r: Rect, pixels: &[Option<Color>]) {
    let points = (r.y..r.bottom() as i32).flat_map(|y| (r.x..r.right() as i32).map(move |x| (x, y)));
    for ((x, y), p) in points.zip(pixels) {
        if let Some(p) = p {
            c.set_pixel(x, y, *p);
        }
    }
}

crate::kernel_test! {
    /// Drawing goes to the back buffer and reads back from it, and the
    /// screen does not change until a flush (on a virtio GPU: nothing is
    /// sent to the device).
    fn back_buffer_holds_a_drawing(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let r = Rect::new(i.width as i32 / 2 - 32, i.height as i32 - 80, 64, 48);
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let (saved, before, updates) = (save(&d.back, r), screen(d, r), gpu_updates(d));
        pattern(&mut d.back, r);
        // The same pattern drawn on a scratch canvas is the reference.
        let mut scratch = alloc::vec![0u8; (r.width * r.height * 4) as usize];
        let mut reference = Canvas::new(&mut scratch, r.width, r.height, r.width as usize * 4, FORMAT)
            .map_err(|_| "cannot make a scratch canvas")?;
        pattern(&mut reference, Rect::new(0, 0, r.width, r.height));
        let same = save(&reference, reference.bounds()) == save(&d.back, r);
        let untouched = screen(d, r) == before && gpu_updates(d) == updates;
        restore(&mut d.back, r, &saved);
        if !same {
            return Err("the back buffer does not hold what was drawn");
        }
        if !untouched { Err("drawing on the back buffer changed the screen") } else { Ok(()) }
    }
}

crate::kernel_test! {
    /// A flush copies exactly its rectangle to the screen (on a virtio GPU:
    /// updates exactly that rectangle).
    fn flush_copies_only_its_rectangle(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let outer = Rect::new(8, i.height as i32 - 72, 64, 64);
        let inner = Rect::new(outer.x + 16, outer.y + 16, 32, 32);
        let red = Color::rgb(0xff, 0, 0);
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let (saved, before) = (save(&d.back, outer), screen(d, outer));
        d.back.fill_rect(outer, red);
        d.flush(inner);
        let (after, updated) = (screen(d, outer), gpu_updates(d));
        restore(&mut d.back, outer, &saved);
        d.flush(outer);
        let restored = screen(d, outer) == before;
        if let Some((last, _)) = updated
            && last != Some(gpu_rect(inner))
        {
            return Err("the virtio GPU was not sent an update of exactly the flushed rectangle");
        }
        let (Some(after), Some(before)) = (after, before) else { return Ok(()) };
        let points = (outer.y..outer.bottom() as i32).flat_map(|y| (outer.x..outer.right() as i32).map(move |x| (x, y)));
        for (((x, y), now), was) in points.zip(&after).zip(&before) {
            let want = if inner.contains(x, y) { Some(red) } else { *was };
            if *now != want {
                kprintln!("display: pixel ({x}, {y}) is {now:?} after the flush, expected {want:?}");
                return Err("the flush did not copy exactly its rectangle");
            }
        }
        if !restored { Err("flushing the restored back buffer did not restore the screen") } else { Ok(()) }
    }
}

crate::kernel_test! {
    /// What the drawing interface promises: `draw` flushes what its closure
    /// returns, out-of-range flushes are clipped.
    fn draw_flushes_what_it_drew(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let r = Rect::new(i.width as i32 - 40, 0, 40, 16);
        let saved = with_canvas(|c| save(c, r)).ok_or("no canvas")?;
        let green = Color::rgb(0, 0xff, 0);
        let drawn = draw(|c| {
            c.fill_rect(r, green);
            Some(r)
        });
        let shown = {
            let guard = DISPLAY.lock();
            let d = guard.as_ref().ok_or("the display went away")?;
            match (screen(d, r), gpu_updates(d)) {
                (Some(pixels), _) => pixels.iter().all(|p| *p == Some(green)),
                (None, Some((last, _))) => last == Some(gpu_rect(r)),
                (None, None) => false,
            }
        };
        with_canvas(|c| restore(c, r, &saved));
        let past = Rect::new(i.width as i32 - 40, -100, 1000, 1000);
        flush(past);
        let clipped = {
            let guard = DISPLAY.lock();
            let d = guard.as_ref().ok_or("the display went away")?;
            gpu_updates(d).is_none_or(|(last, _)| last == Some(gpu_rect(past.intersect(&i.bounds()))))
        };
        flush_all();
        if drawn != Some(r) || !shown {
            return Err("draw did not flush what it drew");
        }
        if !clipped { Err("a flush past the screen's edges was not clipped") } else { Ok(()) }
    }
}

crate::kernel_test! {
    /// On a virtio GPU: the display has the size the command line asked
    /// for, else the primary scanout's preferred mode (from
    /// `GET_DISPLAY_INFO`, asked again here), shown as one resource on
    /// that scanout, and every command so far succeeded.
    fn virtio_gpu_mode_set(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let Output::VirtioGpu(g) = &mut d.output else { return Ok(()) };
        let info = g.gpu.display_info().map_err(|_| "GET_DISPLAY_INFO failed")?;
        let primary = info.enabled(g.gpu.num_scanouts()).next();
        let want = match (asked_mode(env), primary) {
            (Some(m), _) => (m.width as u32, m.height as u32),
            (None, Some((_, s))) => (s.rect.width, s.rect.height),
            (None, None) => (DEFAULT_MODE.width as u32, DEFAULT_MODE.height as u32),
        };
        kprintln!("display: virtio-gpu plan {:?}, stats {:?}", g.plan, g.gpu.stats());
        if (i.width, i.height) != want || (g.fb.width, g.fb.height) != want {
            return Err("the virtio GPU's framebuffer does not have the expected size");
        }
        if g.plan.scanouts[g.plan.primary as usize] != Some(poly_virtio_gpu::Rect::sized(want.0, want.1)) {
            return Err("the primary scanout does not show the whole framebuffer");
        }
        if primary.is_some_and(|(p, _)| p != g.plan.primary) {
            return Err("the primary scanout is not the first enabled one");
        }
        let s = g.gpu.stats();
        if g.gpu.is_broken() || s.errors != 0 || g.failures != 0 || s.updates == 0 {
            return Err("the virtio GPU reported errors, or nothing was ever shown");
        }
        if i.stride != i.width as usize * 4 || d.back.as_bytes().len() != i.stride * i.height as usize {
            return Err("the back buffer is not the resource's guest memory layout");
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// On a virtio GPU: a known pattern drawn and flushed goes out as one
    /// transfer and flush of its rectangle, which the device accepts.
    /// (The screenshot boot test compares what the device shows.)
    fn virtio_gpu_draws_and_flushes_a_pattern(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let r = Rect::new(i.width as i32 / 2 - 48, i.height as i32 / 2 - 24, 96, 48);
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let Some((_, updates)) = gpu_updates(d) else { return Ok(()) };
        let saved = save(&d.back, r);
        pattern(&mut d.back, r);
        d.flush(r);
        let after = gpu_updates(d);
        let errors = match &d.output {
            Output::VirtioGpu(g) => g.gpu.stats().errors + g.failures as u64,
            Output::Vbe(_) => 0,
        };
        restore(&mut d.back, r, &saved);
        d.flush(r);
        if after != Some((Some(gpu_rect(r)), updates + 1)) || errors != 0 {
            return Err("the pattern's flush did not go out as one accepted update of its rectangle");
        }
        Ok(())
    }
}

crate::kernel_test! {
    /// On a virtio GPU: switching to a larger size (as after a display
    /// change) makes a new resource and back buffer that keep the picture,
    /// and planning again from the device's display info switches back,
    /// picture and all, freeing what the larger size took.
    fn virtio_gpu_mode_switch_keeps_the_picture(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        if i.device != DeviceKind::VirtioGpu {
            return Ok(());
        }
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let Output::VirtioGpu(g) = &d.output else { return Ok(()) };
        let (size, old_id) = (g.request.size, g.fb.resource_id);
        let strip = Rect::new(0, 0, i.width, 32.min(i.height));
        let corner = Rect::new(i.width as i32 - 16, i.height as i32 - 16, 16, 16);
        let (top, bottom) = (save(&d.back, strip), save(&d.back, corner));
        let bigger = (i.width + 64, i.height + 32);
        d.replan(Some(bigger)).map_err(|_| "switching to a larger size failed")?;
        let grown = (d.info.width, d.info.height) == bigger
            && save(&d.back, strip) == top
            && save(&d.back, corner) == bottom
            && matches!(&d.output, Output::VirtioGpu(g) if g.fb.resource_id != old_id && (g.fb.width, g.fb.height) == bigger);
        d.replan(size).map_err(|_| "switching back failed")?;
        let back = (d.info.width, d.info.height) == (i.width, i.height)
            && save(&d.back, strip) == top
            && save(&d.back, corner) == bottom;
        let ok = match &d.output {
            Output::VirtioGpu(g) => !g.gpu.is_broken() && g.gpu.stats().errors == 0,
            Output::Vbe(_) => false,
        };
        if !grown {
            return Err("the larger size did not take over with the picture kept");
        }
        if !back || !ok { Err("planning again did not switch back to the original size with the picture") } else { Ok(()) }
    }
}
