//! `IDWriteNumberSubstitution` and `IDWriteFactory::CreateNumberSubstitution`
//! (`specs/windows/dwrite.dll/IDWriteFactory.CreateNumberSubstitution.yaml`).
//!
//! An opaque object: it has no methods of its own, an application hands it
//! back to DirectWrite through its text-analysis source. Poly has no text
//! analyzer, so it only keeps what it was made with.

use core::ffi::c_void;

use poly_dwrite::numsub::check;

use crate::abi::*;
use crate::com::*;
use crate::strings::wstr;

#[repr(C)]
struct NumberSubstitution {
    head: Head,
    #[allow(dead_code)]
    method: u32,
    #[allow(dead_code)]
    locale: alloc::vec::Vec<u16>,
    #[allow(dead_code)]
    ignore_user_override: bool,
}

static IIDS: [GUID; 1] = [IID_IDWriteNumberSubstitution];

static VTBL: IUnknownVtbl = IUnknownVtbl { QueryInterface: qi, AddRef: add_ref, Release: release };

unsafe extern "system" fn qi(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    // SAFETY: forwarded unchanged.
    unsafe { query_interface(this, riid, out) }
}

/// `CreateNumberSubstitution(method, localeName, ignoreUserOverride, out)`.
pub unsafe extern "system" fn create_number_substitution(
    _this: *mut c_void,
    method: u32,
    locale: *mut c_void,
    ignore_user_override: i32,
    out: *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output pointer; `locale` is a NUL-terminated
    // string or null.
    unsafe {
        if out.is_null() {
            return E_POINTER;
        }
        put::<c_void>(out, core::ptr::null_mut());
        let name: Option<&[u16]> = if locale.is_null() { None } else { Some(wstr(locale as *const u16)) };
        if check(method, name).is_err() {
            return E_INVALIDARG;
        }
        put(
            out,
            into_raw(NumberSubstitution {
                head: Head::new(&VTBL as *const _ as *const c_void, drop_box::<NumberSubstitution>, &IIDS),
                method,
                locale: name.map(|n| n.to_vec()).unwrap_or_default(),
                ignore_user_override: ignore_user_override != 0,
            }),
        );
    }
    S_OK
}
