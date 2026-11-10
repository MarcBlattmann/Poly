//! A process's GDI: its objects (DCs, bitmaps, brushes, pens, fonts,
//! regions), their handles, and the drawing calls of the first slice
//! (`specs/windows/gdi32.dll/`). `gdi32.dll` keeps one [`Gdi`] behind a lock
//! and maps each export onto a method; errors come back as the Win32 code
//! the export sets (0: none).

use crate::bitmap::{Bitmap, Rgb, Storage, rop3};
use crate::font::{Face, TextMetric};
use crate::fontmap::{self, LogFont};
use crate::fonttab::{self, FontTable};
use crate::outline::{FaceStyle, FontFiles, Outline, Src as FontSrc, Typeface};
pub use crate::region::Rect;
use crate::region::{COMPLEXREGION, NULLREGION, RGN_AND, RGN_COPY, Region, SIMPLEREGION};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use poly_font::GlyphCache;

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

/// `ExtTextOutW` options (`wingdi.h`).
pub const ETO_OPAQUE: u32 = 0x0002;
pub const ETO_CLIPPED: u32 = 0x0004;
pub const ETO_GLYPH_INDEX: u32 = 0x0010;

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
/// `BS_HATCHED`, `BS_PATTERN`, `BS_DIBPATTERNPT` (wingdi.h).
pub const BS_HATCHED: u32 = 2;
pub const BS_PATTERN: u32 = 3;
pub const BS_DIBPATTERNPT: u32 = 6;
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
    /// The palette `SelectPalette` selected (the stock one at first).
    palette: u32,
    /// The DC has drawn with the selected brush / pen since it was selected
    /// (or the state was saved with it): what a brush or pen deleted while
    /// selected still draws with (DeleteObject.yaml `selected_brush_pen`).
    brush_used: bool,
    pen_used: bool,
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
    /// A deleted object whose handle still answers `GetObjectType` with its
    /// old type and nothing else (DeleteObject.yaml `type_after_delete`,
    /// `after_deselect`): pens, fonts, bitmaps, regions, palettes.
    Dead(u32),
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
        /// The bitmap face of a stock font.
        face: Face,
        stock: bool,
        logfont: Vec<u8>,
        /// What the font mapper made of `logfont`, with the font table's
        /// generation it was made at (a font added or removed since
        /// maps the font again).
        mapped: Cell<Option<Mapped>>,
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
/// `DEFAULT_PALETTE` (wingdi.h).
pub const DEFAULT_PALETTE: u32 = 15;
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
/// How many freed slots wait before the oldest is used again: a deleted
/// object's handle keeps answering `GetObjectType` (DeleteObject.yaml: the
/// old type "forever") until its slot comes round.
const DEAD_SLOTS: usize = 256;

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
    /// Freed slots, oldest first (see [`DEAD_SLOTS`]).
    free: VecDeque<usize>,
    /// The common DC a window released last (window, slot): the window's
    /// next DC is that one again, under the same handle value
    /// (BeginPaint.yaml `dc_value`: consecutive paints of one window see
    /// the same HDC). Released, the handle names nothing meanwhile.
    parked: Option<(u64, usize)>,
    pub screen: Screen,
    /// The font files and the rendered glyphs of outline fonts.
    text: TextState,
}

/// A logical font as the mapper resolved it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mapped {
    generation: u32,
    face: Face,
    outline: Option<Outline>,
}

/// What outline text needs besides the font objects.
struct TextState {
    /// The font store's files and the built-in fonts.
    files: RefCell<FontFiles>,
    /// The fonts the program added.
    table: FontTable,
    glyphs: Option<GlyphCache>,
}

impl core::fmt::Debug for TextState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextState")
            .field("glyph_bytes", &self.glyphs.as_ref().map(GlyphCache::bytes))
            .finish_non_exhaustive()
    }
}

/// The glyph cache's budget: enough for a few hundred glyphs at
/// ordinary sizes.
const GLYPH_CACHE_BYTES: usize = 1 << 20;

impl Default for Gdi {
    fn default() -> Self {
        Gdi::new()
    }
}

/// The font a DC has selected, with the file of an outline font.
pub(crate) struct FontSel {
    face: Face,
    outline: Option<Outline>,
    data: Option<Arc<Vec<u8>>>,
    underline: bool,
    strike: bool,
}

impl FontSel {
    /// To measure and draw with.
    pub(crate) fn typeface(&self) -> Typeface<'_> {
        Typeface::new(self.face, self.outline, self.data.as_deref().map(Vec::as_slice))
    }

    /// `TEXTMETRICW` of the font, with the `LOGFONTW`'s underline and
    /// strike-out where a bitmap face stands in.
    pub(crate) fn metrics(&self) -> TextMetric {
        let mut tm = self.typeface().metrics();
        if self.underline {
            tm.underlined = 255;
        }
        if self.strike {
            tm.struck_out = 255;
        }
        tm
    }
}

/// A pixel source for blits: a bitmap and its DC's colours.
struct Src<'a> {
    bmp: &'a Bitmap,
    bk: Rgb,
}

impl Gdi {
    pub fn new() -> Gdi {
        let mut g = Gdi {
            objs: Vec::new(),
            gens: Vec::new(),
            free: VecDeque::new(),
            parked: None,
            screen: Screen { width: 1024, height: 768 },
            text: TextState {
                files: RefCell::new(FontFiles::new()),
                table: FontTable::new(),
                glyphs: Some(GlyphCache::new(GLYPH_CACHE_BYTES)),
            },
        };
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
        let font = |f: Face| Obj::Font {
            face: f,
            stock: true,
            logfont: Vec::new(),
            mapped: Cell::new(None),
            delete_pending: false,
        };
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
        let reuse = if self.free.len() > DEAD_SLOTS { self.free.pop_front() } else { None };
        let i = match reuse {
            Some(i) => {
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

    /// Whether `h` is a live DC.
    pub fn is_dc(&self, h: u64) -> bool {
        self.dc(h).is_some()
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
            palette: self.stock(DEFAULT_PALETTE),
            brush_used: false,
            pen_used: false,
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
        let dc = Obj::Dc(Dc {
            kind: DcKind::Display { hwnd, persistent },
            state,
            saved: Vec::new(),
            bitmap: 0,
            surface: None,
            vis: None,
            vis_origin: (0, 0),
            sys_clip: None,
        });
        // The window's last common DC again (in its default state), unless
        // another window took a DC since.
        if !persistent && hwnd != 0 {
            match self.parked.take() {
                Some((w, i)) if w == hwnd && self.objs[i].is_none() => {
                    self.objs[i] = Some(dc);
                    return self.handle_of(i);
                }
                Some((_, i)) => self.free.push_back(i),
                None => {}
            }
        }
        self.add(dc)
    }

    /// `ReleaseDC`: a common display DC goes, a persistent one stays; 1
    /// when it was a display DC.
    pub fn release_display_dc(&mut self, h: u64) -> bool {
        match self.dc(h).map(|d| d.kind) {
            Some(DcKind::Display { persistent: false, hwnd }) => {
                self.free_dc(h, false);
                // A window's DC waits for the window's next one.
                if hwnd != 0
                    && let Some(i) = self.free.pop_back()
                    && let Some((_, old)) = self.parked.replace((hwnd, i))
                {
                    self.free.push_back(old);
                }
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

    /// A bitmap's size and `B, G, R, A` pixels, top-down, whatever DC it is
    /// selected into (the icon and clipboard calls copy bitmaps with it).
    pub fn bitmap_bgra(&self, h: u64) -> Option<(i32, i32, Vec<u8>)> {
        let Some(Obj::Bitmap { bmp, .. }) = self.obj(h) else { return None };
        let (w, ht) = (bmp.width, bmp.height);
        if w <= 0 || ht <= 0 || w > 1 << 14 || ht > 1 << 14 {
            return None;
        }
        let mut out = Vec::with_capacity((w * ht * 4) as usize);
        for y in 0..ht {
            for x in 0..w {
                let raw = bmp.raw(x, y);
                let c = bmp.to_rgb(raw).colorref();
                let a = if bmp.bpp == 32 { (raw >> 24) as u8 } else { 255 };
                out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, a]);
            }
        }
        Some((w, ht, out))
    }

    /// A display DC's origin on the screen (`None`: not a display DC).
    pub fn display_origin(&self, h: u64) -> Option<(i32, i32)> {
        let d = self.dc(h)?;
        matches!(d.kind, DcKind::Display { .. }).then_some(d.vis_origin)
    }

    /// Slot `i`'s object is gone: its handle answers `GetObjectType` with
    /// `ty` (0: nothing) until the slot is used again.
    fn release(&mut self, i: usize, ty: u32) {
        self.objs[i] = if ty == 0 { None } else { Some(Obj::Dead(ty)) };
        self.free.push_back(i);
    }

    fn free_dc(&mut self, h: u64, zombie: bool) {
        let Some(i) = self.index_of(h) else { return };
        let Some(Obj::Dc(d)) = self.objs[i].take() else { return };
        // Objects selected into it (or into a saved state of it) whose
        // deletion waited go now.
        for st in core::iter::once(&d.state).chain(d.saved.iter()) {
            for o in [st.brush, st.pen, st.font] {
                self.finish_pending(u64::from(o));
            }
        }
        if let Some(Obj::Bitmap { selected_in, .. }) = self.obj_mut(u64::from(d.bitmap)) {
            *selected_in = 0;
        }
        self.finish_pending(u64::from(d.bitmap));
        if zombie {
            self.objs[i] = Some(Obj::DeadDc);
        } else {
            self.free.push_back(i);
        }
    }

    /// An object whose deletion waited because it was selected: gone once
    /// no DC holds it. A brush leaves nothing (type 0); a pen, a font and a
    /// bitmap keep answering their type (DeleteObject.yaml `after_deselect`,
    /// `selected_bitmap`).
    fn finish_pending(&mut self, h: u64) {
        let held = self.held(h);
        let Some(i) = self.index_of(h) else { return };
        let (pending, ty) = match &self.objs[i] {
            Some(Obj::Brush { delete_pending, .. }) => (*delete_pending, 0),
            Some(Obj::Pen { delete_pending, .. }) => (*delete_pending, OBJ_PEN),
            Some(Obj::Font { delete_pending, .. }) => (*delete_pending, OBJ_FONT),
            Some(Obj::Bitmap { delete_pending, selected_in, .. }) => (*delete_pending && *selected_in == 0, OBJ_BITMAP),
            _ => (false, 0),
        };
        if pending && !held {
            self.release(i, ty);
        }
    }

    /// Whether a DC has `h` selected, now or in a state `SaveDC` kept.
    fn held(&self, h: u64) -> bool {
        let h = h as u32;
        self.objs.iter().flatten().any(|o| match o {
            Obj::Dc(d) => {
                d.bitmap == h
                    || core::iter::once(&d.state)
                        .chain(d.saved.iter())
                        .any(|s| s.brush == h || s.pen == h || s.font == h)
            }
            _ => false,
        })
    }

    /// Whether a DC has palette `h` selected.
    fn palette_held(&self, h: u32) -> bool {
        self.objs.iter().flatten().any(|o| match o {
            Obj::Dc(d) => core::iter::once(&d.state).chain(d.saved.iter()).any(|s| s.palette == h),
            _ => false,
        })
    }

    /// `CreatePalette`: a logical palette (its colours are not kept: Poly's
    /// screens are true-colour).
    pub fn create_palette(&mut self) -> u32 {
        self.add(Obj::Palette)
    }

    /// `SelectPalette`: the palette selected before, 0 when `dc` or `pal`
    /// is not what it should be.
    pub fn select_palette(&mut self, dc: u64, pal: u64) -> u32 {
        if !matches!(self.obj(pal), Some(Obj::Palette)) {
            return 0;
        }
        match self.state(dc) {
            Some(s) => core::mem::replace(&mut s.palette, pal as u32),
            None => 0,
        }
    }

    /// `CreateHatchBrush` / `CreatePatternBrush` / `CreateDIBPatternBrushPt`:
    /// a brush of that style. It paints as a solid brush of `color` for
    /// now (the hatch lines' colour; black for a pattern).
    pub fn create_styled_brush(&mut self, style: u32, color: u32, hatch: u64) -> u32 {
        self.add(Obj::Brush { style, color, hatch, stock: false, deleted: false, delete_pending: false })
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

    /// `DeleteObject` (DeleteObject.yaml). A stock object stays; an object
    /// a DC has selected waits (`delete_pending`) and goes when it is
    /// deselected; a second delete of either fails.
    pub fn delete_object(&mut self, h: u64) -> bool {
        let held = self.held(h);
        let Some(i) = self.index_of(h) else { return false };
        let stock_slot = i <= STOCK_COUNT as usize;
        let Some(o) = self.objs[i].as_mut() else { return false };
        match o {
            Obj::Dc(_) => {
                self.free_dc(h, true);
                true
            }
            Obj::DeadDc | Obj::Dead(_) => false,
            Obj::Brush { stock: true, .. } | Obj::Pen { stock: true, .. } | Obj::Font { stock: true, .. } => true,
            Obj::Bitmap { stock: true, .. } => true,
            Obj::Palette if stock_slot => true,
            Obj::Brush { deleted, delete_pending, .. } => {
                if *deleted || *delete_pending {
                    return false;
                }
                if held {
                    *delete_pending = true;
                } else {
                    // Observed: a brush deleted while no DC has it stays
                    // readable and can still be selected (it paints white).
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
                    let ty = if matches!(o, Obj::Pen { .. }) { OBJ_PEN } else { OBJ_FONT };
                    self.release(i, ty);
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
                    self.release(i, OBJ_BITMAP);
                }
                true
            }
            Obj::Region(_) => {
                self.release(i, OBJ_REGION);
                true
            }
            Obj::Palette => {
                // A palette a DC has selected is not deleted: the call
                // fails and it stays (DeleteObject.yaml `palette`).
                if self.palette_held(h as u32) {
                    return false;
                }
                self.release(i, OBJ_PAL);
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
            Some(Obj::Dead(ty)) => *ty,
            Some(Obj::Bitmap { .. }) => OBJ_BITMAP,
            Some(Obj::Brush { .. }) => OBJ_BRUSH,
            Some(Obj::Pen { .. }) => OBJ_PEN,
            Some(Obj::Font { .. }) => OBJ_FONT,
            Some(Obj::Region(_)) => OBJ_REGION,
            Some(Obj::Palette) => OBJ_PAL,
            None => 0,
        }
    }

    /// `GetObjectW` into a buffer of `size` bytes: as [`Gdi::object_bytes`],
    /// but a DIB section answers its `DIBSECTION` (wingdi.h, 104 bytes) when
    /// the buffer holds one (Learn: GetObject; DeleteObject.yaml
    /// `selected_bitmap`), and a font fills a larger buffer than `LOGFONTW`
    /// (observed: 128 bytes for a buffer of 128; the rest of
    /// `ENUMLOGFONTEXW`, 348 bytes, zero here).
    pub fn object_bytes_for(&self, h: u64, size: usize) -> Option<Vec<u8>> {
        let mut b = self.object_bytes(h)?;
        match self.obj(h)? {
            Obj::Bitmap { bmp, .. } if bmp.dib && size >= 104 => {
                let height = bmp.height.unsigned_abs();
                b.resize(104, 0);
                b[32..36].copy_from_slice(&40u32.to_le_bytes());
                b[36..40].copy_from_slice(&bmp.width.to_le_bytes());
                b[40..44].copy_from_slice(&height.to_le_bytes());
                b[44..46].copy_from_slice(&1u16.to_le_bytes());
                b[46..48].copy_from_slice(&bmp.bpp.to_le_bytes());
                b[52..56].copy_from_slice(&((bmp.stride as u32) * height).to_le_bytes());
            }
            Obj::Font { .. } => b.resize(348, 0),
            _ => {}
        }
        Some(b)
    }

    /// Whether `h` is a region (`GetObject` has nothing to say about one:
    /// 0 with ERROR_INVALID_HANDLE, CreateRectRgn.yaml `in_a_dc`).
    pub fn is_region(&self, h: u64) -> bool {
        matches!(self.obj(h), Some(Obj::Region(_)))
    }

    /// `GetObjectW`'s bytes for an object (`LOGBRUSH`, `BITMAP`, `LOGPEN`,
    /// `LOGFONTW`).
    pub fn object_bytes(&self, h: u64) -> Option<Vec<u8>> {
        Some(match self.obj(h)? {
            // A brush or pen deleted while selected answers nothing
            // (DeleteObject.yaml `selected_brush_pen`: GetObject 0).
            Obj::Brush { delete_pending: true, .. } | Obj::Pen { delete_pending: true, .. } => return None,
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
            // An object whose deletion waits for its DC cannot be selected
            // anywhere (DeleteObject.yaml `selected_brush_pen`: NULL).
            Some(Obj::Brush { delete_pending: true, .. })
            | Some(Obj::Pen { delete_pending: true, .. })
            | Some(Obj::Font { delete_pending: true, .. })
            | Some(Obj::Bitmap { delete_pending: true, .. }) => return Ok(0),
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
            OBJ_BRUSH => {
                d.state.brush_used = false;
                core::mem::replace(&mut d.state.brush, h)
            }
            OBJ_PEN => {
                d.state.pen_used = false;
                core::mem::replace(&mut d.state.pen, h)
            }
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
            OBJ_PAL => d.state.palette,
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

    /// `CreateFontIndirectW`: the font mapper (ADR 0142, `crate::fontmap`)
    /// picks the font from the face name, pitch and family and the height:
    /// an outline font of Poly's font store when its file is there (see
    /// [`Gdi::set_font_loader`]), else a bitmap face. A `LOGFONTW` that is
    /// too short gives the system font's face.
    pub fn create_font(&mut self, logfont: &[u8]) -> u32 {
        self.add(Obj::Font {
            face: Face::System,
            stock: false,
            logfont: logfont.to_vec(),
            mapped: Cell::new(None),
            delete_pending: false,
        })
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
    /// `GetCurrentPositionEx`: the current position (`MoveToEx`, `LineTo`,
    /// `TA_UPDATECP` text).
    pub fn current_position(&mut self, dc: u64) -> Option<(i32, i32)> {
        self.state(dc).map(|s| s.pos)
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
        // Saving the state takes the brush and the pen as the DC's own: one
        // that is deleted afterwards still draws in its colour, also when
        // the DC never drew with it (DeleteObject.yaml `selected_brush_pen`).
        let Some(d) = self.dc(dc) else { return 0 };
        let (brush, pen) = (d.state.brush, d.state.pen);
        let brush_alive =
            matches!(self.obj(u64::from(brush)), Some(Obj::Brush { deleted: false, delete_pending: false, .. }));
        let pen_alive = matches!(self.obj(u64::from(pen)), Some(Obj::Pen { delete_pending: false, .. }));
        let Some(d) = self.dc_mut(dc) else { return 0 };
        d.state.brush_used |= brush_alive;
        d.state.pen_used |= pen_alive;
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
        let dropped: Vec<DcState> = d.saved.drain(target as usize + 1..).collect();
        let old = core::mem::replace(&mut d.state, d.saved.pop().expect("present"));
        // What only the dropped states had selected and waits to be deleted
        // goes now.
        for st in dropped.iter().chain(core::iter::once(&old)) {
            for o in [st.brush, st.pen, st.font] {
                self.finish_pending(u64::from(o));
            }
        }
        true
    }

    /// `GetDeviceCaps` (GetSystemMetrics.yaml `device_caps_screen`): the
    /// screen's capabilities, the same for every DC (the screen's, a
    /// window's, a memory DC); an index that is not defined answers 0;
    /// `None`: not a DC (ERROR_INVALID_HANDLE).
    pub fn device_caps(&self, dc: u64, index: i32) -> Option<i32> {
        self.dc(dc)?;
        // Millimetres at 96 pixels per inch, rounded.
        let mm = |px: i32| (px * 254 + 480) / 960;
        Some(match index {
            0 => 0x4000,                    // DRIVERVERSION
            2 => 1,                         // TECHNOLOGY: DT_RASDISPLAY
            4 => mm(self.screen.width),     // HORZSIZE
            6 => mm(self.screen.height),    // VERTSIZE
            8 | 118 => self.screen.width,   // HORZRES, DESKTOPHORZRES
            10 | 117 => self.screen.height, // VERTRES, DESKTOPVERTRES
            12 => 32,                       // BITSPIXEL
            14 => 1,                        // PLANES
            16 | 18 | 24 => -1,             // NUMBRUSHES, NUMPENS, NUMCOLORS
            28 => 511,                      // CURVECAPS
            30 => 254,                      // LINECAPS
            32 => 255,                      // POLYGONALCAPS
            34 => 30727,                    // TEXTCAPS
            36 => 1,                        // CLIPCAPS: CP_RECTANGLE
            38 => 32409,                    // RASTERCAPS
            40 | 42 => 36,                  // ASPECTX, ASPECTY
            44 => 51,                       // ASPECTXY
            88 | 90 => 96,                  // LOGPIXELSX, LOGPIXELSY
            106 => 20,                      // NUMRESERVED
            108 => 24,                      // COLORRES
            116 => 60,                      // VREFRESH
            119 => 1,                       // BLTALIGNMENT
            121 => 2,                       // COLORMGMTCAPS: CM_GAMMA_RAMP
            _ => 0,
        })
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

    /// The system takes a region over (`SetWindowRgn`): its pixels, and the
    /// handle is no object at all from then on (`GetObjectType` 0 at once,
    /// where a deleted region still answers OBJ_REGION for a while;
    /// SetWindowRgn.yaml `ownership`). `None` when `h` is not a region.
    pub fn take_region(&mut self, h: u64) -> Option<Region> {
        let i = self.index_of(h)?;
        if !matches!(self.objs[i], Some(Obj::Region(_))) {
            return None;
        }
        let Some(Obj::Region(r)) = self.objs[i].take() else { return None };
        self.release(i, 0);
        Some(r)
    }

    /// `SetRectRgn`: any coordinates are kept (observed); false when `h`
    /// is not a region.
    pub fn set_rect_rgn(&mut self, h: u64, r: Rect) -> bool {
        // Observed: on a region deleted just before the call still answers
        // 1 (CreateRectRgn.yaml `indirect_and_set`; nothing is changed).
        if matches!(self.obj(h), Some(Obj::Dead(OBJ_REGION))) {
            return true;
        }
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

    /// The colour the DC's selected brush paints in (`None`: a null brush).
    /// A brush deleted while selected keeps its colour if the DC drew with
    /// it before (or saved its state with it); one the DC never used, and
    /// one that was deleted before it was selected, paints as the default
    /// white brush (DeleteObject.yaml `selected_brush_pen`,
    /// `deleted_not_selected_brush`).
    fn brush_color(&self, brush: u32, state: &DcState) -> Option<Rgb> {
        const WHITE: u32 = 0x00FF_FFFF;
        match self.obj(u64::from(brush)) {
            Some(Obj::Brush { style: BS_NULL, .. }) => None,
            Some(Obj::Brush { deleted: true, .. }) => Some(Rgb::from_colorref(WHITE)),
            Some(Obj::Brush { delete_pending: true, color, .. }) => {
                Some(Rgb::from_colorref(if brush == state.brush && state.brush_used { *color } else { WHITE }))
            }
            Some(Obj::Brush { .. }) if brush == self.stock(DC_BRUSH) => Some(Rgb::from_colorref(state.dc_brush)),
            Some(Obj::Brush { color, .. }) => Some(Rgb::from_colorref(*color)),
            _ => None,
        }
    }

    /// The colour a brush named by its handle paints in (`FillRect`,
    /// `FillRgn`): as [`Gdi::brush_color`], but a brush deleted while
    /// selected that the DC has not drawn with paints nothing
    /// (DeleteObject.yaml `selected_brush_pen`).
    fn handle_brush_color(&self, brush: u32, state: &DcState) -> Option<Rgb> {
        match self.obj(u64::from(brush)) {
            Some(Obj::Brush { delete_pending: true, .. }) if !(brush == state.brush && state.brush_used) => None,
            _ => self.brush_color(brush, state),
        }
    }

    /// The colour the DC's selected pen draws in (`None`: a null pen). A pen
    /// deleted while selected: its own colour if the DC drew with it before,
    /// else the default black pen's.
    fn pen_color(&self, pen: u32, state: &DcState) -> Option<Rgb> {
        match self.obj(u64::from(pen)) {
            Some(Obj::Pen { style: PS_NULL, .. }) => None,
            Some(Obj::Pen { delete_pending: true, color, .. }) => {
                Some(Rgb::from_colorref(if state.pen_used { *color } else { 0 }))
            }
            Some(Obj::Pen { .. }) if pen == self.stock(DC_PEN) => Some(Rgb::from_colorref(state.dc_pen)),
            Some(Obj::Pen { color, .. }) => Some(Rgb::from_colorref(*color)),
            _ => None,
        }
    }

    /// The DC draws with its selected brush (`brush`) and / or pen now: a
    /// live one counts as used from here on.
    fn mark_used(&mut self, dc: u64, brush: bool, pen: bool) {
        let Some(d) = self.dc(dc) else { return };
        let b = brush
            && matches!(
                self.obj(u64::from(d.state.brush)),
                Some(Obj::Brush { deleted: false, delete_pending: false, .. })
            );
        let p = pen && matches!(self.obj(u64::from(d.state.pen)), Some(Obj::Pen { delete_pending: false, .. }));
        if let Some(s) = self.state(dc) {
            s.brush_used |= b;
            s.pen_used |= p;
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
        if uses_pattern {
            self.mark_used(dc, true, false);
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
        let Some(c) = self.handle_brush_color(b, &state) else { return true };
        if b == state.brush {
            self.mark_used(dc, true, false);
        }
        let r = self.device_rect(dc, normalized(r));
        self.with_target(dc, |bmp, _, clip, _| Self::fill(bmp, clip, r, 0xF0, Some(c)));
        true
    }

    /// `FillRgn` (Learn): region `rgn` (logical coordinates) filled with
    /// `brush`; false when one of them is not what it should be.
    pub fn fill_rgn(&mut self, dc: u64, rgn: u64, brush: u64) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        let state = d.state.clone();
        let Some(rects) = self.region(rgn).map(|r| r.rects().to_vec()) else { return false };
        if !matches!(self.obj(brush), Some(Obj::Brush { .. })) {
            return false;
        }
        let Some(c) = self.handle_brush_color(brush as u32, &state) else { return true };
        if brush as u32 == state.brush {
            self.mark_used(dc, true, false);
        }
        let rects: Vec<Rect> = rects.into_iter().map(|r| self.device_rect(dc, r)).collect();
        self.with_target(dc, |bmp, _, clip, _| {
            for r in rects {
                Self::fill(bmp, clip, r, 0xF0, Some(c));
            }
        });
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
        self.mark_used(dc, true, true);
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
        self.mark_used(dc, false, true);
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

    /// Where the font files of outline fonts come from (ADR 0142).
    pub fn set_font_loader(&mut self, loader: crate::outline::Loader) {
        self.text.files.borrow_mut().set_loader(loader);
        self.text.table.generation += 1;
    }

    /// Put a file in the font store directly (hosts without a loader).
    pub fn add_store_file(&mut self, class: fontmap::Class, bold: bool, italic: bool, data: Vec<u8>) -> bool {
        let ok = self.text.files.borrow_mut().add(class, bold, italic, data);
        self.text.table.generation += 1;
        ok
    }

    /// `AddFontMemResourceEx`: the handle of the font added from `data`
    /// (copied), or `None` where it is no usable font.
    pub fn add_font_mem(&mut self, data: &[u8]) -> Option<u32> {
        self.text.table.add_memory(data)
    }

    /// `RemoveFontMemResourceEx`.
    pub fn remove_font_mem(&mut self, handle: u64) -> bool {
        u32::try_from(handle).is_ok_and(|h| self.text.table.remove_memory(h))
    }

    /// `AddFontResourceExW` with the bytes of the file `path` names.
    pub fn add_font_file(&mut self, path: &[u16], flags: u32, data: Vec<u8>) -> bool {
        self.text.table.add_file(path, flags, data)
    }

    /// `RemoveFontResourceExW`: one earlier add of `path` with `flags`.
    pub fn remove_font_file(&mut self, path: &[u16], flags: u32) -> bool {
        self.text.table.remove_file(path, flags)
    }

    /// `EnumFontFamiliesExW`: the faces for a face name (empty: all) and a
    /// character set.
    pub fn enum_fonts(&self, name: &[u16], charset: u8) -> Vec<fonttab::EnumFace> {
        self.text.table.enumerate(name, charset)
    }

    /// The font file of an outline.
    fn font_data(&self, src: FontSrc) -> Option<Arc<Vec<u8>>> {
        match src {
            FontSrc::Installed(slot) => self.text.files.borrow().data(slot),
            FontSrc::Builtin(mono) => Some(self.text.files.borrow_mut().builtin(mono)),
            FontSrc::Private(id) => self.text.table.get(id).map(|f| Arc::clone(&f.data)),
        }
    }

    /// The font mapper (ADR 0142): which font a `LOGFONTW` gets. A face the
    /// program added by that name; else a raster name's bitmap face; else
    /// Poly's font of the name's class: the font store's DejaVu, or the
    /// built-in font where those files are not there.
    fn resolve(&self, lf_bytes: &[u8]) -> (Face, Option<Outline>) {
        let Some(lf) = LogFont::parse(lf_bytes) else { return (Face::System, None) };
        let bitmap = fontmap::bitmap_face(&lf);
        if let Some(found) = self.text.table.find(lf.face_name(), lf.weight >= 600, lf.italic)
            && let Ok(font) = poly_font::Font::parse(&found.face.data)
        {
            return (bitmap, Some(Outline::new(&font, FontSrc::Private(found.face.id), &lf, found.face.style, None)));
        }
        let m = fontmap::map(&lf);
        if m.raster.is_some() {
            return (bitmap, None);
        }
        (bitmap, self.installed_outline(&lf, &m))
    }

    fn installed_outline(&self, lf: &LogFont, m: &fontmap::Mapping) -> Option<Outline> {
        let mut files = self.text.files.borrow_mut();
        if let Some(slot) = files.find(m.class, m.bold, m.italic)
            && let Some(data) = files.data(slot)
            && let Ok(font) = poly_font::Font::parse(&data)
        {
            let bold = slot & 2 != 0;
            let style = FaceStyle { bold, italic: slot & 1 != 0, weight: if bold { 700 } else { 400 } };
            return Some(Outline::new(&font, FontSrc::Installed(slot), lf, style, Some(m.class)));
        }
        let mono = m.class == fontmap::Class::Mono;
        let data = files.builtin(mono);
        let font = poly_font::Font::parse(&data).ok()?;
        let style = FaceStyle { bold: false, italic: false, weight: 400 };
        Some(Outline::new(&font, FontSrc::Builtin(mono), lf, style, Some(m.class)))
    }

    /// The font a DC has selected (the system font for a bad DC).
    pub(crate) fn font_sel(&self, dc: u64) -> FontSel {
        let sel = |face, outline: Option<Outline>, logfont: &[u8]| {
            let data = outline.and_then(|o| self.font_data(o.src));
            FontSel {
                face,
                outline: outline.filter(|_| data.is_some()),
                data,
                underline: logfont.get(21).is_some_and(|&b| b != 0),
                strike: logfont.get(22).is_some_and(|&b| b != 0),
            }
        };
        let Some(d) = self.dc(dc) else { return sel(Face::System, None, &[]) };
        match self.obj(u64::from(d.state.font)) {
            Some(Obj::Font { face, stock: true, logfont, .. }) => sel(*face, None, logfont),
            Some(Obj::Font { logfont, mapped, .. }) => {
                let generation = self.text.table.generation;
                let m = match mapped.get() {
                    Some(m) if m.generation == generation => m,
                    _ => {
                        let (face, outline) = self.resolve(logfont);
                        let m = Mapped { generation, face, outline };
                        mapped.set(Some(m));
                        m
                    }
                };
                sel(m.face, m.outline, logfont)
            }
            _ => sel(Face::System, None, &[]),
        }
    }

    /// `GetTextMetricsW`.
    pub fn text_metrics(&self, dc: u64) -> Option<TextMetric> {
        self.dc(dc)?;
        Some(self.font_sel(dc).metrics())
    }

    /// `GetTextExtentPoint32W`.
    pub fn text_extent(&self, dc: u64, text: &[u16]) -> Option<(i32, i32)> {
        self.dc(dc)?;
        Some(self.font_sel(dc).typeface().extent(text))
    }

    /// `GetTextExtentExPointW`: the extent, how many characters fit in
    /// `max_width`, and for every character the width of the text up to and
    /// including it.
    pub fn text_extent_ex(&self, dc: u64, text: &[u16], max_width: i32) -> Option<((i32, i32), usize, Vec<i32>)> {
        self.dc(dc)?;
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let mut total = 0;
        let mut running = Vec::with_capacity(text.len());
        for &c in text {
            total += tf.advance(c);
            running.push(total);
        }
        let fit = running.iter().take_while(|&&w| w <= max_width).count();
        Some((tf.extent(text), fit, running))
    }

    /// `GetCharWidth32W`: the advance of each character `first..=last`.
    pub fn char_widths(&self, dc: u64, first: u32, last: u32) -> Option<Vec<i32>> {
        self.dc(dc)?;
        if first > last || last > 0xFFFF {
            return None;
        }
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        Some((first..=last).map(|c| tf.width(c as u16)).collect())
    }

    /// `GetTextFaceW`: the face name of the selected font. A face the
    /// program added reports its own spelling of the name (of the family, or
    /// the full name when that was asked for); a name Poly knows its Windows
    /// spelling; a name it does not know the stand-in family's name.
    pub fn text_face(&self, dc: u64) -> Option<Vec<u16>> {
        let d = self.dc(dc)?;
        let name = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        let stock_name = |f: Face| {
            name(match f {
                Face::System => "System",
                Face::Gui => "MS Shell Dlg",
                Face::AnsiFixed => "Courier",
                Face::OemFixed => "Terminal",
            })
        };
        Some(match self.obj(u64::from(d.state.font)) {
            Some(Obj::Font { face, stock: false, logfont, .. }) => match LogFont::parse(logfont) {
                Some(lf) => self.face_name_of(&lf),
                None => stock_name(*face),
            },
            Some(Obj::Font { face, .. }) => stock_name(*face),
            _ => name("System"),
        })
    }

    fn face_name_of(&self, lf: &LogFont) -> Vec<u16> {
        let req = lf.face_name();
        if let Some(f) = self.text.table.find(req, lf.weight >= 600, lf.italic) {
            return if f.by_full_name { f.face.full.clone() } else { f.face.family.clone() };
        }
        if let Some(c) = fonttab::canonical_name(req) {
            return c.encode_utf16().collect();
        }
        if !req.is_empty() && fontmap::is_known(lf) {
            return req.to_vec();
        }
        let stand_in = match fontmap::map(lf).class {
            fontmap::Class::Sans => "Arial",
            fontmap::Class::Serif => "Times New Roman",
            fontmap::Class::Mono => "Courier New",
        };
        stand_in.encode_utf16().collect()
    }

    /// `TextOutW` (`TextOutW.yaml`): false for a bad DC.
    pub fn text_out(&mut self, dc: u64, x: i32, y: i32, text: &[u16]) -> bool {
        self.ext_text_out(dc, x, y, 0, None, text, None)
    }

    /// `ExtTextOutW`: `options` are `ETO_OPAQUE` (the rectangle is filled
    /// with the background colour first), `ETO_CLIPPED` (the text is
    /// clipped to it) and `ETO_GLYPH_INDEX` (`text` holds glyph indices);
    /// `advances` are `lpDx`, the distance from each character's origin to
    /// the next. False for a bad DC or options `ETO_*` cannot all have.
    #[allow(clippy::too_many_arguments)]
    pub fn ext_text_out(
        &mut self,
        dc: u64,
        x: i32,
        y: i32,
        options: u32,
        rect: Option<Rect>,
        text: &[u16],
        advances: Option<&[i32]>,
    ) -> bool {
        let Some(d) = self.dc(dc) else { return false };
        if options == u32::MAX {
            return false;
        }
        let state = d.state.clone();
        let rect = rect.map(normalized);
        if options & ETO_OPAQUE != 0
            && let Some(r) = rect
        {
            self.fill_rect_color(dc, r, state.bk);
        }
        if text.is_empty() {
            return true;
        }
        let glyph_index = options & ETO_GLYPH_INDEX != 0;
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let tm = tf.metrics();
        let advance = |i: usize, c: u16| match advances.and_then(|a| a.get(i)) {
            Some(&dx) => dx,
            None if glyph_index => tf.glyph_width(c),
            None => tf.advance(c),
        };
        let cx: i32 = text.iter().enumerate().map(|(i, &c)| advance(i, c)).sum();
        let (rx, ry) = if state.align & TA_UPDATECP != 0 { state.pos } else { (x, y) };
        let x0 = match state.align & TA_CENTER {
            TA_RIGHT => rx - cx,
            TA_CENTER => rx - cx / 2,
            _ => rx,
        };
        let top = match state.align & TA_BASELINE {
            TA_BOTTOM => ry - tm.height,
            TA_BASELINE => ry - tm.ascent,
            _ => ry,
        };
        // The baseline origin: with an escapement the text's own axes turn
        // about the reference point.
        let esc = tf.outline().map_or(0, |o| o.escapement);
        let (ox, oy) = if esc == 0 {
            (x0, top + tm.ascent)
        } else {
            let (sn, cs) = crate::outline::sin_cos(esc);
            let (vx, vy) = ((x0 - rx) as f32, (top + tm.ascent - ry) as f32);
            // A quarter turn about the reference point puts the baseline one
            // pixel further away (`CreateFontIndirectW.yaml`, escapement).
            let bias = match esc {
                900 => 1.0,
                2700 => -1.0,
                _ => 0.0,
            };
            let dx = vx * cs + vy * sn + bias;
            let dy = -vx * sn + vy * cs;
            (rx + round_pixel(dx), ry + round_pixel(dy))
        };
        let clip = if options & ETO_CLIPPED != 0 { rect } else { None };
        let end = self.draw_run(dc, ox, oy, text, glyph_index, clip, advances, cx, true);
        if state.align & TA_UPDATECP != 0
            && let Some(s) = self.state(dc)
        {
            s.pos.0 = match state.align & TA_CENTER {
                TA_RIGHT => rx - cx,
                TA_CENTER => rx,
                _ => rx + cx,
            };
            if esc != 0 {
                s.pos = end;
            }
        }
        true
    }

    /// Draw `text` (as in [`Gdi::draw_run`]) with its cell's top left at
    /// (`x0`, `y0`), for `DrawText`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_text_run(
        &mut self,
        dc: u64,
        x0: i32,
        y0: i32,
        text: &[u16],
        clip: Option<Rect>,
        advances: Option<&[i32]>,
        fill: bool,
    ) {
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let ascent = tf.metrics().ascent;
        let cx = match advances {
            Some(a) => text.iter().enumerate().map(|(i, &c)| a.get(i).copied().unwrap_or_else(|| tf.advance(c))).sum(),
            None => tf.extent(text).0,
        };
        self.draw_run(dc, x0, y0 + ascent, text, false, clip, advances, cx, fill);
    }

    /// Fill the background cell of a line of text `w` wide whose top left is
    /// (`x`, `y`), in OPAQUE mode (`DrawText` does it once for a line whose
    /// tabs leave gaps).
    pub(crate) fn fill_text_cell(&mut self, dc: u64, x: i32, y: i32, w: i32, clip: Option<Rect>) {
        let height = self.font_sel(dc).typeface().metrics().height;
        let (x, y) = self.device_point(dc, x, y);
        let clip = clip.map(|c| self.device_rect(dc, c));
        self.with_target(dc, |b, state, dclip, _| {
            if state.bk_mode == TRANSPARENT {
                return;
            }
            let clip = match clip {
                Some(c) => dclip.intersect_rect(c),
                None => dclip.clone(),
            };
            Self::fill(b, &clip, Rect::new(x, y, x + w, y + height), 0xF0, Some(Rgb::from_colorref(state.bk)));
        });
    }

    /// Draw `text` (glyph indices if `glyph_index`) with its first pen
    /// position at (`ox`, `oy`) on the baseline, in logical coordinates,
    /// clipped to `clip` too when given; `cx` is its length along the
    /// baseline. A glyph of an outline font is blended into the surface by
    /// its coverage (grey scale antialiasing) or, for a bi-level font, set;
    /// a bitmap face's pixels are set. Underline and strike-out follow the
    /// `LOGFONTW`. Returns where the pen ends (logical).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_run(
        &mut self,
        dc: u64,
        ox: i32,
        oy: i32,
        text: &[u16],
        glyph_index: bool,
        clip: Option<Rect>,
        advances: Option<&[i32]>,
        cx: i32,
        fill: bool,
    ) -> (i32, i32) {
        let sel = self.font_sel(dc);
        let tf = sel.typeface();
        let tm = tf.metrics();
        let (dx0, dy0) = self.device_point(dc, ox, oy);
        let shift = (dx0 - ox, dy0 - oy);
        let clip = clip.map(|c| self.device_rect(dc, c));
        let (under, strike) = tf.line_metrics();
        let esc = tf.outline().map_or(0, |o| o.escapement);
        let mut cache = self.text.glyphs.take().unwrap_or_else(|| GlyphCache::new(GLYPH_CACHE_BYTES));
        let mut end = (dx0, dy0);
        self.with_target(dc, |b, state, dclip, _| {
            let clip = match clip {
                Some(c) => dclip.intersect_rect(c),
                None => dclip.clone(),
            };
            let fg_rgb = Rgb::from_colorref(state.text);
            let fg = b.from_rgb(fg_rgb);
            if state.bk_mode != TRANSPARENT && fill {
                let cell = if esc == 0 {
                    Rect::new(dx0, dy0 - tm.ascent, dx0 + cx, dy0 - tm.ascent + tm.height)
                } else {
                    // The box around the turned cell.
                    let (sn, cs) = crate::outline::sin_cos(esc);
                    let mut bb = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                    for s in [0, cx] {
                        for t in [-tm.ascent, tm.descent] {
                            let px = dx0 as f32 + s as f32 * cs + t as f32 * sn;
                            let py = dy0 as f32 - s as f32 * sn + t as f32 * cs;
                            let (px, py) = (round_pixel(px), round_pixel(py));
                            bb = (bb.0.min(px), bb.1.min(py), bb.2.max(px), bb.3.max(py));
                        }
                    }
                    Rect::new(bb.0, bb.1, bb.2, bb.3)
                };
                Self::fill(b, &clip, cell, 0xF0, Some(Rgb::from_colorref(state.bk)));
            }
            if tf.is_outline() {
                end = tf.draw_outline(&mut cache, text, glyph_index, dx0, dy0, advances, |px, py, a| {
                    if clip.contains_point(px, py) {
                        Self::blend(b, px, py, fg_rgb, a, tf.outline().is_some_and(|o| o.bilevel));
                    }
                });
            } else {
                let y0 = dy0 - tm.ascent;
                let mut x = dx0;
                for (i, &c) in text.iter().enumerate() {
                    let (w, rows) = tf.face.glyph(c);
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
                    x += advances.and_then(|a| a.get(i).copied()).unwrap_or(w);
                }
                end = (x, dy0);
            }
            if esc == 0 {
                if sel.underline {
                    let top = dy0 + under.0;
                    Self::fill(b, &clip, Rect::new(dx0, top, dx0 + cx, top + under.1), 0xF0, Some(fg_rgb));
                }
                if sel.strike {
                    let top = dy0 + strike.0;
                    Self::fill(b, &clip, Rect::new(dx0, top, dx0 + cx, top + strike.1), 0xF0, Some(fg_rgb));
                }
            }
        });
        self.text.glyphs = Some(cache);
        (end.0 - shift.0, end.1 - shift.1)
    }

    /// `DrawText`'s mnemonic underline: `w` pixels wide at `x`, a single
    /// row one below the baseline of the cell whose top is `y` (logical
    /// coordinates), whatever the font's own underline says.
    pub(crate) fn draw_underline(&mut self, dc: u64, x: i32, w: i32, y: i32, clip: Option<Rect>) {
        let sel = self.font_sel(dc);
        let baseline = y + sel.typeface().metrics().ascent;
        let (x, y) = self.device_point(dc, x, baseline + 1);
        let clip = clip.map(|c| self.device_rect(dc, c));
        self.with_target(dc, |b, state, dclip, _| {
            let clip = match clip {
                Some(c) => dclip.intersect_rect(c),
                None => dclip.clone(),
            };
            Self::fill(b, &clip, Rect::new(x, y, x + w, y + 1), 0xF0, Some(Rgb::from_colorref(state.text)));
        });
    }

    /// Paint `fg` over the pixel with `alpha` (1 to 255) of coverage;
    /// `bilevel` text is all or nothing.
    fn blend(b: &mut Bitmap, x: i32, y: i32, fg: Rgb, alpha: [u8; 3], bilevel: bool) {
        if !b.contains(x, y) {
            return;
        }
        if b.bpp <= 8 || bilevel {
            // Too few colours to mix, or no mixing asked for: ink above half
            // coverage (of the average, for colour channels of their own).
            let avg = (u32::from(alpha[0]) + u32::from(alpha[1]) + u32::from(alpha[2])) / 3;
            if avg >= 128 {
                let v = b.from_rgb(fg);
                b.set_raw(x, y, v);
            }
            return;
        }
        let d = b.to_rgb(b.raw(x, y));
        let mix = |f: u8, d: u8, a: u8| {
            let a = u32::from(a);
            ((u32::from(f) * a + u32::from(d) * (255 - a) + 127) / 255) as u8
        };
        let out = Rgb { r: mix(fg.r, d.r, alpha[0]), g: mix(fg.g, d.g, alpha[1]), b: mix(fg.b, d.b, alpha[2]) };
        let v = b.from_rgb(out);
        b.set_raw(x, y, v);
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

/// A float coordinate rounded to the nearest pixel (halves away from zero).
fn round_pixel(v: f32) -> i32 {
    if v < 0.0 { -((-v + 0.5) as i32) } else { (v + 0.5) as i32 }
}

fn normalize(x: i32, y: i32, w: i32, h: i32) -> Rect {
    let (l, r) = if w < 0 { (x + w, x) } else { (x, x + w) };
    let (t, b) = if h < 0 { (y + h, y) } else { (y, y + h) };
    Rect::new(l, t, r, b)
}
