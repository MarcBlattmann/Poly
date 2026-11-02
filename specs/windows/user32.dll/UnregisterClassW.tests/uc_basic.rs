//! UnregisterClassW: succeeds once for a registered class (by name in any
//! letter case, or by atom); afterwards the class is gone (lookups and new
//! windows fail with ERROR_CLASS_DOES_NOT_EXIST / ERROR_CANNOT_FIND_WND_CLASS),
//! a second unregister fails with ERROR_CLASS_DOES_NOT_EXIST by name and
//! ERROR_INVALID_HANDLE by atom; the name registered again gets the same
//! atom; a NULL instance finds the class; a system class cannot be
//! unregistered; atom 0 fails with ERROR_INVALID_PARAMETER.
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
const ERROR_CANNOT_FIND_WND_CLASS: u32 = 1407;
const ERROR_CLASS_DOES_NOT_EXIST: u32 = 1411;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// MAKEINTATOM: a class atom where a class name pointer goes.
fn atom_ptr(atom: u16) -> *const u16 {
    atom as usize as *const u16
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
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

unsafe extern "system" fn proc_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn unregister(t: &mut Test, label: &str, name: *const u16, inst: HINSTANCE) -> i32 {
    // SAFETY: a class name or atom; SetLastError first so a stale code shows.
    let (r, e) = unsafe {
        SetLastError(0xBEEF);
        let r = UnregisterClassW(name, inst);
        (r, GetLastError())
    };
    say!(t, "{label}: result={} error={}", r, if r == 0 { e as i64 } else { -1 });
    r
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let a1 = register(wz!("UcBasic"), Some(proc_a), 0, 0, core::ptr::null_mut());
    t.check("registered", a1 != 0, 0);
    unregister(t, "by_upper_name", wz!("UCBASIC").as_ptr(), instance());
    unregister(t, "again", wz!("UcBasic").as_ptr(), instance());
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cb_size = core::mem::size_of::<WNDCLASSEXW>() as u32;
    // SAFETY: wc is writable.
    let (r, e) = unsafe {
        SetLastError(0);
        let r = GetClassInfoExW(instance(), wz!("UcBasic").as_ptr(), &mut wc);
        (r, GetLastError())
    };
    t.expect("lookup_after_unregister", r as i64, 0);
    t.expect("lookup_error", e as i64, ERROR_CLASS_DOES_NOT_EXIST as i64);
    let h = create(0, wz!("UcBasic"), wz!(""), WS_POPUP, 0, 0, 1, 1, core::ptr::null_mut(), 0);
    t.check("no_window_after_unregister", h.is_null(), 0);
    // SAFETY: plain query.
    t.expect("create_error", unsafe { GetLastError() } as i64, ERROR_CANNOT_FIND_WND_CLASS as i64);

    let a2 = register(wz!("UcBasic"), Some(proc_a), 0, 0, core::ptr::null_mut());
    t.check("registered_again", a2 != 0, 0);
    say!(t, "second_atom_same_as_first={}", (a2 == a1) as i32);
    unregister(t, "by_atom", atom_ptr(a2), instance());
    unregister(t, "by_atom_again", atom_ptr(a2), instance());

    let a3 = register(wz!("UcBasic"), Some(proc_a), 0, 0, core::ptr::null_mut());
    t.check("registered_third_time", a3 != 0, 0);
    unregister(t, "null_instance", wz!("UcBasic").as_ptr(), core::ptr::null_mut());
    unregister(t, "never_registered", wz!("UcNever").as_ptr(), instance());
    unregister(t, "system_class_button", wz!("Button").as_ptr(), core::ptr::null_mut());
    unregister(t, "atom_zero", core::ptr::null(), instance());
}
