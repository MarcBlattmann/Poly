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
use poly_com::apartment::{APTTYPEQUALIFIER_NONE, Apartment, Model};
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
}

#[link(name = "bcryptprimitives", kind = "raw-dylib")]
unsafe extern "system" {
    fn ProcessPrng(buf: *mut u8, len: usize) -> i32;
}

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SysFreeString(s: *mut u16);
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

/// `CoInitializeEx` / `RoInitialize` / `OleInitialize` on this thread.
fn initialize(model: Model) -> Hresult {
    if slot().is_none() {
        return E_OUTOFMEMORY;
    }
    let mut a = apartment();
    let hr = a.initialize(model);
    if hr >= 0 {
        set_apartment(a);
    }
    hr
}

fn uninitialize() {
    let mut a = apartment();
    a.uninitialize();
    set_apartment(a);
}

#[unsafe(no_mangle)]
pub extern "system" fn CoInitializeEx(_reserved: *mut c_void, co_init: u32) -> Hresult {
    initialize(Model::from_coinit(co_init))
}

#[unsafe(no_mangle)]
pub extern "system" fn CoInitialize(_reserved: *mut c_void) -> Hresult {
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
    match apartment().apartment_type() {
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
    if apartment().model().is_none() {
        return CO_E_NOTINITIALIZED;
    }
    REGDB_E_CLASSNOTREG
}

/// `CoCreateInstance`: no class is registered.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoCreateInstance(
    _clsid: *const Guid,
    _outer: *mut c_void,
    _context: u32,
    _iid: *const Guid,
    ppv: *mut *mut c_void,
) -> Hresult {
    // SAFETY: the same checks.
    unsafe { CoGetClassObject(core::ptr::null(), 0, core::ptr::null_mut(), core::ptr::null(), ppv) }
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

/// `CoRegisterMessageFilter`: no filter is kept; the previous one is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoRegisterMessageFilter(_new: *mut c_void, old: *mut *mut c_void) -> Hresult {
    if !old.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *old = core::ptr::null_mut() };
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
        None => CO_E_CLASSSTRING,
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
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if guid.is_null() {
        return E_INVALIDARG;
    }
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
    bytes[6] = bytes[6] & 0x0F | 0x40;
    bytes[8] = bytes[8] & 0x3F | 0x80;
    let g = Guid {
        data1: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        data2: u16::from_le_bytes([bytes[4], bytes[5]]),
        data3: u16::from_le_bytes([bytes[6], bytes[7]]),
        data4: [bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]],
    };
    // SAFETY: the caller's out parameter.
    unsafe { *out = g };
    S_OK
}

// ----- PROPVARIANT ---------------------------------------------------------------------

const VT_EMPTY: u16 = 0;
const VT_BSTR: u16 = 8;
const VT_DISPATCH: u16 = 9;
const VT_UNKNOWN: u16 = 13;
const VT_LPSTR: u16 = 30;
const VT_LPWSTR: u16 = 31;
const VT_BLOB: u16 = 65;
const VT_CLSID: u16 = 72;
const VT_VECTOR: u16 = 0x1000;

/// `PropVariantClear`: frees what the value owns (strings, blobs, vectors'
/// arrays, interface references) and makes it `VT_EMPTY`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PropVariantClear(pv: *mut u8) -> Hresult {
    if pv.is_null() {
        return S_OK;
    }
    // SAFETY: a 24-byte PROPVARIANT: `vt` at 0, the union at 8.
    unsafe {
        let vt = (pv as *const u16).read_unaligned();
        let payload = (pv.add(8) as *const *mut c_void).read_unaligned();
        match vt {
            VT_BSTR => SysFreeString(payload.cast()),
            VT_LPSTR | VT_LPWSTR | VT_CLSID => CoTaskMemFree(payload),
            VT_UNKNOWN | VT_DISPATCH if !payload.is_null() => {
                // vtable[2] of IUnknown is Release.
                let vtbl = *(payload as *const *const unsafe extern "system" fn(*mut c_void) -> u32);
                (*vtbl.add(2))(payload);
            }
            VT_BLOB => CoTaskMemFree((pv.add(16) as *const *mut c_void).read_unaligned()),
            v if v & VT_VECTOR != 0 => CoTaskMemFree((pv.add(16) as *const *mut c_void).read_unaligned()),
            _ => {}
        }
        core::ptr::write_bytes(pv, 0, 24);
        (pv as *mut u16).write_unaligned(VT_EMPTY);
    }
    S_OK
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
pub unsafe extern "system" fn WindowsPreallocateStringBuffer(len: u32, chars: *mut *mut u16, buffer: *mut HString) -> Hresult {
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
