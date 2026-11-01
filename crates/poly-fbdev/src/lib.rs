//! The Linux framebuffer device interface, `/dev/fb0` (ADR 0107): what a
//! program sees of the display through fbdev, kept small on purpose (the
//! compositor moves to DRM/KMS when that is there).
//!
//! Sources (ABI facts only, CLEAN_ROOM.md): the public uapi header
//! `linux/fb.h` (`struct fb_var_screeninfo`, `struct fb_fix_screeninfo`,
//! `struct fb_bitfield`, the `FBIO*` request numbers, `FB_TYPE_*`,
//! `FB_VISUAL_*`, `FB_ACTIVATE_*`, `FB_BLANK_*`), `asm-generic/ioctl.h` for
//! how `_IOW` encodes a request, the kernel's framebuffer documentation
//! (`fb/api.rst`, `fb/framebuffer.rst`: the screen information, panning,
//! `mmap` of the frame buffer memory) and `fbdev(4)`'s device numbers
//! (character major 29). Offsets and sizes were checked against the header
//! with `offsetof` on x86-64.
//!
//! What this crate holds, all host-tested:
//!
//! - [`VarScreenInfo`] and [`FixScreenInfo`] with their byte layouts, and
//!   the ones a [`Mode`] (a 32-bit `XRGB8888` true-colour mode, one page,
//!   no panning) reports.
//! - [`Device::ioctl`]: `FBIOGET_VSCREENINFO`, `FBIOPUT_VSCREENINFO` (only
//!   the current mode), `FBIOGET_FSCREENINFO`, `FBIOPAN_DISPLAY` (offset 0
//!   only: shows the whole frame), `FBIOBLANK`, and Poly's own
//!   [`FBIO_POLY_DAMAGE`], which shows one rectangle. Requests that make
//!   something visible come back as an [`Action`] for the kernel to carry
//!   out.
//! - [`spans`]: the byte ranges of the frame buffer a rectangle covers,
//!   cut at page boundaries, which is how the kernel copies a damaged
//!   rectangle out of frame buffer memory made of separate pages.
//!
//! # Making pixels visible
//!
//! The memory a program maps is not what the display scans out: it is a
//! *shadow* of it, and nothing reaches the screen until the program says
//! so. `FBIOPAN_DISPLAY` with offset (0, 0) — what double-buffering
//! programs call after drawing a frame — shows the whole frame;
//! [`FBIO_POLY_DAMAGE`] (`_IOW('F', 0x80, struct poly_fb_damage)`, a number
//! `linux/fb.h` does not use) shows only the rectangle that changed, which
//! is what a compositor wants under a slow emulator. A `write` shows the
//! rows it wrote.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::vec::Vec;

/// `FBIOGET_VSCREENINFO` (linux/fb.h): read the variable screen information.
pub const FBIOGET_VSCREENINFO: u32 = 0x4600;
/// `FBIOPUT_VSCREENINFO`: set it (and read back what was set).
pub const FBIOPUT_VSCREENINFO: u32 = 0x4601;
/// `FBIOGET_FSCREENINFO`: read the fixed screen information.
pub const FBIOGET_FSCREENINFO: u32 = 0x4602;
/// `FBIOPAN_DISPLAY`: show the frame at `xoffset`, `yoffset`.
pub const FBIOPAN_DISPLAY: u32 = 0x4606;
/// `FBIOBLANK`: blank or unblank the screen; the argument is the level.
pub const FBIOBLANK: u32 = 0x4611;
/// Poly's own request: `_IOW('F', 0x80, struct poly_fb_damage)`, where
/// `struct poly_fb_damage { __u32 x, y, width, height; }`: show that
/// rectangle of the frame buffer.
pub const FBIO_POLY_DAMAGE: u32 = ioc_write(b'F', 0x80, DAMAGE_SIZE as u32);

/// `_IOW(type, nr, size)` (asm-generic/ioctl.h: direction in bits 30-31,
/// size in 16-29, type in 8-15, number in 0-7; write is 1).
pub const fn ioc_write(ty: u8, nr: u8, size: u32) -> u32 {
    1 << 30 | size << 16 | (ty as u32) << 8 | nr as u32
}

/// Bytes of `struct fb_var_screeninfo` on x86-64.
pub const VAR_SIZE: usize = 160;
/// Bytes of `struct fb_fix_screeninfo` on x86-64.
pub const FIX_SIZE: usize = 80;
/// Bytes of `struct poly_fb_damage`.
pub const DAMAGE_SIZE: usize = 16;

/// `FB_TYPE_PACKED_PIXELS`.
pub const FB_TYPE_PACKED_PIXELS: u32 = 0;
/// `FB_VISUAL_TRUECOLOR`.
pub const FB_VISUAL_TRUECOLOR: u32 = 2;
/// `FB_ACTIVATE_TEST` (`activate` of `FBIOPUT_VSCREENINFO`): check only.
pub const FB_ACTIVATE_TEST: u32 = 2;
/// `FB_ACTIVATE_MASK`.
pub const FB_ACTIVATE_MASK: u32 = 15;
/// `FB_BLANK_UNBLANK`; the other levels (1 to 4) blank.
pub const FB_BLANK_UNBLANK: u64 = 0;
/// `FB_BLANK_POWERDOWN`, the deepest level.
pub const FB_BLANK_POWERDOWN: u64 = 4;
/// `FB_VMODE_NONINTERLACED`.
pub const FB_VMODE_NONINTERLACED: u32 = 0;

/// The character device numbers of `/dev/fb<n>` (fbdev(4)): major 29.
pub const MAJOR: u32 = 29;

/// `struct fb_bitfield`: where a colour channel sits in a pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bitfield {
    pub offset: u32,
    pub length: u32,
    pub msb_right: u32,
}

/// `struct fb_var_screeninfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VarScreenInfo {
    pub xres: u32,
    pub yres: u32,
    pub xres_virtual: u32,
    pub yres_virtual: u32,
    pub xoffset: u32,
    pub yoffset: u32,
    pub bits_per_pixel: u32,
    pub grayscale: u32,
    pub red: Bitfield,
    pub green: Bitfield,
    pub blue: Bitfield,
    pub transp: Bitfield,
    pub nonstd: u32,
    pub activate: u32,
    /// Height and width of the picture in millimetres (`height`, `width`).
    pub height_mm: u32,
    pub width_mm: u32,
    pub accel_flags: u32,
    /// Pixel clock in picoseconds.
    pub pixclock: u32,
    pub left_margin: u32,
    pub right_margin: u32,
    pub upper_margin: u32,
    pub lower_margin: u32,
    pub hsync_len: u32,
    pub vsync_len: u32,
    pub sync: u32,
    pub vmode: u32,
    pub rotate: u32,
    pub colorspace: u32,
}

/// The fields of `struct fb_var_screeninfo` in order: every one is a
/// `__u32` (the bit fields three each), then `reserved[4]`.
fn var_words(v: &VarScreenInfo) -> [u32; 36] {
    let b = |f: Bitfield| [f.offset, f.length, f.msb_right];
    let (r, g, bl, t) = (b(v.red), b(v.green), b(v.blue), b(v.transp));
    [
        v.xres,
        v.yres,
        v.xres_virtual,
        v.yres_virtual,
        v.xoffset,
        v.yoffset,
        v.bits_per_pixel,
        v.grayscale,
        r[0],
        r[1],
        r[2],
        g[0],
        g[1],
        g[2],
        bl[0],
        bl[1],
        bl[2],
        t[0],
        t[1],
        t[2],
        v.nonstd,
        v.activate,
        v.height_mm,
        v.width_mm,
        v.accel_flags,
        v.pixclock,
        v.left_margin,
        v.right_margin,
        v.upper_margin,
        v.lower_margin,
        v.hsync_len,
        v.vsync_len,
        v.sync,
        v.vmode,
        v.rotate,
        v.colorspace,
    ]
}

impl VarScreenInfo {
    pub fn to_bytes(&self) -> [u8; VAR_SIZE] {
        let mut out = [0u8; VAR_SIZE];
        for (i, w) in var_words(self).iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(b: &[u8; VAR_SIZE]) -> VarScreenInfo {
        let w = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        let bf = |i: usize| Bitfield { offset: w(i), length: w(i + 1), msb_right: w(i + 2) };
        VarScreenInfo {
            xres: w(0),
            yres: w(1),
            xres_virtual: w(2),
            yres_virtual: w(3),
            xoffset: w(4),
            yoffset: w(5),
            bits_per_pixel: w(6),
            grayscale: w(7),
            red: bf(8),
            green: bf(11),
            blue: bf(14),
            transp: bf(17),
            nonstd: w(20),
            activate: w(21),
            height_mm: w(22),
            width_mm: w(23),
            accel_flags: w(24),
            pixclock: w(25),
            left_margin: w(26),
            right_margin: w(27),
            upper_margin: w(28),
            lower_margin: w(29),
            hsync_len: w(30),
            vsync_len: w(31),
            sync: w(32),
            vmode: w(33),
            rotate: w(34),
            colorspace: w(35),
        }
    }
}

/// `struct fb_fix_screeninfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixScreenInfo {
    /// The driver's name, NUL-padded.
    pub id: [u8; 16],
    /// Physical address of the frame buffer memory (`unsigned long`).
    pub smem_start: u64,
    pub smem_len: u32,
    pub kind: u32,
    pub type_aux: u32,
    pub visual: u32,
    pub xpanstep: u16,
    pub ypanstep: u16,
    pub ywrapstep: u16,
    /// Bytes per line.
    pub line_length: u32,
    pub mmio_start: u64,
    pub mmio_len: u32,
    pub accel: u32,
    pub capabilities: u16,
}

impl FixScreenInfo {
    /// The bytes, at the offsets linux/fb.h gives on x86-64 (`smem_start`
    /// at 16, `line_length` at 48, `mmio_start` at 56, `capabilities` at
    /// 72; the gaps are padding).
    pub fn to_bytes(&self) -> [u8; FIX_SIZE] {
        let mut out = [0u8; FIX_SIZE];
        out[..16].copy_from_slice(&self.id);
        out[16..24].copy_from_slice(&self.smem_start.to_le_bytes());
        out[24..28].copy_from_slice(&self.smem_len.to_le_bytes());
        out[28..32].copy_from_slice(&self.kind.to_le_bytes());
        out[32..36].copy_from_slice(&self.type_aux.to_le_bytes());
        out[36..40].copy_from_slice(&self.visual.to_le_bytes());
        out[40..42].copy_from_slice(&self.xpanstep.to_le_bytes());
        out[42..44].copy_from_slice(&self.ypanstep.to_le_bytes());
        out[44..46].copy_from_slice(&self.ywrapstep.to_le_bytes());
        out[48..52].copy_from_slice(&self.line_length.to_le_bytes());
        out[56..64].copy_from_slice(&self.mmio_start.to_le_bytes());
        out[64..68].copy_from_slice(&self.mmio_len.to_le_bytes());
        out[68..72].copy_from_slice(&self.accel.to_le_bytes());
        out[72..74].copy_from_slice(&self.capabilities.to_le_bytes());
        out
    }
}

/// A display mode as fbdev shows it: `width` × `height` pixels of 32-bit
/// `XRGB8888` (blue in the lowest byte, little-endian), lines exactly
/// `width` pixels long, one page (no panning).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz (for `pixclock`; 0 if unknown).
    pub refresh_mhz: u32,
}

impl Mode {
    pub const BYTES_PER_PIXEL: u32 = 4;

    /// Bytes per line.
    pub const fn line_length(&self) -> u32 {
        self.width * Self::BYTES_PER_PIXEL
    }

    /// Bytes of frame buffer memory: every line.
    pub const fn size(&self) -> u64 {
        self.line_length() as u64 * self.height as u64
    }

    /// The whole screen.
    pub const fn bounds(&self) -> Rect {
        Rect { x: 0, y: 0, width: self.width, height: self.height }
    }

    /// The variable screen information of this mode. Without real timings
    /// (a virtual display has none), `pixclock` is the picosecond length of
    /// one pixel at the refresh rate with no blanking, and the margins and
    /// sync lengths are 0; the picture's size in millimetres is unknown
    /// (`-1`, as fb.h's comment on `height` and `width` allows).
    pub fn var(&self) -> VarScreenInfo {
        let pixels = u64::from(self.width) * u64::from(self.height);
        let pixclock = if self.refresh_mhz == 0 || pixels == 0 {
            0
        } else {
            // 1e12 ps per s / (pixels per frame × frames per s).
            (1_000_000_000_000_000u64 / (pixels * u64::from(self.refresh_mhz))).min(u64::from(u32::MAX)) as u32
        };
        VarScreenInfo {
            xres: self.width,
            yres: self.height,
            xres_virtual: self.width,
            yres_virtual: self.height,
            bits_per_pixel: 32,
            red: Bitfield { offset: 16, length: 8, msb_right: 0 },
            green: Bitfield { offset: 8, length: 8, msb_right: 0 },
            blue: Bitfield { offset: 0, length: 8, msb_right: 0 },
            transp: Bitfield::default(),
            height_mm: u32::MAX,
            width_mm: u32::MAX,
            pixclock,
            vmode: FB_VMODE_NONINTERLACED,
            ..VarScreenInfo::default()
        }
    }

    /// The fixed screen information: packed true-colour pixels, no panning
    /// steps, no MMIO, no acceleration.
    pub fn fix(&self, id: &str, smem_start: u64) -> FixScreenInfo {
        let mut name = [0u8; 16];
        let n = id.len().min(15);
        name[..n].copy_from_slice(&id.as_bytes()[..n]);
        FixScreenInfo {
            id: name,
            smem_start,
            smem_len: self.size().min(u64::from(u32::MAX)) as u32,
            kind: FB_TYPE_PACKED_PIXELS,
            visual: FB_VISUAL_TRUECOLOR,
            line_length: self.line_length(),
            ..FixScreenInfo::default()
        }
    }
}

/// A rectangle of the screen, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// `struct poly_fb_damage`: four `__u32`s, x, y, width, height.
    pub fn from_damage(b: &[u8; DAMAGE_SIZE]) -> Rect {
        let w = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Rect { x: w(0), y: w(4), width: w(8), height: w(12) }
    }

    pub fn to_damage(&self) -> [u8; DAMAGE_SIZE] {
        let mut out = [0u8; DAMAGE_SIZE];
        for (i, v) in [self.x, self.y, self.width, self.height].iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The part inside `bounds` (`None` if nothing is).
    pub fn clip(&self, bounds: Rect) -> Option<Rect> {
        let x0 = self.x.max(bounds.x);
        let y0 = self.y.max(bounds.y);
        let x1 = self.x.saturating_add(self.width).min(bounds.x.saturating_add(bounds.width));
        let y1 = self.y.saturating_add(self.height).min(bounds.y.saturating_add(bounds.height));
        (x1 > x0 && y1 > y0).then(|| Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }

    /// The rows from `first` (inclusive) to `last` (exclusive), whole.
    pub fn rows(mode: &Mode, first: u32, last: u32) -> Option<Rect> {
        let last = last.min(mode.height);
        (last > first).then(|| Rect { x: 0, y: first, width: mode.width, height: last - first })
    }
}

/// A byte range of frame buffer memory inside one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub offset: usize,
    pub len: usize,
}

/// Call `f` with every byte range the pixels of `r` cover in a frame
/// buffer of lines `stride` bytes apart and 4-byte pixels, each cut at
/// multiples of `page` (a power of two), row by row from the top.
pub fn spans(r: Rect, stride: usize, page: usize, mut f: impl FnMut(Span)) {
    let bpp = Mode::BYTES_PER_PIXEL as usize;
    for y in r.y..r.y.saturating_add(r.height) {
        let mut offset = y as usize * stride + r.x as usize * bpp;
        let mut left = r.width as usize * bpp;
        while left > 0 {
            let room = page - (offset & (page - 1));
            let len = left.min(room);
            f(Span { offset, len });
            offset += len;
            left -= len;
        }
    }
}

/// The rows of the frame buffer that the bytes `[offset, offset + len)`
/// touch, as a rectangle (what a `write` there makes visible).
pub fn rows_of(mode: &Mode, offset: u64, len: usize) -> Option<Rect> {
    let line = u64::from(mode.line_length());
    if line == 0 || len == 0 {
        return None;
    }
    let first = offset / line;
    let last = (offset + len as u64).div_ceil(line);
    Rect::rows(mode, first.min(u64::from(u32::MAX)) as u32, last.min(u64::from(u32::MAX)) as u32)
}

/// What an ioctl leaves for the kernel to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Put this rectangle of the frame buffer on the screen.
    Show(Rect),
}

/// An ioctl's result: what to copy back to the caller, the return value,
/// and what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub out: Vec<u8>,
    pub value: u64,
    pub action: Action,
}

impl Reply {
    fn value(value: u64) -> Reply {
        Reply { out: Vec::new(), value, action: Action::None }
    }
}

/// Why an ioctl failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a request of this device (`ENOTTY`).
    NoSuchRequest,
    /// A request with an argument it cannot have (`EINVAL`).
    Invalid,
}

/// One frame buffer device: its mode and what it reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub mode: Mode,
    pub id: &'static str,
    /// What `smem_start` reports: 0 when the memory is not one physically
    /// contiguous block (Poly's shadow pages are not).
    pub smem_start: u64,
    /// The blanking level last set (`FBIOBLANK`).
    pub blank: u64,
}

impl Device {
    pub fn new(mode: Mode, id: &'static str) -> Device {
        Device { mode, id, smem_start: 0, blank: FB_BLANK_UNBLANK }
    }

    /// How many bytes request `request` reads from the address in its
    /// argument (the kernel copies them in before [`Device::ioctl`]);
    /// `None` for a request this device does not have.
    pub fn input_len(request: u32) -> Option<usize> {
        match request {
            FBIOGET_VSCREENINFO | FBIOGET_FSCREENINFO | FBIOBLANK => Some(0),
            FBIOPUT_VSCREENINFO | FBIOPAN_DISPLAY => Some(VAR_SIZE),
            FBIO_POLY_DAMAGE => Some(DAMAGE_SIZE),
            _ => None,
        }
    }

    /// Request `request` with argument `arg` and the bytes it pointed at
    /// (`input`, [`Device::input_len`] of them).
    pub fn ioctl(&mut self, request: u32, arg: u64, input: &[u8]) -> Result<Reply, Error> {
        let need = Self::input_len(request).ok_or(Error::NoSuchRequest)?;
        if input.len() < need {
            return Err(Error::Invalid);
        }
        let var = || -> VarScreenInfo {
            let mut b = [0u8; VAR_SIZE];
            b.copy_from_slice(&input[..VAR_SIZE]);
            VarScreenInfo::from_bytes(&b)
        };
        match request {
            FBIOGET_VSCREENINFO => Ok(Reply { out: self.mode.var().to_bytes().to_vec(), ..Reply::value(0) }),
            FBIOGET_FSCREENINFO => {
                Ok(Reply { out: self.mode.fix(self.id, self.smem_start).to_bytes().to_vec(), ..Reply::value(0) })
            }
            FBIOPUT_VSCREENINFO => {
                // Only the mode there is: same resolution (or 0, "keep"),
                // 32 bits per pixel (or 0), no panning. What is set comes back.
                let v = var();
                let same = |want: u32, have: u32| want == 0 || want == have;
                let m = &self.mode;
                let fits = same(v.xres, m.width)
                    && same(v.yres, m.height)
                    && (v.xres_virtual == 0 || v.xres_virtual == m.width)
                    && (v.yres_virtual == 0 || v.yres_virtual == m.height)
                    && same(v.bits_per_pixel, 32)
                    && v.xoffset == 0
                    && v.yoffset == 0;
                if !fits {
                    return Err(Error::Invalid);
                }
                let mut now = m.var();
                now.activate = v.activate;
                Ok(Reply { out: now.to_bytes().to_vec(), ..Reply::value(0) })
            }
            FBIOPAN_DISPLAY => {
                let v = var();
                if v.xoffset != 0 || v.yoffset != 0 {
                    return Err(Error::Invalid);
                }
                Ok(Reply { action: Action::Show(self.mode.bounds()), ..Reply::value(0) })
            }
            FBIOBLANK => {
                if arg > FB_BLANK_POWERDOWN {
                    return Err(Error::Invalid);
                }
                self.blank = arg;
                // Unblanking shows the frame again.
                let action = if arg == FB_BLANK_UNBLANK { Action::Show(self.mode.bounds()) } else { Action::None };
                Ok(Reply { action, ..Reply::value(0) })
            }
            FBIO_POLY_DAMAGE => {
                let mut b = [0u8; DAMAGE_SIZE];
                b.copy_from_slice(&input[..DAMAGE_SIZE]);
                let action = Rect::from_damage(&b).clip(self.mode.bounds()).map_or(Action::None, Action::Show);
                Ok(Reply { action, ..Reply::value(0) })
            }
            _ => Err(Error::NoSuchRequest),
        }
    }
}

#[cfg(test)]
mod tests;
