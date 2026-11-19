//! The automation marshaller (ADR 0220, B2b; sheet `DllGetClassObject`): `oleaut32`'s class
//! `{00020424-...}` (`CLSID_PSOAInterface`, also answered for the older `{00020420-...}`), an
//! `IPSFactoryBuffer` that makes proxies and stubs for interfaces a registered type library
//! describes. They talk to `ole32` through the public channel interfaces (Learn:
//! "IPSFactoryBuffer", "IRpcProxyBuffer", "IRpcStubBuffer", "IRpcChannelBuffer", "RPCOLEMESSAGE").
//!
//! - [`info`]: an interface's methods from its type library.
//! - [`codec`]: how values travel.
//! - [`proxy`] / [`stub`]: the two ends of a call; [`disp`]: `IDispatch`'s four methods.

pub(crate) mod codec;
pub(crate) mod disp;
pub(crate) mod info;
pub(crate) mod proxy;
pub(crate) mod stub;

use crate::com::*;
use core::ffi::c_void;

pub(crate) const IID_IPSFACTORYBUFFER: Guid =
    Guid { data1: 0xD5F5_69D0, data2: 0x593B, data3: 0x101A, data4: [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A] };
pub(crate) const IID_IRPCPROXYBUFFER: Guid =
    Guid { data1: 0xD5F5_6A34, data2: 0x593B, data3: 0x101A, data4: [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A] };
pub(crate) const IID_IRPCSTUBBUFFER: Guid =
    Guid { data1: 0xD5F5_6AFC, data2: 0x593B, data3: 0x101A, data4: [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A] };
pub(crate) const CLSID_PSOAINTERFACE: Guid =
    Guid { data1: 0x0002_0424, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub(crate) const CLSID_PSDISPATCH: Guid =
    Guid { data1: 0x0002_0420, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// `RPCOLEMESSAGE` (`objidl.h`).
#[repr(C)]
pub(crate) struct RpcMsg {
    pub reserved1: *mut c_void,
    pub data_rep: u32,
    pub buffer: *mut u8,
    pub cb_buffer: u32,
    pub method: u32,
    pub reserved2: [*mut c_void; 5],
    pub flags: u32,
}

impl RpcMsg {
    pub(crate) fn new(method: u32, len: usize) -> RpcMsg {
        RpcMsg {
            reserved1: core::ptr::null_mut(),
            data_rep: 0x10,
            buffer: core::ptr::null_mut(),
            cb_buffer: len as u32,
            method,
            reserved2: [core::ptr::null_mut(); 5],
            flags: 0,
        }
    }
}

/// Send `data` as method `method` of `iid` over `channel` (`IRpcChannelBuffer`): the reply's bytes.
pub(crate) fn send(channel: *mut c_void, iid: &Guid, method: u32, data: &[u8]) -> Result<alloc::vec::Vec<u8>, Hresult> {
    if channel.is_null() {
        return Err(0x8001_0108_u32 as i32); // RPC_E_DISCONNECTED
    }
    let mut msg = RpcMsg::new(method, data.len());
    let hr = vcall!(channel, 3, (*mut RpcMsg, *const Guid) -> Hresult, &mut msg, iid);
    if hr < 0 {
        return Err(hr);
    }
    if !data.is_empty() {
        // SAFETY: the channel's buffer of `cbBuffer` bytes.
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), msg.buffer, data.len()) };
    }
    let mut status = 0u32;
    let hr = vcall!(channel, 4, (*mut RpcMsg, *mut u32) -> Hresult, &mut msg, &mut status);
    if hr < 0 {
        return Err(hr);
    }
    let reply = if msg.buffer.is_null() {
        alloc::vec::Vec::new()
    } else {
        // SAFETY: the reply's `cbBuffer` bytes.
        unsafe { core::slice::from_raw_parts(msg.buffer, msg.cb_buffer as usize) }.to_vec()
    };
    vcall!(channel, 5, (*mut RpcMsg) -> Hresult, &mut msg);
    Ok(reply)
}

// ----- the class factory ----------------------------------------------------------------------------

unsafe extern "system" fn f_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID.
    let g = unsafe { *iid };
    if g == IID_IUNKNOWN || g == IID_IPSFACTORYBUFFER {
        put(out, this);
        S_OK
    } else {
        put(out, core::ptr::null_mut());
        E_NOINTERFACE
    }
}
unsafe extern "system" fn f_add_ref(_this: *mut c_void) -> u32 {
    2
}
unsafe extern "system" fn f_release(_this: *mut c_void) -> u32 {
    1
}

static FACTORY_VTBL: Vtbl<5> = Vtbl([
    f_qi as *const (),
    f_add_ref as *const (),
    f_release as *const (),
    proxy::create_proxy as *const (),
    stub::create_stub as *const (),
]);

/// The one factory object (a vtable pointer; its references are not counted).
struct Factory(*const Vtbl<5>);
// SAFETY: an immutable pointer to a static table.
unsafe impl Sync for Factory {}
static FACTORY: Factory = Factory(&FACTORY_VTBL);

/// `DllGetClassObject(rclsid, riid, ppv)`: the marshaller's factory for `{00020424-...}` and
/// `{00020420-...}`, answering `IUnknown` and `IPSFactoryBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    put(out, core::ptr::null_mut());
    if clsid.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's CLSID.
    let c = unsafe { *clsid };
    if c != CLSID_PSOAINTERFACE && c != CLSID_PSDISPATCH {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let p = &FACTORY as *const Factory as *mut c_void;
    // SAFETY: our factory.
    unsafe { f_qi(p, iid, out) }
}
