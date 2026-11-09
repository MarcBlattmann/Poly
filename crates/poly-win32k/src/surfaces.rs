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

/// The pixel a buffer starts with (white, `COLOR_WINDOW`'s default).
const BLANK: u8 = 0xFF;

#[derive(Debug)]
struct Buffer {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
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
                    let bytes = vec![BLANK; cw as usize * ch as usize * 4];
                    self.buffers.insert(b, Buffer { width: cw, height: ch, bytes });
                    desktop.damage(s, 0, 0, cw, ch);
                }
            }
            if let Some(text) = wm.text(h) {
                desktop.set_title(s, &String::from_utf16_lossy(text));
            }
            desktop.set_visible(s, shown);
        }
    }

    /// What the screen shows at (`x`, `y`) as a `COLORREF`: the topmost
    /// visible top-level window's client pixel there, a layered window
    /// blended with what is below it as `(src * a + below * (255 - a)) / 255`
    /// per channel, its colour key showing what is below, and a layered
    /// window never given attributes showing nothing
    /// (`SetLayeredWindowAttributes.yaml`). Black where no window is.
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
            return false;
        }
        if flags & 2 != 0 && alpha == 0 {
            return true;
        }
        let Some(i) = wm.info(top) else { return false };
        if flags & 1 != 0 && (key & 0xFF) == ((key >> 16) & 0xFF) {
            return self.client_pixel(desktop, top, x - i.client.left, y - i.client.top) == Some(key & 0x00FF_FFFF);
        }
        false
    }

    fn pixel_below(&self, wm: &Wm, desktop: &Desktop, order: &[u32], x: i32, y: i32) -> u32 {
        for (k, &h) in order.iter().enumerate() {
            let Some(i) = wm.info(h) else { continue };
            if !i.visible || i.minimized || i.message_only || !i.client.contains(x, y) {
                continue;
            }
            let Some(pix) = self.client_pixel(desktop, h, x - i.client.left, y - i.client.top) else { continue };
            let Some((key, alpha, flags, attrs, update)) = wm.layered_of(h) else { return pix };
            if !attrs && !update {
                continue;
            }
            if !attrs {
                return pix;
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
        let s = self.surface(hwnd)?;
        let b = desktop.surface(s)?.buffer?;
        let buf = self.buffers.get(&b.id)?;
        if x < 0 || y < 0 || x as u32 >= buf.width || y as u32 >= buf.height {
            return None;
        }
        let at = (y as usize * buf.width as usize + x as usize) * 4;
        let p = buf.bytes.get(at..at + 3)?;
        Some(u32::from(p[2]) | u32::from(p[1]) << 8 | u32::from(p[0]) << 16)
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
        let Some(s) = self.surface(hwnd) else { return false };
        let Some(info) = desktop.surface(s).and_then(|x| x.buffer) else { return false };
        let Some(b) = self.buffers.get_mut(&info.id) else { return false };
        if (b.width, b.height) != (width, height) || pixels.len() != b.bytes.len() {
            return false;
        }
        b.bytes.copy_from_slice(pixels);
        desktop.damage(s, 0, 0, width, height);
        true
    }

    /// Rows `row .. row + rows` of buffer `id` (fewer past its end), for
    /// the device's `READ_BUFFER`.
    pub fn rows(&self, id: BufferId, row: u32, rows: u32) -> Option<&[u8]> {
        let b = self.buffers.get(&id)?;
        let stride = b.width as usize * 4;
        let first = (row.min(b.height)) as usize;
        let last = (row.saturating_add(rows).min(b.height)) as usize;
        Some(&b.bytes[first * stride..last * stride])
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
}
