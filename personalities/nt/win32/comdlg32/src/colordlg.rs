//! The colour dialog (`ChooseColorW`; Learn, "CHOOSECOLORW structure").
//!
//! The 48 basic colours and 16 custom colours are two grids (windows of our own classes that
//! paint cells), the chosen colour shows in a sample and in the hue, saturation, luminance
//! and red, green, blue boxes; "Add to Custom Colors" stores it. The dialog always shows
//! the whole of itself (`CC_FULLOPEN` has nothing more to open).

use crate::sys::{self, H};
use crate::util::*;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_comdlg::cderr;
use poly_comdlg::check::{self, Facts, cc};
use poly_comdlg::color::{self, BASIC};
use poly_comdlg::template::{Class, Dialog, Item, style::*};

/// Offsets in `CHOOSECOLORW` on x64 (`commdlg.h`).
mod at {
    pub const SIZE: usize = 0;
    pub const OWNER: usize = 8;
    pub const INSTANCE: usize = 16;
    pub const RESULT: usize = 24;
    pub const CUSTOM: usize = 32;
    pub const FLAGS: usize = 40;
    pub const HOOK: usize = 56;
    pub const TEMPLATE: usize = 64;
}

/// Control ids (`dlgs.h`): `COLOR_HUE` ... `COLOR_BLUE`, `COLOR_CURRENT`, `COLOR_ADD`,
/// `COLOR_BOX1`, `COLOR_CUSTOM1`.
const ID_HUE: i32 = 703;
const ID_SAT: i32 = 704;
const ID_LUM: i32 = 705;
const ID_RED: i32 = 706;
const ID_GREEN: i32 = 707;
const ID_BLUE: i32 = 708;
const ID_CURRENT: i32 = 709;
const ID_ADD: i32 = 712;
const ID_BASIC: i32 = 720;
const ID_CUSTOM: i32 = 721;

const GWL_ID: i32 = -12;
const COLS: i32 = 8;

struct State {
    cc: *mut u8,
    hook: usize,
    hdlg: H,
    cur: u32,
    custom: *mut u32,
    /// The selected cell: grid id and index.
    sel: Option<(i32, usize)>,
    next_custom: usize,
    updating: bool,
}

impl State {
    fn custom_color(&self, i: usize) -> u32 {
        if self.custom.is_null() {
            return 0xFF_FFFF;
        }
        // SAFETY: `lpCustColors` points at 16 colours (`CHOOSECOLORW`).
        unsafe { *self.custom.add(i) }
    }
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
    if let Err(code) = check::choose_color(&facts, rd::<usize>(c, at::CUSTOM) != 0) {
        crate::set_ext_error(code);
        return 0;
    }
    if flags & cc::ENABLETEMPLATE != 0
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
    register_classes();
    let cur = if flags & cc::RGBINIT != 0 { rd::<u32>(c, at::RESULT) & 0xFF_FFFF } else { 0 };
    let mut st = Box::new(State {
        cc: c,
        hook: rd(c, at::HOOK),
        hdlg: 0,
        cur,
        custom: rd(c, at::CUSTOM),
        sel: None,
        next_custom: 0,
        updating: false,
    });
    let dlg = build_template();
    let bytes = dlg.build();
    let param = &mut *st as *mut State as isize;
    let r = if flags & cc::ENABLETEMPLATE != 0 {
        let (inst, name): (usize, usize) = (rd(c, at::INSTANCE), rd(c, at::TEMPLATE));
        // SAFETY: the program's dialog resource; our procedure.
        let r = unsafe { sys::DialogBoxParamW(inst, name as *const u16, owner, proc_, param) };
        if r == -1 { Err(cderr::DIALOGFAILURE) } else { Ok(r) }
    } else {
        run_modal(&bytes, owner, proc_, param)
    };
    match r {
        Ok(1) => {
            wr(c, at::RESULT, st.cur);
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

fn build_template() -> Dialog<'static> {
    let child = WS_CHILD | WS_VISIBLE;
    let edit = child | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL | ES_NUMBER;
    let label = |text, id, x, y| Item {
        class: Class::Static,
        text,
        id,
        style: child | SS_LEFT,
        ex_style: 0,
        x,
        y,
        cx: 26,
        cy: 9,
    };
    let field = |id: i32, x, y| Item {
        class: Class::Edit,
        text: "",
        id: id as u16,
        style: edit,
        ex_style: 0,
        x,
        y,
        cx: 26,
        cy: 12,
    };
    Dialog {
        style: WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_CENTER | DS_3DLOOK,
        ex_style: 0,
        x: 0,
        y: 0,
        cx: 262,
        cy: 152,
        title: "Color",
        font: Some((8, "MS Shell Dlg")),
        items: alloc::vec![
            Item {
                class: Class::Static,
                text: "&Basic colors:",
                id: 0xFFFF,
                style: child | SS_LEFT,
                ex_style: 0,
                x: 6,
                y: 6,
                cx: 100,
                cy: 9
            },
            Item {
                class: Class::Name("PolyColorGrid"),
                text: "",
                id: ID_BASIC as u16,
                style: child | WS_TABSTOP,
                ex_style: 0,
                x: 6,
                y: 17,
                cx: 120,
                cy: 72
            },
            Item {
                class: Class::Static,
                text: "&Custom colors:",
                id: 0xFFFF,
                style: child | SS_LEFT,
                ex_style: 0,
                x: 6,
                y: 94,
                cx: 100,
                cy: 9
            },
            Item {
                class: Class::Name("PolyColorGrid"),
                text: "",
                id: ID_CUSTOM as u16,
                style: child | WS_TABSTOP,
                ex_style: 0,
                x: 6,
                y: 105,
                cx: 120,
                cy: 24
            },
            Item {
                class: Class::Name("PolyColorSample"),
                text: "",
                id: ID_CURRENT as u16,
                style: child | WS_BORDER,
                ex_style: 0,
                x: 140,
                y: 6,
                cx: 108,
                cy: 40
            },
            label("Hu&e:", 0xFFFF, 140, 58),
            field(ID_HUE, 166, 56),
            label("&Sat:", 0xFFFF, 140, 74),
            field(ID_SAT, 166, 72),
            label("&Lum:", 0xFFFF, 140, 90),
            field(ID_LUM, 166, 88),
            label("&Red:", 0xFFFF, 198, 58),
            field(ID_RED, 222, 56),
            label("&Green:", 0xFFFF, 198, 74),
            field(ID_GREEN, 222, 72),
            label("Bl&ue:", 0xFFFF, 198, 90),
            field(ID_BLUE, 222, 88),
            Item {
                class: Class::Button,
                text: "&Add to Custom Colors",
                id: ID_ADD as u16,
                style: child | WS_TABSTOP | BS_PUSHBUTTON,
                ex_style: 0,
                x: 140,
                y: 108,
                cx: 108,
                cy: 14
            },
            Item {
                class: Class::Button,
                text: "OK",
                id: IDOK as u16,
                style: child | WS_TABSTOP | BS_DEFPUSHBUTTON,
                ex_style: 0,
                x: 140,
                y: 132,
                cx: 50,
                cy: 14
            },
            Item {
                class: Class::Button,
                text: "Cancel",
                id: IDCANCEL as u16,
                style: child | WS_TABSTOP | BS_PUSHBUTTON,
                ex_style: 0,
                x: 198,
                y: 132,
                cx: 50,
                cy: 14
            },
        ],
    }
}

static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Register the two window classes of the dialog once per process.
fn register_classes() {
    if REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    for (name, proc_) in
        [("PolyColorGrid", grid_proc as sys::WndProc), ("PolyColorSample", sample_proc as sys::WndProc)]
    {
        let n = sys::wz(name);
        let mut wc = [0u8; 72];
        wr(wc.as_mut_ptr(), 8, proc_ as usize);
        wr(wc.as_mut_ptr(), 24, dll_instance());
        // SAFETY: the arrow cursor of the system.
        wr(wc.as_mut_ptr(), 40, unsafe { sys::LoadCursorW(0, 32512 as *const u16) });
        wr(wc.as_mut_ptr(), 64, n.as_ptr() as usize);
        // SAFETY: a WNDCLASSW (72 bytes on x64) whose name outlives the call.
        unsafe { sys::RegisterClassW(wc.as_ptr()) };
    }
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
        sync_fields(st);
        if st.hook != 0 {
            // The hook sees the initialised dialog; lParam is the CHOOSECOLORW.
            call_hook(st.hook, hdlg, WM_INITDIALOG, w, st.cc as isize);
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

fn command(st: &mut State, id: i32, code: u32) {
    let hdlg = st.hdlg;
    match id {
        IDOK => {
            crate::set_ext_error(0);
            // SAFETY: the dialog.
            unsafe { sys::EndDialog(hdlg, 1) };
        }
        IDCANCEL => {
            crate::set_ext_error(0);
            // SAFETY: the dialog.
            unsafe { sys::EndDialog(hdlg, 0) };
        }
        ID_ADD => {
            if !st.custom.is_null() {
                let i = st.next_custom % 16;
                // SAFETY: 16 custom colours.
                unsafe { *st.custom.add(i) = st.cur };
                st.next_custom += 1;
                st.sel = Some((ID_CUSTOM, i));
                redraw_grids(st);
            }
        }
        ID_BASIC | ID_CUSTOM if code == BN_CLICKED => {
            // A grid says a cell was chosen: its colour is the current one.
            if let Some((gid, i)) = st.sel {
                st.cur = if gid == ID_BASIC { BASIC[i] } else { st.custom_color(i) };
                sync_fields(st);
                redraw_grids(st);
            }
        }
        ID_HUE | ID_SAT | ID_LUM if code == EN_CHANGE && !st.updating => {
            let h = num(hdlg, ID_HUE);
            let s = num(hdlg, ID_SAT);
            let l = num(hdlg, ID_LUM);
            st.cur = color::from_hsl(h, s, l);
            sync_fields_except(st, true);
        }
        ID_RED | ID_GREEN | ID_BLUE if code == EN_CHANGE && !st.updating => {
            let (r, g, b) = (num(hdlg, ID_RED).min(255), num(hdlg, ID_GREEN).min(255), num(hdlg, ID_BLUE).min(255));
            st.cur = color::rgb(r as u8, g as u8, b as u8);
            sync_fields_except(st, false);
        }
        _ => {}
    }
}

fn num(hdlg: H, id: i32) -> u32 {
    let t = get_text(hdlg, id);
    let mut v: u32 = 0;
    for c in t {
        if (b'0' as u16..=b'9' as u16).contains(&c) {
            v = v.saturating_mul(10).saturating_add(u32::from(c - b'0' as u16));
        }
    }
    v
}

fn put_num(hdlg: H, id: i32, v: u32) {
    let mut digits = alloc::vec::Vec::new();
    let mut n = v;
    loop {
        digits.push(b'0' as u16 + (n % 10) as u16);
        n /= 10;
        if n == 0 {
            break;
        }
    }
    digits.reverse();
    set_text(hdlg, id, &digits);
}

fn sync_fields(st: &mut State) {
    sync_fields_except(st, false);
    sync_fields_except(st, true);
}

/// Show the current colour in the boxes: after the red, green and blue boxes changed
/// (`from_rgb`: false) the hue, saturation and luminance boxes follow, and the other way round.
fn sync_fields_except(st: &mut State, hsl_changed: bool) {
    let hdlg = st.hdlg;
    st.updating = true;
    let c = st.cur;
    if hsl_changed {
        put_num(hdlg, ID_RED, c & 255);
        put_num(hdlg, ID_GREEN, c >> 8 & 255);
        put_num(hdlg, ID_BLUE, c >> 16 & 255);
    } else {
        let (h, s, l) = color::to_hsl(c);
        put_num(hdlg, ID_HUE, h);
        put_num(hdlg, ID_SAT, s);
        put_num(hdlg, ID_LUM, l);
    }
    st.updating = false;
    // SAFETY: windows of the dialog.
    unsafe {
        let s = sys::GetDlgItem(hdlg, ID_CURRENT);
        if s != 0 {
            sys::InvalidateRect(s, core::ptr::null(), 1);
        }
    }
}

fn redraw_grids(st: &State) {
    for id in [ID_BASIC, ID_CUSTOM] {
        // SAFETY: windows of the dialog.
        unsafe {
            let g = sys::GetDlgItem(st.hdlg, id);
            if g != 0 {
                sys::InvalidateRect(g, core::ptr::null(), 1);
            }
        }
    }
}

fn fill(hdc: H, rect: [i32; 4], color: u32) {
    // SAFETY: a brush made and deleted here.
    unsafe {
        let b = sys::CreateSolidBrush(color);
        sys::FillRect(hdc, rect.as_ptr(), b);
        sys::DeleteObject(b);
    }
}

fn dialog_state<'a>(hwnd: H) -> Option<&'a mut State> {
    // SAFETY: a child of the colour dialog.
    state(unsafe { sys::GetParent(hwnd) })
}

/// The cell grid: 8 columns; 6 rows of basic colours or 2 of custom ones.
unsafe extern "system" fn grid_proc(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the control's id.
    let id = unsafe { sys::GetWindowLongPtrW(hwnd, GWL_ID) } as i32;
    let rows = if id == ID_BASIC { 6 } else { 2 };
    let mut rc = [0i32; 4];
    // SAFETY: a window of ours.
    unsafe { sys::GetClientRect(hwnd, rc.as_mut_ptr()) };
    let (cw, ch) = ((rc[2] / COLS).max(1), (rc[3] / rows).max(1));
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps = [0u8; 72];
            // SAFETY: a PAINTSTRUCT of 72 bytes.
            let hdc = unsafe { sys::BeginPaint(hwnd, ps.as_mut_ptr()) };
            let st = dialog_state(hwnd);
            for i in 0..(COLS * rows) as usize {
                let (x, y) = ((i as i32 % COLS) * cw, (i as i32 / COLS) * ch);
                let c = match (&st, id) {
                    (_, ID_BASIC) => BASIC[i],
                    (Some(s), _) => s.custom_color(i),
                    _ => 0xFF_FFFF,
                };
                let selected = st.as_ref().is_some_and(|s| s.sel == Some((id, i)));
                if selected {
                    fill(hdc, [x, y, x + cw, y + ch], 0);
                    fill(hdc, [x + 2, y + 2, x + cw - 2, y + ch - 2], c);
                } else {
                    fill(hdc, [x + 1, y + 1, x + cw - 1, y + ch - 1], c);
                }
            }
            // SAFETY: ends the paint begun above.
            unsafe { sys::EndPaint(hwnd, ps.as_ptr()) };
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
            if let Some(st) = dialog_state(hwnd)
                && x >= 0
                && y >= 0
                && x / cw < COLS
                && y / ch < rows
            {
                st.sel = Some((id, ((y / ch) * COLS + x / cw) as usize));
                // The dialog reads the selection and updates itself.
                // SAFETY: the dialog is the parent.
                unsafe {
                    sys::SendMessageW(
                        sys::GetParent(hwnd),
                        WM_COMMAND,
                        id as usize | (BN_CLICKED as usize) << 16,
                        hwnd as isize,
                    )
                };
            }
            0
        }
        // SAFETY: the default procedure.
        _ => unsafe { sys::DefWindowProcW(hwnd, msg, w, l) },
    }
}

/// The sample of the current colour.
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
            let c = dialog_state(hwnd).map_or(0, |s| s.cur);
            fill(hdc, rc, c);
            // SAFETY: ends the paint begun above.
            unsafe { sys::EndPaint(hwnd, ps.as_ptr()) };
            0
        }
        // SAFETY: the default procedure.
        _ => unsafe { sys::DefWindowProcW(hwnd, msg, w, l) },
    }
}
