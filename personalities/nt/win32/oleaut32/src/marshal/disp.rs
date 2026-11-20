//! `IDispatch`'s four methods across processes (sheet `DllGetClassObject`, rules `dual_dispatch`
//! and `invoke`), for `IDispatch` itself and every interface built on it (slots 3 to 6):
//!
//! - `GetTypeInfoCount`: the count.
//! - `GetTypeInfo`: the type information travels by value — the library the server's type info
//!   belongs to (Poly's file format) and its index; the client gets a copy, which answers
//!   `IMarshal` as a proxy does.
//! - `GetIDsOfNames`: the ids come back also when the call fails (an unknown name gives -1).
//! - `Invoke`: a non-NULL interface id is `DISP_E_UNKNOWNINTERFACE` without reaching the server;
//!   the arguments travel as variants; the result (when asked for), the `EXCEPINFO` of a
//!   `DISP_E_EXCEPTION` and the argument-error index come back.

use super::codec::{self, R, W};
use super::proxy::Proxy;
use super::send;
use crate::com::*;
use crate::typelib::invoke::{DispParams, ExcepInfo};
use crate::typelib::objects;
use crate::variant::{self, Variant};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::Ordering;
use poly_com::typelib::file;
use poly_com::typelib::view::Form;
use poly_kernelbase::oleaut::VARIANT_SIZE;

const DISP_E_UNKNOWNINTERFACE: Hresult = 0x8002_0001_u32 as i32;
const DISP_E_EXCEPTION: Hresult = 0x8002_0009_u32 as i32;
const MAX_ARGS: u32 = 4096;

fn arg(args: *const u64, i: usize) -> u64 {
    // SAFETY: the call's argument slots (`this` and four or eight arguments, all present).
    unsafe { *args.add(i) }
}

fn opt_str(w: &mut W, b: *mut u16) {
    if b.is_null() {
        w.u8(0);
    } else {
        // SAFETY: a BSTR.
        let n = unsafe { crate::SysStringLen(b) } as usize;
        w.u8(1);
        // SAFETY: `n` characters.
        w.wstr(unsafe { core::slice::from_raw_parts(b, n) });
    }
}

fn read_opt_str(r: &mut R) -> Result<*mut u16, Hresult> {
    if r.u8()? == 0 {
        return Ok(core::ptr::null_mut());
    }
    let u = r.wstr()?;
    // SAFETY: `u.len()` characters.
    Ok(unsafe { crate::SysAllocStringLen(u.as_ptr(), u.len() as u32) })
}

/// The proxy side of slot `slot` (3..=6).
pub(crate) fn proxy_call(px: &Proxy, slot: u32, args: *const u64) -> Hresult {
    let chan = px.channel.load(Ordering::Acquire) as *mut c_void;
    let iid = px.iid();
    let mut w = W::default();
    match slot {
        3 => {
            let out = arg(args, 1) as *mut u32;
            if out.is_null() {
                return E_INVALIDARG;
            }
            let reply = match send(chan, &iid, 3, &[]) {
                Ok(r) => r,
                Err(hr) => return hr,
            };
            let mut r = R(&reply);
            let (Ok(hr), Ok(n)) = (r.u32(), r.u32()) else { return codec::RPC_E_INVALID_DATA };
            put(out, n);
            hr as i32
        }
        4 => {
            let out = arg(args, 3) as *mut *mut c_void;
            if out.is_null() {
                return E_INVALIDARG;
            }
            w.u32(arg(args, 1) as u32);
            w.u32(arg(args, 2) as u32);
            let reply = match send(chan, &iid, 4, &w.0) {
                Ok(r) => r,
                // A failure of the channel leaves the out values alone (sheet `CoUnmarshalInterface`,
                // rule `dead_server`).
                Err(hr) => return hr,
            };
            put(out, core::ptr::null_mut());
            let mut r = R(&reply);
            let Ok(hr) = r.u32().map(|v| v as i32) else { return codec::RPC_E_INVALID_DATA };
            if hr >= 0 && r.u8() == Ok(1) {
                let (Ok(bytes), Ok(index), Ok(form)) = (r.bytes(), r.u32(), r.u8()) else {
                    return codec::RPC_E_INVALID_DATA;
                };
                let Ok(lib) = file::read(&bytes) else { return codec::RPC_E_INVALID_DATA };
                let t = objects::new_typelib(lib, false, alloc::string::String::new(), None);
                // SAFETY: the block just made.
                unsafe { (*t).copy.store(true, Ordering::Release) };
                let form = if form == 1 { Form::DualInterface } else { Form::Stored };
                put(out, objects::new_typeinfo(t, index as usize, form));
                objects::lib_release(t);
            }
            hr
        }
        5 => {
            let riid = arg(args, 1) as *const Guid;
            let names = arg(args, 2) as *const *const u16;
            let n = arg(args, 3) as u32;
            let lcid = arg(args, 4) as u32;
            let ids = arg(args, 5) as *mut i32;
            if names.is_null() || ids.is_null() || n > MAX_ARGS {
                return E_INVALIDARG;
            }
            // SAFETY: the caller's IID or NULL.
            w.guid(&if riid.is_null() { IID_NULL } else { unsafe { *riid } });
            w.u32(n);
            for i in 0..n as usize {
                // SAFETY: the caller's array of `n` names.
                let s = unsafe { *names.add(i) };
                if s.is_null() {
                    return E_INVALIDARG;
                }
                let mut k = 0;
                // SAFETY: a NUL-terminated name.
                while unsafe { *s.add(k) } != 0 {
                    k += 1;
                }
                // SAFETY: `k` characters.
                w.wstr(unsafe { core::slice::from_raw_parts(s, k) });
            }
            w.u32(lcid);
            let reply = match send(chan, &iid, 5, &w.0) {
                Ok(r) => r,
                Err(hr) => return hr,
            };
            let mut r = R(&reply);
            let Ok(hr) = r.u32().map(|v| v as i32) else { return codec::RPC_E_INVALID_DATA };
            for i in 0..n as usize {
                let Ok(id) = r.u32() else { break };
                // SAFETY: the caller's array of `n` ids.
                unsafe { *ids.add(i) = id as i32 };
            }
            hr
        }
        _ => proxy_invoke(chan, &iid, args),
    }
}

fn proxy_invoke(chan: *mut c_void, iid: &Guid, args: *const u64) -> Hresult {
    let id = arg(args, 1) as i32;
    let riid = arg(args, 2) as *const Guid;
    let lcid = arg(args, 3) as u32;
    let flags = arg(args, 4) as u16;
    let params = arg(args, 5) as *const DispParams;
    let result = arg(args, 6) as *mut u8;
    let excep = arg(args, 7) as *mut ExcepInfo;
    let arg_err = arg(args, 8) as *mut u32;
    // SAFETY: the caller's IID or NULL.
    if !riid.is_null() && unsafe { *riid } != IID_NULL {
        return DISP_E_UNKNOWNINTERFACE;
    }
    if params.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DISPPARAMS.
    let dp = unsafe { &*params };
    if dp.c_args > MAX_ARGS || dp.c_named > dp.c_args || (dp.c_args > 0 && dp.args.is_null()) {
        return E_INVALIDARG;
    }
    let empty_result = || {
        if !result.is_null() {
            variant::set(result, 0, 0);
        }
    };
    let mut w = W::default();
    w.u32(id as u32);
    w.u32(lcid);
    w.u16(flags);
    w.u32(dp.c_args);
    for i in 0..dp.c_args as usize {
        if let Err(hr) = codec::enc(&mut w, &codec::Ty::Variant, dp.args.wrapping_add(i).cast()) {
            // Not sent: the result is left empty, as a call that fails before it is sent leaves it.
            empty_result();
            return hr;
        }
    }
    w.u32(dp.c_named);
    for i in 0..dp.c_named as usize {
        // SAFETY: the caller's named-argument ids.
        w.u32(unsafe { *dp.named.add(i) } as u32);
    }
    w.u8(u8::from(!result.is_null()));
    let reply = match send(chan, iid, 6, &w.0) {
        Ok(r) => r,
        // A failure of the channel leaves the out values alone (rule `dead_server`).
        Err(hr) => return hr,
    };
    empty_result();
    let mut r = R(&reply);
    let Ok(hr) = r.u32().map(|v| v as i32) else { return codec::RPC_E_INVALID_DATA };
    if !result.is_null() && r.u8() == Ok(1) {
        let mut tmp: Variant = [0; VARIANT_SIZE];
        if codec::dec(&mut r, &codec::Ty::Variant, tmp.as_mut_ptr()).is_err() {
            return codec::RPC_E_INVALID_DATA;
        }
        // SAFETY: the caller's VARIANT.
        unsafe { core::ptr::copy_nonoverlapping(tmp.as_ptr(), result, VARIANT_SIZE) };
    }
    if r.u8() == Ok(1) {
        let (code, scode) = (r.u16().unwrap_or(0), r.u32().unwrap_or(0) as i32);
        let source = read_opt_str(&mut r).unwrap_or(core::ptr::null_mut());
        let description = read_opt_str(&mut r).unwrap_or(core::ptr::null_mut());
        let help_file = read_opt_str(&mut r).unwrap_or(core::ptr::null_mut());
        let help_context = r.u32().unwrap_or(0);
        if excep.is_null() {
            for b in [source, description, help_file] {
                variant::free_bstr(b);
            }
        } else {
            // SAFETY: the caller's EXCEPINFO.
            unsafe {
                core::ptr::write_bytes(excep, 0, 1);
                (*excep).code = code;
                (*excep).scode = scode;
                (*excep).source = source;
                (*excep).description = description;
                (*excep).help_file = help_file;
                (*excep).help_context = help_context;
            }
        }
    }
    if let Ok(e) = r.u32() {
        put(arg_err, e);
    }
    hr
}

/// The stub side of slot `slot` (3..=6) on the object's interface `itf`.
pub(crate) fn stub_call(itf: *mut c_void, slot: u32, data: &[u8]) -> Result<Vec<u8>, Hresult> {
    let mut r = R(data);
    let mut w = W::default();
    match slot {
        3 => {
            let mut n = 0u32;
            let hr = vcall!(itf, 3, (*mut u32) -> Hresult, &mut n);
            w.u32(hr as u32);
            w.u32(n);
        }
        4 => {
            let (i, lcid) = (r.u32()?, r.u32()?);
            let mut ti: *mut c_void = core::ptr::null_mut();
            let hr = vcall!(itf, 4, (u32, u32, *mut *mut c_void) -> Hresult, i, lcid, &mut ti);
            match objects::typeinfo_of(ti) {
                Some(t) if hr >= 0 => {
                    // SAFETY: a live block of ours.
                    let (lib, index, form) = unsafe { ((*t).lib, (*t).index, (*t).form) };
                    w.u32(hr as u32);
                    w.u8(1);
                    w.bytes(&file::encode(&objects::model(lib)));
                    w.u32(index as u32);
                    w.u8(u8::from(form == Form::DualInterface));
                }
                _ => {
                    // A type info that is not Poly's cannot be sent by value.
                    w.u32(if hr >= 0 { E_NOTIMPL as u32 } else { hr as u32 });
                    w.u8(0);
                }
            }
            release(ti);
        }
        5 => {
            let riid = r.guid()?;
            let n = r.u32()?;
            if n > MAX_ARGS {
                return Err(codec::RPC_E_INVALID_DATA);
            }
            let mut names: Vec<Vec<u16>> = Vec::new();
            for _ in 0..n {
                let mut s = r.wstr()?;
                s.push(0);
                names.push(s);
            }
            let lcid = r.u32()?;
            let ptrs: Vec<*const u16> = names.iter().map(|s| s.as_ptr()).collect();
            let mut ids: Vec<i32> = alloc::vec![-1; n as usize];
            let hr = vcall!(itf, 5, (*const Guid, *const *const u16, u32, u32, *mut i32) -> Hresult, &riid, ptrs.as_ptr(), n, lcid, ids.as_mut_ptr());
            w.u32(hr as u32);
            for id in ids {
                w.u32(id as u32);
            }
        }
        _ => stub_invoke(itf, &mut r, &mut w)?,
    }
    Ok(w.0)
}

fn stub_invoke(itf: *mut c_void, r: &mut R, w: &mut W) -> Result<(), Hresult> {
    let id = r.u32()? as i32;
    let lcid = r.u32()?;
    let flags = r.u16()?;
    let n = r.u32()?;
    if n > MAX_ARGS {
        return Err(codec::RPC_E_INVALID_DATA);
    }
    let mut vars: Vec<Variant> = alloc::vec![[0; VARIANT_SIZE]; n as usize];
    let mut decoded = 0;
    let mut status = S_OK;
    for v in vars.iter_mut() {
        if let Err(hr) = codec::dec(r, &codec::Ty::Variant, v.as_mut_ptr()) {
            status = hr;
            break;
        }
        decoded += 1;
    }
    let free = |vars: &mut Vec<Variant>, k: usize| {
        for v in vars.iter_mut().take(k) {
            // SAFETY: variants we decoded.
            unsafe { codec::free_variant(v.as_mut_ptr()) };
        }
    };
    if status < 0 {
        free(&mut vars, decoded);
        return Err(status);
    }
    let c_named = r.u32()?;
    let mut named: Vec<i32> = Vec::new();
    for _ in 0..c_named.min(n) {
        named.push(r.u32()? as i32);
    }
    let want_result = r.u8()? != 0;
    let mut dp = DispParams {
        args: if vars.is_empty() { core::ptr::null_mut() } else { vars.as_mut_ptr() },
        named: if named.is_empty() { core::ptr::null_mut() } else { named.as_mut_ptr() },
        c_args: n,
        c_named: named.len() as u32,
    };
    let mut result: Variant = [0; VARIANT_SIZE];
    // SAFETY: a zeroed EXCEPINFO.
    let mut ex: ExcepInfo = unsafe { core::mem::zeroed() };
    let mut arg_err = 0u32;
    let res_ptr = if want_result { result.as_mut_ptr() } else { core::ptr::null_mut() };
    let hr = vcall!(itf, 6, (i32, *const Guid, u32, u16, *mut DispParams, *mut u8, *mut ExcepInfo, *mut u32) -> Hresult, id, &IID_NULL, lcid, flags, &mut dp, res_ptr, &mut ex, &mut arg_err);
    free(&mut vars, decoded);
    w.u32(hr as u32);
    if want_result {
        let mut rw = W::default();
        if codec::enc(&mut rw, &codec::Ty::Variant, result.as_ptr()).is_ok() {
            w.u8(1);
            w.0.extend_from_slice(&rw.0);
        } else {
            w.u8(0);
        }
        // SAFETY: the result the object wrote (or VT_EMPTY).
        unsafe { crate::VariantClear(result.as_mut_ptr()) };
    }
    if hr == DISP_E_EXCEPTION {
        w.u8(1);
        w.u16(ex.code);
        w.u32(ex.scode as u32);
        opt_str(w, ex.source);
        opt_str(w, ex.description);
        opt_str(w, ex.help_file);
        w.u32(ex.help_context);
    } else {
        w.u8(0);
    }
    for b in [ex.source, ex.description, ex.help_file] {
        variant::free_bstr(b);
    }
    w.u32(arg_err);
    Ok(())
}
