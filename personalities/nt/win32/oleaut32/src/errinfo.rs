//! Error information (Learn: "CreateErrorInfo", "SetErrorInfo", "GetErrorInfo", "IErrorInfo",
//! "ICreateErrorInfo"): one object per error with a GUID, a source, a description, a help file
//! and a help context; one per thread, which `GetErrorInfo` hands over and clears. `ole32`'s
//! functions of these names come here. The marshaller carries it across processes
//! (sheet `DllGetClassObject`, rule `error_info`).

use crate::com::*;
use alloc::boxed::Box;
use alloc::string::String;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

pub(crate) const IID_IERRORINFO: Guid =
    Guid { data1: 0x1CF2_B120, data2: 0x547D, data3: 0x101B, data4: [0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19] };
pub(crate) const IID_ICREATEERRORINFO: Guid =
    Guid { data1: 0x22F0_3340, data2: 0x547D, data3: 0x101B, data4: [0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1, 0x19] };

/// What an error object holds.
#[derive(Clone, Default)]
pub(crate) struct ErrorData {
    pub guid: Guid,
    pub source: Option<String>,
    pub description: Option<String>,
    pub help_file: Option<String>,
    pub help_context: u32,
}

#[repr(C)]
struct ErrorObject {
    ei: *const Vtbl<8>,
    cei: *const Vtbl<8>,
    refs: AtomicU32,
    data: Locked<ErrorData>,
}

const PTR: usize = core::mem::size_of::<usize>();

/// A new error object (as its `IErrorInfo`, one reference).
pub(crate) fn new_error(data: ErrorData) -> *mut c_void {
    Box::into_raw(Box::new(ErrorObject { ei: &EI_VTBL, cei: &CEI_VTBL, refs: AtomicU32::new(1), data: Locked::new(data) }))
        .cast()
}

/// What an `IErrorInfo` (ours or another's) says.
pub(crate) fn read(ei: *mut c_void) -> ErrorData {
    let mut d = ErrorData::default();
    vcall!(ei, 3, (*mut Guid) -> Hresult, &mut d.guid);
    let get = |slot: usize| -> Option<String> {
        let mut b: *mut u16 = core::ptr::null_mut();
        vcall!(ei, slot, (*mut *mut u16) -> Hresult, &mut b);
        // SAFETY: a BSTR the call returned (or NULL), freed here.
        let s = unsafe { bstr_text(b) };
        // SAFETY: as above.
        unsafe { crate::SysFreeString(b) };
        s
    };
    d.source = get(4);
    d.description = get(5);
    d.help_file = get(6);
    vcall!(ei, 7, (*mut u32) -> Hresult, &mut d.help_context);
    d
}

fn obj(this: *mut c_void, second: bool) -> *mut ErrorObject {
    (this as usize - if second { PTR } else { 0 }) as *mut ErrorObject
}

unsafe fn qi(o: *mut ErrorObject, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; a live object.
    unsafe {
        let g = *iid;
        *out = if g == IID_IUNKNOWN || g == IID_IERRORINFO {
            o.cast()
        } else if g == IID_ICREATEERRORINFO {
            (o as usize + PTR) as *mut c_void
        } else {
            *out = core::ptr::null_mut();
            return E_NOINTERFACE;
        };
        (*o).refs.fetch_add(1, Ordering::AcqRel);
    }
    S_OK
}
unsafe fn rel(o: *mut ErrorObject) -> u32 {
    // SAFETY: a live object.
    let n = unsafe { (*o).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: the last reference of a block from `new_error`.
        drop(unsafe { Box::from_raw(o) });
    }
    n
}
unsafe extern "system" fn ei_qi(t: *mut c_void, i: *const Guid, o: *mut *mut c_void) -> Hresult {
    // SAFETY: our IErrorInfo.
    unsafe { qi(obj(t, false), i, o) }
}
unsafe extern "system" fn ei_add_ref(t: *mut c_void) -> u32 {
    // SAFETY: a live object.
    unsafe { (*obj(t, false)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}
unsafe extern "system" fn ei_release(t: *mut c_void) -> u32 {
    // SAFETY: our IErrorInfo.
    unsafe { rel(obj(t, false)) }
}
unsafe extern "system" fn cei_qi(t: *mut c_void, i: *const Guid, o: *mut *mut c_void) -> Hresult {
    // SAFETY: our ICreateErrorInfo.
    unsafe { qi(obj(t, true), i, o) }
}
unsafe extern "system" fn cei_add_ref(t: *mut c_void) -> u32 {
    // SAFETY: a live object.
    unsafe { (*obj(t, true)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}
unsafe extern "system" fn cei_release(t: *mut c_void) -> u32 {
    // SAFETY: our ICreateErrorInfo.
    unsafe { rel(obj(t, true)) }
}
fn with(t: *mut c_void, second: bool, f: impl FnOnce(&mut ErrorData)) -> Hresult {
    // SAFETY: a live object.
    unsafe { (*obj(t, second)).data.with(f) };
    S_OK
}
unsafe extern "system" fn ei_get_guid(t: *mut c_void, g: *mut Guid) -> Hresult {
    if g.is_null() {
        return E_INVALIDARG;
    }
    with(t, false, |d| put(g, d.guid))
}
fn out_str(p: *mut *mut u16, s: &Option<String>) {
    put(p, s.as_deref().map_or(core::ptr::null_mut(), bstr));
}
unsafe extern "system" fn ei_get_source(t: *mut c_void, p: *mut *mut u16) -> Hresult {
    with(t, false, |d| out_str(p, &d.source))
}
unsafe extern "system" fn ei_get_description(t: *mut c_void, p: *mut *mut u16) -> Hresult {
    with(t, false, |d| out_str(p, &d.description))
}
unsafe extern "system" fn ei_get_help_file(t: *mut c_void, p: *mut *mut u16) -> Hresult {
    with(t, false, |d| out_str(p, &d.help_file))
}
unsafe extern "system" fn ei_get_help_context(t: *mut c_void, p: *mut u32) -> Hresult {
    with(t, false, |d| put(p, d.help_context))
}
unsafe extern "system" fn cei_set_guid(t: *mut c_void, g: *const Guid) -> Hresult {
    if g.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    let g = unsafe { *g };
    with(t, true, |d| d.guid = g)
}
unsafe extern "system" fn cei_set_source(t: *mut c_void, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let s = unsafe { from_wide(s) };
    with(t, true, |d| d.source = s)
}
unsafe extern "system" fn cei_set_description(t: *mut c_void, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let s = unsafe { from_wide(s) };
    with(t, true, |d| d.description = s)
}
unsafe extern "system" fn cei_set_help_file(t: *mut c_void, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let s = unsafe { from_wide(s) };
    with(t, true, |d| d.help_file = s)
}
unsafe extern "system" fn cei_set_help_context(t: *mut c_void, c: u32) -> Hresult {
    with(t, true, |d| d.help_context = c)
}

static EI_VTBL: Vtbl<8> = Vtbl([
    ei_qi as *const (),
    ei_add_ref as *const (),
    ei_release as *const (),
    ei_get_guid as *const (),
    ei_get_source as *const (),
    ei_get_description as *const (),
    ei_get_help_file as *const (),
    ei_get_help_context as *const (),
]);
static CEI_VTBL: Vtbl<8> = Vtbl([
    cei_qi as *const (),
    cei_add_ref as *const (),
    cei_release as *const (),
    cei_set_guid as *const (),
    cei_set_source as *const (),
    cei_set_description as *const (),
    cei_set_help_file as *const (),
    cei_set_help_context as *const (),
]);

// ----- the thread's error object -----------------------------------------------------------------

static SLOT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

fn slot() -> Option<u32> {
    let s = SLOT.load(Ordering::Acquire);
    if s != u32::MAX {
        return Some(s);
    }
    // SAFETY: kernel32's TLS.
    let n = unsafe { TlsAlloc() };
    if n == u32::MAX {
        return None;
    }
    match SLOT.compare_exchange(u32::MAX, n, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(n),
        Err(other) => Some(other),
    }
}

/// Replace the thread's object (`p` gets a reference of its own; NULL clears).
pub(crate) fn set(p: *mut c_void) {
    let Some(s) = slot() else { return };
    add_ref(p);
    // SAFETY: kernel32's TLS: our slot holds a referenced object or NULL.
    let old = unsafe { TlsGetValue(s) };
    // SAFETY: as above.
    unsafe { TlsSetValue(s, p) };
    release(old);
}

/// Take the thread's object (the caller owns its reference), clearing it.
pub(crate) fn take() -> Option<*mut c_void> {
    let s = slot()?;
    // SAFETY: as above.
    let p = unsafe { TlsGetValue(s) };
    if p.is_null() {
        return None;
    }
    // SAFETY: as above.
    unsafe { TlsSetValue(s, core::ptr::null_mut()) };
    Some(p)
}

/// The thread's object, with a reference for the caller, left set.
pub(crate) fn peek() -> Option<*mut c_void> {
    let s = slot()?;
    // SAFETY: as above.
    let p = unsafe { TlsGetValue(s) };
    if p.is_null() {
        return None;
    }
    add_ref(p);
    Some(p)
}

/// `CreateErrorInfo(ppcerrinfo)`: a new error object as its `ICreateErrorInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateErrorInfo(out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let p = new_error(ErrorData::default());
    put(out, (p as usize + PTR) as *mut c_void);
    S_OK
}

/// `SetErrorInfo(dwReserved, perrinfo)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetErrorInfo(_reserved: u32, info: *mut c_void) -> Hresult {
    set(info);
    S_OK
}

/// `GetErrorInfo(dwReserved, pperrinfo)`: the thread's object (cleared), or `S_FALSE` and NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetErrorInfo(_reserved: u32, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    match take() {
        Some(p) => {
            put(out, p);
            S_OK
        }
        None => {
            put(out, core::ptr::null_mut());
            S_FALSE
        }
    }
}
