//! Our `ole32.dll` (ADR 0155): what Chrome's headless browser asks of COM
//! and the Windows Runtime on a system with no registered classes.
//!
//! - COM apartments: `CoInitialize[Ex]`, `CoUninitialize`,
//!   `CoGetApartmentType`, `OleInitialize`, `OleUninitialize`. The state
//!   is per thread, in a TLS slot ([`poly_com::apartment`] decides).
//! - Activation: every class is unregistered, so `CoCreateInstance[Ex]` and
//!   `CoGetClassObject` fail with `REGDB_E_CLASSNOTREG` (or
//!   `CO_E_NOTINITIALIZED` on a thread with no apartment) and clear their
//!   out parameters.
//! - Task memory (`CoTaskMemAlloc`, `CoTaskMemRealloc`, `CoTaskMemFree`) is
//!   the process heap's, GUID text and `CoCreateGuid`.
//! - The Windows Runtime (`api-ms-win-core-winrt-*` resolve here, see
//!   `poly_ldr::apiset`): `RoInitialize`/`RoUninitialize` (the same
//!   apartment), `RoGetActivationFactory` and `RoActivateInstance` (no class
//!   is registered) and the `HSTRING` functions ([`poly_com::hstring`]).
//! - Small stand-ins that succeed without effect: `CoInitializeSecurity`,
//!   `CoRegisterMessageFilter`, `RegisterDragDrop`/`RevokeDragDrop`,
//!   `CoCreateFreeThreadedMarshaler` (an object that is only `IUnknown`).
//!
//! Sources: Microsoft Learn's page for each function, `combaseapi.h`,
//! `objbase.h`, `roapi.h`, `winstring.h`, `propidl.h`, `wtypes.h`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::apartment::{APTTYPEQUALIFIER_NONE, Apartment, MainSta, Model, coinit_is_valid};
use poly_com::propvariant::{self, Elem, Kind};
use poly_com::guid::{GUID_TEXT_LEN, Guid};
use poly_com::hresult::*;
use poly_com::hstring::{self, HString, Header};
use poly_win32_rt as _;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapReAlloc(heap: *mut u8, flags: u32, p: *mut u8, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn TlsAlloc() -> u32;
    fn TlsFree(index: u32) -> i32;
    fn TlsGetValue(index: u32) -> *mut u8;
    fn TlsSetValue(index: u32, value: *mut u8) -> i32;
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "bcryptprimitives", kind = "raw-dylib")]
unsafe extern "system" {
    fn ProcessPrng(buf: *mut u8, len: usize) -> i32;
}

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SysFreeString(s: *mut u16);
    fn SysAllocStringByteLen(s: *const u8, len: u32) -> *mut u16;
    fn SysStringByteLen(s: *const u16) -> u32;
    fn SafeArrayDestroy(psa: *mut u8) -> u32;
}

/// `poly-com` links `alloc`: the process heap (16-byte aligned).
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments
// are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

// ----- task memory ---------------------------------------------------------------------

/// `CoTaskMemAlloc`: `cb` bytes from the process heap; NULL for 0 bytes is
/// not returned (Learn: a request for 0 bytes yields a valid pointer).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoTaskMemAlloc(cb: usize) -> *mut c_void {
    // SAFETY: the process heap.
    unsafe { HeapAlloc(GetProcessHeap(), 0, cb.max(1)).cast() }
}

/// `CoTaskMemRealloc`: a NULL `pv` allocates; a size of 0 frees and returns NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoTaskMemRealloc(pv: *mut c_void, cb: usize) -> *mut c_void {
    // SAFETY: the process heap, blocks from CoTaskMemAlloc.
    unsafe {
        if pv.is_null() {
            return CoTaskMemAlloc(cb);
        }
        if cb == 0 {
            HeapFree(GetProcessHeap(), 0, pv.cast());
            return core::ptr::null_mut();
        }
        HeapReAlloc(GetProcessHeap(), 0, pv.cast(), cb).cast()
    }
}

/// `CoTaskMemFree`: NULL is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoTaskMemFree(pv: *mut c_void) {
    if !pv.is_null() {
        // SAFETY: a block from CoTaskMemAlloc.
        unsafe { HeapFree(GetProcessHeap(), 0, pv.cast()) };
    }
}

// ----- apartments ----------------------------------------------------------------------

/// The TLS slot of the apartment word (allocated on first use).
static SLOT: AtomicU32 = AtomicU32::new(u32::MAX);
const TLS_OUT_OF_INDEXES: u32 = u32::MAX;

fn slot() -> Option<u32> {
    let s = SLOT.load(Ordering::Acquire);
    if s != TLS_OUT_OF_INDEXES {
        return Some(s);
    }
    // SAFETY: kernel32's TLS.
    let n = unsafe { TlsAlloc() };
    if n == TLS_OUT_OF_INDEXES {
        return None;
    }
    match SLOT.compare_exchange(TLS_OUT_OF_INDEXES, n, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(n),
        Err(other) => {
            // SAFETY: the slot just allocated, unused.
            unsafe { TlsFree(n) };
            Some(other)
        }
    }
}

fn apartment() -> Apartment {
    match slot() {
        // SAFETY: kernel32's TLS.
        Some(s) => Apartment::from_word(unsafe { TlsGetValue(s) } as usize),
        None => Apartment::new(),
    }
}

fn set_apartment(a: Apartment) {
    if let Some(s) = slot() {
        // SAFETY: kernel32's TLS.
        unsafe { TlsSetValue(s, a.to_word() as *mut u8) };
    }
}

/// The process's main STA (`APTTYPE_MAINSTA`): the first thread that made a single-threaded apartment.
static MAIN_STA: MainSta = MainSta::new();

/// The TLS slot of the thread's message filter (`CoRegisterMessageFilter`).
static FILTER_SLOT: AtomicU32 = AtomicU32::new(u32::MAX);

fn filter_slot() -> Option<u32> {
    let s = FILTER_SLOT.load(Ordering::Acquire);
    if s != TLS_OUT_OF_INDEXES {
        return Some(s);
    }
    // SAFETY: kernel32's TLS.
    let n = unsafe { TlsAlloc() };
    if n == TLS_OUT_OF_INDEXES {
        return None;
    }
    match FILTER_SLOT.compare_exchange(TLS_OUT_OF_INDEXES, n, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(n),
        Err(other) => {
            // SAFETY: the slot just allocated, unused.
            unsafe { TlsFree(n) };
            Some(other)
        }
    }
}

/// `CoInitializeEx` / `RoInitialize` / `OleInitialize` on this thread.
fn initialize(model: Model) -> Hresult {
    if slot().is_none() {
        return E_OUTOFMEMORY;
    }
    let mut a = apartment();
    let hr = a.initialize(model);
    if hr >= 0 {
        set_apartment(a);
        if hr == S_OK && model == Model::Sta {
            // SAFETY: no arguments.
            MAIN_STA.claim(unsafe { GetCurrentThreadId() });
        }
    }
    hr
}

fn uninitialize() {
    let mut a = apartment();
    a.uninitialize();
    set_apartment(a);
    if a.model().is_none() {
        // SAFETY: no arguments.
        MAIN_STA.release(unsafe { GetCurrentThreadId() });
        // The apartment's message filter goes with it.
        if let Some(f) = filter_slot() {
            // SAFETY: kernel32's TLS.
            unsafe {
                let old = TlsGetValue(f);
                if !old.is_null() {
                    TlsSetValue(f, core::ptr::null_mut());
                    release_unknown(old.cast());
                }
            }
        }
    }
}

/// `IUnknown::Release` (vtable slot 2) of a COM object.
unsafe fn release_unknown(p: *mut c_void) {
    // SAFETY: the caller's COM object.
    unsafe {
        let vtbl = *(p as *const *const unsafe extern "system" fn(*mut c_void) -> u32);
        (*vtbl.add(2))(p);
    }
}

/// `IUnknown::AddRef` (vtable slot 1) of a COM object.
unsafe fn add_ref_unknown(p: *mut c_void) {
    // SAFETY: the caller's COM object.
    unsafe {
        let vtbl = *(p as *const *const unsafe extern "system" fn(*mut c_void) -> u32);
        (*vtbl.add(1))(p);
    }
}

/// `CoInitializeEx`: `dwCoInit` may only hold the defined bits and `pvReserved` must be NULL
/// (`E_INVALIDARG`; the thread stays as it was).
#[unsafe(no_mangle)]
pub extern "system" fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> Hresult {
    if !reserved.is_null() || !coinit_is_valid(co_init) {
        return E_INVALIDARG;
    }
    initialize(Model::from_coinit(co_init))
}

#[unsafe(no_mangle)]
pub extern "system" fn CoInitialize(reserved: *mut c_void) -> Hresult {
    if !reserved.is_null() {
        return E_INVALIDARG;
    }
    initialize(Model::Sta)
}

#[unsafe(no_mangle)]
pub extern "system" fn CoUninitialize() {
    uninitialize();
}

#[unsafe(no_mangle)]
pub extern "system" fn OleInitialize(_reserved: *mut c_void) -> Hresult {
    initialize(Model::Sta)
}

#[unsafe(no_mangle)]
pub extern "system" fn OleUninitialize() {
    uninitialize();
}

/// `CoGetApartmentType`: `CO_E_NOTINITIALIZED` when the thread has none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> Hresult {
    if kind.is_null() || qualifier.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: no arguments.
    let main = MAIN_STA.is_main(unsafe { GetCurrentThreadId() });
    match apartment().apartment_type_in(main) {
        Some((t, q)) => {
            // SAFETY: the caller's out parameters.
            unsafe {
                *kind = t;
                *qualifier = q;
            }
            S_OK
        }
        None => {
            // SAFETY: the caller's out parameters.
            unsafe {
                *kind = 0;
                *qualifier = APTTYPEQUALIFIER_NONE;
            }
            CO_E_NOTINITIALIZED
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RoInitialize(init_type: i32) -> Hresult {
    initialize(Model::from_ro(init_type))
}

#[unsafe(no_mangle)]
pub extern "system" fn RoUninitialize() {
    uninitialize();
}

// ----- activation ----------------------------------------------------------------------

use poly_com::apartment::activation_failure as activation_result;

/// `CoGetClassObject`: no class is registered.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetClassObject(
    _clsid: *const Guid,
    _context: u32,
    _server_info: *mut c_void,
    _iid: *const Guid,
    ppv: *mut *mut c_void,
) -> Hresult {
    if ppv.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *ppv = core::ptr::null_mut() };
    activation_result(apartment().model().is_some(), false)
}

/// `CoCreateInstance`: no class is registered.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoCreateInstance(
    _clsid: *const Guid,
    outer: *mut c_void,
    _context: u32,
    _iid: *const Guid,
    ppv: *mut *mut c_void,
) -> Hresult {
    if ppv.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *ppv = core::ptr::null_mut() };
    activation_result(apartment().model().is_some(), !outer.is_null())
}

/// `MULTI_QI` (`objidl.h`).
#[repr(C)]
pub struct MultiQi {
    iid: *const Guid,
    itf: *mut c_void,
    hr: Hresult,
}

/// `CoCreateInstanceEx`: every requested interface fails the same way.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoCreateInstanceEx(
    clsid: *const Guid,
    outer: *mut c_void,
    context: u32,
    _server_info: *mut c_void,
    count: u32,
    results: *mut MultiQi,
) -> Hresult {
    if results.is_null() || count == 0 {
        return E_INVALIDARG;
    }
    let mut unused: *mut c_void = core::ptr::null_mut();
    // SAFETY: as CoCreateInstance.
    let hr = unsafe { CoCreateInstance(clsid, outer, context, core::ptr::null(), &mut unused) };
    for i in 0..count as usize {
        // SAFETY: `count` entries of the caller.
        unsafe {
            let q = results.add(i);
            (*q).itf = core::ptr::null_mut();
            (*q).hr = hr;
        }
    }
    hr
}

/// `RoGetActivationFactory`: no Windows Runtime class is registered.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RoGetActivationFactory(
    _class_id: HString,
    _iid: *const Guid,
    factory: *mut *mut c_void,
) -> Hresult {
    if factory.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *factory = core::ptr::null_mut() };
    REGDB_E_CLASSNOTREG
}

/// `RoActivateInstance`: as above.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RoActivateInstance(_class_id: HString, instance: *mut *mut c_void) -> Hresult {
    if instance.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *instance = core::ptr::null_mut() };
    REGDB_E_CLASSNOTREG
}

// ----- objects that do nothing ---------------------------------------------------------

/// The IUnknown-only object `CoCreateFreeThreadedMarshaler` hands out.
#[repr(C)]
struct Unknown {
    vtbl: *const UnknownVtbl,
    refs: AtomicU32,
}

#[repr(C)]
struct UnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut Unknown, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut Unknown) -> u32,
    release: unsafe extern "system" fn(*mut Unknown) -> u32,
}

// SAFETY: a table of function pointers.
unsafe impl Sync for UnknownVtbl {}

/// `IID_IUnknown` (`unknwn.h`): {00000000-0000-0000-C000-000000000046}.
const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

static UNKNOWN_VTBL: UnknownVtbl = UnknownVtbl { query_interface: unk_qi, add_ref: unk_add_ref, release: unk_release };

unsafe extern "system" fn unk_qi(this: *mut Unknown, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID and out parameter.
    unsafe {
        if !iid.is_null() && *iid == IID_IUNKNOWN {
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            *out = this.cast();
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn unk_add_ref(this: *mut Unknown) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn unk_release(this: *mut Unknown) -> u32 {
    // SAFETY: a live object from CoTaskMemAlloc.
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            CoTaskMemFree(this.cast());
        }
        n
    }
}

/// `CoCreateFreeThreadedMarshaler`: an object that is `IUnknown` and no
/// more (there are no other processes to marshal to that it could serve).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoCreateFreeThreadedMarshaler(_outer: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter; a fresh block.
    unsafe {
        let p = CoTaskMemAlloc(core::mem::size_of::<Unknown>()).cast::<Unknown>();
        if p.is_null() {
            *out = core::ptr::null_mut();
            return E_OUTOFMEMORY;
        }
        p.write(Unknown { vtbl: &UNKNOWN_VTBL, refs: AtomicU32::new(1) });
        *out = p.cast();
    }
    S_OK
}

/// `CoInitializeSecurity`: accepted, nothing enforced.
#[unsafe(no_mangle)]
pub extern "system" fn CoInitializeSecurity(
    _sd: *mut c_void,
    _count: i32,
    _services: *mut c_void,
    _reserved1: *mut c_void,
    _authn_level: u32,
    _imp_level: u32,
    _auth_list: *mut c_void,
    _capabilities: u32,
    _reserved3: *mut c_void,
) -> Hresult {
    S_OK
}

/// `CoSetProxyBlanket`: there are no proxies here, so the object has none.
#[unsafe(no_mangle)]
pub extern "system" fn CoSetProxyBlanket(
    proxy: *mut c_void,
    _authn_svc: u32,
    _authz_svc: u32,
    _server_princ: *mut u16,
    _authn_level: u32,
    _imp_level: u32,
    _auth_info: *mut c_void,
    _capabilities: u32,
) -> Hresult {
    if proxy.is_null() { E_INVALIDARG } else { E_NOINTERFACE }
}

/// `CoRegisterMessageFilter`: only a single-threaded apartment has a message filter
/// (`CO_E_NOT_SUPPORTED` on a multithreaded thread, `CO_E_NOTINITIALIZED` on one with no apartment).
/// The thread keeps the new filter (with a reference of its own) and hands the previous one back
/// (NULL at first); the filter is never called, since no call ever has to be rejected or retried.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoRegisterMessageFilter(new: *mut c_void, old: *mut *mut c_void) -> Hresult {
    match apartment().model() {
        None => return CO_E_NOTINITIALIZED,
        Some(Model::Mta) => return CO_E_NOT_SUPPORTED,
        Some(Model::Sta) => {}
    }
    let Some(f) = filter_slot() else { return E_OUTOFMEMORY };
    // SAFETY: kernel32's TLS; `new` is a COM object or NULL; the caller's out parameter.
    unsafe {
        let previous = TlsGetValue(f).cast::<c_void>();
        if !new.is_null() {
            add_ref_unknown(new);
        }
        TlsSetValue(f, new.cast());
        if !old.is_null() {
            *old = previous;
        } else if !previous.is_null() {
            release_unknown(previous);
        }
    }
    S_OK
}

/// `RegisterDragDrop`: accepted for a window and a target, nothing kept.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterDragDrop(hwnd: usize, target: *mut c_void) -> Hresult {
    if hwnd == 0 {
        DRAGDROP_E_INVALIDHWND
    } else if target.is_null() {
        E_INVALIDARG
    } else {
        S_OK
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RevokeDragDrop(hwnd: usize) -> Hresult {
    if hwnd == 0 { DRAGDROP_E_INVALIDHWND } else { S_OK }
}

// ----- GUIDs ---------------------------------------------------------------------------

/// The characters of a NUL-terminated string, at most `max` of them.
unsafe fn wide_prefix<'a>(p: *const u16, max: usize) -> &'a [u16] {
    let mut n = 0;
    // SAFETY: the caller's NUL-terminated string.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// `CLSIDFromString`: braced text; NULL is `CLSID_NULL`. A ProgID has no
/// registry to be looked up in.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CLSIDFromString(text: *const u16, clsid: *mut Guid) -> Hresult {
    if clsid.is_null() {
        return E_INVALIDARG;
    }
    if text.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *clsid = Guid::default() };
        return S_OK;
    }
    // SAFETY: a NUL-terminated string; one character past the length shows a longer one.
    let s = unsafe { wide_prefix(text, GUID_TEXT_LEN + 1) };
    match Guid::parse(s) {
        Some(g) => {
            // SAFETY: the caller's out parameter.
            unsafe { *clsid = g };
            S_OK
        }
        None => {
            // The sheet: the output is left alone for an empty string and written (partly, by the
            // parse) for any other malformed text; Poly writes the all-zero id.
            if !s.is_empty() {
                // SAFETY: the caller's out parameter.
                unsafe { *clsid = Guid::default() };
            }
            CO_E_CLASSSTRING
        }
    }
}

/// `IIDFromString`: braced text only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IIDFromString(text: *const u16, iid: *mut Guid) -> Hresult {
    if text.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a NUL-terminated string.
    let s = unsafe { wide_prefix(text, GUID_TEXT_LEN + 1) };
    match Guid::parse(s) {
        Some(g) => {
            // SAFETY: the caller's out parameter.
            unsafe { *iid = g };
            S_OK
        }
        None => CO_E_IIDSTRING,
    }
}

/// `StringFromGUID2`: the characters written including the NUL (39), or 0
/// when `max` is too small.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn StringFromGUID2(guid: *const Guid, buf: *mut u16, max: i32) -> i32 {
    if guid.is_null() || buf.is_null() || (max as usize) < GUID_TEXT_LEN + 1 {
        return 0;
    }
    // SAFETY: the caller's GUID and a buffer of at least 39 characters.
    unsafe {
        let t = (*guid).to_text();
        core::ptr::copy_nonoverlapping(t.as_ptr(), buf, GUID_TEXT_LEN);
        *buf.add(GUID_TEXT_LEN) = 0;
    }
    (GUID_TEXT_LEN + 1) as i32
}

/// `StringFromCLSID` / `StringFromIID`: the text in task memory.
unsafe fn string_from_guid(guid: *const Guid, out: *mut *mut u16) -> Hresult {
    // The sheet: a NULL id or a NULL output is E_INVALIDARG.
    if out.is_null() || guid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: task memory for 39 characters, then StringFromGUID2.
    unsafe {
        let p = CoTaskMemAlloc((GUID_TEXT_LEN + 1) * 2).cast::<u16>();
        if p.is_null() {
            return E_OUTOFMEMORY;
        }
        StringFromGUID2(guid, p, (GUID_TEXT_LEN + 1) as i32);
        *out = p;
    }
    S_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn StringFromCLSID(clsid: *const Guid, out: *mut *mut u16) -> Hresult {
    // SAFETY: as documented.
    unsafe { string_from_guid(clsid, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn StringFromIID(iid: *const Guid, out: *mut *mut u16) -> Hresult {
    // SAFETY: as documented.
    unsafe { string_from_guid(iid, out) }
}

/// `CoCreateGuid`: a random (version 4, variant 1) GUID.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoCreateGuid(out: *mut Guid) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let mut bytes = [0u8; 16];
    // SAFETY: our bcryptprimitives fills 16 bytes.
    if unsafe { ProcessPrng(bytes.as_mut_ptr(), 16) } == 0 {
        return E_FAIL;
    }
    // Version 4 is the top nibble of Data3, variant 1 the top bits of Data4[0].
    let g = Guid {
        data1: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        data2: u16::from_le_bytes([bytes[4], bytes[5]]),
        data3: u16::from_le_bytes([bytes[6], bytes[7]]) & 0x0FFF | 0x4000,
        data4: [bytes[8] & 0x3F | 0x80, bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]],
    };
    // SAFETY: the caller's out parameter.
    unsafe { *out = g };
    S_OK
}

// ----- PROPVARIANT ---------------------------------------------------------------------

/// A `PROPVARIANT` is 24 bytes on x64 (`propidl.h`): `vt` at 0, the value at 8 (a pointer, a
/// number, or `cElems` at 8 and `pElems` at 16; a blob's `cbSize` at 8 and `pBlobData` at 16).
const PV_SIZE: usize = 24;

unsafe fn pv_ptr(pv: *const u8, at: usize) -> *mut c_void {
    // SAFETY: the caller's 24-byte variant.
    unsafe { (pv.add(at) as *const *mut c_void).read_unaligned() }
}

unsafe fn pv_set_ptr(pv: *mut u8, at: usize, p: *mut c_void) {
    // SAFETY: the caller's 24-byte variant.
    unsafe { (pv.add(at) as *mut *mut c_void).write_unaligned(p) }
}

/// Free what a `CLIPDATA` pointer owns: the data block and the structure (`cbSize` 4, `ulClipFmt` 4,
/// `pClipData` 8).
unsafe fn free_clipdata(c: *mut u8) {
    if !c.is_null() {
        // SAFETY: a CLIPDATA from CoTaskMemAlloc.
        unsafe {
            CoTaskMemFree((c.add(8) as *const *mut c_void).read_unaligned());
            CoTaskMemFree(c.cast());
        }
    }
}

/// Free what one element of a vector owns (the element is `elem.size()` bytes at `e`).
unsafe fn free_element(elem: Elem, e: *mut u8) {
    // SAFETY: an element of the caller's vector.
    unsafe {
        match elem {
            Elem::Plain(_) => {}
            Elem::Bstr => SysFreeString((e as *const *mut u16).read_unaligned()),
            Elem::TaskString => CoTaskMemFree((e as *const *mut c_void).read_unaligned()),
            Elem::Variant => clear_owned(e),
            Elem::ClipData => CoTaskMemFree((e.add(8) as *const *mut c_void).read_unaligned()),
        }
    }
}

/// Release what a variant of a valid type owns, without changing the variant itself.
unsafe fn clear_owned(pv: *mut u8) {
    // SAFETY: a 24-byte PROPVARIANT.
    unsafe {
        let vt = (pv as *const u16).read_unaligned();
        let Some(kind) = propvariant::classify(vt) else { return };
        let payload = pv_ptr(pv, 8);
        match kind {
            Kind::Plain | Kind::ByRef => {}
            Kind::Bstr => SysFreeString(payload.cast()),
            Kind::TaskString | Kind::Clsid => CoTaskMemFree(payload),
            Kind::Blob => CoTaskMemFree(pv_ptr(pv, 16)),
            Kind::Interface => {
                if !payload.is_null() {
                    release_unknown(payload);
                }
            }
            Kind::ClipData => free_clipdata(payload.cast()),
            Kind::SafeArray => {
                if !payload.is_null() {
                    SafeArrayDestroy(payload.cast());
                }
            }
            Kind::Vector(elem) => {
                let n = (pv.add(8) as *const u32).read_unaligned() as usize;
                let p = pv_ptr(pv, 16).cast::<u8>();
                if !p.is_null() {
                    for i in 0..n {
                        free_element(elem, p.add(i * elem.size()));
                    }
                    CoTaskMemFree(p.cast());
                }
            }
        }
    }
}

/// `PropVariantClear`: frees what the value owns (strings, blobs, vectors' elements and array,
/// interface references, a `SAFEARRAY`) and makes it `VT_EMPTY` with a zero value. A type no
/// `PROPVARIANT` has is `STG_E_INVALIDPARAMETER`; the sheet's variant is emptied all the same
/// (type 0, value words 0). NULL is accepted.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PropVariantClear(pv: *mut u8) -> Hresult {
    if pv.is_null() {
        return S_OK;
    }
    // SAFETY: a 24-byte PROPVARIANT.
    unsafe {
        let vt = (pv as *const u16).read_unaligned();
        let hr = if propvariant::classify(vt).is_some() {
            clear_owned(pv);
            S_OK
        } else {
            STG_E_INVALIDPARAMETER
        };
        core::ptr::write_bytes(pv, 0, PV_SIZE);
        hr
    }
}

/// A copy of `n` bytes in task memory (NULL for no bytes).
unsafe fn task_dup(src: *const u8, n: usize) -> Result<*mut u8, Hresult> {
    if src.is_null() || n == 0 {
        return Ok(core::ptr::null_mut());
    }
    // SAFETY: `n` readable bytes at `src`; fresh task memory.
    unsafe {
        let p = CoTaskMemAlloc(n).cast::<u8>();
        if p.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        core::ptr::copy_nonoverlapping(src, p, n);
        Ok(p)
    }
}

/// The length in bytes (without the NUL) of a task string: `wide` for `LPWSTR`.
unsafe fn task_string_len(p: *const u8, wide: bool) -> usize {
    let mut n = 0;
    // SAFETY: a NUL-terminated string.
    unsafe {
        if wide {
            while (p as *const u16).add(n).read_unaligned() != 0 {
                n += 1;
            }
            n * 2
        } else {
            while *p.add(n) != 0 {
                n += 1;
            }
            n
        }
    }
}

/// Copy one string slot (`*src` -> `*dst`): a `BSTR` or an `LPSTR` / `LPWSTR`.
unsafe fn copy_string_slot(elem: Elem, wide: bool, src: *const u8, dst: *mut u8) -> Hresult {
    // SAFETY: the pointer slots of the caller's variants.
    unsafe {
        let s = (src as *const *const u8).read_unaligned();
        let d: *mut c_void = if s.is_null() {
            core::ptr::null_mut()
        } else if elem == Elem::Bstr {
            let b = SysAllocStringByteLen(s, SysStringByteLen(s.cast()));
            if b.is_null() {
                return E_OUTOFMEMORY;
            }
            b.cast()
        } else {
            let n = task_string_len(s, wide) + if wide { 2 } else { 1 };
            match task_dup(s, n) {
                Ok(p) => p.cast(),
                Err(e) => return e,
            }
        };
        (dst as *mut *mut c_void).write_unaligned(d);
        S_OK
    }
}

/// Copy a variant's owned data: `dst` already holds a bit copy of `src`.
unsafe fn copy_owned(src: *const u8, dst: *mut u8) -> Hresult {
    // SAFETY: two 24-byte PROPVARIANTs.
    unsafe {
        let vt = (src as *const u16).read_unaligned();
        let Some(kind) = propvariant::classify(vt) else { return STG_E_INVALIDPARAMETER };
        let base = vt & !(propvariant::VT_VECTOR | propvariant::VT_ARRAY | propvariant::VT_BYREF);
        match kind {
            Kind::Plain | Kind::ByRef => S_OK,
            Kind::Bstr => copy_string_slot(Elem::Bstr, true, src.add(8), dst.add(8)),
            Kind::TaskString => copy_string_slot(Elem::TaskString, base == propvariant::VT_LPWSTR, src.add(8), dst.add(8)),
            Kind::Clsid => match task_dup(pv_ptr(src, 8).cast(), 16) {
                Ok(p) => {
                    pv_set_ptr(dst, 8, p.cast());
                    S_OK
                }
                Err(e) => e,
            },
            Kind::Blob => {
                let n = (src.add(8) as *const u32).read_unaligned() as usize;
                match task_dup(pv_ptr(src, 16).cast(), n) {
                    Ok(p) => {
                        pv_set_ptr(dst, 16, p.cast());
                        S_OK
                    }
                    Err(e) => e,
                }
            }
            Kind::Interface => {
                let p = pv_ptr(src, 8);
                if !p.is_null() {
                    add_ref_unknown(p);
                }
                S_OK
            }
            Kind::ClipData => match copy_clipdata(pv_ptr(src, 8).cast()) {
                Ok(p) => {
                    pv_set_ptr(dst, 8, p.cast());
                    S_OK
                }
                Err(e) => e,
            },
            Kind::SafeArray => E_NOTIMPL,
            Kind::Vector(elem) => {
                let n = (src.add(8) as *const u32).read_unaligned() as usize;
                let sp = pv_ptr(src, 16).cast::<u8>();
                if sp.is_null() || n == 0 {
                    pv_set_ptr(dst, 16, core::ptr::null_mut());
                    return S_OK;
                }
                let Some(total) = n.checked_mul(elem.size()) else { return E_OUTOFMEMORY };
                let dp = CoTaskMemAlloc(total).cast::<u8>();
                if dp.is_null() {
                    return E_OUTOFMEMORY;
                }
                core::ptr::copy_nonoverlapping(sp, dp, total);
                for i in 0..n {
                    let (s, d) = (sp.add(i * elem.size()), dp.add(i * elem.size()));
                    let hr = match elem {
                        Elem::Plain(_) => S_OK,
                        Elem::Bstr => copy_string_slot(Elem::Bstr, true, s, d),
                        Elem::TaskString => copy_string_slot(elem, base == propvariant::VT_LPWSTR, s, d),
                        Elem::Variant => copy_owned(s, d),
                        Elem::ClipData => copy_clip_slot(s, d),
                    };
                    if hr < 0 {
                        // Elements before `i` own their copies; the rest are bit copies of the source's:
                        // empty them so the partial vector can be freed.
                        for j in i..n {
                            core::ptr::write_bytes(dp.add(j * elem.size()), 0, elem.size());
                        }
                        let mut tmp = [0u8; PV_SIZE];
                        (tmp.as_mut_ptr() as *mut u16).write_unaligned(vt);
                        (tmp.as_mut_ptr().add(8) as *mut u32).write_unaligned(n as u32);
                        pv_set_ptr(tmp.as_mut_ptr(), 16, dp.cast());
                        clear_owned(tmp.as_mut_ptr());
                        return hr;
                    }
                }
                pv_set_ptr(dst, 16, dp.cast());
                S_OK
            }
        }
    }
}

/// A new `CLIPDATA` (and its data block) copied from `c`, NULL for NULL.
unsafe fn copy_clipdata(c: *const u8) -> Result<*mut u8, Hresult> {
    if c.is_null() {
        return Ok(core::ptr::null_mut());
    }
    // SAFETY: a CLIPDATA: size, format, data pointer.
    unsafe {
        let n = c.cast::<u32>().read_unaligned() as usize;
        let d = CoTaskMemAlloc(16).cast::<u8>();
        if d.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        core::ptr::copy_nonoverlapping(c, d, 16);
        // `cbSize` counts the format word too (`ulClipFmt` + the data).
        let data = (c.add(8) as *const *const u8).read_unaligned();
        let len = n.saturating_sub(4);
        match task_dup(data, len) {
            Ok(p) => {
                (d.add(8) as *mut *mut u8).write_unaligned(p);
                Ok(d)
            }
            Err(e) => {
                CoTaskMemFree(d.cast());
                Err(e)
            }
        }
    }
}

/// One `CLIPDATA` element of a vector (inline structure): its data block is copied.
unsafe fn copy_clip_slot(s: *const u8, d: *mut u8) -> Hresult {
    // SAFETY: a CLIPDATA structure inline in a vector.
    unsafe {
        let n = s.cast::<u32>().read_unaligned() as usize;
        let data = (s.add(8) as *const *const u8).read_unaligned();
        match task_dup(data, n.saturating_sub(4)) {
            Ok(p) => {
                (d.add(8) as *mut *mut u8).write_unaligned(p);
                S_OK
            }
            Err(e) => e,
        }
    }
}

/// `PropVariantCopy`: `dst` becomes a deep copy of `src` (what `dst` held before is not cleared).
/// A type no `PROPVARIANT` has is `STG_E_INVALIDPARAMETER` (and `dst` is left empty); a `SAFEARRAY`
/// is `E_NOTIMPL` here.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PropVariantCopy(dst: *mut u8, src: *const u8) -> Hresult {
    if dst.is_null() || src.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: two 24-byte PROPVARIANTs.
    unsafe {
        core::ptr::copy(src, dst, PV_SIZE);
        let hr = copy_owned(src, dst);
        if hr < 0 {
            core::ptr::write_bytes(dst, 0, PV_SIZE);
        }
        hr
    }
}

// ----- HSTRING -------------------------------------------------------------------------

fn put<T>(slot: *mut T, v: T) -> Hresult {
    if slot.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { slot.write(v) };
    S_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsCreateString(src: *const u16, len: u32, out: *mut HString) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's characters.
    match unsafe { hstring::create(src, len) } {
        Ok(h) => put(out, h),
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsCreateStringReference(
    src: *const u16,
    len: u32,
    header: *mut Header,
    out: *mut HString,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's characters and header.
    match unsafe { hstring::create_reference(src, len, header) } {
        Ok(h) => put(out, h),
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsDeleteString(h: HString) -> Hresult {
    // SAFETY: a handle from this DLL.
    unsafe { hstring::delete(h) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsDuplicateString(h: HString, out: *mut HString) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a handle from this DLL.
    match unsafe { hstring::duplicate(h) } {
        Ok(d) => put(out, d),
        Err(e) => e,
    }
}

/// `WindowsGetStringRawBuffer`: the length is written when asked for.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsGetStringRawBuffer(h: HString, len: *mut u32) -> *const u16 {
    // SAFETY: a handle from this DLL.
    let (p, n) = unsafe { hstring::raw_buffer(h) };
    if !len.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *len = n };
    }
    p
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsGetStringLen(h: HString) -> u32 {
    // SAFETY: a handle from this DLL.
    unsafe { hstring::len(h) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsIsStringEmpty(h: HString) -> i32 {
    // SAFETY: a handle from this DLL.
    i32::from(unsafe { hstring::is_empty(h) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsStringHasEmbeddedNull(h: HString, out: *mut i32) -> Hresult {
    // SAFETY: a handle from this DLL.
    put(out, i32::from(unsafe { hstring::has_embedded_null(h) }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsCompareStringOrdinal(a: HString, b: HString, out: *mut i32) -> Hresult {
    // SAFETY: handles from this DLL.
    put(out, unsafe { hstring::compare_ordinal(a, b) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsConcatString(a: HString, b: HString, out: *mut HString) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: handles from this DLL.
    match unsafe { hstring::concat(a, b) } {
        Ok(h) => put(out, h),
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsSubstring(h: HString, start: u32, out: *mut HString) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a handle from this DLL.
    match unsafe { hstring::substring(h, start) } {
        Ok(s) => put(out, s),
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsPreallocateStringBuffer(
    len: u32,
    chars: *mut *mut u16,
    buffer: *mut HString,
) -> Hresult {
    if chars.is_null() || buffer.is_null() {
        return E_POINTER;
    }
    match hstring::preallocate(len) {
        Ok((b, p)) => {
            // SAFETY: the caller's out parameters.
            unsafe {
                *chars = p;
                *buffer = b;
            }
            S_OK
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsPromoteStringBuffer(buffer: HString, out: *mut HString) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a buffer handle from this DLL.
    match unsafe { hstring::promote(buffer) } {
        Ok(h) => put(out, h),
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowsDeleteStringBuffer(buffer: HString) -> Hresult {
    // SAFETY: a buffer handle from this DLL.
    unsafe { hstring::delete_buffer(buffer) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn ole32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod delayed;
