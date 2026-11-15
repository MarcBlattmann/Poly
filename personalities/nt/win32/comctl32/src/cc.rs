//! What the painted common controls (status bar, toolbar, header, list view, tree view) share:
//! the window and GDI calls they make, notifications, the font, colours and a small painter.
//! Structures are `#[repr(C)]` with the native pointer size, so 32-bit and 64-bit builds both
//! have the layouts of `commctrl.h`.

use alloc::vec::Vec;

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn SendMessageW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
    pub fn PostMessageW(hwnd: usize, msg: u32, w: usize, l: isize) -> i32;
    pub fn DefWindowProcW(hwnd: usize, msg: u32, w: usize, l: isize) -> isize;
    pub fn GetParent(hwnd: usize) -> usize;
    pub fn GetDlgCtrlID(hwnd: usize) -> i32;
    pub fn GetClientRect(hwnd: usize, rect: *mut i32) -> i32;
    pub fn GetWindowRect(hwnd: usize, rect: *mut i32) -> i32;
    pub fn GetWindowLongW(hwnd: usize, index: i32) -> i32;
    pub fn SetWindowLongW(hwnd: usize, index: i32, value: i32) -> i32;
    pub fn SetWindowPos(hwnd: usize, after: usize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    pub fn InvalidateRect(hwnd: usize, rect: *const i32, erase: i32) -> i32;
    pub fn GetDC(hwnd: usize) -> usize;
    pub fn ReleaseDC(hwnd: usize, hdc: usize) -> i32;
    pub fn BeginPaint(hwnd: usize, ps: *mut u8) -> usize;
    pub fn EndPaint(hwnd: usize, ps: *const u8) -> i32;
    pub fn FillRect(hdc: usize, rect: *const i32, brush: usize) -> i32;
    pub fn DrawTextW(hdc: usize, text: *const u16, count: i32, rect: *mut i32, format: u32) -> i32;
    pub fn GetSysColor(index: i32) -> u32;
    pub fn GetSysColorBrush(index: i32) -> usize;
    pub fn GetFocus() -> usize;
    pub fn SetFocus(hwnd: usize) -> usize;
    pub fn GetKeyState(vk: i32) -> i16;
    pub fn SetCapture(hwnd: usize) -> usize;
    pub fn ReleaseCapture() -> i32;
    pub fn GetCapture() -> usize;
    pub fn MapWindowPoints(from: usize, to: usize, points: *mut i32, count: u32) -> i32;
    pub fn IsWindowEnabled(hwnd: usize) -> i32;
    pub fn SetTimer(hwnd: usize, id: usize, ms: u32, proc_: usize) -> usize;
    pub fn KillTimer(hwnd: usize, id: usize) -> i32;
    pub fn GetCursorPos(p: *mut i32) -> i32;
    pub fn ScreenToClient(hwnd: usize, p: *mut i32) -> i32;
    pub fn CreateWindowExW(
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
    pub fn DestroyWindow(hwnd: usize) -> i32;
    pub fn IsWindow(hwnd: usize) -> i32;
    pub fn MoveWindow(hwnd: usize, x: i32, y: i32, w: i32, h: i32, repaint: i32) -> i32;
    pub fn ShowWindow(hwnd: usize, cmd: i32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn SelectObject(hdc: usize, obj: usize) -> usize;
    pub fn GetStockObject(index: i32) -> usize;
    pub fn SetTextColor(hdc: usize, color: u32) -> u32;
    pub fn SetBkColor(hdc: usize, color: u32) -> u32;
    pub fn SetBkMode(hdc: usize, mode: i32) -> i32;
    pub fn GetTextExtentPoint32W(hdc: usize, text: *const u16, count: i32, size: *mut i32) -> i32;
    pub fn GetTextMetricsW(hdc: usize, tm: *mut u8) -> i32;
    pub fn CreateSolidBrush(color: u32) -> usize;
    pub fn DeleteObject(h: usize) -> i32;
    pub fn CreateCompatibleDC(hdc: usize) -> usize;
    pub fn DeleteDC(hdc: usize) -> i32;
    pub fn BitBlt(dst: usize, x: i32, y: i32, w: i32, h: i32, src: usize, sx: i32, sy: i32, rop: u32) -> i32;
    pub fn GetObjectW(h: usize, size: i32, out: *mut u8) -> i32;
    pub fn SetViewportOrgEx(hdc: usize, x: i32, y: i32, old: *mut i32) -> i32;
    pub fn IntersectClipRect(hdc: usize, l: i32, t: i32, r: i32, b: i32) -> i32;
    pub fn SaveDC(hdc: usize) -> i32;
    pub fn RestoreDC(hdc: usize, saved: i32) -> i32;
}

pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WM_ENABLE: u32 = 0x000A;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_GETFONT: u32 = 0x0031;
pub const WM_NOTIFY: u32 = 0x004E;
pub const WM_NCCALCSIZE: u32 = 0x0083;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_GETDLGCODE: u32 = 0x0087;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_VSCROLL: u32 = 0x0115;
pub const WM_HSCROLL: u32 = 0x0114;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_CAPTURECHANGED: u32 = 0x0215;
pub const WM_PRINT: u32 = 0x0317;
pub const WM_PRINTCLIENT: u32 = 0x0318;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_CONTEXTMENU: u32 = 0x007B;
pub const WM_STYLECHANGED: u32 = 0x007D;
pub const WM_SETREDRAW: u32 = 0x000B;
pub const WM_SETTEXT: u32 = 0x000C;
pub const WM_GETTEXT: u32 = 0x000D;
pub const WM_GETTEXTLENGTH: u32 = 0x000E;

pub const NM_FIRST: i32 = 0;
pub const NM_OUTOFMEMORY: i32 = NM_FIRST - 1;
pub const NM_CLICK: i32 = NM_FIRST - 2;
pub const NM_DBLCLK: i32 = NM_FIRST - 3;
pub const NM_RETURN: i32 = NM_FIRST - 4;
pub const NM_RCLICK: i32 = NM_FIRST - 5;
pub const NM_RDBLCLK: i32 = NM_FIRST - 6;
pub const NM_SETFOCUS: i32 = NM_FIRST - 7;
pub const NM_KILLFOCUS: i32 = NM_FIRST - 8;
pub const NM_CUSTOMDRAW: i32 = NM_FIRST - 12;
pub const NM_HOVER: i32 = NM_FIRST - 13;

pub const COLOR_SCROLLBAR: i32 = 0;
pub const COLOR_WINDOW: i32 = 5;
pub const COLOR_WINDOWTEXT: i32 = 8;
pub const COLOR_HIGHLIGHT: i32 = 13;
pub const COLOR_HIGHLIGHTTEXT: i32 = 14;
pub const COLOR_BTNFACE: i32 = 15;
pub const COLOR_BTNSHADOW: i32 = 16;
pub const COLOR_GRAYTEXT: i32 = 17;
pub const COLOR_BTNTEXT: i32 = 18;
pub const COLOR_BTNHIGHLIGHT: i32 = 20;
pub const COLOR_3DDKSHADOW: i32 = 21;
pub const COLOR_3DLIGHT: i32 = 22;

pub const CLR_NONE: u32 = 0xFFFF_FFFF;
pub const CLR_DEFAULT: u32 = 0xFF00_0000;

pub const GWL_STYLE: i32 = -16;
pub const DEFAULT_GUI_FONT: i32 = 17;

pub const DT_LEFT: u32 = 0;
pub const DT_CENTER: u32 = 1;
pub const DT_RIGHT: u32 = 2;
pub const DT_VCENTER: u32 = 4;
pub const DT_SINGLELINE: u32 = 0x20;
pub const DT_NOPREFIX: u32 = 0x800;
pub const DT_END_ELLIPSIS: u32 = 0x8000;
pub const DT_NOCLIP: u32 = 0x100;
pub const DT_CALCRECT: u32 = 0x400;

pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_DISABLED: u32 = 0x0800_0000;

pub const VK_LBUTTON: i32 = 1;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_RETURN: u32 = 0x0D;
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
pub const VK_F2: u32 = 0x71;
pub const VK_ADD: u32 = 0x6B;
pub const VK_SUBTRACT: u32 = 0x6D;
pub const VK_MULTIPLY: u32 = 0x6A;

/// `NMHDR` (`commctrl.h`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NmHdr {
    pub hwnd_from: usize,
    pub id_from: usize,
    pub code: u32,
}

pub fn lo(v: isize) -> i32 {
    v as i16 as i32
}

pub fn hi(v: isize) -> i32 {
    (v >> 16) as i16 as i32
}

pub fn make_long(lo: i32, hi: i32) -> isize {
    ((lo as u32 & 0xFFFF) | (hi as u32) << 16) as i32 as isize
}

/// A rectangle as the four integers a `RECT` is.
pub type Rc = [i32; 4];

pub fn rc_w(r: &Rc) -> i32 {
    r[2] - r[0]
}

pub fn rc_h(r: &Rc) -> i32 {
    r[3] - r[1]
}

pub fn rc_contains(r: &Rc, x: i32, y: i32) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

pub fn client_rect(hwnd: usize) -> Rc {
    let mut r = [0i32; 4];
    // SAFETY: a RECT.
    unsafe { GetClientRect(hwnd, r.as_mut_ptr()) };
    r
}

pub fn window_rect(hwnd: usize) -> Rc {
    let mut r = [0i32; 4];
    // SAFETY: a RECT.
    unsafe { GetWindowRect(hwnd, r.as_mut_ptr()) };
    r
}

pub fn style_of(hwnd: usize) -> u32 {
    // SAFETY: a plain call.
    unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 }
}

pub fn invalidate(hwnd: usize) {
    // SAFETY: a window handle and a NULL rectangle.
    unsafe { InvalidateRect(hwnd, core::ptr::null(), 1) };
}

/// Send `WM_NOTIFY` to the parent with the structure `nm` (it starts with the `NMHDR`, which is
/// filled here). The result of the parent.
pub fn notify<T>(hwnd: usize, code: i32, nm: &mut T) -> isize {
    // SAFETY: `T` starts with an NMHDR (the callers' structures do).
    unsafe {
        let hdr = nm as *mut T as *mut NmHdr;
        (*hdr).hwnd_from = hwnd;
        (*hdr).id_from = GetDlgCtrlID(hwnd) as usize;
        (*hdr).code = code as u32;
        let parent = GetParent(hwnd);
        if parent == 0 {
            return 0;
        }
        SendMessageW(parent, WM_NOTIFY, (*hdr).id_from, nm as *mut T as isize)
    }
}

/// A notification that is only the header.
pub fn notify_plain(hwnd: usize, code: i32) -> isize {
    let mut h = NmHdr::default();
    notify(hwnd, code, &mut h)
}

pub fn sys_color(index: i32) -> u32 {
    // SAFETY: a plain call.
    unsafe { GetSysColor(index) }
}

pub fn fill(hdc: usize, r: &Rc, color: u32) {
    if rc_w(r) <= 0 || rc_h(r) <= 0 {
        return;
    }
    // SAFETY: a brush made and freed here.
    unsafe {
        let b = CreateSolidBrush(color);
        FillRect(hdc, r.as_ptr(), b);
        DeleteObject(b);
    }
}

/// A one pixel frame inside `r`.
pub fn frame(hdc: usize, r: &Rc, color: u32) {
    fill(hdc, &[r[0], r[1], r[2], r[1] + 1], color);
    fill(hdc, &[r[0], r[3] - 1, r[2], r[3]], color);
    fill(hdc, &[r[0], r[1], r[0] + 1, r[3]], color);
    fill(hdc, &[r[2] - 1, r[1], r[2], r[3]], color);
}

/// A dotted focus rectangle: every other pixel of the frame.
pub fn focus_rect(hdc: usize, r: &Rc, color: u32) {
    let (w, h) = (rc_w(r), rc_h(r));
    for x in (0..w).step_by(2) {
        fill(hdc, &[r[0] + x, r[1], r[0] + x + 1, r[1] + 1], color);
        fill(hdc, &[r[0] + x, r[3] - 1, r[0] + x + 1, r[3]], color);
    }
    for y in (0..h).step_by(2) {
        fill(hdc, &[r[0], r[1] + y, r[0] + 1, r[1] + y + 1], color);
        fill(hdc, &[r[2] - 1, r[1] + y, r[2], r[1] + y + 1], color);
    }
}

pub fn text_width(hdc: usize, text: &[u16]) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let mut sz = [0i32; 2];
    // SAFETY: a SIZE.
    unsafe { GetTextExtentPoint32W(hdc, text.as_ptr(), text.len() as i32, sz.as_mut_ptr()) };
    sz[0]
}

/// `(height, average width)` of the DC's font.
pub fn font_metrics(hdc: usize) -> (i32, i32) {
    let mut tm = [0u8; 64];
    // SAFETY: a TEXTMETRICW is 60 bytes.
    unsafe { GetTextMetricsW(hdc, tm.as_mut_ptr()) };
    let rd = |o: usize| i32::from_le_bytes([tm[o], tm[o + 1], tm[o + 2], tm[o + 3]]);
    (rd(0), rd(20))
}

/// Draw `text` in `r` with the DC's font and colours.
pub fn draw_text(hdc: usize, text: &[u16], r: &Rc, format: u32) {
    if text.is_empty() {
        return;
    }
    let mut rr = *r;
    // SAFETY: a RECT and `text.len()` units.
    unsafe { DrawTextW(hdc, text.as_ptr(), text.len() as i32, rr.as_mut_ptr(), format) };
}

/// A DC for measuring with a control's font (released by [`Measure::drop`]).
pub struct Measure {
    hwnd: usize,
    pub hdc: usize,
    old: usize,
}

impl Measure {
    pub fn new(hwnd: usize, font: usize) -> Measure {
        // SAFETY: a plain call and a font handle of the control (0: the GUI font).
        unsafe {
            let hdc = GetDC(hwnd);
            let f = if font != 0 { font } else { GetStockObject(DEFAULT_GUI_FONT) };
            let old = SelectObject(hdc, f);
            Measure { hwnd, hdc, old }
        }
    }
}

impl Drop for Measure {
    fn drop(&mut self) {
        // SAFETY: the DC and font taken in `new`.
        unsafe {
            SelectObject(self.hdc, self.old);
            ReleaseDC(self.hwnd, self.hdc);
        }
    }
}

/// `n` units of text at `p`, or NUL-terminated for `n < 0`.
pub unsafe fn wide_at(p: usize, n: i32) -> Vec<u16> {
    if p == 0 || p == usize::MAX {
        return Vec::new();
    }
    // SAFETY: the caller's string.
    unsafe { poly_win32_gui::wide(p as *const u16, n) }
}

/// Copy `text` and a NUL into the caller's buffer of `cap` units; the units copied (without the NUL).
pub unsafe fn put_text(buf: usize, cap: usize, text: &[u16]) -> usize {
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

pub unsafe fn rd<T: Copy>(p: usize, off: usize) -> T {
    // SAFETY: inside a structure of the caller's.
    unsafe { ((p + off) as *const T).read_unaligned() }
}

pub unsafe fn wr<T: Copy>(p: usize, off: usize, v: T) {
    // SAFETY: inside a structure of the caller's.
    unsafe { ((p + off) as *mut T).write_unaligned(v) }
}

/// Whether a window of the control is under the text colour `COLOR_*` as the system has it for
/// the client background: the colour a list or tree view clears with.
pub fn window_color() -> u32 {
    sys_color(COLOR_WINDOW)
}

/// Ask the parent for the notification format (`CCM_GETUNICODEFORMAT`): the controls here always
/// speak Unicode.
pub fn enabled(hwnd: usize) -> bool {
    // SAFETY: a plain call.
    unsafe { IsWindowEnabled(hwnd) != 0 }
}
