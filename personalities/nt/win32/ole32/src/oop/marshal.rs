//! Standard COM marshalling (`CoMarshalInterface`, `CoUnmarshalInterface`, `CoReleaseMarshalData`,
//! `CoGetMarshalSizeMax`, `CoGetStandardMarshal`, `CoGetPSClsid`, `CoRegisterPSClsid`; ADR 0220,
//! B2b).
//!
//! Uses [`poly_com::marshal`] for the `OBJREF` wire representation (`MEOW`; 68-byte
//! `OBJREF_STANDARD` for `LOCAL`/`NOSHAREDMEM`, `OBJREF_CUSTOM` for `INPROC`) and the per-process
//! table of live `NORMAL` / `TABLESTRONG` / `TABLEWEAK` entries.

use super::*;
use crate::{IID_IMARSHAL, IID_IUNKNOWN, add_ref_unknown, com_usable, current_owner, release_unknown, stream};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::git::Owner;
use poly_com::guid::Guid;
use poly_com::hresult::{
    CO_E_CLASSSTRING, CO_E_NOTINITIALIZED, CO_E_OBJNOTCONNECTED, E_INVALIDARG, E_NOINTERFACE, E_POINTER, Hresult,
    REGDB_E_IIDNOTREG, RPC_E_INVALID_OBJREF, S_OK,
};
use poly_com::marshal::{
    CLSID_STD_MARSHAL, MSHCTX_INPROC, MarshalEntry, MarshalKind, MarshalTable, OBJREF_CUSTOM, OBJREF_HEADER_LEN,
    OBJREF_STANDARD, ObjRefData, check_context, decode_custom_head, decode_custom_payload, decode_header,
    decode_standard_body, endpoint_for, marshal_size_max, objref_flags_for, parse_endpoint,
};
use poly_com::wire::ObjRef;

/// `HKEY_CURRENT_USER`, `HKEY_LOCAL_MACHINE` (`winreg.h`).
const HKCU: isize = 0x8000_0001_u32 as i32 as isize;
const HKLM: isize = 0x8000_0002_u32 as i32 as isize;
const KEY_READ: u32 = 0x2_0019;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;

/// Per-process proxy/stub class id cache (`CoGetPSClsid` / `CoRegisterPSClsid`).
static PS_CACHE: Locked<Vec<(Guid, Guid)>> = Locked::new(Vec::new());

/// Per-process table of live marshal packets (`NORMAL`, `TABLESTRONG`, `TABLEWEAK`).
static MARSHALS: Locked<MarshalTable> = Locked::new(MarshalTable::new());

/// Clear the process-wide PSClsid cache when the last apartment in the process ends
/// (sheet `CoRegisterPSClsid`, rule `scope`).
pub(crate) fn clear_ps_cache() {
    PS_CACHE.with(|c| c.clear());
}

fn read_reg_default(root: isize, sub: &str) -> Option<String> {
    let p = wide(sub);
    let mut key = 0isize;
    // SAFETY: NUL-terminated path, live out pointer; closed below.
    unsafe {
        if RegOpenKeyExW(root, p.as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return None;
        }
        let mut buf: Vec<u16> = alloc::vec![0; 256];
        let mut kind = 0u32;
        let mut len = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(
            key,
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut kind,
            buf.as_mut_ptr().cast(),
            &mut len,
        );
        RegCloseKey(key);
        if r != 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
            return None;
        }
        let units = &buf[..(len as usize / 2).min(buf.len())];
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        Some(String::from_utf16_lossy(&units[..end]))
    }
}

/// Look up the proxy/stub CLSID for `iid` in the cache or the merged HKCU/HKLM `Interface` view.
pub(crate) fn ps_clsid_lookup(iid: &Guid) -> Result<Guid, Hresult> {
    if let Some(clsid) = PS_CACHE.with(|c| c.iter().find(|(i, _)| i == iid).map(|(_, cl)| *cl)) {
        return Ok(clsid);
    }
    let text = String::from_utf16_lossy(&iid.to_text());
    let hkcu_path = alloc::format!(r"Software\Classes\Interface\{text}\ProxyStubClsid32");
    let hklm_path = alloc::format!(r"SOFTWARE\Classes\Interface\{text}\ProxyStubClsid32");
    let val = read_reg_default(HKCU, &hkcu_path).or_else(|| read_reg_default(HKLM, &hklm_path));
    let Some(val) = val else {
        return Err(REGDB_E_IIDNOTREG);
    };
    let units: Vec<u16> = val.encode_utf16().collect();
    let Some(clsid) = Guid::parse(&units) else {
        return Err(CO_E_CLASSSTRING);
    };
    PS_CACHE.with(|c| {
        if let Some(slot) = c.iter_mut().find(|(i, _)| i == iid) {
            slot.1 = clsid;
        } else {
            c.push((*iid, clsid));
        }
    });
    Ok(clsid)
}

/// `CoGetPSClsid` (sheet `CoGetPSClsid`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetPSClsid(riid: *const Guid, pclsid: *mut Guid) -> Hresult {
    if riid.is_null() || pclsid.is_null() {
        return E_INVALIDARG;
    }
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    // SAFETY: the caller's pointers.
    let iid = unsafe { *riid };
    match ps_clsid_lookup(&iid) {
        Ok(clsid) => {
            // SAFETY: the caller's out parameter.
            unsafe { *pclsid = clsid };
            S_OK
        }
        Err(hr) => {
            // SAFETY: the caller's out parameter.
            unsafe { *pclsid = Guid::default() };
            hr
        }
    }
}

/// `CoRegisterPSClsid` (sheet `CoRegisterPSClsid`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoRegisterPSClsid(riid: *const Guid, rclsid: *const Guid) -> Hresult {
    if riid.is_null() || rclsid.is_null() {
        return E_INVALIDARG;
    }
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    // SAFETY: the caller's pointers.
    let (iid, clsid) = unsafe { (*riid, *rclsid) };
    PS_CACHE.with(|c| {
        if let Some(slot) = c.iter_mut().find(|(i, _)| *i == iid) {
            slot.1 = clsid;
        } else {
            c.push((iid, clsid));
        }
    });
    S_OK
}

// ----- marshal table lifetime helpers -----------------------------------------------------------

fn release_entry(e: MarshalEntry) {
    // SAFETY: the entry owns one COM reference on `e.object`.
    unsafe { release_unknown(e.object as *mut c_void) };
    if !e.is_remote_proxy && e.oid != 0 {
        exporter::release_refs(e.oid, 1);
    }
}

/// Release all marshal entries belonging to `owner` when its apartment ends.
pub(crate) fn apartment_ended(owner: Owner) {
    let gone = MARSHALS.with(|m| m.remove_apartment(owner));
    for e in gone {
        release_entry(e);
    }
}

/// Release all marshal entries for local object `identity` (`CoDisconnectObject`).
pub(crate) fn disconnect(identity: usize) {
    let gone = MARSHALS.with(|m| m.remove_object(identity));
    for e in gone {
        release_entry(e);
    }
}

// ----- stream OBJREF helpers --------------------------------------------------------------------

unsafe fn read_objref(pstm: *mut c_void) -> Result<ObjRefData, Hresult> {
    let mut head = [0u8; OBJREF_HEADER_LEN];
    // SAFETY: the caller's IStream pointer.
    let hr = unsafe { stream::read_exact(pstm, &mut head) };
    if hr < 0 {
        return Err(hr);
    }
    let hdr = decode_header(&head)?;
    match hdr.flags {
        OBJREF_STANDARD => {
            let mut body = [0u8; 44];
            // SAFETY: the caller's IStream pointer.
            let hr = unsafe { stream::read_exact(pstm, &mut body) };
            if hr < 0 {
                return Err(hr);
            }
            let (data, mut extra) = decode_standard_body(hdr.iid, &body);
            let mut sink = [0u8; 64];
            while extra > 0 {
                let n = extra.min(sink.len());
                // SAFETY: the caller's IStream pointer.
                let hr = unsafe { stream::read_exact(pstm, &mut sink[..n]) };
                if hr < 0 {
                    return Err(hr);
                }
                extra -= n;
            }
            Ok(data)
        }
        OBJREF_CUSTOM => {
            let mut chead = [0u8; 24];
            // SAFETY: the caller's IStream pointer.
            let hr = unsafe { stream::read_exact(pstm, &mut chead) };
            if hr < 0 {
                return Err(hr);
            }
            let (clsid, size) = decode_custom_head(&chead)?;
            if clsid != CLSID_STD_MARSHAL || size > 4096 {
                return Err(RPC_E_INVALID_OBJREF);
            }
            let mut payload = alloc::vec![0u8; size];
            // SAFETY: the caller's IStream pointer.
            let hr = unsafe { stream::read_exact(pstm, &mut payload) };
            if hr < 0 {
                return Err(hr);
            }
            decode_custom_payload(hdr.iid, &payload)
        }
        _ => Err(RPC_E_INVALID_OBJREF),
    }
}

// ----- exported CoMarshal family ----------------------------------------------------------------

/// `CoGetMarshalSizeMax` (sheet `CoGetMarshalSizeMax`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetMarshalSizeMax(
    pul_size: *mut u32,
    _riid: *const Guid,
    punk: *mut c_void,
    dw_dest_context: u32,
    _pv_dest_context: *mut c_void,
    _mshlflags: u32,
) -> Hresult {
    if pul_size.is_null() || punk.is_null() {
        return E_INVALIDARG;
    }
    if let Err(hr) = check_context(dw_dest_context) {
        return hr;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *pul_size = marshal_size_max(dw_dest_context) };
    S_OK
}

/// `CoMarshalInterface` (sheet `CoMarshalInterface`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoMarshalInterface(
    pstm: *mut c_void,
    riid: *const Guid,
    punk: *mut c_void,
    dw_dest_context: u32,
    _pv_dest_context: *mut c_void,
    mshlflags: u32,
) -> Hresult {
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    if pstm.is_null() || punk.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    if let Err(hr) = check_context(dw_dest_context) {
        return hr;
    }
    // SAFETY: the caller's IID.
    let iid = unsafe { *riid };
    // Verify the object implements `iid` before writing anything.
    // SAFETY: the caller's COM object.
    let (hr, itf) = unsafe { query(punk, &iid) };
    if hr < 0 || itf.is_null() {
        return E_NOINTERFACE;
    }
    // SAFETY: temporary reference from `query`.
    unsafe { release_unknown(itf) };

    // Non-built-in interfaces must have a registered proxy/stub (sheet: `no_proxy_stub`).
    if iid != IID_IUNKNOWN && iid != IID_ICLASS_FACTORY {
        if let Err(hr) = ps_clsid_lookup(&iid) {
            return hr;
        }
    }

    let kind = MarshalKind::from_flags(mshlflags);
    let flags = objref_flags_for(dw_dest_context);

    let data = match proxy::marshal_info(punk) {
        Some(proxy::ProxyMarshalInfo::Remote { endpoint, oid }) => {
            let (remote_pid, remote_oxid) = parse_endpoint(&endpoint).unwrap_or((pid(), 0));
            // Hold a reference on the proxy while the marshal packet is live.
            // SAFETY: the caller's proxy object.
            let (_, px_unk) = unsafe { query(punk, &IID_IUNKNOWN) };
            let px_id = if px_unk.is_null() {
                // SAFETY: fall back to the passed pointer.
                unsafe { add_ref_unknown(punk) };
                punk as usize
            } else {
                px_unk as usize
            };
            let token = MARSHALS.with(|m| m.insert(0, px_id, current_owner(), kind, true));
            ObjRefData { flags, iid, mshlflags, pid: remote_pid, oxid: remote_oxid, oid, token }
        }
        Some(proxy::ProxyMarshalInfo::Local { owner, object }) => {
            // SAFETY: the local proxy's live target identity.
            unsafe { add_ref_unknown(object as *mut c_void) };
            let (oxid, oid) = if dw_dest_context == MSHCTX_INPROC {
                (0, 0)
            } else {
                // SAFETY: a second reference handed over to the exporter.
                unsafe { add_ref_unknown(object as *mut c_void) };
                let oref = exporter::export_object(object, owner);
                (exporter::oxid(), oref.oid)
            };
            let token = MARSHALS.with(|m| m.insert(oid, object, owner, kind, false));
            ObjRefData { flags, iid, mshlflags, pid: pid(), oxid, oid, token }
        }
        None => {
            // SAFETY: get the object's canonical IUnknown identity.
            let (_, id) = unsafe { query(punk, &IID_IUNKNOWN) };
            let identity = if id.is_null() {
                // SAFETY: fall back to the caller's pointer.
                unsafe { add_ref_unknown(punk) };
                punk as usize
            } else {
                id as usize
            };
            let owner = current_owner();
            let (oxid, oid) = if dw_dest_context == MSHCTX_INPROC {
                (0, 0)
            } else {
                // SAFETY: a second reference handed over to the exporter.
                unsafe { add_ref_unknown(identity as *mut c_void) };
                let oref = exporter::export_object(identity, owner);
                (exporter::oxid(), oref.oid)
            };
            let token = MARSHALS.with(|m| m.insert(oid, identity, owner, kind, false));
            ObjRefData { flags, iid, mshlflags, pid: pid(), oxid, oid, token }
        }
    };

    let bytes = data.encode();
    // SAFETY: the caller's IStream pointer.
    let hr = unsafe { stream::write_all(pstm, &bytes) };
    if hr < 0 {
        if let Some(entry) = MARSHALS.with(|m| m.release_data(data.token)) {
            release_entry(entry);
        }
        return hr;
    }
    S_OK
}

/// `CoUnmarshalInterface` (sheet `CoUnmarshalInterface`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoUnmarshalInterface(
    pstm: *mut c_void,
    riid: *const Guid,
    ppv: *mut *mut c_void,
) -> Hresult {
    // NULL stream leaves `*ppv` untouched (sheet `CoUnmarshalInterface`, rule `damaged_streams`).
    if pstm.is_null() || ppv.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *ppv = core::ptr::null_mut() };
    if riid.is_null() {
        return E_INVALIDARG;
    }
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    // SAFETY: the caller's IStream pointer.
    let data = match unsafe { read_objref(pstm) } {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    // SAFETY: the caller's IID.
    let want_iid = unsafe { *riid };

    if data.pid == pid() {
        let Some((entry, consumed)) = MARSHALS.with(|m| m.unmarshal(data.token)) else {
            return CO_E_OBJNOTCONNECTED;
        };
        if entry.owner == current_owner() {
            // Same apartment: return the original object's interface pointer.
            // SAFETY: the marshal entry holds a live reference on `entry.object`.
            let (hr, p) = unsafe { query(entry.object as *mut c_void, &want_iid) };
            if consumed {
                release_entry(entry);
            }
            if hr < 0 || p.is_null() {
                return if hr < 0 { hr } else { E_NOINTERFACE };
            }
            // SAFETY: the caller's out parameter.
            unsafe { *ppv = p };
            return S_OK;
        }
        // Different apartment of this process: return a local proxy.
        let px = proxy::local(entry.owner, entry.object);
        if consumed {
            release_entry(entry);
        }
        // SAFETY: `px` is our local proxy's IUnknown with one reference.
        let (hr, p) = unsafe { query(px, &want_iid) };
        // SAFETY: release our temporary reference on `px`.
        unsafe { release_unknown(px) };
        if hr < 0 || p.is_null() {
            return if hr < 0 { hr } else { E_NOINTERFACE };
        }
        // SAFETY: the caller's out parameter.
        unsafe { *ppv = p };
        return S_OK;
    }

    // Object from another process: if this process marshalled a remote proxy itself, release its
    // local token and return the proxy.
    if let Some((entry, consumed)) = MARSHALS.with(|m| m.unmarshal(data.token)) {
        // SAFETY: the marshal entry holds a live reference on the proxy.
        let (hr, p) = unsafe { query(entry.object as *mut c_void, &want_iid) };
        if consumed {
            release_entry(entry);
        }
        if hr < 0 || p.is_null() {
            return if hr < 0 { hr } else { E_NOINTERFACE };
        }
        // SAFETY: the caller's out parameter.
        unsafe { *ppv = p };
        return S_OK;
    }

    let endpoint = endpoint_for(data.pid, data.oxid);
    let px = proxy::remote(ObjRef { endpoint, oid: data.oid });
    // SAFETY: `px` is our remote proxy's IUnknown with one reference.
    let (hr, p) = unsafe { query(px, &want_iid) };
    // SAFETY: release our temporary reference on `px`.
    unsafe { release_unknown(px) };
    if hr < 0 || p.is_null() {
        return if hr < 0 { hr } else { E_NOINTERFACE };
    }
    // SAFETY: the caller's out parameter.
    unsafe { *ppv = p };
    S_OK
}

/// `CoReleaseMarshalData` (sheet `CoReleaseMarshalData`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoReleaseMarshalData(pstm: *mut c_void) -> Hresult {
    if pstm.is_null() {
        return E_INVALIDARG;
    }
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    // SAFETY: the caller's IStream pointer.
    let data = match unsafe { read_objref(pstm) } {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    if let Some(entry) = MARSHALS.with(|m| m.release_data(data.token)) {
        release_entry(entry);
    }
    S_OK
}

// ----- CoGetStandardMarshal and StdMarshal (`IMarshal`) -----------------------------------------

#[repr(C)]
struct StdMarshal {
    vtbl: *const StdMarshalVtbl,
    refs: AtomicU32,
    object: *mut c_void,
}

#[repr(C)]
struct StdMarshalVtbl {
    qi: unsafe extern "system" fn(*mut StdMarshal, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut StdMarshal) -> u32,
    release: unsafe extern "system" fn(*mut StdMarshal) -> u32,
    get_unmarshal_class: unsafe extern "system" fn(
        *mut StdMarshal,
        *const Guid,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut Guid,
    ) -> Hresult,
    get_marshal_size_max: unsafe extern "system" fn(
        *mut StdMarshal,
        *const Guid,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
        *mut u32,
    ) -> Hresult,
    marshal_interface: unsafe extern "system" fn(
        *mut StdMarshal,
        *mut c_void,
        *const Guid,
        *mut c_void,
        u32,
        *mut c_void,
        u32,
    ) -> Hresult,
    unmarshal_interface: unsafe extern "system" fn(*mut StdMarshal, *mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    release_marshal_data: unsafe extern "system" fn(*mut StdMarshal, *mut c_void) -> Hresult,
    disconnect_object: unsafe extern "system" fn(*mut StdMarshal, u32) -> Hresult,
}

// SAFETY: table of function pointers.
unsafe impl Sync for StdMarshalVtbl {}

static STD_MARSHAL_VTBL: StdMarshalVtbl = StdMarshalVtbl {
    qi: sm_qi,
    add_ref: sm_add_ref,
    release: sm_release,
    get_unmarshal_class: sm_get_unmarshal_class,
    get_marshal_size_max: sm_get_marshal_size_max,
    marshal_interface: sm_marshal_interface,
    unmarshal_interface: sm_unmarshal_interface,
    release_marshal_data: sm_release_marshal_data,
    disconnect_object: sm_disconnect_object,
};

unsafe extern "system" fn sm_qi(this: *mut StdMarshal, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; a live object.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_POINTER;
        }
        let g = *iid;
        if g == IID_IUNKNOWN || g == IID_IMARSHAL {
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            *out = this.cast();
            S_OK
        } else {
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn sm_add_ref(this: *mut StdMarshal) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn sm_release(this: *mut StdMarshal) -> u32 {
    // SAFETY: a live object from `Box::into_raw`.
    let n = unsafe { (*this).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: last reference.
        let b = unsafe { Box::from_raw(this) };
        if !b.object.is_null() {
            // SAFETY: the reference taken in `CoGetStandardMarshal`.
            unsafe { release_unknown(b.object) };
        }
    }
    n
}

unsafe extern "system" fn sm_get_unmarshal_class(
    _this: *mut StdMarshal,
    _riid: *const Guid,
    _pv: *mut c_void,
    _ctx: u32,
    _pv_ctx: *mut c_void,
    _flags: u32,
    pcid: *mut Guid,
) -> Hresult {
    if pcid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *pcid = CLSID_STD_MARSHAL };
    S_OK
}

unsafe extern "system" fn sm_get_marshal_size_max(
    this: *mut StdMarshal,
    riid: *const Guid,
    pv: *mut c_void,
    ctx: u32,
    pv_ctx: *mut c_void,
    flags: u32,
    psize: *mut u32,
) -> Hresult {
    // SAFETY: live `StdMarshal` and caller pointers.
    let target = if pv.is_null() { unsafe { (*this).object } } else { pv };
    // SAFETY: delegates to `CoGetMarshalSizeMax`.
    unsafe { CoGetMarshalSizeMax(psize, riid, target, ctx, pv_ctx, flags) }
}

unsafe extern "system" fn sm_marshal_interface(
    this: *mut StdMarshal,
    pstm: *mut c_void,
    riid: *const Guid,
    pv: *mut c_void,
    ctx: u32,
    pv_ctx: *mut c_void,
    flags: u32,
) -> Hresult {
    // SAFETY: live `StdMarshal` and caller pointers.
    let target = if pv.is_null() { unsafe { (*this).object } } else { pv };
    // SAFETY: delegates to `CoMarshalInterface`.
    unsafe { CoMarshalInterface(pstm, riid, target, ctx, pv_ctx, flags) }
}

unsafe extern "system" fn sm_unmarshal_interface(
    _this: *mut StdMarshal,
    pstm: *mut c_void,
    riid: *const Guid,
    ppv: *mut *mut c_void,
) -> Hresult {
    // SAFETY: delegates to `CoUnmarshalInterface`.
    unsafe { CoUnmarshalInterface(pstm, riid, ppv) }
}

unsafe extern "system" fn sm_release_marshal_data(_this: *mut StdMarshal, pstm: *mut c_void) -> Hresult {
    // SAFETY: delegates to `CoReleaseMarshalData`.
    unsafe { CoReleaseMarshalData(pstm) }
}

unsafe extern "system" fn sm_disconnect_object(this: *mut StdMarshal, reserved: u32) -> Hresult {
    // SAFETY: live `StdMarshal`.
    let target = unsafe { (*this).object };
    if !target.is_null() {
        // SAFETY: `target` is a live COM object we hold a reference on.
        return unsafe { entries::CoDisconnectObject(target, reserved) };
    }
    S_OK
}

/// `CoGetStandardMarshal` (sheet `CoGetStandardMarshal`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetStandardMarshal(
    _riid: *const Guid,
    punk: *mut c_void,
    _dw_dest_context: u32,
    _pv_dest_context: *mut c_void,
    _mshlflags: u32,
    pp_marshal: *mut *mut c_void,
) -> Hresult {
    if pp_marshal.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter (cleared even when COM is not initialised: sheet).
    unsafe { *pp_marshal = core::ptr::null_mut() };
    if !com_usable() {
        return CO_E_NOTINITIALIZED;
    }
    if !punk.is_null() {
        // SAFETY: the caller's COM object.
        unsafe { add_ref_unknown(punk) };
    }
    let m = Box::into_raw(Box::new(StdMarshal { vtbl: &STD_MARSHAL_VTBL, refs: AtomicU32::new(1), object: punk }));
    // SAFETY: the caller's out parameter.
    unsafe { *pp_marshal = m.cast() };
    S_OK
}

// ----- proxy `IMarshal` methods (called from `proxy.rs`'s `MAR` vtable) -------------------------

pub(crate) unsafe extern "system" fn proxy_get_unmarshal_class(
    _this: *mut c_void,
    _riid: *const Guid,
    _pv: *mut c_void,
    _ctx: u32,
    _pv_ctx: *mut c_void,
    _flags: u32,
    pcid: *mut Guid,
) -> Hresult {
    if pcid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *pcid = CLSID_STD_MARSHAL };
    S_OK
}

pub(crate) unsafe extern "system" fn proxy_get_marshal_size_max(
    this: *mut c_void,
    riid: *const Guid,
    pv: *mut c_void,
    ctx: u32,
    pv_ctx: *mut c_void,
    flags: u32,
    psize: *mut u32,
) -> Hresult {
    let target = if pv.is_null() { this } else { pv };
    // SAFETY: delegates to `CoGetMarshalSizeMax`.
    unsafe { CoGetMarshalSizeMax(psize, riid, target, ctx, pv_ctx, flags) }
}

pub(crate) unsafe extern "system" fn proxy_marshal_interface(
    this: *mut c_void,
    pstm: *mut c_void,
    riid: *const Guid,
    pv: *mut c_void,
    ctx: u32,
    pv_ctx: *mut c_void,
    flags: u32,
) -> Hresult {
    let target = if pv.is_null() { this } else { pv };
    // SAFETY: delegates to `CoMarshalInterface`.
    unsafe { CoMarshalInterface(pstm, riid, target, ctx, pv_ctx, flags) }
}

pub(crate) unsafe extern "system" fn proxy_unmarshal_interface(
    _this: *mut c_void,
    pstm: *mut c_void,
    riid: *const Guid,
    ppv: *mut *mut c_void,
) -> Hresult {
    // SAFETY: delegates to `CoUnmarshalInterface`.
    unsafe { CoUnmarshalInterface(pstm, riid, ppv) }
}

pub(crate) unsafe extern "system" fn proxy_release_marshal_data(_this: *mut c_void, pstm: *mut c_void) -> Hresult {
    // SAFETY: delegates to `CoReleaseMarshalData`.
    unsafe { CoReleaseMarshalData(pstm) }
}

pub(crate) unsafe extern "system" fn proxy_disconnect_object(_this: *mut c_void, _reserved: u32) -> Hresult {
    S_OK
}
