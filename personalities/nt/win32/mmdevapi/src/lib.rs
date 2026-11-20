//! Our `mmdevapi.dll` (audio plan W2): WASAPI shared-mode render as Chrome's audio service uses it
//! (`specs/windows/mmdevapi.dll/`, the trace in `docs/windows/wasapi-trace.md`): the device
//! enumerator (`CLSID_MMDeviceEnumerator`, served through `poly_com::inproc`), the render endpoint,
//! the audio client and its render client, clock and session. The rules are
//! `poly_kernelbase::wasapi`'s; the sound goes to the kernel's mixer through `\Device\PolyAudio`
//! (ADR 0244). The exports a program imports and Poly does not implement are fail-only stubs
//! ([`apps`]), as the generated stub DLL had them.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use com::*;
use core::ffi::c_void;
use poly_com::guid::Guid;
use poly_com::inproc::CLSID_MM_DEVICE_ENUMERATOR;
use poly_kernelbase::wasapi as wa;

poly_win32_rt::process_heap_allocator!();

mod apps;
mod client;
mod com;
mod endpoint;
mod polyaudio;

const S_FALSE: Hresult = 1;
const CLASS_E_CLASSNOTAVAILABLE: Hresult = hr(0x8004_0111);

// ----- the class object

#[repr(C)]
struct Factory {
    #[allow(dead_code, reason = "read by the caller through the object pointer")]
    vtbl: *const c_void,
}
// SAFETY: immutable.
unsafe impl Sync for Factory {}

static FACTORY_VTBL: Vtbl<5> = Vtbl([
    factory_query_interface as *const c_void,
    factory_count as *const c_void,
    factory_count as *const c_void,
    factory_create as *const c_void,
    factory_lock as *const c_void,
]);
static FACTORY: Factory = Factory { vtbl: (&FACTORY_VTBL as *const Vtbl<5>).cast() };

unsafe extern "system" fn factory_query_interface(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        if iid != IID_IUNKNOWN && iid != IID_ICLASS_FACTORY {
            return E_NOINTERFACE;
        }
        *out = p;
    }
    S_OK
}

/// The class object lives for ever: `AddRef` and `Release` count nothing.
unsafe extern "system" fn factory_count(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn factory_lock(_: *mut c_void, _: i32) -> Hresult {
    S_OK
}

/// `IClassFactory::CreateInstance`: a new enumerator; no aggregation (`CLSID_MMDeviceEnumerator.yaml`).
unsafe extern "system" fn factory_create(
    _: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter; a fresh object handed out through QueryInterface.
    unsafe {
        *out = core::ptr::null_mut();
        if !outer.is_null() {
            return hr(wa::CLASS_E_NOAGGREGATION);
        }
        hand_out::<()>(endpoint::new_enumerator(), iid, out)
    }
}

/// `DllGetClassObject` (Learn): the class object of `CLSID_MMDeviceEnumerator`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const Guid,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if clsid.is_null() || clsid.read_unaligned() != CLSID_MM_DEVICE_ENUMERATOR {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        factory_query_interface((&FACTORY as *const Factory).cast_mut().cast(), iid, out)
    }
}

/// `DllCanUnloadNow` (Learn): a system DLL stays loaded.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> Hresult {
    S_FALSE
}

/// The entry point (`DllMain`).
#[unsafe(no_mangle)]
pub extern "system" fn mmdevapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
