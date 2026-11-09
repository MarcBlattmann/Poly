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
use poly_win32_gui::{SpinLock, set_last_error, wide};
use poly_win32_rt as _;
use poly_win32k::frame::Rect;
use poly_win32k::kbd;
use poly_win32k::metrics;
use poly_win32k::proto::{ClassRef, ClassSpec, CreateSpec, Reply, Request, WinInfo};
use poly_win32k::user::{self as u, CMsg, CRect, Env};
use poly_win32k::wire;

mod headless;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlUpcaseUnicodeChar(c: u16) -> u16;
}

#[link(name = "win32u", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: u64, out: *mut u8, cap: u64, out_len: *mut u64) -> u32;
    fn PolyUserCommitSurface(hwnd: u64, pixels: u64, width: u64, height: u64, stride: u64) -> u32;
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
    fn PolyGdiAttachSurface(hdc: usize, width: u32, height: u32) -> i32;
    fn PolyGdiSurface(hwnd: usize, out: *mut u64) -> i32;
    fn PolyGdiSetDisplayGeometry(
        hdc: usize,
        x: i32,
        y: i32,
        include: *const i32,
        n_include: u32,
        exclude: *const i32,
        n_exclude: u32,
    ) -> i32;
    fn PolyGdiGetRegionRects(rgn: usize, out: *mut i32, cap: u32) -> i32;
    fn PolyGdiSetRegionRects(rgn: usize, rects: *const i32, n: u32) -> i32;
    fn PolyGdiSetSystemClip(hdc: usize, rects: *const i32, n: u32) -> i32;
    fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> usize;
    fn DeleteObject(h: usize) -> i32;
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

    fn paint_dc(&self, hwnd: u32, clip: Option<&[Rect]>) -> u64 {
        // A window DC with its visible region (GetDC), clipped to the update region.
        let dc = GetDC(hwnd as H);
        if let Some(rects) = clip {
            let flat: Vec<i32> = rects.iter().flat_map(|r| [r.left, r.top, r.right, r.bottom]).collect();
            // SAFETY: `rects.len()` quadruples.
            unsafe { PolyGdiSetSystemClip(dc, flat.as_ptr(), rects.len() as u32) };
        }
        dc as u64
    }

    fn release_dc(&self, hwnd: u32, hdc: u64) {
        commit_surface(hwnd as H);
        // SAFETY: our gdi32 checks the handle.
        unsafe { PolyGdiReleaseDisplayDC(hdc as H) };
    }

    fn make_region(&self, rects: &[Rect]) -> u64 {
        region_of(rects) as u64
    }

    fn delete_object(&self, h: u64) {
        // SAFETY: our gdi32 checks the handle.
        unsafe { DeleteObject(h as H) };
    }
}

/// A new GDI region holding `rects`.
fn region_of(rects: &[Rect]) -> H {
    // SAFETY: our gdi32.
    let rgn = unsafe { CreateRectRgn(0, 0, 0, 0) };
    let flat: Vec<i32> = rects.iter().flat_map(|r| [r.left, r.top, r.right, r.bottom]).collect();
    // SAFETY: `rects.len()` quadruples.
    unsafe { PolyGdiSetRegionRects(rgn, flat.as_ptr(), rects.len() as u32) };
    rgn
}

/// The rectangles of GDI region `rgn` (`None`: not a region).
fn rects_of(rgn: H) -> Option<Vec<Rect>> {
    // SAFETY: a size query.
    let n = unsafe { PolyGdiGetRegionRects(rgn, core::ptr::null_mut(), 0) };
    if n < 0 {
        return None;
    }
    let mut flat = vec![0i32; n as usize * 4];
    // SAFETY: room for `n` quadruples.
    unsafe { PolyGdiGetRegionRects(rgn, flat.as_mut_ptr(), n as u32) };
    Some(flat.chunks(4).map(|c| Rect::new(c[0], c[1], c[2], c[3])).collect())
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
    let top_visible =
        info(hwnd).is_some_and(|i| i.style & metrics::WS_CHILD == 0 && i.style & metrics::WS_VISIBLE != 0);
    let was = u::show_window(&SYS, hwnd as u64, cmd);
    // Observed on the interactive desktop: 24 for a top-level window that
    // was visible (GetForegroundWindow.yaml `show_visible`); 1 elsewhere.
    if was && top_visible && u::screen(&SYS).interactive { 24 } else { i32::from(was) }
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
    // NULL is not a window here (ClientToScreen.yaml).
    let Some((x, y)) = (hwnd != 0).then(|| origin(hwnd)).flatten() else { return bad_window() };
    // SAFETY: the caller's POINT.
    unsafe {
        *pt += x;
        *pt.add(1) += y;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ScreenToClient(hwnd: H, pt: *mut i32) -> i32 {
    // NULL is not a window here (ScreenToClient.yaml).
    let Some((x, y)) = (hwnd != 0).then(|| origin(hwnd)).flatten() else { return bad_window() };
    // SAFETY: the caller's POINT.
    unsafe {
        *pt -= x;
        *pt.add(1) -= y;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MapWindowPoints(from: H, to: H, pts: *mut i32, n: u32) -> i32 {
    let map_origin = |hwnd: H| -> Option<u::MapOrigin> {
        if hwnd == 0 {
            return Some(u::MapOrigin::default());
        }
        info(hwnd).map(|i| u::MapOrigin {
            x: if i.ex_style & metrics::WS_EX_LAYOUTRTL != 0 { i.client.right } else { i.client.left },
            y: i.client.top,
            rtl: i.ex_style & metrics::WS_EX_LAYOUTRTL != 0,
        })
    };
    let (Some(a), Some(b)) = (map_origin(from), map_origin(to)) else { return bad_window() };
    let mut points = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        // SAFETY: the caller supplies `n` readable POINTs.
        let point = unsafe { (*pts.add(2 * i), *pts.add(2 * i + 1)) };
        points.push(point);
    }
    let result = u::map_points(a, b, &mut points);
    for (i, point) in points.iter().enumerate() {
        // SAFETY: the caller supplies `n` writable POINTs, matching the slice above.
        unsafe {
            *pts.add(2 * i) = point.0;
            *pts.add(2 * i + 1) = point.1;
        }
    }
    result
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
    if msg.is_null() {
        return 0;
    }
    // SAFETY: the caller's MSG.
    let m = unsafe { &*msg };
    // The dead-key state is the thread's, shared with ToUnicode.
    i32::from(with_dead_state(|d| u::input::translate(&SYS, m, d)))
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

// ----- keyboard layout (`poly_win32k::kbd`: the oracle's German layout)

/// The pending dead key of each thread that has one (Windows keeps it per
/// thread, in the thread's keyboard state, `ToUnicode.yaml`): `ToUnicode`
/// and `ToUnicodeEx` read and update it, and `TranslateMessage` should use
/// the same through [`with_dead_state`]. Threads without a pending dead key
/// have no entry.
static DEAD_KEYS: SpinLock<Vec<(u32, kbd::DeadState)>> = SpinLock::new(Vec::new());

/// Run `f` with the calling thread's dead-key state (for `ToUnicode` and
/// `TranslateMessage`: `poly_win32k::kbd::translate_key`).
fn with_dead_state<R>(f: impl FnOnce(&mut kbd::DeadState) -> R) -> R {
    // SAFETY: no preconditions.
    let tid = unsafe { GetCurrentThreadId() };
    DEAD_KEYS.with(|all| {
        let i = all.iter().position(|e| e.0 == tid);
        let mut state = i.map_or_else(kbd::DeadState::default, |i| all[i].1);
        let r = f(&mut state);
        match (i, state.pending().is_some()) {
            (Some(i), true) => all[i].1 = state,
            (Some(i), false) => {
                all.swap_remove(i);
            }
            (None, true) => all.push((tid, state)),
            (None, false) => {}
        }
        r
    })
}

/// `MapVirtualKeyW` (`MapVirtualKeyW.yaml`).
#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyW(code: u32, map_type: u32) -> u32 {
    kbd::try_map_virtual_key(code, map_type).unwrap_or_else(|e| {
        set_last_error(e);
        0
    })
}

/// `MapVirtualKeyExW`: there is one layout, so the same as
/// `MapVirtualKeyW` for any `layout` (`MapVirtualKeyW.yaml` `same_as_ex`).
#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyExW(code: u32, map_type: u32, _layout: usize) -> u32 {
    MapVirtualKeyW(code, map_type)
}

/// `ToUnicode` (`ToUnicode.yaml`).
///
/// # Safety
/// `key_state` is NULL or 256 readable bytes; `buffer` is NULL or
/// `buffer_len` writable units (at least one when `buffer_len` is 0).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ToUnicode(
    vk: u32,
    scan: u32,
    key_state: *const u8,
    buffer: *mut u16,
    buffer_len: i32,
    flags: u32,
) -> i32 {
    if key_state.is_null() || buffer.is_null() {
        return 0;
    }
    if buffer_len <= 0 {
        if buffer_len == 0 {
            // SAFETY: Windows writes the NUL at unit 0 even with no room
            // (`ToUnicode.yaml` `buffer_fill`); the caller's buffer.
            unsafe { *buffer = 0 };
        }
        return 0;
    }
    let mut ks = [0u8; 256];
    // SAFETY: the caller's 256-byte key-state array.
    unsafe { core::ptr::copy_nonoverlapping(key_state, ks.as_mut_ptr(), 256) };
    // SAFETY: the caller's buffer of `buffer_len` units.
    let buf = unsafe { core::slice::from_raw_parts_mut(buffer, buffer_len as usize) };
    with_dead_state(|dead| kbd::to_unicode(vk, scan, &ks, buf, flags, dead))
}

/// `ToUnicodeEx`: there is one layout, so `ToUnicode` for any `layout`.
///
/// # Safety
/// As [`ToUnicode`].
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ToUnicodeEx(
    vk: u32,
    scan: u32,
    key_state: *const u8,
    buffer: *mut u16,
    buffer_len: i32,
    flags: u32,
    _layout: usize,
) -> i32 {
    // SAFETY: the caller's promises, passed on.
    unsafe { ToUnicode(vk, scan, key_state, buffer, buffer_len, flags) }
}

/// `GetKeyboardLayout`: every thread has the oracle's layout
/// (`MapVirtualKeyW.yaml`: HKL `0x04072000`).
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyboardLayout(_thread: u32) -> usize {
    kbd::HKL as usize
}

// ----- metrics

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetrics(index: i32) -> i32 {
    match metrics::system_metric(index, &u::screen(&SYS)) {
        Some(v) => v,
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

/// GetSystemMetrics.yaml `for_dpi`; DPI 0 is ERROR_INVALID_PARAMETER, an
/// undefined index 0 with the error unchanged.
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32 {
    if dpi == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    metrics::system_metric_for_dpi(index, dpi, &u::screen(&SYS)).unwrap_or(0)
}

// ----- DPI and monitors (one monitor, DPI-unaware processes: 96)

/// The system DPI (GetDpiForSystem.yaml).
const SYSTEM_DPI: u32 = 96;
/// The one monitor's handle: the same value for every point and thread.
const PRIMARY_MONITOR: usize = 0x0001_0001;
/// `MONITOR_DEFAULTTONULL` / `TOPRIMARY` / `TONEAREST` (winuser.h).
const MONITOR_DEFAULTTONULL: u32 = 0;
/// `MONITORINFOF_PRIMARY` (winuser.h).
const MONITORINFOF_PRIMARY: u32 = 1;
/// `ERROR_INVALID_FLAGS`, `ERROR_INVALID_MONITOR_HANDLE` (winerror.h).
const ERROR_INVALID_FLAGS: u32 = 1004;
const ERROR_INVALID_MONITOR_HANDLE: u32 = 1461;

#[unsafe(no_mangle)]
pub extern "system" fn GetDpiForSystem() -> u32 {
    SYSTEM_DPI
}

/// 96 for any window of a DPI-unaware process, the desktop and message
/// windows included; 0 with ERROR_INVALID_PARAMETER otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn GetDpiForWindow(hwnd: H) -> u32 {
    if hwnd != 0 && info(hwnd).is_some() {
        SYSTEM_DPI
    } else {
        set_last_error(ERROR_INVALID_PARAMETER);
        0
    }
}

/// The monitor for a rectangle on the screen or not (`on`), as `flags`
/// asks; bad flags are ERROR_INVALID_FLAGS.
fn monitor_for(on: bool, flags: u32) -> H {
    if flags > 2 {
        set_last_error(ERROR_INVALID_FLAGS);
        return 0;
    }
    if on || flags != MONITOR_DEFAULTTONULL { PRIMARY_MONITOR } else { 0 }
}

/// Whether rectangle `r` (screen coordinates) is on the screen: it overlaps
/// it, or is empty or inverted with its corner inside (MonitorFromPoint.yaml).
fn rect_on_screen(r: [i32; 4]) -> bool {
    let d = u::screen(&SYS);
    let [l, t, rr, b] = r;
    if rr <= l || b <= t {
        return (0..d.width).contains(&l) && (0..d.height).contains(&t);
    }
    l < d.width && rr > 0 && t < d.height && b > 0
}

#[unsafe(no_mangle)]
pub extern "system" fn MonitorFromPoint(pt: u64, flags: u32) -> H {
    let (x, y) = (pt as u32 as i32, (pt >> 32) as u32 as i32);
    let d = u::screen(&SYS);
    monitor_for((0..d.width).contains(&x) && (0..d.height).contains(&y), flags)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MonitorFromRect(rect: *const i32, flags: u32) -> H {
    if rect.is_null() {
        return monitor_for(false, flags);
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { [*rect, *rect.add(1), *rect.add(2), *rect.add(3)] };
    monitor_for(rect_on_screen(r), flags)
}

/// By the window rectangle; the desktop, message-only and never-shown
/// windows count as on the screen (MonitorFromWindow.yaml).
#[unsafe(no_mangle)]
pub extern "system" fn MonitorFromWindow(hwnd: H, flags: u32) -> H {
    if flags > 2 {
        set_last_error(ERROR_INVALID_FLAGS);
        return 0;
    }
    if hwnd == 0 {
        return monitor_for(false, flags);
    }
    let Some(i) = info(hwnd) else { return bad_window() };
    if i.desktop || i.message_only {
        return PRIMARY_MONITOR;
    }
    let w = i.window;
    if w.right <= w.left || w.bottom <= w.top {
        // A window of no size (never shown): on the screen when its corner is.
        return monitor_for(rect_on_screen([w.left, w.top, w.right, w.bottom]), flags);
    }
    monitor_for(rect_on_screen([w.left, w.top, w.right, w.bottom]), flags)
}

/// MONITORINFO (40 bytes) or MONITORINFOEXW (104 bytes, winuser.h).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMonitorInfoW(monitor: H, mi: *mut u8) -> i32 {
    if monitor != PRIMARY_MONITOR {
        set_last_error(ERROR_INVALID_MONITOR_HANDLE);
        return 0;
    }
    if mi.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's structure starts with cbSize.
    let size = unsafe { core::ptr::read_unaligned(mi as *const u32) };
    if size != 40 && size != 104 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let d = u::screen(&SYS);
    let mut out = Vec::with_capacity(36);
    for v in [0, 0, d.width, d.height, d.work.left, d.work.top, d.work.right, d.work.bottom] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&MONITORINFOF_PRIMARY.to_le_bytes());
    if size == 104 {
        // szDevice: "\\.\DISPLAY1" and its NUL; the rest is not written.
        for c in "\\\\.\\DISPLAY1\0".encode_utf16() {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    // SAFETY: `size` bytes of the caller's structure, after cbSize.
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), mi.add(4), out.len()) };
    1
}

/// The `SPI_GET*` queries (`headless::system_parameters_info`, ADR 0155
/// batch 4); the `SPI_SET*` actions fail with `ERROR_INVALID_PARAMETER`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemParametersInfoW(action: u32, param: u32, pv: *mut u8, _ini: u32) -> i32 {
    // SAFETY: the caller's promises for this action, passed on.
    unsafe { headless::system_parameters_info(action, param, pv) }
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
        0 => 0x00C8_C8C8,                   // COLOR_SCROLLBAR
        1 => 0,                             // COLOR_BACKGROUND
        2 => 0x00D1_B499,                   // COLOR_ACTIVECAPTION
        3 => 0x00DB_CDBF,                   // COLOR_INACTIVECAPTION
        4 => 0x00F0_F0F0,                   // COLOR_MENU
        5 => 0x00FF_FFFF,                   // COLOR_WINDOW
        6 => 0x0064_6464,                   // COLOR_WINDOWFRAME
        7 | 8 | 9 | 18 => 0,                // menu, window, caption, button text
        10 => 0x00B4_B4B4,                  // COLOR_ACTIVEBORDER
        11 => 0x00FC_F7F4,                  // COLOR_INACTIVEBORDER
        12 => 0x00AB_ABAB,                  // COLOR_APPWORKSPACE
        19 => 0x0054_4E43,                  // COLOR_INACTIVECAPTIONTEXT
        21 => 0x0069_6969,                  // COLOR_3DDKSHADOW
        22 => 0x00E3_E3E3,                  // COLOR_3DLIGHT
        23 => 0,                            // COLOR_INFOTEXT
        24 => 0x00E1_FFFF,                  // COLOR_INFOBK
        26 => 0x00CC_6600,                  // COLOR_HOTLIGHT
        27 => 0x00EA_D1B9,                  // COLOR_GRADIENTACTIVECAPTION
        28 => 0x00F2_E4D7,                  // COLOR_GRADIENTINACTIVECAPTION
        29 => 0x00D7_7800,                  // COLOR_MENUHILIGHT
        30 => 0x00F0_F0F0,                  // COLOR_MENUBAR
        13 => 0x00D7_7800,                  // COLOR_HIGHLIGHT
        14 => 0x00FF_FFFF,                  // COLOR_HIGHLIGHTTEXT
        15 => 0x00F0_F0F0,                  // COLOR_BTNFACE
        16 => 0x00A0_A0A0,                  // COLOR_BTNSHADOW
        17 => 0x006D_6D6D,                  // COLOR_GRAYTEXT
        20 => 0x00FF_FFFF,                  // COLOR_BTNHIGHLIGHT
        i32::MIN..=-1 | 31..=i32::MAX => 0, // not a colour index
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
        return dc_geometry(&i, attach_surface(i.hwnd, unsafe { PolyGdiCreateDisplayDC(hwnd, 0) }), false);
    };
    if persistent != 0 {
        return dc_geometry(&i, attach_surface(i.hwnd, persistent as H), false);
    }
    // SAFETY: our gdi32.
    let dc = unsafe { PolyGdiCreateDisplayDC(hwnd, 1) };
    call(Request::SetDc { hwnd: i.hwnd, class, dc: dc as u64 });
    dc_geometry(&i, attach_surface(i.hwnd, dc), false)
}

/// Tell our gdi32 what a DC of window `i` can draw on (`GetClipBox.yaml`):
/// its client area (`whole`: the whole window) where it is on the screen,
/// within its parents' client areas, less its children with
/// WS_CLIPCHILDREN and, for a child with WS_CLIPSIBLINGS, less the
/// siblings above it; nothing when it is hidden. Top-level windows do not
/// clip each other (the desktop is composed).
fn dc_geometry(i: &WinInfo, dc: H, whole: bool) -> H {
    let area = if whole { i.window } else { i.client };
    let screen = Rect::new(0, 0, GetSystemMetrics(0), GetSystemMetrics(1));
    let visible_window = |h: u32| info(h as H).filter(|c| c.visible).map(|c| c.window);
    let children = |h: u32| match call(Request::Relatives { hwnd: h }) {
        Reply::Relatives { children, .. } => children,
        _ => Vec::new(),
    };
    let meet = |a: Rect, b: Rect| {
        let r = Rect::new(a.left.max(b.left), a.top.max(b.top), a.right.min(b.right), a.bottom.min(b.bottom));
        if r.is_empty() { Rect::default() } else { r }
    };
    let mut include = if i.visible { meet(area, screen) } else { Rect::default() };
    let mut exclude: Vec<Rect> = Vec::new();
    if i.style & metrics::WS_CLIPCHILDREN != 0 {
        exclude.extend(children(i.hwnd).into_iter().filter_map(visible_window));
    }
    // Up the parents: each limits to its client area; siblings above a
    // WS_CLIPSIBLINGS child (children come topmost first) cut it.
    let (mut hwnd, mut style, mut parent) = (i.hwnd, i.style, i.parent);
    while style & metrics::WS_CHILD != 0 {
        let Some(p) = info(parent as H) else { break };
        include = meet(include, p.client);
        if style & metrics::WS_CLIPSIBLINGS != 0 {
            exclude.extend(children(p.hwnd).into_iter().take_while(|&s| s != hwnd).filter_map(visible_window));
        }
        (hwnd, style, parent) = (p.hwnd, p.style, p.parent);
    }
    let flat = |rs: &[Rect]| -> Vec<i32> { rs.iter().flat_map(|r| [r.left, r.top, r.right, r.bottom]).collect() };
    let (inc, exc) = (flat(&[include]), flat(&exclude));
    if dc != 0 {
        // SAFETY: our gdi32 reads 1 and `exclude.len()` RECTs.
        unsafe {
            PolyGdiSetDisplayGeometry(dc, area.left, area.top, inc.as_ptr(), 1, exc.as_ptr(), exclude.len() as u32)
        };
    }
    dc
}

/// The desktop link (ADR 0131): a window DC draws into the window's
/// surface DIB when win32k gave the window a surface (a compositor is
/// connected and the window is a shown top-level one), else nowhere.
fn attach_surface(hwnd: u32, dc: H) -> H {
    let size = match call(Request::Surface { hwnd }) {
        Reply::Value { value } => value,
        _ => 0,
    };
    if dc != 0 {
        // SAFETY: our gdi32 checks the DC.
        unsafe { PolyGdiAttachSurface(dc, size as u32, (size >> 32) as u32) };
    }
    dc
}

/// What the window's DCs drew goes to its surface (`ReleaseDC`).
fn commit_surface(hwnd: H) {
    let mut s = [0u64; 3];
    // SAFETY: our gdi32 writes three u64s.
    if hwnd == 0 || unsafe { PolyGdiSurface(hwnd, s.as_mut_ptr()) } == 0 {
        return;
    }
    // SAFETY: win32u's stub; the kernel reads `height` rows of the DIB.
    unsafe { PolyUserCommitSurface(hwnd as u64, s[0], s[1], s[2], s[1] * 4) };
}

#[unsafe(no_mangle)]
pub extern "system" fn ReleaseDC(hwnd: H, hdc: H) -> i32 {
    commit_surface(hwnd);
    // SAFETY: our gdi32 checks the handle.
    unsafe { PolyGdiReleaseDisplayDC(hdc) }
}

fn fill(hdc: H, rect: &[i32; 4], brush: H) -> i32 {
    // A system colour index + 1 as the brush.
    let (b, color) = if (1..=31).contains(&brush) {
        (0, GetSysColor(brush as i32 - 1))
    } else if brush == 0 {
        // NULL: the DC's brush (observed: white on a fresh DC).
        (0, 0xFFFF_FFFF)
    } else {
        (brush, 0)
    };
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

// ----- more of user32: keyboard state, ANSI text, rectangles, enumeration

/// `ERROR_NOACCESS` (winerror.h).
const ERROR_NOACCESS: u32 = 998;

/// `GetKeyState` (GetKeyState.yaml): the thread's state as of the input
/// messages it retrieved, sign-extended (0xFF80 down).
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyState(vk: i32) -> i16 {
    if !(0..=0xFF).contains(&vk) {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if vk == 0 || vk == 0xFF {
        return 0;
    }
    let b = u::input::key_state(&SYS)[vk as usize];
    let down: u16 = if b & 0x80 != 0 { 0xFF80 } else { 0 };
    (down | u16::from(b & 0x01)) as i16
}

/// `GetAsyncKeyState` (GetAsyncKeyState.yaml): the physical state.
#[unsafe(no_mangle)]
pub extern "system" fn GetAsyncKeyState(vk: i32) -> i16 {
    if !(1..0xFF).contains(&vk) {
        return 0;
    }
    match call(Request::AsyncKeyState { vk: vk as u32 }) {
        Reply::Value { value } => value as u16 as i16,
        _ => 0,
    }
}

/// `GetKeyboardState` (GetKeyboardState.yaml).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetKeyboardState(state: *mut u8) -> i32 {
    if state.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    let keys = u::input::key_state(&SYS);
    // SAFETY: the caller's 256-byte buffer.
    unsafe { core::ptr::copy_nonoverlapping(keys.as_ptr(), state, 256) };
    1
}

/// `SetKeyboardState`: the calling thread's key state.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetKeyboardState(state: *const u8) -> i32 {
    if state.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    // SAFETY: the caller's 256-byte buffer.
    let keys = unsafe { core::slice::from_raw_parts(state, 256) }.to_vec();
    call(Request::SetKeyboardState { keys });
    1
}

/// `SendInput` (SendInput.yaml): INPUT is 40 bytes on x64 (winuser.h).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SendInput(count: u32, inputs: *const u8, size: i32) -> u32 {
    if count == 0 || size != 40 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if inputs.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    let mut events = Vec::with_capacity(count as usize);
    for k in 0..count as usize {
        // SAFETY: `count` INPUT structures of 40 bytes.
        let b = unsafe { core::slice::from_raw_parts(inputs.add(k * 40), 40) };
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let kind = u32_at(0);
        let e = if kind == 1 {
            poly_win32k::proto::InputEvent {
                kind,
                a: i32::from(u16_at(8)),
                b: i32::from(u16_at(10)),
                data: 0,
                flags: u32_at(12),
            }
        } else {
            poly_win32k::proto::InputEvent {
                kind,
                a: u32_at(8) as i32,
                b: u32_at(12) as i32,
                data: u32_at(16),
                flags: u32_at(20),
            }
        };
        events.push(e);
    }
    match call(Request::SendInput { events }) {
        Reply::Value { value } => value as u32,
        _ => 0,
    }
}

fn message_info() -> u64 {
    match call(Request::MessageInfo {}) {
        Reply::Value { value } => value,
        _ => 0,
    }
}

/// `GetMessageTime`: the time of the last message retrieved.
#[unsafe(no_mangle)]
pub extern "system" fn GetMessageTime() -> i32 {
    message_info() as u32 as i32
}

/// `GetMessagePos`: its position, `MAKELONG(x, y)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetMessagePos() -> u32 {
    (message_info() >> 32) as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMessageExtraInfo() -> isize {
    0
}

/// `GetWindowTextA`: the text converted to code page 1252 (characters it
/// lacks become `?`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowTextA(hwnd: H, buf: *mut u8, max: i32) -> i32 {
    if max <= 0 || buf.is_null() {
        return 0;
    }
    let mut w = vec![0u16; max as usize];
    // SAFETY: our buffer of `max` characters.
    let n = unsafe { GetWindowTextW(hwnd, w.as_mut_ptr(), max) }.max(0) as usize;
    for (i, &c) in w.iter().take(n).enumerate() {
        // SAFETY: the caller's buffer of `max` bytes.
        unsafe { *buf.add(i) = if c < 0x100 { c as u8 } else { b'?' } };
    }
    // SAFETY: as above; n < max.
    unsafe { *buf.add(n) = 0 };
    n as i32
}

/// `SetWindowTextA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWindowTextA(hwnd: H, text: *const u8) -> i32 {
    if text.is_null() {
        return SetWindowTextW(hwnd, core::ptr::null());
    }
    let mut w: Vec<u16> = Vec::new();
    let mut p = text;
    // SAFETY: a NUL-terminated string.
    unsafe {
        while *p != 0 {
            w.push(u16::from(*p));
            p = p.add(1);
        }
    }
    w.push(0);
    SetWindowTextW(hwnd, w.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTextLengthA(hwnd: H) -> i32 {
    GetWindowTextLengthW(hwnd)
}

#[unsafe(no_mangle)]
pub extern "system" fn SendMessageA(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    SendMessageW(hwnd, msg, w, l)
}

#[unsafe(no_mangle)]
pub extern "system" fn PostMessageA(hwnd: H, msg: u32, w: usize, l: isize) -> i32 {
    PostMessageW(hwnd, msg, w, l)
}

#[unsafe(no_mangle)]
pub extern "system" fn DefWindowProcA(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    DefWindowProcW(hwnd, msg, w, l)
}

/// `SendMessageTimeoutW`: the send, its result through the pointer.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn SendMessageTimeoutW(
    hwnd: H,
    msg: u32,
    w: usize,
    l: isize,
    _flags: u32,
    _timeout: u32,
    result: *mut usize,
) -> isize {
    if info(hwnd).is_none() {
        return bad_window();
    }
    let r = SendMessageW(hwnd, msg, w, l);
    if !result.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { *result = r as usize };
    }
    1
}

/// `InSendMessageEx`: `ISMEX_SEND` (1) inside a message another thread sent.
#[unsafe(no_mangle)]
pub extern "system" fn InSendMessageEx(_reserved: usize) -> u32 {
    InSendMessage() as u32
}

/// `GetWindowDC`: a DC for the whole window.
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowDC(hwnd: H) -> H {
    if hwnd == 0 {
        // SAFETY: our gdi32.
        return unsafe { PolyGdiCreateDisplayDC(0, 0) };
    }
    let Some(i) = info(hwnd) else { return bad_window() };
    // SAFETY: our gdi32.
    dc_geometry(&i, unsafe { PolyGdiCreateDisplayDC(hwnd, 0) }, true)
}

/// `GetWindowThreadProcessId`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowThreadProcessId(hwnd: H, pid: *mut u32) -> u32 {
    let Some(i) = info(hwnd) else { return bad_window() };
    if !pid.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { *pid = i.pid };
    }
    i.tid
}

/// The window's own WS_DISABLED bit (IsWindowEnabled.yaml); 0 with
/// ERROR_INVALID_WINDOW_HANDLE for NULL or a bad handle.
#[unsafe(no_mangle)]
pub extern "system" fn IsWindowEnabled(hwnd: H) -> i32 {
    if hwnd == 0 {
        return bad_window();
    }
    match info(hwnd) {
        Some(i) => i32::from(i.style & metrics::WS_DISABLED == 0),
        None => bad_window(),
    }
}

/// `EnableWindow` (EnableWindow.yaml): whether it was disabled before.
#[unsafe(no_mangle)]
pub extern "system" fn EnableWindow(hwnd: H, enable: i32) -> i32 {
    i32::from(u::focus::enable_window(&SYS, hwnd as u64, enable != 0))
}

// ----- focus, activation, the foreground (slice B; poly_win32k::user::focus)

#[unsafe(no_mangle)]
pub extern "system" fn GetFocus() -> H {
    u::focus::input(&SYS).focus as H
}

#[unsafe(no_mangle)]
pub extern "system" fn GetActiveWindow() -> H {
    u::focus::input(&SYS).active as H
}

#[unsafe(no_mangle)]
pub extern "system" fn GetForegroundWindow() -> H {
    u::focus::input(&SYS).foreground as H
}

#[unsafe(no_mangle)]
pub extern "system" fn GetCapture() -> H {
    u::focus::input(&SYS).capture as H
}

#[unsafe(no_mangle)]
pub extern "system" fn SetFocus(hwnd: H) -> H {
    u::focus::set_focus(&SYS, hwnd as u64) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn SetActiveWindow(hwnd: H) -> H {
    u::focus::set_active_window(&SYS, hwnd as u64) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn SetForegroundWindow(hwnd: H) -> i32 {
    i32::from(u::focus::set_foreground_window(&SYS, hwnd as u64))
}

/// Any process may take the foreground on Poly: 1.
#[unsafe(no_mangle)]
pub extern "system" fn AllowSetForegroundWindow(_pid: u32) -> i32 {
    1
}

/// `AttachThreadInput(idAttach, idAttachTo, fAttach)`: the calling thread
/// (`attach`) shares `to`'s input state.
#[unsafe(no_mangle)]
pub extern "system" fn AttachThreadInput(attach: u32, to: u32, on: i32) -> i32 {
    let me = SYS.tid();
    let other = if attach == me { to } else { attach };
    if on != 0 {
        // Attaching: what was active in our queue loses the activation.
        let st = u::focus::input(&SYS);
        if st.active != 0 && !st.fg_mine {
            u::focus::deactivate_away(&SYS, false);
        }
    }
    match call(Request::AttachInput { tid: other, attach: on != 0 }) {
        Reply::Bool { value } => {
            if value && on != 0 && attach == me {
                u::focus::attached_to_foreground(&SYS, other);
            }
            i32::from(value)
        }
        _ => 0,
    }
}

/// `GUITHREADINFO` (winuser.h): cbSize, flags, hwndActive, hwndFocus,
/// hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret, rcCaret (72 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetGUIThreadInfo(tid: u32, gti: *mut u8) -> i32 {
    if gti.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let s = if tid == 0 || tid != SYS.tid() { u::focus::thread_input(&SYS, tid) } else { u::focus::input(&SYS) };
    // SAFETY: the caller's GUITHREADINFO (cbSize first).
    unsafe {
        core::ptr::write_unaligned(gti.add(4) as *mut u32, 0);
        core::ptr::write_unaligned(gti.add(8) as *mut u64, u64::from(s.active));
        core::ptr::write_unaligned(gti.add(16) as *mut u64, u64::from(s.focus));
        core::ptr::write_unaligned(gti.add(24) as *mut u64, u64::from(s.capture));
        core::ptr::write_unaligned(gti.add(32) as *mut u64, 0);
        core::ptr::write_unaligned(gti.add(40) as *mut u64, 0);
        core::ptr::write_unaligned(gti.add(48) as *mut u64, 0);
        core::ptr::write_bytes(gti.add(56), 0, 16);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn ReleaseCapture() -> i32 {
    let st = u::focus::input(&SYS);
    if st.capture != 0 {
        call(Request::SetInput { active: 0, focus: 0, capture: 0, which: poly_win32k::wm::input::SET_CAPTURE });
        u::send(&SYS, u64::from(st.capture), 0x0215, 0, 0); // WM_CAPTURECHANGED
    }
    1
}

/// `GetCursorPos`: (0, 0) until input exists.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCursorPos(pt: *mut i32) -> i32 {
    if pt.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let (x, y) = cursor(None);
    // SAFETY: the caller's POINT.
    unsafe {
        *pt = x;
        *pt.add(1) = y;
    }
    1
}

/// The cursor position, moved to `to` first.
fn cursor(to: Option<(i32, i32)>) -> (i32, i32) {
    let (x, y) = to.unwrap_or((0, 0));
    match call(Request::Cursor { set: to.is_some(), x, y }) {
        Reply::Value { value } => (value as u32 as i32, (value >> 32) as u32 as i32),
        _ => (0, 0),
    }
}

/// Moves the cursor (clamped to the screen).
#[unsafe(no_mangle)]
pub extern "system" fn SetCursorPos(x: i32, y: i32) -> i32 {
    cursor(Some((x, y)));
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDoubleClickTime() -> u32 {
    500
}

#[unsafe(no_mangle)]
pub extern "system" fn MessageBeep(_kind: u32) -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn GetLastActivePopup(hwnd: H) -> H {
    hwnd
}

/// `GetTopWindow`: the first child (the desktop's for NULL).
#[unsafe(no_mangle)]
pub extern "system" fn GetTopWindow(hwnd: H) -> H {
    let parent = if hwnd == 0 { GetDesktopWindow() } else { hwnd };
    GetWindow(parent, 5)
}

type EnumProc = unsafe extern "system" fn(H, isize) -> i32;

unsafe fn enum_children(parent: H, f: EnumProc, l: isize, deep: bool) -> bool {
    let mut kids = Vec::new();
    let mut h = GetWindow(parent, 5);
    while h != 0 {
        kids.push(h);
        h = GetWindow(h, 2);
    }
    for k in kids {
        // SAFETY: the caller's callback.
        if unsafe { f(k, l) } == 0 {
            return false;
        }
        // SAFETY: as above.
        if deep && !unsafe { enum_children(k, f, l, true) } {
            return false;
        }
    }
    true
}

/// `EnumWindows`: the top-level windows, top first.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumWindows(f: Option<EnumProc>, l: isize) -> i32 {
    let Some(f) = f else { return 0 };
    // SAFETY: the caller's callback.
    i32::from(unsafe { enum_children(GetDesktopWindow(), f, l, false) })
}

/// `EnumChildWindows`: every descendant, depth first.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumChildWindows(parent: H, f: Option<EnumProc>, l: isize) -> i32 {
    let Some(f) = f else { return 0 };
    let parent = if parent == 0 { GetDesktopWindow() } else { parent };
    // SAFETY: the caller's callback.
    i32::from(unsafe { enum_children(parent, f, l, true) })
}

/// `GetClassInfoW`: `GetClassInfoExW` into a `WNDCLASSW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClassInfoW(instance: H, name: *const u16, wc: *mut u8) -> i32 {
    if wc.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut ex = [0u8; 80];
    // SAFETY: our WNDCLASSEXW; the caller's name.
    let r = unsafe { GetClassInfoExW(instance, name, ex.as_mut_ptr()) };
    if r != 0 {
        // WNDCLASSW is WNDCLASSEXW without cbSize and hIconSm.
        // SAFETY: the caller's WNDCLASSW (72 bytes).
        unsafe { core::ptr::copy_nonoverlapping(ex.as_ptr().add(4), wc, 4) };
        // SAFETY: as above.
        unsafe { core::ptr::copy_nonoverlapping(ex.as_ptr().add(8), wc.add(8), 64) };
    }
    r
}

// ----- rectangles (`RECT` helpers)

unsafe fn rd(p: *const i32) -> [i32; 4] {
    // SAFETY: the caller's RECT.
    unsafe { [*p, *p.add(1), *p.add(2), *p.add(3)] }
}

unsafe fn wr(p: *mut i32, r: [i32; 4]) {
    // SAFETY: the caller's RECT.
    unsafe {
        *p = r[0];
        *p.add(1) = r[1];
        *p.add(2) = r[2];
        *p.add(3) = r[3];
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetRect(r: *mut i32, l: i32, t: i32, rr: i32, b: i32) -> i32 {
    if r.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    unsafe { wr(r, [l, t, rr, b]) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetRectEmpty(r: *mut i32) -> i32 {
    // SAFETY: as documented.
    unsafe { SetRect(r, 0, 0, 0, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CopyRect(dst: *mut i32, src: *const i32) -> i32 {
    if dst.is_null() || src.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECTs.
    unsafe { wr(dst, rd(src)) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsRectEmpty(r: *const i32) -> i32 {
    if r.is_null() {
        return 1;
    }
    // SAFETY: the caller's RECT.
    let [l, t, rr, b] = unsafe { rd(r) };
    i32::from(rr <= l || b <= t)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn EqualRect(a: *const i32, b: *const i32) -> i32 {
    if a.is_null() || b.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECTs.
    i32::from(unsafe { rd(a) == rd(b) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn OffsetRect(r: *mut i32, dx: i32, dy: i32) -> i32 {
    if r.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    let [l, t, rr, b] = unsafe { rd(r) };
    // SAFETY: as above.
    unsafe { wr(r, [l + dx, t + dy, rr + dx, b + dy]) };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InflateRect(r: *mut i32, dx: i32, dy: i32) -> i32 {
    if r.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    let [l, t, rr, b] = unsafe { rd(r) };
    // SAFETY: as above.
    unsafe { wr(r, [l - dx, t - dy, rr + dx, b + dy]) };
    1
}

/// `PtInRect` (the point by value: x in the low half, y in the high).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PtInRect(r: *const i32, pt: u64) -> i32 {
    if r.is_null() {
        return 0;
    }
    let (x, y) = (pt as u32 as i32, (pt >> 32) as u32 as i32);
    // SAFETY: the caller's RECT.
    let [l, t, rr, b] = unsafe { rd(r) };
    i32::from(x >= l && x < rr && y >= t && y < b)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn IntersectRect(dst: *mut i32, a: *const i32, b: *const i32) -> i32 {
    if dst.is_null() || a.is_null() || b.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECTs.
    let (x, y) = unsafe { (rd(a), rd(b)) };
    let r = [x[0].max(y[0]), x[1].max(y[1]), x[2].min(y[2]), x[3].min(y[3])];
    let empty = r[2] <= r[0] || r[3] <= r[1];
    // SAFETY: as above.
    unsafe { wr(dst, if empty { [0; 4] } else { r }) };
    i32::from(!empty)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnionRect(dst: *mut i32, a: *const i32, b: *const i32) -> i32 {
    if dst.is_null() || a.is_null() || b.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECTs.
    let (x, y) = unsafe { (rd(a), rd(b)) };
    let ex = x[2] <= x[0] || x[3] <= x[1];
    let ey = y[2] <= y[0] || y[3] <= y[1];
    let r = match (ex, ey) {
        (true, true) => [0; 4],
        (true, false) => y,
        (false, true) => x,
        _ => [x[0].min(y[0]), x[1].min(y[1]), x[2].max(y[2]), x[3].max(y[3])],
    };
    // SAFETY: as above.
    unsafe { wr(dst, r) };
    i32::from(r != [0; 4])
}

// Hooks come with the input slice.
poly_win32_gui::not_implemented_stubs! {
    /// `SetWindowsHookExW(idHook, lpfn, hmod, dwThreadId)`: NULL on failure.
    SetWindowsHookExW(id, f, module, thread) => 0;
    /// `UnhookWindowsHookEx(hhk)`: zero on failure.
    UnhookWindowsHookEx(hook) => 0;
    /// `CallNextHookEx(hhk, nCode, wParam, lParam)`: zero.
    CallNextHookEx(hook, code, w, l) => 0;
}

// ----- cursors, capture, the window under a point (slice B)

/// The system cursors (LoadCursorW.yaml): IDC_* id, hot spot, colour.
const CURSORS: [(u16, u32, u32, bool); 16] = [
    (32512, 0, 0, true),    // IDC_ARROW
    (32513, 8, 9, false),   // IDC_IBEAM (monochrome)
    (32514, 16, 16, true),  // IDC_WAIT
    (32515, 16, 16, false), // IDC_CROSS (monochrome)
    (32516, 4, 0, true),    // IDC_UPARROW
    (32642, 8, 8, true),    // IDC_SIZENWSE
    (32643, 8, 8, true),    // IDC_SIZENESW
    (32644, 11, 4, true),   // IDC_SIZEWE
    (32645, 4, 11, true),   // IDC_SIZENS
    (32646, 11, 11, true),  // IDC_SIZEALL
    (32648, 8, 8, true),    // IDC_NO
    (32649, 6, 0, true),    // IDC_HAND
    (32650, 0, 8, true),    // IDC_APPSTARTING
    (32651, 0, 0, true),    // IDC_HELP
    (32671, 6, 0, true),    // IDC_PIN
    (32672, 6, 0, true),    // IDC_PERSON
];
/// A system cursor's handle: one value per IDC_* id.
const CURSOR_BASE: usize = 0x0006_0000;
/// winerror.h.
const ERROR_RESOURCE_DATA_NOT_FOUND: u32 = 1812;
const ERROR_RESOURCE_NAME_NOT_FOUND: u32 = 1814;
const ERROR_INVALID_CURSOR_HANDLE: u32 = 1402;

fn cursor_index(h: usize) -> Option<usize> {
    let i = h.checked_sub(CURSOR_BASE)?;
    (i < CURSORS.len()).then_some(i)
}

/// The cursor shape set (GetCursor / SetCursor), per thread: the
/// process's first thread starts with the arrow, a new thread with the
/// wait cursor (SetCursor.yaml `new_thread`).
static CURSORS_SET: SpinLock<Vec<(u32, usize)>> = SpinLock::new(Vec::new());

fn with_cursor<R>(f: impl FnOnce(&mut usize) -> R) -> R {
    let tid = SYS.tid();
    CURSORS_SET.with(|all| {
        let i = match all.iter().position(|e| e.0 == tid) {
            Some(i) => i,
            None => {
                // IDC_ARROW for the first thread, IDC_WAIT (index 2) after.
                let first = all.is_empty();
                all.push((tid, if first { CURSOR_BASE } else { CURSOR_BASE + 2 }));
                all.len() - 1
            }
        };
        f(&mut all[i].1)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadCursorW(instance: H, name: *const u16) -> H {
    if instance != 0 {
        // Our executables carry no cursor resources.
        set_last_error(ERROR_RESOURCE_DATA_NOT_FOUND);
        return 0;
    }
    let id = if (name as usize) >> 16 == 0 {
        Some(name as usize as u16)
    } else {
        // SAFETY: a NUL-terminated name.
        let s = unsafe { wide(name, -1) };
        if s.first() == Some(&u16::from(b'#')) {
            core::char::decode_utf16(s[1..].iter().copied())
                .collect::<Result<alloc::string::String, _>>()
                .ok()
                .and_then(|t| t.parse::<u16>().ok())
        } else {
            None
        }
    };
    match id {
        // IDC_SIZE, IDC_ICON: obsolete, NULL with the error unchanged.
        Some(32640 | 32641) => 0,
        Some(id) => match CURSORS.iter().position(|c| c.0 == id) {
            Some(i) => CURSOR_BASE + i,
            None => {
                set_last_error(ERROR_RESOURCE_NAME_NOT_FOUND);
                0
            }
        },
        None => {
            set_last_error(ERROR_RESOURCE_NAME_NOT_FOUND);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetCursor() -> H {
    with_cursor(|c| *c)
}

/// Sets the shape; the previous one (SetCursor.yaml).
#[unsafe(no_mangle)]
pub extern "system" fn SetCursor(cursor: H) -> H {
    if cursor != 0 && cursor_index(cursor).is_none() {
        set_last_error(ERROR_INVALID_CURSOR_HANDLE);
        return 0;
    }
    with_cursor(|c| core::mem::replace(c, cursor))
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> usize;
}

/// `ICONINFO` (winuser.h): fIcon, xHotspot, yHotspot, hbmMask, hbmColor.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetIconInfo(icon: H, ii: *mut u8) -> i32 {
    let Some(i) = cursor_index(icon) else {
        set_last_error(ERROR_INVALID_CURSOR_HANDLE);
        return 0;
    };
    if ii.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let (_, hx, hy, colour) = CURSORS[i];
    // SAFETY: our gdi32 makes the bitmaps.
    let (mask, color) = unsafe {
        if colour {
            (CreateBitmap(32, 32, 1, 1, core::ptr::null()), CreateBitmap(32, 32, 1, 32, core::ptr::null()))
        } else {
            (CreateBitmap(32, 64, 1, 1, core::ptr::null()), 0)
        }
    };
    // SAFETY: the caller's ICONINFO (32 bytes on x64).
    unsafe {
        core::ptr::write_unaligned(ii as *mut u32, 0);
        core::ptr::write_unaligned(ii.add(4) as *mut u32, hx);
        core::ptr::write_unaligned(ii.add(8) as *mut u32, hy);
        core::ptr::write_unaligned(ii.add(16) as *mut u64, mask as u64);
        core::ptr::write_unaligned(ii.add(24) as *mut u64, color as u64);
    }
    1
}

/// `WM_CAPTURECHANGED` (winuser.h).
const WM_CAPTURECHANGED: u32 = 0x0215;

/// SetCapture.yaml: the previous capture window of the thread's input.
#[unsafe(no_mangle)]
pub extern "system" fn SetCapture(hwnd: H) -> H {
    let st = u::focus::input(&SYS);
    let set = |h: u32| {
        call(Request::SetInput { active: 0, focus: 0, capture: h, which: poly_win32k::wm::input::SET_CAPTURE });
    };
    if hwnd == 0 {
        if st.capture != 0 {
            set(0);
            u::send(&SYS, u64::from(st.capture), WM_CAPTURECHANGED, 0, 0);
        }
        return st.capture as H;
    }
    let Some(i) = info(hwnd) else { return bad_window() };
    if i.tid != SYS.tid() {
        return 0;
    }
    set(i.hwnd);
    if st.capture != 0 {
        u::send(&SYS, u64::from(st.capture), WM_CAPTURECHANGED, 0, u64::from(i.hwnd));
    }
    st.capture as H
}

/// `WindowFromPoint(POINT)` (by value: x in the low half).
#[unsafe(no_mangle)]
pub extern "system" fn WindowFromPoint(pt: u64) -> H {
    let (x, y) = (pt as u32 as i32, (pt >> 32) as u32 as i32);
    let mut below = 0;
    loop {
        let h = match call(Request::WindowFromPoint { x, y, below }) {
            Reply::Handle { hwnd } => hwnd as H,
            _ => 0,
        };
        // The calling thread's candidate gets WM_NCHITTEST (not a disabled
        // top-level window, nor a framed window's caption); HTTRANSPARENT
        // passes it to its parent, a top-level window's to the window below.
        let Some(i) = info(h) else { return h };
        let top = i.style & metrics::WS_CHILD == 0;
        let caption =
            top && i.style & 0x00C0_0000 == 0x00C0_0000 && y < i.client.top && x >= i.client.left && x < i.client.right;
        if i.tid != SYS.tid() || (top && i.style & metrics::WS_DISABLED != 0) || caption {
            return h;
        }
        let l = u::makelparam(x, y);
        let hit = u::send(&SYS, h as u64, 0x0084, 0, l);
        if hit as i64 as i32 != -1 {
            return h;
        }
        if !top {
            u::send(&SYS, u64::from(i.parent), 0x0084, 0, l);
            return i.parent as H;
        }
        below = i.hwnd;
    }
}

/// The direct child of `parent` at a client point, hidden and disabled
/// ones included; `parent` itself over its own area; NULL outside.
#[unsafe(no_mangle)]
pub extern "system" fn ChildWindowFromPoint(parent: H, pt: u64) -> H {
    let (x, y) = (pt as u32 as i32, (pt >> 32) as u32 as i32);
    let Some(p) = info(parent) else { return bad_window() };
    if x < 0 || y < 0 || x >= p.client.width() || y >= p.client.height() {
        return 0;
    }
    let kids = match call(Request::Relatives { hwnd: p.hwnd }) {
        Reply::Relatives { children, .. } => children,
        _ => Vec::new(),
    };
    for c in kids {
        if let Some(ci) = info(c as H)
            && x >= ci.window_rel.left
            && x < ci.window_rel.right
            && y >= ci.window_rel.top
            && y < ci.window_rel.bottom
        {
            return c as H;
        }
    }
    parent
}

/// `TRACKMOUSEEVENT` (24 bytes): cbSize, dwFlags, hwndTrack, dwHoverTime.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TrackMouseEvent(tme: *mut u8) -> i32 {
    const TME_QUERY: u32 = 0x4000_0000;
    const TME_CANCEL: u32 = 0x8000_0000;
    if tme.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's TRACKMOUSEEVENT.
    let (size, flags, hwnd, hover) = unsafe {
        (
            core::ptr::read_unaligned(tme as *const u32),
            core::ptr::read_unaligned(tme.add(4) as *const u32),
            core::ptr::read_unaligned(tme.add(8) as *const u64),
            core::ptr::read_unaligned(tme.add(16) as *const u32),
        )
    };
    if size != 24 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if flags & TME_QUERY != 0 {
        let v = match call(Request::TrackMouse { hwnd: 0, flags, hover: 0 }) {
            Reply::Value { value } => value,
            _ => 0,
        };
        // SAFETY: as above.
        unsafe {
            core::ptr::write_unaligned(tme.add(4) as *mut u32, (v & 0xFFFF) as u32);
            core::ptr::write_unaligned(tme.add(8) as *mut u64, v >> 32);
            core::ptr::write_unaligned(tme.add(16) as *mut u32, ((v >> 16) & 0xFFFF) as u32);
        }
        return 1;
    }
    if flags & !TME_CANCEL == 0 {
        return 1;
    }
    let Some(i) = info(hwnd as H) else { return bad_window() };
    call(Request::TrackMouse { hwnd: i.hwnd, flags, hover });
    1
}

// ----- painting (slice B; poly_win32k::user::paint)

/// An optional RECT argument (`None` for NULL).
unsafe fn opt_rect(p: *const i32) -> Option<Rect> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { rd(p) };
    Some(Rect::new(r[0], r[1], r[2], r[3]))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn BeginPaint(hwnd: H, ps: *mut u::PaintStruct) -> H {
    let mut out = u::PaintStruct::default();
    let dc = u::begin_paint(&SYS, hwnd as u64, &mut out);
    if !ps.is_null() {
        // SAFETY: the caller's PAINTSTRUCT.
        unsafe { *ps = out };
    }
    dc as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn EndPaint(hwnd: H, ps: *const u::PaintStruct) -> i32 {
    // SAFETY: the caller's PAINTSTRUCT, if any.
    let p = if ps.is_null() { u::PaintStruct::default() } else { unsafe { *ps } };
    i32::from(u::end_paint(&SYS, hwnd as u64, &p))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InvalidateRect(hwnd: H, rect: *const i32, erase: i32) -> i32 {
    // SAFETY: the caller's RECT, if any.
    let r = unsafe { opt_rect(rect) };
    i32::from(u::paint::invalidate(&SYS, hwnd as u64, r.as_ref().map(core::slice::from_ref), erase != 0))
}

#[unsafe(no_mangle)]
pub extern "system" fn InvalidateRgn(hwnd: H, rgn: H, erase: i32) -> i32 {
    let rects = if rgn == 0 { None } else { Some(rects_of(rgn).unwrap_or_default()) };
    i32::from(u::paint::invalidate(&SYS, hwnd as u64, rects.as_deref(), erase != 0))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ValidateRect(hwnd: H, rect: *const i32) -> i32 {
    // SAFETY: the caller's RECT, if any.
    let r = unsafe { opt_rect(rect) };
    // An inverted rectangle removes nothing (ValidateRect.yaml).
    let r = r.map(|r| if r.right < r.left || r.bottom < r.top { Rect::default() } else { r });
    i32::from(u::paint::validate(&SYS, hwnd as u64, r.as_ref().map(core::slice::from_ref)))
}

#[unsafe(no_mangle)]
pub extern "system" fn ValidateRgn(hwnd: H, rgn: H) -> i32 {
    let rects = if rgn == 0 { None } else { Some(rects_of(rgn).unwrap_or_default()) };
    i32::from(u::paint::validate(&SYS, hwnd as u64, rects.as_deref()))
}

#[unsafe(no_mangle)]
pub extern "system" fn UpdateWindow(hwnd: H) -> i32 {
    i32::from(u::paint::update_window(&SYS, hwnd as u64))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RedrawWindow(hwnd: H, rect: *const i32, rgn: H, flags: u32) -> i32 {
    // The region wins over the rectangle (RedrawWindow.yaml rw_flags).
    let rects = if rgn != 0 {
        Some(rects_of(rgn).unwrap_or_default())
    } else {
        // SAFETY: the caller's RECT, if any.
        unsafe { opt_rect(rect) }.map(|r| alloc::vec![r])
    };
    i32::from(u::paint::redraw_window(&SYS, hwnd as u64, rects.as_deref(), flags))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUpdateRect(hwnd: H, rect: *mut i32, erase: i32) -> i32 {
    let Some((any, r)) = u::paint::get_update_rect(&SYS, hwnd as u64, erase != 0) else { return 0 };
    if !rect.is_null() {
        // SAFETY: the caller's RECT.
        unsafe { put_rect(rect, r) };
    }
    i32::from(any)
}

/// The update region into `rgn` (client coordinates); its kind.
#[unsafe(no_mangle)]
pub extern "system" fn GetUpdateRgn(hwnd: H, rgn: H, erase: i32) -> i32 {
    let Some((p, _)) = u::paint::update_area(&SYS, hwnd as u64, erase != 0) else { return 0 };
    let flat: Vec<i32> = p.rects.iter().flat_map(|r| [r.left, r.top, r.right, r.bottom]).collect();
    // SAFETY: `p.rects.len()` quadruples.
    unsafe { PolyGdiSetRegionRects(rgn, flat.as_ptr(), p.rects.len() as u32) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn user32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
