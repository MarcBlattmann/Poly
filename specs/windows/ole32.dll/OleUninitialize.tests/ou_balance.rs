//! OleUninitialize: it returns nothing; one call per successful
//! OleInitialize (including the S_FALSE ones) ends the OLE state when the
//! count reaches 0; an unmatched OleUninitialize on a thread without OLE does
//! not crash and does not make the next OleInitialize return S_FALSE (the
//! count does not go below 0); a drop target registered with RegisterDragDrop
//! is released by OleUninitialize or stays until RevokeDragDrop (what the call
//! does with registrations of windows of the thread); threads are separate.
//! Chrome's UI thread calls it once at the end of its run (S3).
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
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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

const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
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

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
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

/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

/// A window procedure that does the default handling of everything.
unsafe extern "system" fn proc_def(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

type HRESULT = i32;
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const E_POINTER: i32 = 0x8000_4003u32 as i32;
const IID_IUNKNOWN: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IDROPTARGET: GUID = GUID { d1: 0x0000_0122, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// oleidl.h (IDropTarget's method table)
#[repr(C)]
struct IDropTargetVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    drag_enter: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINT, *mut u32) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut c_void, u32, POINT, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINT, *mut u32) -> HRESULT,
}

/// A drop target of the test: counts its AddRef and Release calls and the other calls.
#[repr(C)]
struct TestDropTarget {
    vtbl: *const IDropTargetVtbl,
    refs: core::cell::Cell<i32>,
    adds: core::cell::Cell<i32>,
    releases: core::cell::Cell<i32>,
    queries: core::cell::Cell<i32>,
}

unsafe extern "system" fn dt_query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: `this` is a TestDropTarget; the arguments come from COM.
    unsafe {
        let me = &*(this as *const TestDropTarget);
        me.queries.set(me.queries.get() + 1);
        if iid.is_null() || out.is_null() {
            return E_POINTER;
        }
        if *iid == IID_IUNKNOWN || *iid == IID_IDROPTARGET {
            *out = this;
            me.refs.set(me.refs.get() + 1);
            return S_OK;
        }
        *out = core::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn dt_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a TestDropTarget.
    unsafe {
        let me = &*(this as *const TestDropTarget);
        me.adds.set(me.adds.get() + 1);
        me.refs.set(me.refs.get() + 1);
        me.refs.get() as u32
    }
}

unsafe extern "system" fn dt_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a TestDropTarget.
    unsafe {
        let me = &*(this as *const TestDropTarget);
        me.releases.set(me.releases.get() + 1);
        me.refs.set(me.refs.get() - 1);
        me.refs.get() as u32
    }
}

unsafe extern "system" fn dt_enter(_: *mut c_void, _: *mut c_void, _: u32, _: POINT, effect: *mut u32) -> HRESULT {
    // SAFETY: COM passes a writable effect.
    unsafe { *effect = 0 };
    S_OK
}

unsafe extern "system" fn dt_over(_: *mut c_void, _: u32, _: POINT, effect: *mut u32) -> HRESULT {
    // SAFETY: COM passes a writable effect.
    unsafe { *effect = 0 };
    S_OK
}

unsafe extern "system" fn dt_leave(_: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn dt_drop(_: *mut c_void, _: *mut c_void, _: u32, _: POINT, effect: *mut u32) -> HRESULT {
    // SAFETY: COM passes a writable effect.
    unsafe { *effect = 0 };
    S_OK
}

static DT_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    query_interface: dt_query,
    add_ref: dt_add_ref,
    release: dt_release,
    drag_enter: dt_enter,
    drag_over: dt_over,
    drag_leave: dt_leave,
    drop: dt_drop,
};

fn new_drop_target() -> TestDropTarget {
    TestDropTarget { vtbl: &DT_VTBL, refs: core::cell::Cell::new(1), adds: core::cell::Cell::new(0), releases: core::cell::Cell::new(0), queries: core::cell::Cell::new(0) }
}

/// Run `f(arg)` on a new thread and wait for it.
fn on_thread(f: unsafe extern "system" fn(*mut c_void) -> u32, arg: *mut c_void) -> u32 {
    // SAFETY: the thread is joined before returning; `arg` outlives it.
    unsafe {
        let h = CreateThread(core::ptr::null_mut(), 0, f, arg, 0, core::ptr::null_mut());
        WaitForSingleObject(h, 20_000);
        let mut code = 0xFFFF_FFFFu32;
        GetExitCodeThread(h, &mut code);
        CloseHandle(h);
        code
    }
}

fn hr(h: i32) -> u32 {
    h as u32
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GetExitCodeThread(thread: HANDLE, code: *mut u32) -> BOOL;
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
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OleInitialize(reserved: *mut c_void) -> HRESULT;
    fn OleUninitialize();
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> HRESULT;
    fn RegisterDragDrop(hwnd: HWND, target: *mut c_void) -> HRESULT;
}

poly_wtest::main!(test);

struct Out {
    v: [u32; 16],
    n: usize,
}
impl Out {
    fn put(&mut self, v: u32) {
        if self.n < 16 {
            self.v[self.n] = v;
            self.n += 1;
        }
    }
}

unsafe extern "system" fn unmatched(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread.
    unsafe {
        let o = &mut *(p as *mut Out);
        OleUninitialize();
        OleUninitialize();
        o.put(hr(OleInitialize(core::ptr::null_mut()))); // S_OK, not S_FALSE
        o.put(hr(OleInitialize(core::ptr::null_mut()))); // S_FALSE
        OleUninitialize();
        OleUninitialize();
        OleUninitialize(); // one too many
        let (mut k, mut q) = (-9, -9);
        o.put(hr(CoGetApartmentType(&mut k, &mut q)));
        o.put(hr(OleInitialize(core::ptr::null_mut())));
        OleUninitialize();
        0
    }
}

unsafe extern "system" fn with_target(p: *mut c_void) -> u32 {
    // SAFETY: the Out outlives the thread; the window is created and destroyed here.
    unsafe {
        let o = &mut *(p as *mut Out);
        register(wz!("OuClass"), Some(proc_def), 0, 0, core::ptr::null_mut());
        let w = create(0, wz!("OuClass"), wz!("ou"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, core::ptr::null_mut(), 0);
        let dt = new_drop_target();
        let tp = &dt as *const TestDropTarget as *mut c_void;
        OleInitialize(core::ptr::null_mut());
        o.put(hr(RegisterDragDrop(w, tp)));
        o.put(dt.adds.get() as u32);
        OleUninitialize();
        o.put(dt.releases.get() as u32); // released by the uninitialize?
        DestroyWindow(w);
        0
    }
}

fn test(t: &mut Test) {
    let mut o = Out { v: [0; 16], n: 0 };
    on_thread(unmatched, &mut o as *mut Out as *mut c_void);
    say!(t, "two unmatched OleUninitialize calls, then OleInitialize: {:#x} (S_OK), again {:#x} (S_FALSE)", o.v[0], o.v[1]);
    say!(t, "after three OleUninitialize for two successful OleInitialize: CoGetApartmentType {:#x}, OleInitialize {:#x}", o.v[2], o.v[3]);
    let mut o = Out { v: [0; 16], n: 0 };
    on_thread(with_target, &mut o as *mut Out as *mut c_void);
    say!(t, "RegisterDragDrop {:#x}, the target was AddRef'ed {} time(s)", o.v[0], o.v[1]);
    say!(t, "OleUninitialize released the target {} time(s)", o.v[2]);
}
