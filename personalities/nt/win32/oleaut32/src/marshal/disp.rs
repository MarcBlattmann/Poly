//! `IDispatch`'s four methods across the wire (WIP: not written yet; answer `E_NOTIMPL`).

use super::proxy::Proxy;
use crate::com::*;
use alloc::vec::Vec;
use core::ffi::c_void;

/// The proxy side of slots 3..=6 of an `IDispatch`-based interface.
pub(crate) fn proxy_call(_px: &Proxy, _slot: u32, _args: *const u64) -> Hresult {
    E_NOTIMPL
}

/// The stub side of slots 3..=6.
pub(crate) fn stub_call(_itf: *mut c_void, _slot: u32, _data: &[u8]) -> Result<Vec<u8>, Hresult> {
    Err(E_NOTIMPL)
}
