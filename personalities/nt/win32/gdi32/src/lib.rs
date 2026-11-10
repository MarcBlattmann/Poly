//! Our `gdi32.dll` (ADR 0128, Decision 5): GDI draws in the app's process.
//! The objects and the drawing are `poly-gdi`'s (one `Gdi` per process,
//! behind a lock); this is the export glue: arguments read from the
//! caller's memory, results and last errors written back. DIB sections'
//! pixels are pages from `VirtualAlloc` the caller addresses directly.
//!
//! Exports starting with `PolyGdi` are private, for our `user32.dll`
//! (window DCs, `FillRect`, `DrawTextW`).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_gdi::gdi::{Gdi, Rect};
use poly_gdi::region::Region;
use poly_win32_gui::{SpinLock, last_error, set_last_error, wide};
use poly_win32_rt as _;

#[link(name = "win32u", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: u64, out: *mut u8, cap: u64, out_len: *mut u64) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
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
    fn GetFullPathNameW(name: *const u16, size: u32, buf: *mut u16, file_part: *mut *mut u16) -> u32;
}

const MEM_COMMIT_RESERVE: u32 = 0x3000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
// winerror.h
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;

static GDI: SpinLock<Option<Gdi>> = SpinLock::new(None);

fn gdi<R>(f: impl FnOnce(&mut Gdi) -> R) -> R {
    GDI.with(|g| {
        f(g.get_or_insert_with(|| {
            let mut g = Gdi::new();
            g.set_font_loader(load_font);
            g
        }))
    })
}

/// A file of Poly's font store (ADR 0105, 0142): `C:\Windows\Fonts` on the
/// root disk, the Linux store (`Z:\usr\share\fonts\truetype\dejavu`, all the
/// desktop image has) otherwise; `None` where it is in neither, and the
/// bitmap faces draw instead.
fn load_font(name: &str) -> Option<Vec<u8>> {
    ["C:\\Windows\\Fonts\\", "Z:\\usr\\share\\fonts\\truetype\\dejavu\\"].iter().find_map(|dir| read_file(dir, name))
}

fn read_file(dir: &str, name: &str) -> Option<Vec<u8>> {
    let mut path: Vec<u16> = dir.encode_utf16().collect();
    path.extend(name.encode_utf16());
    path.push(0);
    read_path(&path).ok().filter(|d| !d.is_empty())
}

/// The bytes of the file `path` (NUL-terminated) names, or the last error
/// of the failed open (`ERROR_FILE_NOT_FOUND` if the system gave none).
/// A file that is empty or too big for a font gives an empty vector.
fn read_path(path: &[u16]) -> Result<Vec<u8>, u32> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 3;
    const OPEN_EXISTING: u32 = 3;
    const MAX_FONT_FILE: i64 = 64 << 20;
    // SAFETY: a NUL-terminated path; the handle is closed below.
    let file = unsafe {
        CreateFileW(path.as_ptr(), GENERIC_READ, FILE_SHARE_READ_WRITE, core::ptr::null(), OPEN_EXISTING, 0, 0)
    };
    if file == usize::MAX || file == 0 {
        let e = last_error();
        return Err(if e == 0 { 2 } else { e });
    }
    let mut size = 0i64;
    // SAFETY: an open file and a local out-value.
    let ok = unsafe { GetFileSizeEx(file, &mut size) } != 0 && size > 0 && size <= MAX_FONT_FILE;
    let mut data = Vec::new();
    if ok {
        data.resize(size as usize, 0u8);
        let mut done = 0usize;
        while done < data.len() {
            let mut got = 0u32;
            let want = (data.len() - done).min(1 << 20) as u32;
            // SAFETY: `want` bytes fit in the buffer after `done`.
            let r = unsafe { ReadFile(file, data.as_mut_ptr().add(done), want, &mut got, core::ptr::null_mut()) };
            if r == 0 || got == 0 {
                break;
            }
            done += got as usize;
        }
        data.truncate(done);
    }
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(file) };
    if ok && !data.is_empty() { Ok(data) } else { Ok(Vec::new()) }
}

/// `count` bytes of a string in the ANSI code page as UTF-16 (Latin-1;
/// `count` < 0: NUL-terminated).
///
/// # Safety
/// `p` points at such a string.
unsafe fn ansi(p: *const u8, count: i32) -> Vec<u16> {
    let mut v = Vec::new();
    if p.is_null() {
        return v;
    }
    let mut i = 0usize;
    loop {
        if count >= 0 && i >= count as usize {
            break;
        }
        // SAFETY: the caller's promise.
        let c = unsafe { *p.add(i) };
        if count < 0 && c == 0 {
            break;
        }
        v.push(u16::from(c));
        i += 1;
    }
    v
}

type H = usize;

unsafe fn read_rect(p: *const i32) -> Rect {
    // SAFETY: the caller passes a RECT.
    unsafe { Rect::new(*p, *p.add(1), *p.add(2), *p.add(3)) }
}

unsafe fn write_rect(p: *mut i32, r: Rect) {
    // SAFETY: the caller passes a writable RECT.
    unsafe {
        *p = r.left;
        *p.add(1) = r.top;
        *p.add(2) = r.right;
        *p.add(3) = r.bottom;
    }
}

// ----- objects

#[unsafe(no_mangle)]
pub extern "system" fn GetStockObject(i: i32) -> H {
    gdi(|g| g.stock(i as u32)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateCompatibleDC(hdc: H) -> H {
    gdi(|g| g.create_compatible_dc(hdc as u64)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn DeleteDC(hdc: H) -> i32 {
    let (ok, err) = gdi(|g| g.delete_dc(hdc as u64));
    if !ok {
        set_last_error(err);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn DeleteObject(h: H) -> i32 {
    let (ok, freed) = gdi(|g| {
        let mem = g.dib_memory(h as u64);
        let ok = g.delete_object(h as u64);
        let gone = g.object_type(h as u64) == 0;
        (ok, if gone { mem } else { None })
    });
    if let Some((addr, _)) = freed {
        // SAFETY: pages this DLL allocated for the DIB section.
        unsafe { VirtualFree(addr as *mut u8, 0, MEM_RELEASE) };
    }
    if !ok {
        set_last_error(0);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn SelectObject(hdc: H, obj: H) -> H {
    match gdi(|g| g.select_object(hdc as u64, obj as u64)) {
        Ok(h) => h as H,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetObjectType(h: H) -> u32 {
    gdi(|g| g.object_type(h as u64))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentObject(hdc: H, kind: u32) -> H {
    gdi(|g| g.current_object(hdc as u64, kind)) as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetObjectW(h: H, size: i32, buf: *mut u8) -> i32 {
    // A region has no description: 0 with ERROR_INVALID_HANDLE
    // (CreateRectRgn.yaml `in_a_dc`).
    if gdi(|g| g.is_region(h as u64)) {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    if buf.is_null() {
        return gdi(|g| g.object_bytes(h as u64)).map_or(0, |b| b.len() as i32);
    }
    let Some(bytes) = gdi(|g| g.object_bytes_for(h as u64, size.max(0) as usize)) else { return 0 };
    let n = bytes.len().min(size.max(0) as usize);
    // SAFETY: the caller's buffer holds `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n) };
    n as i32
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateSolidBrush(color: u32) -> H {
    gdi(|g| g.create_solid_brush(color)) as H
}

/// `CreateHatchBrush` (Learn): a brush of one of the six hatch styles.
#[unsafe(no_mangle)]
pub extern "system" fn CreateHatchBrush(hatch: i32, color: u32) -> H {
    if !(0..=5).contains(&hatch) {
        return 0;
    }
    gdi(|g| g.create_styled_brush(poly_gdi::gdi::BS_HATCHED, color, hatch as u64)) as H
}

/// `CreatePatternBrush` (Learn): a brush from a bitmap.
#[unsafe(no_mangle)]
pub extern "system" fn CreatePatternBrush(bitmap: H) -> H {
    if gdi(|g| g.object_type(bitmap as u64)) != poly_gdi::gdi::OBJ_BITMAP {
        return 0;
    }
    gdi(|g| g.create_styled_brush(poly_gdi::gdi::BS_PATTERN, 0, bitmap as u64)) as H
}

/// `CreateDIBPatternBrushPt` (Learn): a brush from a packed DIB.
#[unsafe(no_mangle)]
pub extern "system" fn CreateDIBPatternBrushPt(packed_dib: *const u8, _usage: u32) -> H {
    if packed_dib.is_null() {
        return 0;
    }
    gdi(|g| g.create_styled_brush(poly_gdi::gdi::BS_DIBPATTERNPT, 0, packed_dib as u64)) as H
}

/// `CreatePalette` (Learn): a logical palette; NULL for a NULL `LOGPALETTE`.
#[unsafe(no_mangle)]
pub extern "system" fn CreatePalette(log_palette: *const u8) -> H {
    if log_palette.is_null() {
        return 0;
    }
    gdi(|g| g.create_palette()) as H
}

/// `SelectPalette` (Learn): the DC's previous palette, NULL on failure.
#[unsafe(no_mangle)]
pub extern "system" fn SelectPalette(hdc: H, palette: H, _force_background: i32) -> H {
    gdi(|g| g.select_palette(hdc as u64, palette as u64)) as H
}

/// `RealizePalette` (Learn): the number of entries mapped; none on a
/// true-colour screen.
#[unsafe(no_mangle)]
pub extern "system" fn RealizePalette(_hdc: H) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn CreatePen(style: i32, width: i32, color: u32) -> H {
    gdi(|g| g.create_pen(style as u32, width, color)) as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateBitmap(w: i32, h: i32, _planes: u32, bpp: u32, bits: *const u8) -> H {
    let src = if bits.is_null() || w <= 0 || h <= 0 {
        None
    } else {
        let stride = (w as usize * if bpp == 1 { 1 } else { 32 }).div_ceil(16) * 2;
        // SAFETY: the caller passes rows of the bitmap's size.
        Some(unsafe { core::slice::from_raw_parts(bits, stride * h as usize) })
    };
    gdi(|g| g.create_bitmap(w, h, bpp as u16, src)) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateCompatibleBitmap(hdc: H, w: i32, h: i32) -> H {
    gdi(|g| g.create_compatible_bitmap(hdc as u64, w, h)) as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDIBSection(
    _hdc: H,
    info: *const u8,
    _usage: u32,
    bits: *mut *mut u8,
    _section: H,
    _offset: u32,
) -> H {
    let fail = |code: u32| {
        if !bits.is_null() {
            // SAFETY: the caller's out-pointer.
            unsafe { *bits = core::ptr::null_mut() };
        }
        set_last_error(code);
        0
    };
    if info.is_null() {
        return fail(87);
    }
    // The header, and the colour table for 8 bits per pixel and below.
    // SAFETY: the caller passes a BITMAPINFO.
    let mut bytes: Vec<u8> = unsafe { core::slice::from_raw_parts(info, 40) }.to_vec();
    let size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let bpp = u16::from_le_bytes([bytes[14], bytes[15]]);
    let used = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]) as usize;
    if (40..=124).contains(&size) && matches!(bpp, 1 | 4 | 8) {
        let n = if used == 0 { 1 << bpp } else { used.min(1 << bpp) };
        // SAFETY: a BITMAPINFO with that many colours.
        bytes = unsafe { core::slice::from_raw_parts(info, size + 4 * n) }.to_vec();
    }
    let (w, h, bpp, colors, len) = match Gdi::check_dib(&bytes) {
        Ok(v) => v,
        Err(code) => return fail(code),
    };
    // SAFETY: fresh zeroed pages.
    let mem = unsafe { VirtualAlloc(core::ptr::null_mut(), len.max(1), MEM_COMMIT_RESERVE, PAGE_READWRITE) };
    if mem.is_null() {
        return fail(8);
    }
    let handle = gdi(|g| g.create_dib(w, h, bpp, colors, mem as u64, len));
    if !bits.is_null() {
        // SAFETY: the caller's out-pointer.
        unsafe { *bits = mem };
    }
    handle as H
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFontIndirectW(lf: *const u8) -> H {
    if lf.is_null() {
        return 0;
    }
    // SAFETY: a LOGFONTW (92 bytes).
    let b = unsafe { core::slice::from_raw_parts(lf, 92) };
    gdi(|g| g.create_font(b)) as H
}

/// `CreateFontIndirectA`: a `LOGFONTA` (60 bytes; the face name is 32
/// ANSI characters) becomes the `LOGFONTW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateFontIndirectA(lf: *const u8) -> H {
    if lf.is_null() {
        return 0;
    }
    // SAFETY: a LOGFONTA (60 bytes).
    let a = unsafe { core::slice::from_raw_parts(lf, 60) };
    let mut w = [0u8; 92];
    w[..28].copy_from_slice(&a[..28]);
    for i in 0..32 {
        if a[28 + i] == 0 {
            break;
        }
        w[28 + 2 * i] = a[28 + i];
    }
    gdi(|g| g.create_font(&w)) as H
}

/// The `LOGFONTW` of `CreateFontW`'s arguments (Learn).
#[allow(clippy::too_many_arguments)]
fn logfont_of(
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u32,
    underline: u32,
    strike_out: u32,
    char_set: u32,
    out_precision: u32,
    clip_precision: u32,
    quality: u32,
    pitch_and_family: u32,
) -> [u8; 92] {
    let mut b = [0u8; 92];
    for (i, v) in [height, width, escapement, orientation, weight].iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[20] = italic as u8;
    b[21] = underline as u8;
    b[22] = strike_out as u8;
    b[23] = char_set as u8;
    b[24] = out_precision as u8;
    b[25] = clip_precision as u8;
    b[26] = quality as u8;
    b[27] = pitch_and_family as u8;
    b
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn CreateFontW(
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u32,
    underline: u32,
    strike_out: u32,
    char_set: u32,
    out_precision: u32,
    clip_precision: u32,
    quality: u32,
    pitch_and_family: u32,
    face: *const u16,
) -> H {
    let mut b = logfont_of(
        height,
        width,
        escapement,
        orientation,
        weight,
        italic,
        underline,
        strike_out,
        char_set,
        out_precision,
        clip_precision,
        quality,
        pitch_and_family,
    );
    if !face.is_null() {
        for i in 0..32 {
            // SAFETY: a NUL-terminated name; reading stops at the NUL.
            let c = unsafe { *face.add(i) };
            if c == 0 {
                break;
            }
            b[28 + 2 * i..30 + 2 * i].copy_from_slice(&c.to_le_bytes());
        }
    }
    gdi(|g| g.create_font(&b)) as H
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn CreateFontA(
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u32,
    underline: u32,
    strike_out: u32,
    char_set: u32,
    out_precision: u32,
    clip_precision: u32,
    quality: u32,
    pitch_and_family: u32,
    face: *const u8,
) -> H {
    let b = logfont_of(
        height,
        width,
        escapement,
        orientation,
        weight,
        italic,
        underline,
        strike_out,
        char_set,
        out_precision,
        clip_precision,
        quality,
        pitch_and_family,
    );
    let mut a = [0u8; 60];
    a[..28].copy_from_slice(&b[..28]);
    if !face.is_null() {
        for i in 0..31 {
            // SAFETY: a NUL-terminated name; reading stops at the NUL.
            let c = unsafe { *face.add(i) };
            if c == 0 {
                break;
            }
            a[28 + i] = c;
        }
    }
    // SAFETY: our LOGFONTA.
    unsafe { CreateFontIndirectA(a.as_ptr()) }
}

// ----- regions

/// A (kind, error) result: the error, if any, becomes the last error.
fn kind_or_error((kind, err): (i32, u32)) -> i32 {
    if err != 0 {
        set_last_error(err);
    }
    kind
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateRectRgn(l: i32, t: i32, r: i32, b: i32) -> H {
    match gdi(|g| g.create_rect_rgn(Rect::new(l, t, r, b))) {
        Ok(h) => h as H,
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateRectRgnIndirect(rect: *const i32) -> H {
    if rect.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { read_rect(rect) };
    CreateRectRgn(r.left, r.top, r.right, r.bottom)
}

#[unsafe(no_mangle)]
pub extern "system" fn SetRectRgn(rgn: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    i32::from(gdi(|g| g.set_rect_rgn(rgn as u64, Rect::new(l, t, r, b))))
}

#[unsafe(no_mangle)]
pub extern "system" fn CombineRgn(dst: H, src1: H, src2: H, mode: i32) -> i32 {
    kind_or_error(gdi(|g| g.combine_rgn(dst as u64, src1 as u64, src2 as u64, mode)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetRgnBox(rgn: H, rect: *mut i32) -> i32 {
    let Some((kind, r)) = gdi(|g| g.rgn_box(rgn as u64)) else { return 0 };
    if !rect.is_null() {
        // SAFETY: the caller's RECT.
        unsafe { write_rect(rect, r) };
    }
    kind
}

#[unsafe(no_mangle)]
pub extern "system" fn OffsetRgn(rgn: H, dx: i32, dy: i32) -> i32 {
    gdi(|g| g.offset_rgn(rgn as u64, dx, dy))
}

#[unsafe(no_mangle)]
pub extern "system" fn EqualRgn(a: H, b: H) -> i32 {
    gdi(|g| g.equal_rgn(a as u64, b as u64))
}

#[unsafe(no_mangle)]
pub extern "system" fn PtInRegion(rgn: H, x: i32, y: i32) -> i32 {
    match gdi(|g| g.pt_in_region(rgn as u64, x, y)) {
        Ok(inside) => i32::from(inside),
        Err(e) => {
            set_last_error(e);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RectInRegion(rgn: H, rect: *const i32) -> i32 {
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { read_rect(rect) };
    i32::from(gdi(|g| g.rect_in_region(rgn as u64, r)).unwrap_or(false))
}

/// `GetRegionData` (Learn): the size needed for a NULL buffer, `count`
/// when it was large enough, else 0.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetRegionData(rgn: H, count: u32, data: *mut u8) -> u32 {
    // CreateRectRgn.yaml `region_data_buffer`: not a region is
    // ERROR_INVALID_HANDLE; a buffer smaller than the data is
    // ERROR_INVALID_PARAMETER with nothing written; otherwise the bytes
    // written (the size needed, not the buffer's).
    let Some(bytes) = gdi(|g| g.region_data(rgn as u64)) else {
        set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    };
    if data.is_null() {
        return bytes.len() as u32;
    }
    if (count as usize) < bytes.len() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's buffer holds `count` bytes, at least these.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len()) };
    bytes.len() as u32
}

/// `FillRgn` (Learn): fills a region with a brush.
#[unsafe(no_mangle)]
pub extern "system" fn FillRgn(hdc: H, rgn: H, brush: H) -> i32 {
    i32::from(gdi(|g| g.fill_rgn(hdc as u64, rgn as u64, brush as u64)))
}

// ----- clipping

#[unsafe(no_mangle)]
pub extern "system" fn SelectClipRgn(hdc: H, rgn: H) -> i32 {
    kind_or_error(gdi(|g| g.select_clip_rgn(hdc as u64, rgn as u64)))
}

#[unsafe(no_mangle)]
pub extern "system" fn ExtSelectClipRgn(hdc: H, rgn: H, mode: i32) -> i32 {
    kind_or_error(gdi(|g| g.ext_select_clip_rgn(hdc as u64, rgn as u64, mode)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClipRgn(hdc: H, rgn: H) -> i32 {
    kind_or_error(gdi(|g| g.clip_rgn(hdc as u64, rgn as u64)))
}

#[unsafe(no_mangle)]
pub extern "system" fn IntersectClipRect(hdc: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    kind_or_error(gdi(|g| g.clip_rect(hdc as u64, Rect::new(l, t, r, b), false)))
}

#[unsafe(no_mangle)]
pub extern "system" fn ExcludeClipRect(hdc: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    kind_or_error(gdi(|g| g.clip_rect(hdc as u64, Rect::new(l, t, r, b), true)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetRandomRgn(hdc: H, rgn: H, which: i32) -> i32 {
    gdi(|g| g.random_rgn(hdc as u64, rgn as u64, which))
}

/// `SetViewportOrgEx` (`viewport`) / `SetWindowOrgEx`.
unsafe fn set_origin(hdc: H, viewport: bool, x: i32, y: i32, old: *mut i32) -> i32 {
    let Some((ox, oy)) = gdi(|g| g.set_origin(hdc as u64, viewport, x, y)) else { return 0 };
    if !old.is_null() {
        // SAFETY: the caller's POINT.
        unsafe {
            *old = ox;
            *old.add(1) = oy;
        }
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetViewportOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { set_origin(hdc, true, x, y, old) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWindowOrgEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    // SAFETY: the caller's POINT or NULL.
    unsafe { set_origin(hdc, false, x, y, old) }
}

// ----- attributes

macro_rules! dc_get_set {
    ($($name:ident($($a:ident: $t:ty),*) -> $r:ty = $m:ident;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name(hdc: H $(, $a: $t)*) -> $r {
            gdi(|g| g.$m(hdc as u64 $(, $a)*))
        }
    )*};
}

dc_get_set! {
    SetTextColor(c: u32) -> u32 = set_text_color;
    GetTextColor() -> u32 = text_color;
    SetBkColor(c: u32) -> u32 = set_bk_color;
    GetBkColor() -> u32 = bk_color;
    SetBkMode(m: i32) -> i32 = set_bk_mode;
    GetBkMode() -> i32 = bk_mode;
    SetDCBrushColor(c: u32) -> u32 = set_dc_brush_color;
    SetDCPenColor(c: u32) -> u32 = set_dc_pen_color;
    SaveDC() -> i32 = save_dc;
}

/// `GetDeviceCaps` (GetSystemMetrics.yaml `device_caps_screen`): the
/// screen's values for every DC, of the display win32k has now.
#[unsafe(no_mangle)]
pub extern "system" fn GetDeviceCaps(hdc: H, index: i32) -> i32 {
    let size = display_size();
    let caps = gdi(|g| {
        if let Some((width, height)) = size {
            g.screen = poly_gdi::gdi::Screen { width, height };
        }
        g.device_caps(hdc as u64, index)
    });
    caps.unwrap_or_else(|| {
        set_last_error(ERROR_INVALID_HANDLE);
        0
    })
}

/// The screen's size in pixels, from win32k (`None`: it did not answer).
fn display_size() -> Option<(i32, i32)> {
    use poly_win32k::proto::{Reply, Request};
    let req = poly_win32k::wire::encode(&Request::Desktop {});
    let mut out = [0u8; 256];
    let mut len = 0u64;
    // SAFETY: the buffers are ours and sized as passed.
    let s = unsafe { PolyUserCall(req.as_ptr(), req.len() as u64, out.as_mut_ptr(), out.len() as u64, &mut len) };
    if s != 0 {
        return None;
    }
    match poly_win32k::wire::decode(out.get(..len as usize)?)? {
        Reply::Desktop { width, height, .. } => Some((width, height)),
        _ => None,
    }
}

/// `SetTextAlign`: the previous alignment; `GDI_ERROR` for a bad DC.
#[unsafe(no_mangle)]
pub extern "system" fn SetTextAlign(hdc: H, a: u32) -> u32 {
    let old = gdi(|g| g.set_text_align(hdc as u64, a));
    if old == u32::MAX && !gdi(|g| g.is_dc(hdc as u64)) {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    old
}

/// `GetTextAlign`: the alignment; `GDI_ERROR` for a bad DC.
#[unsafe(no_mangle)]
pub extern "system" fn GetTextAlign(hdc: H) -> u32 {
    let a = gdi(|g| g.text_align(hdc as u64));
    if a == u32::MAX && !gdi(|g| g.is_dc(hdc as u64)) {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    a
}

/// `GetCurrentPositionEx`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCurrentPositionEx(hdc: H, pt: *mut i32) -> i32 {
    match gdi(|g| g.current_position(hdc as u64)) {
        Some((x, y)) => {
            if !pt.is_null() {
                // SAFETY: the caller's POINT.
                unsafe {
                    *pt = x;
                    *pt.add(1) = y;
                }
            }
            1
        }
        None => {
            set_last_error(ERROR_INVALID_HANDLE);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RestoreDC(hdc: H, level: i32) -> i32 {
    i32::from(gdi(|g| g.restore_dc(hdc as u64, level)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn MoveToEx(hdc: H, x: i32, y: i32, old: *mut i32) -> i32 {
    match gdi(|g| g.move_to(hdc as u64, x, y)) {
        Some((ox, oy)) => {
            if !old.is_null() {
                // SAFETY: the caller's POINT.
                unsafe {
                    *old = ox;
                    *old.add(1) = oy;
                }
            }
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClipBox(hdc: H, rect: *mut i32) -> i32 {
    let (kind, r) = gdi(|g| g.clip_box(hdc as u64));
    if kind == 0 {
        set_last_error(ERROR_INVALID_HANDLE);
    } else if !rect.is_null() {
        // SAFETY: the caller's RECT.
        unsafe { write_rect(rect, r) };
    }
    kind
}

// ----- drawing

#[unsafe(no_mangle)]
pub extern "system" fn SetPixel(hdc: H, x: i32, y: i32, c: u32) -> u32 {
    gdi(|g| g.set_pixel(hdc as u64, x, y, c))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetPixelV(hdc: H, x: i32, y: i32, c: u32) -> i32 {
    i32::from(SetPixel(hdc, x, y, c) != poly_gdi::gdi::CLR_INVALID)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetPixel(hdc: H, x: i32, y: i32) -> u32 {
    gdi(|g| g.get_pixel(hdc as u64, x, y))
}

#[unsafe(no_mangle)]
pub extern "system" fn PatBlt(hdc: H, x: i32, y: i32, w: i32, h: i32, rop: u32) -> i32 {
    let ok = gdi(|g| g.pat_blt(hdc as u64, x, y, w, h, rop));
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn BitBlt(dst: H, x: i32, y: i32, w: i32, h: i32, src: H, sx: i32, sy: i32, rop: u32) -> i32 {
    let (ok, err) = gdi(|g| g.bit_blt(dst as u64, x, y, w, h, src as u64, sx, sy, rop));
    if !ok {
        set_last_error(err);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn Rectangle(hdc: H, l: i32, t: i32, r: i32, b: i32) -> i32 {
    i32::from(gdi(|g| g.rectangle(hdc as u64, l, t, r, b)))
}

#[unsafe(no_mangle)]
pub extern "system" fn LineTo(hdc: H, x: i32, y: i32) -> i32 {
    i32::from(gdi(|g| g.line_to(hdc as u64, x, y)))
}

#[unsafe(no_mangle)]
pub extern "system" fn GdiFlush() -> i32 {
    1
}

// ----- text

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TextOutW(hdc: H, x: i32, y: i32, text: *const u16, count: i32) -> i32 {
    // A negative count fails; a NULL string does not draw.
    if count < 0 || (text.is_null() && count > 0) {
        return 0;
    }
    // SAFETY: `count` characters at `text`.
    let s = unsafe { wide(text, count) };
    let ok = gdi(|g| g.text_out(hdc as u64, x, y, &s));
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn TextOutA(hdc: H, x: i32, y: i32, text: *const u8, count: i32) -> i32 {
    // SAFETY: `count` characters at `text`.
    let s = unsafe { ansi(text, count.max(0)) };
    // SAFETY: our own buffer.
    unsafe { TextOutW(hdc, x, y, s.as_ptr(), s.len() as i32) }
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn ExtTextOutW(
    hdc: H,
    x: i32,
    y: i32,
    options: u32,
    rect: *const i32,
    text: *const u16,
    count: u32,
    dx: *const i32,
) -> i32 {
    // Options with every bit set fail, and so does a NULL string to draw.
    if options == u32::MAX || (text.is_null() && count > 0) {
        return 0;
    }
    // SAFETY: `count` characters at `text`, `count` spacings at `dx`, a RECT.
    let (s, rect, dx) = unsafe {
        (
            wide(text, count.min(i32::MAX as u32) as i32),
            if rect.is_null() { None } else { Some(read_rect(rect)) },
            if dx.is_null() { None } else { Some(core::slice::from_raw_parts(dx, count as usize).to_vec()) },
        )
    };
    let ok = gdi(|g| g.ext_text_out(hdc as u64, x, y, options, rect, &s, dx.as_deref()));
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn ExtTextOutA(
    hdc: H,
    x: i32,
    y: i32,
    options: u32,
    rect: *const i32,
    text: *const u8,
    count: u32,
    dx: *const i32,
) -> i32 {
    // SAFETY: `count` characters at `text`.
    let s = unsafe { ansi(text, count.min(i32::MAX as u32) as i32) };
    // SAFETY: our own buffer; the rest is the caller's.
    unsafe { ExtTextOutW(hdc, x, y, options, rect, s.as_ptr(), s.len() as u32, dx) }
}

unsafe fn write_size(size: *mut i32, (cx, cy): (i32, i32)) {
    if !size.is_null() {
        // SAFETY: the caller's SIZE.
        unsafe {
            *size = cx;
            *size.add(1) = cy;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentPoint32W(hdc: H, text: *const u16, count: i32, size: *mut i32) -> i32 {
    if count < 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: `count` characters at `text`.
    let s = unsafe { wide(text, count) };
    match gdi(|g| g.text_extent(hdc as u64, &s)) {
        Some(e) => {
            // SAFETY: the caller's SIZE.
            unsafe { write_size(size, e) };
            1
        }
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentPointW(hdc: H, text: *const u16, count: i32, size: *mut i32) -> i32 {
    // SAFETY: as `GetTextExtentPoint32W`.
    unsafe { GetTextExtentPoint32W(hdc, text, count, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentPoint32A(hdc: H, text: *const u8, count: i32, size: *mut i32) -> i32 {
    // SAFETY: `count` characters at `text`.
    let s = unsafe { ansi(text, count.max(0)) };
    // SAFETY: our own buffer.
    unsafe { GetTextExtentPoint32W(hdc, s.as_ptr(), s.len() as i32, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentPointA(hdc: H, text: *const u8, count: i32, size: *mut i32) -> i32 {
    // SAFETY: as `GetTextExtentPoint32A`.
    unsafe { GetTextExtentPoint32A(hdc, text, count, size) }
}

/// `GetTextExtentExPointW`: `fit` gets how many characters fit in `max`,
/// `dx` the width of the text up to each of them. Without `fit` the maximum
/// is not used (Learn) and `dx` gets the running width of every character;
/// a negative maximum fails all the same (`GetTextExtentExPointW.yaml`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentExPointW(
    hdc: H,
    text: *const u16,
    count: i32,
    max: i32,
    fit: *mut i32,
    dx: *mut i32,
    size: *mut i32,
) -> i32 {
    // A negative maximum or count fails and leaves the outputs alone.
    if max < 0 || count < 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: `count` characters at `text`.
    let s = unsafe { wide(text, count) };
    match gdi(|g| g.text_extent_ex(hdc as u64, &s, max)) {
        Some((e, n, running)) => {
            // SAFETY: the caller's out-pointers; `dx` holds `count` values,
            // of which the characters that fit are written (all of them
            // when the caller did not ask how many fit).
            unsafe {
                write_size(size, e);
                if !fit.is_null() {
                    *fit = n as i32;
                }
                if !dx.is_null() {
                    let filled = if fit.is_null() { running.len() } else { n };
                    for (i, v) in running.iter().take(filled).enumerate() {
                        *dx.add(i) = *v;
                    }
                }
            }
            1
        }
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextExtentExPointA(
    hdc: H,
    text: *const u8,
    count: i32,
    max: i32,
    fit: *mut i32,
    dx: *mut i32,
    size: *mut i32,
) -> i32 {
    // SAFETY: `count` characters at `text`.
    let s = unsafe { ansi(text, count.max(0)) };
    // SAFETY: our own buffer; the rest is the caller's.
    unsafe { GetTextExtentExPointW(hdc, s.as_ptr(), s.len() as i32, max, fit, dx, size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextMetricsW(hdc: H, tm: *mut u8) -> i32 {
    match gdi(|g| g.text_metrics(hdc as u64)) {
        Some(m) => {
            let b = m.to_bytes();
            // SAFETY: the caller's TEXTMETRICW (60 bytes).
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), tm, b.len()) };
            1
        }
        None => 0,
    }
}

/// `GetTextMetricsA`: `TEXTMETRICA` is the same up to `tmWeight`...
/// `tmOverhang`, `tmDigitizedAspect*`, then the five character fields as
/// bytes (`wingdi.h`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextMetricsA(hdc: H, tm: *mut u8) -> i32 {
    match gdi(|g| g.text_metrics(hdc as u64)) {
        Some(m) => {
            let w = m.to_bytes();
            let mut a = [0u8; 56];
            a[..44].copy_from_slice(&w[..44]);
            let byte = |c: u16| if c > 0xFF { 0xFF } else { c as u8 };
            a[44] = byte(m.first_char);
            a[45] = byte(m.last_char);
            a[46] = byte(m.default_char);
            a[47] = byte(m.break_char);
            a[48] = m.italic;
            a[49] = m.underlined;
            a[50] = m.struck_out;
            a[51] = m.pitch_and_family;
            a[52] = m.char_set;
            // SAFETY: the caller's TEXTMETRICA (56 bytes).
            unsafe { core::ptr::copy_nonoverlapping(a.as_ptr(), tm, a.len()) };
            1
        }
        None => 0,
    }
}

/// `GetTextFaceW`: the name (with its NUL) copied up to `count` units; the
/// length written, with the NUL. A NULL buffer asks for the size needed.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextFaceW(hdc: H, count: i32, buf: *mut u16) -> i32 {
    let Some(mut name) = gdi(|g| g.text_face(hdc as u64)) else { return 0 };
    name.push(0);
    if buf.is_null() {
        return name.len() as i32;
    }
    if count <= 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let n = name.len().min(count.max(0) as usize);
    // SAFETY: the caller's buffer of `count` units.
    unsafe { core::ptr::copy_nonoverlapping(name.as_ptr(), buf, n) };
    if n > 0 && n < name.len() {
        // SAFETY: as above.
        unsafe { *buf.add(n - 1) = 0 };
    }
    n as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetTextFaceA(hdc: H, count: i32, buf: *mut u8) -> i32 {
    let Some(mut name) = gdi(|g| g.text_face(hdc as u64)) else { return 0 };
    name.push(0);
    if buf.is_null() {
        return name.len() as i32;
    }
    if count <= 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let n = name.len().min(count.max(0) as usize);
    for (i, &c) in name.iter().take(n).enumerate() {
        // SAFETY: the caller's buffer of `count` bytes.
        unsafe { *buf.add(i) = if c < 0x100 { c as u8 } else { b'?' } };
    }
    if n > 0 && n < name.len() {
        // SAFETY: as above.
        unsafe { *buf.add(n - 1) = 0 };
    }
    n as i32
}

/// `GetCharWidth32W`: the advance of each character of `first..=last`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCharWidth32W(hdc: H, first: u32, last: u32, buf: *mut i32) -> i32 {
    match gdi(|g| g.char_widths(hdc as u64, first, last)) {
        Some(w) => {
            if !buf.is_null() {
                for (i, v) in w.iter().enumerate() {
                    // SAFETY: the caller's buffer of `last - first + 1` values.
                    unsafe { *buf.add(i) = *v };
                }
            }
            1
        }
        None => {
            set_last_error(ERROR_INVALID_PARAMETER);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCharWidthW(hdc: H, first: u32, last: u32, buf: *mut i32) -> i32 {
    // SAFETY: as `GetCharWidth32W`.
    unsafe { GetCharWidth32W(hdc, first, last, buf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCharWidth32A(hdc: H, first: u32, last: u32, buf: *mut i32) -> i32 {
    // SAFETY: as `GetCharWidth32W`; the bytes are Latin-1 characters.
    unsafe { GetCharWidth32W(hdc, first, last, buf) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetCharWidthA(hdc: H, first: u32, last: u32, buf: *mut i32) -> i32 {
    // SAFETY: as `GetCharWidth32A`.
    unsafe { GetCharWidth32A(hdc, first, last, buf) }
}

// ----- for user32

/// A DC for window `hwnd` (0: the screen); `persistent` for a class or
/// own DC.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiCreateDisplayDC(hwnd: H, persistent: i32) -> H {
    gdi(|g| g.create_display_dc(hwnd as u64, persistent != 0)) as H
}

/// `ReleaseDC`'s GDI part: 1 for a display DC.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiReleaseDisplayDC(hdc: H) -> i32 {
    i32::from(gdi(|g| g.release_display_dc(hdc as u64)))
}

/// The window a display DC was made for (`WindowFromDC`): `None` for a
/// memory DC or a handle that is no DC; 0 for the screen's.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiDisplayWindow(hdc: H, out: *mut u64) -> i32 {
    match gdi(|g| g.display_hwnd(hdc as u64)) {
        Some(w) => {
            // SAFETY: the caller passes room for one u64.
            unsafe { *out = w };
            1
        }
        None => 0,
    }
}

/// A window's own or class DC goes with the window.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiDeleteDisplayDC(hdc: H) -> i32 {
    i32::from(gdi(|g| g.delete_dc(hdc as u64).0))
}

/// `count` RECTs at `p` (none for NULL).
unsafe fn read_rects(p: *const i32, count: u32) -> Vec<Rect> {
    if p.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller passes `count` RECTs.
    (0..count as usize).map(|i| unsafe { read_rect(p.add(4 * i)) }).collect()
}

/// What window DC `hdc` can draw on (user32's `GetDC`/`GetWindowDC`):
/// its device 0,0 is at (`x`, `y`) on the screen, and it reaches the
/// union of the `n_include` RECTs at `include` less the union of the
/// `n_exclude` RECTs at `exclude`, in screen coordinates. 1 on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSetDisplayGeometry(
    hdc: H,
    x: i32,
    y: i32,
    include: *const i32,
    n_include: u32,
    exclude: *const i32,
    n_exclude: u32,
) -> i32 {
    // SAFETY: the caller's arrays of RECTs.
    let (inc, exc) = unsafe { (read_rects(include, n_include), read_rects(exclude, n_exclude)) };
    i32::from(gdi(|g| g.set_display_geometry(hdc as u64, (x, y), &inc, &exc)))
}

/// Set window DC `hdc`'s system clip to the union of the `n` RECTs at
/// `rects` (device, i.e. client or window, coordinates), as `BeginPaint`
/// limits its DC to the update region: drawing and `GetClipBox` honour it
/// on top of the visible region and any clip region, `GetClipRgn` does not
/// see it. NULL `rects` removes it; the next `GetDC`/`GetWindowDC` of the
/// DC removes it too. 1 on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSetSystemClip(hdc: H, rects: *const i32, n: u32) -> i32 {
    let clip = if rects.is_null() {
        None
    } else {
        // SAFETY: the caller's array of RECTs.
        let rs = unsafe { read_rects(rects, n) };
        Some(solid_union(&rs))
    };
    i32::from(gdi(|g| g.set_system_clip(hdc as u64, clip)))
}

/// The union of rectangles, empty and inverted ones counting as nothing.
fn solid_union(rs: &[Rect]) -> Region {
    let solid: Vec<Rect> = rs.iter().filter(|r| !r.is_empty()).copied().collect();
    Region::from_rects(&solid)
}

/// Region `hrgn`'s rectangles in canonical band order (`GetRegionData`'s)
/// as (left, top, right, bottom) quadruples into `out`, at most `cap` of
/// them: the total count (`cap` 0 asks for it), -1 when `hrgn` is not a
/// region.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiGetRegionRects(hrgn: H, out: *mut i32, cap: u32) -> i32 {
    let Some(rects) = gdi(|g| g.region(hrgn as u64).map(|r| r.rects().to_vec())) else { return -1 };
    if !out.is_null() {
        for (i, r) in rects.iter().take(cap as usize).enumerate() {
            // SAFETY: the caller's buffer holds `cap` RECTs.
            unsafe { write_rect(out.add(4 * i), *r) };
        }
    }
    rects.len() as i32
}

/// Replace region `hrgn`'s contents by the union of the `n` RECTs at
/// `rects` (empty or inverted ones count as nothing): the new kind
/// (NULLREGION 1, SIMPLEREGION 2, COMPLEXREGION 3), ERROR 0 when `hrgn` is
/// not a region.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSetRegionRects(hrgn: H, rects: *const i32, n: u32) -> i32 {
    // SAFETY: the caller's array of RECTs (or NULL: none).
    let rs = unsafe { read_rects(rects, n) };
    let rgn = solid_union(&rs);
    gdi(|g| g.set_region(hrgn as u64, rgn))
}

/// The desktop link's surface DIBs (ADR 0131): per window, a top-down
/// 32-bpp DIB section of its surface's size that its DCs draw into:
/// (hwnd, bitmap, pixels, width, height).
type SurfaceDib = (H, u64, usize, u32, u32);
static SURFACES: SpinLock<Vec<SurfaceDib>> = SpinLock::new(Vec::new());

/// Point display DC `hdc` of its window at that window's surface DIB of
/// `width` x `height` (made, or remade for a new size); 0 x 0: draw
/// nowhere. 1 on success.
#[unsafe(no_mangle)]
pub extern "system" fn PolyGdiAttachSurface(hdc: H, width: u32, height: u32) -> i32 {
    let Some(hwnd) = gdi(|g| g.display_hwnd(hdc as u64)) else { return 0 };
    if width == 0 || height == 0 {
        gdi(|g| g.set_surface(hdc as u64, None));
        return 1;
    }
    let hwnd = hwnd as H;
    let current = SURFACES.with(|s| s.iter().find(|e| e.0 == hwnd).copied());
    let bitmap = match current {
        Some((_, b, _, w, h)) if (w, h) == (width, height) => b,
        old => {
            let len = width as usize * height as usize * 4;
            // SAFETY: fresh zeroed pages.
            let mem = unsafe { VirtualAlloc(core::ptr::null_mut(), len, MEM_COMMIT_RESERVE, PAGE_READWRITE) };
            if mem.is_null() {
                return 0;
            }
            // White, as the compositor shows a surface before it is drawn.
            // SAFETY: `len` bytes just allocated.
            unsafe { core::ptr::write_bytes(mem, 0xFF, len) };
            let b = gdi(|g| g.create_dib(width as i32, -(height as i32), 32, Vec::new(), mem as u64, len));
            let b = u64::from(b);
            SURFACES.with(|s| {
                s.retain(|e| e.0 != hwnd);
                s.push((hwnd, b, mem as usize, width, height));
            });
            if let Some((_, ob, _, _, _)) = old {
                // DCs still pointing at it draw nowhere until attached again.
                DeleteObject(ob as H);
            }
            b
        }
    };
    gdi(|g| g.set_surface(hdc as u64, Some(bitmap as u32)));
    1
}

/// Window `hwnd`'s surface DIB: its pixels, width and height into
/// `out[0..3]`; 0 when it has none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiSurface(hwnd: H, out: *mut u64) -> i32 {
    let Some((_, _, bits, w, h)) = SURFACES.with(|s| s.iter().find(|e| e.0 == hwnd).copied()) else { return 0 };
    if out.is_null() {
        return 0;
    }
    // SAFETY: the caller passes three writable u64s.
    unsafe {
        *out = bits as u64;
        *out.add(1) = u64::from(w);
        *out.add(2) = u64::from(h);
    }
    1
}

/// `FillRect` with a brush handle, or with a colour (`brush` 0).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiFillRect(hdc: H, rect: *const i32, brush: H, color: u32) -> i32 {
    if rect.is_null() {
        return 0;
    }
    // SAFETY: the caller's RECT.
    let r = unsafe { read_rect(rect) };
    let ok = gdi(|g| {
        if brush == 0 && color == poly_gdi::gdi::CLR_INVALID {
            g.fill_rect(hdc as u64, r, 0)
        } else if brush == 0 {
            g.fill_rect_color(hdc as u64, r, color)
        } else {
            g.fill_rect(hdc as u64, r, brush as u64)
        }
    });
    if !ok {
        set_last_error(ERROR_INVALID_HANDLE);
    }
    i32::from(ok)
}

/// `DrawTextW` (`DT_MODIFYSTRING` writes to `text`, as the API does though
/// the parameter is a `LPCWSTR`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyGdiDrawText(
    hdc: H,
    text: *const u16,
    count: i32,
    rect: *mut i32,
    format: u32,
) -> i32 {
    // SAFETY: as `PolyGdiDrawTextEx`.
    unsafe { PolyGdiDrawTextEx(hdc, text as *mut u16, count, rect, format, -1, 0, 0, core::ptr::null_mut()) }
}

/// `DrawTextExW`: `tab_chars` is `iTabLength` (0: the default; negative:
/// there is no `DRAWTEXTPARAMS`, and with `DT_TABSTOP` bits 8 to 15 of
/// `format` are the tab length, as in `DrawTextW`), the margins
/// narrow the rectangle for laying out the text; `drawn` (if not NULL)
/// gets `uiLengthDrawn`. `DT_MODIFYSTRING` writes the ellipsized text back
/// to `text`. A count of 0 draws nothing and gives 1; below -1, 0.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn PolyGdiDrawTextEx(
    hdc: H,
    text: *mut u16,
    count: i32,
    rect: *mut i32,
    format: u32,
    tab_chars: i32,
    left_margin: i32,
    right_margin: i32,
    drawn: *mut u32,
) -> i32 {
    const DT_MODIFYSTRING: u32 = 0x1_0000;
    if rect.is_null() || count < -1 {
        return 0;
    }
    // A count of 0 is the empty string; with text there it draws nothing.
    // SAFETY: the caller's string is readable at least for its first unit.
    if count == 0 && !text.is_null() && unsafe { *text } != 0 {
        return 1;
    }
    // SAFETY: the caller's string (count -1: NUL-terminated) and RECT.
    let (s, mut r) = unsafe { (wide(text, count), read_rect(rect)) };
    let before = r;
    let params = poly_gdi::text::DrawParams {
        tab_chars: tab_chars.max(0),
        tab_in_format: tab_chars < 0,
        left_margin,
        right_margin,
    };
    let out = gdi(|g| g.draw_text_ex(hdc as u64, &s, &mut r, format, params));
    if out.height == 0 {
        return 0;
    }
    if r != before {
        // SAFETY: as above.
        unsafe { write_rect(rect, r) };
    }
    if format & DT_MODIFYSTRING != 0
        && let Some(m) = out.modified
        && !text.is_null()
    {
        for (i, &c) in m.iter().enumerate() {
            // SAFETY: the caller's buffer holds the string and room for its ellipsis.
            unsafe { *text.add(i) = c };
        }
        // SAFETY: as above.
        unsafe { *text.add(m.len()) = 0 };
    }
    if !drawn.is_null() {
        // SAFETY: the caller's DRAWTEXTPARAMS field.
        unsafe { *drawn = out.drawn as u32 };
    }
    out.height
}

// ----- fonts added by the program, and enumeration

/// `AddFontResourceExW`'s and `AddFontMemResourceEx`'s flag bits: `FR_PRIVATE`
/// and `FR_NOT_ENUM` (`wingdi.h`).
const FR_FLAGS: u32 = 0x30;

/// `AddFontMemResourceEx`: the fonts in `data` for this process only; the
/// handle, and `*count` the number of fonts. NULL (error 87) for NULL data,
/// no length or no `count`; NULL (no error) for data that is no font.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddFontMemResourceEx(
    data: *const u8,
    len: u32,
    _reserved: *mut u8,
    count: *mut u32,
) -> H {
    if data.is_null() || len == 0 || count.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: the caller's font image of `len` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(data, len as usize) };
    match gdi(|g| g.add_font_mem(bytes)) {
        Some(h) => {
            // SAFETY: the caller's out-value.
            unsafe { *count = 1 };
            h as H
        }
        None => {
            set_last_error(0);
            0
        }
    }
}

/// `RemoveFontMemResourceEx`.
#[unsafe(no_mangle)]
pub extern "system" fn RemoveFontMemResourceEx(h: H) -> i32 {
    if h == 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let ok = gdi(|g| g.remove_font_mem(h as u64));
    if !ok {
        set_last_error(0);
    }
    i32::from(ok)
}

/// The NUL-terminated path at `name` as units with the NUL.
///
/// # Safety
/// `name` is NULL or a NUL-terminated string.
unsafe fn path_units(name: *const u16) -> Vec<u16> {
    // SAFETY: the caller's promise.
    let mut v = unsafe { wide(name, -1) };
    v.push(0);
    v
}

/// The file a font resource path (NUL-terminated) names, to tell whether
/// two paths are the same file: its full path without the NUL (the path
/// itself where the system has none for it). An add and its remove may
/// spell the file differently (relative and absolute, `/` and `\`, case).
fn font_file_id(path: &[u16]) -> Vec<u16> {
    let mut buf = alloc::vec![0u16; 520];
    for _ in 0..2 {
        // SAFETY: a NUL-terminated path and a buffer of `buf.len()` units.
        let n = unsafe { GetFullPathNameW(path.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), core::ptr::null_mut()) }
            as usize;
        if n == 0 {
            break;
        }
        if n < buf.len() {
            buf.truncate(n);
            return buf;
        }
        // Too small: `n` is the size needed, with the NUL.
        buf.resize(n + 1, 0);
    }
    path[..path.len() - 1].to_vec()
}

/// `AddFontResourceExW`: the number of fonts added (1), 0 with the last
/// error of the failed open (`ERROR_FILE_NOT_FOUND`), 87 for bad flags, none
/// for a file that is no font.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddFontResourceExW(name: *const u16, flags: u32, _reserved: *mut u8) -> i32 {
    if name.is_null() || flags & !FR_FLAGS != 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // What the caller's last error was: a call that sets none leaves it.
    let before = last_error();
    // SAFETY: a NUL-terminated path.
    let path = unsafe { path_units(name) };
    let data = match read_path(&path) {
        Ok(d) => d,
        Err(e) => {
            set_last_error(e);
            return 0;
        }
    };
    let id = font_file_id(&path);
    let ok = !data.is_empty() && gdi(|g| g.add_font_file(&id, flags, data));
    set_last_error(before);
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn AddFontResourceW(name: *const u16) -> i32 {
    // SAFETY: as `AddFontResourceExW`.
    unsafe { AddFontResourceExW(name, 0, core::ptr::null_mut()) }
}

/// `RemoveFontResourceExW`: removes one earlier add of the same file with
/// the same flags (each add needs a remove of its own). An empty path and
/// flags that are none of `FR_PRIVATE` and `FR_NOT_ENUM` are error 87; the
/// file has to be there (`ERROR_FILE_NOT_FOUND`, and the font stays); a
/// file that was not added with these flags fails without a last error.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RemoveFontResourceExW(name: *const u16, flags: u32, _reserved: *mut u8) -> i32 {
    // SAFETY: a NUL-terminated path or NULL.
    let path = unsafe { path_units(name) };
    if path.len() <= 1 || flags & !FR_FLAGS != 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let before = last_error();
    if let Err(e) = read_path(&path) {
        set_last_error(e);
        return 0;
    }
    let id = font_file_id(&path);
    let ok = gdi(|g| g.remove_font_file(&id, flags));
    set_last_error(before);
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RemoveFontResourceW(name: *const u16) -> i32 {
    // SAFETY: as `RemoveFontResourceExW`.
    unsafe { RemoveFontResourceExW(name, 0, core::ptr::null_mut()) }
}

/// `FONTENUMPROCW` (`wingdi.h`).
type FontEnumProc = unsafe extern "system" fn(*const u8, *const u8, u32, isize) -> i32;

/// `ENUMLOGFONTEXW` (`wingdi.h`): a `LOGFONTW`, the full name (64 units),
/// the style and the script (32 units each).
const ENUMLOGFONTEXW_SIZE: usize = 92 + 128 + 64 + 64;
/// `NEWTEXTMETRICEXW`: `TEXTMETRICW` (60 bytes), `ntmFlags`, `ntmSizeEM`,
/// `ntmCellHeight`, `ntmAvgWidth`, and a `FONTSIGNATURE` (24 bytes).
const NEWTEXTMETRICEXW_SIZE: usize = 60 + 16 + 24;
const TRUETYPE_FONTTYPE: u32 = 4;

fn put_name(buf: &mut [u8], at: usize, max: usize, name: &[u16]) {
    for (i, &u) in name.iter().take(max - 1).enumerate() {
        buf[at + 2 * i..at + 2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
}

/// `EnumFontFamiliesExW`: calls `proc` for each face of the family `lf`
/// names (every family if the name is empty) in the character set it
/// names; returns what the last call returned (1 if there was none), or 0
/// when a call ended the enumeration.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn EnumFontFamiliesExW(
    _hdc: H,
    lf: *const u8,
    proc: Option<FontEnumProc>,
    lparam: isize,
    _flags: u32,
) -> i32 {
    let Some(proc) = proc else {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let (name, charset) = if lf.is_null() {
        (Vec::new(), 1u8)
    } else {
        // SAFETY: a LOGFONTW (92 bytes).
        let b = unsafe { core::slice::from_raw_parts(lf, 92) };
        let units: Vec<u16> = (0..32).map(|i| u16::from_le_bytes([b[28 + 2 * i], b[29 + 2 * i]])).collect();
        let n = units.iter().position(|&c| c == 0).unwrap_or(32);
        (units[..n].to_vec(), b[23])
    };
    let faces = gdi(|g| g.enum_fonts(&name, charset));
    let mut ret = 1;
    for f in faces {
        let mut elf = [0u8; ENUMLOGFONTEXW_SIZE];
        elf[16..20].copy_from_slice(&f.weight.to_le_bytes());
        elf[20] = if f.italic { 255 } else { 0 };
        elf[27] = f.pitch_and_family();
        put_name(&mut elf, 28, 32, &f.family);
        put_name(&mut elf, 92, 64, &f.full);
        put_name(&mut elf, 92 + 128, 32, &f.style_name);
        put_name(&mut elf, 92 + 128 + 64, 32, &"Western".encode_utf16().collect::<Vec<u16>>());
        let mut ntm = [0u8; NEWTEXTMETRICEXW_SIZE];
        ntm[28..32].copy_from_slice(&f.weight.to_le_bytes());
        ntm[52] = elf[20];
        ntm[55] = f.pitch_and_family() | 4 | 2;
        // SAFETY: the caller's callback gets structures of the documented sizes.
        ret = unsafe { proc(elf.as_ptr(), ntm.as_ptr(), TRUETYPE_FONTTYPE, lparam) };
        if ret == 0 {
            break;
        }
    }
    ret
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn gdi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
