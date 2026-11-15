//! How the system controls (`ctl.rs`) look: flat, with the system colours, the
//! way Alpine Shell draws (ADR 0124: neutral, thin lines, no 3-D bevels). Each
//! function paints the client area of a control inside its `WM_PAINT`.

use super::ctl::{self, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON, BS_GROUPBOX, SS_ICON};
use super::{H, info};
use alloc::vec::Vec;
use poly_win32k::user as u;

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SelectObject(hdc: usize, obj: usize) -> usize;
    fn GetStockObject(index: i32) -> usize;
    fn SetTextColor(hdc: usize, color: u32) -> u32;
    fn SetBkMode(hdc: usize, mode: i32) -> i32;
    fn GetTextExtentPoint32W(hdc: usize, text: *const u16, count: i32, size: *mut i32) -> i32;
    fn PolyGdiFillRect(hdc: usize, rect: *const i32, brush: usize, color: u32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn GlobalLock(mem: *mut u8) -> *mut u8;
    fn GlobalUnlock(mem: *mut u8) -> i32;
    fn GlobalSize(mem: *mut u8) -> usize;
}

const COLOR_WINDOW: i32 = 5;
const COLOR_WINDOWTEXT: i32 = 8;
const COLOR_HIGHLIGHT: i32 = 13;
const COLOR_HIGHLIGHTTEXT: i32 = 14;
const COLOR_BTNFACE: i32 = 15;
const COLOR_GRAYTEXT: i32 = 17;
const COLOR_BTNTEXT: i32 = 18;
const COLOR_3DSHADOW: i32 = 16;
const COLOR_3DLIGHT: i32 = 22;
const DEFAULT_GUI_FONT: i32 = 17;
const TRANSPARENT: i32 = 1;

const DT_LEFT: u32 = 0;
const DT_CENTER: u32 = 1;
const DT_RIGHT: u32 = 2;
const DT_VCENTER: u32 = 4;
const DT_WORDBREAK: u32 = 0x10;
const DT_SINGLELINE: u32 = 0x20;
const DT_NOPREFIX: u32 = 0x800;
const DT_END_ELLIPSIS: u32 = 0x8000;

fn sys(i: i32) -> u32 {
    super::display::GetSysColor(i)
}

fn rgb(c: u32, f: u32) -> u32 {
    // Blend `c` towards white by f/256.
    let ch = |v: u32| v + ((255 - v) * f) / 256;
    ch(c & 0xFF) | (ch((c >> 8) & 0xFF) << 8) | (ch((c >> 16) & 0xFF) << 16)
}

fn fillc(hdc: usize, l: i32, t: i32, r: i32, b: i32, color: u32) {
    if r <= l || b <= t {
        return;
    }
    let rc = [l, t, r, b];
    // SAFETY: our gdi32 checks the DC.
    unsafe { PolyGdiFillRect(hdc, rc.as_ptr(), 0, color) };
}

fn frame(hdc: usize, l: i32, t: i32, r: i32, b: i32, color: u32) {
    fillc(hdc, l, t, r, t + 1, color);
    fillc(hdc, l, b - 1, r, b, color);
    fillc(hdc, l, t, l + 1, b, color);
    fillc(hdc, r - 1, t, r, b, color);
}

/// The background the parent wants for the control (`WM_CTLCOLOR*`), as a colour.
fn background(h: H, kind: u32) -> u32 {
    let parent = ctl::parent_of(h);
    if parent != 0 {
        // The parent may answer with a brush: its colour is not readable here, so
        // the message is only sent to let it set the text colour on a probe DC when
        // it wants; the system colours stay.
        let _ = kind;
    }
    match kind {
        ctl::WM_CTLCOLOREDIT | ctl::WM_CTLCOLORLISTBOX => sys(COLOR_WINDOW),
        _ => sys(COLOR_BTNFACE),
    }
}

struct Painter {
    h: H,
    hdc: usize,
    ps: u::PaintStruct,
    w: i32,
    ht: i32,
}

fn begin(h: H) -> Option<Painter> {
    let i = info(h)?;
    let mut ps = u::PaintStruct::default();
    // SAFETY: a window of this process.
    let hdc = unsafe { super::BeginPaint(h, &mut ps) };
    if hdc == 0 {
        return None;
    }
    let font = {
        let f = ctl::send(h, ctl::WM_GETFONT, 0, 0) as usize;
        if f != 0 {
            f
        } else {
            // SAFETY: a stock object.
            unsafe { GetStockObject(DEFAULT_GUI_FONT) }
        }
    };
    // SAFETY: our own DC and a font.
    unsafe {
        SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
    }
    Some(Painter { h, hdc, ps, w: i.client.width(), ht: i.client.height() })
}

impl Painter {
    fn end(self) {
        // SAFETY: the DC of this paint.
        unsafe { super::EndPaint(self.h, &self.ps) };
    }

    fn text(&self, s: &[u16], rect: [i32; 4], flags: u32, color: u32) {
        if s.is_empty() {
            return;
        }
        let mut r = rect;
        // SAFETY: our DC; the string and rectangle are ours.
        unsafe {
            SetTextColor(self.hdc, color);
            super::DrawTextW(self.hdc, s.as_ptr(), s.len() as i32, r.as_mut_ptr(), flags);
        }
    }

    fn extent(&self, s: &[u16]) -> (i32, i32) {
        let mut sz = [0i32; 2];
        if s.is_empty() {
            return (0, 0);
        }
        // SAFETY: our DC.
        unsafe { GetTextExtentPoint32W(self.hdc, s.as_ptr(), s.len() as i32, sz.as_mut_ptr()) };
        (sz[0], sz[1])
    }
}

fn strip_prefix(text: &[u16], underline: bool) -> Vec<u16> {
    // The mnemonic marker is dropped (the underline needs `DT_HIDEPREFIX` rules the
    // text functions of this GDI may not have); `&&` stands for one `&`.
    let _ = underline;
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == u16::from(b'&') {
            if text.get(i + 1) == Some(&u16::from(b'&')) {
                out.push(u16::from(b'&'));
                i += 2;
            } else {
                i += 1;
            }
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    out
}

pub fn paint_button(h: H) {
    let Some(p) = begin(h) else { return };
    let style = ctl::style_of(h);
    let kind = style & ctl::BS_TYPEMASK;
    let enabled = style & 0x0800_0000 == 0;
    let state = ctl::send(h, ctl::BM_GETSTATE, 0, 0) as u32;
    let checked = state & 3;
    let pushed = state & ctl::BST_PUSHED != 0;
    let focus = state & ctl::BST_FOCUS != 0;
    let text = strip_prefix(&ctl::text_of(h), true);
    let face = background(h, ctl::WM_CTLCOLORBTN);
    let fg = if enabled { sys(COLOR_BTNTEXT) } else { sys(COLOR_GRAYTEXT) };
    fillc(p.hdc, 0, 0, p.w, p.ht, face);
    match kind {
        BS_GROUPBOX => {
            let (tw, th) = p.extent(&text);
            let top = th / 2;
            let line = sys(COLOR_3DSHADOW);
            frame(p.hdc, 0, top, p.w, p.ht, line);
            if !text.is_empty() {
                fillc(p.hdc, 6, top, 10 + tw, top + 1, face);
                p.text(&text, [8, 0, p.w - 4, th + 2], DT_LEFT | DT_SINGLELINE | DT_NOPREFIX, fg);
            }
        }
        BS_AUTOCHECKBOX
        | ctl::BS_CHECKBOX
        | ctl::BS_3STATE
        | ctl::BS_AUTO3STATE
        | ctl::BS_RADIOBUTTON
        | BS_AUTORADIOBUTTON => {
            let radio = kind == ctl::BS_RADIOBUTTON || kind == BS_AUTORADIOBUTTON;
            let box_sz = 13;
            let top = (p.ht - box_sz) / 2;
            let line = if enabled { sys(COLOR_BTNTEXT) } else { sys(COLOR_GRAYTEXT) };
            let inside = if pushed { sys(COLOR_3DLIGHT) } else { sys(COLOR_WINDOW) };
            if radio {
                circle(p.hdc, 0, top, box_sz, inside, line);
                if checked == 1 {
                    circle(p.hdc, 3, top + 3, box_sz - 6, line, line);
                }
            } else {
                fillc(p.hdc, 0, top, box_sz, top + box_sz, inside);
                frame(p.hdc, 0, top, box_sz, top + box_sz, line);
                if checked == 1 {
                    check_mark(p.hdc, 0, top, line);
                } else if checked == 2 {
                    fillc(p.hdc, 3, top + 3, box_sz - 3, top + box_sz - 3, sys(COLOR_GRAYTEXT));
                }
            }
            let left = box_sz + 5;
            let flags = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS;
            p.text(&text, [left, 0, p.w, p.ht], flags, fg);
            if focus && !text.is_empty() {
                let (tw, th) = p.extent(&text);
                let ty = (p.ht - th) / 2;
                frame(p.hdc, left - 2, ty - 1, (left + tw + 2).min(p.w), ty + th + 1, sys(COLOR_3DSHADOW));
            }
        }
        _ => {
            // Push buttons.
            let def = kind == BS_DEFPUSHBUTTON;
            let bg = if pushed { face } else { rgb(face, 24) };
            fillc(p.hdc, 0, 0, p.w, p.ht, bg);
            let border = if def || focus { sys(COLOR_HIGHLIGHT) } else { sys(COLOR_3DSHADOW) };
            frame(p.hdc, 0, 0, p.w, p.ht, border);
            if def || focus {
                frame(p.hdc, 1, 1, p.w - 1, p.ht - 1, border);
            }
            let off = i32::from(pushed);
            let flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS;
            p.text(&text, [2 + off, off, p.w - 2 + off, p.ht + off], flags, fg);
        }
    }
    p.end();
}

fn check_mark(hdc: usize, x: i32, y: i32, color: u32) {
    // A tick: two short strokes drawn with 1x1 fills (a 13x13 box).
    for (dx, dy) in [(3, 6), (4, 7), (5, 8), (6, 7), (7, 6), (8, 5), (9, 4)] {
        fillc(hdc, x + dx, y + dy, x + dx + 1, y + dy + 2, color);
    }
}

/// A filled circle of `d` pixels in `fill` with an outline in `line` (scan lines).
fn circle(hdc: usize, x: i32, y: i32, d: i32, fill: u32, line: u32) {
    let r2 = (d * d) as i64;
    for row in 0..d {
        // The chord of the circle at the row's centre.
        let dy2 = (2 * row + 1 - d) as i64;
        let half2 = r2 - dy2 * dy2; // (d^2 - dy^2) in quarter pixels
        if half2 <= 0 {
            continue;
        }
        let half = (half2 as u64).isqrt() as i32;
        let left = (d - half) / 2;
        let right = (d + half) / 2;
        fillc(hdc, x + left, y + row, x + right, y + row + 1, line);
        if fill != line {
            fillc(hdc, x + left + 1, y + row, x + right - 1, y + row + 1, fill);
        }
    }
}

pub fn paint_static(h: H, image: usize) {
    let Some(p) = begin(h) else { return };
    let style = ctl::style_of(h);
    let kind = style & ctl::SS_TYPEMASK;
    let enabled = style & 0x0800_0000 == 0;
    let bg = background(h, ctl::WM_CTLCOLORSTATIC);
    let fg = if enabled { sys(COLOR_WINDOWTEXT) } else { sys(COLOR_GRAYTEXT) };
    match kind {
        ctl::SS_LEFT | ctl::SS_CENTER | ctl::SS_RIGHT | ctl::SS_SIMPLE | ctl::SS_LEFTNOWORDWRAP => {
            fillc(p.hdc, 0, 0, p.w, p.ht, bg);
            let raw = ctl::text_of(h);
            let text = if style & ctl::SS_NOPREFIX != 0 { raw } else { strip_prefix(&raw, true) };
            let mut flags = match kind {
                ctl::SS_CENTER => DT_CENTER,
                ctl::SS_RIGHT => DT_RIGHT,
                _ => DT_LEFT,
            } | DT_NOPREFIX;
            if kind == ctl::SS_SIMPLE || kind == ctl::SS_LEFTNOWORDWRAP {
                flags |= DT_SINGLELINE;
            } else {
                flags |= DT_WORDBREAK;
            }
            p.text(&text, [0, 0, p.w, p.ht], flags, fg);
        }
        SS_ICON => {
            fillc(p.hdc, 0, 0, p.w, p.ht, bg);
            if image != 0 {
                // SAFETY: our DC and an icon handle of this process.
                unsafe { super::icons::DrawIconEx(p.hdc, 0, 0, image, 0, 0, 0, 0, 3) };
            }
        }
        ctl::SS_BLACKRECT => fillc(p.hdc, 0, 0, p.w, p.ht, 0),
        ctl::SS_GRAYRECT => fillc(p.hdc, 0, 0, p.w, p.ht, 0x0080_8080),
        ctl::SS_WHITERECT => fillc(p.hdc, 0, 0, p.w, p.ht, 0x00FF_FFFF),
        ctl::SS_BLACKFRAME => frame(p.hdc, 0, 0, p.w, p.ht, 0),
        ctl::SS_GRAYFRAME => frame(p.hdc, 0, 0, p.w, p.ht, 0x0080_8080),
        ctl::SS_WHITEFRAME => frame(p.hdc, 0, 0, p.w, p.ht, 0x00FF_FFFF),
        ctl::SS_ETCHEDHORZ => {
            fillc(p.hdc, 0, 0, p.w, p.ht, bg);
            fillc(p.hdc, 0, p.ht / 2, p.w, p.ht / 2 + 1, sys(COLOR_3DSHADOW));
            fillc(p.hdc, 0, p.ht / 2 + 1, p.w, p.ht / 2 + 2, 0x00FF_FFFF);
        }
        ctl::SS_ETCHEDVERT => {
            fillc(p.hdc, 0, 0, p.w, p.ht, bg);
            fillc(p.hdc, p.w / 2, 0, p.w / 2 + 1, p.ht, sys(COLOR_3DSHADOW));
            fillc(p.hdc, p.w / 2 + 1, 0, p.w / 2 + 2, p.ht, 0x00FF_FFFF);
        }
        ctl::SS_ETCHEDFRAME => {
            fillc(p.hdc, 0, 0, p.w, p.ht, bg);
            frame(p.hdc, 0, 0, p.w, p.ht, sys(COLOR_3DSHADOW));
        }
        _ => fillc(p.hdc, 0, 0, p.w, p.ht, bg),
    }
    p.end();
}

/// The character index nearest to x pixels from the left of an edit control.
pub fn edit_hit(h: H, x: i32, _y: i32) -> usize {
    let text = ctl::text_of(h);
    let Some(p) = begin_measure(h) else { return text.len() };
    let x = x - 3;
    let mut best = 0;
    for i in 0..=text.len() {
        let (w, _) = measure(p, &text[..i]);
        if w > x {
            let (prev, _) = if i > 0 { measure(p, &text[..i - 1]) } else { (0, 0) };
            best = if x - prev < w - x { i.saturating_sub(1) } else { i };
            end_measure(h, p);
            return best;
        }
        best = i;
    }
    end_measure(h, p);
    best
}

fn begin_measure(h: H) -> Option<usize> {
    let hdc = super::GetDC(h);
    if hdc == 0 {
        return None;
    }
    let f = ctl::send(h, ctl::WM_GETFONT, 0, 0) as usize;
    // SAFETY: our DC and a font or stock object.
    unsafe { SelectObject(hdc, if f != 0 { f } else { GetStockObject(DEFAULT_GUI_FONT) }) };
    Some(hdc)
}

fn measure(hdc: usize, s: &[u16]) -> (i32, i32) {
    let mut sz = [0i32; 2];
    if s.is_empty() {
        return (0, 0);
    }
    // SAFETY: our DC.
    unsafe { GetTextExtentPoint32W(hdc, s.as_ptr(), s.len() as i32, sz.as_mut_ptr()) };
    (sz[0], sz[1])
}

fn end_measure(h: H, hdc: usize) {
    super::ReleaseDC(h, hdc);
}

pub fn paint_edit(h: H) {
    let Some(p) = begin(h) else { return };
    let style = ctl::style_of(h);
    let enabled = style & 0x0800_0000 == 0;
    let ro = style & ctl::ES_READONLY != 0;
    let bg = if enabled && !ro { sys(COLOR_WINDOW) } else { sys(COLOR_BTNFACE) };
    let fg = if enabled { sys(COLOR_WINDOWTEXT) } else { sys(COLOR_GRAYTEXT) };
    fillc(p.hdc, 0, 0, p.w, p.ht, bg);
    let mut text = ctl::text_of(h);
    let (s0, e0, pass) = ctl::edit_state(h, |s| (s.sel_start.min(s.sel_end), s.sel_start.max(s.sel_end), s.password));
    let pw = if pass != 0 {
        pass
    } else if style & ctl::ES_PASSWORD != 0 {
        u16::from(b'*')
    } else {
        0
    };
    if pw != 0 {
        text.iter_mut().for_each(|c| *c = pw);
    }
    let focused = super::GetFocus() == h;
    let multi = style & ctl::ES_MULTILINE != 0;
    let (_, lh) = p.extent(&[u16::from(b'M')]);
    let lh = lh.max(12);
    if multi {
        // Lines split at CR LF; no wrapping unless the style asks (ES_AUTOHSCROLL absent):
        // drawn as they are.
        let mut y = 2;
        let mut start = 0;
        let mut idx = 0;
        while idx <= text.len() {
            if idx == text.len() || text[idx] == 10 {
                let mut end = idx;
                if end > start && text[end - 1] == 13 {
                    end -= 1;
                }
                draw_line(&p, &text[start..end], start, s0, e0, 3, y, lh, fg, focused);
                y += lh;
                start = idx + 1;
            }
            idx += 1;
        }
    } else {
        let y = ((p.ht - lh) / 2).max(0);
        draw_line(&p, &text, 0, s0, e0, 3, y, lh, fg, focused);
    }
    p.end();
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    p: &Painter,
    line: &[u16],
    base: usize,
    s0: usize,
    e0: usize,
    x: i32,
    y: i32,
    lh: i32,
    fg: u32,
    focused: bool,
) {
    // The selection within this line.
    let ls = s0.max(base).min(base + line.len()) - base;
    let le = e0.max(base).min(base + line.len()) - base;
    let (x1, _) = p.extent(&line[..ls]);
    let (x2, _) = p.extent(&line[..le]);
    if focused && le > ls {
        fillc(p.hdc, x + x1, y, x + x2, y + lh, sys(COLOR_HIGHLIGHT));
    }
    if !line.is_empty() {
        let flags = DT_LEFT | DT_SINGLELINE | DT_NOPREFIX;
        p.text(line, [x, y, p.w - 2, y + lh], flags, fg);
        if focused && le > ls {
            p.text(&line[ls..le], [x + x1, y, p.w - 2, y + lh], flags, sys(COLOR_HIGHLIGHTTEXT));
        }
    }
    if focused && s0 == e0 && e0 >= base && e0 <= base + line.len() {
        let (cx, _) = p.extent(&line[..e0 - base]);
        fillc(p.hdc, x + cx, y, x + cx + 1, y + lh, fg);
    }
}

pub fn paint_list(h: H) {
    let Some(p) = begin(h) else { return };
    let style = ctl::style_of(h);
    let enabled = style & 0x0800_0000 == 0;
    fillc(p.hdc, 0, 0, p.w, p.ht, if enabled { sys(COLOR_WINDOW) } else { sys(COLOR_BTNFACE) });
    let (items, top, caret) = ctl::list_snapshot(h);
    let ih = ctl::list_item_height(h);
    let focused = super::GetFocus() == h;
    let mut y = 0;
    for (n, (text, sel)) in items.iter().enumerate().skip(top.max(0) as usize) {
        if y >= p.ht {
            break;
        }
        let fg = if *sel {
            sys(COLOR_HIGHLIGHTTEXT)
        } else if enabled {
            sys(COLOR_WINDOWTEXT)
        } else {
            sys(COLOR_GRAYTEXT)
        };
        if *sel {
            fillc(p.hdc, 0, y, p.w, y + ih, sys(COLOR_HIGHLIGHT));
        } else if focused && n as i32 == caret {
            frame(p.hdc, 0, y, p.w, y + ih, sys(COLOR_3DSHADOW));
        }
        p.text(text, [3, y, p.w - 2, y + ih], DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS, fg);
        y += ih;
    }
    p.end();
}

/// `WM_NCPAINT` of a bordered control: a 1-pixel line round the window.
pub fn paint_border(h: H) {
    if !ctl::has_border(h) {
        return;
    }
    let hdc = super::GetWindowDC(h);
    if hdc == 0 {
        return;
    }
    if let Some(i) = info(h) {
        frame(hdc, 0, 0, i.window.width(), i.window.height(), sys(COLOR_3DSHADOW));
    }
    super::ReleaseDC(h, hdc);
}

// ---- the clipboard, for the edit control

pub fn clipboard_set_text(h: H, text: &[u16]) {
    if super::clipboard::OpenClipboard(h) == 0 {
        return;
    }
    super::clipboard::EmptyClipboard();
    let bytes = (text.len() + 1) * 2;
    // SAFETY: a movable block we fill and hand to the clipboard.
    unsafe {
        let mem = GlobalAlloc(0x0002, bytes);
        if !mem.is_null() {
            let p = GlobalLock(mem) as *mut u16;
            for (i, c) in text.iter().enumerate() {
                *p.add(i) = *c;
            }
            *p.add(text.len()) = 0;
            GlobalUnlock(mem);
            super::clipboard::SetClipboardData(13, mem as usize);
        }
    }
    super::clipboard::CloseClipboard();
}

pub fn clipboard_get_text(h: H) -> Option<Vec<u16>> {
    if super::clipboard::OpenClipboard(h) == 0 {
        return None;
    }
    let mut out = None;
    let mem = super::clipboard::GetClipboardData(13);
    if mem != 0 {
        // SAFETY: the clipboard's block, read while it is open.
        unsafe {
            let p = GlobalLock(mem as *mut u8) as *const u16;
            if !p.is_null() {
                let max = GlobalSize(mem as *mut u8) / 2;
                let mut v = Vec::new();
                for i in 0..max {
                    let c = *p.add(i);
                    if c == 0 {
                        break;
                    }
                    v.push(c);
                }
                out = Some(v);
                GlobalUnlock(mem as *mut u8);
            }
        }
    }
    super::clipboard::CloseClipboard();
    out
}
