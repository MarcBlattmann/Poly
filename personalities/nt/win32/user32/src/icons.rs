//! Icons, cursors and images (ADR 0172): the objects `CreateIconIndirect`,
//! `CreateCursor`, `CopyIcon`, `LoadImageW` and `LoadIconW` make, their
//! lifetime (`DestroyIcon`, `DestroyCursor`), `GetIconInfo`,
//! `GetCursorInfo`, `ShowCursor` and `DrawIconEx`. An icon is kept as its
//! pixels (an AND mask and BGRA colour); `GetIconInfo` makes new GDI
//! bitmaps of them each call. The system cursors keep slice B's handles
//! (`CURSOR_BASE + index`); the seven system icons are
//! `ICON_BASE + index`, shared and never destroyed.

use super::{CURSORS, H, SpinLock, call, cursor_index, set_last_error, wide, with_cursor};
use alloc::vec::Vec;
use poly_win32k::icofile::{self, Image};
use poly_win32k::proto::{Reply, Request};
use poly_win32k::user::Env;

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> usize;
    fn GetObjectType(h: usize) -> u32;
    fn GetPixel(hdc: usize, x: i32, y: i32) -> u32;
    fn SetPixelV(hdc: usize, x: i32, y: i32, color: u32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const u8,
        disposition: u32,
        flags: u32,
        template: usize,
    ) -> usize;
    fn GetFileSizeEx(file: usize, size: *mut i64) -> i32;
    fn ReadFile(file: usize, buf: *mut u8, count: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn CloseHandle(h: usize) -> i32;
}

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOACCESS: u32 = 998;
const ERROR_INVALID_CURSOR_HANDLE: u32 = 1402;
const ERROR_RESOURCE_DATA_NOT_FOUND: u32 = 1812;
const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
const ERROR_RESOURCE_NAME_NOT_FOUND: u32 = 1814;

const OBJ_BRUSH: u32 = 2;
const OBJ_BITMAP: u32 = 7;
const OBJ_REGION: u32 = 8;

const LR_MONOCHROME: u32 = 0x1;
const LR_LOADFROMFILE: u32 = 0x10;
const LR_DEFAULTSIZE: u32 = 0x40;
const LR_SHARED: u32 = 0x8000;

const ICON_BASE: usize = 0x0007_0000;
const SYSTEM_ICONS: usize = 7;
/// Handles of created icons and cursors.
const OBJ_START: usize = 0x0008_0000;

#[derive(Clone)]
struct Obj {
    h: usize,
    cursor: bool,
    img: Image,
    /// A monochrome icon or cursor: `and` holds the AND mask and the XOR
    /// mask below it (twice the height); no colour.
    mono: bool,
    xor: Vec<u8>,
    destroyed: bool,
    /// The rows of the mask a mono icon reports (twice the height, or one more).
    mask_rows: i32,
}

struct Table {
    objs: Vec<Obj>,
    next: usize,
    /// The count of `ShowCursor`, per thread.
    show: Vec<(u32, i32)>,
}

static TABLE: SpinLock<Table> = SpinLock::new(Table { objs: Vec::new(), next: OBJ_START, show: Vec::new() });

fn blank(w: i32, h: i32, cursor: bool, hot: (i32, i32)) -> Image {
    let _ = cursor;
    Image { w, h, bgra: alloc::vec![0; (w * h * 4) as usize], and: alloc::vec![0; (w * h) as usize], alpha: false, hot }
}

fn is_system_icon(h: usize) -> bool {
    h >= ICON_BASE && h < ICON_BASE + SYSTEM_ICONS
}

/// A system icon: 32x32 with a filled disc (the artwork is Poly's).
fn system_icon(index: usize) -> Image {
    let mut img = blank(32, 32, false, (16, 16));
    // A disc of radius 14 whose colour depends on the icon.
    let colors: [[u8; 3]; 7] =
        [[200, 120, 40], [30, 30, 200], [200, 160, 30], [30, 200, 220], [200, 80, 80], [160, 60, 160], [60, 160, 60]];
    for y in 0..32i32 {
        for x in 0..32i32 {
            let (dx, dy) = (x - 16, y - 16);
            let inside = dx * dx + dy * dy <= 14 * 14;
            let at = (y * 32 + x) as usize;
            if inside {
                let c = colors[index % 7];
                img.bgra[at * 4..at * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            } else {
                img.and[at] = 1;
            }
        }
    }
    img.alpha = true;
    img
}

fn system_cursor(index: usize) -> (Image, bool) {
    let (_, hx, hy, colour) = CURSORS[index];
    if colour {
        (blank(32, 32, true, (hx as i32, hy as i32)), false)
    } else {
        (blank(32, 32, true, (hx as i32, hy as i32)), true)
    }
}

/// Run `f` on the object behind a handle (system ones are made on demand);
/// the table is not locked while it runs.
fn with_obj<R>(h: usize, f: impl FnOnce(&Obj) -> R) -> Option<R> {
    if let Some(i) = cursor_index(h) {
        let (img, mono) = system_cursor(i);
        let o = Obj {
            h,
            cursor: true,
            img,
            mono,
            xor: Vec::new(),
            destroyed: false,
            mask_rows: if mono { 64 } else { 32 },
        };
        return Some(f(&o));
    }
    if is_system_icon(h) {
        let o = Obj {
            h,
            cursor: false,
            img: system_icon(h - ICON_BASE),
            mono: false,
            xor: Vec::new(),
            destroyed: false,
            mask_rows: 32,
        };
        return Some(f(&o));
    }
    let o = TABLE.with(|t| t.objs.iter().find(|o| o.h == h && !o.destroyed).cloned())?;
    Some(f(&o))
}

fn add(o: Obj) -> usize {
    TABLE.with(|t| {
        let h = t.next;
        t.next += 4;
        t.objs.push(Obj { h, ..o });
        h
    })
}

fn bad_handle<T: Default>() -> T {
    set_last_error(ERROR_INVALID_CURSOR_HANDLE);
    T::default()
}

// ----- bitmaps to pixels

/// The pixels of a GDI bitmap (`B, G, R, A` rows, top-down) and its size.
fn bitmap_bgra(hbm: usize) -> Option<(i32, i32, Vec<u8>)> {
    super::clipboard::bitmap_pixels(hbm)
}

// ----- creating

/// `ICONINFO`: fIcon (u32 at 0), xHotspot (4), yHotspot (8), hbmMask (16), hbmColor (24).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateIconIndirect(ii: *const u8) -> usize {
    if ii.is_null() {
        return 0;
    }
    // SAFETY: the caller's ICONINFO.
    let (f_icon, hx, hy, mask, color) = unsafe {
        (
            core::ptr::read_unaligned(ii as *const u32),
            core::ptr::read_unaligned(ii.add(4) as *const i32),
            core::ptr::read_unaligned(ii.add(8) as *const i32),
            core::ptr::read_unaligned(ii.add(16) as *const usize),
            core::ptr::read_unaligned(ii.add(24) as *const usize),
        )
    };
    // SAFETY: gdi32 checks handles.
    let (mt, ct) = unsafe { (GetObjectType(mask), if color != 0 { GetObjectType(color) } else { OBJ_BITMAP }) };
    if mt == OBJ_BRUSH {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if mt == OBJ_REGION {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    if mt != OBJ_BITMAP || ct != OBJ_BITMAP {
        return 0;
    }
    let Some((mw, mh, mpx)) = bitmap_bgra(mask) else { return 0 };
    let cursor = f_icon == 0;
    let bit = |p: &[u8]| u8::from(p[0] != 0 || p[1] != 0 || p[2] != 0);
    if color != 0 {
        let Some((cw, ch, cpx)) = bitmap_bgra(color) else { return 0 };
        if (cw, ch) != (mw, mh) {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        let and: Vec<u8> = mpx.chunks_exact(4).map(bit).collect();
        let alpha = cpx.chunks_exact(4).any(|p| p[3] != 0);
        let img = Image { w: cw, h: ch, bgra: cpx, and, alpha, hot: if cursor { (hx, hy) } else { (cw / 2, ch / 2) } };
        return add(Obj { h: 0, cursor, img, mono: false, xor: Vec::new(), destroyed: false, mask_rows: 0 });
    }
    // A mask alone: AND on top, XOR below.
    let ih = mh / 2;
    if ih == 0 {
        // A 1x1 mask has no halves; keep it as a one-pixel icon.
        let img = Image {
            w: mw,
            h: 1,
            bgra: alloc::vec![0; (mw * 4) as usize],
            and: alloc::vec![0; mw as usize],
            alpha: false,
            hot: (mw / 2, 0),
        };
        return add(Obj {
            h: 0,
            cursor,
            img,
            mono: true,
            xor: alloc::vec![0; mw as usize],
            destroyed: false,
            mask_rows: mh,
        });
    }
    let bits: Vec<u8> = mpx.chunks_exact(4).map(bit).collect();
    let n = (mw * ih) as usize;
    let img = Image {
        w: mw,
        h: ih,
        bgra: alloc::vec![0; n * 4],
        and: bits[..n].to_vec(),
        alpha: false,
        hot: if cursor { (hx, hy) } else { (mw / 2, ih / 2) },
    };
    add(Obj { h: 0, cursor, img, mono: true, xor: bits[n..2 * n].to_vec(), destroyed: false, mask_rows: mh })
}

/// `CreateCursor`: 1-bit AND and XOR arrays, rows of whole bytes
/// (`CreateCursor.yaml`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateCursor(
    _instance: H,
    hot_x: i32,
    hot_y: i32,
    w: i32,
    h: i32,
    and: *const u8,
    xor: *const u8,
) -> usize {
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 || hot_x < 0 || hot_y < 0 || hot_x > w || hot_y > h {
        return 0;
    }
    let stride = (w as usize).div_ceil(8);
    let read = |p: *const u8| -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            for x in 0..w as usize {
                // SAFETY: `h` rows of `stride` bytes from the caller.
                let b = unsafe { *p.add(y * stride + x / 8) };
                v.push((b >> (7 - x % 8)) & 1);
            }
        }
        v
    };
    let (a, x) = (read(and), read(xor));
    let img = Image { w, h, bgra: alloc::vec![0; (w * h * 4) as usize], and: a, alpha: false, hot: (hot_x, hot_y) };
    add(Obj { h: 0, cursor: true, img, mono: true, xor: x, destroyed: false, mask_rows: 2 * h })
}

fn copy_of(o: &Obj, w: i32, h: i32) -> usize {
    let img = if (w, h) == (o.img.w, o.img.h) || w <= 0 || h <= 0 {
        o.img.clone()
    } else if o.cursor {
        icofile::scale(&o.img, w, h)
    } else {
        let mut s = icofile::scale(&o.img, w, h);
        // An icon's hotspot is its centre.
        s.hot = (w / 2, h / 2);
        s
    };
    let xor = if o.mono && !o.xor.is_empty() {
        if (img.w, img.h) == (o.img.w, o.img.h) {
            o.xor.clone()
        } else {
            let tmp = Image {
                w: o.img.w,
                h: o.img.h,
                bgra: alloc::vec![0; o.xor.len() * 4],
                and: o.xor.clone(),
                alpha: false,
                hot: (0, 0),
            };
            icofile::scale(&tmp, img.w, img.h).and
        }
    } else {
        Vec::new()
    };
    let mask_rows = if (img.w, img.h) == (o.img.w, o.img.h) { o.mask_rows } else { 2 * img.h };
    add(Obj { h: 0, cursor: o.cursor, img, mono: o.mono, xor, destroyed: false, mask_rows })
}

#[unsafe(no_mangle)]
pub extern "system" fn CopyIcon(icon: usize) -> usize {
    with_obj(icon, |o| copy_of(o, 0, 0)).unwrap_or_else(bad_handle)
}

#[unsafe(no_mangle)]
pub extern "system" fn CopyImage(h: usize, kind: u32, cx: i32, cy: i32, _flags: u32) -> usize {
    match kind {
        1 | 2 => with_obj(h, |o| copy_of(o, cx, cy)).unwrap_or_else(bad_handle),
        _ => 0,
    }
}

// ----- destroying

#[unsafe(no_mangle)]
pub extern "system" fn DestroyIcon(icon: usize) -> i32 {
    destroy(icon, false)
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyCursor(cursor: usize) -> i32 {
    destroy(cursor, true)
}

fn destroy(h: usize, cursor_call: bool) -> i32 {
    if cursor_index(h).is_some() || is_system_icon(h) {
        return 1;
    }
    let found = TABLE.with(|t| {
        t.objs.iter_mut().find(|o| o.h == h && !o.destroyed).map(|o| {
            o.destroyed = true;
        })
    });
    if found.is_none() {
        set_last_error(ERROR_INVALID_CURSOR_HANDLE);
        return 0;
    }
    // The cursor set with SetCursor: destroyed, yet FALSE without an error.
    if cursor_call && with_cursor(|c| *c) == h {
        return 0;
    }
    1
}

/// Whether `h` is a cursor SetCursor may take (system or created).
pub fn is_cursor(h: usize) -> bool {
    cursor_index(h).is_some() || TABLE.with(|t| t.objs.iter().any(|o| o.h == h && o.cursor && !o.destroyed))
}

// ----- GetIconInfo, GetCursorInfo, ShowCursor

fn mask_bits(o: &Obj) -> Vec<u8> {
    // 1 bit per pixel rows padded to words; 1 = white.
    let (w, h) = (o.img.w as usize, o.img.h as usize);
    let rows = if o.mono { (o.mask_rows as usize).max(2 * h) } else { h };
    let stride = w.div_ceil(16) * 2;
    let mut out = alloc::vec![0u8; stride * rows];
    for y in 0..rows {
        for x in 0..w {
            let bit = if y < h {
                o.img.and.get(y * w + x).copied().unwrap_or(0)
            } else {
                o.xor.get((y - h) * w + x).copied().unwrap_or(0)
            };
            if bit != 0 {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetIconInfo(icon: usize, ii: *mut u8) -> i32 {
    let made = with_obj(icon, |o| {
        let (w, h) = (o.img.w, o.img.h);
        let mask_rows = if o.mono { o.mask_rows.max(2 * h) } else { h };
        let bits = mask_bits(o);
        // SAFETY: gdi32 copies the bits.
        let (m, c) = unsafe {
            (
                CreateBitmap(w, mask_rows, 1, 1, bits.as_ptr()),
                if o.mono { 0 } else { CreateBitmap(w, h, 1, 32, o.img.bgra.as_ptr()) },
            )
        };
        (o.cursor, o.img.hot, m, c)
    });
    let Some((cursor, hot, mask, color)) = made else {
        set_last_error(ERROR_INVALID_CURSOR_HANDLE);
        return 0;
    };
    if ii.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    // SAFETY: the caller's ICONINFO (32 bytes on x64).
    unsafe {
        core::ptr::write_unaligned(ii as *mut u32, u32::from(!cursor));
        core::ptr::write_unaligned(ii.add(4) as *mut i32, hot.0);
        core::ptr::write_unaligned(ii.add(8) as *mut i32, hot.1);
        core::ptr::write_unaligned(ii.add(16) as *mut u64, mask as u64);
        core::ptr::write_unaligned(ii.add(24) as *mut u64, color as u64);
    }
    1
}

fn show_count<R>(f: impl FnOnce(&mut i32) -> R) -> R {
    let tid = super::SYS.tid();
    TABLE.with(|t| {
        let i = match t.show.iter().position(|e| e.0 == tid) {
            Some(i) => i,
            None => {
                t.show.push((tid, 0));
                t.show.len() - 1
            }
        };
        f(&mut t.show[i].1)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowCursor(show: i32) -> i32 {
    show_count(|c| {
        *c += if show != 0 { 1 } else { -1 };
        *c
    })
}

/// `CURSORINFO`: cbSize (24), flags, hCursor, ptScreenPos.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCursorInfo(ci: *mut u8) -> i32 {
    if ci.is_null() {
        set_last_error(ERROR_NOACCESS);
        return 0;
    }
    // SAFETY: cbSize is the first field.
    if unsafe { core::ptr::read_unaligned(ci as *const u32) } != 24 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let (x, y) = match call(Request::Cursor { set: false, x: 0, y: 0 }) {
        Reply::Value { value } => (value as u32 as i32, (value >> 32) as u32 as i32),
        _ => (0, 0),
    };
    // The cursor of the thread owning the window under it.
    let mine = match call(Request::WindowFromPoint { x, y, below: 0 }) {
        Reply::Handle { hwnd } if hwnd != 0 => {
            super::info(hwnd as usize).is_some_and(|i| !i.desktop && i.tid == super::SYS.tid())
        }
        _ => false,
    };
    let cur = with_cursor(|c| *c);
    let showing = mine && show_count(|c| *c) >= 0 && cur != 0;
    // SAFETY: the caller's CURSORINFO (24 bytes).
    unsafe {
        core::ptr::write_unaligned(ci as *mut u32, 0);
        core::ptr::write_unaligned(ci.add(4) as *mut u32, u32::from(showing));
        core::ptr::write_unaligned(ci.add(8) as *mut u64, if showing { cur as u64 } else { 0 });
        core::ptr::write_unaligned(ci.add(16) as *mut i32, x);
        core::ptr::write_unaligned(ci.add(20) as *mut i32, y);
    }
    1
}

// ----- LoadIconW, LoadImageW

fn resource_id(name: *const u16) -> Option<u16> {
    if (name as usize) >> 16 == 0 {
        return Some(name as usize as u16);
    }
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
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadIconW(instance: H, name: *const u16) -> usize {
    if instance != 0 {
        set_last_error(ERROR_RESOURCE_DATA_NOT_FOUND);
        return 0;
    }
    match resource_id(name) {
        Some(id @ 32512..=32518) => ICON_BASE + usize::from(id - 32512),
        _ => {
            set_last_error(ERROR_RESOURCE_TYPE_NOT_FOUND);
            0
        }
    }
}

/// The bytes of a file, if it can be read.
fn read_file(name: *const u16) -> Option<Vec<u8>> {
    if name.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated name; kernel32 calls on our own handle.
    unsafe {
        let f = CreateFileW(name, 0x8000_0000, 7, core::ptr::null(), 3, 0x80, 0);
        if f == usize::MAX || f == 0 {
            return None;
        }
        let mut size = 0i64;
        let mut out = Vec::new();
        if GetFileSizeEx(f, &mut size) != 0 && size > 0 && size < (64 << 20) {
            out = alloc::vec![0u8; size as usize];
            let mut got = 0u32;
            if ReadFile(f, out.as_mut_ptr(), size as u32, &mut got, core::ptr::null_mut()) == 0 {
                out.clear();
            } else {
                out.truncate(got as usize);
            }
        }
        CloseHandle(f);
        Some(out)
    }
}

fn obj_from_image(img: Image, cursor: bool, icon_center: bool) -> usize {
    let mut img = img;
    if icon_center {
        img.hot = (img.w / 2, img.h / 2);
    }
    add(Obj { h: 0, cursor, img, mono: false, xor: Vec::new(), destroyed: false, mask_rows: 0 })
}

fn bitmap_from_bgra(w: i32, h: i32, px: &[u8], mono: bool) -> usize {
    if !mono {
        // SAFETY: gdi32 copies w*h*4 bytes.
        return unsafe { CreateBitmap(w, h, 1, 32, px.as_ptr()) };
    }
    let stride = (w as usize).div_ceil(16) * 2;
    let mut bits = alloc::vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = &px[(y * w as usize + x) * 4..];
            let lum = (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3;
            if lum >= 128 {
                bits[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    // SAFETY: gdi32 copies the bits.
    unsafe { CreateBitmap(w, h, 1, 1, bits.as_ptr()) }
}

/// System bitmaps (OBM_*): id, width, height, bits per pixel.
const SYSTEM_BITMAPS: [(u16, i32, i32, u32); 11] = [
    (32754, 42, 23, 32), // OBM_CLOSE
    (32753, 17, 17, 32), // OBM_UPARROW
    (32752, 17, 17, 32), // OBM_DNARROW
    (32751, 17, 17, 32), // OBM_RGARROW
    (32750, 17, 17, 32), // OBM_LFARROW
    (32749, 17, 19, 32), // OBM_REDUCE
    (32748, 20, 18, 32), // OBM_ZOOM
    (32747, 17, 19, 32), // OBM_RESTORE
    (32760, 15, 15, 1),  // OBM_CHECK
    (32766, 17, 17, 32), // OBM_SIZE
    (32758, 13, 13, 32), // OBM_BTNCORNERS
];

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadImageW(
    instance: H,
    name: *const u16,
    kind: u32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> usize {
    if kind > 2 {
        return 0;
    }
    if flags & LR_LOADFROMFILE != 0 {
        return load_file(name, kind, cx, cy, flags);
    }
    if instance != 0 {
        set_last_error(ERROR_RESOURCE_DATA_NOT_FOUND);
        return 0;
    }
    if name.is_null() {
        set_last_error(ERROR_RESOURCE_TYPE_NOT_FOUND);
        return 0;
    }
    let id = resource_id(name);
    match kind {
        1 => match id {
            Some(id @ 32512..=32518) if flags & LR_SHARED != 0 => ICON_BASE + usize::from(id - 32512),
            _ => {
                set_last_error(ERROR_RESOURCE_TYPE_NOT_FOUND);
                0
            }
        },
        2 => match id.and_then(|id| CURSORS.iter().position(|c| c.0 == id)) {
            Some(i) if flags & LR_SHARED != 0 => super::CURSOR_BASE + i,
            _ => {
                set_last_error(ERROR_RESOURCE_NAME_NOT_FOUND);
                0
            }
        },
        _ => match id.and_then(|id| SYSTEM_BITMAPS.iter().find(|b| b.0 == id)) {
            Some(&(_, w, h, bpp)) => {
                // SAFETY: a blank bitmap.
                unsafe { CreateBitmap(w, h, 1, bpp, core::ptr::null()) }
            }
            None => {
                set_last_error(ERROR_RESOURCE_NAME_NOT_FOUND);
                0
            }
        },
    }
}

fn load_file(name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> usize {
    // SAFETY: a NUL-terminated name or NULL.
    let empty = name.is_null() || unsafe { *name } == 0;
    let bytes = if empty { None } else { read_file(name) };
    let Some(bytes) = bytes else {
        set_last_error(ERROR_FILE_NOT_FOUND);
        return 0;
    };
    let (w, h) = if flags & LR_DEFAULTSIZE != 0 && cx == 0 { (32, 32) } else { (cx, cy) };
    if kind == 0 {
        // A BMP file: BITMAPFILEHEADER, then the DIB.
        if bytes.len() < 14 || &bytes[..2] != b"BM" {
            return 0;
        }
        let Some((bw, bh, px)) = poly_win32k::dib::to_bgra(&bytes[14..]) else { return 0 };
        let src = Image { w: bw, h: bh, bgra: px, and: alloc::vec![0; (bw * bh) as usize], alpha: false, hot: (0, 0) };
        let img = if w > 0 && h > 0 { icofile::scale(&src, w, h) } else { src };
        return bitmap_from_bgra(img.w, img.h, &img.bgra, flags & LR_MONOCHROME != 0);
    }
    let Some((ftype, images)) = icofile::parse(&bytes) else { return 0 };
    let pick = icofile::pick(&images, (if w > 0 { w } else { 0 }, if h > 0 { h } else { 0 }));
    let want = if w > 0 && h > 0 { (w, h) } else { (pick.w, pick.h) };
    let img = icofile::scale(pick, want.0, want.1);
    let as_cursor = kind == 2;
    // A cursor takes the directory's fields as the hotspot (an .ico's
    // planes and bit count too); an icon's hotspot is its centre.
    let obj = if as_cursor {
        let mut c = img;
        let src_hot = pick.hot;
        let _ = ftype;
        c.hot = (
            (i64::from(src_hot.0) * i64::from(c.w) / i64::from(pick.w.max(1))) as i32,
            (i64::from(src_hot.1) * i64::from(c.h) / i64::from(pick.h.max(1))) as i32,
        );
        obj_from_image(c, true, false)
    } else {
        obj_from_image(img, false, true)
    };
    obj
}

// ----- DrawIconEx

const DI_MASK: u32 = 1;
const DI_IMAGE: u32 = 2;
const DI_DEFAULTSIZE: u32 = 8;

#[unsafe(no_mangle)]
pub extern "system" fn DrawIconEx(
    hdc: usize,
    x: i32,
    y: i32,
    icon: usize,
    cx: i32,
    cy: i32,
    _step: u32,
    _brush: usize,
    flags: u32,
) -> i32 {
    let ok = with_obj(icon, |o| {
        let (w, h) = if cx == 0 && cy == 0 {
            if flags & DI_DEFAULTSIZE != 0 { (32, 32) } else { (o.img.w, o.img.h) }
        } else {
            (cx, cy)
        };
        if w <= 0 || h <= 0 {
            return false;
        }
        let want_mask = flags & DI_MASK != 0;
        let want_image = flags & DI_IMAGE != 0;
        for dy in 0..h {
            for dx in 0..w {
                let sx = (i64::from(dx) * i64::from(o.img.w) / i64::from(w)) as usize;
                let sy = (i64::from(dy) * i64::from(o.img.h) / i64::from(h)) as usize;
                let at = sy * o.img.w as usize + sx;
                let and = o.img.and.get(at).copied().unwrap_or(0) != 0;
                // SAFETY: gdi32 checks the DC.
                let dst = unsafe { GetPixel(hdc, x + dx, y + dy) };
                if dst == u32::MAX {
                    continue;
                }
                let p = &o.img.bgra[at * 4..at * 4 + 4];
                let col = u32::from(p[2]) | u32::from(p[1]) << 8 | u32::from(p[0]) << 16;
                let out = if o.mono {
                    let xor = o.xor.get(at).copied().unwrap_or(0) != 0;
                    if want_mask && !want_image {
                        if and { 0xFF_FFFF } else { 0 }
                    } else if want_image && !want_mask {
                        if xor { 0xFF_FFFF } else { 0 }
                    } else {
                        (if and { dst } else { 0 }) ^ (if xor { 0xFF_FFFF } else { 0 })
                    }
                } else if o.img.alpha {
                    if want_mask && !want_image {
                        if p[3] != 0 { 0 } else { 0xFF_FFFF }
                    } else {
                        let a = u32::from(p[3]);
                        let mix = |s: u32, d: u32| (s * a + d * (255 - a)) / 255;
                        let (sr, sg, sb) = (col & 255, col >> 8 & 255, col >> 16 & 255);
                        let (dr, dg, db) = (dst & 255, dst >> 8 & 255, dst >> 16 & 255);
                        mix(sr, dr) | mix(sg, dg) << 8 | mix(sb, db) << 16
                    }
                } else if want_mask && !want_image {
                    if and { 0xFF_FFFF } else { 0 }
                } else if want_image && !want_mask {
                    col
                } else if and {
                    dst
                } else {
                    col
                };
                // SAFETY: gdi32 checks the DC.
                unsafe { SetPixelV(hdc, x + dx, y + dy, out) };
            }
        }
        true
    });
    match ok {
        Some(b) => i32::from(b),
        None => {
            set_last_error(ERROR_INVALID_CURSOR_HANDLE);
            0
        }
    }
}
