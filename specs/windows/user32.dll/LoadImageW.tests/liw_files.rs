//! LoadImageW with LR_LOADFROMFILE, on files the test writes itself in its
//! scratch directory (a 16x16 32-bit .ico, a .cur with hotspot 3,4 and a
//! 2x2 32-bit .bmp): the native size is used for cx = cy = 0 without
//! LR_DEFAULTSIZE (16x16, hotspot of an icon = centre 8,8); LR_DEFAULTSIZE
//! scales to 32x32 (system icon size, hotspot 16,16); an explicit size
//! scales (32x32 hotspot 16,16 for an icon; 8x8 hotspot 4,4; a cursor's
//! hotspot scales with it: 3,4 -> 6,8 at 32x32); an .ico loaded as IMAGE_CURSOR
//! takes the directory entry's two 16-bit fields as the hotspot (1,32);
//! a .cur loaded as an icon gives an icon (hotspot centre); bitmaps load at
//! their size (2x2) or scaled (4x4) with 32 bits per pixel; a missing file
//! gives ERROR_FILE_NOT_FOUND (2); a bitmap file as an icon, and an icon file
//! as a bitmap, give NULL. The pixels of a loaded icon are drawn into a DIB:
//! the opaque left half red, the transparent right half leaves the
//! background.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HBRUSH = *mut c_void;
type HDC = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy)]
struct BITMAPINFOHEADER {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// wingdi.h (one RGBQUAD as a u32)
#[repr(C)]
struct BITMAPINFO {
    header: BITMAPINFOHEADER,
    colors: [u32; 1],
}

/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
}

const GWLP_USERDATA: i32 = -21;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
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

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
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

/// "window" or "null" (a handle is never printed).
fn hwnd_kind(h: HWND) -> &'static str {
    if h.is_null() { "null" } else { "window" }
}

/// A memory DC with a 32-bit top-down (or bottom-up) DIB section selected:
/// (dc, bitmap, pixels). Pixels are 0x00RRGGBB, row by row from the top
/// for top-down.
fn dib32(w: i32, h: i32, top_down: bool) -> (HDC, HBITMAP, *mut u32) {
    let info = BITMAPINFO {
        header: BITMAPINFOHEADER {
            size: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            width: w,
            height: if top_down { -h } else { h },
            planes: 1,
            bit_count: 32,
            compression: BI_RGB,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let mut bits: *mut c_void = core::ptr::null_mut();
    // SAFETY: a filled BITMAPINFO; bits receives the pixel pointer.
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
        SelectObject(dc, bmp);
        (dc, bmp, bits as *mut u32)
    }
}

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// Set every pixel of a w x h 32-bit DIB to v.
fn fill_bits(bits: *mut u32, w: i32, h: i32, v: u32) {
    for i in 0..(w * h) as usize {
        // SAFETY: inside the DIB.
        unsafe { *bits.add(i) = v };
    }
}

/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONINFO {
    f_icon: BOOL,
    x_hotspot: u32,
    y_hotspot: u32,
    mask: HBITMAP,
    color: HBITMAP,
}

type HICON = *mut c_void;
/// winuser.h: LoadImage types and flags.
const IMAGE_BITMAP: u32 = 0;

const IMAGE_ICON: u32 = 1;
const IMAGE_CURSOR: u32 = 2;
const LR_MONOCHROME: u32 = 0x0001;
const LR_LOADFROMFILE: u32 = 0x0010;
const LR_DEFAULTSIZE: u32 = 0x0040;
const LR_CREATEDIBSECTION: u32 = 0x2000;
const LR_SHARED: u32 = 0x8000;
const DI_NORMAL: u32 = 0x0003;
/// winbase.h / winnt.h
const CREATE_ALWAYS: u32 = 2;

const FILE_ATTRIBUTE_NORMAL_L: u32 = 0x80;
const INVALID_HANDLE_VALUE_L: usize = usize::MAX;
/// Width, height, planes, bits per pixel of a bitmap (0s for a non-bitmap).
fn ly_bm(h: HBITMAP) -> (i32, i32, u16, u16) {
    let mut b = BITMAP::default();
    // SAFETY: b is writable.
    let n = unsafe { GetObjectW(h, core::mem::size_of::<BITMAP>() as i32, &mut b as *mut BITMAP as *mut c_void) };
    if n == 0 { (0, 0, 0, 0) } else { (b.width, b.height, b.planes, b.bits_pixel) }
}

/// One line about an icon or cursor: GetIconInfo's fIcon, hotspot, and the
/// sizes of its bitmaps; the bitmaps are deleted. Returns false when
/// GetIconInfo failed (the line then has the last error).
fn ly_info(t: &mut Test, label: &str, h: HICON) -> bool {
    let mut ii = ICONINFO {
        f_icon: 7,
        x_hotspot: 0xAAAA,
        y_hotspot: 0xBBBB,
        mask: core::ptr::null_mut(),
        color: core::ptr::null_mut(),
    };
    set_error(0x1234);
    // SAFETY: ii is writable; the bitmaps it returns are deleted.
    unsafe {
        if GetIconInfo(h, &mut ii) == 0 {
            say!(t, "{label}: GetIconInfo FALSE error {}", last_error());
            return false;
        }
        let m = ly_bm(ii.mask);
        let c = ly_bm(ii.color);
        say!(
            t,
            "{label}: fIcon {} hotspot {},{} mask {}x{} planes {} bits {} ; color {}x{} planes {} bits {}{}",
            ii.f_icon,
            ii.x_hotspot,
            ii.y_hotspot,
            m.0,
            m.1,
            m.2,
            m.3,
            c.0,
            c.1,
            c.2,
            c.3,
            if ii.color.is_null() { " (no color bitmap)" } else { "" }
        );
        DeleteObject(ii.mask);
        if !ii.color.is_null() {
            DeleteObject(ii.color);
        }
    }
    true
}

/// Draw `h` with DrawIconEx (flags) into a w x h 32-bit DIB filled with
/// magenta and count the pixels: (background, red, green, other).
fn ly_draw(h: HICON, w: i32, hh: i32, flags: u32) -> (i32, i32, i32, i32) {
    let (dc, bmp, bits) = dib32(w, hh, true);
    fill_bits(bits, w, hh, 0x00FF00FF);
    // SAFETY: a DIB DC of our own, deleted below.
    unsafe { DrawIconEx(dc, 0, 0, h, w, hh, 0, core::ptr::null_mut(), flags) };
    let (mut bg, mut red, mut green, mut other) = (0, 0, 0, 0);
    for y in 0..hh {
        for x in 0..w {
            match px(bits, w, x, y) & 0xFFFFFF {
                0xFF00FF => bg += 1,
                0xFF0000 => red += 1,
                0x00FF00 => green += 1,
                _ => other += 1,
            }
        }
    }
    // SAFETY: our DC and bitmap.
    unsafe {
        DeleteDC(dc);
        DeleteObject(bmp);
    }
    (bg, red, green, other)
}

/// A 16x16 32-bit .ico (or .cur with hotspot 3,4) image file's bytes: the
/// left half opaque red, the right half transparent; returns the length.
fn ly_ico16(cursor: bool, out: &mut [u8; 1200]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8]| {
        for &x in b {
            out[n] = x;
            n += 1;
        }
    };
    put(&[0, 0, if cursor { 2 } else { 1 }, 0, 1, 0]);
    put(&[16, 16, 0, 0]);
    if cursor {
        put(&[3, 0, 4, 0]);
    } else {
        put(&[1, 0, 32, 0]);
    }
    put(&(40u32 + 1024 + 64).to_le_bytes());
    put(&22u32.to_le_bytes());
    put(&40u32.to_le_bytes());
    put(&16i32.to_le_bytes());
    put(&32i32.to_le_bytes());
    put(&[1, 0, 32, 0]);
    put(&0u32.to_le_bytes());
    put(&(1024u32 + 64).to_le_bytes());
    put(&[0; 16]);
    for _y in 0..16 {
        for x in 0..16 {
            if x < 8 {
                put(&[0, 0, 255, 255]);
            } else {
                put(&[0, 0, 0, 0]);
            }
        }
    }
    for _y in 0..16 {
        put(&[0x00, 0xFF, 0, 0]);
    }
    n
}

/// A 2x2 32-bit .bmp file's bytes; returns the length.
fn ly_bmp2(out: &mut [u8; 100]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8]| {
        for &x in b {
            out[n] = x;
            n += 1;
        }
    };
    put(b"BM");
    put(&(14u32 + 40 + 16).to_le_bytes());
    put(&0u32.to_le_bytes());
    put(&54u32.to_le_bytes());
    put(&40u32.to_le_bytes());
    put(&2i32.to_le_bytes());
    put(&2i32.to_le_bytes());
    put(&[1, 0, 32, 0]);
    put(&0u32.to_le_bytes());
    put(&16u32.to_le_bytes());
    put(&[0; 16]);
    for _ in 0..4 {
        put(&[0, 0, 255, 0]);
    }
    n
}

/// Write a file in the current directory (the scratch directory); true when written.
fn ly_write_file(name: &[u16], data: &[u8]) -> bool {
    // SAFETY: plain file creation in the scratch directory.
    unsafe {
        let f = CreateFileW(
            name.as_ptr(),
            0x4000_0000,
            0,
            core::ptr::null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL_L,
            core::ptr::null_mut(),
        );
        if f as usize == INVALID_HANDLE_VALUE_L {
            return false;
        }
        let mut w = 0u32;
        let ok = WriteFile(f, data.as_ptr() as *const c_void, data.len() as u32, &mut w, core::ptr::null_mut());
        CloseHandle(f);
        ok != 0 && w as usize == data.len()
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const c_void,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn WriteFile(file: HANDLE, buf: *const c_void, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn LoadImageW(inst: HINSTANCE, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> HANDLE;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(icon: HICON) -> BOOL;
    fn DrawIconEx(dc: HDC, x: i32, y: i32, icon: HICON, cx: i32, cy: i32, step: u32, brush: HBRUSH, flags: u32)
    -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    fn CreateCompatibleDC(hdc: HDC) -> HDC;
    fn DeleteDC(hdc: HDC) -> BOOL;
    fn CreateDIBSection(
        hdc: HDC,
        info: *const BITMAPINFO,
        usage: u32,
        bits: *mut *mut c_void,
        section: HANDLE,
        offset: u32,
    ) -> HBITMAP;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: files in the scratch directory; handles destroyed.
    unsafe {
        let null = core::ptr::null_mut();
        let mut ib = [0u8; 1200];
        let n = ly_ico16(false, &mut ib);
        let mut cb = [0u8; 1200];
        let nc = ly_ico16(true, &mut cb);
        let mut bb = [0u8; 100];
        let nb = ly_bmp2(&mut bb);
        let w1 = ly_write_file(wz!("ly_t.ico"), &ib[..n]);
        let w2 = ly_write_file(wz!("ly_t.cur"), &cb[..nc]);
        let w3 = ly_write_file(wz!("ly_t.bmp"), &bb[..nb]);
        say!(t, "wrote ico {} cur {} bmp {}", w1, w2, w3);
        for (lbl, name, kind, cx, cy, fl) in [
            ("ico as icon, 0x0", wz!("ly_t.ico"), IMAGE_ICON, 0, 0, 0),
            ("ico as icon, 0x0 LR_DEFAULTSIZE", wz!("ly_t.ico"), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE),
            ("ico as icon, 32x32", wz!("ly_t.ico"), IMAGE_ICON, 32, 32, 0),
            ("ico as icon, 8x8", wz!("ly_t.ico"), IMAGE_ICON, 8, 8, 0),
            ("ico as icon, 16x16", wz!("ly_t.ico"), IMAGE_ICON, 16, 16, 0),
            ("ico as cursor", wz!("ly_t.ico"), IMAGE_CURSOR, 0, 0, 0),
            ("cur as cursor, 0x0", wz!("ly_t.cur"), IMAGE_CURSOR, 0, 0, 0),
            ("cur as cursor, 32x32", wz!("ly_t.cur"), IMAGE_CURSOR, 32, 32, 0),
            ("cur as cursor, 0x0 LR_DEFAULTSIZE", wz!("ly_t.cur"), IMAGE_CURSOR, 0, 0, LR_DEFAULTSIZE),
            ("cur as icon", wz!("ly_t.cur"), IMAGE_ICON, 0, 0, 0),
        ] {
            set_error(0x1234);
            let h = LoadImageW(null, name.as_ptr(), kind, cx, cy, fl | LR_LOADFROMFILE);
            say!(t, "{lbl}: {}", hwnd_kind(h));
            if !h.is_null() {
                ly_info(t, "   shape", h);
                if cx == 0 && cy == 0 && fl == 0 || cx == 16 {
                    let d = ly_draw(h, 16, 16, DI_NORMAL);
                    say!(t, "   drawn 16x16: background {} red {} green {} other {}", d.0, d.1, d.2, d.3);
                }
                DestroyIcon(h);
            }
        }
        for (lbl, cx, cy, fl) in [
            ("bmp 0x0", 0, 0, 0),
            ("bmp 4x4", 4, 4, 0),
            ("bmp 0x0 LR_CREATEDIBSECTION", 0, 0, LR_CREATEDIBSECTION),
            ("bmp 0x0 LR_MONOCHROME", 0, 0, LR_MONOCHROME),
        ] {
            set_error(0x1234);
            let h = LoadImageW(null, wz!("ly_t.bmp").as_ptr(), IMAGE_BITMAP, cx, cy, fl | LR_LOADFROMFILE);
            if h.is_null() {
                say!(t, "{lbl}: NULL error {}", last_error());
            } else {
                let b = ly_bm(h as HBITMAP);
                say!(
                    t,
                    "{lbl}: bitmap {}x{} planes {} bits {} object type {}",
                    b.0,
                    b.1,
                    b.2,
                    b.3,
                    GetObjectType(h as HGDIOBJ)
                );
                DeleteObject(h as HGDIOBJ);
            }
        }
        set_error(0x1234);
        let h = LoadImageW(null, wz!("ly_t.bmp").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE);
        say!(t, "bmp as icon: {} error {:#x}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, wz!("ly_t.ico").as_ptr(), IMAGE_BITMAP, 0, 0, LR_LOADFROMFILE);
        say!(t, "ico as bitmap: {} error {:#x}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, wz!("ly_none.ico").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE);
        say!(t, "missing file: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, wz!("ly_none.bmp").as_ptr(), IMAGE_BITMAP, 0, 0, LR_LOADFROMFILE);
        say!(t, "missing bitmap file: {} error {}", hwnd_kind(h), last_error());
        set_error(0x1234);
        let h = LoadImageW(null, wz!("ly_none.cur").as_ptr(), IMAGE_CURSOR, 0, 0, LR_LOADFROMFILE | LR_SHARED);
        say!(t, "missing cursor file with LR_SHARED: {} error {}", hwnd_kind(h), last_error());
        // a shared file load returns the same handle twice
        let a = LoadImageW(null, wz!("ly_t.ico").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE | LR_SHARED);
        let b = LoadImageW(null, wz!("ly_t.ico").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE | LR_SHARED);
        say!(t, "LR_LOADFROMFILE | LR_SHARED twice: {} {} same {}", hwnd_kind(a), hwnd_kind(b), a == b);
        let c = LoadImageW(null, wz!("ly_t.ico").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE);
        let d = LoadImageW(null, wz!("ly_t.ico").as_ptr(), IMAGE_ICON, 0, 0, LR_LOADFROMFILE);
        say!(t, "LR_LOADFROMFILE twice: same {}", c == d);
        for h in [c, d] {
            if !h.is_null() {
                DestroyIcon(h);
            }
        }
        DeleteFileW(wz!("ly_t.ico").as_ptr());
        DeleteFileW(wz!("ly_t.cur").as_ptr());
        DeleteFileW(wz!("ly_t.bmp").as_ptr());
    }
}
