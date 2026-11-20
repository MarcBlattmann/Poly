//! Window subclassing by ordinal (comctl32 410 SetWindowSubclass, 412 RemoveWindowSubclass,
//! 413 DefSubclassProc): the order messages go through a chain of subclasses, the arguments each
//! one gets (window, wParam, lParam, uIdSubclass, dwRefData), the results that travel back, a
//! subclass that answers without passing the message on, and one that changes the result.
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

sp!(sub_a, "A", pass);
sp!(sub_b, "B", pass);
sp!(sub_c, "C", pass);
// D answers WM_USER+3 itself and does not pass it on; it adds 1000 to the result of WM_USER+2.
sp!(sub_d, "D", |h, m, _w, _l, _id, _rd| {
    if m == WM_USER + 3 {
        return Some(4242);
    }
    if m == WM_USER + 2 {
        // SAFETY: the next procedure of the chain.
        return Some(unsafe { DefSubclassProc(h, m, 0, 0) } + 1000);
    }
    None
});

fn test(t: &mut Test) {
    let cls = register("PolySubChain");
    let h = make_window(&cls);
    // SAFETY: calls into comctl32 and user32 with a window we made.
    unsafe {
        say!(t, "window made: {}", !h.is_null());
        // no subclass yet
        say!(t, "plain send U+1: result {}", SendMessageW(h, WM_USER + 1, 5, 6));
        flush(t, "plain");

        SetLastError(0x5A5A);
        let r1 = SetWindowSubclass(h, sub_a, 1, 100);
        say!(t, "SetWindowSubclass A id 1 ref 100: {} (last error {:#x})", r1, GetLastError());
        let r2 = SetWindowSubclass(h, sub_b, 2, 200);
        let r3 = SetWindowSubclass(h, sub_c, 3, 300);
        say!(t, "B id 2 ref 200: {}, C id 3 ref 300: {}", r2, r3);

        let res = SendMessageW(h, WM_USER + 1, 11, 22);
        say!(t, "send U+1 (w=11 l=22): result {}", res);
        flush(t, "chain A,B,C");
        let res = SendMessageW(h, WM_USER + 2, 0, -1);
        say!(t, "send U+2: result {}", res);
        flush(t, "chain A,B,C");

        // D on top: answers U+3 itself, adds to U+2's result
        let r4 = SetWindowSubclass(h, sub_d, 4, 400);
        say!(t, "D id 4 ref 400: {}", r4);
        let res = SendMessageW(h, WM_USER + 3, 1, 2);
        say!(t, "send U+3 to chain D,C,B,A: result {} (D answers itself)", res);
        flush(t, "D answers");
        let res = SendMessageW(h, WM_USER + 2, 3, 4);
        say!(t, "send U+2 to chain D,C,B,A: result {} (D adds 1000)", res);
        flush(t, "D adds");
        let res = SendMessageW(h, WM_USER + 1, 7, 8);
        say!(t, "send U+1 to chain D,C,B,A: result {}", res);
        flush(t, "D passes");

        // reference data of a full pointer width and large ids
        let h2 = make_window(&cls);
        let big_ref = usize::MAX - 5;
        let big_id = usize::MAX - 1;
        let r = SetWindowSubclass(h2, sub_a, big_id, big_ref);
        say!(t, "set with id {:#x} ref {:#x}: {}", big_id, big_ref, r);
        SendMessageW(h2, WM_USER + 1, 0, 0);
        flush(t, "big values");
        let mut rd = 0usize;
        let g = GetWindowSubclass(h2, sub_a, big_id, &mut rd);
        say!(t, "GetWindowSubclass: {} ref {:#x}", g, rd);
        // id 0 and ref 0
        let h3 = make_window(&cls);
        let r = SetWindowSubclass(h3, sub_b, 0, 0);
        SendMessageW(h3, WM_USER + 1, 0, 0);
        say!(t, "set with id 0 ref 0: {}", r);
        flush(t, "zero values");
        DestroyWindow(h);
        DestroyWindow(h2);
        DestroyWindow(h3);
        flush(t, "destroy");
    }
}
