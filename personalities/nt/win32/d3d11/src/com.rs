//! What every object of this DLL shares: the header (vtable, reference count, the interfaces it
//! answers, a destructor), `IUnknown`, interface identifiers of the public SDK headers
//! (`d3d11.h`, `dxgi.h`, `dxgi1_2.h`), `HRESULT`s and the `stub:` log.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

pub type HRESULT = i32;

// winerror.h
pub const S_OK: HRESULT = 0;
pub const S_FALSE: HRESULT = 1;
pub const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
pub const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: HRESULT = 0x8007_000Eu32 as i32;
pub const DXGI_ERROR_NOT_FOUND: HRESULT = 0x887A_0002u32 as i32;
pub const DXGI_ERROR_UNSUPPORTED: HRESULT = 0x887A_0004u32 as i32;
pub const DXGI_ERROR_FRAME_STATISTICS_DISJOINT: HRESULT = 0x887A_000Bu32 as i32;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
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

pub const IID_IUNKNOWN: Guid = guid(0, 0, 0, 0xC000, 0x0000_0000_0046);
pub const IID_ID3D11DEVICE: Guid = guid(0xdb6f6ddb, 0xac77, 0x4e88, 0x8253, 0x819df9bbf140);
pub const IID_ID3D11DEVICECONTEXT: Guid = guid(0xc0bfa96c, 0xe089, 0x44fb, 0x8eaf, 0x26f8796190da);
pub const IID_ID3D11DEVICECHILD: Guid = guid(0x1841e5c8, 0x16b0, 0x489b, 0xbcc8, 0x44cfb0d5deae);
pub const IID_ID3D11RESOURCE: Guid = guid(0xdc8e63f3, 0xd12b, 0x4952, 0xb47b, 0x5e45026a862d);
pub const IID_ID3D11TEXTURE2D: Guid = guid(0x6f15aaf2, 0xd208, 0x4e89, 0x9ab4, 0x489535d34f9c);
pub const IID_ID3D11VIEW: Guid = guid(0x839d1216, 0xbb2e, 0x412b, 0xb7f4, 0xa9dbebe08ed1);
pub const IID_ID3D11RENDERTARGETVIEW: Guid = guid(0xdfdba067, 0x0b8d, 0x4865, 0x875b, 0xd7b4516cc164);
pub const IID_ID3D11MULTITHREAD: Guid = guid(0x9b7e4e00, 0x342c, 0x4106, 0xa19f, 0x4f2704f689f0);
pub const IID_IDXGIOBJECT: Guid = guid(0xaec22fb8, 0x76f3, 0x4639, 0x9be0, 0x28eb43a67a2e);
pub const IID_IDXGIDEVICE: Guid = guid(0x54ec77fa, 0x1377, 0x44e6, 0x8c32, 0x88fd5f44c84c);
pub const IID_IDXGIDEVICE1: Guid = guid(0x77db970f, 0x6276, 0x48ba, 0xba28, 0x070143b4392c);
pub const IID_IDXGIDEVICESUBOBJECT: Guid = guid(0x3d3e0379, 0xf9de, 0x4d58, 0xbb6c, 0x18d62992f1a6);
pub const IID_IDXGIRESOURCE: Guid = guid(0x035f3ab4, 0x482e, 0x4e50, 0xb41f, 0x8a7f8bd8960b);
pub const IID_IDXGISURFACE: Guid = guid(0xcafcb56c, 0x6ac3, 0x4889, 0xbf47, 0x9e23bbd260ec);
pub const IID_IDXGISWAPCHAIN: Guid = guid(0x310d36a0, 0xd2e7, 0x4c0a, 0xaa04, 0x6a9d23b8886a);
pub const IID_IDXGISWAPCHAIN1: Guid = guid(0x790a45f7, 0x0d42, 0x4876, 0x983a, 0x0a55cfe6f4aa);

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]

#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

/// A line in the kernel log (`stub: d3d11: ...` for what is not implemented, as the fail-only
/// exports log).
pub fn log(text: &str) {
    // SAFETY: the service reads `text.len()` bytes.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

pub fn log_iid(what: &str, iid: &Guid) {
    let d = iid.d4;
    log(&alloc::format!(
        "stub: d3d11: {what} has no interface {{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        iid.d1,
        iid.d2,
        iid.d3,
        d[0],
        d[1],
        d[2],
        d[3],
        d[4],
        d[5],
        d[6],
        d[7]
    ));
}

/// The start of every object.
#[repr(C)]
pub struct Obj {
    pub vtbl: *const c_void,
    pub refs: AtomicU32,
    /// The interfaces `QueryInterface` answers with this object besides `IUnknown`.
    pub iids: &'static [Guid],
    /// Other interfaces, answered by other objects (tear-offs): `None` when there is none.
    pub more: Option<unsafe fn(*mut c_void, &Guid) -> *mut c_void>,
    /// Frees the object (the last reference went).
    pub dtor: unsafe fn(*mut c_void),
    /// For the `stub:` lines.
    pub name: &'static str,
}

impl Obj {
    pub fn new(vtbl: *const c_void, iids: &'static [Guid], dtor: unsafe fn(*mut c_void), name: &'static str) -> Obj {
        Obj { vtbl, refs: AtomicU32::new(1), iids, more: None, dtor, name }
    }
}

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

/// `dtor` for a `Box<T>` whose first field is its `Obj`.
pub unsafe fn drop_box<T>(p: *mut c_void) {
    // SAFETY: made by `Box::into_raw` of a `T`.
    drop(unsafe { alloc::boxed::Box::from_raw(p as *mut T) });
}

pub fn into_raw<T>(v: T) -> *mut c_void {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(v)) as *mut c_void
}

/// The `Obj` of an object of this DLL.
pub unsafe fn obj<'a>(p: *mut c_void) -> &'a Obj {
    // SAFETY: every object of this DLL starts with an `Obj`.
    unsafe { &*(p as *const Obj) }
}

pub unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: an object of this DLL.
    unsafe { obj(this) }.refs.fetch_add(1, Ordering::Relaxed) + 1
}

pub unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: an object of this DLL.
    let o = unsafe { obj(this) };
    let n = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference went.
        unsafe { (o.dtor)(this) };
    }
    n
}

pub unsafe extern "system" fn query_interface(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output pointer.
    unsafe { *out = core::ptr::null_mut() };
    if riid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID; an object of this DLL.
    let (iid, o) = unsafe { (*riid, obj(this)) };
    if iid == IID_IUNKNOWN || o.iids.contains(&iid) {
        // SAFETY: a live object.
        unsafe {
            add_ref(this);
            *out = this;
        }
        return S_OK;
    }
    if let Some(more) = o.more {
        // SAFETY: the object's own lookup; it returns a referenced interface or null.
        let p = unsafe { more(this, &iid) };
        if !p.is_null() {
            // SAFETY: the caller's output pointer.
            unsafe { *out = p };
            return S_OK;
        }
    }
    log_iid(o.name, &iid);
    E_NOINTERFACE
}

/// `GetPrivateData`: nothing is kept.
pub unsafe extern "system" fn get_private_data(
    _: *mut c_void,
    _: *const Guid,
    size: *mut u32,
    _: *mut c_void,
) -> HRESULT {
    if !size.is_null() {
        // SAFETY: the caller's size.
        unsafe { *size = 0 };
    }
    DXGI_ERROR_NOT_FOUND
}

/// `SetPrivateData` / `SetPrivateDataInterface`: accepted, not kept (debug names).
pub unsafe extern "system" fn set_private_data(_: *mut c_void, _: *const Guid, _: u32, _: *const c_void) -> HRESULT {
    S_OK
}

pub unsafe extern "system" fn set_private_data_interface(_: *mut c_void, _: *const Guid, _: *mut c_void) -> HRESULT {
    S_OK
}

/// Write `v` through a `T*` output when it is not null.
pub unsafe fn put<T>(out: *mut T, v: T) {
    if !out.is_null() {
        // SAFETY: the caller's output.
        unsafe { *out = v };
    }
}

/// Methods not implemented: each logs its name the first time it is called and fails with
/// `E_NOTIMPL` (or does nothing, for the many `void` setters of a context).
#[macro_export]
macro_rules! stubs {
    ($m:ident, $iface:literal, [$($name:ident),* $(,)?]) => {
        #[allow(non_snake_case)]
        pub mod $m {
            use core::ffi::c_void;
            use core::sync::atomic::{AtomicBool, Ordering};
            $(
                pub unsafe extern "system" fn $name(_: *mut c_void) -> $crate::com::HRESULT {
                    static ONCE: AtomicBool = AtomicBool::new(false);
                    if !ONCE.swap(true, Ordering::Relaxed) {
                        $crate::com::log(concat!("stub: d3d11: ", $iface, "::", stringify!($name)));
                    }
                    $crate::com::E_NOTIMPL
                }
            )*
        }
    };
}
