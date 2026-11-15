//! Our `dcomp.dll` (ADR 0155, S3 hunt): the DirectComposition a program needs to show a swap chain
//! in a window (Learn: `DCompositionCreateDevice`, `IDCompositionDevice::CreateTargetForHwnd`,
//! `CreateVisual`, `IDCompositionVisual::SetContent`, `IDCompositionTarget::SetRoot`, `Commit`).
//! Chrome's software compositor makes a composition swap chain (`d3d11.dll`), sets it as the
//! content of a visual, makes that visual the root of a target for its window and commits.
//!
//! Poly composes nothing itself: at `Commit`, every swap chain that is the content of a visual of
//! a target is bound to the target's window (`d3d11.dll`'s `PolyD3D11BindSwapChain`), and from then
//! on its `Present` draws into that window. Offsets, transforms, clips and effects are accepted and
//! not applied (the content is drawn at the window's top left); what is not implemented logs its
//! name once (`stub: dcomp: ...`). Interface identifiers and method orders are those of the public
//! `dcomp.h`, whose overloaded methods occupy their vtable slots in reverse order of declaration
//! (the Microsoft C++ ABI).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

poly_win32_rt::process_heap_allocator!();

type HRESULT = i32;
const S_OK: HRESULT = 0;
const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, a: u16, b: u64) -> Guid {
    let a = a.to_be_bytes();
    let b = b.to_be_bytes();
    Guid { d1, d2, d3, d4: [a[0], a[1], b[2], b[3], b[4], b[5], b[6], b[7]] }
}

const IID_IUNKNOWN: Guid = guid(0, 0, 0, 0xC000, 0x46);
const IID_DEVICE: Guid = guid(0xC37EA93A, 0xE7AA, 0x450D, 0xB16F, 0x9746CB0407F3);
const IID_DEVICE2: Guid = guid(0x75F6468D, 0x1B8E, 0x447C, 0x9BC6, 0x75FEA80B5B25);
const IID_DESKTOP_DEVICE: Guid = guid(0x5F4633FE, 0x1E08, 0x4CB8, 0x8C75, 0xCE24333F5602);
const IID_TARGET: Guid = guid(0xeacdd04c, 0x117e, 0x4e17, 0x88f4, 0xd1b12b0e3d89);
const IID_VISUAL: Guid = guid(0x4d93059d, 0x097b, 0x4651, 0x9a60, 0xf0f25116e2f3);
const IID_VISUAL2: Guid = guid(0xE8DE1639, 0x4331, 0x4B26, 0xBC5F, 0x6A321D347A85);

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> usize;
}

fn log(text: &str) {
    // SAFETY: the service reads `text.len()` bytes.
    unsafe { PolyDebugPrint(text.as_ptr(), text.len()) };
}

fn log_iid(what: &str, iid: &Guid) {
    let d = iid.d4;
    log(&alloc::format!(
        "stub: dcomp: {what} has no interface {{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
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

/// `d3d11.dll`'s `PolyD3D11BindSwapChain(content, hwnd)`: whether `content` is one of its swap
/// chains (then bound to `hwnd` and shown there).
fn bind_swap_chain(content: *mut c_void, hwnd: usize) -> bool {
    let dll: [u16; 10] = [100, 51, 100, 49, 49, 46, 100, 108, 108, 0];
    // SAFETY: a NUL-terminated module name and export name.
    let f = unsafe {
        let m = LoadLibraryW(dll.as_ptr());
        if m == 0 {
            return false;
        }
        GetProcAddress(m, c"PolyD3D11BindSwapChain".as_ptr() as *const u8)
    };
    if f == 0 {
        return false;
    }
    type Bind = unsafe extern "system" fn(*mut c_void, usize) -> i32;
    // SAFETY: d3d11.dll's export of that signature.
    unsafe { core::mem::transmute::<usize, Bind>(f)(content, hwnd) != 0 }
}

// ----- objects

#[repr(C)]
struct Obj {
    vtbl: *const c_void,
    refs: AtomicU32,
    iids: &'static [Guid],
    dtor: unsafe fn(*mut c_void),
    name: &'static str,
}

#[repr(transparent)]
struct Vt<const N: usize>([*const c_void; N]);
// SAFETY: function addresses only.
unsafe impl<const N: usize> Sync for Vt<N> {}

impl<const N: usize> Vt<N> {
    fn ptr(&'static self) -> *const c_void {
        self.0.as_ptr() as *const c_void
    }
}

fn into_raw<T>(v: T) -> *mut c_void {
    Box::into_raw(Box::new(v)) as *mut c_void
}

unsafe fn obj<'a>(p: *mut c_void) -> &'a Obj {
    // SAFETY: every object of this DLL starts with an `Obj`.
    unsafe { &*(p as *const Obj) }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    // SAFETY: an object of this DLL.
    unsafe { obj(this) }.refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    // SAFETY: an object of this DLL.
    let o = unsafe { obj(this) };
    let n = o.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference went.
        unsafe { (o.dtor)(this) };
    }
    n
}

unsafe extern "system" fn query_interface(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output.
    unsafe { *out = core::ptr::null_mut() };
    if riid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID; an object of this DLL.
    let (iid, o) = unsafe { (*riid, obj(this)) };
    if iid == IID_IUNKNOWN || o.iids.contains(&iid) {
        // SAFETY: a live object.
        unsafe {
            add_ref(this);
            *out = this;
        }
        return S_OK;
    }
    log_iid(o.name, &iid);
    E_NOINTERFACE
}

/// Release a COM object of any DLL.
unsafe fn com_release(p: *mut c_void) {
    if !p.is_null() {
        // SAFETY: a COM object: Release is entry 2 of its vtable.
        unsafe {
            let vt = *(p as *const *const usize);
            let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
            f(p);
        }
    }
}

/// AddRef a COM object of any DLL.
unsafe fn com_add_ref(p: *mut c_void) {
    if !p.is_null() {
        // SAFETY: a COM object: AddRef is entry 1 of its vtable.
        unsafe {
            let vt = *(p as *const *const usize);
            let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(1));
            f(p);
        }
    }
}

/// Methods not implemented: each logs its name the first time and fails with `E_NOTIMPL`.
macro_rules! stubs {
    ($m:ident, $iface:literal, [$($name:ident),* $(,)?]) => {
        #[allow(non_snake_case)]
        mod $m {
            use core::ffi::c_void;
            use core::sync::atomic::{AtomicBool, Ordering};
            $(
                pub unsafe extern "system" fn $name(_: *mut c_void) -> i32 {
                    static ONCE: AtomicBool = AtomicBool::new(false);
                    if !ONCE.swap(true, Ordering::Relaxed) {
                        super::log(concat!("stub: dcomp: ", $iface, "::", stringify!($name)));
                    }
                    super::E_NOTIMPL
                }
            )*
        }
    };
}

// ----- the device

#[repr(C)]
struct Device {
    obj: Obj,
    /// The targets made, kept until the device goes.
    targets: Vec<*mut c_void>,
}

unsafe fn drop_device(p: *mut c_void) {
    // SAFETY: made by `new_device`.
    let d = unsafe { Box::from_raw(p as *mut Device) };
    for t in d.targets {
        // SAFETY: the references the device kept.
        unsafe { release(t) };
    }
}

stubs!(
    dev_stubs,
    "IDCompositionDevice",
    [
        CreateSurface,
        CreateVirtualSurface,
        CreateSurfaceFromHandle,
        CreateSurfaceFromHwnd,
        CreateTranslateTransform,
        CreateScaleTransform,
        CreateRotateTransform,
        CreateSkewTransform,
        CreateMatrixTransform,
        CreateTransformGroup,
        CreateTranslateTransform3D,
        CreateScaleTransform3D,
        CreateRotateTransform3D,
        CreateMatrixTransform3D,
        CreateTransform3DGroup,
        CreateEffectGroup,
        CreateRectangleClip,
        CreateAnimation,
        CreateSurfaceFactory,
    ]
);

static DEVICE_IIDS: [Guid; 1] = [IID_DEVICE];
static DEVICE2_IIDS: [Guid; 2] = [IID_DEVICE2, IID_DESKTOP_DEVICE];

/// `IDCompositionDevice`.
static DEVICE_VTBL: Vt<27> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    commit as *const c_void,
    wait_for_commit_completion as *const c_void,
    get_frame_statistics as *const c_void,
    create_target_for_hwnd as *const c_void,
    create_visual as *const c_void,
    dev_stubs::CreateSurface as *const c_void,
    dev_stubs::CreateVirtualSurface as *const c_void,
    dev_stubs::CreateSurfaceFromHandle as *const c_void,
    dev_stubs::CreateSurfaceFromHwnd as *const c_void,
    dev_stubs::CreateTranslateTransform as *const c_void,
    dev_stubs::CreateScaleTransform as *const c_void,
    dev_stubs::CreateRotateTransform as *const c_void,
    dev_stubs::CreateSkewTransform as *const c_void,
    dev_stubs::CreateMatrixTransform as *const c_void,
    dev_stubs::CreateTransformGroup as *const c_void,
    dev_stubs::CreateTranslateTransform3D as *const c_void,
    dev_stubs::CreateScaleTransform3D as *const c_void,
    dev_stubs::CreateRotateTransform3D as *const c_void,
    dev_stubs::CreateMatrixTransform3D as *const c_void,
    dev_stubs::CreateTransform3DGroup as *const c_void,
    dev_stubs::CreateEffectGroup as *const c_void,
    dev_stubs::CreateRectangleClip as *const c_void,
    dev_stubs::CreateAnimation as *const c_void,
    check_device_state as *const c_void,
]);

/// `IDCompositionDesktopDevice` (`IDCompositionDevice2` and three methods).
static DESKTOP_DEVICE_VTBL: Vt<27> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    commit as *const c_void,
    wait_for_commit_completion as *const c_void,
    get_frame_statistics as *const c_void,
    create_visual as *const c_void,
    dev_stubs::CreateSurfaceFactory as *const c_void,
    dev_stubs::CreateSurface as *const c_void,
    dev_stubs::CreateVirtualSurface as *const c_void,
    dev_stubs::CreateTranslateTransform as *const c_void,
    dev_stubs::CreateScaleTransform as *const c_void,
    dev_stubs::CreateRotateTransform as *const c_void,
    dev_stubs::CreateSkewTransform as *const c_void,
    dev_stubs::CreateMatrixTransform as *const c_void,
    dev_stubs::CreateTransformGroup as *const c_void,
    dev_stubs::CreateTranslateTransform3D as *const c_void,
    dev_stubs::CreateScaleTransform3D as *const c_void,
    dev_stubs::CreateRotateTransform3D as *const c_void,
    dev_stubs::CreateMatrixTransform3D as *const c_void,
    dev_stubs::CreateTransform3DGroup as *const c_void,
    dev_stubs::CreateEffectGroup as *const c_void,
    dev_stubs::CreateRectangleClip as *const c_void,
    dev_stubs::CreateAnimation as *const c_void,
    create_target_for_hwnd as *const c_void,
    dev_stubs::CreateSurfaceFromHandle as *const c_void,
    dev_stubs::CreateSurfaceFromHwnd as *const c_void,
]);

fn new_device(desktop: bool) -> *mut c_void {
    let (vtbl, iids) =
        if desktop { (DESKTOP_DEVICE_VTBL.ptr(), &DEVICE2_IIDS[..]) } else { (DEVICE_VTBL.ptr(), &DEVICE_IIDS[..]) };
    into_raw(Device {
        obj: Obj { vtbl, refs: AtomicU32::new(1), iids, dtor: drop_device, name: "IDCompositionDevice" },
        targets: Vec::new(),
    })
}

/// `Commit`: every swap chain shown by a target's visual tree is bound to its window.
unsafe extern "system" fn commit(this: *mut c_void) -> HRESULT {
    // SAFETY: a live device.
    let d = unsafe { &*(this as *const Device) };
    for &t in &d.targets {
        // SAFETY: a live target the device keeps.
        let t = unsafe { &*(t as *const Target) };
        if !t.root.is_null() {
            // SAFETY: a live visual of this DLL.
            unsafe { bind_tree(t.root, t.hwnd) };
        }
    }
    S_OK
}

unsafe fn bind_tree(v: *mut c_void, hwnd: usize) {
    // SAFETY: a live visual.
    let v = unsafe { &*(v as *const Visual) };
    if !v.content.is_null() && !bind_swap_chain(v.content, hwnd) {
        log("stub: dcomp: a visual's content is not a swap chain of d3d11.dll");
    }
    for &c in &v.children {
        // SAFETY: a live child visual.
        unsafe { bind_tree(c, hwnd) };
    }
}

unsafe extern "system" fn wait_for_commit_completion(_: *mut c_void) -> HRESULT {
    S_OK
}

/// `GetFrameStatistics`: `DCOMPOSITION_FRAME_STATISTICS` (40 bytes) of a 60 Hz composition.
unsafe extern "system" fn get_frame_statistics(_: *mut c_void, out: *mut u8) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's structure: last frame time, composition rate (60/1), current time,
    // time frequency, next estimated frame time.
    unsafe {
        core::ptr::write_bytes(out, 0, 40);
        (out.add(8) as *mut u32).write(60);
        (out.add(12) as *mut u32).write(1);
        (out.add(24) as *mut i64).write(1);
    }
    S_OK
}

unsafe extern "system" fn check_device_state(_: *mut c_void, valid: *mut i32) -> HRESULT {
    if valid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output.
    unsafe { *valid = 1 };
    S_OK
}

/// `CreateTargetForHwnd(hwnd, topmost, target)`.
unsafe extern "system" fn create_target_for_hwnd(
    this: *mut c_void,
    hwnd: usize,
    _topmost: i32,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let t = into_raw(Target {
        obj: Obj {
            vtbl: TARGET_VTBL.ptr(),
            refs: AtomicU32::new(1),
            iids: &TARGET_IIDS,
            dtor: drop_target,
            name: "IDCompositionTarget",
        },
        hwnd,
        root: core::ptr::null_mut(),
    });
    // SAFETY: a live device; the new target gets one more reference for it.
    unsafe {
        add_ref(t);
        (*(this as *mut Device)).targets.push(t);
        *out = t;
    }
    S_OK
}

/// `CreateVisual(visual)`.
unsafe extern "system" fn create_visual(_: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let v = into_raw(Visual {
        obj: Obj {
            vtbl: VISUAL_VTBL.ptr(),
            refs: AtomicU32::new(1),
            iids: &VISUAL_IIDS,
            dtor: drop_visual,
            name: "IDCompositionVisual",
        },
        content: core::ptr::null_mut(),
        children: Vec::new(),
    });
    // SAFETY: the caller's output.
    unsafe { *out = v };
    S_OK
}

// ----- targets

#[repr(C)]
struct Target {
    obj: Obj,
    hwnd: usize,
    root: *mut c_void,
}

static TARGET_IIDS: [Guid; 1] = [IID_TARGET];

static TARGET_VTBL: Vt<4> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    set_root as *const c_void,
]);

unsafe fn drop_target(p: *mut c_void) {
    // SAFETY: made by `create_target_for_hwnd`.
    let t = unsafe { Box::from_raw(p as *mut Target) };
    if !t.root.is_null() {
        // SAFETY: the root's reference.
        unsafe { release(t.root) };
    }
}

/// `SetRoot(visual)`.
unsafe extern "system" fn set_root(this: *mut c_void, visual: *mut c_void) -> HRESULT {
    // SAFETY: a live target; the caller's visual (of this DLL) or null.
    unsafe {
        let t = &mut *(this as *mut Target);
        if !visual.is_null() {
            add_ref(visual);
        }
        let old = core::mem::replace(&mut t.root, visual);
        if !old.is_null() {
            release(old);
        }
    }
    S_OK
}

// ----- visuals

#[repr(C)]
struct Visual {
    obj: Obj,
    content: *mut c_void,
    children: Vec<*mut c_void>,
}

static VISUAL_IIDS: [Guid; 2] = [IID_VISUAL, IID_VISUAL2];

stubs!(visual_stubs, "IDCompositionVisual", [SetTransformParent, SetEffect]);

/// `IDCompositionVisual2`; the overloaded setters (`SetOffsetX`, `SetOffsetY`, `SetTransform`,
/// `SetClip`) take their two slots each, both accepted and not applied.
static VISUAL_VTBL: Vt<22> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    visual_stubs::SetTransformParent as *const c_void,
    visual_stubs::SetEffect as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    set_content as *const c_void,
    add_visual as *const c_void,
    remove_visual as *const c_void,
    remove_all_visuals as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
    accept as *const c_void,
]);

unsafe fn drop_visual(p: *mut c_void) {
    // SAFETY: made by `create_visual`.
    let v = unsafe { Box::from_raw(p as *mut Visual) };
    // SAFETY: the references the visual kept.
    unsafe {
        com_release(v.content);
        for c in v.children {
            release(c);
        }
    }
}

/// A setter whose value is not applied.
unsafe extern "system" fn accept(_: *mut c_void) -> HRESULT {
    S_OK
}

/// `SetContent(content)`: a swap chain (or nothing).
unsafe extern "system" fn set_content(this: *mut c_void, content: *mut c_void) -> HRESULT {
    // SAFETY: a live visual; the caller's content or null.
    unsafe {
        let v = &mut *(this as *mut Visual);
        com_add_ref(content);
        com_release(core::mem::replace(&mut v.content, content));
    }
    S_OK
}

/// `AddVisual(visual, insertAbove, referenceVisual)`: the order among siblings is not kept.
unsafe extern "system" fn add_visual(this: *mut c_void, child: *mut c_void, _: i32, _: *mut c_void) -> HRESULT {
    if child.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live visual; the caller's visual of this DLL.
    unsafe {
        add_ref(child);
        (*(this as *mut Visual)).children.push(child);
    }
    S_OK
}

unsafe extern "system" fn remove_visual(this: *mut c_void, child: *mut c_void) -> HRESULT {
    // SAFETY: a live visual.
    let v = unsafe { &mut *(this as *mut Visual) };
    match v.children.iter().position(|&c| c == child) {
        Some(i) => {
            v.children.remove(i);
            // SAFETY: the reference AddVisual took.
            unsafe { release(child) };
            S_OK
        }
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn remove_all_visuals(this: *mut c_void) -> HRESULT {
    // SAFETY: a live visual.
    let v = unsafe { &mut *(this as *mut Visual) };
    for c in core::mem::take(&mut v.children) {
        // SAFETY: the references AddVisual took.
        unsafe { release(c) };
    }
    S_OK
}

// ----- the exports

unsafe fn create(desktop: bool, riid: *const c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let d = new_device(desktop);
    // SAFETY: a new device; the caller's interface id and output.
    let hr = unsafe { query_interface(d, riid as *const Guid, out) };
    // SAFETY: the creation reference.
    unsafe { release(d) };
    hr
}

/// `DCompositionCreateDevice(dxgiDevice, iid, dcompositionDevice)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DCompositionCreateDevice(
    _dxgi: *mut c_void,
    riid: *const c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { create(false, riid, out) }
}

/// `DCompositionCreateDevice2(renderingDevice, iid, dcompositionDevice)`: an
/// `IDCompositionDesktopDevice`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DCompositionCreateDevice2(
    _device: *mut c_void,
    riid: *const c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { create(true, riid, out) }
}

/// `DCompositionCreateDevice3`: as `DCompositionCreateDevice2`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DCompositionCreateDevice3(
    _device: *mut c_void,
    riid: *const c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: the caller's arguments.
    unsafe { create(true, riid, out) }
}

poly_win32_rt::fail_exports!(
    "dcomp",
    hresult,
    [
        ord_1038,
        ord_1040,
        CompileEffectDescription,
        CreateEffectDescription,
        DCompositionAttachMouseDragToHwnd,
        DCompositionAttachMouseWheelToHwnd,
        DCompositionCreateSurfaceHandle,
        DeserializeEffectDescription,
        DllGetActivationFactory,
        DwmEnableMMCSS,
        DwmFlush,
        DwmpEnableDDASupport,
        SerializeEffectDescription,
    ]
);

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn dcomp_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
