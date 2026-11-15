//! `ToolbarWindow32`: buttons, bitmaps and image lists, layout, painting and clicks
//! (`specs/windows/comctl32.dll/ToolbarWindow32.yaml`, `CreateToolbarEx.yaml`; Learn, "Toolbar
//! Control Reference"). The model is `poly_comctl::toolbar`.

use crate::cc::*;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use poly_comctl::toolbar::{self as tb, Button, Toolbar};
use poly_win32_gui::SpinLock;

/// `TBBUTTON` (`commctrl.h`), native layout.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TbButton {
    bitmap: i32,
    command: i32,
    state: u8,
    style: u8,
    data: usize,
    string: isize,
}

impl From<TbButton> for Button {
    fn from(b: TbButton) -> Button {
        Button { image: b.bitmap, command: b.command, state: b.state, style: b.style, data: b.data, string: b.string }
    }
}

impl From<Button> for TbButton {
    fn from(b: Button) -> TbButton {
        TbButton { bitmap: b.image, command: b.command, state: b.state, style: b.style, data: b.data, string: b.string }
    }
}

/// `NMTOOLBARW` up to what `TBN_DELETINGBUTTON` and `TBN_DROPDOWN` fill.
#[repr(C)]
struct NmToolbar {
    hdr: NmHdr,
    item: i32,
    button: TbButton,
    cch_text: i32,
    psz_text: usize,
    rect: Rc,
}

const TBN_FIRST: i32 = -700;
const TBN_DROPDOWN: i32 = TBN_FIRST - 10;
const TBN_DELETINGBUTTON: i32 = TBN_FIRST - 15;
const TBSTYLE_LIST: u32 = 0x1000;
const TBSTYLE_TOOLTIPS: u32 = 0x100;
const TBSTYLE_EX_DRAWDDARROWS: u32 = 1;

struct State {
    t: Toolbar,
    strings: Vec<Vec<u16>>,
    images: [usize; 3],
    font: usize,
    hot: i32,
    down: i32,
    pad: (i32, i32),
}

static STATES: SpinLock<BTreeMap<usize, State>> = SpinLock::new(BTreeMap::new());

fn with<R>(h: usize, f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATES.with(|m| m.get_mut(&h).map(f))
}

pub fn on_create(hwnd: usize) {
    let st = State { t: Toolbar::default(), strings: Vec::new(), images: [0; 3], font: 0, hot: -1, down: -1, pad: (tb::PAD_X, tb::PAD_Y) };
    STATES.with(|m| m.insert(hwnd, st));
    sync_style(hwnd);
}

pub fn on_destroy(hwnd: usize) {
    STATES.with(|m| m.remove(&hwnd));
}

/// The style bits the model needs (`TBSTYLE_*`, `CCS_NODIVIDER`).
fn sync_style(hwnd: usize) {
    let style = style_of(hwnd);
    with(hwnd, |s| s.t.style = style);
}

fn client_width(hwnd: usize) -> i32 {
    rc_w(&client_rect(hwnd))
}

fn button_text(s: &State, b: &Button) -> Vec<u16> {
    if b.string >= 0 && (b.string as usize) < s.strings.len() {
        return s.strings[b.string as usize].clone();
    }
    if b.string > 0xFFFF {
        // SAFETY: a string the program gave (`TBBUTTON.iString` as a pointer).
        return unsafe { wide_at(b.string as usize, -1) };
    }
    Vec::new()
}

fn draw(hwnd: usize, hdc: usize) {
    let c = client_rect(hwnd);
    let (face, shadow, light, text_color, gray) = (
        sys_color(COLOR_BTNFACE),
        sys_color(COLOR_BTNSHADOW),
        sys_color(COLOR_BTNHIGHLIGHT),
        sys_color(COLOR_BTNTEXT),
        sys_color(COLOR_GRAYTEXT),
    );
    fill(hdc, &c, face);
    let width = rc_w(&c);
    let Some((buttons, rects, strings, images, font, hot, down, ex, style, bsize, bmps)) = with(hwnd, |s| {
        let texts: Vec<Vec<u16>> = s.t.buttons.iter().map(|b| button_text(s, b)).collect();
        (
            s.t.buttons.clone(),
            s.t.layout(width),
            texts,
            s.images,
            s.font,
            s.hot,
            s.down,
            s.t.ex_style,
            s.t.style,
            s.t.bitmap_size,
            s.t.bitmaps.clone(),
        )
    }) else {
        return;
    };
    let _ = ex;
    // SAFETY: the DC of the control and a font.
    let old = unsafe { SelectObject(hdc, if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) }) };
    // SAFETY: the DC.
    unsafe { SetBkMode(hdc, 1) };
    for (i, b) in buttons.iter().enumerate() {
        let r = rects[i];
        if r[2] <= r[0] {
            continue;
        }
        if b.is_separator() {
            let x = (r[0] + r[2]) / 2;
            fill(hdc, &[x - 1, r[1] + 3, x, r[3] - 3], shadow);
            fill(hdc, &[x, r[1] + 3, x + 1, r[3] - 3], light);
            continue;
        }
        let enabled = b.state & tb::TBSTATE_ENABLED != 0;
        let checked = b.state & tb::TBSTATE_CHECKED != 0;
        let pressed = b.state & tb::TBSTATE_PRESSED != 0 || (down == i as i32 && enabled);
        // A pressed or checked button is sunken, the one under the mouse is framed, the rest flat.
        if pressed || checked {
            fill(hdc, &r, if checked && !pressed { sys_color(COLOR_3DLIGHT) } else { sys_color(COLOR_3DLIGHT) });
            frame(hdc, &r, shadow);
        } else if hot == i as i32 && enabled {
            frame(hdc, &r, shadow);
        }
        let off = i32::from(pressed);
        let (iw, ih) = bsize;
        let il = if !enabled && images[2] != 0 {
            images[2]
        } else if hot == i as i32 && images[1] != 0 {
            images[1]
        } else {
            images[0]
        };
        let text = &strings[i];
        let list = style & TBSTYLE_LIST != 0;
        let img_x = if text.is_empty() || list { r[0] + (if text.is_empty() { (rc_w(&r) - iw) / 2 } else { 3 }) } else { r[0] + (rc_w(&r) - iw) / 2 };
        let img_y = r[1] + if text.is_empty() || list { (rc_h(&r) - ih) / 2 } else { 3 };
        let has_image = b.image >= 0;
        if has_image {
            if il != 0 {
                // SAFETY: the image list of the program.
                unsafe { crate::imagelist::ImageList_Draw(il, b.image, hdc, img_x + off, img_y + off, 1) };
            } else {
                draw_strip_image(hdc, &bmps, b.image, img_x + off, img_y + off, iw, ih);
            }
        }
        if !text.is_empty() {
            // SAFETY: the DC.
            unsafe { SetTextColor(hdc, if enabled { text_color } else { gray }) };
            let tr = if list { [img_x + iw + 4 + off, r[1] + off, r[2] - 2, r[3] + off] } else { [r[0] + off, img_y + ih + 1 + off, r[2] + off, r[3] - 1 + off] };
            draw_text(hdc, text, &tr, DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS | if list { DT_LEFT | DT_VCENTER } else { DT_CENTER });
        }
    }
    // SAFETY: the font taken above.
    unsafe { SelectObject(hdc, old) };
}

/// The image `index` of the bitmaps `TB_ADDBITMAP` added (strips of `iw` x `ih` images).
fn draw_strip_image(hdc: usize, bitmaps: &[(usize, i32)], index: i32, x: i32, y: i32, iw: i32, ih: i32) {
    let mut first = 0;
    for &(bmp, count) in bitmaps {
        if index >= first && index < first + count {
            // SAFETY: a memory DC made and freed here, with the program's bitmap selected.
            unsafe {
                let mem = CreateCompatibleDC(hdc);
                let old = SelectObject(mem, bmp);
                BitBlt(hdc, x, y, iw, ih, mem, (index - first) * iw, 0, 0x00CC_0020);
                SelectObject(mem, old);
                DeleteDC(mem);
            }
            return;
        }
        first += count;
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

/// The toolbar's top margin in the window: the divider is a non-client part.
fn divider(hwnd: usize) -> i32 {
    with(hwnd, |s| {
        s.t.style = style_of(hwnd);
        s.t.divider()
    })
    .unwrap_or(0)
}

fn hit(hwnd: usize, x: i32, y: i32) -> i32 {
    let w = client_width(hwnd);
    with(hwnd, |s| s.t.hit_test(w, x, y)).unwrap_or(-1)
}

fn set_state_bit(hwnd: usize, command: usize, bit: u8, on: bool) -> isize {
    let ok = with(hwnd, |s| match s.t.command_to_index(command as i32) {
        i if i >= 0 => {
            if bit == tb::TBSTATE_CHECKED {
                s.t.check(i as usize, on);
            } else if on {
                s.t.buttons[i as usize].state |= bit;
            } else {
                s.t.buttons[i as usize].state &= !bit;
            }
            true
        }
        _ => false,
    })
    .unwrap_or(false);
    if ok {
        invalidate(hwnd);
    }
    isize::from(ok)
}

fn is_state(hwnd: usize, command: usize, bit: u8) -> isize {
    with(hwnd, |s| match s.t.command_to_index(command as i32) {
        i if i >= 0 => isize::from(s.t.buttons[i as usize].state & bit != 0),
        _ => -1,
    })
    .unwrap_or(-1)
}

const TB_ENABLEBUTTON: u32 = 0x401;
const TB_CHECKBUTTON: u32 = 0x402;
const TB_PRESSBUTTON: u32 = 0x403;
const TB_HIDEBUTTON: u32 = 0x404;
const TB_INDETERMINATE: u32 = 0x405;
const TB_ISBUTTONENABLED: u32 = 0x409;
const TB_ISBUTTONCHECKED: u32 = 0x40A;
const TB_ISBUTTONPRESSED: u32 = 0x40B;
const TB_ISBUTTONHIDDEN: u32 = 0x40C;
const TB_ISBUTTONINDETERMINATE: u32 = 0x40D;
const TB_SETSTATE: u32 = 0x411;
const TB_GETSTATE: u32 = 0x412;
const TB_ADDBITMAP: u32 = 0x413;
const TB_ADDBUTTONSA: u32 = 0x414;
const TB_INSERTBUTTONA: u32 = 0x415;
const TB_DELETEBUTTON: u32 = 0x416;
const TB_GETBUTTON: u32 = 0x417;
const TB_BUTTONCOUNT: u32 = 0x418;
const TB_COMMANDTOINDEX: u32 = 0x419;
const TB_ADDSTRINGA: u32 = 0x41C;
const TB_GETITEMRECT: u32 = 0x41D;
const TB_BUTTONSTRUCTSIZE: u32 = 0x41E;
const TB_SETBUTTONSIZE: u32 = 0x41F;
const TB_SETBITMAPSIZE: u32 = 0x420;
const TB_AUTOSIZE: u32 = 0x421;
const TB_GETTOOLTIPS: u32 = 0x423;
const TB_SETTOOLTIPS: u32 = 0x424;
const TB_SETPARENT: u32 = 0x425;
const TB_SETROWS: u32 = 0x427;
const TB_GETROWS: u32 = 0x428;
const TB_SETCMDID: u32 = 0x42A;
const TB_GETBITMAPFLAGS: u32 = 0x429;
const TB_SETIMAGELIST: u32 = 0x430;
const TB_GETIMAGELIST: u32 = 0x431;
const TB_GETRECT: u32 = 0x433;
const TB_SETHOTIMAGELIST: u32 = 0x434;
const TB_GETHOTIMAGELIST: u32 = 0x435;
const TB_SETDISABLEDIMAGELIST: u32 = 0x436;
const TB_GETDISABLEDIMAGELIST: u32 = 0x437;
const TB_SETSTYLE: u32 = 0x438;
const TB_GETSTYLE: u32 = 0x439;
const TB_GETBUTTONSIZE: u32 = 0x43A;
const TB_SETBUTTONWIDTH: u32 = 0x43B;
const TB_GETTEXTROWS: u32 = 0x43D;
const TB_INSERTBUTTONW: u32 = 0x443;
const TB_ADDBUTTONSW: u32 = 0x444;
const TB_HITTEST: u32 = 0x445;
const TB_GETHOTITEM: u32 = 0x447;
const TB_SETHOTITEM: u32 = 0x448;
const TB_ADDSTRINGW: u32 = 0x44D;
const TB_GETMAXSIZE: u32 = 0x453;
const TB_SETEXTENDEDSTYLE: u32 = 0x454;
const TB_GETEXTENDEDSTYLE: u32 = 0x455;
const TB_GETPADDING: u32 = 0x456;
const TB_SETPADDING: u32 = 0x457;
const TB_SETMAXTEXTROWS: u32 = 0x43C;
const CCM_SETUNICODEFORMAT: u32 = 0x2005;
const CCM_GETUNICODEFORMAT: u32 = 0x2006;
const CCM_SETWINDOWTHEME: u32 = 0x200B;

fn insert_buttons(hwnd: usize, at: i32, count: usize, p: usize) -> bool {
    if p == 0 {
        return false;
    }
    let size = core::mem::size_of::<TbButton>();
    let mut index = at;
    for i in 0..count {
        // SAFETY: `count` TBBUTTON structures of the caller's.
        let b: TbButton = unsafe { rd(p, i * size) };
        let ok = with(hwnd, |s| s.t.insert(index, b.into())).is_some();
        if !ok {
            return false;
        }
        if index >= 0 {
            index += 1;
        }
    }
    invalidate(hwnd);
    true
}

/// The window procedure of the class.
pub fn proc(hwnd: usize, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: the messages' pointers are the caller's structures (`commctrl.h`).
    unsafe {
        match msg {
            WM_SIZE => 0,
            WM_NCCALCSIZE => {
                let d = divider(hwnd);
                if d > 0 {
                    // wParam TRUE: NCCALCSIZE_PARAMS (the proposed rectangle first), else a RECT.
                    let top = l as usize + 4;
                    wr(top, 0, rd::<i32>(top, 0) + d);
                }
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
            WM_STYLECHANGED => {
                sync_style(hwnd);
                invalidate(hwnd);
                0
            }
            WM_SETFONT => {
                with(hwnd, |s| s.font = w);
                invalidate(hwnd);
                0
            }
            WM_GETFONT => with(hwnd, |s| s.font).unwrap_or(0) as isize,
            WM_MOUSEMOVE => {
                let (x, y) = (lo(l), hi(l));
                let i = hit(hwnd, x, y);
                let changed = with(hwnd, |s| core::mem::replace(&mut s.hot, i.max(-1)) != i.max(-1)).unwrap_or(false);
                if changed {
                    invalidate(hwnd);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let i = hit(hwnd, lo(l), hi(l));
                if i >= 0 && with(hwnd, |s| s.t.buttons[i as usize].state & tb::TBSTATE_ENABLED != 0).unwrap_or(false) {
                    with(hwnd, |s| s.down = i);
                    SetCapture(hwnd);
                    invalidate(hwnd);
                }
                0
            }
            WM_LBUTTONUP => {
                let down = with(hwnd, |s| core::mem::replace(&mut s.down, -1)).unwrap_or(-1);
                if down >= 0 {
                    ReleaseCapture();
                    let i = hit(hwnd, lo(l), hi(l));
                    if i == down {
                        let command = with(hwnd, |s| {
                            let b = s.t.buttons[i as usize];
                            if b.style & tb::TBSTYLE_CHECK != 0 {
                                let on = b.state & tb::TBSTATE_CHECKED == 0;
                                s.t.check(i as usize, on);
                            }
                            b.command
                        })
                        .unwrap_or(0);
                        let parent = GetParent(hwnd);
                        if parent != 0 {
                            SendMessageW(parent, WM_COMMAND, command as u16 as usize, hwnd as isize);
                        }
                    }
                    invalidate(hwnd);
                }
                0
            }
            TB_ENABLEBUTTON => set_state_bit(hwnd, w, tb::TBSTATE_ENABLED, lo(l) != 0),
            TB_CHECKBUTTON => set_state_bit(hwnd, w, tb::TBSTATE_CHECKED, lo(l) != 0),
            TB_PRESSBUTTON => set_state_bit(hwnd, w, tb::TBSTATE_PRESSED, lo(l) != 0),
            TB_HIDEBUTTON => set_state_bit(hwnd, w, tb::TBSTATE_HIDDEN, lo(l) != 0),
            TB_INDETERMINATE => set_state_bit(hwnd, w, tb::TBSTATE_INDETERMINATE, lo(l) != 0),
            TB_ISBUTTONENABLED => is_state(hwnd, w, tb::TBSTATE_ENABLED),
            TB_ISBUTTONCHECKED => is_state(hwnd, w, tb::TBSTATE_CHECKED),
            TB_ISBUTTONPRESSED => is_state(hwnd, w, tb::TBSTATE_PRESSED),
            TB_ISBUTTONHIDDEN => is_state(hwnd, w, tb::TBSTATE_HIDDEN),
            TB_ISBUTTONINDETERMINATE => is_state(hwnd, w, tb::TBSTATE_INDETERMINATE),
            TB_GETSTATE => with(hwnd, |s| match s.t.command_to_index(w as i32) {
                i if i >= 0 => s.t.buttons[i as usize].state as isize,
                _ => -1,
            })
            .unwrap_or(-1),
            TB_SETSTATE => {
                let ok = with(hwnd, |s| match s.t.command_to_index(w as i32) {
                    i if i >= 0 => {
                        s.t.buttons[i as usize].state = lo(l) as u8;
                        true
                    }
                    _ => false,
                })
                .unwrap_or(false);
                invalidate(hwnd);
                isize::from(ok)
            }
            TB_ADDBITMAP => {
                // TBADDBITMAP { HINSTANCE hInst; UINT_PTR nID }: a bitmap handle when hInst is NULL.
                let (inst, id): (usize, usize) = (rd(l as usize, 0), rd(l as usize, core::mem::size_of::<usize>()));
                if inst == 0 && id != 0 {
                    return with(hwnd, |s| s.t.add_bitmap(id, w as i32)).unwrap_or(-1) as isize;
                }
                // The standard images the system has (`IDB_STD_SMALL_COLOR`): not here.
                with(hwnd, |s| s.t.add_bitmap(0, w as i32)).unwrap_or(-1) as isize
            }
            TB_ADDBUTTONSA | TB_ADDBUTTONSW => {
                let at = with(hwnd, |s| s.t.buttons.len() as i32).unwrap_or(0);
                // The new buttons go to the end: `insert` with the count as index keeps their order.
                let size = core::mem::size_of::<TbButton>();
                let mut ok = l != 0;
                for i in 0..w {
                    if !ok {
                        break;
                    }
                    let b: TbButton = rd(l as usize, i * size);
                    ok = with(hwnd, |s| s.t.insert(at + i as i32, b.into())).is_some();
                }
                invalidate(hwnd);
                isize::from(ok)
            }
            TB_INSERTBUTTONA | TB_INSERTBUTTONW => {
                let ok = insert_buttons(hwnd, w as i32, 1, l as usize);
                isize::from(ok)
            }
            TB_DELETEBUTTON => {
                let removed = with(hwnd, |s| {
                    if w < s.t.buttons.len() { Some((s.t.buttons[w], s.t.buttons.len())) } else { None }
                })
                .flatten();
                let Some((b, _)) = removed else { return 0 };
                let mut nm = NmToolbar { hdr: NmHdr::default(), item: b.command, button: b.into(), cch_text: 0, psz_text: 0, rect: [0; 4] };
                notify(hwnd, TBN_DELETINGBUTTON, &mut nm);
                with(hwnd, |s| s.t.buttons.remove(w));
                invalidate(hwnd);
                1
            }
            TB_GETBUTTON => {
                let b = with(hwnd, |s| s.t.buttons.get(w).copied()).flatten();
                match b {
                    Some(b) if l != 0 => {
                        let out: TbButton = b.into();
                        wr(l as usize, 0, out);
                        1
                    }
                    _ => 0,
                }
            }
            TB_BUTTONCOUNT => with(hwnd, |s| s.t.buttons.len()).unwrap_or(0) as isize,
            TB_COMMANDTOINDEX => with(hwnd, |s| s.t.command_to_index(w as i32)).unwrap_or(-1) as isize,
            TB_ADDSTRINGA | TB_ADDSTRINGW => {
                let mut texts: Vec<Vec<u16>> = Vec::new();
                if w == 0 && l != 0 {
                    let mut cur: Vec<u16> = Vec::new();
                    let mut at = l as usize;
                    loop {
                        let c: u16 = if msg == TB_ADDSTRINGW { rd::<u16>(at, 0) } else { u16::from(rd::<u8>(at, 0)) };
                        at += if msg == TB_ADDSTRINGW { 2 } else { 1 };
                        if c == 0 {
                            if cur.is_empty() {
                                break;
                            }
                            texts.push(core::mem::take(&mut cur));
                        } else {
                            cur.push(c);
                        }
                    }
                } else {
                    return -1;
                }
                with(hwnd, |s| {
                    let first = s.strings.len() as isize;
                    s.strings.extend(texts);
                    first
                })
                .unwrap_or(-1)
            }
            TB_GETITEMRECT | TB_GETRECT => {
                let width = client_width(hwnd);
                let index = if msg == TB_GETRECT { with(hwnd, |s| s.t.command_to_index(w as i32)).unwrap_or(-1) as isize } else { w as isize };
                let r = with(hwnd, |s| {
                    if index < 0 { return None }
                    let rects = s.t.layout(width);
                    rects.get(index as usize).copied()
                })
                .flatten();
                match r {
                    Some(r) if l != 0 => {
                        for (i, v) in r.iter().enumerate() {
                            wr(l as usize, 4 * i, *v);
                        }
                        isize::from(r[2] > r[0])
                    }
                    _ => 0,
                }
            }
            TB_BUTTONSTRUCTSIZE => {
                with(hwnd, |s| s.t.struct_size = w as u32);
                0
            }
            TB_SETBUTTONSIZE => {
                with(hwnd, |s| s.t.button_size = (lo(l), hi(l)));
                invalidate(hwnd);
                1
            }
            TB_SETBITMAPSIZE => {
                with(hwnd, |s| s.t.set_bitmap_size(lo(l), hi(l)));
                invalidate(hwnd);
                1
            }
            TB_GETBUTTONSIZE => with(hwnd, |s| make_long(s.t.button_size.0, s.t.button_size.1)).unwrap_or(0),
            TB_SETBUTTONWIDTH => {
                with(hwnd, |s| s.t.button_size.0 = lo(l).max(hi(l)));
                1
            }
            TB_AUTOSIZE => {
                // The toolbar sizes itself to the parent's width and its rows (not at all with CCS_NORESIZE).
                0
            }
            TB_GETTOOLTIPS => 0,
            TB_SETTOOLTIPS | TB_SETPARENT | TB_SETCMDID | TB_SETROWS | TB_SETMAXTEXTROWS => 0,
            TB_GETROWS => {
                let width = client_width(hwnd);
                with(hwnd, |s| s.t.row_count(width)).unwrap_or(1) as isize
            }
            TB_GETTEXTROWS => 0,
            TB_GETBITMAPFLAGS => 0,
            TB_SETIMAGELIST | TB_SETHOTIMAGELIST | TB_SETDISABLEDIMAGELIST => {
                let slot = match msg {
                    TB_SETIMAGELIST => 0,
                    TB_SETHOTIMAGELIST => 1,
                    _ => 2,
                };
                let old = with(hwnd, |s| core::mem::replace(&mut s.images[slot], l as usize)).unwrap_or(0);
                if l != 0 && slot == 0 {
                    let (mut cx, mut cy) = (0, 0);
                    crate::imagelist::ImageList_GetIconSize(l as usize, &mut cx, &mut cy);
                    if cx > 0 {
                        with(hwnd, |s| s.t.set_bitmap_size(cx, cy));
                    }
                }
                invalidate(hwnd);
                old as isize
            }
            TB_GETIMAGELIST => with(hwnd, |s| s.images[0]).unwrap_or(0) as isize,
            TB_GETHOTIMAGELIST => with(hwnd, |s| s.images[1]).unwrap_or(0) as isize,
            TB_GETDISABLEDIMAGELIST => with(hwnd, |s| s.images[2]).unwrap_or(0) as isize,
            TB_SETSTYLE => {
                // The TBSTYLE_ bits (the low word of the window style).
                let old = style_of(hwnd);
                SetWindowLongW(hwnd, GWL_STYLE, ((old & 0xFFFF_0000) | (l as u32 & 0xFFFF)) as i32);
                sync_style(hwnd);
                0
            }
            TB_GETSTYLE => (style_of(hwnd) & 0xFFFF) as isize,
            TB_SETEXTENDEDSTYLE => with(hwnd, |s| core::mem::replace(&mut s.t.ex_style, l as u32)).unwrap_or(0) as isize,
            TB_GETEXTENDEDSTYLE => with(hwnd, |s| s.t.ex_style).unwrap_or(0) as isize,
            TB_GETPADDING => with(hwnd, |s| make_long(s.pad.0, s.pad.1)).unwrap_or(0),
            TB_SETPADDING => with(hwnd, |s| {
                let old = make_long(s.pad.0, s.pad.1);
                s.pad = (lo(l), hi(l));
                old
            })
            .unwrap_or(0),
            TB_GETMAXSIZE => {
                let width = client_width(hwnd);
                let (mx, my) = with(hwnd, |s| s.t.max_size(width)).unwrap_or((0, 0));
                if l != 0 {
                    wr(l as usize, 0, mx);
                    wr(l as usize, 4, my);
                }
                1
            }
            TB_HITTEST => hit(hwnd, rd::<i32>(l as usize, 0), rd::<i32>(l as usize, 4)) as isize,
            TB_GETHOTITEM => with(hwnd, |s| s.hot).unwrap_or(-1) as isize,
            TB_SETHOTITEM => with(hwnd, |s| core::mem::replace(&mut s.hot, w as i32)).unwrap_or(-1) as isize,
            CCM_SETUNICODEFORMAT => 1,
            CCM_GETUNICODEFORMAT => 1,
            CCM_SETWINDOWTHEME => 0,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

#[allow(dead_code)]
const _UNUSED: (u32, u32, u32, i32) = (TBSTYLE_TOOLTIPS, TBSTYLE_EX_DRAWDDARROWS, 0, TBN_DROPDOWN);
