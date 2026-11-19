//! Our `propsys.dll` (ADR 0155, step 12). Chrome delay-loads it; a delay load of a missing DLL
//! ends the browser. The functions that return an `HRESULT` return `E_NOTIMPL`; `VariantCompare`
//! returns its comparison: 0.

#![no_std]

use poly_win32_rt as _;

poly_win32_rt::fail_exports!("propsys", hresult, [PSCreateMemoryPropertyStore, PSGetPropertyKeyFromName,]);

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> *mut u8;
}

/// `VT_CLSID` (`wtypes.h`).
const VT_CLSID: u16 = 72;
/// `E_INVALIDARG`, `E_OUTOFMEMORY` (`winerror.h`).
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000E_u32 as i32;

/// `InitPropVariantFromCLSID(clsid, ppropvar)` (`propvarutil.h`; sheet
/// `specs/windows/propsys.dll/InitPropVariantFromCLSID.yaml`): a `VT_CLSID` variant owning a copy
/// of the GUID in a 16-byte `CoTaskMemAlloc` block. Only the type and the pointer are written (the
/// reserved words and the rest of the variant stay as they were). Windows crashes on NULL
/// pointers; Poly answers `E_INVALIDARG`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitPropVariantFromCLSID(clsid: *const u8, variant: *mut u8) -> i32 {
    if clsid.is_null() || variant.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a fresh 16-byte block; the caller's GUID and PROPVARIANT (the type at 0, the value at 8).
    unsafe {
        let p = CoTaskMemAlloc(16);
        if p.is_null() {
            return E_OUTOFMEMORY;
        }
        core::ptr::copy_nonoverlapping(clsid, p, 16);
        variant.cast::<u16>().write_unaligned(VT_CLSID);
        variant.add(8).cast::<*mut u8>().write_unaligned(p);
    }
    0
}

poly_win32_rt::fail_exports!("propsys", value(0), [VariantCompare,]);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn propsys_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod apps;
