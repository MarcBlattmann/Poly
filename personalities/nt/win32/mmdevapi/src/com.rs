//! COM plumbing of `mmdevapi`: an object is a method table, a reference count, the interfaces it
//! answers and its data (as `hnetcfg`'s); the interface ids (`mmdeviceapi.h`, `audioclient.h`,
//! `audiopolicy.h`); a lock; and the methods no sheet specifies, which answer `E_NOTIMPL` with
//! the method's own argument count (a 32-bit `stdcall` callee pops its arguments).

use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::guid::Guid;
use poly_kernelbase::wasapi as wa;

pub type Hresult = i32;

pub const fn hr(v: u32) -> Hresult {
    v as i32
}

pub const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid { data1, data2, data3, data4 }
}

pub const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_ICLASS_FACTORY: Guid = guid(1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_IMM_DEVICE_ENUMERATOR: Guid =
    guid(0xA956_64D2, 0x9614, 0x4F35, [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6]);
pub const IID_IMM_DEVICE_COLLECTION: Guid =
    guid(0x0BD7_A1BE, 0x7A1A, 0x44DB, [0x83, 0x97, 0xCC, 0x53, 0x92, 0x38, 0x7B, 0x5E]);
pub const IID_IMM_DEVICE: Guid = guid(0xD666_063F, 0x1587, 0x4E43, [0x81, 0xF1, 0xB9, 0x48, 0xE8, 0x07, 0x36, 0x3F]);
pub const IID_IAUDIO_CLIENT: Guid = guid(0x1CB9_AD4C, 0xDBFA, 0x4C32, [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2]);
pub const IID_IAUDIO_CLIENT2: Guid =
    guid(0x7267_78CD, 0xF60A, 0x4EDA, [0x82, 0xDE, 0xE4, 0x76, 0x10, 0xCD, 0x78, 0xAA]);
pub const IID_IAUDIO_CLIENT3: Guid =
    guid(0x7ED4_EE07, 0x8E67, 0x4CD4, [0x8C, 0x1A, 0x2B, 0x7A, 0x59, 0x87, 0xAD, 0x42]);
pub const IID_IAUDIO_RENDER_CLIENT: Guid =
    guid(0xF294_ACFC, 0x3146, 0x4483, [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2]);
pub const IID_IAUDIO_CAPTURE_CLIENT: Guid =
    guid(0xC8AD_BD64, 0xE71E, 0x48A0, [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17]);
pub const IID_IAUDIO_CLOCK: Guid = guid(0xCD63_314F, 0x3FBA, 0x4A1B, [0x81, 0x2C, 0xEF, 0x96, 0x35, 0x87, 0x28, 0xE7]);
/// audioclient.h: `IAudioClock2 : IUnknown` (slot 3 `GetDevicePosition(UINT64*, UINT64*)`), which
/// an application gets with `QueryInterface` on the clock (Learn, "IAudioClock2").
pub const IID_IAUDIO_CLOCK2: Guid = guid(0x6F49_FF73, 0x6727, 0x49AC, [0xA0, 0x08, 0xD9, 0x8C, 0xF5, 0xE7, 0x00, 0x48]);
pub const IID_IAUDIO_SESSION_CONTROL: Guid =
    guid(0xF4B1_A599, 0x7266, 0x4319, [0xA8, 0xCA, 0xE7, 0x0A, 0xCB, 0x11, 0xE8, 0xCD]);
pub const IID_ISIMPLE_AUDIO_VOLUME: Guid =
    guid(0x87CE_5498, 0x68D6, 0x44E5, [0x92, 0x15, 0x6D, 0xA4, 0x7E, 0xF8, 0x83, 0xD8]);

pub const S_OK: Hresult = 0;
pub const E_POINTER: Hresult = hr(wa::E_POINTER);
pub const E_INVALIDARG: Hresult = hr(wa::E_INVALIDARG);
pub const E_NOINTERFACE: Hresult = hr(wa::E_NOINTERFACE);
pub const E_NOTIMPL: Hresult = hr(wa::E_NOTIMPL);
pub const E_OUTOFMEMORY: Hresult = hr(wa::E_OUTOFMEMORY);

/// A method table of `N` function pointers.
#[repr(transparent)]
pub struct Vtbl<const N: usize>(pub [*const c_void; N]);
// SAFETY: function pointers only, never written.
unsafe impl<const N: usize> Sync for Vtbl<N> {}

/// A COM object.
#[repr(C)]
pub struct Object<T> {
    vtbl: *const c_void,
    refs: AtomicU32,
    iids: &'static [Guid],
    pub data: T,
}

pub fn new_object<T, const N: usize>(vtbl: &'static Vtbl<N>, iids: &'static [Guid], data: T) -> *mut c_void {
    Box::into_raw(Box::new(Object { vtbl: (vtbl as *const Vtbl<N>).cast(), refs: AtomicU32::new(1), iids, data }))
        .cast()
}

/// The data of our object `p`.
///
/// # Safety
/// `p` is an `Object<T>` of ours, alive.
pub unsafe fn data<'a, T>(p: *mut c_void) -> &'a T {
    // SAFETY: (caller).
    unsafe { &(*p.cast::<Object<T>>()).data }
}

pub unsafe extern "system" fn query<T>(p: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; `p` is our object.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        if !(*p.cast::<Object<T>>()).iids.contains(&iid) {
            return E_NOINTERFACE;
        }
        add_ref::<T>(p);
        *out = p;
    }
    S_OK
}

pub unsafe extern "system" fn add_ref<T>(p: *mut c_void) -> u32 {
    // SAFETY: our object.
    unsafe { (*p.cast::<Object<T>>()).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

pub unsafe extern "system" fn release<T>(p: *mut c_void) -> u32 {
    // SAFETY: our object; freed with its last reference.
    unsafe {
        let left = (*p.cast::<Object<T>>()).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            drop(Box::from_raw(p.cast::<Object<T>>()));
        }
        left
    }
}

/// Hand out a fresh object as `iid` (its own reference is given back).
///
/// # Safety
/// `object` is a fresh object of ours; `out` the caller's.
pub unsafe fn hand_out<T>(object: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: (caller).
    unsafe {
        let r = query::<T>(object, iid, out);
        release::<T>(object);
        r
    }
}

/// Call `IUnknown::AddRef` / `Release` on a foreign object.
///
/// # Safety
/// `p` is a live COM object.
pub unsafe fn foreign_add_ref(p: *mut c_void) {
    // SAFETY: (caller) slot 1 of its method table.
    unsafe {
        let vtbl = *(p as *const *const usize);
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vtbl.add(1));
        f(p);
    }
}

/// # Safety
/// `p` is a live COM object we hold a reference to.
pub unsafe fn foreign_release(p: *mut c_void) {
    // SAFETY: (caller) slot 2 of its method table.
    unsafe {
        let vtbl = *(p as *const *const usize);
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vtbl.add(2));
        f(p);
    }
}

// ----- methods no sheet specifies (by argument count, `this` included)

pub unsafe extern "system" fn not_impl2(_: *mut c_void, _: usize) -> Hresult {
    E_NOTIMPL
}
pub unsafe extern "system" fn not_impl3(_: *mut c_void, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
/// `SetMasterVolume(float, LPCGUID)`: a float and a pointer.
pub unsafe extern "system" fn not_impl_f32_ptr(_: *mut c_void, _: f32, _: usize) -> Hresult {
    E_NOTIMPL
}
/// `GetBufferSizeLimits(const WAVEFORMATEX*, BOOL, REFERENCE_TIME*, REFERENCE_TIME*)`.
pub unsafe extern "system" fn not_impl5(_: *mut c_void, _: usize, _: i32, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}

// ----- a lock (Windows SRW lock: the objects are used from any thread)

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
}

pub struct Locked<T> {
    lock: UnsafeCell<usize>,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only through `with`, under the SRW lock.
unsafe impl<T: Send> Sync for Locked<T> {}
// SAFETY: as above.
unsafe impl<T: Send> Send for Locked<T> {}

impl<T> Locked<T> {
    pub const fn new(v: T) -> Locked<T> {
        Locked { lock: UnsafeCell::new(0), value: UnsafeCell::new(v) }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        // SAFETY: an SRW lock word (zero is SRWLOCK_INIT) that does not move while it is held; the
        // value is reached only under it, and `f` does not take it again.
        unsafe {
            AcquireSRWLockExclusive(self.lock.get());
            let r = f(&mut *self.value.get());
            ReleaseSRWLockExclusive(self.lock.get());
            r
        }
    }
}
