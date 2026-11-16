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
const DXGI_ERROR_INVALID_CALL: HRESULT = 0x887A_0001u32 as i32;
/// The software adapter's locally unique id (non-zero, high part 0: IDXGIAdapter1.GetDesc1.yaml).
const ADAPTER_LUID: u64 = 0x0000_0000_0000_1414;
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
// dxgi.h, dxgi1_3.h .. dxgi1_6.h
const IID_IDXGIDEVICE: Guid = guid(0x54ec77fa, 0x1377, 0x44e6, [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c]);
const IID_IDXGIFACTORY3: Guid = guid(0x25483823, 0xcd46, 0x4c7d, [0x86, 0xca, 0x47, 0xaa, 0x95, 0xb8, 0x37, 0xbd]);
const IID_IDXGIFACTORY4: Guid = guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
const IID_IDXGIFACTORY5: Guid = guid(0x7632e1f5, 0xee65, 0x4dca, [0x87, 0xfd, 0x84, 0xcd, 0x75, 0xf8, 0x83, 0x8d]);
const IID_IDXGIFACTORY6: Guid = guid(0xc1b6694f, 0xff09, 0x44a9, [0xb0, 0x3c, 0x77, 0x90, 0x0a, 0x0a, 0x1d, 0x17]);
const IID_IDXGIFACTORY7: Guid = guid(0xa4966eed, 0x76db, 0x44da, [0x84, 0xc1, 0xee, 0x9a, 0x7a, 0xfb, 0x20, 0xa8]);
const IID_IDXGIADAPTER2: Guid = guid(0x0aa1ae0a, 0xfa0e, 0x4b84, [0x86, 0x44, 0xe0, 0x5f, 0xf8, 0xe5, 0xac, 0xb5]);
const IID_IDXGIADAPTER3: Guid = guid(0x645967a4, 0x1392, 0x4310, [0xa7, 0x98, 0x80, 0x53, 0xce, 0x3e, 0x93, 0xfd]);
const IID_IDXGIADAPTER4: Guid = guid(0x3c8d99d1, 0x4fbf, 0x4181, [0xa8, 0x2c, 0xaf, 0x66, 0xbf, 0x7b, 0xd2, 0x4e]);
const IID_IDXGIOUTPUT: Guid = guid(0xae02eedb, 0xc735, 0x4690, [0x8d, 0x52, 0x5a, 0x8d, 0xc2, 0x02, 0x13, 0xaa]);

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn MonitorFromPoint(x: i64, flags: u32) -> usize;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
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

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
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
    /// A factory's creation flags; for an adapter, 1 when it is the WARP adapter (which has no
    /// outputs, IDXGIAdapter.EnumOutputs.yaml `warp`).
    extra: u32,
    private: core::cell::UnsafeCell<poly_d3d::privdata::Store>,
}

fn new_obj(vtbl: *const c_void, iids: &'static [Guid], parent: *mut c_void) -> *mut c_void {
    new_obj_with(vtbl, iids, parent, 0)
}

fn new_obj_with(vtbl: *const c_void, iids: &'static [Guid], parent: *mut c_void, extra: u32) -> *mut c_void {
    if !parent.is_null() {
        // SAFETY: a live object of this DLL.
        unsafe { add_ref(parent) };
    }
    let private = core::cell::UnsafeCell::new(poly_d3d::privdata::Store::new());
    Box::into_raw(Box::new(Obj { vtbl, refs: AtomicU32::new(1), iids, parent, extra, private })) as *mut c_void
}

fn obj<'a>(p: *mut c_void) -> &'a Obj {
    // SAFETY: every object of this DLL starts with an `Obj`.
    unsafe { &*(p as *const Obj) }
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
        // SAFETY: the object's private data, no longer reachable by anyone else.
        let held: alloc::vec::Vec<usize> = unsafe { &*o.private.get() }.interfaces().collect();
        for p in held {
            // SAFETY: a reference `SetPrivateDataInterface` took.
            unsafe { com_release(p as *mut c_void) };
        }
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

fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..4].copy_from_slice(&g.d1.to_le_bytes());
    b[4..6].copy_from_slice(&g.d2.to_le_bytes());
    b[6..8].copy_from_slice(&g.d3.to_le_bytes());
    b[8..].copy_from_slice(&g.d4);
    b
}

/// AddRef / Release of any COM object through its vtable.
unsafe fn com_add_ref(p: *mut c_void) -> u32 {
    // SAFETY: a live COM object: AddRef is entry 1.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(1));
        f(p)
    }
}

unsafe fn com_release(p: *mut c_void) -> u32 {
    // SAFETY: a live COM object: Release is entry 2.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(2));
        f(p)
    }
}

/// The private data of an object (IDXGIDevice.GetAdapter.yaml `private_data_*`: per COM object).
#[allow(clippy::mut_from_ref)]
fn store<'a>(this: *mut c_void) -> &'a mut poly_d3d::privdata::Store {
    // SAFETY: a live object; its private data is changed by its own calls only (the programs that
    // use DXGI objects from several threads do not set their private data at once).
    unsafe { &mut *obj(this).private.get() }
}

unsafe extern "system" fn set_private_data(this: *mut c_void, guid: *const Guid, size: u32, data: *const c_void) -> HRESULT {
    if guid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID and `size` bytes.
    let (g, bytes) = unsafe {
        let bytes = if data.is_null() { None } else { Some(core::slice::from_raw_parts(data as *const u8, size as usize)) };
        (guid_bytes(&*guid), bytes)
    };
    let (hr, old) = store(this).set(g, bytes, size);
    if let Some(p) = old {
        // SAFETY: the reference the replaced entry held.
        unsafe { com_release(p as *mut c_void) };
    }
    hr
}

unsafe extern "system" fn set_private_data_interface(this: *mut c_void, guid: *const Guid, p: *mut c_void) -> HRESULT {
    if guid.is_null() {
        return E_INVALIDARG;
    }
    if !p.is_null() {
        // SAFETY: the caller's live interface.
        unsafe { com_add_ref(p) };
    }
    // SAFETY: the caller's GUID.
    let old = store(this).set_interface(unsafe { guid_bytes(&*guid) }, p as usize);
    if let Some(o) = old {
        // SAFETY: the reference the replaced entry held.
        unsafe { com_release(o as *mut c_void) };
    }
    S_OK
}

unsafe extern "system" fn get_private_data(this: *mut c_void, guid: *const Guid, size: *mut u32, data: *mut c_void) -> HRESULT {
    use poly_d3d::privdata::Got;
    if guid.is_null() || size.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID and size.
    let (g, asked) = unsafe { (guid_bytes(&*guid), *size) };
    match store(this).get(&g, asked, !data.is_null()) {
        Got::Interface(p) => {
            // SAFETY: a live interface the object holds; the caller's 8 bytes and size.
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
            // SAFETY: the caller's size and buffer (at least `n` bytes when bytes are copied).
            unsafe {
                *size = n;
                if let Some(b) = bytes {
                    core::ptr::copy_nonoverlapping(b.as_ptr(), data as *mut u8, b.len());
                }
            }
            hr
        }
    }
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
    // IDXGIFactory3 .. IDXGIFactory7 (dxgi1_3.h .. dxgi1_6.h)
    get_creation_flags: unsafe extern "system" fn(*mut c_void) -> u32,
    enum_adapter_by_luid: unsafe extern "system" fn(*mut c_void, u64, *const Guid, *mut *mut c_void) -> HRESULT,
    enum_warp_adapter: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
    check_feature_support: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> HRESULT,
    enum_adapter_by_gpu_preference:
        unsafe extern "system" fn(*mut c_void, u32, u32, *const Guid, *mut *mut c_void) -> HRESULT,
    register_adapters_changed_event: unsafe extern "system" fn(*mut c_void, usize, *mut u32) -> HRESULT,
    unregister_adapters_changed_event: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

static FACTORY_IIDS: [Guid; 8] = [
    IID_IDXGIFACTORY,
    IID_IDXGIFACTORY1,
    IID_IDXGIFACTORY2,
    IID_IDXGIFACTORY3,
    IID_IDXGIFACTORY4,
    IID_IDXGIFACTORY5,
    IID_IDXGIFACTORY6,
    IID_IDXGIFACTORY7,
];

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
    get_creation_flags,
    enum_adapter_by_luid,
    enum_warp_adapter,
    check_feature_support,
    enum_adapter_by_gpu_preference,
    register_adapters_changed_event: register_status_event,
    unregister_adapters_changed_event,
};

/// `GetCreationFlags`: the flags of `CreateDXGIFactory2` (the debug bit is not kept: Poly has no
/// debug layer, CreateDXGIFactory1.yaml `factory2_flags`).
unsafe extern "system" fn get_creation_flags(this: *mut c_void) -> u32 {
    obj(this).extra
}

/// A new adapter object of factory `this`, as interface `riid`.
unsafe fn adapter_as(this: *mut c_void, warp: bool, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let a = new_obj_with(&ADAPTER_VTBL as *const AdapterVtbl as *const c_void, &ADAPTER_IIDS, this, u32::from(warp));
    // SAFETY: the new adapter; the caller's output.
    let hr = unsafe { query_interface(a, riid, out) };
    // SAFETY: the reference `new_obj` gave.
    unsafe { release(a) };
    hr
}

/// `EnumAdapterByLuid(AdapterLuid, riid, ppvAdapter)`: the one adapter has the one LUID.
unsafe extern "system" fn enum_adapter_by_luid(this: *mut c_void, luid: u64, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if luid != ADAPTER_LUID {
        if !out.is_null() {
            // SAFETY: the caller's output.
            unsafe { *out = core::ptr::null_mut() };
        }
        return DXGI_ERROR_NOT_FOUND;
    }
    // SAFETY: as `adapter_as`.
    unsafe { adapter_as(this, false, riid, out) }
}

/// `EnumWarpAdapter(riid, ppvAdapter)`: the software adapter, a new object each call.
unsafe extern "system" fn enum_warp_adapter(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: as `adapter_as`.
    unsafe { adapter_as(this, true, riid, out) }
}

/// `EnumAdapterByGpuPreference(Adapter, GpuPreference, riid, ppvAdapter)`: every preference lists
/// the one adapter.
unsafe extern "system" fn enum_adapter_by_gpu_preference(
    this: *mut c_void,
    index: u32,
    _: u32,
    riid: *const Guid,
    out: *mut *mut c_void,
) -> HRESULT {
    if index != 0 {
        if !out.is_null() {
            // SAFETY: the caller's output.
            unsafe { *out = core::ptr::null_mut() };
        }
        return DXGI_ERROR_NOT_FOUND;
    }
    // SAFETY: as `adapter_as`.
    unsafe { adapter_as(this, false, riid, out) }
}

/// `CheckFeatureSupport(Feature, pFeatureSupportData, FeatureSupportDataSize)`: only
/// `DXGI_FEATURE_PRESENT_ALLOW_TEARING` (0) with a 4-byte BOOL (IDXGIFactory5.CheckFeatureSupport.yaml).
unsafe extern "system" fn check_feature_support(_: *mut c_void, feature: u32, data: *mut c_void, size: u32) -> HRESULT {
    if feature != 0 || size != 4 || data.is_null() {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: the caller's BOOL.
    unsafe { *(data as *mut i32) = 1 };
    S_OK
}

unsafe extern "system" fn unregister_adapters_changed_event(_: *mut c_void, _: u32) -> HRESULT {
    S_OK
}

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
        return if f == 0 { DXGI_ERROR_UNSUPPORTED } else { DXGI_ERROR_INVALID_CALL };
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
    let adapter = new_obj_with(&ADAPTER_VTBL as *const AdapterVtbl as *const c_void, &ADAPTER_IIDS, this, 0);
    // SAFETY: the caller's output pointer.
    unsafe { *out = adapter };
    S_OK
}

/// `MakeWindowAssociation(WindowHandle, Flags)`: only the DXGI_MWA bits 1, 2, 4 (dxgi.h) are
/// valid (IDXGIFactory.MakeWindowAssociation.yaml); nothing watches the window.
unsafe extern "system" fn make_window_association(_: *mut c_void, _: usize, flags: u32) -> HRESULT {
    if flags & !7 != 0 { DXGI_ERROR_INVALID_CALL } else { S_OK }
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
    // IDXGIAdapter2 .. IDXGIAdapter4 (dxgi1_2.h .. dxgi1_6.h)
    get_desc2: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
    register_teardown: unsafe extern "system" fn(*mut c_void, usize, *mut u32) -> HRESULT,
    unregister_teardown: unsafe extern "system" fn(*mut c_void, u32),
    query_video_memory_info: unsafe extern "system" fn(*mut c_void, u32, u32, *mut [u64; 4]) -> HRESULT,
    set_video_memory_reservation: unsafe extern "system" fn(*mut c_void, u32, u32, u64) -> HRESULT,
    register_budget: unsafe extern "system" fn(*mut c_void, usize, *mut u32) -> HRESULT,
    unregister_budget: unsafe extern "system" fn(*mut c_void, u32),
    get_desc3: unsafe extern "system" fn(*mut c_void, *mut u8) -> HRESULT,
}

static ADAPTER_IIDS: [Guid; 5] = [IID_IDXGIADAPTER, IID_IDXGIADAPTER1, IID_IDXGIADAPTER2, IID_IDXGIADAPTER3, IID_IDXGIADAPTER4];

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
    get_desc2: adapter_desc2,
    register_teardown: register_status_event,
    unregister_teardown: unregister_status,
    query_video_memory_info,
    set_video_memory_reservation,
    register_budget: register_status_event,
    unregister_budget: unregister_status,
    get_desc3: adapter_desc2,
};

/// `GetDesc2` / `GetDesc3`: the description, Flags SOFTWARE, and the preemption granularities
/// INSTRUCTION_BOUNDARY (4, 4) of a software adapter (IDXGIAdapter1.GetDesc1.yaml `get_desc2`).
unsafe extern "system" fn adapter_desc2(_: *mut c_void, desc: *mut u8) -> HRESULT {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DXGI_ADAPTER_DESC2 (320 bytes).
    let out = unsafe { core::slice::from_raw_parts_mut(desc, 320) };
    write_adapter_desc(&mut out[..304]);
    out[304..320].fill(0);
    out[304..308].copy_from_slice(&DXGI_ADAPTER_FLAG_SOFTWARE.to_le_bytes());
    out[308..312].copy_from_slice(&4u32.to_le_bytes());
    out[312..316].copy_from_slice(&4u32.to_le_bytes());
    S_OK
}

/// `QueryVideoMemoryInfo(NodeIndex, MemorySegmentGroup, pVideoMemoryInfo)`: node 0 only; the local
/// group has a budget, the non-local none (IDXGIAdapter3.QueryVideoMemoryInfo.yaml).
unsafe extern "system" fn query_video_memory_info(_: *mut c_void, node: u32, group: u32, out: *mut [u64; 4]) -> HRESULT {
    if node != 0 || group > 1 || out.is_null() {
        return E_INVALIDARG;
    }
    let budget = if group == 0 { 256u64 << 20 } else { 0 };
    // SAFETY: the caller's DXGI_QUERY_VIDEO_MEMORY_INFO: Budget, CurrentUsage,
    // AvailableForReservation, CurrentReservation.
    unsafe { *out = [budget, 0, budget / 2, 0] };
    S_OK
}

/// `SetVideoMemoryReservation`: no device made on this adapter object reserves memory.
unsafe extern "system" fn set_video_memory_reservation(_: *mut c_void, _: u32, _: u32, _: u64) -> HRESULT {
    E_INVALIDARG
}

/// `EnumOutputs`: output 0 is the screen.
unsafe extern "system" fn enum_outputs(this: *mut c_void, index: u32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // The WARP adapter has no outputs (IDXGIAdapter.EnumOutputs.yaml `warp`); the machine's
    // adapter shows the screen.
    if index != 0 || obj(this).extra == 1 {
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
    // AdapterLuid: a locally unique id (LowPart, HighPart 0).
    out[296..304].copy_from_slice(&ADAPTER_LUID.to_le_bytes());
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

/// `CheckInterfaceSupport(InterfaceName, pUMDVersion)`: a user-mode driver version for
/// `IDXGIDevice` (the software device's), DXGI_ERROR_UNSUPPORTED for anything else, the version not
/// written (IDXGIAdapter1.GetDesc1.yaml `check_interface_support`).
unsafe extern "system" fn check_interface_support(_: *mut c_void, iid: *const Guid, out: *mut i64) -> HRESULT {
    // SAFETY: the caller's GUID.
    if iid.is_null() || unsafe { *iid } != IID_IDXGIDEVICE {
        return DXGI_ERROR_UNSUPPORTED;
    }
    if !out.is_null() {
        // SAFETY: the caller's output: 10.0.26100.1 as a driver version (four 16-bit parts).
        unsafe { *out = (10i64 << 48) | (26100i64 << 16) | 1 };
    }
    S_OK
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
    // SAFETY: as `create_factory_with`.
    unsafe { create_factory_with(riid, out, 0) }
}

unsafe fn create_factory_with(riid: *const Guid, out: *mut *mut c_void, flags: u32) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let f = new_obj_with(&FACTORY_VTBL as *const FactoryVtbl as *const c_void, &FACTORY_IIDS, core::ptr::null_mut(), flags);
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
pub unsafe extern "system" fn CreateDXGIFactory2(flags: u32, riid: *const c_void, out: *mut *mut c_void) -> HRESULT {
    // DXGI_CREATE_FACTORY_DEBUG alone needs the debug layer, which Poly does not have; with other
    // bits it is dropped (CreateDXGIFactory1.yaml `factory2_flags`, the oracle's values).
    if flags == 1 {
        if !out.is_null() {
            // SAFETY: the caller's output pointer.
            unsafe { *out = core::ptr::null_mut() };
        }
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: the caller's GUID and output pointer.
    unsafe { create_factory_with(riid as *const Guid, out, flags & !1) }
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
