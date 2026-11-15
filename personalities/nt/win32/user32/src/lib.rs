//! Our `user32.dll` (ADR 0087, 0128): windows, messages and the default
//! window procedure. Window state lives in win32k (the NT personality),
//! reached through `win32u.dll`'s `PolyUserCall`; the message sequences
//! that call window procedures are `poly_win32k::user`'s, run here with
//! [`Sys`]; drawing goes to our `gdi32.dll`. Also the two string functions
//! 7-Zip's command line imports (`CharUpperW`, `CharPrevExA`, ADR 0087).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

mod acc;
mod ansi;
mod ansiapi;
mod ansires;
mod caret;
mod caretdraw;
mod clipboard;
mod cstructs;
mod ctl;
mod display;
mod dlg;
mod icons;
mod layered;
mod menu;
mod menuansi;
mod menutrack;
mod msgbox;
mod paintctl;
mod s3;
mod strings;
mod winsta;

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
mod hooks;
mod winevent;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RtlUpcaseUnicodeChar(c: u16) -> u16;
}

#[cfg_attr(target_arch = "x86", link(name = "win32u", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "win32u", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: usize, out: *mut u8, cap: usize, out_len: *mut usize) -> u32;
    fn PolyUserCommitSurface(hwnd: usize, pixels: usize, width: usize, height: usize, stride: usize) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetTickCount() -> u32;
    fn CreateEventW(attrs: *mut u8, manual: i32, initial: i32, name: *const u16) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn WaitForMultipleObjects(n: u32, handles: *const usize, all: i32, ms: u32) -> u32;
    fn Sleep(ms: u32);
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyGdiCreateDisplayDC(hwnd: usize, persistent: i32) -> usize;
    fn PolyGdiReleaseDisplayDC(hdc: usize) -> i32;
    fn PolyGdiDeleteDisplayDC(hdc: usize) -> i32;
    fn PolyGdiDisplayWindow(hdc: usize, out: *mut u64) -> i32;
    fn PolyGdiFillRect(hdc: usize, rect: *const i32, brush: usize, color: u32) -> i32;
    fn PolyGdiDrawText(hdc: usize, text: *const u16, count: i32, rect: *mut i32, format: u32) -> i32;
    fn PolyGdiDrawTextEx(
        hdc: usize,
        text: *mut u16,
        count: i32,
        rect: *mut i32,
        format: u32,
        tab_chars: i32,
        left_margin: i32,
        right_margin: i32,
        drawn: *mut u32,
    ) -> i32;
    fn PolyGdiAttachSurface(hdc: usize, width: u32, height: u32) -> i32;
    fn PolyGdiSetDisplayBase(hdc: usize, x: i32, y: i32) -> i32;
    fn PolyGdiAttachSurfaceRows(hdc: usize, width: u32, total: u32, skip: u32, rows: u32) -> i32;
    fn PolyGdiAttachSurfaceRowsTo(hdc: usize, surface_hwnd: usize, width: u32, total: u32, skip: u32, rows: u32)
    -> i32;
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
                | Request::Clip { .. }
        );
        let mut out = vec![0u8; 4096];
        loop {
            let mut len = 0usize;
            // SAFETY: the buffers are ours and sized as passed.
            let s = unsafe { PolyUserCall(bytes.as_ptr(), bytes.len(), out.as_mut_ptr(), out.len(), &mut len) };
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
        if proc_ != 0 && proc_ <= poly_win32k::dlg::sysproc::MAX {
            // A dialog window takes either form itself (an ANSI dialog hands its procedure bytes).
            if proc_ == poly_win32k::dlg::sysproc::DIALOG {
                let form = if ansi::is_text_message(msg) { ansi::take_form() } else { ansi::Form::Wide };
                return dlg::dialog_entry(hwnd as H, msg, w as usize, l as isize, form) as u64;
            }
            // A system class's procedure (`Button`, `Edit`, ...): UTF-16.
            if ansi::is_text_message(msg) && ansi::take_form() == ansi::Form::Ansi {
                let c = ansi::enter(hwnd as H, ansi::Form::Ansi, ansi::Form::Wide, msg, w as usize, l as isize);
                let r = ctl::builtin(proc_, hwnd as H, msg, c.w, c.l) as u64;
                return c.finish(r as isize) as u64;
            }
            return ctl::builtin(proc_, hwnd as H, msg, w as usize, l as isize) as u64;
        }
        if proc_ == 0 {
            if ansi::is_text_message(msg) && ansi::take_form() == ansi::Form::Ansi {
                return ansiapi::def_form(hwnd as H, msg, w as usize, l as isize, ansi::Form::Ansi) as u64;
            }
            return u::def_window_proc(self, u64::from(hwnd), msg, w, l);
        }
        // SAFETY: a window procedure the app registered.
        let f: WndProc = unsafe { core::mem::transmute(proc_ as usize) };
        if ansi::is_text_message(msg) {
            // A message with text in it arrives in the form of the window procedure.
            let (from, to) = (ansi::take_form(), ansi::proc_form(hwnd as H));
            if from != to {
                let c = ansi::enter(hwnd as H, from, to, msg, w as usize, l as isize);
                // SAFETY: the documented call.
                let r = unsafe { f(hwnd as H, msg, c.w, c.l) };
                return c.finish(r) as u64;
            }
        }
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

    fn window_created(&self, hwnd: u32) {
        ime_hook(hwnd as H);
    }

    fn menu_bar_shown(&self, menu: u64) -> bool {
        menu::bar_shown(menu)
    }

    fn window_freed(&self, hwnd: u32, menu: u64) {
        menu::window_freed(hwnd, menu);
    }

    fn menu_hit(&self, hwnd: u32, x: i32, y: i32) -> bool {
        menutrack::hit_menu(hwnd as H, x, y)
    }

    fn menu_click(&self, hwnd: u32, x: i32, y: i32) {
        menutrack::nc_click(hwnd as H, x, y);
    }

    fn menu_sys_key(&self, hwnd: u32, msg: u32, w: u64, l: u64) -> bool {
        menutrack::sys_key(hwnd as H, msg, w, l)
    }

    fn menu_paint(&self, hwnd: u32) {
        menutrack::draw_bar(hwnd as H);
    }

    fn set_cursor(&self, cursor: u64) {
        with_cursor(|c| *c = cursor as usize);
    }

    fn region_rects(&self, rgn: u64) -> Option<Vec<Rect>> {
        rects_of(rgn as H)
    }

    fn set_region_rects(&self, rgn: u64, rects: &[Rect]) -> i32 {
        let flat: Vec<i32> = rects.iter().flat_map(|r| [r.left, r.top, r.right, r.bottom]).collect();
        // SAFETY: `rects.len()` quadruples.
        unsafe { PolyGdiSetRegionRects(rgn as H, flat.as_ptr(), rects.len() as u32) }
    }

    fn delete_object(&self, h: u64) {
        // SAFETY: our gdi32 checks the handle.
        unsafe { DeleteObject(h as H) };
    }

    fn sent_hook(&self, hwnd: u32, msg: u32, w: u64, l: u64, same_thread: bool) {
        hooks::sent(hwnd, msg, w, l, same_thread);
    }

    fn message_hook(&self, msg: &mut CMsg, remove: bool) {
        if remove {
            // The message brings its own extra information (0: the ones posted with PostMessage).
            extra_info(Some(0));
        }
        hooks::retrieved(msg, remove);
    }

    fn ime_ui_window(&self, ime: u32) -> u32 {
        make_ime_ui(ime as H) as u32
    }

    fn win_event(&self, event: u32, hwnd: u32, id_object: i32, id_child: i32) {
        winevent::raise(event, hwnd, id_object, id_child);
    }

    fn pump_events(&self) {
        winevent::pump();
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

/// The mapping a `Char*` function does: one unit of a wide string.
fn map_wide(kind: poly_kernelbase::locale::MapKind, c: u16) -> u16 {
    poly_kernelbase::locale::map_unit(kind, c)
}

/// A `CharUpper`/`CharLower` function on a wide string or one character in the low word.
unsafe fn map_w(kind: poly_kernelbase::locale::MapKind, s: *mut u16) -> *mut u16 {
    if poly_kernelbase::oleaut::is_single_char(s as usize) {
        return usize::from(map_wide(kind, s as usize as u16)) as *mut u16;
    }
    let mut p = s;
    // SAFETY: a NUL-terminated string the caller owns.
    unsafe {
        while *p != 0 {
            *p = map_wide(kind, *p);
            p = p.add(1);
        }
    }
    s
}

/// A `CharUpper`/`CharLower` function on an ANSI string or one character in the low word.
unsafe fn map_a(kind: poly_kernelbase::locale::MapKind, s: *mut u8) -> *mut u8 {
    if poly_kernelbase::oleaut::is_single_char(s as usize) {
        return usize::from(poly_kernelbase::locale::map_ansi_byte(kind, s as usize as u8)) as *mut u8;
    }
    let mut p = s;
    // SAFETY: a NUL-terminated string the caller owns.
    unsafe {
        while *p != 0 {
            *p = poly_kernelbase::locale::map_ansi_byte(kind, *p);
            p = p.add(1);
        }
    }
    s
}

/// `CharLowerW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharLowerW(s: *mut u16) -> *mut u16 {
    // SAFETY: the caller's string or character.
    unsafe { map_w(poly_kernelbase::locale::MapKind::Lower, s) }
}

/// `CharUpperA`, `CharLowerA`: the ANSI code page's letters.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharUpperA(s: *mut u8) -> *mut u8 {
    // SAFETY: the caller's string or character.
    unsafe { map_a(poly_kernelbase::locale::MapKind::Upper, s) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharLowerA(s: *mut u8) -> *mut u8 {
    // SAFETY: the caller's string or character.
    unsafe { map_a(poly_kernelbase::locale::MapKind::Lower, s) }
}

/// `CharUpperBuffW` and the three like it: map `cch` characters in place and return how many
/// (Learn: the length of the string, or 0 for a length of 0, nothing for a NULL pointer).
unsafe fn buff_w(kind: poly_kernelbase::locale::MapKind, s: *mut u16, cch: u32) -> u32 {
    if s.is_null() {
        return 0;
    }
    for i in 0..cch as usize {
        // SAFETY: the caller's buffer of `cch` units.
        unsafe { *s.add(i) = map_wide(kind, *s.add(i)) };
    }
    cch
}

unsafe fn buff_a(kind: poly_kernelbase::locale::MapKind, s: *mut u8, cch: u32) -> u32 {
    if s.is_null() {
        return 0;
    }
    for i in 0..cch as usize {
        // SAFETY: the caller's buffer of `cch` bytes.
        unsafe { *s.add(i) = poly_kernelbase::locale::map_ansi_byte(kind, *s.add(i)) };
    }
    cch
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharUpperBuffW(s: *mut u16, cch: u32) -> u32 {
    // SAFETY: the caller's buffer.
    unsafe { buff_w(poly_kernelbase::locale::MapKind::Upper, s, cch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharLowerBuffW(s: *mut u16, cch: u32) -> u32 {
    // SAFETY: the caller's buffer.
    unsafe { buff_w(poly_kernelbase::locale::MapKind::Lower, s, cch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharUpperBuffA(s: *mut u8, cch: u32) -> u32 {
    // SAFETY: the caller's buffer.
    unsafe { buff_a(poly_kernelbase::locale::MapKind::Upper, s, cch) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharLowerBuffA(s: *mut u8, cch: u32) -> u32 {
    // SAFETY: the caller's buffer.
    unsafe { buff_a(poly_kernelbase::locale::MapKind::Lower, s, cch) }
}

/// `CharNextA`, `CharNextW`: the next character, or the same pointer at the terminating NUL
/// (every code page Poly serves to ANSI programs is single-byte, as for `CharPrevExA`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharNextA(p: *const u8) -> *mut u8 {
    // SAFETY: a NUL-terminated string.
    if p.is_null() || unsafe { *p } == 0 { p as *mut u8 } else { p.wrapping_add(1) as *mut u8 }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CharNextW(p: *const u16) -> *mut u16 {
    // SAFETY: a NUL-terminated string.
    if p.is_null() || unsafe { *p } == 0 { p as *mut u16 } else { p.wrapping_add(1) as *mut u16 }
}

/// `CharPrevW`: the character before `current`, or `start`.
#[unsafe(no_mangle)]
pub extern "system" fn CharPrevW(start: *const u16, current: *const u16) -> *mut u16 {
    if current as usize <= start as usize { start as *mut u16 } else { current.wrapping_sub(1) as *mut u16 }
}

/// `IsCharAlphaW`, `IsCharAlphaNumericW`, `IsCharUpperW`, `IsCharLowerW`: `CT_CTYPE1` of the
/// character (Learn: a letter, a letter or a digit, an upper- or a lower-case letter).
#[unsafe(no_mangle)]
pub extern "system" fn IsCharAlphaW(c: u16) -> i32 {
    i32::from(poly_kernelbase::locale::ctype1(c) & poly_kernelbase::locale::C1_ALPHA != 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn IsCharAlphaNumericW(c: u16) -> i32 {
    use poly_kernelbase::locale::{C1_ALPHA, C1_DIGIT, ctype1};
    i32::from(ctype1(c) & (C1_ALPHA | C1_DIGIT) != 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn IsCharUpperW(c: u16) -> i32 {
    i32::from(poly_kernelbase::locale::ctype1(c) & poly_kernelbase::locale::C1_UPPER != 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn IsCharLowerW(c: u16) -> i32 {
    i32::from(poly_kernelbase::locale::ctype1(c) & poly_kernelbase::locale::C1_LOWER != 0)
}

/// The strings `wsprintf` reads: NUL-terminated, 16-bit units or bytes widened one to one.
fn read_printf_string(p: usize, wide: bool) -> alloc::vec::Vec<u16> {
    let mut v = alloc::vec::Vec::new();
    // SAFETY: the caller of `wsprintf` passed a pointer to a NUL-terminated string for `%s`.
    unsafe {
        if wide {
            let mut q = p as *const u16;
            while *q != 0 {
                v.push(*q);
                q = q.add(1);
            }
        } else {
            let mut q = p as *const u8;
            while *q != 0 {
                v.push(poly_kernelbase::text::from_ansi(&[*q])[0]);
                q = q.add(1);
            }
        }
    }
    v
}

/// `wsprintfW(buffer, format, ...)`: a `cdecl` function with a variable argument list. The
/// arguments are read as the words the caller pushed (Rust cannot define a variadic function;
/// words past the caller's own are the stack's, which the formatter reads only if the format
/// asks for them).
///
/// # Safety
/// `buffer` holds 1024 units; `format` is NUL-terminated and its arguments are present.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsprintfW(
    buffer: *mut u16,
    format: *const u16,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    a8: usize,
    a9: usize,
    a10: usize,
    a11: usize,
) -> i32 {
    let args = [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11];
    // SAFETY: the format is a NUL-terminated string.
    let fmt = unsafe { poly_kernelbase::text::wide_at(format) };
    let out = poly_kernelbase::wsprintf::format(
        fmt,
        true,
        &args,
        core::mem::size_of::<usize>(),
        &read_printf_string,
    );
    // SAFETY: the buffer holds 1024 units; the output is at most 1023.
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), buffer, out.len());
        buffer.add(out.len()).write(0);
    }
    out.len() as i32
}

/// `wsprintfA`: the same with ANSI strings.
///
/// # Safety
/// As [`wsprintfW`], with bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wsprintfA(
    buffer: *mut u8,
    format: *const u8,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    a8: usize,
    a9: usize,
    a10: usize,
    a11: usize,
) -> i32 {
    let args = [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11];
    // SAFETY: the format is a NUL-terminated string.
    let fmt = poly_kernelbase::text::from_ansi(unsafe { core::slice::from_raw_parts(format, poly_kernelbase::text::strlen(format)) });
    let out = poly_kernelbase::wsprintf::format(
        &fmt,
        false,
        &args,
        core::mem::size_of::<usize>(),
        &read_printf_string,
    );
    let bytes = poly_kernelbase::text::to_ansi(&out);
    // SAFETY: the buffer holds 1024 bytes; the output is at most 1023.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len());
        buffer.add(bytes.len()).write(0);
    }
    bytes.len() as i32
}

/// `CharPrevExA`.
#[unsafe(no_mangle)]
pub extern "system" fn CharPrevExA(_code_page: u16, start: *const u8, current: *const u8, _flags: u32) -> *mut u8 {
    poly_kernelbase::oleaut::char_prev(start as usize, current as usize) as *mut u8
}

// ----- classes

pub(crate) fn register_class(spec: ClassSpec) -> u16 {
    register(spec)
}

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
    // SAFETY: a WNDCLASSEXW (winuser.h: 80 bytes on x64, 48 in a 32-bit process).
    unsafe {
        if (wc as *const u32).read_unaligned() as usize != core::mem::size_of::<cstructs::WndClassEx>() {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        let c = (wc as *const cstructs::WndClassEx).read_unaligned();
        let name = c.class_name as *const u16;
        if (name as usize) < 0x1_0000 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        register(ClassSpec {
            name: wide(name, -1),
            style: c.style,
            wndproc: c.wndproc as u64,
            cls_extra: c.cls_extra,
            wnd_extra: c.wnd_extra,
            instance: c.instance as u64,
            icon: c.icon as u64,
            cursor: c.cursor as u64,
            brush: c.brush as u64,
            menu_name: c.menu_name as u64,
            icon_sm: c.icon_sm as u64,
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
    // SAFETY: a WNDCLASSW (72 bytes on x64, 40 in a 32-bit process).
    unsafe {
        let c = (wc as *const cstructs::WndClass).read_unaligned();
        let name = c.class_name as *const u16;
        if (name as usize) < 0x1_0000 {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        register(ClassSpec {
            name: wide(name, -1),
            style: c.style,
            wndproc: c.wndproc as u64,
            cls_extra: c.cls_extra,
            wnd_extra: c.wnd_extra,
            instance: c.instance as u64,
            icon: c.icon as u64,
            cursor: c.cursor as u64,
            brush: c.brush as u64,
            menu_name: c.menu_name as u64,
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
            let wndproc = if info.wndproc == 0 { DefWindowProcW as *const () as usize } else { info.wndproc as usize };
            let c = cstructs::WndClassEx {
                size: core::mem::size_of::<cstructs::WndClassEx>() as u32,
                style: info.style,
                wndproc,
                cls_extra: info.cls_extra,
                wnd_extra: info.wnd_extra,
                instance: info.instance as usize,
                icon: info.icon as usize,
                cursor: info.cursor as usize,
                brush: info.brush as usize,
                menu_name: info.menu_name as usize,
                class_name: name as usize,
                icon_sm: info.icon_sm as usize,
            };
            // SAFETY: the caller's WNDCLASSEXW; its cbSize is left as the caller set it.
            unsafe {
                let size = (wc as *const u32).read_unaligned();
                (wc as *mut cstructs::WndClassEx).write_unaligned(c);
                (wc as *mut u32).write_unaligned(size);
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

/// `SetClassLongPtrW`: the previous value, or 0 with the last error for a bad window or index.
#[unsafe(no_mangle)]
pub extern "system" fn SetClassLongPtrW(hwnd: H, index: i32, value: isize) -> usize {
    match call(Request::SetClassLong { hwnd: hwnd as u64, index, value: value as usize as u64 }) {
        Reply::Value { value } => value as usize,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

/// `SetClassLongW`: the 32-bit form (the value is a `LONG`, the previous value a `DWORD`).
#[unsafe(no_mangle)]
pub extern "system" fn SetClassLongW(hwnd: H, index: i32, value: i32) -> u32 {
    SetClassLongPtrW(hwnd, index, value as u32 as isize) as u32
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClassLongW(hwnd: H, index: i32) -> u32 {
    GetClassLongPtrW(hwnd, index) as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClassNameW(hwnd: H, buf: *mut u16, max: i32) -> i32 {
    match call(Request::ClassName { hwnd: hwnd as u64 }) {
        Reply::Text { text } => {
            if buf.is_null() || max < 0 {
                // ERROR_NOACCESS: no buffer, or a size that is negative.
                set_last_error(998);
                return 0;
            }
            if max <= 1 {
                // ERROR_INSUFFICIENT_BUFFER: no room for a character and the NUL; nothing is written.
                set_last_error(122);
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
    first_call();
    if class.is_null() {
        set_last_error(poly_win32k::wm::ERROR_CANNOT_FIND_WND_CLASS);
        return 0;
    }
    // SAFETY: a class name or atom.
    let class = unsafe { class_ref(class) };
    let mut menu = menu;
    if style & 0x4000_0000 == 0 {
        // A top-level window's menu: the handle given, else a copy of the class's menu.
        if menu != 0 {
            if !menu::is_menu(menu) {
                set_last_error(menu::ERROR_INVALID_MENU_HANDLE);
                return 0;
            }
        } else if let Reply::Class { info } =
            call(Request::ClassInfo { class: class.clone(), instance: instance as u64 })
            && info.menu_name != 0
        {
            menu = menu::class_menu(info.instance as usize, info.menu_name);
        }
    }
    if style & poly_win32k::metrics::WS_CHILD != 0 && ex_style & poly_win32k::guic::WS_EX_LAYERED != 0 {
        layered_children();
    }
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
        ansi: ansiapi::ansi_next_window(),
    };
    let h = u::create_window(&SYS, spec, name as u64, param as u64) as H;
    if h != 0 && menu != 0 && style & 0x4000_0000 == 0 {
        menu::after_create(h);
    }
    h
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
}

/// The bitmap resource `ord` of `inst` (a static control's `SS_BITMAP`).
pub(crate) fn load_bitmap_ordinal(inst: usize, ord: u16) -> usize {
    // SAFETY: a resource id as the name.
    unsafe { icons::LoadBitmapW(inst, ord as usize as *const u16) }
}

/// The menu resource `ord` of `inst` (0 until menus are loaded here).
pub(crate) fn load_menu_ordinal(_inst: usize, _ord: u16) -> usize {
    0
}

/// The menu resource `name` of `inst`.
pub(crate) fn load_menu_name(_inst: usize, _name: *const u16) -> usize {
    0
}

/// Tell win32k once whether the executable's manifest declares Windows 8
/// or later, which layered child windows need (Learn, "Layered Windows").
fn layered_children() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static TOLD: AtomicBool = AtomicBool::new(false);
    if TOLD.swap(true, Ordering::Relaxed) {
        return;
    }
    // SAFETY: NULL names the executable, mapped in this process.
    let base = unsafe { GetModuleHandleW(core::ptr::null()) } as u64;
    // SAFETY: the executable's base.
    let manifest = unsafe { poly_kernelbase::version::image_manifest(base) };
    if manifest.is_some_and(|m| poly_kernelbase::version::declares_win8(&m)) {
        call(Request::LayeredChildren {});
    }
}

/// The class of a thread's default IME window.
static IME_CLASS: [u16; 4] = [b'I' as u16, b'M' as u16, b'E' as u16, 0];

/// Each thread gets its hidden default IME window with its first window
/// (ImmGetDefaultIMEWnd.yaml: class "IME", WS_POPUP | WS_DISABLED |
/// WS_CLIPSIBLINGS, owned by the thread's first window; GetFocus.yaml
/// `ime_window`: it is made before that window is shown and lies below it).
/// The interactive session has one for every thread, as Windows does;
/// session 0 only for a process that has `imm32.dll` loaded.
fn ime_hook(hwnd: H) {
    if !matches!(call(Request::DefaultIme { tid: 0 }), Reply::Handle { hwnd: 0 }) {
        return;
    }
    // Making it is not the caller's business: the last error stays as it
    // was (a second thread registers the class again and would leave
    // ERROR_CLASS_ALREADY_EXISTS; OpenClipboard.yaml oc_threads reads 0).
    let error = poly_win32_gui::last_error();
    make_ime_window(hwnd);
    set_last_error(error);
}

/// The class of the text services' helper window.
static IME_UI_CLASS: [u16; 12] = [
    b'M' as u16,
    b'S' as u16,
    b'C' as u16,
    b'T' as u16,
    b'F' as u16,
    b'I' as u16,
    b'M' as u16,
    b'E' as u16,
    b' ' as u16,
    b'U' as u16,
    b'I' as u16,
    0,
];

/// `MSCTFIME UI`, the helper window of the default IME window `ime`: a hidden disabled popup it
/// owns, of a class with `CS_IME`, made with the thread's first activation (sw_chrome_show).
fn make_ime_ui(ime: H) -> H {
    // The last error stays as it was, as for the IME window itself.
    let error = poly_win32_gui::last_error();
    let mut wc = [0u8; 80];
    wc[0..4].copy_from_slice(&80u32.to_le_bytes());
    wc[4..8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    wc[8..16].copy_from_slice(&(DefWindowProcW as *const () as usize as u64).to_le_bytes());
    wc[64..72].copy_from_slice(&(IME_UI_CLASS.as_ptr() as u64).to_le_bytes());
    let style = metrics::WS_POPUP | metrics::WS_DISABLED;
    // SAFETY: a valid class description (the class may exist already); a hidden popup owned by `ime`.
    let ui = unsafe {
        RegisterClassExW(wc.as_ptr());
        let ui = CreateWindowExW(0, IME_UI_CLASS.as_ptr(), IME_UI_CLASS.as_ptr(), style, 0, 0, 0, 0, ime, 0, 0, 0);
        if ui != 0 {
            call(Request::SetStyle { hwnd: ui as u32, which: -16, value: style | metrics::WS_CLIPSIBLINGS });
        }
        ui
    };
    set_last_error(error);
    ui
}

fn make_ime_window(hwnd: H) {
    if !u::screen(&SYS).interactive {
        let name: [u16; 10] = [105, 109, 109, 51, 50, 46, 100, 108, 108, 0];
        // SAFETY: a kernel32 query with a NUL-terminated name.
        if unsafe { GetModuleHandleW(name.as_ptr()) } == 0 {
            return;
        }
    }
    // WNDCLASSEXW (80 bytes): cbSize, style, wndproc, extra, extra, instance,
    // icon, cursor, brush, menu name, class name, small icon.
    let mut wc = [0u8; 80];
    wc[0..4].copy_from_slice(&80u32.to_le_bytes());
    wc[8..16].copy_from_slice(&(DefWindowProcW as *const () as usize as u64).to_le_bytes());
    wc[64..72].copy_from_slice(&(IME_CLASS.as_ptr() as u64).to_le_bytes());
    // SAFETY: a valid class description (the class may exist already);
    // a hidden popup owned by the thread's first window.
    unsafe {
        RegisterClassExW(wc.as_ptr());
        let style = metrics::WS_POPUP | metrics::WS_DISABLED;
        let ime = CreateWindowExW(0, IME_CLASS.as_ptr(), IME_CLASS.as_ptr(), style, 0, 0, 0, 0, hwnd, 0, 0, 0);
        // The window's own style has WS_CLIPSIBLINGS, though the procedure's `CREATESTRUCT` has
        // not (cw_chrome_widget): added after its creation messages.
        if ime != 0 {
            call(Request::SetStyle { hwnd: ime as u32, which: -16, value: style | metrics::WS_CLIPSIBLINGS });
        }
    }
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
    // In session 0 no window is visible: 0 for every shown window too (the
    // sheets' `not_covered` of slice A; ShowWindow.yaml sw_activation
    // prints every window as not visible there).
    i32::from(u::screen(&SYS).interactive && info(hwnd).is_some_and(|i| i.visible))
}

#[unsafe(no_mangle)]
pub extern "system" fn IsWindowUnicode(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.unicode))
}

#[unsafe(no_mangle)]
pub extern "system" fn IsIconic(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.style & metrics::WS_MINIMIZE != 0))
}

#[unsafe(no_mangle)]
pub extern "system" fn IsZoomed(hwnd: H) -> i32 {
    i32::from(info(hwnd).is_some_and(|i| i.style & metrics::WS_MAXIMIZE != 0))
}

/// `GetShellWindow`: "the handle of the Shell's desktop window. If no Shell process is present,
/// the return value is NULL" (Learn). Poly's desktop is its own shell, not a Windows shell
/// process with a window in this session: NULL. (Chrome delay-loads the function when it starts;
/// ADR 0155, step 9. Doc-derived.)
#[unsafe(no_mangle)]
pub extern "system" fn GetShellWindow() -> H {
    // The interactive session has windows of another process (the shell's): the lowest of them is
    // the shell window (GetKeyboardLayout.yaml `other_threads`). None in session 0.
    let desktop = u::desktop(&SYS);
    let Reply::Relatives { children, .. } = call(Request::Relatives { hwnd: desktop }) else { return 0 };
    children.into_iter().rev().find(|&c| info(c as H).is_some_and(|i| i.pid >= 0xFFFF_FFF0)).map_or(0, |c| c as H)
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
    // A NULL or made-up handle is no window: ERROR_INVALID_WINDOW_HANDLE (IsChild.yaml `chrome`).
    if info(parent).is_none() || info(child).is_none() {
        return bad_window();
    }
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

/// ClientToScreen / ScreenToClient of the caller's POINT (`WS_EX_LAYOUTRTL`
/// windows count x from the client area's right edge).
unsafe fn convert(hwnd: H, pt: *mut i32, to_screen: bool) -> i32 {
    // SAFETY: the caller's POINT, if any.
    let p = if pt.is_null() { None } else { Some(unsafe { (*pt, *pt.add(1)) }) };
    // A bad window is reported before a bad pointer.
    let Some(i) = (hwnd != 0).then(|| info(hwnd)).flatten() else { return bad_window() };
    let Some(p) = p else {
        set_last_error(ERROR_NOACCESS);
        return 0;
    };
    let Some((x, y)) = u::convert_point(&SYS, u64::from(i.hwnd), p, to_screen) else { return 0 };
    // SAFETY: the caller's POINT, checked above.
    unsafe {
        *pt = x;
        *pt.add(1) = y;
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ClientToScreen(hwnd: H, pt: *mut i32) -> i32 {
    // NULL is not a window here (ClientToScreen.yaml).
    // SAFETY: the caller's POINT.
    unsafe { convert(hwnd, pt, true) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ScreenToClient(hwnd: H, pt: *mut i32) -> i32 {
    // NULL is not a window here (ScreenToClient.yaml).
    // SAFETY: the caller's POINT.
    unsafe { convert(hwnd, pt, false) }
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
        // The dialog procedure of an ANSI dialog reads as a thunk through the wide function
        // (DialogBoxParamA.yaml: `GetWindowLongPtrW(DWLP_DLGPROC)` is not the procedure given).
        Reply::Value { value } if index == 8 && value != 0 && ansiapi::is_ansi_dialog(hwnd) => {
            (value ^ ansiapi::THUNK_TAG) as isize
        }
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
        // (A window without redirection bitmap, Chrome's, keeps its client rectangle until
        // SetWindowPos(SWP_FRAMECHANGED): swl_chrome.)
        if index == GWL_STYLE
            && i.ex_style & metrics::WS_EX_NOREDIRECTIONBITMAP == 0
            && u::frame_style_changed(old, stored)
        {
            u::style_frame_changed(&SYS, h);
        }
        return u64::from(old);
    }
    if index == -12 && !menu::check_top_level_id(hwnd, value) {
        return 0;
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
        winevent::raise(poly_win32k::winevent::EVENT_OBJECT_NAMECHANGE, hwnd as u32, 0, 0);
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
    layout: usize,
) -> i32 {
    // A layout that is not installed gives no character (ToUnicodeEx.yaml `layouts`).
    if layout != 0 && s3::PrivateLayoutInstalled(layout) == 0 {
        if !buffer.is_null() && buffer_len > 0 {
            // SAFETY: the caller's buffer of `buffer_len` characters.
            unsafe { *buffer = 0 };
        }
        return 0;
    }
    // SAFETY: the caller's promises, passed on.
    unsafe { ToUnicode(vk, scan, key_state, buffer, buffer_len, flags) }
}

/// `GetKeyboardLayout`: every thread has the oracle's layout
/// (`MapVirtualKeyW.yaml`: HKL `0x04072000`).
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyboardLayout(thread: u32) -> usize {
    // A thread that has no window (or does not exist) has no layout of its own to give: 0
    // (GetKeyboardLayout.yaml `other_threads`).
    if thread == 0 || thread == SYS.tid() {
        return kbd::HKL as usize;
    }
    match call(Request::ThreadHasWindows { tid: thread }) {
        Reply::Bool { value: true } => kbd::HKL as usize,
        _ => 0,
    }
}

// ----- metrics

/// Whether one of the functions that clear the last error once has run in this
/// process (`SystemParametersInfoW.yaml` `first_call`).
static FIRST_CALL_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The FIRST call of the process to `GetSystemMetrics`, `GetSystemMetricsForDpi`,
/// `SystemParametersInfoW`, `AdjustWindowRect` or `CreateWindowExW` sets the
/// last error to 0 before it returns; once per process, not per thread, and
/// after one of them has done it none of the others does. The sheet has this
/// in a process of a desktop session only (in session 0, without a desktop,
/// no call clears the error); every Poly program runs on Poly's desktop, so
/// it always does.
pub(crate) fn first_call() {
    if !FIRST_CALL_DONE.swap(true, core::sync::atomic::Ordering::AcqRel) {
        set_last_error(0);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetrics(index: i32) -> i32 {
    first_call();
    match metrics::system_metric(index, &u::screen(&SYS)) {
        Some(v) => v,
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

/// `ERROR_FILE_NOT_FOUND` (winerror.h): what index 87 (SM_MEDIACENTER) leaves.
const ERROR_FILE_NOT_FOUND: u32 = 2;

/// GetSystemMetricsForDpi.yaml `dpi_validity`: a DPI of 0 or of 0x80000000 and
/// more fails for every index, a metric that scales takes the DPI modulo 65536
/// and fails for a multiple of it, the others accept 1..0x7FFFFFFF; an
/// undefined index is 0 with ERROR_INVALID_PARAMETER at every DPI; index 30
/// clears the last error with every successful call, 87 sets
/// ERROR_FILE_NOT_FOUND.
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32 {
    first_call();
    let Some(dpi) = metrics::metric_dpi(index, dpi) else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    match metrics::system_metric_for_dpi(index, dpi, &u::screen(&SYS)) {
        Some(v) => {
            match index {
                30 => set_last_error(0),
                87 => set_last_error(ERROR_FILE_NOT_FOUND),
                _ => {}
            }
            v
        }
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
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
    // A minimized window is judged by its restored rectangle; a window of
    // no size by its corner (`rect_on_screen`).
    let w = u::place::monitor_rect(&SYS, &i);
    monitor_for(rect_on_screen([w.left, w.top, w.right, w.bottom]), flags)
}

/// `GetWindowPlacement` (Learn; MonitorFromWindow.yaml `state_rects`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowPlacement(hwnd: H, wp: *mut u::place::WindowPlacement) -> i32 {
    if wp.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut out = u::place::WindowPlacement::default();
    if !u::place::get_window_placement(&SYS, hwnd as u64, &mut out) {
        return 0;
    }
    // SAFETY: the caller's WINDOWPLACEMENT.
    unsafe { wp.write_unaligned(out) };
    1
}

/// `SetWindowPlacement` (Learn; MonitorFromWindow.yaml `state_rects`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWindowPlacement(hwnd: H, wp: *const u::place::WindowPlacement) -> i32 {
    if wp.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's WINDOWPLACEMENT.
    let wp = unsafe { wp.read_unaligned() };
    i32::from(u::place::set_window_placement(&SYS, hwnd as u64, &wp))
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

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AdjustWindowRectEx(rect: *mut i32, style: u32, menu: i32, ex: u32) -> i32 {
    first_call();
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

// ----- painting with GDI

#[unsafe(no_mangle)]
pub extern "system" fn GetDC(hwnd: H) -> H {
    const CS_OWNDC: u32 = 0x20;
    const CS_CLASSDC: u32 = 0x40;
    if hwnd == 0 {
        // The screen: all of it is visible.
        let d = u::screen(&SYS);
        let all = [0, 0, d.width, d.height];
        // SAFETY: our gdi32.
        let dc = unsafe { PolyGdiCreateDisplayDC(0, 0) };
        if dc != 0 {
            // SAFETY: one RECT.
            unsafe { PolyGdiSetDisplayGeometry(dc, 0, 0, all.as_ptr(), 1, core::ptr::null(), 0) };
        }
        return dc;
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
    // Not GetSystemMetrics: a DC must not use up the first-call clearing of the last error.
    let d = u::screen(&SYS);
    let screen = Rect::new(0, 0, d.width, d.height);
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
        // A child window's DC draws into its top-level window's surface, at the child's place in it.
        let (ox, oy, bx, by) = match root_surface(i) {
            Some((root, w, total)) => {
                // The surface holds the root's menu bar above its client area.
                let bar = (root.menu_bar.max(0) as u32).min(total.saturating_sub(1));
                // SAFETY: our gdi32 checks the DC.
                unsafe { PolyGdiAttachSurfaceRowsTo(dc, root.hwnd as H, w, total, bar, total - bar) };
                (root.client.left, root.client.top, area.left - root.client.left, area.top - root.client.top)
            }
            None => (area.left, area.top, 0, 0),
        };
        // SAFETY: our gdi32 reads 1 and `exclude.len()` RECTs.
        unsafe {
            PolyGdiSetDisplayGeometry(dc, ox, oy, inc.as_ptr(), 1, exc.as_ptr(), exclude.len() as u32);
            PolyGdiSetDisplayBase(dc, bx, by);
        };
    }
    dc
}

/// The top-level window a child window's surface is, with the surface's size, when it has one.
fn root_surface(i: &WinInfo) -> Option<(WinInfo, u32, u32)> {
    if i.style & metrics::WS_CHILD == 0 {
        return None;
    }
    let mut cur = i.clone();
    while cur.style & metrics::WS_CHILD != 0 {
        cur = info(cur.parent as H)?;
    }
    match call(Request::Surface { hwnd: cur.hwnd }) {
        Reply::Value { value } if value != 0 => Some((cur, value as u32, (value >> 32) as u32)),
        _ => None,
    }
}

/// The top-level window of `hwnd` (itself when it is one).
fn top_level_of(hwnd: H) -> H {
    let mut cur = hwnd;
    for _ in 0..64 {
        match info(cur) {
            Some(i) if i.style & metrics::WS_CHILD != 0 => cur = i.parent as H,
            _ => break,
        }
    }
    cur
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
        // The surface holds the menu bar above the client area: a client DC draws
        // on the rows below it.
        let (w, total) = (size as u32, (size >> 32) as u32);
        let bar = if total > 0 { info(hwnd as H).map_or(0, |i| i.menu_bar.max(0) as u32).min(total - 1) } else { 0 };
        // SAFETY: our gdi32 checks the DC.
        unsafe { PolyGdiAttachSurfaceRows(dc, w, total, bar, total - bar) };
    }
    dc
}

/// What the window's DCs drew goes to its surface (`ReleaseDC`).
fn commit_surface(hwnd: H) {
    let hwnd = if hwnd == 0 { 0 } else { top_level_of(hwnd) };
    let mut s = [0u64; 3];
    // SAFETY: our gdi32 writes three u64s.
    if hwnd == 0 || unsafe { PolyGdiSurface(hwnd, s.as_mut_ptr()) } == 0 {
        return;
    }
    // SAFETY: win32u's stub; the kernel reads `height` rows of the DIB.
    unsafe { PolyUserCommitSurface(hwnd as usize, s[0] as usize, s[1] as usize, s[2] as usize, s[1] as usize * 4) };
}

/// `WindowFromDC` (BeginPaint.yaml `erase_sent`): the window a DC from
/// GetDC, GetWindowDC or BeginPaint belongs to; the desktop window for the
/// screen's DC; NULL for a memory DC, a released DC or a made-up handle.
#[unsafe(no_mangle)]
pub extern "system" fn WindowFromDC(hdc: H) -> H {
    let mut w = 0u64;
    // SAFETY: our gdi32 writes one u64.
    if unsafe { PolyGdiDisplayWindow(hdc, &mut w) } == 0 {
        return 0;
    }
    if w == 0 { GetDesktopWindow() } else { w as H }
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
        (0, display::GetSysColor(brush as i32 - 1))
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

/// `DT_MODIFYSTRING` and `DT_TABSTOP` (`winuser.h`).
const DT_MODIFYSTRING: u32 = 0x1_0000;
const DT_TABSTOP: u32 = 0x0080;

/// `DrawTextExW`; `DRAWTEXTPARAMS` is `cbSize`, `iTabLength`,
/// `iLeftMargin`, `iRightMargin`, `uiLengthDrawn` (`winuser.h`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DrawTextExW(
    hdc: H,
    text: *mut u16,
    count: i32,
    rect: *mut i32,
    format: u32,
    params: *mut u32,
) -> i32 {
    // Without a parameter block the call is `DrawTextW`: the tab length of
    // `DT_TABSTOP` is in the format (a negative `tab` tells gdi32 so).
    let (mut tab, mut left, mut right) = (-1, 0, 0);
    let mut drawn: *mut u32 = core::ptr::null_mut();
    if !params.is_null() {
        // SAFETY: the caller's DRAWTEXTPARAMS: `cbSize` first.
        unsafe {
            if *params != 20 {
                set_last_error(ERROR_INVALID_PARAMETER);
                return 0;
            }
            tab = if format & DT_TABSTOP != 0 { (*params.add(1) as i32).max(0) } else { 0 };
            left = *params.add(2) as i32;
            right = *params.add(3) as i32;
            drawn = params.add(4);
        }
    }
    // SAFETY: our gdi32 reads the string and RECT as documented.
    unsafe { PolyGdiDrawTextEx(hdc, text, count, rect, format, tab, left, right, drawn) }
}

/// A string in the ANSI code page as UTF-16 with room for an ellipsis and
/// the NUL (Latin-1, as the rest of Poly's ANSI functions).
unsafe fn ansi_text(p: *const u8, count: i32) -> (Vec<u16>, usize) {
    let mut v: Vec<u16> = Vec::new();
    if !p.is_null() {
        let mut i = 0usize;
        loop {
            // SAFETY: the caller's string (count -1: NUL-terminated).
            let c = unsafe { *p.add(i) };
            if (count >= 0 && i >= count as usize) || (count < 0 && c == 0) {
                break;
            }
            v.push(u16::from(c));
            i += 1;
        }
    }
    let n = v.len();
    v.resize(n + 8, 0);
    (v, n)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DrawTextExA(
    hdc: H,
    text: *mut u8,
    count: i32,
    rect: *mut i32,
    format: u32,
    params: *mut u32,
) -> i32 {
    // SAFETY: the caller's string.
    let (mut w, n) = unsafe { ansi_text(text, count) };
    // SAFETY: our buffer of `n` units and room for an ellipsis.
    let r = unsafe { DrawTextExW(hdc, w.as_mut_ptr(), n as i32, rect, format, params) };
    if format & DT_MODIFYSTRING != 0 && !text.is_null() {
        let len = w.iter().position(|&c| c == 0).unwrap_or(n);
        for (i, &c) in w.iter().take(len).enumerate() {
            // SAFETY: the caller's buffer, as large as the Learn page asks.
            unsafe { *text.add(i) = if c < 0x100 { c as u8 } else { b'?' } };
        }
        // SAFETY: as above.
        unsafe { *text.add(len) = 0 };
    }
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DrawTextA(hdc: H, text: *const u8, count: i32, rect: *mut i32, format: u32) -> i32 {
    // SAFETY: as `DrawTextExA`.
    unsafe { DrawTextExA(hdc, text as *mut u8, count, rect, format, core::ptr::null_mut()) }
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

/// Each thread's `GetMessageExtraInfo` value (GetMessageExtraInfo.yaml): what `SetMessageExtraInfo`
/// set, until a message the thread takes from its queue brings its own (0 for a posted one).
static EXTRA_INFO: SpinLock<Vec<(u32, isize)>> = SpinLock::new(Vec::new());

fn extra_info(set: Option<isize>) -> isize {
    let tid = SYS.tid();
    EXTRA_INFO.with(|v| {
        let i = v.iter().position(|e| e.0 == tid);
        let old = i.map_or(0, |i| v[i].1);
        if let Some(x) = set {
            match i {
                Some(i) => v[i].1 = x,
                None => v.push((tid, x)),
            }
        }
        old
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMessageExtraInfo() -> isize {
    extra_info(None)
}

#[unsafe(no_mangle)]
pub extern "system" fn SetMessageExtraInfo(value: isize) -> isize {
    extra_info(Some(value))
}

/// `CloseWindow`: minimizes the window (Learn).
#[unsafe(no_mangle)]
pub extern "system" fn CloseWindow(hwnd: H) -> i32 {
    if info(hwnd).is_none() {
        return bad_window();
    }
    u::show_window(&SYS, hwnd as u64, u::SW_MINIMIZE);
    1
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
    // The desktop and message-only parent windows belong to the system: a thread and a process that
    // are not ours (GetWindowThreadProcessId.yaml `other_process`).
    let (p, t) = if i.desktop || i.tid == 0 { (SYSTEM_PID, SYSTEM_TID) } else { (i.pid, i.tid) };
    if !pid.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { *pid = p };
    }
    t
}

/// The process and thread ids the desktop window answers.
const SYSTEM_PID: u32 = 0x1C4;
const SYSTEM_TID: u32 = 0x1CC;

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
/// hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret, rcCaret (72 bytes, 48 in a 32-bit
/// process).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetGUIThreadInfo(tid: u32, gti: *mut u8) -> i32 {
    if gti.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let s = if tid == 0 || tid != SYS.tid() { u::focus::thread_input(&SYS, tid) } else { u::focus::input(&SYS) };
    let caret = match call(Request::Caret { op: poly_win32k::caret::OP_QUERY, hwnd: 0, a: 0, b: 0, tid }) {
        Reply::Caret { hwnd, width, height, x, y, shown, .. } => {
            poly_win32k::caret::Snapshot { hwnd, width, height, x, y, shown }
        }
        _ => poly_win32k::caret::Snapshot::default(),
    };
    // SAFETY: the caller's GUITHREADINFO (cbSize first).
    unsafe {
        let size = (gti as *const u32).read_unaligned();
        let info = cstructs::GuiThreadInfo {
            size,
            active: s.active as usize,
            focus: s.focus as usize,
            capture: s.capture as usize,
            flags: u32::from(caret.shown),
            caret: caret.hwnd as usize,
            caret_rect: caret.rect(),
            ..Default::default()
        };
        (gti as *mut cstructs::GuiThreadInfo).write_unaligned(info);
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
        // GetCursorPos.yaml `null`.
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    let (x, y) = self::cursor(None);
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
    let mut ex = [0u8; core::mem::size_of::<cstructs::WndClassEx>()];
    // SAFETY: our WNDCLASSEXW; the caller's name.
    let r = unsafe { GetClassInfoExW(instance, name, ex.as_mut_ptr()) };
    if r != 0 {
        // WNDCLASSW is WNDCLASSEXW without cbSize and hIconSm.
        // SAFETY: our WNDCLASSEXW.
        let e = unsafe { (ex.as_ptr() as *const cstructs::WndClassEx).read_unaligned() };
        let c = cstructs::WndClass {
            style: e.style,
            wndproc: e.wndproc,
            cls_extra: e.cls_extra,
            wnd_extra: e.wnd_extra,
            instance: e.instance,
            icon: e.icon,
            cursor: e.cursor,
            brush: e.brush,
            menu_name: e.menu_name,
            class_name: e.class_name,
        };
        // SAFETY: the caller's WNDCLASSW.
        unsafe { (wc as *mut cstructs::WndClass).write_unaligned(c) };
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
        return icons::load_module_icon(instance, name, 2, 0, 0, 0x40 | 0x8000);
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
    if cursor != 0 && !icons::is_cursor(cursor) {
        set_last_error(ERROR_INVALID_CURSOR_HANDLE);
        return 0;
    }
    let prev = with_cursor(|c| core::mem::replace(c, cursor));
    show_cursor_shape(cursor);
    prev
}

/// Tell the compositor which system cursor to show over the window under
/// the pointer (created cursors and NULL keep the previous look).
fn show_cursor_shape(shape: H) {
    let Some(i) = cursor_index(shape) else { return };
    let (x, y) = self::cursor(None);
    if let Reply::Handle { hwnd } = call(Request::WindowFromPoint { x, y, below: 0 })
        && hwnd != 0
        && let Reply::Handle { hwnd: root } = call(Request::Ancestor { hwnd: u64::from(hwnd), flags: 2 })
        && root != 0
    {
        call(Request::CursorShape { hwnd: root, idc: CURSORS[i].0 });
    }
}

/// SetCapture.yaml: the previous capture window of the thread's input.
#[unsafe(no_mangle)]
pub extern "system" fn SetCapture(hwnd: H) -> H {
    u::focus::set_capture(&SYS, hwnd as u64) as H
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
    let t = unsafe { (tme as *const cstructs::TrackMouseEvent).read_unaligned() };
    let (size, flags, hwnd, hover) = (t.size, t.flags, t.hwnd as u64, t.hover);
    if size as usize != core::mem::size_of::<cstructs::TrackMouseEvent>() {
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
            let t = cstructs::TrackMouseEvent {
                size,
                flags: (v & 0xFFFF) as u32,
                hwnd: (v >> 32) as usize,
                hover: ((v >> 16) & 0xFFFF) as u32,
            };
            (tme as *mut cstructs::TrackMouseEvent).write_unaligned(t);
        }
        return 1;
    }
    if flags & !TME_CANCEL == 0 {
        return 1;
    }
    let Some(i) = info(hwnd as H) else { return bad_window() };
    const TME_NONCLIENT: u32 = 0x10;
    if flags & TME_NONCLIENT != 0 {
        u::input::track_nonclient(&SYS, i.hwnd, flags);
        return 1;
    }
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
    let r = i32::from(u::end_paint(&SYS, hwnd as u64, &p));
    // What the window painted may have covered the caret.
    caretdraw::painted(hwnd);
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn InvalidateRect(hwnd: H, rect: *const i32, erase: i32) -> i32 {
    // SAFETY: the caller's RECT, if any.
    let r = unsafe { opt_rect(rect) };
    i32::from(u::paint::invalidate(&SYS, hwnd as u64, r.as_ref().map(core::slice::from_ref), erase != 0))
}

#[unsafe(no_mangle)]
pub extern "system" fn InvalidateRgn(hwnd: H, rgn: H, erase: i32) -> i32 {
    i32::from(u::paint::invalidate_rgn(&SYS, hwnd as u64, rgn as u64, erase != 0))
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
    u::paint::get_update_rgn(&SYS, hwnd as u64, rgn as u64, erase != 0)
}

/// The entry point (`DllMain`): `DLL_PROCESS_ATTACH` connects to the window manager once. In a
/// process whose creation attributes disabled the win32k system calls (the storage service's and
/// the renderers' mitigation words, the `UpdateProcThreadAttribute` sheet) the connection is
/// refused and the DLL does not initialise: `LoadLibraryW("user32.dll")` fails with
/// `ERROR_DLL_INIT_FAILED` (1114).
#[unsafe(no_mangle)]
pub extern "system" fn user32_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        let bytes = wire::encode(&Request::Winsta { op: poly_win32k::winsta::WinstaOp::Connect {} });
        let mut out = [0u8; 256];
        let mut len = 0usize;
        // SAFETY: the buffers are ours and sized as passed.
        let s = unsafe { PolyUserCall(bytes.as_ptr(), bytes.len(), out.as_mut_ptr(), out.len(), &mut len) };
        // `STATUS_ACCESS_DENIED`: the lockdown. A named station and desktop the process may not
        // use: the connection is refused too.
        if s == 0xC000_0022 {
            return 0;
        }
        if s == 0
            && let Some(Reply::Winsta { res: poly_win32k::winsta::WinstaRes::Err { .. } }) =
                wire::decode::<Reply>(&out[..(len as usize).min(out.len())])
        {
            return 0;
        }
    }
    1
}

mod delayed;

#[cfg(target_arch = "x86_64")]
mod apps;
