//! The scaffolding every COM object of this DLL shares: a header with the
//! vtable pointer and the reference count, generic `AddRef`/`Release`/
//! `QueryInterface`, `HRESULT` constants and small pointer helpers.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::abi::{GUID, HRESULT, IID_IUnknown};

// winerror.h
pub const S_OK: HRESULT = 0;
pub const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
pub const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
pub const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
#[allow(dead_code)]
pub const E_OUTOFMEMORY: HRESULT = 0x8007_000Eu32 as i32;
/// `ERROR_FILE_NOT_FOUND` as an `HRESULT`.
pub const E_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
/// `ERROR_INSUFFICIENT_BUFFER` as an `HRESULT`.
pub const E_INSUFFICIENT_BUFFER: HRESULT = 0x8007_007Au32 as i32;
// dwrite.h
pub const DWRITE_E_FILEFORMAT: HRESULT = 0x8898_5000u32 as i32;
pub const DWRITE_E_NOFONT: HRESULT = 0x8898_5002u32 as i32;
pub const DWRITE_E_FILENOTFOUND: HRESULT = 0x8898_5003u32 as i32;
pub const DWRITE_E_FILEACCESS: HRESULT = 0x8898_5004u32 as i32;
pub const DWRITE_E_ALREADYREGISTERED: HRESULT = 0x8898_5006u32 as i32;
pub const DWRITE_E_CACHEFORMAT: HRESULT = 0x8898_5007u32 as i32;
pub const DWRITE_E_NOCOLOR: HRESULT = 0x8898_500Cu32 as i32;

/// The start of every object this DLL hands out.
#[repr(C)]
pub struct Head {
    pub vtbl: *const c_void,
    pub refs: AtomicU32,
    /// Frees the object when the last reference goes.
    pub dtor: unsafe fn(*mut Head),
    /// The interfaces `QueryInterface` answers besides `IUnknown`.
    pub iids: &'static [GUID],
}

impl Head {
    pub fn new(vtbl: *const c_void, dtor: unsafe fn(*mut Head), iids: &'static [GUID]) -> Head {
        Head { vtbl, refs: AtomicU32::new(1), dtor, iids }
    }
}

/// `dtor` for an object allocated with `Box` whose first field is its
/// `Head`.
pub unsafe fn drop_box<T>(h: *mut Head) {
    // SAFETY: `h` is the address of a `Box<T>` whose first field is the head.
    drop(unsafe { alloc::boxed::Box::from_raw(h as *mut T) });
}

/// Move `obj` to the heap and give out the pointer (reference count 1).
pub fn into_raw<T>(obj: T) -> *mut T {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(obj))
}

pub unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: every object of this DLL starts with a `Head`.
    let h = unsafe { &*(this as *const Head) };
    h.refs.fetch_add(1, Ordering::Relaxed) + 1
}

pub unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: every object of this DLL starts with a `Head`.
    let h = unsafe { &*(this as *const Head) };
    let n = h.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference is gone.
        unsafe { (h.dtor)(this as *mut Head) };
    }
    n
}

pub unsafe extern "system" fn query_interface(this: *mut c_void, riid: *mut c_void, out: *mut c_void) -> HRESULT {
    let out = out as *mut *mut c_void;
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` is the caller's output pointer.
    unsafe { *out = core::ptr::null_mut() };
    if riid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller passes a GUID.
    let iid = unsafe { *(riid as *const GUID) };
    // SAFETY: every object of this DLL starts with a `Head`.
    let h = unsafe { &*(this as *const Head) };
    if iid == IID_IUnknown || h.iids.contains(&iid) {
        // SAFETY: `this` is live; `out` is writable.
        unsafe {
            add_ref(this);
            *out = this;
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

/// Write a pointer through a `T**` output parameter.
pub unsafe fn put<T>(out: *mut c_void, v: *mut T) {
    if !out.is_null() {
        // SAFETY: the caller passed a pointer to a pointer.
        unsafe { *(out as *mut *mut T) = v };
    }
}

/// Write a value through an output parameter.
pub unsafe fn put_val<T>(out: *mut c_void, v: T) {
    if !out.is_null() {
        // SAFETY: the caller passed a pointer to a `T`.
        unsafe { *(out as *mut T) = v };
    }
}

/// A slice of a caller's array (empty for a null pointer or count 0).
pub unsafe fn slice<'a, T>(p: *const c_void, n: u32) -> &'a [T] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        // SAFETY: the caller promises `n` elements at `p`.
        unsafe { core::slice::from_raw_parts(p as *const T, n as usize) }
    }
}

/// The vtable of a COM object.
pub unsafe fn vtbl<'a, V>(obj: *mut c_void) -> &'a V {
    // SAFETY: the first word of a COM object points to its vtable.
    unsafe { &**(obj as *const *const V) }
}

/// `AddRef` on any COM object.
pub unsafe fn com_add_ref(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: a live COM object: `AddRef` is the second slot.
    unsafe { (vtbl::<crate::abi::IUnknownVtbl>(obj).AddRef)(obj) }
}

/// `Release` on any COM object.
pub unsafe fn com_release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: a live COM object: `Release` is the third slot.
    unsafe { (vtbl::<crate::abi::IUnknownVtbl>(obj).Release)(obj) }
}

/// `E_NOTIMPL` for every slot without an implementation; the x64 calling
/// convention lets a function that reads no arguments stand in for any
/// signature.
pub unsafe extern "system" fn not_implemented() -> HRESULT {
    E_NOTIMPL
}

/// A slot filled with [`not_implemented`].
#[macro_export]
macro_rules! stub {
    () => {
        // SAFETY: on x64 an `extern "system"` function ignores arguments it
        // does not read; the result is an HRESULT in EAX.
        unsafe { core::mem::transmute::<unsafe extern "system" fn() -> i32, _>($crate::com::not_implemented) }
    };
}

/// A void slot that takes a pointer argument and does nothing.
pub unsafe extern "system" fn noop_void(_: *mut c_void, _: *mut c_void) {}
