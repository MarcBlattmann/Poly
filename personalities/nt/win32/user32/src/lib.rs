//! Our `user32.dll` (ADR 0087, 0128): windows, messages and the default
//! window procedure. Window state lives in win32k (the NT personality),
//! reached through `win32u.dll`'s `PolyUserCall`; the message sequences
//! that call window procedures are `poly_win32k::user`'s, run here with
//! [`Sys`]; drawing goes to our `gdi32.dll`. Also the two string functions
//! 7-Zip's command line imports (`CharUpperW`, `CharPrevExA`, ADR 0087).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use poly_win32_gui::{set_last_error, wide};
use poly_win32_rt as _;
use poly_win32k::frame::Rect;
use poly_win32k::metrics;
use poly_win32k::proto::{ClassRef, ClassSpec, CreateSpec, Reply, Request, WinInfo};
use poly_win32k::user::{self as u, CMsg, CRect, Env};
use poly_win32k::wire;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlUpcaseUnicodeChar(c: u16) -> u16;
}

#[link(name = "win32u", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: u64, out: *mut u8, cap: u64, out_len: *mut u64) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetTickCount() -> u32;
    fn CreateEventW(attrs: *mut u8, manual: i32, initial: i32, name: *const u16) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn WaitForMultipleObjects(n: u32, handles: *const usize, all: i32, ms: u32) -> u32;
    fn Sleep(ms: u32);
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyGdiCreateDisplayDC(hwnd: usize, persistent: i32) -> usize;
    fn PolyGdiReleaseDisplayDC(hdc: usize) -> i32;
    fn PolyGdiDeleteDisplayDC(hdc: usize) -> i32;
    fn PolyGdiFillRect(hdc: usize, rect: *const i32, brush: usize, color: u32) -> i32;
    fn PolyGdiDrawText(hdc: usize, text: *const u16, count: i32, rect: *mut i32, format: u32) -> i32;
}

const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;
const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_TIMEOUT: u32 = 0x102;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_INVALID_HANDLE: u32 = 6;

type H = usize;
type WndProc = unsafe extern "system" fn(H, u32, usize, isize) -> isize;

/// This process's side of win32k.
struct Sys;

static SYS: Sys = Sys;

fn call(req: Request) -> Reply {
    SYS.call(req)
}

impl Sys {
    fn queue_event(&self) -> usize {
        if let Reply::Value { value } = call(Request::RegisterEvent { handle: 0 })
            && value != 0
        {
            return value as usize;
        }
        // SAFETY: an unnamed auto-reset event.
        let h = unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) };
        call(Request::RegisterEvent { handle: h as u64 });
        h
    }
}

fn ms_of(ms: Option<u64>) -> u32 {
    ms.map_or(INFINITE, |m| m.min(0xFFFF_FFFE) as u32)
}

impl Env for Sys {
    fn call(&self, req: Request) -> Reply {
        let bytes = wire::encode(&req);
        let retry = matches!(
            req,
            Request::GetText { .. }
                | Request::Relatives { .. }
                | Request::ClassName { .. }
                | Request::ClassInfo { .. }
                | Request::Window { .. }
        );
        let mut out = vec![0u8; 4096];
        loop {
            let mut len = 0u64;
            // SAFETY: the buffers are ours and sized as passed.
            let s = unsafe {
                PolyUserCall(bytes.as_ptr(), bytes.len() as u64, out.as_mut_ptr(), out.len() as u64, &mut len)
            };
            if s == STATUS_BUFFER_TOO_SMALL && retry && len as usize > out.len() {
                out = vec![0u8; len as usize];
                continue;
            }
            if s != 0 {
                return Reply::Error { code: ERROR_INVALID_PARAMETER };
            }
            return wire::decode(&out[..len as usize]).unwrap_or(Reply::Error { code: ERROR_INVALID_PARAMETER });
        }
    }

    fn call_proc(&self, proc_: u64, hwnd: u32, msg: u32, w: u64, l: u64) -> u64 {
        if proc_ == 0 {
            return u::def_window_proc(self, u64::from(hwnd), msg, w, l);
        }
        // SAFETY: a window procedure the app registered.
        let f: WndProc = unsafe { core::mem::transmute(proc_ as usize) };
        // SAFETY: the documented call.
        unsafe { f(hwnd as H, msg, w as usize, l as isize) as u64 }
    }

    fn tid(&self) -> u32 {
        // SAFETY: no preconditions.
        unsafe { GetCurrentThreadId() }
    }

    fn set_last_error(&self, e: u32) {
        set_last_error(e);
    }

    fn tick(&self) -> u32 {
        // SAFETY: no preconditions.
        unsafe { GetTickCount() }
    }

    fn wait_queue(&self, ms: Option<u64>) {
        let h = self.queue_event();
        // SAFETY: our event.
        unsafe { WaitForSingleObject(h, ms_of(ms)) };
    }

    fn wait_handles(&self, handles: &[u64], wait_all: bool, ms: Option<u64>, queue: bool) -> Option<u32> {
        let mut hs: Vec<usize> = handles.iter().map(|&h| h as usize).collect();
        if queue {
            hs.push(self.queue_event());
        }
        if hs.is_empty() {
            // SAFETY: no preconditions.
            unsafe { Sleep(ms_of(ms)) };
            return None;
        }
        // SAFETY: valid handles of the caller and ours.
        let r = unsafe { WaitForMultipleObjects(hs.len() as u32, hs.as_ptr(), i32::from(wait_all), ms_of(ms)) };
        if r == WAIT_TIMEOUT || r as usize >= hs.len() { None } else { Some(r) }
    }

    fn fill_rect(&self, hdc: u64, rect: CRect, brush: u64) {
        let r = [rect.left, rect.top, rect.right, rect.bottom];
        fill(hdc as H, &r, brush as H);
    }

    fn delete_dc(&self, hdc: u64) {
        // SAFETY: a DC our gdi32 made.
        unsafe { PolyGdiDeleteDisplayDC(hdc as H) };
    }
}

fn info(hwnd: H) -> Option<WinInfo> {
    match call(Request::Window { hwnd: hwnd as u64 }) {
        Reply::Window { info } => Some(info),
        _ => None,
    }
}

fn bad_window<T: Default>() -> T {
    set_last_error(ERROR_INVALID_WINDOW_HANDLE);
    T::default()
}

/// A class named by an atom (a value below 0x10000) or a string.
unsafe fn class_ref(p: *const u16) -> ClassRef {
    if (p as usize) < 0x1_0000 {
        ClassRef::Atom { atom: p as usize as u16 }
    } else {
        // SAFETY: a NUL-terminated name.
        ClassRef::Name { name: unsafe { wide(p, -1) } }
    }
}

// ----- strings (ADR 0087)

fn upcase(c: u16) -> u16 {
    // SAFETY: an ntdll export without preconditions.
    unsafe { RtlUpcaseUnicodeChar(c) }
}

/// `CharUpperW`: a string in place, or one character in the low word.
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

/// `CharPrevExA`.
#[unsafe(no_mangle)]
pub extern "system" fn CharPrevExA(_code_page: u16, start: *const u8, current: *const u8, _flags: u32) -> *mut u8 {
    poly_kernelbase::oleaut::char_prev(start as usize, current as usize) as *mut u8
}

// ----- classes

fn register(spec: ClassSpec) -> u16 {
    if spec.cls_extra < 0 || spec.wnd_extra < 0 || spec.name.len() > 255 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if spec.name.is_empty() {
        set_last_error(ERROR_INVALID_NAME);
        return 0;
    }
    match call(Request::RegisterClass { spec }) {
        Reply::Atom { atom } => atom,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterClassExW(wc: *const u8) -> u16 {
    if wc.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a WNDCLASSEXW (winuser.h, 80 bytes on x64).
    unsafe {
        let r32 = |o: usize| (wc.add(o) as *const u32).read_unaligned();
        let r64 = |o: usize| (wc.add(o) as *const u64).read_unaligned();
        if r32(0) != 80 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        let name = r64(64) as *const u16;
        if (name as usize) < 0x1_0000 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        register(ClassSpec {
            name: wide(name, -1),
            style: r32(4),
            wndproc: r64(8),
            cls_extra: r32(16) as i32,
            wnd_extra: r32(20) as i32,
            instance: r64(24),
            icon: r64(32),
            cursor: r64(40),
            brush: r64(48),
            menu_name: r64(56),
            icon_sm: r64(72),
            unicode: true,
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterClassW(wc: *const u8) -> u16 {
    if wc.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a WNDCLASSW (72 bytes on x64).
    unsafe {
        let r32 = |o: usize| (wc.add(o) as *const u32).read_unaligned();
        let r64 = |o: usize| (wc.add(o) as *const u64).read_unaligned();
        let name = r64(64) as *const u16;
        if (name as usize) < 0x1_0000 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        register(ClassSpec {
            name: wide(name, -1),
            style: r32(0),
            wndproc: r64(8),
            cls_extra: r32(16) as i32,
            wnd_extra: r32(20) as i32,
            instance: r64(24),
            icon: r64(32),
            cursor: r64(40),
            brush: r64(48),
            menu_name: r64(56),
            icon_sm: 0,
            unicode: true,
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnregisterClassW(name: *const u16, instance: H) -> i32 {
    if name.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a name or an atom.
    let class = unsafe { class_ref(name) };
    match call(Request::UnregisterClass { class, instance: instance as u64 }) {
        Reply::Ok {} => 1,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClassInfoExW(instance: H, name: *const u16, wc: *mut u8) -> i32 {
    if name.is_null() || wc.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a name or an atom.
    let class = unsafe { class_ref(name) };
    match call(Request::ClassInfo { class, instance: instance as u64 }) {
        Reply::Class { info } => {
            // SAFETY: the caller's WNDCLASSEXW.
            unsafe {
                let w32 = |o: usize, v: u32| (wc.add(o) as *mut u32).write_unaligned(v);
                let w64 = |o: usize, v: u64| (wc.add(o) as *mut u64).write_unaligned(v);
                w32(4, info.style);
                w64(8, if info.wndproc == 0 { DefWindowProcW as *const () as u64 } else { info.wndproc });
                w32(16, info.cls_extra as u32);
                w32(20, info.wnd_extra as u32);
                w64(24, info.instance);
                w64(32, info.icon);
                w64(40, info.cursor);
                w64(48, info.brush);
                w64(56, info.menu_name);
                w64(64, name as u64);
                w64(72, info.icon_sm);
            }
            i32::from(info.atom)
        }
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClassLongPtrW(hwnd: H, index: i32) -> usize {
    match call(Request::ClassLong { hwnd: hwnd as u64, index }) {
        Reply::Value { value } => value as usize,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClassLongW(hwnd: H, index: i32) -> u32 {
    GetClassLongPtrW(hwnd, index) as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClassNameW(hwnd: H, buf: *mut u16, max: i32) -> i32 {
    match call(Request::ClassName { hwnd: hwnd as u64 }) {
        Reply::Text { text } => {
            if max <= 0 || buf.is_null() {
                return 0;
            }
            let n = text.len().min(max as usize - 1);
            // SAFETY: the caller's buffer of `max` characters.
            unsafe {
                core::ptr::copy_nonoverlapping(text.as_ptr(), buf, n);
                *buf.add(n) = 0;
            }
            n as i32
        }
        _ => bad_window(),
    }
}

// ----- windows

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateWindowExW(
    ex_style: u32,
    class: *const u16,
    name: *const u16,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: H,
    menu: H,
    instance: H,
    param: H,
) -> H {
    if class.is_null() {
        set_last_error(poly_win32k::wm::ERROR_CANNOT_FIND_WND_CLASS);
        return 0;
    }
    // SAFETY: a class name or atom.
    let class = unsafe { class_ref(class) };
    let spec = CreateSpec {
        ex_style,
        class,
        style,
        x,
        y,
        cx: w,
        cy: h,
        parent: parent as u64,
        menu: menu as u64,
        instance: instance as u64,
    };
    u::create_window(&SYS, spec, name as u64, param as u64) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyWindow(hwnd: H) -> i32 {
    i32::from(u::destroy_window(&SYS, hwnd as u64))
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowWindow(hwnd: H, cmd: i32) -> i32 {
    i32::from(u::show_window(&SYS, hwnd as u64, cmd))
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetWindowPos(hwnd: H, after: H, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32 {
    i32::from(u::set_window_pos(&SYS, hwnd as u64, after as u64, x, y, cx, cy, flags))
}

#[unsafe(no_mangle)]
pub extern "system" fn MoveWindow(hwnd: H, x: i32, y: i32, w: i32, h: i32, repaint: i32) -> i32 {
    let flags = u::SWP_NOZORDER | u::SWP_NOACTIVATE | if repaint == 0 { u::SWP_NOREDRAW } else { 0 };
    i32::from(u::set_window_pos(&SYS, hwnd as u64, 0, x, y, w, h, flags))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetParent(hwnd: H, parent: H) -> H {
    u::set_parent(&SYS, hwnd as u64, parent as u64) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn IsWindow(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some())
}

#[unsafe(no_mangle)]
pub extern "system" fn IsWindowVisible(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.visible))
}

#[unsafe(no_mangle)]
pub extern "system" fn IsWindowUnicode(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.unicode))
}

#[unsafe(no_mangle)]
pub extern "system" fn IsIconic(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.minimized))
}

#[unsafe(no_mangle)]
pub extern "system" fn IsZoomed(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.maximized))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDesktopWindow() -> H {
    u::desktop(&SYS) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn GetParent(hwnd: H) -> H {
    match call(Request::GetParent { hwnd: hwnd as u64 }) {
        Reply::Handle { hwnd } => hwnd as H,
        _ => bad_window(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetAncestor(hwnd: H, flags: u32) -> H {
    match call(Request::Ancestor { hwnd: hwnd as u64, flags }) {
        Reply::Handle { hwnd } => hwnd as H,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindow(hwnd: H, cmd: u32) -> H {
    match call(Request::GetWindow { hwnd: hwnd as u64, cmd }) {
        Reply::Handle { hwnd } => hwnd as H,
        _ => bad_window(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn IsChild(parent: H, child: H) -> i32 {
    i32::from(matches!(
        call(Request::IsChild { parent: parent as u64, child: child as u64 }),
        Reply::Bool { value: true }
    ))
}

unsafe fn find(parent: H, after: H, class: *const u16, title: *const u16) -> H {
    // SAFETY: names or atoms, or NULL.
    let class = if class.is_null() { None } else { Some(unsafe { class_ref(class) }) };
    // SAFETY: NULL or a NUL-terminated title.
    let title = if title.is_null() { None } else { Some(unsafe { wide(title, -1) }) };
    match call(Request::FindWindow { class, title, parent: parent as u64, after: after as u64 }) {
        Reply::Handle { hwnd } => hwnd as H,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindWindowW(class: *const u16, title: *const u16) -> H {
    // SAFETY: as documented.
    unsafe { find(0, 0, class, title) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn FindWindowExW(parent: H, after: H, class: *const u16, title: *const u16) -> H {
    // SAFETY: as documented.
    unsafe { find(parent, after, class, title) }
}

unsafe fn put_rect(p: *mut i32, r: Rect) {
    // SAFETY: the caller's RECT.
    unsafe {
        *p = r.left;
        *p.add(1) = r.top;
        *p.add(2) = r.right;
        *p.add(3) = r.bottom;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowRect(hwnd: H, rect: *mut i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    // SAFETY: the caller's RECT.
    unsafe { put_rect(rect, i.window) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClientRect(hwnd: H, rect: *mut i32) -> i32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    let r = if i.minimized { Rect::default() } else { Rect::new(0, 0, i.client.width(), i.client.height()) };
    // SAFETY: the caller's RECT.
    unsafe { put_rect(rect, r) };
    1
}

/// The client origin of a window on the screen (the screen itself: 0,0).
fn origin(hwnd: H) -> Option<(i32, i32)> {
    if hwnd == 0 {
        return Some((0, 0));
    }
    info(hwnd).map(|i| (i.client.left, i.client.top))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ClientToScreen(hwnd: H, pt: *mut i32) -> i32 {
    let Some((x, y)) = origin(hwnd) else { return bad_window() };
    // SAFETY: the caller's POINT.
    unsafe {
        *pt += x;
        *pt.add(1) += y;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ScreenToClient(hwnd: H, pt: *mut i32) -> i32 {
    let Some((x, y)) = origin(hwnd) else { return bad_window() };
    // SAFETY: the caller's POINT.
    unsafe {
        *pt -= x;
        *pt.add(1) -= y;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapWindowPoints(from: H, to: H, pts: *mut i32, n: u32) -> i32 {
    let (Some(a), Some(b)) = (origin(from), origin(to)) else { return bad_window() };
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    for i in 0..n as usize {
        // SAFETY: the caller's `n` POINTs.
        unsafe {
            *pts.add(2 * i) += dx;
            *pts.add(2 * i + 1) += dy;
        }
    }
    (dx as u16 as u32 | ((dy as u16 as u32) << 16)) as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowLongPtrW(hwnd: H, index: i32) -> isize {
    match call(Request::GetLong { hwnd: hwnd as u64, index, size: 8 }) {
        Reply::Value { value } => value as isize,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowLongW(hwnd: H, index: i32) -> i32 {
    match call(Request::GetLong { hwnd: hwnd as u64, index, size: 4 }) {
        Reply::Value { value } => value as i32,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

fn set_long(hwnd: H, index: i32, size: u8, value: u64) -> u64 {
    const GWL_STYLE: i32 = -16;
    const GWL_EXSTYLE: i32 = -20;
    if index == GWL_STYLE || index == GWL_EXSTYLE {
        let Some(i) = info(hwnd) else { return bad_window() };
        let old = if index == GWL_STYLE { i.style } else { i.ex_style };
        let mut ss = u::StyleStruct { old, new: value as u32 };
        let h = hwnd as u64;
        u::send(&SYS, h, u::WM_STYLECHANGING, index as i64 as u64, &mut ss as *mut _ as u64);
        let stored = match call(Request::SetStyle { hwnd: i.hwnd, which: index, value: ss.new }) {
            Reply::Value { value } => value as u32,
            _ => ss.new,
        };
        let mut done = u::StyleStruct { old, new: stored };
        u::send(&SYS, h, u::WM_STYLECHANGED, index as i64 as u64, &mut done as *mut _ as u64);
        return u64::from(old);
    }
    match call(Request::SetLong { hwnd: hwnd as u64, index, size, value }) {
        Reply::Value { value } => value,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowLongPtrW(hwnd: H, index: i32, value: isize) -> isize {
    set_long(hwnd, index, 8, value as u64) as isize
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowLongW(hwnd: H, index: i32, value: i32) -> i32 {
    set_long(hwnd, index, 4, value as u32 as u64) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowTextW(hwnd: H, buf: *mut u16, max: i32) -> i32 {
    if info(hwnd).is_none() {
        if max > 0 && !buf.is_null() {
            // SAFETY: the caller's buffer.
            unsafe { *buf = 0 };
        }
        return bad_window();
    }
    if max <= 0 || buf.is_null() {
        return 0;
    }
    u::send(&SYS, hwnd as u64, u::WM_GETTEXT, max as u64, buf as u64) as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTextLengthW(hwnd: H) -> i32 {
    if info(hwnd).is_none() {
        return bad_window();
    }
    u::send(&SYS, hwnd as u64, u::WM_GETTEXTLENGTH, 0, 0) as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowTextW(hwnd: H, text: *const u16) -> i32 {
    // Observed: a made-up handle returns 1 without an error.
    if info(hwnd).is_some() {
        u::send(&SYS, hwnd as u64, u::WM_SETTEXT, 0, text as u64);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn DefWindowProcW(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    u::def_window_proc(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CallWindowProcW(proc_: H, hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    SYS.call_proc(proc_ as u64, hwnd as u32, msg, w as u64, l as u64) as isize
}

// ----- messages

#[unsafe(no_mangle)]
pub extern "system" fn SendMessageW(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    u::send(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize
}

#[unsafe(no_mangle)]
pub extern "system" fn SendNotifyMessageW(hwnd: H, msg: u32, w: usize, l: isize) -> i32 {
    match info(hwnd) {
        Some(i) if i.tid == SYS.tid() => {
            u::send(&SYS, hwnd as u64, msg, w as u64, l as u64);
            1
        }
        Some(_) => PostMessageW(hwnd, msg, w, l),
        None => bad_window(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn PostMessageW(hwnd: H, msg: u32, w: usize, l: isize) -> i32 {
    match call(Request::Post { hwnd: hwnd as u64, msg, w: w as u64, l: l as u64 }) {
        Reply::Ok {} => 1,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn PostThreadMessageW(tid: u32, msg: u32, w: usize, l: isize) -> i32 {
    match call(Request::PostThread { tid, msg, w: w as u64, l: l as u64 }) {
        Reply::Ok {} => 1,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn PostQuitMessage(code: i32) {
    call(Request::PostQuit { code: code as u32 });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMessageW(msg: *mut CMsg, hwnd: H, min: u32, max: u32) -> i32 {
    let mut m = CMsg::default();
    let r = u::get_message(&SYS, &mut m, hwnd as u64, min, max);
    if r != -1 {
        // SAFETY: the caller's MSG.
        unsafe { *msg = m };
    }
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn PeekMessageW(msg: *mut CMsg, hwnd: H, min: u32, max: u32, remove: u32) -> i32 {
    let mut m = CMsg::default();
    let r = u::peek_message(&SYS, &mut m, hwnd as u64, min, max, remove);
    if r {
        // SAFETY: the caller's MSG.
        unsafe { *msg = m };
    }
    i32::from(r)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DispatchMessageW(msg: *const CMsg) -> isize {
    // SAFETY: the caller's MSG.
    u::dispatch_message(&SYS, unsafe { &*msg }) as isize
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TranslateMessage(msg: *const CMsg) -> i32 {
    // SAFETY: the caller's MSG.
    i32::from(u::translate_message(&SYS, unsafe { &*msg }))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetQueueStatus(flags: u32) -> u32 {
    match call(Request::QueueStatus { flags, clear: true }) {
        Reply::Bits { now, changed, .. } => (now << 16) | changed,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetInputState() -> i32 {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn WaitMessage() -> i32 {
    u::msg_wait(&SYS, &[], false, INFINITE, 0x04FF, 0);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn InSendMessage() -> i32 {
    i32::from(matches!(call(Request::InSend {}), Reply::Bool { value: true }))
}

#[unsafe(no_mangle)]
pub extern "system" fn ReplyMessage(result: isize) -> i32 {
    i32::from(matches!(call(Request::Reply { result: result as u64 }), Reply::Bool { value: true }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MsgWaitForMultipleObjectsEx(
    n: u32,
    handles: *const H,
    ms: u32,
    mask: u32,
    flags: u32,
) -> u32 {
    let hs: Vec<u64> = (0..n as usize)
        .map(|i| {
            // SAFETY: the caller's `n` handles.
            unsafe { *handles.add(i) as u64 }
        })
        .collect();
    const MWMO_WAITALL: u32 = 1;
    u::msg_wait(&SYS, &hs, flags & MWMO_WAITALL != 0, ms, mask, flags)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MsgWaitForMultipleObjects(
    n: u32,
    handles: *const H,
    all: i32,
    ms: u32,
    mask: u32,
) -> u32 {
    // SAFETY: as above.
    unsafe { MsgWaitForMultipleObjectsEx(n, handles, ms, mask, u32::from(all != 0)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetTimer(hwnd: H, id: usize, elapse: u32, timer_proc: H) -> usize {
    match call(Request::SetTimer { hwnd: hwnd as u64, id: id as u64, elapse, timer_proc: timer_proc as u64 }) {
        Reply::Value { value } => value as usize,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn KillTimer(hwnd: H, id: usize) -> i32 {
    match call(Request::KillTimer { hwnd: hwnd as u64, id: id as u64 }) {
        Reply::Bool { value: true } => 1,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => {
            set_last_error(0);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyW(code: u32, kind: u32) -> u32 {
    const SCANS: &[(u32, u32)] = &[
        (0x1B, 1),
        (0x31, 2),
        (0x32, 3),
        (0x33, 4),
        (0x34, 5),
        (0x35, 6),
        (0x36, 7),
        (0x37, 8),
        (0x38, 9),
        (0x39, 10),
        (0x30, 11),
        (0x08, 14),
        (0x09, 15),
        (0x51, 16),
        (0x57, 17),
        (0x45, 18),
        (0x52, 19),
        (0x54, 20),
        (0x59, 21),
        (0x55, 22),
        (0x49, 23),
        (0x4F, 24),
        (0x50, 25),
        (0x0D, 28),
        (0x11, 29),
        (0x41, 30),
        (0x53, 31),
        (0x44, 32),
        (0x46, 33),
        (0x47, 34),
        (0x48, 35),
        (0x4A, 36),
        (0x4B, 37),
        (0x4C, 38),
        (0x10, 42),
        (0x5A, 44),
        (0x58, 45),
        (0x43, 46),
        (0x56, 47),
        (0x42, 48),
        (0x4E, 49),
        (0x4D, 50),
        (0x12, 56),
        (0x20, 57),
        (0x70, 59),
        (0x71, 60),
        (0x72, 61),
        (0x73, 62),
        (0x74, 63),
        (0x75, 64),
        (0x76, 65),
        (0x77, 66),
        (0x78, 67),
        (0x79, 68),
        (0x26, 72),
        (0x25, 75),
        (0x27, 77),
        (0x28, 80),
    ];
    match kind {
        0 => SCANS.iter().find(|s| s.0 == code).map_or(0, |s| s.1),
        1 | 3 => SCANS.iter().find(|s| s.1 == code).map_or(0, |s| s.0),
        2 => {
            u::key_char(code).map_or(0, |c| if (0x61..=0x7A).contains(&c) { u32::from(c) - 0x20 } else { u32::from(c) })
        }
        _ => 0,
    }
}

// ----- metrics

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetrics(index: i32) -> i32 {
    metrics::system_metric(index, u::screen(&SYS))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustWindowRectEx(rect: *mut i32, style: u32, menu: i32, ex: u32) -> i32 {
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    unsafe {
        let r = Rect::new(*rect, *rect.add(1), *rect.add(2), *rect.add(3));
        put_rect(rect, metrics::adjust_window_rect(r, style, menu != 0, ex));
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustWindowRect(rect: *mut i32, style: u32, menu: i32) -> i32 {
    // SAFETY: as documented.
    unsafe { AdjustWindowRectEx(rect, style, menu, 0) }
}

/// `GetSysColor`: Windows 10's default colours.
#[unsafe(no_mangle)]
pub extern "system" fn GetSysColor(index: i32) -> u32 {
    match index {
        0 => 0x00C8_C8C8,    // COLOR_SCROLLBAR
        1 => 0,              // COLOR_BACKGROUND
        2 => 0x00D1_B499,    // COLOR_ACTIVECAPTION
        3 => 0x00DB_CDBF,    // COLOR_INACTIVECAPTION
        4 => 0x00F0_F0F0,    // COLOR_MENU
        5 => 0x00FF_FFFF,    // COLOR_WINDOW
        6 => 0x0064_6464,    // COLOR_WINDOWFRAME
        7 | 8 | 9 | 18 => 0, // menu, window, caption, button text
        13 => 0x00D7_7800,   // COLOR_HIGHLIGHT
        14 => 0x00FF_FFFF,   // COLOR_HIGHLIGHTTEXT
        15 => 0x00F0_F0F0,   // COLOR_BTNFACE
        16 => 0x00A0_A0A0,   // COLOR_BTNSHADOW
        17 => 0x006D_6D6D,   // COLOR_GRAYTEXT
        20 => 0x00FF_FFFF,   // COLOR_BTNHIGHLIGHT
        _ => 0x00F0_F0F0,
    }
}

// ----- painting with GDI

#[unsafe(no_mangle)]
pub extern "system" fn GetDC(hwnd: H) -> H {
    const CS_OWNDC: u32 = 0x20;
    const CS_CLASSDC: u32 = 0x40;
    if hwnd == 0 {
        // SAFETY: our gdi32.
        return unsafe { PolyGdiCreateDisplayDC(0, 0) };
    }
    let Some(i) = info(hwnd) else { return bad_window() };
    let (persistent, class) = if i.class_style & CS_OWNDC != 0 {
        (i.own_dc, false)
    } else if i.class_style & CS_CLASSDC != 0 {
        (i.class_dc, true)
    } else {
        // SAFETY: our gdi32.
        return unsafe { PolyGdiCreateDisplayDC(hwnd, 0) };
    };
    if persistent != 0 {
        return persistent as H;
    }
    // SAFETY: our gdi32.
    let dc = unsafe { PolyGdiCreateDisplayDC(hwnd, 1) };
    call(Request::SetDc { hwnd: i.hwnd, class, dc: dc as u64 });
    dc
}

#[unsafe(no_mangle)]
pub extern "system" fn ReleaseDC(_hwnd: H, hdc: H) -> i32 {
    // SAFETY: our gdi32 checks the handle.
    unsafe { PolyGdiReleaseDisplayDC(hdc) }
}

fn fill(hdc: H, rect: &[i32; 4], brush: H) -> i32 {
    // A system colour index + 1 as the brush.
    let (b, color) = if (1..=31).contains(&brush) { (0, GetSysColor(brush as i32 - 1)) } else { (brush, 0) };
    // SAFETY: our gdi32 checks the DC.
    unsafe { PolyGdiFillRect(hdc, rect.as_ptr(), b, color) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn FillRect(hdc: H, rect: *const i32, brush: H) -> i32 {
    if rect.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { [*rect, *rect.add(1), *rect.add(2), *rect.add(3)] };
    let ok = fill(hdc, &r, brush);
    if ok == 0 {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    ok
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DrawTextW(hdc: H, text: *const u16, count: i32, rect: *mut i32, format: u32) -> i32 {
    // SAFETY: our gdi32 reads the string and RECT as documented.
    unsafe { PolyGdiDrawText(hdc, text, count, rect, format) }
}

// Painting through update regions (WM_PAINT) is not part of the first
// slice (the oracle's desktop is not visible): stubs until its spec sheets.
poly_win32_gui::not_implemented_stubs! {
    /// `UpdateWindow(hWnd)`: zero on failure.
    UpdateWindow(hwnd) => 0;
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
