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
    fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
    fn RtlCaptureStackBackTrace(skip: u32, count: u32, frames: *mut *mut u8, hash: *mut u32) -> u16;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentProcess() -> isize;
    fn GetTickCount64() -> u64;
    fn LoadLibraryExW(name: *const u16, file: *mut u8, flags: u32) -> *mut u8;
    fn GetProcAddress(module: *mut u8, name: *const u8) -> *mut u8;
    fn FreeLibrary(module: *mut u8) -> i32;
    fn CreateEventW(security: *mut u8, manual_reset: i32, initial_state: i32, name: *const u16) -> isize;
    fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
    fn CloseHandle(handle: isize) -> i32;
    fn GetThreadPreferredUILanguages(flags: u32, count: *mut u32, buf: *mut u16, cch: *mut u32) -> i32;
    fn LocalFree(block: *mut u8) -> *mut u8;
    fn GetModuleFileNameW(module: *mut u8, buf: *mut u16, size: u32) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn GetModuleHandleExW(flags: u32, address: *const u8, out: *mut *mut u8) -> i32;
    fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualProtect(addr: *mut u8, size: usize, protect: u32, old: *mut u32) -> i32;
    fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
    fn ReadProcessMemory(process: isize, base: *const u8, buf: *mut u8, size: usize, read: *mut usize) -> i32;
    fn WriteProcessMemory(process: isize, base: *mut u8, buf: *const u8, size: usize, written: *mut usize) -> i32;
    fn QueryProcessCycleTime(process: isize, cycles: *mut u64) -> i32;
    fn GetProcessInformation(process: isize, class: u32, info: *mut u8, size: u32) -> i32;
    fn SetProcessShutdownParameters(level: u32, flags: u32) -> i32;
    fn GetProcessShutdownParameters(level: *mut u32, flags: *mut u32) -> i32;
    fn K32GetMappedFileNameW(process: isize, address: usize, name: *mut u16, size: u32) -> u32;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn LdrLockLoaderLock(flags: u32, disposition: *mut u32, cookie: *mut usize) -> u32;
    fn LdrUnlockLoaderLock(flags: u32, cookie: usize) -> u32;
    fn RtlFormatCurrentUserKeyPath(path: *mut UnicodeString) -> u32;
    fn RtlFreeUnicodeString(s: *mut UnicodeString);
    fn NtCreateKey(
        key: *mut isize,
        access: u32,
        attributes: *const ObjectAttributes,
        title_index: u32,
        class: *const UnicodeString,
        options: u32,
        disposition: *mut u32,
    ) -> u32;
    fn NtClose(handle: isize) -> u32;
}

/// `UNICODE_STRING` (`winternl.h`).
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

/// `OBJECT_ATTRIBUTES` (`winternl.h`).
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: isize,
    name: *const UnicodeString,
    attributes: u32,
    security_descriptor: *const u8,
    security_qos: *const u8,
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CommandLineToArgvW(cmd_line: *const u16, num_args: *mut i32) -> *mut *mut u16;
    fn SHGetFolderPathW(hwnd: *mut u8, folder: i32, token: *mut u8, flags: u32, path: *mut u16) -> u32;
    fn SHGetKnownFolderPath(id: *const [u8; 16], flags: u32, token: *mut u8, path: *mut *mut u16) -> u32;
    fn SHGetPropertyStoreForWindow(hwnd: usize, iid: *const [u8; 16], out: *mut *mut u8) -> u32;
}

#[link(name = "userenv", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterGPNotification(event: isize, machine: i32) -> i32;
    fn UnregisterGPNotification(event: isize) -> i32;
    fn GetProfileType(flags: *mut u32) -> i32;
}

#[link(name = "winmm", kind = "raw-dylib")]
unsafe extern "system" {
    fn timeGetTime() -> u32;
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
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
    fn GetDesktopWindow() -> Hwnd;
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

    // The keyboard layout list: three installed layouts, as the oracle of
    // specs/windows/imm32.dll/ImmIsIME.yaml has (its visible test records the
    // count); the first one is the thread's current layout.
    check_eq(GetKeyboardLayoutList(0, core::ptr::null_mut()) as usize, 3);
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
    // A type that does not exist: ERROR_BAD_ARGUMENTS as an HRESULT
    // (specs/windows/shcore.dll/GetDpiForMonitor.yaml `bad_type`).
    check_hr(GetDpiForMonitor(mon, 9, &mut dx, &mut dy), 0x8007_00A0);
    let mut aw = 9u32;
    check_hr(GetProcessDpiAwareness(0, &mut aw), S_OK);
    check_eq(aw as usize, 2); // per monitor, set above
}

// ----- GDI: fonts, text metrics, device caps -----------------------------------------

unsafe extern "C" {
    static __ImageBase: u8;
}

/// RtlCaptureStackBackTrace: the frames of this thread's stack, the first in this program.
unsafe fn backtrace() {
    let mut frames = [core::ptr::null_mut::<u8>(); 32];
    let mut hash = 0u32;
    let n = RtlCaptureStackBackTrace(0, 32, frames.as_mut_ptr(), &mut hash) as usize;
    check_got(n >= 2, n as u32);
    let base = &raw const __ImageBase as usize;
    let inside = frames[0] as usize;
    check_got(inside > base && inside < base + 0x100000, 0);
    // Skipping one drops the first frame.
    let mut again = [core::ptr::null_mut::<u8>(); 32];
    let m = RtlCaptureStackBackTrace(1, 32, again.as_mut_ptr(), core::ptr::null_mut()) as usize;
    check_eq(usize::from(m + 1 == n || m == n), 1);
    check_eq(again[0] as usize, frames[1] as usize);
    check_got(hash != 0, hash);
}

/// `NtCurrentTeb()`: `gs:[0x30]`.
unsafe fn teb() -> *const u8 {
    let teb: *const u8;
    core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, readonly, preserves_flags));
    teb
}

/// The loader lock through the PEB (spec-chrome-elf-null items 1, 2, 4, 5): `PEB+0x110` points
/// to an `RTL_CRITICAL_SECTION` inside ntdll that is free here (-1, 0, 0) and is the one
/// `LdrLockLoaderLock` takes (-2, 1, this thread's id, TEB+0x48) and `LdrUnlockLoaderLock` frees.
/// This is what Chrome's crash reporter reads without calling anything.
unsafe fn loader_lock() {
    let teb = teb();
    let peb = *(teb.add(0x60) as *const *const u8);
    check_got(!peb.is_null(), 0);
    let lock = *(peb.add(0x110) as *const *const u8);
    check_got(!lock.is_null(), 0);
    // Inside ntdll's image.
    let mut owner = core::ptr::null_mut();
    check_eq(GetModuleHandleExW(0x4, lock, &mut owner) as usize, 1); // FROM_ADDRESS
    let ntdll = w::<16>("ntdll.dll");
    check_eq(owner as usize, GetModuleHandleW(ntdll.as_ptr()) as usize);
    let tid = *(teb.add(0x48) as *const u64);
    check_eq(tid as usize, GetCurrentThreadId() as usize);
    let fields = || {
        (
            *(lock.add(0x08) as *const i32),
            *(lock.add(0x0C) as *const i32),
            *(lock.add(0x10) as *const u64),
            *(lock.add(0x18) as *const u64),
        )
    };
    // Free: LockCount -1, RecursionCount 0, OwningThread 0, no semaphore.
    check_got(fields() == (-1, 0, 0, 0), 0);
    let (mut disposition, mut cookie) = (0u32, 0usize);
    check_eq(LdrLockLoaderLock(0, &mut disposition, &mut cookie) as usize, 0);
    check_eq(disposition as usize, 1); // LDR_LOCK_LOADER_LOCK_DISPOSITION_LOCK_ACQUIRED
    check_got(cookie != 0, 0);
    // Held: LockCount -2, RecursionCount 1, OwningThread = this thread.
    check_got(fields() == (-2, 1, tid, 0), 0);
    check_eq(LdrUnlockLoaderLock(0, cookie) as usize, 0);
    check_got(fields() == (-1, 0, 0, 0), 0);
    // A statically initialised section with a debug record (`winnt.h`:
    // RTL_CRITICAL_SECTION_FLAG_STATIC_INIT in SpinCount; DebugInfo points back to it).
    let debug = *(lock as *const *const u8);
    check_got(!debug.is_null(), 0);
    check_eq(*(debug.add(0x08) as *const usize), lock as usize);
    check_eq(*(lock.add(0x20) as *const usize), 0x0400_0000);
    // The field does not move for the life of the process.
    check_eq(*(peb.add(0x110) as *const usize), lock as usize);
}

/// The current user's registry key, the way chrome_elf opens it (cold trace n=275..277):
/// `RtlFormatCurrentUserKeyPath`, a backslash appended, `NtCreateKey` with access 3
/// (KEY_QUERY_VALUE | KEY_SET_VALUE) and OBJ_CASE_INSENSITIVE succeeds because the hive is there.
unsafe fn current_user_key() {
    let mut path = UnicodeString { length: 0, maximum_length: 0, buffer: core::ptr::null_mut() };
    check_eq(RtlFormatCurrentUserKeyPath(&mut path) as usize, 0);
    let chars = usize::from(path.length) / 2;
    // "\REGISTRY\USER\S-1-5-21-...": at least the prefix and a SID.
    check_got(chars > 20 && chars < 120 && !path.buffer.is_null(), chars as u32);
    let mut name = [0u16; 128];
    core::ptr::copy_nonoverlapping(path.buffer, name.as_mut_ptr(), chars);
    name[chars] = u16::from(b'\\');
    RtlFreeUnicodeString(&mut path);
    let name = UnicodeString {
        length: ((chars + 1) * 2) as u16,
        maximum_length: ((chars + 2) * 2) as u16,
        buffer: name.as_mut_ptr(),
    };
    let attributes = ObjectAttributes {
        length: core::mem::size_of::<ObjectAttributes>() as u32,
        root: 0,
        name: &name,
        attributes: 0x40, // OBJ_CASE_INSENSITIVE
        security_descriptor: core::ptr::null(),
        security_qos: core::ptr::null(),
    };
    let mut key = 0isize;
    check_eq(NtCreateKey(&mut key, 3, &attributes, 0, core::ptr::null(), 0, core::ptr::null_mut()) as usize, 0);
    check_got(key != 0, 0);
    check_eq(NtClose(key) as usize, 0);
}

/// Whether the NUL-terminated wide string at `p` is `want`.
unsafe fn wide_is(p: *const u16, want: &str) -> bool {
    let mut i = 0;
    for c in want.encode_utf16() {
        if *p.add(i) != c {
            return false;
        }
        i += 1;
    }
    *p.add(i) == 0
}

/// shell32 (Learn: `CommandLineToArgvW`, `SHGetFolderPathW`): the arguments of a command line
/// like the one Chrome starts its crash handler with, in one block freed with `LocalFree`;
/// and the user's local application data folder (CSIDL_LOCAL_APPDATA, what Chrome asks for).
unsafe fn shell32() {
    let line = w::<96>(
        "\"C:\\Program Files\\Chrome\\chrome.exe\" --type=x \"--dir=C:\\my profile\" a\\\\\\\"b\\\" /prefetch:4",
    );
    let mut count = 0i32;
    let argv = CommandLineToArgvW(line.as_ptr(), &mut count);
    check_got(!argv.is_null(), 0);
    check_eq(count as usize, 5);
    check_got(wide_is(*argv, "C:\\Program Files\\Chrome\\chrome.exe"), 0);
    check_got(wide_is(*argv.add(1), "--type=x"), 1);
    check_got(wide_is(*argv.add(2), "--dir=C:\\my profile"), 2);
    check_got(wide_is(*argv.add(3), "a\\\"b\""), 3);
    check_got(wide_is(*argv.add(4), "/prefetch:4"), 4);
    // One block: the strings lie behind the pointers, and one LocalFree releases it.
    check_got((*argv as usize) > argv as usize && (*argv as usize) < argv as usize + 256, 0);
    check_eq(LocalFree(argv.cast()) as usize, 0);
    // An empty command line gives the executable's path.
    let empty = [0u16; 1];
    let argv = CommandLineToArgvW(empty.as_ptr(), &mut count);
    check_got(!argv.is_null() && count == 1, count as u32);
    let mut own = [0u16; 260];
    let n = GetModuleFileNameW(core::ptr::null_mut(), own.as_mut_ptr(), 260) as usize;
    check_got(n > 0 && (0..=n).all(|i| *(*argv).add(i) == own[i]), n as u32);
    check_eq(LocalFree(argv.cast()) as usize, 0);
    // The contract Chrome loads first for it (LOAD_LIBRARY_SEARCH_SYSTEM32, 0x800) has the function.
    let contract = w::<48>("api-ms-win-downlevel-shell32-l1-1-0.dll");
    let module = LoadLibraryExW(contract.as_ptr(), core::ptr::null_mut(), 0x800);
    check_got(!module.is_null(), 0);
    check_got(!GetProcAddress(module, c"CommandLineToArgvW".as_ptr().cast()).is_null(), 0);
    check_eq(FreeLibrary(module) as usize, 1);
    // CSIDL_LOCAL_APPDATA (0x1C), SHGFP_TYPE_CURRENT (0): %LOCALAPPDATA%, no trailing backslash.
    let mut path = [0u16; 260];
    check_eq(SHGetFolderPathW(core::ptr::null_mut(), 0x1C, core::ptr::null_mut(), 0, path.as_mut_ptr()) as usize, 0);
    let mut want = [0u16; 260];
    let name = w::<16>("LOCALAPPDATA");
    let n = GetEnvironmentVariableW(name.as_ptr(), want.as_mut_ptr(), 260) as usize;
    check_got(n > 3 && (0..=n).all(|i| path[i] == want[i]) && path[n - 1] != u16::from(b'\\'), n as u32);
    // A value that is no CSIDL: E_INVALIDARG.
    check_eq(
        SHGetFolderPathW(core::ptr::null_mut(), 0x7F, core::ptr::null_mut(), 0, path.as_mut_ptr()) as usize,
        0x8007_0057,
    );
}

/// `GetThreadPreferredUILanguages` the way chrome.dll calls it when it starts (Learn; a FALSE
/// here is fatal for Chrome): `MUI_LANGUAGE_NAME | MUI_UI_FALLBACK` (0x38), first for the size,
/// then for the list, which is the user's language and its neutral language.
unsafe fn ui_languages() {
    let (mut count, mut size) = (0u32, 0u32);
    check_eq(GetThreadPreferredUILanguages(0x38, &mut count, core::ptr::null_mut(), &mut size) as usize, 1);
    check_eq(count as usize, 2);
    check_eq(size as usize, 10); // "en-US\0en\0\0"
    let mut list = [0xAAAAu16; 16];
    let mut cch = size;
    check_eq(GetThreadPreferredUILanguages(0x38, &mut count, list.as_mut_ptr(), &mut cch) as usize, 1);
    check_got(wide_is(list.as_ptr(), "en-US") && wide_is(list.as_ptr().add(6), "en") && list[9] == 0, cch);
    check_eq(list[10] as usize, 0xAAAA);
    // A buffer that is too small: ERROR_INSUFFICIENT_BUFFER (122) and the size needed.
    let mut small = 4u32;
    check_eq(GetThreadPreferredUILanguages(0x38, &mut count, list.as_mut_ptr(), &mut small) as usize, 0);
    check_eq(GetLastError() as usize, 122);
    check_eq(small as usize, 10);
    // Both forms at once: ERROR_INVALID_PARAMETER (87).
    let mut cch = 16u32;
    check_eq(GetThreadPreferredUILanguages(0xC, &mut count, list.as_mut_ptr(), &mut cch) as usize, 0);
    check_eq(GetLastError() as usize, 87);
}

/// userenv's policy notifications (Learn: `RegisterGPNotification`), which Chrome's policy
/// loader delay-loads (the export has to exist): registering an event succeeds for the user
/// and the machine, the event is not set (policy does not change on Poly), and it unregisters.
unsafe fn gp_notification() {
    let event = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
    check_got(event != 0, 0);
    check_eq(RegisterGPNotification(event, 0) as usize, 1);
    check_eq(RegisterGPNotification(event, 1) as usize, 1);
    check_eq(WaitForSingleObject(event, 0) as usize, 258); // WAIT_TIMEOUT
    check_eq(UnregisterGPNotification(event) as usize, 1);
    check_eq(RegisterGPNotification(0, 0) as usize, 0);
    check_eq(GetLastError() as usize, 87);
    check_eq(CloseHandle(event) as usize, 1);
}

/// winmm's timer functions (Learn: `timeGetTime`, `timeBeginPeriod`, `timeEndPeriod`): the
/// time is milliseconds since the system started (non-zero after boot, not going back; the
/// same clock as `GetTickCount`), and a 1 ms period request is granted and ended.
unsafe fn winmm() {
    let a = timeGetTime();
    check_got(a != 0, a);
    let ticks = GetTickCount64();
    let b = timeGetTime();
    // The same clock as GetTickCount64, read between the two (wrapping as a DWORD).
    check_got(b.wrapping_sub(a) < 10_000 && (ticks as u32).wrapping_sub(a) < 10_000, b);
    check_eq(timeBeginPeriod(1) as usize, 0); // TIMERR_NOERROR
    check_eq(timeEndPeriod(1) as usize, 0);
    check_eq(timeBeginPeriod(0) as usize, 97); // TIMERR_NOCANDO: out of range
}

/// GetCurrentThreadStackLimits: the stack this code runs on lies between the limits.
unsafe fn stack_limits() {
    let (mut low, mut high) = (0usize, 0usize);
    GetCurrentThreadStackLimits(&mut low, &mut high);
    let here = &low as *const usize as usize;
    check_got(low != 0 && low < here && here < high, 0);
    check_eq(high % 16, 0);
}

/// ReadProcessMemory / WriteProcessMemory on the calling process (Chrome's chrome_elf reads and
/// patches ntdll's stubs this way).
unsafe fn process_memory() {
    let me = GetCurrentProcess();
    let page = VirtualAlloc(core::ptr::null_mut(), 4096, 0x3000, 0x04); // MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE
    check_got(!page.is_null(), GetLastError());
    page.write_bytes(0x11, 4096);
    let mut buf = [0u8; 32];
    let mut n = 0usize;
    check_eq(ReadProcessMemory(me, page, buf.as_mut_ptr(), 32, &mut n) as usize, 1);
    check_got(n == 32 && buf == [0x11; 32], n as u32);
    // A read-only page is written by making it writable for the copy.
    let mut old = 0u32;
    check_eq(VirtualProtect(page, 4096, 0x02, &mut old) as usize, 1); // PAGE_READONLY
    let patch = [0xE9u8, 1, 2, 3, 4];
    check_eq(WriteProcessMemory(me, page.add(8), patch.as_ptr(), 5, &mut n) as usize, 1);
    check_eq(n, 5);
    check_got(*page.add(8) == 0xE9 && *page.add(12) == 4 && *page.add(13) == 0x11, 0);
    let mut now = 0u32;
    check_eq(VirtualProtect(page, 4096, 0x02, &mut now) as usize, 1);
    check_eq(now as usize, 0x02); // put back
    // The page after the allocation is reserved but not committed: a read across it fails
    // halfway with ERROR_PARTIAL_COPY (299) and says how far it got.
    let mut big = [0u8; 64];
    SetLastError(0);
    let r = ReadProcessMemory(me, page.add(4096 - 16), big.as_mut_ptr(), 64, &mut n);
    check_got(r != 0 || (GetLastError() == 299 && n == 16), GetLastError());
    // Another process is not reachable yet.
    SetLastError(0);
    check_fail(ReadProcessMemory(0x7777_0000, page, buf.as_mut_ptr(), 8, &mut n) as usize, 6); // ERROR_INVALID_HANDLE
    VirtualFree(page, 0, 0x8000);
}

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

/// A function of a DLL, found the way Chrome's delay-load helper finds it: `LoadLibraryExW`
/// then `GetProcAddress` by name or, for a `name` of one byte, by that ordinal.
unsafe fn delay_load(dll: &str, name: &core::ffi::CStr) -> *mut u8 {
    let mut wide = [0u16; 64];
    for (to, c) in wide.iter_mut().zip(dll.encode_utf16()) {
        *to = c;
    }
    let module = LoadLibraryExW(wide.as_ptr(), core::ptr::null_mut(), 0);
    check_got(!module.is_null(), GetLastError());
    let f = GetProcAddress(module, name.as_ptr().cast());
    check_got(!f.is_null(), GetLastError());
    f
}

/// [`delay_load`] by ordinal.
unsafe fn delay_load_ordinal(dll: &str, ordinal: usize) -> *mut u8 {
    let mut wide = [0u16; 64];
    for (to, c) in wide.iter_mut().zip(dll.encode_utf16()) {
        *to = c;
    }
    let module = LoadLibraryExW(wide.as_ptr(), core::ptr::null_mut(), 0);
    check_got(!module.is_null(), GetLastError());
    let f = GetProcAddress(module, ordinal as *const u8);
    check_got(!f.is_null(), GetLastError());
    f
}

/// A COM object for `QISearch`: one interface (a vtable pointer) and a reference count.
#[repr(C)]
struct Counted {
    vtbl: *const [unsafe extern "system" fn(*mut Counted) -> u32; 3],
    refs: u32,
}

unsafe extern "system" fn counted_nop(_this: *mut Counted) -> u32 {
    0
}

unsafe extern "system" fn counted_add_ref(this: *mut Counted) -> u32 {
    (*this).refs += 1;
    (*this).refs
}

static COUNTED_VTBL: [unsafe extern "system" fn(*mut Counted) -> u32; 3] = [counted_nop, counted_add_ref, counted_nop];

/// The DLLs and functions Chrome's browser needs right after `ChromeMain` (ADR 0155, step 9),
/// each reached the way Chrome reaches it. From Learn; none observed on Windows.
unsafe fn step9() {
    // shlwapi: PathMatchSpecW by name, IsOS by ordinal 437, QISearch by ordinal 219.
    let match_spec: unsafe extern "system" fn(*const u16, *const u16) -> i32 =
        core::mem::transmute(delay_load("SHLWAPI.dll", c"PathMatchSpecW"));
    let (file, spec, other) = (w::<8>("main.c"), w::<12>("*.txt; *.c"), w::<8>("*.h"));
    check_eq(match_spec(file.as_ptr(), spec.as_ptr()) as usize, 1);
    check_eq(match_spec(file.as_ptr(), other.as_ptr()) as usize, 0);
    let is_os: unsafe extern "system" fn(u32) -> i32 = core::mem::transmute(delay_load_ordinal("SHLWAPI.dll", 437));
    check_eq(is_os(1) as usize, 1); // OS_NT
    check_eq(is_os(29) as usize, 0); // OS_ANYSERVER
    let qi_search: unsafe extern "system" fn(
        *mut Counted,
        *const [usize; 2],
        *const [u8; 16],
        *mut *mut Counted,
    ) -> u32 = core::mem::transmute(delay_load_ordinal("SHLWAPI.dll", 219));
    let mut object = Counted { vtbl: &COUNTED_VTBL, refs: 1 };
    let iid = CLSID_NOWHERE;
    // A QITAB: { &iid, offset 0 }, then the terminating entry.
    let table: [[usize; 2]; 2] = [[iid.as_ptr() as usize, 0], [0, 0]];
    let mut found: *mut Counted = core::ptr::null_mut();
    check_hr(qi_search(&mut object, table.as_ptr(), &iid, &mut found) as i32, S_OK);
    check_got(core::ptr::eq(found, &object) && object.refs == 2, object.refs);
    check_hr(qi_search(&mut object, table.as_ptr(), &IID_IUNKNOWN, &mut found) as i32, S_OK);
    check_eq(object.refs as usize, 3);
    let unknown = [7u8; 16];
    check_hr(qi_search(&mut object, table.as_ptr(), &unknown, &mut found) as i32, E_NOINTERFACE);
    check_got(found.is_null() && object.refs == 3, object.refs);

    // wtsapi32: the session notification and the connect state of the current session.
    let register: unsafe extern "system" fn(usize, u32) -> i32 =
        core::mem::transmute(delay_load("WTSAPI32.dll", c"WTSRegisterSessionNotification"));
    let unregister: unsafe extern "system" fn(usize) -> i32 =
        core::mem::transmute(delay_load("WTSAPI32.dll", c"WTSUnRegisterSessionNotification"));
    let query: unsafe extern "system" fn(usize, u32, u32, *mut *mut u8, *mut u32) -> i32 =
        core::mem::transmute(delay_load("WTSAPI32.dll", c"WTSQuerySessionInformationW"));
    let free: unsafe extern "system" fn(*mut u8) = core::mem::transmute(delay_load("WTSAPI32.dll", c"WTSFreeMemory"));
    // The sheet: what is no window is ERROR_INVALID_PARAMETER for both; a window is registered,
    // unregistered, and then not registered (ERROR_FILE_NOT_FOUND).
    check_fail(register(0x1234, 0) as usize, ERROR_INVALID_PARAMETER);
    check_fail(unregister(0x1234) as usize, ERROR_INVALID_PARAMETER);
    check_fail(register(0, 0) as usize, ERROR_INVALID_PARAMETER);
    let desktop = GetDesktopWindow();
    check_eq(register(desktop, 0) as usize, 1);
    check_eq(unregister(desktop) as usize, 1);
    check_fail(unregister(desktop) as usize, 2);
    let (mut block, mut bytes) = (core::ptr::null_mut(), 0u32);
    // WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSConnectState: WTSActive (0).
    check_eq(query(0, u32::MAX, 8, &mut block, &mut bytes) as usize, 1);
    check_got(!block.is_null() && bytes == 4 && block.cast::<u32>().read_unaligned() == 0, bytes);
    free(block);
    // WTSUserName: a NUL-terminated string.
    check_eq(query(0, u32::MAX, 5, &mut block, &mut bytes) as usize, 1);
    check_got(bytes >= 2 && bytes % 2 == 0 && block.add(bytes as usize - 2).cast::<u16>().read_unaligned() == 0, bytes);
    free(block);

    // comctl32: an INITCOMMONCONTROLSEX of 8 bytes is accepted.
    let init: unsafe extern "system" fn(*const u32) -> i32 =
        core::mem::transmute(delay_load("COMCTL32.dll", c"InitCommonControlsEx"));
    let icc = [8u32, 0xFFFF];
    check_eq(init(icc.as_ptr()) as usize, 1);
    let bad = [4u32, 0xFFFF];
    check_eq(init(bad.as_ptr()) as usize, 0);

    // The power API set: ProcessorInformation (11) fills one 24-byte record per processor.
    let power: unsafe extern "system" fn(u32, *const u8, u32, *mut u8, u32) -> u32 =
        core::mem::transmute(delay_load("api-ms-win-power-base-l1-1-0.dll", c"CallNtPowerInformation"));
    let mut records = [0u8; 24 * 64];
    check_eq(power(11, core::ptr::null(), 0, records.as_mut_ptr(), records.len() as u32) as usize, 0);
    let mhz = u32::from_le_bytes([records[4], records[5], records[6], records[7]]);
    check_got(mhz != 0, mhz);
    check_eq(power(11, core::ptr::null(), 0, records.as_mut_ptr(), 8) as usize, 0xC000_0023);

    // The accessibility and media DLLs load and have the names Chrome calls.
    for (dll, name) in [
        ("OLEACC.dll", c"CreateStdAccessibleObject"),
        ("OLEACC.dll", c"LresultFromObject"),
        ("UIAutomationCore.DLL", c"UiaReturnRawElementProvider"),
        ("UIAutomationCore.DLL", c"UiaHostProviderFromHwnd"),
        ("UIAutomationCore.DLL", c"UiaDisconnectProvider"),
        ("MFPlat.DLL", c"MFStartup"),
        ("ntdll.dll", c"NtSuspendProcess"),
        ("ntdll.dll", c"NtResumeProcess"),
    ] {
        delay_load(dll, name);
    }

    // kernel32: the cycle time grows, GetProcessInformation reads the default memory priority,
    // the shutdown parameters are kept, and the mapped file name of this program is a device path.
    let (mut before, mut after) = (0u64, 0u64);
    check_eq(QueryProcessCycleTime(GetCurrentProcess(), &mut before) as usize, 1);
    let start = GetTickCount64();
    while GetTickCount64() < start + 30 {
        core::hint::spin_loop();
    }
    check_eq(QueryProcessCycleTime(GetCurrentProcess(), &mut after) as usize, 1);
    check_got(after > before, (after - before) as u32);
    let mut priority = 0u32;
    check_eq(GetProcessInformation(GetCurrentProcess(), 0, (&raw mut priority).cast(), 4) as usize, 1);
    check_eq(priority as usize, 5); // MEMORY_PRIORITY_NORMAL
    check_fail(GetProcessInformation(GetCurrentProcess(), 0, (&raw mut priority).cast(), 8) as usize, 24);
    check_eq(SetProcessShutdownParameters(0x100, 1) as usize, 1);
    let (mut level, mut flags) = (0u32, 0u32);
    check_eq(GetProcessShutdownParameters(&mut level, &mut flags) as usize, 1);
    check_got(level == 0x100 && flags == 1, level);
    // The sheet: every level is accepted; a flag other than SHUTDOWN_NORETRY is error 641.
    check_eq(SetProcessShutdownParameters(0x500, 0) as usize, 1);
    check_fail(SetProcessShutdownParameters(0x100, 2) as usize, 641);
    check_eq(SetProcessShutdownParameters(0x100, 1) as usize, 1);
    let mut name = [0u16; 260];
    let n =
        K32GetMappedFileNameW(GetCurrentProcess(), s1test_main as *const () as usize, name.as_mut_ptr(), 260) as usize;
    let device = w::<8>("\\Device\\");
    check_got(n > 20 && name[..8] == device[..8] && name[n] == 0, n as u32);
    check_got(name[n - 10..n].iter().zip("s1test.exe".encode_utf16()).all(|(a, b)| *a == b), n as u32);
    // An address nothing is mapped at: ERROR_UNEXP_NET_ERR (59) on Windows (the K32GetMappedFileNameW sheet).
    check_fail(K32GetMappedFileNameW(GetCurrentProcess(), 0x10, name.as_mut_ptr(), 260) as usize, 59);

    // shell32: FOLDERID_LocalAppData is %LOCALAPPDATA%, in a CoTaskMemAlloc block.
    let local_app_data: [u8; 16] =
        [0x85, 0x27, 0xB3, 0xF1, 0xBA, 0x6F, 0xCF, 0x4F, 0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91];
    let mut path: *mut u16 = core::ptr::null_mut();
    check_hr(SHGetKnownFolderPath(&local_app_data, 0, core::ptr::null_mut(), &mut path) as i32, S_OK);
    let mut want = [0u16; 260];
    let variable = w::<16>("LOCALAPPDATA");
    let n = GetEnvironmentVariableW(variable.as_ptr(), want.as_mut_ptr(), 260) as usize;
    check_got(n > 3 && (0..=n).all(|i| *path.add(i) == want[i]), n as u32);
    CoTaskMemFree(path.cast());
    // An id that is no known folder: HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND) and a null path (the sheet).
    check_hr(SHGetKnownFolderPath(&CLSID_NOWHERE, 0, core::ptr::null_mut(), &mut path) as i32, 0x8007_0002);
    check_got(path.is_null(), 0);

    // The property store of a window: what is set is read back, by a second store object too.
    let iid_store: [u8; 16] =
        [0xEB, 0x8E, 0x6D, 0x88, 0xF2, 0x8C, 0x46, 0x44, 0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99];
    // A real window: the store of a window that is gone takes no value (the sheet).
    let class_name = w::<11>("PolyS1Test");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        class_name.as_ptr(),
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
    let mut store: *mut *const [usize; 8] = core::ptr::null_mut();
    check_hr(SHGetPropertyStoreForWindow(hwnd, &iid_store, (&raw mut store).cast()) as i32, S_OK);
    check_got(!store.is_null(), 0);
    let vtbl = &**store;
    let set_value: unsafe extern "system" fn(*mut c_void, *const [u8; 20], *const [u8; 24]) -> u32 =
        core::mem::transmute(vtbl[6]);
    let get_value: unsafe extern "system" fn(*mut c_void, *const [u8; 20], *mut [u8; 24]) -> u32 =
        core::mem::transmute(vtbl[5]);
    let get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> u32 = core::mem::transmute(vtbl[3]);
    let commit: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vtbl[7]);
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vtbl[2]);
    // PKEY_AppUserModel_ID: {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5 (propkey.h).
    let key: [u8; 20] =
        [0x55, 0x28, 0x4C, 0x9F, 0x79, 0x9F, 0x39, 0x4B, 0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3, 5, 0, 0, 0];
    let id = w::<16>("Poly.S1Test");
    let mut variant = [0u8; 24];
    variant[0] = 31; // VT_LPWSTR
    variant[8..16].copy_from_slice(&(id.as_ptr() as u64).to_le_bytes());
    check_hr(set_value(store.cast(), &key, &variant) as i32, S_OK);
    check_hr(commit(store.cast()) as i32, S_OK);
    let mut count = 0u32;
    check_hr(get_count(store.cast(), &mut count) as i32, S_OK);
    check_eq(count as usize, 1);
    check_eq(release(store.cast()) as usize, 0);
    check_hr(SHGetPropertyStoreForWindow(hwnd, &iid_store, (&raw mut store).cast()) as i32, S_OK);
    let mut read = [0u8; 24];
    check_hr(get_value(store.cast(), &key, &mut read) as i32, S_OK);
    let text = u64::from_le_bytes([read[8], read[9], read[10], read[11], read[12], read[13], read[14], read[15]]);
    check_got(read[0] == 31 && text != 0 && wide_is(text as *const u16, "Poly.S1Test"), u32::from(read[0]));
    check_got(text != id.as_ptr() as u64, 0);
    CoTaskMemFree(text as *mut u8);
    // A key the store does not have: VT_EMPTY, S_OK.
    let mut other = key;
    other[16] = 9;
    check_hr(get_value(store.cast(), &other, &mut read) as i32, S_OK);
    check_eq(usize::from(read[0]), 0);
    check_eq(release(store.cast()) as usize, 0);
    // Another interface is refused.
    check_hr(SHGetPropertyStoreForWindow(hwnd, &CLSID_NOWHERE, (&raw mut store).cast()) as i32, E_NOINTERFACE);
    check_got(store.is_null(), 0);
    check_eq(DestroyWindow(hwnd) as usize, 1);

    // oleaut32 by ordinal, as chrome.dll's delay loads bind it: a vector of four VT_I4 (3).
    let create: unsafe extern "system" fn(u16, i32, u32) -> *mut u8 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 411));
    let upper: unsafe extern "system" fn(*mut u8, u32, *mut i32) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 19));
    let access: unsafe extern "system" fn(*mut u8, *mut *mut i32) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 23));
    let unaccess: unsafe extern "system" fn(*mut u8) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 24));
    let get: unsafe extern "system" fn(*mut u8, *const i32, *mut i32) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 25));
    let vartype: unsafe extern "system" fn(*mut u8, *mut u16) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 77));
    let destroy: unsafe extern "system" fn(*mut u8) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 16));
    let byte_len: unsafe extern "system" fn(*mut u16) -> u32 =
        core::mem::transmute(delay_load_ordinal("OLEAUT32.dll", 149));
    for ordinal in [17, 20, 26, 150, 162, 314] {
        delay_load_ordinal("OLEAUT32.dll", ordinal);
    }
    let array = create(3, 10, 4);
    check_got(!array.is_null(), 0);
    let mut high = 0i32;
    check_hr(upper(array, 1, &mut high) as i32, S_OK);
    check_eq(high as usize, 13);
    check_hr(upper(array, 2, &mut high) as i32, 0x8002_000B); // DISP_E_BADINDEX
    let mut data: *mut i32 = core::ptr::null_mut();
    check_hr(access(array, &mut data) as i32, S_OK);
    check_got(!data.is_null() && *data == 0, 0);
    *data.add(2) = 77;
    check_hr(destroy(array) as i32, 0x8002_000D); // DISP_E_ARRAYISLOCKED
    check_hr(unaccess(array) as i32, S_OK);
    let (index, mut element) = (12i32, 0i32);
    check_hr(get(array, &index, &mut element) as i32, S_OK);
    check_eq(element as usize, 77);
    let mut vt = 0u16;
    check_hr(vartype(array, &mut vt) as i32, S_OK);
    check_eq(usize::from(vt), 3);
    check_hr(destroy(array) as i32, S_OK);
    check_eq(byte_len(core::ptr::null_mut()) as usize, 0);

    // The system's version, read the way Learn's "Getting the System Version" says and Chrome
    // does when it starts: the version resource of kernelbase.dll and of kernel32.dll.
    let info_size: unsafe extern "system" fn(*const u16, *mut u32) -> u32 =
        core::mem::transmute(delay_load("VERSION.dll", c"GetFileVersionInfoSizeW"));
    let info: unsafe extern "system" fn(*const u16, u32, u32, *mut u8) -> i32 =
        core::mem::transmute(delay_load("VERSION.dll", c"GetFileVersionInfoW"));
    let query: unsafe extern "system" fn(*const u8, *const u16, *mut *const u8, *mut u32) -> i32 =
        core::mem::transmute(delay_load("VERSION.dll", c"VerQueryValueW"));
    for file in [w::<16>("kernelbase.dll"), w::<16>("kernel32.dll")] {
        let mut handle = 1u32;
        let size = info_size(file.as_ptr(), &mut handle);
        check_got(size > 52 && size < 2048, GetLastError());
        let mut block = [0u8; 2048];
        check_eq(info(file.as_ptr(), 0, size, block.as_mut_ptr()) as usize, 1);
        let root = w::<2>("\\");
        let (mut fixed, mut len) = (core::ptr::null(), 0u32);
        check_eq(query(block.as_ptr(), root.as_ptr(), &mut fixed, &mut len) as usize, 1);
        check_eq(len as usize, 52);
        // VS_FIXEDFILEINFO: the signature, then dwProductVersionMS at 16: 10.0.
        check_eq(fixed.cast::<u32>().read_unaligned() as usize, 0xFEEF_04BD);
        check_eq(fixed.add(16).cast::<u32>().read_unaligned() as usize, 0x000A_0000);
        let name = w::<48>("\\StringFileInfo\\040904B0\\ProductVersion");
        let (mut text, mut chars) = (core::ptr::null(), 0u32);
        check_eq(query(block.as_ptr(), name.as_ptr(), &mut text, &mut chars) as usize, 1);
        check_got(chars == 16 && wide_is(text.cast(), "10.0.19045.2006"), chars);
    }

    // userenv: the profile is a local one.
    let mut profile = 0xFFu32;
    check_eq(GetProfileType(&mut profile) as usize, 1);
    check_eq(profile as usize, 0);
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
        process_memory();
        stack_limits();
        backtrace();
        loader_lock();
        current_user_key();
        shell32();
        ui_languages();
        gp_notification();
        winmm();
        step9();
        out(b"s1test: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}
