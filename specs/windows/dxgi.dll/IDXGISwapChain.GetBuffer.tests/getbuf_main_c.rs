//! IDXGISwapChain::GetBuffer: valid indexes per swap effect, the interfaces that can be asked for, the texture's description,
//! reference counts, whether repeated calls return the same object, render target views of the buffers.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 3 of the observations, the others write the rest.)
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
    pub mod ID3D11DeviceChild {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const GetDevice: usize = 3;
        pub const GetPrivateData: usize = 4;
        pub const SetPrivateData: usize = 5;
        pub const SetPrivateDataInterface: usize = 6;
        pub const COUNT: usize = 7;
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
    pub mod IDXGIAdapter {
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
        pub const COUNT: usize = 10;
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
    pub mod IDXGIDeviceSubObject {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const GetDevice: usize = 7;
        pub const COUNT: usize = 8;
    }
    pub mod IDXGIFactory1 {
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
        pub const COUNT: usize = 14;
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
    pub mod IDXGIFactory4 {
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
        pub const GetCreationFlags: usize = 25;
        pub const EnumAdapterByLuid: usize = 26;
        pub const EnumWarpAdapter: usize = 27;
        pub const COUNT: usize = 28;
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
    pub mod IDXGIResource {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const GetDevice: usize = 7;
        pub const GetSharedHandle: usize = 8;
        pub const GetUsage: usize = 9;
        pub const SetEvictionPriority: usize = 10;
        pub const GetEvictionPriority: usize = 11;
        pub const COUNT: usize = 12;
    }
    pub mod IDXGISurface {
        pub const QueryInterface: usize = 0;
        pub const AddRef: usize = 1;
        pub const Release: usize = 2;
        pub const SetPrivateData: usize = 3;
        pub const SetPrivateDataInterface: usize = 4;
        pub const GetPrivateData: usize = 5;
        pub const GetParent: usize = 6;
        pub const GetDevice: usize = 7;
        pub const GetDesc: usize = 8;
        pub const Map: usize = 9;
        pub const Unmap: usize = 10;
        pub const COUNT: usize = 11;
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
    pub mod IDXGISwapChain1 {
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
        pub const GetDesc1: usize = 18;
        pub const GetFullscreenDesc: usize = 19;
        pub const GetHwnd: usize = 20;
        pub const GetCoreWindow: usize = 21;
        pub const Present1: usize = 22;
        pub const IsTemporaryMonoSupported: usize = 23;
        pub const GetRestrictToOutput: usize = 24;
        pub const SetBackgroundColor: usize = 25;
        pub const GetBackgroundColor: usize = 26;
        pub const SetRotation: usize = 27;
        pub const GetRotation: usize = 28;
        pub const COUNT: usize = 29;
    }
}
use slots::*;
const IID_ID3D11Buffer: GUID =
    GUID { d1: 0x48570b85, d2: 0xd1ee, d3: 0x4fcd, d4: [0xa2, 0x50, 0xeb, 0x35, 0x07, 0x22, 0xb0, 0x37] };
const IID_ID3D11Device: GUID =
    GUID { d1: 0xdb6f6ddb, d2: 0xac77, d3: 0x4e88, d4: [0x82, 0x53, 0x81, 0x9d, 0xf9, 0xbb, 0xf1, 0x40] };
const IID_ID3D11DeviceChild: GUID =
    GUID { d1: 0x1841e5c8, d2: 0x16b0, d3: 0x489b, d4: [0xbc, 0xc8, 0x44, 0xcf, 0xb0, 0xd5, 0xde, 0xae] };
const IID_ID3D11Resource: GUID =
    GUID { d1: 0xdc8e63f3, d2: 0xd12b, d3: 0x4952, d4: [0xb4, 0x7b, 0x5e, 0x45, 0x02, 0x6a, 0x86, 0x2d] };
const IID_ID3D11Texture1D: GUID =
    GUID { d1: 0xf8fb5c27, d2: 0xc6b3, d3: 0x4f75, d4: [0xa4, 0xc8, 0x43, 0x9a, 0xf2, 0xef, 0x56, 0x4c] };
const IID_ID3D11Texture2D: GUID =
    GUID { d1: 0x6f15aaf2, d2: 0xd208, d3: 0x4e89, d4: [0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c] };
const IID_ID3D11Texture2D1: GUID =
    GUID { d1: 0x51218251, d2: 0x1e33, d3: 0x4617, d4: [0x9c, 0xcb, 0x4d, 0x3a, 0x43, 0x67, 0xe7, 0xbb] };
const IID_IDXGIAdapter1: GUID =
    GUID { d1: 0x29038f61, d2: 0x3839, d3: 0x4626, d4: [0x91, 0xfd, 0x08, 0x68, 0x79, 0x01, 0x1a, 0x05] };
const IID_IDXGIDevice: GUID =
    GUID { d1: 0x54ec77fa, d2: 0x1377, d3: 0x44e6, d4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c] };
const IID_IDXGIDeviceSubObject: GUID =
    GUID { d1: 0x3d3e0379, d2: 0xf9de, d3: 0x4d58, d4: [0xbb, 0x6c, 0x18, 0xd6, 0x29, 0x92, 0xf1, 0xa6] };
const IID_IDXGIFactory1: GUID =
    GUID { d1: 0x770aae78, d2: 0xf26f, d3: 0x4dba, d4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87] };
const IID_IDXGIFactory2: GUID =
    GUID { d1: 0x50c83a1c, d2: 0xe072, d3: 0x4c48, d4: [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0] };
const IID_IDXGIFactory4: GUID =
    GUID { d1: 0x1bc6ea02, d2: 0xef36, d3: 0x464f, d4: [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a] };
const IID_IDXGIKeyedMutex: GUID =
    GUID { d1: 0x9d8e1289, d2: 0xd7b3, d3: 0x465f, d4: [0x81, 0x26, 0x25, 0x0e, 0x34, 0x9a, 0xf8, 0x5d] };
const IID_IDXGIObject: GUID =
    GUID { d1: 0xaec22fb8, d2: 0x76f3, d3: 0x4639, d4: [0x9b, 0xe0, 0x28, 0xeb, 0x43, 0xa6, 0x7a, 0x2e] };
const IID_IDXGIResource: GUID =
    GUID { d1: 0x035f3ab4, d2: 0x482e, d3: 0x4e50, d4: [0xb4, 0x1f, 0x8a, 0x7f, 0x8b, 0xd8, 0x96, 0x0b] };
const IID_IDXGIResource1: GUID =
    GUID { d1: 0x30961379, d2: 0x4609, d3: 0x4a41, d4: [0x99, 0x8e, 0x54, 0xfe, 0x56, 0x7e, 0xe0, 0xc1] };
const IID_IDXGISurface: GUID =
    GUID { d1: 0xcafcb56c, d2: 0x6ac3, d3: 0x4889, d4: [0xbf, 0x47, 0x9e, 0x23, 0xbb, 0xd2, 0x60, 0xec] };
const IID_IDXGISurface1: GUID =
    GUID { d1: 0x4ae63092, d2: 0x6327, d3: 0x4c1b, d4: [0x80, 0xae, 0xbf, 0xe1, 0x2e, 0xa3, 0x2b, 0x86] };
const IID_IDXGISurface2: GUID =
    GUID { d1: 0xaba496dd, d2: 0xb617, d3: 0x4cb8, d4: [0xa8, 0x66, 0xbc, 0x44, 0xd7, 0xeb, 0x1f, 0xa2] };
const IID_IDXGISwapChain: GUID =
    GUID { d1: 0x310d36a0, d2: 0xd2e7, d3: 0x4c0a, d4: [0xaa, 0x04, 0x6a, 0x9d, 0x23, 0xb8, 0x88, 0x6a] };

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

// ---- helpers of the DXGI entries (sheets_b_dxgi*): factories, adapters, outputs, swap chains ----

const ADAPTER_FLAG_REMOTE: u32 = 1;
const ADAPTER_FLAG_SOFTWARE: u32 = 2;
const PRESENT_USE_DURATION: u32 = 0x100;

/// The first hardware factory as IDXGIFactory2 (or whatever iid asks).
fn fac(iid: &GUID) -> *mut c_void {
    factory1(iid).1
}

/// IDXGIFactory1::EnumAdapters1(i): the HRESULT and the adapter.
fn enum_adapter1(f: *mut c_void, i: u32) -> (HRESULT, *mut c_void) {
    let mut a: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live factory; valid output.
    let hr = unsafe { vcall!(f, IDXGIFactory1::EnumAdapters1, fn(u32, *mut *mut c_void) -> HRESULT, i, &mut a) };
    (hr, a)
}

/// IDXGIFactory4::EnumWarpAdapter(iid)
fn enum_warp(f: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut a: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live factory; valid output.
    let hr =
        unsafe { vcall!(f, IDXGIFactory4::EnumWarpAdapter, fn(*const GUID, *mut *mut c_void) -> HRESULT, iid, &mut a) };
    (hr, a)
}

/// The WARP adapter as IDXGIAdapter1 (a new reference), from a fresh factory.
fn warp_adapter1() -> *mut c_void {
    let f = fac(&IID_IDXGIFactory4);
    let a = enum_warp(f, &IID_IDXGIAdapter1).1;
    release(f);
    a
}

/// The first adapter of EnumAdapters1 (the default hardware adapter), a new reference.
fn hw_adapter1() -> *mut c_void {
    let f = fac(&IID_IDXGIFactory1);
    let a = enum_adapter1(f, 0).1;
    release(f);
    a
}

/// The LUID of an adapter (as one u64) from GetDesc1.
fn adapter_luid(a: *mut c_void) -> u64 {
    let (_, d) = adapter_desc1(a);
    rd_u64(&d, 296)
}

/// IDXGIAdapter::EnumOutputs(i)
fn enum_outputs(a: *mut c_void, i: u32) -> (HRESULT, *mut c_void) {
    let mut o: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live adapter; valid output.
    let hr = unsafe { vcall!(a, IDXGIAdapter::EnumOutputs, fn(u32, *mut *mut c_void) -> HRESULT, i, &mut o) };
    (hr, o)
}

/// The WARP D3D11 device made on the WARP adapter? No: D3D_DRIVER_TYPE_WARP with a NULL adapter (see `warp`); the
/// factory that created its adapter is reached through GetAdapter/GetParent (`adapter_of`, `factory_of_adapter`).
fn dev_factory2(d: Dev) -> *mut c_void {
    let a = adapter_of(d);
    let f = factory_of_adapter(a);
    release(a);
    f
}

/// IDXGIFactory2::CreateSwapChainForHwnd(device, hwnd, desc, fullscreen desc, restrict, &sc)
fn sc_for_hwnd(f: *mut c_void, dev: *mut c_void, hwnd: HANDLE, desc: Option<&Sc1>) -> (HRESULT, *mut c_void) {
    let mut sc: *mut c_void = core::ptr::null_mut();
    // SAFETY: live objects; `desc` is live for the call.
    let hr = unsafe {
        vcall!(
            f,
            IDXGIFactory2::CreateSwapChainForHwnd,
            fn(*mut c_void, HANDLE, *const Sc1, *const c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
            dev,
            hwnd,
            desc.map_or(core::ptr::null(), |d| d as *const Sc1),
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut sc
        )
    };
    (hr, sc)
}

/// IDXGIFactory2::CreateSwapChainForComposition(device, desc, restrict, &sc)
fn sc_for_comp(f: *mut c_void, dev: *mut c_void, desc: Option<&Sc1>) -> (HRESULT, *mut c_void) {
    let mut sc: *mut c_void = core::ptr::null_mut();
    // SAFETY: live objects; `desc` is live for the call.
    let hr = unsafe {
        vcall!(
            f,
            IDXGIFactory2::CreateSwapChainForComposition,
            fn(*mut c_void, *const Sc1, *mut c_void, *mut *mut c_void) -> HRESULT,
            dev,
            desc.map_or(core::ptr::null(), |d| d as *const Sc1),
            core::ptr::null_mut(),
            &mut sc
        )
    };
    (hr, sc)
}

/// IDXGISwapChain1::GetDesc1
fn sc_desc1(sc: *mut c_void) -> (HRESULT, Sc1) {
    let mut d: Sc1 = [0xAAAA_AAAA; 12];
    // SAFETY: a live swap chain; `d` has room for DXGI_SWAP_CHAIN_DESC1.
    let hr = unsafe { vcall!(sc, IDXGISwapChain1::GetDesc1, fn(*mut Sc1) -> HRESULT, &mut d) };
    (hr, d)
}

/// IDXGISwapChain::GetBuffer(i, iid)
fn sc_buffer(sc: *mut c_void, i: u32, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut b: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live swap chain; valid output.
    let hr = unsafe {
        vcall!(sc, IDXGISwapChain::GetBuffer, fn(u32, *const GUID, *mut *mut c_void) -> HRESULT, i, iid, &mut b)
    };
    (hr, b)
}

/// IDXGISwapChain::Present(interval, flags)
fn present(sc: *mut c_void, interval: u32, flags: u32) -> HRESULT {
    // SAFETY: a live swap chain.
    unsafe { vcall!(sc, IDXGISwapChain::Present, fn(u32, u32) -> HRESULT, interval, flags) }
}

/// IDXGISwapChain::ResizeBuffers(count, w, h, format, flags)
fn resize(sc: *mut c_void, count: u32, w: u32, h: u32, format: u32, flags: u32) -> HRESULT {
    // SAFETY: a live swap chain.
    unsafe {
        vcall!(sc, IDXGISwapChain::ResizeBuffers, fn(u32, u32, u32, u32, u32) -> HRESULT, count, w, h, format, flags)
    }
}

/// "w x h fmt stereo samples usage buffers scaling effect alpha flags" of a DESC1 as text.
fn sc1_text(d: &Sc1) -> Buf<160> {
    let mut b = Buf::<160>::new();
    let _ = write!(
        b,
        "{}x{} fmt {} stereo {} samples {}/{} usage {:#x} buffers {} scaling {} effect {} alpha {} flags {:#x}",
        d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7], d[8], d[9], d[10], d[11]
    );
    b
}

/// Like `sc1_text` but the width and height are shown as relations to the window's client size.
fn sc1_text_rel(d: &Sc1, cw: i32, ch: i32) -> Buf<160> {
    let mut b = Buf::<160>::new();
    let _ = write!(
        b,
        "size-is-client={} fmt {} stereo {} samples {}/{} usage {:#x} buffers {} scaling {} effect {} alpha {} flags {:#x}",
        d[0] as i32 == cw && d[1] as i32 == ch,
        d[2],
        d[3],
        d[4],
        d[5],
        d[6],
        d[7],
        d[8],
        d[9],
        d[10],
        d[11]
    );
    b
}

/// Fill the back buffer 0 (as ID3D11Texture2D) with a colour through a render target view and ClearRenderTargetView.
fn clear_buffer(d: Dev, tex: *mut c_void, rgba: [f32; 4]) -> HRESULT {
    let mut rtv: *mut c_void = core::ptr::null_mut();
    // SAFETY: live device and texture; valid output.
    let hr = unsafe {
        vcall!(
            d.dev,
            ID3D11Device::CreateRenderTargetView,
            fn(*mut c_void, *const c_void, *mut *mut c_void) -> HRESULT,
            tex,
            core::ptr::null(),
            &mut rtv
        )
    };
    if hr >= 0 {
        // SAFETY: live context and view.
        unsafe {
            vcall!(
                d.ctx,
                ID3D11DeviceContext::ClearRenderTargetView,
                fn(*mut c_void, *const [f32; 4]) -> (),
                rtv,
                &rgba
            )
        };
        release(rtv);
    }
    hr
}

/// Copy a texture of the swap chain to a new staging texture and return the pixel at (x, y) (0xFFFFFFFE if anything failed).
fn read_pixel(d: Dev, tex: *mut c_void, x: u32, y: u32) -> u32 {
    if tex.is_null() {
        return 0xFFFF_FFFE;
    }
    let desc = get_desc(tex);
    let st = staging(d, desc[0], desc[1], desc[4]);
    if st.is_null() {
        return 0xFFFF_FFFE;
    }
    // SAFETY: live context and resources of the same size and format.
    unsafe { vcall!(d.ctx, ID3D11DeviceContext::CopyResource, fn(*mut c_void, *mut c_void) -> (), st, tex) };
    let p = pixel32(d, st, x, y);
    release(st);
    p
}

/// DXGI_SWAP_CHAIN_FULLSCREEN_DESC: RefreshRate numerator, denominator, ScanlineOrdering, Scaling, Windowed.
type ScFs = [u32; 5];

/// CreateSwapChainForHwnd with the optional fullscreen desc.
fn sc_for_hwnd_fs(
    f: *mut c_void,
    dev: *mut c_void,
    hwnd: HANDLE,
    desc: &Sc1,
    fs: Option<&ScFs>,
) -> (HRESULT, *mut c_void) {
    let mut sc: *mut c_void = core::ptr::null_mut();
    // SAFETY: live objects; the descs are live for the call.
    let hr = unsafe {
        vcall!(
            f,
            IDXGIFactory2::CreateSwapChainForHwnd,
            fn(*mut c_void, HANDLE, *const Sc1, *const ScFs, *mut c_void, *mut *mut c_void) -> HRESULT,
            dev,
            hwnd,
            desc as *const Sc1,
            fs.map_or(core::ptr::null(), |d| d as *const ScFs),
            core::ptr::null_mut(),
            &mut sc
        )
    };
    (hr, sc)
}

/// Create a swap chain for `hwnd` with `desc`, say the result and the GetDesc1 read-back (size as a relation to the
/// window's client area `cw` x `ch`), the reference count of the swap chain, then release it.
fn try_hwnd(t: &mut Test, label: &str, f: *mut c_void, dev: *mut c_void, hwnd: HANDLE, desc: &Sc1) -> HRESULT {
    let (hr, sc) = sc_for_hwnd(f, dev, hwnd, Some(desc));
    if sc.is_null() {
        say!(t, "{label}: {}", res(hr, sc).as_str());
    } else {
        let (cw, ch) = client_size(hwnd);
        let (dh, d) = sc_desc1(sc);
        say!(t, "{label}: {} refs {} GetDesc1 {} {}", Hr(hr), refs(sc), Hr(dh), sc1_text_rel(&d, cw, ch).as_str());
    }
    release(sc);
    hr
}

/// Same for CreateSwapChainForComposition (the size is printed as it is).
fn try_comp(t: &mut Test, label: &str, f: *mut c_void, dev: *mut c_void, desc: &Sc1) -> HRESULT {
    let (hr, sc) = sc_for_comp(f, dev, Some(desc));
    if sc.is_null() {
        say!(t, "{label}: {}", res(hr, sc).as_str());
    } else {
        let (dh, d) = sc_desc1(sc);
        say!(t, "{label}: {} refs {} GetDesc1 {} {}", Hr(hr), refs(sc), Hr(dh), sc1_text(&d).as_str());
    }
    release(sc);
    hr
}

fn desc_with(mut d: Sc1, index: usize, v: u32) -> Sc1 {
    d[index] = v;
    d
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDesktopWindow() -> HANDLE;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetWindowPos(h: HANDLE, after: HANDLE, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> BOOL;
}

/// Resize a window's client area (not moved, not activated): a new outer size big enough for `cw` x `ch`.
fn set_client_size(h: HANDLE, cw: i32, ch: i32) {
    let (ow, oh) = {
        let (a, b) = client_size(h);
        let mut r = RECT::default();
        // SAFETY: a live window.
        unsafe { GetWindowRect(h, &mut r) };
        (r.right - r.left - a, r.bottom - r.top - b)
    };
    // SAFETY: a live window of this thread.
    unsafe { SetWindowPos(h, core::ptr::null_mut(), 0, 0, cw + ow, ch + oh, 0x16) };
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowRect(h: HANDLE, r: *mut RECT) -> BOOL;
}

poly_wtest::main!(test);

fn desc_text(tex: *mut c_void) -> Buf<160> {
    let mut b = Buf::<160>::new();
    if tex.is_null() {
        let _ = write!(b, "no texture");
        return b;
    }
    let d = get_desc(tex);
    let _ = write!(
        b,
        "{}x{} mips {} array {} format {} samples {}/{} usage {} bind {:#x} cpu {:#x} misc {:#x}",
        d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7], d[8], d[9], d[10]
    );
    b
}

fn test(t: &mut Test) {
    SAY_PART.store(3, core::sync::atomic::Ordering::Relaxed);
    let d = warp();
    let f = dev_factory2(d);
    let w = plain_window();

    // indexes per effect and count
    for (en, eff, n) in [
        ("DISCARD", SWAP_DISCARD, 1u32),
        ("DISCARD", SWAP_DISCARD, 3),
        ("SEQUENTIAL", SWAP_SEQUENTIAL, 2),
        ("SEQUENTIAL", SWAP_SEQUENTIAL, 3),
        ("FLIP_SEQUENTIAL", SWAP_FLIP_SEQUENTIAL, 2),
        ("FLIP_SEQUENTIAL", SWAP_FLIP_SEQUENTIAL, 4),
        ("FLIP_DISCARD", SWAP_FLIP_DISCARD, 2),
        ("FLIP_DISCARD", SWAP_FLIP_DISCARD, 4),
    ] {
        let (_, sc) = sc_for_hwnd(f, d.dev, w, Some(&sc1(48, 32, F_B8G8R8A8_UNORM, n, eff)));
        let mut line = Buf::<200>::new();
        let mut objs = [core::ptr::null_mut(); 6];
        for i in 0..6u32 {
            let (hr, b) = sc_buffer(sc, i, &IID_ID3D11Texture2D);
            let _ = write!(line, "[{i}: {} ptr={}] ", Hr(hr), np(b));
            objs[i as usize] = b;
        }
        say!(t, "{en} x{n}: {}", line.as_str());
        let distinct = (0..6).all(|i| (0..i).all(|j| objs[i].is_null() || objs[j].is_null() || objs[i] != objs[j]));
        say!(t, "{en} x{n}: the accessible buffers are distinct objects: {distinct}");
        for o in objs {
            release(o);
        }
        for i in [n, 16, 0xffff_ffff] {
            let (hr, b) = sc_buffer(sc, i, &IID_ID3D11Texture2D);
            say!(t, "{en} x{n}: GetBuffer({}) {}", if i == n { "count" } else { "big" }, res(hr, b).as_str());
            release(b);
        }
        release(sc);
    }

    // interfaces
    let (_, sc) = sc_for_hwnd(f, d.dev, w, Some(&sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD)));
    let (_, base) = sc_buffer(sc, 0, &IID_ID3D11Texture2D);
    for (name, iid) in [
        ("IUnknown", &IID_IUnknown),
        ("IDXGIObject", &IID_IDXGIObject),
        ("IDXGIDeviceSubObject", &IID_IDXGIDeviceSubObject),
        ("IDXGIResource", &IID_IDXGIResource),
        ("IDXGIResource1", &IID_IDXGIResource1),
        ("IDXGISurface", &IID_IDXGISurface),
        ("IDXGISurface1", &IID_IDXGISurface1),
        ("IDXGISurface2", &IID_IDXGISurface2),
        ("ID3D11DeviceChild", &IID_ID3D11DeviceChild),
        ("ID3D11Resource", &IID_ID3D11Resource),
        ("ID3D11Texture2D", &IID_ID3D11Texture2D),
        ("ID3D11Texture2D1", &IID_ID3D11Texture2D1),
        ("ID3D11Texture1D", &IID_ID3D11Texture1D),
        ("ID3D11Buffer", &IID_ID3D11Buffer),
        ("IDXGIKeyedMutex", &IID_IDXGIKeyedMutex),
        ("made-up", &IID_Nothing),
    ] {
        let (hr, b) = sc_buffer(sc, 0, iid);
        let q = qi(base, iid);
        say!(
            t,
            "GetBuffer(0, {name}): {} same object as the texture {}; QueryInterface of the texture {}",
            res(hr, b).as_str(),
            b == base,
            Hr(q.0)
        );
        release(b);
        release(q.1);
    }
    // repeated calls and reference counts
    let r0 = refs(base);
    let (_, b2) = sc_buffer(sc, 0, &IID_ID3D11Texture2D);
    say!(t, "refs of buffer 0: {} then with a second GetBuffer {} (same object {})", r0, refs(base), b2 == base);
    release(b2);
    say!(t, "refs with the swap chain: {}", refs(sc));
    let (_, other) = sc_buffer(sc, 0, &IID_IDXGISurface);
    let (_, base_dx) = qi(base, &IID_IDXGISurface);
    say!(t, "GetBuffer(IDXGISurface) equals QI of the texture {}", other == base_dx);
    // IDXGISurface::GetDesc: Width, Height, Format, SampleDesc
    let mut sd = [0xAAu32; 4];
    // SAFETY: a live surface.
    let hr = unsafe { vcall!(other, IDXGISurface::GetDesc, fn(*mut [u32; 4]) -> HRESULT, &mut sd) };
    say!(t, "IDXGISurface::GetDesc {}: {}x{} format {} samples {}/{}", Hr(hr), sd[0], sd[1], sd[2], sd[3], 0);
    // GetDevice of the buffer
    let mut dv: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live texture.
    unsafe { vcall!(base, ID3D11DeviceChild::GetDevice, fn(*mut *mut c_void) -> (), &mut dv) };
    say!(t, "texture GetDevice is the device {}", dv == d.dev);
    release(dv);
    let mut par: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live surface.
    let hr = unsafe {
        vcall!(
            other,
            IDXGIObject::GetParent,
            fn(*const GUID, *mut *mut c_void) -> HRESULT,
            &IID_IDXGISwapChain,
            &mut par
        )
    };
    say!(t, "surface GetParent(IDXGISwapChain): {} is the swap chain {}", Hr(hr), par == sc);
    release(par);
    let mut dev2: *mut c_void = core::ptr::null_mut();
    // SAFETY: a live surface.
    let hr = unsafe {
        vcall!(
            other,
            IDXGIDeviceSubObject::GetDevice,
            fn(*const GUID, *mut *mut c_void) -> HRESULT,
            &IID_ID3D11Device,
            &mut dev2
        )
    };
    say!(t, "surface GetDevice: {} is the device {}", Hr(hr), dev2 == d.dev);
    release(dev2);
    // IDXGIResource: usage and shared handle
    let (_, rs) = qi(base, &IID_IDXGIResource);
    let mut usage = 0xAAAAu32;
    // SAFETY: a live resource.
    let hr = unsafe { vcall!(rs, IDXGIResource::GetUsage, fn(*mut u32) -> HRESULT, &mut usage) };
    say!(t, "IDXGIResource::GetUsage: {} {:#x}", Hr(hr), usage);
    let mut sh: HANDLE = 0x55usize as HANDLE;
    let hr = unsafe { vcall!(rs, IDXGIResource::GetSharedHandle, fn(*mut HANDLE) -> HRESULT, &mut sh) };
    say!(t, "GetSharedHandle: {} handle {}", Hr(hr), np(sh));
    let mut ev = 9u32;
    let hr = unsafe { vcall!(rs, IDXGIResource::GetEvictionPriority, fn(*mut u32) -> HRESULT, &mut ev) };
    say!(t, "GetEvictionPriority: {} {}", Hr(hr), ev);
    release(rs);
    release(other);
    release(base_dx);

    // render target views
    let rtv_try = |t: &mut Test, name: &str, desc: *const [u32; 4]| {
        let mut v: *mut c_void = core::ptr::null_mut();
        // SAFETY: live objects; `desc` is NULL or valid.
        let hr = unsafe {
            vcall!(
                d.dev,
                ID3D11Device::CreateRenderTargetView,
                fn(*mut c_void, *const [u32; 4], *mut *mut c_void) -> HRESULT,
                base,
                desc,
                &mut v
            )
        };
        say!(t, "CreateRenderTargetView {name}: {}", res(hr, v).as_str());
        release(v);
    };
    rtv_try(t, "default", core::ptr::null());
    // D3D11_RENDER_TARGET_VIEW_DESC: Format, ViewDimension (4 = TEXTURE2D), MipSlice
    rtv_try(t, "B8G8R8A8_UNORM Texture2D", &[F_B8G8R8A8_UNORM, 4, 0, 0]);
    rtv_try(t, "B8G8R8A8_UNORM_SRGB Texture2D", &[F_B8G8R8A8_UNORM_SRGB, 4, 0, 0]);
    rtv_try(t, "R8G8B8A8_UNORM Texture2D", &[F_R8G8B8A8_UNORM, 4, 0, 0]);
    rtv_try(t, "B8G8R8A8_TYPELESS Texture2D", &[90, 4, 0, 0]);
    // the buffer cannot be bound as a shader resource without SHADER_INPUT usage
    let mut v: *mut c_void = core::ptr::null_mut();
    // SAFETY: live objects.
    let hr = unsafe {
        vcall!(
            d.dev,
            ID3D11Device::CreateShaderResourceView,
            fn(*mut c_void, *const c_void, *mut *mut c_void) -> HRESULT,
            base,
            core::ptr::null(),
            &mut v
        )
    };
    say!(t, "CreateShaderResourceView on a RENDER_TARGET_OUTPUT buffer: {}", res(hr, v).as_str());
    release(v);
    release(base);
    release(sc);

    // texture descriptions for different swap chain descriptions
    for (name, desc) in [
        ("default", sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD)),
        ("usage SHADER_INPUT|RTO", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD), 6, 0x30)),
        ("usage 0", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD), 6, 0)),
        ("GDI_COMPATIBLE", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD), 11, 4)),
        ("ALLOW_TEARING", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD), 11, 0x800)),
        ("FRAME_LATENCY_WAITABLE_OBJECT", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD), 11, 0x40)),
        ("R16G16B16A16_FLOAT", sc1(48, 32, F_R16G16B16A16_FLOAT, 2, SWAP_FLIP_DISCARD)),
        ("R10G10B10A2", sc1(48, 32, F_R10G10B10A2_UNORM, 2, SWAP_FLIP_DISCARD)),
        ("R8G8B8A8 DISCARD", sc1(48, 32, F_R8G8B8A8_UNORM, 1, SWAP_DISCARD)),
        ("DISCARD 4 samples", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 1, SWAP_DISCARD), 4, 4)),
        ("stereo", desc_with(sc1(48, 32, F_B8G8R8A8_UNORM, 2, SWAP_FLIP_DISCARD), 3, 1)),
    ] {
        let (hr, sc) = sc_for_hwnd(f, d.dev, w, Some(&desc));
        let (_, b) = sc_buffer(sc, 0, &IID_ID3D11Texture2D);
        say!(t, "texture of {name}: create {} {}", Hr(hr), desc_text(b).as_str());
        release(b);
        release(sc);
    }
    destroy(w);
    release(f);
    dev_release(d);
}
