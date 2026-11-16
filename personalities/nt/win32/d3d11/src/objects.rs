//! The device's `Create*` and `Check*` methods (ADR 0211): resources, views, state objects,
//! queries, and the shader entry points, which refuse every bytecode (the software device runs no
//! shaders; ID3D11Device.CreateVertexShader.yaml observes only refusals). The rules are
//! `poly_d3d`'s; the objects live here.

use crate::com::*;
use crate::device::{Device, device_of};
use crate::texture::{self, Kind, ViewKind};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::Ordering;
use poly_d3d::{format, query, resource, state, view};

/// `D3D11_SUBRESOURCE_DATA` (d3d11.h): pSysMem, SysMemPitch, SysMemSlicePitch.
#[repr(C)]
#[derive(Clone, Copy)]
struct SubData {
    mem: *const u8,
    pitch: u32,
    slice_pitch: u32,
}

/// Set the caller's output to NULL (on failure).
unsafe fn null_out(out: *mut *mut c_void) {
    // SAFETY: the caller's optional output.
    unsafe { put(out, core::ptr::null_mut()) };
}

/// `CreateBuffer(pDesc, pInitialData, ppBuffer)`.
pub unsafe extern "system" fn create_buffer(
    this: *mut c_void,
    desc: *const resource::BufferDesc,
    init: *const SubDataPub,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    // SAFETY: a live device.
    let d = unsafe { device_of(this) };
    if d.removed.load(Ordering::Acquire) {
        return DXGI_ERROR_DEVICE_REMOVED;
    }
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's description and optional initial data.
    let (b, data) = unsafe { (*desc, if init.is_null() { None } else { Some(*(init as *const SubData)) }) };
    let checked = match resource::check_buffer(&b, data.map(|s| !s.mem.is_null())) {
        Ok(c) => c,
        Err(hr) => return hr,
    };
    if out.is_null() {
        return S_FALSE;
    }
    if b.byte_width >= 0xFFFF_0000 {
        return E_OUTOFMEMORY;
    }
    // SAFETY: a live device.
    let p = unsafe { texture::make(this, Kind::Buffer(checked), None) };
    if let Some(s) = data {
        // SAFETY: the new buffer; the caller's ByteWidth bytes.
        unsafe {
            let t = texture::res(p).expect("a buffer");
            core::ptr::copy_nonoverlapping(s.mem, t.data().as_mut_ptr(), b.byte_width as usize);
        }
    }
    // SAFETY: the caller's output.
    unsafe { *out = p };
    S_OK
}

/// The public name of `D3D11_SUBRESOURCE_DATA` for the vtable signatures.
pub type SubDataPub = [u8; 16];

/// `CreateTexture2D(pDesc, pInitialData, ppTexture2D)` (and `CreateTexture2D1`, whose
/// description has one more word, the layout, which must be UNDEFINED).
pub unsafe extern "system" fn create_texture_2d(
    this: *mut c_void,
    desc: *const resource::Tex2dDesc,
    init: *const SubDataPub,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    // SAFETY: a live device.
    let d = unsafe { device_of(this) };
    if d.removed.load(Ordering::Acquire) {
        return DXGI_ERROR_DEVICE_REMOVED;
    }
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's description.
    let t = unsafe { *desc };
    let subs = (if t.mips == 0 { resource::full_chain(t.width, t.height) } else { t.mips }) * t.array.max(1);
    let data: Option<Vec<SubData>> = if init.is_null() {
        None
    } else {
        // SAFETY: the caller's array of one D3D11_SUBRESOURCE_DATA per subresource.
        Some((0..subs.min(4096) as usize).map(|i| unsafe { *(init as *const SubData).add(i) }).collect())
    };
    let ok = data.as_ref().map(|v| v.iter().all(|s| !s.mem.is_null() && s.pitch != 0));
    let checked = match resource::check_tex2d(&t, ok) {
        Ok(c) => c,
        Err(hr) => return hr,
    };
    if out.is_null() {
        return S_FALSE;
    }
    // SAFETY: a live device.
    let p = unsafe { texture::make(this, Kind::Tex2d(checked), None) };
    if let Some(v) = data {
        // SAFETY: the new texture.
        let tx = unsafe { texture::res(p) }.expect("a texture");
        for (i, s) in v.iter().enumerate() {
            // SAFETY: the caller's rows of subresource `i`.
            unsafe { texture::fill_sub(tx, i as u32, s.mem, s.pitch as usize, s.slice_pitch as usize) };
        }
    }
    // SAFETY: the caller's output.
    unsafe { *out = p };
    S_OK
}

/// The view methods share this: check, then make.
unsafe fn make_view(
    this: *mut c_void,
    res: *mut c_void,
    kind: ViewKind,
    desc: Option<[u32; 6]>,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    // SAFETY: the caller's resource.
    let Some(t) = (unsafe { texture::res(res) }) else { return E_INVALIDARG };
    let r = texture::view_res(t);
    let done = match kind {
        ViewKind::Rtv => view::rtv(&r, desc.map(|d| [d[0], d[1], d[2], d[3], d[4]])).map(|d| [d[0], d[1], d[2], d[3], d[4], 0]),
        ViewKind::Srv => view::srv(&r, desc),
        ViewKind::Dsv => view::dsv(&r, desc),
        ViewKind::Uav => uav(&r, desc),
    };
    let done = match done {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    if out.is_null() {
        return S_FALSE;
    }
    // SAFETY: a live device and resource; the caller's output.
    unsafe { *out = texture::new_view(this, res, kind, done) };
    S_OK
}

/// An unordered-access view: the resource needs the UAV bind; a NULL description takes the
/// resource's format (Learn, "D3D11_UNORDERED_ACCESS_VIEW_DESC").
fn uav(r: &view::Res, desc: Option<[u32; 6]>) -> Result<[u32; 6], HRESULT> {
    match r {
        view::Res::Buffer(b) => {
            if b.bind & resource::bind::UNORDERED_ACCESS == 0 {
                return Err(E_INVALIDARG);
            }
            let d = desc.unwrap_or([0, 1, 0, b.byte_width / b.stride.max(4), 0, 0]);
            Ok([d[0], d[1], d[2], d[3], d[4], 0])
        }
        view::Res::Tex2d(t) => {
            if t.bind & resource::bind::UNORDERED_ACCESS == 0 {
                return Err(E_INVALIDARG);
            }
            match desc {
                None if format::is_typeless(t.format) => Err(E_INVALIDARG),
                // D3D11_UAV_DIMENSION_TEXTURE2D (4) / TEXTURE2DARRAY (5).
                None if t.array > 1 => Ok([t.format, 5, 0, 0, t.array, 0]),
                None => Ok([t.format, 4, 0, 0, 0, 0]),
                Some(d) => Ok([if d[0] == 0 { t.format } else { d[0] }, d[1], d[2], d[3], d[4], 0]),
            }
        }
    }
}

unsafe fn desc_words<const N: usize>(p: *const u32) -> Option<[u32; 6]> {
    if p.is_null() {
        return None;
    }
    let mut w = [0u32; 6];
    // SAFETY: the caller's description of N words.
    unsafe { core::ptr::copy_nonoverlapping(p, w.as_mut_ptr(), N) };
    Some(w)
}

pub unsafe extern "system" fn create_rtv(this: *mut c_void, res: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { make_view(this, res, ViewKind::Rtv, desc_words::<5>(desc), out) }
}

pub unsafe extern "system" fn create_srv(this: *mut c_void, res: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { make_view(this, res, ViewKind::Srv, desc_words::<6>(desc), out) }
}

pub unsafe extern "system" fn create_dsv(this: *mut c_void, res: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { make_view(this, res, ViewKind::Dsv, desc_words::<6>(desc), out) }
}

pub unsafe extern "system" fn create_uav(this: *mut c_void, res: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { make_view(this, res, ViewKind::Uav, desc_words::<5>(desc), out) }
}

// ----- shaders and input layouts: no bytecode is accepted

/// `Create{Vertex,Pixel,Geometry,Hull,Domain,Compute}Shader(pShaderBytecode, BytecodeLength,
/// pClassLinkage, ppShader)`: E_INVALIDARG, the output NULL (also for a NULL output).
pub unsafe extern "system" fn create_shader(
    _: *mut c_void,
    _: *const c_void,
    _: usize,
    _: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    log_once_shader();
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    E_INVALIDARG
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn create_gs_with_so(
    _: *mut c_void,
    _: *const c_void,
    _: usize,
    _: *const c_void,
    _: u32,
    _: *const u32,
    _: u32,
    _: u32,
    _: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    log_once_shader();
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    E_INVALIDARG
}

/// `CreateInputLayout(pInputElementDescs, NumElements, pShaderBytecodeWithInputSignature,
/// BytecodeLength, ppInputLayout)`.
pub unsafe extern "system" fn create_input_layout(
    _: *mut c_void,
    _: *const c_void,
    _: u32,
    _: *const c_void,
    _: usize,
    out: *mut *mut c_void,
) -> HRESULT {
    log_once_shader();
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    E_INVALIDARG
}

fn log_once_shader() {
    static ONCE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if !ONCE.swap(true, Ordering::Relaxed) {
        log("stub: d3d11: the software device runs no shaders (Create*Shader / CreateInputLayout refused)");
    }
}

// ----- state objects

/// Which state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateKind {
    Blend,
    Depth,
    Raster,
    Sampler,
}

#[repr(C)]
pub struct StateObj {
    obj: Obj,
    pub kind: StateKind,
    pub words: Vec<u32>,
    device: *mut c_void,
}

static BLEND_IIDS: [Guid; 3] = [IID_ID3D11BLENDSTATE, IID_ID3D11BLENDSTATE1, IID_ID3D11DEVICECHILD];
static DEPTH_IIDS: [Guid; 2] = [IID_ID3D11DEPTHSTENCILSTATE, IID_ID3D11DEVICECHILD];
static RASTER_IIDS: [Guid; 4] =
    [IID_ID3D11RASTERIZERSTATE, IID_ID3D11RASTERIZERSTATE1, IID_ID3D11RASTERIZERSTATE2, IID_ID3D11DEVICECHILD];
static SAMPLER_IIDS: [Guid; 2] = [IID_ID3D11SAMPLERSTATE, IID_ID3D11DEVICECHILD];

pub static STATE_VTBL: Vt<10> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    state_get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    state_get_desc as *const c_void,
    state_get_desc1 as *const c_void,
    state_get_desc2 as *const c_void,
]);

/// One of our state objects.
pub unsafe fn state_obj<'a>(p: *mut c_void) -> Option<&'a StateObj> {
    // SAFETY: a COM object's first word is its vtable.
    if p.is_null() || unsafe { *(p as *const *const c_void) } != STATE_VTBL.ptr() {
        return None;
    }
    // SAFETY: an object with the state vtable.
    Some(unsafe { &*(p as *const StateObj) })
}

/// The cached object of these normalised words, or a new one; `D3D11_ERROR_TOO_MANY_UNIQUE_STATE_OBJECTS`
/// past the limit.
unsafe fn state(this: *mut c_void, kind: StateKind, words: &[u32], out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return S_FALSE;
    }
    // SAFETY: a live device.
    let d = unsafe { device_of(this) };
    let limit = if kind == StateKind::Sampler { state::MAX_SAMPLERS } else { state::MAX_STATES };
    d.states.with(|cache| {
        if let Some(&(_, _, p)) = cache.iter().find(|(k, w, _)| *k == kind && w.as_slice() == words) {
            // SAFETY: a live cached object (it leaves the cache when its last reference goes).
            unsafe {
                add_ref(p as *mut c_void);
                *out = p as *mut c_void;
            }
            return S_OK;
        }
        if cache.iter().filter(|(k, _, _)| *k == kind).count() >= limit {
            return D3D11_ERROR_TOO_MANY_UNIQUE_STATE_OBJECTS;
        }
        let (iids, name): (&'static [Guid], _) = match kind {
            StateKind::Blend => (&BLEND_IIDS, "ID3D11BlendState"),
            StateKind::Depth => (&DEPTH_IIDS, "ID3D11DepthStencilState"),
            StateKind::Raster => (&RASTER_IIDS, "ID3D11RasterizerState"),
            StateKind::Sampler => (&SAMPLER_IIDS, "ID3D11SamplerState"),
        };
        // SAFETY: a live device.
        unsafe { add_ref(this) };
        let p = into_raw(StateObj { obj: Obj::new(STATE_VTBL.ptr(), iids, drop_state, name), kind, words: words.to_vec(), device: this });
        cache.push((kind, words.to_vec(), p as usize));
        // SAFETY: the caller's output.
        unsafe { *out = p };
        S_OK
    })
}

unsafe fn drop_state(p: *mut c_void) {
    // SAFETY: a state object whose last reference went.
    let device = unsafe { (*(p as *mut StateObj)).device };
    // SAFETY: a live device (the object holds it).
    unsafe { device_of(device) }.states.with(|c| c.retain(|(_, _, q)| *q != p as usize));
    // SAFETY: made by `state`; the device reference it took.
    unsafe {
        drop_box::<StateObj>(p);
        release(device);
    }
}

unsafe extern "system" fn state_get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live state object; the caller's output.
    unsafe {
        let d = (*(this as *mut StateObj)).device;
        add_ref(d);
        put(out, d);
    }
}

/// `GetDesc`: the normalised description in the interface's own layout.
unsafe extern "system" fn state_get_desc(this: *mut c_void, out: *mut u32) {
    // SAFETY: a live state object.
    let s = unsafe { &*(this as *const StateObj) };
    let words: Vec<u32> = match s.kind {
        StateKind::Blend => state::blend_to_desc(&s.words[..].try_into().expect("82 words")).to_vec(),
        StateKind::Raster => s.words[..10].to_vec(),
        _ => s.words.clone(),
    };
    // SAFETY: the caller's description.
    unsafe { write_words(out, &words) };
}

/// `GetDesc1` (blend: the 82-word form; rasterizer: 11 words).
unsafe extern "system" fn state_get_desc1(this: *mut c_void, out: *mut u32) {
    // SAFETY: a live state object.
    let s = unsafe { &*(this as *const StateObj) };
    let n = if s.kind == StateKind::Raster { 11 } else { s.words.len() };
    // SAFETY: the caller's description.
    unsafe { write_words(out, &s.words[..n]) };
}

unsafe extern "system" fn state_get_desc2(this: *mut c_void, out: *mut u32) {
    // SAFETY: a live state object; the caller's description.
    unsafe { write_words(out, &(*(this as *const StateObj)).words) };
}

unsafe fn write_words(out: *mut u32, w: &[u32]) {
    if !out.is_null() {
        // SAFETY: the caller's buffer of the description's size.
        unsafe { core::ptr::copy_nonoverlapping(w.as_ptr(), out, w.len()) };
    }
}

unsafe fn read<const N: usize>(p: *const u32) -> Option<[u32; N]> {
    if p.is_null() {
        return None;
    }
    let mut w = [0u32; N];
    // SAFETY: the caller's description of N words.
    unsafe { core::ptr::copy_nonoverlapping(p, w.as_mut_ptr(), N) };
    Some(w)
}

pub unsafe extern "system" fn create_blend_state(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output and description.
    unsafe { null_out(out) };
    let Some(d) = (unsafe { read::<66>(desc) }) else { return E_INVALIDARG };
    match state::blend(&state::blend_from_desc(&d)) {
        // SAFETY: as `state`.
        Ok(n) => unsafe { state(this, StateKind::Blend, &n, out) },
        Err(hr) => hr,
    }
}

pub unsafe extern "system" fn create_blend_state1(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output and description.
    unsafe { null_out(out) };
    let Some(d) = (unsafe { read::<82>(desc) }) else { return E_INVALIDARG };
    match state::blend(&d) {
        // SAFETY: as `state`.
        Ok(n) => unsafe { state(this, StateKind::Blend, &n, out) },
        Err(hr) => hr,
    }
}

pub unsafe extern "system" fn create_depth_stencil_state(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output and description.
    unsafe { null_out(out) };
    let Some(d) = (unsafe { read::<13>(desc) }) else { return E_INVALIDARG };
    match state::depth(&d) {
        // SAFETY: as `state`.
        Ok(n) => unsafe { state(this, StateKind::Depth, &n, out) },
        Err(hr) => hr,
    }
}

unsafe fn raster_n<const N: usize>(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output and description.
    unsafe { null_out(out) };
    let Some(d) = (unsafe { read::<N>(desc) }) else { return E_INVALIDARG };
    let mut w = [0u32; 12];
    w[..N].copy_from_slice(&d);
    match state::raster(&w) {
        // SAFETY: as `state`.
        Ok(n) => unsafe { state(this, StateKind::Raster, &n, out) },
        Err(hr) => hr,
    }
}

pub unsafe extern "system" fn create_rasterizer_state(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { raster_n::<10>(this, desc, out) }
}

pub unsafe extern "system" fn create_rasterizer_state1(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { raster_n::<11>(this, desc, out) }
}

pub unsafe extern "system" fn create_rasterizer_state2(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { raster_n::<12>(this, desc, out) }
}

pub unsafe extern "system" fn create_sampler_state(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output and description.
    unsafe { null_out(out) };
    let Some(d) = (unsafe { read::<13>(desc) }) else { return E_INVALIDARG };
    match state::sampler(&d) {
        // SAFETY: as `state`.
        Ok(n) => unsafe { state(this, StateKind::Sampler, &n, out) },
        Err(hr) => hr,
    }
}

// ----- queries

#[repr(C)]
pub struct QueryObj {
    obj: Obj,
    pub q: Lock<query::Query>,
    device: *mut c_void,
}

static QUERY_IIDS: [Guid; 4] = [IID_ID3D11QUERY, IID_ID3D11QUERY1, IID_ID3D11ASYNCHRONOUS, IID_ID3D11DEVICECHILD];
static PREDICATE_IIDS: [Guid; 5] =
    [IID_ID3D11PREDICATE, IID_ID3D11QUERY, IID_ID3D11QUERY1, IID_ID3D11ASYNCHRONOUS, IID_ID3D11DEVICECHILD];

pub static QUERY_VTBL: Vt<10> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    query_get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    query_get_data_size as *const c_void,
    query_get_desc as *const c_void,
    query_get_desc1 as *const c_void,
]);

/// One of our queries.
pub unsafe fn query_obj<'a>(p: *mut c_void) -> Option<&'a QueryObj> {
    // SAFETY: a COM object's first word is its vtable.
    if p.is_null() || unsafe { *(p as *const *const c_void) } != QUERY_VTBL.ptr() {
        return None;
    }
    // SAFETY: an object with the query vtable.
    Some(unsafe { &*(p as *const QueryObj) })
}

unsafe fn make_query(this: *mut c_void, desc: *const u32, out: *mut *mut c_void, predicate: bool) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    let Some(d) = (unsafe { read::<2>(desc) }) else { return E_INVALIDARG };
    let checked = if predicate { query::check_predicate(d[0], d[1]) } else { query::check_query(d[0], d[1]) };
    if let Err(hr) = checked {
        return hr;
    }
    if out.is_null() {
        return S_FALSE;
    }
    let iids: &'static [Guid] = if query::is_predicate(d[0]) { &PREDICATE_IIDS } else { &QUERY_IIDS };
    // SAFETY: a live device.
    unsafe { add_ref(this) };
    let p = into_raw(QueryObj {
        obj: Obj::new(QUERY_VTBL.ptr(), iids, drop_query, "ID3D11Query"),
        q: Lock::new(query::Query::new(d[0], d[1])),
        device: this,
    });
    // SAFETY: the caller's output.
    unsafe { *out = p };
    S_OK
}

unsafe fn drop_query(p: *mut c_void) {
    // SAFETY: a query whose last reference went.
    let device = unsafe { (*(p as *mut QueryObj)).device };
    // SAFETY: made by `make_query`; the device reference it took.
    unsafe {
        drop_box::<QueryObj>(p);
        release(device);
    }
}

pub unsafe extern "system" fn create_query(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { make_query(this, desc, out, false) }
}

pub unsafe extern "system" fn create_predicate(this: *mut c_void, desc: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { make_query(this, desc, out, true) }
}

/// `CreateCounter`: the software device has no counters.
pub unsafe extern "system" fn create_counter(_: *mut c_void, _: *const u32, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    E_INVALIDARG
}

/// `CheckCounterInfo`: no device-dependent counters, one parallel unit.
pub unsafe extern "system" fn check_counter_info(_: *mut c_void, out: *mut [u32; 3]) {
    // SAFETY: the caller's D3D11_COUNTER_INFO.
    unsafe { put(out, [0, 0, 1]) };
}

pub unsafe extern "system" fn check_counter(_: *mut c_void, _: *const u32, _: *mut u32, _: *mut u32, _: *mut u8, _: *mut u32, _: *mut u8, _: *mut u32, _: *mut u8, _: *mut u32) -> HRESULT {
    E_INVALIDARG
}

unsafe extern "system" fn query_get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a live query; the caller's output.
    unsafe {
        let d = (*(this as *mut QueryObj)).device;
        add_ref(d);
        put(out, d);
    }
}

unsafe extern "system" fn query_get_data_size(this: *mut c_void) -> u32 {
    // SAFETY: a live query.
    let k = unsafe { &*(this as *const QueryObj) }.q.with(|q| q.kind);
    query::data_size(k).unwrap_or(0)
}

unsafe extern "system" fn query_get_desc(this: *mut c_void, out: *mut [u32; 2]) {
    // SAFETY: a live query.
    let d = unsafe { &*(this as *const QueryObj) }.q.with(|q| [q.kind, q.misc]);
    // SAFETY: the caller's output.
    unsafe { put(out, d) };
}

/// `GetDesc1`: the type, the misc flags and `ContextType` 0 (ALL).
unsafe extern "system" fn query_get_desc1(this: *mut c_void, out: *mut [u32; 3]) {
    // SAFETY: a live query.
    let d = unsafe { &*(this as *const QueryObj) }.q.with(|q| [q.kind, q.misc, 0]);
    // SAFETY: the caller's output.
    unsafe { put(out, d) };
}

// ----- checks

/// `CheckFormatSupport(Format, pFormatSupport)`.
pub unsafe extern "system" fn check_format_support(this: *mut c_void, f: u32, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live device.
    let level = unsafe { device_of(this) }.level;
    let r = format::check_support(f, level);
    // SAFETY: the caller's output.
    unsafe { *out = r.unwrap_or(0) };
    r.map_or_else(|hr| hr, |_| S_OK)
}

/// `CheckMultisampleQualityLevels(Format, SampleCount, pNumQualityLevels)`
/// (ID3D11Device.CheckMultisampleQualityLevels.yaml).
pub unsafe extern "system" fn check_multisample_quality_levels(this: *mut c_void, f: u32, count: u32, out: *mut u32) -> HRESULT {
    // SAFETY: a live device.
    let level = unsafe { device_of(this) }.level;
    let mask = format::support_mask(f, level);
    if mask == 0 && f != 0 {
        // SAFETY: the caller's output.
        unsafe { put(out, 0) };
        return E_INVALIDARG;
    }
    if count == 0 || count > 32 {
        // SAFETY: the caller's output.
        unsafe { put(out, 0) };
        return E_FAIL;
    }
    let ms = mask & format::support::MULTISAMPLE_RENDERTARGET != 0 || mask & format::support::DEPTH_STENCIL != 0;
    let n = match count {
        1 => 1,
        2 | 4 | 8 | 16 if ms => 1,
        _ => 0,
    };
    // SAFETY: the caller's output.
    unsafe { put(out, n) };
    S_OK
}

/// `CheckFeatureSupport(Feature, pFeatureSupportData, FeatureSupportDataSize)`
/// (ID3D11Device.CheckFeatureSupport.yaml): the data of a WARP device of the device's level.
pub unsafe extern "system" fn check_feature_support(this: *mut c_void, feature: u32, data: *mut u8, size: u32) -> HRESULT {
    // SAFETY: a live device.
    let level = unsafe { device_of(this) }.level;
    let need: &[u32] = match feature {
        0 => &[8],
        1 | 4 | 6 | 7 | 9 | 11 | 12 | 15 | 17 | 18 | 19 => &[4],
        2 | 3 => &[8],
        5 => &[56],
        8 | 16 | 20 => &[8],
        10 | 13 => &[16],
        14 => &[32, 36],
        _ => return E_INVALIDARG,
    };
    if data.is_null() || !need.contains(&size) {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's structure of `size` bytes.
    let buf = unsafe { core::slice::from_raw_parts_mut(data, size as usize) };
    let words = |buf: &mut [u8], w: &[u32]| {
        for (i, v) in w.iter().enumerate() {
            buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    };
    let low = level < 0xB000;
    let l111 = level >= 0xB100;
    match feature {
        0 => words(buf, &[1, 0]),
        1 => words(buf, &[u32::from(!low)]),
        2 | 3 => {
            let f = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            let v = if feature == 2 { format::support_mask(f, level) } else { format::support2(f, level) };
            if feature == 3 && format::support_mask(f, level) == 0 {
                return E_INVALIDARG;
            }
            words(buf, &[f, v]);
        }
        4 => words(buf, &[1]),
        5 => {
            let mut w = [0u32; 14];
            for v in w.iter_mut().take(6) {
                *v = 1;
            }
            w[1] = u32::from(!low);
            words(buf, &w);
        }
        6 => words(buf, &[0]),
        7 | 9 | 11 | 12 | 15 | 17 => words(buf, &[1]),
        8 => words(buf, &[3, 3]),
        // D3D11_OPTIONS1: tiled resources tier, min/max filtering, clear view, map on default.
        10 => words(buf, &if l111 { [2, 1, 1, 1] } else if low { [0, 1, 1, 0] } else { [1, 1, 1, 1] }),
        13 => words(buf, &[1, 1, 1, 1]),
        // D3D11_OPTIONS2 (ID3D11Device.CheckFeatureSupport.yaml `levels`).
        14 => {
            let w = if l111 {
                [1, 1, 1, 3, 3, 1, 0, 0, 0]
            } else if low {
                [0, 0, 0, 0, 0, 1, 0, 0, 0]
            } else {
                [0, 1, 0, 0, 0, 1, 0, 0, 0]
            };
            words(buf, &w[..size as usize / 4]);
        }
        16 => words(buf, &[0x20, 0x2F]),
        18 => words(buf, &[0]),
        19 => words(buf, &[3]),
        20 => words(buf, &[0, 3]),
        _ => buf.fill(0),
    }
    S_OK
}

/// `OpenSharedResource(hResource, ReturnedInterface, ppResource)`: a legacy shared handle (the
/// value `GetSharedHandle` gave) opened on this device: a new object over the same storage.
pub unsafe extern "system" fn open_shared_resource(
    this: *mut c_void,
    handle: usize,
    riid: *const Guid,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    let Some(store) = texture::shares_find(handle) else { return E_INVALIDARG };
    let Some(kind) = texture::kind_of_store(&store) else { return E_INVALIDARG };
    // SAFETY: a live device; a new object over the shared storage.
    let p = unsafe { texture::make(this, kind, Some(store)) };
    // SAFETY: the new object; the caller's output.
    let hr = unsafe { query_interface(p, riid, out) };
    // SAFETY: the reference `make` gave.
    unsafe { release(p) };
    hr
}

/// The device's state that these methods read.
pub fn removed(d: &Device) -> bool {
    d.removed.load(Ordering::Acquire)
}

/// A new resource of this device over `store`, as interface `riid`.
unsafe fn open_store(this: *mut c_void, store: alloc::sync::Arc<texture::Store>, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    let Some(kind) = texture::kind_of_store(&store) else { return E_INVALIDARG };
    // SAFETY: a live device; a new object over the shared storage.
    let p = unsafe { texture::make(this, kind, Some(store)) };
    // SAFETY: the new object; the caller's output.
    let hr = unsafe { query_interface(p, riid, out) };
    // SAFETY: the reference `make` gave.
    unsafe { release(p) };
    hr
}

/// `OpenSharedResource1(hResource, returnedInterface, ppResource)`: an NT handle of
/// `CreateSharedHandle`; a legacy handle, a closed or foreign handle: E_INVALIDARG.
pub unsafe extern "system" fn open_shared_resource1(this: *mut c_void, handle: usize, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    match texture::nt_share(handle) {
        // SAFETY: as `open_store`.
        Some(s) => unsafe { open_store(this, s, riid, out) },
        None => E_INVALIDARG,
    }
}

/// `OpenSharedResourceByName(lpName, dwDesiredAccess, returnedInterface, ppResource)`.
pub unsafe extern "system" fn open_shared_resource_by_name(
    this: *mut c_void,
    name: *const u16,
    access: u32,
    riid: *const Guid,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { null_out(out) };
    if access == 0 {
        // The value the oracle gives for no access (OpenSharedResourceByName, CreateSharedHandle.yaml `by_name`).
        return 0x8876_086Au32 as i32;
    }
    // SAFETY: the caller's name.
    match texture::nt_share_by_name(&unsafe { texture::name_of(name) }) {
        // SAFETY: as `open_store`.
        Some(s) => unsafe { open_store(this, s, riid, out) },
        None => E_INVALIDARG,
    }
}
