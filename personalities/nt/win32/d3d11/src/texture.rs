//! 2D textures in memory (Learn: `ID3D11Device::CreateTexture2D`, `D3D11_TEXTURE2D_DESC`), their
//! render target views, and the DXGI views of a texture (`IDXGIResource`, `IDXGISurface`): one
//! subresource (mip 0 of slice 0), rows `pitch` bytes apart.

use crate::com::*;
use crate::stubs;
use alloc::vec::Vec;
use core::ffi::c_void;

/// `D3D11_RESOURCE_DIMENSION_TEXTURE2D` (d3d11.h).
const DIMENSION_TEXTURE2D: u32 = 3;
/// `D3D11_BIND_RENDER_TARGET` (d3d11.h).
pub const BIND_RENDER_TARGET: u32 = 0x20;

/// Bytes per pixel of a format (the 32-bit formats and their relatives; 4 for the rest).
pub fn bytes_per_pixel(format: u32) -> usize {
    match format {
        // R16G16B16A16 (float, unorm, uint, snorm, sint).
        10..=14 => 8,
        // R32G32B32A32.
        1..=4 => 16,
        // R8G8.
        48..=52 => 2,
        // R8, A8.
        60..=65 => 1,
        _ => 4,
    }
}

/// Whether a format stores red in the low byte (`DXGI_FORMAT_R8G8B8A8_*`, 27..=32 in
/// dxgiformat.h) rather than blue (`B8G8R8A8`, 87 and up).
pub fn red_first(format: u32) -> bool {
    matches!(format, 27..=32)
}

/// A texture: `D3D11_TEXTURE2D_DESC` as 11 words, and its pixels.
#[repr(C)]
pub struct Tex {
    obj: Obj,
    resource: Part,
    surface: Part,
    pub desc: [u32; 11],
    pub pitch: usize,
    pub data: Vec<u8>,
    device: *mut c_void,
}

/// A tear-off interface of an object: its own vtable, the object's lifetime.
#[repr(C)]
pub struct Part {
    pub vtbl: *const c_void,
    pub owner: *mut c_void,
}

impl Tex {
    pub fn width(&self) -> usize {
        self.desc[0] as usize
    }
    pub fn height(&self) -> usize {
        self.desc[1] as usize
    }
    pub fn format(&self) -> u32 {
        self.desc[4]
    }

    /// A copy of the pixels and description, no COM object (a copy within one texture).
    pub fn snapshot(t: &Tex) -> Tex {
        let none = core::ptr::null_mut();
        Tex {
            obj: Obj::new(TEX_VTBL.ptr(), &TEX_IIDS, drop_tex, "snapshot"),
            resource: Part { vtbl: RESOURCE_VTBL.ptr(), owner: none },
            surface: Part { vtbl: SURFACE_VTBL.ptr(), owner: none },
            desc: t.desc,
            pitch: t.pitch,
            data: t.data.clone(),
            device: none,
        }
    }
}

pub unsafe extern "system" fn part_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: a `Part` of a live object.
    unsafe { add_ref((*(this as *const Part)).owner) }
}

pub unsafe extern "system" fn part_release(this: *mut c_void) -> u32 {
    // SAFETY: a `Part` of a live object.
    unsafe { release((*(this as *const Part)).owner) }
}

pub unsafe extern "system" fn part_query_interface(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a `Part` of a live object.
    unsafe { query_interface((*(this as *const Part)).owner, riid, out) }
}

static TEX_IIDS: [Guid; 3] = [IID_ID3D11TEXTURE2D, IID_ID3D11RESOURCE, IID_ID3D11DEVICECHILD];

pub static TEX_VTBL: Vt<11> = Vt([
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

/// The texture behind a resource pointer, if it is one of ours.
pub unsafe fn tex<'a>(p: *mut c_void) -> Option<&'a mut Tex> {
    // SAFETY: a COM object's first word is its vtable.
    if p.is_null() || unsafe { *(p as *const *const c_void) } != TEX_VTBL.ptr() {
        return None;
    }
    // SAFETY: an object with the texture vtable is a `Tex`.
    Some(unsafe { &mut *(p as *mut Tex) })
}

/// A new texture of `device` (which it keeps a reference to) with `desc`, its pixels from
/// `init` (`D3D11_SUBRESOURCE_DATA`: the bytes and their row pitch) or zero.
pub unsafe fn create(device: *mut c_void, desc: [u32; 11], init: Option<(*const u8, usize)>) -> *mut c_void {
    let (w, h) = (desc[0] as usize, desc[1] as usize);
    let pitch = w * bytes_per_pixel(desc[4]);
    let mut data = alloc::vec![0u8; pitch * h];
    if let Some((src, src_pitch)) = init
        && !src.is_null()
    {
        for y in 0..h {
            // SAFETY: the caller's initial data has `h` rows `src_pitch` apart.
            let row = unsafe { core::slice::from_raw_parts(src.add(y * src_pitch), pitch) };
            data[y * pitch..(y + 1) * pitch].copy_from_slice(row);
        }
    }
    // SAFETY: a live device of this DLL.
    unsafe { add_ref(device) };
    let mut obj = Obj::new(TEX_VTBL.ptr(), &TEX_IIDS, drop_tex, "ID3D11Texture2D");
    obj.more = Some(tex_more);
    let p = into_raw(Tex {
        obj,
        resource: Part { vtbl: RESOURCE_VTBL.ptr(), owner: core::ptr::null_mut() },
        surface: Part { vtbl: SURFACE_VTBL.ptr(), owner: core::ptr::null_mut() },
        desc,
        pitch,
        data,
        device,
    });
    // SAFETY: the new texture.
    let t = unsafe { &mut *(p as *mut Tex) };
    t.resource.owner = p;
    t.surface.owner = p;
    p
}

unsafe fn drop_tex(p: *mut c_void) {
    // SAFETY: a texture whose last reference went.
    let device = unsafe { (*(p as *mut Tex)).device };
    // SAFETY: made by `create`.
    unsafe { drop_box::<Tex>(p) };
    // SAFETY: the reference `create` took.
    unsafe { release(device) };
}

/// The texture's DXGI interfaces.
unsafe fn tex_more(p: *mut c_void, iid: &Guid) -> *mut c_void {
    // SAFETY: a live texture.
    let t = unsafe { &mut *(p as *mut Tex) };
    let part = if *iid == IID_IDXGIRESOURCE || *iid == IID_IDXGIDEVICESUBOBJECT || *iid == IID_IDXGIOBJECT {
        &mut t.resource as *mut Part
    } else if *iid == IID_IDXGISURFACE {
        &mut t.surface as *mut Part
    } else {
        return core::ptr::null_mut();
    };
    // SAFETY: the texture is live.
    unsafe { add_ref(p) };
    part as *mut c_void
}

unsafe extern "system" fn get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live texture; the caller's output.
    unsafe {
        let d = (*(this as *mut Tex)).device;
        add_ref(d);
        put(out, d);
    }
}

unsafe extern "system" fn get_type(_: *mut c_void, out: *mut u32) {
    // SAFETY: the caller's output.
    unsafe { put(out, DIMENSION_TEXTURE2D) };
}

unsafe extern "system" fn set_eviction_priority(_: *mut c_void, _: u32) {}

unsafe extern "system" fn get_eviction_priority(_: *mut c_void) -> u32 {
    0
}

unsafe extern "system" fn get_desc(this: *mut c_void, out: *mut [u32; 11]) {
    // SAFETY: a live texture; the caller's D3D11_TEXTURE2D_DESC.
    unsafe { put(out, (*(this as *mut Tex)).desc) };
}

// ----- IDXGIResource, IDXGISurface

stubs!(res_stubs, "IDXGIResource", [GetSharedHandle]);

static RESOURCE_VTBL: Vt<12> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    dxgi_get_device as *const c_void,
    res_stubs::GetSharedHandle as *const c_void,
    get_usage as *const c_void,
    dxgi_set_eviction as *const c_void,
    dxgi_get_eviction as *const c_void,
]);

static SURFACE_VTBL: Vt<11> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    dxgi_get_device as *const c_void,
    surface_desc as *const c_void,
    surface_map as *const c_void,
    surface_unmap as *const c_void,
]);

unsafe fn part_tex<'a>(this: *mut c_void) -> &'a mut Tex {
    // SAFETY: a `Part` of a live texture.
    unsafe { &mut *((*(this as *const Part)).owner as *mut Tex) }
}

/// `GetParent` / `GetDevice` of a DXGI view: the device.
unsafe extern "system" fn dxgi_get_parent(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a live texture's device.
    unsafe { query_interface(part_tex(this).device, riid, out) }
}

unsafe extern "system" fn dxgi_get_device(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as above.
    unsafe { dxgi_get_parent(this, riid, out) }
}

/// `GetUsage`: `DXGI_USAGE_RENDER_TARGET_OUTPUT` for a render target, else `SHADER_INPUT`.
unsafe extern "system" fn get_usage(this: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: a live texture.
    let t = unsafe { part_tex(this) };
    let usage = if t.desc[8] & BIND_RENDER_TARGET != 0 { 0x20 } else { 0x10 };
    // SAFETY: the caller's output.
    unsafe { put(out, usage) };
    S_OK
}

unsafe extern "system" fn dxgi_set_eviction(_: *mut c_void, _: u32) -> HRESULT {
    S_OK
}

unsafe extern "system" fn dxgi_get_eviction(_: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { put(out, 0) };
    S_OK
}

/// `DXGI_SURFACE_DESC`: width, height, format, sample count and quality.
unsafe extern "system" fn surface_desc(this: *mut c_void, out: *mut [u32; 5]) -> HRESULT {
    // SAFETY: a live texture.
    let t = unsafe { part_tex(this) };
    // SAFETY: the caller's output.
    unsafe { put(out, [t.desc[0], t.desc[1], t.desc[4], t.desc[5], t.desc[6]]) };
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
        (out.add(8) as *mut *mut u8).write(t.data.as_mut_ptr());
    }
    S_OK
}

unsafe extern "system" fn surface_unmap(_: *mut c_void) -> HRESULT {
    S_OK
}

// ----- render target views

#[repr(C)]
pub struct Rtv {
    obj: Obj,
    pub tex: *mut c_void,
    desc: [u32; 5],
}

static RTV_IIDS: [Guid; 3] = [IID_ID3D11RENDERTARGETVIEW, IID_ID3D11VIEW, IID_ID3D11DEVICECHILD];

pub static RTV_VTBL: Vt<9> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    rtv_get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    rtv_get_resource as *const c_void,
    rtv_get_desc as *const c_void,
]);

/// A render target view of texture `tex` (which it keeps).
pub unsafe fn new_rtv(tex: *mut c_void, desc: Option<[u32; 5]>) -> *mut c_void {
    // SAFETY: a live texture.
    let t = unsafe { &*(tex as *const Tex) };
    // D3D11_RTV_DIMENSION_TEXTURE2D (4) of the texture's format.
    let desc = desc.unwrap_or([t.desc[4], 4, 0, 0, 0]);
    // SAFETY: as above.
    unsafe { add_ref(tex) };
    into_raw(Rtv { obj: Obj::new(RTV_VTBL.ptr(), &RTV_IIDS, drop_rtv, "ID3D11RenderTargetView"), tex, desc })
}

unsafe fn drop_rtv(p: *mut c_void) {
    // SAFETY: a view whose last reference went.
    let tex = unsafe { (*(p as *mut Rtv)).tex };
    // SAFETY: made by `new_rtv`; the texture reference it took.
    unsafe {
        drop_box::<Rtv>(p);
        release(tex);
    }
}

/// The texture of one of our render target views.
pub unsafe fn rtv_tex<'a>(p: *mut c_void) -> Option<&'a mut Tex> {
    // SAFETY: a COM object's first word is its vtable.
    if p.is_null() || unsafe { *(p as *const *const c_void) } != RTV_VTBL.ptr() {
        return None;
    }
    // SAFETY: a view with our vtable; its texture.
    unsafe { tex((*(p as *mut Rtv)).tex) }
}

unsafe extern "system" fn rtv_get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live view of a live texture.
    unsafe { get_device((*(this as *mut Rtv)).tex, out) }
}

unsafe extern "system" fn rtv_get_resource(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live view; the caller's output.
    unsafe {
        let t = (*(this as *mut Rtv)).tex;
        add_ref(t);
        put(out, t);
    }
}

unsafe extern "system" fn rtv_get_desc(this: *mut c_void, out: *mut [u32; 5]) {
    // SAFETY: a live view; the caller's D3D11_RENDER_TARGET_VIEW_DESC.
    unsafe { put(out, (*(this as *mut Rtv)).desc) };
}

/// Copy the rectangle `(sx, sy, w, h)` of `src` to `(dx, dy)` in `dst` (clipped to both), the
/// bytes as they are when the formats store the same size.
pub fn copy_region(dst: &mut Tex, dx: usize, dy: usize, src: &Tex, sx: usize, sy: usize, w: usize, h: usize) {
    let bpp = bytes_per_pixel(dst.format());
    if bpp != bytes_per_pixel(src.format()) {
        return;
    }
    let w = w.min(src.width().saturating_sub(sx)).min(dst.width().saturating_sub(dx));
    let h = h.min(src.height().saturating_sub(sy)).min(dst.height().saturating_sub(dy));
    let swap = red_first(dst.format()) != red_first(src.format()) && bpp == 4;
    for y in 0..h {
        let s = (sy + y) * src.pitch + sx * bpp;
        let d = (dy + y) * dst.pitch + dx * bpp;
        let row = &src.data[s..s + w * bpp];
        dst.data[d..d + w * bpp].copy_from_slice(row);
        if swap {
            for px in dst.data[d..d + w * bpp].chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
    }
}
