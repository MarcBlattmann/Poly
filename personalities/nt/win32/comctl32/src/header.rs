//! `SysHeader32`: the column headings of a list view, and the messages programs send one
//! (`HDM_*`; Learn, "Header Control Reference"). A list view keeps its header in step through
//! [`sync`]; a header made by a program keeps its own items.

use crate::cc::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_win32_gui::SpinLock;

pub const HDS_BUTTONS: u32 = 0x0002;
pub const HDS_HIDDEN: u32 = 0x0008;

const HDF_LEFT: i32 = 0;
const HDF_RIGHT: i32 = 1;
const HDF_CENTER: i32 = 2;
const HDF_JUSTIFYMASK: i32 = 3;

/// One heading.
#[derive(Clone, Debug, Default)]
pub struct Item {
    pub text: Vec<u16>,
    pub width: i32,
    pub fmt: i32,
    pub image: i32,
    pub param: isize,
}

struct State {
    items: Vec<Item>,
    font: usize,
    images: usize,
}

static STATES: SpinLock<BTreeMap<usize, State>> = SpinLock::new(BTreeMap::new());

fn with<R>(h: usize, f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATES.with(|m| m.get_mut(&h).map(f))
}

pub fn on_create(hwnd: usize) {
    STATES.with(|m| m.insert(hwnd, State { items: Vec::new(), font: 0, images: 0 }));
}

pub fn on_destroy(hwnd: usize) {
    STATES.with(|m| m.remove(&hwnd));
}

/// The list view replaces the headings with its columns.
pub fn sync(hwnd: usize, items: Vec<Item>) {
    with(hwnd, |s| s.items = items);
    invalidate(hwnd);
}

pub fn set_font(hwnd: usize, font: usize) {
    with(hwnd, |s| s.font = font);
}

pub fn count(hwnd: usize) -> usize {
    with(hwnd, |s| s.items.len()).unwrap_or(0)
}

/// The rectangle of heading `i` in the header's client area.
pub fn item_rect(hwnd: usize, i: usize) -> Option<Rc> {
    let h = rc_h(&client_rect(hwnd));
    with(hwnd, |s| {
        if i >= s.items.len() {
            return None;
        }
        let left: i32 = s.items[..i].iter().map(|x| x.width).sum();
        Some([left, 0, left + s.items[i].width, h])
    })
    .flatten()
}

fn draw(hwnd: usize, hdc: usize) {
    let c = client_rect(hwnd);
    let (face, shadow, light, text_color) =
        (sys_color(COLOR_BTNFACE), sys_color(COLOR_BTNSHADOW), sys_color(COLOR_BTNHIGHLIGHT), sys_color(COLOR_BTNTEXT));
    fill(hdc, &c, face);
    let Some((items, font)) = with(hwnd, |s| (s.items.clone(), s.font)) else { return };
    // SAFETY: the DC and a font.
    let old = unsafe { SelectObject(hdc, if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) }) };
    // SAFETY: the DC.
    unsafe {
        SetBkMode(hdc, 1);
        SetTextColor(hdc, text_color);
    }
    let mut x = 0;
    for it in &items {
        let r = [x, 0, x + it.width, rc_h(&c)];
        x += it.width;
        // A flat heading: a divider on the right, a line below.
        fill(hdc, &[r[2] - 1, r[1] + 2, r[2], r[3] - 2], shadow);
        fill(hdc, &[r[0], r[3] - 1, r[2], r[3]], shadow);
        fill(hdc, &[r[0], r[1], r[2] - 1, r[1] + 1], light);
        let inner = [r[0] + 6, r[1], r[2] - 6, r[3] - 1];
        let align = match it.fmt & HDF_JUSTIFYMASK {
            HDF_RIGHT => DT_RIGHT,
            HDF_CENTER => DT_CENTER,
            _ => DT_LEFT,
        };
        draw_text(hdc, &it.text, &inner, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | align);
    }
    // SAFETY: the font taken above.
    unsafe { SelectObject(hdc, old) };
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

const HDM_GETITEMCOUNT: u32 = 0x1200;
const HDM_INSERTITEMA: u32 = 0x1201;
const HDM_DELETEITEM: u32 = 0x1202;
const HDM_GETITEMA: u32 = 0x1203;
const HDM_SETITEMA: u32 = 0x1204;
const HDM_GETITEMRECT: u32 = 0x1207;
const HDM_SETIMAGELIST: u32 = 0x1208;
const HDM_GETIMAGELIST: u32 = 0x1209;
const HDM_INSERTITEMW: u32 = 0x120A;
const HDM_GETITEMW: u32 = 0x120B;
const HDM_SETITEMW: u32 = 0x120C;
const HDM_ORDERTOINDEX: u32 = 0x120F;
const HDF_STRING: u32 = 0x4000;
const HDI_WIDTH: u32 = 1;
const HDI_TEXT: u32 = 2;
const HDI_FORMAT: u32 = 4;
const HDI_LPARAM: u32 = 8;
const HDI_IMAGE: u32 = 0x20;

/// `HDITEMW` (`commctrl.h`), native layout.
#[repr(C)]
#[derive(Clone, Copy)]
struct HdItem {
    mask: u32,
    cxy: i32,
    text: usize,
    bitmap: usize,
    text_max: i32,
    fmt: i32,
    param: isize,
    image: i32,
    order: i32,
}

pub fn proc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the messages' pointers are the caller's structures (`commctrl.h`).
    unsafe {
        match msg {
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
                invalidate(hwnd);
                0
            }
            WM_GETFONT => with(hwnd, |s| s.font).unwrap_or(0) as isize,
            HDM_GETITEMCOUNT => count(hwnd) as isize,
            HDM_INSERTITEMW | HDM_INSERTITEMA => {
                if l == 0 {
                    return -1;
                }
                let it: HdItem = rd(l as usize, 0);
                let text = if it.mask & HDI_TEXT != 0 { wide_at(it.text, -1) } else { Vec::new() };
                let item = Item {
                    text,
                    width: if it.mask & HDI_WIDTH != 0 { it.cxy } else { 0 },
                    fmt: if it.mask & HDI_FORMAT != 0 { it.fmt } else { 0 },
                    image: if it.mask & HDI_IMAGE != 0 { it.image } else { -1 },
                    param: if it.mask & HDI_LPARAM != 0 { it.param } else { 0 },
                };
                let r = with(hwnd, |s| {
                    let at = (w as i32).clamp(0, s.items.len() as i32) as usize;
                    s.items.insert(at, item);
                    at as isize
                })
                .unwrap_or(-1);
                invalidate(hwnd);
                r
            }
            HDM_DELETEITEM => {
                let ok = with(hwnd, |s| {
                    if w < s.items.len() {
                        s.items.remove(w);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
                invalidate(hwnd);
                isize::from(ok)
            }
            HDM_GETITEMW | HDM_GETITEMA => {
                let Some(it) = with(hwnd, |s| s.items.get(w).cloned()).flatten() else { return 0 };
                if l == 0 {
                    return 0;
                }
                let mut h: HdItem = rd(l as usize, 0);
                if h.mask & HDI_WIDTH != 0 {
                    h.cxy = it.width;
                }
                if h.mask & HDI_FORMAT != 0 {
                    h.fmt = it.fmt | HDF_STRING as i32;
                }
                if h.mask & HDI_LPARAM != 0 {
                    h.param = it.param;
                }
                if h.mask & HDI_IMAGE != 0 {
                    h.image = it.image;
                }
                if h.mask & HDI_TEXT != 0 {
                    put_text(h.text, h.text_max.max(0) as usize, &it.text);
                }
                wr(l as usize, 0, h);
                1
            }
            HDM_SETITEMW | HDM_SETITEMA => {
                if l == 0 {
                    return 0;
                }
                let h: HdItem = rd(l as usize, 0);
                let text = if h.mask & HDI_TEXT != 0 { Some(wide_at(h.text, -1)) } else { None };
                let ok = with(hwnd, |s| match s.items.get_mut(w) {
                    Some(it) => {
                        if h.mask & HDI_WIDTH != 0 {
                            it.width = h.cxy;
                        }
                        if h.mask & HDI_FORMAT != 0 {
                            it.fmt = h.fmt;
                        }
                        if h.mask & HDI_LPARAM != 0 {
                            it.param = h.param;
                        }
                        if h.mask & HDI_IMAGE != 0 {
                            it.image = h.image;
                        }
                        if let Some(t) = text {
                            it.text = t;
                        }
                        true
                    }
                    None => false,
                })
                .unwrap_or(false);
                invalidate(hwnd);
                isize::from(ok)
            }
            HDM_GETITEMRECT => match item_rect(hwnd, w) {
                Some(r) if l != 0 => {
                    for (i, v) in r.iter().enumerate() {
                        wr(l as usize, 4 * i, *v);
                    }
                    1
                }
                _ => 0,
            },
            HDM_ORDERTOINDEX => w as isize,
            HDM_SETIMAGELIST => with(hwnd, |s| core::mem::replace(&mut s.images, l as usize)).unwrap_or(0) as isize,
            HDM_GETIMAGELIST => with(hwnd, |s| s.images).unwrap_or(0) as isize,
            0x2005 | 0x2006 => 1,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
