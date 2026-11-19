//! Interface stubs (sheet `DllGetClassObject`, rule `create_stub`): one per interface and server
//! object; `Invoke` decodes a call's `[in]` values, calls the object's method, and encodes the
//! `HRESULT`, the `[out]` values and the error information the method left.

use super::codec::{self, R, Ty, W};
use super::info::{Info, info_for};
use super::{IID_IRPCSTUBBUFFER, RpcMsg};
use crate::com::*;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

#[repr(C)]
struct Stub {
    vtbl: *const Vtbl<10>,
    refs: AtomicU32,
    info: Arc<Info>,
    server: AtomicUsize,
}

unsafe extern "system" fn s_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID.
    let g = unsafe { *iid };
    if g == IID_IUNKNOWN || g == IID_IRPCSTUBBUFFER {
        // SAFETY: our stub.
        unsafe { s_add_ref(this) };
        put(out, this);
        S_OK
    } else {
        put(out, core::ptr::null_mut());
        E_NOINTERFACE
    }
}
unsafe extern "system" fn s_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: a live stub.
    unsafe { (*(this as *mut Stub)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}
unsafe extern "system" fn s_release(this: *mut c_void) -> u32 {
    let s = this as *mut Stub;
    // SAFETY: a live stub.
    let n = unsafe { (*s).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: the last reference of a block from `create_stub`.
        let b = unsafe { Box::from_raw(s) };
        release(b.server.swap(0, Ordering::AcqRel) as *mut c_void);
    }
    n
}
unsafe extern "system" fn s_connect(this: *mut c_void, server: *mut c_void) -> Hresult {
    let s = this as *mut Stub;
    add_ref(server);
    // SAFETY: a live stub.
    release(unsafe { (*s).server.swap(server as usize, Ordering::AcqRel) } as *mut c_void);
    S_OK
}
unsafe extern "system" fn s_disconnect(this: *mut c_void) {
    let s = this as *mut Stub;
    // SAFETY: a live stub.
    release(unsafe { (*s).server.swap(0, Ordering::AcqRel) } as *mut c_void);
}
unsafe extern "system" fn s_is_iid_supported(this: *mut c_void, iid: *const Guid) -> *mut c_void {
    let s = this as *mut Stub;
    if iid.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's IID; a live stub.
    let (g, info) = unsafe { (*iid, &(*s).info) };
    if g == info.iid || (info.dispatch && g == IID_IDISPATCH) {
        // SAFETY: our stub.
        unsafe { s_add_ref(this) };
        this
    } else {
        core::ptr::null_mut()
    }
}
unsafe extern "system" fn s_count_refs(this: *mut c_void) -> u32 {
    // SAFETY: a live stub.
    u32::from(unsafe { (*(this as *mut Stub)).server.load(Ordering::Acquire) } != 0)
}
unsafe extern "system" fn s_debug_qi(this: *mut c_void, out: *mut *mut c_void) -> Hresult {
    let s = this as *mut Stub;
    // SAFETY: a live stub.
    let (server, iid) = unsafe { ((*s).server.load(Ordering::Acquire) as *mut c_void, (&(*s).info).iid) };
    if server.is_null() {
        put(out, core::ptr::null_mut());
        return E_UNEXPECTED;
    }
    let (hr, p) = query(server, &iid);
    put(out, p);
    hr
}
unsafe extern "system" fn s_debug_release(_this: *mut c_void, p: *mut c_void) {
    release(p);
}

/// `IRpcStubBuffer::Invoke(pRpcMsg, pRpcChannelBuffer)`.
unsafe extern "system" fn s_invoke(this: *mut c_void, msg: *mut RpcMsg, channel: *mut c_void) -> Hresult {
    let s = this as *mut Stub;
    if msg.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: a live stub and the channel's message.
    let (server, info, slot, data) = unsafe {
        let m = &*msg;
        let data = if m.buffer.is_null() {
            Vec::new()
        } else {
            core::slice::from_raw_parts(m.buffer, m.cb_buffer as usize).to_vec()
        };
        ((*s).server.load(Ordering::Acquire) as *mut c_void, (*s).info.clone(), m.method, data)
    };
    if server.is_null() {
        return 0x8001_0108_u32 as i32; // RPC_E_DISCONNECTED
    }
    let (hr, itf) = query(server, &info.iid);
    if itf.is_null() {
        return if hr < 0 { hr } else { E_NOINTERFACE };
    }
    let reply = if info.dispatch && (3..=6).contains(&slot) {
        super::disp::stub_call(itf, slot, &data)
    } else {
        call(&info, itf, slot, &data)
    };
    release(itf);
    let reply = match reply {
        Ok(r) => r,
        Err(hr) => return hr,
    };
    // SAFETY: the channel's message: the reply goes into a buffer the channel gives.
    unsafe {
        (*msg).cb_buffer = reply.len() as u32;
        let hr = vcall!(channel, 3, (*mut RpcMsg, *const Guid) -> Hresult, msg, &info.iid);
        if hr < 0 {
            return hr;
        }
        if !reply.is_empty() {
            core::ptr::copy_nonoverlapping(reply.as_ptr(), (*msg).buffer, reply.len());
        }
    }
    S_OK
}

/// Decode the arguments of method `slot`, call it on `itf`, encode the results.
fn call(info: &Info, itf: *mut c_void, slot: u32, data: &[u8]) -> Result<Vec<u8>, Hresult> {
    let Some(m) = info.method(slot) else { return Err(0x8001_010F_u32 as i32) };
    let mut r = R(data);
    // One 24-byte storage per parameter (a VARIANT is the largest value).
    let mut store: Vec<[u64; 3]> = alloc::vec![[0; 3]; m.params.len()];
    let mut decoded: Vec<Option<Ty>> = alloc::vec![None; m.params.len()];
    let mut status = S_OK;
    for (i, p) in m.params.iter().enumerate() {
        let mem = store[i].as_mut_ptr().cast::<u8>();
        let ty = match (&p.ty, p.output) {
            (Ty::Ptr(inner), true) => (**inner).clone(),
            (t, _) => t.clone(),
        };
        if !p.output || p.input {
            if let Err(hr) = codec::dec(&mut r, &ty, mem) {
                status = hr;
                break;
            }
        }
        decoded[i] = Some(ty);
    }
    let mut w = W::default();
    if status < 0 {
        free_all(&mut store, &decoded);
        return Err(status);
    }
    let mut args: Vec<u64> = alloc::vec![itf as u64];
    for (i, p) in m.params.iter().enumerate() {
        let mem = store[i].as_mut_ptr();
        args.push(if p.output || p.ty == Ty::Variant { mem as u64 } else { store[i][0] });
    }
    crate::errinfo::set(core::ptr::null_mut());
    // SAFETY: the object's vtable; the method at `slot` takes these arguments.
    let hr = unsafe {
        let f = *(*(itf as *const *const usize)).add(slot as usize);
        crate::call::call(f, &args, core::ptr::null_mut())
    } as u32 as i32;
    let mut out = W::default();
    let mut hr2 = hr;
    if hr >= 0 {
        for (i, p) in m.params.iter().enumerate() {
            if p.output {
                if let Some(ty) = &decoded[i] {
                    if let Err(e) = codec::enc(&mut out, ty, store[i].as_ptr().cast()) {
                        hr2 = e;
                        break;
                    }
                }
            }
        }
    }
    w.u32(hr2 as u32);
    if hr2 >= 0 {
        w.0.extend_from_slice(&out.0);
    }
    super::proxy::write_error_info(&mut w, hr2);
    free_all(&mut store, &decoded);
    Ok(w.0)
}

fn free_all(store: &mut [[u64; 3]], decoded: &[Option<Ty>]) {
    for (s, t) in store.iter_mut().zip(decoded) {
        if let Some(t) = t {
            codec::free(t, s.as_mut_ptr().cast());
        }
    }
}

static STUB_VTBL: Vtbl<10> = Vtbl([
    s_qi as *const (),
    s_add_ref as *const (),
    s_release as *const (),
    s_connect as *const (),
    s_disconnect as *const (),
    s_invoke as *const (),
    s_is_iid_supported as *const (),
    s_count_refs as *const (),
    s_debug_qi as *const (),
    s_debug_release as *const (),
]);

/// `IPSFactoryBuffer::CreateStub(riid, pUnkServer, ppStub)`: no check that the object has the
/// interface (sheet); the stub holds the object.
pub(crate) unsafe extern "system" fn create_stub(
    _this: *mut c_void,
    iid: *const Guid,
    server: *mut c_void,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    put(out, core::ptr::null_mut());
    // SAFETY: the caller's IID.
    let info = match info_for(unsafe { &*iid }) {
        Ok(i) => i,
        Err(hr) => return hr,
    };
    add_ref(server);
    let s = Box::into_raw(Box::new(Stub {
        vtbl: &STUB_VTBL,
        refs: AtomicU32::new(1),
        info,
        server: AtomicUsize::new(server as usize),
    }));
    put(out, s.cast());
    S_OK
}
