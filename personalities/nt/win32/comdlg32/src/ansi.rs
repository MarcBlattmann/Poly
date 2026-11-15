//! The ANSI entry points (`GetOpenFileNameA` and the others): the structure is copied to its
//! wide twin, the wide function runs, and what it wrote comes back narrowed. Names are
//! converted with the ANSI code page (`MultiByteToWideChar`, `WideCharToMultiByte`).
//! PuTTY and several other programs import these (`windows-app-survey`).

use crate::file;
use crate::sys;
use crate::util::*;
use alloc::vec::Vec;
use poly_comdlg::cderr;

/// A NUL-terminated ANSI string as UTF-16 (no NUL), at most `max` bytes.
fn widen(p: *const u8, max: usize) -> Vec<u16> {
    if p.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string of the caller's.
    while n < max && unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    widen_n(p, n)
}

fn widen_n(p: *const u8, n: usize) -> Vec<u16> {
    if n == 0 {
        return Vec::new();
    }
    let mut out = alloc::vec![0u16; n];
    // SAFETY: `n` bytes in, room for `n` units out.
    let got = unsafe { sys::MultiByteToWideChar(0, 0, p, n as i32, out.as_mut_ptr(), n as i32) };
    out.truncate(got.max(0) as usize);
    out
}

/// A double-NUL list as UTF-16 with its NULs (and the final one).
fn widen_list(p: *const u8) -> Vec<u16> {
    if p.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    loop {
        // SAFETY: a list the caller ended with two NULs.
        let (a, b) = unsafe { (*p.add(n), *p.add(n + 1)) };
        if a == 0 && b == 0 {
            n += 2;
            break;
        }
        n += 1;
        if n > 1 << 16 {
            break;
        }
    }
    widen_n(p, n)
}

fn narrow(w: &[u16]) -> Vec<u8> {
    if w.is_empty() {
        return Vec::new();
    }
    let cap = w.len() * 2;
    let mut out = alloc::vec![0u8; cap];
    // SAFETY: `w.len()` units in, room for twice as many bytes out.
    let got = unsafe {
        sys::WideCharToMultiByte(
            0,
            0,
            w.as_ptr(),
            w.len() as i32,
            out.as_mut_ptr(),
            cap as i32,
            core::ptr::null(),
            core::ptr::null_mut(),
        )
    };
    out.truncate(got.max(0) as usize);
    out
}

fn with_nul(mut v: Vec<u16>) -> Vec<u16> {
    v.push(0);
    v
}

/// `OPENFILENAMEA` offsets are those of `OPENFILENAMEW`: pointers and numbers keep their places.
const FILTER: usize = 24;
const CUSTOM: usize = 32;
const N_CUSTOM: usize = 40;
const FILE: usize = 48;
const N_FILE: usize = 56;
const TITLE_BUF: usize = 64;
const N_TITLE_BUF: usize = 72;
const INITIAL_DIR: usize = 80;
const TITLE: usize = 88;
const FLAGS: usize = 96;
const OFFSET: usize = 100;
const DEF_EXT: usize = 104;
const TEMPLATE: usize = 128;
const INDEX: usize = 44;

fn open_a(o: *mut u8, save: bool) -> i32 {
    if o.is_null() {
        return file::run(o, save);
    }
    let size: u32 = rd(o, 0);
    if size != 152 && size != 136 {
        // Not a structure we can read the strings of: the wide function reports the size.
        return file::run(o, save);
    }
    let mut w = [0u8; 152];
    // SAFETY: the caller's structure of `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(o, w.as_mut_ptr(), size as usize) };
    let wp = w.as_mut_ptr();
    let ptr_a = |off: usize| rd::<*const u8>(o, off);
    let filter = with_nul(widen_list(ptr_a(FILTER)));
    let initial = with_nul(widen(ptr_a(INITIAL_DIR), 32768));
    let title = with_nul(widen(ptr_a(TITLE), 32768));
    let def_ext = with_nul(widen(ptr_a(DEF_EXT), 32768));
    let tname_a = ptr_a(TEMPLATE);
    let tname = if (tname_a as usize) >= 0x1_0000 { with_nul(widen(tname_a, 512)) } else { Vec::new() };
    if !ptr_a(FILTER).is_null() {
        wr(wp, FILTER, filter.as_ptr());
    }
    if !ptr_a(INITIAL_DIR).is_null() {
        wr(wp, INITIAL_DIR, initial.as_ptr());
    }
    if !ptr_a(TITLE).is_null() {
        wr(wp, TITLE, title.as_ptr());
    }
    if !ptr_a(DEF_EXT).is_null() {
        wr(wp, DEF_EXT, def_ext.as_ptr());
    }
    if !tname.is_empty() {
        wr(wp, TEMPLATE, tname.as_ptr());
    }
    // The file name buffer: in as wide text, out narrowed.
    let n_file: u32 = rd(o, N_FILE);
    let file_a = rd::<*mut u8>(o, FILE);
    let mut file_w = alloc::vec![0u16; n_file as usize];
    if !file_a.is_null() && n_file > 0 {
        let init = widen(file_a, n_file as usize - 1);
        file_w[..init.len()].copy_from_slice(&init);
        wr(wp, FILE, file_w.as_mut_ptr());
    }
    let n_title: u32 = rd(o, N_TITLE_BUF);
    let title_a = rd::<*mut u8>(o, TITLE_BUF);
    let mut title_w = alloc::vec![0u16; n_title as usize];
    if !title_a.is_null() && n_title > 0 {
        wr(wp, TITLE_BUF, title_w.as_mut_ptr());
    }
    let n_cust: u32 = rd(o, N_CUSTOM);
    let cust_a = rd::<*mut u8>(o, CUSTOM);
    let mut cust_w = alloc::vec![0u16; n_cust as usize];
    if !cust_a.is_null() && n_cust > 0 {
        let init = widen_list(cust_a);
        let k = init.len().min(n_cust as usize);
        cust_w[..k].copy_from_slice(&init[..k]);
        wr(wp, CUSTOM, cust_w.as_mut_ptr());
    }
    let r = file::run(wp, save);
    // Back: the buffers narrowed, the numbers copied.
    if !file_a.is_null() && n_file > 0 && r != 0 {
        // A list ends at its double NUL; a single name at its NUL.
        let mut end = 0;
        while end + 1 < file_w.len() && !(file_w[end] == 0 && file_w[end + 1] == 0) {
            end += 1;
        }
        let bytes = narrow(&file_w[..(end + 2).min(file_w.len())]);
        let k = bytes.len().min(n_file as usize);
        // SAFETY: the caller's buffer of `n_file` bytes.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), file_a, k) };
    }
    if !title_a.is_null() && n_title > 0 && r != 0 {
        let t: Vec<u16> = title_w.iter().copied().take_while(|&c| c != 0).collect();
        let bytes = narrow(&t);
        let k = bytes.len().min(n_title as usize - 1);
        // SAFETY: the caller's buffer of `n_title` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), title_a, k);
            *title_a.add(k) = 0;
        }
    }
    if !cust_a.is_null() && n_cust > 0 {
        let bytes = narrow(&cust_w);
        let k = bytes.len().min(n_cust as usize);
        // SAFETY: the caller's buffer of `n_cust` bytes.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), cust_a, k) };
    }
    for off in [FLAGS, INDEX] {
        wr(o, off, rd::<u32>(wp, off));
    }
    wr(o, OFFSET, rd::<u32>(wp, OFFSET));
    r
}

/// `GetOpenFileNameA(lpofn)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetOpenFileNameA(ofn: *mut u8) -> i32 {
    open_a(ofn, false)
}

/// `GetSaveFileNameA(lpofn)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSaveFileNameA(ofn: *mut u8) -> i32 {
    open_a(ofn, true)
}

/// `GetFileTitleA(file, title, size)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileTitleA(file: *const u8, title: *mut u8, size: u16) -> i16 {
    let w = with_nul(widen(file, 32768));
    let mut out = alloc::vec![0u16; usize::from(size).max(1)];
    // SAFETY: a NUL-terminated wide copy and a buffer of `size` units.
    let r = unsafe { crate::GetFileTitleW(w.as_ptr(), out.as_mut_ptr(), size) };
    if r == 0 && !title.is_null() && size > 0 {
        let t: Vec<u16> = out.iter().copied().take_while(|&c| c != 0).collect();
        let bytes = narrow(&t);
        // SAFETY: the caller's buffer of `size` bytes (the name fits: the wide check said so).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), title, bytes.len().min(usize::from(size) - 1));
            *title.add(bytes.len().min(usize::from(size) - 1)) = 0;
        }
    }
    r
}

/// `ChooseColorA(lpcc)`: the same structure; only the template name is ANSI.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ChooseColorA(cc: *mut u8) -> i32 {
    if cc.is_null() {
        crate::set_ext_error(cderr::INITIALIZATION);
        return 0;
    }
    const TEMPLATE_AT: usize = 64;
    const FLAGS_AT: usize = 40;
    let name_a = rd::<*const u8>(cc, TEMPLATE_AT);
    let flags: u32 = rd(cc, FLAGS_AT);
    if flags & poly_comdlg::check::cc::ENABLETEMPLATE != 0 && (name_a as usize) >= 0x1_0000 {
        let w = with_nul(widen(name_a, 512));
        wr(cc, TEMPLATE_AT, w.as_ptr());
        let r = crate::colordlg::run(cc);
        wr(cc, TEMPLATE_AT, name_a);
        return r;
    }
    crate::colordlg::run(cc)
}

/// `ChooseFontA(lpcf)`: the `LOGFONTA` and the style buffer are converted both ways.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ChooseFontA(cf: *mut u8) -> i32 {
    if cf.is_null() {
        crate::set_ext_error(cderr::INITIALIZATION);
        return 0;
    }
    const LOGFONT_AT: usize = 24;
    const STYLE_AT: usize = 80;
    const TEMPLATE_AT: usize = 64;
    let lf_a = rd::<*mut u8>(cf, LOGFONT_AT);
    let style_a = rd::<*mut u8>(cf, STYLE_AT);
    let name_a = rd::<*const u8>(cf, TEMPLATE_AT);
    // LOGFONTA is 60 bytes (28 of numbers, then a 32-byte face name); LOGFONTW is 92.
    let mut lf_w = [0u8; 92];
    if !lf_a.is_null() {
        // SAFETY: the caller's LOGFONTA of 60 bytes.
        unsafe { core::ptr::copy_nonoverlapping(lf_a, lf_w.as_mut_ptr(), 28) };
        let face = widen(
            // SAFETY: the face name of the LOGFONTA.
            unsafe { lf_a.add(28) },
            31,
        );
        for (i, c) in face.iter().take(31).enumerate() {
            wr(lf_w.as_mut_ptr(), 28 + 2 * i, *c);
        }
        wr(cf, LOGFONT_AT, lf_w.as_mut_ptr());
    }
    let mut style_w = [0u16; 64];
    if !style_a.is_null() {
        wr(cf, STYLE_AT, style_w.as_mut_ptr());
    }
    let tname = if (name_a as usize) >= 0x1_0000 { with_nul(widen(name_a, 512)) } else { Vec::new() };
    if !tname.is_empty() {
        wr(cf, TEMPLATE_AT, tname.as_ptr());
    }
    let r = crate::fontdlg::run(cf);
    wr(cf, LOGFONT_AT, lf_a);
    wr(cf, STYLE_AT, style_a);
    wr(cf, TEMPLATE_AT, name_a);
    if r != 0 && !lf_a.is_null() {
        // SAFETY: the caller's LOGFONTA of 60 bytes.
        unsafe { core::ptr::copy_nonoverlapping(lf_w.as_ptr(), lf_a, 28) };
        let face: Vec<u16> = (0..32).map(|i| rd::<u16>(lf_w.as_ptr(), 28 + 2 * i)).take_while(|&c| c != 0).collect();
        let bytes = narrow(&face);
        let k = bytes.len().min(31);
        // SAFETY: the face name of the LOGFONTA is 32 bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), lf_a.add(28), k);
            *lf_a.add(28 + k) = 0;
        }
    }
    if r != 0 && !style_a.is_null() {
        let t: Vec<u16> = style_w.iter().copied().take_while(|&c| c != 0).collect();
        let bytes = narrow(&t);
        // SAFETY: the program's style buffer (64 bytes by the documentation of `lpszStyle`).
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), style_a, bytes.len().min(63));
            *style_a.add(bytes.len().min(63)) = 0;
        }
    }
    r
}
