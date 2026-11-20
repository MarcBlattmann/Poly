//! The window procedures of the system classes `Button`, `Static`, `Edit`,
//! `ListBox`, `ComboBox` and `ScrollBar` (`poly_win32k::dlg::sysproc`): what a
//! dialog template's controls are. The behaviour is that of the spec sheets
//! (`CheckDlgButton`, `IsDlgButtonChecked`, `SendDlgItemMessageW`,
//! `IsDialogMessageW`, ...) and Microsoft Learn's "Button Messages", "Edit
//! Control Messages", "List Box Messages", "Static Control Messages" and
//! "Dialog Box Messages"; constants are `winuser.h`'s.
//!
//! Per-control state lives here, in tables keyed by the window handle, and
//! goes with `WM_NCDESTROY`. No table is locked while a window procedure runs.

use super::{H, SYS, SpinLock, call, info, set_last_error, u};
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use poly_win32k::dlg::{
    self, DLGC_BUTTON, DLGC_DEFPUSHBUTTON, DLGC_HASSETSEL, DLGC_RADIOBUTTON, DLGC_STATIC, DLGC_UNDEFPUSHBUTTON,
    DLGC_WANTARROWS, DLGC_WANTCHARS,
};
use poly_win32k::proto::{Reply, Request};

// ---- messages (winuser.h)

pub const WM_DESTROY: u32 = 0x0002;
pub const WM_ENABLE: u32 = 0x000A;
pub const WM_SETTEXT: u32 = 0x000C;
pub const WM_GETTEXT: u32 = 0x000D;
pub const WM_GETTEXTLENGTH: u32 = 0x000E;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_GETFONT: u32 = 0x0031;
pub const WM_NCDESTROY: u32 = 0x0082;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_NCPAINT: u32 = 0x0085;
pub const WM_GETDLGCODE: u32 = 0x0087;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_CTLCOLOREDIT: u32 = 0x0133;
pub const WM_CTLCOLORLISTBOX: u32 = 0x0134;
pub const WM_CTLCOLORBTN: u32 = 0x0135;
pub const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_GETICON: u32 = 0x007F;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_CAPTURECHANGED: u32 = 0x0215;
pub const WM_CLEAR: u32 = 0x0303;
pub const WM_CUT: u32 = 0x0300;
pub const WM_COPY: u32 = 0x0301;
pub const WM_PASTE: u32 = 0x0302;
pub const WM_NCCREATE: u32 = 0x0081;

pub const BM_GETCHECK: u32 = 0x00F0;
pub const BM_SETCHECK: u32 = 0x00F1;
pub const BM_GETSTATE: u32 = 0x00F2;
pub const BM_SETSTATE: u32 = 0x00F3;
pub const BM_SETSTYLE: u32 = 0x00F4;
pub const BM_CLICK: u32 = 0x00F5;
pub const BM_GETIMAGE: u32 = 0x00F6;
pub const BM_SETIMAGE: u32 = 0x00F7;
pub const BM_SETDONTCLICK: u32 = 0x00F8;

pub const BN_CLICKED: u32 = 0;
pub const BN_SETFOCUS: u32 = 6;
pub const BN_KILLFOCUS: u32 = 7;
pub const BS_NOTIFY: u32 = 0x4000;

pub const EN_SETFOCUS: u32 = 0x0100;
pub const EN_KILLFOCUS: u32 = 0x0200;
pub const EN_CHANGE: u32 = 0x0300;
pub const EN_UPDATE: u32 = 0x0400;

pub const EM_GETSEL: u32 = 0x00B0;
pub const EM_SETSEL: u32 = 0x00B1;
pub const EM_GETRECT: u32 = 0x00B2;
pub const EM_SCROLLCARET: u32 = 0x00B7;
pub const EM_GETMODIFY: u32 = 0x00B8;
pub const EM_SETMODIFY: u32 = 0x00B9;
pub const EM_GETLINECOUNT: u32 = 0x00BA;
pub const EM_LINEINDEX: u32 = 0x00BB;
pub const EM_LINELENGTH: u32 = 0x00C1;
pub const EM_REPLACESEL: u32 = 0x00C2;
pub const EM_GETLINE: u32 = 0x00C4;
pub const EM_LIMITTEXT: u32 = 0x00C5;
pub const EM_CANUNDO: u32 = 0x00C6;
pub const EM_UNDO: u32 = 0x00C7;
pub const EM_LINEFROMCHAR: u32 = 0x00C9;
pub const EM_SETPASSWORDCHAR: u32 = 0x00CC;
pub const EM_EMPTYUNDOBUFFER: u32 = 0x00CD;
pub const EM_SETREADONLY: u32 = 0x00CF;
pub const EM_GETPASSWORDCHAR: u32 = 0x00D2;
pub const EM_SETMARGINS: u32 = 0x00D3;
pub const EM_GETMARGINS: u32 = 0x00D4;
pub const EM_GETLIMITTEXT: u32 = 0x00D5;
pub const EM_SETLIMITTEXT: u32 = EM_LIMITTEXT;

pub const ES_MULTILINE: u32 = 0x0004;
pub const ES_UPPERCASE: u32 = 0x0008;
pub const ES_LOWERCASE: u32 = 0x0010;
pub const ES_PASSWORD: u32 = 0x0020;
pub const ES_AUTOHSCROLL: u32 = 0x0080;
pub const ES_READONLY: u32 = 0x0800;
pub const ES_WANTRETURN: u32 = 0x1000;
pub const ES_NUMBER: u32 = 0x2000;
pub const ES_CENTER: u32 = 0x0001;
pub const ES_RIGHT: u32 = 0x0002;

pub const LB_ERR: isize = -1;
pub const LB_ADDSTRING: u32 = 0x0180;
pub const LB_INSERTSTRING: u32 = 0x0181;
pub const LB_DELETESTRING: u32 = 0x0182;
pub const LB_SELITEMRANGEEX: u32 = 0x0183;
pub const LB_RESETCONTENT: u32 = 0x0184;
pub const LB_SETSEL: u32 = 0x0185;
pub const LB_SETCURSEL: u32 = 0x0186;
pub const LB_GETSEL: u32 = 0x0187;
pub const LB_GETCURSEL: u32 = 0x0188;
pub const LB_GETTEXT: u32 = 0x0189;
pub const LB_GETTEXTLEN: u32 = 0x018A;
pub const LB_GETCOUNT: u32 = 0x018B;
pub const LB_SELECTSTRING: u32 = 0x018C;
pub const LB_DIR: u32 = 0x018D;
pub const LB_GETTOPINDEX: u32 = 0x018E;
pub const LB_FINDSTRING: u32 = 0x018F;
pub const LB_GETSELCOUNT: u32 = 0x0190;
pub const LB_GETSELITEMS: u32 = 0x0191;
pub const LB_SETTABSTOPS: u32 = 0x0192;
pub const LB_GETHORIZONTALEXTENT: u32 = 0x0193;
pub const LB_SETHORIZONTALEXTENT: u32 = 0x0194;
pub const LB_SETCOLUMNWIDTH: u32 = 0x0195;
pub const LB_SETTOPINDEX: u32 = 0x0197;
pub const LB_GETITEMRECT: u32 = 0x0198;
pub const LB_GETITEMDATA: u32 = 0x0199;
pub const LB_SETITEMDATA: u32 = 0x019A;
pub const LB_SELITEMRANGE: u32 = 0x019B;
pub const LB_SETITEMHEIGHT: u32 = 0x01A0;
pub const LB_GETITEMHEIGHT: u32 = 0x01A1;
pub const LB_FINDSTRINGEXACT: u32 = 0x01A2;
pub const LB_SETLOCALE: u32 = 0x01A5;
pub const LB_GETLOCALE: u32 = 0x01A6;
pub const LB_SETCOUNT: u32 = 0x01A7;
pub const LB_INITSTORAGE: u32 = 0x01A8;
pub const LB_ITEMFROMPOINT: u32 = 0x01A9;
pub const LB_GETCARETINDEX: u32 = 0x019F;
pub const LB_SETCARETINDEX: u32 = 0x019E;
pub const LB_ADDFILE: u32 = 0x0196;
pub const LBN_SELCHANGE: u32 = 1;
pub const LBN_DBLCLK: u32 = 2;
pub const LBN_SETFOCUS: u32 = 4;
pub const LBN_KILLFOCUS: u32 = 5;
pub const LBS_NOTIFY: u32 = 0x0001;
pub const LBS_SORT: u32 = 0x0002;
pub const LBS_MULTIPLESEL: u32 = 0x0008;
pub const LBS_EXTENDEDSEL: u32 = 0x0800;
pub const LBS_OWNERDRAWFIXED: u32 = 0x0010;
pub const LBS_OWNERDRAWVARIABLE: u32 = 0x0020;
pub const LBS_HASSTRINGS: u32 = 0x0040;

pub const STM_SETICON: u32 = 0x0170;
pub const STM_GETICON: u32 = 0x0171;
pub const STM_SETIMAGE: u32 = 0x0172;
pub const STM_GETIMAGE: u32 = 0x0173;
pub const SS_TYPEMASK: u32 = 0x1F;
pub const SS_LEFT: u32 = 0;
pub const SS_CENTER: u32 = 1;
pub const SS_RIGHT: u32 = 2;
pub const SS_ICON: u32 = 3;
pub const SS_BLACKRECT: u32 = 4;
pub const SS_GRAYRECT: u32 = 5;
pub const SS_WHITERECT: u32 = 6;
pub const SS_BLACKFRAME: u32 = 7;
pub const SS_GRAYFRAME: u32 = 8;
pub const SS_WHITEFRAME: u32 = 9;
pub const SS_SIMPLE: u32 = 0x0B;
pub const SS_LEFTNOWORDWRAP: u32 = 0x0C;
pub const SS_BITMAP: u32 = 0x0E;
pub const SS_ETCHEDHORZ: u32 = 0x10;
pub const SS_ETCHEDVERT: u32 = 0x11;
pub const SS_ETCHEDFRAME: u32 = 0x12;
pub const SS_NOPREFIX: u32 = 0x80;

pub const HTTRANSPARENT: isize = -1;
pub const HTCLIENT: isize = 1;

pub const BST_UNCHECKED: u32 = 0;
pub const BST_CHECKED: u32 = 1;
pub const BST_INDETERMINATE: u32 = 2;
pub const BST_PUSHED: u32 = 4;
pub const BST_FOCUS: u32 = 8;

pub const BS_PUSHBUTTON: u32 = 0;
pub const BS_DEFPUSHBUTTON: u32 = 1;
pub const BS_CHECKBOX: u32 = 2;
pub const BS_AUTOCHECKBOX: u32 = 3;
pub const BS_RADIOBUTTON: u32 = 4;
pub const BS_3STATE: u32 = 5;
pub const BS_AUTO3STATE: u32 = 6;
pub const BS_GROUPBOX: u32 = 7;
pub const BS_USERBUTTON: u32 = 8;
pub const BS_AUTORADIOBUTTON: u32 = 9;
pub const BS_PUSHBOX: u32 = 0xA;
pub const BS_OWNERDRAW: u32 = 0xB;
pub const BS_TYPEMASK: u32 = 0xF;

pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_PRIOR: u32 = 0x21;
pub const VK_NEXT: u32 = 0x22;
pub const VK_END: u32 = 0x23;
pub const VK_HOME: u32 = 0x24;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_DELETE: u32 = 0x2E;

const WS_VISIBLE: u32 = 0x1000_0000;
const WS_DISABLED: u32 = 0x0800_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_GROUP: u32 = 0x0002_0000;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;

// ---- window helpers

pub fn send(h: H, msg: u32, w: usize, l: isize) -> isize {
    u::send(&SYS, h as u64, msg, w as u64, l as u64) as isize
}

pub fn style_of(h: H) -> u32 {
    info(h).map_or(0, |i| i.style)
}

pub fn ex_style_of(h: H) -> u32 {
    info(h).map_or(0, |i| i.ex_style)
}

/// The control id (`GetDlgCtrlID`).
pub fn id_of(h: H) -> u32 {
    info(h).map_or(0, |i| i.id as u32)
}

pub fn parent_of(h: H) -> H {
    info(h).map_or(0, |i| i.parent as H)
}

pub fn text_of(h: H) -> Vec<u16> {
    match call(Request::GetText { hwnd: h as u64 }) {
        Reply::Text { text } => text,
        _ => Vec::new(),
    }
}

pub fn set_text_raw(h: H, text: Vec<u16>) {
    call(Request::SetText { hwnd: h as u32, text });
}

/// The children of `h` in z order (first created first).
pub fn children_of(h: H) -> Vec<H> {
    match call(Request::Relatives { hwnd: h as u32 }) {
        Reply::Relatives { children, .. } => children.into_iter().map(|c| c as H).collect(),
        _ => Vec::new(),
    }
}

/// `WM_COMMAND` to the parent: `code` in the high word of wParam.
pub fn notify_parent(h: H, code: u32) {
    let parent = parent_of(h);
    if parent != 0 {
        send(parent, WM_COMMAND, ((code as usize) << 16) | (id_of(h) as usize & 0xFFFF), h as isize);
    }
}

fn def(h: H, msg: u32, w: usize, l: isize) -> isize {
    u::def_window_proc(&SYS, h as u64, msg, w as u64, l as u64) as isize
}

fn invalidate(h: H) {
    // SAFETY: a window handle and a NULL rectangle.
    unsafe { super::InvalidateRect(h, core::ptr::null(), 1) };
}

fn key_down(vk: u32) -> bool {
    super::GetKeyState(vk as i32) < 0
}

pub fn lo(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

/// The entry for the system procedure `id`.
pub fn builtin(id: u64, h: H, msg: u32, w: usize, l: isize) -> isize {
    match id {
        dlg::sysproc::BUTTON => button(h, msg, w, l),
        dlg::sysproc::STATIC => statik(h, msg, w, l),
        dlg::sysproc::EDIT => edit(h, msg, w, l),
        dlg::sysproc::LISTBOX | dlg::sysproc::COMBOLBOX => listbox(h, msg, w, l),
        dlg::sysproc::DIALOG => super::dlg::dialog_proc(h, msg, w, l),
        dlg::sysproc::COMBOBOX => combobox(h, msg, w, l),
        dlg::sysproc::SCROLLBAR => super::scroll::control_message(h, msg, w, l).unwrap_or_else(|| other(h, msg, w, l)),
        dlg::sysproc::MENU => def(h, msg, w, l),
        _ => def(h, msg, w, l),
    }
}

/// Fonts: the control's own, the dialog's, or the system GUI font.
static FONTS: SpinLock<BTreeMap<u32, usize>> = SpinLock::new(BTreeMap::new());

fn font_of(h: H) -> usize {
    FONTS.with(|m| m.get(&(h as u32)).copied().unwrap_or(0))
}

/// What every control does with the font messages; true when `msg` was one.
fn font_msg(h: H, msg: u32, w: usize, l: isize, out: &mut isize) -> bool {
    match msg {
        WM_SETFONT => {
            FONTS.with(|m| m.insert(h as u32, w));
            if l & 0xFFFF != 0 {
                invalidate(h);
            }
            *out = 0;
            true
        }
        WM_GETFONT => {
            *out = font_of(h) as isize;
            true
        }
        _ => false,
    }
}

fn forget(h: H) {
    FONTS.with(|m| m.remove(&(h as u32)));
}

// ---- other classes: text and default behaviour

fn other(h: H, msg: u32, w: usize, l: isize) -> isize {
    let mut out = 0;
    if font_msg(h, msg, w, l, &mut out) {
        return out;
    }
    if msg == WM_NCDESTROY {
        forget(h);
    }
    def(h, msg, w, l)
}

/// A combo box: `CBS_HASSTRINGS` is on unless the box draws its items itself, and the window
/// text it was made with is not kept (its text is the edit field's: empty at first;
/// `InitCommonControlsEx.yaml` `classes_5_82`).
const WM_CREATE: u32 = 0x0001;

fn combobox(h: H, msg: u32, w: usize, l: isize) -> isize {
    let r = other(h, msg, w, l);
    if msg == WM_NCCREATE && r != 0 {
        const CBS_OWNERDRAW: u32 = 0x30;
        const CBS_HASSTRINGS: u32 = 0x200;
        let style = style_of(h);
        if style & CBS_OWNERDRAW == 0 {
            call(Request::SetStyle { hwnd: h as u32, which: -16, value: style | CBS_HASSTRINGS });
        }
        let empty = [0u16];
        def(h, WM_SETTEXT, 0, empty.as_ptr() as isize);
    }
    r
}

// ---- Button

#[derive(Clone, Copy, Default)]
struct Btn {
    check: u32,
    pushed: bool,
    focus: bool,
    /// The focus came by the mouse: a radio button is not clicked by it.
    dont_click: bool,
    image: usize,
}

static BTNS: SpinLock<BTreeMap<u32, Btn>> = SpinLock::new(BTreeMap::new());

fn btn<R>(h: H, f: impl FnOnce(&mut Btn) -> R) -> R {
    BTNS.with(|m| f(m.entry(h as u32).or_default()))
}

fn btn_kind(style: u32) -> u32 {
    style & BS_TYPEMASK
}

/// `WM_GETDLGCODE` of a button (`SendDlgItemMessageW.yaml` `results`: check box
/// 0x2000, default push button 0x2010, push button 0x2020, radio 0x2040).
pub fn button_dlgcode(style: u32) -> u32 {
    match btn_kind(style) {
        BS_DEFPUSHBUTTON => DLGC_BUTTON | DLGC_DEFPUSHBUTTON,
        BS_PUSHBUTTON | BS_USERBUTTON | BS_PUSHBOX | BS_OWNERDRAW => DLGC_BUTTON | DLGC_UNDEFPUSHBUTTON,
        BS_RADIOBUTTON | BS_AUTORADIOBUTTON => DLGC_BUTTON | DLGC_RADIOBUTTON,
        _ => DLGC_BUTTON,
    }
}

/// Whether the button can be checked (`BM_SETCHECK` does something).
fn checkable(kind: u32) -> bool {
    matches!(kind, BS_CHECKBOX | BS_AUTOCHECKBOX | BS_RADIOBUTTON | BS_AUTORADIOBUTTON | BS_3STATE | BS_AUTO3STATE)
}

fn is_radio(kind: u32) -> bool {
    matches!(kind, BS_RADIOBUTTON | BS_AUTORADIOBUTTON)
}

fn is_tristate(kind: u32) -> bool {
    matches!(kind, BS_3STATE | BS_AUTO3STATE)
}

fn set_check(h: H, w: usize) {
    let kind = btn_kind(style_of(h));
    if !checkable(kind) {
        return;
    }
    let v = lo(w) as u32 | (w as u32 & 0xFFFF_0000);
    let c = if v == 0 {
        0
    } else if v == BST_INDETERMINATE && is_tristate(kind) {
        2
    } else {
        1
    };
    btn(h, |b| b.check = c);
    invalidate(h);
}

fn check_of(h: H) -> u32 {
    btn(h, |b| b.check)
}

/// The radio buttons of the group `h` belongs to (the controls from the last
/// one with `WS_GROUP` at or before it up to the next such).
fn radio_group(h: H) -> Vec<H> {
    let parent = parent_of(h);
    let sibs = children_of(parent);
    let Some(pos) = sibs.iter().position(|&c| c == h) else { return vec![h] };
    let mut start = pos;
    while start > 0 && style_of(sibs[start]) & WS_GROUP == 0 {
        start -= 1;
    }
    let mut end = pos + 1;
    while end < sibs.len() && style_of(sibs[end]) & WS_GROUP == 0 {
        end += 1;
    }
    sibs[start..end].to_vec()
}

/// `BM_CLICK`: what a click on the button does, then `BN_CLICKED` to the parent.
pub fn button_click(h: H) {
    let kind = btn_kind(style_of(h));
    match kind {
        BS_AUTOCHECKBOX => {
            let c = btn(h, |b| b.check);
            btn(h, |b| b.check = u32::from(c == 0));
        }
        BS_AUTO3STATE => {
            let c = btn(h, |b| b.check);
            btn(h, |b| b.check = (c + 1) % 3);
        }
        BS_AUTORADIOBUTTON => {
            for g in radio_group(h) {
                if g != h && btn_kind(style_of(g)) == BS_AUTORADIOBUTTON && check_of(g) != 0 {
                    btn(g, |b| b.check = 0);
                    invalidate(g);
                }
            }
            btn(h, |b| b.check = 1);
        }
        _ => {}
    }
    invalidate(h);
    notify_parent(h, BN_CLICKED);
}

fn button(h: H, msg: u32, w: usize, l: isize) -> isize {
    let mut out = 0;
    if font_msg(h, msg, w, l, &mut out) {
        return out;
    }
    let style = style_of(h);
    let kind = btn_kind(style);
    match msg {
        WM_GETDLGCODE => button_dlgcode(style) as isize,
        BM_GETCHECK => check_of(h) as isize,
        BM_SETCHECK => {
            set_check(h, w);
            0
        }
        BM_GETSTATE => {
            let (c, p, f) = btn(h, |b| (b.check, b.pushed, b.focus));
            (c | if p { BST_PUSHED } else { 0 } | if f { BST_FOCUS } else { 0 }) as isize
        }
        BM_SETSTATE => {
            let on = w != 0;
            btn(h, |b| b.pushed = on);
            invalidate(h);
            0
        }
        BM_SETSTYLE => {
            call(Request::SetStyle { hwnd: h as u32, which: -16, value: (style & !0xFFFF) | (w as u32 & 0xFFFF) });
            if l & 0xFFFF != 0 {
                invalidate(h);
            }
            0
        }
        BM_SETDONTCLICK => {
            btn(h, |b| b.dont_click = w != 0);
            0
        }
        BM_CLICK => {
            btn(h, |b| b.pushed = true);
            button_click(h);
            btn(h, |b| b.pushed = false);
            0
        }
        BM_GETIMAGE => btn(h, |b| b.image) as isize,
        BM_SETIMAGE => {
            let old = btn(h, |b| core::mem::replace(&mut b.image, l as usize));
            invalidate(h);
            old as isize
        }
        WM_SETFOCUS => {
            let dont = btn(h, |b| {
                b.focus = true;
                core::mem::take(&mut b.dont_click)
            });
            // A radio button that gets the focus by the keyboard is clicked (IsDialogMessageW.yaml `arrows`).
            if kind == BS_AUTORADIOBUTTON && !dont {
                button_click(h);
            }
            if style & BS_NOTIFY != 0 {
                notify_parent(h, BN_SETFOCUS);
            }
            invalidate(h);
            0
        }
        WM_KILLFOCUS => {
            btn(h, |b| b.focus = false);
            if style & BS_NOTIFY != 0 {
                notify_parent(h, BN_KILLFOCUS);
            }
            invalidate(h);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if style & WS_DISABLED != 0 {
                return 0;
            }
            btn(h, |b| {
                b.dont_click = true;
                b.pushed = true;
            });
            if kind != BS_GROUPBOX {
                super::SetFocus(h);
                super::SetCapture(h);
            }
            invalidate(h);
            0
        }
        WM_LBUTTONUP => {
            let was = btn(h, |b| core::mem::take(&mut b.pushed));
            if was {
                super::ReleaseCapture();
                let x = (l & 0xFFFF) as i16 as i32;
                let y = ((l >> 16) & 0xFFFF) as i16 as i32;
                if let Some(i) = info(h)
                    && x >= 0
                    && y >= 0
                    && x < i.client.width()
                    && y < i.client.height()
                {
                    button_click(h);
                }
            }
            invalidate(h);
            0
        }
        WM_KEYDOWN if w as u32 == VK_SPACE => {
            btn(h, |b| b.pushed = true);
            invalidate(h);
            0
        }
        WM_KEYUP if w as u32 == VK_SPACE => {
            if btn(h, |b| core::mem::take(&mut b.pushed)) {
                button_click(h);
            }
            0
        }
        WM_ENABLE | WM_SETTEXT => {
            let r = def(h, msg, w, l);
            invalidate(h);
            r
        }
        WM_NCHITTEST if kind == BS_GROUPBOX => HTTRANSPARENT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            super::paintctl::paint_button(h);
            0
        }
        WM_NCDESTROY => {
            BTNS.with(|m| m.remove(&(h as u32)));
            forget(h);
            def(h, msg, w, l)
        }
        _ => def(h, msg, w, l),
    }
}

// ---- Static

#[derive(Clone, Copy, Default)]
struct Stat {
    image: usize,
}

static STATS: SpinLock<BTreeMap<u32, Stat>> = SpinLock::new(BTreeMap::new());

fn statik(h: H, msg: u32, w: usize, l: isize) -> isize {
    let mut out = 0;
    if font_msg(h, msg, w, l, &mut out) {
        return out;
    }
    match msg {
        WM_GETDLGCODE => DLGC_STATIC as isize,
        WM_NCHITTEST => HTTRANSPARENT,
        STM_SETICON | STM_SETIMAGE => {
            let old = STATS.with(|m| {
                core::mem::replace(
                    &mut m.entry(h as u32).or_default().image,
                    if msg == STM_SETICON { w } else { l as usize },
                )
            });
            invalidate(h);
            old as isize
        }
        STM_GETICON | STM_GETIMAGE => STATS.with(|m| m.get(&(h as u32)).map_or(0, |s| s.image)) as isize,
        WM_SETTEXT => {
            let r = def(h, msg, w, l);
            invalidate(h);
            r
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            super::paintctl::paint_static(h, STATS.with(|m| m.get(&(h as u32)).map_or(0, |s| s.image)));
            0
        }
        WM_NCDESTROY => {
            STATS.with(|m| m.remove(&(h as u32)));
            forget(h);
            def(h, msg, w, l)
        }
        _ => def(h, msg, w, l),
    }
}

// ---- Edit

#[derive(Clone, Default)]
pub struct Edit {
    pub sel_start: usize,
    pub sel_end: usize,
    pub limit: usize,
    pub modified: bool,
    pub readonly: bool,
    pub password: u16,
    pub scroll: usize,
}

static EDITS: SpinLock<BTreeMap<u32, Edit>> = SpinLock::new(BTreeMap::new());

pub fn edit_state<R>(h: H, f: impl FnOnce(&mut Edit) -> R) -> R {
    EDITS.with(|m| f(m.entry(h as u32).or_insert_with(|| Edit { limit: 0x7FFF_FFFE, ..Edit::default() })))
}

fn edit_changed(h: H, notify: bool) {
    if notify {
        notify_parent(h, EN_UPDATE);
        notify_parent(h, EN_CHANGE);
    }
    invalidate(h);
}

/// Replace the text of the selection with `ins`; the notifications follow.
fn edit_replace(h: H, ins: &[u16]) {
    let style = style_of(h);
    let mut text = text_of(h);
    let (s, e, limit) = edit_state(h, |s| (s.sel_start.min(s.sel_end), s.sel_start.max(s.sel_end), s.limit));
    let (s, e) = (s.min(text.len()), e.min(text.len()));
    let mut ins: Vec<u16> = ins.to_vec();
    if style & ES_NUMBER != 0 {
        ins.retain(|c| (0x30..=0x39).contains(c));
    }
    if style & ES_UPPERCASE != 0 {
        ins.iter_mut().for_each(|c| *c = upcase(*c));
    }
    if style & ES_LOWERCASE != 0 {
        ins.iter_mut().for_each(|c| *c = poly_win32k::dlg::fold(*c));
    }
    let room = limit.saturating_sub(text.len() - (e - s));
    ins.truncate(room);
    text.splice(s..e, ins.iter().copied());
    let caret = s + ins.len();
    set_text_raw(h, text);
    edit_state(h, |st| {
        st.sel_start = caret;
        st.sel_end = caret;
        st.modified = true;
    });
    edit_changed(h, true);
}

fn upcase(c: u16) -> u16 {
    match c {
        0x61..=0x7A => c - 0x20,
        _ => c,
    }
}

fn edit_move(h: H, to: usize, extend: bool) {
    let len = text_of(h).len();
    let to = to.min(len);
    edit_state(h, |s| {
        if !extend {
            s.sel_start = to;
        }
        s.sel_end = to;
    });
    invalidate(h);
}

fn edit_caret(h: H) -> usize {
    edit_state(h, |s| s.sel_end)
}

fn copy_selection(h: H) {
    let text = text_of(h);
    let (s, e) = edit_state(h, |s| (s.sel_start.min(s.sel_end), s.sel_start.max(s.sel_end)));
    if e > s && e <= text.len() && style_of(h) & ES_PASSWORD == 0 {
        super::paintctl::clipboard_set_text(h, &text[s..e]);
    }
}

fn edit(h: H, msg: u32, w: usize, l: isize) -> isize {
    let mut out = 0;
    if font_msg(h, msg, w, l, &mut out) {
        return out;
    }
    let style = style_of(h);
    let multi = style & ES_MULTILINE != 0;
    match msg {
        WM_GETDLGCODE => {
            let mut c = DLGC_WANTCHARS | DLGC_WANTARROWS | DLGC_HASSETSEL;
            if multi && style & ES_WANTRETURN != 0 && l != 0 {
                c |= dlg::DLGC_WANTMESSAGE;
            }
            c as isize
        }
        WM_GETTEXT => {
            // The edit control's own answer: size 0 touches nothing, any other size sets the
            // last error to 0 (`SetDlgItemTextW.tests/sdt_text.rs`: "edit, size 1: 0 err=0").
            if w == 0 {
                return 0;
            }
            let text = text_of(h);
            let n = text.len().min((w & 0xFFFF_FFFF).saturating_sub(1));
            let buf = l as *mut u16;
            // SAFETY: the caller's buffer of `w` units.
            unsafe {
                for (i, c) in text.iter().take(n).enumerate() {
                    *buf.add(i) = *c;
                }
                *buf.add(n) = 0;
            }
            set_last_error(0);
            n as isize
        }
        WM_SETTEXT => {
            let r = def(h, msg, w, l);
            edit_state(h, |s| {
                s.sel_start = 0;
                s.sel_end = 0;
            });
            edit_changed(h, true);
            r
        }
        EM_GETSEL => {
            let (s, e) = edit_state(h, |s| (s.sel_start.min(s.sel_end), s.sel_start.max(s.sel_end)));
            if w != 0 {
                // SAFETY: the caller's DWORD.
                unsafe { (w as *mut u32).write_unaligned(s as u32) };
            }
            if l != 0 {
                // SAFETY: the caller's DWORD.
                unsafe { (l as *mut u32).write_unaligned(e as u32) };
            }
            ((e as u32) << 16 | (s as u32 & 0xFFFF)) as isize
        }
        EM_SETSEL => {
            let len = text_of(h).len();
            let (s, e) = (w as i32, l as i32);
            let (s, e) = if s == -1 {
                (e.max(0) as usize, e.max(0) as usize)
            } else {
                (s as usize, if e == -1 { len } else { e as usize })
            };
            let (s, e) = if s == 0 && e == usize::MAX { (0, len) } else { (s.min(len), e.min(len)) };
            edit_state(h, |st| {
                st.sel_start = s;
                st.sel_end = e;
            });
            invalidate(h);
            1
        }
        EM_REPLACESEL => {
            // SAFETY: a NUL-terminated string.
            let ins = unsafe { super::wide(l as *const u16, -1) };
            edit_replace(h, &ins);
            1
        }
        EM_SETLIMITTEXT => {
            edit_state(h, |s| s.limit = if w == 0 { 0x7FFF_FFFE } else { w });
            0
        }
        EM_GETLIMITTEXT => edit_state(h, |s| s.limit) as isize,
        EM_GETMODIFY => isize::from(edit_state(h, |s| s.modified)),
        EM_SETMODIFY => {
            edit_state(h, |s| s.modified = w != 0);
            0
        }
        EM_SETREADONLY => {
            let on = w != 0;
            edit_state(h, |s| s.readonly = on);
            let new = if on { style | ES_READONLY } else { style & !ES_READONLY };
            call(Request::SetStyle { hwnd: h as u32, which: -16, value: new });
            1
        }
        EM_SETPASSWORDCHAR => {
            edit_state(h, |s| s.password = w as u16);
            0
        }
        EM_GETPASSWORDCHAR => edit_state(h, |s| s.password) as isize,
        EM_CANUNDO | EM_EMPTYUNDOBUFFER | EM_UNDO | EM_SCROLLCARET | EM_SETMARGINS | EM_GETMARGINS => 0,
        EM_GETLINECOUNT => (text_of(h).iter().filter(|&&c| c == 10).count() + 1) as isize,
        EM_LINEFROMCHAR => {
            let text = text_of(h);
            let at = if w as i32 == -1 { edit_caret(h) } else { w };
            text.iter().take(at).filter(|&&c| c == 10).count() as isize
        }
        EM_LINEINDEX => {
            let text = text_of(h);
            let want = if w as i32 == -1 { text.iter().take(edit_caret(h)).filter(|&&c| c == 10).count() } else { w };
            let mut line = 0;
            for (i, &c) in text.iter().enumerate() {
                if line == want {
                    return i as isize;
                }
                if c == 10 {
                    line += 1;
                }
            }
            if line == want { text.len() as isize } else { -1 }
        }
        // EM_GETLINE: line `w` into the buffer `l`, whose first unit says how many fit; no NUL is written
        // and the count copied comes back (0 when there is no such line).
        EM_GETLINE => {
            if l == 0 {
                return 0;
            }
            let text = text_of(h);
            let mut line = 0;
            let mut start = 0;
            for (i, &c) in text.iter().enumerate() {
                if line == w {
                    break;
                }
                if c == 10 {
                    line += 1;
                    start = i + 1;
                }
            }
            if line != w {
                return 0;
            }
            // SAFETY: the caller's buffer starts with the number of units it holds.
            let room = unsafe { (l as *const u16).read_unaligned() } as usize;
            let body: Vec<u16> = text[start..].iter().copied().take_while(|&c| c != 13 && c != 10).collect();
            let n = body.len().min(room);
            // SAFETY: `room` units are the caller's.
            unsafe { core::ptr::copy_nonoverlapping(body.as_ptr(), l as *mut u16, n) };
            n as isize
        }
        EM_LINELENGTH => {
            let text = text_of(h);
            let start = if w as i32 == -1 { 0 } else { w.min(text.len()) };
            text[start..].iter().take_while(|&&c| c != 13 && c != 10).count() as isize
        }
        WM_SETFOCUS => {
            let n = text_of(h).len();
            edit_state(h, |s| {
                if s.sel_start == s.sel_end && s.sel_end == 0 && n > 0 && style & ES_MULTILINE == 0 {
                    s.sel_start = 0;
                    s.sel_end = n;
                }
            });
            notify_parent(h, EN_SETFOCUS);
            invalidate(h);
            0
        }
        WM_KILLFOCUS => {
            notify_parent(h, EN_KILLFOCUS);
            invalidate(h);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            super::SetFocus(h);
            super::SetCapture(h);
            let at = super::paintctl::edit_hit(h, (l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
            if msg == WM_LBUTTONDBLCLK {
                let text = text_of(h);
                let word = |c: u16| (c as u8 as char).is_alphanumeric() || c > 0x7F;
                let mut s = at.min(text.len());
                let mut e = s;
                while s > 0 && word(text[s - 1]) {
                    s -= 1;
                }
                while e < text.len() && word(text[e]) {
                    e += 1;
                }
                edit_state(h, |st| {
                    st.sel_start = s;
                    st.sel_end = e;
                });
            } else {
                edit_move(h, at, key_down(VK_SHIFT));
            }
            invalidate(h);
            0
        }
        WM_MOUSEMOVE => {
            if super::GetCapture() == h && key_down(0x01) {
                let at = super::paintctl::edit_hit(h, (l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
                edit_move(h, at, true);
            }
            0
        }
        WM_LBUTTONUP => {
            if super::GetCapture() == h {
                super::ReleaseCapture();
            }
            0
        }
        WM_KEYDOWN => {
            let shift = key_down(VK_SHIFT);
            let ctrl = key_down(VK_CONTROL);
            let len = text_of(h).len();
            let caret = edit_caret(h);
            match w as u32 {
                VK_LEFT => {
                    let to = if ctrl { word_left(&text_of(h), caret) } else { caret.saturating_sub(1) };
                    let collapse = !shift && edit_state(h, |s| s.sel_start != s.sel_end);
                    if collapse {
                        let s = edit_state(h, |s| s.sel_start.min(s.sel_end));
                        edit_move(h, s, false);
                    } else {
                        edit_move(h, to, shift);
                    }
                }
                VK_RIGHT => {
                    let to = if ctrl { word_right(&text_of(h), caret) } else { (caret + 1).min(len) };
                    let collapse = !shift && edit_state(h, |s| s.sel_start != s.sel_end);
                    if collapse {
                        let e = edit_state(h, |s| s.sel_start.max(s.sel_end));
                        edit_move(h, e, false);
                    } else {
                        edit_move(h, to, shift);
                    }
                }
                VK_HOME => edit_move(h, if multi && !ctrl { line_start(&text_of(h), caret) } else { 0 }, shift),
                VK_END => edit_move(h, if multi && !ctrl { line_end(&text_of(h), caret) } else { len }, shift),
                VK_UP if multi => edit_move(h, vertical(&text_of(h), caret, -1), shift),
                VK_DOWN if multi => edit_move(h, vertical(&text_of(h), caret, 1), shift),
                VK_DELETE if !edit_state(h, |s| s.readonly) => {
                    if shift && edit_state(h, |s| s.sel_start != s.sel_end) {
                        copy_selection(h);
                        edit_replace(h, &[]);
                    } else if edit_state(h, |s| s.sel_start != s.sel_end) {
                        edit_replace(h, &[]);
                    } else if caret < len {
                        edit_state(h, |s| s.sel_end = caret + 1);
                        edit_replace(h, &[]);
                    }
                }
                _ => {}
            }
            0
        }
        WM_CHAR => {
            let c = w as u16;
            let ro = edit_state(h, |s| s.readonly) || style & ES_READONLY != 0;
            match c {
                0x01 => {
                    let n = text_of(h).len();
                    edit_state(h, |s| {
                        s.sel_start = 0;
                        s.sel_end = n;
                    });
                    invalidate(h);
                }
                0x03 => copy_selection(h),
                0x18 => {
                    copy_selection(h);
                    if !ro {
                        edit_replace(h, &[]);
                    }
                }
                0x16 => {
                    if !ro && let Some(t) = super::paintctl::clipboard_get_text(h) {
                        let t: Vec<u16> =
                            if multi { t } else { t.into_iter().take_while(|&c| c != 13 && c != 10).collect() };
                        edit_replace(h, &t);
                    }
                }
                8 => {
                    if !ro {
                        let caret = edit_caret(h);
                        if edit_state(h, |s| s.sel_start != s.sel_end) {
                            edit_replace(h, &[]);
                        } else if caret > 0 {
                            edit_state(h, |s| {
                                s.sel_start = caret - 1;
                                s.sel_end = caret;
                            });
                            edit_replace(h, &[]);
                        }
                    }
                }
                13 => {
                    if multi && !ro {
                        edit_replace(h, &[13, 10]);
                    }
                }
                9 | 10 | 27 => {}
                c if c >= 0x20 => {
                    if !ro {
                        edit_replace(h, &[c]);
                    }
                }
                _ => {}
            }
            0
        }
        WM_CUT => {
            copy_selection(h);
            edit_replace(h, &[]);
            0
        }
        WM_COPY => {
            copy_selection(h);
            0
        }
        WM_PASTE => {
            send(h, WM_CHAR, 0x16, 0);
            0
        }
        WM_CLEAR => {
            edit_replace(h, &[]);
            0
        }
        WM_ENABLE => {
            let r = def(h, msg, w, l);
            invalidate(h);
            r
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            super::paintctl::paint_edit(h);
            0
        }
        WM_NCPAINT => {
            super::paintctl::paint_border(h);
            0
        }
        WM_NCDESTROY => {
            EDITS.with(|m| m.remove(&(h as u32)));
            forget(h);
            def(h, msg, w, l)
        }
        _ => def(h, msg, w, l),
    }
}

fn word_left(t: &[u16], mut i: usize) -> usize {
    while i > 0 && t[i - 1] == 0x20 {
        i -= 1;
    }
    while i > 0 && t[i - 1] != 0x20 {
        i -= 1;
    }
    i
}

fn word_right(t: &[u16], mut i: usize) -> usize {
    while i < t.len() && t[i] != 0x20 {
        i += 1;
    }
    while i < t.len() && t[i] == 0x20 {
        i += 1;
    }
    i
}

fn line_start(t: &[u16], i: usize) -> usize {
    t[..i.min(t.len())].iter().rposition(|&c| c == 10).map_or(0, |p| p + 1)
}

fn line_end(t: &[u16], i: usize) -> usize {
    let mut j = i.min(t.len());
    while j < t.len() && t[j] != 13 && t[j] != 10 {
        j += 1;
    }
    j
}

fn vertical(t: &[u16], i: usize, dir: i32) -> usize {
    let start = line_start(t, i);
    let col = i - start;
    if dir < 0 {
        if start == 0 {
            return 0;
        }
        let prev_end = start - 1;
        let prev_end = if prev_end > 0 && t[prev_end - 1] == 13 { prev_end - 1 } else { prev_end };
        let prev_start = line_start(t, prev_end);
        (prev_start + col).min(prev_end)
    } else {
        let end = line_end(t, i);
        if end >= t.len() {
            return t.len();
        }
        let next_start = if t[end] == 13 && t.get(end + 1) == Some(&10) { end + 2 } else { end + 1 };
        let next_end = line_end(t, next_start);
        (next_start + col).min(next_end)
    }
}

// ---- ListBox

#[derive(Clone, Default)]
struct Item {
    text: Vec<u16>,
    data: usize,
    selected: bool,
}

#[derive(Clone, Default)]
pub struct List {
    items: Vec<Item>,
    cur: i32,
    top: i32,
    caret: i32,
    item_height: i32,
    anchor: i32,
}

static LISTS: SpinLock<BTreeMap<u32, List>> = SpinLock::new(BTreeMap::new());

fn list<R>(h: H, f: impl FnOnce(&mut List) -> R) -> R {
    LISTS.with(|m| {
        f(m.entry(h as u32).or_insert_with(|| List { cur: -1, caret: -1, item_height: 0, ..List::default() }))
    })
}

pub fn list_snapshot(h: H) -> (Vec<(Vec<u16>, bool)>, i32, i32) {
    list(h, |l| (l.items.iter().map(|i| (i.text.clone(), i.selected)).collect(), l.top, l.caret))
}

pub fn list_item_height(h: H) -> i32 {
    let v = list(h, |l| l.item_height);
    if v > 0 { v } else { 16 }
}

fn fold_cmp(a: &[u16], b: &[u16]) -> core::cmp::Ordering {
    a.iter().map(|&c| dlg::fold(c)).cmp(b.iter().map(|&c| dlg::fold(c)))
}

fn lb_multi(style: u32) -> bool {
    style & (LBS_MULTIPLESEL | LBS_EXTENDEDSEL) != 0
}

fn list_find(h: H, start: i32, text: &[u16], exact: bool) -> isize {
    list(h, |l| {
        let n = l.items.len() as i32;
        if n == 0 {
            return LB_ERR;
        }
        for k in 1..=n {
            let i = (start + k).rem_euclid(n);
            let it = &l.items[i as usize].text;
            let hit = if exact {
                fold_cmp(it, text) == core::cmp::Ordering::Equal
            } else {
                it.len() >= text.len() && fold_cmp(&it[..text.len()], text) == core::cmp::Ordering::Equal
            };
            if hit {
                return i as isize;
            }
        }
        LB_ERR
    })
}

fn visible_rows(h: H) -> i32 {
    let ih = list_item_height(h);
    info(h).map_or(1, |i| (i.client.height() / ih).max(1))
}

fn list_select(h: H, i: i32, notify: bool) {
    let style = style_of(h);
    let n = list(h, |l| l.items.len() as i32);
    if n == 0 {
        return;
    }
    let i = i.clamp(0, n - 1);
    if !lb_multi(style) {
        list(h, |l| {
            for it in &mut l.items {
                it.selected = false;
            }
            l.items[i as usize].selected = true;
            l.cur = i;
            l.caret = i;
        });
    } else {
        list(h, |l| {
            let was = l.items[i as usize].selected;
            l.items[i as usize].selected = !was;
            l.caret = i;
            l.cur = i;
        });
    }
    let rows = visible_rows(h);
    list(h, |l| {
        if i < l.top {
            l.top = i;
        } else if i >= l.top + rows {
            l.top = i - rows + 1;
        }
    });
    invalidate(h);
    if notify && style & LBS_NOTIFY != 0 {
        notify_parent(h, LBN_SELCHANGE);
    }
}

fn listbox(h: H, msg: u32, w: usize, l: isize) -> isize {
    let mut out = 0;
    if font_msg(h, msg, w, l, &mut out) {
        return out;
    }
    let style = style_of(h);
    let w32 = w as u32 as i32;
    match msg {
        WM_CREATE => {
            // A list box whose style has no LBS_NOINTEGRALHEIGHT shows whole items only: its height
            // becomes a multiple of the item height (plus its frame).
            const LBS_NOINTEGRALHEIGHT: u32 = 0x100;
            if style & (LBS_NOINTEGRALHEIGHT | LBS_OWNERDRAWVARIABLE) == 0
                && let Some(i) = info(h)
            {
                let ih = list_item_height(h).max(1);
                let frame = i.window.height() - i.client.height();
                let client = (i.client.height() / ih) * ih;
                if client + frame != i.window.height() {
                    // SAFETY: a plain window call (no move, no z order change, no activation).
                    unsafe { super::SetWindowPos(h, 0, 0, 0, i.window.width(), client + frame, 0x16) };
                }
            }
            0
        }
        WM_GETDLGCODE => (DLGC_WANTARROWS | DLGC_WANTCHARS) as isize,
        LB_ADDSTRING | LB_INSERTSTRING => {
            // SAFETY: a NUL-terminated string (or item data for an owner-drawn list).
            let text = if style & (LBS_OWNERDRAWFIXED | LBS_OWNERDRAWVARIABLE) != 0 && style & LBS_HASSTRINGS == 0 {
                Vec::new()
            } else {
                unsafe { super::wide(l as *const u16, -1) }
            };
            let data = if text.is_empty() && style & (LBS_OWNERDRAWFIXED | LBS_OWNERDRAWVARIABLE) != 0 {
                l as usize
            } else {
                0
            };
            let at = list(h, |ls| {
                let n = ls.items.len();
                let at = if msg == LB_INSERTSTRING && w32 >= 0 && (w32 as usize) < n {
                    w32 as usize
                } else if style & LBS_SORT != 0 {
                    ls.items
                        .iter()
                        .position(|it| fold_cmp(&it.text, &text) == core::cmp::Ordering::Greater)
                        .unwrap_or(n)
                } else {
                    n
                };
                if ls.cur >= at as i32 {
                    ls.cur += 1;
                }
                ls.items.insert(at, Item { text, data, selected: false });
                at
            });
            invalidate(h);
            at as isize
        }
        LB_DELETESTRING => {
            let ok = list(h, |ls| {
                if w32 < 0 || w32 as usize >= ls.items.len() {
                    return None;
                }
                ls.items.remove(w32 as usize);
                if ls.cur == w32 {
                    ls.cur = -1;
                } else if ls.cur > w32 {
                    ls.cur -= 1;
                }
                Some(ls.items.len())
            });
            invalidate(h);
            ok.map_or(LB_ERR, |n| n as isize)
        }
        LB_RESETCONTENT => {
            list(h, |ls| {
                ls.items.clear();
                ls.cur = -1;
                ls.top = 0;
                ls.caret = -1;
            });
            invalidate(h);
            0
        }
        LB_GETCOUNT => list(h, |ls| ls.items.len()) as isize,
        LB_SETCOUNT => 0,
        LB_GETCURSEL => {
            if lb_multi(style) {
                LB_ERR
            } else {
                list(h, |ls| ls.cur) as isize
            }
        }
        LB_SETCURSEL => {
            if lb_multi(style) {
                return LB_ERR;
            }
            let n = list(h, |ls| ls.items.len() as i32);
            if w32 < 0 || w32 >= n {
                list(h, |ls| {
                    for it in &mut ls.items {
                        it.selected = false;
                    }
                    ls.cur = -1;
                });
                invalidate(h);
                return LB_ERR;
            }
            list_select(h, w32, false);
            w32 as isize
        }
        LB_GETSEL => list(h, |ls| ls.items.get(w).map_or(LB_ERR, |i| isize::from(i.selected))),
        LB_SETSEL => {
            let ok = list(h, |ls| {
                if l as i32 == -1 {
                    for it in &mut ls.items {
                        it.selected = w != 0;
                    }
                    true
                } else if let Some(it) = ls.items.get_mut(l as u32 as usize) {
                    it.selected = w != 0;
                    true
                } else {
                    false
                }
            });
            invalidate(h);
            if ok { 0 } else { LB_ERR }
        }
        LB_SELITEMRANGE | LB_SELITEMRANGEEX => {
            let (a, b) = if msg == LB_SELITEMRANGE {
                ((l & 0xFFFF) as usize, ((l >> 16) & 0xFFFF) as usize)
            } else {
                (w, l as usize)
            };
            list(h, |ls| {
                for i in a.min(b)..=a.max(b) {
                    if let Some(it) = ls.items.get_mut(i) {
                        it.selected = if msg == LB_SELITEMRANGE { w != 0 } else { a <= b };
                    }
                }
            });
            invalidate(h);
            0
        }
        LB_GETSELCOUNT => {
            if lb_multi(style) {
                list(h, |ls| ls.items.iter().filter(|i| i.selected).count()) as isize
            } else {
                LB_ERR
            }
        }
        LB_GETSELITEMS => {
            let picks: Vec<u32> =
                list(h, |ls| ls.items.iter().enumerate().filter(|(_, i)| i.selected).map(|(n, _)| n as u32).collect());
            let n = picks.len().min(w);
            for (i, p) in picks.iter().take(n).enumerate() {
                // SAFETY: the caller's array of `w` ints.
                unsafe { (l as *mut u32).add(i).write_unaligned(*p) };
            }
            n as isize
        }
        LB_GETTEXT => list(h, |ls| match ls.items.get(w) {
            Some(it) => {
                let buf = l as *mut u16;
                // SAFETY: the caller's buffer is large enough (`LB_GETTEXTLEN`).
                unsafe {
                    for (i, c) in it.text.iter().enumerate() {
                        *buf.add(i) = *c;
                    }
                    *buf.add(it.text.len()) = 0;
                }
                it.text.len() as isize
            }
            None => LB_ERR,
        }),
        LB_GETTEXTLEN => list(h, |ls| ls.items.get(w).map_or(LB_ERR, |i| i.text.len() as isize)),
        LB_GETITEMDATA => list(h, |ls| ls.items.get(w).map_or(LB_ERR, |i| i.data as isize)),
        LB_SETITEMDATA => list(h, |ls| match ls.items.get_mut(w) {
            Some(i) => {
                i.data = l as usize;
                0
            }
            None => LB_ERR,
        }),
        LB_FINDSTRING | LB_FINDSTRINGEXACT | LB_SELECTSTRING => {
            // SAFETY: a NUL-terminated string.
            let t = unsafe { super::wide(l as *const u16, -1) };
            let r = list_find(h, w32, &t, msg == LB_FINDSTRINGEXACT);
            if msg == LB_SELECTSTRING && r != LB_ERR {
                list_select(h, r as i32, false);
            }
            r
        }
        LB_GETTOPINDEX => list(h, |ls| ls.top) as isize,
        LB_SETTOPINDEX => {
            let n = list(h, |ls| ls.items.len() as i32);
            if w32 < 0 || w32 >= n {
                return LB_ERR;
            }
            list(h, |ls| ls.top = w32);
            invalidate(h);
            0
        }
        LB_GETCARETINDEX => list(h, |ls| ls.caret.max(0)) as isize,
        LB_SETCARETINDEX => {
            list(h, |ls| ls.caret = w32);
            0
        }
        LB_SETITEMHEIGHT => {
            list(h, |ls| ls.item_height = (l & 0xFFFF) as i32);
            0
        }
        LB_GETITEMHEIGHT => list_item_height(h) as isize,
        LB_GETITEMRECT => {
            let ih = list_item_height(h);
            let (top, n) = list(h, |ls| (ls.top, ls.items.len() as i32));
            if w32 < 0 || w32 >= n {
                return LB_ERR;
            }
            let cw = info(h).map_or(0, |i| i.client.width());
            let r = [0i32, (w32 - top) * ih, cw, (w32 - top + 1) * ih];
            // SAFETY: the caller's RECT.
            unsafe { core::ptr::copy_nonoverlapping(r.as_ptr(), l as *mut i32, 4) };
            0
        }
        LB_ITEMFROMPOINT => {
            let ih = list_item_height(h);
            let y = ((l >> 16) & 0xFFFF) as i16 as i32;
            let (top, n) = list(h, |ls| (ls.top, ls.items.len() as i32));
            let i = top + y.max(0) / ih;
            (if i < n { i } else { n - 1 }.max(0)) as isize
        }
        LB_SETTABSTOPS | LB_SETHORIZONTALEXTENT | LB_SETCOLUMNWIDTH | LB_INITSTORAGE | LB_SETLOCALE => 1,
        LB_GETHORIZONTALEXTENT => 0,
        LB_GETLOCALE => 0x0409,
        LB_DIR | LB_ADDFILE => LB_ERR,
        WM_SETFOCUS => {
            notify_parent(h, LBN_SETFOCUS);
            invalidate(h);
            0
        }
        WM_KILLFOCUS => {
            notify_parent(h, LBN_KILLFOCUS);
            invalidate(h);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            super::SetFocus(h);
            let ih = list_item_height(h);
            let y = ((l >> 16) & 0xFFFF) as i16 as i32;
            let (top, n) = list(h, |ls| (ls.top, ls.items.len() as i32));
            let i = top + y.max(0) / ih;
            if i < n {
                list_select(h, i, true);
                if msg == WM_LBUTTONDBLCLK {
                    notify_parent(h, LBN_DBLCLK);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((w >> 16) & 0xFFFF) as i16 as i32;
            let rows = visible_rows(h);
            list(h, |ls| {
                let max = (ls.items.len() as i32 - rows).max(0);
                ls.top = (ls.top - delta / 120 * 3).clamp(0, max);
            });
            invalidate(h);
            0
        }
        WM_KEYDOWN => {
            let (cur, n) = list(h, |ls| (if ls.caret >= 0 { ls.caret } else { ls.cur }, ls.items.len() as i32));
            let rows = visible_rows(h);
            let to = match w as u32 {
                VK_UP => cur - 1,
                VK_DOWN => cur + 1,
                VK_PRIOR => cur - rows,
                VK_NEXT => cur + rows,
                VK_HOME => 0,
                VK_END => n - 1,
                _ => return 0,
            };
            if n > 0 {
                let to = to.clamp(0, n - 1);
                if lb_multi(style) {
                    list(h, |ls| ls.caret = to);
                    let rows = visible_rows(h);
                    list(h, |ls| {
                        if to < ls.top {
                            ls.top = to;
                        } else if to >= ls.top + rows {
                            ls.top = to - rows + 1;
                        }
                    });
                    invalidate(h);
                } else {
                    list_select(h, to, true);
                }
            }
            0
        }
        WM_CHAR => {
            if w as u16 >= 0x20 {
                let (cur, _) = list(h, |ls| (ls.cur, 0));
                let c = [w as u16];
                let r = list_find(h, cur, &c, false);
                if r != LB_ERR {
                    list_select(h, r as i32, true);
                }
            } else if w as u16 == 0x20 && lb_multi(style) {
                let c = list(h, |ls| ls.caret);
                if c >= 0 {
                    list_select(h, c, true);
                }
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            super::paintctl::paint_list(h);
            0
        }
        WM_NCPAINT => {
            super::paintctl::paint_border(h);
            0
        }
        WM_NCDESTROY => {
            LISTS.with(|m| m.remove(&(h as u32)));
            forget(h);
            def(h, msg, w, l)
        }
        _ => def(h, msg, w, l),
    }
}

pub fn has_border(h: H) -> bool {
    style_of(h) & WS_BORDER != 0 || ex_style_of(h) & WS_EX_CLIENTEDGE != 0
}

pub fn visible(h: H) -> bool {
    style_of(h) & WS_VISIBLE != 0
}
