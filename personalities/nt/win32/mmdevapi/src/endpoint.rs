//! The endpoint objects: the enumerator (`IMMDeviceEnumerator`), its collections
//! (`IMMDeviceCollection`) and the render endpoint (`IMMDevice`). Poly has one render endpoint,
//! the sound card, when there is one (`\Device\PolyAudio` answers); no capture endpoint.

use crate::client;
use crate::com::*;
use crate::polyaudio;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, Ordering};
use poly_com::guid::Guid;
use poly_kernelbase::wasapi::{self as wa, IdKind};

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub fn CoTaskMemAlloc(bytes: usize) -> *mut u8;
}

/// The notification clients registered with any enumerator of the process (each holds a
/// reference). Poly's endpoint never changes, so none is ever called.
static NOTIFY: Locked<Vec<usize>> = Locked::new(Vec::new());

// ----- the enumerator

const ENUMERATOR_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IMM_DEVICE_ENUMERATOR];
static ENUMERATOR_VTBL: Vtbl<8> = Vtbl([
    query::<()> as *const c_void,
    add_ref::<()> as *const c_void,
    release::<()> as *const c_void,
    enum_audio_endpoints as *const c_void,
    get_default_audio_endpoint as *const c_void,
    get_device as *const c_void,
    register_notification as *const c_void,
    unregister_notification as *const c_void,
]);

/// The process's enumerator, once created; the module holds one reference on it for good.
static ENUMERATOR: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// The enumerator for a creation, with a reference for the caller (`hand_out` gives it back).
/// There is one per process, and the module keeps a reference of its own. That reference is
/// the one an application sees in the counts on Windows (`AddRef` on a freshly created
/// enumerator gives one more than the caller's own references; `CLSID_MMDeviceEnumerator.yaml`,
/// `references` and `second_creation`: whether two creations give the same object is not
/// specified). It holds no state of its own: the notification registrations are the process's.
pub fn new_enumerator() -> *mut c_void {
    let mut e = ENUMERATOR.load(Ordering::Acquire);
    if e.is_null() {
        let fresh = new_object(&ENUMERATOR_VTBL, ENUMERATOR_IIDS, ());
        e = match ENUMERATOR.compare_exchange(null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => fresh,
            Err(other) => {
                // Another thread created it first.
                // SAFETY: our fresh object, with its only reference.
                unsafe { release::<()>(fresh) };
                other
            }
        };
    }
    // SAFETY: the process's enumerator, never freed (the module's reference).
    unsafe { add_ref::<()>(e) };
    e
}

unsafe extern "system" fn enum_audio_endpoints(_: *mut c_void, flow: u32, mask: u32, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    match wa::enum_endpoints(flow, mask) {
        Err(e) => hr(e),
        Ok(render) => {
            let count = u32::from(render && polyaudio::present());
            // SAFETY: a fresh object of ours.
            unsafe {
                hand_out::<u32>(new_object(&COLLECTION_VTBL, COLLECTION_IIDS, count), &IID_IMM_DEVICE_COLLECTION, out)
            }
        }
    }
}

unsafe extern "system" fn get_default_audio_endpoint(
    _: *mut c_void,
    flow: u32,
    role: u32,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if let Err(e) = wa::default_endpoint(flow, role) {
        return hr(e);
    }
    if !polyaudio::present() {
        return hr(wa::E_NOTFOUND);
    }
    // SAFETY: the caller's out parameter.
    unsafe { new_device(out) }
}

unsafe extern "system" fn get_device(_: *mut c_void, id: *const u16, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || id.is_null() {
        if !out.is_null() {
            // SAFETY: the caller's out parameter.
            unsafe { *out = core::ptr::null_mut() };
        }
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter and NUL-terminated id.
    unsafe {
        *out = core::ptr::null_mut();
        let mut n = 0;
        while *id.add(n) != 0 && n < 512 {
            n += 1;
        }
        match wa::classify_id(core::slice::from_raw_parts(id, n)) {
            IdKind::Ours if polyaudio::present() => new_device(out),
            IdKind::Ours | IdKind::Elsewhere => hr(wa::E_NOTFOUND),
            IdKind::Malformed => E_INVALIDARG,
        }
    }
}

unsafe extern "system" fn register_notification(_: *mut c_void, client: *mut c_void) -> Hresult {
    if client.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's object; we keep a reference.
    unsafe { foreign_add_ref(client) };
    NOTIFY.with(|n| n.push(client as usize));
    S_OK
}

unsafe extern "system" fn unregister_notification(_: *mut c_void, client: *mut c_void) -> Hresult {
    if client.is_null() {
        return E_POINTER;
    }
    let found = NOTIFY.with(|n| n.iter().position(|&c| c == client as usize).map(|i| n.remove(i)));
    if found.is_some() {
        // SAFETY: the reference taken at registration.
        unsafe { foreign_release(client) };
    }
    S_OK
}

// ----- collections

const COLLECTION_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IMM_DEVICE_COLLECTION];
static COLLECTION_VTBL: Vtbl<5> = Vtbl([
    query::<u32> as *const c_void,
    add_ref::<u32> as *const c_void,
    release::<u32> as *const c_void,
    collection_count as *const c_void,
    collection_item as *const c_void,
]);

unsafe extern "system" fn collection_count(p: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our collection and the caller's out parameter.
    unsafe { *out = *data::<u32>(p) };
    S_OK
}

unsafe extern "system" fn collection_item(p: *mut c_void, i: u32, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our collection and the caller's out parameter.
    unsafe {
        *out = core::ptr::null_mut();
        if i >= *data::<u32>(p) {
            return E_INVALIDARG;
        }
        new_device(out)
    }
}

// ----- the render endpoint

const DEVICE_IIDS: &[Guid] = &[IID_IUNKNOWN, IID_IMM_DEVICE];
static DEVICE_VTBL: Vtbl<7> = Vtbl([
    query::<()> as *const c_void,
    add_ref::<()> as *const c_void,
    release::<()> as *const c_void,
    device_activate as *const c_void,
    not_impl3 as *const c_void,
    device_get_id as *const c_void,
    device_get_state as *const c_void,
]);

/// A new device object (every lookup makes one) into `out`.
///
/// # Safety
/// `out` is the caller's out parameter.
unsafe fn new_device(out: *mut *mut c_void) -> Hresult {
    // SAFETY: a fresh object of ours.
    unsafe { hand_out::<()>(new_object(&DEVICE_VTBL, DEVICE_IIDS, ()), &IID_IMM_DEVICE, out) }
}

unsafe extern "system" fn device_activate(
    _: *mut c_void,
    iid: *const Guid,
    _context: u32,
    _params: *const c_void,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let want = iid.read_unaligned();
        if want != IID_IAUDIO_CLIENT && want != IID_IAUDIO_CLIENT2 && want != IID_IAUDIO_CLIENT3 {
            return E_NOINTERFACE;
        }
        client::new_client(iid, out)
    }
}

unsafe extern "system" fn device_get_id(_: *mut c_void, out: *mut *mut u16) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    let id: Vec<u16> = wa::RENDER_ID.encode_utf16().chain(core::iter::once(0)).collect();
    // SAFETY: a CoTaskMem block for the caller, filled and handed over.
    unsafe {
        let p = CoTaskMemAlloc(id.len() * 2).cast::<u16>();
        if p.is_null() {
            *out = core::ptr::null_mut();
            return E_OUTOFMEMORY;
        }
        core::ptr::copy_nonoverlapping(id.as_ptr(), p, id.len());
        *out = p;
    }
    S_OK
}

unsafe extern "system" fn device_get_state(_: *mut c_void, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = wa::DEVICE_STATE_ACTIVE };
    S_OK
}
