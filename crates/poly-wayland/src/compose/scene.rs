//! The scene: windows in z-order with what they show, and the damage —
//! the parts of the output that must be drawn again.

use super::frame::{BORDER, FrameLayout, TITLE_HEIGHT, Theme};
use super::text::{BitmapFont, TextRenderer};
use super::{Framebuffer, Rect};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Damage beyond this many rectangles collapses into their bounding box.
const MAX_DAMAGE_RECTS: usize = 16;

/// A window of the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u32);

/// How a buffer maps onto its surface besides the buffer scale: the
/// buffer transform (`wl_surface.set_buffer_transform`, a
/// `wl_output.transform` the client already applied, which the compositor
/// undoes) and a viewport (`wp_viewport`: a source rectangle cropped from
/// the surface and the size it is shown at).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// `wl_output.transform`, 0 to 7: 90° steps counter-clockwise, 4 and
    /// up flipped around the vertical axis first.
    pub transform: u32,
    /// The source rectangle in surface coordinates before the viewport
    /// scales them, in 1/256 logical pixels (`wl_fixed`): x, y, width,
    /// height. `None`: all of the surface.
    pub source: Option<(i32, i32, i32, i32)>,
    /// The size shown, logical. `None`: the source's size.
    pub destination: Option<(i32, i32)>,
}

impl View {
    /// Nothing but the buffer scale.
    pub fn is_identity(&self) -> bool {
        *self == View::default()
    }
}

/// Where surface pixel (`u`, `v`) of a `w` × `h` surface (both in buffer
/// pixels, the transform undone) is in a buffer the client drew with
/// `transform` applied (wayland.xml's `wl_output.transform`: flip around
/// the vertical axis, then turn counter-clockwise).
pub fn buffer_point(transform: u32, u: i32, v: i32, w: i32, h: i32) -> (i32, i32) {
    let u = if transform & 4 != 0 { w - 1 - u } else { u };
    match transform & 3 {
        0 => (u, v),
        1 => (v, w - 1 - u),
        2 => (w - 1 - u, h - 1 - v),
        _ => (h - 1 - v, u),
    }
}

/// What a window shows: `width` × `height` premultiplied `0xAARRGGBB`
/// pixels, row by row, drawn at `1 / scale` logical pixels each (the
/// surface's buffer scale), through its [`View`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub pixels: Vec<u32>,
    pub view: View,
}

impl Content {
    /// The buffer's size with the transform undone, in buffer pixels.
    pub fn transformed_size(&self) -> (u32, u32) {
        if self.view.transform & 1 != 0 { (self.height, self.width) } else { (self.width, self.height) }
    }

    /// The content's size in logical pixels: the viewport's destination,
    /// else its source's size, else the transformed buffer over its scale.
    pub fn logical_size(&self) -> (i32, i32) {
        if self.width == 0 || self.height == 0 {
            return (0, 0);
        }
        if let Some(d) = self.view.destination {
            return d;
        }
        if let Some((_, _, w, h)) = self.view.source {
            return (w >> 8, h >> 8);
        }
        let s = self.scale.max(1);
        let (w, h) = self.transformed_size();
        ((w / s) as i32, (h / s) as i32)
    }

    /// The buffer pixel shown at physical pixel (`x`, `y`) of a picture of
    /// the content `dw` × `dh` physical pixels large (nearest neighbour).
    pub fn sample(&self, x: i32, y: i32, dw: i32, dh: i32) -> u32 {
        let s = i64::from(self.scale.max(1));
        let (tw, th) = self.transformed_size();
        let (tw, th) = (i64::from(tw), i64::from(th));
        if tw == 0 || th == 0 {
            return 0;
        }
        // The source in 1/256 logical pixels.
        let (sx, sy, sw, sh) = match self.view.source {
            Some((x, y, w, h)) => (i64::from(x), i64::from(y), i64::from(w), i64::from(h)),
            None => (0, 0, tw * 256 / s, th * 256 / s),
        };
        let (dw, dh) = (i64::from(dw.max(1)), i64::from(dh.max(1)));
        // The pixel's centre in surface coordinates, then in buffer pixels.
        let su = sx + (2 * i64::from(x) + 1) * sw / (2 * dw);
        let sv = sy + (2 * i64::from(y) + 1) * sh / (2 * dh);
        let u = (su * s / 256).clamp(0, tw - 1) as i32;
        let v = (sv * s / 256).clamp(0, th - 1) as i32;
        let (bx, by) = buffer_point(self.view.transform, u, v, tw as i32, th as i32);
        let i = by as usize * self.width as usize + bx as usize;
        self.pixels.get(i).copied().unwrap_or(0)
    }
}

/// What the pointer looks like ([`Scene::set_cursor_look`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorLook {
    /// Poly's arrow.
    #[default]
    Arrow,
    /// A client's cursor surface (`wl_pointer.set_cursor`): its content,
    /// with the hot spot at (`hx`, `hy`) logical pixels into it.
    Image { content: Content, hx: i32, hy: i32 },
    /// No cursor (`set_cursor` with no surface).
    Hidden,
}

/// The composed picture of what lies behind a window (the layers below it
/// and the windows under it, without the window itself): what a material
/// like Liquid Glass samples and blurs. Physical pixels of [`Backdrop::rect`],
/// rows of `xrgb8888`, kept up to date as frames are drawn: the renderer
/// draws each damaged rectangle bottom to top, so just before it draws the
/// window the framebuffer holds exactly what is behind it there, and that
/// part is copied. Nothing is drawn twice for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backdrop {
    rect: Rect,
    pixels: Vec<u32>,
    /// What changed since [`Scene::take_backdrop_damage`], physical.
    damage: Vec<Rect>,
}

impl Backdrop {
    /// Where the picture is on the output, physical: the window's extent
    /// (frame included), cut to the output.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Rows of [`Backdrop::rect`]'s width, `xrgb8888`.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// The pixel at output position (`x`, `y`), physical, as `0xRRGGBB`.
    pub fn pixel(&self, x: i32, y: i32) -> Option<u32> {
        if !self.rect.contains(x, y) {
            return None;
        }
        let i = (y - self.rect.y) as usize * self.rect.width as usize + (x - self.rect.x) as usize;
        Some(self.pixels[i] & 0x00ff_ffff)
    }

    /// Take over the part `clip` (inside `rect`) of what `fb` holds.
    pub(crate) fn capture(&mut self, fb: &mut dyn Framebuffer, clip: Rect) {
        let r = clip.intersect(&self.rect);
        if r.is_empty() {
            return;
        }
        let w = self.rect.width as usize;
        for y in r.y..r.bottom() {
            let src = &fb.row_mut(y as u32)[r.x as usize..r.right() as usize];
            let at = (y - self.rect.y) as usize * w + (r.x - self.rect.x) as usize;
            self.pixels[at..at + r.width as usize].copy_from_slice(src);
        }
        add_damage(&mut self.damage, r);
    }

    /// Follow the window to `rect`: a new, black picture if it moved or
    /// changed size (the scene damaged all of it then, so the frame being
    /// drawn fills it in).
    pub(crate) fn follow(&mut self, rect: Rect) {
        if rect != self.rect {
            self.rect = rect;
            self.pixels = vec![0xff00_0000; rect.width.max(0) as usize * rect.height.max(0) as usize];
            self.damage.clear();
        }
    }
}

/// Add `r` to a damage list: dropped if covered, covering ones replaced,
/// the bounding box past [`MAX_DAMAGE_RECTS`].
fn add_damage(list: &mut Vec<Rect>, r: Rect) {
    if r.is_empty() || list.iter().any(|d| d.intersect(&r) == r) {
        return;
    }
    list.retain(|d| r.intersect(d) != *d);
    list.push(r);
    if list.len() > MAX_DAMAGE_RECTS {
        let all = list.iter().fold(Rect::default(), |a, d| a.union(d));
        *list = vec![all];
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
    /// The window it is part of (a subsurface's parent, a popup's): it
    /// moves and is raised with it.
    parent: Option<WindowId>,
}

impl Window {
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

    /// The window this one is part of, if any ([`Scene::set_parent`]).
    pub fn parent(&self) -> Option<WindowId> {
        self.parent
    }

    /// A window without content is not drawn, frame included.
    pub fn is_visible(&self) -> bool {
        let (w, h) = self.content.logical_size();
        w > 0 && h > 0
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
    cursor_look: CursorLook,
    /// The windows whose backdrop is kept ([`Scene::set_backdrop`]).
    backdrops: BTreeMap<WindowId, Backdrop>,
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
            cursor_look: CursorLook::Arrow,
            backdrops: BTreeMap::new(),
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

    /// The rectangle the cursor covers with its hot spot at (`x`, `y`),
    /// logical.
    pub fn cursor_rect(&self, x: i32, y: i32) -> Rect {
        match &self.cursor_look {
            CursorLook::Arrow => Rect::new(x, y, CURSOR_SIZE.0, CURSOR_SIZE.1),
            CursorLook::Image { content, hx, hy } => {
                let (w, h) = content.logical_size();
                Rect::new(x - hx, y - hy, w, h)
            }
            CursorLook::Hidden => Rect::default(),
        }
    }

    /// Show the software cursor with its hot spot at (`x`, `y`), logical, or
    /// hide it (`None`).
    pub fn set_cursor(&mut self, at: Option<(i32, i32)>) {
        if at == self.cursor {
            return;
        }
        if let Some((x, y)) = self.cursor {
            self.damage_logical(self.cursor_rect(x, y));
        }
        self.cursor = at;
        if let Some((x, y)) = at {
            self.damage_logical(self.cursor_rect(x, y));
        }
    }

    pub fn cursor(&self) -> Option<(i32, i32)> {
        self.cursor
    }

    /// What the cursor looks like from now on: the arrow, a client's
    /// image, or nothing.
    pub fn set_cursor_look(&mut self, look: CursorLook) {
        if look == self.cursor_look {
            return;
        }
        if let Some((x, y)) = self.cursor {
            self.damage_logical(self.cursor_rect(x, y));
        }
        self.cursor_look = look;
        if let Some((x, y)) = self.cursor {
            self.damage_logical(self.cursor_rect(x, y));
        }
    }

    pub fn cursor_look(&self) -> &CursorLook {
        &self.cursor_look
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

    /// The physical rectangles waiting to be drawn.
    pub fn damage(&self) -> &[Rect] {
        &self.damage
    }

    /// Mark `r` (logical) for drawing.
    pub fn damage_logical(&mut self, r: Rect) {
        let s = self.scale as i32;
        self.damage_physical(r.scale(s));
    }

    /// Mark `r` (physical) for drawing.
    pub fn damage_physical(&mut self, r: Rect) {
        let r = r.intersect(&Rect::new(0, 0, self.width as i32, self.height as i32));
        add_damage(&mut self.damage, r);
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
            parent: None,
        });
        id
    }

    /// Take a window away; the windows that were part of it are on their
    /// own from now on (their owner removes them too).
    pub fn remove_window(&mut self, id: WindowId) {
        if let Some(i) = self.index(id) {
            self.damage_window(i);
            self.windows.remove(i);
            for w in self.windows.iter_mut().filter(|w| w.parent == Some(id)) {
                w.parent = None;
            }
            self.backdrops.remove(&id);
        }
    }

    /// Make `id` part of `parent` (a subsurface of its parent surface, a
    /// popup of its parent): it moves with it and is raised with it.
    /// Nothing if that would make a cycle.
    pub fn set_parent(&mut self, id: WindowId, parent: Option<WindowId>) {
        if let Some(p) = parent
            && (p == id || self.ancestors(p).contains(&id))
        {
            return;
        }
        if let Some(i) = self.index(id) {
            self.windows[i].parent = parent;
        }
    }

    /// The windows `id` is part of, nearest first.
    fn ancestors(&self, id: WindowId) -> Vec<WindowId> {
        let mut out = Vec::new();
        let mut at = self.window(id).and_then(|w| w.parent);
        while let Some(p) = at {
            if out.contains(&p) {
                break;
            }
            out.push(p);
            at = self.window(p).and_then(|w| w.parent);
        }
        out
    }

    /// The window `id` is ultimately part of (itself if none).
    pub fn root(&self, id: WindowId) -> WindowId {
        self.ancestors(id).last().copied().unwrap_or(id)
    }

    /// `id` and every window part of it, bottom to top.
    pub fn family(&self, id: WindowId) -> Vec<WindowId> {
        self.windows.iter().filter(|w| w.id == id || self.ancestors(w.id).contains(&id)).map(|w| w.id).collect()
    }

    /// Move a window's content to (`x`, `y`), logical; the windows part of
    /// it move along.
    pub fn set_position(&mut self, id: WindowId, x: i32, y: i32) {
        let Some(w) = self.window(id) else { return };
        let (dx, dy) = (x - w.x, y - w.y);
        if (dx, dy) == (0, 0) {
            return;
        }
        for member in self.family(id) {
            if let Some(i) = self.index(member) {
                self.damage_window(i);
                self.windows[i].x += dx;
                self.windows[i].y += dy;
                self.damage_window(i);
            }
        }
    }

    /// Take `id`'s family out of the stack and put it back at the index
    /// `at` picks among what is left, in its order.
    fn move_family(&mut self, id: WindowId, at: impl FnOnce(&[Window]) -> usize) {
        let members = self.family(id);
        let mut moved = Vec::new();
        for m in &members {
            if let Some(i) = self.index(*m) {
                moved.push(self.windows.remove(i));
            }
        }
        let at = at(&self.windows).min(self.windows.len());
        for (k, w) in moved.into_iter().enumerate() {
            self.windows.insert(at + k, w);
        }
        for m in members {
            if let Some(i) = self.index(m) {
                self.damage_window(i);
            }
        }
    }

    /// Put a window on top, with the windows part of it.
    pub fn raise(&mut self, id: WindowId) {
        let family = self.family(id);
        let n = self.windows.len();
        let on_top = self.windows[n.saturating_sub(family.len())..].iter().map(|w| w.id).eq(family.iter().copied());
        if family.is_empty() || on_top {
            return;
        }
        self.move_family(id, |rest| rest.len());
    }

    /// Put `id` (with the windows part of it) on top of `parent` and
    /// everything part of `parent` (popups, dialogs).
    pub fn place_above(&mut self, id: WindowId, parent: WindowId) {
        if self.index(id).is_none() || self.index(parent).is_none() || self.family(id).contains(&parent) {
            return;
        }
        let family = self.family(parent);
        self.move_family(id, |rest| {
            rest.iter().rposition(|w| family.contains(&w.id) && w.id != id).map_or(rest.len(), |i| i + 1)
        });
    }

    /// Put `id` (with its family) directly above `sibling`
    /// (`wl_subsurface.place_above`): right above its parent's own surface
    /// if `sibling` is its parent, else above `sibling`'s whole family.
    pub fn restack_above(&mut self, id: WindowId, sibling: WindowId) {
        if self.index(sibling).is_none() || self.family(id).contains(&sibling) {
            return;
        }
        let own = self.window(id).and_then(|w| w.parent) == Some(sibling);
        let family = if own { vec![sibling] } else { self.family(sibling) };
        self.move_family(id, |rest| rest.iter().rposition(|w| family.contains(&w.id)).map_or(rest.len(), |i| i + 1));
    }

    /// Put `id` (with its family) directly below `sibling`
    /// (`wl_subsurface.place_below`): right below its parent's own surface
    /// if `sibling` is its parent, else below `sibling`'s whole family.
    pub fn restack_below(&mut self, id: WindowId, sibling: WindowId) {
        if self.index(sibling).is_none() || self.family(id).contains(&sibling) {
            return;
        }
        let own = self.window(id).and_then(|w| w.parent) == Some(sibling);
        let family = if own { vec![sibling] } else { self.family(sibling) };
        self.move_family(id, |rest| rest.iter().position(|w| family.contains(&w.id)).unwrap_or(0));
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
            let view = w.content.view;
            w.content =
                Content { width, height, scale: scale.max(1), pixels: vec![0; width as usize * height as usize], view };
        });
    }

    /// Replace a window's content (all of it damaged).
    pub fn set_content(&mut self, id: WindowId, content: Content) {
        self.update(id, |w| w.content = content);
    }

    /// Show a window's content through `view` (buffer transform, viewport).
    pub fn set_view(&mut self, id: WindowId, view: View) {
        if self.window(id).is_some_and(|w| w.content.view != view) {
            self.update(id, |w| w.content.view = view);
        }
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
        if !c.view.is_identity() {
            // Through a transform or a viewport: all of it.
            let e = w.surface_rect();
            self.damage_logical(e);
            return;
        }
        let logical = r.scale_down_out(c.scale as i32).translate(w.x, w.y);
        self.damage_logical(logical);
    }

    /// Keep the picture of what lies behind window `id` up to date (or
    /// stop): see [`Backdrop`]. Starting fills it in with the next frame.
    pub fn set_backdrop(&mut self, id: WindowId, keep: bool) {
        if !keep {
            self.backdrops.remove(&id);
            return;
        }
        if self.index(id).is_none() || self.backdrops.contains_key(&id) {
            return;
        }
        self.backdrops.insert(id, Backdrop::default());
        if let Some(i) = self.index(id) {
            self.damage_window(i);
        }
    }

    /// The picture behind window `id`, if it is kept.
    pub fn backdrop(&self, id: WindowId) -> Option<&Backdrop> {
        self.backdrops.get(&id)
    }

    /// The parts of window `id`'s backdrop drawn again since the last call
    /// (physical, inside [`Backdrop::rect`]): what a blur must redo.
    pub fn take_backdrop_damage(&mut self, id: WindowId) -> Vec<Rect> {
        self.backdrops.get_mut(&id).map(|b| core::mem::take(&mut b.damage)).unwrap_or_default()
    }

    /// The frame's outer rectangle of a decorated window, logical.
    pub fn frame_rect(&self, id: WindowId) -> Option<Rect> {
        self.window(id)?.frame().map(|f| f.outer)
    }

    /// What is at the logical point (`x`, `y`), topmost first.
    pub fn hit_test(&self, x: i32, y: i32) -> Hit {
        self.hit_test_where(x, y, |_, _, _| true)
    }

    /// [`Scene::hit_test`] over the windows `accept` takes (the server
    /// skips surfaces whose input region leaves the point out).
    pub fn hit_test_where(&self, x: i32, y: i32, accept: impl Fn(&Window, i32, i32) -> bool) -> Hit {
        for w in self.windows.iter().rev().filter(|w| w.is_visible()) {
            // Framed content is drawn only inside its bounds.
            let surface = if w.decorated { w.bounds() } else { w.surface_rect() };
            if let Some(f) = w.frame() {
                if f.close.contains(x, y) {
                    return Hit::Close(w.id);
                }
                if f.title.contains(x, y) {
                    return Hit::TitleBar(w.id);
                }
            }
            if surface.contains(x, y) && accept(w, x - w.x, y - w.y) {
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
        let damage = self.draw_damage(fb, below, above, &[]);
        if !damage.is_empty() {
            fb.present(&damage);
        }
        damage
    }

    /// Draw the damage, plus `extra` (physical rectangles a framebuffer
    /// with several buffers must bring up to date), into `fb` without
    /// presenting it; returns what was drawn, `extra` included.
    pub fn draw_damage(
        &mut self,
        fb: &mut dyn Framebuffer,
        below: Option<&dyn super::Layer>,
        above: Option<&dyn super::Layer>,
        extra: &[Rect],
    ) -> Vec<Rect> {
        if (fb.width(), fb.height()) != (self.width, self.height) {
            self.damage_all();
        }
        let screen = Rect::new(0, 0, fb.width().min(self.width) as i32, fb.height().min(self.height) as i32);
        let mut all = core::mem::take(&mut self.damage);
        for r in extra {
            add_damage(&mut all, *r);
        }
        let damage: Vec<Rect> = all.iter().map(|d| d.intersect(&screen)).filter(|d| !d.is_empty()).collect();
        let mut backdrops = core::mem::take(&mut self.backdrops);
        let s = self.scale as i32;
        for (id, b) in backdrops.iter_mut() {
            let extent = self.window(*id).map_or(Rect::default(), |w| w.extent().scale(s).intersect(&screen));
            b.follow(extent);
        }
        for r in &damage {
            super::render::draw(self, fb, *r, below, above, &mut backdrops);
        }
        self.backdrops = backdrops;
        damage
    }
}
