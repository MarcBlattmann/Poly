//! The common control window classes of comctl32 5.82 (`InitCommonControlsEx.yaml`
//! `classes_5_82`, `messages_5_82`): `InitCommonControls*` registers them, one window procedure
//! serves all and keeps each window's state (`poly_comctl`); what this file does is decode the
//! messages (numbers and structures from `commctrl.h`, x64 layouts) and call the models.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_comctl::{Column, ListView, Progress, StatusBar, Tab, Tabs, Trackbar, TreeView, UpDown, class, class::Size};
use poly_win32_gui::SpinLock;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(wc: *const u8) -> u16;
    fn DefWindowProcW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
    fn GetClassNameW(hwnd: usize, buf: *mut u16, max: i32) -> i32;
    fn GetWindowLongW(hwnd: usize, index: i32) -> i32;
    fn SetWindowLongW(hwnd: usize, index: i32, value: i32) -> i32;
    fn GetParent(hwnd: usize) -> usize;
    fn GetClientRect(hwnd: usize, rect: *mut i32) -> i32;
    fn GetWindowRect(hwnd: usize, rect: *mut i32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetWindowPos(hwnd: usize, after: usize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn LoadCursorW(instance: usize, name: *const u16) -> usize;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: usize,
        menu: usize,
        instance: usize,
        param: usize,
    ) -> usize;
    fn SendMessageW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn GetLocalTime(st: *mut u16);
}

const GWL_STYLE: i32 = -16;
const WM_NCCREATE: u32 = 0x0081;
const WM_CREATE: u32 = 0x0001;
const WM_NCDESTROY: u32 = 0x0082;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const CCS_NORESIZE: u32 = 0x0004;
const CCS_NOPARENTALIGN: u32 = 0x0008;

/// What a window of each class keeps.
enum Model {
    Plain,
    Status(StatusBar),
    Progress(Progress),
    Track(Trackbar),
    UpDown(UpDown),
    Tabs(Tabs),
    List(ListView),
    Tree(TreeView),
    /// The header and toolbar start with no items; this counts them.
    Count(i32),
    /// The rebar's bands.
    Rebar(i32),
}

static CTLS: SpinLock<BTreeMap<usize, Model>> = SpinLock::new(BTreeMap::new());
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Registers the classes (once per process): called by `InitCommonControls` and `InitCommonControlsEx`.
pub fn register_all() {
    if REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    // SAFETY: plain calls; the structure below is a WNDCLASSEXW (80 bytes).
    unsafe {
        let module = GetModuleHandleW(core::ptr::null());
        let cursor = LoadCursorW(0, 32512usize as *const u16);
        for name in class::NAMES {
            let mut wide: Vec<u16> = name.encode_utf16().collect();
            wide.push(0);
            let mut wc = [0u8; 80];
            wc[0..4].copy_from_slice(&80u32.to_le_bytes());
            // CS_GLOBALCLASS | CS_DBLCLKS
            wc[4..8].copy_from_slice(&0x4008u32.to_le_bytes());
            wc[8..16].copy_from_slice(&(control_proc as usize as u64).to_le_bytes());
            wc[24..32].copy_from_slice(&(module as u64).to_le_bytes());
            wc[40..48].copy_from_slice(&(cursor as u64).to_le_bytes());
            wc[64..72].copy_from_slice(&(wide.as_ptr() as u64).to_le_bytes());
            RegisterClassExW(wc.as_ptr());
        }
    }
}

fn model_for(name: &str) -> Model {
    match name.to_ascii_lowercase().as_str() {
        "msctls_statusbar32" => Model::Status(StatusBar::default()),
        "msctls_progress32" => Model::Progress(Progress::default()),
        "msctls_trackbar32" => Model::Track(Trackbar::default()),
        "msctls_updown32" => Model::UpDown(UpDown::default()),
        "systabcontrol32" => Model::Tabs(Tabs::default()),
        "syslistview32" => Model::List(ListView::default()),
        "systreeview32" => Model::Tree(TreeView::default()),
        "sysheader32" | "toolbarwindow32" => Model::Count(0),
        "rebarwindow32" => Model::Rebar(0),
        _ => Model::Plain,
    }
}

fn class_name(hwnd: usize) -> String {
    let mut buf = [0u16; 64];
    // SAFETY: a buffer of 64 units.
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), 64) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

unsafe fn rd<T: Copy>(p: usize, off: usize) -> T {
    // SAFETY: inside a structure of the caller's.
    unsafe { ((p + off) as *const T).read_unaligned() }
}

unsafe fn wr<T: Copy>(p: usize, off: usize, v: T) {
    // SAFETY: inside a structure of the caller's.
    unsafe { ((p + off) as *mut T).write_unaligned(v) }
}

/// A NUL-terminated wide string.
unsafe fn wide_at(p: usize) -> Vec<u16> {
    // 0 is no text and LPSTR_TEXTCALLBACK (-1) says the control asks its owner for the text.
    if p == 0 || p == usize::MAX {
        return Vec::new();
    }
    // SAFETY: a NUL-terminated string.
    unsafe { poly_win32_gui::wide(p as *const u16, -1) }
}

/// A NUL-terminated ANSI string widened (code page 1252).
unsafe fn wide_of_ansi(p: usize) -> Vec<u16> {
    if p == 0 || p == usize::MAX {
        return Vec::new();
    }
    let mut b = Vec::new();
    // SAFETY: a NUL-terminated string.
    unsafe {
        let mut q = p as *const u8;
        while *q != 0 {
            b.push(*q);
            q = q.add(1);
        }
    }
    poly_kernelbase::text::from_ansi(&b)
}

/// Copies `text` into the caller's buffer of `cap` units (a NUL after it); the units copied.
unsafe fn put(buf: usize, cap: usize, text: &[u16]) -> usize {
    if buf == 0 || cap == 0 {
        return 0;
    }
    let n = text.len().min(cap - 1);
    // SAFETY: the caller's buffer of `cap` units.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf as *mut u16, n);
        *(buf as *mut u16).add(n) = 0;
    }
    n
}

fn today() -> Vec<u16> {
    let mut st = [0u16; 8];
    // SAFETY: a SYSTEMTIME of 16 bytes.
    unsafe { GetLocalTime(st.as_mut_ptr()) };
    alloc::format!("{}.{}.{}", st[3], st[1], st[0]).encode_utf16().collect()
}

/// The window procedure of every common control class.
pub unsafe extern "system" fn control_proc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_NCCREATE => {
            let name = class_name(hwnd);
            let make = class::make(&name);
            CTLS.with(|m| m.insert(hwnd, model_for(&name)));
            if let Some(mk) = make {
                // SAFETY: plain window calls on the window being made.
                unsafe {
                    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                    let new = (style | mk.add_style) & !mk.drop_style;
                    if new != style {
                        SetWindowLongW(hwnd, GWL_STYLE, new as i32);
                    }
                }
            }
            // SAFETY: the default procedure keeps the text the window was made with.
            let r = unsafe { DefWindowProcW(hwnd, msg, w, l) };
            let text: Option<Vec<u16>> = match name.to_ascii_lowercase().as_str() {
                "sysdatetimepick32" | "sysmonthcal32" => Some(today()),
                _ => make.and_then(|m| m.text).map(|t| t.encode_utf16().collect()),
            };
            if let Some(mut t) = text {
                t.push(0);
                // SAFETY: a NUL-terminated string.
                unsafe { DefWindowProcW(hwnd, WM_SETTEXT, 0, t.as_ptr() as isize) };
            }
            r
        }
        WM_CREATE => {
            let name = class_name(hwnd);
            if let Some(mk) = class::make(&name) {
                size_on_create(hwnd, mk.size);
            }
            // The status bar's first part has the text the window was made with.
            // SAFETY: the structure is the CREATESTRUCTW of the message (name at offset 72).
            let text = unsafe { wide_at(rd::<usize>(l as usize, 56)) };
            CTLS.with(|m| {
                if let Some(Model::Status(s)) = m.get_mut(&hwnd) {
                    s.set_text(0, 0, &text);
                }
            });
            0
        }
        WM_NCDESTROY => {
            CTLS.with(|m| m.remove(&hwnd));
            // SAFETY: the default procedure.
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }
        _ => {
            if let Some(r) = CTLS.with(|m| m.get_mut(&hwnd).and_then(|model| handle(model, msg, w, l))) {
                return r;
            }
            // SAFETY: the default procedure.
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }
    }
}

/// What the control does to its size at creation (toolbar and rebar across the top, status bar
/// along the bottom, ...).
fn size_on_create(hwnd: usize, size: Size) {
    // SAFETY: plain window calls on the window being made.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let mut cr = [0i32; 4];
        let parent = GetParent(hwnd);
        if parent != 0 {
            GetClientRect(parent, cr.as_mut_ptr());
        }
        let (pw, ph) = (cr[2] - cr[0], cr[3] - cr[1]);
        let mut r = [0i32; 4];
        GetWindowRect(hwnd, r.as_mut_ptr());
        let (cw, ch) = (r[2] - r[0], r[3] - r[1]);
        let aligned = style & (CCS_NORESIZE | CCS_NOPARENTALIGN) == 0;
        match size {
            Size::TopBar(h) if aligned => {
                SetWindowPos(hwnd, 0, 0, 0, pw, h, 0x14);
            }
            Size::BottomBar(h) if aligned => {
                SetWindowPos(hwnd, 0, 0, ph - h, pw, h, 0x14);
            }
            Size::ScrollWidth => {
                SetWindowPos(hwnd, 0, 0, 0, GetSystemMetrics(2), ch, 0x16);
            }
            Size::Height(h) => {
                SetWindowPos(hwnd, 0, 0, 0, cw, h, 0x16);
            }
            _ => {}
        }
    }
}

fn lo(v: isize) -> i32 {
    v as i16 as i32
}

fn hi(v: isize) -> i32 {
    (v >> 16) as i16 as i32
}

/// The messages of the models; `None` leaves a message to the default procedure.
fn handle(model: &mut Model, msg: u32, w: usize, l: isize) -> Option<isize> {
    // SAFETY: the pointers in the messages are the caller's structures (commctrl.h).
    unsafe {
        match model {
            Model::Status(s) => status(s, msg, w, l),
            Model::Progress(p) => progress(p, msg, w, l),
            Model::Track(t) => track(t, msg, w, l),
            Model::UpDown(u) => updown(u, msg, w, l),
            Model::Tabs(t) => tabs(t, msg, w, l),
            Model::List(v) => list(v, msg, w, l),
            Model::Tree(t) => tree(t, msg, w, l),
            Model::Count(n) => count(n, msg, w, l),
            Model::Rebar(n) => rebar(n, msg, w),
            Model::Plain => None,
        }
    }
}

unsafe fn status(s: &mut StatusBar, msg: u32, w: usize, l: isize) -> Option<isize> {
    const SB_SETTEXTA: u32 = 0x401;
    const SB_GETTEXTA: u32 = 0x402;
    const SB_GETTEXTLENGTHA: u32 = 0x403;
    const SB_SETPARTS: u32 = 0x404;
    const SB_GETPARTS: u32 = 0x406;
    const SB_SIMPLE: u32 = 0x409;
    const SB_SETTEXTW: u32 = 0x40B;
    const SB_GETTEXTLENGTHW: u32 = 0x40C;
    const SB_GETTEXTW: u32 = 0x40D;
    const SB_ISSIMPLE: u32 = 0x40E;
    let text_len = |s: &StatusBar, part: usize| -> isize {
        s.text(part).map_or(0, |(t, ty)| (t.len() as u32 | ty << 16) as isize)
    };
    Some(match msg {
        SB_SETPARTS => {
            let n = w.min(256);
            let mut edges = Vec::with_capacity(n);
            for i in 0..n {
                // SAFETY: `w` integers.
                edges.push(unsafe { rd::<i32>(l as usize, 4 * i) });
            }
            isize::from(l != 0 && s.set_parts(&edges))
        }
        SB_GETPARTS => {
            if l != 0 {
                for (i, e) in s.edges.iter().enumerate().take(w) {
                    // SAFETY: `w` integers.
                    unsafe { wr(l as usize, 4 * i, *e) };
                }
            }
            s.edges.len() as isize
        }
        SB_SETTEXTA | SB_SETTEXTW => {
            // SAFETY: a NUL-terminated string.
            let t =
                if msg == SB_SETTEXTW { unsafe { wide_at(l as usize) } } else { unsafe { wide_of_ansi(l as usize) } };
            isize::from(s.set_text(w & 0xFF, (w as u32) & !0xFF, &t))
        }
        SB_GETTEXTLENGTHA | SB_GETTEXTLENGTHW => text_len(s, w & 0xFF),
        SB_GETTEXTA | SB_GETTEXTW => {
            let part = w & 0xFF;
            let Some((t, ty)) = s.text(part).cloned() else { return Some(0) };
            if l != 0 {
                if msg == SB_GETTEXTW {
                    // SAFETY: the caller's buffer holds the text.
                    unsafe { put(l as usize, t.len() + 1, &t) };
                } else {
                    let a = poly_kernelbase::text::to_ansi(&t);
                    // SAFETY: the caller's buffer holds the text.
                    unsafe {
                        core::ptr::copy_nonoverlapping(a.as_ptr(), l as *mut u8, a.len());
                        *(l as *mut u8).add(a.len()) = 0;
                    }
                }
            }
            (t.len() as u32 | ty << 16) as isize
        }
        SB_SIMPLE => {
            s.simple = w != 0;
            1
        }
        SB_ISSIMPLE => isize::from(s.simple),
        WM_SETTEXT => {
            // SAFETY: a NUL-terminated string.
            let t = unsafe { wide_at(l as usize) };
            s.set_text(0, 0, &t);
            return None;
        }
        WM_GETTEXT => {
            let t = s.text(0).map(|x| x.0.clone()).unwrap_or_default();
            // SAFETY: the caller's buffer of `w` units.
            unsafe { put(l as usize, w, &t) as isize }
        }
        WM_GETTEXTLENGTH => s.text(0).map_or(0, |x| x.0.len() as isize),
        _ => return None,
    })
}

unsafe fn progress(p: &mut Progress, msg: u32, w: usize, l: isize) -> Option<isize> {
    const PBM_SETRANGE: u32 = 0x401;
    const PBM_SETPOS: u32 = 0x402;
    const PBM_DELTAPOS: u32 = 0x403;
    const PBM_SETSTEP: u32 = 0x404;
    const PBM_STEPIT: u32 = 0x405;
    const PBM_SETRANGE32: u32 = 0x406;
    const PBM_GETRANGE: u32 = 0x407;
    const PBM_GETPOS: u32 = 0x408;
    Some(match msg {
        PBM_SETRANGE => p.set_range(l as u16 as i32, (l >> 16) as u16 as i32) as isize,
        PBM_SETRANGE32 => p.set_range(w as i32, l as i32) as isize,
        PBM_SETPOS => p.set_pos(w as i32) as isize,
        PBM_DELTAPOS => p.delta_pos(w as i32) as isize,
        PBM_SETSTEP => p.set_step(w as i32) as isize,
        PBM_STEPIT => p.step_it() as isize,
        PBM_GETPOS => p.pos as isize,
        PBM_GETRANGE => {
            if l != 0 {
                // SAFETY: a PBRANGE (two integers).
                unsafe {
                    wr(l as usize, 0, p.lo);
                    wr(l as usize, 4, p.hi);
                }
            }
            (if w != 0 { p.lo } else { p.hi }) as isize
        }
        _ => return None,
    })
}

unsafe fn track(t: &mut Trackbar, msg: u32, w: usize, l: isize) -> Option<isize> {
    const TBM_GETPOS: u32 = 0x400;
    const TBM_GETRANGEMIN: u32 = 0x401;
    const TBM_GETRANGEMAX: u32 = 0x402;
    const TBM_SETPOS: u32 = 0x405;
    const TBM_SETRANGE: u32 = 0x406;
    const TBM_SETRANGEMIN: u32 = 0x407;
    const TBM_SETRANGEMAX: u32 = 0x408;
    const TBM_SETPAGESIZE: u32 = 0x415;
    const TBM_GETPAGESIZE: u32 = 0x416;
    const TBM_SETLINESIZE: u32 = 0x417;
    const TBM_GETLINESIZE: u32 = 0x418;
    Some(match msg {
        TBM_GETPOS => t.pos as isize,
        TBM_GETRANGEMIN => t.lo as isize,
        TBM_GETRANGEMAX => t.hi as isize,
        TBM_SETPOS => {
            t.set_pos(l as i32);
            0
        }
        TBM_SETRANGE => {
            t.set_range(l as u16 as i32, (l >> 16) as u16 as i32);
            0
        }
        TBM_SETRANGEMIN => {
            t.set_range(l as i32, t.hi);
            0
        }
        TBM_SETRANGEMAX => {
            t.set_range(t.lo, l as i32);
            0
        }
        TBM_SETPAGESIZE => core::mem::replace(&mut t.page, l as i32) as isize,
        TBM_GETPAGESIZE => t.page as isize,
        TBM_SETLINESIZE => core::mem::replace(&mut t.line, l as i32) as isize,
        TBM_GETLINESIZE => t.line as isize,
        _ => {
            let _ = w;
            return None;
        }
    })
}

unsafe fn updown(u: &mut UpDown, msg: u32, w: usize, l: isize) -> Option<isize> {
    const UDM_SETRANGE: u32 = 0x465;
    const UDM_GETRANGE: u32 = 0x466;
    const UDM_SETPOS: u32 = 0x467;
    const UDM_GETPOS: u32 = 0x468;
    const UDM_SETBASE: u32 = 0x46D;
    const UDM_GETBASE: u32 = 0x46E;
    const UDM_SETRANGE32: u32 = 0x46F;
    const UDM_GETRANGE32: u32 = 0x470;
    const UDM_SETPOS32: u32 = 0x471;
    const UDM_GETPOS32: u32 = 0x472;
    Some(match msg {
        // MAKELONG(upper, lower)
        UDM_SETRANGE => {
            u.set_range(l as i16 as i32, (l >> 16) as i16 as i32);
            0
        }
        UDM_GETRANGE => ((u.lower as u32 & 0xFFFF) << 16 | (u.upper as u32 & 0xFFFF)) as isize,
        UDM_SETRANGE32 => {
            u.set_range(l as i32, w as i32);
            0
        }
        UDM_GETRANGE32 => {
            // SAFETY: two integers (when given).
            unsafe {
                if w != 0 {
                    wr(w, 0, u.lower);
                }
                if l != 0 {
                    wr(l as usize, 0, u.upper);
                }
            }
            0
        }
        UDM_SETPOS => (u.set_pos(lo(l)) as u32 & 0xFFFF) as isize,
        UDM_GETPOS => (u.pos as u32 & 0xFFFF) as isize,
        UDM_SETPOS32 => u.set_pos(l as i32) as isize,
        UDM_GETPOS32 => {
            if l != 0 {
                // SAFETY: a BOOL (when given): no error.
                unsafe { wr(l as usize, 0, 0i32) };
            }
            u.pos as isize
        }
        UDM_SETBASE => {
            if w == 10 || w == 16 {
                core::mem::replace(&mut u.base, w as i32) as isize
            } else {
                0
            }
        }
        UDM_GETBASE => u.base as isize,
        _ => return None,
    })
}

unsafe fn tabs(t: &mut Tabs, msg: u32, w: usize, l: isize) -> Option<isize> {
    const TCM_GETITEMCOUNT: u32 = 0x1304;
    const TCM_INSERTITEMA: u32 = 0x1307;
    const TCM_DELETEITEM: u32 = 0x1308;
    const TCM_DELETEALLITEMS: u32 = 0x1309;
    const TCM_GETCURSEL: u32 = 0x130B;
    const TCM_SETCURSEL: u32 = 0x130C;
    const TCM_GETITEMW: u32 = 0x133C;
    const TCM_SETITEMW: u32 = 0x133D;
    const TCM_INSERTITEMW: u32 = 0x133E;
    const TCIF_TEXT: u32 = 1;
    const TCIF_IMAGE: u32 = 2;
    const TCIF_PARAM: u32 = 8;
    Some(match msg {
        TCM_GETITEMCOUNT => t.items.len() as isize,
        TCM_INSERTITEMW | TCM_INSERTITEMA => {
            if l == 0 {
                return Some(-1);
            }
            // SAFETY: a TCITEM (mask 0, pszText 16, iImage 28, lParam 32).
            let (mask, text, image, param) = unsafe {
                (
                    rd::<u32>(l as usize, 0),
                    rd::<usize>(l as usize, 16),
                    rd::<i32>(l as usize, 28),
                    rd::<isize>(l as usize, 32),
                )
            };
            let text = if mask & TCIF_TEXT != 0 {
                // SAFETY: a NUL-terminated string.
                if msg == TCM_INSERTITEMW { unsafe { wide_at(text) } } else { unsafe { wide_of_ansi(text) } }
            } else {
                Vec::new()
            };
            t.insert(
                w as i32,
                Tab {
                    text,
                    image: if mask & TCIF_IMAGE != 0 { image } else { -1 },
                    param: if mask & TCIF_PARAM != 0 { param } else { 0 },
                },
            ) as isize
        }
        TCM_DELETEITEM => isize::from(t.delete(w as i32)),
        TCM_DELETEALLITEMS => {
            t.delete_all();
            1
        }
        TCM_GETCURSEL => t.cur as isize,
        TCM_SETCURSEL => t.set_cur_sel(w as i32) as isize,
        TCM_GETITEMW | TCM_SETITEMW => {
            let Some(tab) = t.items.get_mut(w) else { return Some(0) };
            if l == 0 {
                return Some(0);
            }
            // SAFETY: a TCITEM.
            let mask = unsafe { rd::<u32>(l as usize, 0) };
            if msg == TCM_GETITEMW {
                if mask & TCIF_TEXT != 0 {
                    // SAFETY: the caller's buffer.
                    unsafe { put(rd::<usize>(l as usize, 16), rd::<i32>(l as usize, 24).max(0) as usize, &tab.text) };
                }
                if mask & TCIF_IMAGE != 0 {
                    // SAFETY: as above.
                    unsafe { wr(l as usize, 28, tab.image) };
                }
                if mask & TCIF_PARAM != 0 {
                    // SAFETY: as above.
                    unsafe { wr(l as usize, 32, tab.param) };
                }
            } else {
                if mask & TCIF_TEXT != 0 {
                    // SAFETY: a NUL-terminated string.
                    tab.text = unsafe { wide_at(rd::<usize>(l as usize, 16)) };
                }
                if mask & TCIF_IMAGE != 0 {
                    // SAFETY: as above.
                    tab.image = unsafe { rd::<i32>(l as usize, 28) };
                }
                if mask & TCIF_PARAM != 0 {
                    // SAFETY: as above.
                    tab.param = unsafe { rd::<isize>(l as usize, 32) };
                }
            }
            1
        }
        _ => return None,
    })
}

unsafe fn list(v: &mut ListView, msg: u32, w: usize, l: isize) -> Option<isize> {
    const LVM_GETITEMCOUNT: u32 = 0x1004;
    const LVM_DELETEITEM: u32 = 0x1008;
    const LVM_DELETEALLITEMS: u32 = 0x1009;
    const LVM_GETHEADER: u32 = 0x101F;
    const LVM_GETITEMW: u32 = 0x104B;
    const LVM_SETITEMW: u32 = 0x104C;
    const LVM_INSERTITEMW: u32 = 0x104D;
    const LVM_INSERTCOLUMNW: u32 = 0x1061;
    const LVM_GETITEMTEXTW: u32 = 0x1073;
    const LVM_SETITEMTEXTW: u32 = 0x1074;
    const LVIF_TEXT: u32 = 1;
    const LVCF_WIDTH: u32 = 2;
    const LVCF_TEXT: u32 = 4;
    Some(match msg {
        LVM_GETITEMCOUNT => v.items.len() as isize,
        LVM_GETHEADER => 0,
        LVM_INSERTCOLUMNW => {
            if l == 0 {
                return Some(-1);
            }
            // SAFETY: an LVCOLUMN (mask 0, cx 8, pszText 16).
            let (mask, cx, text) =
                unsafe { (rd::<u32>(l as usize, 0), rd::<i32>(l as usize, 8), rd::<usize>(l as usize, 16)) };
            let col = Column {
                // SAFETY: a NUL-terminated string.
                text: if mask & LVCF_TEXT != 0 { unsafe { wide_at(text) } } else { Vec::new() },
                width: if mask & LVCF_WIDTH != 0 { cx } else { 0 },
            };
            v.insert_column(w as i32, col) as isize
        }
        LVM_INSERTITEMW => {
            if l == 0 {
                return Some(-1);
            }
            // SAFETY: an LVITEM (mask 0, iItem 4, pszText 24).
            let (mask, item, text) =
                unsafe { (rd::<u32>(l as usize, 0), rd::<i32>(l as usize, 4), rd::<usize>(l as usize, 24)) };
            // SAFETY: a NUL-terminated string.
            let t = if mask & LVIF_TEXT != 0 { unsafe { wide_at(text) } } else { Vec::new() };
            v.insert_item(item, &t) as isize
        }
        LVM_SETITEMTEXTW | LVM_SETITEMW => {
            if l == 0 {
                return Some(0);
            }
            // SAFETY: an LVITEM (mask 0, iItem 4, iSubItem 8, pszText 24).
            let (mask, item, sub, text) = unsafe {
                (
                    rd::<u32>(l as usize, 0),
                    rd::<i32>(l as usize, 4),
                    rd::<i32>(l as usize, 8),
                    rd::<usize>(l as usize, 24),
                )
            };
            let item = if msg == LVM_SETITEMTEXTW { w as i32 } else { item };
            if msg == LVM_SETITEMW && mask & LVIF_TEXT == 0 {
                return Some(isize::from((item as usize) < v.items.len()));
            }
            // SAFETY: a NUL-terminated string.
            let t = unsafe { wide_at(text) };
            isize::from(v.set_text(item, sub, &t))
        }
        LVM_GETITEMTEXTW | LVM_GETITEMW => {
            if l == 0 {
                return Some(0);
            }
            // SAFETY: an LVITEM (mask 0, iItem 4, iSubItem 8, pszText 24, cchTextMax 32).
            let (mask, item, sub, buf, cap) = unsafe {
                (
                    rd::<u32>(l as usize, 0),
                    rd::<i32>(l as usize, 4),
                    rd::<i32>(l as usize, 8),
                    rd::<usize>(l as usize, 24),
                    rd::<i32>(l as usize, 32).max(0) as usize,
                )
            };
            let item = if msg == LVM_GETITEMTEXTW { w as i32 } else { item };
            if msg == LVM_GETITEMW && mask & LVIF_TEXT == 0 {
                return Some(isize::from((item as usize) < v.items.len()));
            }
            match v.text(item, sub) {
                // SAFETY: the caller's buffer of `cap` units.
                Some(t) => {
                    let n = unsafe { put(buf, cap, t) };
                    if msg == LVM_GETITEMW { 1 } else { n as isize }
                }
                None => 0,
            }
        }
        LVM_DELETEITEM => isize::from(v.delete(w as i32)),
        LVM_DELETEALLITEMS => {
            v.items.clear();
            1
        }
        _ => return None,
    })
}

unsafe fn tree(t: &mut TreeView, msg: u32, w: usize, l: isize) -> Option<isize> {
    const TVM_INSERTITEMA: u32 = 0x1100;
    const TVM_DELETEITEM: u32 = 0x1101;
    const TVM_GETCOUNT: u32 = 0x1105;
    const TVM_GETNEXTITEM: u32 = 0x110A;
    const TVM_SELECTITEM: u32 = 0x110B;
    const TVM_INSERTITEMW: u32 = 0x1132;
    const TVM_GETITEMW: u32 = 0x113E;
    const TVIF_TEXT: u32 = 1;
    Some(match msg {
        TVM_GETCOUNT => t.count() as isize,
        TVM_INSERTITEMW | TVM_INSERTITEMA => {
            if l == 0 {
                return Some(0);
            }
            // SAFETY: a TVINSERTSTRUCT (hParent 0, hInsertAfter 8, mask 16, pszText 40).
            let (parent, after, mask, text) = unsafe {
                (
                    rd::<u64>(l as usize, 0),
                    rd::<u64>(l as usize, 8),
                    rd::<u32>(l as usize, 16),
                    rd::<usize>(l as usize, 40),
                )
            };
            let parent = if parent == poly_comctl::TVI_ROOT { 0 } else { parent as u32 };
            let txt = if mask & TVIF_TEXT != 0 {
                // SAFETY: a NUL-terminated string.
                if msg == TVM_INSERTITEMW { unsafe { wide_at(text) } } else { unsafe { wide_of_ansi(text) } }
            } else {
                Vec::new()
            };
            t.insert(parent, after, &txt) as isize
        }
        TVM_GETNEXTITEM => t.next_item(w as u32, l as u32) as isize,
        TVM_SELECTITEM => {
            if w as u32 == poly_comctl::TVGN_CARET {
                isize::from(t.select(l as u32))
            } else {
                1
            }
        }
        TVM_DELETEITEM => isize::from(t.delete(l as u64)),
        TVM_GETITEMW => {
            if l == 0 {
                return Some(0);
            }
            // SAFETY: a TVITEM (mask 0, hItem 8, pszText 24, cchTextMax 32).
            let (mask, item, buf, cap) = unsafe {
                (
                    rd::<u32>(l as usize, 0),
                    rd::<u64>(l as usize, 8),
                    rd::<usize>(l as usize, 24),
                    rd::<i32>(l as usize, 32).max(0) as usize,
                )
            };
            let Some(txt) = t.text(item as u32) else { return Some(0) };
            if mask & TVIF_TEXT != 0 {
                // SAFETY: the caller's buffer.
                unsafe { put(buf, cap, txt) };
            }
            1
        }
        _ => return None,
    })
}

unsafe fn count(n: &mut i32, msg: u32, w: usize, l: isize) -> Option<isize> {
    // The header's, the toolbar's and the rebar's messages that change how many items there are
    // (they are told apart by their numbers: the toolbar's and rebar's are in the user range).
    const HDM_GETITEMCOUNT: u32 = 0x1200;
    const HDM_DELETEITEM: u32 = 0x1202;
    const HDM_INSERTITEMW: u32 = 0x120A;
    const TB_ADDBITMAP: u32 = 0x413;
    const TB_ADDBUTTONSA: u32 = 0x414;
    const TB_DELETEBUTTON: u32 = 0x416;
    const TB_BUTTONCOUNT: u32 = 0x418;
    const TB_BUTTONSTRUCTSIZE: u32 = 0x41E;
    const TB_AUTOSIZE: u32 = 0x421;
    const TB_ADDBUTTONSW: u32 = 0x444;
    Some(match msg {
        HDM_GETITEMCOUNT | TB_BUTTONCOUNT => *n as isize,
        HDM_INSERTITEMW => {
            let at = w as i32;
            *n += 1;
            at.clamp(0, *n - 1) as isize
        }
        TB_ADDBITMAP => 0,
        TB_ADDBUTTONSA | TB_ADDBUTTONSW => {
            *n += w as i32;
            1
        }
        HDM_DELETEITEM | TB_DELETEBUTTON => {
            if (w as i32) < *n {
                *n -= 1;
                1
            } else {
                0
            }
        }
        TB_BUTTONSTRUCTSIZE | TB_AUTOSIZE => {
            let _ = l;
            0
        }
        _ => return None,
    })
}

unsafe fn rebar(n: &mut i32, msg: u32, w: usize) -> Option<isize> {
    const RB_DELETEBAND: u32 = 0x402;
    const RB_INSERTBANDW: u32 = 0x40A;
    const RB_GETBANDCOUNT: u32 = 0x40C;
    Some(match msg {
        RB_GETBANDCOUNT => *n as isize,
        RB_INSERTBANDW => {
            *n += 1;
            1
        }
        RB_DELETEBAND => {
            if (w as i32) < *n {
                *n -= 1;
                1
            } else {
                0
            }
        }
        _ => return None,
    })
}

fn wz(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// `CreateStatusWindowW(style, text, parent, id)`: a status bar made with the style given.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateStatusWindowW(style: u32, text: *const u16, parent: usize, id: usize) -> usize {
    register_all();
    let class = wz("msctls_statusbar32");
    // SAFETY: plain calls with a NUL-terminated class name.
    unsafe {
        CreateWindowExW(0, class.as_ptr(), text, style, 0, 0, 0, 0, parent, id, GetModuleHandleW(core::ptr::null()), 0)
    }
}

/// `CreateToolbarEx(...)`: a toolbar with `iNumButtons` buttons from `lpButtons` (`TBBUTTON`s of
/// `uStructSize` bytes). The bitmaps are not kept.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn CreateToolbarEx(
    parent: usize,
    style: u32,
    id: usize,
    n_bitmaps: i32,
    _bm_inst: usize,
    _bm_id: usize,
    buttons: usize,
    n_buttons: i32,
    _dx_button: i32,
    _dy_button: i32,
    _dx_bitmap: i32,
    _dy_bitmap: i32,
    struct_size: u32,
) -> usize {
    register_all();
    let class = wz("ToolbarWindow32");
    // SAFETY: plain calls with a NUL-terminated class name.
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            core::ptr::null(),
            style,
            0,
            0,
            0,
            0,
            parent,
            id,
            GetModuleHandleW(core::ptr::null()),
            0,
        );
        if h == 0 {
            return 0;
        }
        SendMessageW(h, 0x41E, struct_size as usize, 0);
        if n_bitmaps > 0 {
            SendMessageW(h, 0x413, n_bitmaps as usize, 0);
        }
        if n_buttons > 0 && buttons != 0 {
            SendMessageW(h, 0x414, n_buttons as usize, buttons as isize);
        }
        h
    }
}
