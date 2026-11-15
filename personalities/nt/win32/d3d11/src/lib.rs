//! Our `d3d11.dll` (ADR 0155, S3 hunt): a software Direct3D 11 device. Chrome's software compositor
//! (`--disable-gpu`) draws each frame with its own rasteriser, copies it into a Direct3D 11 texture
//! and presents it with a DXGI swap chain into its "Intermediate D3D Window"; on Windows that is
//! the basic render driver's device. Poly's device keeps every 2D texture in memory, implements the
//! copies, maps and clears (Learn: `ID3D11DeviceContext::Map`, `CopyResource`,
//! `CopySubresourceRegion`, `UpdateSubresource`, `ClearRenderTargetView`), and its swap chains
//! present the back buffer into the window through GDI (`SetDIBitsToDevice`). Drawing with shaders
//! is not implemented: those methods log their name once (`stub: d3d11: ...`) and fail.
//!
//! `dxgi.dll` makes swap chains for this device through the private export
//! `PolyD3D11CreateSwapChain`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

pub mod com;
mod device;
mod swap;
mod texture;

use com::*;
use core::ffi::c_void;
use poly_win32_rt as _;

poly_win32_rt::process_heap_allocator!();

/// `D3D_FEATURE_LEVEL_11_0` (d3dcommon.h): what the device offers.
pub const FEATURE_LEVEL: u32 = 0xb000;
/// `D3D11_SDK_VERSION` (d3d11.h).
const SDK_VERSION: u32 = 7;

/// `D3D11CreateDevice(pAdapter, DriverType, Software, Flags, pFeatureLevels, FeatureLevels,
/// SDKVersion, ppDevice, pFeatureLevel, ppImmediateContext)`: a device of the first level asked
/// for that is 11_0 or lower (all of them when none is asked for).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn D3D11CreateDevice(
    _adapter: *mut c_void,
    _driver_type: u32,
    _software: usize,
    flags: u32,
    levels: *const u32,
    nlevels: u32,
    sdk: u32,
    out_device: *mut *mut c_void,
    out_level: *mut u32,
    out_context: *mut *mut c_void,
) -> HRESULT {
    if sdk != SDK_VERSION {
        return E_INVALIDARG;
    }
    let level = if levels.is_null() || nlevels == 0 {
        FEATURE_LEVEL
    } else {
        // SAFETY: the caller's array of `nlevels` levels.
        let asked = unsafe { core::slice::from_raw_parts(levels, nlevels as usize) };
        match asked.iter().copied().find(|&l| l <= FEATURE_LEVEL) {
            Some(l) => l,
            None => return DXGI_ERROR_UNSUPPORTED,
        }
    };
    // SAFETY: the caller's optional outputs.
    unsafe { put(out_level, level) };
    if out_device.is_null() && out_context.is_null() {
        // Learn: only the feature level was asked for.
        return S_FALSE;
    }
    let dev = device::new(flags, level);
    // SAFETY: a new device; the caller's optional outputs.
    unsafe {
        if !out_context.is_null() {
            *out_context = device::immediate_context(dev);
        }
        if !out_device.is_null() {
            *out_device = dev;
        } else {
            release(dev);
        }
    }
    S_OK
}

/// `D3D11CreateDeviceAndSwapChain`: the device, then a swap chain from a `DXGI_SWAP_CHAIN_DESC`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn D3D11CreateDeviceAndSwapChain(
    adapter: *mut c_void,
    driver_type: u32,
    software: usize,
    flags: u32,
    levels: *const u32,
    nlevels: u32,
    sdk: u32,
    desc: *const u8,
    out_swap: *mut *mut c_void,
    out_device: *mut *mut c_void,
    out_level: *mut u32,
    out_context: *mut *mut c_void,
) -> HRESULT {
    let mut dev = core::ptr::null_mut();
    // SAFETY: the caller's arguments, passed on.
    let hr = unsafe {
        D3D11CreateDevice(adapter, driver_type, software, flags, levels, nlevels, sdk, &mut dev, out_level, out_context)
    };
    if hr != S_OK {
        return hr;
    }
    if !out_swap.is_null() {
        // SAFETY: a new device; the caller's description.
        let hr = unsafe { swap::from_legacy_desc(dev, desc, out_swap) };
        if hr != S_OK {
            // SAFETY: the reference made above.
            unsafe { release(dev) };
            return hr;
        }
    }
    if out_device.is_null() {
        // SAFETY: the reference made above.
        unsafe { release(dev) };
    } else {
        // SAFETY: the caller's output.
        unsafe { *out_device = dev };
    }
    S_OK
}

/// For `dxgi.dll`: a swap chain of `device` (one of this DLL's) presenting into `hwnd`, as
/// `IDXGIFactory2::CreateSwapChainForHwnd` with a `DXGI_SWAP_CHAIN_DESC1` (44 bytes) describes it.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyD3D11CreateSwapChain(
    device: *mut c_void,
    hwnd: usize,
    desc1: *const u8,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's (dxgi.dll's) arguments.
    let hr = unsafe { swap::create(device, hwnd, desc1, out) };
    log(&alloc::format!("d3d11: swap chain for window {hwnd:#x}: {hr:#x}"));
    hr
}

/// For `dcomp.dll`: bind `swap` (when it is one of this DLL's swap chains) to window `hwnd`, where
/// its `Present` draws from then on; nonzero when it is one.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyD3D11BindSwapChain(swap: *mut c_void, hwnd: usize) -> i32 {
    // SAFETY: the caller's object.
    i32::from(unsafe { swap::bind(swap, hwnd) })
}

/// For `dxgi.dll`: the same for `IDXGIFactory::CreateSwapChain` with a `DXGI_SWAP_CHAIN_DESC`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PolyD3D11CreateSwapChainLegacy(
    device: *mut c_void,
    desc: *const u8,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's (dxgi.dll's) arguments.
    unsafe { swap::from_legacy_desc(device, desc, out) }
}

poly_win32_rt::fail_exports!(
    "d3d11",
    hresult,
    [
        CreateDirect3D11DeviceFromDXGIDevice,
        D3D11CoreCreateDevice,
        D3D11CoreCreateLayeredDevice,
        D3D11CoreGetLayeredDeviceSize,
        D3D11CoreRegisterLayers,
        D3D11On12CreateDevice,
        D3DKMTCheckVidPnExclusiveOwnership,
        D3DKMTCloseAdapter,
        D3DKMTCreateAllocation,
        D3DKMTCreateContext,
        D3DKMTCreateDevice,
        D3DKMTCreateSynchronizationObject,
        D3DKMTDestroyAllocation,
        D3DKMTDestroyContext,
        D3DKMTDestroyDevice,
        D3DKMTDestroySynchronizationObject,
        D3DKMTEscape,
        D3DKMTGetContextSchedulingPriority,
        D3DKMTGetDeviceState,
        D3DKMTGetDisplayModeList,
        D3DKMTGetMultisampleMethodList,
        D3DKMTGetRuntimeData,
        D3DKMTGetSharedPrimaryHandle,
        D3DKMTLock,
        D3DKMTOpenAdapterFromGdiDisplayName,
        D3DKMTOpenAdapterFromHdc,
        D3DKMTOpenResource,
        D3DKMTPresent,
        D3DKMTQueryAdapterInfo,
        D3DKMTQueryAllocationResidency,
        D3DKMTQueryResourceInfo,
        D3DKMTRender,
        D3DKMTSetAllocationPriority,
        D3DKMTSetContextSchedulingPriority,
        D3DKMTSetDisplayMode,
        D3DKMTSetDisplayPrivateDriverFormat,
        D3DKMTSetGammaRamp,
        D3DKMTSetVidPnSourceOwner,
        D3DKMTSignalSynchronizationObject,
        D3DKMTUnlock,
        D3DKMTWaitForSynchronizationObject,
        D3DKMTWaitForVerticalBlankEvent,
        OpenAdapter10,
        OpenAdapter10_2,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn d3d11_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
