//! What the COM objects of this DLL share (ADR 0223): a header with the table pointer, the
//! reference count and the interfaces the object answers; `IUnknown`'s three methods; the class
//! factory `DllGetClassObject` hands to `ole32` for the classes in [`poly_com::inproc`]; small
//! helpers to call another object's methods through its table and to hand out strings.
//!
//! Every object here has one table: its interfaces form one chain (`IShellItem2` extends
//! `IShellItem`, `ITaskbarList3` extends `ITaskbarList2` and `ITaskbarList`), so one pointer
//! serves all of them (Learn: "QueryInterface", "Rules for Implementing QueryInterface").

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
pub use poly_com::guid::Guid;
use poly_com::inproc::{
    CLSID_DESTINATION_LIST, CLSID_FILE_OPERATION, CLSID_SHELL, CLSID_SHELL_LINK, CLSID_SHELL_WINDOWS,
    CLSID_TASKBAR_LIST, IID_ICLASS_FACTORY, IID_IUNKNOWN,
};

pub type Hresult = i32;
pub use poly_com::hresult::{
    CLASS_E_NOAGGREGATION, E_FAIL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED,
    S_FALSE, S_OK,
};
/// `CLASS_E_CLASSNOTAVAILABLE` (`winerror.h`).
pub const CLASS_E_CLASSNOTAVAILABLE: Hresult = 0x8004_0111_u32 as i32;

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> *mut u8;
    fn CoTaskMemFree(p: *mut u8);
    fn CoGetApartmentType(kind: *mut i32, qualifier: *mut i32) -> Hresult;
}

/// A guid from its parts, as the SDK headers write them.
pub const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
    Guid { data1, data2, data3, data4 }
}

/// A method table: `N` function pointers.
#[repr(transparent)]
pub struct Vtbl<const N: usize>(pub [*const c_void; N]);
// SAFETY: the table only holds function pointers and is never written.
unsafe impl<const N: usize> Sync for Vtbl<N> {}

/// The start of every object of this DLL.
#[repr(C)]
pub struct Head {
    pub vtbl: *const c_void,
    pub refs: AtomicU32,
    /// The interfaces `QueryInterface` answers besides `IUnknown`.
    pub iids: &'static [Guid],
    /// Frees the object when the last reference goes (`None`: the object lives for ever).
    pub drop: Option<unsafe fn(*mut Head)>,
}

impl Head {
    pub fn new<const N: usize>(vtbl: &'static Vtbl<N>, iids: &'static [Guid], drop: unsafe fn(*mut Head)) -> Head {
        Head { vtbl: (vtbl as *const Vtbl<N>).cast(), refs: AtomicU32::new(1), iids, drop: Some(drop) }
    }
}

/// `drop` for an object made by [`into_raw`] whose first field is its `Head`.
pub unsafe fn drop_box<T>(h: *mut Head) {
    // SAFETY: `h` is the address of a `Box<T>` whose first field is the head.
    drop(unsafe { alloc::boxed::Box::from_raw(h.cast::<T>()) });
}

/// Move `obj` to the heap; the pointer carries the one reference.
pub fn into_raw<T>(obj: T) -> *mut T {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(obj))
}

pub unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: every object of this DLL starts with a `Head`.
    unsafe { (*this.cast::<Head>()).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

pub unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: every object of this DLL starts with a `Head`; freed when the last reference goes.
    unsafe {
        let h = this.cast::<Head>();
        let left = (*h).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0
            && let Some(d) = (*h).drop
        {
            d(h);
        }
        left
    }
}

pub unsafe extern "system" fn query_interface(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; `this` starts with a `Head`.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        if iid != IID_IUNKNOWN && !(*this.cast::<Head>()).iids.contains(&iid) {
            return E_NOINTERFACE;
        }
        add_ref(this);
        *out = this;
    }
    S_OK
}

/// Hand out `object` (holding one reference) as `iid`: the caller's reference is taken by
/// `QueryInterface` and the creator's is dropped, so a refused interface frees the object.
pub unsafe fn hand_out(object: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: a live object of this DLL; the caller's out parameter.
    unsafe {
        let hr = query_interface(object, iid, out);
        release(object);
        hr
    }
}

/// Call method `slot` of another object's table.
#[macro_export]
macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let obj = $obj as *mut core::ffi::c_void;
        let vtbl = *(obj as *const *const usize);
        let f: unsafe extern "system" fn(*mut core::ffi::c_void $(, $t)*) -> $r = core::mem::transmute(*vtbl.add($slot));
        f(obj $(, $a)*)
    }};
}

/// Whether the calling thread has started COM (an apartment of its own or the implicit
/// multithreaded one): `CO_E_NOTINITIALIZED` otherwise.
pub fn com_started() -> Result<(), Hresult> {
    let (mut kind, mut qualifier) = (0, 0);
    // SAFETY: two locals.
    let hr = unsafe { CoGetApartmentType(&mut kind, &mut qualifier) };
    if hr < 0 { Err(hr) } else { Ok(()) }
}

/// `text` (without a NUL) in a `CoTaskMemAlloc` block with the NUL, for a `LPWSTR *` out parameter.
pub unsafe fn task_string(text: &[u16], out: *mut *mut u16) -> Hresult {
    // SAFETY: a fresh block of the right size; the caller's out parameter.
    unsafe {
        let p = CoTaskMemAlloc(2 * (text.len() + 1)).cast::<u16>();
        if p.is_null() {
            *out = core::ptr::null_mut();
            return E_OUTOFMEMORY;
        }
        core::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
        *p.add(text.len()) = 0;
        *out = p;
    }
    S_OK
}

/// Free a `CoTaskMemAlloc` block.
pub unsafe fn task_free(p: *mut c_void) {
    // SAFETY: a block of the task allocator (NULL is ignored).
    unsafe { CoTaskMemFree(p.cast()) };
}

/// A NUL-terminated string of the caller, without the NUL (empty for NULL).
pub unsafe fn wstr<'a>(p: *const u16) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string of the caller.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// `text` with a NUL, for the functions that take one.
pub fn nul(text: &[u16]) -> alloc::vec::Vec<u16> {
    let mut v = alloc::vec::Vec::with_capacity(text.len() + 1);
    v.extend_from_slice(text);
    v.push(0);
    v
}

// ----- the class factory --------------------------------------------------------------------------

/// The class objects (`IClassFactory`): one per class, living for ever.
#[repr(C)]
struct Factory {
    head: Head,
    create: unsafe fn(*const Guid, *mut *mut c_void) -> Hresult,
}
// SAFETY: the factories are immutable apart from their counts, which are atomic.
unsafe impl Sync for Factory {}

/// `IClassFactory` (`unknwn.h`): `IUnknown`, then `CreateInstance` 3, `LockServer` 4.
static FACTORY_VTBL: Vtbl<5> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    factory_create as *const c_void,
    factory_lock as *const c_void,
]);
static FACTORY_IIDS: [Guid; 1] = [IID_ICLASS_FACTORY];

macro_rules! factory {
    ($name:ident, $create:path) => {
        static $name: Factory = Factory {
            head: Head {
                vtbl: (&FACTORY_VTBL as *const Vtbl<5>).cast(),
                refs: AtomicU32::new(1),
                iids: &FACTORY_IIDS,
                drop: None,
            },
            create: $create,
        };
    };
}
factory!(FILE_OPERATION_FACTORY, crate::fileop::create);
factory!(TASKBAR_LIST_FACTORY, crate::taskbar::create_taskbar_list);
factory!(DESTINATION_LIST_FACTORY, crate::taskbar::create_destination_list);
factory!(SHELL_FACTORY, crate::dispatch::create_shell_app);
factory!(SHELL_WINDOWS_FACTORY, crate::dispatch::create_shell_windows);
factory!(SHELL_LINK_FACTORY, crate::link::create);

unsafe extern "system" fn factory_create(
    this: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter; `this` is one of the factories above.
    unsafe {
        *out = core::ptr::null_mut();
        if !outer.is_null() {
            return CLASS_E_NOAGGREGATION;
        }
        ((*this.cast::<Factory>()).create)(iid, out)
    }
}

unsafe extern "system" fn factory_lock(_this: *mut c_void, _lock: i32) -> Hresult {
    S_OK
}

/// `DllGetClassObject(rclsid, riid, ppv)` (Learn): the class object of one of this DLL's classes
/// ([`poly_com::inproc`]), `CLASS_E_CLASSNOTAVAILABLE` for any other class.
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
        if clsid.is_null() {
            return E_INVALIDARG;
        }
        let clsid = clsid.read_unaligned();
        let factory: &'static Factory = if clsid == CLSID_FILE_OPERATION {
            &FILE_OPERATION_FACTORY
        } else if clsid == CLSID_TASKBAR_LIST {
            &TASKBAR_LIST_FACTORY
        } else if clsid == CLSID_DESTINATION_LIST {
            &DESTINATION_LIST_FACTORY
        } else if clsid == CLSID_SHELL {
            &SHELL_FACTORY
        } else if clsid == CLSID_SHELL_WINDOWS {
            &SHELL_WINDOWS_FACTORY
        } else if clsid == CLSID_SHELL_LINK {
            &SHELL_LINK_FACTORY
        } else {
            return CLASS_E_CLASSNOTAVAILABLE;
        };
        query_interface((factory as *const Factory).cast_mut().cast(), iid, out)
    }
}

/// `DllCanUnloadNow` (Learn): the DLL stays loaded (`S_FALSE`).
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> Hresult {
    S_FALSE
}
