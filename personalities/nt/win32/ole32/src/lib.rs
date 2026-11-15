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
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

extern crate alloc;
use poly_com::apartment::{APTTYPEQUALIFIER_NONE, Apartment, Model};
use poly_com::guid::{GUID_TEXT_LEN, Guid};
use poly_com::hresult::*;
use poly_com::hstring::{self, HString, Header};
use poly_com::propvariant::{self, Elem, Kind};
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
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> usize;
    fn CloseHandle(handle: usize) -> i32;
    fn WaitForSingleObject(handle: usize, ms: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
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

/// The thread that is the process's main STA (0: none), and how many threads have the MTA.
static MAIN_STA: AtomicU32 = AtomicU32::new(0);
static MTA_THREADS: AtomicU32 = AtomicU32::new(0);

/// Whether thread `tid` still runs (a thread that ended without `CoUninitialize` is not the main STA any more).
fn thread_alive(tid: u32) -> bool {
    // SAFETY: SYNCHRONIZE | THREAD_QUERY_LIMITED_INFORMATION; the handle is closed at once.
    unsafe {
        let h = OpenThread(0x0010_0000 | 0x0800, 0, tid);
        if h == 0 {
            return false;
        }
        let r = WaitForSingleObject(h, 0);
        CloseHandle(h);
        r == 0x102
    }
}

/// The first STA of the process while it lives is the main STA (`APTTYPE_MAINSTA`).
fn claim_main_sta() -> bool {
    // SAFETY: no preconditions.
    let me = unsafe { GetCurrentThreadId() };
    loop {
        let cur = MAIN_STA.load(Ordering::Acquire);
        if cur == me {
            return true;
        }
        if cur != 0 && thread_alive(cur) {
            return false;
        }
        if MAIN_STA.compare_exchange(cur, me, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            return true;
        }
    }
}

/// The thread got its apartment just now.
fn apartment_begins(a: &mut Apartment, model: Model) {
    match model {
        Model::Sta => a.set_main(claim_main_sta()),
        Model::Mta => {
            MTA_THREADS.fetch_add(1, Ordering::AcqRel);
        }
    }
}

/// The thread's apartment ended (its last initialisation was balanced): `before` is what it was.
fn apartment_ends(before: Apartment) {
    if before.is_main() {
        // SAFETY: no preconditions.
        let me = unsafe { GetCurrentThreadId() };
        let _ = MAIN_STA.compare_exchange(me, 0, Ordering::AcqRel, Ordering::Acquire);
    }
    if before.model() == Some(Model::Mta) {
        let _ = MTA_THREADS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }
    // The message filter of the apartment goes with it.
    let f = filter();
    if !f.is_null() {
        set_filter(core::ptr::null_mut());
        // SAFETY: a filter the thread registered (we hold a reference).
        unsafe { release(f) };
    }
}

/// `CoInitializeEx` / `RoInitialize` on this thread.
fn initialize(model: Model) -> Hresult {
    if slot().is_none() {
        return E_OUTOFMEMORY;
    }
    let mut a = apartment();
    let fresh = a.model().is_none();
    let hr = a.initialize(model);
    if hr >= 0 {
        if fresh {
            apartment_begins(&mut a, model);
        }
        set_apartment(a);
    }
    hr
}

fn uninitialize() {
    let before = apartment();
    let mut a = before;
    a.uninitialize();
    set_apartment(a);
    if before.model().is_some() && a.model().is_none() {
        apartment_ends(before);
    }
}

/// `CoInitializeEx`: `dwCoInit` may only hold the defined bits and `pvReserved` must be NULL
/// (`E_INVALIDARG`; the thread stays as it was).
#[unsafe(no_mangle)]
pub extern "system" fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> Hresult {
    // A reserved pointer and any bit that is no `COINIT` value are refused (CoInitializeEx.yaml `bad_arguments`).
    if !reserved.is_null() || !poly_com::apartment::coinit_is_valid(co_init) {
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

/// `OleInitialize`: COM for the STA plus OLE's own count on the thread.
#[unsafe(no_mangle)]
pub extern "system" fn OleInitialize(reserved: *mut c_void) -> Hresult {
    if !reserved.is_null() {
        return E_INVALIDARG;
    }
    if slot().is_none() {
        return E_OUTOFMEMORY;
    }
    let mut a = apartment();
    let fresh = a.model().is_none();
    let hr = a.ole_initialize();
    if hr >= 0 {
        if fresh {
            apartment_begins(&mut a, Model::Sta);
        }
        set_apartment(a);
    }
    hr
}

/// `OleUninitialize`: the last one releases the drop targets registered on the thread.
#[unsafe(no_mangle)]
pub extern "system" fn OleUninitialize() {
    let before = apartment();
    let mut a = before;
    let last = a.ole_uninitialize();
    set_apartment(a);
    if last {
        release_drop_targets();
    }
    if before.model().is_some() && a.model().is_none() {
        apartment_ends(before);
    }
}

/// `CoGetApartmentType`: `CO_E_NOTINITIALIZED` when the thread has none and the process no MTA; a
/// thread without COM in a process that has the MTA is in it implicitly.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> Hresult {
    if kind.is_null() || qualifier.is_null() {
        return E_INVALIDARG;
    }
    let (hr, t, q) = match apartment().apartment_type() {
        Some((t, q)) => (S_OK, t, q),
        None if MTA_THREADS.load(Ordering::Acquire) > 0 => {
            (S_OK, poly_com::apartment::APTTYPE_MTA, poly_com::apartment::APTTYPEQUALIFIER_IMPLICIT_MTA)
        }
        None => (CO_E_NOTINITIALIZED, 0, APTTYPEQUALIFIER_NONE),
    };
    // SAFETY: the caller's out parameters.
    unsafe {
        *kind = t;
        *qualifier = q;
    }
    hr
}

#[unsafe(no_mangle)]
pub extern "system" fn RoInitialize(init_type: i32) -> Hresult {
    initialize(Model::from_ro(init_type))
}

#[unsafe(no_mangle)]
pub extern "system" fn RoUninitialize() {
    uninitialize();
}

// ----- the thread's message filter ---------------------------------------------------------

/// The TLS slot of the thread's message filter (an `IMessageFilter` pointer we hold a reference to).
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

fn filter() -> *mut c_void {
    // SAFETY: kernel32's TLS.
    filter_slot().map_or(core::ptr::null_mut(), |s| unsafe { TlsGetValue(s) }.cast())
}

fn set_filter(f: *mut c_void) {
    if let Some(s) = filter_slot() {
        // SAFETY: kernel32's TLS.
        unsafe { TlsSetValue(s, f.cast()) };
    }
}

/// `IUnknown::AddRef` / `Release` of a COM object (vtable slots 1 and 2).
unsafe fn add_ref(p: *mut c_void) {
    // SAFETY: the caller's COM object.
    unsafe {
        let vtbl = *(p as *const *const unsafe extern "system" fn(*mut c_void) -> u32);
        (*vtbl.add(1))(p);
    }
}

unsafe fn release(p: *mut c_void) {
    // SAFETY: the caller's COM object.
    unsafe {
        let vtbl = *(p as *const *const unsafe extern "system" fn(*mut c_void) -> u32);
        (*vtbl.add(2))(p);
    }
}

/// `CoRegisterMessageFilter`: one filter per STA thread, AddRef'ed; the one it replaces (or
/// removes) goes to the caller with the reference the registration held.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoRegisterMessageFilter(new: *mut c_void, old: *mut *mut c_void) -> Hresult {
    match apartment().model() {
        None => return CO_E_NOTINITIALIZED,
        Some(Model::Mta) => return CO_E_NOT_SUPPORTED,
        Some(Model::Sta) => {}
    }
    let prev = filter();
    if !new.is_null() {
        // SAFETY: the caller's filter.
        unsafe { add_ref(new) };
    }
    set_filter(new);
    if !old.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *old = prev };
    } else if !prev.is_null() {
        // SAFETY: the filter the thread had registered.
        unsafe { release(prev) };
    }
    S_OK
}

// ----- drag and drop -------------------------------------------------------------------

/// A spin lock around a value (no `std` here).
struct Locked<T> {
    flag: core::sync::atomic::AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

// SAFETY: the flag serialises every access to the value.
unsafe impl<T> Sync for Locked<T> {}

impl<T> Locked<T> {
    const fn new(v: T) -> Locked<T> {
        Locked { flag: core::sync::atomic::AtomicBool::new(false), value: core::cell::UnsafeCell::new(v) }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held.
        let r = f(unsafe { &mut *self.value.get() });
        self.flag.store(false, Ordering::Release);
        r
    }
}

/// The drop targets of the process: window, target, the registering thread.
static TARGETS: Locked<alloc::vec::Vec<(usize, usize, u32)>> = Locked::new(alloc::vec::Vec::new());

/// The window property a registration leaves (Learn: "RegisterDragDrop"): `OleDropTargetInterface`.
const DROP_PROP: &[u16] = &[
    b'O' as u16,
    b'l' as u16,
    b'e' as u16,
    b'D' as u16,
    b'r' as u16,
    b'o' as u16,
    b'p' as u16,
    b'T' as u16,
    b'a' as u16,
    b'r' as u16,
    b'g' as u16,
    b'e' as u16,
    b't' as u16,
    b'I' as u16,
    b'n' as u16,
    b't' as u16,
    b'e' as u16,
    b'r' as u16,
    b'f' as u16,
    b'a' as u16,
    b'c' as u16,
    b'e' as u16,
    0,
];

static USER32: [AtomicUsize; 4] = [AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0)];

/// A function of `user32.dll`, looked up when first used (a program with no windows never loads it).
fn user32_fn(which: usize, name: &[u8]) -> usize {
    let cached = USER32[which].load(Ordering::Acquire);
    if cached != 0 {
        return cached;
    }
    let lib: [u16; 11] = [117, 115, 101, 114, 51, 50, 46, 100, 108, 108, 0];
    // SAFETY: kernel32 calls with NUL-terminated names.
    let p = unsafe { GetProcAddress(LoadLibraryW(lib.as_ptr()), name.as_ptr()) };
    USER32[which].store(p, Ordering::Release);
    p
}

unsafe fn is_window(hwnd: usize) -> bool {
    let p = user32_fn(0, b"IsWindow\0");
    // SAFETY: user32's IsWindow, or `p` is 0 (checked).
    p != 0 && unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> i32>(p)(hwnd) } != 0
}

unsafe fn get_prop(hwnd: usize) -> usize {
    let p = user32_fn(1, b"GetPropW\0");
    if p == 0 {
        return 0;
    }
    // SAFETY: user32's GetPropW.
    unsafe {
        core::mem::transmute::<usize, unsafe extern "system" fn(usize, *const u16) -> usize>(p)(
            hwnd,
            DROP_PROP.as_ptr(),
        )
    }
}

unsafe fn set_prop(hwnd: usize, value: usize) {
    let p = user32_fn(2, b"SetPropW\0");
    if p != 0 {
        // SAFETY: user32's SetPropW.
        unsafe {
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, *const u16, usize) -> i32>(p)(
                hwnd,
                DROP_PROP.as_ptr(),
                value,
            )
        };
    }
}

unsafe fn remove_prop(hwnd: usize) {
    let p = user32_fn(3, b"RemovePropW\0");
    if p != 0 {
        // SAFETY: user32's RemovePropW.
        unsafe {
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, *const u16) -> usize>(p)(
                hwnd,
                DROP_PROP.as_ptr(),
            )
        };
    }
}

/// `RegisterDragDrop`: the target is AddRef'ed and kept (as the window's `OleDropTargetInterface`
/// property) until `RevokeDragDrop` or the last `OleUninitialize` of this thread; it needs OLE on
/// the thread; `DestroyWindow` does not revoke.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterDragDrop(hwnd: usize, target: *mut c_void) -> Hresult {
    // SAFETY: user32's functions on a handle value; the target is the caller's COM object.
    unsafe {
        if hwnd == 0 || !is_window(hwnd) {
            return DRAGDROP_E_INVALIDHWND;
        }
        if target.is_null() {
            return E_INVALIDARG;
        }
        let a = apartment();
        if a.ole_count() == 0 {
            return if a.ole_used() { CO_E_NOTINITIALIZED } else { E_OUTOFMEMORY };
        }
        if get_prop(hwnd) != 0 {
            return DRAGDROP_E_ALREADYREGISTERED;
        }
        add_ref(target);
        set_prop(hwnd, target as usize);
        let me = GetCurrentThreadId();
        TARGETS.with(|t| t.push((hwnd, target as usize, me)));
    }
    S_OK
}

/// `RevokeDragDrop`: the target is released once and the property goes.
#[unsafe(no_mangle)]
pub extern "system" fn RevokeDragDrop(hwnd: usize) -> Hresult {
    // SAFETY: user32's functions on a handle value; the target we AddRef'ed.
    unsafe {
        if hwnd == 0 || !is_window(hwnd) {
            return DRAGDROP_E_INVALIDHWND;
        }
        let target = get_prop(hwnd);
        if target == 0 {
            return DRAGDROP_E_NOTREGISTERED;
        }
        remove_prop(hwnd);
        TARGETS.with(|t| t.retain(|e| e.0 != hwnd));
        release(target as *mut c_void);
    }
    S_OK
}

/// The last `OleUninitialize` of the thread: its drop targets are released.
fn release_drop_targets() {
    // SAFETY: no preconditions.
    let me = unsafe { GetCurrentThreadId() };
    let mine: alloc::vec::Vec<(usize, usize, u32)> = TARGETS.with(|t| {
        let (mine, rest) = t.drain(..).partition(|e| e.2 == me);
        *t = rest;
        mine
    });
    for (hwnd, target, _) in mine {
        // SAFETY: the registration's target and window handle.
        unsafe {
            if is_window(hwnd) && get_prop(hwnd) == target {
                remove_prop(hwnd);
            }
            release(target as *mut c_void);
        }
    }
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

/// `IID_IUnknown` (`unknwn.h`): {00000000-0000-0000-C000-000000000046}.
const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `IID_IMarshal` (`objidl.h`): {00000003-0000-0000-C000-000000000046}.
const IID_IMARSHAL: Guid = Guid { data1: 3, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

type UnkFn = unsafe extern "system" fn(*mut c_void) -> u32;
type QiFn = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult;

/// The free-threaded marshaler `CoCreateFreeThreadedMarshaler` hands out: the inner `IUnknown`
/// (this struct's first field is its vtable pointer) and, inside the same block, its `IMarshal`
/// interface. Without an outer object it counts its own references; aggregated, the `IMarshal`
/// interface delegates `QueryInterface`, `AddRef` and `Release` to the outer unknown.
#[repr(C)]
struct Marshaler {
    vtbl: *const UnknownVtbl,
    refs: AtomicU32,
    outer: *mut c_void,
    marshal: MarshalIface,
}

#[repr(C)]
struct MarshalIface {
    vtbl: *const MarshalVtbl,
    owner: *mut Marshaler,
}

#[repr(C)]
struct UnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut Marshaler, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut Marshaler) -> u32,
    release: unsafe extern "system" fn(*mut Marshaler) -> u32,
}

/// `IMarshal`: the three `IUnknown` methods and the six of the interface (all `E_NOTIMPL`: there is no
/// other process to marshal to).
#[repr(C)]
struct MarshalVtbl {
    query_interface: unsafe extern "system" fn(*mut MarshalIface, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut MarshalIface) -> u32,
    release: unsafe extern "system" fn(*mut MarshalIface) -> u32,
    rest: [unsafe extern "system" fn() -> Hresult; 6],
}

// SAFETY: tables of function pointers.
unsafe impl Sync for UnknownVtbl {}
// SAFETY: as above.
unsafe impl Sync for MarshalVtbl {}

static UNKNOWN_VTBL: UnknownVtbl = UnknownVtbl { query_interface: unk_qi, add_ref: unk_add_ref, release: unk_release };
static MARSHAL_VTBL: MarshalVtbl =
    MarshalVtbl { query_interface: mar_qi, add_ref: mar_add_ref, release: mar_release, rest: [not_impl; 6] };

unsafe extern "system" fn not_impl() -> Hresult {
    E_NOTIMPL
}

unsafe extern "system" fn unk_qi(this: *mut Marshaler, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID and out parameter; a live object.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        if *iid == IID_IUNKNOWN {
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            *out = this.cast();
            S_OK
        } else if *iid == IID_IMARSHAL {
            // The interface counts with the outer object when there is one.
            if (*this).outer.is_null() {
                (*this).refs.fetch_add(1, Ordering::AcqRel);
            } else {
                add_ref((*this).outer);
            }
            *out = core::ptr::addr_of_mut!((*this).marshal).cast();
            S_OK
        } else {
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn unk_add_ref(this: *mut Marshaler) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn unk_release(this: *mut Marshaler) -> u32 {
    // SAFETY: a live object from CoTaskMemAlloc.
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            CoTaskMemFree(this.cast());
        }
        n
    }
}

unsafe extern "system" fn mar_qi(this: *mut MarshalIface, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: the interface lives inside its owner.
    unsafe {
        let owner = (*this).owner;
        if (*owner).outer.is_null() {
            unk_qi(owner, iid, out)
        } else {
            let outer = (*owner).outer;
            let vtbl = *(outer as *const *const QiFn);
            (*vtbl)(outer, iid, out)
        }
    }
}

unsafe extern "system" fn mar_add_ref(this: *mut MarshalIface) -> u32 {
    // SAFETY: the interface lives inside its owner.
    unsafe {
        let owner = (*this).owner;
        if (*owner).outer.is_null() {
            unk_add_ref(owner)
        } else {
            let outer = (*owner).outer;
            let vtbl = *(outer as *const *const UnkFn);
            (*vtbl.add(1))(outer)
        }
    }
}

unsafe extern "system" fn mar_release(this: *mut MarshalIface) -> u32 {
    // SAFETY: the interface lives inside its owner.
    unsafe {
        let owner = (*this).owner;
        if (*owner).outer.is_null() {
            unk_release(owner)
        } else {
            let outer = (*owner).outer;
            let vtbl = *(outer as *const *const UnkFn);
            (*vtbl.add(2))(outer)
        }
    }
}

/// `CoCreateFreeThreadedMarshaler`: the inner `IUnknown` of an object that is `IMarshal` too (there
/// are no other processes to marshal to that it could serve). Creating it does not call the outer
/// object.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoCreateFreeThreadedMarshaler(outer: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter; a fresh block.
    unsafe {
        let p = CoTaskMemAlloc(core::mem::size_of::<Marshaler>()).cast::<Marshaler>();
        if p.is_null() {
            *out = core::ptr::null_mut();
            return E_OUTOFMEMORY;
        }
        p.write(Marshaler {
            vtbl: &UNKNOWN_VTBL,
            refs: AtomicU32::new(1),
            outer,
            marshal: MarshalIface { vtbl: &MARSHAL_VTBL, owner: p },
        });
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
    // The sheets: a NULL id or a NULL output is E_INVALIDARG.
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
                    release(payload);
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
        // `VT_ILLEGAL` (0xFFFF) is accepted and owns nothing (PropVariantClear.yaml `pvc_errors`).
        let hr = if vt == 0xFFFF {
            S_OK
        } else if propvariant::classify(vt).is_some() {
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
            Kind::TaskString => {
                copy_string_slot(Elem::TaskString, base == propvariant::VT_LPWSTR, src.add(8), dst.add(8))
            }
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
                    add_ref(p);
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

#[cfg(target_arch = "x86_64")]
mod apps;

/// `IsEqualGUID(rguid1, rguid2)`: whether two GUIDs are equal (Learn: `guiddef.h`; the logic is
/// `poly_kernelbase::apps::guid_eq`). Doc-derived, ADR 0155 addendum, step 10.
///
/// # Safety
/// Both point to 16 readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsEqualGUID(a: *const [u8; 16], b: *const [u8; 16]) -> i32 {
    // SAFETY: the caller's contract.
    i32::from(poly_kernelbase::apps::guid_eq(unsafe { &*a }, unsafe { &*b }))
}
