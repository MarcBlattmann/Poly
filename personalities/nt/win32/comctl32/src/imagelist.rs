//! `ImageList_*` (Learn, "Image Lists"; `commctrl.h`): the DLL side of
//! `poly_imagelist`. A list lives in this process, behind a handle that is
//! looked up in a table (a destroyed or made-up handle fails the way the
//! oracle shows: count 0, size FALSE, add -1); images come from and go to
//! GDI bitmaps and icons through gdi32 and user32.

use alloc::vec;
use alloc::vec::Vec;
use poly_imagelist::{Bitmap, CLR_DEFAULT, CLR_NONE, ILC_MASK, ILD_OVERLAYMASK, ILD_TRANSPARENT, ImageList, Shade};
use poly_win32_gui::SpinLock;

type H = usize;

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyGdiBitmapPixels(hbm: H, out: *mut u8, cap: usize) -> u64;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const u8) -> H;
    fn DeleteObject(h: H) -> i32;
    fn GetPixel(hdc: H, x: i32, y: i32) -> u32;
    fn SetPixelV(hdc: H, x: i32, y: i32, color: u32) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetIconInfo(icon: H, info: *mut u8) -> i32;
    fn CreateIconIndirect(info: *const u8) -> H;
    fn LoadImageW(instance: H, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> H;
    fn GetSysColor(index: i32) -> u32;
}

/// `COLOR_HIGHLIGHT` (`winuser.h`).
const COLOR_HIGHLIGHT: i32 = 13;
/// `IMAGE_BITMAP`, `IMAGE_ICON`.
const IMAGE_BITMAP: u32 = 0;
const IMAGE_ICON: u32 = 1;
/// `LR_CREATEDIBSECTION`.
const LR_CREATEDIBSECTION: u32 = 0x2000;

/// The handles this process's lists have (made up, never memory).
const HANDLE_BASE: usize = 0x00C0_0000;

struct Table {
    lists: Vec<(H, ImageList)>,
    next: H,
}

static TABLE: SpinLock<Table> = SpinLock::new(Table { lists: Vec::new(), next: HANDLE_BASE });

/// Run `f` on the list behind `h`; `None` when `h` is no list.
fn with<R>(h: H, f: impl FnOnce(&mut ImageList) -> R) -> Option<R> {
    TABLE.with(|t| t.lists.iter_mut().find(|(k, _)| *k == h).map(|(_, l)| f(l)))
}

fn insert(list: ImageList) -> H {
    TABLE.with(|t| {
        let h = t.next;
        t.next += 0x10;
        t.lists.push((h, list));
        h
    })
}

/// A bitmap's pixels: width, height, `B, G, R, A` rows top-down.
fn read_bitmap(hbm: H) -> Option<(i32, i32, Vec<u8>)> {
    if hbm == 0 {
        return None;
    }
    // SAFETY: a size query, then a buffer of the size answered.
    let size = unsafe { PolyGdiBitmapPixels(hbm, core::ptr::null_mut(), 0) };
    if size == 0 {
        return None;
    }
    let (w, h) = (size as u32 as i32, (size >> 32) as u32 as i32);
    let mut px = vec![0u8; (w as usize) * (h as usize) * 4];
    // SAFETY: room for w * h * 4 bytes.
    unsafe { PolyGdiBitmapPixels(hbm, px.as_mut_ptr(), px.len()) };
    Some((w, h, px))
}

/// A 32-bit bitmap of `B, G, R, A` rows (a monochrome one when `mono`).
fn make_bitmap(w: i32, h: i32, bgra: &[u8], mono: bool) -> H {
    if !mono {
        // SAFETY: gdi32 copies w * h * 4 bytes.
        return unsafe { CreateBitmap(w, h, 1, 32, bgra.as_ptr()) };
    }
    let stride = (w as usize).div_ceil(16) * 2;
    let mut bits = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = &bgra[(y * w as usize + x) * 4..];
            if p[0] != 0 || p[1] != 0 || p[2] != 0 {
                bits[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    // SAFETY: gdi32 copies the rows.
    unsafe { CreateBitmap(w, h, 1, 1, bits.as_ptr()) }
}

fn rd<T: Copy>(p: *const u8, off: usize) -> T {
    // SAFETY: the caller's structure (ICONINFO, IMAGEINFO ...) of at least `off + size_of::<T>()` bytes.
    unsafe { core::ptr::read_unaligned(p.add(off) as *const T) }
}

fn wr<T: Copy>(p: *mut u8, off: usize, v: T) {
    // SAFETY: as for `rd`.
    unsafe { core::ptr::write_unaligned(p.add(off) as *mut T, v) };
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Create(cx: i32, cy: i32, flags: u32, initial: i32, grow: i32) -> H {
    match ImageList::new(cx, cy, flags, initial, grow) {
        Some(l) => insert(l),
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Destroy(himl: H) -> i32 {
    TABLE.with(|t| {
        let before = t.lists.len();
        t.lists.retain(|(k, _)| *k != himl);
        i32::from(t.lists.len() != before)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetImageCount(himl: H) -> i32 {
    with(himl, |l| l.count()).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetImageCount(himl: H, n: u32) -> i32 {
    with(himl, |l| l.set_count(n)).map_or(0, i32::from)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImageList_GetIconSize(himl: H, cx: *mut i32, cy: *mut i32) -> i32 {
    match with(himl, |l| (l.cx, l.cy)) {
        Some((w, h)) => {
            // SAFETY: the caller's optional out parameters.
            unsafe {
                if !cx.is_null() {
                    *cx = w;
                }
                if !cy.is_null() {
                    *cy = h;
                }
            }
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetIconSize(himl: H, cx: i32, cy: i32) -> i32 {
    with(himl, |l| l.set_icon_size(cx, cy)).map_or(0, i32::from)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetBkColor(himl: H) -> u32 {
    with(himl, |l| l.bk).unwrap_or(CLR_NONE)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetBkColor(himl: H, color: u32) -> u32 {
    with(himl, |l| l.set_bk(color)).unwrap_or(CLR_NONE)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetFlags(himl: H) -> u32 {
    with(himl, |l| l.flags).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Add(himl: H, image: H, mask: H) -> i32 {
    let Some((w, h, px)) = read_bitmap(image) else { return -1 };
    let m = read_bitmap(mask);
    let c = Bitmap { w, h, bgra: &px };
    let mk = m.as_ref().map(|(w, h, b)| Bitmap { w: *w, h: *h, bgra: b });
    with(himl, |l| l.add(&c, mk.as_ref())).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_AddMasked(himl: H, image: H, mask: u32) -> i32 {
    let Some((w, h, px)) = read_bitmap(image) else { return -1 };
    let c = Bitmap { w, h, bgra: &px };
    with(himl, |l| l.add_masked(&c, mask)).unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Replace(himl: H, index: i32, image: H, mask: H) -> i32 {
    let Some((w, h, px)) = read_bitmap(image) else { return 0 };
    let m = read_bitmap(mask);
    let c = Bitmap { w, h, bgra: &px };
    let mk = m.as_ref().map(|(w, h, b)| Bitmap { w: *w, h: *h, bgra: b });
    with(himl, |l| l.replace(index, &c, mk.as_ref())).map_or(0, i32::from)
}

/// The colour and mask bitmaps of an icon, read; the GDI copies are freed.
fn icon_pixels(icon: H) -> Option<((i32, i32, Vec<u8>), (i32, i32, Vec<u8>))> {
    let mut info = [0u8; 32];
    // SAFETY: an ICONINFO of 32 bytes.
    if icon == 0 || unsafe { GetIconInfo(icon, info.as_mut_ptr()) } == 0 {
        return None;
    }
    let (mask, color): (H, H) = (rd(info.as_ptr(), 16), rd(info.as_ptr(), 24));
    let m = read_bitmap(mask);
    let c = read_bitmap(color);
    for b in [mask, color] {
        if b != 0 {
            // SAFETY: bitmaps GetIconInfo made for this call.
            unsafe { DeleteObject(b) };
        }
    }
    // A monochrome icon has only the mask: no colour picture to take.
    let c = c?;
    Some((c, m.unwrap_or((0, 0, Vec::new()))))
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_ReplaceIcon(himl: H, index: i32, icon: H) -> i32 {
    if with(himl, |_| ()).is_none() {
        return -1;
    }
    let Some((c, m)) = icon_pixels(icon) else { return -1 };
    let cb = Bitmap { w: c.0, h: c.1, bgra: &c.2 };
    let mb = Bitmap { w: m.0, h: m.1, bgra: &m.2 };
    with(himl, |l| l.replace_icon(index, &cb, &mb)).unwrap_or(-1)
}

/// `ImageList_AddIcon` is `ImageList_ReplaceIcon(himl, -1, icon)` in `commctrl.h`.
#[unsafe(no_mangle)]
pub extern "system" fn ImageList_AddIcon(himl: H, icon: H) -> i32 {
    ImageList_ReplaceIcon(himl, -1, icon)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Remove(himl: H, index: i32) -> i32 {
    with(himl, |l| l.remove(index)).map_or(0, i32::from)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_GetIcon(himl: H, index: i32, flags: u32) -> H {
    let Some((cx, cy, parts)) = with(himl, |l| (l.cx, l.cy, l.icon_parts(index, flags))) else { return 0 };
    let Some((color, mask)) = parts else { return 0 };
    let cbm = make_bitmap(cx, cy, &color, false);
    let mbm = make_bitmap(cx, cy, &mask, true);
    let mut info = [0u8; 32];
    wr(info.as_mut_ptr(), 0, 1u32);
    wr(info.as_mut_ptr(), 16, mbm);
    wr(info.as_mut_ptr(), 24, cbm);
    // SAFETY: an ICONINFO of 32 bytes with two bitmaps; the icon keeps copies.
    let icon = unsafe { CreateIconIndirect(info.as_ptr()) };
    // SAFETY: bitmaps made above.
    unsafe {
        DeleteObject(cbm);
        DeleteObject(mbm);
    }
    icon
}

/// `IMAGEINFO` (`commctrl.h`, 40 bytes on x64): `hbmImage`, `hbmMask`, two unused ints, `rcImage`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImageList_GetImageInfo(himl: H, index: i32, info: *mut u8) -> i32 {
    if info.is_null() {
        return 0;
    }
    let Some(Some((rect, strip, mask))) = with(himl, |l| {
        l.image_rect(index).map(|r| {
            let (w, h, s) = l.strip_bgra();
            let (_, _, m) = l.mask_bgra();
            (r, (w, h, s), m)
        })
    }) else {
        return 0;
    };
    // The bitmaps are made for the caller (it does not own the list's, which are not kept):
    // the callers of this function read `rcImage`, and use the bitmaps as read-only pictures.
    let hbm = make_bitmap(strip.0, strip.1, &strip.2, false);
    let hmask = make_bitmap(strip.0, strip.1, &mask, true);
    wr(info, 0, hbm);
    wr(info, 8, hmask);
    wr(info, 16, 0i32);
    wr(info, 20, 0i32);
    for (i, v) in rect.iter().enumerate() {
        wr(info, 24 + 4 * i, *v);
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImageList_GetImageRect(himl: H, index: i32, rect: *mut i32) -> i32 {
    match with(himl, |l| l.image_rect(index)).flatten() {
        Some(r) if !rect.is_null() => {
            for (i, v) in r.iter().enumerate() {
                // SAFETY: a RECT of the caller.
                unsafe { *rect.add(i) = *v };
            }
            1
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Duplicate(himl: H) -> H {
    match with(himl, |l| l.clone()) {
        Some(l) => insert(l),
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Copy(dst: H, di: i32, src: H, si: i32, flags: u32) -> i32 {
    if dst == src {
        return with(dst, |l| l.copy_within(di, si, flags)).map_or(0, i32::from);
    }
    // Two lists: work on a copy of the source, then store it back.
    let Some(mut from) = with(src, |l| l.clone()) else { return 0 };
    let ok = with(dst, |l| l.copy_from(di, &mut from, si, flags)).unwrap_or(false);
    if ok && flags & poly_imagelist::ILCF_SWAP != 0 {
        with(src, |l| *l = from);
    }
    i32::from(ok)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Merge(h1: H, i1: i32, h2: H, i2: i32, dx: i32, dy: i32) -> H {
    let Some(a) = with(h1, |l| l.clone()) else { return 0 };
    let Some(b) = with(h2, |l| l.clone()) else { return 0 };
    match ImageList::merge(&a, i1, &b, i2, dx, dy) {
        Some(m) => insert(m),
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_SetOverlayImage(himl: H, image: i32, overlay: i32) -> i32 {
    with(himl, |l| l.set_overlay(image, overlay)).map_or(0, i32::from)
}

/// Draw image `index` of `himl` into `hdc` at (`x`, `y`), `dx` by `dy` (0: the image's size).
#[allow(clippy::too_many_arguments)]
fn draw(himl: H, index: i32, hdc: H, x: i32, y: i32, dx: i32, dy: i32, bk: u32, fg: u32, flags: u32) -> i32 {
    let Some(list) = with(himl, |l| l.clone()) else { return 0 };
    if hdc == 0 || index < 0 || index >= list.count() {
        return 0;
    }
    // SAFETY: a system colour query.
    let highlight = unsafe { GetSysColor(COLOR_HIGHLIGHT) };
    let (w, h) = (if dx > 0 { dx } else { list.cx }, if dy > 0 { dy } else { list.cy });
    let mut layers = vec![(index, flags)];
    if let Some(o) = list.overlay_index(flags) {
        layers.push((o, flags & !ILD_OVERLAYMASK | ILD_TRANSPARENT));
    }
    for (i, f) in layers {
        for py in 0..h {
            for px in 0..w {
                let sx = (i64::from(px) * i64::from(list.cx) / i64::from(w)) as i32;
                let sy = (i64::from(py) * i64::from(list.cy) / i64::from(h)) as i32;
                let s = list.shade(i, sx, sy, f, bk, fg, highlight);
                put(hdc, x + px, y + py, s);
            }
        }
    }
    1
}

fn put(hdc: H, x: i32, y: i32, s: Shade) {
    let dst = if s.needs_dst() {
        // SAFETY: gdi32 checks the DC.
        let c = unsafe { GetPixel(hdc, x, y) };
        if c == u32::MAX {
            return;
        }
        c
    } else {
        0
    };
    if let Some(c) = s.apply(dst) {
        // SAFETY: gdi32 checks the DC.
        unsafe { SetPixelV(hdc, x, y, c) };
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_Draw(himl: H, index: i32, hdc: H, x: i32, y: i32, style: u32) -> i32 {
    draw(himl, index, hdc, x, y, 0, 0, CLR_DEFAULT, CLR_DEFAULT, style)
}

#[unsafe(no_mangle)]
pub extern "system" fn ImageList_DrawEx(
    himl: H,
    index: i32,
    hdc: H,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    bk: u32,
    fg: u32,
    style: u32,
) -> i32 {
    draw(himl, index, hdc, x, y, dx, dy, bk, fg, style)
}

/// `IMAGELISTDRAWPARAMS` (x64): `cbSize` 0, `himl` 8, `i` 16, `hdcDst` 24, `x` 32, `y` 36,
/// `cx` 40, `cy` 44, `xBitmap` 48, `yBitmap` 52, `rgbBk` 56, `rgbFg` 60, `fStyle` 64.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImageList_DrawIndirect(p: *const u8) -> i32 {
    if p.is_null() || rd::<u32>(p, 0) < 68 {
        return 0;
    }
    draw(rd(p, 8), rd(p, 16), rd(p, 24), rd(p, 32), rd(p, 36), rd(p, 40), rd(p, 44), rd(p, 56), rd(p, 60), rd(p, 64))
}

/// `ImageList_LoadImageW(hi, lpbmp, cx, cGrow, crMask, uType, uFlags)`: a list from a bitmap
/// (or an icon) resource or file: images `cx` wide, as many as the bitmap holds.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImageList_LoadImageW(
    hi: H,
    name: *const u16,
    cx: i32,
    grow: i32,
    mask: u32,
    kind: u32,
    flags: u32,
) -> H {
    // SAFETY: the caller's module and name.
    let img = unsafe { LoadImageW(hi, name, kind, 0, 0, flags & !LR_CREATEDIBSECTION) };
    if img == 0 {
        return 0;
    }
    let list = if kind == IMAGE_ICON {
        let Some((c, m)) = icon_pixels(img) else { return 0 };
        let Some(mut l) = ImageList::new(if cx > 0 { cx } else { c.0 }, c.1, ILC_MASK | 0x20, 1, grow) else {
            return 0;
        };
        let cb = Bitmap { w: c.0, h: c.1, bgra: &c.2 };
        let mb = Bitmap { w: m.0, h: m.1, bgra: &m.2 };
        l.replace_icon(-1, &cb, &mb);
        l
    } else if kind == IMAGE_BITMAP {
        let Some((w, h, px)) = read_bitmap(img) else { return 0 };
        let cx = if cx > 0 { cx } else { h };
        let mut f = 0x20;
        if mask != CLR_NONE {
            f |= ILC_MASK;
        }
        let Some(mut l) = ImageList::new(cx, h, f, 1, grow) else { return 0 };
        let c = Bitmap { w, h, bgra: &px };
        if mask != CLR_NONE {
            l.add_masked(&c, mask);
        } else {
            l.add(&c, None);
        }
        // SAFETY: the bitmap LoadImageW made for this call.
        unsafe { DeleteObject(img) };
        l
    } else {
        return 0;
    };
    insert(list)
}

/// `ImageList_LoadImageA`: names are the same on the two sides for a resource number; a
/// name is converted from the ANSI code page (ASCII here).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ImageList_LoadImageA(
    hi: H,
    name: *const u8,
    cx: i32,
    grow: i32,
    mask: u32,
    kind: u32,
    flags: u32,
) -> H {
    if (name as usize) < 0x1_0000 {
        // SAFETY: a resource number is passed through unchanged.
        return unsafe { ImageList_LoadImageW(hi, name as *const u16, cx, grow, mask, kind, flags) };
    }
    let mut wide: Vec<u16> = Vec::new();
    let mut i = 0usize;
    loop {
        // SAFETY: a NUL-terminated name.
        let b = unsafe { *name.add(i) };
        if b == 0 {
            break;
        }
        wide.push(u16::from(b));
        i += 1;
    }
    wide.push(0);
    // SAFETY: a NUL-terminated wide copy.
    unsafe { ImageList_LoadImageW(hi, wide.as_ptr(), cx, grow, mask, kind, flags) }
}
