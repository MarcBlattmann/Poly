//! `s1test.exe` (ADR 0155, batch 4): the program the kernel's ring-3 test
//! `nt_chrome_headless_surface_program_runs` starts. It makes the calls a
//! headless Chrome makes of `ole32`, the Windows Runtime's API sets,
//! `user32` and `gdi32` — apartments, `CoCreateInstance`, `HSTRING`s,
//! message-only windows, window properties, registered messages,
//! `SystemParametersInfoW`, the keyboard layout list, DPI, icons, fonts and
//! device caps — and checks the documented results (Microsoft Learn's page
//! for each function; constants from `winerror.h`, `winuser.h`,
//! `wingdi.h`).
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `s1test: step N failed (got G)` on its standard output and
//! exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]
// Test code: every function below is unsafe as a whole.
#![allow(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;
type Hwnd = usize;
type Hresult = i32;
const INVALID_HANDLE_VALUE: Handle = -1;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
/// `ExitProcess` code of a run that passed every check.
const PASS: u32 = 0x42;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> Hresult;
    fn CoUninitialize();
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> Hresult;
    fn CoCreateInstance(
        clsid: *const [u8; 16],
        outer: *mut c_void,
        ctx: u32,
        iid: *const [u8; 16],
        ppv: *mut *mut c_void,
    ) -> Hresult;
    fn CoTaskMemAlloc(cb: usize) -> *mut u8;
    fn CoTaskMemRealloc(p: *mut u8, cb: usize) -> *mut u8;
    fn CoTaskMemFree(p: *mut u8);
    fn CLSIDFromString(text: *const u16, clsid: *mut [u8; 16]) -> Hresult;
    fn StringFromGUID2(guid: *const [u8; 16], buf: *mut u16, max: i32) -> i32;
    fn StringFromCLSID(guid: *const [u8; 16], out: *mut *mut u16) -> Hresult;
    fn CoCreateGuid(out: *mut [u8; 16]) -> Hresult;
    fn OleInitialize(reserved: *mut c_void) -> Hresult;
    fn OleUninitialize();
    fn PropVariantClear(pv: *mut u8) -> Hresult;
    fn CoCreateFreeThreadedMarshaler(outer: *mut c_void, out: *mut *mut c_void) -> Hresult;
    fn RegisterDragDrop(hwnd: Hwnd, target: *mut c_void) -> Hresult;
    fn RevokeDragDrop(hwnd: Hwnd) -> Hresult;
}

// The Windows Runtime functions are imported the way Chrome imports them:
// through their API-set contracts, which resolve to ole32.dll.
#[link(name = "api-ms-win-core-winrt-l1-1-0", kind = "raw-dylib")]
unsafe extern "system" {
    fn RoInitialize(init_type: i32) -> Hresult;
    fn RoUninitialize();
    fn RoGetActivationFactory(class_id: usize, iid: *const [u8; 16], factory: *mut *mut c_void) -> Hresult;
    fn RoActivateInstance(class_id: usize, instance: *mut *mut c_void) -> Hresult;
}

#[link(name = "api-ms-win-core-winrt-string-l1-1-0", kind = "raw-dylib")]
unsafe extern "system" {
    fn WindowsCreateString(src: *const u16, len: u32, out: *mut usize) -> Hresult;
    fn WindowsCreateStringReference(src: *const u16, len: u32, header: *mut [u64; 3], out: *mut usize) -> Hresult;
    fn WindowsDeleteString(h: usize) -> Hresult;
    fn WindowsGetStringRawBuffer(h: usize, len: *mut u32) -> *const u16;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(wc: *const u8) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        menu: usize,
        inst: usize,
        param: *mut u8,
    ) -> Hwnd;
    fn DestroyWindow(h: Hwnd) -> i32;
    fn DefWindowProcW(h: Hwnd, msg: u32, w: usize, l: isize) -> isize;
    fn IsWindow(h: Hwnd) -> i32;
    fn IsWindowVisible(h: Hwnd) -> i32;
    fn GetClassNameW(h: Hwnd, buf: *mut u16, max: i32) -> i32;
    fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
    fn GetDpiForWindow(h: Hwnd) -> u32;
    fn SetPropW(h: Hwnd, name: *const u16, data: usize) -> i32;
    fn GetPropW(h: Hwnd, name: *const u16) -> usize;
    fn RemovePropW(h: Hwnd, name: *const u16) -> usize;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn EnumThreadWindows(tid: u32, f: unsafe extern "system" fn(Hwnd, isize) -> i32, l: isize) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, pv: *mut u8, ini: u32) -> i32;
    fn GetSysColor(index: i32) -> u32;
    fn GetSysColorBrush(index: i32) -> usize;
    fn GetKeyboardLayout(tid: u32) -> usize;
    fn GetKeyboardLayoutList(n: i32, list: *mut usize) -> i32;
    fn GetKeyboardLayoutNameW(buf: *mut u16) -> i32;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> i32;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> i32;
    fn GetLastInputInfo(info: *mut u32) -> i32;
    fn GetCaretBlinkTime() -> u32;
    fn EnumDisplayDevicesW(device: *const u16, index: u32, dd: *mut u8, flags: u32) -> i32;
    fn LoadIconW(inst: usize, name: usize) -> usize;
    fn CreateIconIndirect(info: *const u8) -> usize;
    fn DestroyIcon(icon: usize) -> i32;
    fn GetDC(h: Hwnd) -> usize;
    fn ReleaseDC(h: Hwnd, dc: usize) -> i32;
    fn NotifyWinEvent(event: u32, h: Hwnd, object: i32, child: i32);
    fn RegisterPowerSettingNotification(recipient: Hwnd, guid: *const [u8; 16], flags: u32) -> usize;
    fn UnregisterPowerSettingNotification(h: usize) -> i32;
}

#[link(name = "api-ms-win-shcore-scaling-l1-1-1", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: usize, kind: u32, x: *mut u32, y: *mut u32) -> Hresult;
    fn GetProcessDpiAwareness(process: usize, awareness: *mut u32) -> Hresult;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MonitorFromWindow(h: Hwnd, flags: u32) -> usize;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStockObject(i: i32) -> usize;
    fn GetObjectW(h: usize, cb: i32, out: *mut u8) -> i32;
    fn CreateFontIndirectW(lf: *const u8) -> usize;
    fn GetTextMetricsW(dc: usize, tm: *mut u8) -> i32;
    fn GetDeviceCaps(dc: usize, index: i32) -> i32;
    fn SelectObject(dc: usize, obj: usize) -> usize;
    fn DeleteObject(obj: usize) -> i32;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> usize;
}

// ----- output and checks --------------------------------------------------------------

fn out(bytes: &[u8]) {
    // SAFETY: plain kernel32 calls with a buffer of ours.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h != 0 && h != INVALID_HANDLE_VALUE {
            let mut n = 0;
            WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut());
        }
    }
}

fn out_num(mut v: u32) {
    let mut b = [0u8; 10];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out(&b[i..]);
}

static STEP: AtomicU32 = AtomicU32::new(0);

/// Check `ok` as the next step, reporting `got` on failure.
fn check_got(ok: bool, got: u32) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        out(b"s1test: step ");
        out_num(step);
        out(b" failed (got ");
        out_num(got);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

/// An `HRESULT` equals `want`.
fn check_hr(got: Hresult, want: u32) {
    check_got(got as u32 == want, got as u32);
}

/// A value equals `want`.
fn check_eq(got: usize, want: usize) {
    check_got(got == want, got as u32);
}

/// A call returned FALSE/0 with `want` as the last error.
fn check_fail(r: usize, want: u32) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(r == 0 && e == want, e);
}

const fn w<const N: usize>(s: &str) -> [u16; N] {
    let b = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    while i < b.len() {
        out[i] = b[i] as u16;
        i += 1;
    }
    out
}

fn wide_eq(p: &[u16], s: &str) -> bool {
    let n = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    n == s.len() && s.bytes().enumerate().all(|(i, c)| p[i] == u16::from(c))
}

const S_OK: u32 = 0;
const S_FALSE: u32 = 1;
const RPC_E_CHANGED_MODE: u32 = 0x8001_0106;
const CO_E_NOTINITIALIZED: u32 = 0x8004_01F0;
const REGDB_E_CLASSNOTREG: u32 = 0x8004_0154;
const CO_E_CLASSSTRING: u32 = 0x8004_01F3;
const E_NOINTERFACE: u32 = 0x8000_4002;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_ACCESS_DENIED: u32 = 5;
const HWND_MESSAGE: Hwnd = -3isize as usize;

/// `IID_IUnknown`.
const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// Some CLSID that is registered nowhere ({11111111-2222-3333-4444-555555555555}).
const CLSID_NOWHERE: [u8; 16] =
    [0x11, 0x11, 0x11, 0x11, 0x22, 0x22, 0x33, 0x33, 0x44, 0x44, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55];

// ----- COM ----------------------------------------------------------------------------

unsafe fn com() {
    // No apartment yet.
    let (mut kind, mut qual) = (7, 7);
    check_hr(CoGetApartmentType(&mut kind, &mut qual), CO_E_NOTINITIALIZED);
    let mut p: *mut c_void = 1 as *mut c_void;
    check_hr(CoCreateInstance(&CLSID_NOWHERE, core::ptr::null_mut(), 1, &IID_IUNKNOWN, &mut p), CO_E_NOTINITIALIZED);
    check_eq(p as usize, 0);

    // CoInitializeEx: S_OK, S_FALSE for the same model, RPC_E_CHANGED_MODE for the other.
    check_hr(CoInitializeEx(core::ptr::null_mut(), 0), S_OK); // COINIT_MULTITHREADED
    check_hr(CoInitializeEx(core::ptr::null_mut(), 0), S_FALSE);
    check_hr(CoInitializeEx(core::ptr::null_mut(), 2), RPC_E_CHANGED_MODE); // COINIT_APARTMENTTHREADED
    check_hr(CoGetApartmentType(&mut kind, &mut qual), S_OK);
    check_eq(kind as usize, 1); // APTTYPE_MTA
    CoUninitialize();
    CoUninitialize();
    check_hr(CoGetApartmentType(&mut kind, &mut qual), CO_E_NOTINITIALIZED);
    // The model can change once the thread is balanced.
    check_hr(CoInitializeEx(core::ptr::null_mut(), 2), S_OK);
    check_hr(CoGetApartmentType(&mut kind, &mut qual), S_OK);
    check_eq(kind as usize, 0); // APTTYPE_STA
    check_hr(OleInitialize(core::ptr::null_mut()), S_FALSE);
    OleUninitialize();

    // No class is registered.
    let mut p: *mut c_void = 1 as *mut c_void;
    check_hr(CoCreateInstance(&CLSID_NOWHERE, core::ptr::null_mut(), 1, &IID_IUNKNOWN, &mut p), REGDB_E_CLASSNOTREG);
    check_eq(p as usize, 0);

    // The free-threaded marshaler is an IUnknown.
    let mut m: *mut c_void = core::ptr::null_mut();
    check_hr(CoCreateFreeThreadedMarshaler(core::ptr::null_mut(), &mut m), S_OK);
    check_got(!m.is_null(), 0);
    let vtbl = *(m as *const *const usize);
    let qi: unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> Hresult =
        core::mem::transmute(*vtbl);
    let add_ref: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vtbl.add(1));
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vtbl.add(2));
    let mut q: *mut c_void = core::ptr::null_mut();
    check_hr(qi(m, &IID_IUNKNOWN, &mut q), S_OK);
    check_eq(q as usize, m as usize);
    check_eq(release(q) as usize, 1);
    check_eq(add_ref(m) as usize, 2);
    check_eq(release(m) as usize, 1);
    let mut bad: *mut c_void = 1 as *mut c_void;
    check_hr(qi(m, &CLSID_NOWHERE, &mut bad), E_NOINTERFACE);
    check_eq(bad as usize, 0);
    check_eq(release(m) as usize, 0);

    // Drag and drop registration is accepted.
    check_hr(RegisterDragDrop(0x1234, 1 as *mut c_void), S_OK);
    check_hr(RevokeDragDrop(0x1234), S_OK);
    CoUninitialize();

    // Task memory.
    let a = CoTaskMemAlloc(100);
    check_got(!a.is_null(), 0);
    a.write_bytes(0x5A, 100);
    let b = CoTaskMemRealloc(a, 1000);
    check_got(!b.is_null() && *b == 0x5A && *b.add(99) == 0x5A, 0);
    CoTaskMemFree(b);
    CoTaskMemFree(core::ptr::null_mut());

    // GUID text.
    let text = w::<39>("{00000000-0000-0000-C000-000000000046}");
    let mut g = [0u8; 16];
    check_hr(CLSIDFromString(text.as_ptr(), &mut g), S_OK);
    check_got(g == IID_IUNKNOWN, 0);
    let mut buf = [0u16; 39];
    check_eq(StringFromGUID2(&g, buf.as_mut_ptr(), 39) as usize, 39);
    check_got(wide_eq(&buf, "{00000000-0000-0000-C000-000000000046}"), 0);
    check_eq(StringFromGUID2(&g, buf.as_mut_ptr(), 38) as usize, 0);
    let mut s: *mut u16 = core::ptr::null_mut();
    check_hr(StringFromCLSID(&CLSID_NOWHERE, &mut s), S_OK);
    check_got(wide_eq(core::slice::from_raw_parts(s, 39), "{11111111-2222-3333-4444-555555555555}"), 0);
    CoTaskMemFree(s.cast());
    let bad_text = w::<8>("garbage");
    check_hr(CLSIDFromString(bad_text.as_ptr(), &mut g), CO_E_CLASSSTRING);
    check_hr(CLSIDFromString(core::ptr::null(), &mut g), S_OK);
    check_got(g == [0u8; 16], 0);
    let (mut g1, mut g2) = ([0u8; 16], [0u8; 16]);
    check_hr(CoCreateGuid(&mut g1), S_OK);
    check_hr(CoCreateGuid(&mut g2), S_OK);
    check_got(g1 != g2 && g1[7] >> 4 == 4, 0);

    // PropVariantClear frees a string and empties the value.
    let mut pv = [0u8; 24];
    let str_mem = CoTaskMemAlloc(8);
    pv[0] = 31; // VT_LPWSTR
    pv[8..16].copy_from_slice(&(str_mem as usize).to_le_bytes());
    check_hr(PropVariantClear(pv.as_mut_ptr()), S_OK);
    check_got(pv == [0u8; 24], 0);
}

// ----- the Windows Runtime ------------------------------------------------------------

unsafe fn winrt() {
    // The same apartment bookkeeping: MTA (RO_INIT_MULTITHREADED), then STA is refused.
    check_hr(RoInitialize(1), S_OK);
    check_hr(RoInitialize(1), S_FALSE);
    check_hr(RoInitialize(0), RPC_E_CHANGED_MODE);
    RoUninitialize();
    RoUninitialize();

    // HSTRINGs.
    let text = w::<23>("Windows.Foundation.Uri");
    let mut h: usize = 0;
    check_hr(WindowsCreateString(text.as_ptr(), 22, &mut h), S_OK);
    check_got(h != 0, 0);
    let mut len = 0u32;
    let buf = WindowsGetStringRawBuffer(h, &mut len);
    check_eq(len as usize, 22);
    check_got(core::slice::from_raw_parts(buf, 23) == &text[..], 0);
    // The empty string is NULL and has an empty buffer.
    let mut e: usize = 7;
    check_hr(WindowsCreateString(core::ptr::null(), 0, &mut e), S_OK);
    check_eq(e, 0);
    let eb = WindowsGetStringRawBuffer(0, &mut len);
    check_got(len == 0 && *eb == 0, 0);
    // A fast-pass string uses the caller's header and characters.
    let mut header = [0u64; 3];
    let mut r: usize = 0;
    check_hr(WindowsCreateStringReference(text.as_ptr(), 22, &mut header, &mut r), S_OK);
    check_eq(r, header.as_ptr() as usize);
    check_eq(WindowsGetStringRawBuffer(r, core::ptr::null_mut()) as usize, text.as_ptr() as usize);

    // No Windows Runtime class is registered.
    let mut f: *mut c_void = 1 as *mut c_void;
    check_hr(RoGetActivationFactory(h, &IID_IUNKNOWN, &mut f), REGDB_E_CLASSNOTREG);
    check_eq(f as usize, 0);
    let mut inst: *mut c_void = 1 as *mut c_void;
    check_hr(RoActivateInstance(h, &mut inst), REGDB_E_CLASSNOTREG);
    check_eq(inst as usize, 0);

    check_hr(WindowsDeleteString(r), S_OK);
    check_hr(WindowsDeleteString(h), S_OK);
    check_hr(WindowsDeleteString(0), S_OK);
}

// ----- windows without a screen -------------------------------------------------------

unsafe extern "system" fn proc_(h: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    unsafe { DefWindowProcW(h, msg, w, l) }
}

static ENUM_SEEN: AtomicU32 = AtomicU32::new(0);
static ENUM_TARGET: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn enum_cb(h: Hwnd, _l: isize) -> i32 {
    ENUM_SEEN.fetch_add(1, Ordering::Relaxed);
    i32::from(h as u32 != ENUM_TARGET.load(Ordering::Relaxed))
}

unsafe fn windows() {
    let class_name = w::<11>("PolyS1Test");
    let mut wc = [0u8; 80];
    wc[0..4].copy_from_slice(&80u32.to_le_bytes());
    wc[8..16].copy_from_slice(&(proc_ as *const () as usize).to_le_bytes());
    wc[64..72].copy_from_slice(&(class_name.as_ptr() as usize).to_le_bytes());
    check_got(RegisterClassExW(wc.as_ptr()) != 0, GetLastError());

    // RegisterWindowMessageW: one number per name (case-insensitive), in 0xC000..=0xFFFF.
    let m1 = RegisterWindowMessageW(w::<16>("Poly_S1_Message").as_ptr());
    check_got((0xC000..=0xFFFF).contains(&m1), m1);
    check_eq(RegisterWindowMessageW(w::<16>("POLY_S1_MESSAGE").as_ptr()) as usize, m1 as usize);
    let m2 = RegisterWindowMessageW(w::<6>("Other").as_ptr());
    check_got(m2 != m1 && (0xC000..=0xFFFF).contains(&m2), m2);
    SetLastError(0);
    check_fail(RegisterWindowMessageW(w::<1>("").as_ptr()) as usize, ERROR_INVALID_PARAMETER);

    // A message-only window.
    let title = w::<7>("hidden");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        0,
        0,
        core::ptr::null_mut(),
    );
    check_got(hwnd != 0, GetLastError());
    check_eq(IsWindow(hwnd) as usize, 1);
    check_eq(IsWindowVisible(hwnd) as usize, 0);
    let mut pid = 0u32;
    check_eq(GetWindowThreadProcessId(hwnd, &mut pid) as usize, GetCurrentThreadId() as usize);
    check_eq(pid as usize, GetCurrentProcessId() as usize);
    let mut name = [0u16; 32];
    check_eq(GetClassNameW(hwnd, name.as_mut_ptr(), 32) as usize, 10);
    check_got(wide_eq(&name, "PolyS1Test"), 0);
    let mut short = [0xFFFFu16; 5];
    check_eq(GetClassNameW(hwnd, short.as_mut_ptr(), 5) as usize, 4);
    check_got(wide_eq(&short, "Poly"), 0);
    check_eq(GetDpiForWindow(hwnd) as usize, 96);

    // EnumThreadWindows finds it, and stops when asked to.
    ENUM_TARGET.store(hwnd as u32, Ordering::Relaxed);
    ENUM_SEEN.store(0, Ordering::Relaxed);
    check_eq(EnumThreadWindows(GetCurrentThreadId(), enum_cb, 0) as usize, 1);
    check_got(ENUM_SEEN.load(Ordering::Relaxed) >= 1, ENUM_SEEN.load(Ordering::Relaxed));
    ENUM_SEEN.store(0, Ordering::Relaxed);
    check_eq(EnumThreadWindows(0xFFFF_0000, enum_cb, 0) as usize, 1);
    check_eq(ENUM_SEEN.load(Ordering::Relaxed) as usize, 0);

    // Window properties: by name (case-insensitively) and by atom.
    let prop = w::<7>("Prop_A");
    check_eq(GetPropW(hwnd, prop.as_ptr()), 0);
    check_eq(SetPropW(hwnd, prop.as_ptr(), 0x1234) as usize, 1);
    check_eq(GetPropW(hwnd, w::<7>("PROP_A").as_ptr()), 0x1234);
    check_eq(SetPropW(hwnd, 0xC123 as *const u16, 0x77) as usize, 1);
    check_eq(GetPropW(hwnd, 0xC123 as *const u16), 0x77);
    check_eq(RemovePropW(hwnd, w::<7>("prop_a").as_ptr()), 0x1234);
    check_eq(RemovePropW(hwnd, prop.as_ptr()), 0);
    check_eq(RemovePropW(hwnd, 0xC123 as *const u16), 0x77);
    SetLastError(0);
    check_fail(SetPropW(0x7777_7777, prop.as_ptr(), 1) as usize, ERROR_INVALID_WINDOW_HANDLE);

    // Notifications and queries that answer "nothing".
    NotifyWinEvent(0x8000, hwnd, 0, 0);
    let guid = CLSID_NOWHERE;
    let reg = RegisterPowerSettingNotification(hwnd, &guid, 0);
    check_got(reg != 0, GetLastError());
    check_eq(UnregisterPowerSettingNotification(reg) as usize, 1);
    let mut li = [0u32; 2];
    li[0] = 8;
    check_eq(GetLastInputInfo(li.as_mut_ptr()) as usize, 1);
    li[0] = 4;
    SetLastError(0);
    check_fail(GetLastInputInfo(li.as_mut_ptr()) as usize, ERROR_INVALID_PARAMETER);
    check_eq(GetCaretBlinkTime() as usize, 530);

    // Display devices: one adapter, one monitor under it.
    let mut dd = [0u8; 840];
    dd[0..4].copy_from_slice(&840u32.to_le_bytes());
    check_eq(EnumDisplayDevicesW(core::ptr::null(), 0, dd.as_mut_ptr(), 0) as usize, 1);
    let dev: [u16; 12] = core::array::from_fn(|i| u16::from_le_bytes([dd[4 + i * 2], dd[5 + i * 2]]));
    check_got(wide_eq(&dev, "\\\\.\\DISPLAY1"), 0);
    check_eq(EnumDisplayDevicesW(core::ptr::null(), 1, dd.as_mut_ptr(), 0) as usize, 0);

    // Icons.
    check_got(LoadIconW(0, 32512) != 0, GetLastError());
    let mask = CreateBitmap(16, 16, 1, 1, core::ptr::null());
    let color = CreateBitmap(16, 16, 1, 32, core::ptr::null());
    let mut ii = [0u8; 32];
    ii[0..4].copy_from_slice(&1u32.to_le_bytes());
    ii[16..24].copy_from_slice(&(mask as u64).to_le_bytes());
    ii[24..32].copy_from_slice(&(color as u64).to_le_bytes());
    let icon = CreateIconIndirect(ii.as_ptr());
    check_got(icon != 0, GetLastError());
    check_eq(DestroyIcon(icon) as usize, 1);
    SetLastError(0);
    check_got(DestroyIcon(icon) == 0, 0);
    DeleteObject(mask);
    DeleteObject(color);

    check_eq(DestroyWindow(hwnd) as usize, 1);
    check_eq(IsWindow(hwnd) as usize, 0);
}

// ----- system parameters, colours, keyboard, DPI --------------------------------------

unsafe fn system() {
    // SPI_GETNONCLIENTMETRICS fills a 504-byte NONCLIENTMETRICSW.
    let mut ncm = [0u8; 504];
    ncm[0..4].copy_from_slice(&504u32.to_le_bytes());
    check_eq(SystemParametersInfoW(0x29, 504, ncm.as_mut_ptr(), 0) as usize, 1);
    check_eq(u32::from_le_bytes([ncm[8], ncm[9], ncm[10], ncm[11]]) as usize, 17); // iScrollWidth
    let face: [u16; 32] = core::array::from_fn(|i| u16::from_le_bytes([ncm[28 + 24 + i * 2], ncm[29 + 24 + i * 2]]));
    check_got(wide_eq(&face, "Segoe UI"), 0);
    // A wrong size fails.
    ncm[0..4].copy_from_slice(&100u32.to_le_bytes());
    SetLastError(0);
    check_fail(SystemParametersInfoW(0x29, 100, ncm.as_mut_ptr(), 0) as usize, ERROR_INVALID_PARAMETER);
    // SPI_GETWORKAREA, SPI_GETWHEELSCROLLLINES, an unknown action.
    let mut rect = [0i32; 4];
    check_eq(SystemParametersInfoW(0x30, 0, rect.as_mut_ptr().cast(), 0) as usize, 1);
    check_got(rect[2] > 0 && rect[3] > 0, rect[2] as u32);
    let mut lines = 0u32;
    check_eq(SystemParametersInfoW(0x68, 0, (&mut lines as *mut u32).cast(), 0) as usize, 1);
    check_eq(lines as usize, 3);
    SetLastError(0);
    check_fail(SystemParametersInfoW(0x7777, 0, (&mut lines as *mut u32).cast(), 0) as usize, ERROR_INVALID_PARAMETER);

    // System colours and brushes.
    check_eq(GetSysColor(5) as usize, 0x00FF_FFFF); // COLOR_WINDOW
    check_eq(GetSysColor(-1) as usize, 0);
    check_eq(GetSysColor(31) as usize, 0);
    let brush = GetSysColorBrush(5);
    check_got(brush != 0, GetLastError());
    check_eq(GetSysColorBrush(5), brush);
    check_eq(GetSysColorBrush(31), 0);

    // The keyboard layout list.
    check_eq(GetKeyboardLayoutList(0, core::ptr::null_mut()) as usize, 1);
    let mut hkl = 0usize;
    check_eq(GetKeyboardLayoutList(1, &mut hkl) as usize, 1);
    check_eq(hkl, GetKeyboardLayout(0));
    let mut kl = [0u16; 9];
    check_eq(GetKeyboardLayoutNameW(kl.as_mut_ptr()) as usize, 1);
    check_got(wide_eq(&kl, "00000407"), 0);

    // DPI: once per process; the context reads back; everything is 96.
    check_eq(SetProcessDpiAwarenessContext(-4) as usize, 1); // PER_MONITOR_AWARE_V2
    SetLastError(0);
    check_fail(SetProcessDpiAwarenessContext(-3) as usize, ERROR_ACCESS_DENIED);
    check_eq(AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(), -4) as usize, 1);
    check_eq(AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(), -1) as usize, 0);
    // The per-monitor functions come through the shcore contract (user32 here).
    let mon = MonitorFromWindow(0, 1); // MONITOR_DEFAULTTOPRIMARY
    check_got(mon != 0, 0);
    let (mut dx, mut dy) = (0u32, 0u32);
    check_hr(GetDpiForMonitor(mon, 0, &mut dx, &mut dy), S_OK);
    check_got(dx == 96 && dy == 96, dx);
    check_hr(GetDpiForMonitor(mon, 9, &mut dx, &mut dy), 0x8007_0057);
    let mut aw = 9u32;
    check_hr(GetProcessDpiAwareness(0, &mut aw), S_OK);
    check_eq(aw as usize, 2); // per monitor, set above
}

// ----- GDI: fonts, text metrics, device caps -----------------------------------------

unsafe fn gdi() {
    // DEFAULT_GUI_FONT -> LOGFONTW (92 bytes) -> a new font -> metrics.
    let stock = GetStockObject(17);
    check_got(stock != 0, GetLastError());
    let mut lf = [0u8; 92];
    check_eq(GetObjectW(stock, 92, lf.as_mut_ptr()) as usize, 92);
    let font = CreateFontIndirectW(lf.as_ptr());
    check_got(font != 0, GetLastError());
    let dc = GetDC(0);
    check_got(dc != 0, GetLastError());
    check_eq(GetDeviceCaps(dc, 88) as usize, 96); // LOGPIXELSX
    check_eq(GetDeviceCaps(dc, 90) as usize, 96); // LOGPIXELSY
    check_got(GetDeviceCaps(dc, 8) > 0 && GetDeviceCaps(dc, 10) > 0, 0); // HORZRES, VERTRES
    check_got(GetDeviceCaps(dc, 12) >= 8, GetDeviceCaps(dc, 12) as u32); // BITSPIXEL
    let old = SelectObject(dc, font);
    check_got(old != 0, GetLastError());
    let mut tm = [0u8; 60];
    check_eq(GetTextMetricsW(dc, tm.as_mut_ptr()) as usize, 1);
    let height = i32::from_le_bytes([tm[0], tm[1], tm[2], tm[3]]);
    check_got(height > 0, height as u32);
    SelectObject(dc, old);
    check_eq(ReleaseDC(0, dc) as usize, 1);
    check_eq(DeleteObject(font) as usize, 1);
}

#[unsafe(no_mangle)]
pub extern "system" fn s1test_main(_peb: *mut u8) -> u32 {
    out(b"s1test: start\n");
    // SAFETY: every call is an ole32/user32/gdi32/kernel32 export with valid arguments.
    unsafe {
        com();
        winrt();
        windows();
        system();
        gdi();
        out(b"s1test: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}
