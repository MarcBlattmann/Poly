//! `IDWriteLocalizedStrings`.

use core::ffi::c_void;

use poly_dwrite::names::{Localized, locale_malformed};

use crate::abi::*;
use crate::com::*;

#[repr(C)]
pub struct Strings {
    head: Head,
    s: Localized,
}

static IIDS: [GUID; 1] = [IID_IDWriteLocalizedStrings];

static VTBL: IDWriteLocalizedStringsVtbl = IDWriteLocalizedStringsVtbl {
    QueryInterface: qi,
    AddRef: add_ref,
    Release: release,
    GetCount: get_count,
    FindLocaleName: find_locale_name,
    GetLocaleNameLength: get_locale_name_length,
    GetLocaleName: get_locale_name,
    GetStringLength: get_string_length,
    GetString: get_string,
};

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

/// A new strings object (reference count 1).
pub fn new_strings(s: Localized) -> *mut c_void {
    into_raw(Strings { head: Head::new(&VTBL as *const _ as *const c_void, drop_box::<Strings>, &IIDS), s })
        as *mut c_void
}

unsafe fn me<'a>(this: *mut c_void) -> &'a Localized {
    // SAFETY: our object.
    unsafe { &(*(this as *const Strings)).s }
}

unsafe extern "system" fn get_count(this: *mut c_void) -> u32 {
    // SAFETY: our object.
    unsafe { me(this).entries.len() as u32 }
}

unsafe extern "system" fn find_locale_name(
    this: *mut c_void,
    name: *mut c_void,
    index: *mut c_void,
    exists: *mut c_void,
) -> HRESULT {
    // SAFETY: our object; `name` is a NUL-terminated string.
    unsafe {
        put_val(index, u32::MAX);
        put_val(exists, 0i32);
        if name.is_null() {
            return E_INVALIDARG;
        }
        let name = wstr(name as *const u16);
        if locale_malformed(name) {
            return E_INVALIDARG;
        }
        if let Some(i) = me(this).find(name) {
            put_val(index, i as u32);
            put_val(exists, 1i32);
        }
    }
    S_OK
}

/// The units of a NUL-terminated string.
pub unsafe fn wstr<'a>(p: *const u16) -> &'a [u16] {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

unsafe extern "system" fn get_locale_name_length(this: *mut c_void, i: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our object.
    unsafe {
        match me(this).entries.get(i as usize) {
            Some(e) => {
                put_val(out, e.0.len() as u32);
                S_OK
            }
            None => {
                put_val(out, u32::MAX);
                E_FAIL
            }
        }
    }
}

unsafe fn copy_out(src: &[u16], buf: *mut c_void, cap: u32) -> HRESULT {
    let out = buf as *mut u16;
    if out.is_null() || cap == 0 {
        return E_INSUFFICIENT_BUFFER;
    }
    let n = src.len().min(cap as usize - 1);
    // SAFETY: the caller's buffer holds `cap` units.
    unsafe {
        for (k, c) in src[..n].iter().enumerate() {
            *out.add(k) = *c;
        }
        *out.add(n) = 0;
    }
    if src.len() < cap as usize { S_OK } else { E_INSUFFICIENT_BUFFER }
}

unsafe extern "system" fn get_locale_name(this: *mut c_void, i: u32, buf: *mut c_void, cap: u32) -> HRESULT {
    // SAFETY: our object; the buffer holds `cap` units.
    unsafe {
        match me(this).entries.get(i as usize) {
            Some(e) => {
                let w: alloc::vec::Vec<u16> = e.0.encode_utf16().collect();
                copy_out(&w, buf, cap)
            }
            None => {
                if !buf.is_null() && cap > 0 {
                    *(buf as *mut u16) = 0;
                }
                E_FAIL
            }
        }
    }
}

unsafe extern "system" fn get_string_length(this: *mut c_void, i: u32, out: *mut c_void) -> HRESULT {
    // SAFETY: our object.
    unsafe {
        match me(this).entries.get(i as usize) {
            Some(e) => {
                put_val(out, e.1.len() as u32);
                S_OK
            }
            None => {
                put_val(out, u32::MAX);
                E_FAIL
            }
        }
    }
}

unsafe extern "system" fn get_string(this: *mut c_void, i: u32, buf: *mut c_void, cap: u32) -> HRESULT {
    // SAFETY: our object; the buffer holds `cap` units.
    unsafe {
        match me(this).entries.get(i as usize) {
            Some(e) => copy_out(&e.1, buf, cap),
            None => {
                if !buf.is_null() && cap > 0 {
                    *(buf as *mut u16) = 0;
                }
                E_FAIL
            }
        }
    }
}
