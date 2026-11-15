//! The font dialog (`ChooseFontW`; Learn, "CHOOSEFONTW structure").
//!
//! Three lists (font, style, size), the effects (strikeout, underline) when asked for, and a
//! sample drawn with the font chosen. The faces come from `EnumFontFamiliesExW`; when the
//! system lists none, a few generic names stand in (the font mapper finds what it can).

use crate::sys::{self, H};
use crate::util::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_comdlg::cderr;
use poly_comdlg::check::{self, Facts, cf};
use poly_comdlg::font::{self, SIZES, STYLES};
use poly_comdlg::template::{Class, Dialog, Item, style::*};

/// Offsets in `CHOOSEFONTW` on x64 (`commdlg.h`).
mod at {
    pub const SIZE: usize = 0;
    pub const OWNER: usize = 8;
    pub const DC: usize = 16;
    pub const LOGFONT: usize = 24;
    pub const POINT_SIZE: usize = 32;
    pub const FLAGS: usize = 36;
    pub const HOOK: usize = 56;
    pub const TEMPLATE: usize = 64;
    pub const INSTANCE: usize = 72;
    pub const STYLE: usize = 80;
    pub const FONT_TYPE: usize = 88;
    pub const SIZE_MIN: usize = 92;
    pub const SIZE_MAX: usize = 96;
}

/// `LOGFONTW` field offsets (`wingdi.h`).
mod lf {
    pub const HEIGHT: usize = 0;
    pub const WEIGHT: usize = 16;
    pub const ITALIC: usize = 20;
    pub const UNDERLINE: usize = 21;
    pub const STRIKE_OUT: usize = 22;
    pub const CHAR_SET: usize = 23;
    pub const PITCH: usize = 27;
    pub const FACE: usize = 28;
    pub const SIZE: usize = 92;
}

/// Control ids (`dlgs.h`): `cmb1`, `cmb2`, `cmb3` (the three lists here), `chx1`, `chx2`, `stc5`.
const ID_FONT: i32 = 1136;
const ID_STYLE: i32 = 1137;
const ID_SIZE: i32 = 1138;
const ID_STRIKE: i32 = 1040;
const ID_UNDERLINE: i32 = 1041;
const ID_SAMPLE: i32 = 1092;

/// `TRUETYPE_FONTTYPE`, `SCREEN_FONTTYPE`, `REGULAR_FONTTYPE`, `BOLD_FONTTYPE`, `ITALIC_FONTTYPE`.
const TRUETYPE_FONTTYPE: u32 = 0x4;
const SCREEN_FONTTYPE: u16 = 0x2000;
const BOLD_FONTTYPE: u16 = 0x100;
const ITALIC_FONTTYPE: u16 = 0x200;
const REGULAR_FONTTYPE: u16 = 0x400;

struct State {
    cf: *mut u8,
    hook: usize,
    hdlg: H,
    flags: u32,
    faces: Vec<Vec<u16>>,
    sizes: Vec<i32>,
    face: usize,
    style: usize,
    size: usize,
    dpi: i32,
}

pub fn run(c: *mut u8) -> i32 {
    if c.is_null() {
        crate::set_ext_error(cderr::INITIALIZATION);
        return 0;
    }
    let flags: u32 = rd(c, at::FLAGS);
    let facts = Facts {
        struct_size: rd(c, at::SIZE),
        flags,
        has_hook: rd::<usize>(c, at::HOOK) != 0,
        has_template_name: rd::<usize>(c, at::TEMPLATE) != 0,
        has_instance: rd::<usize>(c, at::INSTANCE) != 0,
    };
    let logfont: *mut u8 = rd(c, at::LOGFONT);
    if let Err(code) = check::choose_font(&facts, !logfont.is_null(), rd(c, at::SIZE_MIN), rd(c, at::SIZE_MAX)) {
        crate::set_ext_error(code);
        return 0;
    }
    if flags & cf::ENABLETEMPLATE != 0
        && !has_dialog_resource(rd(c, at::INSTANCE), rd::<usize>(c, at::TEMPLATE) as *const u16)
    {
        crate::set_ext_error(cderr::FINDRESFAILURE);
        return 0;
    }
    let owner: H = rd(c, at::OWNER);
    // SAFETY: a window handle query.
    if owner != 0 && unsafe { sys::IsWindow(owner) } == 0 {
        crate::set_ext_error(cderr::DIALOGFAILURE);
        return 0;
    }
    register_class();
    let mut sizes: Vec<i32> = SIZES.to_vec();
    if flags & cf::LIMITSIZE != 0 {
        let (lo, hi): (i32, i32) = (rd(c, at::SIZE_MIN), rd(c, at::SIZE_MAX));
        sizes.retain(|&s| s >= lo && s <= hi);
    }
    let mut st = Box::new(State {
        cf: c,
        hook: rd(c, at::HOOK),
        hdlg: 0,
        flags,
        faces: Vec::new(),
        sizes,
        face: 0,
        style: 0,
        size: 0,
        dpi: 96,
    });
    let dc: H = rd(c, at::DC);
    st.dpi = screen_dpi(dc);
    st.faces = list_faces(dc, flags);
    let bytes = build_template(flags).build();
    let param = &mut *st as *mut State as isize;
    let r = if flags & cf::ENABLETEMPLATE != 0 {
        let (inst, name): (usize, usize) = (rd(c, at::INSTANCE), rd(c, at::TEMPLATE));
        // SAFETY: the program's dialog resource; our procedure.
        let r = unsafe { sys::DialogBoxParamW(inst, name as *const u16, owner, proc_, param) };
        if r == -1 { Err(cderr::DIALOGFAILURE) } else { Ok(r) }
    } else {
        run_modal(&bytes, owner, proc_, param)
    };
    match r {
        Ok(1) => {
            crate::set_ext_error(0);
            1
        }
        Ok(_) => {
            crate::set_ext_error(0);
            0
        }
        Err(code) => {
            crate::set_ext_error(code);
            0
        }
    }
}

fn screen_dpi(dc: H) -> i32 {
    // SAFETY: a DC query; the desktop's when none is given.
    unsafe {
        let (d, own) = if dc != 0 { (dc, false) } else { (sys::GetDC(0), true) };
        let v = if d != 0 { sys::GetDeviceCaps(d, 90) } else { 0 };
        if own && d != 0 {
            sys::ReleaseDC(0, d);
        }
        if v > 0 { v } else { 96 }
    }
}

/// Faces collected by the `EnumFontFamiliesExW` callback.
struct Collect {
    flags: u32,
    faces: Vec<Vec<u16>>,
}

unsafe extern "system" fn enum_proc(elf: *const u8, _tm: *const u8, font_type: u32, lparam: isize) -> i32 {
    // SAFETY: `lparam` is the `Collect` of `list_faces`.
    let c = unsafe { &mut *(lparam as *mut Collect) };
    let name = read_wstr(
        // SAFETY: an ENUMLOGFONTEXW starts with a LOGFONTW.
        unsafe { elf.add(lf::FACE) } as *const u16,
        32,
    );
    if name.is_empty() || name[0] == b'@' as u16 {
        return 1;
    }
    let pitch: u8 = rd(elf, lf::PITCH);
    if c.flags & cf::TTONLY != 0 && font_type & TRUETYPE_FONTTYPE == 0 {
        return 1;
    }
    if c.flags & cf::FIXEDPITCHONLY != 0 && pitch & 3 != 1 {
        return 1;
    }
    if !c.faces.contains(&name) {
        c.faces.push(name);
    }
    1
}

fn list_faces(dc: H, flags: u32) -> Vec<Vec<u16>> {
    let mut c = Collect { flags, faces: Vec::new() };
    let mut want = [0u8; lf::SIZE];
    // DEFAULT_CHARSET: every script.
    wr(want.as_mut_ptr(), lf::CHAR_SET, 1u8);
    // SAFETY: a LOGFONTW, our callback and our state.
    unsafe {
        let (d, own) = if dc != 0 { (dc, false) } else { (sys::GetDC(0), true) };
        if d != 0 {
            sys::EnumFontFamiliesExW(d, want.as_ptr(), enum_proc, &mut c as *mut Collect as isize, 0);
            if own {
                sys::ReleaseDC(0, d);
            }
        }
    }
    c.faces.sort_by(|a, b| {
        let la: Vec<u16> = a.iter().map(|&x| poly_comdlg::filter::lower(x)).collect();
        let lb: Vec<u16> = b.iter().map(|&x| poly_comdlg::filter::lower(x)).collect();
        la.cmp(&lb)
    });
    if c.faces.is_empty() {
        for n in ["MS Shell Dlg", "Arial", "Courier New", "Times New Roman"] {
            c.faces.push(n.encode_utf16().collect());
        }
    }
    c.faces
}

fn build_template(flags: u32) -> Dialog<'static> {
    let child = WS_CHILD | WS_VISIBLE;
    let list = child | WS_BORDER | WS_VSCROLL | WS_TABSTOP | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT;
    let mut items = alloc::vec![
        Item {
            class: Class::Static,
            text: "&Font:",
            id: 0xFFFF,
            style: child | SS_LEFT,
            ex_style: 0,
            x: 6,
            y: 6,
            cx: 100,
            cy: 9
        },
        Item {
            class: Class::ListBox,
            text: "",
            id: ID_FONT as u16,
            style: list,
            ex_style: 0,
            x: 6,
            y: 17,
            cx: 100,
            cy: 80
        },
        Item {
            class: Class::Static,
            text: "Font st&yle:",
            id: 0xFFFF,
            style: child | SS_LEFT,
            ex_style: 0,
            x: 112,
            y: 6,
            cx: 70,
            cy: 9
        },
        Item {
            class: Class::ListBox,
            text: "",
            id: ID_STYLE as u16,
            style: list,
            ex_style: 0,
            x: 112,
            y: 17,
            cx: 70,
            cy: 80
        },
        Item {
            class: Class::Static,
            text: "&Size:",
            id: 0xFFFF,
            style: child | SS_LEFT,
            ex_style: 0,
            x: 188,
            y: 6,
            cx: 40,
            cy: 9
        },
        Item {
            class: Class::ListBox,
            text: "",
            id: ID_SIZE as u16,
            style: list,
            ex_style: 0,
            x: 188,
            y: 17,
            cx: 40,
            cy: 80
        },
        Item {
            class: Class::Button,
            text: "OK",
            id: IDOK as u16,
            style: child | WS_TABSTOP | BS_DEFPUSHBUTTON,
            ex_style: 0,
            x: 236,
            y: 17,
            cx: 48,
            cy: 14
        },
        Item {
            class: Class::Button,
            text: "Cancel",
            id: IDCANCEL as u16,
            style: child | WS_TABSTOP | BS_PUSHBUTTON,
            ex_style: 0,
            x: 236,
            y: 35,
            cx: 48,
            cy: 14
        },
        Item {
            class: Class::Button,
            text: "Sample",
            id: 0xFFFF,
            style: child | BS_GROUPBOX,
            ex_style: 0,
            x: 112,
            y: 104,
            cx: 116,
            cy: 46
        },
        Item {
            class: Class::Name("PolyFontSample"),
            text: "",
            id: ID_SAMPLE as u16,
            style: child,
            ex_style: 0,
            x: 118,
            y: 116,
            cx: 104,
            cy: 28
        },
    ];
    if flags & cf::EFFECTS != 0 {
        items.push(Item {
            class: Class::Button,
            text: "Effects",
            id: 0xFFFF,
            style: child | BS_GROUPBOX,
            ex_style: 0,
            x: 6,
            y: 104,
            cx: 100,
            cy: 46,
        });
        items.push(Item {
            class: Class::Button,
            text: "Stri&keout",
            id: ID_STRIKE as u16,
            style: child | WS_TABSTOP | BS_AUTOCHECKBOX,
            ex_style: 0,
            x: 12,
            y: 118,
            cx: 90,
            cy: 10,
        });
        items.push(Item {
            class: Class::Button,
            text: "&Underline",
            id: ID_UNDERLINE as u16,
            style: child | WS_TABSTOP | BS_AUTOCHECKBOX,
            ex_style: 0,
            x: 12,
            y: 132,
            cx: 90,
            cy: 10,
        });
    }
    Dialog {
        style: WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_CENTER | DS_3DLOOK,
        ex_style: 0,
        x: 0,
        y: 0,
        cx: 290,
        cy: 158,
        title: "Font",
        font: Some((8, "MS Shell Dlg")),
        items,
    }
}

static REGISTERED: AtomicBool = AtomicBool::new(false);

fn register_class() {
    if REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    let n = sys::wz("PolyFontSample");
    let mut wc = [0u8; 72];
    wr(wc.as_mut_ptr(), 8, sample_proc as sys::WndProc as usize);
    wr(wc.as_mut_ptr(), 24, dll_instance());
    // SAFETY: the system arrow cursor.
    wr(wc.as_mut_ptr(), 40, unsafe { sys::LoadCursorW(0, 32512 as *const u16) });
    wr(wc.as_mut_ptr(), 64, n.as_ptr() as usize);
    // SAFETY: a WNDCLASSW (72 bytes on x64) whose name outlives the call.
    unsafe { sys::RegisterClassW(wc.as_ptr()) };
}

fn state<'a>(hdlg: H) -> Option<&'a mut State> {
    // SAFETY: the pointer `WM_INITDIALOG` stored, valid while the dialog lives.
    let p = unsafe { sys::GetWindowLongPtrW(hdlg, GWLP_USERDATA) } as *mut State;
    // SAFETY: see above.
    unsafe { p.as_mut() }
}

unsafe extern "system" fn proc_(hdlg: H, msg: u32, w: usize, l: isize) -> isize {
    if msg == WM_INITDIALOG {
        // SAFETY: `l` is the `State` `run` made.
        unsafe { sys::SetWindowLongPtrW(hdlg, GWLP_USERDATA, l) };
        let Some(st) = state(hdlg) else { return 0 };
        st.hdlg = hdlg;
        init(st);
        if st.hook != 0 {
            call_hook(st.hook, hdlg, WM_INITDIALOG, w, st.cf as isize);
        }
        return 1;
    }
    let Some(st) = state(hdlg) else { return 0 };
    if st.hook != 0 {
        let r = call_hook(st.hook, hdlg, msg, w, l);
        if r != 0 {
            return r as isize;
        }
    }
    if msg == WM_COMMAND {
        command(st, loword(w) as i32, hiword(w));
        return 1;
    }
    0
}

fn init(st: &mut State) {
    let hdlg = st.hdlg;
    for f in &st.faces {
        lb_add(hdlg, ID_FONT, f, 0);
    }
    for s in &STYLES {
        let t: Vec<u16> = s.name.encode_utf16().collect();
        lb_add(hdlg, ID_STYLE, &t, 0);
    }
    for &s in &st.sizes {
        let mut t = Vec::new();
        for ch in alloc::format!("{s}").encode_utf16() {
            t.push(ch);
        }
        lb_add(hdlg, ID_SIZE, &t, s as isize);
    }
    let logfont: *mut u8 = rd(st.cf, at::LOGFONT);
    st.face = 0;
    st.style = 0;
    st.size = st.sizes.iter().rposition(|&s| s <= 10).unwrap_or(0);
    if st.flags & cf::INITTOLOGFONTSTRUCT != 0 && !logfont.is_null() {
        let name = read_wstr(
            // SAFETY: a LOGFONTW's face name.
            unsafe { logfont.add(lf::FACE) } as *const u16,
            32,
        );
        let lower = |v: &[u16]| v.iter().map(|&c| poly_comdlg::filter::lower(c)).collect::<Vec<u16>>();
        if let Some(i) = st.faces.iter().position(|f| lower(f) == lower(&name)) {
            st.face = i;
        }
        let weight: i32 = rd(logfont, lf::WEIGHT);
        let italic: u8 = rd(logfont, lf::ITALIC);
        st.style = font::style_index(if weight == 0 { 400 } else { weight }, italic != 0);
        let height: i32 = rd(logfont, lf::HEIGHT);
        if height != 0 {
            let pt = font::points_for_height(height, st.dpi);
            st.size = st.sizes.iter().rposition(|&s| s <= pt).unwrap_or(0);
        }
        // SAFETY: plain check boxes.
        unsafe {
            if rd::<u8>(logfont, lf::UNDERLINE) != 0 {
                sys::CheckDlgButton(hdlg, ID_UNDERLINE, BST_CHECKED);
            }
            if rd::<u8>(logfont, lf::STRIKE_OUT) != 0 {
                sys::CheckDlgButton(hdlg, ID_STRIKE, BST_CHECKED);
            }
        }
    }
    lb_sel(hdlg, ID_FONT, st.face);
    lb_sel(hdlg, ID_STYLE, st.style);
    lb_sel(hdlg, ID_SIZE, st.size);
    redraw_sample(st);
}

fn redraw_sample(st: &State) {
    // SAFETY: a window of the dialog.
    unsafe {
        let s = sys::GetDlgItem(st.hdlg, ID_SAMPLE);
        if s != 0 {
            sys::InvalidateRect(s, core::ptr::null(), 1);
        }
    }
}

fn command(st: &mut State, id: i32, code: u32) {
    let hdlg = st.hdlg;
    match id {
        IDOK => {
            commit(st);
            crate::set_ext_error(0);
            // SAFETY: the dialog.
            unsafe { sys::EndDialog(hdlg, 1) };
        }
        IDCANCEL => {
            crate::set_ext_error(0);
            // SAFETY: the dialog.
            unsafe { sys::EndDialog(hdlg, 0) };
        }
        ID_FONT | ID_STYLE | ID_SIZE if code == LBN_SELCHANGE => {
            if let Some(i) = lb_cur(hdlg, id) {
                match id {
                    ID_FONT => st.face = i,
                    ID_STYLE => st.style = i,
                    _ => st.size = i,
                }
            }
            redraw_sample(st);
        }
        ID_STRIKE | ID_UNDERLINE => redraw_sample(st),
        _ => {}
    }
}

/// The font as the dialog stands: a `LOGFONTW` of 92 bytes.
fn logfont_of(st: &State, base: Option<*const u8>) -> [u8; lf::SIZE] {
    let mut out = [0u8; lf::SIZE];
    if let Some(b) = base {
        // SAFETY: the program's LOGFONTW of 92 bytes.
        unsafe { core::ptr::copy_nonoverlapping(b, out.as_mut_ptr(), lf::SIZE) };
    }
    let pt = st.sizes.get(st.size).copied().unwrap_or(10);
    let style = STYLES[st.style.min(STYLES.len() - 1)];
    wr(out.as_mut_ptr(), lf::HEIGHT, font::height_for_points(pt, st.dpi));
    wr(out.as_mut_ptr(), lf::WEIGHT, style.weight);
    wr(out.as_mut_ptr(), lf::ITALIC, u8::from(style.italic));
    // SAFETY: plain check box queries.
    let (u, s) =
        unsafe { (sys::IsDlgButtonChecked(st.hdlg, ID_UNDERLINE), sys::IsDlgButtonChecked(st.hdlg, ID_STRIKE)) };
    wr(out.as_mut_ptr(), lf::UNDERLINE, u8::from(u == BST_CHECKED));
    wr(out.as_mut_ptr(), lf::STRIKE_OUT, u8::from(s == BST_CHECKED));
    // The font's name, cut to 31 characters and a NUL.
    let face = st.faces.get(st.face).cloned().unwrap_or_default();
    for i in 0..32 {
        let c = if i < face.len().min(31) { face[i] } else { 0 };
        wr(out.as_mut_ptr(), lf::FACE + 2 * i, c);
    }
    out
}

/// OK: the choice goes into the program's structures.
fn commit(st: &mut State) {
    let c = st.cf;
    let logfont: *mut u8 = rd(c, at::LOGFONT);
    let lfont = logfont_of(st, if logfont.is_null() { None } else { Some(logfont as *const u8) });
    if !logfont.is_null() {
        // SAFETY: the program's LOGFONTW of 92 bytes.
        unsafe { core::ptr::copy_nonoverlapping(lfont.as_ptr(), logfont, lf::SIZE) };
    }
    let pt = st.sizes.get(st.size).copied().unwrap_or(10);
    wr(c, at::POINT_SIZE, pt * 10);
    let style = STYLES[st.style.min(STYLES.len() - 1)];
    let mut ty = SCREEN_FONTTYPE | if style.weight >= 600 { BOLD_FONTTYPE } else { REGULAR_FONTTYPE };
    if style.italic {
        ty |= ITALIC_FONTTYPE;
    }
    wr(c, at::FONT_TYPE, ty);
    let buf: *mut u16 = rd(c, at::STYLE);
    if st.flags & cf::USESTYLE != 0 && !buf.is_null() {
        let name: Vec<u16> = style.name.encode_utf16().collect();
        write_wstr(buf, 64, &name);
    }
}

/// The sample text in the chosen font.
unsafe extern "system" fn sample_proc(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps = [0u8; 72];
            // SAFETY: a PAINTSTRUCT of 72 bytes.
            let hdc = unsafe { sys::BeginPaint(hwnd, ps.as_mut_ptr()) };
            let mut rc = [0i32; 4];
            // SAFETY: a window of ours.
            unsafe { sys::GetClientRect(hwnd, rc.as_mut_ptr()) };
            // SAFETY: the dialog is the parent.
            let st = state(unsafe { sys::GetParent(hwnd) });
            // SAFETY: GDI calls on the paint DC; objects made here are freed here.
            unsafe {
                let back = sys::GetSysColorBrush(15); // COLOR_3DFACE
                sys::FillRect(hdc, rc.as_ptr(), back);
                if let Some(st) = st {
                    let mut lfont = logfont_of(st, None);
                    // The sample at a size that fits the box.
                    let h: i32 = rd(lfont.as_ptr(), lf::HEIGHT);
                    wr(lfont.as_mut_ptr(), lf::HEIGHT, h.max(-(rc[3] - 4).max(8)));
                    let f = sys::CreateFontIndirectW(lfont.as_ptr());
                    let old = sys::SelectObject(hdc, f);
                    sys::SetBkMode(hdc, 1);
                    let text = sys::wz("AaBbYyZz");
                    // DT_CENTER | DT_VCENTER | DT_SINGLELINE
                    sys::DrawTextW(hdc, text.as_ptr(), -1, rc.as_mut_ptr(), 0x25);
                    sys::SelectObject(hdc, old);
                    sys::DeleteObject(f);
                }
                sys::EndPaint(hwnd, ps.as_ptr());
            }
            0
        }
        // SAFETY: the default procedure.
        _ => unsafe { sys::DefWindowProcW(hwnd, msg, w, l) },
    }
}
