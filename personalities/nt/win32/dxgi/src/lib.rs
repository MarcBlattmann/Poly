//! Our `dxgi.dll` (ADR 0155, S3 hunt): what a program that draws in software asks of DXGI. Chrome's
//! software compositor (`--disable-gpu`) creates a factory and walks its adapters and their outputs
//! (Learn: `CreateDXGIFactory1`, `IDXGIFactory1::EnumAdapters1`, `IDXGIAdapter::EnumOutputs`,
//! `IDXGIOutput::GetDesc`); a factory that cannot be created ends the GPU process.
//!
//! Poly has no hardware adapter: the factory lists one software adapter (`DXGI_ADAPTER_FLAG_SOFTWARE`,
//! Learn: "Microsoft Basic Render Driver", vendor `0x1414`) whose one output is the screen. What would
//! draw through DXGI (swap chains, display modes, gamma) answers `DXGI_ERROR_UNSUPPORTED`. Layouts,
//! interface identifiers and error codes are those of the public `dxgi.h` / `winerror.h`.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

poly_win32_rt::process_heap_allocator!();

type HRESULT = i32;

// winerror.h
const S_OK: HRESULT = 0;
const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
const DXGI_ERROR_NOT_FOUND: HRESULT = 0x887A_0002u32 as i32;
const DXGI_ERROR_UNSUPPORTED: HRESULT = 0x887A_0004u32 as i32;
// dxgi.h
const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;
const DXGI_MODE_ROTATION_IDENTITY: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Guid {
    Guid { d1, d2, d3, d4 }
}

const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDXGIOBJECT: Guid = guid(0xaec22fb8, 0x76f3, 0x4639, [0x9b, 0xe0, 0x28, 0xeb, 0x43, 0xa6, 0x7a, 0x2e]);
const IID_IDXGIFACTORY: Guid = guid(0x7b7166ec, 0x21c7, 0x44ae, [0xb2, 0x1a, 0xc9, 0xae, 0x32, 0x1a, 0xe3, 0x69]);
const IID_IDXGIFACTORY1: Guid = guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
const IID_IDXGIFACTORY2: Guid = guid(0x50c83a1c, 0xe072, 0x4c48, [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0]);
const IID_IDXGIADAPTER: Guid = guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
const IID_IDXGIADAPTER1: Guid = guid(0x29038f61, 0x3839, 0x4626, [0x91, 0xfd, 0x08, 0x68, 0x79, 0x01, 0x1a, 0x05]);
const IID_IDXGIOUTPUT: Guid = guid(0xae02eedb, 0xc735, 0x4690, [0x8d, 0x52, 0x5a, 0x8d, 0xc2, 0x02, 0x13, 0xaa]);

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn MonitorFromPoint(x: i64, flags: u32) -> usize;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn Sleep(ms: u32);
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
}

/// `d3d11.dll`'s private export `name` (its swap chains present its textures): loaded when a
/// swap chain is first made, so the two DLLs do not import each other.
fn d3d11_export(name: &core::ffi::CStr) -> usize {
    let dll: [u16; 10] = [100, 51, 100, 49, 49, 46, 100, 108, 108, 0];
    // SAFETY: a NUL-terminated name.
    let module = unsafe { LoadLibraryW(dll.as_ptr()) };
    if module == 0 {
        return 0;
    }
    // SAFETY: a loaded module and a NUL-terminated name.
    unsafe { GetProcAddress(module, name.as_ptr() as *const u8) }
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

/// Log an interface an object was asked for and does not have (`stub: dxgi ...`, like the
/// lines of the fail-only exports), so a program that needs a newer one shows which.
fn log_missing(iid: &Guid) {
    let d4 = iid.d4;
    let text = alloc::format!(
        "stub: dxgi: no interface {{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        iid.d1,
        iid.d2,
        iid.d3,
        d4[0],
        d4[1],
        d4[2],
        d4[3],
        d4[4],
        d4[5],
        d4[6],
        d4[7]
    );
    // SAFETY: the service reads `text.len()` bytes.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

/// The start of every object: its vtable, reference count, the interfaces it answers and what it
/// was made by (`GetParent`; null for the factory).
#[repr(C)]
struct Obj {
    vtbl: *const c_void,
    refs: AtomicU32,
    iids: &'static [Guid],
    parent: *mut c_void,
}

fn new_obj(vtbl: *const c_void, iids: &'static [Guid], parent: *mut c_void) -> *mut c_void {
    if !parent.is_null() {
        // SAFETY: a live object of this DLL.
        unsafe { add_ref(parent) };
    }
    Box::into_raw(Box::new(Obj { vtbl, refs: AtomicU32::new(1), iids, parent })) as *mut c_void
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: every object of this DLL starts with an `Obj`.
    unsafe { &*(this as *const Obj) }.refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: every object of this DLL starts with an `Obj`.
    let o = unsafe { &*(this as *const Obj) };
    let n = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        let parent = o.parent;
        // SAFETY: the last reference is gone; the object was made by `new_obj`.
        drop(unsafe { Box::from_raw(this as *mut Obj) });
        if !parent.is_null() {
            // SAFETY: the reference `new_obj` took.
            unsafe { release(parent) };
        }
    }
    n
}

unsafe extern "system" fn query_interface(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output pointer.
    unsafe { *out = core::ptr::null_mut() };
    if riid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller passes a GUID; every object starts with an `Obj`.
    let (iid, o) = unsafe { (*riid, &*(this as *const Obj)) };
    if iid == IID_IUNKNOWN || iid == IID_IDXGIOBJECT || o.iids.contains(&iid) {
        // SAFETY: `this` is live, `out` writable.
        unsafe {
            add_ref(this);
            *out = this;
        }
        S_OK
    } else {
        log_missing(&iid);
        E_NOINTERFACE
    }
}

// ----- IDXGIObject

unsafe extern "system" fn set_private_data(_: *mut c_void, _: *const Guid, _: u32, _: *const c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn set_private_data_interface(_: *mut c_void, _: *const Guid, _: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn get_private_data(_: *mut c_void, _: *const Guid, size: *mut u32, _: *mut c_void) -> HRESULT {
    if !size.is_null() {
        // SAFETY: the caller's size.
        unsafe { *size = 0 };
    }
    DXGI_ERROR_NOT_FOUND
}

/// `GetParent`: the factory of an adapter, the adapter of an output.
unsafe extern "system" fn get_parent(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: every object starts with an `Obj`.
    let parent = unsafe { &*(this as *const Obj) }.parent;
    if parent.is_null() {
        if !out.is_null() {
            // SAFETY: the caller's output pointer.
            unsafe { *out = core::ptr::null_mut() };
        }
        return E_NOINTERFACE;
    }
    // SAFETY: a live object of this DLL.
    unsafe { query_interface(parent, riid, out) }
}

// ----- the factory

#[repr(C)]
struct FactoryVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_private_data: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void) -> HRESULT,
    set_private_data_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void) -> HRESULT,
    get_private_data: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut c_void) -> HRESULT,
    get_parent: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    enum_adapters: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    make_window_association: unsafe extern "system" fn(*mut c_void, usize, u32) -> HRESULT,
    get_window_association: unsafe extern "system" fn(*mut c_void, *mut usize) -> HRESULT,
    create_swap_chain: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    create_software_adapter: unsafe extern "system" fn(*mut c_void, usize, *mut *mut c_void) -> HRESULT,
    enum_adapters1: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    is_current: unsafe extern "system" fn(*mut c_void) -> i32,
    // IDXGIFactory2 (dxgi1_2.h)
    is_windowed_stereo_enabled: unsafe extern "system" fn(*mut c_void) -> i32,
    create_swap_chain_for_hwnd: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        usize,
        *const c_void,
        *const c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    create_swap_chain_for_core_window: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *const c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    get_shared_resource_adapter_luid: unsafe extern "system" fn(*mut c_void, usize, *mut u64) -> HRESULT,
    register_stereo_status_window: unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32) -> HRESULT,
    register_stereo_status_event: unsafe extern "system" fn(*mut c_void, usize, *mut u32) -> HRESULT,
    unregister_stereo_status: unsafe extern "system" fn(*mut c_void, u32),
    register_occlusion_status_window: unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32) -> HRESULT,
    register_occlusion_status_event: unsafe extern "system" fn(*mut c_void, usize, *mut u32) -> HRESULT,
    unregister_occlusion_status: unsafe extern "system" fn(*mut c_void, u32),
    create_swap_chain_for_composition:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *const c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
}

static FACTORY_IIDS: [Guid; 3] = [IID_IDXGIFACTORY, IID_IDXGIFACTORY1, IID_IDXGIFACTORY2];

static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
    query_interface,
    add_ref,
    release,
    set_private_data,
    set_private_data_interface,
    get_private_data,
    get_parent,
    enum_adapters,
    make_window_association,
    get_window_association,
    create_swap_chain,
    create_software_adapter,
    enum_adapters1: enum_adapters,
    is_current,
    is_windowed_stereo_enabled,
    create_swap_chain_for_hwnd,
    create_swap_chain_for_core_window,
    get_shared_resource_adapter_luid,
    register_stereo_status_window: register_status_window,
    register_stereo_status_event: register_status_event,
    unregister_stereo_status: unregister_status,
    register_occlusion_status_window: register_status_window,
    register_occlusion_status_event: register_status_event,
    unregister_occlusion_status: unregister_status,
    create_swap_chain_for_composition,
};

/// `IsWindowedStereoEnabled`: no stereo display.
unsafe extern "system" fn is_windowed_stereo_enabled(_: *mut c_void) -> i32 {
    0
}

/// `CreateSwapChainForHwnd(pDevice, hWnd, pDesc, pFullscreenDesc, pRestrictToOutput,
/// ppSwapChain)`: a swap chain of a `d3d11.dll` device presenting into the window.
unsafe extern "system" fn create_swap_chain_for_hwnd(
    _: *mut c_void,
    device: *mut c_void,
    hwnd: usize,
    desc: *const c_void,
    _: *const c_void,
    _: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    let f = d3d11_export(c"PolyD3D11CreateSwapChain");
    if f == 0 || device.is_null() || desc.is_null() {
        if !out.is_null() {
            // SAFETY: the caller's output pointer.
            unsafe { *out = core::ptr::null_mut() };
        }
        return DXGI_ERROR_UNSUPPORTED;
    }
    type Make = unsafe extern "system" fn(*mut c_void, usize, *const c_void, *mut *mut c_void) -> HRESULT;
    // SAFETY: d3d11.dll's export of that signature.
    unsafe { core::mem::transmute::<usize, Make>(f)(device, hwnd, desc, out) }
}

unsafe extern "system" fn create_swap_chain_for_core_window(
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *const c_void,
    _: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's output pointer or null.
    unsafe { create_swap_chain(core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), out) }
}

/// `CreateSwapChainForComposition(pDevice, pDesc, pRestrictToOutput, ppSwapChain)`: a swap chain
/// of a `d3d11.dll` device without a window; DirectComposition (`dcomp.dll`) shows it in one.
unsafe extern "system" fn create_swap_chain_for_composition(
    this: *mut c_void,
    device: *mut c_void,
    desc: *const c_void,
    _: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: as CreateSwapChainForHwnd without a window.
    unsafe { create_swap_chain_for_hwnd(this, device, 0, desc, core::ptr::null(), core::ptr::null_mut(), out) }
}

/// `GetSharedResourceAdapterLuid`: no shared resources exist.
unsafe extern "system" fn get_shared_resource_adapter_luid(_: *mut c_void, _: usize, _: *mut u64) -> HRESULT {
    E_INVALIDARG
}

/// `Register{Stereo,Occlusion}StatusWindow`: a cookie; the status never changes, nothing is sent.
unsafe extern "system" fn register_status_window(_: *mut c_void, _: usize, _: u32, cookie: *mut u32) -> HRESULT {
    if cookie.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's cookie.
    unsafe { *cookie = 1 };
    S_OK
}

/// `Register{Stereo,Occlusion}StatusEvent`: as the window form.
unsafe extern "system" fn register_status_event(_: *mut c_void, _: usize, cookie: *mut u32) -> HRESULT {
    if cookie.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's cookie.
    unsafe { *cookie = 1 };
    S_OK
}

unsafe extern "system" fn unregister_status(_: *mut c_void, _: u32) {}

/// `EnumAdapters` / `EnumAdapters1`: adapter 0 is the software adapter, there is no other.
unsafe extern "system" fn enum_adapters(this: *mut c_void, index: u32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if index != 0 {
        // SAFETY: the caller's output pointer.
        unsafe { *out = core::ptr::null_mut() };
        return DXGI_ERROR_NOT_FOUND;
    }
    let adapter = new_obj(&ADAPTER_VTBL as *const AdapterVtbl as *const c_void, &ADAPTER_IIDS, this);
    // SAFETY: the caller's output pointer.
    unsafe { *out = adapter };
    S_OK
}

unsafe extern "system" fn make_window_association(_: *mut c_void, _: usize, _: u32) -> HRESULT {
    S_OK
}

unsafe extern "system" fn get_window_association(_: *mut c_void, out: *mut usize) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *out = 0 };
    S_OK
}

/// `CreateSwapChain(pDevice, pDesc, ppSwapChain)`: a swap chain of a `d3d11.dll` device.
unsafe extern "system" fn create_swap_chain(
    _: *mut c_void,
    device: *mut c_void,
    desc: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    if !out.is_null() {
        // SAFETY: the caller's output pointer.
        unsafe { *out = core::ptr::null_mut() };
    }
    let f = d3d11_export(c"PolyD3D11CreateSwapChainLegacy");
    if f == 0 || device.is_null() || desc.is_null() {
        return DXGI_ERROR_UNSUPPORTED;
    }
    type Make = unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> HRESULT;
    // SAFETY: d3d11.dll's export of that signature.
    unsafe { core::mem::transmute::<usize, Make>(f)(device, desc, out) }
}

unsafe extern "system" fn create_software_adapter(_: *mut c_void, _: usize, out: *mut *mut c_void) -> HRESULT {
    if !out.is_null() {
        // SAFETY: the caller's output pointer.
        unsafe { *out = core::ptr::null_mut() };
    }
    DXGI_ERROR_UNSUPPORTED
}

/// `IsCurrent`: the adapters do not change.
unsafe extern "system" fn is_current(_: *mut c_void) -> i32 {
    1
}

// ----- the adapter

#[repr(C)]
struct AdapterVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_private_data: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void) -> HRESULT,
    set_private_data_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void) -> HRESULT,
    get_private_data: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut c_void) -> HRESULT,
    get_parent: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    enum_outputs: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    get_desc: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    check_interface_support: unsafe extern "system" fn(*mut c_void, *const Guid, *mut i64) -> HRESULT,
    get_desc1: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
}

static ADAPTER_IIDS: [Guid; 2] = [IID_IDXGIADAPTER, IID_IDXGIADAPTER1];

static ADAPTER_VTBL: AdapterVtbl = AdapterVtbl {
    query_interface,
    add_ref,
    release,
    set_private_data,
    set_private_data_interface,
    get_private_data,
    get_parent,
    enum_outputs,
    get_desc: adapter_desc,
    check_interface_support,
    get_desc1: adapter_desc1,
};

/// `EnumOutputs`: output 0 is the screen.
unsafe extern "system" fn enum_outputs(this: *mut c_void, index: u32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if index != 0 {
        // SAFETY: the caller's output pointer.
        unsafe { *out = core::ptr::null_mut() };
        return DXGI_ERROR_NOT_FOUND;
    }
    let output = new_obj(&OUTPUT_VTBL as *const OutputVtbl as *const c_void, &OUTPUT_IIDS, this);
    // SAFETY: the caller's output pointer.
    unsafe { *out = output };
    S_OK
}

/// `DXGI_ADAPTER_DESC` (304 bytes): the description, ids, memory sizes and the LUID.
fn write_adapter_desc(out: &mut [u8]) {
    out.fill(0);
    for (i, u) in "Microsoft Basic Render Driver".encode_utf16().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
    // VendorId, DeviceId (the basic render driver's), SubSysId, Revision.
    out[256..260].copy_from_slice(&0x1414u32.to_le_bytes());
    out[260..264].copy_from_slice(&0x8Cu32.to_le_bytes());
    // Shared system memory: 256 MiB (dedicated: none).
    out[288..296].copy_from_slice(&(256u64 << 20).to_le_bytes());
    // AdapterLuid: a locally unique id.
    out[296..300].copy_from_slice(&0x1414u32.to_le_bytes());
}

unsafe extern "system" fn adapter_desc(_: *mut c_void, desc: *mut u8) -> HRESULT {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DXGI_ADAPTER_DESC.
    write_adapter_desc(unsafe { core::slice::from_raw_parts_mut(desc, 304) });
    S_OK
}

/// `DXGI_ADAPTER_DESC1` (312 bytes): the above and `Flags`.
unsafe extern "system" fn adapter_desc1(_: *mut c_void, desc: *mut u8) -> HRESULT {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DXGI_ADAPTER_DESC1.
    let out = unsafe { core::slice::from_raw_parts_mut(desc, 312) };
    write_adapter_desc(&mut out[..304]);
    out[304..312].fill(0);
    out[304..308].copy_from_slice(&DXGI_ADAPTER_FLAG_SOFTWARE.to_le_bytes());
    S_OK
}

/// `CheckInterfaceSupport`: no Direct3D device can be made on it.
unsafe extern "system" fn check_interface_support(_: *mut c_void, _: *const Guid, _: *mut i64) -> HRESULT {
    DXGI_ERROR_UNSUPPORTED
}

// ----- the output

#[repr(C)]
struct OutputVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_private_data: unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void) -> HRESULT,
    set_private_data_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void) -> HRESULT,
    get_private_data: unsafe extern "system" fn(*mut c_void, *const Guid, *mut u32, *mut c_void) -> HRESULT,
    get_parent: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    get_desc: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    get_display_mode_list: unsafe extern "system" fn(*mut c_void, u32, u32, *mut u32, *mut c_void) -> HRESULT,
    find_closest_matching_mode:
        unsafe extern "system" fn(*mut c_void, *const c_void, *mut c_void, *mut c_void) -> HRESULT,
    wait_for_vblank: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    take_ownership: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
    release_ownership: unsafe extern "system" fn(*mut c_void),
    get_gamma_control_capabilities: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_gamma_control: unsafe extern "system" fn(*mut c_void, *const c_void) -> HRESULT,
    get_gamma_control: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_display_surface: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    get_display_surface_data: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    get_frame_statistics: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

static OUTPUT_IIDS: [Guid; 1] = [IID_IDXGIOUTPUT];

unsafe extern "system" fn unsupported1(_: *mut c_void, _: *mut c_void) -> HRESULT {
    DXGI_ERROR_UNSUPPORTED
}

unsafe extern "system" fn unsupported1c(_: *mut c_void, _: *const c_void) -> HRESULT {
    DXGI_ERROR_UNSUPPORTED
}

unsafe extern "system" fn mode_list(_: *mut c_void, _: u32, _: u32, _: *mut u32, _: *mut c_void) -> HRESULT {
    DXGI_ERROR_UNSUPPORTED
}

unsafe extern "system" fn closest_mode(_: *mut c_void, _: *const c_void, _: *mut c_void, _: *mut c_void) -> HRESULT {
    DXGI_ERROR_UNSUPPORTED
}

unsafe extern "system" fn take_ownership(_: *mut c_void, _: *mut c_void, _: i32) -> HRESULT {
    DXGI_ERROR_UNSUPPORTED
}

unsafe extern "system" fn release_ownership(_: *mut c_void) {}

/// `WaitForVBlank`: the screen is refreshed 60 times a second.
unsafe extern "system" fn wait_for_vblank(_: *mut c_void) -> HRESULT {
    // SAFETY: kernel32's Sleep.
    unsafe { Sleep(16) };
    S_OK
}

static OUTPUT_VTBL: OutputVtbl = OutputVtbl {
    query_interface,
    add_ref,
    release,
    set_private_data,
    set_private_data_interface,
    get_private_data,
    get_parent,
    get_desc: output_desc,
    get_display_mode_list: mode_list,
    find_closest_matching_mode: closest_mode,
    wait_for_vblank,
    take_ownership,
    release_ownership,
    get_gamma_control_capabilities: unsupported1,
    set_gamma_control: unsupported1c,
    get_gamma_control: unsupported1,
    set_display_surface: unsupported1,
    get_display_surface_data: unsupported1,
    get_frame_statistics: unsupported1,
};

/// `DXGI_OUTPUT_DESC` (96 bytes): `\\.\DISPLAY1`, the screen's rectangle, attached, not rotated,
/// its monitor.
unsafe extern "system" fn output_desc(_: *mut c_void, desc: *mut u8) -> HRESULT {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DXGI_OUTPUT_DESC.
    let out = unsafe { core::slice::from_raw_parts_mut(desc, 96) };
    out.fill(0);
    for (i, u) in "\\\\.\\DISPLAY1".encode_utf16().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
    // SAFETY: user32's metrics and monitor (SM_CXSCREEN, SM_CYSCREEN; MONITOR_DEFAULTTOPRIMARY).
    let (w, h, monitor) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1), MonitorFromPoint(0, 1)) };
    out[64..68].copy_from_slice(&0i32.to_le_bytes());
    out[68..72].copy_from_slice(&0i32.to_le_bytes());
    out[72..76].copy_from_slice(&w.to_le_bytes());
    out[76..80].copy_from_slice(&h.to_le_bytes());
    out[80..84].copy_from_slice(&1i32.to_le_bytes());
    out[84..88].copy_from_slice(&DXGI_MODE_ROTATION_IDENTITY.to_le_bytes());
    out[88..96].copy_from_slice(&(monitor as u64).to_le_bytes());
    S_OK
}

// ----- the exports

/// A new factory for `riid` (`IDXGIFactory` or `IDXGIFactory1`; later versions: `E_NOINTERFACE`).
unsafe fn create_factory(riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let f = new_obj(&FACTORY_VTBL as *const FactoryVtbl as *const c_void, &FACTORY_IIDS, core::ptr::null_mut());
    // SAFETY: a new object; the caller's output pointer.
    let hr = unsafe { query_interface(f, riid, out) };
    // SAFETY: the creation reference.
    unsafe { release(f) };
    hr
}

/// `CreateDXGIFactory(riid, ppFactory)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDXGIFactory(riid: *const c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's GUID and output pointer.
    unsafe { create_factory(riid as *const Guid, out) }
}

/// `CreateDXGIFactory1(riid, ppFactory)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDXGIFactory1(riid: *const c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's GUID and output pointer.
    unsafe { create_factory(riid as *const Guid, out) }
}

/// `CreateDXGIFactory2(Flags, riid, ppFactory)`: the flags (`DXGI_CREATE_FACTORY_DEBUG`) change nothing.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDXGIFactory2(_flags: u32, riid: *const c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's GUID and output pointer.
    unsafe { create_factory(riid as *const Guid, out) }
}

poly_win32_rt::fail_exports!(
    "dxgi",
    value(0x887A_0004),
    [DXGID3D10CreateDevice, DXGID3D10RegisterLayers, DXGIGetDebugInterface1]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dxgi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
