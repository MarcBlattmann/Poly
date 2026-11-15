//! A 32-bit Windows GUI program (ADR 0186, stage 4): built for `i686-pc-windows-msvc` by
//! `cargo xtask wow64`, it imports from Poly's i686 `user32.dll`, `gdi32.dll` and `kernel32.dll`
//! and checks, one line on standard output each:
//!
//! - `class`: `RegisterClassExW` with the 48-byte 32-bit `WNDCLASSEXW`;
//! - `create`: `CreateWindowExW` makes an overlapped window; its window procedure (a 32-bit
//!   `stdcall` function `user32` calls) sees `WM_CREATE` with the `lpParam` given in the 32-bit
//!   `CREATESTRUCTW`, and `GetWindowTextW` / `GetClientRect` answer;
//! - `paint`: with `--paint` (on Poly's desktop), `ShowWindow` + `UpdateWindow` send
//!   `WM_PAINT`, whose handler fills the update rectangle through `BeginPaint` / `FillRect` /
//!   `EndPaint` (the 64-byte 32-bit `PAINTSTRUCT`) and reads the colour back with `GetPixel`;
//!   without it (the kernel test, no interactive display: nothing is visible, so nothing is
//!   invalid) `BeginPaint` is called directly and the fill goes to a memory DC;
//! - `messages`: `SendMessageW` returns the window procedure's result; a posted message reaches
//!   it through `GetMessageW` / `DispatchMessageW` with its `wParam` and `lParam`, and
//!   `DestroyWindow` / `PostQuitMessage` end the loop with the quit code.
//!
//! Started with `--show` it keeps its window open for 30 seconds instead of closing it (for a
//! screenshot: `cargo xtask win-app run --mode desktop --args "--show --paint"`). It ends with `ExitProcess(0)` when every check passed, else with the number of
//! the first that failed.

#![no_std]
#![no_main]

extern crate poly_msvc32;

use core::sync::atomic::{AtomicU32, Ordering};

type Handle = isize;

#[repr(C)]
struct WndClassEx {
    size: u32,
    style: u32,
    wndproc: extern "system" fn(Handle, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: Handle,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PaintStruct {
    hdc: Handle,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct Msg {
    hwnd: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WndClassEx) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: usize,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
    fn ShowWindow(hwnd: Handle, cmd: i32) -> i32;
    fn UpdateWindow(hwnd: Handle) -> i32;
    fn BeginPaint(hwnd: Handle, ps: *mut PaintStruct) -> Handle;
    fn EndPaint(hwnd: Handle, ps: *const PaintStruct) -> i32;
    fn FillRect(hdc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn GetClientRect(hwnd: Handle, rect: *mut Rect) -> i32;
    fn GetWindowTextW(hwnd: Handle, text: *mut u16, max: i32) -> i32;
    fn SendMessageW(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize;
    fn PostMessageW(hwnd: Handle, msg: u32, w: usize, l: isize) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn DestroyWindow(hwnd: Handle) -> i32;
    fn PostQuitMessage(code: i32);
    fn SetTimer(hwnd: Handle, id: usize, ms: u32, f: usize) -> usize;
}

#[link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn CreateSolidBrush(color: u32) -> Handle;
    fn GetPixel(hdc: Handle, x: i32, y: i32) -> u32;
    fn DeleteObject(object: Handle) -> i32;
    fn CreateCompatibleDC(hdc: Handle) -> Handle;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bits: u32, data: *const u8) -> Handle;
    fn SelectObject(hdc: Handle, object: Handle) -> Handle;
    fn DeleteDC(hdc: Handle) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetCommandLineW() -> *const u16;
    fn ExitProcess(code: u32) -> !;
}

/// `WM_*`, `WS_OVERLAPPEDWINDOW`, `SW_SHOW` (`winuser.h`).
const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_PAINT: u32 = 0x000F;
const WM_TIMER: u32 = 0x0113;
const WM_USER: u32 = 0x0400;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const SW_SHOW: i32 = 5;
/// The fill colour (`COLORREF`: 0x00BBGGRR) and the `lpParam` given to `CreateWindowExW`.
const FILL: u32 = 0x00C0_478A;
const PARAM: usize = 0x1234_5678;

static CREATED_WITH: AtomicU32 = AtomicU32::new(0);
static PAINTED: AtomicU32 = AtomicU32::new(u32::MAX);
static POSTED: AtomicU32 = AtomicU32::new(0);
static SHOW: AtomicU32 = AtomicU32::new(0);
static PAINT_ON_SCREEN: AtomicU32 = AtomicU32::new(0);
/// How many `WM_PAINT`s came, and the last `BeginPaint` DC.
static PAINTS: AtomicU32 = AtomicU32::new(0);
static PAINT_DC: AtomicU32 = AtomicU32::new(0);

fn out(text: &[u8]) {
    let mut written = 0u32;
    // SAFETY: plain kernel32 calls with valid buffers.
    unsafe { WriteFile(GetStdHandle(-11i32 as u32), text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut()) };
}

fn wide<const N: usize>(text: &str) -> [u16; N] {
    let mut b = [0u16; N];
    for (d, s) in b.iter_mut().zip(text.encode_utf16()) {
        *d = s;
    }
    b
}

extern "system" fn wndproc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: user32's documented calls with the arguments it passed.
    unsafe {
        match msg {
            WM_CREATE => {
                // CREATESTRUCTW: `lpCreateParams` first.
                let params = (l as *const usize).read();
                CREATED_WITH.store(params as u32, Ordering::SeqCst);
                0
            }
            WM_PAINT => {
                let mut ps: PaintStruct = core::mem::zeroed();
                PAINTS.fetch_add(1, Ordering::SeqCst);
                let hdc = BeginPaint(hwnd, &mut ps);
                PAINT_DC.store(hdc as u32, Ordering::SeqCst);
                if hdc != 0 {
                    let brush = CreateSolidBrush(FILL);
                    FillRect(hdc, &ps.paint, brush);
                    DeleteObject(brush);
                    PAINTED.store(GetPixel(hdc, 5, 5), Ordering::SeqCst);
                }
                EndPaint(hwnd, &ps);
                0
            }
            m if m == WM_USER + 1 => {
                POSTED.store((w as u32) << 16 | l as u32, Ordering::SeqCst);
                if SHOW.load(Ordering::SeqCst) != 0 {
                    SetTimer(hwnd, 1, 30_000, 0);
                } else {
                    DestroyWindow(hwnd);
                }
                0
            }
            m if m == WM_USER + 2 => 0x55,
            WM_TIMER => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(3);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

/// `v` as 8 hex digits.
fn hex(v: u32) -> [u8; 8] {
    core::array::from_fn(|i| b"0123456789abcdef"[(v >> (28 - 4 * i)) as usize & 0xF])
}

fn fail(n: u32, what: &[u8]) -> ! {
    out(b"failed: ");
    out(what);
    out(b" (painted ");
    out(&hex(PAINTED.load(Ordering::SeqCst)));
    out(b", paints ");
    out(&hex(PAINTS.load(Ordering::SeqCst)));
    out(b", dc ");
    out(&hex(PAINT_DC.load(Ordering::SeqCst)));
    out(b", posted ");
    out(&hex(POSTED.load(Ordering::SeqCst)));
    out(b")\n");
    // SAFETY: ends the process.
    unsafe { ExitProcess(n) }
}

/// Whether the command line contains `flag` (at most 7 characters).
fn flag_given(flag: &str) -> bool {
    // SAFETY: the process's NUL-terminated command line.
    unsafe {
        let line = GetCommandLineW();
        let mut v = [0u16; 512];
        let mut n = 0;
        while n < v.len() && line.add(n).read() != 0 {
            v[n] = line.add(n).read();
            n += 1;
        }
        let flag: [u16; 7] = wide(flag);
        let len = flag.iter().position(|&c| c == 0).unwrap_or(7);
        v[..n].windows(len).any(|w| w == &flag[..len])
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> u32 {
    if flag_given("--show") {
        SHOW.store(1, Ordering::SeqCst);
    }
    if flag_given("--paint") {
        PAINT_ON_SCREEN.store(1, Ordering::SeqCst);
    }
    out(b"gui32\n");
    let class: [u16; 16] = wide("PolyGui32");
    let title: [u16; 32] = wide("Poly 32-bit window");
    // SAFETY: plain user32/gdi32 calls with valid structures.
    unsafe {
        let instance = GetModuleHandleW(core::ptr::null());
        let wc = WndClassEx {
            size: core::mem::size_of::<WndClassEx>() as u32,
            style: 3,
            wndproc,
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: core::ptr::null(),
            class_name: class.as_ptr(),
            icon_sm: 0,
        };
        if core::mem::size_of::<WndClassEx>() != 48 || RegisterClassExW(&wc) == 0 {
            fail(2, b"class");
        }
        out(b"class\n");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            40,
            40,
            320,
            200,
            0,
            0,
            instance,
            PARAM,
        );
        if hwnd == 0 || CREATED_WITH.load(Ordering::SeqCst) != PARAM as u32 {
            fail(3, b"create");
        }
        let mut text = [0u16; 32];
        let n = GetWindowTextW(hwnd, text.as_mut_ptr(), 32) as usize;
        let mut client = Rect::default();
        if text[..n] != title[..18] || GetClientRect(hwnd, &mut client) == 0 || client.right <= 0 || client.bottom <= 0
        {
            fail(3, b"create");
        }
        out(b"create\n");
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        // On a desktop (`--paint`) the window was invalid when shown and UpdateWindow sent
        // WM_PAINT; without one (the kernel test) nothing of it is visible and nothing is
        // painted (Poly paints only on an interactive display), so BeginPaint is called here
        // and the drawing goes to a memory DC.
        if PAINT_ON_SCREEN.load(Ordering::SeqCst) != 0 {
            if PAINTED.load(Ordering::SeqCst) != FILL {
                fail(4, b"paint");
            }
        } else {
            let mut ps: PaintStruct = core::mem::zeroed();
            ps.restore = 0x5A5A;
            let dc = BeginPaint(hwnd, &mut ps);
            let same = dc != 0 && ps.hdc == dc && ps.restore == 0;
            EndPaint(hwnd, &ps);
            let mem = CreateCompatibleDC(0);
            let bitmap = CreateBitmap(16, 16, 1, 32, core::ptr::null());
            let old = SelectObject(mem, bitmap);
            let brush = CreateSolidBrush(FILL);
            let r = Rect { left: 0, top: 0, right: 16, bottom: 16 };
            FillRect(mem, &r, brush);
            PAINTED.store(GetPixel(mem, 5, 5), Ordering::SeqCst);
            SelectObject(mem, old);
            DeleteObject(brush);
            DeleteObject(bitmap);
            DeleteDC(mem);
            if !same || PAINTED.load(Ordering::SeqCst) != FILL {
                fail(4, b"paint");
            }
        }
        if core::mem::size_of::<PaintStruct>() != 64 {
            fail(4, b"paint");
        }
        out(b"paint\n");
        if SendMessageW(hwnd, WM_USER + 2, 0, 0) != 0x55 || PostMessageW(hwnd, WM_USER + 1, 7, 9) == 0 {
            fail(5, b"messages");
        }
        let mut msg = Msg::default();
        let mut quit = None;
        for _ in 0..10_000 {
            match GetMessageW(&mut msg, 0, 0, 0) {
                0 => {
                    quit = Some(msg.wparam);
                    break;
                }
                -1 => break,
                _ => {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        if quit != Some(3) || POSTED.load(Ordering::SeqCst) != (7 << 16 | 9) {
            fail(5, b"messages");
        }
        out(b"messages\n");
        ExitProcess(0)
    }
}

/// MSVC-style code references this once floating point is used.
#[unsafe(no_mangle)]
#[used]
static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: ends the process.
    unsafe { ExitProcess(0xDEAD) }
}
