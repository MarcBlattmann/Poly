//! `CLSID_GlobalOptions` (sheets `CLSID_GlobalOptions`, `IGlobalOptions.Set`,
//! `IGlobalOptions.Query`): a class built into COM, served in-process without a registry entry.
//! Every object reads and writes the same options, which belong to the process
//! (`poly_com::globalopts`).

use crate::{ACTIVE_APARTMENTS, Locked, com_usable, is_guid};
use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use poly_com::globalopts::Options;
use poly_com::guid::Guid;
use poly_com::hresult::*;

/// `CLSID_GlobalOptions` (`combaseapi.h`): {0000034B-0000-0000-C000-000000000046}.
pub(crate) const CLSID_GLOBAL_OPTIONS: Guid =
    Guid { data1: 0x34B, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `IID_IGlobalOptions` (`objidl.h`): {0000015B-0000-0000-C000-000000000046}.
const IID_IGLOBAL_OPTIONS: Guid = Guid { data1: 0x15B, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// The process's options.
static OPTIONS: Locked<Options> = Locked::new(Options::new());
/// The process has used RPC: marshalled an interface, tried a local server or set its security
/// (the thread-pool setting is fixed from then on).
static RPC_USED: AtomicBool = AtomicBool::new(false);

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
}

/// What the second reserved property reads: an address (sheet `IGlobalOptions.Query`, rule
/// `defaults`: "a pointer-sized value that looks like an address"), here one page the process
/// reserves at the top of its address space once.
fn reserved2() -> usize {
    /// `MEM_RESERVE | MEM_TOP_DOWN`, `PAGE_NOACCESS` (`winnt.h`).
    const RESERVE_TOP_DOWN: u32 = 0x2000 | 0x10_0000;
    const PAGE_NOACCESS: u32 = 1;
    static PAGE: AtomicUsize = AtomicUsize::new(0);
    let known = PAGE.load(Ordering::Acquire);
    if known != 0 {
        return known;
    }
    // SAFETY: a fresh reservation, never freed.
    let p = unsafe { VirtualAlloc(core::ptr::null_mut(), 0x1000, RESERVE_TOP_DOWN, PAGE_NOACCESS) } as usize;
    let p = if p == 0 { core::ptr::addr_of!(OPTIONS) as usize } else { p };
    match PAGE.compare_exchange(0, p, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => p,
        Err(other) => other,
    }
}

/// Note that the process used RPC (sheet `IGlobalOptions.Set`, rule `rpc_threadpool`).
pub(crate) fn note_rpc_used() {
    RPC_USED.store(true, Ordering::Relaxed);
}

/// The exception-handling setting (`COMGLB_EXCEPTION_*`) for the calls COM serves.
pub(crate) fn exception_handling() -> usize {
    OPTIONS.with(|o| o.exception_handling())
}

/// The class object, if `clsid` is this class and `ctx` allows an in-process one (sheet
/// `CLSID_GlobalOptions`, rule `contexts`: a local or remote server alone does not find it).
pub(crate) fn class_object(clsid: &Guid, ctx: u32, iid: &Guid) -> Option<Result<*mut c_void, Hresult>> {
    /// `CLSCTX_INPROC_SERVER | CLSCTX_INPROC_HANDLER` (`wtypes.h`).
    const INPROC: u32 = 0x1 | 0x2;
    if *clsid != CLSID_GLOBAL_OPTIONS || ctx & INPROC == 0 {
        return None;
    }
    Some(if *iid == IID_IUNKNOWN || *iid == crate::oop::IID_ICLASS_FACTORY {
        Ok(core::ptr::addr_of!(FACTORY) as *mut c_void)
    } else {
        Err(E_NOINTERFACE)
    })
}

// ----- the class object ----------------------------------------------------------------

#[repr(C)]
struct FactoryVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    create: unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    lock: unsafe extern "system" fn(*mut c_void, i32) -> Hresult,
}

#[repr(C)]
struct Factory {
    vtbl: &'static FactoryVtbl,
}

// SAFETY: the object is immutable.
unsafe impl Sync for Factory {}

static FACTORY_VTBL: FactoryVtbl =
    FactoryVtbl { qi: factory_qi, add_ref: static_ref, release: static_ref, create: factory_create, lock: factory_lock };
static FACTORY: Factory = Factory { vtbl: &FACTORY_VTBL };

unsafe extern "system" fn factory_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's GUID and out parameter.
    unsafe {
        if is_guid(iid, &IID_IUNKNOWN) || is_guid(iid, &crate::oop::IID_ICLASS_FACTORY) {
            *out = this;
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

/// The class object lives as long as the process.
unsafe extern "system" fn static_ref(_this: *mut c_void) -> u32 {
    1
}

/// `IClassFactory::CreateInstance`: a new object each time; the class cannot be aggregated
/// (sheet rule `aggregation`: asking for `IUnknown` gives `CLASS_E_NOAGGREGATION`, another
/// interface `E_INVALIDARG`).
unsafe extern "system" fn factory_create(
    _this: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter and GUID.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        if !outer.is_null() {
            return if *iid == IID_IUNKNOWN { CLASS_E_NOAGGREGATION } else { E_INVALIDARG };
        }
        if *iid != IID_IUNKNOWN && *iid != IID_IGLOBAL_OPTIONS {
            return E_NOINTERFACE;
        }
        *out = Box::into_raw(Box::new(Object { vtbl: &OBJECT_VTBL, refs: AtomicU32::new(1) })).cast();
    }
    S_OK
}

unsafe extern "system" fn factory_lock(_this: *mut c_void, _lock: i32) -> Hresult {
    S_OK
}

// ----- the object ----------------------------------------------------------------------

#[repr(C)]
struct ObjectVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set: unsafe extern "system" fn(*mut c_void, u32, usize) -> Hresult,
    query: unsafe extern "system" fn(*mut c_void, u32, *mut usize) -> Hresult,
}

#[repr(C)]
struct Object {
    vtbl: &'static ObjectVtbl,
    refs: AtomicU32,
}

static OBJECT_VTBL: ObjectVtbl = ObjectVtbl { qi: object_qi, add_ref, release, set, query };

/// `IUnknown` and `IGlobalOptions` give the same pointer; nothing else is answered (rule
/// `interfaces`: not `IMarshal` either).
unsafe extern "system" fn object_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's GUID and out parameter; `this` is one of our objects.
    unsafe {
        if is_guid(iid, &IID_IUNKNOWN) || is_guid(iid, &IID_IGLOBAL_OPTIONS) {
            add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: one of our objects.
    unsafe { (*this.cast::<Object>()).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: one of our objects; the last reference frees it.
    unsafe {
        let left = (*this.cast::<Object>()).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(Box::from_raw(this.cast::<Object>()));
        }
        left
    }
}

/// Whether the object can be used from this thread: COM works for it, or some thread of the
/// process has COM initialised (rule `objects`: "from any thread ... while the creating thread's COM
/// is still initialised"; `uninitialised_thread`: not after the last `CoUninitialize`).
fn usable() -> bool {
    com_usable() || ACTIVE_APARTMENTS.load(Ordering::Acquire) > 0
}

/// `IGlobalOptions::Set`.
unsafe extern "system" fn set(_this: *mut c_void, property: u32, value: usize) -> Hresult {
    if !usable() {
        return CO_E_NOTINITIALIZED;
    }
    let used = RPC_USED.load(Ordering::Relaxed) || crate::oop::security::is_locked();
    match OPTIONS.with(|o| o.set(property, value, used)) {
        Ok(()) => S_OK,
        Err(hr) => hr,
    }
}

/// `IGlobalOptions::Query`: a failure leaves the output alone (rule `failure_leaves_output`).
unsafe extern "system" fn query(_this: *mut c_void, property: u32, value: *mut usize) -> Hresult {
    if !usable() {
        return CO_E_NOTINITIALIZED;
    }
    if value.is_null() {
        return E_INVALIDARG;
    }
    let reserved2 = reserved2();
    match OPTIONS.with(|o| o.query(property, reserved2)) {
        Ok(v) => {
            // SAFETY: the caller's out parameter, checked above.
            unsafe { *value = v };
            S_OK
        }
        Err(hr) => hr,
    }
}
