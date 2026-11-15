//! GetDeviceCaps across kinds of DC, as relations: the screen DC
//! (GetDC(NULL)), CreateDCW(L"DISPLAY") and CreateICW, the DC of an own
//! window (GetDC and GetWindowDC), and a memory DC. The memory DC starts
//! with its 1x1 monochrome bitmap (BITSPIXEL 1), then gets a compatible
//! bitmap (the screen's depth), a CreateBitmap monochrome one, a 32-bit DIB
//! section and a 24-bit DIB section; BITSPIXEL, PLANES, HORZRES, VERTRES,
//! TECHNOLOGY and NUMCOLORS follow the bitmap. All sizes printed are the
//! test's own bitmap sizes or equalities.
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

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
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
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const BITSPIXEL: i32 = 12;
const PLANES: i32 = 14;
const LOGPIXELSX: i32 = 88;
const LOGPIXELSY: i32 = 90;
const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
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

const DC_TECHNOLOGY: i32 = 2;
const DC_NUMCOLORS: i32 = 24;
const DC_RASTERCAPS: i32 = 38;
const DC_SIZEPALETTE: i32 = 104;
const DC_COLORRES: i32 = 108;
const DC_BLTALIGNMENT: i32 = 119;

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
    fn GetDC(hwnd: HWND) -> HDC;
    fn GetWindowDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HBITMAP;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
    fn GetDeviceCaps(hdc: HDC, index: i32) -> i32;
    fn CreateDCW(driver: *const u16, device: *const u16, port: *const u16, mode: *const c_void) -> HDC;
    fn CreateICW(driver: *const u16, device: *const u16, port: *const u16, mode: *const c_void) -> HDC;
}

poly_wtest::main!(test);

const IDX: [(&str, i32); 12] = [
    ("BITSPIXEL", BITSPIXEL),
    ("PLANES", PLANES),
    ("LOGPIXELSX", LOGPIXELSX),
    ("LOGPIXELSY", LOGPIXELSY),
    ("HORZRES", HORZRES),
    ("VERTRES", VERTRES),
    ("NUMCOLORS", DC_NUMCOLORS),
    ("COLORRES", DC_COLORRES),
    ("TECHNOLOGY", DC_TECHNOLOGY),
    ("RASTERCAPS", DC_RASTERCAPS),
    ("SIZEPALETTE", DC_SIZEPALETTE),
    ("BLTALIGNMENT", DC_BLTALIGNMENT),
];

fn same(t: &mut Test, what: &str, a: HDC, b: HDC) {
    let mut diff = Buf::<200>::new();
    // SAFETY: the DCs are live DCs of this test.
    unsafe {
        for (name, i) in IDX {
            if GetDeviceCaps(a, i) != GetDeviceCaps(b, i) {
                let _ = write!(diff, " {name}");
            }
        }
    }
    say!(t, "{what}: differs from the screen DC in:{}", if diff.n == 0 { " nothing" } else { diff.as_str() });
}

fn mem_row(t: &mut Test, what: &str, m: HDC) {
    // SAFETY: a live memory DC.
    unsafe {
        say!(
            t,
            "memory DC with {what}: BITSPIXEL {} PLANES {} HORZRES {} VERTRES {} NUMCOLORS {} TECHNOLOGY {}",
            GetDeviceCaps(m, BITSPIXEL),
            GetDeviceCaps(m, PLANES),
            GetDeviceCaps(m, HORZRES),
            GetDeviceCaps(m, VERTRES),
            GetDeviceCaps(m, DC_NUMCOLORS),
            GetDeviceCaps(m, DC_TECHNOLOGY)
        );
    }
}

fn dib_info(bpp: u16, w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        header: BITMAPINFOHEADER {
            size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            width: w,
            height: -h,
            planes: 1,
            bit_count: bpp,
            compression: BI_RGB,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    }
}

fn test(t: &mut Test) {
    tag_register();
    // SAFETY: GDI objects and one window of this test.
    unsafe {
        let screen = GetDC(core::ptr::null_mut());
        let display = CreateDCW(wz!("DISPLAY").as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null());
        let info = CreateICW(wz!("DISPLAY").as_ptr(), core::ptr::null(), core::ptr::null(), core::ptr::null());
        let win = mkw(b'A', 0, WS_OVERLAPPEDWINDOW, 40, 40, 200, 150, core::ptr::null_mut());
        let wdc = GetDC(win);
        let wfull = GetWindowDC(win);
        same(t, "CreateDCW(DISPLAY)", display, screen);
        same(t, "CreateICW(DISPLAY)", info, screen);
        same(t, "GetDC(window)", wdc, screen);
        same(t, "GetWindowDC(window)", wfull, screen);
        say!(t, "object types: screen {} display {} info {} window {}", GetObjectType(screen), GetObjectType(display), GetObjectType(info), GetObjectType(wdc));

        let mem = CreateCompatibleDC(screen);
        same(t, "CreateCompatibleDC(screen), fresh", mem, screen);
        mem_row(t, "the initial bitmap", mem);
        let cb = CreateCompatibleBitmap(screen, 20, 10);
        SelectObject(mem, cb);
        mem_row(t, "CreateCompatibleBitmap(screen, 20, 10)", mem);
        same(t, "  that DC", mem, screen);
        let mono = CreateBitmap(12, 7, 1, 1, core::ptr::null());
        SelectObject(mem, mono);
        mem_row(t, "CreateBitmap(12, 7, 1, 1)", mem);
        let q = CreateBitmap(6, 5, 1, 32, core::ptr::null());
        SelectObject(mem, q);
        mem_row(t, "CreateBitmap(6, 5, 1, 32)", mem);
        let mut bits: *mut c_void = core::ptr::null_mut();
        let i32b = dib_info(32, 9, 8);
        let d32 = CreateDIBSection(mem, &i32b, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(mem, d32);
        mem_row(t, "32-bit DIB section 9x8", mem);
        let i24 = dib_info(24, 5, 3);
        let d24 = CreateDIBSection(mem, &i24, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(mem, d24);
        mem_row(t, "24-bit DIB section 5x3", mem);
        say!(t, "compatible DC of the memory DC: BITSPIXEL {}", {
            let m2 = CreateCompatibleDC(mem);
            let v = GetDeviceCaps(m2, BITSPIXEL);
            DeleteDC(m2);
            v
        });
        let mono_dc = CreateCompatibleDC(mem);
        SelectObject(mono_dc, mono);
        let m3 = CreateCompatibleBitmap(mono_dc, 4, 4);
        let mut bm = BITMAP::default();
        GetObjectW(m3, core::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut c_void);
        say!(t, "CreateCompatibleBitmap of the monochrome DC: bpp {} planes {}", bm.bits_pixel, bm.planes);
        DeleteObject(m3);
        DeleteDC(mono_dc);

        DeleteDC(mem);
        for b in [cb, mono, q, d32, d24] {
            DeleteObject(b);
        }
        ReleaseDC(win, wfull);
        ReleaseDC(win, wdc);
        DestroyWindow(win);
        DeleteDC(info);
        DeleteDC(display);
        ReleaseDC(core::ptr::null_mut(), screen);
    }
    settle(20);
}
