//! RegisterClassExW and the class lookup: GetClassInfoExW of a name never
//! registered fails with ERROR_CLASS_DOES_NOT_EXIST; system classes are
//! found with a NULL instance; a class registered with a NULL hInstance is
//! observed; windows of a W class are Unicode; class extra bytes start at
//! zero; CS_GLOBALCLASS is kept in the style.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
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

const WS_POPUP: u32 = 0x8000_0000;
const CS_OWNDC: u32 = 0x0020;
const CS_GLOBALCLASS: u32 = 0x4000;
const GWLP_HINSTANCE: i32 = -6;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn UnregisterClassW(class: *const u16, instance: HINSTANCE) -> BOOL;
    fn GetClassInfoExW(instance: HINSTANCE, class: *const u16, out: *mut WNDCLASSEXW) -> BOOL;
    fn GetClassLongPtrW(hwnd: HWND, index: i32) -> usize;
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
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn IsWindowUnicode(hwnd: HWND) -> BOOL;
}

unsafe extern "system" fn proc_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn info(t: &mut Test, label: &str, inst: HINSTANCE, name: *const u16) -> (i32, WNDCLASSEXW) {
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cb_size = core::mem::size_of::<WNDCLASSEXW>() as u32;
    // SAFETY: wc is writable; SetLastError first so a stale code shows.
    let (r, e) = unsafe {
        SetLastError(0xBEEF);
        let r = GetClassInfoExW(inst, name, &mut wc);
        (r, GetLastError())
    };
    say!(t, "{label}: found={} error={}", (r != 0) as i32, if r == 0 { e as i64 } else { -1 });
    (r, wc)
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    info(t, "never_registered", instance(), wz!("RcLookupNothing").as_ptr());
    let (r, wc) = info(t, "button_null_instance", core::ptr::null_mut(), wz!("Button").as_ptr());
    if r != 0 {
        say!(t, "button: style={:#x} wnd_extra={} cls_extra={}", wc.style, wc.wnd_extra, wc.cls_extra);
    }
    info(t, "button_our_instance", instance(), wz!("Button").as_ptr());
    info(t, "static_null_instance", core::ptr::null_mut(), wz!("Static").as_ptr());
    info(t, "edit_null_instance", core::ptr::null_mut(), wz!("Edit").as_ptr());

    // A class registered with hInstance NULL.
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_GLOBALCLASS | CS_OWNDC,
        wnd_proc: Some(proc_a),
        cls_extra: 16,
        wnd_extra: 0,
        instance: core::ptr::null_mut(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: wz!("RcLookupNullInst").as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW.
    let atom = unsafe { RegisterClassExW(&wc) };
    t.check("null_instance_registers", atom != 0, 0);
    let (r, got) = info(t, "null_instance_class_by_null", core::ptr::null_mut(), wz!("RcLookupNullInst").as_ptr());
    if r != 0 {
        say!(
            t,
            "null-instance class reports instance: null {} exe {}",
            got.instance.is_null() as i32,
            (got.instance == instance()) as i32
        );
        t.expect("global_class_style_kept", got.style as i64, (CS_GLOBALCLASS | CS_OWNDC) as i64);
        t.expect("cls_extra_kept", got.cls_extra as i64, 16);
    }
    info(t, "null_instance_class_by_exe", instance(), wz!("RcLookupNullInst").as_ptr());

    let h = create(0, wz!("RcLookupNullInst"), wz!(""), WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    t.check("window_created", !h.is_null(), 0);
    // SAFETY: plain queries of our window.
    unsafe {
        t.expect("window_is_unicode", IsWindowUnicode(h) as i64, 1);
        t.expect("class_extra_0_starts_zero", GetClassLongPtrW(h, 0) as i64, 0);
        t.expect("class_extra_8_starts_zero", GetClassLongPtrW(h, 8) as i64, 0);
        SetLastError(0);
        let v = GetClassLongPtrW(h, 16);
        say!(t, "class_extra_past_end: value={} error={}", v, GetLastError());
        t.check("hinstance_of_window", GetWindowLongPtrW(h, GWLP_HINSTANCE) == instance() as isize, 0);
        DestroyWindow(h);
        t.check(
            "unregister_null_instance",
            UnregisterClassW(wz!("RcLookupNullInst").as_ptr(), core::ptr::null_mut()) != 0,
            0,
        );
    }
}
