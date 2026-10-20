//! The display: a linear framebuffer on QEMU's standard VGA or
//! `bochs-display` device, a back buffer in RAM, and drawing with
//! `poly-gfx` ([ADR 0019](../../docs/adr/0019-display-framebuffer-and-drawing.md)).
//!
//! # Bring-up ([`init`])
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
//! `poly.display=off` skips all of it and leaves the device as the firmware
//! set it up.
//!
//! # Drawing
//!
//! Everything draws into the back buffer ([`with_canvas`]), which is
//! ordinary write-back RAM, so reading pixels back (scrolling a terminal
//! with `Canvas::copy_within`, blending) is fast. [`flush`] then copies a
//! rectangle of it to the framebuffer, row by row: the framebuffer is only
//! ever written, in long runs, which is what write-combining is good at
//! (and what an uncached mapping is least bad at). [`draw`] does both under
//! one lock. The lock is a spinlock (interrupts off while drawing), so any
//! context may draw, as a panic screen will want to; keep the work done
//! under it to what changed.

use crate::arch::dispi::DispiPorts;
use crate::boot_info::BootInfo;
use crate::frames;
use crate::pci::{self, Device, MmioRegion, Size};
use crate::sync::SpinLock;
use crate::{arch, kprintln, mm};
use core::fmt;
use poly_bochs_vbe::{self as vbe, Mode, ModeInfo, Registers};
use poly_gfx::{Canvas, Color, PixelFormat, Rect};
use poly_mem::PAGE_SIZE;
use poly_pci::{Bdf, Mmio, command};

/// The mode without `poly.display=` on the command line: 16:10 and small
/// enough for a laptop screen at 100 % scaling. Its back buffer (4000 KiB)
/// fits in one 4 MiB block of frames.
pub const DEFAULT_MODE: Mode = Mode::new(1280, 800);

/// Kernel argument choosing the mode: `poly.display=<width>x<height>`, or
/// `poly.display=off` for no display at all.
pub const MODE_ARG: &str = "poly.display";

/// Kernel flag saying the boot test runs with a display; the display tests
/// then require one (`cargo xtask` always passes it: QEMU's `q35` machine
/// comes with a standard VGA unless told otherwise).
pub const TEST_DISPLAY_FLAG: &str = "poly.test.display";

/// The pixel format every mode set here uses.
const FORMAT: PixelFormat = PixelFormat::Xrgb8888;

/// What the display shows and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    /// Bytes per framebuffer line. (The back buffer's lines are exactly
    /// `width` pixels long.)
    pub stride: usize,
    pub format: PixelFormat,
    /// Physical address and size of the framebuffer (BAR 0).
    pub framebuffer: u64,
    pub vram: u64,
    /// Whether the framebuffer is mapped write-combining (else uncached).
    pub write_combining: bool,
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
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoFramebuffer => write!(f, "BAR 0 (the framebuffer) is missing or unassigned"),
            Error::NoRegisters => write!(f, "no MMIO register BAR and no VGA I/O ports"),
            Error::Vbe(e) => write!(f, "{e}"),
            Error::MapFailed => write!(f, "cannot map the framebuffer"),
            Error::NoBackBuffer(bytes) => write!(f, "no {} of contiguous memory for the back buffer", Size(*bytes)),
        }
    }
}

struct Display {
    info: DisplayInfo,
    bdf: Bdf,
    /// The mode asked for, when the device refused it and got
    /// [`DEFAULT_MODE`] instead.
    refused: Option<Mode>,
    regs: Regs,
    /// The framebuffer, as mapped (all of BAR 0 behind it).
    front: Canvas<'static>,
    /// The back buffer in RAM, where all drawing happens.
    back: Canvas<'static>,
}

impl Display {
    /// Copy `r` (clipped) from the back buffer to the framebuffer.
    fn flush(&mut self, r: Rect) {
        let r = r.intersect(&self.info.bounds());
        if !r.is_empty() {
            self.front.blit(r.x, r.y, &self.back, r);
        }
    }
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

/// Copy rectangle `r` of the back buffer to the screen.
pub fn flush(r: Rect) {
    if let Some(d) = DISPLAY.lock().as_mut() {
        d.flush(r);
    }
}

/// Copy the whole back buffer to the screen.
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
    let mode = match arg.map(|a| (a, Mode::parse(a))) {
        None => DEFAULT_MODE,
        Some((_, Some(mode))) => mode,
        Some((a, None)) => {
            kprintln!("display: {MODE_ARG}={a} is not <width>x<height>; using {}", DEFAULT_MODE);
            DEFAULT_MODE
        }
    };
    let Some(dev) = find() else {
        kprintln!("display: no QEMU standard VGA or bochs-display (PCI 1234:1111)");
        return;
    };
    let mut display = match bring_up(dev, mode) {
        Ok(d) => d,
        Err(e @ Error::Vbe(vbe::Error::Unsupported { .. } | vbe::Error::TooLarge { .. })) if mode != DEFAULT_MODE => {
            kprintln!("display: {}: {mode}: {e}; trying {DEFAULT_MODE}", dev.bdf());
            match bring_up(dev, DEFAULT_MODE) {
                Ok(d) => Display { refused: Some(mode), ..d },
                Err(e) => return kprintln!("display: {}: {e}", dev.bdf()),
            }
        }
        Err(e) => return kprintln!("display: {}: {e}", dev.bdf()),
    };
    let i = display.info;
    poly_gfx::boot_screen::draw(&mut display.back, env!("CARGO_PKG_VERSION"));
    display.flush(i.bounds());
    kprintln!(
        "display: {} {}x{}x32, {} bytes per line; framebuffer {:#x} ({}) mapped {}; registers through {}",
        display.bdf,
        i.width,
        i.height,
        i.stride,
        i.framebuffer,
        Size(i.vram),
        if i.write_combining { "write-combining" } else { "uncached" },
        match display.regs {
            Regs::Mmio(_) => "MMIO (BAR 2)",
            Regs::Ports(_) => "I/O ports 0x1ce/0x1cf",
        },
    );
    *DISPLAY.lock() = Some(display);
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

    let back_stride = width as usize * FORMAT.bytes_per_pixel();
    let back_len = back_stride * height as usize;
    let back_frames = (back_len as u64).div_ceil(PAGE_SIZE);
    let phys = frames::allocate_contiguous(back_frames).ok_or(Error::NoBackBuffer(back_len as u64))?;
    let back_virt =
        arch::phys_range_to_virt(phys, back_frames * PAGE_SIZE).ok_or(Error::NoBackBuffer(back_len as u64))?;
    // SAFETY: the frames were just allocated for the display, which keeps
    // them for the kernel's lifetime, so nothing else uses them; the direct
    // map makes them writable at `back_virt`, physically contiguous, so
    // contiguous here too. They are zeroed before the slice exists.
    let back = unsafe {
        core::ptr::write_bytes(back_virt as *mut u8, 0, back_len);
        core::slice::from_raw_parts_mut(back_virt as *mut u8, back_len)
    };
    let back = Canvas::new(back, width, height, back_stride, FORMAT).map_err(|_| Error::MapFailed)?;

    let info =
        DisplayInfo { width, height, stride, format: FORMAT, framebuffer: fb.address, vram: fb.size, write_combining };
    Ok(Display { info, bdf, refused: None, regs, front, back })
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

/// The mode the boot asked for.
fn expected_mode(env: &crate::ktest::TestEnv) -> Mode {
    env.boot.arg(MODE_ARG).and_then(Mode::parse).unwrap_or(DEFAULT_MODE)
}

/// The display, or why the test cannot run: an error if the boot said
/// there is one.
fn require(env: &crate::ktest::TestEnv) -> Result<Option<DisplayInfo>, &'static str> {
    match info() {
        Some(i) => Ok(Some(i)),
        None if env.boot.has_flag(TEST_DISPLAY_FLAG) => Err("poly.test.display is set but no display came up"),
        None => Ok(None),
    }
}

crate::kernel_test! {
    /// The device is found, and its registers show the mode that was asked
    /// for (or the default, if the device refused that), the linear
    /// framebuffer on, starting at the framebuffer's start.
    fn device_found_and_mode_set(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let guard = DISPLAY.lock();
        let d = guard.as_ref().ok_or("the display went away")?;
        let want = match d.refused {
            Some(refused) if refused == expected_mode(env) => DEFAULT_MODE,
            Some(_) => return Err("the display fell back from a mode the command line did not ask for"),
            None => expected_mode(env),
        };
        if (i.width, i.height) != (want.width as u32, want.height as u32) {
            return Err("the display does not have the mode the command line asked for");
        }
        let id = d.regs.read(vbe::index::ID);
        let shown: Option<ModeInfo> = vbe::current(&d.regs);
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
        let dev = pci::devices().iter().find(|x| x.bdf() == d.bdf).ok_or("the display's function is gone")?;
        if !is_vga(dev) || !matches!(d.regs, Regs::Mmio(_)) {
            return Ok(());
        }
        // The ports need I/O decoding, which MMIO-driven bring-up left off.
        poly_pci::enable(dev.config(), dev.bdf(), command::IO_SPACE);
        for index in [vbe::index::ID, vbe::index::XRES, vbe::index::YRES, vbe::index::BPP, vbe::index::VIRT_WIDTH] {
            if DispiPorts.read(index) != d.regs.read(index) {
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
        let virt = d.front.as_bytes().as_ptr() as u64;
        let want = if arch::cpu::features().pat { mm::Caching::WriteCombining } else { mm::Caching::Uncached };
        if mm::caching(virt) != Some(want) || i.write_combining != (want == mm::Caching::WriteCombining) {
            return Err("the framebuffer is not mapped with the expected memory type");
        }
        let last = (i.width as i32 - 1, i.height as i32 - 1);
        if mm::caching(virt + (i.stride * (i.height as usize - 1)) as u64) != Some(want) {
            return Err("the end of the framebuffer is not mapped like its start");
        }
        let old = d.front.pixel(last.0, last.1).ok_or("the last pixel is outside the framebuffer")?;
        let probe = Color::rgb(0x12, 0x34, 0x56);
        d.front.set_pixel(last.0, last.1, probe);
        let back = d.front.pixel(last.0, last.1);
        d.front.set_pixel(last.0, last.1, old);
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
    /// screen does not change until a flush.
    fn back_buffer_holds_a_drawing(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let r = Rect::new(i.width as i32 / 2 - 32, i.height as i32 - 80, 64, 48);
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let (saved, screen) = (save(&d.back, r), save(&d.front, r));
        pattern(&mut d.back, r);
        // The same pattern drawn on a scratch canvas is the reference.
        let mut scratch = alloc::vec![0u8; (r.width * r.height * 4) as usize];
        let mut reference = Canvas::new(&mut scratch, r.width, r.height, r.width as usize * 4, FORMAT)
            .map_err(|_| "cannot make a scratch canvas")?;
        pattern(&mut reference, Rect::new(0, 0, r.width, r.height));
        let same = save(&reference, reference.bounds()) == save(&d.back, r);
        let untouched = save(&d.front, r) == screen;
        restore(&mut d.back, r, &saved);
        if !same {
            return Err("the back buffer does not hold what was drawn");
        }
        if !untouched { Err("drawing on the back buffer changed the screen") } else { Ok(()) }
    }
}

crate::kernel_test! {
    /// A flush copies exactly its rectangle to the screen.
    fn flush_copies_only_its_rectangle(env: &TestEnv) -> TestResult {
        let Some(i) = require(env)? else { return Ok(()) };
        let outer = Rect::new(8, i.height as i32 - 72, 64, 64);
        let inner = Rect::new(outer.x + 16, outer.y + 16, 32, 32);
        let red = Color::rgb(0xff, 0, 0);
        let mut guard = DISPLAY.lock();
        let d = guard.as_mut().ok_or("the display went away")?;
        let (saved, screen) = (save(&d.back, outer), save(&d.front, outer));
        d.back.fill_rect(outer, red);
        d.flush(inner);
        let after = save(&d.front, outer);
        restore(&mut d.back, outer, &saved);
        d.flush(outer);
        let restored = save(&d.front, outer) == screen;
        let points = (outer.y..outer.bottom() as i32).flat_map(|y| (outer.x..outer.right() as i32).map(move |x| (x, y)));
        for (((x, y), now), before) in points.zip(&after).zip(&screen) {
            let want = if inner.contains(x, y) { Some(red) } else { *before };
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
            save(&d.front, r).iter().all(|p| *p == Some(green))
        };
        with_canvas(|c| restore(c, r, &saved));
        flush(Rect::new(i.width as i32 - 40, -100, 1000, 1000));
        flush_all();
        if drawn != Some(r) || !shown { Err("draw did not flush what it drew") } else { Ok(()) }
    }
}
