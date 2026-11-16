//! The device (`ID3D11Device`), its immediate context (`ID3D11DeviceContext`), and the device's
//! `IDXGIDevice1` and `ID3D11Multithread` interfaces. The context and the other interfaces share
//! the device's reference count (they are parts of one object).

use crate::com::*;
use crate::texture::{Part, part_add_ref, part_query_interface, part_release};
use crate::{context, objects, stubs};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

#[repr(C)]
pub struct Device {
    obj: Obj,
    context: Part,
    dxgi: Part,
    multithread: Part,
    /// The device's identity for `IUnknown` (not the device pointer, D3D11CreateDevice.yaml).
    unknown: Part,
    /// The context's `ID3DUserDefinedAnnotation`.
    annotation: Part,
    flags: u32,
    pub level: u32,
    protected: AtomicBool,
    /// An invalid copy box removed the device (CopyResource.yaml `invalid_boxes_remove_device`).
    pub removed: AtomicBool,
    exception_mode: AtomicU32,
    gpu_priority: AtomicI32,
    frame_latency: AtomicU32,
    /// The live state objects by their normalised description (equal ones are one object).
    pub states: Lock<Vec<(objects::StateKind, Vec<u32>, usize)>>,
    /// The immediate context's bindings.
    pub pipe: Lock<context::State>,
    /// The immediate context's own reference count; while it is above 0 the context holds one
    /// reference on the device (D3D11CreateDevice.yaml `references`: device 2, context 1).
    ctx_refs: AtomicU32,
    /// The adapter (a reference the device holds), made on first use when none was given.
    adapter: core::sync::atomic::AtomicPtr<c_void>,
}

static DEVICE_IIDS: [Guid; 6] =
    [IID_ID3D11DEVICE, IID_ID3D11DEVICE1, IID_ID3D11DEVICE2, IID_ID3D11DEVICE3, IID_ID3D11DEVICE4, IID_ID3D11DEVICE5];

/// A new device; `adapter` (one more reference is taken) is the adapter it was made on, or null.
pub fn new(flags: u32, level: u32, adapter: *mut c_void) -> *mut c_void {
    if !adapter.is_null() {
        // SAFETY: the caller's live adapter.
        unsafe { com_add_ref(adapter) };
    }
    let mut obj = Obj::new(DEVICE_VTBL.ptr(), &DEVICE_IIDS, drop_device, "ID3D11Device");
    obj.more = Some(device_more);
    let p = into_raw(Device {
        obj,
        context: Part { vtbl: CONTEXT_VTBL.ptr(), owner: core::ptr::null_mut() },
        dxgi: Part { vtbl: DXGI_DEVICE_VTBL.ptr(), owner: core::ptr::null_mut() },
        multithread: Part { vtbl: MULTITHREAD_VTBL.ptr(), owner: core::ptr::null_mut() },
        unknown: Part { vtbl: UNKNOWN_VTBL.ptr(), owner: core::ptr::null_mut() },
        annotation: Part { vtbl: ANNOTATION_VTBL.ptr(), owner: core::ptr::null_mut() },
        flags,
        level,
        protected: AtomicBool::new(false),
        removed: AtomicBool::new(false),
        exception_mode: AtomicU32::new(0),
        gpu_priority: AtomicI32::new(0),
        frame_latency: AtomicU32::new(3),
        states: Lock::new(Vec::new()),
        pipe: Lock::new(context::State::new()),
        ctx_refs: AtomicU32::new(0),
        adapter: core::sync::atomic::AtomicPtr::new(adapter),
    });
    // SAFETY: the new device.
    let d = unsafe { &mut *(p as *mut Device) };
    d.context.owner = p;
    d.dxgi.owner = p;
    d.multithread.owner = p;
    d.unknown.owner = p;
    d.annotation.owner = p;
    p
}

unsafe fn drop_device(p: *mut c_void) {
    // SAFETY: a device whose last reference went.
    let a = unsafe { (*(p as *mut Device)).adapter.load(Ordering::Acquire) };
    // SAFETY: made by `new`.
    unsafe { drop_box::<Device>(p) };
    if !a.is_null() {
        // SAFETY: the reference the device held.
        unsafe { com_release(a) };
    }
}

/// The device of `p` (the device itself or one of its parts: `this` of any of its interfaces).
pub unsafe fn device_of<'a>(p: *mut c_void) -> &'a Device {
    // SAFETY: a COM object's first word is its vtable.
    let vt = unsafe { *(p as *const *const c_void) };
    let dev = if vt == DEVICE_VTBL.ptr() { p } else { unsafe { part_owner(p) } };
    // SAFETY: a live device of this DLL.
    unsafe { &*(dev as *const Device) }
}

/// The immediate context of a device, with a reference.
pub unsafe fn immediate_context(dev: *mut c_void) -> *mut c_void {
    // SAFETY: a live device.
    unsafe {
        ctx_add_ref(dev);
        &mut (*(dev as *mut Device)).context as *mut Part as *mut c_void
    }
}

/// One more reference on the immediate context of device `dev`.
unsafe fn ctx_add_ref(dev: *mut c_void) -> u32 {
    // SAFETY: a live device.
    let d = unsafe { &*(dev as *const Device) };
    let n = d.ctx_refs.fetch_add(1, Ordering::AcqRel) + 1;
    if n == 1 {
        // SAFETY: as above.
        unsafe { add_ref(dev) };
    }
    n
}

unsafe extern "system" fn ctx_part_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: a part of the context of a live device.
    unsafe { ctx_add_ref(part_owner(this)) }
}

unsafe extern "system" fn ctx_part_release(this: *mut c_void) -> u32 {
    // SAFETY: a part of the context of a live device.
    let dev = unsafe { part_owner(this) };
    // SAFETY: as above.
    let d = unsafe { &*(dev as *const Device) };
    let n = d.ctx_refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the reference the context held.
        unsafe { release(dev) };
    }
    n
}

unsafe fn device_more(p: *mut c_void, iid: &Guid) -> *mut c_void {
    // SAFETY: a live device.
    let d = unsafe { &mut *(p as *mut Device) };
    let dxgi =
        [IID_IDXGIOBJECT, IID_IDXGIDEVICE, IID_IDXGIDEVICE1, IID_IDXGIDEVICE2, IID_IDXGIDEVICE3, IID_IDXGIDEVICE4];
    let part = if dxgi.contains(iid) {
        &mut d.dxgi as *mut Part
    } else if *iid == IID_IUNKNOWN {
        &mut d.unknown as *mut Part
    } else if *iid == IID_ID3D11MULTITHREAD {
        &mut d.multithread as *mut Part
    } else {
        return core::ptr::null_mut();
    };
    // SAFETY: the device is live.
    unsafe { add_ref(p) };
    part as *mut c_void
}

/// The device of `p` when it is one of our devices or its IDXGIDevice.
pub unsafe fn as_device(p: *mut c_void) -> Option<*mut c_void> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a COM object's first word is its vtable.
    let vt = unsafe { *(p as *const *const c_void) };
    if vt == DEVICE_VTBL.ptr() {
        Some(p)
    } else if vt == DXGI_DEVICE_VTBL.ptr() {
        // SAFETY: a part of a live device.
        Some(unsafe { part_owner(p) })
    } else {
        None
    }
}

/// Whether `p` is one of our devices.
pub unsafe fn is_device(p: *mut c_void) -> bool {
    // SAFETY: a COM object's first word is its vtable.
    !p.is_null() && unsafe { *(p as *const *const c_void) } == DEVICE_VTBL.ptr()
}

// ----- ID3D11Device

stubs!(
    dev_stubs,
    "ID3D11Device",
    [
        CreateTexture1D,
        CreateTexture3D,
        CreateClassLinkage,
        CreateDeferredContext,
        CreateDeferredContext1,
        CreateDeviceContextState,
        CreateDeferredContext2,
        GetResourceTiling,
        CreateTexture3D1,
        CreateDeferredContext3,
        WriteToSubresource,
        ReadFromSubresource,
        OpenSharedFence,
        CreateFence,
    ]
);

/// `ID3D11Device5` (the SDK header's method order; generated from d3d11*.h).
static DEVICE_VTBL: Vt<69> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    objects::create_buffer as *const c_void,
    dev_stubs::CreateTexture1D as *const c_void,
    objects::create_texture_2d as *const c_void,
    dev_stubs::CreateTexture3D as *const c_void,
    objects::create_srv as *const c_void,
    objects::create_uav as *const c_void,
    objects::create_rtv as *const c_void,
    objects::create_dsv as *const c_void,
    objects::create_input_layout as *const c_void,
    objects::create_shader as *const c_void,
    objects::create_shader as *const c_void,
    objects::create_gs_with_so as *const c_void,
    objects::create_shader as *const c_void,
    objects::create_shader as *const c_void,
    objects::create_shader as *const c_void,
    objects::create_shader as *const c_void,
    dev_stubs::CreateClassLinkage as *const c_void,
    objects::create_blend_state as *const c_void,
    objects::create_depth_stencil_state as *const c_void,
    objects::create_rasterizer_state as *const c_void,
    objects::create_sampler_state as *const c_void,
    objects::create_query as *const c_void,
    objects::create_predicate as *const c_void,
    objects::create_counter as *const c_void,
    dev_stubs::CreateDeferredContext as *const c_void,
    objects::open_shared_resource as *const c_void,
    objects::check_format_support as *const c_void,
    objects::check_multisample_quality_levels as *const c_void,
    objects::check_counter_info as *const c_void,
    objects::check_counter as *const c_void,
    objects::check_feature_support as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_feature_level as *const c_void,
    get_creation_flags as *const c_void,
    get_device_removed_reason as *const c_void,
    get_immediate_context as *const c_void,
    set_exception_mode as *const c_void,
    get_exception_mode as *const c_void,
    get_immediate_context as *const c_void,
    dev_stubs::CreateDeferredContext1 as *const c_void,
    objects::create_blend_state1 as *const c_void,
    objects::create_rasterizer_state1 as *const c_void,
    dev_stubs::CreateDeviceContextState as *const c_void,
    objects::open_shared_resource1 as *const c_void,
    objects::open_shared_resource_by_name as *const c_void,
    get_immediate_context as *const c_void,
    dev_stubs::CreateDeferredContext2 as *const c_void,
    dev_stubs::GetResourceTiling as *const c_void,
    check_multisample_quality_levels1 as *const c_void,
    objects::create_texture_2d as *const c_void,
    dev_stubs::CreateTexture3D1 as *const c_void,
    objects::create_rasterizer_state2 as *const c_void,
    objects::create_srv as *const c_void,
    objects::create_uav as *const c_void,
    objects::create_rtv as *const c_void,
    objects::create_query as *const c_void,
    get_immediate_context as *const c_void,
    dev_stubs::CreateDeferredContext3 as *const c_void,
    dev_stubs::WriteToSubresource as *const c_void,
    dev_stubs::ReadFromSubresource as *const c_void,
    register_device_removed_event as *const c_void,
    unregister_device_removed as *const c_void,
    dev_stubs::OpenSharedFence as *const c_void,
    dev_stubs::CreateFence as *const c_void,
]);

/// `CheckMultisampleQualityLevels1(Format, SampleCount, Flags, pNumQualityLevels)`.
unsafe extern "system" fn check_multisample_quality_levels1(
    this: *mut c_void,
    format: u32,
    count: u32,
    _flags: u32,
    out: *mut u32,
) -> HRESULT {
    // SAFETY: the caller's arguments, passed on.
    unsafe { objects::check_multisample_quality_levels(this, format, count, out) }
}

/// `RegisterDeviceRemovedEvent(hEvent, pdwCookie)`: a software device is never removed by a driver,
/// so the event is never set; the cookie names the registration.
unsafe extern "system" fn register_device_removed_event(_: *mut c_void, event: usize, cookie: *mut u32) -> HRESULT {
    if event == 0 || cookie.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *cookie = 1 };
    S_OK
}

unsafe extern "system" fn unregister_device_removed(_: *mut c_void, _: u32) {}

/// `IUnknown` of the device's identity part.
static UNKNOWN_VTBL: Vt<3> =
    Vt([part_query_interface as *const c_void, part_add_ref as *const c_void, part_release as *const c_void]);

unsafe extern "system" fn get_feature_level(this: *mut c_void) -> u32 {
    // SAFETY: a live device.
    unsafe { (*(this as *mut Device)).level }
}

unsafe extern "system" fn get_creation_flags(this: *mut c_void) -> u32 {
    // SAFETY: a live device.
    unsafe { (*(this as *mut Device)).flags }
}

/// `GetDeviceRemovedReason`: S_OK, or DXGI_ERROR_DEVICE_REMOVED after an invalid copy box.
unsafe extern "system" fn get_device_removed_reason(this: *mut c_void) -> HRESULT {
    // SAFETY: a live device.
    if unsafe { device_of(this) }.removed.load(Ordering::Acquire) { DXGI_ERROR_DEVICE_REMOVED } else { S_OK }
}

unsafe extern "system" fn get_immediate_context(this: *mut c_void, out: *mut *mut c_void) {
    if !out.is_null() {
        // SAFETY: a live device; the caller's output.
        unsafe { *out = immediate_context(this) };
    }
}

/// `SetExceptionMode`: 0 or D3D11_RAISE_FLAG_DRIVER_INTERNAL_ERROR (1); others leave the mode
/// (ID3D11Device.GetCreationFlags.yaml `exception_mode`).
unsafe extern "system" fn set_exception_mode(this: *mut c_void, mode: u32) -> HRESULT {
    if mode > 1 {
        return E_INVALIDARG;
    }
    // SAFETY: a live device.
    unsafe { device_of(this) }.exception_mode.store(mode, Ordering::Relaxed);
    S_OK
}

unsafe extern "system" fn get_exception_mode(this: *mut c_void) -> u32 {
    // SAFETY: a live device.
    unsafe { device_of(this) }.exception_mode.load(Ordering::Relaxed)
}

// ----- ID3D11DeviceContext

stubs!(
    ctx_stubs,
    "ID3D11DeviceContext",
    [
        CopyStructureCount,
        ExecuteCommandList,
        CSSetUnorderedAccessViews,
        CSGetUnorderedAccessViews,
        FinishCommandList,
        SwapDeviceContextState,
        UpdateTileMappings,
        CopyTileMappings,
        CopyTiles,
        UpdateTiles,
        ResizeTilePool,
        TiledResourceBarrier,
        SetHardwareProtectionState,
        GetHardwareProtectionState,
        Signal,
        Wait,
    ]
);

/// `ID3D11DeviceContext4` (the SDK header's method order; generated from d3d11*.h).
static CONTEXT_VTBL: Vt<149> = Vt([
    ctx_query_interface as *const c_void,
    ctx_part_add_ref as *const c_void,
    ctx_part_release as *const c_void,
    ctx_get_device as *const c_void,
    part_get_private_data as *const c_void,
    part_set_private_data as *const c_void,
    part_set_private_data_interface as *const c_void,
    context::vs_set_cb as *const c_void,
    context::ps_set_srv as *const c_void,
    context::ps_set_sh as *const c_void,
    context::ps_set_smp as *const c_void,
    context::vs_set_sh as *const c_void,
    context::draw3 as *const c_void,
    context::draw2 as *const c_void,
    context::map as *const c_void,
    context::unmap as *const c_void,
    context::ps_set_cb as *const c_void,
    context::ia_set_input_layout as *const c_void,
    context::ia_set_vertex_buffers as *const c_void,
    context::ia_set_index_buffer as *const c_void,
    context::draw5 as *const c_void,
    context::draw4 as *const c_void,
    context::gs_set_cb as *const c_void,
    context::gs_set_sh as *const c_void,
    context::ia_set_topology as *const c_void,
    context::vs_set_srv as *const c_void,
    context::vs_set_smp as *const c_void,
    context::begin as *const c_void,
    context::end as *const c_void,
    context::get_data as *const c_void,
    context::set_predication as *const c_void,
    context::gs_set_srv as *const c_void,
    context::gs_set_smp as *const c_void,
    context::om_set_render_targets as *const c_void,
    context::om_set_rt_and_uavs as *const c_void,
    context::om_set_blend_state as *const c_void,
    context::om_set_depth_state as *const c_void,
    context::so_set_targets as *const c_void,
    context::draw0 as *const c_void,
    context::draw_indirect as *const c_void,
    context::draw_indirect as *const c_void,
    context::dispatch as *const c_void,
    context::draw_indirect as *const c_void,
    context::rs_set_state as *const c_void,
    context::rs_set_viewports as *const c_void,
    context::rs_set_scissors as *const c_void,
    context::copy_subresource_region as *const c_void,
    context::copy_resource as *const c_void,
    context::update_subresource as *const c_void,
    ctx_stubs::CopyStructureCount as *const c_void,
    context::clear_render_target_view as *const c_void,
    context::clear_uav_uint as *const c_void,
    context::clear_uav_float as *const c_void,
    context::clear_depth_stencil_view as *const c_void,
    context::generate_mips as *const c_void,
    context::set_resource_min_lod as *const c_void,
    context::get_resource_min_lod as *const c_void,
    context::resolve_subresource as *const c_void,
    ctx_stubs::ExecuteCommandList as *const c_void,
    context::hs_set_srv as *const c_void,
    context::hs_set_sh as *const c_void,
    context::hs_set_smp as *const c_void,
    context::hs_set_cb as *const c_void,
    context::ds_set_srv as *const c_void,
    context::ds_set_sh as *const c_void,
    context::ds_set_smp as *const c_void,
    context::ds_set_cb as *const c_void,
    context::cs_set_srv as *const c_void,
    ctx_stubs::CSSetUnorderedAccessViews as *const c_void,
    context::cs_set_sh as *const c_void,
    context::cs_set_smp as *const c_void,
    context::cs_set_cb as *const c_void,
    context::vs_get_cb as *const c_void,
    context::ps_get_srv as *const c_void,
    context::ps_get_sh as *const c_void,
    context::ps_get_smp as *const c_void,
    context::vs_get_sh as *const c_void,
    context::ps_get_cb as *const c_void,
    context::ia_get_input_layout as *const c_void,
    context::ia_get_vertex_buffers as *const c_void,
    context::ia_get_index_buffer as *const c_void,
    context::gs_get_cb as *const c_void,
    context::gs_get_sh as *const c_void,
    context::ia_get_topology as *const c_void,
    context::vs_get_srv as *const c_void,
    context::vs_get_smp as *const c_void,
    context::get_predication as *const c_void,
    context::gs_get_srv as *const c_void,
    context::gs_get_smp as *const c_void,
    context::om_get_render_targets as *const c_void,
    context::om_get_rt_and_uavs as *const c_void,
    context::om_get_blend_state as *const c_void,
    context::om_get_depth_state as *const c_void,
    context::so_get_targets as *const c_void,
    context::rs_get_state as *const c_void,
    context::rs_get_viewports as *const c_void,
    context::rs_get_scissors as *const c_void,
    context::hs_get_srv as *const c_void,
    context::hs_get_sh as *const c_void,
    context::hs_get_smp as *const c_void,
    context::hs_get_cb as *const c_void,
    context::ds_get_srv as *const c_void,
    context::ds_get_sh as *const c_void,
    context::ds_get_smp as *const c_void,
    context::ds_get_cb as *const c_void,
    context::cs_get_srv as *const c_void,
    ctx_stubs::CSGetUnorderedAccessViews as *const c_void,
    context::cs_get_sh as *const c_void,
    context::cs_get_smp as *const c_void,
    context::cs_get_cb as *const c_void,
    context::clear_state as *const c_void,
    nothing as *const c_void,
    get_type as *const c_void,
    get_context_flags as *const c_void,
    ctx_stubs::FinishCommandList as *const c_void,
    context::copy_subresource_region1 as *const c_void,
    context::update_subresource1 as *const c_void,
    discard as *const c_void,
    discard as *const c_void,
    context::vs_set_cb1 as *const c_void,
    context::hs_set_cb1 as *const c_void,
    context::ds_set_cb1 as *const c_void,
    context::gs_set_cb1 as *const c_void,
    context::ps_set_cb1 as *const c_void,
    context::cs_set_cb1 as *const c_void,
    context::vs_get_cb1 as *const c_void,
    context::hs_get_cb1 as *const c_void,
    context::ds_get_cb1 as *const c_void,
    context::gs_get_cb1 as *const c_void,
    context::ps_get_cb1 as *const c_void,
    context::cs_get_cb1 as *const c_void,
    ctx_stubs::SwapDeviceContextState as *const c_void,
    context::clear_view as *const c_void,
    discard_view1 as *const c_void,
    ctx_stubs::UpdateTileMappings as *const c_void,
    ctx_stubs::CopyTileMappings as *const c_void,
    ctx_stubs::CopyTiles as *const c_void,
    ctx_stubs::UpdateTiles as *const c_void,
    ctx_stubs::ResizeTilePool as *const c_void,
    ctx_stubs::TiledResourceBarrier as *const c_void,
    is_annotation_enabled as *const c_void,
    set_marker_int as *const c_void,
    set_marker_int as *const c_void,
    nothing as *const c_void,
    flush1 as *const c_void,
    ctx_stubs::SetHardwareProtectionState as *const c_void,
    ctx_stubs::GetHardwareProtectionState as *const c_void,
    ctx_stubs::Signal as *const c_void,
    ctx_stubs::Wait as *const c_void,
]);

static CONTEXT_IIDS: [Guid; 6] = [
    IID_ID3D11DEVICECONTEXT,
    IID_ID3D11DEVICECONTEXT1,
    IID_ID3D11DEVICECONTEXT2,
    IID_ID3D11DEVICECONTEXT3,
    IID_ID3D11DEVICECONTEXT4,
    IID_ID3D11DEVICECHILD,
];

/// The immediate context's `QueryInterface`: its own interfaces (the same pointer), the annotation
/// and multithread interfaces (other pointers), never the device's (D3D11CreateDevice.yaml
/// `interfaces_context`).
unsafe extern "system" fn ctx_query_interface(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output pointer.
    unsafe { *out = core::ptr::null_mut() };
    if riid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID; the context is a `Part` of a live device.
    let (iid, dev) = unsafe { (*riid, (*(this as *const Part)).owner) };
    // SAFETY: a live device.
    let d = unsafe { &mut *(dev as *mut Device) };
    let p = if iid == IID_IUNKNOWN || CONTEXT_IIDS.contains(&iid) {
        this
    } else if iid == IID_ID3DUSERDEFINEDANNOTATION {
        &mut d.annotation as *mut Part as *mut c_void
    } else if iid == IID_ID3D11MULTITHREAD {
        // SAFETY: the device is live; the caller's output.
        unsafe {
            add_ref(dev);
            *out = &mut d.multithread as *mut Part as *mut c_void;
        }
        return S_OK;
    } else {
        log_iid("ID3D11DeviceContext", &iid);
        return E_NOINTERFACE;
    };
    // SAFETY: the device is live; the caller's output.
    unsafe {
        ctx_add_ref(dev);
        *out = p;
    }
    S_OK
}

/// `DiscardResource`, `DiscardView`: the contents may be dropped; keeping them is allowed.
unsafe extern "system" fn discard(_: *mut c_void, _: *mut c_void) {}

unsafe extern "system" fn discard_view1(_: *mut c_void, _: *mut c_void, _: *const c_void, _: u32) {}

/// `IsAnnotationEnabled`: no profiling tool is attached.
unsafe extern "system" fn is_annotation_enabled(_: *mut c_void) -> i32 {
    0
}

/// `SetMarkerInt(pLabel, Data)`, `BeginEventInt(pLabel, Data)`: nothing records them.
unsafe extern "system" fn set_marker_int(_: *mut c_void, _: *const u16, _: i32) {}

/// `Flush1(ContextType, hEvent)`: the work is done; the event is set at once.
unsafe extern "system" fn flush1(_: *mut c_void, _: u32, event: usize) {
    if event != 0 {
        // SAFETY: the caller's event handle.
        unsafe { SetEvent(event) };
    }
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetEvent(event: usize) -> i32;
}

// ----- ID3DUserDefinedAnnotation (Learn: without a profiling tool nothing records the events and
// GetStatus is FALSE)

static ANNOTATION_VTBL: Vt<7> = Vt([
    annotation_query_interface as *const c_void,
    ctx_part_add_ref as *const c_void,
    ctx_part_release as *const c_void,
    begin_event as *const c_void,
    end_event as *const c_void,
    set_marker as *const c_void,
    get_status as *const c_void,
]);

unsafe extern "system" fn annotation_query_interface(
    this: *mut c_void,
    riid: *const Guid,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: a `Part` of a live device: its context answers.
    unsafe {
        let d = &mut *((*(this as *const Part)).owner as *mut Device);
        ctx_query_interface(&mut d.context as *mut Part as *mut c_void, riid, out)
    }
}

/// `BeginEvent` / `EndEvent`: -1, nothing records events (Learn: the value means something only
/// under a profiling tool).
unsafe extern "system" fn begin_event(_: *mut c_void, _: *const u16) -> i32 {
    -1
}

unsafe extern "system" fn end_event(_: *mut c_void) -> i32 {
    -1
}

unsafe extern "system" fn set_marker(_: *mut c_void, _: *const u16) {}

unsafe extern "system" fn get_status(_: *mut c_void) -> i32 {
    0
}

/// `ClearState`, `Flush`: the copies are done when they are made.
unsafe extern "system" fn nothing(_: *mut c_void) {}

/// `GetType`: `D3D11_DEVICE_CONTEXT_IMMEDIATE`.
unsafe extern "system" fn get_type(_: *mut c_void) -> u32 {
    0
}

unsafe extern "system" fn get_context_flags(_: *mut c_void) -> u32 {
    0
}

pub unsafe extern "system" fn ctx_get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a `Part` of a live device; the caller's output.
    unsafe {
        let d = (*(this as *const Part)).owner;
        add_ref(d);
        put(out, d);
    }
}

// ----- IDXGIDevice1

#[cfg_attr(target_arch = "x86", link(name = "dxgi", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "dxgi", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateDXGIFactory1(riid: *const c_void, out: *mut *mut c_void) -> HRESULT;
}

/// `IID_IDXGIFactory1` (dxgi.h).
const IID_IDXGIFACTORY1: Guid = guid(0x770aae78, 0xf26f, 0x4dba, 0xa829, 0x253c83d1b387);

stubs!(dxgi_stubs, "IDXGIDevice", [CreateSurface]);

/// `IDXGIDevice4` (dxgi1_5.h).
static DXGI_DEVICE_VTBL: Vt<20> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    part_set_private_data as *const c_void,
    part_set_private_data_interface as *const c_void,
    part_get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    get_adapter as *const c_void,
    dxgi_stubs::CreateSurface as *const c_void,
    query_resource_residency as *const c_void,
    set_gpu_thread_priority as *const c_void,
    get_gpu_thread_priority as *const c_void,
    set_maximum_frame_latency as *const c_void,
    get_maximum_frame_latency as *const c_void,
    // IDXGIDevice2
    offer_resources as *const c_void,
    reclaim_resources as *const c_void,
    enqueue_set_event as *const c_void,
    // IDXGIDevice3: Trim
    nothing as *const c_void,
    // IDXGIDevice4
    offer_resources1 as *const c_void,
    reclaim_resources1 as *const c_void,
]);

/// `OfferResources(NumResources, ppResources, Priority)`: this device never takes memory back from
/// a resource, so an offer is accepted and changes nothing.
unsafe extern "system" fn offer_resources(_: *mut c_void, n: u32, res: *const *mut c_void, _: u32) -> HRESULT {
    if n == 0 || res.is_null() { E_INVALIDARG } else { S_OK }
}

unsafe extern "system" fn offer_resources1(
    this: *mut c_void,
    n: u32,
    res: *const *mut c_void,
    priority: u32,
    _: u32,
) -> HRESULT {
    // SAFETY: the caller's arguments, passed on.
    unsafe { offer_resources(this, n, res, priority) }
}

/// `ReclaimResources(NumResources, ppResources, pDiscarded)`: nothing was discarded.
unsafe extern "system" fn reclaim_resources(
    _: *mut c_void,
    n: u32,
    res: *const *mut c_void,
    discarded: *mut i32,
) -> HRESULT {
    if n == 0 || res.is_null() {
        return E_INVALIDARG;
    }
    if !discarded.is_null() {
        for i in 0..n as usize {
            // SAFETY: the caller's array of `n` BOOLs.
            unsafe { *discarded.add(i) = 0 };
        }
    }
    S_OK
}

/// `ReclaimResources1`: every result `DXGI_RECLAIM_RESOURCE_RESULT_OK` (0, dxgi1_5.h).
unsafe extern "system" fn reclaim_resources1(
    this: *mut c_void,
    n: u32,
    res: *const *mut c_void,
    results: *mut i32,
) -> HRESULT {
    // SAFETY: the same outputs (one 32-bit value each).
    unsafe { reclaim_resources(this, n, res, results) }
}

/// `EnqueueSetEvent(hEvent)`: the work queued before it is done, so the event is set now.
unsafe extern "system" fn enqueue_set_event(_: *mut c_void, event: usize) -> HRESULT {
    // SAFETY: the caller's handle.
    if event == 0 || unsafe { SetEvent(event) } == 0 { E_INVALIDARG } else { S_OK }
}

/// `GetAdapter`: the adapter the device was made on (the one passed to `D3D11CreateDevice`, or
/// dxgi's software adapter made once for the device), with one more reference
/// (IDXGIDevice.GetAdapter.yaml `adapter`).
unsafe extern "system" fn get_adapter(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a `Part` of a live device.
    let d = unsafe { &*((*(this as *const Part)).owner as *const Device) };
    let mut a = d.adapter.load(Ordering::Acquire);
    if a.is_null() {
        let mut made: *mut c_void = core::ptr::null_mut();
        // SAFETY: our output.
        let hr = unsafe { first_adapter(&mut made) };
        if hr != S_OK {
            return hr;
        }
        match d.adapter.compare_exchange(core::ptr::null_mut(), made, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => a = made,
            Err(other) => {
                // SAFETY: the adapter just made, not kept.
                unsafe { com_release(made) };
                a = other;
            }
        }
    }
    // SAFETY: a live adapter the device holds; the caller's output.
    unsafe {
        com_add_ref(a);
        *out = a;
    }
    S_OK
}

/// Adapter 0 of a new factory: dxgi's software adapter.
unsafe fn first_adapter(out: *mut *mut c_void) -> HRESULT {
    let mut factory: *mut c_void = core::ptr::null_mut();
    // SAFETY: dxgi's export; our output.
    let hr = unsafe { CreateDXGIFactory1(&IID_IDXGIFACTORY1 as *const Guid as *const c_void, &mut factory) };
    if hr != S_OK {
        return hr;
    }
    type EnumAdapters = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT;
    // SAFETY: IDXGIFactory's vtable: EnumAdapters is entry 7, Release entry 2.
    unsafe {
        let vt = *(factory as *const *const usize);
        let enum_adapters: EnumAdapters = core::mem::transmute(*vt.add(7));
        let hr = enum_adapters(factory, 0, out);
        let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        rel(factory);
        hr
    }
}

/// `GetParent` of the DXGI device: its adapter's interface `riid`.
unsafe extern "system" fn dxgi_get_parent(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    let mut adapter: *mut c_void = core::ptr::null_mut();
    // SAFETY: as GetAdapter (`this` is the DXGI part).
    let hr = unsafe { get_adapter(this, &mut adapter) };
    if hr != S_OK {
        return hr;
    }
    // SAFETY: a live adapter of dxgi.dll: QueryInterface is entry 0, Release entry 2.
    unsafe {
        let vt = *(adapter as *const *const usize);
        let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT =
            core::mem::transmute(*vt);
        let hr = qi(adapter, riid, out);
        let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        rel(adapter);
        hr
    }
}

/// `QueryResourceResidency`: every resource is in memory (`DXGI_RESIDENCY_FULLY_RESIDENT`).
unsafe extern "system" fn query_resource_residency(
    _: *mut c_void,
    _: *const *mut c_void,
    out: *mut u32,
    n: u32,
) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    for i in 0..n as usize {
        // SAFETY: the caller's array of `n` statuses.
        unsafe { *out.add(i) = 1 };
    }
    S_OK
}

/// `SetGPUThreadPriority`: -7 .. 7 (IDXGIDevice.SetGPUThreadPriority.yaml).
unsafe extern "system" fn set_gpu_thread_priority(this: *mut c_void, priority: i32) -> HRESULT {
    if !(-7..=7).contains(&priority) {
        return E_INVALIDARG;
    }
    // SAFETY: a part of a live device.
    unsafe { device_of(this) }.gpu_priority.store(priority, Ordering::Relaxed);
    S_OK
}

unsafe extern "system" fn get_gpu_thread_priority(this: *mut c_void, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a part of a live device; the caller's output.
    unsafe { *out = device_of(this).gpu_priority.load(Ordering::Relaxed) };
    S_OK
}

/// `SetMaximumFrameLatency`: 0 .. 16 (IDXGIDevice1.SetMaximumFrameLatency.yaml).
unsafe extern "system" fn set_maximum_frame_latency(this: *mut c_void, n: u32) -> HRESULT {
    if n > 16 {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: a part of a live device.
    unsafe { device_of(this) }.frame_latency.store(n, Ordering::Relaxed);
    S_OK
}

/// `GetMaximumFrameLatency`: 3 until set (Learn).
unsafe extern "system" fn get_maximum_frame_latency(this: *mut c_void, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a part of a live device; the caller's output.
    unsafe { *out = device_of(this).frame_latency.load(Ordering::Relaxed) };
    S_OK
}

// ----- ID3D11Multithread

static MULTITHREAD_VTBL: Vt<7> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    nothing as *const c_void,
    nothing as *const c_void,
    set_multithread_protected as *const c_void,
    get_multithread_protected as *const c_void,
]);

/// `SetMultithreadProtected`: the old setting. Every operation of this device is a memory copy
/// that does not need the lock (`Enter` and `Leave` do nothing).
unsafe extern "system" fn set_multithread_protected(this: *mut c_void, on: i32) -> i32 {
    // SAFETY: a `Part` of a live device.
    let d = unsafe { &*((*(this as *const Part)).owner as *const Device) };
    i32::from(d.protected.swap(on != 0, Ordering::Relaxed))
}

unsafe extern "system" fn get_multithread_protected(this: *mut c_void) -> i32 {
    // SAFETY: a `Part` of a live device.
    let d = unsafe { &*((*(this as *const Part)).owner as *const Device) };
    i32::from(d.protected.load(Ordering::Relaxed))
}
