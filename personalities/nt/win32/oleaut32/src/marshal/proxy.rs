//! Interface proxies (sheet `DllGetClassObject`, rule `create_proxy`): one block with the
//! interface (a vtable of [`crate::call::ENTRIES`], one per method slot) and its
//! `IRpcProxyBuffer`. The interface's `IUnknown` methods go to the outer object (`ole32`'s proxy
//! manager) when there is one; `CreateProxy` writes no interface pointer (`*ppv` is NULL: the
//! manager asks the proxy buffer for the interface). A call encodes the `[in]` values, sends them
//! through the connected channel and decodes the `HRESULT`, the `[out]` values and the error
//! information.

use super::codec::{self, R, RPC_X_NULL_REF_POINTER, Ty, W};
use super::info::{Info, info_for};
use super::{IID_IRPCPROXYBUFFER, send};
use crate::com::*;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

const PTR: usize = core::mem::size_of::<usize>();

#[repr(C)]
pub(crate) struct Proxy {
    itf: *const usize,
    buf: *const Vtbl<5>,
    refs: AtomicU32,
    outer: *mut c_void,
    pub(crate) channel: AtomicUsize,
    pub(crate) info: Arc<Info>,
    table: Box<[usize]>,
}

impl Proxy {
    pub(crate) fn iid(&self) -> Guid {
        self.info.iid
    }
}

unsafe fn inner_add_ref(p: *mut Proxy) -> u32 {
    // SAFETY: a live block.
    unsafe { (*p).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe fn inner_release(p: *mut Proxy) -> u32 {
    // SAFETY: a live block.
    let n = unsafe { (*p).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: the last reference of a block from `create_proxy`.
        let b = unsafe { Box::from_raw(p) };
        release(b.channel.swap(0, Ordering::AcqRel) as *mut c_void);
    }
    n
}

// The interface's IUnknown: delegated to the outer object.

unsafe extern "system" fn itf_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let p = this as *mut Proxy;
    // SAFETY: a live block.
    let outer = unsafe { (*p).outer };
    if !outer.is_null() {
        return vcall!(outer, 0, (*const Guid, *mut *mut c_void) -> Hresult, iid, out);
    }
    // SAFETY: as above.
    unsafe { buf_qi_block(p, iid, out) }
}
unsafe extern "system" fn itf_add_ref(this: *mut c_void) -> u32 {
    let p = this as *mut Proxy;
    // SAFETY: a live block.
    let outer = unsafe { (*p).outer };
    if outer.is_null() {
        // SAFETY: as above.
        unsafe { inner_add_ref(p) }
    } else {
        add_ref(outer)
    }
}
unsafe extern "system" fn itf_release(this: *mut c_void) -> u32 {
    let p = this as *mut Proxy;
    // SAFETY: a live block.
    let outer = unsafe { (*p).outer };
    if outer.is_null() {
        // SAFETY: as above.
        unsafe { inner_release(p) }
    } else {
        release(outer)
    }
}

// IRpcProxyBuffer (one pointer into the block).

unsafe fn buf_qi_block(p: *mut Proxy, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID; a live block.
    let g = unsafe { *iid };
    // SAFETY: as above.
    let info = unsafe { &(*p).info };
    if g == IID_IUNKNOWN || g == IID_IRPCPROXYBUFFER {
        // SAFETY: as above.
        unsafe { inner_add_ref(p) };
        put(out, (p as usize + PTR) as *mut c_void);
        S_OK
    } else if g == info.iid || (info.dispatch && g == IID_IDISPATCH) {
        // SAFETY: the interface (its references go where the interface's go).
        unsafe { itf_add_ref(p.cast()) };
        put(out, p.cast());
        S_OK
    } else {
        put(out, core::ptr::null_mut());
        E_NOINTERFACE
    }
}
fn of_buf(this: *mut c_void) -> *mut Proxy {
    (this as usize - PTR) as *mut Proxy
}
unsafe extern "system" fn buf_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our IRpcProxyBuffer.
    unsafe { buf_qi_block(of_buf(this), iid, out) }
}
unsafe extern "system" fn buf_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { inner_add_ref(of_buf(this)) }
}
unsafe extern "system" fn buf_release(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { inner_release(of_buf(this)) }
}
unsafe extern "system" fn buf_connect(this: *mut c_void, channel: *mut c_void) -> Hresult {
    let p = of_buf(this);
    add_ref(channel);
    // SAFETY: a live block.
    let old = unsafe { (*p).channel.swap(channel as usize, Ordering::AcqRel) };
    release(old as *mut c_void);
    S_OK
}
unsafe extern "system" fn buf_disconnect(this: *mut c_void) {
    let p = of_buf(this);
    // SAFETY: a live block.
    let old = unsafe { (*p).channel.swap(0, Ordering::AcqRel) };
    release(old as *mut c_void);
}

static BUF_VTBL: Vtbl<5> = Vtbl([
    buf_qi as *const (),
    buf_add_ref as *const (),
    buf_release as *const (),
    buf_connect as *const (),
    buf_disconnect as *const (),
]);

/// `IPSFactoryBuffer::CreateProxy(pUnkOuter, riid, ppProxy, ppv)`.
pub(crate) unsafe extern "system" fn create_proxy(
    _this: *mut c_void,
    outer: *mut c_void,
    iid: *const Guid,
    proxy: *mut *mut c_void,
    ppv: *mut *mut c_void,
) -> Hresult {
    put(ppv, core::ptr::null_mut());
    if proxy.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    put(proxy, core::ptr::null_mut());
    if !crate::call::AVAILABLE {
        return REGDB_E_IIDNOTREG;
    }
    // SAFETY: the caller's IID.
    let info = match info_for(unsafe { &*iid }) {
        Ok(i) => i,
        Err(hr) => return hr,
    };
    let slots = (info.last_slot as usize + 1).min(crate::call::MAX_SLOTS);
    let mut table: Vec<usize> = Vec::with_capacity(slots);
    table.push(itf_qi as usize);
    table.push(itf_add_ref as usize);
    table.push(itf_release as usize);
    #[cfg(target_arch = "x86_64")]
    for k in 3..slots {
        table.push(crate::call::ENTRIES[k] as usize);
    }
    let table = table.into_boxed_slice();
    let p = Box::into_raw(Box::new(Proxy {
        itf: table.as_ptr(),
        buf: &BUF_VTBL,
        refs: AtomicU32::new(1),
        outer,
        channel: AtomicUsize::new(0),
        info,
        table,
    }));
    put(proxy, (p as usize + PTR) as *mut c_void);
    S_OK
}

/// The memory of argument `i` (0 is `this`): its 8-byte slot, or for the second to fourth
/// arguments of floating type, the saved XMM register.
fn arg_mem(args: *const u64, xmm: *const u64, i: usize, ty: &Ty) -> *const u8 {
    if matches!(ty, Ty::Float(_)) && (1..=3).contains(&i) {
        return xmm.wrapping_add(i - 1).cast();
    }
    let slot = args.wrapping_add(i);
    if *ty == Ty::Variant {
        // A VARIANT is passed by reference (Learn: x64 calling convention, aggregates over 8 bytes).
        // SAFETY: the argument's slot.
        return unsafe { *slot } as usize as *const u8;
    }
    slot.cast()
}

/// Every proxy vtable entry of slot 3 and above comes here ([`crate::call`]).
#[cfg(target_arch = "x86_64")]
pub(crate) unsafe extern "system" fn dispatch(args: *const u64, slot: u32, xmm: *const u64) -> u64 {
    // SAFETY: `this` is our interface pointer: the block.
    let p = unsafe { *args } as usize as *mut Proxy;
    // SAFETY: a live block.
    let px = unsafe { &*p };
    let hr = if px.info.dispatch && (3..=6).contains(&slot) {
        super::disp::proxy_call(px, slot, args)
    } else {
        call(px, slot, args, xmm)
    };
    hr as u32 as u64
}

fn call(px: &Proxy, slot: u32, args: *const u64, xmm: *const u64) -> Hresult {
    let Some(m) = px.info.method(slot) else { return E_NOTIMPL };
    let mut w = W::default();
    // The caller's out pointers (by parameter index).
    let mut outs: Vec<(usize, *mut u8, Ty)> = Vec::new();
    for (i, p) in m.params.iter().enumerate() {
        let mem = arg_mem(args, xmm, i + 1, &p.ty);
        if p.output {
            let Ty::Ptr(inner) = &p.ty else { return E_INVALIDARG };
            // SAFETY: the argument slot holds the out pointer.
            let q = unsafe { *(mem as *const usize) } as *mut u8;
            if q.is_null() {
                return RPC_X_NULL_REF_POINTER;
            }
            if p.input {
                if let Err(hr) = codec::enc(&mut w, inner, q) {
                    return hr;
                }
            }
            outs.push((i, q, (**inner).clone()));
        } else if let Err(hr) = codec::enc(&mut w, &p.ty, mem) {
            return hr;
        }
    }
    let reply = match send(px.channel.load(Ordering::Acquire) as *mut c_void, &px.iid(), slot, &w.0) {
        Ok(r) => r,
        Err(hr) => return hr,
    };
    let mut r = R(&reply);
    let Ok(hr) = r.u32().map(|v| v as i32) else { return codec::RPC_E_INVALID_DATA };
    if hr >= 0 {
        for (i, q, ty) in &outs {
            if m.params[*i].input {
                codec::free(ty, *q);
            }
            if let Err(e) = codec::dec(&mut r, ty, *q) {
                return e;
            }
        }
    }
    read_error_info(&mut r);
    hr
}

/// Write the error information a reply carries (or none) as the calling thread's.
pub(crate) fn read_error_info(r: &mut R) {
    let ei = match r.u8() {
        Ok(1) => {
            let guid = r.guid().unwrap_or_default();
            let mut s = || -> Option<alloc::string::String> {
                match r.u8() {
                    Ok(1) => r.wstr().ok().map(|u| alloc::string::String::from_utf16_lossy(&u)),
                    _ => None,
                }
            };
            let (source, description, help_file) = (s(), s(), s());
            let help_context = r.u32().unwrap_or(0);
            let d = crate::errinfo::ErrorData { guid, source, description, help_file, help_context };
            crate::errinfo::new_error(d)
        }
        _ => core::ptr::null_mut(),
    };
    crate::errinfo::set(ei);
    release(ei);
}

/// Append the error information a method left (if any) to a reply.
pub(crate) fn write_error_info(w: &mut W, hr: Hresult) {
    let ei = crate::errinfo::take();
    match ei {
        Some(p) if hr < 0 => {
            let d = crate::errinfo::read(p);
            w.u8(1);
            w.guid(&d.guid);
            for s in [&d.source, &d.description, &d.help_file] {
                match s {
                    Some(t) => {
                        w.u8(1);
                        w.wstr(&t.encode_utf16().collect::<Vec<u16>>());
                    }
                    None => w.u8(0),
                }
            }
            w.u32(d.help_context);
            release(p);
        }
        other => {
            if let Some(p) = other {
                release(p);
            }
            w.u8(0);
        }
    }
}
