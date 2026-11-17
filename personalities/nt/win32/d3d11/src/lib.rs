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
mod context;
mod device;
mod objects;
mod swap;
mod texture;

use com::*;
use core::ffi::c_void;
use poly_d3d::create;
use poly_win32_rt as _;

poly_win32_rt::process_heap_allocator!();

/// `D3D11CreateDevice(pAdapter, DriverType, Software, Flags, pFeatureLevels, FeatureLevels,
/// SDKVersion, ppDevice, pFeatureLevel, ppImmediateContext)`: a device of the first level asked
/// for that is 11_0 or lower (all of them when none is asked for).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn D3D11CreateDevice(
    adapter: *mut c_void,
    driver_type: u32,
    software: usize,
    flags: u32,
    levels: *const u32,
    nlevels: u32,
    sdk: u32,
    out_device: *mut *mut c_void,
    out_level: *mut u32,
    out_context: *mut *mut c_void,
) -> HRESULT {
    let list = if levels.is_null() {
        None
    } else {
        // SAFETY: the caller's array of `nlevels` levels.
        Some(unsafe { core::slice::from_raw_parts(levels, nlevels as usize) })
    };
    let args = create::Args {
        driver: driver_type,
        adapter: !adapter.is_null(),
        software: software != 0,
        flags,
        levels: list,
        count: nlevels,
        sdk,
    };
    let level = match create::level(&args) {
        Ok(l) => l,
        Err(hr) => {
            // SAFETY: the caller's optional outputs: NULL and level 0 on failure.
            unsafe {
                put(out_device, core::ptr::null_mut());
                put(out_context, core::ptr::null_mut());
                put(out_level, 0);
            }
            return hr;
        }
    };
    // SAFETY: the caller's optional outputs.
    unsafe { put(out_level, level) };
    if out_device.is_null() && out_context.is_null() {
        // Learn: only the feature level was asked for.
        return S_FALSE;
    }
    let dev =
        device::new(flags, level, if driver_type == create::driver::UNKNOWN { adapter } else { core::ptr::null_mut() });
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

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
}

/// The entry point (`DllMain`): on process attach, one line in the kernel log saying which kind
/// of process loaded the DLL (the `--type=` switch of its command line, as programs with several
/// process kinds pass it), so a run's log shows how many processes use the device.
#[unsafe(no_mangle)]
pub extern "system" fn d3d11_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    if reason == 1 {
        // SAFETY: the process's command line, NUL terminated.
        let line = unsafe { GetCommandLineW() };
        let mut text = alloc::string::String::new();
        let mut i = 0;
        // SAFETY: reading up to the terminating NUL.
        while !line.is_null() && unsafe { *line.add(i) } != 0 && i < 4096 {
            // SAFETY: as above.
            text.push(char::from_u32(unsafe { *line.add(i) } as u32).unwrap_or('?'));
            i += 1;
        }
        let kind = text.split(' ').find_map(|w| w.strip_prefix("--type=")).unwrap_or("main");
        // The names of the switches (no values) tell the kinds of GPU process apart.
        let switches: alloc::vec::Vec<&str> = text
            .split(' ')
            .filter_map(|w| w.strip_prefix("--"))
            .map(|w| w.split('=').next().unwrap_or(""))
            .filter(|w| !w.is_empty() && *w != "type")
            .collect();
        let mut names = switches.join(",");
        names.truncate(400);
        log(&alloc::format!("d3d11: loaded (process type {kind}) switches {names}"));
    }
    1
}
