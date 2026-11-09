//! RegisterClassExW: a local class gets an atom in the string-atom range
//! (0xC000..=0xFFFF); GetClassInfoExW finds it by name (any letter case)
//! and by atom and returns the registered fields; a window of the class
//! reports them through GetClassLongPtrW and GetClassNameW.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type HGDIOBJ = *mut c_void;
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

const WS_OVERLAPPED: u32 = 0x0000_0000;
const CS_VREDRAW: u32 = 0x0001;
const CS_HREDRAW: u32 = 0x0002;
const CS_DBLCLKS: u32 = 0x0008;
const GCLP_HBRBACKGROUND: i32 = -10;
const GCL_CBWNDEXTRA: i32 = -18;
const GCL_CBCLSEXTRA: i32 = -20;
const GCL_STYLE: i32 = -26;
const GCW_ATOM: i32 = -32;
const GRAY_BRUSH: i32 = 2;
const ERROR_CANNOT_FIND_WND_CLASS: u32 = 1407;
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
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn GetClassInfoExW(instance: HINSTANCE, class: *const u16, out: *mut WNDCLASSEXW) -> BOOL;
    fn GetClassLongPtrW(hwnd: HWND, index: i32) -> usize;
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
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
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStockObject(index: i32) -> HGDIOBJ;
}

static PROCS: Global<u32> = Global::new(0);

unsafe extern "system" fn proc_a(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    *PROCS.get() += 1;
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: a stock object.
    let brush = unsafe { GetStockObject(GRAY_BRUSH) };
    let atom = register(wz!("RcBasicClass"), Some(proc_a), CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, 24, brush);
    t.check("atom_nonzero", atom != 0, 0);
    t.check("atom_in_string_range", atom >= 0xC000, atom as i64);

    let lower = wz!("rcbasicclass");
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cb_size = core::mem::size_of::<WNDCLASSEXW>() as u32;
    // SAFETY: wc is writable, the name NUL-terminated.
    let r = unsafe { GetClassInfoExW(instance(), lower.as_ptr(), &mut wc) };
    t.check("info_by_lowercase_name", r != 0, r as i64);
    t.expect("info_returns_the_atom", r as i64, atom as i64);
    t.expect("info_style", wc.style as i64, (CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS) as i64);
    t.expect("info_wnd_extra", wc.wnd_extra as i64, 24);
    t.expect("info_cls_extra", wc.cls_extra as i64, 0);
    t.check("info_proc", wc.wnd_proc.map(|p| p as usize) == Some(proc_a as *const () as usize), 0);
    t.check("info_brush", wc.background == brush, 0);
    t.check("info_instance", wc.instance == instance(), 0);
    t.check("info_menu_name_null", wc.menu_name.is_null(), 0);
    t.check("info_class_name_is_the_argument", wc.class_name == lower.as_ptr(), 0);

    let mut wc2: WNDCLASSEXW = zeroed();
    wc2.cb_size = core::mem::size_of::<WNDCLASSEXW>() as u32;
    // SAFETY: as above, the class named by its atom.
    let r = unsafe { GetClassInfoExW(instance(), atom_ptr(atom), &mut wc2) };
    t.expect("info_by_atom", r as i64, atom as i64);
    t.expect("info_by_atom_wnd_extra", wc2.wnd_extra as i64, 24);

    let h = create(0, wz!("RCBASICCLASS"), wz!("w"), WS_OVERLAPPED, 0, 0, 50, 50, core::ptr::null_mut(), 0);
    t.check("window_by_uppercase_name", !h.is_null(), 0);
    t.check("proc_called_during_create", *PROCS.get() > 0, 0);
    // SAFETY: plain queries of our window.
    unsafe {
        t.expect("class_atom", GetClassLongPtrW(h, GCW_ATOM) as i64, atom as i64);
        t.expect("class_style", GetClassLongPtrW(h, GCL_STYLE) as i64, (CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS) as i64);
        t.expect("class_wnd_extra", GetClassLongPtrW(h, GCL_CBWNDEXTRA) as i64, 24);
        t.expect("class_cls_extra", GetClassLongPtrW(h, GCL_CBCLSEXTRA) as i64, 0);
        t.check("class_brush", GetClassLongPtrW(h, GCLP_HBRBACKGROUND) == brush as usize, 0);
        let mut name = [0u16; 64];
        let n = GetClassNameW(h, name.as_mut_ptr(), 64);
        t.expect("class_name_length", n as i64, 12);
        t.check("class_name_as_registered", name[..13] == *wz!("RcBasicClass"), 0);
        let n = GetClassNameW(h, name.as_mut_ptr(), 5);
        t.expect("class_name_truncated_length", n as i64, 4);
        t.check("class_name_truncated_text", name[..5] == *wz!("RcBa"), 0);
        t.check("destroyed", DestroyWindow(h) != 0, 0);
    }

    let h = create(0, wz!("w"), wz!(""), 0, 0, 0, 1, 1, core::ptr::null_mut(), 0);
    t.check("no_such_class_fails", h.is_null(), 0);
    // SAFETY: plain query.
    t.expect("no_such_class_error", unsafe { GetLastError() } as i64, ERROR_CANNOT_FIND_WND_CLASS as i64);
    // SAFETY: an atom used as a class name.
    let h = unsafe {
        CreateWindowExW(
            0,
            atom_ptr(atom),
            core::ptr::null(),
            0,
            0,
            0,
            1,
            1,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            instance(),
            core::ptr::null_mut(),
        )
    };
    t.check("window_by_atom", !h.is_null(), 0);
    // SAFETY: our window.
    unsafe { DestroyWindow(h) };
}
