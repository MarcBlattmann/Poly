//! The scene: windows in z-order with what they show, and the damage —
//! the parts of the output that must be drawn again.

use super::frame::{BORDER, FrameLayout, TITLE_HEIGHT, Theme};
use super::text::{BitmapFont, TextRenderer};
use super::{Framebuffer, Rect};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Damage beyond this many rectangles collapses into their bounding box.
const MAX_DAMAGE_RECTS: usize = 16;

/// A window of the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u32);

/// What a window shows: `width` × `height` premultiplied `0xAARRGGBB`
/// pixels, row by row, drawn at `1 / scale` logical pixels each (the
/// surface's buffer scale).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub pixels: Vec<u32>,
}

impl Content {
    /// The content's size in logical pixels.
    pub fn logical_size(&self) -> (i32, i32) {
        let s = self.scale.max(1);
        ((self.width / s) as i32, (self.height / s) as i32)
    }
}

/// A window: a surface's content at a position, maybe in a frame.
#[derive(Clone, Debug)]
pub struct Window {
    id: WindowId,
    /// What the server knows the window as.
    owner: u64,
    /// The content's top-left corner, logical.
    x: i32,
    y: i32,
    content: Content,
    decorated: bool,
    title: String,
    active: bool,
    /// The visible bounds inside the content (xdg window geometry).
    geometry: Option<Rect>,
    /// Minimized: kept, not drawn (ADR 0117).
    hidden: bool,
    /// Maximized or tiled: the frame shows a restore button.
    maximized: bool,
}

impl Window {
    /// Minimized (not drawn, not hit).
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Maximized or tiled (the frame's middle button restores).
    pub fn is_maximized(&self) -> bool {
        self.maximized
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn owner(&self) -> u64 {
        self.owner
    }

    pub fn position(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_decorated(&self) -> bool {
        self.decorated
    }

    /// A window without content is not drawn, frame included; nor is a
    /// minimized one.
    pub fn is_visible(&self) -> bool {
        let (w, h) = self.content.logical_size();
        w > 0 && h > 0 && !self.hidden
    }

    /// The content's rectangle, logical.
    pub fn surface_rect(&self) -> Rect {
        let (w, h) = self.content.logical_size();
        Rect::new(self.x, self.y, w, h)
    }

    /// The visible bounds, logical: the geometry (clamped to the content)
    /// or all of the content.
    pub fn bounds(&self) -> Rect {
        let surface = self.surface_rect();
        match self.geometry {
            Some(g) => {
                let g = g.translate(self.x, self.y).intersect(&surface);
                if g.is_empty() { surface } else { g }
            }
            None => surface,
        }
    }

    pub(crate) fn frame(&self) -> Option<FrameLayout> {
        self.decorated.then(|| FrameLayout::around(self.bounds()))
    }

    /// Everything the window draws on, logical.
    pub fn extent(&self) -> Rect {
        if !self.is_visible() {
            return Rect::default();
        }
        let surface = self.surface_rect();
        match self.frame() {
            Some(f) => surface.union(&f.outer),
            None => surface,
        }
    }
}

/// What is at a point of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// No window.
    Desktop,
    /// A window's content, at (`x`, `y`) in its surface's coordinates.
    Surface {
        window: WindowId,
        x: i32,
        y: i32,
    },
    TitleBar(WindowId),
    Close(WindowId),
    Border(WindowId),
    /// The title bar's maximize (or restore) button.
    Maximize(WindowId),
    /// The title bar's minimize button.
    Minimize(WindowId),
}

/// Where new toplevels go ([`Scene::place`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// From the work area's top left, each new window a step further.
    #[default]
    Cascade,
    /// The first window centred in the work area, the next ones a step
    /// down and right of the centre (the shell's choice, ADR 0107).
    Centered,
}

/// The software cursor's side, logical (the arrow fits in it).
pub const CURSOR_SIZE: (i32, i32) = (12, 19);

/// Windows bottom to top on one output.
#[derive(Debug)]
pub struct Scene {
    /// The output in physical pixels, and its scale.
    width: u32,
    height: u32,
    scale: u32,
    windows: Vec<Window>,
    /// Physical pixels to draw again.
    damage: Vec<Rect>,
    theme: Theme,
    text: Box<dyn TextRenderer>,
    next_id: u32,
    placed: u32,
    placement: Placement,
    /// Logical rows at the top kept free of windows (the shell's ridge).
    reserved_top: i32,
    /// The software cursor's hot spot, logical, if it is shown.
    cursor: Option<(i32, i32)>,
}

impl Scene {
    /// An empty scene on an output of `width` × `height` physical pixels at
    /// `scale` (at least 1), titles in the 8×16 Spleen font.
    pub fn new(width: u32, height: u32, scale: u32) -> Scene {
        Scene::with_text(width, height, scale, Box::new(BitmapFont(&poly_gfx::SPLEEN_8X16)))
    }

    pub fn with_text(width: u32, height: u32, scale: u32, text: Box<dyn TextRenderer>) -> Scene {
        let mut s = Scene {
            width,
            height,
            scale: scale.max(1),
            windows: Vec::new(),
            damage: Vec::new(),
            theme: Theme::NEUTRAL,
            text,
            next_id: 1,
            placed: 0,
            placement: Placement::Cascade,
            reserved_top: 0,
            cursor: None,
        };
        s.damage_all();
        s
    }

    pub fn set_placement(&mut self, p: Placement) {
        self.placement = p;
    }

    /// Keep the top `rows` logical rows free of new, maximized and dragged
    /// windows (a shell's strip along the top).
    pub fn set_reserved_top(&mut self, rows: i32) {
        self.reserved_top = rows.max(0);
    }

    pub fn reserved_top(&self) -> i32 {
        self.reserved_top
    }

    /// Where windows go, logical: the output less the reserved rows.
    pub fn work_area(&self) -> Rect {
        let (w, h) = self.logical_size();
        let top = self.reserved_top.min(h);
        Rect::new(0, top, w, h - top)
    }

    /// The rectangle the cursor covers at (`x`, `y`), logical.
    fn cursor_rect(x: i32, y: i32) -> Rect {
        Rect::new(x, y, CURSOR_SIZE.0, CURSOR_SIZE.1)
    }

    /// Show the software cursor with its hot spot at (`x`, `y`), logical, or
    /// hide it (`None`).
    pub fn set_cursor(&mut self, at: Option<(i32, i32)>) {
        if at == self.cursor {
            return;
        }
        if let Some((x, y)) = self.cursor {
            self.damage_logical(Self::cursor_rect(x, y));
        }
        self.cursor = at;
        if let Some((x, y)) = at {
            self.damage_logical(Self::cursor_rect(x, y));
        }
    }

    pub fn cursor(&self) -> Option<(i32, i32)> {
        self.cursor
    }

    /// Draw titles with `text` from now on (an outline font, ADR 0107).
    pub fn set_text(&mut self, text: Box<dyn TextRenderer>) {
        self.text = text;
        self.damage_all();
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.damage_all();
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub(crate) fn text(&self) -> &dyn TextRenderer {
        &*self.text
    }

    /// The output: physical width and height, and scale.
    pub fn output(&self) -> (u32, u32, u32) {
        (self.width, self.height, self.scale)
    }

    /// The output's size in logical pixels.
    pub fn logical_size(&self) -> (i32, i32) {
        ((self.width / self.scale) as i32, (self.height / self.scale) as i32)
    }

    pub fn set_output(&mut self, width: u32, height: u32, scale: u32) {
        (self.width, self.height, self.scale) = (width, height, scale.max(1));
        self.damage_all();
    }

    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn index(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    /// Mark the whole output for drawing.
    pub fn damage_all(&mut self) {
        self.damage = vec![Rect::new(0, 0, self.width as i32, self.height as i32)];
    }

    pub fn has_damage(&self) -> bool {
        !self.damage.is_empty()
    }

    /// Mark `r` (logical) for drawing.
    pub fn damage_logical(&mut self, r: Rect) {
        let s = self.scale as i32;
        self.damage_physical(r.scale(s));
    }

    fn damage_physical(&mut self, r: Rect) {
        let r = r.intersect(&Rect::new(0, 0, self.width as i32, self.height as i32));
        if r.is_empty() || self.damage.iter().any(|d| d.intersect(&r) == r) {
            return;
        }
        self.damage.retain(|d| r.intersect(d) != *d);
        self.damage.push(r);
        if self.damage.len() > MAX_DAMAGE_RECTS {
            let all = self.damage.iter().fold(Rect::default(), |a, d| a.union(d));
            self.damage = vec![all];
        }
    }

    fn damage_window(&mut self, i: usize) {
        let e = self.windows[i].extent();
        self.damage_logical(e);
    }

    /// A new window on top of the others, without content (so not shown
    /// yet), decorated or not; `owner` is the caller's name for it.
    pub fn add_window(&mut self, owner: u64, decorated: bool) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows.push(Window {
            id,
            owner,
            x: 0,
            y: 0,
            content: Content { scale: 1, ..Content::default() },
            decorated,
            title: String::new(),
            active: false,
            geometry: None,
            hidden: false,
            maximized: false,
        });
        id
    }

    /// Minimize a window (`true`: not drawn, not hit) or show it again.
    pub fn set_hidden(&mut self, id: WindowId, hidden: bool) {
        if let Some(i) = self.index(id)
            && self.windows[i].hidden != hidden
        {
            // Damage while visible: before hiding, after showing.
            self.damage_window(i);
            self.windows[i].hidden = hidden;
            self.damage_window(i);
        }
    }

    /// Show the frame's restore button instead of maximize, or not.
    pub fn set_maximized(&mut self, id: WindowId, maximized: bool) {
        if self.window(id).is_some_and(|w| w.maximized != maximized) {
            self.update(id, |w| w.maximized = maximized);
        }
    }

    /// Put a window at the bottom (under every other).
    pub fn lower(&mut self, id: WindowId) {
        if let Some(i) = self.index(id)
            && i != 0
        {
            let w = self.windows.remove(i);
            self.windows.insert(0, w);
            self.damage_window(0);
        }
    }

    pub fn remove_window(&mut self, id: WindowId) {
        if let Some(i) = self.index(id) {
            self.damage_window(i);
            self.windows.remove(i);
        }
    }

    /// Move a window's content to (`x`, `y`), logical.
    pub fn set_position(&mut self, id: WindowId, x: i32, y: i32) {
        if let Some(i) = self.index(id) {
            self.damage_window(i);
            (self.windows[i].x, self.windows[i].y) = (x, y);
            self.damage_window(i);
        }
    }

    /// Put a window on top.
    pub fn raise(&mut self, id: WindowId) {
        if let Some(i) = self.index(id) {
            if i + 1 == self.windows.len() {
                return;
            }
            let w = self.windows.remove(i);
            self.windows.push(w);
            let top = self.windows.len() - 1;
            self.damage_window(top);
        }
    }

    /// Put `id` right above `parent` (popups, dialogs).
    pub fn place_above(&mut self, id: WindowId, parent: WindowId) {
        let (Some(i), Some(_)) = (self.index(id), self.index(parent)) else { return };
        let w = self.windows.remove(i);
        let p = self.index(parent).unwrap_or(self.windows.len());
        self.windows.insert((p + 1).min(self.windows.len()), w);
        let i = self.index(id).unwrap_or(0);
        self.damage_window(i);
    }

    fn update(&mut self, id: WindowId, f: impl FnOnce(&mut Window)) {
        if let Some(i) = self.index(id) {
            self.damage_window(i);
            f(&mut self.windows[i]);
            self.damage_window(i);
        }
    }

    pub fn set_decorated(&mut self, id: WindowId, decorated: bool) {
        if self.window(id).is_some_and(|w| w.decorated != decorated) {
            self.update(id, |w| w.decorated = decorated);
        }
    }

    pub fn set_title(&mut self, id: WindowId, title: &str) {
        if self.window(id).is_some_and(|w| w.title != title) {
            self.update(id, |w| w.title = title.into());
        }
    }

    pub fn set_active(&mut self, id: WindowId, active: bool) {
        if self.window(id).is_some_and(|w| w.active != active) {
            self.update(id, |w| w.active = active);
        }
    }

    /// The visible bounds inside the content, logical, relative to the
    /// content (`None`: all of it).
    pub fn set_geometry(&mut self, id: WindowId, geometry: Option<Rect>) {
        if self.window(id).is_some_and(|w| w.geometry != geometry) {
            self.update(id, |w| w.geometry = geometry);
        }
    }

    /// Give a window content of `width` × `height` pixels at `scale`,
    /// cleared to transparent, all of it damaged; nothing if it already has
    /// that size.
    pub fn resize_content(&mut self, id: WindowId, width: u32, height: u32, scale: u32) {
        let Some(w) = self.window(id) else { return };
        let c = &w.content;
        if (c.width, c.height, c.scale) == (width, height, scale.max(1)) {
            return;
        }
        self.update(id, |w| {
            w.content =
                Content { width, height, scale: scale.max(1), pixels: vec![0; width as usize * height as usize] };
        });
    }

    /// The content to write new pixels into; say which with
    /// [`Scene::damage_content`].
    pub fn content_mut(&mut self, id: WindowId) -> Option<&mut Content> {
        let i = self.index(id)?;
        Some(&mut self.windows[i].content)
    }

    /// Mark `r`, in the window's buffer pixels, as changed.
    pub fn damage_content(&mut self, id: WindowId, r: Rect) {
        let Some(w) = self.window(id) else { return };
        let c = &w.content;
        let r = r.intersect(&Rect::new(0, 0, c.width as i32, c.height as i32));
        if r.is_empty() {
            return;
        }
        let logical = r.scale_down_out(c.scale as i32).translate(w.x, w.y);
        self.damage_logical(logical);
    }

    /// The frame's outer rectangle of a decorated window, logical.
    pub fn frame_rect(&self, id: WindowId) -> Option<Rect> {
        self.window(id)?.frame().map(|f| f.outer)
    }

    /// What is at the logical point (`x`, `y`), topmost first.
    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        for w in self.windows.iter().rev().filter(|w| w.is_visible()) {
            // Framed content is drawn only inside its bounds.
            let surface = if w.decorated { w.bounds() } else { w.surface_rect() };
            if let Some(f) = w.frame() {
                if f.close.contains(x, y) {
                    return Hit::Close(w.id);
                }
                if f.maximize.contains(x, y) {
                    return Hit::Maximize(w.id);
                }
                if f.minimize.contains(x, y) {
                    return Hit::Minimize(w.id);
                }
                if f.title.contains(x, y) {
                    return Hit::TitleBar(w.id);
                }
            }
            if surface.contains(x, y) {
                return Hit::Surface { window: w.id, x: x - w.x, y: y - w.y };
            }
            if w.frame().is_some_and(|f| f.outer.contains(x, y)) {
                return Hit::Border(w.id);
            }
        }
        Hit::Desktop
    }

    /// Where to put a new window whose visible bounds are `width` ×
    /// `height` logical pixels at `(gx, gy)` in its surface, so that its
    /// frame starts on the output: cascading from the top left, each new
    /// window a step further, back to the start when one would leave the
    /// output. Returns the content's position.
    pub fn place(&mut self, gx: i32, gy: i32, width: i32, height: i32, decorated: bool) -> (i32, i32) {
        const START: i32 = 32;
        const STEP: i32 = 32;
        let area = self.work_area();
        let (left, top) = if decorated { (BORDER, TITLE_HEIGHT + BORDER) } else { (0, 0) };
        // The frame's outer size.
        let (fw, fh) = (width + 2 * left, height + top + if decorated { BORDER } else { 0 });
        let origin = match self.placement {
            Placement::Cascade => (area.x + START, area.y + START),
            Placement::Centered => (area.x + ((area.width - fw) / 2).max(0), area.y + ((area.height - fh) / 2).max(0)),
        };
        let mut k = self.placed as i32;
        let at = |k: i32| (origin.0 + STEP * k, origin.1 + STEP * k);
        let (mut fx, mut fy) = at(k);
        if k > 0 && (fx + fw > area.right() || fy + fh > area.bottom()) {
            k = 0;
            (fx, fy) = at(0);
        }
        self.placed = k as u32 + 1;
        (fx + left - gx, fy + top - gy)
    }

    /// Draw what changed into `fb` and present it; returns the rectangles
    /// drawn (physical). A framebuffer of another size than the output's
    /// gets the whole picture, clipped to the smaller of the two.
    pub fn render(&mut self, fb: &mut dyn Framebuffer) -> Vec<Rect> {
        self.render_with(fb, None, None)
    }

    /// [`Scene::render`] with a layer drawn instead of the plain background
    /// (`below`: a wallpaper, a shell's strip) and one over the windows
    /// (`above`: a launcher); the cursor goes on top of everything.
    pub fn render_with(
        &mut self,
        fb: &mut dyn Framebuffer,
        below: Option<&dyn super::Layer>,
        above: Option<&dyn super::Layer>,
    ) -> Vec<Rect> {
        if (fb.width(), fb.height()) != (self.width, self.height) {
            self.damage_all();
        }
        let screen = Rect::new(0, 0, fb.width().min(self.width) as i32, fb.height().min(self.height) as i32);
        let damage: Vec<Rect> =
            core::mem::take(&mut self.damage).iter().map(|d| d.intersect(&screen)).filter(|d| !d.is_empty()).collect();
        for r in &damage {
            super::render::draw(self, fb, *r, below, above);
        }
        if !damage.is_empty() {
            fb.present(&damage);
        }
        damage
    }
}
