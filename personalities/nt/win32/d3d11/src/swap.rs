//! Swap chains of an HWND (Learn: `IDXGIFactory2::CreateSwapChainForHwnd`, `IDXGISwapChain1`): one
//! back buffer, a 2D texture of the device; `Present` copies it into the window through GDI
//! (`GetDC`, `SetDIBitsToDevice`), the way a window shows a bitmap.

use crate::com::*;
use crate::stubs;
use crate::texture::{self, BIND_RENDER_TARGET, Tex};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetDC(hwnd: usize) -> usize;
    fn ReleaseDC(hwnd: usize, dc: usize) -> i32;
    fn GetClientRect(hwnd: usize, rect: *mut [i32; 4]) -> i32;
    fn IsWindow(hwnd: usize) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    #[allow(clippy::too_many_arguments)]
    fn SetDIBitsToDevice(
        dc: usize,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        sx: i32,
        sy: i32,
        start: u32,
        lines: u32,
        bits: *const u8,
        info: *const u8,
        usage: u32,
    ) -> i32;
}

/// `DXGI_SWAP_CHAIN_DESC1` as 12 words: Width, Height, Format, Stereo, SampleDesc (2),
/// BufferUsage, BufferCount, Scaling, SwapEffect, AlphaMode, Flags.
type Desc1 = [u32; 12];

#[repr(C)]
pub struct SwapChain {
    obj: Obj,
    device: *mut c_void,
    hwnd: usize,
    desc: Desc1,
    back: *mut c_void,
    presents: AtomicU32,
}

static SWAP_IIDS: [Guid; 4] = [IID_IDXGISWAPCHAIN1, IID_IDXGISWAPCHAIN, IID_IDXGIDEVICESUBOBJECT, IID_IDXGIOBJECT];

/// A swap chain of `device` for `hwnd` described by `desc1` (`DXGI_SWAP_CHAIN_DESC1`).
pub unsafe fn create(device: *mut c_void, hwnd: usize, desc1: *const u8, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's output.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: the caller's device.
    if desc1.is_null() || !unsafe { crate::device::is_device(device) } {
        return E_INVALIDARG;
    }
    // A composition swap chain (`hwnd` 0) has no window until a DirectComposition commit
    // binds it to one (`bind`).
    // SAFETY: a window handle of the caller.
    if hwnd != 0 && unsafe { IsWindow(hwnd) } == 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DXGI_SWAP_CHAIN_DESC1 (48 bytes).
    let mut desc: Desc1 = unsafe { *(desc1 as *const Desc1) };
    let (w, h) = if hwnd == 0 { (1, 1) } else { client_size(hwnd) };
    if desc[0] == 0 {
        desc[0] = w.max(1);
    }
    if desc[1] == 0 {
        desc[1] = h.max(1);
    }
    // SAFETY: a live device of this DLL.
    unsafe { add_ref(device) };
    let back = new_back_buffer(device, &desc);
    let p = into_raw(SwapChain {
        obj: Obj::new(SWAP_VTBL.ptr(), &SWAP_IIDS, drop_swap, "IDXGISwapChain1"),
        device,
        hwnd,
        desc,
        back,
        presents: AtomicU32::new(0),
    });
    // SAFETY: the caller's output.
    unsafe { *out = p };
    S_OK
}

/// The same from a `DXGI_SWAP_CHAIN_DESC` (72 bytes: the mode, samples, usage, count, the
/// window, windowed, the swap effect, flags).
pub unsafe fn from_legacy_desc(device: *mut c_void, desc: *const u8, out: *mut *mut c_void) -> HRESULT {
    if desc.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DXGI_SWAP_CHAIN_DESC.
    let (d, hwnd) = unsafe {
        let w = |o: usize| *(desc.add(o) as *const u32);
        ([w(0), w(4), w(16), 0, w(28), w(32), w(36), w(40), 0, w(60), 0, w(64)], *(desc.add(48) as *const usize))
    };
    // SAFETY: as `create`.
    unsafe { create(device, hwnd, d.as_ptr() as *const u8, out) }
}

/// Bind swap chain `p` (if it is one of ours) to window `hwnd` and show its back buffer there.
pub unsafe fn bind(p: *mut c_void, hwnd: usize) -> bool {
    // SAFETY: a COM object's first word is its vtable.
    if p.is_null() || unsafe { *(p as *const *const c_void) } != SWAP_VTBL.ptr() {
        return false;
    }
    // SAFETY: one of our swap chains.
    let s = unsafe { chain(p) };
    if s.hwnd != hwnd {
        log(&alloc::format!("d3d11: swap chain {}x{} bound to window {hwnd:#x}", s.desc[0], s.desc[1]));
    }
    s.hwnd = hwnd;
    // SAFETY: the back buffer is a live texture of this DLL.
    if let Some(t) = unsafe { texture::tex(s.back) } {
        show(hwnd, t);
    }
    true
}

fn client_size(hwnd: usize) -> (u32, u32) {
    let mut r = [0i32; 4];
    // SAFETY: a window handle and our RECT.
    if unsafe { GetClientRect(hwnd, &mut r) } == 0 {
        return (1, 1);
    }
    ((r[2] - r[0]).max(0) as u32, (r[3] - r[1]).max(0) as u32)
}

fn new_back_buffer(device: *mut c_void, d: &Desc1) -> *mut c_void {
    // D3D11_TEXTURE2D_DESC: width, height, 1 mip, 1 slice, format, samples, default usage,
    // bound as a render target.
    let desc = [d[0], d[1], 1, 1, d[2], 1, 0, 0, BIND_RENDER_TARGET, 0, 0];
    // SAFETY: a live device.
    unsafe { texture::create(device, desc, None) }
}

unsafe fn drop_swap(p: *mut c_void) {
    // SAFETY: a swap chain whose last reference went.
    let (device, back) = unsafe { ((*(p as *mut SwapChain)).device, (*(p as *mut SwapChain)).back) };
    // SAFETY: made by `create`; the references it took.
    unsafe {
        drop_box::<SwapChain>(p);
        release(back);
        release(device);
    }
}

unsafe fn chain<'a>(p: *mut c_void) -> &'a mut SwapChain {
    // SAFETY: a live swap chain.
    unsafe { &mut *(p as *mut SwapChain) }
}

stubs!(swap_stubs, "IDXGISwapChain1", [GetContainingOutput, GetCoreWindow]);

static SWAP_VTBL: Vt<29> = Vt([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    set_private_data as *const c_void,
    set_private_data_interface as *const c_void,
    get_private_data as *const c_void,
    get_parent as *const c_void,
    get_device as *const c_void,
    present as *const c_void,
    get_buffer as *const c_void,
    set_fullscreen_state as *const c_void,
    get_fullscreen_state as *const c_void,
    get_desc as *const c_void,
    resize_buffers as *const c_void,
    resize_target as *const c_void,
    swap_stubs::GetContainingOutput as *const c_void,
    get_frame_statistics as *const c_void,
    get_last_present_count as *const c_void,
    get_desc1 as *const c_void,
    get_fullscreen_desc as *const c_void,
    get_hwnd as *const c_void,
    swap_stubs::GetCoreWindow as *const c_void,
    present1 as *const c_void,
    is_temporary_mono_supported as *const c_void,
    get_restrict_to_output as *const c_void,
    set_background_color as *const c_void,
    get_background_color as *const c_void,
    set_rotation as *const c_void,
    get_rotation as *const c_void,
]);

unsafe extern "system" fn get_parent(_: *mut c_void, _: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { put(out, core::ptr::null_mut()) };
    E_NOINTERFACE
}

unsafe extern "system" fn get_device(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: a live swap chain's device.
    unsafe { query_interface(chain(this).device, riid, out) }
}

/// `Present(SyncInterval, Flags)`: the back buffer into the window (`DXGI_PRESENT_TEST`, 1,
/// presents nothing).
unsafe extern "system" fn present(this: *mut c_void, _sync: u32, flags: u32) -> HRESULT {
    // SAFETY: a live swap chain.
    let s = unsafe { chain(this) };
    if flags & 1 != 0 {
        return S_OK;
    }
    s.presents.fetch_add(1, Ordering::Relaxed);
    // SAFETY: the back buffer is a live texture of this DLL.
    let Some(t) = (unsafe { texture::tex(s.back) }) else { return S_OK };
    if s.hwnd != 0 {
        show(s.hwnd, t);
    }
    S_OK
}

/// `Present1(SyncInterval, PresentFlags, pPresentParameters)`: as `Present` (the whole buffer).
unsafe extern "system" fn present1(this: *mut c_void, sync: u32, flags: u32, _: *const c_void) -> HRESULT {
    // SAFETY: as `Present`.
    unsafe { present(this, sync, flags) }
}

/// Copy a texture into the window's client area, top-down 32-bit pixels.
fn show(hwnd: usize, t: &Tex) {
    if texture::bytes_per_pixel(t.format()) != 4 {
        return;
    }
    let (w, h) = (t.width() as u32, t.height() as u32);
    let swapped: Vec<u8>;
    let bits = if texture::red_first(t.format()) {
        swapped = t.data.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        swapped.as_ptr()
    } else {
        t.data.as_ptr()
    };
    // BITMAPINFOHEADER: 40 bytes, width, -height (top-down), 1 plane, 32 bits, BI_RGB.
    let mut info = [0u8; 40];
    info[0..4].copy_from_slice(&40u32.to_le_bytes());
    info[4..8].copy_from_slice(&(w as i32).to_le_bytes());
    info[8..12].copy_from_slice(&(-(h as i32)).to_le_bytes());
    info[12..14].copy_from_slice(&1u16.to_le_bytes());
    info[14..16].copy_from_slice(&32u16.to_le_bytes());
    // SAFETY: the window's DC, released below; `h` rows of `w` pixels at `bits`.
    let (dc, lines) = unsafe {
        let dc = GetDC(hwnd);
        let lines = if dc == 0 { 0 } else { SetDIBitsToDevice(dc, 0, 0, w, h, 0, 0, 0, h, bits, info.as_ptr(), 0) };
        if dc != 0 {
            ReleaseDC(hwnd, dc);
        }
        (dc, lines)
    };
    // The first frames shown say where they went (the kernel log).
    static SHOWN: AtomicU32 = AtomicU32::new(0);
    if SHOWN.fetch_add(1, Ordering::Relaxed) < 3 || (dc == 0 || lines == 0) && SHOWN.load(Ordering::Relaxed) < 20 {
        let first = t.data.get(..4).map_or(0, |p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]));
        log(&alloc::format!(
            "d3d11: present {w}x{h} format {} into window {hwnd:#x}: dc {dc:#x}, {lines} lines, first pixel {first:#010x}",
            t.format()
        ));
    }
}

/// `GetBuffer(Buffer, riid, ppSurface)`: buffer 0 is the back buffer.
unsafe extern "system" fn get_buffer(
    this: *mut c_void,
    index: u32,
    riid: *const Guid,
    out: *mut *mut c_void,
) -> HRESULT {
    if index != 0 {
        // SAFETY: the caller's output.
        unsafe { put(out, core::ptr::null_mut()) };
        return DXGI_ERROR_NOT_FOUND;
    }
    // SAFETY: a live swap chain's back buffer.
    unsafe { query_interface(chain(this).back, riid, out) }
}

unsafe extern "system" fn set_fullscreen_state(_: *mut c_void, full: i32, _: *mut c_void) -> HRESULT {
    if full != 0 { DXGI_ERROR_UNSUPPORTED } else { S_OK }
}

unsafe extern "system" fn get_fullscreen_state(_: *mut c_void, full: *mut i32, target: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's optional outputs.
    unsafe {
        put(full, 0);
        put(target, core::ptr::null_mut());
    }
    S_OK
}

/// `GetDesc`: the `DXGI_SWAP_CHAIN_DESC` of the description (windowed).
unsafe extern "system" fn get_desc(this: *mut c_void, out: *mut u8) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live swap chain; the caller's DXGI_SWAP_CHAIN_DESC (72 bytes).
    unsafe {
        let s = chain(this);
        core::ptr::write_bytes(out, 0, 72);
        let w = |o: usize, v: u32| (out.add(o) as *mut u32).write(v);
        w(0, s.desc[0]);
        w(4, s.desc[1]);
        w(16, s.desc[2]);
        w(28, s.desc[4]);
        w(32, s.desc[5]);
        w(36, s.desc[6]);
        w(40, s.desc[7]);
        (out.add(48) as *mut usize).write(s.hwnd);
        w(56, 1);
        w(60, s.desc[9]);
        w(64, s.desc[11]);
    }
    S_OK
}

/// `ResizeBuffers(BufferCount, Width, Height, NewFormat, SwapChainFlags)`: a new back buffer
/// (0 keeps the count and format, a size of 0 is the window's).
unsafe extern "system" fn resize_buffers(
    this: *mut c_void,
    count: u32,
    w: u32,
    h: u32,
    format: u32,
    flags: u32,
) -> HRESULT {
    // SAFETY: a live swap chain.
    let s = unsafe { chain(this) };
    let (cw, ch) = client_size(s.hwnd);
    s.desc[0] = if w == 0 { cw.max(1) } else { w };
    s.desc[1] = if h == 0 { ch.max(1) } else { h };
    if s.hwnd == 0 && (w == 0 || h == 0) {
        // A composition swap chain keeps its size when none is given.
        s.desc[0] = if w == 0 { s.desc[0].max(1) } else { w };
    }
    if format != 0 {
        s.desc[2] = format;
    }
    if count != 0 {
        s.desc[7] = count;
    }
    s.desc[11] = flags;
    let old = core::mem::replace(&mut s.back, new_back_buffer(s.device, &s.desc));
    // SAFETY: the old back buffer's reference.
    unsafe { release(old) };
    S_OK
}

unsafe extern "system" fn resize_target(_: *mut c_void, _: *const c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn get_frame_statistics(_: *mut c_void, _: *mut c_void) -> HRESULT {
    DXGI_ERROR_FRAME_STATISTICS_DISJOINT
}

unsafe extern "system" fn get_last_present_count(this: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: a live swap chain; the caller's output.
    unsafe { put(out, chain(this).presents.load(Ordering::Relaxed)) };
    S_OK
}

unsafe extern "system" fn get_desc1(this: *mut c_void, out: *mut Desc1) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live swap chain; the caller's output.
    unsafe { *out = chain(this).desc };
    S_OK
}

/// `GetFullscreenDesc`: `DXGI_SWAP_CHAIN_FULLSCREEN_DESC` of a windowed swap chain.
unsafe extern "system" fn get_fullscreen_desc(_: *mut c_void, out: *mut [u32; 5]) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output: refresh rate, scanline ordering, scaling, windowed.
    unsafe { *out = [0, 0, 0, 0, 1] };
    S_OK
}

unsafe extern "system" fn get_hwnd(this: *mut c_void, out: *mut usize) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live swap chain; the caller's output.
    unsafe { *out = chain(this).hwnd };
    S_OK
}

unsafe extern "system" fn is_temporary_mono_supported(_: *mut c_void) -> i32 {
    0
}

unsafe extern "system" fn get_restrict_to_output(_: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { put(out, core::ptr::null_mut()) };
    S_OK
}

unsafe extern "system" fn set_background_color(_: *mut c_void, _: *const c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn get_background_color(_: *mut c_void, out: *mut [f32; 4]) -> HRESULT {
    // SAFETY: the caller's output.
    unsafe { put(out, [0.0; 4]) };
    S_OK
}

unsafe extern "system" fn set_rotation(_: *mut c_void, rotation: u32) -> HRESULT {
    if rotation <= 1 { S_OK } else { DXGI_ERROR_UNSUPPORTED }
}

unsafe extern "system" fn get_rotation(_: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: the caller's output: DXGI_MODE_ROTATION_IDENTITY.
    unsafe { put(out, 1) };
    S_OK
}
