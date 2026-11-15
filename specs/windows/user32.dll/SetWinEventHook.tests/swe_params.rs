//! SetWinEventHook's return value for good and bad parameters: an
//! out-of-context hook for the own process and thread (a nonzero handle
//! that UnhookWinEvent removes), the whole event range, min above max, a
//! NULL callback, flags 0 (WINEVENT_OUTOFCONTEXT is 0), WINEVENT_INCONTEXT
//! without a module, unknown flag bits, skip flags alone and combined,
//! made-up process and thread ids, ranges outside the documented events,
//! the same range installed twice (two handles). Every result is printed
//! with the last error it left (the error is set to a marker before each
//! call, so "unchanged" means the call did not set one). Hooks that were
//! installed are removed again at the end.
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
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
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

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
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

type HHOOK = *mut c_void;
const WINEVENT_OUTOFCONTEXT: u32 = 0;
const WINEVENT_SKIPOWNTHREAD: u32 = 1;
const EVENT_OBJECT_CREATE: u32 = 0x8000;
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
const WINEVENT_INCONTEXT: u32 = 4;
const WINEVENT_SKIPOWNPROCESS: u32 = 2;
const EVENT_MIN: u32 = 0x0000_0001;
const EVENT_MAX: u32 = 0x7FFF_FFFF;
const EVENT_AIA_START: u32 = 0xA000;
const EVENT_AIA_END: u32 = 0xAFFF;
const EV_MAX_RECS: usize = 120;
/// One callback call: which hook (its label), the event, the window (a
/// name registered with `ev_known`), idObject, idChild, whether the event
/// thread is the test's thread and whether the callback ran in it.
#[derive(Clone, Copy)]
struct EvRec {
    label: u8,
    event: u32,
    win: u8,
    obj: i32,
    child: i32,
    ev_thread_ours: bool,
    cb_thread_ours: bool,
}

/// The recorder: windows and hooks known by name, the recorded calls.
struct EvLog {
    recs: [EvRec; EV_MAX_RECS],
    n: usize,
    lost: usize,
    total: u32,
    wins: [(usize, &'static str); 16],
    nwins: usize,
    hooks: [(usize, u8); 16],
    nhooks: usize,
    tid: u32,
    only_known: bool,
}

impl EvLog {
    const fn new() -> Self {
        EvLog {
            recs: [EvRec { label: 0, event: 0, win: 0, obj: 0, child: 0, ev_thread_ours: false, cb_thread_ours: false }; EV_MAX_RECS],
            n: 0,
            lost: 0,
            total: 0,
            wins: [(0, ""); 16],
            nwins: 0,
            hooks: [(0, 0); 16],
            nhooks: 0,
            tid: 0,
            only_known: true,
        }
    }
}

static EV: Global<EvLog> = Global::new(EvLog::new());
/// Start a recording: this thread is "ours", windows and hooks known so far
/// are forgotten, and only events of registered windows (and of null) are
/// recorded when `only_known` (events of the thread's other windows, such as
/// the default IME window, are then left out).
fn ev_reset(only_known: bool) {
    let l = EV.get();
    l.n = 0;
    l.lost = 0;
    l.total = 0;
    l.nwins = 0;
    l.nhooks = 0;
    l.only_known = only_known;
    // SAFETY: plain query.
    l.tid = unsafe { GetCurrentThreadId() };
}

/// Give a window a name for the recording (at most 16).
fn ev_known(h: HWND, name: &'static str) {
    let l = EV.get();
    if l.nwins < 16 {
        l.wins[l.nwins] = (h as usize, name);
        l.nwins += 1;
    }
}

/// How many callbacks ran at all since `ev_reset`, recorded or not.
fn ev_total() -> u32 {
    EV.get().total
}

/// The callback of the hooks of these tests: records its arguments.
unsafe extern "system" fn ev_cb(hook: HHOOK, event: u32, hwnd: HWND, obj: i32, child: i32, thread: u32, _time: u32) {
    let l = EV.get();
    l.total += 1;
    let mut win = if hwnd.is_null() { 1u8 } else { 255u8 };
    for i in 0..l.nwins {
        if l.wins[i].0 == hwnd as usize {
            win = 2 + i as u8;
        }
    }
    if win == 255 && l.only_known {
        return;
    }
    let mut label = b'?';
    for i in 0..l.nhooks {
        if l.hooks[i].0 == hook as usize {
            label = l.hooks[i].1;
        }
    }
    if l.n >= EV_MAX_RECS {
        l.lost += 1;
        return;
    }
    // SAFETY: plain query.
    let me = unsafe { GetCurrentThreadId() };
    l.recs[l.n] = EvRec { label, event, win, obj, child, ev_thread_ours: thread == l.tid, cb_thread_ours: me == l.tid };
    l.n += 1;
}

/// "nonzero" or "zero" for a handle (a handle is never printed).
fn hk(h: HHOOK) -> &'static str {
    if h.is_null() { "zero" } else { "nonzero" }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn SetWinEventHook(min: u32, max: u32, module: HINSTANCE, f: *mut c_void, pid: u32, tid: u32, flags: u32) -> HHOOK;
    fn UnhookWinEvent(hook: HHOOK) -> BOOL;
}

poly_wtest::main!(test);

type Cb = unsafe extern "system" fn(HHOOK, u32, HWND, i32, i32, u32, u32);

const MARK: u32 = 0x1234_5678;

fn cb() -> *mut c_void {
    ev_cb as Cb as usize as *mut c_void
}

fn rep(t: &mut Test, what: &str, h: HHOOK, keep: &mut [HHOOK; 32], n: &mut usize) {
    let e = last_error();
    if e == MARK {
        say!(t, "{what}: {} (last error unchanged)", hk(h));
    } else {
        say!(t, "{what}: {} (last error {e})", hk(h));
    }
    if !h.is_null() && *n < 32 {
        keep[*n] = h;
        *n += 1;
    }
}

fn test(t: &mut Test) {
    ev_reset(true);
    let mut keep = [core::ptr::null_mut::<c_void>(); 32];
    let mut n = 0usize;
    // SAFETY: plain queries; the callback is ev_cb, a function with the
    // WINEVENTPROC signature; every hook is removed at the end.
    unsafe {
        let (pid, tid) = (GetCurrentProcessId(), GetCurrentThreadId());
        let odd = 0x0FFF_FFFDu32; // never a process or thread id (ids are multiples of 4)
        let nul = core::ptr::null_mut();

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "own process and thread, CREATE..DESTROY, OUTOFCONTEXT", h, &mut keep, &mut n);
        let first = h;

        set_error(MARK);
        let h = SetWinEventHook(EVENT_MIN, EVENT_MAX, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "whole range EVENT_MIN..EVENT_MAX", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "the first range again", h, &mut keep, &mut n);
        say!(t, "  the second handle for the same range differs from the first: {}", (h != first) as i32);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_CREATE, nul, cb(), pid, tid, 0);
        rep(t, "single event (min == max), flags 0", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_CREATE, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "min above max", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(0, 0, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "range 0..0 (below EVENT_MIN)", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(0, EVENT_MAX, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "range 0..EVENT_MAX", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_MIN, 0xFFFF_FFFF, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "range EVENT_MIN..0xFFFFFFFF", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_AIA_START, EVENT_AIA_END, nul, cb(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "application-defined range EVENT_AIA_START..EVENT_AIA_END", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, core::ptr::null_mut(), pid, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "NULL callback", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, WINEVENT_INCONTEXT);
        rep(t, "WINEVENT_INCONTEXT without a module", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, 0x8);
        rep(t, "unknown flag bit 0x8", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, 0x8000_0000);
        rep(t, "unknown flag bit 0x80000000", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, WINEVENT_SKIPOWNTHREAD);
        rep(t, "WINEVENT_SKIPOWNTHREAD", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, WINEVENT_SKIPOWNPROCESS);
        rep(t, "WINEVENT_SKIPOWNPROCESS", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, tid, WINEVENT_SKIPOWNTHREAD | WINEVENT_SKIPOWNPROCESS);
        rep(t, "both skip flags", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, 0, WINEVENT_OUTOFCONTEXT);
        rep(t, "own process, tid 0 (every thread)", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), 0, tid, WINEVENT_OUTOFCONTEXT);
        rep(t, "pid 0, own thread", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), odd, 0, WINEVENT_OUTOFCONTEXT);
        rep(t, "made-up process id, tid 0", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), pid, odd, WINEVENT_OUTOFCONTEXT);
        rep(t, "own process, made-up thread id", h, &mut keep, &mut n);

        set_error(MARK);
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, nul, cb(), odd, odd, WINEVENT_OUTOFCONTEXT);
        rep(t, "made-up process and thread ids", h, &mut keep, &mut n);

        set_error(MARK);
        let module = GetModuleHandleW(core::ptr::null());
        let h = SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, module, cb(), odd, odd, WINEVENT_INCONTEXT);
        rep(t, "WINEVENT_INCONTEXT with this program as the module, made-up ids (never fires)", h, &mut keep, &mut n);

        say!(t, "== the installed hooks are removed");
        let mut ok = 0;
        for i in 0..n {
            ok += UnhookWinEvent(keep[i]);
        }
        say!(t, "  all {} installed hooks removed: {}", n, (ok == n as i32) as i32);
        t.check("every installed hook was removed", ok == n as i32, 0);
    }
    settle(30);
    say!(t, "callbacks that ran: {}", ev_total());
}
