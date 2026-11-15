//! Chrome's uses of CreateRectRgnIndirect: a clip region for painting
//! (SelectClipRgn into a 32-bit DIB DC; the PatBlt covers exactly the
//! rectangle, the region may be deleted right after it is selected,
//! ExtSelectClipRgn with RGN_AND and RGN_DIFF against a second rectangle,
//! SelectClipRgn(NULL) resets) and a window region (SetWindowRgn with a
//! rectangle region, GetWindowRgnBox returns the box in window coordinates,
//! also for a reversed rectangle, and a NULL region removes it). The
//! window is at (40,40) and destroyed at the end. After SetWindowRgn the
//! system owns the region handle, so it is not used again.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    wnd_proc: WNDPROC,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HINSTANCE,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: HBRUSH,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    time: u32,
    pt: POINT,
    private: u32,
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// wingdi.h (one RGBQUAD as a u32)
#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 1],
}

const WM_NCCREATE: u32 = 0x0081;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const QS_KEY: u32 = 0x0001;
const QS_MOUSEMOVE: u32 = 0x0002;
const QS_MOUSEBUTTON: u32 = 0x0004;
const QS_POSTMESSAGE: u32 = 0x0008;
const QS_TIMER: u32 = 0x0010;
const QS_PAINT: u32 = 0x0020;
const QS_SENDMESSAGE: u32 = 0x0040;
const QS_HOTKEY: u32 = 0x0080;
const QS_RAWINPUT: u32 = 0x0400;
const QS_INPUT: u32 = QS_MOUSEMOVE | QS_MOUSEBUTTON | QS_KEY | QS_RAWINPUT;
const QS_ALLEVENTS: u32 = QS_INPUT | QS_POSTMESSAGE | QS_TIMER | QS_PAINT | QS_HOTKEY;
const QS_ALLINPUT: u32 = QS_ALLEVENTS | QS_SENDMESSAGE;
const BLACKNESS: u32 = 0x0000_0042;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
}

/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Register a class with `proc_` (and no menu, icon or cursor).
fn register(name: &[u16], proc_: WNDPROC, style: u32, wnd_extra: i32, brush: HBRUSH) -> u16 {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        wnd_proc: proc_,
        cls_extra: 0,
        wnd_extra,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: brush,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW; the name is NUL-terminated and static.
    unsafe { RegisterClassExW(&wc) }
}

/// CreateWindowExW with this executable's instance.
fn create(
    ex: u32,
    class: &[u16],
    name: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    // SAFETY: NUL-terminated names; the parent is a window or NULL.
    unsafe {
        CreateWindowExW(
            ex,
            class.as_ptr(),
            if name.is_empty() { core::ptr::null() } else { name.as_ptr() },
            style,
            x,
            y,
            w,
            h,
            parent,
            menu as HMENU,
            instance(),
            core::ptr::null_mut(),
        )
    }
}

/// Dispatch every waiting message.
fn pump() -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable; it is the message just removed.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        n += 1;
    }
    n
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// The tag the next window created by `create_tagged` gets.
static NEXT_TAG: Global<u8> = Global::new(b'?');

/// `create` for a window whose procedure logs under `tag`.
fn create_tagged(
    tag: u8,
    ex: u32,
    class: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    *NEXT_TAG.get() = tag;
    let hwnd = create(ex, class, &[], style, x, y, w, h, parent, menu);
    *NEXT_TAG.get() = b'?';
    hwnd
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// A memory DC with a 32-bit top-down (or bottom-up) DIB section selected:
/// (dc, bitmap, pixels). Pixels are 0x00RRGGBB, row by row from the top
/// for top-down.
fn dib32(w: i32, h: i32, top_down: bool) -> (HDC, HBITMAP, *mut u32) {
    let info = BITMAPINFO {
        header: BITMAPINFOHEADER {
            size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            width: w,
            height: if top_down { -h } else { h },
            planes: 1,
            bit_count: 32,
            compression: BI_RGB,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(dc, bmp);
        (dc, bmp, bits as *mut u32)
    }
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// Set every pixel of a w x h 32-bit DIB to v.
fn fill_bits(bits: *mut u32, w: i32, h: i32, v: u32) {
    for i in 0..(w * h) as usize {
        // SAFETY: inside the DIB.
        unsafe { *bits.add(i) = v };
    }
}

/// The bounding box and count of pixels that are not `bg`.
fn ink(bits: *mut u32, w: i32, h: i32, bg: u32) -> (i32, i32, i32, i32, u32) {
    let (mut l, mut tp, mut r, mut b, mut n) = (i32::MAX, i32::MAX, -1, -1, 0);
    for y in 0..h {
        for x in 0..w {
            if px(bits, w, x, y) != bg {
                l = l.min(x);
                tp = tp.min(y);
                r = r.max(x + 1);
                b = b.max(y + 1);
                n += 1;
            }
        }
    }
    if n == 0 { (0, 0, 0, 0, 0) } else { (l, tp, r, b, n) }
}

type HRGN = *mut c_void;
/// wingdi.h: RGNDATAHEADER followed by up to 64 rectangles.
#[repr(C)]
struct RGNDATA64 {
    size: u32,
    kind: u32,
    count: u32,
    rgn_size: u32,
    bound: RECT,
    rects: [RECT; 64],
}

const RGN_AND: i32 = 1;
const RGN_DIFF: i32 = 4;
/// Dispatch everything waiting, then keep waiting up to `ms` for anything
/// further (input arrives asynchronously), dispatching as it comes.
fn settle(ms: u32) -> u32 {
    let mut n = pump();
    // SAFETY: plain timing call.
    let start = unsafe { GetTickCount() };
    loop {
        // SAFETY: plain timing call.
        let elapsed = unsafe { GetTickCount() }.wrapping_sub(start);
        if elapsed >= ms {
            break;
        }
        // SAFETY: no handles; wakes on any input or message.
        unsafe { MsgWaitForMultipleObjects(0, core::ptr::null(), 0, ms - elapsed, QS_ALLINPUT) };
        n += pump();
    }
    n
}

/// The rectangles of a region, "l,t,r,b" joined with " + " (at most 64),
/// or "empty".
fn rgn_text(rgn: HRGN, out: &mut Buf<300>) {
    let mut d: RGNDATA64 = zeroed();
    // SAFETY: d is large enough for the header and 64 rectangles.
    let n = unsafe { GetRegionData(rgn, core::mem::size_of::<RGNDATA64>() as u32, &mut d) };
    if n == 0 {
        let _ = out.write_str("(no data)");
        return;
    }
    if d.count == 0 {
        let _ = out.write_str("empty");
        return;
    }
    for i in 0..(d.count as usize).min(64) {
        let r = d.rects[i];
        if i > 0 {
            let _ = out.write_str(" + ");
        }
        let _ = write!(out, "{},{},{},{}", r.left, r.top, r.right, r.bottom);
    }
}

/// "l,t,r,b" of a rectangle.
struct R(RECT);

impl core::fmt::Display for R {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{},{},{},{}", self.0.left, self.0.top, self.0.right, self.0.bottom)
    }
}

/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

/// A window procedure that only remembers the window's tag (the letter the
/// next `mkw` asked for) in GWLP_USERDATA and does the default handling.
unsafe extern "system" fn proc_tag(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: the window is being created on this thread.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, *NEXT_TAG.get() as isize) };
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Register the class `Tagged` of `mkw`.
fn tag_register() {
    register(wz!("Tagged"), Some(proc_tag), 0, 0, core::ptr::null_mut());
}

/// A window of the class `Tagged` with the tag letter `tag` (children get
/// the id 1).
fn mkw(tag: u8, ex: u32, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND) -> HWND {
    create_tagged(tag, ex, wz!("Tagged"), style, x, y, w, h, parent, if style & WS_CHILD != 0 { 1 } else { 0 })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetWindowRgnBox(hwnd: HWND, rect: *mut RECT) -> i32;
    fn SetWindowRgn(hwnd: HWND, rgn: HRGN, redraw: BOOL) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetClipBox(hdc: HDC, rect: *mut RECT) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
    fn PatBlt(hdc: HDC, x: i32, y: i32, w: i32, h: i32, rop: u32) -> BOOL;
    fn GdiFlush() -> BOOL;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> HRGN;
    fn CreateRectRgnIndirect(rect: *const RECT) -> HRGN;
    fn SelectClipRgn(hdc: HDC, rgn: HRGN) -> i32;
    fn ExtSelectClipRgn(hdc: HDC, rgn: HRGN, mode: i32) -> i32;
    fn GetClipRgn(hdc: HDC, rgn: HRGN) -> i32;
    fn GetRegionData(rgn: HRGN, n: u32, data: *mut RGNDATA64) -> u32;
}

poly_wtest::main!(test);

fn rect(l: i32, t: i32, r: i32, b: i32) -> HRGN {
    let rc = RECT { left: l, top: t, right: r, bottom: b };
    // SAFETY: rc is a valid RECT.
    unsafe { CreateRectRgnIndirect(&rc) }
}

fn paint(t: &mut Test, what: &str, dc: HDC, bits: *mut u32) {
    let bg = 0x0011_2233;
    fill_bits(bits, 16, 12, bg);
    // SAFETY: a DC with a 16x12 DIB selected.
    unsafe {
        let mut cb = RECT::default();
        let k = GetClipBox(dc, &mut cb);
        PatBlt(dc, 0, 0, 16, 12, BLACKNESS);
        GdiFlush();
        let (l, tp, r, b, n) = ink(bits, 16, 12, bg);
        say!(t, "{what}: clip box type {k} {} ; ink box {l},{tp},{r},{b} pixels {n}", R(cb));
    }
}

fn test(t: &mut Test) {
    tag_register();
    // SAFETY: GDI objects and one window of this test.
    unsafe {
        let (dc, bmp, bits) = dib32(16, 12, true);
        paint(t, "no clip", dc, bits);
        let rgn = rect(4, 3, 12, 9);
        say!(t, "SelectClipRgn(rect 4,3,12,9): {}", SelectClipRgn(dc, rgn));
        DeleteObject(rgn);
        paint(t, "clipped (region deleted after selecting)", dc, bits);
        let second = rect(8, 0, 16, 6);
        say!(t, "ExtSelectClipRgn RGN_AND: {}", ExtSelectClipRgn(dc, second, RGN_AND));
        paint(t, "and 8,0,16,6", dc, bits);
        say!(t, "ExtSelectClipRgn RGN_DIFF with it again: {}", ExtSelectClipRgn(dc, second, RGN_DIFF));
        paint(t, "then diff", dc, bits);
        say!(t, "SelectClipRgn(NULL): {}", SelectClipRgn(dc, core::ptr::null_mut()));
        paint(t, "reset", dc, bits);
        let rev = rect(10, 8, 2, 2);
        say!(t, "SelectClipRgn(reversed rect 10,8,2,2): {}", SelectClipRgn(dc, rev));
        paint(t, "reversed rectangle", dc, bits);
        let out = CreateRectRgn(0, 0, 0, 0);
        let k = GetClipRgn(dc, out);
        let mut text = Buf::<300>::new();
        rgn_text(out, &mut text);
        say!(t, "GetClipRgn: {k} rects {}", text.as_str());
        let empty = rect(5, 5, 5, 9);
        say!(t, "SelectClipRgn(empty region): {}", SelectClipRgn(dc, empty));
        paint(t, "empty clip", dc, bits);
        for h in [second, rev, out, empty] {
            DeleteObject(h);
        }
        DeleteDC(dc);
        DeleteObject(bmp);

        let win = mkw(b'A', 0, WS_OVERLAPPEDWINDOW, 40, 40, 300, 200, core::ptr::null_mut());
        let mut bx = RECT::default();
        say!(t, "before: GetWindowRgnBox {}", GetWindowRgnBox(win, &mut bx));
        say!(t, "SetWindowRgn(rect 10,20,110,90): {}", SetWindowRgn(win, rect(10, 20, 110, 90), 0));
        let k = GetWindowRgnBox(win, &mut bx);
        say!(t, "GetWindowRgnBox: type {k} box {}", R(bx));
        say!(t, "SetWindowRgn(reversed rect 150,120,50,40): {}", SetWindowRgn(win, rect(150, 120, 50, 40), 0));
        let k = GetWindowRgnBox(win, &mut bx);
        say!(t, "GetWindowRgnBox: type {k} box {}", R(bx));
        say!(t, "SetWindowRgn(empty rect 5,5,5,50): {}", SetWindowRgn(win, rect(5, 5, 5, 50), 0));
        let k = GetWindowRgnBox(win, &mut bx);
        say!(t, "GetWindowRgnBox: type {k} box {}", R(bx));
        say!(t, "SetWindowRgn(NULL): {}", SetWindowRgn(win, core::ptr::null_mut(), 0));
        say!(t, "after: GetWindowRgnBox {}", GetWindowRgnBox(win, &mut bx));
        DestroyWindow(win);
    }
    settle(20);
}
