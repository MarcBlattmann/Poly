//! `MessageBoxW` and its relatives: a dialog of class `#32770` built in memory (the
//! controls the sheet lists: the buttons first, an icon with id 20 when asked, the
//! text with id 0xFFFF), run by the dialog manager (`dlg.rs`).

use super::ctl::{self, send};
use super::dlg::run_dialog;
use super::{H, SYS, set_last_error, u};
use alloc::vec;
use alloc::vec::Vec;
use poly_win32k::dlg::{DS_MODALFRAME, DlgItem, DlgTemplate, SzOrd};

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateCompatibleDC(hdc: usize) -> usize;
    fn DeleteDC(hdc: usize) -> i32;
    fn SelectObject(hdc: usize, obj: usize) -> usize;
    fn GetStockObject(index: i32) -> usize;
    fn CreateFontW(
        h: i32,
        w: i32,
        e: i32,
        o: i32,
        weight: i32,
        italic: u32,
        u: u32,
        s: u32,
        cs: u32,
        op: u32,
        cp: u32,
        q: u32,
        pf: u32,
        face: *const u16,
    ) -> usize;
    fn DeleteObject(h: usize) -> i32;
}

/// The message font: Segoe UI at 9 points (the dialog font of Windows 11).
fn message_font() -> usize {
    let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated face name.
    unsafe { CreateFontW(-12, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, face.as_ptr()) }
}

const MB_TYPEMASK: u32 = 0xF;
const MB_ICONMASK: u32 = 0xF0;
const MB_DEFMASK: u32 = 0xF00;
const MB_HELP: u32 = 0x4000;
const MB_SYSTEMMODAL: u32 = 0x1000;
const MB_TOPMOST: u32 = 0x40000;
const MB_RIGHT: u32 = 0x80000;

const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_GROUP: u32 = 0x0002_0000;
const WS_EX_TOPMOST: u32 = 0x8;
const DEFAULT_GUI_FONT: i32 = 17;

const IDOK: u32 = 1;
const IDCANCEL: u32 = 2;
const IDABORT: u32 = 3;
const IDRETRY: u32 = 4;
const IDIGNORE: u32 = 5;
const IDYES: u32 = 6;
const IDNO: u32 = 7;
const IDHELP: u32 = 9;
const IDTRYAGAIN: u32 = 10;
const IDCONTINUE: u32 = 11;

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// The buttons of a type (`MB_OK` .. `MB_CANCELTRYCONTINUE`): ids and texts. `MB_OK`'s
/// button has the id of Cancel (so that Escape and the close box work).
fn buttons(kind: u32) -> Option<Vec<(u32, &'static str)>> {
    Some(match kind {
        0 => vec![(IDCANCEL, "OK")],
        1 => vec![(IDOK, "OK"), (IDCANCEL, "Cancel")],
        2 => vec![(IDABORT, "&Abort"), (IDRETRY, "&Retry"), (IDIGNORE, "&Ignore")],
        3 => vec![(IDYES, "&Yes"), (IDNO, "&No"), (IDCANCEL, "Cancel")],
        4 => vec![(IDYES, "&Yes"), (IDNO, "&No")],
        5 => vec![(IDRETRY, "&Retry"), (IDCANCEL, "Cancel")],
        6 => vec![(IDCANCEL, "Cancel"), (IDTRYAGAIN, "&Try Again"), (IDCONTINUE, "&Continue")],
        _ => return None,
    })
}

fn text_size(text: &[u16], max_w: i32) -> (i32, i32) {
    if text.is_empty() {
        return (0, 16);
    }
    // SAFETY: gdi32 calls on a memory DC of ours.
    unsafe {
        let dc = CreateCompatibleDC(0);
        if dc == 0 {
            return (max_w.min(200), 16);
        }
        let font = message_font();
        SelectObject(dc, if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) });
        let mut r = [0, 0, max_w, 0];
        // DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX
        super::DrawTextW(dc, text.as_ptr(), text.len() as i32, r.as_mut_ptr(), 0x400 | 0x10 | 0x800);
        DeleteDC(dc);
        if font != 0 {
            DeleteObject(font);
        }
        (r[2] - r[0], (r[3] - r[1]).max(16))
    }
}

static LAST_DEFAULT: super::SpinLock<Vec<(u32, u32)>> = super::SpinLock::new(Vec::new());

unsafe extern "system" fn box_proc(h: H, msg: u32, w: usize, _l: isize) -> isize {
    match msg {
        super::dlg::WM_INITDIALOG => {
            let def = LAST_DEFAULT.with(|v| v.iter().find(|x| x.0 == 0).map_or(0, |x| x.1));
            // The default button gets the focus (and says so with DM_SETDEFID).
            if def != 0 {
                let c = super::dlg::GetDlgItem(h, def as i32);
                if c != 0 {
                    super::SetFocus(c);
                }
            }
            0
        }
        ctl::WM_COMMAND => {
            let id = (w & 0xFFFF) as u32;
            if (1..=11).contains(&id) {
                super::dlg::EndDialog(h, id as isize);
            }
            1
        }
        _ => 0,
    }
}

/// `MessageBoxW(hWnd, lpText, lpCaption, uType)`.
pub fn message_box(owner: H, text: &[u16], caption: &[u16], ty: u32) -> i32 {
    message_box_in(owner, text, caption, ty, 0, 0)
}

/// `MB_USERICON` (`winuser.h`).
const MB_USERICON: u32 = 0x80;

/// The box of `message_box` with the instance of its resources and, for `MB_USERICON`, the
/// number of the icon group in that instance.
pub fn message_box_in(owner: H, text: &[u16], caption: &[u16], ty: u32, inst: usize, user_icon: u16) -> i32 {
    if owner != 0 && super::info(owner).is_none() {
        set_last_error(1400);
        return 0;
    }
    let Some(btns) = buttons(ty & MB_TYPEMASK) else {
        set_last_error(87);
        return 0;
    };
    let mut btns = btns;
    if ty & MB_HELP != 0 {
        btns.push((IDHELP, "Help"));
    }
    let icon = match ty & MB_ICONMASK {
        0x10 => Some(32513u16),
        0x20 => Some(32514),
        0x30 => Some(32515),
        0x40 => Some(32516),
        // MB_USERICON: the icon static is there with or without an icon to put into it.
        MB_USERICON => Some(user_icon),
        _ => None,
    };
    let def_index = (((ty & MB_DEFMASK) >> 8) as usize).min(btns.len() - 1);
    // Layout (pixels): 12 px margins, a 32 px icon, text up to 360 px wide, buttons 75 x 23.
    let screen = u::screen(&SYS);
    let max_text = (screen.width * 2 / 5).clamp(200, 520) - if icon.is_some() { 44 } else { 0 };
    let (tw, th) = text_size(text, max_text);
    let (bw, bh, gap) = (75, 23, 6);
    let row = btns.len() as i32 * bw + (btns.len() as i32 - 1) * gap;
    let icon_w = if icon.is_some() { 32 + 12 } else { 0 };
    let content_w = (icon_w + tw).max(row).max(120);
    let client_w = content_w + 24;
    let top_h = th.max(if icon.is_some() { 32 } else { 0 });
    let client_h = 12 + top_h + 18 + bh + 12;
    let mut items = Vec::new();
    let mut bx = (client_w - row) / 2;
    let by = 12 + top_h + 18;
    for (n, (id, label)) in btns.iter().enumerate() {
        let mut style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | if n == def_index { 1 } else { 0 };
        if n == 0 {
            style |= WS_GROUP;
        }
        items.push(DlgItem {
            style,
            ex_style: 0,
            help_id: 0,
            x: bx as i16,
            y: by as i16,
            cx: bw as i16,
            cy: bh as i16,
            id: *id,
            class: SzOrd::Ord(0x80),
            title: SzOrd::Str(utf16(label)),
            data: Vec::new(),
        });
        bx += bw + gap;
    }
    if let Some(ord) = icon {
        items.push(DlgItem {
            style: WS_CHILD | WS_VISIBLE | ctl::SS_ICON,
            ex_style: 0,
            help_id: 0,
            x: 12,
            y: 12,
            cx: 32,
            cy: 32,
            id: 20,
            class: SzOrd::Ord(0x82),
            title: SzOrd::Ord(ord),
            data: Vec::new(),
        });
    }
    items.push(DlgItem {
        style: WS_CHILD | WS_VISIBLE | ctl::SS_LEFT | ctl::SS_NOPREFIX,
        ex_style: 0,
        help_id: 0,
        x: (12 + icon_w) as i16,
        y: 12,
        cx: tw.max(20) as i16,
        cy: th as i16,
        id: 0xFFFF,
        class: SzOrd::Ord(0x82),
        title: SzOrd::Str(text.to_vec()),
        data: Vec::new(),
    });
    let mut ex = 0;
    if ty & (MB_SYSTEMMODAL | MB_TOPMOST) != 0 {
        ex |= WS_EX_TOPMOST;
    }
    if ty & MB_RIGHT != 0 {
        ex |= 0x1000;
    }
    let tpl = DlgTemplate {
        ex: false,
        style: WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE | DS_MODALFRAME,
        ex_style: ex,
        help_id: 0,
        x: 0,
        y: 0,
        cx: client_w as i16,
        cy: client_h as i16,
        menu: SzOrd::None,
        class: SzOrd::None,
        title: caption.to_vec(),
        font: Some(poly_win32k::dlg::DlgFont {
            points: 9,
            weight: 400,
            italic: false,
            charset: 1,
            face: utf16("Segoe UI"),
        }),
        items,
    };
    // The default button's id for the dialog procedure (one box runs at a time per thread).
    LAST_DEFAULT.with(|v| {
        v.retain(|x| x.0 != 0);
        v.push((0, btns[def_index].0));
    });
    let r = run_dialog(&tpl, inst, owner, box_proc as usize, 0, true);
    if r <= 0 {
        return 0;
    }
    let r = r as i32;
    // MB_OK's result is always OK (`MessageBoxIndirectW.yaml` `result`): its button has the id of Cancel.
    if ty & MB_TYPEMASK == 0 { IDOK as i32 } else { r }
}

fn caption_of(p: *const u16) -> Vec<u16> {
    if p.is_null() {
        utf16("Error")
    } else {
        // SAFETY: a NUL-terminated string.
        unsafe { super::wide(p, -1) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MessageBoxW(owner: H, text: *const u16, caption: *const u16, ty: u32) -> i32 {
    // SAFETY: NUL-terminated strings or NULL.
    let t = if text.is_null() { Vec::new() } else { unsafe { super::wide(text, -1) } };
    message_box(owner, &t, &caption_of(caption), ty)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MessageBoxExW(
    owner: H,
    text: *const u16,
    caption: *const u16,
    ty: u32,
    _lang: u16,
) -> i32 {
    // SAFETY: as MessageBoxW.
    unsafe { MessageBoxW(owner, text, caption, ty) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MessageBoxA(owner: H, text: *const u8, caption: *const u8, ty: u32) -> i32 {
    // SAFETY: NUL-terminated strings or NULL.
    let t = unsafe { super::ansi::wide_of_a(text) };
    let c = if caption.is_null() { utf16("Error") } else { unsafe { super::ansi::wide_of_a(caption) } };
    message_box(owner, &t, &c, ty)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MessageBoxExA(owner: H, text: *const u8, caption: *const u8, ty: u32, _lang: u16) -> i32 {
    // SAFETY: as MessageBoxA.
    unsafe { MessageBoxA(owner, text, caption, ty) }
}

/// `MSGBOXPARAMSW` (`winuser.h`: 80 bytes on x64, 40 in 32 bits). The size is not checked.
#[repr(C)]
struct MsgBoxParams {
    size: u32,
    owner: usize,
    instance: usize,
    text: usize,
    caption: usize,
    style: u32,
    icon: usize,
    help_id: usize,
    callback: usize,
    language: u32,
}

/// A text or caption of a `MSGBOXPARAMS`: a string, or a string id of the instance (`None` for NULL).
unsafe fn param_text(inst: usize, p: usize, ansi: bool) -> Option<Vec<u16>> {
    if p == 0 {
        return None;
    }
    if p < 0x1_0000 {
        let mut buf = [0u16; 1024];
        // SAFETY: our buffer; the instance may be anything (LoadStringW says so).
        let n = if inst != 0 { unsafe { super::strings::LoadStringW(inst as *mut u8, p as u32, buf.as_mut_ptr(), 1024) } } else { 0 };
        return Some(buf[..n.max(0) as usize].to_vec());
    }
    if ansi {
        // SAFETY: a NUL-terminated string.
        Some(unsafe { super::ansi::wide_of_a(p as *const u8) })
    } else {
        // SAFETY: a NUL-terminated string.
        Some(unsafe { super::wide(p as *const u16, -1) })
    }
}

unsafe fn indirect(p: *const MsgBoxParams, ansi: bool) -> i32 {
    if p.is_null() {
        set_last_error(998);
        return 0;
    }
    // SAFETY: the caller's MSGBOXPARAMS.
    let m = unsafe { core::ptr::read_unaligned(p) };
    // SAFETY: strings or ids as above.
    let text = unsafe { param_text(m.instance, m.text, ansi) }.unwrap_or_default();
    let caption = unsafe { param_text(m.instance, m.caption, ansi) }.unwrap_or_else(|| utf16("Error"));
    let icon = if m.icon != 0 && m.icon < 0x1_0000 { m.icon as u16 } else { 0 };
    message_box_in(m.owner, &text, &caption, m.style, m.instance, icon)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MessageBoxIndirectW(p: *const u8) -> i32 {
    // SAFETY: the caller's MSGBOXPARAMSW.
    unsafe { indirect(p as *const MsgBoxParams, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MessageBoxIndirectA(p: *const u8) -> i32 {
    // SAFETY: the caller's MSGBOXPARAMSA.
    unsafe { indirect(p as *const MsgBoxParams, true) }
}
