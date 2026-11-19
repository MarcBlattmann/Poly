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
    fn GetDesktopWindow() -> usize;
}

#[cfg_attr(target_arch = "x86", link(name = "win32u", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "win32u", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: usize, out: *mut u8, cap: usize, out_len: *mut usize) -> u32;
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
    source: (u32, u32),
    latency: u32,
    matrix: [f32; 6],
}

static SWAP_IIDS: [Guid; 7] = [
    IID_IDXGISWAPCHAIN1,
    IID_IDXGISWAPCHAIN,
    IID_IDXGISWAPCHAIN2,
    IID_IDXGISWAPCHAIN3,
    IID_IDXGISWAPCHAIN4,
    IID_IDXGIDEVICESUBOBJECT,
    IID_IDXGIOBJECT,
];

/// A swap chain of `device` for `hwnd` described by `desc1` (`DXGI_SWAP_CHAIN_DESC1`).
pub unsafe fn create(device: *mut c_void, hwnd: usize, desc1: *const u8, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: the caller's output.
    unsafe { *out = core::ptr::null_mut() };
    // The device argument may be the device or its IDXGIDevice (CreateSwapChainForHwnd.yaml
    // `device_argument`); anything else is an invalid call.
    // SAFETY: the caller's object.
    let Some(device) = (unsafe { crate::device::as_device(device) }) else { return DXGI_ERROR_INVALID_CALL };
    if desc1.is_null() {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: the caller's DXGI_SWAP_CHAIN_DESC1 (48 bytes).
    let mut desc: Desc1 = unsafe { *(desc1 as *const Desc1) };
    // A composition swap chain (`hwnd` 0) has no window until a DirectComposition commit
    // binds it to one (`bind`).
    if let Err(hr) = poly_d3d::swap::check(&desc, hwnd == 0) {
        return hr;
    }
    // SAFETY: a window handle of the caller.
    if hwnd != 0 && unsafe { IsWindow(hwnd) } == 0 {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: a system call.
    if hwnd != 0 && hwnd == unsafe { GetDesktopWindow() } {
        return E_ACCESSDENIED;
    }
    // Either size 0: both come from the window's client area.
    if hwnd != 0 && (desc[0] == 0 || desc[1] == 0) {
        let (w, h) = client_size(hwnd);
        desc[0] = w.max(1);
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
        source: (0, 0),
        latency: 1,
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
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
        ([w(0), w(4), w(16), 0, w(28), w(32), w(36), w(40), 0, w(60), 3, w(64)], *(desc.add(48) as *const usize))
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
    if s.hwnd != hwnd && hwnd != 0 {
        tell_alpha(hwnd, s.desc[10] == DXGI_ALPHA_MODE_PREMULTIPLIED);
    }
    s.hwnd = hwnd;
    // SAFETY: the back buffer is a live texture of this DLL.
    if let Some(t) = unsafe { texture::tex(s.back) } {
        show(hwnd, t);
    }
    true
}

/// `DXGI_ALPHA_MODE_PREMULTIPLIED` (`dxgi1_2.h`).
const DXGI_ALPHA_MODE_PREMULTIPLIED: u32 = 1;

/// Tell win32k whether what this swap chain shows in window `hwnd` has premultiplied alpha, so
/// the window is composed with it (ADR 0225; Learn, "DXGI_ALPHA_MODE": a composition swap chain's
/// premultiplied pixels are blended over what is below the window). Chrome's menus and bubbles
/// are transparent outside their rounded corners and in their shadow.
fn tell_alpha(hwnd: usize, premultiplied: bool) {
    use poly_win32k::proto::Request;
    let req = poly_win32k::wire::encode(&Request::SurfaceAlpha { hwnd: hwnd as u32, premultiplied });
    let mut out = [0u8; 64];
    let mut len = 0usize;
    // SAFETY: the buffers are ours and sized as passed.
    unsafe { PolyUserCall(req.as_ptr(), req.len(), out.as_mut_ptr(), out.len(), &mut len) };
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

/// `IDXGISwapChain4` (dxgi1_5.h): `IDXGISwapChain1` and the methods of `2` .. `4` after it.
static SWAP_VTBL: Vt<41> = Vt([
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
    // IDXGISwapChain2
    set_source_size as *const c_void,
    get_source_size as *const c_void,
    set_maximum_frame_latency as *const c_void,
    get_maximum_frame_latency as *const c_void,
    get_frame_latency_waitable_object as *const c_void,
    set_matrix_transform as *const c_void,
    get_matrix_transform as *const c_void,
    // IDXGISwapChain3
    get_current_back_buffer_index as *const c_void,
    check_color_space_support as *const c_void,
    set_color_space1 as *const c_void,
    resize_buffers1 as *const c_void,
    // IDXGISwapChain4
    set_hdr_metadata as *const c_void,
]);

/// `SetSourceSize(Width, Height)`: a part of the buffers no larger than them.
unsafe extern "system" fn set_source_size(this: *mut c_void, w: u32, h: u32) -> HRESULT {
    // SAFETY: a live swap chain.
    let s = unsafe { chain(this) };
    if w == 0 || h == 0 || w > s.desc[0] || h > s.desc[1] {
        return DXGI_ERROR_INVALID_CALL;
    }
    s.source = (w, h);
    S_OK
}

unsafe extern "system" fn get_source_size(this: *mut c_void, w: *mut u32, h: *mut u32) -> HRESULT {
    // SAFETY: a live swap chain; the caller's outputs.
    unsafe {
        let s = chain(this);
        let (sw, sh) = if s.source.0 == 0 { (s.desc[0], s.desc[1]) } else { s.source };
        put(w, sw);
        put(h, sh);
    }
    S_OK
}

/// The latency calls need the FRAME_LATENCY_WAITABLE_OBJECT flag (0x40, dxgi.h).
const WAITABLE: u32 = 0x40;

unsafe extern "system" fn set_maximum_frame_latency(this: *mut c_void, n: u32) -> HRESULT {
    // SAFETY: a live swap chain.
    let s = unsafe { chain(this) };
    if s.desc[11] & WAITABLE == 0 || n == 0 || n > 16 {
        return DXGI_ERROR_INVALID_CALL;
    }
    s.latency = n;
    S_OK
}

unsafe extern "system" fn get_maximum_frame_latency(this: *mut c_void, out: *mut u32) -> HRESULT {
    // SAFETY: a live swap chain.
    let s = unsafe { chain(this) };
    if s.desc[11] & WAITABLE == 0 {
        return DXGI_ERROR_INVALID_CALL;
    }
    // SAFETY: the caller's output.
    unsafe { put(out, s.latency) };
    S_OK
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateEventW(attrs: *const c_void, manual: i32, initial: i32, name: *const u16) -> usize;
}

/// `GetFrameLatencyWaitableObject`: NULL without the flag; with it, an event that is set (the
/// frames are presented at once, so the swap chain is always ready for the next one).
unsafe extern "system" fn get_frame_latency_waitable_object(this: *mut c_void) -> usize {
    // SAFETY: a live swap chain.
    let s = unsafe { chain(this) };
    if s.desc[11] & WAITABLE == 0 {
        return 0;
    }
    // SAFETY: a new event, signalled; the caller closes it.
    unsafe { CreateEventW(core::ptr::null(), 1, 1, core::ptr::null()) }
}

unsafe extern "system" fn set_matrix_transform(this: *mut c_void, m: *const [f32; 6]) -> HRESULT {
    if m.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live swap chain; the caller's DXGI_MATRIX_3X2_F.
    unsafe { chain(this).matrix = *m };
    S_OK
}

unsafe extern "system" fn get_matrix_transform(this: *mut c_void, m: *mut [f32; 6]) -> HRESULT {
    // SAFETY: a live swap chain; the caller's output.
    unsafe { put(m, chain(this).matrix) };
    S_OK
}

/// `GetCurrentBackBufferIndex`: the one back buffer.
unsafe extern "system" fn get_current_back_buffer_index(_: *mut c_void) -> u32 {
    0
}

/// `CheckColorSpaceSupport(ColorSpace, pColorSpaceSupport)`: sRGB (G22 P709, 0) is presentable.
unsafe extern "system" fn check_color_space_support(_: *mut c_void, cs: u32, out: *mut u32) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's output: DXGI_SWAP_CHAIN_COLOR_SPACE_SUPPORT_FLAG_PRESENT (1).
    unsafe { *out = u32::from(cs == 0) };
    S_OK
}

unsafe extern "system" fn set_color_space1(_: *mut c_void, cs: u32) -> HRESULT {
    if cs == 0 { S_OK } else { E_INVALIDARG }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn resize_buffers1(
    this: *mut c_void,
    count: u32,
    w: u32,
    h: u32,
    format: u32,
    flags: u32,
    _: *const u32,
    _: *const *mut c_void,
) -> HRESULT {
    // SAFETY: as `ResizeBuffers`.
    unsafe { resize_buffers(this, count, w, h, format, flags) }
}

unsafe extern "system" fn set_hdr_metadata(_: *mut c_void, kind: u32, size: u32, data: *const c_void) -> HRESULT {
    // DXGI_HDR_METADATA_TYPE_NONE (0) only, without data.
    if kind == 0 && (size == 0 || !data.is_null()) { S_OK } else { DXGI_ERROR_INVALID_CALL }
}

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
unsafe extern "system" fn present(this: *mut c_void, sync: u32, flags: u32) -> HRESULT {
    // SyncInterval 0 .. 4 (Learn, "IDXGISwapChain::Present"; Present.yaml).
    if sync > 4 {
        return DXGI_ERROR_INVALID_CALL;
    }
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
/// `Present1(SyncInterval, PresentFlags, pPresentParameters)`: dirty and scroll rectangles are for
/// flip-model swap chains only.
unsafe extern "system" fn present1(this: *mut c_void, sync: u32, flags: u32, params: *const u32) -> HRESULT {
    if !params.is_null() {
        // SAFETY: the caller's DXGI_PRESENT_PARAMETERS: DirtyRectsCount first.
        let dirty = unsafe { *params };
        // SAFETY: a live swap chain.
        if dirty != 0 && !poly_d3d::swap::is_flip(unsafe { chain(this) }.desc[9]) {
            return DXGI_ERROR_INVALID_CALL;
        }
    }
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
        swapped = t.data().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        swapped.as_ptr()
    } else {
        t.data().as_ptr()
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
        let first = t.data().get(..4).map_or(0, |p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]));
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
        return DXGI_ERROR_INVALID_CALL;
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
