//! Resources in memory (ADR 0211): buffers and 2D textures (with mips, arrays and samples) of the
//! device, their DXGI interfaces (`IDXGIResource1`, `IDXGISurface2`, `IDXGIKeyedMutex`), and the
//! views of them (render-target, shader-resource, depth-stencil, unordered-access). Which
//! descriptions are valid and how they are completed is `poly_d3d`'s; this module holds the memory.
//!
//! The bytes of a resource are its subresources one after the other (subresource `mip + slice *
//! mips`, rows without padding: `poly_d3d::format::pitch_rows`). They are allocated on first use,
//! so a huge buffer that is only described costs nothing. Resources opened from a shared handle
//! share the storage of the original.

use crate::com::*;
use crate::stubs;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use poly_d3d::format;
use poly_d3d::resource::{BufferDesc, DIMENSION_BUFFER, DIMENSION_TEXTURE2D, PRIORITY_NORMAL, Tex2dDesc, bind, misc};
use poly_d3d::view;

/// `D3D11_BIND_RENDER_TARGET` (d3d11.h).
pub const BIND_RENDER_TARGET: u32 = bind::RENDER_TARGET;

/// The bytes of a resource, shared by the objects opened from one shared handle.
pub struct Store {
    bytes: UnsafeCell<Vec<u8>>,
    /// Memory the bytes are in instead of `bytes` (a swap chain's buffer: a view of a section the
    /// compositor maps, ADR 0224), or null. It outlives the store's use by a texture: the swap
    /// chain gives the texture a store of its own before it unmaps the view.
    outside: *mut u8,
    len: usize,
    /// What the resource is (an object opened from a shared handle is the same kind).
    pub kind: Kind,
    /// The keyed mutex: its key and the object that holds it (0: free).
    pub key: AtomicU64,
    pub holder: core::sync::atomic::AtomicUsize,
}

// SAFETY: the bytes are reached only through the resource's methods; the device's calls are made
// in the order the program makes them, as on a device without the multithread lock.
unsafe impl Sync for Store {}
// SAFETY: as above.
unsafe impl Send for Store {}

impl Store {
    pub fn new(len: usize, kind: Kind) -> Arc<Store> {
        Arc::new(Store {
            bytes: UnsafeCell::new(Vec::new()),
            outside: core::ptr::null_mut(),
            len,
            kind,
            key: AtomicU64::new(0),
            holder: core::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// A store whose `len` bytes are at `memory` (mapped by the caller, who keeps it mapped while
    /// the store is in use).
    pub fn outside(memory: *mut u8, len: usize, kind: Kind) -> Arc<Store> {
        Arc::new(Store {
            bytes: UnsafeCell::new(Vec::new()),
            outside: memory,
            len,
            kind,
            key: AtomicU64::new(0),
            holder: core::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// The bytes, allocated (zeroed) on first use.
    #[allow(clippy::mut_from_ref)]
    pub fn bytes(&self) -> &mut [u8] {
        if !self.outside.is_null() {
            // SAFETY: `outside` is `len` bytes the swap chain keeps mapped while this store is a
            // texture's (`Store::outside`); see `Sync` for the access rule.
            return unsafe { core::slice::from_raw_parts_mut(self.outside, self.len) };
        }
        // SAFETY: see `Sync`: no two calls of the device touch one resource at once.
        let v = unsafe { &mut *self.bytes.get() };
        if v.len() != self.len {
            v.resize(self.len, 0);
        }
        v
    }
}

/// What a resource is.
#[derive(Clone, Copy)]
pub enum Kind {
    Buffer(BufferDesc),
    Tex2d(Tex2dDesc),
}

/// A buffer or a 2D texture.
#[repr(C)]
pub struct Tex {
    obj: Obj,
    resource: Part,
    surface: Part,
    mutex: Part,
    pub kind: Kind,
    /// The row pitch of subresource 0 (textures; the byte width of a buffer).
    pub pitch: usize,
    pub store: Arc<Store>,
    pub device: *mut c_void,
    priority: AtomicU32,
    /// The legacy shared handle value (`GetSharedHandle`), 0 when the resource is not shared so.
    shared: usize,
}

/// A tear-off interface of an object: its own vtable, the object's lifetime.
#[repr(C)]
pub struct Part {
    pub vtbl: *const c_void,
    pub owner: *mut c_void,
}

impl Tex {
    pub fn desc(&self) -> Option<&Tex2dDesc> {
        match &self.kind {
            Kind::Tex2d(d) => Some(d),
            Kind::Buffer(_) => None,
        }
    }
    pub fn width(&self) -> usize {
        self.desc().map_or(self.pitch, |d| d.width as usize)
    }
    pub fn height(&self) -> usize {
        self.desc().map_or(1, |d| d.height as usize)
    }
    pub fn format(&self) -> u32 {
        self.desc().map_or(0, |d| d.format)
    }
    pub fn is_buffer(&self) -> bool {
        matches!(self.kind, Kind::Buffer(_))
    }
    /// The bytes of the whole resource.
    pub fn data(&self) -> &mut [u8] {
        self.store.bytes()
    }
    pub fn subresources(&self) -> u32 {
        self.desc().map_or(1, |d| d.subresources())
    }
    /// Subresource `sub`: its offset, row pitch, rows, width and height in texels.
    pub fn sub(&self, sub: u32) -> Option<Sub> {
        match &self.kind {
            Kind::Buffer(b) => (sub == 0).then_some(Sub {
                offset: 0,
                pitch: b.byte_width as usize,
                rows: 1,
                width: b.byte_width,
                height: 1,
            }),
            Kind::Tex2d(d) => {
                if sub >= d.subresources() {
                    return None;
                }
                let mut offset = 0;
                for i in 0..=sub {
                    let (w, h) = d.mip_size(i % d.mips);
                    let (pitch, rows) = format::pitch_rows(d.format, w, h);
                    if i == sub {
                        return Some(Sub { offset, pitch: pitch as usize, rows: rows as usize, width: w, height: h });
                    }
                    offset += pitch as usize * rows as usize;
                }
                None
            }
        }
    }
    /// The bytes of one subresource.
    pub fn sub_bytes(&self, sub: u32) -> Option<(&mut [u8], Sub)> {
        let s = self.sub(sub)?;
        let len = s.pitch * s.rows;
        Some((&mut self.data()[s.offset..s.offset + len], s))
    }
}

/// Where a subresource is.
#[derive(Clone, Copy, Debug)]
pub struct Sub {
    pub offset: usize,
    pub pitch: usize,
    pub rows: usize,
    pub width: u32,
    pub height: u32,
}

/// The total bytes of a texture's subresources.
fn tex_len(d: &Tex2dDesc) -> usize {
    let mut n = 0usize;
    for m in 0..d.mips {
        let (w, h) = d.mip_size(m);
        let (p, r) = format::pitch_rows(d.format, w, h);
        n += p as usize * r as usize;
    }
    n * d.array as usize
}

/// Bytes per pixel of a format (4 for the formats without a plain layout).
pub fn bytes_per_pixel(f: u32) -> usize {
    match format::texel_bytes(f) {
        0 => 4,
        n => n as usize,
    }
}

pub fn red_first(f: u32) -> bool {
    format::red_first(f)
}

pub unsafe extern "system" fn part_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: a `Part` of a live object.
    unsafe { add_ref((*(this as *const Part)).owner) }
}

pub unsafe extern "system" fn part_release(this: *mut c_void) -> u32 {
    // SAFETY: a `Part` of a live object.
    unsafe { release((*(this as *const Part)).owner) }
}

pub unsafe extern "system" fn part_query_interface(
    this: *mut c_void,
    riid: *const Guid,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: a `Part` of a live object.
    unsafe { query_interface((*(this as *const Part)).owner, riid, out) }
}

static TEX_IIDS: [Guid; 4] = [IID_ID3D11TEXTURE2D, IID_ID3D11TEXTURE2D1, IID_ID3D11RESOURCE, IID_ID3D11DEVICECHILD];
static BUF_IIDS: [Guid; 3] = [IID_ID3D11BUFFER, IID_ID3D11RESOURCE, IID_ID3D11DEVICECHILD];

pub static TEX_VTBL: Vt<12> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_type as *const c_void,
    set_eviction_priority as *const c_void,
    get_eviction_priority as *const c_void,
    get_desc as *const c_void,
    get_desc as *const c_void,
]);

pub static BUF_VTBL: Vt<11> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_type as *const c_void,
    set_eviction_priority as *const c_void,
    get_eviction_priority as *const c_void,
    get_desc as *const c_void,
]);

/// The resource behind a pointer, if it is one of ours (a buffer or a texture, or one of their
/// DXGI interfaces).
pub unsafe fn res<'a>(p: *mut c_void) -> Option<&'a mut Tex> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a COM object's first word is its vtable.
    let vt = unsafe { *(p as *const *const c_void) };
    if vt == TEX_VTBL.ptr() || vt == BUF_VTBL.ptr() {
        // SAFETY: an object with one of our resource vtables is a `Tex`.
        return Some(unsafe { &mut *(p as *mut Tex) });
    }
    if vt == RESOURCE_VTBL.ptr() || vt == SURFACE_VTBL.ptr() || vt == MUTEX_VTBL.ptr() {
        // SAFETY: a `Part` of a resource.
        return Some(unsafe { &mut *((*(p as *const Part)).owner as *mut Tex) });
    }
    None
}

/// The 2D texture behind a pointer (not a buffer).
pub unsafe fn tex<'a>(p: *mut c_void) -> Option<&'a mut Tex> {
    // SAFETY: as `res`.
    unsafe { res(p) }.filter(|t| !t.is_buffer())
}

/// A new resource of `device` (it keeps a reference) of `kind`, with `store` (new storage when None).
pub unsafe fn make(device: *mut c_void, kind: Kind, store: Option<Arc<Store>>) -> *mut c_void {
    let (vtbl, iids, name, pitch, len): (_, &'static [Guid], _, _, _) = match &kind {
        Kind::Buffer(b) => (BUF_VTBL.ptr(), &BUF_IIDS, "ID3D11Buffer", b.byte_width as usize, b.byte_width as usize),
        Kind::Tex2d(d) => {
            let (p, _) = format::pitch_rows(d.format, d.width, d.height);
            (TEX_VTBL.ptr(), &TEX_IIDS, "ID3D11Texture2D", p as usize, tex_len(d))
        }
    };
    let shared_misc = match &kind {
        Kind::Buffer(b) => b.misc,
        Kind::Tex2d(d) => d.misc,
    };
    // SAFETY: a live device of this DLL.
    unsafe { add_ref(device) };
    let mut obj = Obj::new(vtbl, iids, drop_tex, name);
    obj.more = Some(tex_more);
    let store = store.unwrap_or_else(|| Store::new(len, kind));
    let shared =
        if shared_misc & (misc::SHARED | misc::SHARED_KEYEDMUTEX) != 0 && shared_misc & misc::SHARED_NTHANDLE == 0 {
            Arc::as_ptr(&store) as usize
        } else {
            0
        };
    let p = into_raw(Tex {
        obj,
        resource: Part { vtbl: RESOURCE_VTBL.ptr(), owner: core::ptr::null_mut() },
        surface: Part { vtbl: SURFACE_VTBL.ptr(), owner: core::ptr::null_mut() },
        mutex: Part { vtbl: MUTEX_VTBL.ptr(), owner: core::ptr::null_mut() },
        kind,
        pitch,
        store,
        device,
        priority: AtomicU32::new(PRIORITY_NORMAL),
        shared,
    });
    // SAFETY: the new resource.
    let t = unsafe { &mut *(p as *mut Tex) };
    t.resource.owner = p;
    t.surface.owner = p;
    t.mutex.owner = p;
    if shared != 0 {
        shares_add(shared, &t.store);
    }
    p
}

/// A new 2D texture from a `D3D11_TEXTURE2D_DESC` as 11 words that is already checked, with the
/// pixels of subresource 0 from `init` (bytes and row pitch) or zero (for the swap chains).
pub unsafe fn create(device: *mut c_void, desc: [u32; 11], init: Option<(*const u8, usize)>) -> *mut c_void {
    let d = Tex2dDesc {
        width: desc[0],
        height: desc[1],
        mips: desc[2].max(1),
        array: desc[3].max(1),
        format: desc[4],
        samples: desc[5].max(1),
        quality: desc[6],
        usage: desc[7],
        bind: desc[8],
        cpu: desc[9],
        misc: desc[10],
    };
    // SAFETY: as `make`.
    let p = unsafe { make(device, Kind::Tex2d(d), None) };
    if let Some((src, pitch)) = init {
        // SAFETY: the new texture; the caller's data.
        unsafe { fill_sub(tex(p).expect("a texture"), 0, src, pitch, 0) };
    }
    p
}

/// Copy the caller's rows (`pitch` apart; slices `slice_pitch` apart) into subresource `sub`.
pub unsafe fn fill_sub(t: &Tex, sub: u32, src: *const u8, pitch: usize, _slice_pitch: usize) {
    let Some((bytes, s)) = t.sub_bytes(sub) else { return };
    if src.is_null() {
        return;
    }
    let row = s.pitch;
    for y in 0..s.rows {
        // SAFETY: the caller's data holds `rows` rows of at least `row` bytes, `pitch` apart.
        let from = unsafe { core::slice::from_raw_parts(src.add(y * pitch), row) };
        bytes[y * row..(y + 1) * row].copy_from_slice(from);
    }
}

unsafe fn drop_tex(p: *mut c_void) {
    // SAFETY: a resource whose last reference went.
    let (device, shared) = unsafe { ((*(p as *mut Tex)).device, (*(p as *mut Tex)).shared) };
    let _ = shared;
    // SAFETY: made by `make`.
    unsafe { drop_box::<Tex>(p) };
    // SAFETY: the reference `make` took.
    unsafe { release(device) };
}

/// The resource's DXGI interfaces.
unsafe fn tex_more(p: *mut c_void, iid: &Guid) -> *mut c_void {
    // SAFETY: a live resource.
    let t = unsafe { &mut *(p as *mut Tex) };
    let m = match &t.kind {
        Kind::Tex2d(d) => d.misc,
        Kind::Buffer(b) => b.misc,
    };
    let part = if [IID_IDXGIRESOURCE, IID_IDXGIRESOURCE1, IID_IDXGIDEVICESUBOBJECT, IID_IDXGIOBJECT].contains(iid) {
        &mut t.resource as *mut Part
    } else if !t.is_buffer() && [IID_IDXGISURFACE, IID_IDXGISURFACE1, IID_IDXGISURFACE2].contains(iid) {
        &mut t.surface as *mut Part
    } else if *iid == IID_IDXGIKEYEDMUTEX && m & misc::SHARED_KEYEDMUTEX != 0 {
        &mut t.mutex as *mut Part
    } else {
        return core::ptr::null_mut();
    };
    // SAFETY: the resource is live.
    unsafe { add_ref(p) };
    part as *mut c_void
}

unsafe extern "system" fn get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live resource; the caller's output.
    unsafe {
        let d = (*(this as *mut Tex)).device;
        add_ref(d);
        put(out, d);
    }
}

unsafe extern "system" fn get_type(this: *mut c_void, out: *mut u32) {
    // SAFETY: a live resource; the caller's output.
    let kind = if unsafe { (*(this as *mut Tex)).is_buffer() } { DIMENSION_BUFFER } else { DIMENSION_TEXTURE2D };
    // SAFETY: the caller's output.
    unsafe { put(out, kind) };
}

unsafe extern "system" fn set_eviction_priority(this: *mut c_void, p: u32) {
    // SAFETY: a live resource.
    unsafe { (*(this as *mut Tex)).priority.store(p, Ordering::Relaxed) };
}

unsafe extern "system" fn get_eviction_priority(this: *mut c_void) -> u32 {
    // SAFETY: a live resource.
    unsafe { (*(this as *mut Tex)).priority.load(Ordering::Relaxed) }
}

/// `GetDesc` (and `GetDesc1` of a texture: the layout word after it is 0, UNDEFINED).
unsafe extern "system" fn get_desc(this: *mut c_void, out: *mut u32) {
    if out.is_null() {
        return;
    }
    // SAFETY: a live resource; the caller's description.
    unsafe {
        match &(*(this as *mut Tex)).kind {
            Kind::Buffer(b) => (out as *mut BufferDesc).write(*b),
            Kind::Tex2d(d) => (out as *mut Tex2dDesc).write(*d),
        }
    }
}

/// `ID3D11Texture2D1::GetDesc1`: the description and `TextureLayout` 0.
pub unsafe extern "system" fn get_desc1(this: *mut c_void, out: *mut u32) {
    // SAFETY: as `get_desc`; the caller's 12 words.
    unsafe {
        get_desc(this, out);
        if !out.is_null() {
            *out.add(11) = 0;
        }
    }
}

// ----- the legacy shared handles: a handle value names a store

struct Shares(UnsafeCell<Vec<(usize, alloc::sync::Weak<Store>)>>);
// SAFETY: guarded by `SHARES_LOCK`.
unsafe impl Sync for Shares {}
static SHARES: Shares = Shares(UnsafeCell::new(Vec::new()));
static SHARES_LOCK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn with_shares<R>(f: impl FnOnce(&mut Vec<(usize, alloc::sync::Weak<Store>)>) -> R) -> R {
    while SHARES_LOCK.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let r = f(unsafe { &mut *SHARES.0.get() });
    SHARES_LOCK.store(false, Ordering::Release);
    r
}

fn shares_add(handle: usize, store: &Arc<Store>) {
    with_shares(|v| {
        v.retain(|(_, w)| w.strong_count() > 0);
        v.push((handle, Arc::downgrade(store)));
    });
}

/// What the resources of a store are.
pub fn kind_of_store(s: &Store) -> Option<Kind> {
    Some(s.kind)
}

/// The store a legacy shared handle names.
pub fn shares_find(handle: usize) -> Option<Arc<Store>> {
    with_shares(|v| v.iter().find(|(h, _)| *h == handle).and_then(|(_, w)| w.upgrade()))
}

// ----- IDXGIResource1, IDXGISurface2, IDXGIKeyedMutex

stubs!(res_stubs, "IDXGIResource1", [CreateSubresourceSurface]);
stubs!(surf_stubs, "IDXGISurface2", [GetDC, ReleaseDC]);

static RESOURCE_VTBL: Vt<14> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    part_set_private_data as *const c_void,
    part_set_private_data_interface as *const c_void,
    part_get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    dxgi_get_device as *const c_void,
    get_shared_handle as *const c_void,
    get_usage as *const c_void,
    dxgi_set_eviction as *const c_void,
    dxgi_get_eviction as *const c_void,
    res_stubs::CreateSubresourceSurface as *const c_void,
    create_shared_handle as *const c_void,
]);

static SURFACE_VTBL: Vt<14> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    part_set_private_data as *const c_void,
    part_set_private_data_interface as *const c_void,
    part_get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    dxgi_get_device as *const c_void,
    surface_desc as *const c_void,
    surface_map as *const c_void,
    surface_unmap as *const c_void,
    surf_stubs::GetDC as *const c_void,
    surf_stubs::ReleaseDC as *const c_void,
    surface_get_resource as *const c_void,
]);

static MUTEX_VTBL: Vt<10> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    part_set_private_data as *const c_void,
    part_set_private_data_interface as *const c_void,
    part_get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    dxgi_get_device as *const c_void,
    acquire_sync as *const c_void,
    release_sync as *const c_void,
]);

unsafe fn part_tex<'a>(this: *mut c_void) -> &'a mut Tex {
    // SAFETY: a `Part` of a live resource.
    unsafe { &mut *((*(this as *const Part)).owner as *mut Tex) }
}

/// `GetParent` / `GetDevice` of a DXGI view of a resource: the device.
unsafe extern "system" fn dxgi_get_parent(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a live resource's device.
    unsafe { query_interface(part_tex(this).device, riid, out) }
}

unsafe extern "system" fn dxgi_get_device(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as above.
    unsafe { dxgi_get_parent(this, riid, out) }
}

/// `GetSharedHandle`: the legacy handle of a SHARED / SHARED_KEYEDMUTEX resource, NULL for an
/// unshared one, E_INVALIDARG for an NT-handle resource (IDXGIResource1.CreateSharedHandle.yaml).
unsafe extern "system" fn get_shared_handle(this: *mut c_void, out: *mut usize) -> HRESULT {
    // SAFETY: a live resource.
    let t = unsafe { part_tex(this) };
    let m = match &t.kind {
        Kind::Tex2d(d) => d.misc,
        Kind::Buffer(b) => b.misc,
    };
    if out.is_null() {
        return E_INVALIDARG;
    }
    if m & misc::SHARED_NTHANDLE != 0 {
        // SAFETY: the caller's output.
        unsafe { *out = 0 };
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *out = t.shared };
    S_OK
}

/// `GetUsage`: `DXGI_USAGE_SHADER_INPUT` (0x10) for a shader resource, `RENDER_TARGET_OUTPUT`
/// (0x20) for a render target (OpenSharedResource1.yaml `usage`: 0x30 for both).
unsafe extern "system" fn get_usage(this: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: a live resource.
    let t = unsafe { part_tex(this) };
    let b = match &t.kind {
        Kind::Tex2d(d) => d.bind,
        Kind::Buffer(b) => b.bind,
    };
    let mut usage = 0;
    if b & bind::SHADER_RESOURCE != 0 {
        usage |= 0x10;
    }
    if b & bind::RENDER_TARGET != 0 {
        usage |= 0x20;
    }
    if b & bind::UNORDERED_ACCESS != 0 {
        usage |= 0x400;
    }
    // SAFETY: the caller's output.
    unsafe { put(out, usage) };
    S_OK
}

unsafe extern "system" fn dxgi_set_eviction(this: *mut c_void, p: u32) -> HRESULT {
    // SAFETY: a live resource.
    unsafe { part_tex(this).priority.store(p, Ordering::Relaxed) };
    S_OK
}

unsafe extern "system" fn dxgi_get_eviction(this: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: a live resource; the caller's output.
    unsafe { put(out, part_tex(this).priority.load(Ordering::Relaxed)) };
    S_OK
}

/// `DXGI_SURFACE_DESC`: width, height, format, sample count and quality.
unsafe extern "system" fn surface_desc(this: *mut c_void, out: *mut [u32; 5]) -> HRESULT {
    // SAFETY: a live texture.
    let t = unsafe { part_tex(this) };
    let Some(d) = t.desc() else { return E_INVALIDARG };
    // SAFETY: the caller's output.
    unsafe { put(out, [d.width, d.height, d.format, d.samples, d.quality]) };
    S_OK
}

/// `IDXGISurface::Map`: `DXGI_MAPPED_RECT` (the pitch, then the bits).
unsafe extern "system" fn surface_map(this: *mut c_void, out: *mut u8, _flags: u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live texture; the caller's DXGI_MAPPED_RECT (16 bytes).
    unsafe {
        let t = part_tex(this);
        (out as *mut i32).write(t.pitch as i32);
        (out.add(8) as *mut *mut u8).write(t.data().as_mut_ptr());
    }
    S_OK
}

unsafe extern "system" fn surface_unmap(_: *mut c_void) -> HRESULT {
    S_OK
}

/// `IDXGISurface2::GetResource(riid, ppParentResource, pSubresourceIndex)`.
unsafe extern "system" fn surface_get_resource(
    this: *mut c_void,
    riid: *const Guid,
    out: *mut *mut c_void,
    sub: *mut u32,
) -> HRESULT {
    // SAFETY: the caller's output; a `Part` of a live texture.
    unsafe {
        put(sub, 0);
        query_interface((*(this as *const Part)).owner, riid, out)
    }
}

/// `WAIT_TIMEOUT` (winerror.h): the success code of a wait that timed out.
const WAIT_TIMEOUT: HRESULT = 0x102;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn Sleep(ms: u32);
    fn GetTickCount() -> u32;
}

/// `AcquireSync(Key, dwMilliseconds)` (IDXGIKeyedMutex.AcquireSync.yaml): wait until the mutex is
/// free and its key is `key`; the holder is this mutex object.
unsafe extern "system" fn acquire_sync(this: *mut c_void, key: u64, ms: u32) -> HRESULT {
    // SAFETY: a live resource.
    let t = unsafe { part_tex(this) };
    let me = this as usize;
    if t.store.holder.load(Ordering::Acquire) == me {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: a system call.
    let start = unsafe { GetTickCount() };
    loop {
        if t.store.key.load(Ordering::Acquire) == key
            && t.store.holder.compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire).is_ok()
        {
            return S_OK;
        }
        // SAFETY: a system call.
        let waited = unsafe { GetTickCount() }.wrapping_sub(start);
        if ms != u32::MAX && waited >= ms {
            return WAIT_TIMEOUT;
        }
        // SAFETY: a system call.
        unsafe { Sleep(1) };
    }
}

/// `ReleaseSync(Key)`: only the holding mutex object may release; the key becomes `key`.
unsafe extern "system" fn release_sync(this: *mut c_void, key: u64) -> HRESULT {
    // SAFETY: a live resource.
    let t = unsafe { part_tex(this) };
    if t.store.holder.load(Ordering::Acquire) != this as usize {
        return DXGI_ERROR_INVALID_CALL;
    }
    t.store.key.store(key, Ordering::Release);
    t.store.holder.store(0, Ordering::Release);
    S_OK
}

/// Whether the caller may touch a keyed-mutex resource's contents: it is not keyed, or some mutex
/// object of it holds it (writes and reads without the lock are dropped, ReleaseSync.yaml
/// `unacquired_use`).
pub fn unlocked(t: &Tex) -> bool {
    let m = match &t.kind {
        Kind::Tex2d(d) => d.misc,
        Kind::Buffer(b) => b.misc,
    };
    m & misc::SHARED_KEYEDMUTEX == 0 || t.store.holder.load(Ordering::Acquire) != 0
}

// ----- views

/// Which kind of view.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Rtv,
    Srv,
    Dsv,
    Uav,
}

#[repr(C)]
pub struct View {
    obj: Obj,
    pub res: *mut c_void,
    pub kind: ViewKind,
    pub desc: [u32; 6],
    device: *mut c_void,
}

static RTV_IIDS: [Guid; 4] =
    [IID_ID3D11RENDERTARGETVIEW, IID_ID3D11RENDERTARGETVIEW1, IID_ID3D11VIEW, IID_ID3D11DEVICECHILD];
static SRV_IIDS: [Guid; 4] =
    [IID_ID3D11SHADERRESOURCEVIEW, IID_ID3D11SHADERRESOURCEVIEW1, IID_ID3D11VIEW, IID_ID3D11DEVICECHILD];
static DSV_IIDS: [Guid; 3] = [IID_ID3D11DEPTHSTENCILVIEW, IID_ID3D11VIEW, IID_ID3D11DEVICECHILD];
static UAV_IIDS: [Guid; 4] =
    [IID_ID3D11UNORDEREDACCESSVIEW, IID_ID3D11UNORDEREDACCESSVIEW1, IID_ID3D11VIEW, IID_ID3D11DEVICECHILD];

pub static VIEW_VTBL: Vt<10> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    view_get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    view_get_resource as *const c_void,
    view_get_desc as *const c_void,
    view_get_desc1 as *const c_void,
]);

/// A view of `res` with the completed description `desc`. The view holds the resource (a
/// reference not counted against the creator's, CreateRenderTargetView.yaml `references`: the
/// public count of the resource is the caller's own business, so the view takes one and gives it
/// back when it goes).
pub unsafe fn new_view(device: *mut c_void, res: *mut c_void, kind: ViewKind, desc: [u32; 6]) -> *mut c_void {
    let (iids, name): (&'static [Guid], _) = match kind {
        ViewKind::Rtv => (&RTV_IIDS, "ID3D11RenderTargetView"),
        ViewKind::Srv => (&SRV_IIDS, "ID3D11ShaderResourceView"),
        ViewKind::Dsv => (&DSV_IIDS, "ID3D11DepthStencilView"),
        ViewKind::Uav => (&UAV_IIDS, "ID3D11UnorderedAccessView"),
    };
    // SAFETY: a live resource and device of this DLL.
    unsafe {
        hold(res);
        add_ref(device);
    }
    into_raw(View { obj: Obj::new(VIEW_VTBL.ptr(), iids, drop_view, name), res, kind, desc, device })
}

/// A hidden reference: the object stays alive, its public count (what AddRef / Release return)
/// does not show it. Kept in the count's upper half.
const HIDDEN: u32 = 1 << 20;

pub unsafe fn hold(p: *mut c_void) {
    // SAFETY: an object of this DLL.
    unsafe { obj(p) }.refs.fetch_add(HIDDEN, Ordering::Relaxed);
}

pub unsafe fn unhold(p: *mut c_void) {
    // SAFETY: an object of this DLL; `release` frees it when nothing is left.
    let o = unsafe { obj(p) };
    let n = o.refs.fetch_sub(HIDDEN, Ordering::AcqRel) - HIDDEN;
    if n == 0 {
        // SAFETY: the last (hidden) reference went.
        unsafe { (o.dtor)(p) };
    }
}

unsafe fn drop_view(p: *mut c_void) {
    // SAFETY: a view whose last reference went.
    let (res, device) = unsafe { ((*(p as *mut View)).res, (*(p as *mut View)).device) };
    // SAFETY: made by `new_view`; the references it took.
    unsafe {
        drop_box::<View>(p);
        unhold(res);
        release(device);
    }
}

/// One of our views of `kind`.
pub unsafe fn view<'a>(p: *mut c_void, kind: ViewKind) -> Option<&'a mut View> {
    // SAFETY: a COM object's first word is its vtable.
    if p.is_null() || unsafe { *(p as *const *const c_void) } != VIEW_VTBL.ptr() {
        return None;
    }
    // SAFETY: an object with the view vtable is a `View`.
    let v = unsafe { &mut *(p as *mut View) };
    (v.kind == kind).then_some(v)
}

/// The texture of one of our render target views.
pub unsafe fn rtv_tex<'a>(p: *mut c_void) -> Option<&'a mut Tex> {
    // SAFETY: as `view`.
    let v = unsafe { view(p, ViewKind::Rtv) }?;
    // SAFETY: the view's live resource.
    unsafe { res(v.res) }
}

unsafe extern "system" fn view_get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live view; the caller's output.
    unsafe {
        let d = (*(this as *mut View)).device;
        add_ref(d);
        put(out, d);
    }
}

unsafe extern "system" fn view_get_resource(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live view; the caller's output.
    unsafe {
        let r = (*(this as *mut View)).res;
        add_ref(r);
        put(out, r);
    }
}

unsafe extern "system" fn view_get_desc(this: *mut c_void, out: *mut u32) {
    if out.is_null() {
        return;
    }
    // SAFETY: a live view; the caller's description of the view's kind.
    unsafe {
        let v = &*(this as *mut View);
        let n = match v.kind {
            ViewKind::Rtv => 5,
            ViewKind::Dsv | ViewKind::Srv => 6,
            ViewKind::Uav => 5,
        };
        core::ptr::copy_nonoverlapping(v.desc.as_ptr(), out, n);
    }
}

/// `GetDesc1`: the description with the plane slice (0) after it.
unsafe extern "system" fn view_get_desc1(this: *mut c_void, out: *mut u32) {
    // SAFETY: as `view_get_desc`.
    unsafe { view_get_desc(this, out) };
}

/// The resource description a view rule of `poly_d3d::view` takes.
pub fn view_res(t: &Tex) -> view::Res {
    match &t.kind {
        Kind::Buffer(b) => view::Res::Buffer(*b),
        Kind::Tex2d(d) => view::Res::Tex2d(*d),
    }
}

/// Copy the rectangle `(sx, sy, w, h)` of subresource `ss` of `src` to `(dx, dy)` of subresource
/// `ds` of `dst`; nothing when the texel sizes differ (CopyResource.yaml `format_mismatch`).
#[allow(clippy::too_many_arguments)]
pub fn copy_region(
    dst: &Tex,
    ds: u32,
    dx: usize,
    dy: usize,
    src: &Tex,
    ss: u32,
    sx: usize,
    sy: usize,
    w: usize,
    h: usize,
) {
    let (Some(d), Some(s)) = (dst.sub(ds), src.sub(ss)) else { return };
    if dst.is_buffer() || src.is_buffer() {
        // Bytes: x is a byte offset.
        let n = w.min(s.pitch.saturating_sub(sx)).min(d.pitch.saturating_sub(dx));
        let from: Vec<u8> = src.data()[s.offset + sx..s.offset + sx + n].to_vec();
        dst.data()[d.offset + dx..d.offset + dx + n].copy_from_slice(&from);
        return;
    }
    if dst.format() != src.format() && !same_family(dst.format(), src.format()) {
        return;
    }
    let bpp = bytes_per_pixel(dst.format());
    let w = w.min((s.width as usize).saturating_sub(sx)).min((d.width as usize).saturating_sub(dx));
    let h = h.min((s.height as usize).saturating_sub(sy)).min((d.height as usize).saturating_sub(dy));
    let mut rows = Vec::with_capacity(w * bpp * h);
    let sb = src.data();
    for y in 0..h {
        let at = s.offset + (sy + y) * s.pitch + sx * bpp;
        rows.extend_from_slice(&sb[at..at + w * bpp]);
    }
    let db = dst.data();
    for y in 0..h {
        let at = d.offset + (dy + y) * d.pitch + dx * bpp;
        db[at..at + w * bpp].copy_from_slice(&rows[y * w * bpp..(y + 1) * w * bpp]);
    }
}

/// Whether two formats share a bit layout (the copy rule: same family).
pub fn same_family(a: u32, b: u32) -> bool {
    a == b || format::family(a).is_some_and(|f| f.contains(&b))
}

// ----- NT shared handles (IDXGIResource1.CreateSharedHandle.yaml, ID3D11Device1.OpenSharedResource1.yaml)

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateEventW(attrs: *const c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn GetLastError() -> u32;
    fn CloseHandle(h: usize) -> i32;
    fn GetHandleInformation(h: usize, flags: *mut u32) -> i32;
}

/// `DXGI_ERROR_NAME_ALREADY_EXISTS` (winerror.h).
const DXGI_ERROR_NAME_ALREADY_EXISTS: HRESULT = 0x887A_002Cu32 as i32;
/// `ERROR_ALREADY_EXISTS` (winerror.h).
const ERROR_ALREADY_EXISTS: u32 = 183;

/// The NT handles made by `CreateSharedHandle` in this process: the handle value, its name, the
/// storage. The kernel object behind a handle is an event (a closable handle with an optional name
/// in the object namespace); it names the resource only through this table.
static NT_SHARES: Lock<Vec<(usize, Vec<u16>, Arc<Store>)>> = Lock::new(Vec::new());

fn handle_open(h: usize) -> bool {
    let mut f = 0;
    // SAFETY: our output; any handle value is safe to ask about.
    unsafe { GetHandleInformation(h, &mut f) != 0 }
}

/// The storage an NT handle (still open) names.
pub fn nt_share(h: usize) -> Option<Arc<Store>> {
    if h == 0 || !handle_open(h) {
        return None;
    }
    NT_SHARES.with(|v| v.iter().find(|(x, _, _)| *x == h).map(|(_, _, s)| s.clone()))
}

/// The storage a name names, when one of its handles is still open.
pub fn nt_share_by_name(name: &[u16]) -> Option<Arc<Store>> {
    NT_SHARES.with(|v| v.iter().find(|(h, n, _)| n.as_slice() == name && handle_open(*h)).map(|(_, _, s)| s.clone()))
}

unsafe fn wide(p: *const u16) -> Vec<u16> {
    let mut v = Vec::new();
    if p.is_null() {
        return v;
    }
    let mut i = 0;
    // SAFETY: the caller's NUL-terminated string.
    while unsafe { *p.add(i) } != 0 {
        // SAFETY: as above.
        v.push(unsafe { *p.add(i) });
        i += 1;
    }
    v
}

/// `CreateSharedHandle(pAttributes, dwAccess, lpName, pHandle)`: only a resource made with
/// SHARED_NTHANDLE has one; a name already in use is DXGI_ERROR_NAME_ALREADY_EXISTS.
pub unsafe extern "system" fn create_shared_handle(
    this: *mut c_void,
    attrs: *const c_void,
    access: u32,
    name: *const u16,
    out: *mut usize,
) -> HRESULT {
    // SAFETY: a live resource.
    let t = unsafe { part_tex(this) };
    let m = match &t.kind {
        Kind::Tex2d(d) => d.misc,
        Kind::Buffer(b) => b.misc,
    };
    // SAFETY: the caller's output.
    unsafe { put(out, 0) };
    if out.is_null() || m & misc::SHARED_NTHANDLE == 0 {
        return E_INVALIDARG;
    }
    if access == u32::MAX {
        return E_FAIL;
    }
    // SAFETY: the caller's optional name; a new event.
    let n = unsafe { wide(name) };
    let mut z = n.clone();
    z.push(0);
    let h = unsafe {
        CreateEventW(attrs, 1, 0, if name.is_null() || n.is_empty() { core::ptr::null() } else { z.as_ptr() })
    };
    if h == 0 {
        return E_FAIL;
    }
    // SAFETY: a system call right after the creation.
    if !n.is_empty() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: the handle just made.
        unsafe { CloseHandle(h) };
        return DXGI_ERROR_NAME_ALREADY_EXISTS;
    }
    let store = t.store.clone();
    NT_SHARES.with(|v| {
        v.retain(|(x, _, _)| handle_open(*x));
        v.push((h, n, store));
    });
    // SAFETY: the caller's output.
    unsafe { *out = h };
    S_OK
}

/// The name of a shared-handle open (`OpenSharedResourceByName`).
pub unsafe fn name_of(p: *const u16) -> Vec<u16> {
    // SAFETY: the caller's string.
    unsafe { wide(p) }
}
