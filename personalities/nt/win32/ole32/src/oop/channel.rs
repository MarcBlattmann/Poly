//! The channel between interface proxies and stubs (ADR 0220, B2b; Learn: "IRpcChannelBuffer",
//! "RPCOLEMESSAGE"). A proxy made by an interface's proxy/stub class (for interfaces of a type
//! library: `oleaut32`'s automation marshaller) is connected to a client channel: `SendReceive`
//! sends the marshalled call to the object's exporter as `Op::Invoke` (or, for an object of another
//! apartment of this process, runs the stub there) and hands back the reply. On the server, the
//! exporter makes the interface's stub and calls `IRpcStubBuffer::Invoke` with a server channel,
//! whose `GetBuffer` gives the stub room for its reply.

use super::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::git::Owner;
use poly_com::hresult::{E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, S_OK};
use poly_com::security::{Blanket, E_ACCESSDENIED};
use poly_com::wire::{Caller, Op, Request};

/// `IID_IRpcChannelBuffer` (`objidl.h`): {D5F56B60-593B-101A-B569-08002B2DBF7A}.
const IID_IRPC_CHANNEL_BUFFER: Guid =
    Guid { data1: 0xD5F5_6B60, data2: 0x593B, data3: 0x101A, data4: [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A] };
/// `IID_IPSFactoryBuffer` (`objidl.h`).
pub(crate) const IID_IPS_FACTORY_BUFFER: Guid =
    Guid { data1: 0xD5F5_69D0, data2: 0x593B, data3: 0x101A, data4: [0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A] };
const CLSID_PSOA_INTERFACE: Guid =
    Guid { data1: 0x0002_0424, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const CLSID_PS_DISPATCH: Guid = Guid { data1: 0x0002_0420, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `CLSCTX_INPROC_SERVER`.
const CLSCTX_INPROC_SERVER: u32 = 1;
/// `MSHCTX_LOCAL`.
const MSHCTX_LOCAL: u32 = 0;

#[cfg_attr(target_arch = "x86", link(name = "oleaut32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "oleaut32", kind = "raw-dylib"))]
unsafe extern "system" {
    #[link_name = "DllGetClassObject"]
    fn oa_dll_get_class_object(clsid: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> Hresult;
}

/// `RPCOLEMESSAGE`.
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

/// Where a client channel sends its calls.
pub(crate) enum Target {
    /// An exporter: its pipe and the object's id there.
    Remote { endpoint: String, oid: u64 },
    /// An object of another apartment of this process.
    Local { owner: Owner, object: usize },
    /// The server end: it only gives buffers.
    Server,
}

#[repr(C)]
struct Channel {
    vtbl: *const ChannelVtbl,
    refs: AtomicU32,
    iid: Guid,
    target: Target,
    /// The blanket of the interface proxy the channel serves (B2c): calls carry its impersonation
    /// level, and a refusing one stops them here.
    blanket: Option<Arc<Locked<Blanket>>>,
}

#[repr(C)]
struct ChannelVtbl {
    qi: unsafe extern "system" fn(*mut Channel, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut Channel) -> u32,
    release: unsafe extern "system" fn(*mut Channel) -> u32,
    get_buffer: unsafe extern "system" fn(*mut Channel, *mut RpcMsg, *const Guid) -> Hresult,
    send_receive: unsafe extern "system" fn(*mut Channel, *mut RpcMsg, *mut u32) -> Hresult,
    free_buffer: unsafe extern "system" fn(*mut Channel, *mut RpcMsg) -> Hresult,
    get_dest_ctx: unsafe extern "system" fn(*mut Channel, *mut u32, *mut *mut c_void) -> Hresult,
    is_connected: unsafe extern "system" fn(*mut Channel) -> Hresult,
}

// SAFETY: a table of function pointers.
unsafe impl Sync for ChannelVtbl {}

static VTBL: ChannelVtbl = ChannelVtbl {
    qi: ch_qi,
    add_ref: ch_add_ref,
    release: ch_release,
    get_buffer: ch_get_buffer,
    send_receive: ch_send_receive,
    free_buffer: ch_free_buffer,
    get_dest_ctx: ch_get_dest_ctx,
    is_connected: ch_is_connected,
};

/// A new channel (one reference) as its `IRpcChannelBuffer`.
pub(crate) fn new_channel(iid: Guid, target: Target, blanket: Option<Arc<Locked<Blanket>>>) -> *mut c_void {
    Box::into_raw(Box::new(Channel { vtbl: &VTBL, refs: AtomicU32::new(1), iid, target, blanket })).cast()
}

unsafe extern "system" fn ch_qi(this: *mut Channel, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID and out pointer; a live channel.
    unsafe {
        let g = *iid;
        if g == crate::IID_IUNKNOWN || g == IID_IRPC_CHANNEL_BUFFER {
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            *out = this.cast();
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}
unsafe extern "system" fn ch_add_ref(this: *mut Channel) -> u32 {
    // SAFETY: a live channel.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}
unsafe extern "system" fn ch_release(this: *mut Channel) -> u32 {
    // SAFETY: a live channel.
    let n = unsafe { (*this).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: the last reference of a block from `new_channel`.
        drop(unsafe { Box::from_raw(this) });
    }
    n
}

/// Replace the message's buffer by a new one of `cbBuffer` bytes.
unsafe fn give_buffer(msg: *mut RpcMsg) -> Hresult {
    // SAFETY: the caller's message.
    unsafe {
        let old = (*msg).buffer;
        if !old.is_null() {
            crate::CoTaskMemFree(old.cast());
        }
        let p = crate::CoTaskMemAlloc(((*msg).cb_buffer as usize).max(1)).cast::<u8>();
        (*msg).buffer = p;
        if p.is_null() { E_OUTOFMEMORY } else { S_OK }
    }
}

unsafe extern "system" fn ch_get_buffer(_this: *mut Channel, msg: *mut RpcMsg, _iid: *const Guid) -> Hresult {
    if msg.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's message.
    unsafe { give_buffer(msg) }
}

unsafe extern "system" fn ch_free_buffer(_this: *mut Channel, msg: *mut RpcMsg) -> Hresult {
    if msg.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's message: a buffer from `give_buffer`.
    unsafe {
        if !(*msg).buffer.is_null() {
            crate::CoTaskMemFree((*msg).buffer.cast());
            (*msg).buffer = core::ptr::null_mut();
        }
    }
    S_OK
}

unsafe extern "system" fn ch_get_dest_ctx(_this: *mut Channel, ctx: *mut u32, pv: *mut *mut c_void) -> Hresult {
    if !ctx.is_null() {
        // SAFETY: the caller's out pointer.
        unsafe { *ctx = MSHCTX_LOCAL };
    }
    if !pv.is_null() {
        // SAFETY: as above.
        unsafe { *pv = core::ptr::null_mut() };
    }
    S_OK
}

unsafe extern "system" fn ch_is_connected(_this: *mut Channel) -> Hresult {
    S_OK
}

unsafe extern "system" fn ch_send_receive(this: *mut Channel, msg: *mut RpcMsg, status: *mut u32) -> Hresult {
    if msg.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live channel and the caller's message.
    let (ch, data, method) = unsafe {
        let m = &*msg;
        let data = if m.buffer.is_null() {
            Vec::new()
        } else {
            core::slice::from_raw_parts(m.buffer, m.cb_buffer as usize).to_vec()
        };
        (&*this, data, m.method)
    };
    let iid = ch.iid;
    let r = match &ch.target {
        Target::Remote { endpoint, oid } => {
            let (refuse, level) = ch.blanket.as_ref().map_or((false, 1), |b| b.with(|b| (b.refuse, b.token_level())));
            if refuse {
                return E_ACCESSDENIED;
            }
            let caller = Caller { level, activation: false };
            let req = Request::Call { oid: *oid, op: Op::Invoke { iid, method, data }, caller };
            let ep = endpoint.clone();
            // An STA caller keeps serving calls into its apartment (callbacks) while it waits.
            apt::blocking(move || match pipe::transact(&ep, &req) {
                Ok(rep) if rep.hr >= 0 => Ok(rep.data),
                Ok(rep) => Err(rep.hr),
                Err(f) => Err(f.hr()),
            })
        }
        Target::Local { owner, object } => {
            let o = *object;
            apt::run_in(*owner, || invoke_stub(o, &iid, method, &data))
        }
        Target::Server => Err(E_NOTIMPL),
    };
    if !status.is_null() {
        // SAFETY: the caller's out pointer.
        unsafe { *status = 0 };
    }
    // SAFETY: the caller's message.
    unsafe {
        match r {
            Ok(reply) => {
                (*msg).cb_buffer = reply.len() as u32;
                let hr = give_buffer(msg);
                if hr < 0 {
                    return hr;
                }
                if !reply.is_empty() {
                    core::ptr::copy_nonoverlapping(reply.as_ptr(), (*msg).buffer, reply.len());
                }
                S_OK
            }
            Err(hr) => {
                ch_free_buffer(this, msg);
                hr
            }
        }
    }
}

/// The proxy/stub factory (`IPSFactoryBuffer`) of the interface `iid`: its class from
/// `CoGetPSClsid`; the automation marshaller's classes are `oleaut32`'s.
pub(crate) fn ps_factory(iid: &Guid) -> Result<*mut c_void, Hresult> {
    let clsid = marshal::ps_clsid_lookup(iid)?;
    let mut f: *mut c_void = core::ptr::null_mut();
    let hr = if clsid == CLSID_PSOA_INTERFACE || clsid == CLSID_PS_DISPATCH {
        // SAFETY: oleaut32's class factory entry with live pointers.
        unsafe { oa_dll_get_class_object(&clsid, &IID_IPS_FACTORY_BUFFER, &mut f) }
    } else {
        // SAFETY: as above, through ole32's own CoGetClassObject.
        unsafe {
            crate::CoGetClassObject(
                &clsid,
                CLSCTX_INPROC_SERVER,
                core::ptr::null_mut(),
                &IID_IPS_FACTORY_BUFFER,
                &mut f,
            )
        }
    };
    if hr < 0 || f.is_null() { Err(if hr < 0 { hr } else { E_NOINTERFACE }) } else { Ok(f) }
}

/// Run a call on `object` (its identity) of this process: its interface's stub decodes `data` and
/// calls the method; the reply's bytes.
pub(crate) fn invoke_stub(object: usize, iid: &Guid, method: u32, data: &[u8]) -> Result<Vec<u8>, Hresult> {
    let f = ps_factory(iid)?;
    let mut stub: *mut c_void = core::ptr::null_mut();
    // SAFETY: IPSFactoryBuffer::CreateStub (slot 4).
    let hr = unsafe {
        let v = *(f as *const *const usize);
        let create: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void, *mut *mut c_void) -> Hresult =
            core::mem::transmute(*v.add(4));
        create(f, iid, object as *mut c_void, &mut stub)
    };
    // SAFETY: the factory's reference.
    unsafe { crate::release_unknown(f) };
    if hr < 0 || stub.is_null() {
        return Err(if hr < 0 { hr } else { E_NOINTERFACE });
    }
    let chan = new_channel(*iid, Target::Server, None);
    let mut msg = RpcMsg {
        reserved1: core::ptr::null_mut(),
        data_rep: 0x10,
        buffer: core::ptr::null_mut(),
        cb_buffer: data.len() as u32,
        method,
        reserved2: [core::ptr::null_mut(); 5],
        flags: 0,
    };
    // SAFETY: our message; the stub's Invoke (slot 5) and Disconnect (slot 4).
    let r = unsafe {
        let mut r = give_buffer(&mut msg);
        if r >= 0 {
            if !data.is_empty() {
                core::ptr::copy_nonoverlapping(data.as_ptr(), msg.buffer, data.len());
            }
            let v = *(stub as *const *const usize);
            let invoke: unsafe extern "system" fn(*mut c_void, *mut RpcMsg, *mut c_void) -> Hresult =
                core::mem::transmute(*v.add(5));
            r = invoke(stub, &mut msg, chan);
            let disconnect: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(*v.add(4));
            disconnect(stub);
        }
        if r < 0 {
            Err(r)
        } else if msg.buffer.is_null() {
            Ok(Vec::new())
        } else {
            Ok(core::slice::from_raw_parts(msg.buffer, msg.cb_buffer as usize).to_vec())
        }
    };
    // SAFETY: our message's buffer, the stub's and the channel's references.
    unsafe {
        ch_free_buffer(chan.cast(), &mut msg);
        crate::release_unknown(stub);
        crate::release_unknown(chan);
    }
    r
}
