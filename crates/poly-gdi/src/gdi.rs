//! A process's GDI: its objects (DCs, bitmaps, brushes, pens, fonts,
//! regions), their handles, and the drawing calls of the first slice
//! (`specs/windows/gdi32.dll/`). `gdi32.dll` keeps one [`Gdi`] behind a lock
//! and maps each export onto a method; errors come back as the Win32 code
//! the export sets (0: none).

use crate::bitmap::{Bitmap, Rgb, Storage, rop3};
use crate::font::{self, Face, TextMetric};
pub use crate::region::Rect;
use crate::region::{COMPLEXREGION, NULLREGION, RGN_AND, RGN_COPY, Region, SIMPLEREGION};
use alloc::vec;
use alloc::vec::Vec;

// Object types (`wingdi.h`).
pub const OBJ_PEN: u32 = 1;
pub const OBJ_BRUSH: u32 = 2;
pub const OBJ_DC: u32 = 3;
pub const OBJ_PAL: u32 = 5;
pub const OBJ_FONT: u32 = 6;
pub const OBJ_BITMAP: u32 = 7;
pub const OBJ_REGION: u32 = 8;
pub const OBJ_MEMDC: u32 = 10;

pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const CLR_INVALID: u32 = 0xFFFF_FFFF;

pub const TRANSPARENT: i32 = 1;
pub const OPAQUE: i32 = 2;

// Text alignment.
pub const TA_UPDATECP: u32 = 1;
pub const TA_RIGHT: u32 = 2;
pub const TA_CENTER: u32 = 6;
pub const TA_BOTTOM: u32 = 8;
pub const TA_BASELINE: u32 = 24;

// Brush styles.
pub const BS_SOLID: u32 = 0;
pub const BS_NULL: u32 = 1;
// Pen styles.
pub const PS_NULL: u32 = 5;

/// `GetRandomRgn`'s system region (`wingdi.h`).
pub const SYSRGN: i32 = 4;

/// The coordinates a region may have when made (`CreateRectRgn.yaml`,
/// observed: -2^27 to 2^27-1; others fail with ERROR_INVALID_PARAMETER).
const RGN_MIN: i32 = -(1 << 27);
const RGN_MAX: i32 = (1 << 27) - 1;

fn rgn_coord_ok(v: i32) -> bool {
    (RGN_MIN..=RGN_MAX).contains(&v)
}

/// What a DC draws on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DcKind {
    /// `CreateCompatibleDC`: the bitmap selected into it.
    Memory,
    /// `GetDC` of a window (or the screen, `hwnd` 0); `persistent` for a
    /// class or own DC, which survives `ReleaseDC`.
    Display { hwnd: u64, persistent: bool },
}

#[derive(Clone, Debug)]
struct DcState {
    text: u32,
    bk: u32,
    bk_mode: i32,
    align: u32,
    brush: u32,
    pen: u32,
    font: u32,
    pos: (i32, i32),
    dc_brush: u32,
    dc_pen: u32,
    /// The clip region, in device coordinates (`SelectClipRgn.yaml`).
    clip: Option<Region>,
    /// `SetViewportOrgEx` / `SetWindowOrgEx`: device = logical - window
    /// origin + viewport origin.
    viewport_org: (i32, i32),
    window_org: (i32, i32),
}

impl DcState {
    /// What to add to logical coordinates for device ones.
    fn to_device(&self) -> (i32, i32) {
        (self.viewport_org.0.wrapping_sub(self.window_org.0), self.viewport_org.1.wrapping_sub(self.window_org.1))
    }
}

#[derive(Debug)]
struct Dc {
    kind: DcKind,
    state: DcState,
    saved: Vec<DcState>,
    bitmap: u32,
    /// A window surface a display DC draws into (the desktop link).
    surface: Option<u32>,
    /// A window DC's visible region in device coordinates (the part of
    /// the window on the screen, less clipped children and siblings), as
    /// user32 tells it ([`Gdi::set_display_geometry`]); `None`: not told
    /// (the screen, or the surface bitmap when there is one).
    vis: Option<Region>,
    /// Where device 0,0 is on the screen (`GetRandomRgn` answers in screen
    /// coordinates).
    vis_origin: (i32, i32),
    /// A system clip on top of the visible region (device coordinates),
    /// [`Gdi::set_system_clip`].
    sys_clip: Option<Region>,
}

#[derive(Debug)]
enum Obj {
    Dc(Dc),
    /// A deleted DC: still answers `OBJ_DC` (`CreateCompatibleDC.yaml`).
    DeadDc,
    Bitmap {
        bmp: Bitmap,
        selected_in: u32,
        delete_pending: bool,
        stock: bool,
    },
    Brush {
        style: u32,
        color: u32,
        hatch: u64,
        stock: bool,
        deleted: bool,
        delete_pending: bool,
    },
    Pen {
        style: u32,
        width: i32,
        color: u32,
        stock: bool,
        delete_pending: bool,
    },
    Font {
        face: Face,
        stock: bool,
        logfont: Vec<u8>,
        delete_pending: bool,
    },
    Region(Region),
    Palette,
}

/// Stock object indexes (`GetStockObject`, `wingdi.h`).
pub const WHITE_BRUSH: u32 = 0;
pub const BLACK_BRUSH: u32 = 4;
pub const NULL_BRUSH: u32 = 5;
pub const BLACK_PEN: u32 = 7;
pub const NULL_PEN: u32 = 8;
pub const SYSTEM_FONT: u32 = 13;
pub const DC_BRUSH: u32 = 18;
pub const DC_PEN: u32 = 19;
/// Poly's own index for the default 1x1 monochrome bitmap.
const STOCK_BITMAP: u32 = 20;
const STOCK_COUNT: u32 = 21;

/// Handles are `generation << 24 | index << 2`: a slot used again gets a
/// new generation (1 to 127), so a deleted object's handle stays invalid
/// (observed: a PatBlt after selecting a deleted brush's handle paints
/// nothing, even when a new brush took its place).
const MAX_GENERATION: u8 = 0x7F;

/// What `GetDeviceCaps` of the screen reports.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    pub width: i32,
    pub height: i32,
}

/// A process's GDI objects.
#[derive(Debug)]
pub struct Gdi {
    objs: Vec<Option<Obj>>,
    /// Each slot's generation (the handle's top byte).
    gens: Vec<u8>,
    pub screen: Screen,
}

impl Default for Gdi {
    fn default() -> Self {
        Gdi::new()
    }
}

/// A pixel source for blits: a bitmap and its DC's colours.
struct Src<'a> {
    bmp: &'a Bitmap,
    bk: Rgb,
}

impl Gdi {
    pub fn new() -> Gdi {
        let mut g = Gdi { objs: Vec::new(), gens: Vec::new(), screen: Screen { width: 1024, height: 768 } };
        g.objs.push(None); // index 0 is never a handle
        let brush = |c: u32, style: u32| Obj::Brush {
            style,
            color: c,
            hatch: 0,
            stock: true,
            deleted: false,
            delete_pending: false,
        };
        let pen = |c: u32, style: u32| Obj::Pen { style, width: 1, color: c, stock: true, delete_pending: false };
        let font = |f: Face| Obj::Font { face: f, stock: true, logfont: Vec::new(), delete_pending: false };
        let stock: [Obj; STOCK_COUNT as usize] = [
            brush(0x00FF_FFFF, BS_SOLID), // WHITE_BRUSH
            brush(0x00C0_C0C0, BS_SOLID), // LTGRAY_BRUSH
            brush(0x0080_8080, BS_SOLID), // GRAY_BRUSH
            brush(0x0040_4040, BS_SOLID), // DKGRAY_BRUSH
            brush(0, BS_SOLID),           // BLACK_BRUSH
            brush(0, BS_NULL),            // NULL_BRUSH
            pen(0x00FF_FFFF, 0),          // WHITE_PEN
            pen(0, 0),                    // BLACK_PEN
            pen(0, PS_NULL),              // NULL_PEN
            Obj::Palette,                 // 9: unused
            font(Face::OemFixed),         // OEM_FIXED_FONT
            font(Face::AnsiFixed),        // ANSI_FIXED_FONT
            font(Face::Gui),              // ANSI_VAR_FONT
            font(Face::System),           // SYSTEM_FONT
            font(Face::System),           // DEVICE_DEFAULT_FONT
            Obj::Palette,                 // DEFAULT_PALETTE
            font(Face::AnsiFixed),        // SYSTEM_FIXED_FONT
            font(Face::Gui),              // DEFAULT_GUI_FONT
            brush(0x00FF_FFFF, BS_SOLID), // DC_BRUSH
            pen(0, 0),                    // DC_PEN
            Obj::Bitmap { bmp: Bitmap::ddb(1, 1, 1), selected_in: 0, delete_pending: false, stock: true },
        ];
        for o in stock {
            g.objs.push(Some(o));
        }
        g
    }

    fn handle_of(&self, index: usize) -> u32 {
        let generation = self.gens.get(index).copied().unwrap_or(1);
        (u32::from(generation) << 24) | ((index as u32) << 2)
    }

    fn index_of(&self, h: u64) -> Option<usize> {
        let h = h as u32;
        if h & 3 != 0 || h >> 24 == 0 || h >> 24 > u32::from(MAX_GENERATION) {
            return None;
        }
        let i = ((h & 0x00FF_FFFF) >> 2) as usize;
        let current = self.gens.get(i).map_or(1, |&g| u32::from(g));
        (i > 0 && i < self.objs.len() && self.objs[i].is_some() && h >> 24 == current).then_some(i)
    }

    fn add(&mut self, o: Obj) -> u32 {
        let i = match self.objs.iter().skip(1 + STOCK_COUNT as usize).position(Option::is_none) {
            Some(p) => {
                let i = p + 1 + STOCK_COUNT as usize;
                if self.gens.len() <= i {
                    self.gens.resize(i + 1, 1);
                }
                self.gens[i] = if self.gens[i] >= MAX_GENERATION { 1 } else { self.gens[i] + 1 };
                i
            }
            None => {
                self.objs.push(None);
                self.objs.len() - 1
            }
        };
        if self.gens.len() <= i {
            self.gens.resize(i + 1, 1);
        }
        self.objs[i] = Some(o);
        self.handle_of(i)
    }

    /// `GetStockObject`.
    pub fn stock(&self, i: u32) -> u32 {
        if i >= STOCK_COUNT - 1 || i == 9 {
            return 0;
        }
        self.handle_of(1 + i as usize)
    }

    fn obj(&self, h: u64) -> Option<&Obj> {
        self.index_of(h).and_then(|i| self.objs[i].as_ref())
    }

    fn obj_mut(&mut self, h: u64) -> Option<&mut Obj> {
        let i = self.index_of(h)?;
        self.objs[i].as_mut()
    }

    fn dc(&self, h: u64) -> Option<&Dc> {
        match self.obj(h) {
            Some(Obj::Dc(d)) => Some(d),
            _ => None,
        }
    }

    fn dc_mut(&mut self, h: u64) -> Option<&mut Dc> {
        match self.obj_mut(h) {
            Some(Obj::Dc(d)) => Some(d),
            _ => None,
        }
    }

    fn default_state(&self) -> DcState {
        DcState {
            text: 0,
            bk: 0x00FF_FFFF,
            bk_mode: OPAQUE,
            align: 0,
            brush: self.stock(WHITE_BRUSH),
            pen: self.stock(BLACK_PEN),
            font: self.stock(SYSTEM_FONT),
            pos: (0, 0),
            dc_brush: 0x00FF_FFFF,
            dc_pen: 0,
            clip: None,
            viewport_org: (0, 0),
            window_org: (0, 0),
        }
    }

    // ----- DCs

    /// `CreateCompatibleDC`: a memory DC with the default bitmap.
    pub fn create_compatible_dc(&mut self, _of: u64) -> u32 {
        let state = self.default_state();
        let bitmap = self.handle_of(1 + STOCK_BITMAP as usize);
        self.add(Obj::Dc(Dc {
            kind: DcKind::Memory,
            state,
            saved: Vec::new(),
            bitmap,
            surface: None,
            vis: None,
            vis_origin: (0, 0),
            sys_clip: None,
        }))
    }

    /// A DC for a window's client area or the screen (`GetDC`).
    pub fn create_display_dc(&mut self, hwnd: u64, persistent: bool) -> u32 {
        let state = self.default_state();
        self.add(Obj::Dc(Dc {
            kind: DcKind::Display { hwnd, persistent },
            state,
            saved: Vec::new(),
            bitmap: 0,
            surface: None,
            vis: None,
            vis_origin: (0, 0),
            sys_clip: None,
        }))
    }

    /// `ReleaseDC`: a common display DC goes, a persistent one stays; 1
    /// when it was a display DC.
    pub fn release_display_dc(&mut self, h: u64) -> bool {
        match self.dc(h).map(|d| d.kind) {
            Some(DcKind::Display { persistent: false, .. }) => {
                self.free_dc(h, false);
                true
            }
            Some(DcKind::Display { persistent: true, .. }) => true,
            _ => false,
        }
    }

    /// Whether a handle is a DC of a window (`hwnd`), for `ReleaseDC`.
    pub fn display_hwnd(&self, h: u64) -> Option<u64> {
        match self.dc(h)?.kind {
            DcKind::Display { hwnd, .. } => Some(hwnd),
            DcKind::Memory => None,
        }
    }

    fn free_dc(&mut self, h: u64, zombie: bool) {
        let Some(i) = self.index_of(h) else { return };
        let Some(Obj::Dc(d)) = self.objs[i].take() else { return };
        // Objects selected into it whose deletion waited go now.
        for o in [d.state.brush, d.state.pen, d.state.font] {
            self.finish_pending(u64::from(o));
        }
        if let Some(Obj::Bitmap { selected_in, .. }) = self.obj_mut(u64::from(d.bitmap)) {
            *selected_in = 0;
        }
        self.finish_pending(u64::from(d.bitmap));
        if zombie {
            self.objs[i] = Some(Obj::DeadDc);
        }
    }

    /// An object whose deletion waited because it was selected: gone once
    /// no DC holds it.
    fn finish_pending(&mut self, h: u64) {
        let held = self.held(h);
        let Some(i) = self.index_of(h) else { return };
        let pending = match &self.objs[i] {
            Some(Obj::Brush { delete_pending, .. })
            | Some(Obj::Pen { delete_pending, .. })
            | Some(Obj::Font { delete_pending, .. }) => *delete_pending,
            Some(Obj::Bitmap { delete_pending, selected_in, .. }) => *delete_pending && *selected_in == 0,
            _ => false,
        };
        if pending && !held {
            self.objs[i] = None;
        }
    }

    fn held(&self, h: u64) -> bool {
        let h = h as u32;
        self.objs.iter().flatten().any(|o| match o {
            Obj::Dc(d) => d.state.brush == h || d.state.pen == h || d.state.font == h || d.bitmap == h,
            _ => false,
        })
    }

    /// `DeleteDC`: (result, error).
    pub fn delete_dc(&mut self, h: u64) -> (bool, u32) {
        match self.obj(h) {
            Some(Obj::Dc(_)) => {
                self.free_dc(h, true);
                (true, 0)
            }
            _ => (false, 0),
        }
    }

    /// `DeleteObject`.
    pub fn delete_object(&mut self, h: u64) -> bool {
        let held = self.held(h);
        let Some(o) = self.obj_mut(h) else { return false };
        match o {
            Obj::Dc(_) => {
                self.free_dc(h, true);
                true
            }
            Obj::DeadDc => false,
            Obj::Brush { stock: true, .. } | Obj::Pen { stock: true, .. } | Obj::Font { stock: true, .. } => true,
            Obj::Bitmap { stock: true, .. } => true,
            Obj::Brush { deleted, delete_pending, .. } => {
                if *deleted || *delete_pending {
                    return false;
                }
                if held {
                    *delete_pending = true;
                } else {
                    // Observed: a deleted solid brush stays readable.
                    *deleted = true;
                }
                true
            }
            Obj::Pen { delete_pending, .. } | Obj::Font { delete_pending, .. } => {
                if *delete_pending {
                    return false;
                }
                if held {
                    *delete_pending = true;
                } else {
                    let i = self.index_of(h).expect("valid");
                    self.objs[i] = None;
                }
                true
            }
            Obj::Bitmap { selected_in, delete_pending, .. } => {
                if *delete_pending {
                    return false;
                }
                if *selected_in != 0 {
                    *delete_pending = true;
                } else {
                    let i = self.index_of(h).expect("valid");
                    self.objs[i] = None;
                }
                true
            }
            Obj::Region(_) | Obj::Palette => {
                let i = self.index_of(h).expect("valid");
                self.objs[i] = None;
                true
            }
        }
    }

    /// `GetObjectType`.
    pub fn object_type(&self, h: u64) -> u32 {
        match self.obj(h) {
            Some(Obj::Dc(d)) => {
                if d.kind == DcKind::Memory {
                    OBJ_MEMDC
                } else {
                    OBJ_DC
                }
            }
            Some(Obj::DeadDc) => OBJ_DC,
            Some(Obj::Bitmap { .. }) => OBJ_BITMAP,
            Some(Obj::Brush { .. }) => OBJ_BRUSH,
            Some(Obj::Pen { .. }) => OBJ_PEN,
            Some(Obj::Font { .. }) => OBJ_FONT,
            Some(Obj::Region(_)) => OBJ_REGION,
            Some(Obj::Palette) => OBJ_PAL,
            None => 0,
        }
    }

    /// `GetObjectW`'s bytes for an object (`LOGBRUSH`, `BITMAP`, `LOGPEN`,
    /// `LOGFONTW`).
    pub fn object_bytes(&self, h: u64) -> Option<Vec<u8>> {
        Some(match self.obj(h)? {
            Obj::Brush { style, color, hatch, .. } => {
                let mut b = vec![0u8; 16];
                b[0..4].copy_from_slice(&style.to_le_bytes());
                b[4..8].copy_from_slice(&color.to_le_bytes());
                b[8..16].copy_from_slice(&hatch.to_le_bytes());
                b
            }
            Obj::Bitmap { bmp, .. } => {
                let mut b = vec![0u8; 32];
                b[4..8].copy_from_slice(&bmp.width.to_le_bytes());
                b[8..12].copy_from_slice(&bmp.height.unsigned_abs().to_le_bytes());
                b[12..16].copy_from_slice(&(bmp.width_bytes() as u32).to_le_bytes());
                b[16..18].copy_from_slice(&1u16.to_le_bytes());
                b[18..20].copy_from_slice(&bmp.bpp.to_le_bytes());
                let bits = if bmp.dib { bmp.bits_address() } else { 0 };
                b[24..32].copy_from_slice(&bits.to_le_bytes());
                b
            }
            Obj::Pen { style, width, color, .. } => {
                let mut b = vec![0u8; 16];
                b[0..4].copy_from_slice(&style.to_le_bytes());
                b[4..8].copy_from_slice(&width.to_le_bytes());
                b[12..16].copy_from_slice(&color.to_le_bytes());
                b
            }
            Obj::Font { face, logfont, .. } => {
                if logfont.len() == 92 {
                    logfont.clone()
                } else {
                    let mut b = vec![0u8; 92];
                    let tm = face.metrics();
                    b[0..4].copy_from_slice(&tm.height.to_le_bytes());
                    b[16..20].copy_from_slice(&tm.weight.to_le_bytes());
                    b[23] = tm.char_set;
                    b[27] = tm.pitch_and_family;
                    b
                }
            }
            _ => return None,
        })
    }

    /// `SelectObject`: the replaced object, or the error.
    pub fn select_object(&mut self, dc: u64, obj: u64) -> Result<u32, u32> {
        if self.dc(dc).is_none() {
            return Err(ERROR_INVALID_HANDLE);
        }
        let kind = match self.obj(obj) {
            Some(Obj::Brush { .. }) => OBJ_BRUSH,
            Some(Obj::Pen { .. }) => OBJ_PEN,
            Some(Obj::Font { .. }) => OBJ_FONT,
            Some(Obj::Bitmap { selected_in, .. }) => {
                if *selected_in != 0 && u64::from(*selected_in) != dc & 0xFFFF_FFFF {
                    return Ok(0);
                }
                OBJ_BITMAP
            }
            // Selecting a region is SelectClipRgn (Learn: SelectObject).
            Some(Obj::Region(_)) => return Ok(self.select_clip_rgn(dc, obj).0 as u32),
            _ => return Ok(0),
        };
        let h = obj as u32;
        let d = self.dc_mut(dc).expect("checked");
        let old = match kind {
            OBJ_BRUSH => core::mem::replace(&mut d.state.brush, h),
            OBJ_PEN => core::mem::replace(&mut d.state.pen, h),
            OBJ_FONT => core::mem::replace(&mut d.state.font, h),
            _ => {
                if d.kind != DcKind::Memory {
                    return Ok(0);
                }
                let old = core::mem::replace(&mut d.bitmap, h);
                if let Some(Obj::Bitmap { selected_in, .. }) = self.obj_mut(u64::from(old)) {
                    *selected_in = 0;
                }
                if let Some(Obj::Bitmap { selected_in, stock: false, .. }) = self.obj_mut(obj) {
                    *selected_in = dc as u32;
                }
                self.finish_pending(u64::from(old));
                return Ok(old);
            }
        };
        self.finish_pending(u64::from(old));
        Ok(old)
    }

    /// `GetCurrentObject`.
    pub fn current_object(&self, dc: u64, kind: u32) -> u32 {
        let Some(d) = self.dc(dc) else { return 0 };
        match kind {
            OBJ_PEN => d.state.pen,
            OBJ_BRUSH => d.state.brush,
            OBJ_FONT => d.state.font,
            OBJ_BITMAP => d.bitmap,
            OBJ_PAL => self.stock(15),
            _ => 0,
        }
    }

    // ----- creating objects

    pub fn create_solid_brush(&mut self, color: u32) -> u32 {
        self.add(Obj::Brush { style: BS_SOLID, color, hatch: 0, stock: false, deleted: false, delete_pending: false })
    }

    pub fn create_pen(&mut self, style: u32, width: i32, color: u32) -> u32 {
        self.add(Obj::Pen { style, width, color, stock: false, delete_pending: false })
    }

    /// `CreateFontIndirectW`: Poly maps every face onto the stock faces by
    /// height for now.
    pub fn create_font(&mut self, logfont: &[u8]) -> u32 {
        let height = logfont.get(0..4).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).unsigned_abs();
        let face = if height != 0 && height <= 13 { Face::Gui } else { Face::System };
        self.add(Obj::Font { face, stock: false, logfont: logfont.to_vec(), delete_pending: false })
    }

    /// `CreateBitmap`: monochrome for 1 bit per pixel, else 32 bits;
    /// `bits` (rows rounded to 2 bytes) initialise it.
    pub fn create_bitmap(&mut self, w: i32, h: i32, bpp: u16, bits: Option<&[u8]>) -> u32 {
        let (w, h) = if w <= 0 || h <= 0 { (1, 1) } else { (w, h) };
        let mut b = Bitmap::ddb(w, h, bpp);
        if let Some(src) = bits {
            let src_stride = (w as usize * usize::from(b.bpp)).div_ceil(16) * 2;
            for y in 0..h {
                for x in 0..w {
                    let off = y as usize * src_stride;
                    let v = if b.bpp == 1 {
                        src.get(off + x as usize / 8).map_or(0, |&byte| u32::from(byte >> (7 - x % 8)) & 1)
                    } else {
                        let o = off + x as usize * 4;
                        src.get(o..o + 4).map_or(0, |q| u32::from_le_bytes([q[0], q[1], q[2], q[3]]))
                    };
                    b.set_raw(x, y, v);
                }
            }
        }
        self.add(Obj::Bitmap { bmp: b, selected_in: 0, delete_pending: false, stock: false })
    }

    /// `CreateCompatibleBitmap`: the format of the DC's bitmap (a memory
    /// DC with its default bitmap: monochrome), 32 bits for display DCs;
    /// 0x0 is a 1x1 monochrome bitmap.
    pub fn create_compatible_bitmap(&mut self, dc: u64, w: i32, h: i32) -> u32 {
        let Some(d) = self.dc(dc) else { return 0 };
        if w == 0 || h == 0 {
            return self.create_bitmap(1, 1, 1, None);
        }
        let bpp = match d.kind {
            DcKind::Memory => match self.obj(u64::from(d.bitmap)) {
                Some(Obj::Bitmap { bmp, .. }) if bmp.bpp != 1 => 32,
                _ => 1,
            },
            DcKind::Display { .. } => 32,
        };
        self.create_bitmap(w, h, bpp, None)
    }

    /// Check a `BITMAPINFO` for `CreateDIBSection` (`CreateDIBSection.yaml`):
    /// (width, height, bpp, colours, bytes) or the error code.
    pub fn check_dib(info: &[u8]) -> Result<(i32, i32, u16, Vec<u32>, usize), u32> {
        let rd32 = |o: usize| info.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let rd16 = |o: usize| info.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let size = rd32(0).ok_or(0u32)?;
        if !(40..=124).contains(&size) {
            return Err(0);
        }
        let w = rd32(4).ok_or(0u32)? as i32;
        let h = rd32(8).ok_or(0u32)? as i32;
        let planes = rd16(12).ok_or(0u32)?;
        let bpp = rd16(14).ok_or(0u32)?;
        let compression = rd32(16).ok_or(0u32)?;
        let clr_used = rd32(32).unwrap_or(0);
        if planes != 1 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) || w <= 0 || h == 0 || compression != 0 {
            return Err(0);
        }
        let stride = Bitmap::dib_stride(w, bpp);
        let bytes = (stride as u64).checked_mul(u64::from(h.unsigned_abs())).ok_or(0u32)?;
        if bytes > 1 << 30 {
            return Err(0);
        }
        let mut colors = Vec::new();
        if bpp <= 8 {
            let n = if clr_used == 0 { 1usize << bpp } else { (clr_used as usize).min(1 << bpp) };
            for i in 0..n {
                colors.push(rd32(size as usize + 4 * i).unwrap_or(0) & 0x00FF_FFFF);
            }
        }
        Ok((w, h, bpp, colors, bytes as usize))
    }

    /// Make a DIB section over `bits` (zeroed memory of `bytes` bytes the
    /// caller allocated for `check_dib`'s answer).
    pub fn create_dib(&mut self, w: i32, h: i32, bpp: u16, colors: Vec<u32>, bits: u64, bytes: usize) -> u32 {
        let b = Bitmap {
            width: w,
            height: h.abs(),
            bpp,
            stride: Bitmap::dib_stride(w, bpp),
            top_down: h < 0,
            dib: true,
            colors,
            storage: Storage::External(bits, bytes),
        };
        self.add(Obj::Bitmap { bmp: b, selected_in: 0, delete_pending: false, stock: false })
    }

    /// The memory of a DIB section whose object is gone, for the caller to
    /// free: none while the object exists.
    pub fn dib_memory(&self, h: u64) -> Option<(u64, usize)> {
        match self.obj(h) {
            Some(Obj::Bitmap { bmp: Bitmap { storage: Storage::External(a, n), .. }, .. }) => Some((*a, *n)),
            _ => None,
        }
    }

    // ----- DC attributes

    fn state(&mut self, dc: u64) -> Option<&mut DcState> {
        self.dc_mut(dc).map(|d| &mut d.state)
    }

    pub fn set_text_color(&mut self, dc: u64, c: u32) -> u32 {
        self.state(dc).map_or(CLR_INVALID, |s| core::mem::replace(&mut s.text, c))
    }
    pub fn text_color(&mut self, dc: u64) -> u32 {
        self.state(dc).map_or(CLR_INVALID, |s| s.text)
    }
    pub fn set_bk_color(&mut self, dc: u64, c: u32) -> u32 {
        self.state(dc).map_or(CLR_INVALID, |s| core::mem::replace(&mut s.bk, c))
    }
    pub fn bk_color(&mut self, dc: u64) -> u32 {
        self.state(dc).map_or(CLR_INVALID, |s| s.bk)
    }
    pub fn set_bk_mode(&mut self, dc: u64, m: i32) -> i32 {
        self.state(dc).map_or(0, |s| core::mem::replace(&mut s.bk_mode, m))
    }
    pub fn bk_mode(&mut self, dc: u64) -> i32 {
        self.state(dc).map_or(0, |s| s.bk_mode)
    }
    pub fn set_text_align(&mut self, dc: u64, a: u32) -> u32 {
        self.state(dc).map_or(0xFFFF_FFFF, |s| core::mem::replace(&mut s.align, a))
    }
    pub fn text_align(&mut self, dc: u64) -> u32 {
        self.state(dc).map_or(0xFFFF_FFFF, |s| s.align)
    }
    pub fn set_dc_brush_color(&mut self, dc: u64, c: u32) -> u32 {
        self.state(dc).map_or(CLR_INVALID, |s| core::mem::replace(&mut s.dc_brush, c))
    }
    pub fn set_dc_pen_color(&mut self, dc: u64, c: u32) -> u32 {
        self.state(dc).map_or(CLR_INVALID, |s| core::mem::replace(&mut s.dc_pen, c))
    }

    /// `MoveToEx`: the previous position.
    pub fn move_to(&mut self, dc: u64, x: i32, y: i32) -> Option<(i32, i32)> {
        self.state(dc).map(|s| core::mem::replace(&mut s.pos, (x, y)))
    }

    /// `SaveDC`: the new level (0 on failure).
    pub fn save_dc(&mut self, dc: u64) -> i32 {
        let Some(d) = self.dc_mut(dc) else { return 0 };
        d.saved.push(d.state.clone());
        d.saved.len() as i32
    }

    /// `RestoreDC`.
    pub fn restore_dc(&mut self, dc: u64, level: i32) -> bool {
        let Some(d) = self.dc_mut(dc) else { return false };
        let n = d.saved.len() as i32;
        let target = if level < 0 { n + level } else { level - 1 };
        if target < 0 || target >= n {
            return false;
        }
        d.saved.truncate(target as usize + 1);
        d.state = d.saved.pop().expect("present");
        true
    }

    /// `GetDeviceCaps`.
    pub fn device_caps(&self, dc: u64, index: i32) -> i32 {
        if self.dc(dc).is_none() {
            return 0;
        }
        match index {
            2 => 1,                             // TECHNOLOGY: DT_RASDISPLAY
            4 => self.screen.width * 254 / 960, // HORZSIZE (mm)
            6 => self.screen.height * 254 / 960,
            8 | 118 => self.screen.width, // HORZRES, DESKTOPHORZRES
            10 | 117 => self.screen.height,
            12 => 32,      // BITSPIXEL
            14 => 1,       // PLANES
            24 => -1,      // NUMCOLORS
            40 | 42 => 36, // ASPECTX, ASPECTY
            44 => 51,      // ASPECTXY
            88 | 90 => 96, // LOGPIXELSX, LOGPIXELSY
            108 => 24,     // COLORRES
            116 => 60,     // VREFRESH
            _ => 0,
        }
    }

    // ----- regions (`CreateRectRgn.yaml`, `CombineRgn.yaml`, `GetRgnBox.yaml`, `PtInRegion.yaml`)

    /// The region behind a handle.
    pub fn region(&self, h: u64) -> Option<&Region> {
        match self.obj(h) {
            Some(Obj::Region(r)) => Some(r),
            _ => None,
        }
    }

    fn region_mut(&mut self, h: u64) -> Option<&mut Region> {
        match self.obj_mut(h) {
            Some(Obj::Region(r)) => Some(r),
            _ => None,
        }
    }

    /// `CreateRectRgn`: the new region, or the error for coordinates
    /// outside -2^27..2^27-1.
    pub fn create_rect_rgn(&mut self, r: Rect) -> Result<u32, u32> {
        if ![r.left, r.top, r.right, r.bottom].into_iter().all(rgn_coord_ok) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        Ok(self.create_region(Region::from_rect(r)))
    }

    /// A new region handle for `rgn`.
    pub fn create_region(&mut self, rgn: Region) -> u32 {
        self.add(Obj::Region(rgn))
    }

    /// `SetRectRgn`: any coordinates are kept (observed); false when `h`
    /// is not a region.
    pub fn set_rect_rgn(&mut self, h: u64, r: Rect) -> bool {
        let Some(rgn) = self.region_mut(h) else { return false };
        *rgn = Region::from_rect(r);
        true
    }

    /// Replace a region's contents: its new kind, ERROR (0) when `h` is not
    /// a region.
    pub fn set_region(&mut self, h: u64, rgn: Region) -> i32 {
        let Some(r) = self.region_mut(h) else { return 0 };
        *r = rgn;
        r.kind()
    }

    /// `CombineRgn`: (kind, error); the destination is untouched on error.
    pub fn combine_rgn(&mut self, dst: u64, src1: u64, src2: u64, mode: i32) -> (i32, u32) {
        if !(RGN_AND..=RGN_COPY).contains(&mode) {
            return (0, ERROR_INVALID_PARAMETER);
        }
        if self.region(dst).is_none() {
            return (0, ERROR_INVALID_HANDLE);
        }
        let Some(a) = self.region(src1) else { return (0, ERROR_INVALID_HANDLE) };
        let out = if mode == RGN_COPY {
            a.clone()
        } else {
            let Some(b) = self.region(src2) else { return (0, ERROR_INVALID_HANDLE) };
            a.combine(b, mode).expect("mode checked")
        };
        let kind = out.kind();
        *self.region_mut(dst).expect("checked") = out;
        (kind, 0)
    }

    /// `GetRgnBox`: (kind, box); `None` when `h` is not a region.
    pub fn rgn_box(&self, h: u64) -> Option<(i32, Rect)> {
        self.region(h).map(|r| (r.kind(), r.bounds()))
    }

    /// `OffsetRgn`: the kind, or ERROR (0) when `h` is not a region or a
    /// coordinate would leave -2^27..2^27-1 (the region stays as it was).
    pub fn offset_rgn(&mut self, h: u64, dx: i32, dy: i32) -> i32 {
        let Some(rgn) = self.region_mut(h) else { return 0 };
        let mut moved = rgn.clone();
        let fits = moved.offset(dx, dy)
            && moved.rects().iter().all(|r| [r.left, r.top, r.right, r.bottom].into_iter().all(rgn_coord_ok));
        if !fits {
            return 0;
        }
        *rgn = moved;
        rgn.kind()
    }

    /// `EqualRgn`: 1 when both are regions with the same pixels.
    pub fn equal_rgn(&self, a: u64, b: u64) -> i32 {
        match (self.region(a), self.region(b)) {
            (Some(a), Some(b)) => i32::from(a == b),
            _ => 0,
        }
    }

    /// `GetRegionData`'s bytes: an `RGNDATAHEADER` (`wingdi.h`: dwSize 32,
    /// iType RDH_RECTANGLES 1, nCount, nRgnSize, rcBound) and the
    /// rectangles.
    pub fn region_data(&self, h: u64) -> Option<Vec<u8>> {
        const RDH_RECTANGLES: u32 = 1;
        let rgn = self.region(h)?;
        let mut b = Vec::with_capacity(32 + 16 * rgn.rects().len());
        let n = rgn.rects().len() as u32;
        for v in [32, RDH_RECTANGLES, n, 16 * n] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for r in core::iter::once(rgn.bounds()).chain(rgn.rects().iter().copied()) {
            for v in [r.left, r.top, r.right, r.bottom] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        Some(b)
    }

    /// `PtInRegion`: whether the point is in it, or the error.
    pub fn pt_in_region(&self, h: u64, x: i32, y: i32) -> Result<bool, u32> {
        self.region(h).map(|r| r.contains_point(x, y)).ok_or(ERROR_INVALID_HANDLE)
    }

    /// `RectInRegion`: `None` when `h` is not a region (no error set).
    pub fn rect_in_region(&self, h: u64, r: Rect) -> Option<bool> {
        self.region(h).map(|g| g.intersects_rect(r))
    }

    // ----- clipping (`GetClipBox.yaml`, `SelectClipRgn.yaml`)

    /// What a DC can draw on, in device coordinates, before its clip
    /// region: a memory DC's bitmap; a window DC's visible region; the
    /// surface bitmap or the screen for display DCs nobody described.
    fn surface_region(&self, d: &Dc) -> Region {
        let bitmap = match d.kind {
            DcKind::Memory => Some(d.bitmap),
            DcKind::Display { .. } => d.surface,
        };
        let base = match (&d.vis, bitmap.and_then(|h| self.obj(u64::from(h)))) {
            (Some(v), _) if d.kind != DcKind::Memory => v.clone(),
            (_, Some(Obj::Bitmap { bmp, .. })) => Region::from_rect(Rect::new(0, 0, bmp.width, bmp.height)),
            _ => Region::from_rect(Rect::new(0, 0, self.screen.width, self.screen.height)),
        };
        match &d.sys_clip {
            Some(s) => base.intersect(s),
            None => base,
        }
    }

    /// What drawing on a DC reaches, in device coordinates: the surface
    /// within the clip region.
    fn drawable(&self, d: &Dc) -> Region {
        let s = self.surface_region(d);
        match &d.state.clip {
            Some(c) => s.intersect(c),
            None => s,
        }
    }

    /// `GetClipBox`: (kind, box in logical coordinates); kind 0 for a bad
    /// DC.
    pub fn clip_box(&self, dc: u64) -> (i32, Rect) {
        let Some(d) = self.dc(dc) else { return (0, Rect::default()) };
        let r = self.drawable(d);
        if r.is_empty() {
            return (NULLREGION, Rect::default());
        }
        let (dx, dy) = d.state.to_device();
        (r.kind(), r.bounds().offset(dx.wrapping_neg(), dy.wrapping_neg()))
    }

    /// The kind of what a DC draws on now (the clip calls' result).
    fn drawable_kind(&self, dc: u64) -> i32 {
        self.dc(dc).map_or(0, |d| self.drawable(d).kind())
    }

    /// `SelectClipRgn`: (kind, error). A copy of the region becomes the
    /// clip region (device coordinates); none removes it (SIMPLEREGION); a
    /// handle that is no region fails without an error.
    pub fn select_clip_rgn(&mut self, dc: u64, rgn: u64) -> (i32, u32) {
        self.ext_select_clip_rgn(dc, rgn, RGN_COPY)
    }

    /// `ExtSelectClipRgn`: (kind, error). The modes combine the region with
    /// the clip region (none: the whole surface); RGN_COPY replaces it.
    pub fn ext_select_clip_rgn(&mut self, dc: u64, rgn: u64, mode: i32) -> (i32, u32) {
        let Some(d) = self.dc(dc) else { return (0, ERROR_INVALID_HANDLE) };
        if !(RGN_AND..=RGN_COPY).contains(&mode) {
            return (0, ERROR_INVALID_PARAMETER);
        }
        let new = if rgn == 0 {
            if mode != RGN_COPY {
                return (0, 0);
            }
            None
        } else {
            let Some(r) = self.region(rgn) else { return (0, 0) };
            if mode == RGN_COPY {
                Some(r.clone())
            } else {
                let base = d.state.clip.clone().unwrap_or_else(|| self.surface_region(d));
                base.combine(r, mode)
            }
        };
        let removed = new.is_none();
        self.dc_mut(dc).expect("checked").state.clip = new;
        if removed { (SIMPLEREGION, 0) } else { (self.drawable_kind(dc), 0) }
    }

    /// `GetClipRgn`: 1 and the clip region copied into `rgn`, 0 without a
    /// clip region, -1 for a bad DC or region (with the error).
    pub fn clip_rgn(&mut self, dc: u64, rgn: u64) -> (i32, u32) {
        let Some(d) = self.dc(dc) else { return (-1, ERROR_INVALID_PARAMETER) };
        let Some(c) = d.state.clip.clone() else { return (0, 0) };
        let Some(out) = self.region_mut(rgn) else { return (-1, ERROR_INVALID_HANDLE) };
        *out = c;
        (1, 0)
    }

    /// `IntersectClipRect` (`exclude` false) and `ExcludeClipRect`: the
    /// clip region (none: the surface) within / less a logical rectangle;
    /// (COMPLEXREGION, or NULLREGION when nothing is left, observed; error).
    pub fn clip_rect(&mut self, dc: u64, r: Rect, exclude: bool) -> (i32, u32) {
        let Some(d) = self.dc(dc) else { return (0, ERROR_INVALID_HANDLE) };
        let (dx, dy) = d.state.to_device();
        let r = Region::from_rect(r.normalized().offset(dx, dy));
        let base = d.state.clip.clone().unwrap_or_else(|| self.surface_region(d));
        let new = if exclude { base.subtract(&r) } else { base.intersect(&r) };
        self.dc_mut(dc).expect("checked").state.clip = Some(new);
        (if self.drawable_kind(dc) == NULLREGION { NULLREGION } else { COMPLEXREGION }, 0)
    }

    /// `GetRandomRgn`: SYSRGN copies the system region (what the DC can
    /// draw on, in screen coordinates) into `rgn`: 1; -1 on failure.
    pub fn random_rgn(&mut self, dc: u64, rgn: u64, which: i32) -> i32 {
        let Some(d) = self.dc(dc) else { return -1 };
        if which != SYSRGN {
            return -1;
        }
        let sys = self.surface_region(d).offset_by(d.vis_origin.0, d.vis_origin.1);
        let Some(out) = self.region_mut(rgn) else { return -1 };
        *out = sys;
        1
    }

    /// `SetViewportOrgEx` (`viewport`) / `SetWindowOrgEx`: the old origin.
    pub fn set_origin(&mut self, dc: u64, viewport: bool, x: i32, y: i32) -> Option<(i32, i32)> {
        let s = self.state(dc)?;
        let o = if viewport { &mut s.viewport_org } else { &mut s.window_org };
        Some(core::mem::replace(o, (x, y)))
    }

    /// Describe a window DC (user32's `GetDC`/`GetWindowDC`): device 0,0
    /// is at `origin` on the screen and the DC can draw on the union of
    /// `include` less the union of `exclude` (screen coordinates; empty or
    /// inverted rectangles count as nothing). False for a DC that is not a
    /// display DC.
    pub fn set_display_geometry(&mut self, dc: u64, origin: (i32, i32), include: &[Rect], exclude: &[Rect]) -> bool {
        let Some(d) = self.dc_mut(dc) else { return false };
        if d.kind == DcKind::Memory {
            return false;
        }
        let union = |rs: &[Rect]| {
            let solid: Vec<Rect> = rs.iter().filter(|r| !r.is_empty()).copied().collect();
            Region::from_rects(&solid)
        };
        let vis = union(include).subtract(&union(exclude));
        d.vis = Some(vis.offset_by(origin.0.wrapping_neg(), origin.1.wrapping_neg()));
        d.vis_origin = origin;
        // A DC taken anew is not limited to an update region
        // (`GetClipBox.yaml`, `begin_paint`).
        d.sys_clip = None;
        true
    }

    /// Set (`Some`) or remove a display DC's system clip: a region in
    /// device coordinates (`BeginPaint`'s update region) that limits what
    /// it draws on as the visible region does. `GetClipBox` reflects it,
    /// `GetClipRgn` does not (`BeginPaint.yaml`). False for a DC that is
    /// not a display DC.
    pub fn set_system_clip(&mut self, dc: u64, clip: Option<Region>) -> bool {
        let Some(d) = self.dc_mut(dc) else { return false };
        if d.kind == DcKind::Memory {
            return false;
        }
        d.sys_clip = clip;
        true
    }

    // ----- drawing

    fn target<'a>(&'a self, d: &Dc) -> Option<&'a Bitmap> {
        let h = match d.kind {
            DcKind::Memory => d.bitmap,
            DcKind::Display { .. } => d.surface?,
        };
        match self.obj(u64::from(h)) {
            // The default 1x1 bitmap is not drawn on (`SetPixel.yaml`).
            Some(Obj::Bitmap { bmp, stock: false, .. }) => Some(bmp),
            _ => None,
        }
    }

    /// The bitmap a DC draws on, with the DC's state and what drawing may
    /// reach (device coordinates: the visible part of the surface within
    /// the clip region and the bitmap).
    fn with_target<R>(&mut self, dc: u64, f: impl FnOnce(&mut Bitmap, &DcState, &Region, &Gdi) -> R) -> Option<R> {
        let d = self.dc(dc)?;
        let state = d.state.clone();
        let drawable = self.drawable(d);
        let h = match d.kind {
            DcKind::Memory => d.bitmap,
            DcKind::Display { .. } => d.surface?,
        };
        let i = self.index_of(u64::from(h))?;
        let mut taken = self.objs[i].take();
        let r = match &mut taken {
            Some(Obj::Bitmap { bmp, stock: false, .. }) => {
                let clip = drawable.intersect_rect(Rect::new(0, 0, bmp.width, bmp.height));
                Some(f(bmp, &state, &clip, self))
            }
            _ => None,
        };
        self.objs[i] = taken;
        r
    }

    /// The colour a brush paints in (`None`: a null brush).
    fn brush_color(&self, brush: u32, state: &DcState) -> Option<Rgb> {
        match self.obj(u64::from(brush)) {
            Some(Obj::Brush { style: BS_NULL, .. }) => None,
            // A deleted solid brush stays readable and can be selected (the
            // old selection comes back), but paints nothing (observed: a
            // PatBlt with it selected leaves the white destination); one
            // deleted while selected stops painting at once too (observed;
            // it goes when deselected).
            Some(Obj::Brush { deleted: true, .. }) | Some(Obj::Brush { delete_pending: true, .. }) => None,
            Some(Obj::Brush { .. }) if brush == self.stock(DC_BRUSH) => Some(Rgb::from_colorref(state.dc_brush)),
            Some(Obj::Brush { color, .. }) => Some(Rgb::from_colorref(*color)),
            _ => None,
        }
    }

    fn pen_color(&self, pen: u32, state: &DcState) -> Option<Rgb> {
        match self.obj(u64::from(pen)) {
            Some(Obj::Pen { style: PS_NULL, .. }) => None,
            Some(Obj::Pen { .. }) if pen == self.stock(DC_PEN) => Some(Rgb::from_colorref(state.dc_pen)),
            Some(Obj::Pen { color, .. }) => Some(Rgb::from_colorref(*color)),
            _ => None,
        }
    }

    /// Logical point (`x`, `y`) of a DC in device coordinates.
    fn device_point(&self, dc: u64, x: i32, y: i32) -> (i32, i32) {
        let (dx, dy) = self.dc(dc).map_or((0, 0), |d| d.state.to_device());
        (x.wrapping_add(dx), y.wrapping_add(dy))
    }

    /// A logical rectangle of a DC in device coordinates.
    fn device_rect(&self, dc: u64, r: Rect) -> Rect {
        let (dx, dy) = self.dc(dc).map_or((0, 0), |d| d.state.to_device());
        r.offset(dx, dy)
    }

    /// `SetPixel`: the colour stored, `CLR_INVALID` outside.
    pub fn set_pixel(&mut self, dc: u64, x: i32, y: i32, c: u32) -> u32 {
        let (x, y) = self.device_point(dc, x, y);
        self.with_target(dc, |b, _, clip, _| {
            if !clip.contains_point(x, y) {
                return CLR_INVALID;
            }
            let v = b.from_rgb(Rgb::from_colorref(c));
            b.set_raw(x, y, v);
            b.to_rgb(b.raw(x, y)).colorref()
        })
        .unwrap_or(CLR_INVALID)
    }

    /// `GetPixel`.
    pub fn get_pixel(&mut self, dc: u64, x: i32, y: i32) -> u32 {
        let (x, y) = self.device_point(dc, x, y);
        self.with_target(dc, |b, _, _, _| if b.contains(x, y) { b.to_rgb(b.raw(x, y)).colorref() } else { CLR_INVALID })
            .unwrap_or(CLR_INVALID)
    }

    /// Fill `r` with raster operation `code` and pattern colour `pattern`,
    /// within `clip`.
    fn fill(b: &mut Bitmap, clip: &Region, r: Rect, code: u8, pattern: Option<Rgb>) {
        let p = pattern.map_or(0, |c| b.from_rgb(c));
        let mask = b.raw_mask();
        for piece in clip.rects() {
            let r = r.intersect(piece);
            if r.is_empty() {
                continue;
            }
            for y in r.top..r.bottom {
                for x in r.left..r.right {
                    let d = b.raw(x, y);
                    b.set_raw(x, y, rop3(code, p, 0, d) & mask);
                }
            }
        }
    }

    /// `PatBlt`.
    pub fn pat_blt(&mut self, dc: u64, x: i32, y: i32, w: i32, h: i32, rop: u32) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        let state = d.state.clone();
        let pattern = self.brush_color(state.brush, &state);
        let code = (rop >> 16) as u8;
        // A null brush paints nothing with a pattern operation.
        let uses_pattern = (code >> 4) & 0xF != code & 0xF;
        if uses_pattern && pattern.is_none() {
            return true;
        }
        let r = self.device_rect(dc, normalize(x, y, w, h));
        self.with_target(dc, |b, _, clip, _| Self::fill(b, clip, r, code, pattern));
        true
    }

    /// `FillRect` with `brush` (0: the DC's), not changing the DC.
    pub fn fill_rect(&mut self, dc: u64, r: Rect, brush: u64) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        let state = d.state.clone();
        let b = if brush == 0 { state.brush } else { brush as u32 };
        let Some(c) = self.brush_color(b, &state) else { return true };
        let r = self.device_rect(dc, normalized(r));
        self.with_target(dc, |bmp, _, clip, _| Self::fill(bmp, clip, r, 0xF0, Some(c)));
        true
    }

    /// `FillRect` with a system colour (`COLOR_* + 1` as the brush).
    pub fn fill_rect_color(&mut self, dc: u64, r: Rect, color: u32) -> bool {
        if self.dc(dc).is_none() {
            return false;
        }
        let r = self.device_rect(dc, normalized(r));
        self.with_target(dc, |bmp, _, clip, _| Self::fill(bmp, clip, r, 0xF0, Some(Rgb::from_colorref(color))));
        true
    }

    /// `Rectangle`: the pen's 1-pixel outline, the brush inside; without a
    /// pen the filled area is a pixel smaller right and bottom.
    pub fn rectangle(&mut self, dc: u64, l: i32, t: i32, r: i32, b: i32) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        let state = d.state.clone();
        let fill = self.brush_color(state.brush, &state);
        let pen = self.pen_color(state.pen, &state);
        let n = self.device_rect(dc, Rect::new(l, t, r, b).normalized());
        let (l, t, r, b) = (n.left, n.top, n.right, n.bottom);
        self.with_target(dc, |bmp, _, clip, _| match pen {
            Some(pc) => {
                if let Some(fc) = fill {
                    Self::fill(bmp, clip, Rect::new(l + 1, t + 1, r - 1, b - 1), 0xF0, Some(fc));
                }
                for edge in [
                    Rect::new(l, t, r, t + 1),
                    Rect::new(l, b - 1, r, b),
                    Rect::new(l, t, l + 1, b),
                    Rect::new(r - 1, t, r, b),
                ] {
                    Self::fill(bmp, clip, edge, 0xF0, Some(pc));
                }
            }
            None => {
                if let Some(fc) = fill {
                    Self::fill(bmp, clip, Rect::new(l, t, r - 1, b - 1), 0xF0, Some(fc));
                }
            }
        });
        true
    }

    /// `LineTo`: a 1-pixel line from the current position, which moves.
    pub fn line_to(&mut self, dc: u64, x: i32, y: i32) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        let state = d.state.clone();
        let pen = self.pen_color(state.pen, &state);
        if let Some(c) = pen {
            let (x0, y0) = self.device_point(dc, state.pos.0, state.pos.1);
            let (x, y) = self.device_point(dc, x, y);
            self.with_target(dc, |b, _, clip, _| {
                let v = b.from_rgb(c);
                let (dx, dy) = ((x - x0).abs(), -(y - y0).abs());
                let (sx, sy) = (if x0 < x { 1 } else { -1 }, if y0 < y { 1 } else { -1 });
                let (mut cx, mut cy, mut e) = (x0, y0, dx + dy);
                while (cx, cy) != (x, y) {
                    if clip.contains_point(cx, cy) {
                        b.set_raw(cx, cy, v);
                    }
                    let e2 = 2 * e;
                    if e2 >= dy {
                        e += dy;
                        cx += sx;
                    }
                    if e2 <= dx {
                        e += dx;
                        cy += sy;
                    }
                }
            });
        }
        if let Some(s) = self.state(dc) {
            s.pos = (x, y);
        }
        true
    }

    /// `BitBlt` (`BitBlt.yaml`): (result, error).
    #[allow(clippy::too_many_arguments)]
    pub fn bit_blt(
        &mut self,
        dst: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src: u64,
        sx: i32,
        sy: i32,
        rop: u32,
    ) -> (bool, u32) {
        let Some(d) = self.dc(dst) else { return (false, ERROR_INVALID_HANDLE) };
        let dstate = d.state.clone();
        let code = (rop >> 16) as u8;
        let uses_source = (code >> 2) & 0x33 != code & 0x33;
        if !uses_source {
            let pattern = self.brush_color(dstate.brush, &dstate);
            let r = self.device_rect(dst, normalize(x, y, w, h));
            self.with_target(dst, |b, _, clip, _| Self::fill(b, clip, r, code, pattern));
            return (true, 0);
        }
        let Some(sdc) = self.dc(src) else { return (false, 0) };
        let sbk = Rgb::from_colorref(sdc.state.bk);
        let pattern = self.brush_color(dstate.brush, &dstate);
        let (x, y) = self.device_point(dst, x, y);
        let (sx, sy) = self.device_point(src, sx, sy);
        let (mut x, mut y, mut sx, mut sy, mut w, mut h) = (x, y, sx, sy, w, h);
        if w < 0 {
            x += w;
            sx += w;
            w = -w;
        }
        if h < 0 {
            y += h;
            sy += h;
            h = -h;
        }
        if w == 0 || h == 0 {
            return (true, 0);
        }
        // Copy the source pixels first (one DC may be both).
        let Some(s) = self.target(sdc) else { return (true, 0) };
        let src_view = Src { bmp: s, bk: sbk };
        let mut cells: Vec<Option<(Rgb, u32, bool)>> = Vec::with_capacity((w * h) as usize);
        for j in 0..h {
            for i in 0..w {
                let (px, py) = (sx + i, sy + j);
                cells.push(if src_view.bmp.contains(px, py) {
                    let raw = src_view.bmp.raw(px, py);
                    let rgb = src_view.bmp.to_rgb(raw);
                    let is_bk = rgb == src_view.bk;
                    Some((rgb, raw, is_bk))
                } else {
                    None
                });
            }
        }
        let src_bpp = s.bpp;
        let src_dib = s.dib;
        let src_colors = s.colors.clone();
        let src_mono = !s.dib && s.bpp == 1;
        let text = Rgb::from_colorref(dstate.text);
        let bk = Rgb::from_colorref(dstate.bk);
        self.with_target(dst, |b, _, clip, _| {
            let p = pattern.map_or(0, |c| b.from_rgb(c));
            let mask = b.raw_mask();
            for j in 0..h {
                for i in 0..w {
                    let (dx, dy) = (x + i, y + j);
                    if !clip.contains_point(dx, dy) {
                        continue;
                    }
                    let Some((rgb, raw, is_bk)) = cells[(j * w + i) as usize] else { continue };
                    let s = if src_mono && !(b.bpp == 1 && !b.dib) {
                        b.from_rgb(if raw & 1 == 0 { text } else { bk })
                    } else if b.bpp == 1 && !b.dib && !src_mono {
                        u32::from(is_bk)
                    } else if src_bpp == b.bpp && (b.bpp >= 16 || (src_dib == b.dib && src_colors == b.colors)) {
                        raw
                    } else {
                        b.from_rgb(rgb)
                    };
                    let d = b.raw(dx, dy);
                    b.set_raw(dx, dy, rop3(code, p, s, d) & mask);
                }
            }
        });
        (true, 0)
    }

    // ----- text

    pub(crate) fn face_of(&self, dc: u64) -> Face {
        let Some(d) = self.dc(dc) else { return Face::System };
        match self.obj(u64::from(d.state.font)) {
            Some(Obj::Font { face, .. }) => *face,
            _ => Face::System,
        }
    }

    /// `GetTextMetricsW`.
    pub fn text_metrics(&self, dc: u64) -> Option<TextMetric> {
        self.dc(dc)?;
        Some(self.face_of(dc).metrics())
    }

    /// `GetTextExtentPoint32W`.
    pub fn text_extent(&self, dc: u64, text: &[u16]) -> Option<(i32, i32)> {
        self.dc(dc)?;
        Some(font::extent(self.face_of(dc), text))
    }

    /// `TextOutW` (`TextOutW.yaml`): false for a bad DC.
    pub fn text_out(&mut self, dc: u64, x: i32, y: i32, text: &[u16]) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        if text.is_empty() {
            return true;
        }
        let state = d.state.clone();
        let face = self.face_of(dc);
        let tm = face.metrics();
        let (cx, _) = font::extent(face, text);
        let (rx, ry) = if state.align & TA_UPDATECP != 0 { state.pos } else { (x, y) };
        let x0 = match state.align & TA_CENTER {
            TA_RIGHT => rx - cx,
            TA_CENTER => rx - cx / 2,
            _ => rx,
        };
        let y0 = match state.align & TA_BASELINE {
            TA_BOTTOM => ry - tm.height,
            TA_BASELINE => ry - tm.ascent,
            _ => ry,
        };
        self.draw_text_run(dc, x0, y0, text, None);
        if state.align & TA_UPDATECP != 0
            && let Some(s) = self.state(dc)
        {
            s.pos.0 = match state.align & TA_CENTER {
                TA_RIGHT => rx - cx,
                TA_CENTER => rx,
                _ => rx + cx,
            };
        }
        true
    }

    /// Draw `text` with its cell's top left at (`x0`, `y0`), clipped to
    /// `clip` too when given (`DrawTextW`).
    pub fn draw_text_run(&mut self, dc: u64, x0: i32, y0: i32, text: &[u16], clip: Option<Rect>) {
        let face = self.face_of(dc);
        let tm = face.metrics();
        let (x0, y0) = self.device_point(dc, x0, y0);
        let clip = clip.map(|c| self.device_rect(dc, c));
        self.with_target(dc, |b, state, dclip, _| {
            let clip = match clip {
                Some(c) => dclip.intersect_rect(c),
                None => dclip.clone(),
            };
            let fg = b.from_rgb(Rgb::from_colorref(state.text));
            let (cx, _) = font::extent(face, text);
            if state.bk_mode != TRANSPARENT {
                Self::fill(
                    b,
                    &clip,
                    Rect::new(x0, y0, x0 + cx, y0 + tm.height),
                    0xF0,
                    Some(Rgb::from_colorref(state.bk)),
                );
            }
            let mut x = x0;
            for &c in text {
                let (w, rows) = face.glyph(c);
                for (r, &row) in rows.iter().enumerate().take(tm.height as usize) {
                    if row == 0 {
                        continue;
                    }
                    for col in 0..w.min(16) {
                        if row & (0x8000 >> col) != 0 {
                            let (px, py) = (x + col, y0 + r as i32);
                            if clip.contains_point(px, py) {
                                b.set_raw(px, py, fg);
                            }
                        }
                    }
                }
                x += w;
            }
        });
    }

    /// Point a display DC at a window surface bitmap (the desktop link):
    /// `bitmap` is a DIB made with [`Gdi::create_dib`].
    pub fn set_surface(&mut self, dc: u64, bitmap: Option<u32>) {
        if let Some(d) = self.dc_mut(dc) {
            d.surface = bitmap;
        }
    }
}

/// `FillRect`'s rectangle: an inverted one is filled with its sides
/// swapped (observed: 9,5 to 8,4 fills the pixel at 8,4); an empty one
/// stays empty.
fn normalized(r: Rect) -> Rect {
    Rect::new(r.left.min(r.right), r.top.min(r.bottom), r.left.max(r.right), r.top.max(r.bottom))
}

fn normalize(x: i32, y: i32, w: i32, h: i32) -> Rect {
    let (l, r) = if w < 0 { (x + w, x) } else { (x, x + w) };
    let (t, b) = if h < 0 { (y + h, y) } else { (y, y + h) };
    Rect::new(l, t, r, b)
}
