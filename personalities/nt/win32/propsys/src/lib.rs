//! Our `propsys.dll` (ADR 0155, step 12). Chrome delay-loads it; a delay load of a missing DLL
//! ends the browser. The functions that return an `HRESULT` return `E_NOTIMPL`; `VariantCompare`
//! returns its comparison: 0.

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!(
    "propsys",
    hresult,
    [InitPropVariantFromCLSID, PSCreateMemoryPropertyStore, PSGetPropertyKeyFromName,]
);

poly_win32_rt::fail_exports!("propsys", value(0), [VariantCompare,]);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn propsys_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
