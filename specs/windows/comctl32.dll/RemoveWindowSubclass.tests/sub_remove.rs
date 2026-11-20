//! Window subclassing by ordinal: removing and adding subclasses from inside a subclass
//! procedure (self, one still to be called, one already called), a subclass that removes itself
//! and then passes the message on, adding during a message, a subclass answering the message
//! while others are removed, and removing the last one.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, static_mut_refs, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes; "Subclassing Controls") and the
// public SDK headers winuser.h, commctrl.h and winerror.h; the helpers are this test's own.

// ---- shared helpers of the window subclassing tests (this kit's own code) ----
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type BOOL = i32;
type LRESULT = isize;

/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    size: u32,
    style: u32,
    wnd_proc: unsafe extern "system" fn(HWND, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
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
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
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
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> HWND;
    fn DestroyWindow(h: HWND) -> BOOL;
    fn SendMessageW(h: HWND, m: u32, w: usize, l: isize) -> LRESULT;
    fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> LRESULT;
    fn GetWindowLongPtrW(h: HWND, index: i32) -> isize;
    fn SetWindowLongPtrW(h: HWND, index: i32, v: isize) -> isize;
    fn GetDesktopWindow() -> HWND;
    fn IsWindow(h: HWND) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn CreateThread(
        attrs: *mut c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CloseHandle(h: HANDLE) -> BOOL;
}

// comctl32 by ordinal: 410 SetWindowSubclass, 411 GetWindowSubclass, 412 RemoveWindowSubclass,
// 413 DefSubclassProc (the names are the documented ones; Learn: "Subclassing Controls").
#[link(name = "comctl32", kind = "raw-dylib")]
unsafe extern "system" {
    #[link_ordinal(410)]
    fn SetWindowSubclass(h: HWND, pfn: SubProc, id: usize, ref_data: usize) -> BOOL;
    #[link_ordinal(411)]
    fn GetWindowSubclass(h: HWND, pfn: SubProc, id: usize, ref_data: *mut usize) -> BOOL;
    #[link_ordinal(412)]
    fn RemoveWindowSubclass(h: HWND, pfn: SubProc, id: usize) -> BOOL;
    #[link_ordinal(413)]
    fn DefSubclassProc(h: HWND, m: u32, w: usize, l: isize) -> LRESULT;
}

/// SUBCLASSPROC (commctrl.h)
type SubProc = unsafe extern "system" fn(HWND, u32, usize, isize, usize, usize) -> LRESULT;

const WM_DESTROY: u32 = 0x0002;
const WM_NCDESTROY: u32 = 0x0082;
const WM_USER: u32 = 0x0400;
const GWLP_WNDPROC: i32 = -4;

// ---- the message log: callbacks append lines, the test prints them ----
static mut LOG: [u8; 12000] = [0; 12000];
static mut LOG_LEN: usize = 0;

fn log_str(s: &str) {
    // SAFETY: the tests that log run on one thread.
    unsafe {
        #[allow(static_mut_refs)]
        let (b, n) = (&mut LOG, &mut LOG_LEN);
        for &c in s.as_bytes() {
            if *n < b.len() - 1 {
                b[*n] = c;
                *n += 1;
            }
        }
        if *n < b.len() - 1 {
            b[*n] = b'\n';
            *n += 1;
        }
    }
}

macro_rules! logf {
    ($($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        log_str(b.as_str());
    }};
}

/// Prints and clears the log, one `label| line` each.
fn flush(t: &mut Test, label: &str) {
    // SAFETY: single thread; the buffer is copied out before it is cleared.
    let text: Buf<12000> = unsafe {
        #[allow(static_mut_refs)]
        let (b, n) = (&LOG, LOG_LEN);
        let mut x = Buf::<12000>::new();
        x.b[..n].copy_from_slice(&b[..n]);
        x.n = n;
        LOG_LEN = 0;
        x
    };
    let mut any = false;
    for line in text.as_str().lines() {
        say!(t, "{}| {}", label, line);
        any = true;
    }
    if !any {
        say!(t, "{}| (nothing logged)", label);
    }
}

/// The windows a test made, in order: the log names them w1, w2, ... (never the handle).
static mut WINS: [HWND; 6] = [core::ptr::null_mut(); 6];
static mut NWINS: usize = 0;

fn who(h: HWND) -> &'static str {
    // SAFETY: read of statics written before the callbacks run.
    unsafe {
        for i in 0..NWINS {
            if WINS[i] == h {
                return ["w1", "w2", "w3", "w4", "w5", "w6"][i];
            }
        }
    }
    "other"
}

fn msg_name(m: u32) -> Option<&'static str> {
    match m {
        0x0401 => Some("U+1"),
        0x0402 => Some("U+2"),
        0x0403 => Some("U+3"),
        0x0404 => Some("U+4"),
        WM_DESTROY => Some("WM_DESTROY"),
        WM_NCDESTROY => Some("WM_NCDESTROY"),
        _ => None,
    }
}

/// The original window procedure: logs the watched messages, answers 777 to WM_USER+1.
unsafe extern "system" fn base_proc(h: HWND, m: u32, w: usize, l: isize) -> isize {
    if let Some(n) = msg_name(m) {
        logf!("base {} {} w={} l={}", n, who(h), w, l);
    }
    if m == WM_USER + 1 {
        return 777;
    }
    if m == WM_USER + 2 {
        return 888;
    }
    // SAFETY: the default procedure with the arguments we got.
    unsafe { DefWindowProcW(h, m, w, l) }
}

fn wide(s: &str) -> [u16; 24] {
    let mut o = [0u16; 24];
    for (i, c) in s.bytes().enumerate() {
        o[i] = c as u16;
    }
    o
}

fn register(name: &str) -> [u16; 24] {
    let cls = wide(name);
    let wc = WNDCLASSEXW {
        size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        wnd_proc: base_proc,
        cls_extra: 0,
        wnd_extra: 0,
        // SAFETY: GetModuleHandleW(NULL) is the image base.
        instance: unsafe { GetModuleHandleW(core::ptr::null()) },
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: cls.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW whose class name outlives the call.
    unsafe { RegisterClassExW(&wc) };
    cls
}

fn make_window(cls: &[u16; 24]) -> HWND {
    let title = wide("t");
    // SAFETY: a plain overlapped window of a class registered above, never shown.
    let h = unsafe {
        CreateWindowExW(
            0,
            cls.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            10,
            10,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    // SAFETY: remembered for the log.
    unsafe {
        if NWINS < 6 {
            WINS[NWINS] = h;
            NWINS += 1;
        }
    }
    h
}

/// A subclass procedure that logs the watched messages (tag, id, reference data, wParam, lParam)
/// and then runs `$body(h, m, w, l, id, rd)`, a `fn` returning `Some(result)` to answer itself or
/// `None` to pass the message on with DefSubclassProc.
macro_rules! sp {
    ($name:ident, $tag:expr, $body:expr) => {
        unsafe extern "system" fn $name(h: HWND, m: u32, w: usize, l: isize, id: usize, rd: usize) -> isize {
            if let Some(n) = msg_name(m) {
                logf!("{} {} {} id={} ref={} w={} l={}", $tag, n, who(h), id, rd, w, l);
            }
            let f: fn(HWND, u32, usize, isize, usize, usize) -> Option<isize> = $body;
            match f(h, m, w, l, id, rd) {
                Some(v) => v,
                // SAFETY: the next procedure of the chain.
                None => unsafe { DefSubclassProc(h, m, w, l) },
            }
        }
    };
}

/// Passes everything on.
fn pass(_: HWND, _: u32, _: usize, _: isize, _: usize, _: usize) -> Option<isize> {
    None
}
// ---- end of the shared helpers ----

poly_wtest::main!(test);

sp!(s1, "S1", pass);
// S2 removes itself and then passes the message on.
sp!(s2, "S2", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 1 {
        // SAFETY: a call into comctl32 with our window.
        let r = unsafe { RemoveWindowSubclass(h, s2, 2) };
        logf!("  S2 removes itself: {}", r);
    }
    None
});
// S3 removes S1, which is still to be called (it is further down the chain).
sp!(s3, "S3", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 1 {
        // SAFETY: a call into comctl32 with our window.
        let r = unsafe { RemoveWindowSubclass(h, s1, 1) };
        logf!("  S3 removes S1: {}", r);
    }
    None
});
sp!(s4, "S4", pass);

sp!(t1, "T1", pass);
sp!(t2, "T2", pass);
// T3 removes T4, which was already called (it is above T3 in the chain).
sp!(t3, "T3", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 1 {
        // SAFETY: a call into comctl32 with our window.
        let r = unsafe { RemoveWindowSubclass(h, t4, 4) };
        logf!("  T3 removes T4 (already called): {}", r);
    }
    None
});
sp!(t4, "T4", pass);

// U3 adds U5 on top of the chain while handling a message.
sp!(u1, "U1", pass);
sp!(u2, "U2", pass);
sp!(u3, "U3", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 1 {
        // SAFETY: a call into comctl32 with our window.
        let r = unsafe { SetWindowSubclass(h, u5, 5, 55) };
        logf!("  U3 adds U5: {}", r);
    }
    None
});
sp!(u5, "U5", pass);

// V2 removes V1 (below it) and answers the message itself.
sp!(v1, "V1", pass);
sp!(v2, "V2", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 1 {
        // SAFETY: a call into comctl32 with our window.
        let r = unsafe { RemoveWindowSubclass(h, v1, 1) };
        logf!("  V2 removes V1: {}", r);
        return Some(99);
    }
    None
});

// W1 removes itself and returns without passing the message on.
sp!(w1, "W1", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 1 {
        // SAFETY: a call into comctl32 with our window.
        let r = unsafe { RemoveWindowSubclass(h, w1, 1) };
        logf!("  W1 removes itself: {}", r);
        return Some(55);
    }
    None
});

fn test(t: &mut Test) {
    let cls = register("PolySubRemove");
    // SAFETY: calls into comctl32 and user32 with windows we made.
    unsafe {
        // chain top to bottom: S4, S3, S2, S1
        let h = make_window(&cls);
        SetWindowSubclass(h, s1, 1, 10);
        SetWindowSubclass(h, s2, 2, 20);
        SetWindowSubclass(h, s3, 3, 30);
        SetWindowSubclass(h, s4, 4, 40);
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "first send: result {}", r);
        flush(t, "S4,S3,S2,S1");
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "second send: result {}", r);
        flush(t, "S4,S3");
        DestroyWindow(h);
        flush(t, "destroy");

        // chain top to bottom: T4, T3, T2, T1
        let h = make_window(&cls);
        SetWindowSubclass(h, t1, 1, 1);
        SetWindowSubclass(h, t2, 2, 2);
        SetWindowSubclass(h, t3, 3, 3);
        SetWindowSubclass(h, t4, 4, 4);
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send: result {}", r);
        flush(t, "T4,T3,T2,T1");
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send again: result {}", r);
        flush(t, "T3,T2,T1");
        DestroyWindow(h);
        flush(t, "destroy");

        // adding during a message
        let h = make_window(&cls);
        SetWindowSubclass(h, u1, 1, 1);
        SetWindowSubclass(h, u2, 2, 2);
        SetWindowSubclass(h, u3, 3, 3);
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send: result {}", r);
        flush(t, "U3,U2,U1");
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send again: result {}", r);
        flush(t, "U5,U3,U2,U1");
        say!(t, "U3 now set once more: {}", SetWindowSubclass(h, u3, 3, 33));
        DestroyWindow(h);
        flush(t, "destroy");

        // answering while removing the next one
        let h = make_window(&cls);
        SetWindowSubclass(h, v1, 1, 1);
        SetWindowSubclass(h, v2, 2, 2);
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send: result {}", r);
        flush(t, "V2,V1");
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send again: result {}", r);
        flush(t, "V2 only");
        DestroyWindow(h);
        flush(t, "destroy");

        // removing the only subclass without passing on
        let h = make_window(&cls);
        SetWindowSubclass(h, w1, 1, 1);
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send: result {}", r);
        flush(t, "W1");
        let r = SendMessageW(h, WM_USER + 1, 0, 0);
        say!(t, "send again: result {}", r);
        flush(t, "no subclass");
        DestroyWindow(h);
    }
}
