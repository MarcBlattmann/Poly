//! The dialog manager: `DialogBoxParamW` and its relatives, `DefDlgProcW` (the
//! procedure of the class `#32770`), the `...DlgItem...` functions, the
//! keyboard navigation of `IsDialogMessageW` and the modal loop. The
//! templates and the order of controls are `poly_win32k::dlg`'s (host-tested);
//! the behaviour is the spec sheets' (`DialogBoxParamW`, `CreateDialogParamW`,
//! `EndDialog`, `GetDlgItem`, `SetDlgItemTextW`, `IsDialogMessageW`, ...).

use super::ansi::{self, Form};
use super::ansiapi;
use super::ctl::{self, BM_CLICK, BN_CLICKED, send};
use super::{H, SYS, SpinLock, call, info, set_last_error, u, wide};
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use poly_win32k::dlg::{self, BaseUnits, DlgTemplate, NavCtl, SzOrd};
use poly_win32k::proto::Request;
use poly_win32k::user::{CMsg, CreateStructW};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn FindResourceW(module: usize, name: *const u16, ty: *const u16) -> usize;
    fn LoadResource(module: usize, res: usize) -> usize;
    fn LockResource(data: usize) -> *const u8;
    fn SizeofResource(module: usize, res: usize) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_family: u32,
        face: *const u16,
    ) -> usize;
    fn CreateCompatibleDC(hdc: usize) -> usize;
    fn DeleteDC(hdc: usize) -> i32;
    fn SelectObject(hdc: usize, obj: usize) -> usize;
    fn GetStockObject(index: i32) -> usize;
    fn GetTextExtentPoint32W(hdc: usize, text: *const u16, count: i32, size: *mut i32) -> i32;
    fn GetTextMetricsW(hdc: usize, tm: *mut u8) -> i32;
}

const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_WINDOW_NOT_DIALOG: u32 = 1420;
const ERROR_CONTROL_ID_NOT_FOUND: u32 = 1421;

pub const WM_INITDIALOG: u32 = 0x0110;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_NEXTDLGCTL: u32 = 0x0028;
pub const WM_NCCREATE: u32 = 0x0081;
pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_SYSCHAR: u32 = 0x0106;
pub const WM_CTLCOLORMSGBOX: u32 = 0x0132;
pub const WM_CTLCOLORDLG: u32 = 0x0136;
pub const WM_COMPAREITEM: u32 = 0x0039;
pub const WM_VKEYTOITEM: u32 = 0x002E;
pub const WM_CHARTOITEM: u32 = 0x002F;
pub const WM_QUERYDRAGICON: u32 = 0x0037;
const SW_HIDE: i32 = 0;
const SW_SHOWNORMAL: i32 = 1;
const GWL_STYLE: i32 = -16;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_EX_CLIENTEDGE: u32 = 0x0000_0200;
const DWLP_MSGRESULT: i32 = 0;
const DWLP_DLGPROC: i32 = 8;

type DlgProc = unsafe extern "system" fn(H, u32, usize, isize) -> isize;

/// What the dialog class's `WM_NCCREATE` finds in `lpCreateParams`.
#[repr(C)]
struct InitData {
    magic: u64,
    proc_: usize,
}

const MAGIC: u64 = 0x504F_4C59_4449_4C47;

#[derive(Clone, Default)]
struct Dlg {
    modal: bool,
    ended: bool,
    result: isize,
    /// The id `DM_GETDEFID` reports (0: none), and the one the template made default.
    defid: u32,
    orig_def: u32,
    font: usize,
    units: (i32, i32),
    /// The control that had the focus when the dialog was deactivated.
    focus: usize,
    /// The dialog procedure when the class has no room for it.
    proc_: usize,
}

static DLGS: SpinLock<BTreeMap<u32, Dlg>> = SpinLock::new(BTreeMap::new());

/// The context help ids of windows (`SetWindowContextHelpId`; a dialog made from an extended
/// template has the template's).
static HELP_IDS: SpinLock<BTreeMap<u32, u32>> = SpinLock::new(BTreeMap::new());

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowContextHelpId(hwnd: H) -> u32 {
    if info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    HELP_IDS.with(|m| m.get(&(hwnd as u32)).copied().unwrap_or(0))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowContextHelpId(hwnd: H, id: u32) -> i32 {
    if info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    HELP_IDS.with(|m| m.insert(hwnd as u32, id));
    1
}

fn dlg_state<R>(h: H, f: impl FnOnce(&mut Dlg) -> R) -> R {
    DLGS.with(|m| f(m.entry(h as u32).or_default()))
}

fn is_dialog(h: H) -> bool {
    DLGS.with(|m| m.contains_key(&(h as u32))) || info(h).is_some_and(|i| i.wndproc == dlg::sysproc::DIALOG)
}

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn get_long(h: H, index: i32) -> Option<u64> {
    match call(Request::GetLong { hwnd: h as u64, index, size: 8 }) {
        poly_win32k::proto::Reply::Value { value } => Some(value),
        _ => None,
    }
}

fn set_long(h: H, index: i32, value: u64) -> bool {
    matches!(call(Request::SetLong { hwnd: h as u64, index, size: 8, value }), poly_win32k::proto::Reply::Value { .. })
}

fn dlgproc_of(h: H) -> usize {
    match get_long(h, DWLP_DLGPROC) {
        Some(v) => v as usize,
        None => dlg_state(h, |d| d.proc_),
    }
}

// ---- resources

/// The bytes of resource `name` of type `ty` in `module` (0: the executable).
fn find_template(module: usize, name: *const u16) -> Option<Vec<u8>> {
    // SAFETY: arguments of the kernel32 resource functions; RT_DIALOG is 5.
    unsafe {
        let r = FindResourceW(module, name, 5usize as *const u16);
        if r == 0 {
            return None;
        }
        let data = LoadResource(module, r);
        if data == 0 {
            return None;
        }
        let p = LockResource(data);
        let n = SizeofResource(module, r) as usize;
        if p.is_null() {
            return None;
        }
        Some(core::slice::from_raw_parts(p, n).to_vec())
    }
}

// ---- units and fonts

fn measure_font(hfont: usize) -> BaseUnits {
    let letters: Vec<u16> = (b'a'..=b'z').chain(b'A'..=b'Z').map(u16::from).collect();
    // SAFETY: gdi32 calls on a memory DC of ours.
    unsafe {
        let dc = CreateCompatibleDC(0);
        if dc == 0 {
            return BaseUnits { x: 6, y: 13 };
        }
        let font = if hfont != 0 { hfont } else { GetStockObject(13) };
        SelectObject(dc, font);
        let mut sz = [0i32; 2];
        let mut tm = [0u8; 80];
        let ok = GetTextExtentPoint32W(dc, letters.as_ptr(), 52, sz.as_mut_ptr());
        let ok2 = GetTextMetricsW(dc, tm.as_mut_ptr());
        DeleteDC(dc);
        let height = i32::from_le_bytes([tm[0], tm[1], tm[2], tm[3]]);
        if ok == 0 || ok2 == 0 || sz[0] <= 0 || height <= 0 {
            return BaseUnits { x: 6, y: 13 };
        }
        BaseUnits::from_extent(sz[0], height)
    }
}

/// The font the template asks for and the base units of it.
fn dialog_font(tpl: &DlgTemplate) -> (usize, BaseUnits) {
    let Some(f) = &tpl.font else { return (0, measure_font(0)) };
    let mut face = f.face.clone();
    face.push(0);
    let height = -dlg::mul_div(i32::from(f.points), 96, 72);
    // SAFETY: a NUL-terminated face name.
    let font = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            i32::from(f.weight),
            u32::from(f.italic),
            0,
            0,
            u32::from(f.charset),
            0,
            0,
            0,
            0,
            face.as_ptr(),
        )
    };
    (font, measure_font(font))
}

/// `GetDialogBaseUnits`: the system font's units (low word x, high word y).
#[unsafe(no_mangle)]
pub extern "system" fn GetDialogBaseUnits() -> i32 {
    let b = measure_font(0);
    ((b.y << 16) | (b.x & 0xFFFF)) as i32
}

/// `MapDialogRect`: dialog units to pixels with the dialog's font.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapDialogRect(hdlg: H, rect: *mut i32) -> i32 {
    if rect.is_null() || info(hdlg).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let (ux, uy) = dlg_state(hdlg, |d| d.units);
    let b = if ux > 0 && uy > 0 { BaseUnits { x: ux, y: uy } } else { measure_font(0) };
    // SAFETY: the caller's RECT.
    unsafe {
        *rect = b.x_px(*rect);
        *rect.add(1) = b.y_px(*rect.add(1));
        *rect.add(2) = b.x_px(*rect.add(2));
        *rect.add(3) = b.y_px(*rect.add(3));
    }
    1
}

// ---- creating a dialog

fn first_focus(children: &[H]) -> H {
    let navs = nav_list(children);
    match dlg::next_tab_item(&navs, None, false) {
        Some(i) => children[i],
        None => children.first().copied().unwrap_or(0),
    }
}

fn nav_of(c: H) -> NavCtl {
    let i = info(c);
    NavCtl {
        id: ctl::id_of(c),
        style: i.as_ref().map_or(0, |i| i.style),
        visible: i.as_ref().is_some_and(|i| i.style & WS_VISIBLE != 0),
        enabled: i.as_ref().is_some_and(|i| i.style & 0x0800_0000 == 0),
    }
}

fn nav_list(children: &[H]) -> Vec<NavCtl> {
    children.iter().map(|&c| nav_of(c)).collect()
}

/// Make the dialog window of `tpl` and its controls and send `WM_INITDIALOG`;
/// 0 with the last error set on failure.
pub fn create_dialog_window(
    tpl: &DlgTemplate,
    inst: usize,
    parent: H,
    proc_: usize,
    param: isize,
    modal: bool,
    px_units: bool,
) -> H {
    // An `A` dialog (`DialogBoxParamA` and the others) makes ANSI windows of the dialog and of
    // all its controls; their texts go through the ANSI code page when they are made.
    let ansi = ansiapi::take_ansi_dialog();
    if parent != 0 && info(parent).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let (style, ex) = dlg::window_styles(tpl);
    // A modal dialog is a top-level window (DialogBox*: `ERROR_TLW_WITH_WSCHILD`).
    if modal && style & WS_CHILD != 0 {
        set_last_error(poly_win32k::wm::ERROR_TLW_WITH_WSCHILD);
        return 0;
    }
    let visible = tpl.style & WS_VISIBLE != 0;
    let child_dialog = style & WS_CHILD != 0;
    let (hfont, mut units) = dialog_font(tpl);
    if px_units {
        // The template's numbers are pixels (a message box lays itself out).
        units = BaseUnits { x: 4, y: 8 };
    }
    // The window rectangle from the template's client size.
    let (cw, ch) = (units.x_px(i32::from(tpl.cx)), units.y_px(i32::from(tpl.cy)));
    let mut rc = [0, 0, cw, ch];
    let has_menu = !matches!(tpl.menu, SzOrd::None);
    // SAFETY: a RECT of ours.
    unsafe { super::AdjustWindowRectEx(rc.as_mut_ptr(), style, i32::from(has_menu), ex) };
    let (ww, wh) = (rc[2] - rc[0], rc[3] - rc[1]);
    let mut x = units.x_px(i32::from(tpl.x));
    let mut y = units.y_px(i32::from(tpl.y));
    let screen = u::screen(&SYS);
    if !child_dialog {
        let centre = tpl.style & dlg::DS_CENTER != 0;
        if !centre && tpl.style & dlg::DS_ABSALIGN == 0 && parent != 0 {
            if let Some(p) = info(parent) {
                x += p.client.left;
                y += p.client.top;
            }
        }
        if centre {
            let (ox, oy, ow, oh) = (0, 0, screen.work.right - screen.work.left, screen.work.bottom - screen.work.top);
            x = ox + (ow - ww) / 2;
            y = oy + (oh - wh) / 2;
        }
        x = x.min(screen.work.right - ww).max(screen.work.left);
        y = y.min(screen.work.bottom - wh).max(screen.work.top);
    }
    let class = tpl.class.class_name().map_or_else(
        || wstr("#32770"),
        |mut c| {
            c.push(0);
            c
        },
    );
    let mut title = if ansi { lossy(&tpl.title) } else { tpl.title.clone() };
    title.push(0);
    let init = InitData { magic: MAGIC, proc_ };
    let menu = match &tpl.menu {
        SzOrd::Ord(o) => super::load_menu_ordinal(inst, *o),
        SzOrd::Str(s) => {
            let mut s = s.clone();
            s.push(0);
            super::load_menu_name(inst, s.as_ptr())
        }
        SzOrd::None => 0,
    };
    // SAFETY: NUL-terminated names; `init` outlives the creation messages.
    let hwnd = unsafe {
        create_w(
            ansi,
            ex,
            class.as_ptr(),
            title.as_ptr(),
            style & !WS_VISIBLE,
            x,
            y,
            ww,
            wh,
            parent,
            menu,
            inst,
            &init as *const InitData as usize,
        )
    };
    if hwnd == 0 {
        return 0;
    }
    if tpl.help_id != 0 {
        HELP_IDS.with(|m| m.insert(hwnd as u32, tpl.help_id));
    }
    dlg_state(hwnd, |d| {
        d.modal = modal;
        d.font = hfont;
        d.units = (units.x, units.y);
        if dlgproc_of_unset(hwnd) {
            d.proc_ = proc_;
        }
    });
    // The controls, in template order.
    let mut icons: Vec<(H, u16, u32)> = Vec::new();
    for it in &tpl.items {
        let Some(mut cls) = it.class.class_name() else {
            set_last_error(poly_win32k::wm::ERROR_CANNOT_FIND_WND_CLASS);
            abort_creation(hwnd);
            return 0;
        };
        cls.push(0);
        let mut text = match &it.title {
            SzOrd::Str(s) if ansi => lossy(s),
            SzOrd::Str(s) => s.clone(),
            _ => Vec::new(),
        };
        text.push(0);
        let mut cstyle = it.style | WS_CHILD;
        let mut cex = it.ex_style;
        let cname = it.class.class_name().unwrap_or_default();
        let is_edit_like =
            [wstr("Edit"), wstr("ListBox"), wstr("ComboBox")].iter().any(|n| eq_fold(&n[..n.len() - 1], &cname));
        if is_edit_like && cstyle & WS_BORDER != 0 {
            cstyle &= !WS_BORDER;
            cex |= WS_EX_CLIENTEDGE;
        }
        let (px, py) = (units.x_px(i32::from(it.x)), units.y_px(i32::from(it.y)));
        let (pw, ph) = (units.x_px(i32::from(it.cx)), units.y_px(i32::from(it.cy)));
        let data = if it.data.is_empty() { 0 } else { it.data.as_ptr() as usize };
        // SAFETY: NUL-terminated names; the creation data outlives the call.
        let c = unsafe {
            create_w(
                ansi,
                cex,
                cls.as_ptr(),
                text.as_ptr(),
                cstyle,
                px,
                py,
                pw,
                ph,
                hwnd,
                it.id as usize,
                inst,
                data,
            )
        };
        if c == 0 {
            if tpl.style & dlg::DS_NOFAILCREATE != 0 {
                continue;
            }
            abort_creation(hwnd);
            return 0;
        }
        if let SzOrd::Ord(o) = it.title {
            icons.push((c, o, cstyle));
        }
        if hfont != 0 {
            send(c, ctl::WM_SETFONT, hfont, 0);
        }
    }
    for (c, ord, st) in icons {
        let kind = st & ctl::SS_TYPEMASK;
        if kind == ctl::SS_ICON {
            // SAFETY: a resource id as the name.
            let icon = unsafe { super::icons::LoadIconW(inst, ord as usize as *const u16) };
            if icon != 0 {
                send(c, ctl::STM_SETICON, icon, 0);
            }
        } else if kind == ctl::SS_BITMAP {
            let bmp = super::load_bitmap_ordinal(inst, ord);
            if bmp != 0 {
                send(c, ctl::STM_SETIMAGE, 0, bmp as isize);
            }
        }
    }
    let children = ctl::children_of(hwnd);
    // The default push button (DM_GETDEFID: DC_HASDEFID and its id).
    for &c in &children {
        if send(c, ctl::WM_GETDLGCODE, 0, 0) as u32 & dlg::DLGC_DEFPUSHBUTTON != 0 {
            let id = ctl::id_of(c);
            dlg_state(hwnd, |d| {
                d.defid = id;
                d.orig_def = id;
            });
            break;
        }
    }
    if hfont != 0 {
        send(hwnd, ctl::WM_SETFONT, hfont, 0);
    }
    let focus = first_focus(&children);
    send(hwnd, WM_INITDIALOG, focus, param);
    if visible && info(hwnd).is_some() && !dlg_state(hwnd, |d| d.ended) {
        super::ShowWindow(hwnd, SW_SHOWNORMAL);
    }
    // A made dialog leaves the last error at 0 (`CreateDialogParamW.tests/cdp_modeless.rs`).
    set_last_error(0);
    hwnd
}

/// What a text becomes when it goes through the ANSI code page and back.
fn lossy(text: &[u16]) -> Vec<u16> {
    poly_kernelbase::text::from_ansi(&poly_kernelbase::text::to_ansi(text))
}

/// `CreateWindowExW`, for an ANSI window when `ansi`.
#[allow(clippy::too_many_arguments)]
unsafe fn create_w(
    ansi: bool,
    ex: u32,
    class: *const u16,
    name: *const u16,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: H,
    menu: H,
    inst: usize,
    param: usize,
) -> H {
    if ansi {
        // SAFETY: as the caller's.
        ansiapi::with_ansi_window(|| unsafe {
            super::CreateWindowExW(ex, class, name, style, x, y, w, h, parent, menu, inst, param)
        })
    } else {
        // SAFETY: as the caller's.
        unsafe { super::CreateWindowExW(ex, class, name, style, x, y, w, h, parent, menu, inst, param) }
    }
}

fn dlgproc_of_unset(h: H) -> bool {
    get_long(h, DWLP_DLGPROC).is_none()
}

fn abort_creation(hwnd: H) {
    let err = poly_win32_gui::last_error();
    super::DestroyWindow(hwnd);
    DLGS.with(|m| m.remove(&(hwnd as u32)));
    set_last_error(err);
}

fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| dlg::fold(x) == dlg::fold(y))
}

// ---- the dialog procedure of the class #32770

fn control_color_brush(msg: u32) -> isize {
    let idx = if msg == ctl::WM_CTLCOLOREDIT || msg == ctl::WM_CTLCOLORLISTBOX { 5 } else { 15 };
    super::display::GetSysColorBrush(idx) as isize
}

pub fn dialog_proc(h: H, msg: u32, w: usize, l: isize) -> isize {
    dialog_entry(h, msg, w, l, Form::Wide)
}

/// The dialog procedure for a message of form `form`. A message of the other form than an ANSI
/// dialog window's is converted once, before anything sees it (so the text the dialog keeps has
/// been through the code page, `SendMessageW(WM_SETTEXT)` on an ANSI dialog).
pub fn dialog_entry(h: H, msg: u32, w: usize, l: isize, form: Form) -> isize {
    if form == Form::Wide && ansi::is_text_message(msg) && info(h).is_some_and(|i| !i.unicode) {
        let c = ansi::enter(h, Form::Wide, Form::Ansi, msg, w, l);
        let (cw, cl) = (c.w, c.l);
        let r = dialog_proc_in(h, msg, cw, cl, Form::Ansi);
        return c.finish(r);
    }
    dialog_proc_in(h, msg, w, l, form)
}

/// The default handling for a message of form `form`; the dialog procedure of the dialog gets
/// the message in the form of the dialog window (an ANSI window: bytes).
fn dialog_proc_in(h: H, msg: u32, w: usize, l: isize, form: Form) -> isize {
    let def = |h: H, msg: u32, w: usize, l: isize| ansiapi::def_form(h, msg, w, l, form);
    if msg == WM_NCCREATE {
        let cs = l as *const CreateStructW;
        if !cs.is_null() {
            // SAFETY: lParam of WM_NCCREATE is a CREATESTRUCTW.
            let p = unsafe { (*cs).create_params } as *const InitData;
            // SAFETY: a pointer from CreateWindowExW's creator; the magic tells ours.
            if !p.is_null() && p as usize & 7 == 0 && unsafe { (*p).magic } == MAGIC {
                // SAFETY: as above.
                let proc_ = unsafe { (*p).proc_ };
                if !set_long(h, DWLP_DLGPROC, proc_ as u64) {
                    dlg_state(h, |d| d.proc_ = proc_);
                }
            }
        }
        return def(h, msg, w, l);
    }
    let proc_ = dlgproc_of(h);
    let mut handled = false;
    let mut r = 0;
    if proc_ != 0 {
        // SAFETY: the application's dialog procedure.
        let f: DlgProc = unsafe { core::mem::transmute(proc_) };
        let to = if info(h).is_some_and(|i| !i.unicode) { Form::Ansi } else { Form::Wide };
        let c = ansi::enter(h, form, to, msg, w, l);
        let (cw, cl) = (c.w, c.l);
        // SAFETY: the documented call.
        let raw = unsafe { f(h, msg, cw, cl) };
        r = c.finish(raw);
        handled = r != 0;
    }
    match msg {
        WM_INITDIALOG => {
            // Nonzero: the system sets the focus to the control `w`.
            if (handled || proc_ == 0) && info(h).is_some() && !dlg_state(h, |d| d.ended) && w != 0 {
                dlg_set_focus(h, w);
            }
            return r;
        }
        0x0132..=0x0138 | WM_COMPAREITEM | WM_VKEYTOITEM | WM_CHARTOITEM | WM_QUERYDRAGICON => {
            if handled {
                return r;
            }
        }
        _ => {
            if handled {
                return get_long(h, DWLP_MSGRESULT).unwrap_or(0) as isize;
            }
        }
    }
    match msg {
        WM_CLOSE => {
            let cancel = find_item(h, 2);
            if cancel != 0 && info(cancel).is_some_and(|i| i.style & 0x0800_0000 == 0) {
                // The dialog manager posts IDCANCEL (DefDlgProcA.yaml `close`: not sent).
                super::PostMessageW(h, ctl::WM_COMMAND, 2, cancel as isize);
            } else {
                super::MessageBeep(0);
            }
            0
        }
        dlg::DM_GETDEFID => {
            let id = dlg_state(h, |d| d.defid);
            if id != 0 { ((dlg::DC_HASDEFID << 16) | (id & 0xFFFF)) as isize } else { 0 }
        }
        dlg::DM_SETDEFID => {
            set_default(h, w as u32);
            dlg_state(h, |d| d.orig_def = w as u32);
            1
        }
        WM_NEXTDLGCTL => {
            let children = ctl::children_of(h);
            let navs = nav_list(&children);
            let target = if l & 0xFFFF != 0 {
                w
            } else {
                let focus = super::GetFocus();
                let from = children.iter().position(|&c| c == focus || super::IsChild(c, focus) != 0);
                match dlg::next_tab_item(&navs, from, w != 0) {
                    Some(i) => children[i],
                    None => 0,
                }
            };
            if target != 0 {
                dlg_set_focus(h, target);
            }
            0
        }
        ctl::WM_SETFONT => {
            dlg_state(h, |d| d.font = w);
            0
        }
        ctl::WM_GETFONT => dlg_state(h, |d| d.font) as isize,
        0x0132..=0x0138 => control_color_brush(msg),
        ctl::WM_ERASEBKGND => {
            if let Some(i) = info(h) {
                let rc = [0, 0, i.client.width(), i.client.height()];
                // SAFETY: the DC the system gave for erasing.
                unsafe { super::FillRect(w, rc.as_ptr(), 16) };
            }
            1
        }
        WM_ACTIVATE => {
            if w & 0xFFFF == 0 {
                let f = super::GetFocus();
                dlg_state(h, |d| d.focus = f);
            } else {
                let f = dlg_state(h, |d| d.focus);
                if f != 0 && info(f).is_some() && !dlg_state(h, |d| d.ended) {
                    super::SetFocus(f);
                }
            }
            def(h, msg, w, l)
        }
        ctl::WM_SETFOCUS => {
            let children = ctl::children_of(h);
            let f = dlg_state(h, |d| d.focus);
            let target = if f != 0 && info(f).is_some() { f } else { first_focus(&children) };
            if target != 0 && !dlg_state(h, |d| d.ended) {
                super::SetFocus(target);
            }
            0
        }
        ctl::WM_NCDESTROY => {
            DLGS.with(|m| m.remove(&(h as u32)));
            HELP_IDS.with(|m| m.remove(&(h as u32)));
            def(h, msg, w, l)
        }
        _ => def(h, msg, w, l),
    }
}

fn def(h: H, msg: u32, w: usize, l: isize) -> isize {
    u::def_window_proc(&SYS, h as u64, msg, w as u64, l as u64) as isize
}

#[unsafe(no_mangle)]
pub extern "system" fn DefDlgProcW(h: H, msg: u32, w: usize, l: isize) -> isize {
    dialog_proc(h, msg, w, l)
}

#[unsafe(no_mangle)]
pub extern "system" fn DefDlgProcA(h: H, msg: u32, w: usize, l: isize) -> isize {
    dialog_entry(h, msg, w, l, Form::Ansi)
}

// ---- items

/// The first child of `dlg` with id `id` (no last error).
fn find_item(dlg: H, id: u32) -> H {
    ctl::children_of(dlg).into_iter().find(|&c| ctl::id_of(c) == id).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDlgItem(dlg: H, id: i32) -> H {
    if info(dlg).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let c = find_item(dlg, id as u32);
    if c == 0 {
        set_last_error(ERROR_CONTROL_ID_NOT_FOUND);
    }
    c
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDlgCtrlID(h: H) -> i32 {
    match info(h) {
        None => {
            set_last_error(ERROR_INVALID_WINDOW_HANDLE);
            0
        }
        Some(i) if i.style & WS_CHILD != 0 => i.id as u32 as i32,
        Some(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetDlgItemTextW(dlg: H, id: i32, text: *const u16) -> i32 {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    send(c, ctl::WM_SETTEXT, 0, text as isize);
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDlgItemTextW(dlg: H, id: i32, buf: *mut u16, max: i32) -> u32 {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        if max > 0 && !buf.is_null() {
            // SAFETY: the caller's buffer of at least one unit.
            unsafe { *buf = 0 };
        }
        return 0;
    }
    send(c, ctl::WM_GETTEXT, max as u32 as usize, buf as isize) as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetDlgItemTextA(dlg: H, id: i32, text: *const u8) -> i32 {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    ansiapi::send_form(c, ctl::WM_SETTEXT, 0, text as isize, Form::Ansi);
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDlgItemTextA(dlg: H, id: i32, buf: *mut u8, max: i32) -> u32 {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        if max > 0 && !buf.is_null() {
            // SAFETY: the caller's buffer of at least one byte.
            unsafe { *buf = 0 };
        }
        return 0;
    }
    if max < 0 {
        // A negative size: an empty string and the last error 0 (GetDlgItemTextA.yaml `size -1`).
        if !buf.is_null() {
            // SAFETY: the caller's buffer.
            unsafe { *buf = 0 };
        }
        set_last_error(0);
        return 0;
    }
    if max == 0 {
        return 0;
    }
    let n = ansiapi::send_form(c, ctl::WM_GETTEXT, max as usize, buf as isize, Form::Ansi) as u32;
    if n == 0 && max == 1 {
        set_last_error(0);
    }
    n
}

#[unsafe(no_mangle)]
pub extern "system" fn SendDlgItemMessageA(dlg: H, id: i32, msg: u32, w: usize, l: isize) -> isize {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    ansiapi::send_form(c, msg, w, l, Form::Ansi)
}

#[unsafe(no_mangle)]
pub extern "system" fn SendDlgItemMessageW(dlg: H, id: i32, msg: u32, w: usize, l: isize) -> isize {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    send(c, msg, w, l)
}

#[unsafe(no_mangle)]
pub extern "system" fn CheckDlgButton(dlg: H, id: i32, check: u32) -> i32 {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    send(c, ctl::BM_SETCHECK, check as usize, 0);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn IsDlgButtonChecked(dlg: H, id: i32) -> u32 {
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    send(c, ctl::BM_GETCHECK, 0, 0) as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn CheckRadioButton(dlg: H, first: i32, last: i32, check: i32) -> i32 {
    if info(dlg).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let (first, last) = (i64::from(first), i64::from(last));
    let mut id = first;
    while id <= last {
        let c = find_item(dlg, id as u32);
        if c != 0 {
            send(c, ctl::BM_SETCHECK, usize::from(id == i64::from(check)), 0);
        }
        id += 1;
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn SetDlgItemInt(dlg: H, id: i32, value: u32, signed: i32) -> i32 {
    let mut text = dlg::format_int(value, signed != 0);
    text.push(0);
    // SAFETY: a NUL-terminated string of ours.
    unsafe { SetDlgItemTextW(dlg, id, text.as_ptr()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetDlgItemInt(dlg: H, id: i32, translated: *mut i32, signed: i32) -> u32 {
    if !translated.is_null() {
        // SAFETY: the caller's BOOL.
        unsafe { *translated = 0 };
    }
    let c = GetDlgItem(dlg, id);
    if c == 0 {
        return 0;
    }
    let n = send(c, ctl::WM_GETTEXTLENGTH, 0, 0).max(0) as usize;
    let mut buf = vec![0u16; n + 1];
    send(c, ctl::WM_GETTEXT, n + 1, buf.as_mut_ptr() as isize);
    buf.truncate(n);
    let (v, ok) = dlg::parse_int(&buf, signed != 0);
    if !translated.is_null() {
        // SAFETY: the caller's BOOL.
        unsafe { *translated = i32::from(ok) };
    }
    v
}

// ---- tab order

fn direct_child(dlg: H, from: H) -> Option<usize> {
    let children = ctl::children_of(dlg);
    children.iter().position(|&c| c == from)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetNextDlgTabItem(dlg: H, ctrl: H, previous: i32) -> H {
    if info(dlg).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let children = ctl::children_of(dlg);
    let navs = nav_list(&children);
    let from = children.iter().position(|&c| c == ctrl);
    dlg::next_tab_item(&navs, from, previous != 0).map_or(0, |i| children[i])
}

#[unsafe(no_mangle)]
pub extern "system" fn GetNextDlgGroupItem(dlg: H, ctrl: H, previous: i32) -> H {
    if info(dlg).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let children = ctl::children_of(dlg);
    let navs = nav_list(&children);
    let from = children.iter().position(|&c| c == ctrl);
    dlg::next_group_item(&navs, from, previous != 0).map_or(0, |i| children[i])
}

// ---- default push button and focus

/// Make `id` the default push button: the styles of the old and the new one follow.
fn set_default(dlg: H, id: u32) {
    let old = dlg_state(dlg, |d| core::mem::replace(&mut d.defid, id));
    if old == id {
        return;
    }
    for (cid, def) in [(old, false), (id, true)] {
        if cid == 0 {
            continue;
        }
        let c = find_item(dlg, cid);
        if c != 0
            && send(c, ctl::WM_GETDLGCODE, 0, 0) as u32 & (dlg::DLGC_DEFPUSHBUTTON | dlg::DLGC_UNDEFPUSHBUTTON) != 0
        {
            send(c, ctl::BM_SETSTYLE, if def { ctl::BS_DEFPUSHBUTTON } else { ctl::BS_PUSHBUTTON } as usize, 1);
        }
    }
}

/// `SetFocus` on a control the way the dialog manager does it: a push button becomes
/// the default one, anything else gives the original default back.
fn dlg_set_focus(dlg: H, c: H) {
    super::SetFocus(c);
    let code = send(c, ctl::WM_GETDLGCODE, 0, 0) as u32;
    if code & (dlg::DLGC_DEFPUSHBUTTON | dlg::DLGC_UNDEFPUSHBUTTON) != 0 {
        set_default(dlg, ctl::id_of(c));
    } else {
        let orig = dlg_state(dlg, |d| d.orig_def);
        if orig != 0 {
            set_default(dlg, orig);
        }
    }
}

// ---- IsDialogMessageW

fn dispatch(m: &CMsg) {
    // SAFETY: a message of the caller's.
    unsafe {
        super::TranslateMessage(m);
        super::DispatchMessageW(m);
    }
}

fn press_default(dlg: H, id: u32) {
    let c = find_item(dlg, id);
    if c != 0 && info(c).is_some_and(|i| i.style & 0x0800_0000 != 0) {
        super::MessageBeep(0);
        return;
    }
    send(dlg, ctl::WM_COMMAND, (BN_CLICKED as usize) << 16 | id as usize, c as isize);
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsDialogMessageW(dlg: H, msg: *mut CMsg) -> i32 {
    if msg.is_null() || info(dlg).is_none() {
        return 0;
    }
    // SAFETY: the caller's MSG.
    let m = unsafe { *msg };
    let target = m.hwnd as H;
    if target == 0 || (target != dlg && super::IsChild(dlg, target) == 0) {
        return 0;
    }
    let vk = m.w as u32;
    match m.message {
        ctl::WM_KEYDOWN => {
            let code =
                if target == dlg { 0 } else { send(target, ctl::WM_GETDLGCODE, vk as usize, msg as isize) as u32 };
            match vk {
                ctl::VK_TAB => {
                    if code & (dlg::DLGC_WANTTAB | dlg::DLGC_WANTALLKEYS) != 0 {
                        dispatch(&m);
                        return 1;
                    }
                    let children = ctl::children_of(dlg);
                    let navs = nav_list(&children);
                    let from = children.iter().position(|&c| c == target || super::IsChild(c, target) != 0);
                    let back = super::GetKeyState(ctl::VK_SHIFT as i32) < 0;
                    if let Some(i) = dlg::next_tab_item(&navs, from, back) {
                        dlg_set_focus(dlg, children[i]);
                    }
                    1
                }
                ctl::VK_LEFT | ctl::VK_UP | ctl::VK_RIGHT | ctl::VK_DOWN => {
                    if code & (dlg::DLGC_WANTARROWS | dlg::DLGC_WANTALLKEYS) != 0 {
                        dispatch(&m);
                        return 1;
                    }
                    let children = ctl::children_of(dlg);
                    let navs = nav_list(&children);
                    let from = children.iter().position(|&c| c == target || super::IsChild(c, target) != 0);
                    let back = vk == ctl::VK_LEFT || vk == ctl::VK_UP;
                    if let (Some(f), Some(i)) = (from, dlg::next_group_item(&navs, from, back))
                        && i != f
                    {
                        let c = children[i];
                        dlg_set_focus(dlg, c);
                        if send(c, ctl::WM_GETDLGCODE, 0, 0) as u32 & dlg::DLGC_RADIOBUTTON != 0 {
                            send(c, BM_CLICK, 0, 0);
                        }
                    }
                    1
                }
                ctl::VK_RETURN => {
                    if code & (dlg::DLGC_WANTALLKEYS | dlg::DLGC_WANTMESSAGE) != 0 {
                        dispatch(&m);
                        return 1;
                    }
                    let id = match dlg_state(dlg, |d| d.defid) {
                        0 => dlg::IDOK,
                        id => id,
                    };
                    press_default(dlg, id);
                    1
                }
                ctl::VK_ESCAPE => {
                    if code & dlg::DLGC_WANTALLKEYS != 0 {
                        dispatch(&m);
                        return 1;
                    }
                    press_default(dlg, dlg::IDCANCEL);
                    1
                }
                _ => {
                    dispatch(&m);
                    1
                }
            }
        }
        WM_SYSCHAR => {
            let c = m.w as u16;
            if c == 0x20 || c == u16::from(b'-') {
                return 0;
            }
            let want = dlg::fold(c);
            let children = ctl::children_of(dlg);
            let focus = children.iter().position(|&k| k == target || super::IsChild(k, target) != 0).unwrap_or(0);
            let n = children.len();
            for step in 1..=n {
                let i = (focus + step) % n.max(1);
                let k = children[i];
                let code = send(k, ctl::WM_GETDLGCODE, 0, 0) as u32;
                if code & (dlg::DLGC_BUTTON | dlg::DLGC_STATIC) == 0 {
                    continue;
                }
                let nav = nav_of(k);
                if !nav.visible || !nav.enabled || dlg::mnemonic(&ctl::text_of(k)) != Some(want) {
                    continue;
                }
                if code & dlg::DLGC_STATIC != 0 {
                    let navs = nav_list(&children);
                    if let Some(t) = dlg::next_tab_item(&navs, Some(i), false) {
                        dlg_set_focus(dlg, children[t]);
                    }
                } else {
                    dlg_set_focus(dlg, k);
                    if code & dlg::DLGC_RADIOBUTTON == 0 {
                        send(k, BM_CLICK, 0, 0);
                    }
                }
                return 1;
            }
            super::MessageBeep(0);
            1
        }
        _ => {
            dispatch(&m);
            1
        }
    }
}

// ---- the modal loop

fn run_modal(hdlg: H, owner: H) -> isize {
    let mut owner_disabled = false;
    if owner != 0 {
        // The owner (the top-level window of a child) is disabled while the dialog runs.
        let top = super::GetAncestor(owner, 2);
        let top = if top == 0 { owner } else { top };
        if info(top).is_some() {
            super::EnableWindow(top, 0);
            owner_disabled = true;
        }
    }
    let top_owner = if owner != 0 {
        {
            let t = super::GetAncestor(owner, 2);
            if t == 0 { owner } else { t }
        }
    } else {
        0
    };
    loop {
        if dlg_state(hdlg, |d| d.ended) || info(hdlg).is_none() {
            break;
        }
        let mut m = CMsg::default();
        let r = u::get_message(&SYS, &mut m, 0, 0, 0);
        if r == 0 {
            // WM_QUIT: the dialog ends and the quit goes on to the caller's loop.
            super::PostQuitMessage(m.w as i32);
            break;
        }
        if r == -1 {
            break;
        }
        // SAFETY: a message just retrieved.
        let handled = unsafe { IsDialogMessageW(hdlg, &mut m) } != 0;
        if !handled {
            dispatch(&m);
        }
    }
    let result = dlg_state(hdlg, |d| d.result);
    if owner_disabled && info(top_owner).is_some() {
        super::EnableWindow(top_owner, 1);
    }
    if info(hdlg).is_some() {
        super::DestroyWindow(hdlg);
    }
    result
}

// ---- the API

unsafe fn template_from(inst: usize, name: *const u16) -> Option<DlgTemplate> {
    let bytes = find_template(inst, name)?;
    dlg::parse_template(&bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDialogParamW(
    inst: usize,
    name: *const u16,
    parent: H,
    proc_: usize,
    param: isize,
) -> H {
    // SAFETY: a resource id or a NUL-terminated name.
    let Some(tpl) = (unsafe { template_from(inst, name) }) else {
        if poly_win32_gui::last_error() == 0 {
            set_last_error(87);
        }
        return 0;
    };
    create_dialog_window(&tpl, inst, parent, proc_, param, false, false)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDialogIndirectParamW(
    inst: usize,
    template: *const u8,
    parent: H,
    proc_: usize,
    param: isize,
) -> H {
    let Some(tpl) = (unsafe { template_in_memory(template) }) else {
        set_last_error(87);
        return 0;
    };
    create_dialog_window(&tpl, inst, parent, proc_, param, false, false)
}

/// A resource name of an `A` function as the `W` functions take it.
unsafe fn name_w(name: *const u8) -> (Vec<u16>, *const u16) {
    if (name as usize) < 0x1_0000 {
        return (Vec::new(), name as *const u16);
    }
    // SAFETY: a NUL-terminated name.
    let mut w = unsafe { ansi::wide_of_a(name) };
    w.push(0);
    let p = w.as_ptr();
    (w, p)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDialogParamA(
    inst: usize,
    name: *const u8,
    parent: H,
    proc_: usize,
    param: isize,
) -> H {
    // SAFETY: a resource id or a NUL-terminated name.
    let (_keep, p) = unsafe { name_w(name) };
    // SAFETY: the name just made.
    ansiapi::with_ansi_dialog(|| unsafe { CreateDialogParamW(inst, p, parent, proc_, param) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDialogIndirectParamA(
    inst: usize,
    template: *const u8,
    parent: H,
    proc_: usize,
    param: isize,
) -> H {
    // SAFETY: the caller's template.
    ansiapi::with_ansi_dialog(|| unsafe { CreateDialogIndirectParamW(inst, template, parent, proc_, param) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DialogBoxParamA(
    inst: usize,
    name: *const u8,
    parent: H,
    proc_: usize,
    param: isize,
) -> isize {
    // SAFETY: a resource id or a NUL-terminated name.
    let (_keep, p) = unsafe { name_w(name) };
    // SAFETY: the name just made.
    ansiapi::with_ansi_dialog(|| unsafe { DialogBoxParamW(inst, p, parent, proc_, param) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DialogBoxIndirectParamA(
    inst: usize,
    template: *const u8,
    parent: H,
    proc_: usize,
    param: isize,
) -> isize {
    // SAFETY: the caller's template.
    ansiapi::with_ansi_dialog(|| unsafe { DialogBoxIndirectParamW(inst, template, parent, proc_, param) })
}

/// A template in memory has no stated length: the parser reads what it needs.
unsafe fn template_in_memory(p: *const u8) -> Option<DlgTemplate> {
    if p.is_null() {
        return None;
    }
    // Templates are far smaller than this; the parser stops at its own end.
    // SAFETY: the caller's template, at most 64 KiB (the format's limit of controls).
    let bytes = unsafe { core::slice::from_raw_parts(p, 0x10000) };
    dlg::parse_template(bytes)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DialogBoxParamW(
    inst: usize,
    name: *const u16,
    parent: H,
    proc_: usize,
    param: isize,
) -> isize {
    // SAFETY: a resource id or a NUL-terminated name.
    let Some(tpl) = (unsafe { template_from(inst, name) }) else {
        return -1;
    };
    run_dialog(&tpl, inst, parent, proc_, param, false)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DialogBoxIndirectParamW(
    inst: usize,
    template: *const u8,
    parent: H,
    proc_: usize,
    param: isize,
) -> isize {
    let Some(tpl) = (unsafe { template_in_memory(template) }) else {
        return -1;
    };
    run_dialog(&tpl, inst, parent, proc_, param, false)
}

pub fn run_dialog(tpl: &DlgTemplate, inst: usize, parent: H, proc_: usize, param: isize, px_units: bool) -> isize {
    if parent != 0 && info(parent).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    let h = create_dialog_window(tpl, inst, parent, proc_, param, true, px_units);
    if h == 0 {
        // The error of the failed creation; a failed creation returns -1.
        return -1;
    }
    run_modal(h, parent)
}

#[unsafe(no_mangle)]
pub extern "system" fn EndDialog(h: H, result: isize) -> i32 {
    if info(h).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    if !is_dialog(h) {
        set_last_error(ERROR_WINDOW_NOT_DIALOG);
        return 0;
    }
    let modal = dlg_state(h, |d| {
        if d.modal {
            d.ended = true;
            d.result = result;
        }
        d.modal
    });
    if modal {
        super::ShowWindow(h, SW_HIDE);
        // Wake the loop up.
        super::PostMessageW(h, 0, 0, 0);
    }
    1
}

/// `SetWindowLongPtr(hdlg, DWLP_MSGRESULT, ..)`'s companion for dialog procedures that
/// would rather ask: nothing to export; kept as the documented index.
#[allow(dead_code)]
const _DWLP: i32 = GWL_STYLE + DWLP_MSGRESULT;

#[allow(dead_code)]
pub fn wide_of(p: *const u16) -> Vec<u16> {
    // SAFETY: a NUL-terminated string.
    unsafe { wide(p, -1) }
}
