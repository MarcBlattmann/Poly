//! The device (`ID3D11Device`), its immediate context (`ID3D11DeviceContext`), and the device's
//! `IDXGIDevice1` and `ID3D11Multithread` interfaces. The context and the other interfaces share
//! the device's reference count (they are parts of one object).

use crate::com::*;
use crate::stubs;
use crate::texture::{self, Part, Tex, part_add_ref, part_query_interface, part_release};
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
pub struct Device {
    obj: Obj,
    context: Part,
    dxgi: Part,
    multithread: Part,
    flags: u32,
    level: u32,
    protected: AtomicBool,
}

static DEVICE_IIDS: [Guid; 1] = [IID_ID3D11DEVICE];

pub fn new(flags: u32, level: u32) -> *mut c_void {
    let mut obj = Obj::new(DEVICE_VTBL.ptr(), &DEVICE_IIDS, drop_box::<Device>, "ID3D11Device");
    obj.more = Some(device_more);
    let p = into_raw(Device {
        obj,
        context: Part { vtbl: CONTEXT_VTBL.ptr(), owner: core::ptr::null_mut() },
        dxgi: Part { vtbl: DXGI_DEVICE_VTBL.ptr(), owner: core::ptr::null_mut() },
        multithread: Part { vtbl: MULTITHREAD_VTBL.ptr(), owner: core::ptr::null_mut() },
        flags,
        level,
        protected: AtomicBool::new(false),
    });
    // SAFETY: the new device.
    let d = unsafe { &mut *(p as *mut Device) };
    d.context.owner = p;
    d.dxgi.owner = p;
    d.multithread.owner = p;
    p
}

/// The immediate context of a device, with a reference.
pub unsafe fn immediate_context(dev: *mut c_void) -> *mut c_void {
    // SAFETY: a live device.
    unsafe {
        add_ref(dev);
        &mut (*(dev as *mut Device)).context as *mut Part as *mut c_void
    }
}

unsafe fn device_more(p: *mut c_void, iid: &Guid) -> *mut c_void {
    // SAFETY: a live device.
    let d = unsafe { &mut *(p as *mut Device) };
    let part = if *iid == IID_IDXGIDEVICE || *iid == IID_IDXGIDEVICE1 || *iid == IID_IDXGIOBJECT {
        &mut d.dxgi as *mut Part
    } else if *iid == IID_ID3D11MULTITHREAD {
        &mut d.multithread as *mut Part
    } else {
        return core::ptr::null_mut();
    };
    // SAFETY: the device is live.
    unsafe { add_ref(p) };
    part as *mut c_void
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
        CreateBuffer,
        CreateTexture1D,
        CreateTexture3D,
        CreateShaderResourceView,
        CreateUnorderedAccessView,
        CreateDepthStencilView,
        CreateInputLayout,
        CreateVertexShader,
        CreateGeometryShader,
        CreateGeometryShaderWithStreamOutput,
        CreatePixelShader,
        CreateHullShader,
        CreateDomainShader,
        CreateComputeShader,
        CreateClassLinkage,
        CreateBlendState,
        CreateDepthStencilState,
        CreateRasterizerState,
        CreateSamplerState,
        CreateQuery,
        CreatePredicate,
        CreateCounter,
        CreateDeferredContext,
        OpenSharedResource,
        CheckCounterInfo,
        CheckCounter,
    ]
);

static DEVICE_VTBL: Vt<43> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    dev_stubs::CreateBuffer as *const c_void,
    dev_stubs::CreateTexture1D as *const c_void,
    create_texture_2d as *const c_void,
    dev_stubs::CreateTexture3D as *const c_void,
    dev_stubs::CreateShaderResourceView as *const c_void,
    dev_stubs::CreateUnorderedAccessView as *const c_void,
    create_render_target_view as *const c_void,
    dev_stubs::CreateDepthStencilView as *const c_void,
    dev_stubs::CreateInputLayout as *const c_void,
    dev_stubs::CreateVertexShader as *const c_void,
    dev_stubs::CreateGeometryShader as *const c_void,
    dev_stubs::CreateGeometryShaderWithStreamOutput as *const c_void,
    dev_stubs::CreatePixelShader as *const c_void,
    dev_stubs::CreateHullShader as *const c_void,
    dev_stubs::CreateDomainShader as *const c_void,
    dev_stubs::CreateComputeShader as *const c_void,
    dev_stubs::CreateClassLinkage as *const c_void,
    dev_stubs::CreateBlendState as *const c_void,
    dev_stubs::CreateDepthStencilState as *const c_void,
    dev_stubs::CreateRasterizerState as *const c_void,
    dev_stubs::CreateSamplerState as *const c_void,
    dev_stubs::CreateQuery as *const c_void,
    dev_stubs::CreatePredicate as *const c_void,
    dev_stubs::CreateCounter as *const c_void,
    dev_stubs::CreateDeferredContext as *const c_void,
    dev_stubs::OpenSharedResource as *const c_void,
    check_format_support as *const c_void,
    check_multisample_quality_levels as *const c_void,
    dev_stubs::CheckCounterInfo as *const c_void,
    dev_stubs::CheckCounter as *const c_void,
    check_feature_support as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_feature_level as *const c_void,
    get_creation_flags as *const c_void,
    get_device_removed_reason as *const c_void,
    get_immediate_context as *const c_void,
    set_exception_mode as *const c_void,
    get_exception_mode as *const c_void,
]);

/// `CreateTexture2D(pDesc, pInitialData, ppTexture2D)`: one mip level of one slice; a null
/// output only checks the description (`S_FALSE`, Learn).
unsafe extern "system" fn create_texture_2d(
    this: *mut c_void,
    desc: *const [u32; 11],
    init: *const u8,
    out: *mut *mut c_void,
) -> HRESULT {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's D3D11_TEXTURE2D_DESC.
    let mut d = unsafe { *desc };
    if d[0] == 0 || d[1] == 0 || d[0] > 16384 || d[1] > 16384 {
        return E_INVALIDARG;
    }
    if d[2] > 1 || d[3] > 1 {
        log("stub: d3d11: CreateTexture2D: only the first mip level of the first slice is kept");
    }
    d[2] = d[2].max(1);
    d[3] = d[3].max(1);
    if out.is_null() {
        return S_FALSE;
    }
    let init = if init.is_null() {
        None
    } else {
        // SAFETY: the caller's D3D11_SUBRESOURCE_DATA (pSysMem, SysMemPitch, SysMemSlicePitch).
        unsafe { Some((*(init as *const *const u8), *(init.add(8) as *const u32) as usize)) }
    };
    // SAFETY: a live device; the caller's output.
    unsafe { *out = texture::create(this, d, init) };
    S_OK
}

/// `CreateRenderTargetView(pResource, pDesc, ppRTView)`: a view of one of our 2D textures.
unsafe extern "system" fn create_render_target_view(
    _: *mut c_void,
    res: *mut c_void,
    desc: *const [u32; 5],
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's resource.
    if unsafe { texture::tex(res) }.is_none() {
        return E_INVALIDARG;
    }
    if out.is_null() {
        return S_FALSE;
    }
    // SAFETY: the caller's optional description.
    let desc = if desc.is_null() { None } else { Some(unsafe { *desc }) };
    // SAFETY: a live texture; the caller's output.
    unsafe { *out = texture::new_rtv(res, desc) };
    S_OK
}

/// `CheckFormatSupport`: a 2D texture, render target, shader sample and display format for the
/// formats a texture can hold (`D3D11_FORMAT_SUPPORT_*`).
unsafe extern "system" fn check_format_support(_: *mut c_void, format: u32, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // TEXTURE2D 0x20, SHADER_SAMPLE 0x200, RENDER_TARGET 0x4000, BLENDABLE 0x8000,
    // CPU_LOCKABLE 0x20000, DISPLAY 0x80000.
    let support = if format == 0 { 0 } else { 0x20 | 0x200 | 0x4000 | 0x8000 | 0x20000 | 0x80000 };
    // SAFETY: the caller's output.
    unsafe { *out = support };
    if support == 0 { E_INVALIDARG } else { S_OK }
}

/// `CheckMultisampleQualityLevels`: one quality level without multisampling, none with it.
unsafe extern "system" fn check_multisample_quality_levels(_: *mut c_void, _: u32, count: u32, out: *mut u32) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { put(out, u32::from(count == 1)) };
    S_OK
}

/// `CheckFeatureSupport(Feature, pFeatureSupportData, FeatureSupportDataSize)`: no optional
/// feature is supported (the structure filled with zeros).
unsafe extern "system" fn check_feature_support(_: *mut c_void, _: u32, data: *mut u8, size: u32) -> HRESULT {
    if data.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's structure of `size` bytes.
    unsafe { core::ptr::write_bytes(data, 0, size as usize) };
    S_OK
}

unsafe extern "system" fn get_feature_level(this: *mut c_void) -> u32 {
    // SAFETY: a live device.
    unsafe { (*(this as *mut Device)).level }
}

unsafe extern "system" fn get_creation_flags(this: *mut c_void) -> u32 {
    // SAFETY: a live device.
    unsafe { (*(this as *mut Device)).flags }
}

unsafe extern "system" fn get_device_removed_reason(_: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn get_immediate_context(this: *mut c_void, out: *mut *mut c_void) {
    if !out.is_null() {
        // SAFETY: a live device; the caller's output.
        unsafe { *out = immediate_context(this) };
    }
}

unsafe extern "system" fn set_exception_mode(_: *mut c_void, _: u32) -> HRESULT {
    S_OK
}

unsafe extern "system" fn get_exception_mode(_: *mut c_void) -> u32 {
    0
}

// ----- ID3D11DeviceContext

stubs!(
    ctx_stubs,
    "ID3D11DeviceContext",
    [
        VSSetConstantBuffers,
        PSSetShaderResources,
        PSSetShader,
        PSSetSamplers,
        VSSetShader,
        DrawIndexed,
        Draw,
        PSSetConstantBuffers,
        IASetInputLayout,
        IASetVertexBuffers,
        IASetIndexBuffer,
        DrawIndexedInstanced,
        DrawInstanced,
        GSSetConstantBuffers,
        GSSetShader,
        IASetPrimitiveTopology,
        VSSetShaderResources,
        VSSetSamplers,
        Begin,
        End,
        GetData,
        SetPredication,
        GSSetShaderResources,
        GSSetSamplers,
        OMSetRenderTargetsAndUnorderedAccessViews,
        OMSetBlendState,
        OMSetDepthStencilState,
        SOSetTargets,
        DrawAuto,
        DrawIndexedInstancedIndirect,
        DrawInstancedIndirect,
        Dispatch,
        DispatchIndirect,
        RSSetState,
        CopyStructureCount,
        ClearUnorderedAccessViewUint,
        ClearUnorderedAccessViewFloat,
        ClearDepthStencilView,
        GenerateMips,
        SetResourceMinLOD,
        GetResourceMinLOD,
        ResolveSubresource,
        ExecuteCommandList,
        HSSetShaderResources,
        HSSetShader,
        HSSetSamplers,
        HSSetConstantBuffers,
        DSSetShaderResources,
        DSSetShader,
        DSSetSamplers,
        DSSetConstantBuffers,
        CSSetShaderResources,
        CSSetUnorderedAccessViews,
        CSSetShader,
        CSSetSamplers,
        CSSetConstantBuffers,
        VSGetConstantBuffers,
        PSGetShaderResources,
        PSGetShader,
        PSGetSamplers,
        VSGetShader,
        PSGetConstantBuffers,
        IAGetInputLayout,
        IAGetVertexBuffers,
        IAGetIndexBuffer,
        GSGetConstantBuffers,
        GSGetShader,
        IAGetPrimitiveTopology,
        VSGetShaderResources,
        VSGetSamplers,
        GetPredication,
        GSGetShaderResources,
        GSGetSamplers,
        OMGetRenderTargets,
        OMGetRenderTargetsAndUnorderedAccessViews,
        OMGetBlendState,
        OMGetDepthStencilState,
        SOGetTargets,
        RSGetState,
        RSGetViewports,
        RSGetScissorRects,
        HSGetShaderResources,
        HSGetShader,
        HSGetSamplers,
        HSGetConstantBuffers,
        DSGetShaderResources,
        DSGetShader,
        DSGetSamplers,
        DSGetConstantBuffers,
        CSGetShaderResources,
        CSGetUnorderedAccessViews,
        CSGetShader,
        CSGetSamplers,
        CSGetConstantBuffers,
        FinishCommandList,
    ]
);

static CONTEXT_VTBL: Vt<115> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    ctx_get_device as *const c_void,
    get_private_data as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    ctx_stubs::VSSetConstantBuffers as *const c_void,
    ctx_stubs::PSSetShaderResources as *const c_void,
    ctx_stubs::PSSetShader as *const c_void,
    ctx_stubs::PSSetSamplers as *const c_void,
    ctx_stubs::VSSetShader as *const c_void,
    ctx_stubs::DrawIndexed as *const c_void,
    ctx_stubs::Draw as *const c_void,
    map as *const c_void,
    unmap as *const c_void,
    ctx_stubs::PSSetConstantBuffers as *const c_void,
    ctx_stubs::IASetInputLayout as *const c_void,
    ctx_stubs::IASetVertexBuffers as *const c_void,
    ctx_stubs::IASetIndexBuffer as *const c_void,
    ctx_stubs::DrawIndexedInstanced as *const c_void,
    ctx_stubs::DrawInstanced as *const c_void,
    ctx_stubs::GSSetConstantBuffers as *const c_void,
    ctx_stubs::GSSetShader as *const c_void,
    ctx_stubs::IASetPrimitiveTopology as *const c_void,
    ctx_stubs::VSSetShaderResources as *const c_void,
    ctx_stubs::VSSetSamplers as *const c_void,
    ctx_stubs::Begin as *const c_void,
    ctx_stubs::End as *const c_void,
    ctx_stubs::GetData as *const c_void,
    ctx_stubs::SetPredication as *const c_void,
    ctx_stubs::GSSetShaderResources as *const c_void,
    ctx_stubs::GSSetSamplers as *const c_void,
    om_set_render_targets as *const c_void,
    ctx_stubs::OMSetRenderTargetsAndUnorderedAccessViews as *const c_void,
    ctx_stubs::OMSetBlendState as *const c_void,
    ctx_stubs::OMSetDepthStencilState as *const c_void,
    ctx_stubs::SOSetTargets as *const c_void,
    ctx_stubs::DrawAuto as *const c_void,
    ctx_stubs::DrawIndexedInstancedIndirect as *const c_void,
    ctx_stubs::DrawInstancedIndirect as *const c_void,
    ctx_stubs::Dispatch as *const c_void,
    ctx_stubs::DispatchIndirect as *const c_void,
    ctx_stubs::RSSetState as *const c_void,
    nothing3 as *const c_void,
    nothing3 as *const c_void,
    copy_subresource_region as *const c_void,
    copy_resource as *const c_void,
    update_subresource as *const c_void,
    ctx_stubs::CopyStructureCount as *const c_void,
    clear_render_target_view as *const c_void,
    ctx_stubs::ClearUnorderedAccessViewUint as *const c_void,
    ctx_stubs::ClearUnorderedAccessViewFloat as *const c_void,
    ctx_stubs::ClearDepthStencilView as *const c_void,
    ctx_stubs::GenerateMips as *const c_void,
    ctx_stubs::SetResourceMinLOD as *const c_void,
    ctx_stubs::GetResourceMinLOD as *const c_void,
    ctx_stubs::ResolveSubresource as *const c_void,
    ctx_stubs::ExecuteCommandList as *const c_void,
    ctx_stubs::HSSetShaderResources as *const c_void,
    ctx_stubs::HSSetShader as *const c_void,
    ctx_stubs::HSSetSamplers as *const c_void,
    ctx_stubs::HSSetConstantBuffers as *const c_void,
    ctx_stubs::DSSetShaderResources as *const c_void,
    ctx_stubs::DSSetShader as *const c_void,
    ctx_stubs::DSSetSamplers as *const c_void,
    ctx_stubs::DSSetConstantBuffers as *const c_void,
    ctx_stubs::CSSetShaderResources as *const c_void,
    ctx_stubs::CSSetUnorderedAccessViews as *const c_void,
    ctx_stubs::CSSetShader as *const c_void,
    ctx_stubs::CSSetSamplers as *const c_void,
    ctx_stubs::CSSetConstantBuffers as *const c_void,
    ctx_stubs::VSGetConstantBuffers as *const c_void,
    ctx_stubs::PSGetShaderResources as *const c_void,
    ctx_stubs::PSGetShader as *const c_void,
    ctx_stubs::PSGetSamplers as *const c_void,
    ctx_stubs::VSGetShader as *const c_void,
    ctx_stubs::PSGetConstantBuffers as *const c_void,
    ctx_stubs::IAGetInputLayout as *const c_void,
    ctx_stubs::IAGetVertexBuffers as *const c_void,
    ctx_stubs::IAGetIndexBuffer as *const c_void,
    ctx_stubs::GSGetConstantBuffers as *const c_void,
    ctx_stubs::GSGetShader as *const c_void,
    ctx_stubs::IAGetPrimitiveTopology as *const c_void,
    ctx_stubs::VSGetShaderResources as *const c_void,
    ctx_stubs::VSGetSamplers as *const c_void,
    ctx_stubs::GetPredication as *const c_void,
    ctx_stubs::GSGetShaderResources as *const c_void,
    ctx_stubs::GSGetSamplers as *const c_void,
    ctx_stubs::OMGetRenderTargets as *const c_void,
    ctx_stubs::OMGetRenderTargetsAndUnorderedAccessViews as *const c_void,
    ctx_stubs::OMGetBlendState as *const c_void,
    ctx_stubs::OMGetDepthStencilState as *const c_void,
    ctx_stubs::SOGetTargets as *const c_void,
    ctx_stubs::RSGetState as *const c_void,
    ctx_stubs::RSGetViewports as *const c_void,
    ctx_stubs::RSGetScissorRects as *const c_void,
    ctx_stubs::HSGetShaderResources as *const c_void,
    ctx_stubs::HSGetShader as *const c_void,
    ctx_stubs::HSGetSamplers as *const c_void,
    ctx_stubs::HSGetConstantBuffers as *const c_void,
    ctx_stubs::DSGetShaderResources as *const c_void,
    ctx_stubs::DSGetShader as *const c_void,
    ctx_stubs::DSGetSamplers as *const c_void,
    ctx_stubs::DSGetConstantBuffers as *const c_void,
    ctx_stubs::CSGetShaderResources as *const c_void,
    ctx_stubs::CSGetUnorderedAccessViews as *const c_void,
    ctx_stubs::CSGetShader as *const c_void,
    ctx_stubs::CSGetSamplers as *const c_void,
    ctx_stubs::CSGetConstantBuffers as *const c_void,
    nothing as *const c_void,
    nothing as *const c_void,
    get_type as *const c_void,
    get_context_flags as *const c_void,
    ctx_stubs::FinishCommandList as *const c_void,
]);

/// `RSSetViewports`, `RSSetScissorRects`: nothing is rasterised.
unsafe extern "system" fn nothing3(_: *mut c_void, _: u32, _: *const c_void) {}

/// `ClearState`, `Flush`: the copies are done when they are made.
unsafe extern "system" fn nothing(_: *mut c_void) {}

/// `GetType`: `D3D11_DEVICE_CONTEXT_IMMEDIATE`.
unsafe extern "system" fn get_type(_: *mut c_void) -> u32 {
    0
}

unsafe extern "system" fn get_context_flags(_: *mut c_void) -> u32 {
    0
}

unsafe extern "system" fn ctx_get_device(this: *mut c_void, out: *mut *mut c_void) {
    // SAFETY: a `Part` of a live device; the caller's output.
    unsafe {
        let d = (*(this as *const Part)).owner;
        add_ref(d);
        put(out, d);
    }
}

/// `OMSetRenderTargets`: nothing is drawn through them.
unsafe extern "system" fn om_set_render_targets(_: *mut c_void, _: u32, _: *const *mut c_void, _: *mut c_void) {}

/// `Map(pResource, Subresource, MapType, MapFlags, pMappedResource)`: the texture's memory
/// (`D3D11_MAPPED_SUBRESOURCE`: pData, RowPitch, DepthPitch).
unsafe extern "system" fn map(_: *mut c_void, res: *mut c_void, sub: u32, _: u32, _: u32, out: *mut u8) -> HRESULT {
    // SAFETY: the caller's resource.
    let Some(t) = (unsafe { texture::tex(res) }) else {
        log("stub: d3d11: Map of a resource that is not a 2D texture");
        return E_INVALIDARG;
    };
    if sub != 0 || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's D3D11_MAPPED_SUBRESOURCE (16 bytes).
    unsafe {
        (out as *mut *mut u8).write(t.data.as_mut_ptr());
        (out.add(8) as *mut u32).write(t.pitch as u32);
        (out.add(12) as *mut u32).write((t.pitch * t.height()) as u32);
    }
    S_OK
}

unsafe extern "system" fn unmap(_: *mut c_void, _: *mut c_void, _: u32) {}

/// `CopyResource(pDstResource, pSrcResource)`.
unsafe extern "system" fn copy_resource(this: *mut c_void, dst: *mut c_void, src: *mut c_void) {
    // SAFETY: the caller's resources, passed on.
    unsafe { copy_subresource_region(this, dst, 0, 0, 0, 0, src, 0, core::ptr::null()) }
}

/// `CopySubresourceRegion(pDstResource, DstSubresource, DstX, DstY, DstZ, pSrcResource,
/// SrcSubresource, pSrcBox)`: `D3D11_BOX` is left, top, front, right, bottom, back.
#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn copy_subresource_region(
    _: *mut c_void,
    dst: *mut c_void,
    _: u32,
    dx: u32,
    dy: u32,
    _: u32,
    src: *mut c_void,
    _: u32,
    bx: *const [u32; 6],
) {
    // SAFETY: the caller's resources.
    let (Some(s), Some(_)) = (unsafe { texture::tex(src) }, unsafe { texture::tex(dst) }) else {
        log("stub: d3d11: CopySubresourceRegion of a resource that is not a 2D texture");
        return;
    };
    let (sx, sy, w, h) = if bx.is_null() {
        (0, 0, s.width(), s.height())
    } else {
        // SAFETY: the caller's box.
        let b = unsafe { *bx };
        (b[0] as usize, b[1] as usize, b[3].saturating_sub(b[0]) as usize, b[4].saturating_sub(b[1]) as usize)
    };
    if src == dst {
        // One texture: copy from a snapshot.
        let snapshot = Tex::snapshot(s);
        // SAFETY: as above.
        let d = unsafe { texture::tex(dst) }.expect("checked");
        texture::copy_region(d, dx as usize, dy as usize, &snapshot, sx, sy, w, h);
    } else {
        // SAFETY: as above, distinct objects.
        let d = unsafe { texture::tex(dst) }.expect("checked");
        texture::copy_region(d, dx as usize, dy as usize, s, sx, sy, w, h);
    }
}

/// `UpdateSubresource(pDstResource, DstSubresource, pDstBox, pSrcData, SrcRowPitch,
/// SrcDepthPitch)`.
unsafe extern "system" fn update_subresource(
    _: *mut c_void,
    dst: *mut c_void,
    _: u32,
    bx: *const [u32; 6],
    data: *const u8,
    row_pitch: u32,
    _: u32,
) {
    // SAFETY: the caller's resource.
    let Some(d) = (unsafe { texture::tex(dst) }) else {
        log("stub: d3d11: UpdateSubresource of a resource that is not a 2D texture");
        return;
    };
    if data.is_null() {
        return;
    }
    let (x, y, w, h) = if bx.is_null() {
        (0, 0, d.width(), d.height())
    } else {
        // SAFETY: the caller's box.
        let b = unsafe { *bx };
        (b[0] as usize, b[1] as usize, b[3].saturating_sub(b[0]) as usize, b[4].saturating_sub(b[1]) as usize)
    };
    let bpp = texture::bytes_per_pixel(d.format());
    let w = w.min(d.width().saturating_sub(x));
    let h = h.min(d.height().saturating_sub(y));
    for r in 0..h {
        // SAFETY: the caller's data: `h` rows `row_pitch` apart, `w` pixels each.
        let row = unsafe { core::slice::from_raw_parts(data.add(r * row_pitch as usize), w * bpp) };
        let at = (y + r) * d.pitch + x * bpp;
        d.data[at..at + w * bpp].copy_from_slice(row);
    }
}

/// `ClearRenderTargetView(pRenderTargetView, ColorRGBA)`.
unsafe extern "system" fn clear_render_target_view(_: *mut c_void, rtv: *mut c_void, rgba: *const [f32; 4]) {
    // SAFETY: the caller's view.
    let Some(t) = (unsafe { texture::rtv_tex(rtv) }) else { return };
    if rgba.is_null() || texture::bytes_per_pixel(t.format()) != 4 {
        return;
    }
    // SAFETY: the caller's four floats.
    let c = unsafe { *rgba };
    let byte = |v: f32| (if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) } * 255.0 + 0.5) as u8;
    let (r, g, b, a) = (byte(c[0]), byte(c[1]), byte(c[2]), byte(c[3]));
    let px = if texture::red_first(t.format()) { [r, g, b, a] } else { [b, g, r, a] };
    for p in t.data.chunks_exact_mut(4) {
        p.copy_from_slice(&px);
    }
}

// ----- IDXGIDevice1

#[link(name = "dxgi", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateDXGIFactory1(riid: *const c_void, out: *mut *mut c_void) -> HRESULT;
}

/// `IID_IDXGIFactory1` (dxgi.h).
const IID_IDXGIFACTORY1: Guid = guid(0x770aae78, 0xf26f, 0x4dba, 0xa829, 0x253c83d1b387);

stubs!(dxgi_stubs, "IDXGIDevice", [CreateSurface]);

static DXGI_DEVICE_VTBL: Vt<14> = Vt([
    part_query_interface as *const c_void,
    part_add_ref as *const c_void,
    part_release as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_private_data as *const c_void,
    dxgi_get_parent as *const c_void,
    get_adapter as *const c_void,
    dxgi_stubs::CreateSurface as *const c_void,
    query_resource_residency as *const c_void,
    set_gpu_thread_priority as *const c_void,
    get_gpu_thread_priority as *const c_void,
    set_maximum_frame_latency as *const c_void,
    get_maximum_frame_latency as *const c_void,
]);

/// `GetAdapter`: dxgi's software adapter (adapter 0 of a factory).
unsafe extern "system" fn get_adapter(_: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
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
    // SAFETY: as GetAdapter.
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
unsafe extern "system" fn query_resource_residency(_: *mut c_void, _: *const *mut c_void, out: *mut u32, n: u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    for i in 0..n as usize {
        // SAFETY: the caller's array of `n` statuses.
        unsafe { *out.add(i) = 1 };
    }
    S_OK
}

unsafe extern "system" fn set_gpu_thread_priority(_: *mut c_void, priority: i32) -> HRESULT {
    if (-7..=7).contains(&priority) { S_OK } else { E_INVALIDARG }
}

unsafe extern "system" fn get_gpu_thread_priority(_: *mut c_void, out: *mut i32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output.
    unsafe { *out = 0 };
    S_OK
}

unsafe extern "system" fn set_maximum_frame_latency(_: *mut c_void, _: u32) -> HRESULT {
    S_OK
}

/// `GetMaximumFrameLatency`: the default, 3 (Learn).
unsafe extern "system" fn get_maximum_frame_latency(_: *mut c_void, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output.
    unsafe { *out = 3 };
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
