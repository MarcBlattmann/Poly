//! The clipboard (ADR 0172): `user32`'s side of `poly_win32k::clip`. The
//! kernel keeps the formats and their bytes; this module copies a block
//! in at `SetClipboardData`, builds a block out for `GetClipboardData`
//! (the setter's own handle comes back unchanged), sends the owner its
//! `WM_RENDERFORMAT` and `WM_DESTROYCLIPBOARD`, and makes bitmaps out of
//! DIBs and the reverse through `gdi32`.

use super::{H, SpinLock, call, set_last_error, wide};
use alloc::vec::Vec;
use poly_win32k::clip::{self, CF_BITMAP, CF_DIB, CF_DIBV5, CF_OEMTEXT, CF_TEXT, CF_UNICODETEXT, ClipOp, ClipRes};
use poly_win32k::dib;
use poly_win32k::proto::{Reply, Request};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> usize;
    fn GlobalFree(mem: usize) -> usize;
    fn GlobalSize(mem: usize) -> usize;
    fn GlobalLock(mem: usize) -> *mut u8;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyGdiBitmapPixels(hbm: usize, out: *mut u8, cap: usize) -> u64;
    fn CreateDIBSection(
        dc: usize,
        bmi: *const u8,
        usage: u32,
        bits: *mut *mut u8,
        section: usize,
        offset: u32,
    ) -> usize;
}

const GMEM_MOVEABLE: u32 = 0x0002;

/// Blocks made for synthesized formats and other processes' data: freed
/// when the clipboard has changed since (the system owns them).
static MADE: SpinLock<(u32, Vec<usize>)> = SpinLock::new((0, Vec::new()));

fn op(op: ClipOp) -> ClipRes {
    match call(Request::Clip { op }) {
        Reply::Clip { res } => res,
        Reply::Error { code } => ClipRes::Err { code },
        _ => ClipRes::Err { code: 87 },
    }
}

fn sequence() -> u32 {
    match op(ClipOp::Sequence {}) {
        ClipRes::Val { value } => value as u32,
        _ => 0,
    }
}

fn fail(res: ClipRes) -> bool {
    if let ClipRes::Err { code } = res {
        set_last_error(code);
        true
    } else {
        false
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn OpenClipboard(hwnd: H) -> i32 {
    if hwnd > u32::MAX as usize {
        set_last_error(1400);
        return 0;
    }
    i32::from(!fail(op(ClipOp::Open { hwnd: hwnd as u32 })))
}

#[unsafe(no_mangle)]
pub extern "system" fn CloseClipboard() -> i32 {
    i32::from(!fail(op(ClipOp::Close {})))
}

#[unsafe(no_mangle)]
pub extern "system" fn EmptyClipboard() -> i32 {
    match op(ClipOp::Empty {}) {
        ClipRes::Val { value } => {
            free_made();
            if value != 0 {
                super::SendMessageW(value as H, poly_win32k::user::WM_DESTROYCLIPBOARD, 0, 0);
            }
            1
        }
        r => {
            fail(r);
            0
        }
    }
}

/// Free the blocks made for readers if the clipboard changed.
fn free_made() {
    let seq = sequence();
    let old: Vec<usize> = MADE.with(|m| {
        if m.0 == seq {
            return Vec::new();
        }
        m.0 = seq;
        core::mem::take(&mut m.1)
    });
    for h in old {
        // SAFETY: blocks this module allocated.
        unsafe { GlobalFree(h) };
    }
}

/// The bytes of a global block.
fn block_bytes(h: usize) -> Option<Vec<u8>> {
    // SAFETY: the caller's HGLOBAL (fixed blocks in Poly).
    let n = unsafe { GlobalSize(h) };
    if n == 0 {
        return None;
    }
    // SAFETY: as above; `n` bytes.
    let p = unsafe { GlobalLock(h) };
    if p.is_null() {
        return None;
    }
    // SAFETY: the block holds `n` bytes.
    Some(unsafe { core::slice::from_raw_parts(p, n) }.to_vec())
}

fn make_block(bytes: &[u8]) -> usize {
    // SAFETY: a new block.
    let h = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) };
    if h != 0 {
        // SAFETY: `bytes.len()` bytes of the new block.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), GlobalLock(h), bytes.len()) };
        MADE.with(|m| m.1.push(h));
    }
    h
}

/// A bitmap's pixels as a packed 32-bit DIB.
pub(crate) fn bitmap_to_dib(hbm: usize) -> Option<Vec<u8>> {
    let (w, h, px) = bitmap_pixels(hbm)?;
    Some(dib::make32(w, h, &px))
}

/// A bitmap's size and its `B, G, R, A` pixels, top-down.
pub(crate) fn bitmap_pixels(hbm: usize) -> Option<(i32, i32, Vec<u8>)> {
    // SAFETY: gdi32 checks the handle.
    let size = unsafe { PolyGdiBitmapPixels(hbm, core::ptr::null_mut(), 0) };
    let (w, h) = (size as u32 as i32, (size >> 32) as u32 as i32);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut px = alloc::vec![0u8; (w as usize) * (h as usize) * 4];
    // SAFETY: `px` has the room asked.
    unsafe { PolyGdiBitmapPixels(hbm, px.as_mut_ptr(), px.len()) };
    Some((w, h, px))
}

/// A 32-bit bitmap holding a packed DIB's pixels.
fn dib_to_bitmap(d: &[u8]) -> Option<usize> {
    let (w, h, px) = dib::to_bgra(d)?;
    let mut bmi = [0u8; 40];
    bmi[0..4].copy_from_slice(&40u32.to_le_bytes());
    bmi[4..8].copy_from_slice(&w.to_le_bytes());
    bmi[8..12].copy_from_slice(&(-h).to_le_bytes());
    bmi[12..14].copy_from_slice(&1u16.to_le_bytes());
    bmi[14..16].copy_from_slice(&32u16.to_le_bytes());
    let mut bits: *mut u8 = core::ptr::null_mut();
    // SAFETY: a valid BITMAPINFOHEADER and an out pointer.
    let sec = unsafe { CreateDIBSection(0, bmi.as_ptr(), 0, &mut bits, 0, 0) };
    if sec == 0 || bits.is_null() {
        return None;
    }
    // SAFETY: the section holds w*h*4 bytes.
    unsafe { core::ptr::copy_nonoverlapping(px.as_ptr(), bits, px.len()) };
    Some(sec)
}

#[unsafe(no_mangle)]
pub extern "system" fn SetClipboardData(format: u32, mem: usize) -> usize {
    // What the kernel keeps: the block's bytes, or a bitmap's DIB. GDI
    // objects (formats 0x300..0x3FF, palettes, metafiles) are held by
    // handle only.
    let data = if mem == 0 {
        None
    } else if format == CF_BITMAP {
        Some(bitmap_to_dib(mem).unwrap_or_default())
    } else if (0x300..=0x3FF).contains(&format) || format == 3 || format == 9 || format == 14 {
        Some(Vec::new())
    } else {
        match block_bytes(mem) {
            Some(mut b) => {
                // A text block without its terminating NUL has its last unit
                // replaced by NUL, in the caller's block (GetClipboardData.yaml).
                let unit = if format == CF_UNICODETEXT {
                    2
                } else if format == CF_TEXT || format == CF_OEMTEXT {
                    1
                } else {
                    0
                };
                if unit != 0 && b.len() >= unit && b[b.len() - unit..].iter().any(|&x| x != 0) {
                    let n = b.len();
                    for x in &mut b[n - unit..] {
                        *x = 0;
                    }
                    // SAFETY: the block holds `n` bytes.
                    unsafe { core::ptr::write_bytes(GlobalLock(mem).add(n - unit), 0, unit) };
                }
                Some(b)
            }
            // An empty block is refused with the error untouched.
            None => return 0,
        }
    };
    match op(ClipOp::Set { format, handle: mem as u64, data }) {
        ClipRes::Ok {} => {
            if mem != 0 {
                set_last_error(0);
            }
            mem
        }
        ClipRes::Err { code } => {
            if mem != 0 {
                set_last_error(code);
            }
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardData(format: u32) -> usize {
    free_made();
    let mut rendered = false;
    loop {
        match op(ClipOp::Get { format }) {
            ClipRes::Data { handle, same, from, bytes } => {
                set_last_error(0);
                if same {
                    return handle as usize;
                }
                return build(format, from, &bytes);
            }
            ClipRes::Render { owner, format: f } => {
                if rendered {
                    op(ClipOp::RenderDone { format: f });
                    set_last_error(0);
                    return 0;
                }
                rendered = true;
                super::SendMessageW(owner as H, poly_win32k::user::WM_RENDERFORMAT, f as usize, 0);
                op(ClipOp::RenderDone { format: f });
            }
            ClipRes::Nothing {} => {
                set_last_error(0);
                return 0;
            }
            r => {
                fail(r);
                return 0;
            }
        }
    }
}

/// The block (or bitmap) `format` made from `bytes` of format `from`.
fn build(format: u32, from: u32, bytes: &[u8]) -> usize {
    let is_text = |f: u32| matches!(f, CF_TEXT | CF_OEMTEXT | CF_UNICODETEXT);
    if is_text(format) && is_text(from) {
        return make_block(&clip::convert_text(from, format, bytes));
    }
    match (format, from) {
        (CF_BITMAP, _) => {
            let dib = if from == CF_DIBV5 { dib::from_v5(bytes) } else { Some(bytes.to_vec()) };
            dib.and_then(|d| dib_to_bitmap(&d)).unwrap_or_default()
        }
        (CF_DIB, CF_BITMAP) => make_block(bytes),
        (CF_DIB, CF_DIBV5) => dib::from_v5(bytes).map_or(0, |d| make_block(&d)),
        (CF_DIBV5, CF_BITMAP | CF_DIB) => dib::to_v5(bytes).map_or(0, |d| make_block(&d)),
        _ => make_block(bytes),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn IsClipboardFormatAvailable(format: u32) -> i32 {
    match op(ClipOp::Available { format }) {
        ClipRes::Val { value } => value as i32,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn CountClipboardFormats() -> i32 {
    match op(ClipOp::Count {}) {
        ClipRes::Val { value } => value as i32,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn EnumClipboardFormats(prev: u32) -> u32 {
    match op(ClipOp::Enum { prev }) {
        ClipRes::Val { value } => {
            set_last_error(0);
            value as u32
        }
        r => {
            fail(r);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterClipboardFormatW(name: *const u16) -> u32 {
    if name.is_null() {
        set_last_error(87);
        return 0;
    }
    // SAFETY: a NUL-terminated name.
    let n = unsafe { wide(name, -1) };
    match op(ClipOp::Register { name: n }) {
        ClipRes::Val { value } => value as u32,
        r => {
            fail(r);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClipboardFormatNameW(format: u32, buf: *mut u16, max: i32) -> i32 {
    if max < 0 {
        set_last_error(998);
        return 0;
    }
    match op(ClipOp::Name { id: format }) {
        ClipRes::Name { name } => {
            if max <= 1 || buf.is_null() {
                set_last_error(122);
                return 0;
            }
            let n = name.len().min(max as usize - 1);
            // SAFETY: `max` units of the caller's buffer.
            unsafe {
                core::ptr::copy_nonoverlapping(name.as_ptr(), buf, n);
                *buf.add(n) = 0;
            }
            n as i32
        }
        r => {
            fail(r);
            0
        }
    }
}

fn val(o: ClipOp) -> u64 {
    match op(o) {
        ClipRes::Val { value } => value,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardOwner() -> H {
    val(ClipOp::Owner {}) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn GetOpenClipboardWindow() -> H {
    val(ClipOp::OpenWindow {}) as H
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardSequenceNumber() -> u32 {
    val(ClipOp::Sequence {}) as u32
}

fn listener(hwnd: H, add: bool) -> i32 {
    if hwnd > u32::MAX as usize {
        set_last_error(1400);
        return 0;
    }
    match op(ClipOp::Listener { hwnd: hwnd as u32, add }) {
        ClipRes::Ok {} => 1,
        // The desktop window: 5, the last error left as it was.
        ClipRes::Val { value } => value as i32,
        r => {
            fail(r);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AddClipboardFormatListener(hwnd: H) -> i32 {
    listener(hwnd, true)
}

#[unsafe(no_mangle)]
pub extern "system" fn RemoveClipboardFormatListener(hwnd: H) -> i32 {
    listener(hwnd, false)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPriorityClipboardFormat(list: *const u32, count: i32) -> i32 {
    let list: Vec<u32> = if list.is_null() || count <= 0 {
        Vec::new()
    } else {
        // SAFETY: `count` formats of the caller.
        unsafe { core::slice::from_raw_parts(list, count as usize) }.to_vec()
    };
    val(ClipOp::Priority { list }) as u32 as i32
}
