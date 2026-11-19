//! The automation marshaller (ADR 0220, B2b): `oleaut32`'s class `{00020424-...}` (and the older
//! `{00020420-...}`), which makes proxies and stubs for interfaces described by a registered type
//! library.

pub(crate) mod proxy;

use crate::com::*;
use core::ffi::c_void;

/// `DllGetClassObject(rclsid, riid, ppv)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(_clsid: *const Guid, _iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    put(out, core::ptr::null_mut());
    CLASS_E_CLASSNOTAVAILABLE
}
