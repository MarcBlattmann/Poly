//! Which windows the desktop link shows, and their pixels (ADR 0131,
//! "The desktop link").
//!
//! [`Surfaces::sync`] brings [`Desktop`]'s surfaces in line with the
//! window manager after every change: a visible, not minimized top-level
//! window of an app with a client area gets a surface whose rectangle is
//! its client area on the screen (Poly's frame is drawn around it; the
//! window keeps Windows' own non-client metrics, which the first slice's
//! tests pin), titled with the window text, framed when the window has a
//! caption. A hidden or minimized window's surface is hidden; a destroyed
//! window's surface goes.
//!
//! A surface's buffer is memory kept here, client-sized, 32 bits per pixel
//! (`B, G, R, x`: a top-down 32-bpp DIB's layout), white until the app
//! draws. `user32` draws a window's DCs into a DIB of the same size in the
//! app ([`crate::proto::Request::Surface`] tells it the size) and commits
//! it with `PolyUserCommitSurface` ([`Surfaces::commit`]); the compositor
//! copies the damaged rows out with the device's
//! [`READ_BUFFER`](crate::link::READ_BUFFER) request ([`Surfaces::rows`]).
//!
//! Nothing happens before a compositor first connects, unless the session
//! is interactive (slice B): headless programs of session 0 (slice A's
//! tests) get no surfaces and their window DCs draw nowhere.

use crate::desktop::Desktop;
use crate::link::{BufferId, PixelFormat, SurfaceId, SurfaceKind};
use crate::metrics::WS_CAPTION;
use crate::wm::Wm;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// The largest surface side with a buffer: a window as large as an 8K screen. Its buffer is kept
/// in bands ([`BAND_BYTES`]); a window in output pixels on a 2560x1600 screen at 200 % is wider
/// than 2048 (ADR 0219).
pub const MAX_SIDE: u32 = 8192;

/// The widest own border a window's surface passes the pointer to (a window
/// with a border wider than this gets this much of it).
const MAX_MARGIN: i32 = 32;

/// A buffer is kept in bands of whole rows of at most this many bytes: one
/// kernel allocation holds 4 MiB at most (`kernel/src/heap.rs`), which a
/// maximized window's client area on a 1920x1080 screen exceeds.
pub const BAND_BYTES: usize = 1024 * 1024;

/// The pixel a buffer starts with (white, `COLOR_WINDOW`'s default).
const BLANK: u8 = 0xFF;

#[derive(Debug)]
struct Buffer {
    width: u32,
    height: u32,
    /// The rows, `rows_per_band` to a band (the last one shorter).
    bands: Vec<Vec<u8>>,
    rows_per_band: u32,
}

impl Buffer {
    fn new(width: u32, height: u32) -> Buffer {
        let stride = width as usize * 4;
        let rows_per_band = (BAND_BYTES / stride.max(1)).max(1) as u32;
        let mut bands = Vec::new();
        let mut row = 0;
        while row < height {
            let n = rows_per_band.min(height - row);
            bands.push(vec![BLANK; n as usize * stride]);
            row += n;
        }
        Buffer { width, height, bands, rows_per_band }
    }

    /// Copy whole rows from `row` on out of `pixels`; false when they do
    /// not fit.
    fn write(&mut self, row: u32, pixels: &[u8]) -> bool {
        let stride = self.width as usize * 4;
        if stride == 0 || !pixels.len().is_multiple_of(stride) {
            return false;
        }
        let rows = (pixels.len() / stride) as u32;
        if row > self.height || rows > self.height - row {
            return false;
        }
        for (i, src) in pixels.chunks(stride).enumerate() {
            let y = row + i as u32;
            let band = &mut self.bands[(y / self.rows_per_band) as usize];
            let at = (y % self.rows_per_band) as usize * stride;
            band[at..at + stride].copy_from_slice(src);
        }
        true
    }

    /// Row `y` from column `x` on: as much of `src` (4 bytes a pixel) as fits.
    fn write_span(&mut self, x: u32, y: u32, src: &[u8]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let n = (src.len() / 4).min((self.width - x) as usize) * 4;
        let band = &mut self.bands[(y / self.rows_per_band) as usize];
        let at = ((y % self.rows_per_band) as usize * self.width as usize + x as usize) * 4;
        band[at..at + n].copy_from_slice(&src[..n]);
    }

    /// Take `old`'s picture where both buffers have pixels, from the top-left corner.
    fn keep(&mut self, old: &Buffer) {
        let n = self.width.min(old.width) as usize * 4;
        for y in 0..self.height.min(old.height) {
            let row = old.read(y, y + 1);
            self.write_span(0, y, &row[..n]);
        }
    }

    /// The pixel (`B, G, R, x`) at (`x`, `y`), inside the buffer.
    fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let band = self.bands.get((y / self.rows_per_band) as usize)?;
        let at = ((y % self.rows_per_band) as usize * self.width as usize + x as usize) * 4;
        band.get(at..at + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    /// Rows `first .. last`, row after row.
    fn read(&self, first: u32, last: u32) -> Vec<u8> {
        let stride = self.width as usize * 4;
        let mut out = Vec::with_capacity((last - first) as usize * stride);
        for y in first..last {
            let band = &self.bands[(y / self.rows_per_band) as usize];
            let at = (y % self.rows_per_band) as usize * stride;
            out.extend_from_slice(&band[at..at + stride]);
        }
        out
    }
}

/// Where a child window's picture goes ([`Surfaces::child_area`]): the top-level window, the
/// child's client area in its surface, and the surface's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChildArea {
    pub top: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub surface: (u32, u32),
}

/// The largest content `UpdateLayeredWindow` may leave here (bytes).
pub const MAX_CONTENT: usize = 1 << 20;

/// What `UpdateLayeredWindow` gave a layered window: pixels (`B, G, R, A`,
/// top-down) covering the window from its top-left corner.
#[derive(Debug)]
struct Content {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

impl Content {
    fn at(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        self.bytes.get(at..at + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }
}

/// A physical size in a buffer's units (`scale`: buffer units, physical pixels), rounded down.
fn own_units(v: u32, (num, den): (u32, u32)) -> u32 {
    if num == den { v } else { (u64::from(v) * u64::from(num) / u64::from(den.max(1))) as u32 }
}

/// A physical coordinate in a buffer's units, rounded to the nearest.
fn own_coord(v: i32, (num, den): (u32, u32)) -> i32 {
    if num == den { v } else { crate::dpiscale::mul_div_round(v, num, den) }
}

/// Whether Poly frames a top-level window (ADR 0183 addendum "Windows apps"): it has a
/// caption (`WS_CAPTION`), and its client area, as its `WM_NCCALCSIZE` answer placed it, leaves
/// room for one above it. A window whose client area starts less than a caption's height
/// (`SM_CYCAPTION`) below its top edge draws its own title bar in its client area (Chrome's
/// `Chrome_WidgetWin_1`: `WS_OVERLAPPEDWINDOW`, client area from the window's top, keeping the
/// 8-pixel side and bottom borders, `s3-windows.md`); Poly's frame around it would be a second
/// one. Its surface is its client area as for every window.
pub fn poly_frames(style: u32, window: crate::frame::Rect, client: crate::frame::Rect) -> bool {
    style & WS_CAPTION == WS_CAPTION && client.top - window.top >= crate::metrics::CAPTION
}

/// The surfaces of top-level windows and their buffers.
#[derive(Debug, Default)]
pub struct Surfaces {
    windows: BTreeMap<u32, SurfaceId>,
    buffers: BTreeMap<BufferId, Buffer>,
    /// The contents of `UpdateLayeredWindow`-ed windows.
    contents: BTreeMap<u32, Content>,
    /// Windows whose pixels come with premultiplied alpha from a DirectComposition swap chain
    /// (ADR 0225).
    premultiplied: BTreeSet<u32>,
    /// Each surface's buffer units against physical pixels (`Wm::buffer_scale`, ADR 0231).
    scales: BTreeMap<u32, (u32, u32)>,
}

impl Surfaces {
    pub fn new() -> Surfaces {
        Surfaces::default()
    }

    /// The window a surface shows.
    pub fn hwnd_of(&self, surface: SurfaceId) -> Option<u32> {
        self.windows.iter().find(|e| *e.1 == surface).map(|e| *e.0)
    }

    /// The surface of window `hwnd`.
    pub fn surface(&self, hwnd: u32) -> Option<SurfaceId> {
        self.windows.get(&hwnd).copied()
    }

    /// Bring the desktop's surfaces in line with the window manager, and
    /// free the buffers the compositor let go of.
    pub fn sync(&mut self, wm: &Wm, desktop: &mut Desktop) {
        for b in desktop.take_released() {
            self.buffers.remove(&b);
        }
        // A content lasts while its window is layered by UpdateLayeredWindow.
        self.contents.retain(|h, _| wm.layered(*h).is_some_and(|l| l.update));
        self.premultiplied.retain(|h| wm.info(*h).is_some());
        // Surfaces exist with a compositor, and in an interactive session
        // even without one (window DCs keep their pixels, BeginPaint.yaml
        // reads them back).
        if !desktop.is_connected() && !wm.display().interactive && self.windows.is_empty() {
            return;
        }
        let tops = wm.top_levels();
        // Gone, or no longer top-level: the surface goes.
        let gone: Vec<u32> = self.windows.keys().filter(|h| !tops.contains(h)).copied().collect();
        self.scales.retain(|h, _| tops.contains(h));
        for h in gone {
            if let Some(s) = self.windows.remove(&h) {
                desktop.destroy(s);
            }
        }
        // Bottom to top, so an owner has its surface before what it owns.
        for &h in tops.iter().rev() {
            let Some(i) = wm.info(h) else { continue };
            if i.pid == 0 || i.message_only || wm.is_foreign(h) {
                continue;
            }
            let client = i.client;
            let shown = i.visible && !i.minimized && client.right > client.left && client.bottom > client.top;
            let framed = poly_frames(i.style, i.window, client);
            // The picture of a surface made again below, for its new one.
            let mut carried = None;
            // A window shown before its owner had a surface was told to the compositor without
            // one: once the owner has its surface, the window's is made again with it, so that it
            // moves with its owner and stays above it (an owned window is always above its
            // owner; Learn, "Window Features": owned windows). Chrome shows its "Restore pages?"
            // bubble before its main window.
            if shown
                && let Some(&s) = self.windows.get(&h)
                && let Some(&owner) = self.windows.get(&i.owner)
                && desktop.surface(s).is_some_and(|x| x.owner != owner)
            {
                carried = desktop.surface(s).and_then(|x| x.buffer).map(|b| b.id);
                self.windows.remove(&h);
                desktop.destroy(s);
            }
            // The window changed its mind about its caption (a WM_NCCALCSIZE answer after
            // SWP_FRAMECHANGED): its surface is made again with the new frame.
            if shown
                && let Some(&s) = self.windows.get(&h)
                && i.style & WS_CAPTION == WS_CAPTION
                && desktop.surface(s).is_some_and(|x| x.framed != framed)
            {
                self.windows.remove(&h);
                desktop.destroy(s);
            }
            let s = match self.windows.get(&h) {
                Some(&s) => s,
                None if shown => {
                    // A captioned window that draws its own caption is still an application
                    // window (a top level of the desktop), only without Poly's frame.
                    let kind =
                        if i.style & WS_CAPTION == WS_CAPTION { SurfaceKind::TopLevel } else { SurfaceKind::Popup };
                    let owner = self.windows.get(&i.owner).copied().unwrap_or(0);
                    let s = desktop.create(kind, owner, framed);
                    desktop.set_process(s, i.pid);
                    self.windows.insert(h, s);
                    s
                }
                None => continue,
            };
            if shown {
                // The surface holds the window's menu bar above the client area.
                let bar = i.menu_bar.max(0);
                let (cw, ch) = ((client.right - client.left) as u32, (client.bottom - client.top + bar) as u32);
                desktop.set_geometry(s, client.left, client.top - bar, cw, ch);
                // A window the system scales (unaware, GDI-scaled) draws in its own units: its
                // buffer is that size and the compositor enlarges it (ADR 0231).
                let scale = wm.buffer_scale(h);
                self.scales.insert(h, scale);
                let (cw, ch) = (own_units(cw, scale).max(1), own_units(ch, scale).max(1));
                let format = self.pixel_format(wm, h);
                let current = desktop.surface(s).and_then(|x| x.buffer).map(|b| (b.id, b.width, b.height, b.format));
                if current.map(|c| (c.1, c.2, c.3)) != Some((cw, ch, format))
                    && cw <= MAX_SIDE
                    && ch <= MAX_SIDE
                    && let Some(b) = desktop.attach_buffer(s, cw, ch, cw * 4, format)
                {
                    // A resized window keeps what it showed, from its top-left corner: commits
                    // carry only what was drawn since (ADR 0219).
                    let mut new = Buffer::new(cw, ch);
                    if let Some(old) = current.map(|c| c.0).or(carried).and_then(|b| self.buffers.get(&b)) {
                        new.keep(old);
                    }
                    self.buffers.insert(b, new);
                    desktop.damage(s, 0, 0, cw, ch);
                }
            }
            if let Some(text) = wm.text(h) {
                desktop.set_title(s, &String::from_utf16_lossy(text));
            }
            // The compositor tells a minimized window from a hidden one by this, and shows it
            // again through the window's own restore (`Request::Restore`).
            let state = if i.minimized {
                crate::link::WindowState::Minimized
            } else if i.maximized {
                crate::link::WindowState::Maximized
            } else {
                crate::link::WindowState::Normal
            };
            desktop.set_state(s, state);
            // A window that draws its own caption keeps its own sizing borders around the
            // surface (invisible, as on Windows); the compositor passes the pointer there to it.
            let margins = if shown && !framed && i.style & WS_CAPTION == WS_CAPTION && !i.maximized {
                let side = |v: i32| v.clamp(0, MAX_MARGIN) as u16;
                [
                    side(client.left - i.window.left),
                    side(client.top - i.menu_bar.max(0) - i.window.top),
                    side(i.window.right - client.right),
                    side(i.window.bottom - client.bottom),
                ]
            } else {
                [0; 4]
            };
            desktop.set_margins(s, margins);
            desktop.set_visible(s, shown);
        }
    }

    /// How the compositor reads window `hwnd`'s pixels (ADR 0225). Two kinds of window come with
    /// premultiplied alpha (Learn, "BLENDFUNCTION", "DXGI_ALPHA_MODE"), and their transparent
    /// pixels show what is below them (menus, bubbles and their shadows, rounded corners):
    /// - one that a DirectComposition swap chain of `DXGI_ALPHA_MODE_PREMULTIPLIED` shows in
    ///   ([`Surfaces::set_premultiplied`]; Chrome's popups);
    /// - a layered window given its content by `UpdateLayeredWindow` with `ULW_ALPHA` and
    ///   `AC_SRC_ALPHA`.
    ///
    /// Every other window is opaque: GDI leaves the alpha byte of its pixels undefined.
    pub fn pixel_format(&self, wm: &Wm, hwnd: u32) -> PixelFormat {
        let layered_alpha = wm
            .layered(hwnd)
            .is_some_and(|l| l.update && !l.attrs && l.ulw_flags & crate::guic::ULW_ALPHA != 0 && l.ulw_src_alpha);
        if layered_alpha || self.premultiplied.contains(&hwnd) { PixelFormat::Argb8888 } else { PixelFormat::Xrgb8888 }
    }

    /// What window `hwnd` shows comes with premultiplied alpha, or no longer does (a swap chain
    /// of that alpha mode bound to it); false for a window that does not exist. A child window's
    /// pixels are part of its top-level window's surface, so that surface takes the alpha
    /// (Chrome's GPU process binds its swap chain to a child of the popup).
    pub fn set_premultiplied(&mut self, wm: &Wm, hwnd: u32, on: bool) -> bool {
        let Some(mut i) = wm.info(hwnd) else { return false };
        let mut top = hwnd;
        while i.style & crate::metrics::WS_CHILD != 0 {
            top = i.parent;
            let Some(p) = wm.info(top) else { return false };
            i = p;
        }
        if on {
            self.premultiplied.insert(top);
        } else {
            self.premultiplied.remove(&top);
        }
        true
    }

    /// `UpdateLayeredWindow` gave layered window `hwnd` this content:
    /// `width` x `height` pixels (`B, G, R, A`, top-down) from its top-left
    /// corner; `width` 0 takes it away. False for a window that is not
    /// layered or pixels that do not fit the size.
    pub fn set_content(&mut self, wm: &Wm, hwnd: u32, width: u32, height: u32, pixels: Vec<u8>) -> bool {
        if wm.layered(hwnd).is_none() {
            return false;
        }
        if width == 0 || height == 0 {
            self.contents.remove(&hwnd);
            return true;
        }
        let size = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
        if size != Some(pixels.len()) || pixels.len() > MAX_CONTENT {
            return false;
        }
        self.contents.insert(hwnd, Content { width, height, bytes: pixels });
        true
    }

    /// What the screen shows at (`x`, `y`) as a `COLORREF`: the topmost
    /// visible top-level window's client pixel there, a layered window
    /// blended with what is below it as `(src * a + below * (255 - a)) / 255`
    /// per channel, its colour key showing what is below, and a layered
    /// window never given attributes showing nothing
    /// (`SetLayeredWindowAttributes.yaml`); a window given its content by
    /// `UpdateLayeredWindow` composed as that call said
    /// (`UpdateLayeredWindow.yaml`); nothing of a window outside its region
    /// (`SetWindowRgn.yaml`). Black where no window is.
    pub fn screen_pixel(&self, wm: &Wm, desktop: &Desktop, x: i32, y: i32) -> u32 {
        self.pixel_below(wm, desktop, &wm.top_levels(), x, y)
    }

    /// Whether a point on layered top-level window `top` hits what is below
    /// it: alpha 0, or a colour key whose red and blue parts are equal and
    /// the pixel is the key (`SetLayeredWindowAttributes.yaml`
    /// `hit_testing_*`).
    pub fn hit_passes(&self, wm: &Wm, desktop: &Desktop, top: u32, x: i32, y: i32) -> bool {
        let Some((key, alpha, flags, attrs, _)) = wm.layered_of(top) else { return false };
        if !attrs {
            // UpdateLayeredWindow's content: a pixel without alpha is not hit
            // (`UpdateLayeredWindow.yaml` `hit_testing`).
            let (Some(l), Some(c), Some(i)) = (wm.layered(top), self.contents.get(&top), wm.info(top)) else {
                return false;
            };
            return c.at(x - i.window.left, y - i.window.top).is_some_and(|p| l.ulw_hit_passes(p));
        }
        if flags & 2 != 0 && alpha == 0 {
            return true;
        }
        let Some(i) = wm.info(top) else { return false };
        if flags & 1 != 0 && (key & 0xFF) == ((key >> 16) & 0xFF) {
            return self.client_pixel(desktop, top, x - i.client.left, y - i.client.top + i.menu_bar)
                == Some(key & 0x00FF_FFFF);
        }
        false
    }

    fn pixel_below(&self, wm: &Wm, desktop: &Desktop, order: &[u32], x: i32, y: i32) -> u32 {
        for (k, &h) in order.iter().enumerate() {
            let Some(i) = wm.info(h) else { continue };
            if !i.visible || i.minimized || i.message_only || wm.region_excludes(h, x, y) {
                continue;
            }
            let lay = wm.layered(h);
            // UpdateLayeredWindow's content covers the window from its
            // top-left; any other window shows its client area's surface.
            let content = self.contents.get(&h).filter(|_| lay.is_some_and(|l| l.update));
            let px = match content {
                Some(c) => c.at(x - i.window.left, y - i.window.top),
                None if i.client.contains(x, y)
                    || (x >= i.client.left
                        && x < i.client.right
                        && y >= i.client.top - i.menu_bar
                        && y < i.client.top) =>
                {
                    self.client_pixel(desktop, h, x - i.client.left, y - i.client.top + i.menu_bar)
                        .map(|c| [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255])
                }
                None => None,
            };
            let Some(px) = px else { continue };
            let pix = u32::from(px[2]) | u32::from(px[1]) << 8 | u32::from(px[0]) << 16;
            let Some(l) = lay else { return pix };
            let (key, alpha, flags, attrs, update) = (l.key, l.alpha, l.flags, l.attrs, l.update);
            if !attrs && !update {
                continue;
            }
            if !attrs {
                if content.is_none() {
                    return pix;
                }
                let below = self.pixel_below(wm, desktop, &order[k + 1..], x, y);
                return l.ulw_pixel(px, below);
            }
            if flags & 1 != 0 && pix == key & 0x00FF_FFFF {
                continue;
            }
            if flags & 2 == 0 {
                return pix;
            }
            let below = self.pixel_below(wm, desktop, &order[k + 1..], x, y);
            let a = u32::from(alpha);
            let mix = |s: u32, d: u32| (s * a + d * (255 - a)) / 255;
            let ch = |c: u32, sh: u32| (c >> sh) & 255;
            return mix(ch(pix, 0), ch(below, 0))
                | mix(ch(pix, 8), ch(below, 8)) << 8
                | mix(ch(pix, 16), ch(below, 16)) << 16;
        }
        0
    }

    /// The `COLORREF` of window `hwnd`'s surface at (`x`, `y`) in client
    /// coordinates.
    fn client_pixel(&self, desktop: &Desktop, hwnd: u32, x: i32, y: i32) -> Option<u32> {
        let scale = self.scale_of(hwnd);
        let (x, y) = (own_coord(x, scale), own_coord(y, scale));
        let s = self.surface(hwnd)?;
        let b = desktop.surface(s)?.buffer?;
        let buf = self.buffers.get(&b.id)?;
        if x < 0 || y < 0 || x as u32 >= buf.width || y as u32 >= buf.height {
            return None;
        }
        let p = buf.pixel(x as u32, y as u32)?;
        Some(u32::from(p[2]) | u32::from(p[1]) << 8 | u32::from(p[0]) << 16)
    }

    /// The units of window `hwnd`'s buffer against physical pixels (`Wm::buffer_scale`).
    fn scale_of(&self, hwnd: u32) -> (u32, u32) {
        self.scales.get(&hwnd).copied().unwrap_or((1, 1))
    }

    /// The size of window `hwnd`'s surface buffer, for its DCs.
    pub fn size(&self, desktop: &Desktop, hwnd: u32) -> Option<(u32, u32)> {
        let s = self.surface(hwnd)?;
        let b = desktop.surface(s)?.buffer?;
        self.buffers.contains_key(&b.id).then_some((b.width, b.height))
    }

    /// Where child window `hwnd`'s picture goes: its top-level window's surface (Windows: a child
    /// draws into its top-level window), at the child's client area, which may lie partly outside
    /// it. `None` for a window that is not a child of a window with a surface. Its DCs draw into a
    /// bitmap of the child's client size (`size` of a child), which its process commits here, also
    /// when the top-level window is another process's (Chrome's GPU process draws into a child of
    /// the browser's window).
    pub fn child_area(&self, wm: &Wm, desktop: &Desktop, hwnd: u32) -> Option<ChildArea> {
        let i = wm.info(hwnd)?;
        if i.style & crate::metrics::WS_CHILD == 0 {
            return None;
        }
        let mut top = i.parent;
        let mut t = wm.info(top)?;
        while t.style & crate::metrics::WS_CHILD != 0 {
            top = t.parent;
            t = wm.info(top)?;
        }
        let (sw, sh) = self.size(desktop, top)?;
        // In the units of the top-level window's buffer (ADR 0231).
        let scale = self.scale_of(top);
        Some(ChildArea {
            top,
            x: own_coord(i.client.left - t.client.left, scale),
            y: own_coord(i.client.top - t.client.top, scale),
            width: own_units(i.client.width().max(0) as u32, scale),
            height: own_units(i.client.height().max(0) as u32, scale),
            surface: (sw, sh),
        })
    }

    /// Rows `row ..` of a child's picture (`area.width` pixels of 4 bytes each, row after row)
    /// into its top-level window's surface, clipped to it. False when the surface is gone or
    /// changed size.
    pub fn write_child_rows(&mut self, desktop: &Desktop, area: &ChildArea, row: u32, pixels: &[u8]) -> bool {
        let Some(s) = self.surface(area.top) else { return false };
        let Some(info) = desktop.surface(s).and_then(|x| x.buffer) else { return false };
        let Some(b) = self.buffers.get_mut(&info.id) else { return false };
        if (b.width, b.height) != area.surface || area.width == 0 {
            return false;
        }
        let stride = area.width as usize * 4;
        for (i, src) in pixels.chunks(stride).enumerate() {
            let y = area.y + row as i32 + i as i32;
            if y < 0 {
                continue;
            }
            let (x, src) = if area.x < 0 {
                let skip = (area.x.unsigned_abs() as usize * 4).min(src.len());
                (0, &src[skip..])
            } else {
                (area.x as u32, src)
            };
            b.write_span(x, y as u32, src);
        }
        true
    }

    /// The child's rows are written: its area of the top-level surface is damaged.
    pub fn child_committed(&mut self, desktop: &mut Desktop, area: &ChildArea) -> bool {
        let Some(s) = self.surface(area.top) else { return false };
        let (sw, sh) = (area.surface.0 as i32, area.surface.1 as i32);
        let (l, t) = (area.x.clamp(0, sw), area.y.clamp(0, sh));
        let (r, b) = ((area.x + area.width as i32).clamp(0, sw), (area.y + area.height as i32).clamp(0, sh));
        if r > l && b > t {
            desktop.damage(s, l as u32, t as u32, (r - l) as u32, (b - t) as u32);
        }
        true
    }

    /// The app drew window `hwnd`'s surface: `pixels` are `width` x
    /// `height`, 4 bytes each, row after row. False when the size is not
    /// the buffer's (the window was resized since; the app asks again).
    pub fn commit(&mut self, desktop: &mut Desktop, hwnd: u32, width: u32, height: u32, pixels: &[u8]) -> bool {
        pixels.len() == width as usize * height as usize * 4
            && self.write_rows(desktop, hwnd, width, height, 0, pixels)
            && self.committed(desktop, hwnd)
    }

    /// Part of a commit: whole rows from `row` on of window `hwnd`'s
    /// `width` x `height` surface (a large surface comes in several pieces,
    /// [`BAND_BYTES`]); [`Surfaces::committed`] ends it. False when the size
    /// is not the buffer's or the rows do not fit.
    pub fn write_rows(
        &mut self,
        desktop: &Desktop,
        hwnd: u32,
        width: u32,
        height: u32,
        row: u32,
        pixels: &[u8],
    ) -> bool {
        let Some(s) = self.surface(hwnd) else { return false };
        let Some(info) = desktop.surface(s).and_then(|x| x.buffer) else { return false };
        let Some(b) = self.buffers.get_mut(&info.id) else { return false };
        (b.width, b.height) == (width, height) && b.write(row, pixels)
    }

    /// Spans of `x, row, width` pixels from row `row` on out of `pixels`
    /// (`width * 4` bytes a row) into window `hwnd`'s buffer of `size`
    /// (ADR 0219); false when the window has no such buffer or they do
    /// not fit.
    pub fn write_area(
        &mut self,
        desktop: &Desktop,
        hwnd: u32,
        size: (u32, u32),
        (x, row, width): (u32, u32, u32),
        pixels: &[u8],
    ) -> bool {
        let Some(s) = self.surface(hwnd) else { return false };
        let Some(info) = desktop.surface(s).and_then(|x| x.buffer) else { return false };
        let Some(b) = self.buffers.get_mut(&info.id) else { return false };
        let span = width as usize * 4;
        if (b.width, b.height) != size || span == 0 || !pixels.len().is_multiple_of(span) {
            return false;
        }
        let rows = (pixels.len() / span) as u32;
        if x.saturating_add(width) > b.width || row.saturating_add(rows) > b.height {
            return false;
        }
        for (i, src) in pixels.chunks(span).enumerate() {
            b.write_span(x, row + i as u32, src);
        }
        true
    }

    /// The area [`Surfaces::write_area`] wrote is the window's picture
    /// there now: only it is damaged (`x, y, width, height`).
    pub fn committed_area(&mut self, desktop: &mut Desktop, hwnd: u32, (x, y, w, h): (u32, u32, u32, u32)) -> bool {
        let Some(s) = self.surface(hwnd) else { return false };
        if desktop.surface(s).and_then(|x| x.buffer).is_none() {
            return false;
        }
        desktop.damage(s, x, y, w, h);
        true
    }

    /// The rows [`Surfaces::write_rows`] wrote are the window's picture now:
    /// the whole surface is damaged.
    pub fn committed(&mut self, desktop: &mut Desktop, hwnd: u32) -> bool {
        let Some(s) = self.surface(hwnd) else { return false };
        let Some(info) = desktop.surface(s).and_then(|x| x.buffer) else { return false };
        let Some(b) = self.buffers.get(&info.id) else { return false };
        desktop.damage(s, 0, 0, b.width, b.height);
        true
    }

    /// Rows `row .. row + rows` of buffer `id` (fewer past its end), for
    /// the device's `READ_BUFFER`; never more than [`BAND_BYTES`] of them.
    pub fn rows(&self, id: BufferId, row: u32, rows: u32) -> Option<Vec<u8>> {
        let b = self.buffers.get(&id)?;
        let stride = (b.width as usize * 4).max(1);
        let most = (BAND_BYTES / stride).max(1) as u32;
        let first = row.min(b.height);
        let last = row.saturating_add(rows.min(most)).min(b.height);
        Some(b.read(first, last))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::frame::FrameMetrics;
    use crate::frame::Rect;
    use crate::link::{FromCompositor, Stream, ToCompositor, VERSION, encode_all};
    use crate::proto::{ClassRef, ClassSpec, CreateSpec, Reply, Request};
    use crate::wm::{Ctx, Effects};

    const CTX: Ctx = Ctx { pid: 7, tid: 8, now: 0 };

    fn call(wm: &mut Wm, req: Request) -> Reply {
        wm.handle(CTX, req, &mut Effects::default())
    }

    fn records(d: &mut Desktop) -> Vec<ToCompositor> {
        let mut s = Stream::<ToCompositor>::new();
        s.push(&d.take_output());
        s.map(|r| r.unwrap()).collect()
    }

    fn connect(d: &mut Desktop) {
        let hello =
            FromCompositor::Hello { version: VERSION, width: 1280, height: 800, scale: 1, frame: FrameMetrics::POLY };
        d.receive(&encode_all(&[hello]));
    }

    /// A captioned 300x200 window at (100, 50) titled "Hello", as
    /// `user32` makes it (the window manager's part).
    fn window(wm: &mut Wm, visible: bool) -> u32 {
        let name: Vec<u16> = "Fill".encode_utf16().collect();
        let spec = ClassSpec { name: name.clone(), wndproc: 1, unicode: true, ..ClassSpec::default() };
        call(wm, Request::RegisterClass { spec });
        let spec = CreateSpec {
            class: ClassRef::Name { name },
            style: 0x00CF_0000,
            cx: 300,
            cy: 200,
            ..CreateSpec::default()
        };
        let Reply::Created { created } = call(wm, Request::CreateWindow { spec }) else { panic!("not created") };
        let h = created.hwnd;
        let window_rel = Some(Rect::new(100, 50, 400, 250));
        let client_rel = Some(Rect::new(108, 81, 392, 242));
        call(wm, Request::SetRects { hwnd: h, window_rel, client_rel });
        call(wm, Request::FinishCreate { hwnd: h });
        call(wm, Request::SetText { hwnd: h, text: "Hello".encode_utf16().collect() });
        call(
            wm,
            Request::SetState {
                hwnd: h,
                visible: Some(visible),
                minimized: None,
                maximized: None,
                normal: None,
                size_move_pending: None,
            },
        );
        h
    }

    /// A 40x30 popup at (`x`, `y`) with extended style `ex`, shown, and
    /// its surface filled with the `COLORREF` `fill`.
    fn popup(wm: &mut Wm, d: &mut Desktop, s: &mut Surfaces, x: i32, y: i32, ex: u32, fill: u32) -> u32 {
        let name: Vec<u16> = "Pop".encode_utf16().collect();
        let spec = ClassSpec { name: name.clone(), wndproc: 1, unicode: true, ..ClassSpec::default() };
        call(wm, Request::RegisterClass { spec });
        let spec =
            CreateSpec { class: ClassRef::Name { name }, style: 0x8000_0000, ex_style: ex, ..CreateSpec::default() };
        let Reply::Created { created } = call(wm, Request::CreateWindow { spec }) else { panic!("not created") };
        let h = created.hwnd;
        let r = Some(Rect::new(x, y, x + 40, y + 30));
        call(wm, Request::SetRects { hwnd: h, window_rel: r, client_rel: r });
        call(wm, Request::FinishCreate { hwnd: h });
        call(
            wm,
            Request::SetState {
                hwnd: h,
                visible: Some(true),
                minimized: None,
                maximized: None,
                normal: None,
                size_move_pending: None,
            },
        );
        s.sync(wm, d);
        let px: Vec<u8> =
            (0..40 * 30).flat_map(|_| [(fill >> 16) as u8, (fill >> 8) as u8, fill as u8, 0xFF]).collect();
        assert!(s.commit(d, h, 40, 30, &px));
        h
    }

    const BLUE: u32 = 0x00FF_0000;
    const RED: u32 = 0x0000_00FF;
    const GREEN: u32 = 0x0000_FF00;

    /// A child draws into its top-level window's surface where it is, clipped to it (Chrome's
    /// GPU process presents into a child of the browser's window).
    #[test]
    fn a_child_window_draws_into_its_top_level_surface() {
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        let top = popup(&mut wm, &mut d, &mut s, 100, 100, 0, BLUE);
        let child = |wm: &mut Wm, x: i32, y: i32, w: i32, h: i32| {
            let spec = CreateSpec {
                class: ClassRef::Name { name: "Pop".encode_utf16().collect() },
                style: 0x5000_0000,
                parent: u64::from(top),
                ..CreateSpec::default()
            };
            let Reply::Created { created } = call(wm, Request::CreateWindow { spec }) else { panic!() };
            let r = Some(Rect::new(x, y, x + w, y + h));
            call(wm, Request::SetRects { hwnd: created.hwnd, window_rel: r, client_rel: r });
            created.hwnd
        };
        let c = child(&mut wm, 10, 5, 20, 10);
        let area = s.child_area(&wm, &d, c).expect("a child of a window with a surface");
        assert_eq!((area.top, area.x, area.y, area.width, area.height), (top, 10, 5, 20, 10));
        let red: Vec<u8> = (0..20 * 10).flat_map(|_| [0, 0, 0xFF, 0xFF]).collect();
        assert!(s.write_child_rows(&d, &area, 0, &red));
        assert!(s.child_committed(&mut d, &area));
        assert_eq!(s.screen_pixel(&wm, &d, 110, 105), RED);
        assert_eq!(s.screen_pixel(&wm, &d, 129, 114), RED);
        assert_eq!(s.screen_pixel(&wm, &d, 109, 105), BLUE);
        assert_eq!(s.screen_pixel(&wm, &d, 130, 105), BLUE);
        // Partly outside: only what is inside the 40x30 surface is written.
        let c2 = child(&mut wm, 30, 25, 20, 10);
        let a2 = s.child_area(&wm, &d, c2).unwrap();
        let green: Vec<u8> = (0..20 * 10).flat_map(|_| [0, 0xFF, 0, 0xFF]).collect();
        assert!(s.write_child_rows(&d, &a2, 0, &green));
        assert_eq!(s.screen_pixel(&wm, &d, 139, 129), GREEN);
        // A top-level window is no child.
        assert_eq!(s.child_area(&wm, &d, top), None);
    }

    /// ADR 0224: a section presented into a child window is a layer of the top-level window's
    /// surface, placed at the child and clipped to it and to the surface; presenting it again only
    /// damages; another section is a second buffer; a resized swap chain's old buffers go; the
    /// layer goes with its swap chain or its process, and its buffers' pages are found by id.
    #[test]
    fn a_presented_section_is_a_layer_of_the_top_level_surface() {
        use crate::layers::{Layers, Outcome, Presented};
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        let mut l = Layers::new();
        connect(&mut d);
        let top = popup(&mut wm, &mut d, &mut s, 100, 100, 0, BLUE);
        let surface = s.surface(top).unwrap();
        let spec = CreateSpec {
            class: ClassRef::Name { name: "Pop".encode_utf16().collect() },
            style: 0x5000_0000,
            parent: u64::from(top),
            ..CreateSpec::default()
        };
        let Reply::Created { created } = call(&mut wm, Request::CreateWindow { spec }) else { panic!() };
        let child = created.hwnd;
        let r = Some(Rect::new(10, 5, 50, 35));
        call(&mut wm, Request::SetRects { hwnd: child, window_rel: r, client_rel: r });
        records(&mut d);
        let a = Presented { object: 70, backing: 700, width: 40, height: 30, stride: 160 };
        let got = l.present(&s, &wm, &mut d, child, 9, a, &[(1, 2, 4, 6)]);
        assert_eq!(got, Some(Outcome { adopted: true, dropped: vec![] }));
        let r = records(&mut d);
        let ToCompositor::LayerBuffer { layer, buffer: ba, .. } = r[0] else { panic!("{r:?}") };
        assert_eq!(
            r,
            vec![
                ToCompositor::LayerBuffer { surface, layer, buffer: ba, width: 40, height: 30, stride: 160 },
                // At the child's (10, 5); the surface is 40x30, so 30x25 of it shows.
                ToCompositor::LayerPlace { surface, layer, x: 10, y: 5, clip: (10, 5, 30, 25) },
                ToCompositor::LayerDamage { surface, layer, buffer: ba, x: 1, y: 2, width: 3, height: 4 },
            ]
        );
        assert_eq!(l.backing(ba), Some(700));
        // Again: only damage (the whole buffer without rectangles).
        assert_eq!(l.present(&s, &wm, &mut d, child, 9, a, &[]), Some(Outcome::default()));
        assert_eq!(
            records(&mut d),
            vec![ToCompositor::LayerDamage { surface, layer, buffer: ba, x: 0, y: 0, width: 40, height: 30 }]
        );
        // The second buffer of a flip-model swap chain.
        let b = Presented { object: 71, backing: 701, ..a };
        assert!(l.present(&s, &wm, &mut d, child, 9, b, &[]).unwrap().adopted);
        let r = records(&mut d);
        assert!(matches!(r[0], ToCompositor::LayerBuffer { buffer, .. } if buffer != ba), "{r:?}");
        // Resized: the old buffers go, their objects are let go of.
        let c = Presented { object: 72, backing: 702, width: 20, height: 10, stride: 80 };
        let got = l.present(&s, &wm, &mut d, child, 9, c, &[]).unwrap();
        assert_eq!(got, Outcome { adopted: true, dropped: vec![70, 71] });
        let r = records(&mut d);
        assert!(matches!(r[0], ToCompositor::LayerDrop { buffer, .. } if buffer == ba), "{r:?}");
        assert!(r.contains(&ToCompositor::LayerPlace { surface, layer, x: 10, y: 5, clip: (10, 5, 20, 10) }));
        // A window that shows nowhere has no layer.
        assert_eq!(l.present(&s, &wm, &mut d, 0xDEAD, 9, a, &[]), None);
        // A top-level window's own layer, replayed to a reconnecting compositor; it goes with its
        // process.
        let mut d = Desktop::new();
        connect(&mut d);
        let top2 = popup(&mut wm, &mut d, &mut s, 300, 300, 0, BLUE);
        let mut l2 = Layers::new();
        l2.present(&s, &wm, &mut d, top2, 5, a, &[]).unwrap();
        let s2 = s.surface(top2).unwrap();
        records(&mut d);
        connect(&mut d);
        let r = records(&mut d);
        assert!(r.iter().any(|x| matches!(x, ToCompositor::LayerBuffer { surface, .. } if *surface == s2)));
        assert!(r.contains(&ToCompositor::LayerPlace { surface: s2, layer: 1, x: 0, y: 0, clip: (0, 0, 40, 30) }));
        assert!(r.iter().any(|x| matches!(x, ToCompositor::LayerDamage { width: 40, height: 30, .. })));
        assert_eq!(l2.remove_process(&mut d, 5), vec![70]);
        assert_eq!(records(&mut d), vec![ToCompositor::LayerGone { surface: s2, layer: 1 }]);
        assert!(l2.is_empty());
    }

    /// SetWindowRgn.yaml `shape_hit_test`, `window_coordinates`, `shapes`,
    /// `removal`: only the part of a window inside its region is on the
    /// screen and hit; the region moves with the window; an empty region
    /// hides it; a region larger than the window does not enlarge it.
    #[test]
    fn a_window_region_cuts_the_screen_and_the_hit_test() {
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        let below = popup(&mut wm, &mut d, &mut s, 245, 245, 0, BLUE);
        let top = popup(&mut wm, &mut d, &mut s, 250, 250, 0, RED);
        let at = |wm: &mut Wm, x, y| match call(wm, Request::WindowFromPoint { x, y, below: 0 }) {
            Reply::Handle { hwnd } => hwnd,
            r => panic!("{r:?}"),
        };
        assert_eq!(at(&mut wm, 255, 255), top);
        assert_eq!(s.screen_pixel(&wm, &d, 255, 255), RED);
        // The region 10,10,30,20 covers the screen points 260..279, 260..269.
        let rgn = |wm: &mut Wm, rects: Vec<u32>| call(wm, Request::WindowRgn { hwnd: top, set: true, rects });
        assert_eq!(rgn(&mut wm, vec![10, 10, 30, 20]), Reply::Ok {});
        for (x, y) in [(260, 260), (279, 269), (265, 265)] {
            assert_eq!(at(&mut wm, x, y), top, "{x},{y}");
            assert_eq!(s.screen_pixel(&wm, &d, x, y), RED, "{x},{y}");
        }
        for (x, y) in [(255, 265), (265, 270), (280, 265), (259, 259)] {
            assert_eq!(at(&mut wm, x, y), below, "{x},{y}");
            assert_eq!(s.screen_pixel(&wm, &d, x, y), BLUE, "{x},{y}");
        }
        assert!(wm.region_excludes(top, 255, 265) && !wm.region_excludes(top, 265, 265));
        // The region moves with the window.
        let r = Some(Rect::new(255, 255, 295, 285));
        call(&mut wm, Request::SetRects { hwnd: top, window_rel: r, client_rel: r });
        assert_eq!(at(&mut wm, 265, 265), top);
        assert_eq!(at(&mut wm, 262, 262), below);
        assert_eq!(at(&mut wm, 284, 274), top);
        let r = Some(Rect::new(250, 250, 290, 280));
        call(&mut wm, Request::SetRects { hwnd: top, window_rel: r, client_rel: r });
        // Two rectangles; an empty region hides the whole window.
        rgn(&mut wm, vec![0, 0, 5, 5, 20, 20, 40, 30]);
        assert_eq!(at(&mut wm, 252, 252), top);
        assert_eq!(at(&mut wm, 275, 275), top);
        assert_eq!(at(&mut wm, 260, 260), below);
        rgn(&mut wm, Vec::new());
        for (x, y) in [(250, 250), (265, 265), (279, 269)] {
            assert_eq!(at(&mut wm, x, y), below, "{x},{y}");
            assert_eq!(s.screen_pixel(&wm, &d, x, y), BLUE, "{x},{y}");
        }
        // A region larger than the window does not enlarge it.
        rgn(&mut wm, vec![(-50i32) as u32, (-50i32) as u32, 500, 500]);
        assert_eq!(at(&mut wm, 289, 279), top);
        assert_ne!(at(&mut wm, 290, 279), top);
        assert_ne!(at(&mut wm, 249, 250), top);
        // Removing it gives the whole window back.
        rgn(&mut wm, vec![10, 10, 30, 20]);
        assert_eq!(call(&mut wm, Request::WindowRgn { hwnd: top, set: false, rects: Vec::new() }), Reply::Ok {});
        assert_eq!(at(&mut wm, 255, 255), top);
        assert_eq!(s.screen_pixel(&wm, &d, 255, 255), RED);
        let gone = Request::WindowRgn { hwnd: 0xBAD0, set: false, rects: Vec::new() };
        assert!(matches!(call(&mut wm, gone), Reply::Error { code: 1400 }));
    }

    /// UpdateLayeredWindow.yaml: the content the call gave is what the
    /// screen shows, composed by the call's flags over what is below; hits
    /// pass where a per-pixel alpha is 0; SetLayeredWindowAttributes after
    /// it takes over; the content goes with the layered style.
    #[test]
    fn update_layered_window_content_is_composed_on_the_screen() {
        const LAYERED: u32 = 0x0008_0000;
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        let below = popup(&mut wm, &mut d, &mut s, 100, 100, 0, BLUE);
        let top = popup(&mut wm, &mut d, &mut s, 100, 100, LAYERED, RED);
        let lay = |wm: &mut Wm, op, key, alpha, flags| call(wm, Request::Layered { hwnd: top, op, key, alpha, flags });
        // Never given attributes or content: nothing of it shows.
        assert_eq!(s.screen_pixel(&wm, &d, 105, 105), BLUE);
        assert!(!s.set_content(&wm, below, 1, 1, vec![0; 4]));
        // 4x1 pixels: opaque green, 50% red (premultiplied), alpha 0 with colour, all zero.
        let content = vec![0, 0xFF, 0, 0xFF, 0, 0, 0x80, 0x80, 0, 0, 0xFF, 0, 0, 0, 0, 0];
        assert!(!s.set_content(&wm, top, 4, 2, content.clone()));
        assert!(s.set_content(&wm, top, 4, 1, content));
        // ULW_ALPHA with AC_SRC_ALPHA, constant alpha 255.
        assert_eq!(lay(&mut wm, 2, 0, 0, 0), Reply::Ok {});
        assert_eq!(lay(&mut wm, 3, 0, 255, 2 | 0x100), Reply::Ok {});
        let shown = |s: &Surfaces, wm: &Wm, d: &Desktop, x: i32| {
            let c = s.screen_pixel(wm, d, 100 + x, 100);
            (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
        };
        assert_eq!(shown(&s, &wm, &d, 0), 0x00FF00);
        assert_eq!(shown(&s, &wm, &d, 1), 0x80007F);
        assert_eq!(shown(&s, &wm, &d, 2), 0xFF00FF);
        assert_eq!(shown(&s, &wm, &d, 3), 0x0000FF);
        // Past the content the window shows nothing.
        assert_eq!(shown(&s, &wm, &d, 4), 0x0000FF);
        assert_eq!(s.screen_pixel(&wm, &d, 100, 101), BLUE);
        assert!(!s.hit_passes(&wm, &d, top, 100, 100));
        assert!(!s.hit_passes(&wm, &d, top, 101, 100));
        assert!(s.hit_passes(&wm, &d, top, 102, 100));
        assert!(s.hit_passes(&wm, &d, top, 103, 100));
        // The same content with a colour key (green) and no blend.
        assert_eq!(lay(&mut wm, 3, GREEN, 0, 1), Reply::Ok {});
        assert_eq!(shown(&s, &wm, &d, 0), 0x0000FF);
        assert_eq!(shown(&s, &wm, &d, 1), 0x800000);
        assert!(!s.hit_passes(&wm, &d, top, 103, 100));
        // SetLayeredWindowAttributes after it: the constant alpha of that call.
        assert!(matches!(lay(&mut wm, 0, 0, 128, 2), Reply::Ok {}));
        assert!(matches!(lay(&mut wm, 2, 0, 0, 0), Reply::Error { code: 87 }));
        assert_eq!(shown(&s, &wm, &d, 0), 0x00807F);
        // Without the style the content is forgotten.
        call(&mut wm, Request::SetStyle { hwnd: top, which: -20, value: 0 });
        s.sync(&wm, &mut d);
        call(&mut wm, Request::SetStyle { hwnd: top, which: -20, value: LAYERED });
        assert_eq!(lay(&mut wm, 3, 0, 255, 2 | 0x100), Reply::Ok {});
        assert_eq!(s.screen_pixel(&wm, &d, 105, 105), RED);
    }

    /// A popup given its content by UpdateLayeredWindow with per-pixel alpha (Chrome's menus and
    /// bubbles) reaches the compositor with that alpha: its buffer is premultiplied `Argb8888`, so
    /// its transparent corners and shadow show what is below instead of black. Other windows stay
    /// opaque, and so does a layered window that loses the per-pixel alpha.
    #[test]
    fn a_layered_window_with_per_pixel_alpha_reaches_the_compositor_with_it() {
        const LAYERED: u32 = 0x0008_0000;
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        let plain = popup(&mut wm, &mut d, &mut s, 10, 10, 0, BLUE);
        let top = popup(&mut wm, &mut d, &mut s, 100, 100, LAYERED, RED);
        let format =
            |s: &Surfaces, d: &Desktop, h: u32| d.surface(s.surface(h).unwrap()).unwrap().buffer.unwrap().format;
        assert_eq!(format(&s, &d, plain), PixelFormat::Xrgb8888);
        // Not yet given content: opaque, as before.
        assert_eq!(format(&s, &d, top), PixelFormat::Xrgb8888);
        // ULW_ALPHA with AC_SRC_ALPHA: a new buffer of the same size, premultiplied alpha.
        let lay = |wm: &mut Wm, flags| call(wm, Request::Layered { hwnd: top, op: 3, key: 0, alpha: 255, flags });
        assert_eq!(lay(&mut wm, 2 | 0x100), Reply::Ok {});
        s.sync(&wm, &mut d);
        assert_eq!(format(&s, &d, top), PixelFormat::Argb8888);
        assert_eq!(d.surface(s.surface(top).unwrap()).unwrap().buffer.unwrap().width, 40);
        // A transparent corner pixel keeps its alpha 0 in what the compositor reads.
        let mut px: Vec<u8> = (0..40 * 30).flat_map(|_| [0xFF, 0xFF, 0xFF, 0xFF]).collect();
        px[..4].copy_from_slice(&[0, 0, 0, 0]);
        assert!(s.commit(&mut d, top, 40, 30, &px));
        let id = d.surface(s.surface(top).unwrap()).unwrap().buffer.unwrap().id;
        assert_eq!(&s.rows(id, 0, 1).unwrap()[..8], &[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        // The same call again changes nothing.
        s.sync(&wm, &mut d);
        assert_eq!(d.surface(s.surface(top).unwrap()).unwrap().buffer.unwrap().id, id);
        // ULW_ALPHA without AC_SRC_ALPHA, or ULW_OPAQUE: opaque again.
        assert_eq!(lay(&mut wm, 4), Reply::Ok {});
        s.sync(&wm, &mut d);
        assert_eq!(format(&s, &d, top), PixelFormat::Xrgb8888);
        // Without the layered style: opaque.
        assert_eq!(lay(&mut wm, 2 | 0x100), Reply::Ok {});
        s.sync(&wm, &mut d);
        assert_eq!(format(&s, &d, top), PixelFormat::Argb8888);
        call(&mut wm, Request::SetStyle { hwnd: top, which: -20, value: 0 });
        s.sync(&wm, &mut d);
        assert_eq!(format(&s, &d, top), PixelFormat::Xrgb8888);
    }

    /// A visible child of `parent` covering its 40x30 client area.
    fn child_of(wm: &mut Wm, parent: u32) -> u32 {
        let spec = CreateSpec {
            class: ClassRef::Name { name: "Pop".encode_utf16().collect() },
            style: 0x5000_0000,
            parent: u64::from(parent),
            ..CreateSpec::default()
        };
        let Reply::Created { created } = call(wm, Request::CreateWindow { spec }) else { panic!() };
        let r = Some(Rect::new(0, 0, 40, 30));
        call(wm, Request::SetRects { hwnd: created.hwnd, window_rel: r, client_rel: r });
        created.hwnd
    }

    /// Chrome's menus and bubbles: popups with `WS_EX_NOREDIRECTIONBITMAP` that a
    /// DirectComposition swap chain of `DXGI_ALPHA_MODE_PREMULTIPLIED` shows in. Their surface
    /// reaches the compositor with that alpha (a corner pixel of alpha 0 is not black), keeps its
    /// picture across the new buffer, and is opaque again when the swap chain goes.
    #[test]
    fn a_window_a_premultiplied_swap_chain_shows_in_is_composed_with_its_alpha() {
        const NO_REDIRECTION: u32 = 0x0020_0000;
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        let menu = popup(&mut wm, &mut d, &mut s, 100, 100, NO_REDIRECTION, RED);
        let info = |s: &Surfaces, d: &Desktop| d.surface(s.surface(menu).unwrap()).unwrap().buffer.unwrap();
        assert_eq!(info(&s, &d).format, PixelFormat::Xrgb8888);
        assert!(!s.set_premultiplied(&wm, 0xBAD0, true));
        // The swap chain is bound to a child of the popup: the popup's surface takes the alpha.
        let child = child_of(&mut wm, menu);
        assert!(s.set_premultiplied(&wm, child, true));
        assert!(s.surface(child).is_none());
        s.sync(&wm, &mut d);
        let b = info(&s, &d);
        assert_eq!((b.format, b.width, b.height), (PixelFormat::Argb8888, 40, 30));
        // The new buffer keeps what the window showed until the next present.
        assert_eq!(s.screen_pixel(&wm, &d, 105, 105), RED);
        // The compositor was told: a Buffer record with the premultiplied format.
        let mut stream = Stream::<ToCompositor>::new();
        stream.push(&d.take_output());
        let records: Vec<ToCompositor> = stream.map(Result::unwrap).collect();
        assert!(
            records.iter().any(|r| matches!(
                r,
                ToCompositor::Buffer { format: PixelFormat::Argb8888, width: 40, height: 30, .. }
            ))
        );
        assert!(s.set_premultiplied(&wm, menu, false));
        s.sync(&wm, &mut d);
        assert_eq!(info(&s, &d).format, PixelFormat::Xrgb8888);
    }

    /// A popup shown before its owner had a surface (Chrome's "Restore pages?" bubble, shown
    /// before the browser window) gets a surface of its own owner once the owner has one: it then
    /// moves with its owner and stays above it, and keeps its picture.
    #[test]
    fn a_popup_shown_before_its_owner_joins_the_owner_once_it_has_a_surface() {
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        let owner = window(&mut wm, false);
        let name: Vec<u16> = "Pop".encode_utf16().collect();
        let spec = ClassSpec { name: name.clone(), wndproc: 1, unicode: true, ..ClassSpec::default() };
        call(&mut wm, Request::RegisterClass { spec });
        let spec = CreateSpec {
            class: ClassRef::Name { name },
            style: 0x8000_0000,
            parent: u64::from(owner),
            ..CreateSpec::default()
        };
        let Reply::Created { created } = call(&mut wm, Request::CreateWindow { spec }) else { panic!() };
        let pop = created.hwnd;
        let r = Some(Rect::new(150, 100, 190, 130));
        call(&mut wm, Request::SetRects { hwnd: pop, window_rel: r, client_rel: r });
        call(&mut wm, Request::FinishCreate { hwnd: pop });
        let show = |wm: &mut Wm, h: u32| {
            call(
                wm,
                Request::SetState {
                    hwnd: h,
                    visible: Some(true),
                    minimized: None,
                    maximized: None,
                    normal: None,
                    size_move_pending: None,
                },
            )
        };
        show(&mut wm, pop);
        s.sync(&wm, &mut d);
        let first = s.surface(pop).expect("the popup has a surface");
        assert_eq!(d.surface(first).unwrap().owner, 0, "no owner surface yet");
        let px: Vec<u8> = (0..40 * 30).flat_map(|_| [0, 0, 0xFF, 0xFF]).collect();
        assert!(s.commit(&mut d, pop, 40, 30, &px));
        // The owner is shown: the popup's surface is made again with it.
        show(&mut wm, owner);
        s.sync(&wm, &mut d);
        let os = s.surface(owner).expect("the owner has a surface");
        let again = s.surface(pop).unwrap();
        assert_ne!(again, first);
        assert_eq!(d.surface(again).unwrap().owner, os);
        assert!(d.surface(again).unwrap().shown);
        // Its picture came along.
        assert_eq!(s.screen_pixel(&wm, &d, 160, 110), RED);
        // Nothing changes at the next sync.
        s.sync(&wm, &mut d);
        assert_eq!(s.surface(pop), Some(again));
    }

    /// SetLayeredWindowAttributes.yaml `children`: a WS_CHILD window with
    /// WS_EX_LAYERED is not created (no last error: code 0), and
    /// SetWindowLong leaves the bit clear on a child.
    #[test]
    fn a_layered_child_window_is_not_created() {
        const LAYERED: u32 = 0x0008_0000;
        let mut wm = Wm::new();
        let parent = window(&mut wm, true);
        let name: Vec<u16> = "Fill".encode_utf16().collect();
        let child = |ex| CreateSpec {
            class: ClassRef::Name { name: name.clone() },
            style: 0x4000_0000,
            ex_style: ex,
            parent: u64::from(parent),
            cx: 10,
            cy: 10,
            ..CreateSpec::default()
        };
        assert!(matches!(call(&mut wm, Request::CreateWindow { spec: child(LAYERED) }), Reply::Error { code: 0 }));
        let Reply::Created { created } = call(&mut wm, Request::CreateWindow { spec: child(0) }) else { panic!() };
        call(&mut wm, Request::SetStyle { hwnd: created.hwnd, which: -20, value: LAYERED });
        assert_eq!(wm.layered(created.hwnd), None);
        let set = Request::Layered { hwnd: created.hwnd, op: 0, key: 0, alpha: 1, flags: 2 };
        assert!(matches!(call(&mut wm, set), Reply::Error { code: 87 }));
    }

    #[test]
    fn nothing_happens_without_a_compositor() {
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        let h = window(&mut wm, true);
        s.sync(&wm, &mut d);
        assert_eq!(s.surface(h), None);
        assert_eq!(s.size(&d, h), None);
    }

    #[test]
    fn a_window_that_draws_its_own_caption_gets_no_poly_frame() {
        // The decision alone: Chrome's 900x500 window at 40,40 with its client area from the
        // window's top (884x492, s3-windows.md), maximized (client 8 px below the top), and a
        // plain captioned window (8 px border + 23 px caption).
        let ovl = 0x16CF_0000;
        assert!(!poly_frames(ovl, Rect::new(40, 40, 940, 540), Rect::new(48, 40, 932, 532)));
        assert!(!poly_frames(ovl, Rect::new(-8, -8, 1288, 808), Rect::new(0, 0, 1280, 800)));
        assert!(poly_frames(ovl, Rect::new(100, 50, 400, 250), Rect::new(108, 81, 392, 242)));
        // No caption: never framed (a popup).
        assert!(!poly_frames(0x9400_0000, Rect::new(0, 0, 10, 10), Rect::new(0, 0, 10, 10)));

        // Through the surfaces: the window is a top level without Poly's frame, and gets it
        // when a frame change gives it a caption area again.
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        records(&mut d);
        let h = window(&mut wm, true);
        let rel = |l, t, r, b| Some(Rect::new(l, t, r, b));
        call(
            &mut wm,
            Request::SetRects { hwnd: h, window_rel: rel(100, 50, 400, 250), client_rel: rel(108, 50, 392, 242) },
        );
        s.sync(&wm, &mut d);
        let id = s.surface(h).unwrap();
        let r = records(&mut d);
        assert!(matches!(
            r[0],
            ToCompositor::Create { surface, kind: SurfaceKind::TopLevel, framed: false, .. } if surface == id
        ));
        // Its own sizing borders (8 px left, right and below; none above) take the pointer.
        assert!(r.contains(&ToCompositor::Margins { surface: id, left: 8, top: 0, right: 8, bottom: 8 }));
        s.sync(&wm, &mut d);
        assert!(records(&mut d).is_empty());
        call(
            &mut wm,
            Request::SetRects { hwnd: h, window_rel: rel(100, 50, 400, 250), client_rel: rel(108, 81, 392, 242) },
        );
        s.sync(&wm, &mut d);
        let again = s.surface(h).unwrap();
        assert_ne!(again, id);
        let r = records(&mut d);
        assert!(r.contains(&ToCompositor::Destroy { surface: id }));
        assert!(r.iter().any(|x| matches!(
            x,
            ToCompositor::Create { surface, kind: SurfaceKind::TopLevel, framed: true, .. } if *surface == again
        )));
    }

    #[test]
    fn a_visible_window_is_a_framed_surface_over_its_client_area() {
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        records(&mut d);
        let hidden = window(&mut wm, false);
        s.sync(&wm, &mut d);
        assert_eq!(s.surface(hidden), None);

        let h = window(&mut wm, true);
        s.sync(&wm, &mut d);
        let id = s.surface(h).unwrap();
        let i = wm.info(h).unwrap();
        let (cw, ch) = ((i.client.right - i.client.left) as u32, (i.client.bottom - i.client.top) as u32);
        assert_eq!(s.size(&d, h), Some((cw, ch)));
        let r = records(&mut d);
        assert!(
            matches!(r[0], ToCompositor::Create { surface, kind: SurfaceKind::TopLevel, framed: true, .. } if surface == id)
        );
        assert!(r.contains(&ToCompositor::Geometry {
            surface: id,
            x: i.client.left,
            y: i.client.top,
            width: cw,
            height: ch
        }));
        assert!(r.contains(&ToCompositor::Title { surface: id, text: String::from("Hello") }));
        assert!(r.contains(&ToCompositor::Visible { surface: id, shown: true }));
        let buffer = r
            .iter()
            .find_map(|x| if let ToCompositor::Buffer { buffer, .. } = x { Some(*buffer) } else { None })
            .unwrap();
        // White until drawn.
        assert!(s.rows(buffer, 0, 1).unwrap().iter().all(|&b| b == 0xFF));

        // The app's pixels, committed: damage for the compositor to copy.
        let mut px = vec![0u8; (cw * ch * 4) as usize];
        px[0..4].copy_from_slice(&[0x10, 0x20, 0x30, 0]);
        assert!(s.commit(&mut d, h, cw, ch, &px));
        assert!(!s.commit(&mut d, h, cw + 1, ch, &px));
        assert_eq!(records(&mut d), vec![ToCompositor::Damage { surface: id, x: 0, y: 0, width: cw, height: ch }]);
        assert_eq!(&s.rows(buffer, 0, 1).unwrap()[0..4], &[0x10, 0x20, 0x30, 0]);
        assert_eq!(s.rows(buffer, ch - 1, 5).unwrap().len(), (cw * 4) as usize);

        // Nothing changed: nothing sent.
        s.sync(&wm, &mut d);
        assert!(records(&mut d).is_empty());

        // Destroyed: the surface goes; its buffer once released.
        let mut fx = Effects::default();
        wm.handle(CTX, Request::Free { hwnd: h }, &mut fx);
        s.sync(&wm, &mut d);
        assert_eq!(records(&mut d), vec![ToCompositor::Destroy { surface: id }]);
        assert_eq!(s.surface(h), None);
        d.receive(&encode_all(&[FromCompositor::Released { buffer }]));
        s.sync(&wm, &mut d);
        assert!(s.rows(buffer, 0, 1).is_none());
    }

    /// ADR 0219: a commit of an area changes and damages only that area, and a resized window's
    /// new buffer keeps its picture from the top-left corner (commits carry only what is drawn).
    #[test]
    fn areas_commit_alone_and_a_resized_buffer_keeps_its_picture() {
        let mut wm = Wm::new();
        let mut d = Desktop::new();
        let mut s = Surfaces::new();
        connect(&mut d);
        records(&mut d);
        let h = popup(&mut wm, &mut d, &mut s, 10, 10, 0, 0x00_11_22_33);
        let id = s.surface(h).unwrap();
        records(&mut d);
        let blue = [0xEE, 0, 0, 0];
        let span: Vec<u8> = (0..3).flat_map(|_| blue).collect();
        assert!(s.write_area(&d, h, (40, 30), (5, 6, 3), &[span.clone(), span.clone()].concat()));
        assert!(!s.write_area(&d, h, (40, 30), (38, 6, 3), &span), "beyond the right edge");
        assert!(!s.write_area(&d, h, (41, 30), (5, 6, 3), &span), "another size");
        assert!(s.committed_area(&mut d, h, (5, 6, 3, 2)));
        assert_eq!(records(&mut d), vec![ToCompositor::Damage { surface: id, x: 5, y: 6, width: 3, height: 2 }]);
        let buffer = d.surface(id).unwrap().buffer.unwrap().id;
        let px = |s: &Surfaces, b, x: usize, y: u32| s.rows(b, y, 1).unwrap()[x * 4..x * 4 + 4].to_vec();
        assert_eq!((px(&s, buffer, 5, 6), px(&s, buffer, 7, 7)), (blue.to_vec(), blue.to_vec()));
        assert_eq!(px(&s, buffer, 8, 6), [0x11, 0x22, 0x33, 0xFF], "outside: the fill committed before");

        // Wider and lower: the new buffer starts with the old picture where both have pixels.
        let r = Some(Rect::new(10, 10, 70, 30));
        call(&mut wm, Request::SetRects { hwnd: h, window_rel: r, client_rel: r });
        s.sync(&wm, &mut d);
        let bigger = d.surface(id).unwrap().buffer.unwrap();
        assert_eq!((bigger.width, bigger.height), (60, 20));
        assert_eq!(px(&s, bigger.id, 6, 7), blue.to_vec());
        assert_eq!(px(&s, bigger.id, 39, 19), [0x11, 0x22, 0x33, 0xFF]);
        assert_eq!(px(&s, bigger.id, 40, 0), [BLANK; 4], "new pixels are blank until drawn");
    }

    #[test]
    fn a_large_buffer_is_kept_in_bands_of_rows() {
        // A maximized window's client area on a 1920x1080 screen: more than
        // one kernel allocation holds, so no band may be larger than BAND_BYTES.
        let (w, h) = (1920u32, 1033u32);
        let stride = (w * 4) as usize;
        let mut b = Buffer::new(w, h);
        assert!(b.bands.len() > 1 && b.bands.iter().all(|x| x.len() <= BAND_BYTES && x.len() % stride == 0));
        assert_eq!(b.bands.iter().map(Vec::len).sum::<usize>(), stride * h as usize);
        // Rows written in pieces that straddle the bands, each row filled with its number.
        let mut y = 0u32;
        while y < h {
            let n = 200.min(h - y);
            let part: Vec<u8> = (y..y + n).flat_map(|r| core::iter::repeat_n((r % 251) as u8, stride)).collect();
            assert!(b.write(y, &part));
            y += n;
        }
        for r in [0, 1, b.rows_per_band - 1, b.rows_per_band, h - 1] {
            let row = b.read(r, r + 1);
            assert_eq!(row.len(), stride);
            assert!(row.iter().all(|&v| v == (r % 251) as u8), "row {r}");
        }
        assert_eq!(b.read(b.rows_per_band - 2, b.rows_per_band + 2).len(), 4 * stride);
        // Rows past the end, or a piece that is not whole rows, are refused.
        assert!(!b.write(h, &vec![0u8; stride]));
        assert!(!b.write(0, &vec![0u8; stride + 1]));
        assert!(b.write(h, &[]));
    }
}
