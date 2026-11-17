//! Our `taskschd.dll` (ADR 0226): the Task Scheduler 2.0 object model. `CLSID_TaskScheduler` is an
//! in-process class; its `ITaskService` makes definitions (which live in this process, [`defn`])
//! and opens folders and registered tasks, which live in the scheduler process `schedsvc.exe`
//! ([`svc`], over the pipe of [`client`]).
//!
//! Every interface is dual (`taskschd.h`): `IUnknown`, then `IDispatch` (answered without type
//! information: the updater and the tests call the vtables), then its own methods from slot 7.
//! Interface identifiers and method orders are those of the public `taskschd.h`; the behaviour is
//! the sheets' (`specs/windows/taskschd.dll`). The logic is `poly_taskschd`'s (host-tested).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

pub mod client;
pub mod defn;
pub mod svc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
pub use poly_taskschd::hr::*;
use poly_win32_rt as _;

poly_win32_rt::process_heap_allocator!();

pub type HRESULT = i32;
pub type Bstr = *mut u16;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

/// A GUID from its text form's fields (`d1-d2-d3-d4[0..2]-d4[2..8]`).
pub const fn guid(d1: u32, d2: u16, d3: u16, a: u16, b: u64) -> Guid {
    let a = a.to_be_bytes();
    let b = b.to_be_bytes();
    Guid { d1, d2, d3, d4: [a[0], a[1], b[2], b[3], b[4], b[5], b[6], b[7]] }
}

// objidl.h / oaidl.h / unknwn.h.
pub const IID_IUNKNOWN: Guid = guid(0, 0, 0, 0xC000, 0x46);
pub const IID_IDISPATCH: Guid = guid(0x0002_0400, 0, 0, 0xC000, 0x46);
pub const IID_ICLASSFACTORY: Guid = guid(1, 0, 0, 0xC000, 0x46);
pub const IID_IENUMVARIANT: Guid = guid(0x0002_0404, 0, 0, 0xC000, 0x46);
// taskschd.h.
pub const CLSID_TASKSCHEDULER: Guid = guid(0x0f87369f, 0xa4e5, 0x4cfc, 0xbd3e, 0x73e6154572dd);
pub const IID_ITASKSERVICE: Guid = guid(0x2faba4c7, 0x4da9, 0x4013, 0x9697, 0x20cc3fd40f85);
pub const IID_ITASKFOLDER: Guid = guid(0x8cfac062, 0xa080, 0x4c15, 0x9a88, 0xaa7c2af80dfc);
pub const IID_ITASKFOLDERCOLLECTION: Guid = guid(0x79184a66, 0x8664, 0x423f, 0x97f1, 0x637356a5d812);
pub const IID_IREGISTEREDTASK: Guid = guid(0x9c86f320, 0xdee3, 0x4dd1, 0xb972, 0xa303f26b061e);
pub const IID_IREGISTEREDTASKCOLLECTION: Guid = guid(0x86627eb4, 0x42a7, 0x41e4, 0xa4d9, 0xac33a72f2d52);
pub const IID_ITASKDEFINITION: Guid = guid(0xf5bc8fc5, 0x536d, 0x4f77, 0xb852, 0xfbc1356fdeb6);
pub const IID_ITASKSETTINGS: Guid = guid(0x8fd4711d, 0x2d02, 0x4c8c, 0x87e3, 0xeff699de127e);
pub const IID_ITASKSETTINGS2: Guid = guid(0x2c05c3f0, 0x6eed, 0x4c05, 0xa15f, 0xed7d7a98a369);
pub const IID_ITASKSETTINGS3: Guid = guid(0x0ad9d0d7, 0x0c7f, 0x4ebb, 0x9a5f, 0xd1c648dca528);
pub const IID_ITRIGGERCOLLECTION: Guid = guid(0x85df5081, 0x1b24, 0x4f32, 0x878a, 0xd9d14df4cb77);
pub const IID_ITRIGGER: Guid = guid(0x09941815, 0xea89, 0x4b5b, 0x89e0, 0x2a773801fac3);
pub const IID_ITIMETRIGGER: Guid = guid(0xb45747e0, 0xeba7, 0x4276, 0x9f29, 0x85c5bb300006);
pub const IID_IDAILYTRIGGER: Guid = guid(0x126c5cd8, 0xb288, 0x41d5, 0x8dbf, 0xe491446adc5c);
pub const IID_ILOGONTRIGGER: Guid = guid(0x72dade38, 0xfae4, 0x4b3e, 0xbaf4, 0x5d009af02b1c);
pub const IID_IBOOTTRIGGER: Guid = guid(0x2a9c35da, 0xd357, 0x41f4, 0xbbc1, 0x207ac1b1f3cb);
pub const IID_IIDLETRIGGER: Guid = guid(0xd537d2b0, 0x9fb3, 0x4d34, 0x9739, 0x1ff5ce7b1ef3);
pub const IID_IREGISTRATIONTRIGGER: Guid = guid(0x4c8fec3a, 0xc218, 0x4e0c, 0xb23d, 0x629024db91a2);
pub const IID_IEVENTTRIGGER: Guid = guid(0xd45b0167, 0x9653, 0x4eef, 0xb94f, 0x0732ca7af251);
pub const IID_IWEEKLYTRIGGER: Guid = guid(0x5038fc98, 0x82ff, 0x436d, 0x8728, 0xa512a57c9dc1);
pub const IID_IMONTHLYTRIGGER: Guid = guid(0x97c45ef1, 0x6b02, 0x4a1a, 0x9c0e, 0x1ebfba1500ac);
pub const IID_IMONTHLYDOWTRIGGER: Guid = guid(0x77d025a3, 0x90fa, 0x43aa, 0xb52e, 0xcda5499a946a);
pub const IID_ISESSIONSTATECHANGETRIGGER: Guid = guid(0x754da71b, 0x4385, 0x4475, 0x9dd9, 0x598294fa3641);
pub const IID_IACTIONCOLLECTION: Guid = guid(0x02820e19, 0x7b98, 0x4ed2, 0xb2e8, 0xfdccceff619b);
pub const IID_IACTION: Guid = guid(0xbae54997, 0x48b1, 0x4cbe, 0x9965, 0xd6be263ebea4);
pub const IID_IEXECACTION: Guid = guid(0x4c3d624d, 0xfd6b, 0x49a3, 0xb9b7, 0x09cb3cd3f047);
pub const IID_IEXECACTION2: Guid = guid(0xf2a82542, 0xbda5, 0x4e6b, 0x9143, 0xe2bf4f8987b6);
pub const IID_ICOMHANDLERACTION: Guid = guid(0x6d2fd252, 0x75c5, 0x4f66, 0x90ba, 0x2a7d8cc3039f);
pub const IID_IEMAILACTION: Guid = guid(0x10f62c64, 0x7e16, 0x4314, 0xa0c2, 0x0c3683f99d40);
pub const IID_ISHOWMESSAGEACTION: Guid = guid(0x505e9e68, 0xaf89, 0x46b8, 0xa30f, 0x56162a83d537);
pub const IID_IPRINCIPAL: Guid = guid(0xd98d51e5, 0xc9b4, 0x496a, 0xa9c1, 0x18980261cf0f);
pub const IID_IPRINCIPAL2: Guid = guid(0x248919ae, 0xe345, 0x4a6d, 0x8aeb, 0xe0d3165c904e);
pub const IID_IREGISTRATIONINFO: Guid = guid(0x416d8b73, 0xcb41, 0x4ea1, 0x805c, 0x9be9a5ac4a74);
pub const IID_IREPETITIONPATTERN: Guid = guid(0x7fb9acf1, 0x26be, 0x400e, 0x85b5, 0x294b9c75dfd6);
pub const IID_IIDLESETTINGS: Guid = guid(0x84594461, 0x0053, 0x4342, 0xa8fd, 0x088fabf11f32);
pub const IID_INETWORKSETTINGS: Guid = guid(0x9f7dea84, 0xc30b, 0x4245, 0x80b6, 0x00e9f646f1b4);
pub const IID_IRUNNINGTASK: Guid = guid(0x653758fb, 0x7b9a, 0x4f1e, 0xa471, 0xbeeb8e9b834e);
pub const IID_IRUNNINGTASKCOLLECTION: Guid = guid(0x6a67614b, 0x6828, 0x4fec, 0xaa54, 0x6d52e8f1f2db);

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "oleaut32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "oleaut32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SysAllocStringLen(s: *const u16, n: u32) -> Bstr;
    fn SysStringLen(s: Bstr) -> u32;
}

/// A line in the kernel log.
pub fn log(text: &str) {
    // SAFETY: the service reads `text.len()` bytes.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

// ------------------------------------------------------------------------------- strings

/// A new BSTR of `s`.
pub fn bstr(s: &str) -> Bstr {
    let w: Vec<u16> = s.encode_utf16().collect();
    // SAFETY: the call copies `w.len()` units.
    unsafe { SysAllocStringLen(w.as_ptr(), w.len() as u32) }
}

/// A BSTR as text (`None` for NULL).
pub fn from_bstr(b: Bstr) -> Option<String> {
    if b.is_null() {
        return None;
    }
    // SAFETY: a BSTR holds `SysStringLen` units.
    let u = unsafe { core::slice::from_raw_parts(b, SysStringLen(b) as usize) };
    Some(String::from_utf16_lossy(u))
}

/// Write an optional text to a `BSTR*` out parameter (NULL for none).
pub unsafe fn put_text(out: *mut Bstr, v: &Option<String>) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = v.as_deref().map_or(core::ptr::null_mut(), bstr) };
    S_OK
}

/// `oaidl.h` `VARIANT` (24 bytes on x64, 16 on x86): the type, three reserved words, the value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Variant {
    pub vt: u16,
    pub r: [u16; 3],
    pub v: [usize; 2],
}

pub const VT_EMPTY: u16 = 0;
pub const VT_NULL: u16 = 1;
pub const VT_I2: u16 = 2;
pub const VT_I4: u16 = 3;
pub const VT_BSTR: u16 = 8;
pub const VT_DISPATCH: u16 = 9;
pub const VT_ERROR: u16 = 10;
pub const VT_INT: u16 = 22;

impl Variant {
    /// The text of a `VT_BSTR` variant (`Some(None)` for a NULL BSTR); `None` for another type.
    pub fn text(&self) -> Option<Option<String>> {
        (self.vt == VT_BSTR).then(|| from_bstr(self.v[0] as Bstr))
    }

    /// The number of a `VT_I4`, `VT_I2` or `VT_INT` variant.
    pub fn int(&self) -> Option<i32> {
        match self.vt {
            VT_I4 | VT_INT => Some(self.v[0] as u32 as i32),
            VT_I2 => Some(self.v[0] as u16 as i16 as i32),
            _ => None,
        }
    }

    /// Whether the variant holds nothing (empty, NULL or a missing optional argument).
    pub fn is_empty(&self) -> bool {
        matches!(self.vt, VT_EMPTY | VT_NULL | VT_ERROR)
    }

    /// An optional text argument: a BSTR (NULL as empty), or nothing.
    pub fn opt_text(&self) -> Option<String> {
        match self.text() {
            Some(t) => Some(t.unwrap_or_default()),
            None => None,
        }
    }
}

// ------------------------------------------------------------------------------- locks

/// A small spin lock (held for a few instructions, never across a call out).
pub struct Lock<T> {
    busy: AtomicBool,
    v: UnsafeCell<T>,
}

// SAFETY: the value is reached only with the lock held.
unsafe impl<T: Send> Sync for Lock<T> {}
// SAFETY: as above.
unsafe impl<T: Send> Send for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(v: T) -> Lock<T> {
        Lock { busy: AtomicBool::new(false), v: UnsafeCell::new(v) }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.busy.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held.
        let r = f(unsafe { &mut *self.v.get() });
        self.busy.store(false, Ordering::Release);
        r
    }
}

// ------------------------------------------------------------------------------- objects

/// A vtable of `N` entries in a static.
#[repr(transparent)]
pub struct Vt<const N: usize>(pub [*const c_void; N]);
// SAFETY: function addresses only.
unsafe impl<const N: usize> Sync for Vt<N> {}

impl<const N: usize> Vt<N> {
    pub fn ptr(&'static self) -> *const c_void {
        self.0.as_ptr() as *const c_void
    }
}

/// The start of every object: the primary vtable, a second one for an interface that does not
/// derive from the first (`ITaskSettings2`, `IPrincipal2`; NULL when there is none), the count,
/// the interfaces each pointer answers, and the destructor.
#[repr(C)]
pub struct Head {
    pub vtbl: *const c_void,
    pub vtbl2: *const c_void,
    refs: AtomicU32,
    iids: &'static [Guid],
    iids2: &'static [Guid],
    drop: unsafe fn(*mut c_void),
}

#[repr(C)]
pub struct Obj<T> {
    pub h: Head,
    pub d: T,
}

unsafe fn drop_obj<T>(p: *mut c_void) {
    // SAFETY: made by `Box::into_raw` of an `Obj<T>` in `make`.
    drop(unsafe { Box::from_raw(p as *mut Obj<T>) });
}

/// A new object with one reference: the primary vtable answers `IUnknown`, `IDispatch` and `iids`.
pub fn make<T>(vtbl: *const c_void, iids: &'static [Guid], d: T) -> *mut c_void {
    make2(vtbl, iids, core::ptr::null(), &[], d)
}

/// A new object with a second interface pointer (`vtbl2`, answering `iids2`).
pub fn make2<T>(vtbl: *const c_void, iids: &'static [Guid], vtbl2: *const c_void, iids2: &'static [Guid], d: T) -> *mut c_void {
    let o = Obj { h: Head { vtbl, vtbl2, refs: AtomicU32::new(1), iids, iids2, drop: drop_obj::<T> }, d };
    Box::into_raw(Box::new(o)) as *mut c_void
}

/// The data of an object of this DLL (its primary pointer).
pub unsafe fn data<'a, T>(this: *mut c_void) -> &'a T {
    // SAFETY: the caller passes the primary pointer of an `Obj<T>`.
    unsafe { &(*(this as *mut Obj<T>)).d }
}

/// The primary pointer of an object, from its second interface pointer.
pub fn primary(this2: *mut c_void) -> *mut c_void {
    (this2 as usize - core::mem::size_of::<usize>()) as *mut c_void
}

fn head<'a>(this: *mut c_void) -> &'a Head {
    // SAFETY: every object of this DLL starts with a `Head`.
    unsafe { &*(this as *const Head) }
}

pub unsafe extern "system" fn query_interface(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if iid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's IID.
    let iid = unsafe { &*iid };
    let h = head(this);
    let p = if *iid == IID_IUNKNOWN || *iid == IID_IDISPATCH || h.iids.contains(iid) {
        this
    } else if !h.vtbl2.is_null() && h.iids2.contains(iid) {
        (this as usize + core::mem::size_of::<usize>()) as *mut c_void
    } else {
        return E_NOINTERFACE;
    };
    h.refs.fetch_add(1, Ordering::Relaxed);
    // SAFETY: as above.
    unsafe { *out = p };
    S_OK
}

pub unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    head(this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

pub unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let h = head(this);
    let n = h.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        let d = h.drop;
        // SAFETY: the last reference went.
        unsafe { d(this) };
    }
    n
}

pub unsafe extern "system" fn query_interface2(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a second interface pointer of one of our objects.
    unsafe { query_interface(primary(this), iid, out) }
}

pub unsafe extern "system" fn add_ref2(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { add_ref(primary(this)) }
}

pub unsafe extern "system" fn release2(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { release(primary(this)) }
}

/// `IDispatch::GetTypeInfoCount`: no type information.
pub unsafe extern "system" fn type_info_count(_this: *mut c_void, n: *mut u32) -> HRESULT {
    if n.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *n = 0 };
    S_OK
}

/// `GetTypeInfo`, `GetIDsOfNames`, `Invoke`: not available without type information.
pub unsafe extern "system" fn not_impl(_this: *mut c_void) -> HRESULT {
    E_NOTIMPL
}

/// The seven entries every dual interface starts with.
#[macro_export]
macro_rules! dual {
    ($($rest:expr),* $(,)?) => {
        $crate::Vt([
            $crate::query_interface as *const core::ffi::c_void,
            $crate::add_ref as *const core::ffi::c_void,
            $crate::release as *const core::ffi::c_void,
            $crate::type_info_count as *const core::ffi::c_void,
            $crate::not_impl as *const core::ffi::c_void,
            $crate::not_impl as *const core::ffi::c_void,
            $crate::not_impl as *const core::ffi::c_void,
            $($rest as *const core::ffi::c_void),*
        ])
    };
}

/// The same for a second interface pointer.
#[macro_export]
macro_rules! dual2 {
    ($($rest:expr),* $(,)?) => {
        $crate::Vt([
            $crate::query_interface2 as *const core::ffi::c_void,
            $crate::add_ref2 as *const core::ffi::c_void,
            $crate::release2 as *const core::ffi::c_void,
            $crate::type_info_count as *const core::ffi::c_void,
            $crate::not_impl as *const core::ffi::c_void,
            $crate::not_impl as *const core::ffi::c_void,
            $crate::not_impl as *const core::ffi::c_void,
            $($rest as *const core::ffi::c_void),*
        ])
    };
}

/// AddRef / Release of any COM object (through its vtable).
pub unsafe fn com_add_ref(p: *mut c_void) {
    // SAFETY: a live COM object: AddRef is entry 1.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(1));
        f(p);
    }
}

pub unsafe fn com_release(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    // SAFETY: a live COM object: Release is entry 2.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(2));
        f(p);
    }
}

/// Hand an object out through an out parameter (or release it when there is none).
pub unsafe fn hand_out(out: *mut *mut c_void, p: *mut c_void) {
    if out.is_null() {
        // SAFETY: our own new object.
        unsafe { release(p) };
    } else {
        // SAFETY: the caller's out parameter.
        unsafe { *out = p };
    }
}

/// Set an out pointer to NULL if there is one.
pub unsafe fn clear(out: *mut *mut c_void) {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
}

/// A `VARIANT_BOOL` out parameter.
pub unsafe fn put_bool(out: *mut i16, v: i16) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = v };
    S_OK
}

/// A numeric out parameter.
pub unsafe fn put_val<T: Copy>(out: *mut T, v: T) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = v };
    S_OK
}

// ------------------------------------------------------------------------------- the enumerator

/// `IEnumVARIANT` over a list of objects (`get__NewEnum` of the collections).
struct EnumData {
    items: Vec<usize>,
    at: Lock<usize>,
}

impl Drop for EnumData {
    fn drop(&mut self) {
        for &p in &self.items {
            // SAFETY: references this enumerator holds.
            unsafe { com_release(p as *mut c_void) };
        }
    }
}

unsafe extern "system" fn enum_next(this: *mut c_void, n: u32, out: *mut Variant, fetched: *mut u32) -> HRESULT {
    // SAFETY: an enumerator of ours.
    let e = unsafe { data::<EnumData>(this) };
    if out.is_null() {
        return E_POINTER;
    }
    let mut got = 0u32;
    e.at.with(|at| {
        while got < n && *at < e.items.len() {
            let p = e.items[*at] as *mut c_void;
            // SAFETY: a live object of the list; the variant takes one reference.
            unsafe {
                com_add_ref(p);
                *out.add(got as usize) = Variant { vt: VT_DISPATCH, r: [0; 3], v: [p as usize, 0] };
            }
            *at += 1;
            got += 1;
        }
    });
    if !fetched.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *fetched = got };
    }
    if got == n { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_skip(this: *mut c_void, n: u32) -> HRESULT {
    // SAFETY: an enumerator of ours.
    let e = unsafe { data::<EnumData>(this) };
    e.at.with(|at| {
        *at += n as usize;
        if *at > e.items.len() {
            *at = e.items.len();
            S_FALSE
        } else {
            S_OK
        }
    })
}

unsafe extern "system" fn enum_reset(this: *mut c_void) -> HRESULT {
    // SAFETY: an enumerator of ours.
    unsafe { data::<EnumData>(this) }.at.with(|at| *at = 0);
    S_OK
}

unsafe extern "system" fn enum_clone(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: an enumerator of ours.
    let e = unsafe { data::<EnumData>(this) };
    let items = e.items.clone();
    for &p in &items {
        // SAFETY: live objects; the clone holds its own references.
        unsafe { com_add_ref(p as *mut c_void) };
    }
    let at = e.at.with(|a| *a);
    // SAFETY: the caller's out parameter.
    unsafe { *out = make(ENUM_VTBL.ptr(), ENUM_IIDS, EnumData { items, at: Lock::new(at) }) };
    S_OK
}

static ENUM_VTBL: Vt<7> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    enum_next as *const c_void,
    enum_skip as *const c_void,
    enum_reset as *const c_void,
    enum_clone as *const c_void,
]);
static ENUM_IIDS: &[Guid] = &[IID_IENUMVARIANT];

/// A new enumerator over `items` (each with one reference that the enumerator takes over).
pub unsafe fn new_enum(items: Vec<*mut c_void>, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        for p in items {
            // SAFETY: the references handed in.
            unsafe { com_release(p) };
        }
        return E_POINTER;
    }
    let items = items.into_iter().map(|p| p as usize).collect();
    // SAFETY: the caller's out parameter.
    unsafe { *out = make(ENUM_VTBL.ptr(), ENUM_IIDS, EnumData { items, at: Lock::new(0) }) };
    S_OK
}

// ------------------------------------------------------------------------------- the class

unsafe extern "system" fn factory_create(_this: *mut c_void, outer: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    if !outer.is_null() {
        return CLASS_E_NOAGGREGATION;
    }
    let s = svc::new_service();
    // SAFETY: our new object; the query takes its own reference.
    let hr = unsafe { query_interface(s, iid, out) };
    // SAFETY: as above.
    unsafe { release(s) };
    hr
}

unsafe extern "system" fn factory_lock(_this: *mut c_void, _lock: i32) -> HRESULT {
    S_OK
}

unsafe extern "system" fn factory_add_ref(_this: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn factory_release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn factory_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID.
    let ok = !iid.is_null() && unsafe { *iid == IID_IUNKNOWN || *iid == IID_ICLASSFACTORY };
    // SAFETY: the caller's out parameter.
    unsafe { *out = if ok { this } else { core::ptr::null_mut() } };
    if ok { S_OK } else { E_NOINTERFACE }
}

static FACTORY_VTBL: Vt<5> = Vt([
    factory_qi as *const c_void,
    factory_add_ref as *const c_void,
    factory_release as *const c_void,
    factory_create as *const c_void,
    factory_lock as *const c_void,
]);

/// The class factory: a static object (its count does not matter).
#[repr(C)]
struct Factory {
    vtbl: *const c_void,
}
// SAFETY: a pointer to a static table only.
unsafe impl Sync for Factory {}
static FACTORY: Factory = Factory { vtbl: &FACTORY_VTBL as *const Vt<5> as *const c_void };

/// `DllGetClassObject`: the factory of `CLSID_TaskScheduler`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: the caller's CLSID.
    if clsid.is_null() || unsafe { *clsid } != CLSID_TASKSCHEDULER {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    // SAFETY: a static object; the query writes `out`.
    unsafe { factory_qi(&FACTORY as *const Factory as *mut c_void, iid, out) }
}

/// `DllCanUnloadNow`: the DLL stays (objects may be alive in any thread).
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

/// `DllRegisterServer` / `DllUnregisterServer`: the class is registered by Poly's layout (ADR 0226).
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    S_OK
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    S_OK
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn taskschd_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
