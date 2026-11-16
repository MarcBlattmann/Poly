//! `msctls_statusbar32`: parts, texts, layout along the parent's bottom (or top) and painting
//! (`specs/windows/comctl32.dll/msctls_statusbar32.yaml`, `CreateStatusWindowW.yaml`; Learn,
//! "Status Bar Control Reference"). The model is `poly_comctl::StatusBar`.

use crate::cc::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_comctl::StatusBar;
use poly_win32_gui::SpinLock;

pub const CCS_TOP: u32 = 0x0001;
pub const CCS_NORESIZE: u32 = 0x0004;
pub const CCS_NOPARENTALIGN: u32 = 0x0008;
pub const SBARS_SIZEGRIP: u32 = 0x0100;

/// The height of a status bar.
pub const HEIGHT: i32 = 22;
/// `SB_GETBORDERS`: horizontal, vertical and between the parts.
const BORDERS: [i32; 3] = [0, 2, 2];

const SBT_NOBORDERS: u32 = 0x100;
const SBT_POPOUT: u32 = 0x200;
const SBT_OWNERDRAW: u32 = 0x1000;
const SB_SIMPLEID: usize = 0xFF;

struct State {
    bar: StatusBar,
    simple_text: Vec<u16>,
    min_height: i32,
    font: usize,
    bk: u32,
}

static STATES: SpinLock<BTreeMap<usize, State>> = SpinLock::new(BTreeMap::new());

fn with<R>(h: usize, f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATES.with(|m| m.get_mut(&h).map(f))
}

pub fn on_create(hwnd: usize, text: Vec<u16>) {
    let mut bar = StatusBar::default();
    bar.set_text(0, 0, &text);
    STATES.with(|m| m.insert(hwnd, State { bar, simple_text: Vec::new(), min_height: 0, font: 0, bk: CLR_DEFAULT }));
}

pub fn on_destroy(hwnd: usize) {
    STATES.with(|m| m.remove(&hwnd));
}

/// The rectangles of the parts of a bar `width` wide and `height` high: each starts after the
/// previous one with the border between, below the vertical border.
pub fn part_rects(edges: &[i32], width: i32, height: i32) -> Vec<Rc> {
    let mut out = Vec::with_capacity(edges.len());
    let mut left = 0;
    for (i, &e) in edges.iter().enumerate() {
        let right = if e < 0 || e > width { width } else { e };
        out.push([if i == 0 { 0 } else { left }, BORDERS[1], right.max(left), height]);
        left = right + BORDERS[2];
    }
    out
}

/// Put the bar along the parent's bottom (`CCS_TOP`: top) at its height, unless the style
/// says otherwise (`WM_SIZE`: `CCS_NORESIZE` keeps the size, `CCS_NOPARENTALIGN` takes the height only).
fn layout(hwnd: usize) {
    let style = style_of(hwnd);
    if style & CCS_NORESIZE != 0 {
        return;
    }
    let h = with(hwnd, |s| HEIGHT.max(s.min_height)).unwrap_or(HEIGHT);
    // SAFETY: plain window calls.
    unsafe {
        if style & CCS_NOPARENTALIGN != 0 {
            let r = window_rect(hwnd);
            SetWindowPos(hwnd, 0, 0, 0, rc_w(&r), h, 0x16);
            return;
        }
        let parent = GetParent(hwnd);
        if parent == 0 {
            return;
        }
        let pc = client_rect(parent);
        let y = if style & 3 == CCS_TOP { 0 } else { pc[3] - h };
        SetWindowPos(hwnd, 0, 0, y, rc_w(&pc), h, 0x14);
    }
}

fn draw(hwnd: usize, hdc: usize) {
    let c = client_rect(hwnd);
    let (face, shadow, light, text_color) =
        (sys_color(COLOR_BTNFACE), sys_color(COLOR_BTNSHADOW), sys_color(COLOR_BTNHIGHLIGHT), sys_color(COLOR_BTNTEXT));
    let Some((parts, texts, simple, simple_text, font, bk)) =
        with(hwnd, |s| (s.bar.edges.clone(), s.bar.texts.clone(), s.bar.simple, s.simple_text.clone(), s.font, s.bk))
    else {
        return;
    };
    fill(hdc, &c, if bk == CLR_DEFAULT { face } else { bk });
    // SAFETY: the DC of the control and a font of its own or the GUI font.
    let old = unsafe { SelectObject(hdc, if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) }) };
    // SAFETY: the DC.
    unsafe {
        SetBkMode(hdc, 1);
        SetTextColor(hdc, text_color);
    }
    let rects =
        if simple { alloc::vec![[0, BORDERS[1], rc_w(&c), rc_h(&c)]] } else { part_rects(&parts, rc_w(&c), rc_h(&c)) };
    for (i, r) in rects.iter().enumerate() {
        let (text, kind) = if simple { (simple_text.clone(), 0) } else { texts.get(i).cloned().unwrap_or_default() };
        if kind & SBT_NOBORDERS == 0 {
            // A sunken part: the top and left edges are in shadow, the bottom and right in light
            // (popped out: the other way round).
            let (a, b) = if kind & SBT_POPOUT != 0 { (light, shadow) } else { (shadow, light) };
            fill(hdc, &[r[0], r[1], r[2], r[1] + 1], a);
            fill(hdc, &[r[0], r[1], r[0] + 1, r[3]], a);
            fill(hdc, &[r[0], r[3] - 1, r[2], r[3]], b);
            fill(hdc, &[r[2] - 1, r[1], r[2], r[3]], b);
        }
        if kind & SBT_OWNERDRAW != 0 {
            continue;
        }
        draw_part_text(hdc, r, &text);
    }
    if style_of(hwnd) & SBARS_SIZEGRIP != 0 {
        // The grip: dots on the diagonals at the bottom right corner.
        for k in 1..=3 {
            for j in 0..k {
                let (x, y) = (c[2] - 3 * (j + 1) - 1, c[3] - 3 * (k - j) - 1);
                fill(hdc, &[x, y, x + 2, y + 2], shadow);
            }
        }
    }
    // SAFETY: the font taken above.
    unsafe { SelectObject(hdc, old) };
}

/// A part's text: left aligned, or split at tab characters into a centred and a right aligned part.
fn draw_part_text(hdc: usize, r: &Rc, text: &[u16]) {
    let inner = [r[0] + 3, r[1] + 1, r[2] - 3, r[3] - 1];
    let fmt = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS;
    let mut pieces = text.split(|&c| c == 9);
    if let Some(first) = pieces.next() {
        draw_text(hdc, first, &inner, fmt | DT_LEFT);
    }
    if let Some(mid) = pieces.next() {
        draw_text(hdc, mid, &inner, fmt | DT_CENTER);
    }
    if let Some(right) = pieces.next() {
        draw_text(hdc, right, &inner, fmt | DT_RIGHT);
    }
}

fn paint_event(hwnd: usize) {
    let mut ps = [0u8; 72];
    // SAFETY: a PAINTSTRUCT of at most 72 bytes.
    unsafe {
        let hdc = BeginPaint(hwnd, ps.as_mut_ptr());
        draw(hwnd, hdc);
        EndPaint(hwnd, ps.as_ptr());
    }
}

const SB_SETTEXTA: u32 = 0x401;
const SB_GETTEXTA: u32 = 0x402;
const SB_GETTEXTLENGTHA: u32 = 0x403;
const SB_SETPARTS: u32 = 0x404;
const SB_GETPARTS: u32 = 0x406;
const SB_GETBORDERS: u32 = 0x407;
const SB_SETMINHEIGHT: u32 = 0x408;
const SB_SIMPLE: u32 = 0x409;
const SB_GETRECT: u32 = 0x40A;
const SB_SETTEXTW: u32 = 0x40B;
const SB_GETTEXTLENGTHW: u32 = 0x40C;
const SB_GETTEXTW: u32 = 0x40D;
const SB_ISSIMPLE: u32 = 0x40E;
const SB_SETICON: u32 = 0x40F;
const SB_GETICON: u32 = 0x414;
const CCM_SETBKCOLOR: u32 = 0x2001;
const CCM_SETUNICODEFORMAT: u32 = 0x2005;
const CCM_GETUNICODEFORMAT: u32 = 0x2006;

unsafe fn ansi_of(p: usize) -> Vec<u16> {
    if p == 0 {
        return Vec::new();
    }
    let mut b = Vec::new();
    let mut q = p as *const u8;
    // SAFETY: a NUL-terminated string.
    unsafe {
        while *q != 0 {
            b.push(*q);
            q = q.add(1);
        }
    }
    poly_kernelbase::text::from_ansi(&b)
}

/// The window procedure of the class (every message but the ones `controls::control_proc` handles).
pub fn proc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the messages' pointers are the caller's structures (`commctrl.h`).
    unsafe {
        match msg {
            WM_SIZE => {
                layout(hwnd);
                0
            }
            WM_PAINT => {
                paint_event(hwnd);
                0
            }
            WM_PRINT | WM_PRINTCLIENT => {
                draw(hwnd, w);
                0
            }
            WM_ERASEBKGND => 1,
            WM_SETFONT => {
                with(hwnd, |s| s.font = w);
                if l & 0xFFFF != 0 {
                    invalidate(hwnd);
                }
                0
            }
            WM_GETFONT => with(hwnd, |s| s.font).unwrap_or(0) as isize,
            WM_SETTEXT => {
                let t = wide_at(l as usize, -1);
                with(hwnd, |s| s.bar.set_text(0, 0, &t));
                invalidate(hwnd);
                1
            }
            WM_GETTEXT => {
                let t = with(hwnd, |s| s.bar.text(0).map(|x| x.0.clone()).unwrap_or_default()).unwrap_or_default();
                put_text(l as usize, w, &t) as isize
            }
            WM_GETTEXTLENGTH => with(hwnd, |s| s.bar.text(0).map_or(0, |x| x.0.len())).unwrap_or(0) as isize,
            SB_SETPARTS => {
                let n = w.min(256);
                let edges: Vec<i32> = (0..n).map(|i| rd::<i32>(l as usize, 4 * i)).collect();
                let ok = l != 0 && with(hwnd, |s| s.bar.set_parts(&edges)).unwrap_or(false);
                invalidate(hwnd);
                isize::from(ok)
            }
            SB_GETPARTS => {
                let edges = with(hwnd, |s| s.bar.edges.clone()).unwrap_or_default();
                if l != 0 {
                    for (i, e) in edges.iter().enumerate().take(w) {
                        wr(l as usize, 4 * i, *e);
                    }
                }
                edges.len() as isize
            }
            SB_GETBORDERS => {
                if l != 0 {
                    for (i, b) in BORDERS.iter().enumerate() {
                        wr(l as usize, 4 * i, *b);
                    }
                }
                1
            }
            SB_SETMINHEIGHT => {
                with(hwnd, |s| s.min_height = w as i32);
                layout(hwnd);
                0
            }
            SB_GETRECT => {
                let c = client_rect(hwnd);
                let rects = with(hwnd, |s| {
                    if s.bar.simple {
                        alloc::vec![[0, BORDERS[1], rc_w(&c), rc_h(&c)]]
                    } else {
                        part_rects(&s.bar.edges, rc_w(&c), rc_h(&c))
                    }
                })
                .unwrap_or_default();
                match rects.get(w & 0xFF) {
                    Some(r) if l != 0 => {
                        for (i, v) in r.iter().enumerate() {
                            wr(l as usize, 4 * i, *v);
                        }
                        1
                    }
                    _ => 0,
                }
            }
            SB_SIMPLE => {
                with(hwnd, |s| s.bar.simple = w != 0);
                invalidate(hwnd);
                1
            }
            SB_ISSIMPLE => with(hwnd, |s| s.bar.simple).unwrap_or(false) as isize,
            SB_SETTEXTA | SB_SETTEXTW => {
                let t = if msg == SB_SETTEXTW { wide_at(l as usize, -1) } else { ansi_of(l as usize) };
                let ok = if w & 0xFF == SB_SIMPLEID {
                    with(hwnd, |s| s.simple_text = t);
                    true
                } else {
                    with(hwnd, |s| s.bar.set_text(w & 0xFF, (w as u32) & !0xFF, &t)).unwrap_or(false)
                };
                invalidate(hwnd);
                isize::from(ok)
            }
            SB_GETTEXTLENGTHA | SB_GETTEXTLENGTHW => with(hwnd, |s| {
                if w & 0xFF == SB_SIMPLEID {
                    return s.simple_text.len() as isize;
                }
                s.bar.text(w & 0xFF).map_or(0, |(t, ty)| (t.len() as u32 | ty << 16) as isize)
            })
            .unwrap_or(0),
            SB_GETTEXTA | SB_GETTEXTW => {
                let found = with(hwnd, |s| {
                    if w & 0xFF == SB_SIMPLEID {
                        Some((s.simple_text.clone(), 0))
                    } else {
                        s.bar.text(w & 0xFF).cloned()
                    }
                })
                .flatten();
                let Some((t, ty)) = found else { return 0 };
                if l != 0 {
                    if msg == SB_GETTEXTW {
                        put_text(l as usize, t.len() + 1, &t);
                    } else {
                        let a = poly_kernelbase::text::to_ansi(&t);
                        core::ptr::copy_nonoverlapping(a.as_ptr(), l as *mut u8, a.len());
                        *(l as *mut u8).add(a.len()) = 0;
                    }
                }
                (t.len() as u32 | ty << 16) as isize
            }
            SB_SETICON | SB_GETICON => 0,
            CCM_SETBKCOLOR => {
                let old = with(hwnd, |s| core::mem::replace(&mut s.bk, l as u32)).unwrap_or(CLR_DEFAULT);
                invalidate(hwnd);
                old as isize
            }
            CCM_SETUNICODEFORMAT => 1,
            CCM_GETUNICODEFORMAT => 1,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
