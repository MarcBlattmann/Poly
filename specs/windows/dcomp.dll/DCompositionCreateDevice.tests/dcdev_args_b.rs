//! DCompositionCreateDevice / 2 / 3: the rendering device argument (NULL, IDXGIDevice, ID3D11Device, other objects),
//! reference counts of the rendering device, devices created and used on other threads, release order.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 2 of the observations, the others write the rest.)
#![no_std]
#![no_main]
#![allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    unused_macros,
    clippy::all
)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts, the order of the
// methods in the vtables: d3d11*.h, dxgi*.h, dcomp.h, d3dcommon.h, winerror.h);
// the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HRESULT = i32;
type HANDLE = *mut c_void;

/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const IID_IUnknown: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// A made-up interface id nothing implements.
const IID_Nothing: GUID = GUID { d1: 0x1234_5678, d2: 0x9abc, d3: 0xdef0, d4: [1, 2, 3, 4, 5, 6, 7, 8] };

/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where observation lines go.
trait Sink {
    fn put(&mut self, s: &str);
}

impl Sink for Test {
    fn put(&mut self, s: &str) {
        if say_keep() {
            self.line(s);
        }
    }
}

/// Which observations of this run are written: 0 all of them, 1/2/3 every third one starting at the first/second/third
/// (a test kept in three files writes a third of its observations in each; every call is made in all three).
static SAY_PART: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SAY_N: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn say_keep() -> bool {
    let p = SAY_PART.load(core::sync::atomic::Ordering::Relaxed);
    if p == 0 {
        return true;
    }
    let n = SAY_N.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    n % 3 + 1 == p
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<600>::new();
        let _ = write!(b, $($a)*);
        $t.put(b.as_str());
    }};
}

/// HRESULT as `0x........`.
struct Hr(HRESULT);

impl core::fmt::Display for Hr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#010x}", self.0 as u32)
    }
}

/// A pointer as "null" / "set" (addresses are never printed).
fn np(p: *const c_void) -> &'static str {
    if p.is_null() { "null" } else { "set" }
}

// winerror.h
const S_OK: HRESULT = 0;
const S_FALSE: HRESULT = 1;
const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
const E_OUTOFMEMORY: HRESULT = 0x8007_000Eu32 as i32;
const DXGI_ERROR_INVALID_CALL: HRESULT = 0x887A_0001u32 as i32;
const DXGI_ERROR_NOT_FOUND: HRESULT = 0x887A_0002u32 as i32;
const DXGI_ERROR_MORE_DATA: HRESULT = 0x887A_0003u32 as i32;
const DXGI_ERROR_UNSUPPORTED: HRESULT = 0x887A_0004u32 as i32;
const DXGI_ERROR_DEVICE_REMOVED: HRESULT = 0x887A_0005u32 as i32;
const DXGI_ERROR_WAS_STILL_DRAWING: HRESULT = 0x887A_000Au32 as i32;
const DXGI_STATUS_OCCLUDED: HRESULT = 0x087A_0001;
const DXGI_ERROR_NONEXCLUSIVE: HRESULT = 0x887A_0021u32 as i32;
const D3D11_ERROR_FILE_NOT_FOUND: HRESULT = 0x887C_0002u32 as i32;
const D3D11_ERROR_TOO_MANY_UNIQUE_STATE_OBJECTS: HRESULT = 0x887C_0001u32 as i32;

// ---- COM through vtable slots (the slot numbers are in `slots`, from the SDK headers) ----

/// The function at vtable slot `slot` of COM object `obj`, as `fn(this, args..) -> ret`.
macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $ret:ty $(, $arg:expr)* $(,)?) => {{
        let o: *mut c_void = $obj;
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $ret =
            core::mem::transmute(*(*(o as *const *const usize)).add($slot));
        f(o $(, $arg)*)
    }};
}

/// QueryInterface on any COM object: the HRESULT and the interface pointer (null on failure).
fn qi(obj: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: `obj` is a live COM object; `iid` and `out` are valid for the call.
    let hr = unsafe { vcall!(obj, 0, fn(*const GUID, *mut *mut c_void) -> HRESULT, iid, &mut out) };
    (hr, out)
}

/// AddRef on any COM object: the new reference count.
fn addref(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: `obj` is a live COM object.
    unsafe { vcall!(obj, 1, fn() -> u32) }
}

/// Release on any COM object: the new reference count.
fn release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: `obj` is a live COM object this test holds a reference to.
    unsafe { vcall!(obj, 2, fn() -> u32) }
}

/// The reference count of `obj` as AddRef then Release see it (the count with this probe's own reference removed).
fn refs(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    addref(obj);
    release(obj)
}

/// Whether `obj` answers QueryInterface for `iid`, as "yes" / the HRESULT; releases what it got.
fn has_iface(obj: *mut c_void, iid: &GUID) -> bool {
    let (hr, p) = qi(obj, iid);
    if hr >= 0 {
        release(p);
        true
    } else {
        false
    }
}

/// "S_OK-like" text of a result for an observation line: the HRESULT, and `(pointer set)` or `(pointer null)`.
fn res(hr: HRESULT, p: *mut c_void) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let _ = write!(b, "{} ptr={}", Hr(hr), np(p));
    b
}

/// The words of `bytes` as hex without separators (up to 16 bytes).
fn hex(bytes: &[u8]) -> Buf<64> {
    let mut b = Buf::<64>::new();
    for x in bytes.iter().take(24) {
        let _ = write!(b, "{:02x}", x);
    }
    b
}

/// A UTF-16 name up to its NUL as ASCII text.
fn utf16_text<const N: usize>(w: &[u16]) -> Buf<N> {
    let mut b = Buf::<N>::new();
    for &c in w {
        if c == 0 {
            break;
        }
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

fn wide(s: &str) -> [u16; 64] {
    let mut w = [0u16; 64];
    for (i, c) in s.chars().enumerate().take(63) {
        w[i] = c as u32 as u16;
    }
    w
}

// Vtable slots and interface ids: the order of the methods in the C `...Vtbl` structures of the SDK headers.
#[allow(non_snake_case, dead_code)]
mod slots {
    pub mod ID3D11Device {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const CreateBuffer: usize = 3;
        pub const CreateTexture1D: usize = 4;
        pub const CreateTexture2D: usize = 5;
        pub const CreateTexture3D: usize = 6;
        pub const CreateShaderResourceView: usize = 7;
        pub const CreateUnorderedAccessView: usize = 8;
        pub const CreateRenderTargetView: usize = 9;
        pub const CreateDepthStencilView: usize = 10;
        pub const CreateInputLayout: usize = 11;
        pub const CreateVertexShader: usize = 12;
        pub const CreateGeometryShader: usize = 13;
        pub const CreateGeometryShaderWithStreamOutput: usize = 14;
        pub const CreatePixelShader: usize = 15;
        pub const CreateHullShader: usize = 16;
        pub const CreateDomainShader: usize = 17;
        pub const CreateComputeShader: usize = 18;
        pub const CreateClassLinkage: usize = 19;
        pub const CreateBlendState: usize = 20;
        pub const CreateDepthStencilState: usize = 21;
        pub const CreateRasterizerState: usize = 22;
        pub const CreateSamplerState: usize = 23;
        pub const CreateQuery: usize = 24;
        pub const CreatePredicate: usize = 25;
        pub const CreateCounter: usize = 26;
        pub const CreateDeferredContext: usize = 27;
        pub const OpenSharedResource: usize = 28;
        pub const CheckFormatSupport: usize = 29;
        pub const CheckMultisampleQualityLevels: usize = 30;
        pub const CheckCounterInfo: usize = 31;
        pub const CheckCounter: usize = 32;
        pub const CheckFeatureSupport: usize = 33;
        pub const GetPrivateData: usize = 34;
        pub const SetPrivateData: usize = 35;
        pub const SetPrivateDataInterface: usize = 36;
        pub const GetFeatureLevel: usize = 37;
        pub const GetCreationFlags: usize = 38;
        pub const GetDeviceRemovedReason: usize = 39;
        pub const GetImmediateContext: usize = 40;
        pub const SetExceptionMode: usize = 41;
        pub const GetExceptionMode: usize = 42;
        pub const COUNT: usize = 43;
    }
    pub mod ID3D11DeviceContext {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const GetDevice: usize = 3;
        pub const GetPrivateData: usize = 4;
        pub const SetPrivateData: usize = 5;
        pub const SetPrivateDataInterface: usize = 6;
        pub const VSSetConstantBuffers: usize = 7;
        pub const PSSetShaderResources: usize = 8;
        pub const PSSetShader: usize = 9;
        pub const PSSetSamplers: usize = 10;
        pub const VSSetShader: usize = 11;
        pub const DrawIndexed: usize = 12;
        pub const Draw: usize = 13;
        pub const Map: usize = 14;
        pub const Unmap: usize = 15;
        pub const PSSetConstantBuffers: usize = 16;
        pub const IASetInputLayout: usize = 17;
        pub const IASetVertexBuffers: usize = 18;
        pub const IASetIndexBuffer: usize = 19;
        pub const DrawIndexedInstanced: usize = 20;
        pub const DrawInstanced: usize = 21;
        pub const GSSetConstantBuffers: usize = 22;
        pub const GSSetShader: usize = 23;
        pub const IASetPrimitiveTopology: usize = 24;
        pub const VSSetShaderResources: usize = 25;
        pub const VSSetSamplers: usize = 26;
        pub const Begin: usize = 27;
        pub const End: usize = 28;
        pub const GetData: usize = 29;
        pub const SetPredication: usize = 30;
        pub const GSSetShaderResources: usize = 31;
        pub const GSSetSamplers: usize = 32;
        pub const OMSetRenderTargets: usize = 33;
        pub const OMSetRenderTargetsAndUnorderedAccessViews: usize = 34;
        pub const OMSetBlendState: usize = 35;
        pub const OMSetDepthStencilState: usize = 36;
        pub const SOSetTargets: usize = 37;
        pub const DrawAuto: usize = 38;
        pub const DrawIndexedInstancedIndirect: usize = 39;
        pub const DrawInstancedIndirect: usize = 40;
        pub const Dispatch: usize = 41;
        pub const DispatchIndirect: usize = 42;
        pub const RSSetState: usize = 43;
        pub const RSSetViewports: usize = 44;
        pub const RSSetScissorRects: usize = 45;
        pub const CopySubresourceRegion: usize = 46;
        pub const CopyResource: usize = 47;
        pub const UpdateSubresource: usize = 48;
        pub const CopyStructureCount: usize = 49;
        pub const ClearRenderTargetView: usize = 50;
        pub const ClearUnorderedAccessViewUint: usize = 51;
        pub const ClearUnorderedAccessViewFloat: usize = 52;
        pub const ClearDepthStencilView: usize = 53;
        pub const GenerateMips: usize = 54;
        pub const SetResourceMinLOD: usize = 55;
        pub const GetResourceMinLOD: usize = 56;
        pub const ResolveSubresource: usize = 57;
        pub const ExecuteCommandList: usize = 58;
        pub const HSSetShaderResources: usize = 59;
        pub const HSSetShader: usize = 60;
        pub const HSSetSamplers: usize = 61;
        pub const HSSetConstantBuffers: usize = 62;
        pub const DSSetShaderResources: usize = 63;
        pub const DSSetShader: usize = 64;
        pub const DSSetSamplers: usize = 65;
        pub const DSSetConstantBuffers: usize = 66;
        pub const CSSetShaderResources: usize = 67;
        pub const CSSetUnorderedAccessViews: usize = 68;
        pub const CSSetShader: usize = 69;
        pub const CSSetSamplers: usize = 70;
        pub const CSSetConstantBuffers: usize = 71;
        pub const VSGetConstantBuffers: usize = 72;
        pub const PSGetShaderResources: usize = 73;
        pub const PSGetShader: usize = 74;
        pub const PSGetSamplers: usize = 75;
        pub const VSGetShader: usize = 76;
        pub const PSGetConstantBuffers: usize = 77;
        pub const IAGetInputLayout: usize = 78;
        pub const IAGetVertexBuffers: usize = 79;
        pub const IAGetIndexBuffer: usize = 80;
        pub const GSGetConstantBuffers: usize = 81;
        pub const GSGetShader: usize = 82;
        pub const IAGetPrimitiveTopology: usize = 83;
        pub const VSGetShaderResources: usize = 84;
        pub const VSGetSamplers: usize = 85;
        pub const GetPredication: usize = 86;
        pub const GSGetShaderResources: usize = 87;
        pub const GSGetSamplers: usize = 88;
        pub const OMGetRenderTargets: usize = 89;
        pub const OMGetRenderTargetsAndUnorderedAccessViews: usize = 90;
        pub const OMGetBlendState: usize = 91;
        pub const OMGetDepthStencilState: usize = 92;
        pub const SOGetTargets: usize = 93;
        pub const RSGetState: usize = 94;
        pub const RSGetViewports: usize = 95;
        pub const RSGetScissorRects: usize = 96;
        pub const HSGetShaderResources: usize = 97;
        pub const HSGetShader: usize = 98;
        pub const HSGetSamplers: usize = 99;
        pub const HSGetConstantBuffers: usize = 100;
        pub const DSGetShaderResources: usize = 101;
        pub const DSGetShader: usize = 102;
        pub const DSGetSamplers: usize = 103;
        pub const DSGetConstantBuffers: usize = 104;
        pub const CSGetShaderResources: usize = 105;
        pub const CSGetUnorderedAccessViews: usize = 106;
        pub const CSGetShader: usize = 107;
        pub const CSGetSamplers: usize = 108;
        pub const CSGetConstantBuffers: usize = 109;
        pub const ClearState: usize = 110;
        pub const Flush: usize = 111;
        pub const GetType: usize = 112;
        pub const GetContextFlags: usize = 113;
        pub const FinishCommandList: usize = 114;
        pub const COUNT: usize = 115;
    }
    pub mod ID3D11Texture2D {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const GetDevice: usize = 3;
        pub const GetPrivateData: usize = 4;
        pub const SetPrivateData: usize = 5;
        pub const SetPrivateDataInterface: usize = 6;
        pub const GetType: usize = 7;
        pub const SetEvictionPriority: usize = 8;
        pub const GetEvictionPriority: usize = 9;
        pub const GetDesc: usize = 10;
        pub const COUNT: usize = 11;
    }
    pub mod IDCompositionDesktopDevice {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const Commit: usize = 3;
        pub const WaitForCommitCompletion: usize = 4;
        pub const GetFrameStatistics: usize = 5;
        pub const CreateVisual: usize = 6;
        pub const CreateSurfaceFactory: usize = 7;
        pub const CreateSurface: usize = 8;
        pub const CreateVirtualSurface: usize = 9;
        pub const CreateTranslateTransform: usize = 10;
        pub const CreateScaleTransform: usize = 11;
        pub const CreateRotateTransform: usize = 12;
        pub const CreateSkewTransform: usize = 13;
        pub const CreateMatrixTransform: usize = 14;
        pub const CreateTransformGroup: usize = 15;
        pub const CreateTranslateTransform3D: usize = 16;
        pub const CreateScaleTransform3D: usize = 17;
        pub const CreateRotateTransform3D: usize = 18;
        pub const CreateMatrixTransform3D: usize = 19;
        pub const CreateTransform3DGroup: usize = 20;
        pub const CreateEffectGroup: usize = 21;
        pub const CreateRectangleClip: usize = 22;
        pub const CreateAnimation: usize = 23;
        pub const CreateTargetForHwnd: usize = 24;
        pub const CreateSurfaceFromHandle: usize = 25;
        pub const CreateSurfaceFromHwnd: usize = 26;
        pub const COUNT: usize = 27;
    }
    pub mod IDCompositionDevice {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const Commit: usize = 3;
        pub const WaitForCommitCompletion: usize = 4;
        pub const GetFrameStatistics: usize = 5;
        pub const CreateTargetForHwnd: usize = 6;
        pub const CreateVisual: usize = 7;
        pub const CreateSurface: usize = 8;
        pub const CreateVirtualSurface: usize = 9;
        pub const CreateSurfaceFromHandle: usize = 10;
        pub const CreateSurfaceFromHwnd: usize = 11;
        pub const CreateTranslateTransform: usize = 12;
        pub const CreateScaleTransform: usize = 13;
        pub const CreateRotateTransform: usize = 14;
        pub const CreateSkewTransform: usize = 15;
        pub const CreateMatrixTransform: usize = 16;
        pub const CreateTransformGroup: usize = 17;
        pub const CreateTranslateTransform3D: usize = 18;
        pub const CreateScaleTransform3D: usize = 19;
        pub const CreateRotateTransform3D: usize = 20;
        pub const CreateMatrixTransform3D: usize = 21;
        pub const CreateTransform3DGroup: usize = 22;
        pub const CreateEffectGroup: usize = 23;
        pub const CreateRectangleClip: usize = 24;
        pub const CreateAnimation: usize = 25;
        pub const CheckDeviceState: usize = 26;
        pub const COUNT: usize = 27;
    }
    pub mod IDCompositionDevice2 {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const Commit: usize = 3;
        pub const WaitForCommitCompletion: usize = 4;
        pub const GetFrameStatistics: usize = 5;
        pub const CreateVisual: usize = 6;
        pub const CreateSurfaceFactory: usize = 7;
        pub const CreateSurface: usize = 8;
        pub const CreateVirtualSurface: usize = 9;
        pub const CreateTranslateTransform: usize = 10;
        pub const CreateScaleTransform: usize = 11;
        pub const CreateRotateTransform: usize = 12;
        pub const CreateSkewTransform: usize = 13;
        pub const CreateMatrixTransform: usize = 14;
        pub const CreateTransformGroup: usize = 15;
        pub const CreateTranslateTransform3D: usize = 16;
        pub const CreateScaleTransform3D: usize = 17;
        pub const CreateRotateTransform3D: usize = 18;
        pub const CreateMatrixTransform3D: usize = 19;
        pub const CreateTransform3DGroup: usize = 20;
        pub const CreateEffectGroup: usize = 21;
        pub const CreateRectangleClip: usize = 22;
        pub const CreateAnimation: usize = 23;
        pub const COUNT: usize = 24;
    }
    pub mod IDCompositionTarget {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetRoot: usize = 3;
        pub const COUNT: usize = 4;
    }
    pub mod IDCompositionVisual {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetOffsetX: usize = 3;
        pub const SetOffsetX2: usize = 4;
        pub const SetOffsetY: usize = 5;
        pub const SetOffsetY2: usize = 6;
        pub const SetTransform: usize = 7;
        pub const SetTransform2: usize = 8;
        pub const SetTransformParent: usize = 9;
        pub const SetEffect: usize = 10;
        pub const SetBitmapInterpolationMode: usize = 11;
        pub const SetBorderMode: usize = 12;
        pub const SetClip: usize = 13;
        pub const SetClip2: usize = 14;
        pub const SetContent: usize = 15;
        pub const AddVisual: usize = 16;
        pub const RemoveVisual: usize = 17;
        pub const RemoveAllVisuals: usize = 18;
        pub const SetCompositeMode: usize = 19;
        pub const COUNT: usize = 20;
    }
    pub mod IDXGIAdapter1 {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const EnumOutputs: usize = 7;
        pub const GetDesc: usize = 8;
        pub const CheckInterfaceSupport: usize = 9;
        pub const GetDesc1: usize = 10;
        pub const COUNT: usize = 11;
    }
    pub mod IDXGIDevice {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const GetAdapter: usize = 7;
        pub const CreateSurface: usize = 8;
        pub const QueryResourceResidency: usize = 9;
        pub const SetGPUThreadPriority: usize = 10;
        pub const GetGPUThreadPriority: usize = 11;
        pub const COUNT: usize = 12;
    }
    pub mod IDXGIFactory2 {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const EnumAdapters: usize = 7;
        pub const MakeWindowAssociation: usize = 8;
        pub const GetWindowAssociation: usize = 9;
        pub const CreateSwapChain: usize = 10;
        pub const CreateSoftwareAdapter: usize = 11;
        pub const EnumAdapters1: usize = 12;
        pub const IsCurrent: usize = 13;
        pub const IsWindowedStereoEnabled: usize = 14;
        pub const CreateSwapChainForHwnd: usize = 15;
        pub const CreateSwapChainForCoreWindow: usize = 16;
        pub const GetSharedResourceAdapterLuid: usize = 17;
        pub const RegisterStereoStatusWindow: usize = 18;
        pub const RegisterStereoStatusEvent: usize = 19;
        pub const UnregisterStereoStatus: usize = 20;
        pub const RegisterOcclusionStatusWindow: usize = 21;
        pub const RegisterOcclusionStatusEvent: usize = 22;
        pub const UnregisterOcclusionStatus: usize = 23;
        pub const CreateSwapChainForComposition: usize = 24;
        pub const COUNT: usize = 25;
    }
    pub mod IDXGIObject {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const COUNT: usize = 7;
    }
    pub mod IDXGISwapChain {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const GetDevice: usize = 7;
        pub const Present: usize = 8;
        pub const GetBuffer: usize = 9;
        pub const SetFullscreenState: usize = 10;
        pub const GetFullscreenState: usize = 11;
        pub const GetDesc: usize = 12;
        pub const ResizeBuffers: usize = 13;
        pub const ResizeTarget: usize = 14;
        pub const GetContainingOutput: usize = 15;
        pub const GetFrameStatistics: usize = 16;
        pub const GetLastPresentCount: usize = 17;
        pub const COUNT: usize = 18;
    }
}
use slots::*;
const IID_ID3D11Multithread: GUID =
    GUID { d1: 0x9b7e4e00, d2: 0x342c, d3: 0x4106, d4: [0xa1, 0x9f, 0x4f, 0x27, 0x04, 0xf6, 0x89, 0xf0] };
const IID_IDCompositionDesktopDevice: GUID =
    GUID { d1: 0x5f4633fe, d2: 0x1e08, d3: 0x4cb8, d4: [0x8c, 0x75, 0xce, 0x24, 0x33, 0x3f, 0x56, 0x02] };
const IID_IDCompositionDevice: GUID =
    GUID { d1: 0xc37ea93a, d2: 0xe7aa, d3: 0x450d, d4: [0xb1, 0x6f, 0x97, 0x46, 0xcb, 0x04, 0x07, 0xf3] };
const IID_IDCompositionDevice4: GUID =
    GUID { d1: 0x85fc5cca, d2: 0x2da6, d3: 0x494c, d4: [0x86, 0xb6, 0x4a, 0x77, 0x5c, 0x04, 0x9b, 0x8a] };
const IID_IDXGIDevice: GUID =
    GUID { d1: 0x54ec77fa, d2: 0x1377, d3: 0x44e6, d4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c] };
const IID_IDXGIFactory2: GUID =
    GUID { d1: 0x50c83a1c, d2: 0xe072, d3: 0x4c48, d4: [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0] };

// ---- Direct3D 11: exports, descriptions, helpers (d3d11.h, d3dcommon.h, dxgiformat.h) ----

#[link(name = "d3d11", kind = "raw-dylib")]
unsafe extern "system" {
    fn D3D11CreateDevice(
        adapter: *mut c_void,
        driver_type: u32,
        software: *mut c_void,
        flags: u32,
        levels: *const u32,
        n_levels: u32,
        sdk: u32,
        device: *mut *mut c_void,
        level: *mut u32,
        context: *mut *mut c_void,
    ) -> HRESULT;
    fn D3D11CreateDeviceAndSwapChain(
        adapter: *mut c_void,
        driver_type: u32,
        software: *mut c_void,
        flags: u32,
        levels: *const u32,
        n_levels: u32,
        sdk: u32,
        sc_desc: *const c_void,
        swap_chain: *mut *mut c_void,
        device: *mut *mut c_void,
        level: *mut u32,
        context: *mut *mut c_void,
    ) -> HRESULT;
}

const D3D11_SDK_VERSION: u32 = 7;
// D3D_DRIVER_TYPE
const D3D_DRIVER_TYPE_UNKNOWN: u32 = 0;
const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
const D3D_DRIVER_TYPE_REFERENCE: u32 = 2;
const D3D_DRIVER_TYPE_NULL: u32 = 3;
const D3D_DRIVER_TYPE_SOFTWARE: u32 = 4;
const D3D_DRIVER_TYPE_WARP: u32 = 5;
// D3D_FEATURE_LEVEL
const FL_9_1: u32 = 0x9100;
const FL_9_2: u32 = 0x9200;
const FL_9_3: u32 = 0x9300;
const FL_10_0: u32 = 0xa000;
const FL_10_1: u32 = 0xa100;
const FL_11_0: u32 = 0xb000;
const FL_11_1: u32 = 0xb100;
const FL_12_0: u32 = 0xc000;
const FL_12_1: u32 = 0xc100;
// D3D11_CREATE_DEVICE_FLAG
const CD_SINGLETHREADED: u32 = 1;
const CD_DEBUG: u32 = 2;
const CD_SWITCH_TO_REF: u32 = 4;
const CD_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS: u32 = 8;
const CD_BGRA_SUPPORT: u32 = 0x20;
const CD_DEBUGGABLE: u32 = 0x40;
const CD_VIDEO_SUPPORT: u32 = 0x800;
// DXGI_FORMAT
const F_UNKNOWN: u32 = 0;
const F_R32G32B32A32_FLOAT: u32 = 2;
const F_R16G16B16A16_FLOAT: u32 = 10;
const F_R16G16B16A16_UNORM: u32 = 11;
const F_R10G10B10A2_UNORM: u32 = 24;
const F_R8G8B8A8_TYPELESS: u32 = 27;
const F_R8G8B8A8_UNORM: u32 = 28;
const F_R8G8B8A8_UNORM_SRGB: u32 = 29;
const F_R16G16_UNORM: u32 = 35;
const F_D32_FLOAT: u32 = 40;
const F_R32_FLOAT: u32 = 41;
const F_D24_UNORM_S8_UINT: u32 = 45;
const F_R8G8_UNORM: u32 = 49;
const F_R16_UNORM: u32 = 56;
const F_R8_UNORM: u32 = 61;
const F_A8_UNORM: u32 = 65;
const F_BC1_UNORM: u32 = 71;
const F_B5G6R5_UNORM: u32 = 85;
const F_B8G8R8A8_UNORM: u32 = 87;
const F_B8G8R8X8_UNORM: u32 = 88;
const F_B8G8R8A8_UNORM_SRGB: u32 = 91;
const F_NV12: u32 = 103;
const F_P010: u32 = 104;
// D3D11_USAGE
const USAGE_DEFAULT: u32 = 0;
const USAGE_IMMUTABLE: u32 = 1;
const USAGE_DYNAMIC: u32 = 2;
const USAGE_STAGING: u32 = 3;
// D3D11_BIND_FLAG
const BIND_VERTEX_BUFFER: u32 = 1;
const BIND_INDEX_BUFFER: u32 = 2;
const BIND_CONSTANT_BUFFER: u32 = 4;
const BIND_SHADER_RESOURCE: u32 = 8;
const BIND_STREAM_OUTPUT: u32 = 0x10;
const BIND_RENDER_TARGET: u32 = 0x20;
const BIND_DEPTH_STENCIL: u32 = 0x40;
const BIND_UNORDERED_ACCESS: u32 = 0x80;
// D3D11_CPU_ACCESS_FLAG
const CPU_WRITE: u32 = 0x10000;
const CPU_READ: u32 = 0x20000;
// D3D11_RESOURCE_MISC_FLAG
const MISC_GENERATE_MIPS: u32 = 1;
const MISC_SHARED: u32 = 2;
const MISC_TEXTURECUBE: u32 = 4;
const MISC_SHARED_KEYEDMUTEX: u32 = 0x100;
const MISC_GDI_COMPATIBLE: u32 = 0x200;
const MISC_SHARED_NTHANDLE: u32 = 0x800;
// D3D11_MAP
const MAP_READ: u32 = 1;
const MAP_WRITE: u32 = 2;
const MAP_READ_WRITE: u32 = 3;
const MAP_WRITE_DISCARD: u32 = 4;
const MAP_WRITE_NO_OVERWRITE: u32 = 5;
const MAP_FLAG_DO_NOT_WAIT: u32 = 0x100000;
// D3D11_RESOURCE_DIMENSION
const DIM_BUFFER: u32 = 1;
const DIM_TEXTURE2D: u32 = 3;

/// D3D11_MAPPED_SUBRESOURCE
#[repr(C)]
#[derive(Clone, Copy)]
struct Mapped {
    data: *mut u8,
    row_pitch: u32,
    depth_pitch: u32,
}

/// D3D11_SUBRESOURCE_DATA
#[repr(C)]
#[derive(Clone, Copy)]
struct SubData {
    mem: *const u8,
    pitch: u32,
    slice_pitch: u32,
}

/// D3D11_TEXTURE2D_DESC as 11 words: Width, Height, MipLevels, ArraySize, Format, SampleDesc.Count,
/// SampleDesc.Quality, Usage, BindFlags, CPUAccessFlags, MiscFlags.
type Tex2D = [u32; 11];

fn tex_desc(w: u32, h: u32, format: u32, usage: u32, bind: u32, cpu: u32) -> Tex2D {
    [w, h, 1, 1, format, 1, 0, usage, bind, cpu, 0]
}

/// A device and its immediate context (both null when creation failed).
#[derive(Clone, Copy)]
struct Dev {
    dev: *mut c_void,
    ctx: *mut c_void,
    level: u32,
}

/// `D3D11CreateDevice(NULL, WARP, ...)`: the HRESULT and the device.
fn warp_with(flags: u32, levels: &[u32]) -> (HRESULT, Dev) {
    let mut d = Dev { dev: core::ptr::null_mut(), ctx: core::ptr::null_mut(), level: 0 };
    // SAFETY: outputs are valid for the call; `levels` is a live array.
    let hr = unsafe {
        D3D11CreateDevice(
            core::ptr::null_mut(),
            D3D_DRIVER_TYPE_WARP,
            core::ptr::null_mut(),
            flags,
            if levels.is_empty() { core::ptr::null() } else { levels.as_ptr() },
            levels.len() as u32,
            D3D11_SDK_VERSION,
            &mut d.dev,
            &mut d.level,
            &mut d.ctx,
        )
    };
    (hr, d)
}

fn warp() -> Dev {
    warp_with(CD_BGRA_SUPPORT, &[]).1
}

fn dev_release(d: Dev) {
    release(d.ctx);
    release(d.dev);
}

/// ID3D11Device::CreateTexture2D(desc, init, &tex)
fn create_tex2d(d: Dev, desc: &Tex2D, init: Option<&[SubData]>) -> (HRESULT, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: `desc` and `init` are live for the call.
    let hr = unsafe {
        vcall!(
            d.dev,
            ID3D11Device::CreateTexture2D,
            fn(*const Tex2D, *const SubData, *mut *mut c_void) -> HRESULT,
            desc,
            init.map_or(core::ptr::null(), |i| i.as_ptr()),
            &mut out
        )
    };
    (hr, out)
}

/// ID3D11Device::CreateBuffer(desc, init, &buf); desc = ByteWidth, Usage, BindFlags, CPUAccessFlags, MiscFlags, StructureByteStride
fn create_buffer(d: Dev, desc: &[u32; 6], init: Option<&SubData>) -> (HRESULT, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: `desc` and `init` are live for the call.
    let hr = unsafe {
        vcall!(
            d.dev,
            ID3D11Device::CreateBuffer,
            fn(*const [u32; 6], *const SubData, *mut *mut c_void) -> HRESULT,
            desc,
            init.map_or(core::ptr::null(), |i| i as *const SubData),
            &mut out
        )
    };
    (hr, out)
}

/// ID3D11Texture2D::GetDesc
fn get_desc(tex: *mut c_void) -> Tex2D {
    let mut d: Tex2D = [0xAAAA_AAAA; 11];
    // SAFETY: a live texture; `d` is valid.
    unsafe { vcall!(tex, ID3D11Texture2D::GetDesc, fn(*mut Tex2D) -> (), &mut d) };
    d
}

/// ID3D11DeviceContext::Map(res, sub, type, flags, &mapped)
fn map(d: Dev, res: *mut c_void, sub: u32, kind: u32, flags: u32) -> (HRESULT, Mapped) {
    let mut m = Mapped { data: core::ptr::null_mut(), row_pitch: 0xAAAA_AAAA, depth_pitch: 0xAAAA_AAAA };
    // SAFETY: live context and resource; `m` is valid.
    let hr = unsafe {
        vcall!(
            d.ctx,
            ID3D11DeviceContext::Map,
            fn(*mut c_void, u32, u32, u32, *mut Mapped) -> HRESULT,
            res,
            sub,
            kind,
            flags,
            &mut m
        )
    };
    (hr, m)
}

fn unmap(d: Dev, res: *mut c_void, sub: u32) {
    // SAFETY: live context and resource.
    unsafe { vcall!(d.ctx, ID3D11DeviceContext::Unmap, fn(*mut c_void, u32) -> (), res, sub) }
}

/// A staging texture of the given size and format with CPU read and write access.
fn staging(d: Dev, w: u32, h: u32, format: u32) -> *mut c_void {
    create_tex2d(d, &tex_desc(w, h, format, USAGE_STAGING, 0, CPU_READ | CPU_WRITE), None).1
}

/// Fill a mapped 32-bit-per-pixel staging texture with `f(x, y)`.
fn fill32(d: Dev, tex: *mut c_void, w: u32, h: u32, f: &dyn Fn(u32, u32) -> u32) -> HRESULT {
    let (hr, m) = map(d, tex, 0, MAP_WRITE, 0);
    if hr < 0 {
        return hr;
    }
    for y in 0..h {
        for x in 0..w {
            // SAFETY: inside the mapped rows.
            unsafe { *(m.data.add((y * m.row_pitch + x * 4) as usize) as *mut u32) = f(x, y) };
        }
    }
    unmap(d, tex, 0);
    hr
}

/// The 32-bit pixel at (x, y) of a staging texture (read through Map); 0xFFFF_FFFF - 1 if the map failed.
fn pixel32(d: Dev, tex: *mut c_void, x: u32, y: u32) -> u32 {
    let (hr, m) = map(d, tex, 0, MAP_READ, 0);
    if hr < 0 {
        return 0xFFFF_FFFE;
    }
    // SAFETY: inside the mapped rows.
    let v = unsafe { *(m.data.add((y * m.row_pitch + x * 4) as usize) as *const u32) };
    unmap(d, tex, 0);
    v
}

/// Bytes per pixel of a few formats.
fn bpp(format: u32) -> u32 {
    match format {
        F_R32G32B32A32_FLOAT => 16,
        F_R16G16B16A16_FLOAT | F_R16G16B16A16_UNORM => 8,
        F_R8_UNORM | F_A8_UNORM => 1,
        F_R8G8_UNORM | F_R16_UNORM | F_B5G6R5_UNORM => 2,
        _ => 4,
    }
}

// ---- DXGI: exports and helpers (dxgi.h .. dxgi1_6.h) ----

#[link(name = "dxgi", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateDXGIFactory(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn CreateDXGIFactory1(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn CreateDXGIFactory2(flags: u32, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
}

// DXGI_USAGE
const USAGE_SHADER_INPUT: u32 = 0x10;
const USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
const USAGE_BACK_BUFFER: u32 = 0x40;
const USAGE_SHARED: u32 = 0x80;
const USAGE_READ_ONLY: u32 = 0x100;
// DXGI_SWAP_EFFECT
const SWAP_DISCARD: u32 = 0;
const SWAP_SEQUENTIAL: u32 = 1;
const SWAP_FLIP_SEQUENTIAL: u32 = 3;
const SWAP_FLIP_DISCARD: u32 = 4;
// DXGI_SCALING
const SCALING_STRETCH: u32 = 0;
const SCALING_NONE: u32 = 1;
const SCALING_ASPECT_RATIO_STRETCH: u32 = 2;
// DXGI_ALPHA_MODE
const ALPHA_UNSPECIFIED: u32 = 0;
const ALPHA_PREMULTIPLIED: u32 = 1;
const ALPHA_STRAIGHT: u32 = 2;
const ALPHA_IGNORE: u32 = 3;
// DXGI_SWAP_CHAIN_FLAG
const SC_FLAG_ALLOW_MODE_SWITCH: u32 = 2;
const SC_FLAG_GDI_COMPATIBLE: u32 = 4;
const SC_FLAG_FRAME_LATENCY_WAITABLE_OBJECT: u32 = 0x40;
const SC_FLAG_ALLOW_TEARING: u32 = 0x800;
// DXGI_PRESENT
const PRESENT_TEST: u32 = 0x1;
const PRESENT_DO_NOT_SEQUENCE: u32 = 0x2;
const PRESENT_RESTART: u32 = 0x4;
const PRESENT_DO_NOT_WAIT: u32 = 0x8;
const PRESENT_ALLOW_TEARING: u32 = 0x200;

/// DXGI_SWAP_CHAIN_DESC1 as 12 words: Width, Height, Format, Stereo, SampleDesc.Count, SampleDesc.Quality, BufferUsage,
/// BufferCount, Scaling, SwapEffect, AlphaMode, Flags.
type Sc1 = [u32; 12];

fn sc1(w: u32, h: u32, format: u32, buffers: u32, effect: u32) -> Sc1 {
    [w, h, format, 0, 1, 0, USAGE_RENDER_TARGET_OUTPUT, buffers, SCALING_STRETCH, effect, ALPHA_UNSPECIFIED, 0]
}

/// CreateDXGIFactory1(iid): the HRESULT and the factory.
fn factory1(iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut f: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { CreateDXGIFactory1(iid, &mut f) };
    (hr, f)
}

/// The IDXGIDevice of a D3D11 device (a new reference).
fn dxgi_device(d: Dev) -> *mut c_void {
    qi(d.dev, &IID_IDXGIDevice).1
}

/// The adapter of a D3D11 device through IDXGIDevice::GetAdapter (a new reference).
fn adapter_of(d: Dev) -> *mut c_void {
    let x = dxgi_device(d);
    let mut a: *mut c_void = core::ptr::null_mut();
    if !x.is_null() {
        // SAFETY: a live IDXGIDevice.
        unsafe { vcall!(x, IDXGIDevice::GetAdapter, fn(*mut *mut c_void) -> HRESULT, &mut a) };
        release(x);
    }
    a
}

/// The factory of an adapter through IDXGIObject::GetParent(IDXGIFactory2) (a new reference).
fn factory_of_adapter(a: *mut c_void) -> *mut c_void {
    let mut f: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live adapter.
    unsafe {
        vcall!(a, IDXGIObject::GetParent, fn(*const GUID, *mut *mut c_void) -> HRESULT, &IID_IDXGIFactory2, &mut f)
    };
    f
}

/// IDXGIAdapter1::GetDesc1 into 312 bytes: description at 0 (128 UTF-16 units), VendorId 256, DeviceId 260,
/// SubSysId 264, Revision 268, DedicatedVideoMemory 272, DedicatedSystemMemory 280, SharedSystemMemory 288,
/// AdapterLuid 296 (low u32, high i32), Flags 304.
fn adapter_desc1(a: *mut c_void) -> (HRESULT, [u8; 312]) {
    let mut b = [0xAAu8; 312];
    // SAFETY: a live IDXGIAdapter1; `b` is large enough for DXGI_ADAPTER_DESC1.
    let hr = unsafe { vcall!(a, IDXGIAdapter1::GetDesc1, fn(*mut u8) -> HRESULT, b.as_mut_ptr()) };
    (hr, b)
}

fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn rd_u64(b: &[u8], off: usize) -> u64 {
    (rd_u32(b, off + 4) as u64) << 32 | rd_u32(b, off) as u64
}

/// The adapter description text of a DXGI_ADAPTER_DESC(1) buffer.
fn adapter_name(b: &[u8]) -> Buf<160> {
    let mut u = [0u16; 128];
    for i in 0..128 {
        u[i] = u16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
    }
    utf16_text::<160>(&u)
}

// ---- a window for swap chains, and the few Win32 calls the tests need ----

#[repr(C)]
struct WNDCLASSW {
    style: u32,
    wnd_proc: unsafe extern "system" fn(HANDLE, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HANDLE,
    icon: HANDLE,
    cursor: HANDLE,
    background: HANDLE,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassW(wc: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HANDLE,
        menu: HANDLE,
        instance: HANDLE,
        param: *mut c_void,
    ) -> HANDLE;
    fn DestroyWindow(h: HANDLE) -> BOOL;
    fn DefWindowProcW(h: HANDLE, msg: u32, w: usize, l: isize) -> isize;
    fn ShowWindow(h: HANDLE, cmd: i32) -> BOOL;
    fn GetClientRect(h: HANDLE, r: *mut RECT) -> BOOL;
    fn GetDC(h: HANDLE) -> HANDLE;
    fn ReleaseDC(h: HANDLE, dc: HANDLE) -> i32;
    fn WindowFromDC(dc: HANDLE) -> HANDLE;
    fn IsWindow(h: HANDLE) -> BOOL;
    fn PeekMessageW(msg: *mut c_void, h: HANDLE, min: u32, max: u32, remove: u32) -> BOOL;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
    fn Sleep(ms: u32);
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn CreateEventW(attrs: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
}

const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_EX_NOREDIRECTIONBITMAP: u32 = 0x0020_0000;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TRANSPARENT: u32 = 0x20;
const SW_SHOWNOACTIVATE: i32 = 4;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;

unsafe extern "system" fn wndproc(h: HANDLE, m: u32, w: usize, l: isize) -> isize {
    // SAFETY: the default window procedure.
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// A window of the test's own class (registered on first use): `style`, `ex_style`, client size roughly w x h.
fn make_window(class: &str, ex_style: u32, style: u32, w: i32, h: i32, parent: HANDLE) -> HANDLE {
    let cls = wide(class);
    // SAFETY: plain window-class and window creation with live arguments.
    unsafe {
        let inst = GetModuleHandleW(core::ptr::null());
        let wc = WNDCLASSW {
            style: 0,
            wnd_proc: wndproc,
            cls_extra: 0,
            wnd_extra: 0,
            instance: inst,
            icon: core::ptr::null_mut(),
            cursor: core::ptr::null_mut(),
            background: core::ptr::null_mut(),
            menu_name: core::ptr::null(),
            class_name: cls.as_ptr(),
        };
        RegisterClassW(&wc);
        let title = wide("poly d3d test");
        CreateWindowExW(
            ex_style,
            cls.as_ptr(),
            title.as_ptr(),
            style,
            10,
            10,
            w,
            h,
            parent,
            core::ptr::null_mut(),
            inst,
            core::ptr::null_mut(),
        )
    }
}

/// A plain top-level window (not shown) with a client area of about 160 x 120.
fn plain_window() -> HANDLE {
    make_window("PolyD3dWnd", 0, WS_OVERLAPPEDWINDOW, 176, 160, core::ptr::null_mut())
}

fn client_size(h: HANDLE) -> (i32, i32) {
    let mut r = RECT::default();
    // SAFETY: a live window; `r` is valid.
    unsafe { GetClientRect(h, &mut r) };
    (r.right - r.left, r.bottom - r.top)
}

fn destroy(h: HANDLE) {
    // SAFETY: a window of this thread.
    unsafe { DestroyWindow(h) };
}

// ---- DirectComposition: exports and helpers (dcomp.h) ----

#[link(name = "dcomp", kind = "raw-dylib")]
unsafe extern "system" {
    fn DCompositionCreateDevice(dxgi_device: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn DCompositionCreateDevice2(rendering_device: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn DCompositionCreateDevice3(rendering_device: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn DCompositionCreateSurfaceHandle(access: u32, attrs: *const c_void, out: *mut HANDLE) -> HRESULT;
}

// dcomptypes.h
const DCOMP_INTERP_NEAREST: i32 = 0;
const DCOMP_INTERP_LINEAR: i32 = 1;
const DCOMP_INHERIT: i32 = -1; // 0xffffffff in every one of the enums below
const DCOMP_BORDER_SOFT: i32 = 0;
const DCOMP_BORDER_HARD: i32 = 1;
const DCOMP_COMPOSITE_SOURCE_OVER: i32 = 0;
const DCOMP_COMPOSITE_DESTINATION_INVERT: i32 = 1;
const DCOMP_COMPOSITE_MIN_BLEND: i32 = 2;
const DCOMP_OPACITY_LAYER: i32 = 0;
const DCOMP_OPACITY_MULTIPLY: i32 = 1;

// IDCompositionVisual vtable: the methods with two overloads take two consecutive slots. The order of the slots was
// observed with the calls of the test `dcvis_slots` (the animation overload comes FIRST, the value overload second: the
// reverse of the declaration order in dcomp.h). Names: V_<method> = value/matrix/rect/object overload that is not the animation.
const V_SET_OFFSET_X_ANIM: usize = 3;
const V_SET_OFFSET_X: usize = 4;
const V_SET_OFFSET_Y_ANIM: usize = 5;
const V_SET_OFFSET_Y: usize = 6;
const V_SET_TRANSFORM_OBJ: usize = 7;
const V_SET_TRANSFORM_MATRIX: usize = 8;
const V_SET_TRANSFORM_PARENT: usize = 9;
const V_SET_EFFECT: usize = 10;
const V_SET_BITMAP_INTERP: usize = 11;
const V_SET_BORDER_MODE: usize = 12;
const V_SET_CLIP_OBJ: usize = 13;
const V_SET_CLIP_RECT: usize = 14;
const V_SET_CONTENT: usize = 15;
const V_ADD_VISUAL: usize = 16;
const V_REMOVE_VISUAL: usize = 17;
const V_REMOVE_ALL: usize = 18;
const V_SET_COMPOSITE_MODE: usize = 19;
// IDCompositionVisual2
const V2_SET_OPACITY_MODE: usize = 20;
const V2_SET_BACKFACE: usize = 21;

/// Calls the COM method at vtable `slot` of `obj` with one `f32` argument and a NULL pointer in the register
/// the pointer overload would read (a value overload reads the float, an object overload reads the pointer).
fn call_float_and_null(obj: *mut c_void, slot: usize, v: f32) -> HRESULT {
    let ret: i32;
    // SAFETY: `obj` is a live COM object; the function at `slot` takes `this` and one argument that is either a float
    // (xmm1) or a pointer (rdx): both registers are set, the stack is aligned and has the 32-byte shadow space.
    unsafe {
        let f = *(*(obj as *const *const usize)).add(slot);
        core::arch::asm!(
            "mov r12, rsp",
            "and rsp, -16",
            "sub rsp, 32",
            "call {f}",
            "mov rsp, r12",
            f = in(reg) f,
            in("rcx") obj,
            in("rdx") 0usize,
            in("xmm1") v,
            out("r12") _,
            lateout("rax") ret,
            clobber_abi("system"),
        );
    }
    ret
}

/// Calls the COM method at vtable `slot` of `obj` with the integer-register arguments (rdx, r8, r9) and the float
/// registers (xmm1, xmm2, xmm3) all set: whichever arguments the method declares (an int, a pointer, a float, in that
/// position) are read from the matching register.
fn call_regs(obj: *mut c_void, slot: usize, i1: usize, i2: usize, i3: usize, f1: f32, f2: f32, f3: f32) -> HRESULT {
    let ret: i32;
    // SAFETY: `obj` is a live COM object; the stack is aligned and has the 32-byte shadow space; the caller passes
    // values that are valid for the argument types of the method in each position (NULL or small integers).
    unsafe {
        let f = *(*(obj as *const *const usize)).add(slot);
        core::arch::asm!(
            "mov r12, rsp",
            "and rsp, -16",
            "sub rsp, 32",
            "call {f}",
            "mov rsp, r12",
            f = in(reg) f,
            in("rcx") obj,
            in("rdx") i1,
            in("r8") i2,
            in("r9") i3,
            in("xmm1") f1,
            in("xmm2") f2,
            in("xmm3") f3,
            out("r12") _,
            lateout("rax") ret,
            clobber_abi("system"),
        );
    }
    ret
}

/// DCompositionCreateDevice3(device, iid): the HRESULT and the object (NULL on failure).
fn dc3(rendering: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { DCompositionCreateDevice3(rendering, iid, &mut o) };
    (hr, o)
}

fn dc2(rendering: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { DCompositionCreateDevice2(rendering, iid, &mut o) };
    (hr, o)
}

fn dc1(dxgi: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { DCompositionCreateDevice(dxgi, iid, &mut o) };
    (hr, o)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(
        attrs: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
}

/// Runs `f(param)` on a new thread and waits for it; the thread's return value.
fn on_thread(f: unsafe extern "system" fn(*mut c_void) -> u32, param: *mut c_void) -> u32 {
    // SAFETY: plain thread creation; the thread is joined before returning.
    unsafe {
        let h = CreateThread(core::ptr::null(), 0, f, param, 0, core::ptr::null_mut());
        if h.is_null() {
            return 0xdead;
        }
        WaitForSingleObject(h, 20000);
        let mut code = 0u32;
        GetExitCodeThread(h, &mut code);
        CloseHandle(h);
        code
    }
}

// IDCompositionVisual3 (the animation overload comes first, as in IDCompositionVisual)
const V3_SET_DEPTH_MODE: usize = 26;
const V3_SET_OFFSET_Z_ANIM: usize = 27;
const V3_SET_OFFSET_Z: usize = 28;
const V3_SET_OPACITY_ANIM: usize = 29;
const V3_SET_OPACITY: usize = 30;
const V3_SET_TRANSFORM_OBJ: usize = 31;
const V3_SET_TRANSFORM_MATRIX: usize = 32;
const V3_SET_VISIBLE: usize = 33;

/// A WARP D3D11 device, its IDXGIDevice and a desktop DirectComposition device (IDCompositionDesktopDevice, which is also
/// IDCompositionDevice2) made with DCompositionCreateDevice3.
#[derive(Clone, Copy)]
struct Dc {
    d: Dev,
    x: *mut c_void,
    dev: *mut c_void,
}

fn dc_setup() -> Dc {
    let d = warp_with(CD_BGRA_SUPPORT, &[FL_11_0]).1;
    let x = dxgi_device(d);
    let (_, dev) = dc3(x, &IID_IDCompositionDesktopDevice);
    Dc { d, x, dev }
}

fn dc_free(c: Dc) {
    release(c.dev);
    release(c.x);
    release(c.d.ctx);
    release(c.d.dev);
}

/// IDCompositionDevice2::CreateVisual on a device made by `dc_setup` (a new reference).
fn new_visual(dev: *mut c_void) -> *mut c_void {
    let mut v: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live device.
    unsafe { vcall!(dev, IDCompositionDevice2::CreateVisual, fn(*mut *mut c_void) -> HRESULT, &mut v) };
    v
}

/// IDCompositionDevice::CreateVisual (the v1 device's slot, which is 7, not 6).
fn new_visual_v1(dev: *mut c_void) -> *mut c_void {
    let mut v: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live device.
    unsafe { vcall!(dev, IDCompositionDevice::CreateVisual, fn(*mut *mut c_void) -> HRESULT, &mut v) };
    v
}

/// IDCompositionDesktopDevice::CreateTargetForHwnd.
fn new_target(dev: *mut c_void, hwnd: HANDLE, topmost: BOOL) -> (HRESULT, *mut c_void) {
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live device.
    let hr = unsafe {
        vcall!(
            dev,
            IDCompositionDesktopDevice::CreateTargetForHwnd,
            fn(HANDLE, BOOL, *mut *mut c_void) -> HRESULT,
            hwnd,
            topmost,
            &mut o
        )
    };
    (hr, o)
}

fn set_root(target: *mut c_void, visual: *mut c_void) -> HRESULT {
    // SAFETY: a live target.
    unsafe { vcall!(target, IDCompositionTarget::SetRoot, fn(*mut c_void) -> HRESULT, visual) }
}

fn commit(dev: *mut c_void) -> HRESULT {
    // SAFETY: a live device.
    unsafe { vcall!(dev, IDCompositionDevice::Commit, fn() -> HRESULT) }
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDesktopWindow() -> HANDLE;
    fn GetShellWindow() -> HANDLE;
    fn GetWindowThreadProcessId(h: HANDLE, pid: *mut u32) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetEvent(h: HANDLE) -> BOOL;
}

/// IDXGIFactory2::CreateSwapChainForComposition(device, desc, NULL output): the HRESULT and the swap chain.
fn comp_swapchain(factory: *mut c_void, device: *mut c_void, desc: &Sc1) -> (HRESULT, *mut c_void) {
    let mut sc: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live factory; the description has the 12 words of DXGI_SWAP_CHAIN_DESC1.
    let hr = unsafe {
        vcall!(
            factory,
            IDXGIFactory2::CreateSwapChainForComposition,
            fn(*mut c_void, *const Sc1, *mut c_void, *mut *mut c_void) -> HRESULT,
            device,
            desc,
            core::ptr::null_mut(),
            &mut sc
        )
    };
    (hr, sc)
}

/// IDXGISwapChain::Present.
fn present(sc: *mut c_void, interval: u32, flags: u32) -> HRESULT {
    // SAFETY: a live swap chain.
    unsafe { vcall!(sc, IDXGISwapChain::Present, fn(u32, u32) -> HRESULT, interval, flags) }
}

/// IDCompositionVisual::SetContent.
fn set_content(visual: *mut c_void, content: *mut c_void) -> HRESULT {
    // SAFETY: a live visual; the content pointer is an object or NULL.
    unsafe { vcall!(visual, V_SET_CONTENT, fn(*mut c_void) -> HRESULT, content) }
}

/// IDCompositionDevice::CreateTargetForHwnd of the v1 device (vtable slot 6; the desktop device has it at slot 24).
fn new_target_v1(dev: *mut c_void, hwnd: HANDLE, topmost: BOOL) -> (HRESULT, *mut c_void) {
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live v1 device.
    let hr = unsafe {
        vcall!(
            dev,
            IDCompositionDevice::CreateTargetForHwnd,
            fn(HANDLE, BOOL, *mut *mut c_void) -> HRESULT,
            hwnd,
            topmost,
            &mut o
        )
    };
    (hr, o)
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn QueryPerformanceCounter(v: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(v: *mut i64) -> BOOL;
}

poly_wtest::main!(test);

static mut RESULT: [i32; 8] = [0; 8];

unsafe extern "system" fn thread_create(_p: *mut c_void) -> u32 {
    let d = warp_with(CD_BGRA_SUPPORT, &[FL_11_0]).1;
    let x = dxgi_device(d);
    let (hr, dev) = dc3(x, &IID_IDCompositionDesktopDevice);
    let mut cm = -1;
    if !dev.is_null() {
        // SAFETY: a live device.
        cm = unsafe { vcall!(dev, IDCompositionDevice::Commit, fn() -> HRESULT) };
    }
    // SAFETY: only this thread writes RESULT while the test waits for it.
    unsafe {
        RESULT[0] = hr;
        RESULT[1] = cm;
    }
    release(dev);
    release(x);
    release(d.dev);
    release(d.ctx);
    0
}

fn test(t: &mut Test) {
    SAY_PART.store(2, core::sync::atomic::Ordering::Relaxed);
    let d = warp_with(CD_BGRA_SUPPORT, &[FL_11_0]).1;
    let x = dxgi_device(d);

    // the rendering device argument
    let (hr, p) = dc1(core::ptr::null_mut(), &IID_IDCompositionDevice);
    say!(t, "v1 NULL device: {}", res(hr, p).as_str());
    release(p);
    let (hr, p) = dc2(core::ptr::null_mut(), &IID_IDCompositionDevice);
    say!(t, "v2 NULL device, IDCompositionDevice: {}", res(hr, p).as_str());
    release(p);
    let (hr, p) = dc2(core::ptr::null_mut(), &IID_IDCompositionDesktopDevice);
    say!(t, "v2 NULL device, IDCompositionDesktopDevice: {}", res(hr, p).as_str());
    release(p);
    let (hr, p) = dc3(core::ptr::null_mut(), &IID_IDCompositionDesktopDevice);
    say!(t, "v3 NULL device, IDCompositionDesktopDevice: {}", res(hr, p).as_str());
    if !p.is_null() {
        // SAFETY: a live device.
        let c = unsafe { vcall!(p, IDCompositionDevice::Commit, fn() -> HRESULT) };
        let mut v: *mut c_void = core::ptr::null_mut();
        // SAFETY: a live device.
        let cv = unsafe { vcall!(p, IDCompositionDevice2::CreateVisual, fn(*mut *mut c_void) -> HRESULT, &mut v) };
        say!(t, "  device without rendering device: Commit {} CreateVisual {}", Hr(c), res(cv, v).as_str());
        release(v);
    }
    release(p);
    let (hr, p) = dc3(core::ptr::null_mut(), &IID_IDCompositionDevice4);
    say!(t, "v3 NULL device, IDCompositionDevice4: {}", res(hr, p).as_str());
    release(p);

    // the D3D11 device itself, its context, a factory, other objects
    let (_, unk) = qi(d.dev, &IID_IUnknown);
    let a = adapter_of(d);
    let f = factory_of_adapter(a);
    let (_, mt) = qi(d.dev, &IID_ID3D11Multithread);
    for (name, o) in [
        ("ID3D11Device", d.dev),
        ("IUnknown of the D3D11 device", unk),
        ("ID3D11DeviceContext", d.ctx),
        ("IDXGIAdapter", a),
        ("IDXGIFactory2", f),
        ("ID3D11Multithread", mt),
    ] {
        let (h1, p1) = dc1(o, &IID_IDCompositionDevice);
        let (h2, p2) = dc2(o, &IID_IDCompositionDevice);
        let (h3, p3) = dc3(o, &IID_IDCompositionDesktopDevice);
        say!(
            t,
            "{name} as the device: v1 {} v2 {} v3 {}",
            res(h1, p1).as_str(),
            res(h2, p2).as_str(),
            res(h3, p3).as_str()
        );
        release(p1);
        release(p2);
        release(p3);
    }
    release(mt);
    release(f);
    release(a);
    release(unk);

    // a D3D11 device object that is not an IDXGIDevice: a swap chain's COM object, a 2D texture
    let t2 = create_tex2d(d, &tex_desc(8, 8, F_B8G8R8A8_UNORM, USAGE_DEFAULT, BIND_SHADER_RESOURCE, 0), None).1;
    let (h3, p3) = dc3(t2, &IID_IDCompositionDesktopDevice);
    say!(t, "a texture as the device: v3 {}", res(h3, p3).as_str());
    release(p3);
    release(t2);

    // reference counts of the rendering device
    let before = (refs(x), refs(d.dev));
    let (hr, a1) = dc3(x, &IID_IDCompositionDesktopDevice);
    let during = (refs(x), refs(d.dev));
    say!(
        t,
        "after creation {}: IDXGIDevice refs grew {} D3D11 device refs grew {} same growth {}",
        Hr(hr),
        during.0 > before.0,
        during.1 > before.1,
        during.0 - before.0 == during.1 - before.1
    );
    let (_, a2) = dc3(x, &IID_IDCompositionDesktopDevice);
    say!(t, "after a second device: the growth doubled {}", refs(x) - before.0 == 2 * (during.0 - before.0));
    release(a1);
    say!(t, "after releasing the first: back to one device's growth {}", refs(x) - before.0 == during.0 - before.0);
    release(a2);
    say!(t, "after releasing both: IDXGIDevice refs back to the count before {}", refs(x) == before.0);
    // the D3D11 device with the same ids: the same through ID3D11Device (v3)
    let (hr, a3) = dc3(d.dev, &IID_IDCompositionDesktopDevice);
    say!(
        t,
        "from the ID3D11Device pointer: {} IDXGIDevice refs grew by the same as with the IDXGIDevice pointer {}",
        Hr(hr),
        refs(x) == during.0
    );
    release(a3);

    // the rendering device can go away before the DirectComposition device
    let (_, a4) = dc3(x, &IID_IDCompositionDesktopDevice);
    release(x);
    release(d.ctx);
    release(d.dev);
    // SAFETY: a live device.
    let c = unsafe { vcall!(a4, IDCompositionDevice::Commit, fn() -> HRESULT) };
    let mut v: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live device.
    let cv = unsafe { vcall!(a4, IDCompositionDevice2::CreateVisual, fn(*mut *mut c_void) -> HRESULT, &mut v) };
    say!(
        t,
        "after releasing every reference to the D3D11 device: Commit {} CreateVisual {}",
        Hr(c),
        res(cv, v).as_str()
    );
    release(v);
    release(a4);

    // a device without BGRA support, a single-threaded one
    for (name, flags) in
        [("no flags", 0u32), ("SINGLETHREADED", CD_SINGLETHREADED), ("VIDEO_SUPPORT", CD_VIDEO_SUPPORT)]
    {
        let (hrd, d2) = warp_with(flags, &[FL_11_0]);
        if hrd >= 0 {
            let x2 = dxgi_device(d2);
            let (hr, p) = dc3(x2, &IID_IDCompositionDesktopDevice);
            say!(t, "D3D11 device with {name}: create {}", res(hr, p).as_str());
            release(p);
            release(x2);
            release(d2.ctx);
            release(d2.dev);
        } else {
            say!(t, "D3D11 device with {name}: not created {}", Hr(hrd));
        }
    }

    // a thread without a message pump
    let code = on_thread(thread_create, core::ptr::null_mut());
    // SAFETY: the thread has finished.
    let (h, cm) = unsafe { (RESULT[0], RESULT[1]) };
    say!(
        t,
        "created and committed on another thread without a message loop: exit {code} create {} Commit {}",
        Hr(h),
        Hr(cm)
    );

    // a device made on this thread, used on another
    let d = warp_with(CD_BGRA_SUPPORT, &[FL_11_0]).1;
    let x = dxgi_device(d);
    let (_, a5) = dc3(x, &IID_IDCompositionDesktopDevice);
    unsafe extern "system" fn commit_there(p: *mut c_void) -> u32 {
        // SAFETY: a live device pointer passed by the test.
        let hr = unsafe { vcall!(p, IDCompositionDevice::Commit, fn() -> HRESULT,) };
        // SAFETY: only this thread writes RESULT while the test waits for it.
        unsafe { RESULT[2] = hr };
        0
    }
    on_thread(commit_there, a5);
    // SAFETY: the thread has finished.
    say!(t, "Commit on a device created by another thread: {}", Hr(unsafe { RESULT[2] }));
    release(a5);
    release(x);
    release(d.ctx);
    release(d.dev);
}
