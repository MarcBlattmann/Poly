//! Our `user32.dll` (ADR 0087): the two string functions console programs
//! such as 7-Zip's command line import from it, `CharUpperW` and
//! `CharPrevExA`, and the first Phase 7 slice's exports as explicit stubs
//! (ADR 0128). The string functions' decisions are
//! `poly_kernelbase::oleaut`'s; the case mapping is our `ntdll`'s
//! (`RtlUpcaseUnicodeChar`, the object manager's upcase table).

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_rt as _;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlUpcaseUnicodeChar(c: u16) -> u16;
}

/// `poly-kernelbase` links `alloc`, so the DLL needs an allocator, though
/// nothing here allocates: every request fails.
struct NoHeap;

// SAFETY: `alloc` returns null (an allocation failure), which is always
// sound; `dealloc` is never reached with a block.
unsafe impl core::alloc::GlobalAlloc for NoHeap {
    unsafe fn alloc(&self, _: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: NoHeap = NoHeap;

fn upcase(c: u16) -> u16 {
    // SAFETY: an ntdll export without preconditions.
    unsafe { RtlUpcaseUnicodeChar(c) }
}

/// `CharUpperW`: Learn: a string is converted in place and returned; a
/// value whose high-order word is zero is a single character, returned
/// converted in the low-order word. Poly maps case without the locale
/// (Learn: "CharUpper always maps lowercase I ("i") to uppercase I").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharUpperW(s: *mut u16) -> *mut u16 {
    if poly_kernelbase::oleaut::is_single_char(s as usize) {
        return usize::from(upcase(s as usize as u16)) as *mut u16;
    }
    let mut p = s;
    // SAFETY: a NUL-terminated string the caller owns.
    unsafe {
        while *p != 0 {
            *p = upcase(*p);
            p = p.add(1);
        }
    }
    s
}

/// `CharPrevExA`: the character before `current`, or `start` at the start
/// (Poly's code pages have no lead bytes; `dwFlags` is reserved).
#[unsafe(no_mangle)]
pub extern "system" fn CharPrevExA(_code_page: u16, start: *const u8, current: *const u8, _flags: u32) -> *mut u8 {
    poly_kernelbase::oleaut::char_prev(start as usize, current as usize) as *mut u8
}

// The first Phase 7 slice (ADR 0128): window classes and windows, the
// message loop, timers, painting. Stubs until its spec sheets arrive: each
// sets `ERROR_CALL_NOT_IMPLEMENTED` and returns its Learn page's failure
// value (`poly_win32k::exports::USER32_STUBS` is the list `cargo xtask
// win32` checks). Windows, queues and input are win32k's (the NT
// personality, through `win32u.dll`'s `PolyUser*` stubs); window procedures,
// `DefWindowProc` and message dispatch will run here.
poly_win32_gui::not_implemented_stubs! {
    /// `RegisterClassExW(lpwcx)`: zero on failure.
    RegisterClassExW(wcx) => 0;
    /// `RegisterClassW(lpWndClass)`: zero on failure.
    RegisterClassW(wc) => 0;
    /// `UnregisterClassW(lpClassName, hInstance)`: zero on failure.
    UnregisterClassW(class, instance) => 0;
    /// `GetClassInfoExW(hInstance, lpszClass, lpwcx)`: zero on failure.
    GetClassInfoExW(instance, class, wcx) => 0;
    /// `CreateWindowExW(...)`: NULL on failure.
    CreateWindowExW(ex_style, class, name, style, x, y, width, height, parent, menu, instance, param) => 0;
    /// `DestroyWindow(hWnd)`: zero on failure.
    DestroyWindow(hwnd) => 0;
    /// `ShowWindow(hWnd, nCmdShow)`: zero (the window was hidden before).
    ShowWindow(hwnd, cmd) => 0;
    /// `UpdateWindow(hWnd)`: zero on failure.
    UpdateWindow(hwnd) => 0;
    /// `SetWindowPos(hWnd, hWndInsertAfter, X, Y, cx, cy, uFlags)`: zero on failure.
    SetWindowPos(hwnd, after, x, y, cx, cy, flags) => 0;
    /// `MoveWindow(hWnd, X, Y, nWidth, nHeight, bRepaint)`: zero on failure.
    MoveWindow(hwnd, x, y, width, height, repaint) => 0;
    /// `GetWindowRect(hWnd, lpRect)`: zero on failure.
    GetWindowRect(hwnd, rect) => 0;
    /// `GetClientRect(hWnd, lpRect)`: zero on failure.
    GetClientRect(hwnd, rect) => 0;
    /// `GetWindowLongPtrW(hWnd, nIndex)`: zero on failure.
    GetWindowLongPtrW(hwnd, index) => 0;
    /// `SetWindowLongPtrW(hWnd, nIndex, dwNewLong)`: zero on failure.
    SetWindowLongPtrW(hwnd, index, value) => 0;
    /// `SetWindowTextW(hWnd, lpString)`: zero on failure.
    SetWindowTextW(hwnd, text) => 0;
    /// `GetWindowTextW(hWnd, lpString, nMaxCount)`: zero on failure.
    GetWindowTextW(hwnd, text, max) => 0;
    /// `DefWindowProcW(hWnd, Msg, wParam, lParam)`: zero.
    DefWindowProcW(hwnd, msg, wparam, lparam) => 0;
    /// `GetMessageW(lpMsg, hWnd, wMsgFilterMin, wMsgFilterMax)`: -1 on an error.
    GetMessageW(msg, hwnd, min, max) => usize::MAX;
    /// `PeekMessageW(lpMsg, hWnd, wMsgFilterMin, wMsgFilterMax, wRemoveMsg)`: zero (no message).
    PeekMessageW(msg, hwnd, min, max, remove) => 0;
    /// `TranslateMessage(lpMsg)`: zero (not translated).
    TranslateMessage(msg) => 0;
    /// `DispatchMessageW(lpMsg)`: zero.
    DispatchMessageW(msg) => 0;
    /// `PostMessageW(hWnd, Msg, wParam, lParam)`: zero on failure.
    PostMessageW(hwnd, msg, wparam, lparam) => 0;
    /// `PostThreadMessageW(idThread, Msg, wParam, lParam)`: zero on failure.
    PostThreadMessageW(thread, msg, wparam, lparam) => 0;
    /// `SendMessageW(hWnd, Msg, wParam, lParam)`: zero.
    SendMessageW(hwnd, msg, wparam, lparam) => 0;
    /// `PostQuitMessage(nExitCode)`: no return value.
    PostQuitMessage(code) => 0;
    /// `WaitMessage()`: zero on failure.
    WaitMessage() => 0;
    /// `MsgWaitForMultipleObjectsEx(nCount, pHandles, dwMilliseconds, dwWakeMask, dwFlags)`: `WAIT_FAILED`.
    MsgWaitForMultipleObjectsEx(count, handles, ms, wake_mask, flags) => 0xFFFF_FFFF;
    /// `SetTimer(hWnd, nIDEvent, uElapse, lpTimerFunc)`: zero on failure.
    SetTimer(hwnd, id, elapse, func) => 0;
    /// `KillTimer(hWnd, uIDEvent)`: zero on failure.
    KillTimer(hwnd, id) => 0;
    /// `BeginPaint(hWnd, lpPaint)`: NULL on failure.
    BeginPaint(hwnd, paint) => 0;
    /// `EndPaint(hWnd, lpPaint)`: always nonzero.
    EndPaint(hwnd, paint) => 1;
    /// `InvalidateRect(hWnd, lpRect, bErase)`: zero on failure.
    InvalidateRect(hwnd, rect, erase) => 0;
    /// `ValidateRect(hWnd, lpRect)`: zero on failure.
    ValidateRect(hwnd, rect) => 0;
    /// `GetUpdateRect(hWnd, lpRect, bErase)`: zero (no update region).
    GetUpdateRect(hwnd, rect, erase) => 0;
    /// `GetDC(hWnd)`: NULL on failure.
    GetDC(hwnd) => 0;
    /// `ReleaseDC(hWnd, hDC)`: zero (not released).
    ReleaseDC(hwnd, hdc) => 0;
    /// `FillRect(hDC, lprc, hbr)`: zero on failure.
    FillRect(hdc, rect, brush) => 0;
    /// `LoadCursorW(hInstance, lpCursorName)`: NULL on failure.
    LoadCursorW(instance, name) => 0;
    /// `LoadIconW(hInstance, lpIconName)`: NULL on failure.
    LoadIconW(instance, name) => 0;
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn user32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
