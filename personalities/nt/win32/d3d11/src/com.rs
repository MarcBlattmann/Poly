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
pub const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
pub const E_ACCESSDENIED: HRESULT = 0x8007_0005u32 as i32;
pub const DXGI_ERROR_INVALID_CALL: HRESULT = 0x887A_0001u32 as i32;
pub const DXGI_ERROR_MORE_DATA: HRESULT = 0x887A_0003u32 as i32;
pub const DXGI_ERROR_DEVICE_REMOVED: HRESULT = 0x887A_0005u32 as i32;
pub const D3D11_ERROR_TOO_MANY_UNIQUE_STATE_OBJECTS: HRESULT = 0x887C_0001u32 as i32;
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
// d3d11_1.h .. d3d11_4.h, dxgi1_2.h .. dxgi1_6.h
pub const IID_ID3D11DEVICE1: Guid = guid(0xa04bfb29, 0x08ef, 0x43d6, 0xa49c, 0xa9bdbdcbe686);
pub const IID_ID3D11DEVICE2: Guid = guid(0x9d06dffa, 0xd1e5, 0x4d07, 0x83a8, 0x1bb123f2f841);
pub const IID_ID3D11DEVICE3: Guid = guid(0xa05c8c37, 0xd2c6, 0x4732, 0xb3a0, 0x9ce0b0dc9ae6);
pub const IID_ID3D11DEVICE4: Guid = guid(0x8992ab71, 0x02e6, 0x4b8d, 0xba48, 0xb056dcda42c4);
pub const IID_ID3D11DEVICE5: Guid = guid(0x8ffde202, 0xa0e7, 0x45df, 0x9e01, 0xe837801b5ea0);
pub const IID_ID3D11DEVICECONTEXT1: Guid = guid(0xbb2c6faa, 0xb5fb, 0x4082, 0x8e6b, 0x388b8cfa90e1);
pub const IID_ID3D11DEVICECONTEXT2: Guid = guid(0x420d5b32, 0xb90c, 0x4da4, 0xbef0, 0x359f6a24a83a);
pub const IID_ID3D11DEVICECONTEXT3: Guid = guid(0xb4e3c01d, 0xe79e, 0x4637, 0x91b2, 0x510e9f4c9b8f);
pub const IID_ID3D11DEVICECONTEXT4: Guid = guid(0x917600da, 0xf58c, 0x4c33, 0x98d8, 0x3e15b390fa24);
pub const IID_ID3DUSERDEFINEDANNOTATION: Guid = guid(0xb2daad8b, 0x03d4, 0x4dbf, 0x95eb, 0x32ab4b63d0ab);
pub const IID_IDXGIDEVICE2: Guid = guid(0x05008617, 0xfbfd, 0x4051, 0xa790, 0x144884b4f6a9);
pub const IID_IDXGIDEVICE3: Guid = guid(0x6007896c, 0x3244, 0x4afd, 0xbf18, 0xa6d3beda5023);
pub const IID_IDXGIDEVICE4: Guid = guid(0x95b4f95f, 0xd8da, 0x4ca4, 0x9ee6, 0x3b76d5968a10);
pub const IID_ID3D11TEXTURE2D1: Guid = guid(0x51218251, 0x1e33, 0x4617, 0x9ccb, 0x4d3a4367e7bb);
pub const IID_IDXGIRESOURCE1: Guid = guid(0x30961379, 0x4609, 0x4a41, 0x998e, 0x54fe567ee0c1);
pub const IID_IDXGISURFACE1: Guid = guid(0x4ae63092, 0x6327, 0x4c1b, 0x80ae, 0xbfe12ea32b86);
pub const IID_IDXGISURFACE2: Guid = guid(0xaba496dd, 0xb617, 0x4cb8, 0xa866, 0xbc44d7eb1fa2);
pub const IID_IDXGIKEYEDMUTEX: Guid = guid(0x9d8e1289, 0xd7b3, 0x465f, 0x8126, 0x250e349af85d);
pub const IID_ID3D11BUFFER: Guid = guid(0x48570b85, 0xd1ee, 0x4fcd, 0xa250, 0xeb350722b037);
pub const IID_ID3D11SHADERRESOURCEVIEW: Guid = guid(0xb0e06fe0, 0x8192, 0x4e1a, 0xb1ca, 0x36d7414710b2);
pub const IID_ID3D11SHADERRESOURCEVIEW1: Guid = guid(0x91308b87, 0x9040, 0x411d, 0x8c67, 0xc39253ce3802);
pub const IID_ID3D11RENDERTARGETVIEW1: Guid = guid(0xffbe2e23, 0xf011, 0x418a, 0xac56, 0x5ceed7c5b94b);
pub const IID_ID3D11DEPTHSTENCILVIEW: Guid = guid(0x9fdac92a, 0x1876, 0x48c3, 0xafad, 0x25b94f84a9b6);
pub const IID_ID3D11UNORDEREDACCESSVIEW: Guid = guid(0x28acf509, 0x7f5c, 0x48f6, 0x8611, 0xf316010a6380);
pub const IID_ID3D11UNORDEREDACCESSVIEW1: Guid = guid(0x7b3b6153, 0xa886, 0x4544, 0xab37, 0x6537c8500403);
pub const IID_ID3D11BLENDSTATE: Guid = guid(0x75b68faa, 0x347d, 0x4159, 0x8f45, 0xa0640f01cd9a);
pub const IID_ID3D11BLENDSTATE1: Guid = guid(0xcc86fabe, 0xda55, 0x401d, 0x85e7, 0xe3c9de2877e9);
pub const IID_ID3D11RASTERIZERSTATE: Guid = guid(0x9bb4ab81, 0xab1a, 0x4d8f, 0xb506, 0xfc04200b6ee7);
pub const IID_ID3D11RASTERIZERSTATE1: Guid = guid(0x1217d7a6, 0x5039, 0x418c, 0xb042, 0x9cbe256afd6e);
pub const IID_ID3D11RASTERIZERSTATE2: Guid = guid(0x6fbd02fb, 0x209f, 0x46c4, 0xb059, 0x2ed15586a6ac);
pub const IID_ID3D11DEPTHSTENCILSTATE: Guid = guid(0x03823efb, 0x8d8f, 0x4e1c, 0x9aa2, 0xf64bb2cbfdf1);
pub const IID_ID3D11SAMPLERSTATE: Guid = guid(0xda6fea51, 0x564c, 0x4487, 0x9810, 0xf0d0f9b4e3a5);
pub const IID_ID3D11QUERY: Guid = guid(0xd6c00747, 0x87b7, 0x425e, 0xb84d, 0x44d108560afd);
pub const IID_ID3D11QUERY1: Guid = guid(0x631b4766, 0x36dc, 0x461d, 0x8db6, 0xc47e13e60916);
pub const IID_ID3D11ASYNCHRONOUS: Guid = guid(0x4b35d0cd, 0x1e15, 0x4258, 0x9c98, 0x1b1333f6dd3b);
pub const IID_ID3D11PREDICATE: Guid = guid(0x9eb576dd, 0x9f77, 0x4d86, 0x81aa, 0x8bab5fe490e2);
pub const IID_ID3D11COUNTER: Guid = guid(0x6e8c49fb, 0xa371, 0x4770, 0xb440, 0x29086022b741);
pub const IID_ID3D11FENCE: Guid = guid(0xaffde9d1, 0x1df7, 0x4bb7, 0x8a34, 0x0f46251dab80);
pub const IID_ID3D11VIDEOCONTEXT: Guid = guid(0x61f21c45, 0x3c0e, 0x4a74, 0x9cea, 0x67100d9ad5e4);

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
    /// `SetPrivateData` / `SetPrivateDataInterface`.
    pub private: Lock<poly_d3d::privdata::Store>,
}

impl Obj {
    pub fn new(vtbl: *const c_void, iids: &'static [Guid], dtor: unsafe fn(*mut c_void), name: &'static str) -> Obj {
        let private = Lock::new(poly_d3d::privdata::Store::new());
        Obj { vtbl, refs: AtomicU32::new(1), iids, more: None, dtor, name, private }
    }
}

/// A small spin lock (the objects' private data and the device's caches; held for a few
/// instructions).
pub struct Lock<T> {
    busy: core::sync::atomic::AtomicBool,
    v: core::cell::UnsafeCell<T>,
}

// SAFETY: the value is reached only with the lock held.
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(v: T) -> Lock<T> {
        Lock { busy: core::sync::atomic::AtomicBool::new(false), v: core::cell::UnsafeCell::new(v) }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.busy.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: the lock is held.
        let r = f(unsafe { &mut *self.v.get() });
        self.busy.store(false, Ordering::Release);
        r
    }
}

/// References a view or a pipeline binding holds on an object without showing in its public count
/// (what AddRef / Release return): the count's upper bits.
pub const HIDDEN_SHIFT: u32 = 20;
const PUBLIC: u32 = (1 << HIDDEN_SHIFT) - 1;

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
    (unsafe { obj(this) }.refs.fetch_add(1, Ordering::Relaxed) + 1) & PUBLIC
}

pub unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: an object of this DLL.
    let o = unsafe { obj(this) };
    let n = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference went.
        unsafe { drop_obj(this) };
    }
    n & PUBLIC
}

/// Free an object whose last reference went, and the interfaces its private data held.
pub unsafe fn drop_obj(this: *mut c_void) {
    // SAFETY: an object of this DLL.
    let o = unsafe { obj(this) };
    let held: alloc::vec::Vec<usize> = o.private.with(|s| s.interfaces().collect());
    for p in held {
        // SAFETY: a reference `SetPrivateDataInterface` took.
        unsafe { com_release(p as *mut c_void) };
    }
    // SAFETY: the last reference went.
    unsafe { (o.dtor)(this) };
}

/// AddRef / Release of any COM object (through its vtable).
pub unsafe fn com_add_ref(p: *mut c_void) -> u32 {
    // SAFETY: a live COM object: AddRef is entry 1.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(1));
        f(p)
    }
}

pub unsafe fn com_release(p: *mut c_void) -> u32 {
    // SAFETY: a live COM object: Release is entry 2.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(2));
        f(p)
    }
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
    // An object with tear-offs may answer IUnknown with its own identity part (the device: "IUnknown
    // gives the same pointer every time but not the device pointer", D3D11CreateDevice.yaml).
    if iid == IID_IUNKNOWN
        && let Some(more) = o.more
    {
        // SAFETY: the object's own lookup; it returns a referenced interface or null.
        let p = unsafe { more(this, &iid) };
        if !p.is_null() {
            // SAFETY: the caller's output pointer.
            unsafe { *out = p };
            return S_OK;
        }
    }
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

fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..4].copy_from_slice(&g.d1.to_le_bytes());
    b[4..6].copy_from_slice(&g.d2.to_le_bytes());
    b[6..8].copy_from_slice(&g.d3.to_le_bytes());
    b[8..].copy_from_slice(&g.d4);
    b
}

/// The object a tear-off part belongs to (every part is `{ vtbl, owner }`).
pub unsafe fn part_owner(this: *mut c_void) -> *mut c_void {
    // SAFETY: a part's second word is its owner.
    unsafe { *(this as *const *mut c_void).add(1) }
}

/// `GetPrivateData(guid, pDataSize, pData)` of ID3D11DeviceChild / IDXGIObject
/// (ID3D11Device.GetCreationFlags.yaml `private_data`, IDXGIDevice.GetAdapter.yaml).
pub unsafe extern "system" fn get_private_data(
    this: *mut c_void,
    guid: *const Guid,
    size: *mut u32,
    data: *mut c_void,
) -> HRESULT {
    use poly_d3d::privdata::Got;
    if guid.is_null() || size.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID and size; an object of this DLL.
    let (g, asked, o) = unsafe { (guid_bytes(&*guid), *size, obj(this)) };
    o.private.with(|s| match s.get(&g, asked, !data.is_null()) {
        Got::Interface(p) => {
            // SAFETY: a live interface the object holds; the caller's 8 bytes.
            unsafe {
                if p != 0 {
                    com_add_ref(p as *mut c_void);
                }
                *(data as *mut usize) = p;
                *size = 8;
            }
            S_OK
        }
        Got::Data { hr, size: n, bytes } => {
            // SAFETY: the caller's size, and a buffer of at least `n` bytes when bytes are copied.
            unsafe {
                *size = n;
                if let Some(b) = bytes {
                    core::ptr::copy_nonoverlapping(b.as_ptr(), data as *mut u8, b.len());
                }
            }
            hr
        }
    })
}

/// `SetPrivateData(guid, DataSize, pData)`.
pub unsafe extern "system" fn set_private_data(
    this: *mut c_void,
    guid: *const Guid,
    size: u32,
    data: *const c_void,
) -> HRESULT {
    if guid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID and `size` bytes of data.
    let (g, bytes) = unsafe {
        let bytes = if data.is_null() { None } else { Some(core::slice::from_raw_parts(data as *const u8, size as usize)) };
        (guid_bytes(&*guid), bytes)
    };
    // SAFETY: an object of this DLL.
    let (hr, old) = unsafe { obj(this) }.private.with(|s| s.set(g, bytes, size));
    if let Some(p) = old {
        // SAFETY: the reference the replaced entry held.
        unsafe { com_release(p as *mut c_void) };
    }
    hr
}

/// `SetPrivateDataInterface(guid, pUnknown)`: the object holds a reference to it.
pub unsafe extern "system" fn set_private_data_interface(this: *mut c_void, guid: *const Guid, p: *mut c_void) -> HRESULT {
    if guid.is_null() {
        return E_INVALIDARG;
    }
    if !p.is_null() {
        // SAFETY: the caller's live interface.
        unsafe { com_add_ref(p) };
    }
    // SAFETY: the caller's GUID.
    let g = unsafe { guid_bytes(&*guid) };
    // SAFETY: an object of this DLL.
    let old = unsafe { obj(this) }.private.with(|s| s.set_interface(g, p as usize));
    if let Some(o) = old {
        // SAFETY: the reference the replaced entry held.
        unsafe { com_release(o as *mut c_void) };
    }
    S_OK
}

pub unsafe extern "system" fn part_get_private_data(
    this: *mut c_void,
    g: *const Guid,
    size: *mut u32,
    data: *mut c_void,
) -> HRESULT {
    // SAFETY: a part of a live object.
    unsafe { get_private_data(part_owner(this), g, size, data) }
}

pub unsafe extern "system" fn part_set_private_data(
    this: *mut c_void,
    g: *const Guid,
    size: u32,
    data: *const c_void,
) -> HRESULT {
    // SAFETY: a part of a live object.
    unsafe { set_private_data(part_owner(this), g, size, data) }
}

pub unsafe extern "system" fn part_set_private_data_interface(this: *mut c_void, g: *const Guid, p: *mut c_void) -> HRESULT {
    // SAFETY: a part of a live object.
    unsafe { set_private_data_interface(part_owner(this), g, p) }
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
