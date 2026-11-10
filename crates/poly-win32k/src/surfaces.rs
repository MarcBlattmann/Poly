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
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// The largest surface side with a buffer (16 MiB of kernel memory).
pub const MAX_SIDE: u32 = 2048;

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

/// The surfaces of top-level windows and their buffers.
#[derive(Debug, Default)]
pub struct Surfaces {
    windows: BTreeMap<u32, SurfaceId>,
    buffers: BTreeMap<BufferId, Buffer>,
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
        // Surfaces exist with a compositor, and in an interactive session
        // even without one (window DCs keep their pixels, BeginPaint.yaml
        // reads them back).
        if !desktop.is_connected() && !wm.display().interactive && self.windows.is_empty() {
            return;
        }
        let tops = wm.top_levels();
        // Gone, or no longer top-level: the surface goes.
        let gone: Vec<u32> = self.windows.keys().filter(|h| !tops.contains(h)).copied().collect();
        for h in gone {
            if let Some(s) = self.windows.remove(&h) {
                desktop.destroy(s);
            }
        }
        // Bottom to top, so an owner has its surface before what it owns.
        for &h in tops.iter().rev() {
            let Some(i) = wm.info(h) else { continue };
            if i.pid == 0 || i.message_only || wm.is_keeper(h) {
                continue;
            }
            let client = i.client;
            let shown = i.visible && !i.minimized && client.right > client.left && client.bottom > client.top;
            let s = match self.windows.get(&h) {
                Some(&s) => s,
                None if shown => {
                    let framed = i.style & WS_CAPTION == WS_CAPTION;
                    let kind = if framed { SurfaceKind::TopLevel } else { SurfaceKind::Popup };
                    let owner = self.windows.get(&i.owner).copied().unwrap_or(0);
                    let s = desktop.create(kind, owner, framed);
                    desktop.set_process(s, i.pid);
                    self.windows.insert(h, s);
                    s
                }
                None => continue,
            };
            if shown {
                let (cw, ch) = ((client.right - client.left) as u32, (client.bottom - client.top) as u32);
                desktop.set_geometry(s, client.left, client.top, cw, ch);
                let current = desktop.surface(s).and_then(|x| x.buffer).map(|b| (b.width, b.height));
                if current != Some((cw, ch))
                    && cw <= MAX_SIDE
                    && ch <= MAX_SIDE
                    && let Some(b) = desktop.attach_buffer(s, cw, ch, cw * 4, PixelFormat::Xrgb8888)
                {
                    self.buffers.insert(b, Buffer::new(cw, ch));
                    desktop.damage(s, 0, 0, cw, ch);
                }
            }
            if let Some(text) = wm.text(h) {
                desktop.set_title(s, &String::from_utf16_lossy(text));
            }
            desktop.set_visible(s, shown);
        }
    }

    /// The size of window `hwnd`'s surface buffer, for its DCs.
    pub fn size(&self, desktop: &Desktop, hwnd: u32) -> Option<(u32, u32)> {
        let s = self.surface(hwnd)?;
        let b = desktop.surface(s)?.buffer?;
        self.buffers.contains_key(&b.id).then_some((b.width, b.height))
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
