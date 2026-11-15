//! Showing menus: `TrackPopupMenu(Ex)` and `EndMenu`, the loop that runs a menu bar
//! and its popups (the messages its sheets record: `TrackPopupMenu.yaml`,
//! `TrackPopupMenuEx.yaml`), and the drawing of both in Poly's desktop look (Alpine
//! Shell: dark surfaces, white text, 10 % white for what is chosen).
//!
//! A menu bar is part of its window's surface, above the client area (`WinInfo::menu_bar`,
//! `surfaces.rs`); a popup is a window of its own (class `#32768`, owned by the window
//! the menu belongs to, never activated). The layout is `poly_win32k::menu`'s. Where
//! the session has no desktop (session 0) no window is made: the loop only runs the
//! messages, which is what the tests see.

use super::menu::{self, h32, items_of, with};
use super::{H, SYS, call, info};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use poly_win32_gui::{SpinLock, set_last_error};
use poly_win32k::frame::Rect;
use poly_win32k::menu::{self as mn, Cell, Item, Metrics};
use poly_win32k::metrics as wm_metrics;
use poly_win32k::proto::{ClassSpec, Request};
use poly_win32k::user::{self as u, CMsg};

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

pub const TPM_CENTERALIGN: u32 = 0x0004;
pub const TPM_RIGHTALIGN: u32 = 0x0008;
pub const TPM_VCENTERALIGN: u32 = 0x0010;
pub const TPM_BOTTOMALIGN: u32 = 0x0020;
pub const TPM_NONOTIFY: u32 = 0x0080;
pub const TPM_RETURNCMD: u32 = 0x0100;

const WM_QUIT: u32 = 0x0012;
const WM_COMMAND: u32 = 0x0111;
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_INITMENU: u32 = 0x0116;
const WM_INITMENUPOPUP: u32 = 0x0117;
const WM_MENUSELECT: u32 = 0x011F;
const WM_UNINITMENUPOPUP: u32 = 0x0125;
const WM_ENTERMENULOOP: u32 = 0x0211;
const WM_EXITMENULOOP: u32 = 0x0212;
const WM_PAINT: u32 = 0x000F;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MOUSEMOVE: u32 = 0x0200;

const VK_BACK: u32 = 0x08;
const VK_RETURN: u32 = 0x0D;
const VK_MENU: u32 = 0x12;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_F10: u32 = 0x79;

const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const SW_SHOWNOACTIVATE: i32 = 4;

/// Set by `EndMenu`: the running menu loop ends.
static ENDED: AtomicBool = AtomicBool::new(false);
/// How many menu loops run (this process).
static LOOPS: AtomicU32 = AtomicU32::new(0);

/// `EndMenu`: ends the menu loop the calling thread runs (a menu loop is modal, so this
/// is called from a window procedure the loop dispatches to).
#[unsafe(no_mangle)]
pub extern "system" fn EndMenu() -> i32 {
    if LOOPS.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    ENDED.store(true, Ordering::Relaxed);
    1
}

fn send(hwnd: H, msg: u32, w: u64, l: u64) -> u64 {
    u::send(&SYS, hwnd as u64, msg, w, l)
}

/// `MAKEWPARAM(lo, hi)`.
fn mk(lo: u32, hi: u32) -> u64 {
    u64::from(lo & 0xFFFF) | (u64::from(hi & 0xFFFF) << 16)
}

fn desktop_session() -> bool {
    u::screen(&SYS).interactive
}

// ----- drawing: Alpine Shell's colours (`poly-shell`'s theme) over the dark surface

const BAR_BG: u32 = rgb(0x20, 0x20, 0x20);
const POPUP_BG: u32 = rgb(0x24, 0x24, 0x24);
const BORDER: u32 = rgb(0x3C, 0x3C, 0x3C);
/// White at 10 % and at 5 % over the surface.
const SELECTED: u32 = rgb(0x42, 0x42, 0x42);
const TEXT: u32 = rgb(0xF2, 0xF2, 0xF2);
const TEXT_DIM: u32 = rgb(0x76, 0x76, 0x76);
const ACCEL: u32 = rgb(0x9A, 0x9A, 0x9A);
const SEPARATOR: u32 = rgb(0x44, 0x44, 0x44);

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontW(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> usize;
    fn SelectObject(hdc: usize, obj: usize) -> usize;
    fn SetTextColor(hdc: usize, color: u32) -> u32;
    fn SetBkMode(hdc: usize, mode: i32) -> i32;
    fn GetTextExtentPoint32W(hdc: usize, text: *const u16, count: i32, size: *mut i32) -> i32;
    fn PolyGdiCreateDisplayDC(hwnd: usize, persistent: i32) -> usize;
    fn PolyGdiFillRect(hdc: usize, rect: *const i32, brush: usize, color: u32) -> i32;
    fn PolyGdiDrawText(hdc: usize, text: *const u16, count: i32, rect: *mut i32, format: u32) -> i32;
    fn PolyGdiAttachSurfaceRows(hdc: usize, width: u32, total: u32, skip: u32, rows: u32) -> i32;
    fn PolyGdiSetDisplayGeometry(
        hdc: usize,
        x: i32,
        y: i32,
        include: *const i32,
        n_include: u32,
        exclude: *const i32,
        n_exclude: u32,
    ) -> i32;
}

/// The menu font (Segoe UI, 9 points: the font Windows menus use), made once.
fn font() -> usize {
    static FONT: AtomicUsize = AtomicUsize::new(0);
    let f = FONT.load(Ordering::Relaxed);
    if f != 0 {
        return f;
    }
    let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    // SAFETY: our gdi32; the face is NUL-terminated.
    let f = unsafe { CreateFontW(-12, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) };
    FONT.store(f, Ordering::Relaxed);
    f
}

/// Tools for drawing on one DC: fill, text, text width.
struct Pen {
    hdc: usize,
}

impl Pen {
    fn new(hdc: usize) -> Pen {
        // SAFETY: our gdi32 checks the DC.
        unsafe {
            SelectObject(hdc, font());
            SetBkMode(hdc, 1);
        }
        Pen { hdc }
    }

    fn fill(&self, l: i32, t: i32, r: i32, b: i32, color: u32) {
        if r <= l || b <= t {
            return;
        }
        let rect = [l, t, r, b];
        // SAFETY: our gdi32 checks the DC; one RECT.
        unsafe { PolyGdiFillRect(self.hdc, rect.as_ptr(), 0, color) };
    }

    fn width(&self, text: &[u16]) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let mut size = [0i32; 2];
        // SAFETY: our gdi32; `text.len()` units and a SIZE.
        unsafe { GetTextExtentPoint32W(self.hdc, text.as_ptr(), text.len() as i32, size.as_mut_ptr()) };
        size[0]
    }

    /// `text` in the cell `l..r` x `t..b`: left (or right) aligned and centred vertically.
    fn text(&self, text: &[u16], l: i32, t: i32, r: i32, b: i32, color: u32, right: bool) {
        if text.is_empty() {
            return;
        }
        let mut rect = [l, t, r, b];
        // DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_NOCLIP, DT_LEFT or DT_RIGHT.
        let format = 0x20 | 0x04 | 0x800 | 0x100 | if right { 2 } else { 0 };
        // SAFETY: our gdi32 checks the DC; `text.len()` units and one RECT.
        unsafe {
            SetTextColor(self.hdc, color);
            PolyGdiDrawText(self.hdc, text.as_ptr(), text.len() as i32, rect.as_mut_ptr(), format);
        }
    }

    /// A label with its mnemonic underlined.
    fn label(&self, raw: &[u16], l: i32, t: i32, r: i32, b: i32, color: u32) {
        let text = mn::strip_amp(raw);
        self.text(&text, l, t, r, b, color, false);
        if let Some(at) = mnemonic_index(raw) {
            let x0 = l + self.width(&text[..at]);
            let x1 = x0 + self.width(&text[at..=at]);
            let y = (t + b) / 2 + 5;
            self.fill(x0, y.min(b - 1), x1, y.min(b - 1) + 1, color);
        }
    }
}

/// The index, in the text without its `&`s, of the mnemonic character.
fn mnemonic_index(raw: &[u16]) -> Option<usize> {
    let amp = u16::from(b'&');
    let (mut i, mut out) = (0, 0);
    while i < raw.len() {
        if raw[i] == amp {
            match raw.get(i + 1) {
                Some(&c) if c == amp => {
                    i += 2;
                    out += 1;
                }
                Some(_) => return Some(out),
                None => return None,
            }
        } else {
            i += 1;
            out += 1;
        }
    }
    None
}

/// A DC measuring text in the menu font (nothing is drawn with it).
fn with_measure<R>(f: impl FnOnce(&Pen) -> R) -> R {
    // SAFETY: our gdi32.
    let dc = unsafe { PolyGdiCreateDisplayDC(0, 0) };
    let r = f(&Pen::new(dc));
    // SAFETY: our gdi32 checks the handle.
    unsafe { super::PolyGdiDeleteDisplayDC(dc) };
    r
}

const METRICS: Metrics = Metrics::POLY;
const BAR_PAD: i32 = 8;

// ----- the menu bar

/// The bar of a window: its items, their cells (relative to the bar's top-left corner),
/// the screen position of that corner and the width.
struct BarView {
    menu: u32,
    items: Vec<Item>,
    cells: Vec<Cell>,
    origin: (i32, i32),
    width: i32,
    height: i32,
    hwnd: H,
    total: u32,
}

fn bar_view(hwnd: H) -> Option<BarView> {
    let i = info(hwnd)?;
    if i.style & 0x4000_0000 != 0 || i.id == 0 || i.menu_bar <= 0 || i.id >> 32 != 0 {
        return None;
    }
    let menu = i.id as u32;
    let items = items_of(menu);
    let width = i.client.width();
    let (cells, _rows) = with_measure(|p| mn::bar_layout(&items, i.menu_bar - 2, BAR_PAD, width, &|t| p.width(t)));
    Some(BarView {
        menu,
        items,
        cells,
        origin: (i.client.left, i.client.top - i.menu_bar),
        width,
        height: i.menu_bar,
        hwnd,
        total: (i.client.height() + i.menu_bar) as u32,
    })
}

/// Draw the window's menu bar into the top rows of its surface; `hot` is the item drawn chosen.
fn paint_bar(hwnd: H, hot: Option<usize>, open: bool) {
    if !desktop_session() {
        return;
    }
    let Some(v) = bar_view(hwnd) else { return };
    // SAFETY: our gdi32; one RECT.
    let dc = unsafe { PolyGdiCreateDisplayDC(hwnd, 0) };
    if dc == 0 {
        return;
    }
    let screen_rect = [v.origin.0, v.origin.1, v.origin.0 + v.width, v.origin.1 + v.height];
    // SAFETY: our gdi32 checks the DC; one RECT.
    unsafe {
        PolyGdiSetDisplayGeometry(dc, v.origin.0, v.origin.1, screen_rect.as_ptr(), 1, core::ptr::null(), 0);
        PolyGdiAttachSurfaceRows(dc, v.width as u32, v.total, 0, v.height as u32);
    }
    let p = Pen::new(dc);
    p.fill(0, 0, v.width, v.height, BAR_BG);
    for (n, (it, c)) in v.items.iter().zip(&v.cells).enumerate() {
        if it.is_separator() {
            continue;
        }
        let chosen = hot == Some(n);
        if chosen {
            p.fill(c.left, c.top + 1, c.right, c.bottom + 1, if open { SELECTED } else { SELECTED });
        }
        let color = if it.state & mn::MFS_GRAYED != 0 { TEXT_DIM } else { TEXT };
        p.label(it.string(), c.left + BAR_PAD, c.top + 1, c.right - BAR_PAD, c.bottom + 1, color);
    }
    // SAFETY: our gdi32 checks the handle.
    unsafe { super::ReleaseDC(hwnd, dc) };
}

/// Draw the menu bar of window `hwnd` (a window that has none draws nothing).
pub(crate) fn draw_bar(hwnd: H) {
    paint_bar(hwnd, None, false);
}

/// The bar item that screen point (`x`, `y`) is on.
fn bar_hit(v: &BarView, x: i32, y: i32) -> Option<usize> {
    let (lx, ly) = (x - v.origin.0, y - v.origin.1);
    if lx < 0 || ly < 0 || ly >= v.height || lx >= v.width {
        return None;
    }
    v.cells.iter().zip(&v.items).position(|(c, it)| !it.is_separator() && lx >= c.left && lx < c.right)
}

// ----- popups

const CLASS: &str = "#32768\0";

struct PopupDraw {
    hwnd: H,
    items: Vec<Item>,
    cells: Vec<Cell>,
    width: i32,
    height: i32,
    hot: Option<usize>,
}

static POPUPS: SpinLock<Vec<PopupDraw>> = SpinLock::new(Vec::new());

/// The window procedure of popup menu windows: they paint themselves from their record.
unsafe extern "system" fn popup_proc(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    if msg == WM_MOUSEACTIVATE {
        return 3; // MA_NOACTIVATE
    }
    let r = u::def_window_proc(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize;
    if msg == WM_PAINT {
        draw_popup(hwnd);
    }
    r
}

fn register_class() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let spec = ClassSpec {
        name: CLASS.encode_utf16().take(CLASS.len() - 1).collect(),
        style: 0x0008, // CS_DBLCLKS
        wndproc: popup_proc as usize as u64,
        cls_extra: 0,
        wnd_extra: 0,
        instance: 0,
        icon: 0,
        cursor: 0,
        brush: 0,
        menu_name: 0,
        icon_sm: 0,
        unicode: true,
    };
    super::register(spec);
}

fn draw_popup(hwnd: H) {
    let Some(rec) = POPUPS.with(|p| {
        p.iter().find(|r| r.hwnd == hwnd).map(|r| (r.items.clone(), r.cells.clone(), r.width, r.height, r.hot))
    }) else {
        return;
    };
    let (items, cells, w, h, hot) = rec;
    let dc = super::GetDC(hwnd);
    if dc == 0 {
        return;
    }
    let p = Pen::new(dc);
    p.fill(0, 0, w, h, BORDER);
    p.fill(1, 1, w - 1, h - 1, POPUP_BG);
    for (n, (it, c)) in items.iter().zip(&cells).enumerate() {
        if it.is_separator() {
            let y = c.top + METRICS.separator / 2;
            p.fill(c.left + 8, y, c.right - 8, y + 1, SEPARATOR);
            continue;
        }
        if hot == Some(n) {
            p.fill(c.left + 3, c.top, c.right - 3, c.bottom, SELECTED);
        }
        let gray = it.state & mn::MFS_GRAYED != 0;
        let color = if gray { TEXT_DIM } else { TEXT };
        let (label, accel) = mn::split_tab(it.string());
        let mid = (c.top + c.bottom) / 2;
        if it.state & mn::MFS_CHECKED != 0 {
            draw_check(&p, c.left + METRICS.pad_x - 2, mid, it.ftype & mn::MFT_RADIOCHECK != 0, color);
        }
        let lx = c.left + METRICS.pad_x + METRICS.check;
        p.label(label, lx, c.top, c.right - METRICS.pad_x, c.bottom, color);
        if it.sub != 0 {
            draw_arrow(&p, c.right - METRICS.pad_x - 6, mid, color);
        } else if !accel.is_empty() {
            p.text(accel, c.left, c.top, c.right - METRICS.pad_x, c.bottom, if gray { TEXT_DIM } else { ACCEL }, true);
        }
    }
    // SAFETY: the window's own DC.
    unsafe { super::ReleaseDC(hwnd, dc) };
}

/// A check mark (or, for a radio item, a dot) centred at x + 6, `mid`.
fn draw_check(p: &Pen, x: i32, mid: i32, radio: bool, color: u32) {
    if radio {
        p.fill(x + 3, mid - 2, x + 9, mid + 3, color);
        p.fill(x + 4, mid - 3, x + 8, mid + 4, color);
        return;
    }
    // Two strokes, 2 pixels thick.
    for k in 0..3 {
        p.fill(x + 2 + k, mid + k - 1, x + 4 + k, mid + k + 1, color);
    }
    for k in 0..6 {
        p.fill(x + 5 + k, mid + 1 - k, x + 7 + k, mid + 3 - k, color);
    }
}

/// A right-pointing triangle with its tip at (x + 5, mid).
fn draw_arrow(p: &Pen, x: i32, mid: i32, color: u32) {
    for k in 0..4 {
        p.fill(x + k, mid - 4 + k, x + k + 1, mid + 5 - k, color);
    }
}

/// One level of open menu: a popup window.
struct Level {
    menu: u32,
    hwnd: H,
    /// Screen rectangle.
    rect: Rect,
    items: Vec<Item>,
    cells: Vec<Cell>,
    hot: Option<usize>,
    system: bool,
}

// ----- the loop

struct Tracker {
    owner: H,
    notify: bool,
    /// `TrackPopupMenu` (the first menu is a popup) or a window's bar.
    popup_root: bool,
    levels: Vec<Level>,
    bar: Option<BarState>,
    /// The mouse was over an item (a release only chooses then).
    armed: bool,
    chosen: Option<(u32, bool)>,
    done: bool,
    desktop: bool,
}

struct BarState {
    view: BarView,
    hot: Option<usize>,
}

impl Tracker {
    fn sys_bit(&self, menu: u32) -> u32 {
        u32::from(menu::system_menu_of(self.owner as u32) == menu) << 16
    }

    /// Open `menu` as a popup at screen point (`x`, `y`); `pos` is the item of the menu
    /// above that opens it.
    fn open_level(&mut self, menu: u32, x: i32, y: i32, pos: u32, align: u32) -> bool {
        let system = menu::system_menu_of(self.owner as u32) == menu;
        if self.notify {
            send(self.owner, WM_INITMENUPOPUP, u64::from(menu), u64::from(pos & 0xFFFF) | u64::from(system) << 16);
        }
        let items = items_of(menu);
        if items.is_empty() {
            return false;
        }
        let (w, h, cells) = if self.desktop {
            with_measure(|p| mn::popup_layout(&items, &METRICS, &|t| p.width(t)))
        } else {
            (160, 24 * items.len() as i32, Vec::new())
        };
        let screen = u::screen(&SYS);
        let mut left = x;
        let mut top = y;
        if align & TPM_CENTERALIGN != 0 {
            left -= w / 2;
        } else if align & TPM_RIGHTALIGN != 0 {
            left -= w;
        }
        if align & TPM_VCENTERALIGN != 0 {
            top -= h / 2;
        } else if align & TPM_BOTTOMALIGN != 0 {
            top -= h;
        }
        if left + w > screen.width {
            left = (screen.width - w).max(0);
        }
        if top + h > screen.height {
            top = (screen.height - h).max(0);
        }
        left = left.max(0);
        top = top.max(0);
        let rect = Rect::new(left, top, left + w, top + h);
        let mut hwnd = 0;
        if self.desktop {
            register_class();
            let class: Vec<u16> = CLASS.encode_utf16().collect();
            // SAFETY: a registered class and a live owner window.
            hwnd = unsafe {
                super::CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    core::ptr::null(),
                    WS_POPUP,
                    left,
                    top,
                    w,
                    h,
                    self.owner,
                    0,
                    0,
                    0,
                )
            };
            if hwnd != 0 {
                POPUPS.with(|p| {
                    p.push(PopupDraw {
                        hwnd,
                        items: items.clone(),
                        cells: cells.clone(),
                        width: w,
                        height: h,
                        hot: None,
                    })
                });
                super::ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                draw_popup(hwnd);
            }
        }
        self.levels.push(Level { menu, hwnd, rect, items, cells, hot: None, system });
        true
    }

    /// Close the levels above `depth` (0: all).
    fn close_to(&mut self, depth: usize) {
        while self.levels.len() > depth {
            let lv = self.levels.pop().expect("open level");
            if self.notify {
                send(self.owner, WM_UNINITMENUPOPUP, u64::from(lv.menu), u64::from(lv.system) << 16);
            }
            if lv.hwnd != 0 {
                POPUPS.with(|p| p.retain(|r| r.hwnd != lv.hwnd));
                super::DestroyWindow(lv.hwnd);
            }
        }
    }

    fn redraw(&self, depth: usize) {
        let Some(lv) = self.levels.get(depth) else { return };
        if lv.hwnd != 0 {
            POPUPS.with(|p| {
                if let Some(r) = p.iter_mut().find(|r| r.hwnd == lv.hwnd) {
                    r.hot = lv.hot;
                }
            });
            draw_popup(lv.hwnd);
        }
    }

    fn select_msg(&self, menu: u32, item: Option<&Item>, pos: usize) {
        if !self.notify {
            return;
        }
        match item {
            Some(it) => {
                let mut flags = it.menu_state(0) & !0xFF00;
                if it.sub != 0 {
                    flags |= mn::MF_POPUP;
                }
                flags |= mn::MF_HILITE;
                let id = if it.sub != 0 { pos as u32 } else { it.id };
                send(self.owner, WM_MENUSELECT, mk(id, flags), u64::from(menu));
            }
            None => {
                send(self.owner, WM_MENUSELECT, mk(0, 0xFFFF), 0);
            }
        }
    }

    /// Make item `idx` of level `depth` the chosen one (None: none).
    fn set_hot(&mut self, depth: usize, idx: Option<usize>) {
        let changed = self.levels.get(depth).is_some_and(|l| l.hot != idx);
        if !changed {
            return;
        }
        self.close_to(depth + 1);
        let (menu, item) = {
            let lv = &mut self.levels[depth];
            lv.hot = idx;
            (lv.menu, idx.and_then(|i| lv.items.get(i).cloned()))
        };
        self.redraw(depth);
        self.select_msg(menu, item.as_ref(), idx.unwrap_or(0));
    }

    /// Open the popup of the item `idx` of level `depth` beside it.
    fn open_sub(&mut self, depth: usize, idx: usize) {
        let Some(lv) = self.levels.get(depth) else { return };
        let Some(it) = lv.items.get(idx) else { return };
        if it.sub == 0 || it.state & mn::MFS_GRAYED != 0 || self.levels.len() > depth + 1 {
            return;
        }
        let sub = it.sub;
        let cell = lv.cells.get(idx).copied().unwrap_or_default();
        let (x, y) = (lv.rect.right - 3, lv.rect.top + cell.top - 3);
        self.open_level(sub, x, y, idx as u32, 0);
    }

    /// Choose item `idx` of level `depth`: a popup item opens its popup, a command item
    /// ends the menu with its id.
    fn activate(&mut self, depth: usize, idx: usize) {
        let Some(lv) = self.levels.get(depth) else { return };
        let Some(it) = lv.items.get(idx).cloned() else { return };
        if it.is_separator() || it.state & mn::MFS_GRAYED != 0 {
            return;
        }
        if it.sub != 0 {
            self.open_sub(depth, idx);
            if self.levels.len() > depth + 1 {
                self.set_hot(depth + 1, self.first_enabled(depth + 1));
            }
            return;
        }
        self.chosen = Some((it.id, lv.system));
        self.done = true;
    }

    fn first_enabled(&self, depth: usize) -> Option<usize> {
        self.levels.get(depth)?.items.iter().position(|i| !i.is_separator())
    }

    /// The next selectable item from `from` in direction `dir` (wrapping).
    fn step(&self, depth: usize, from: Option<usize>, dir: i32) -> Option<usize> {
        let items = &self.levels.get(depth)?.items;
        let n = items.len() as i32;
        if n == 0 {
            return None;
        }
        let mut i = match from {
            Some(f) => f as i32,
            None if dir > 0 => -1,
            None => n,
        };
        for _ in 0..n {
            i = (i + dir).rem_euclid(n);
            if !items[i as usize].is_separator() {
                return Some(i as usize);
            }
        }
        None
    }

    fn level_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..self.levels.len()).rev().find(|&d| self.levels[d].rect.contains(x, y))
    }

    fn item_at(&self, depth: usize, x: i32, y: i32) -> Option<usize> {
        let lv = &self.levels[depth];
        let (lx, ly) = (x - lv.rect.left, y - lv.rect.top);
        lv.cells.iter().zip(&lv.items).position(|(c, it)| !it.is_separator() && c.contains(lx, ly))
    }

    // ----- the bar

    fn bar_hot(&mut self, idx: Option<usize>, open_popup: bool) {
        let Some(bar) = self.bar.as_ref() else { return };
        let changed = bar.hot != idx;
        if changed || !open_popup {
            self.close_to(0);
        }
        let hwnd = self.owner;
        let (menu, item) = {
            let bar = self.bar.as_mut().expect("bar");
            bar.hot = idx;
            (bar.view.menu, idx.and_then(|i| bar.view.items.get(i).cloned()))
        };
        paint_bar(hwnd, idx, open_popup);
        if changed {
            self.select_msg(menu, item.as_ref(), idx.unwrap_or(0));
        }
        if open_popup {
            self.open_bar_popup(false);
        }
    }

    /// Open the popup of the chosen bar item below it (`first`: with its first item chosen).
    fn open_bar_popup(&mut self, first: bool) {
        if !self.levels.is_empty() {
            return;
        }
        let Some(bar) = &self.bar else { return };
        let Some(idx) = bar.hot else { return };
        let Some(it) = bar.view.items.get(idx) else { return };
        if it.sub == 0 || it.state & mn::MFS_GRAYED != 0 {
            return;
        }
        let cell = bar.view.cells[idx];
        let (x, y) = (bar.view.origin.0 + cell.left, bar.view.origin.1 + bar.view.height);
        let sub = it.sub;
        if self.open_level(sub, x, y, idx as u32, 0) && first {
            let f = self.first_enabled(0);
            self.set_hot(0, f);
        }
    }

    fn bar_activate_idx(&mut self, idx: usize) {
        let Some(bar) = &self.bar else { return };
        let Some(it) = bar.view.items.get(idx).cloned() else { return };
        if it.state & mn::MFS_GRAYED != 0 {
            return;
        }
        if it.sub != 0 {
            self.bar_hot(Some(idx), true);
            self.open_bar_popup(true);
        } else {
            self.chosen = Some((it.id, false));
            self.done = true;
        }
    }

    // ----- input

    fn key(&mut self, m: &CMsg) {
        let vk = m.w as u32;
        if !matches!(m.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
            return;
        }
        match vk {
            VK_ESCAPE => {
                if self.levels.len() > 1 || (self.bar.is_some() && !self.levels.is_empty()) {
                    let keep = self.levels.len() - 1;
                    self.close_to(keep);
                    if keep == 0 {
                        self.bar_hot(self.bar.as_ref().and_then(|b| b.hot), false);
                    }
                } else {
                    self.done = true;
                }
            }
            VK_MENU | VK_F10 if self.bar.is_some() => self.done = true,
            VK_UP | VK_DOWN => {
                let dir = if vk == VK_DOWN { 1 } else { -1 };
                if self.levels.is_empty() {
                    if self.bar.is_some() {
                        self.open_bar_popup(true);
                    }
                    return;
                }
                let d = self.levels.len() - 1;
                let from = self.levels[d].hot;
                let next = self.step(d, from, dir);
                self.set_hot(d, next);
            }
            VK_HOME | VK_END if !self.levels.is_empty() => {
                let d = self.levels.len() - 1;
                let next = if vk == VK_HOME { self.step(d, None, 1) } else { self.step(d, None, -1) };
                self.set_hot(d, next);
            }
            VK_RIGHT => {
                if let Some(d) = self.levels.len().checked_sub(1) {
                    let hot = self.levels[d].hot;
                    if hot.and_then(|h| self.levels[d].items.get(h)).is_some_and(|i| i.sub != 0) {
                        self.activate(d, hot.expect("hot"));
                        return;
                    }
                }
                self.bar_move(1);
            }
            VK_LEFT => {
                if self.levels.len() > 1 {
                    let keep = self.levels.len() - 1;
                    self.close_to(keep);
                    return;
                }
                self.bar_move(-1);
            }
            VK_RETURN | VK_SPACE => {
                if let Some(d) = self.levels.len().checked_sub(1) {
                    if let Some(h) = self.levels[d].hot {
                        self.activate(d, h);
                    }
                } else if let Some(h) = self.bar.as_ref().and_then(|b| b.hot) {
                    self.bar_activate_idx(h);
                }
            }
            VK_BACK => {}
            0x30..=0x39 | 0x41..=0x5A => self.mnemonic(vk as u16),
            _ => {}
        }
    }

    fn bar_move(&mut self, dir: i32) {
        let Some(bar) = &self.bar else { return };
        let n = bar.view.items.len() as i32;
        if n == 0 {
            return;
        }
        let was_open = !self.levels.is_empty();
        let mut i = bar.hot.map_or(if dir > 0 { -1 } else { n }, |h| h as i32);
        for _ in 0..n {
            i = (i + dir).rem_euclid(n);
            if !bar.view.items[i as usize].is_separator() {
                break;
            }
        }
        self.bar_hot(Some(i as usize), was_open);
    }

    fn mnemonic(&mut self, ch: u16) {
        let upper = |c: u16| if (0x61..=0x7A).contains(&c) { c - 0x20 } else { c };
        if let Some(d) = self.levels.len().checked_sub(1) {
            let hits: Vec<usize> = self.levels[d]
                .items
                .iter()
                .enumerate()
                .filter(|(_, it)| mn::mnemonic(it.string()).map(upper) == Some(upper(ch)))
                .map(|(i, _)| i)
                .collect();
            match hits.as_slice() {
                [] => {}
                [one] => self.activate(d, *one),
                many => {
                    let cur = self.levels[d].hot;
                    let next = many.iter().copied().find(|&i| Some(i) > cur).unwrap_or(many[0]);
                    self.set_hot(d, Some(next));
                }
            }
        } else if let Some(bar) = &self.bar {
            let hit = bar
                .view
                .items
                .iter()
                .position(|it| !it.is_separator() && mn::mnemonic(it.string()).map(upper) == Some(upper(ch)));
            if let Some(h) = hit {
                self.bar_activate_idx(h);
            }
        }
    }

    fn mouse(&mut self, m: &CMsg) {
        let (x, y) = (m.pt.x, m.pt.y);
        let down = matches!(m.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN);
        let up = matches!(m.message, WM_LBUTTONUP | WM_RBUTTONUP);
        if let Some(d) = self.level_at(x, y) {
            let item = self.item_at(d, x, y);
            if m.message == WM_MOUSEMOVE || down {
                if item.is_some() {
                    self.armed = true;
                }
                self.set_hot(d, item);
                if let Some(i) = item
                    && self.levels.get(d).and_then(|l| l.items.get(i)).is_some_and(|it| it.sub != 0)
                    && self.levels.len() == d + 1
                {
                    self.open_sub(d, i);
                }
                if down && let Some(i) = item {
                    self.activate_down(d, i);
                }
            } else if up
                && self.armed
                && let Some(i) = item
            {
                self.activate(d, i);
            }
            return;
        }
        // On the bar?
        if let Some(bar) = &self.bar
            && let Some(i) = bar_hit(&bar.view, x, y)
        {
            let open = !self.levels.is_empty();
            if m.message == WM_MOUSEMOVE {
                if bar.hot != Some(i) {
                    self.bar_hot(Some(i), open);
                }
            } else if down {
                if bar.hot == Some(i) && open {
                    // A second click on the open item closes the menu.
                    self.done = true;
                } else {
                    self.armed = true;
                    self.bar_hot(Some(i), true);
                }
            }
            return;
        }
        if down || (up && self.armed && self.bar.is_none()) {
            // A click outside every menu closes them.
            self.done = true;
        }
    }

    fn activate_down(&mut self, d: usize, i: usize) {
        if self.levels.get(d).and_then(|l| l.items.get(i)).is_some_and(|it| it.sub != 0) {
            self.open_sub(d, i);
        }
    }

    fn handle(&mut self, m: &CMsg) -> bool {
        match m.message {
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
                self.key(m);
                true
            }
            WM_CHAR | WM_SYSCHAR => true,
            0x0200..=0x020E | 0x00A0..=0x00A9 => {
                if self.desktop {
                    self.mouse(m);
                }
                true
            }
            _ => false,
        }
    }
}

/// Run messages until the menu ends.
fn run_loop(t: &mut Tracker) {
    while !ENDED.load(Ordering::Relaxed) && !t.done {
        let mut m = CMsg::default();
        match u::get_message(&SYS, &mut m, 0, 0, 0) {
            0 => {
                // WM_QUIT ends the loop and is passed on.
                super::PostQuitMessage(m.w as i32);
                break;
            }
            n if n < 0 => break,
            _ => {}
        }
        if m.message == WM_QUIT {
            break;
        }
        if t.handle(&m) {
            continue;
        }
        u::translate_message(&SYS, &m);
        u::dispatch_message(&SYS, &m);
    }
}

fn finish(t: &mut Tracker, hwnd: H) {
    t.close_to(0);
    if t.desktop {
        super::ReleaseCapture();
    }
    LOOPS.fetch_sub(1, Ordering::Relaxed);
    ENDED.store(false, Ordering::Relaxed);
    if t.bar.take().is_some() {
        paint_bar(hwnd, None, false);
    }
    send(hwnd, WM_MENUSELECT, mk(0, 0xFFFF), 0);
    if t.notify {
        send(hwnd, WM_EXITMENULOOP, u64::from(t.popup_root), 0);
    }
}

/// Deliver the command a finished menu chose: returned (`TPM_RETURNCMD`) or posted.
fn deliver(hwnd: H, chosen: Option<(u32, bool)>, flags: u32) -> i32 {
    let id = chosen.map_or(0, |c| c.0);
    if flags & TPM_RETURNCMD != 0 {
        return id as i32;
    }
    if let Some((id, system)) = chosen {
        if system {
            super::PostMessageW(hwnd, WM_SYSCOMMAND, id as usize, 0);
        } else {
            super::PostMessageW(hwnd, WM_COMMAND, id as usize, 0);
        }
    }
    1
}

fn track(menu: H, flags: u32, x: i32, y: i32, hwnd: H) -> i32 {
    let h = h32(menu);
    let kind = with(|m| m.get(h).map(|x| (x.popup, x.items.len())));
    let Some((popup, count)) = kind else {
        set_last_error(menu::ERROR_INVALID_MENU_HANDLE);
        return 0;
    };
    if info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    if !popup {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    if count == 0 {
        return 0;
    }
    let notify = flags & TPM_NONOTIFY == 0;
    ENDED.store(false, Ordering::Relaxed);
    LOOPS.fetch_add(1, Ordering::Relaxed);
    let desktop = desktop_session();
    let mut t = Tracker {
        owner: hwnd,
        notify,
        popup_root: true,
        levels: Vec::new(),
        bar: None,
        armed: false,
        chosen: None,
        done: false,
        desktop,
    };
    if notify {
        send(hwnd, WM_ENTERMENULOOP, 1, 0);
        send(hwnd, WM_INITMENU, u64::from(h), 0);
    }
    if desktop {
        super::SetCapture(hwnd);
    }
    t.open_level(h, x, y, 0, flags);
    run_loop(&mut t);
    let chosen = t.chosen;
    finish(&mut t, hwnd);
    deliver(hwnd, chosen, flags)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TrackPopupMenu(
    menu: H,
    flags: u32,
    x: i32,
    y: i32,
    _reserved: i32,
    hwnd: H,
    _rect: *const i32,
) -> i32 {
    track(menu, flags, x, y, hwnd)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TrackPopupMenuEx(menu: H, flags: u32, x: i32, y: i32, hwnd: H, tpm: *const u8) -> i32 {
    if !tpm.is_null() {
        // SAFETY: a TPMPARAMS (cbSize 4 bytes, then the exclusion RECT: 20 bytes).
        let size = unsafe { (tpm as *const u32).read_unaligned() };
        if size != 20 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    }
    track(menu, flags, x, y, hwnd)
}

// ----- the window's own bar and system menu

/// Where a bar loop starts.
enum Start {
    /// A click on the bar at a screen point.
    Click(i32, i32),
    /// The first item (Alt or F10).
    First,
    /// The item with this mnemonic.
    Char(u16),
    /// The system menu below the window's top-left corner.
    System,
}

fn bar_loop(hwnd: H, start: Start) -> bool {
    if !desktop_session() || LOOPS.load(Ordering::Relaxed) != 0 {
        return false;
    }
    let sys = match start {
        Start::System => {
            let m = menu::GetSystemMenu(hwnd, 0);
            if m == 0 {
                return false;
            }
            Some(m as u32)
        }
        _ => None,
    };
    let view = bar_view(hwnd);
    if view.is_none() && sys.is_none() {
        return false;
    }
    ENDED.store(false, Ordering::Relaxed);
    LOOPS.fetch_add(1, Ordering::Relaxed);
    let mut t = Tracker {
        owner: hwnd,
        notify: true,
        popup_root: false,
        levels: Vec::new(),
        bar: view.map(|view| BarState { view, hot: None }),
        armed: false,
        chosen: None,
        done: false,
        desktop: true,
    };
    let bar_menu = t.bar.as_ref().map(|b| b.view.menu);
    send(hwnd, WM_ENTERMENULOOP, 0, 0);
    send(hwnd, WM_INITMENU, u64::from(sys.or(bar_menu).unwrap_or(0)), 0);
    super::SetCapture(hwnd);
    match start {
        Start::Click(x, y) => {
            let hit = t.bar.as_ref().and_then(|b| bar_hit(&b.view, x, y));
            if let Some(i) = hit {
                t.armed = true;
                t.bar_hot(Some(i), true);
            }
        }
        Start::First => {
            let first = t.bar.as_ref().and_then(|b| b.view.items.iter().position(|i| !i.is_separator()));
            t.bar_hot(first, false);
        }
        Start::Char(c) => t.mnemonic(c),
        Start::System => {
            if let (Some(m), Some(i)) = (sys, info(hwnd)) {
                t.open_level(m, i.client.left, i.client.top - i.menu_bar, 0, 0);
            }
        }
    }
    run_loop(&mut t);
    let chosen = t.chosen;
    finish(&mut t, hwnd);
    deliver(hwnd, chosen, 0);
    true
}

/// `WM_NCLBUTTONDOWN` on the bar (`HTMENU`).
pub(crate) fn nc_click(hwnd: H, x: i32, y: i32) {
    bar_loop(hwnd, Start::Click(x, y));
}

static ALT_ALONE: AtomicBool = AtomicBool::new(false);

/// What `DefWindowProc` does with `WM_SYSKEYDOWN`, `WM_SYSKEYUP` and `WM_SYSCHAR`: Alt
/// alone or F10 enters the bar, Alt and a letter opens the item with that mnemonic, Alt+Space
/// the system menu. True when the message was used.
pub(crate) fn sys_key(hwnd: H, msg: u32, w: u64, _l: u64) -> bool {
    let vk = w as u32;
    match msg {
        WM_SYSKEYDOWN => {
            if vk == VK_MENU {
                ALT_ALONE.store(true, Ordering::Relaxed);
                return false;
            }
            ALT_ALONE.store(false, Ordering::Relaxed);
            if vk == VK_F10 {
                return bar_loop(hwnd, Start::First);
            }
            if vk == VK_SPACE {
                return bar_loop(hwnd, Start::System);
            }
            false
        }
        WM_SYSKEYUP => {
            if vk == VK_MENU && ALT_ALONE.swap(false, Ordering::Relaxed) {
                return bar_loop(hwnd, Start::First);
            }
            false
        }
        WM_SYSCHAR => {
            let c = w as u16;
            if c != u16::from(b' ') && (c as u8).is_ascii_alphanumeric() && c < 0x80 {
                return bar_loop(hwnd, Start::Char(c));
            }
            false
        }
        _ => false,
    }
}

/// What `DefWindowProc` answers `WM_NCHITTEST` for the menu bar: `HTMENU` over it.
pub(crate) fn hit_menu(hwnd: H, x: i32, y: i32) -> bool {
    let Some(i) = info(hwnd) else { return false };
    i.menu_bar > 0
        && i.style & 0x4000_0000 == 0
        && x >= i.client.left
        && x < i.client.right
        && y >= i.client.top - i.menu_bar
        && y < i.client.top
}

/// The bar's height for a window that shows one now (what the client area gives up).
pub(crate) fn bar_height() -> i32 {
    wm_metrics::menu_for_dpi(96)
}

/// Tell win32k the height of the bar the window draws (0: none), so its surface has room.
pub(crate) fn set_bar_height(hwnd: H, height: i32) {
    let Some(i) = info(hwnd) else { return };
    if i.menu_bar != height {
        call(Request::MenuBar { hwnd: i.hwnd, height });
    }
}
