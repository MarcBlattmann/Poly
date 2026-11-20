//! COM security and caller identity (ADR 0220, B2c).
//!
//! - **Process security** (`CoInitializeSecurity`): once per process, locked also by the first
//!   attempt to reach a local server. The descriptor (or the `AppID`'s `AccessPermission`, or the
//!   default: the process's user, LocalSystem and Administrators) is what the exporter checks every
//!   incoming call's client against, with the client's token (`AccessCheck`).
//! - **The call context**: while the exporter runs a call, the serving thread holds the call
//!   (a TLS slot): the client's process (from the pipe), the impersonation level its proxy asked
//!   for, and whether it is an activation. `CoGetCallContext` hands out the call's
//!   `IServerSecurity`; `CoQueryClientBlanket`, `CoImpersonateClient` and `CoRevertToSelf` act on it;
//!   `rpcrt4!I_RpcOpenClientProcess` (a forwarder to this DLL) opens the caller's process, which for
//!   an activation is the activation process.
//!
//! Rules from the sheets `CoInitializeSecurity`, `CoGetCallContext.InCall`, `IServerSecurity`,
//! `CoQueryClientBlanket`, `CoImpersonateClient`, `CoRevertToSelf`, `AppID.Security` and
//! `rpcrt4.dll/I_RpcOpenClientProcess`; the decisions are `poly_com::security`'s.

use super::*;
use crate::{TlsAlloc, TlsFree, TlsGetValue, TlsSetValue};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_com::hresult::{
    CO_E_NOTINITIALIZED, E_INVALIDARG, E_NOINTERFACE, E_POINTER, RPC_E_TOO_LATE, S_OK, from_win32,
};
use poly_com::security::*;
use poly_com::wire::Caller;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn DuplicateTokenEx(token: Handle, access: u32, sa: *const c_void, level: u32, kind: u32, out: *mut Handle) -> i32;
    fn SetThreadToken(thread: *const Handle, token: Handle) -> i32;
    fn AccessCheck(
        sd: *const u8,
        token: Handle,
        desired: u32,
        mapping: *const GenericMapping,
        privileges: *mut u8,
        privileges_len: *mut u32,
        granted: *mut u32,
        status: *mut i32,
    ) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
    fn MakeSelfRelativeSD(absolute: *const c_void, relative: *mut u8, len: *mut u32) -> i32;
    fn GetNamedPipeClientProcessId(pipe: Handle, pid: *mut u32) -> i32;
    fn GetNamedPipeServerProcessId(pipe: Handle, pid: *mut u32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LookupAccountSidW(
        system: *const u16,
        sid: *const u8,
        name: *mut u16,
        cch_name: *mut u32,
        domain: *mut u16,
        cch_domain: *mut u32,
        kind: *mut u32,
    ) -> i32;
    fn LookupAccountNameW(
        system: *const u16,
        name: *const u16,
        sid: *mut u8,
        cb_sid: *mut u32,
        domain: *mut u16,
        cch_domain: *mut u32,
        kind: *mut u32,
    ) -> i32;
}

/// `GENERIC_MAPPING` (`winnt.h`).
#[repr(C)]
struct GenericMapping {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}

/// COM's rights have no generic forms: every generic right means `COM_RIGHTS_EXECUTE`.
const COM_MAPPING: GenericMapping =
    GenericMapping { read: COM_RIGHTS_EXECUTE, write: COM_RIGHTS_EXECUTE, execute: COM_RIGHTS_EXECUTE, all: 0x1F };

/// `PROCESS_QUERY_LIMITED_INFORMATION`; `TOKEN_DUPLICATE`, `TOKEN_QUERY`, `TOKEN_ALL_ACCESS`;
/// `TokenUser`, `TokenImpersonation` (`winnt.h`).
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TOKEN_DUPLICATE: u32 = 0x2;
const TOKEN_QUERY: u32 = 0x8;
const TOKEN_ALL_ACCESS: u32 = 0xF_01FF;
const TOKEN_USER: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;
/// `GetCurrentProcess()`.
const CURRENT_PROCESS: Handle = -1;
/// `ERROR_INVALID_PARAMETER`.
const ERROR_INVALID_PARAMETER: u32 = 87;

// ----- process security -------------------------------------------------------------------

/// Whom the process's objects admit.
#[derive(Clone)]
enum Access {
    /// `CoInitializeSecurity` with no descriptor: no check.
    Everyone,
    /// A self-relative descriptor.
    Descriptor(Vec<u8>),
}

struct State {
    /// `CoInitializeSecurity` succeeded, or the first out-of-process activation was tried.
    locked: bool,
    /// `None`: the default (the process's user, LocalSystem, Administrators).
    access: Option<Access>,
}

static STATE: Locked<State> = Locked::new(State { locked: false, access: None });

/// The first attempt to reach a local server sets the process's security (sheet
/// `CoInitializeSecurity`, rule `once_per_process`: also after a failed one).
pub(crate) fn lock() {
    STATE.with(|s| s.locked = true);
}

/// Whether the process's security is fixed (set, or a local server was tried).
pub(crate) fn is_locked() -> bool {
    STATE.with(|s| s.locked)
}

/// This process's user SID (bytes).
fn own_sid() -> Vec<u8> {
    static OWN: Locked<Vec<u8>> = Locked::new(Vec::new());
    let known = OWN.with(|v| v.clone());
    if !known.is_empty() {
        return known;
    }
    let mut t: Handle = 0;
    // SAFETY: the current-process pseudo-handle; the token is closed below.
    if unsafe { OpenProcessToken(CURRENT_PROCESS, TOKEN_QUERY, &mut t) } == 0 {
        return Vec::new();
    }
    let sid = token_user(t).unwrap_or_default();
    close(t);
    OWN.with(|v| *v = sid.clone());
    sid
}

/// The user SID of token `t`.
fn token_user(t: Handle) -> Option<Vec<u8>> {
    let mut buf = [0u64; 64];
    let mut len = 0u32;
    // SAFETY: a buffer of 512 bytes, 8-byte aligned.
    if unsafe { GetTokenInformation(t, TOKEN_USER, buf.as_mut_ptr().cast(), 512, &mut len) } == 0 {
        return None;
    }
    // SAFETY: TOKEN_USER starts with SID_AND_ATTRIBUTES, whose SID points into the buffer.
    unsafe {
        let sid = *(buf.as_ptr() as *const *const u8);
        if sid.is_null() {
            return None;
        }
        let n = 8 + 4 * usize::from(*sid.add(1));
        Some(core::slice::from_raw_parts(sid, n).to_vec())
    }
}

/// `<DOMAIN>\<user>` of token `t`.
fn token_account(t: Handle) -> Option<String> {
    let sid = token_user(t)?;
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut n, mut d, mut kind) = (256u32, 256u32, 0u32);
    // SAFETY: a SID and two buffers of 256 units.
    let ok = unsafe {
        LookupAccountSidW(
            core::ptr::null(),
            sid.as_ptr(),
            name.as_mut_ptr(),
            &mut n,
            domain.as_mut_ptr(),
            &mut d,
            &mut kind,
        )
    };
    if ok == 0 {
        return None;
    }
    let mut s = String::from_utf16_lossy(&domain[..(d as usize).min(256)]);
    s.push('\\');
    s.push_str(&String::from_utf16_lossy(&name[..(n as usize).min(256)]));
    Some(s)
}

/// The primary token of process `pid` (`TOKEN_DUPLICATE | TOKEN_QUERY`).
fn process_token(pid: u32) -> Option<Handle> {
    // SAFETY: closed below.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h == 0 {
        return None;
    }
    let mut t: Handle = 0;
    // SAFETY: a process handle of ours.
    let ok = unsafe { OpenProcessToken(h, TOKEN_DUPLICATE | TOKEN_QUERY, &mut t) } != 0;
    close(h);
    ok.then_some(t)
}

/// An impersonation token of process `pid`'s user at `SECURITY_IMPERSONATION_LEVEL` `level`.
fn client_token(pid: u32, level: u8) -> Option<Handle> {
    let t = process_token(pid)?;
    let mut out: Handle = 0;
    // SAFETY: a token of ours; a new impersonation token comes back.
    let ok = unsafe {
        DuplicateTokenEx(t, TOKEN_ALL_ACCESS, core::ptr::null(), u32::from(level.max(1)), TOKEN_IMPERSONATION, &mut out)
    } != 0;
    close(t);
    ok.then_some(out)
}

/// Whether `token` holds `rights` under `sd`.
fn access_check(sd: &[u8], token: Handle, rights: u32) -> bool {
    let mut privileges = [0u64; 32];
    let mut plen = 256u32;
    let (mut granted, mut status) = (0u32, 0i32);
    // SAFETY: a self-relative descriptor, an impersonation token, live out parameters.
    let ok = unsafe {
        AccessCheck(
            sd.as_ptr(),
            token,
            rights,
            &COM_MAPPING,
            privileges.as_mut_ptr().cast(),
            &mut plen,
            &mut granted,
            &mut status,
        )
    };
    ok != 0 && status != 0 && granted & rights == rights
}

/// Whether process `pid`'s user holds `rights` under the self-relative descriptor `sd`.
pub(crate) fn client_holds(sd: &[u8], pid: u32, rights: u32) -> Option<bool> {
    let t = client_token(pid, 1)?;
    let r = access_check(sd, t, rights);
    close(t);
    Some(r)
}

/// Whether this process's objects admit a call from process `pid` (sheet `CoInitializeSecurity`,
/// rule `access_check`).
pub(crate) fn admits(pid: u32) -> bool {
    let access = STATE.with(|s| s.access.clone());
    let sd = match access {
        Some(Access::Everyone) => return true,
        Some(Access::Descriptor(sd)) => sd,
        None => default_access(&own_sid()),
    };
    // A client whose token cannot be read is admitted only by the default descriptor.
    client_holds(&sd, pid, COM_RIGHTS_EXECUTE).unwrap_or(access_is_default())
}

fn access_is_default() -> bool {
    STATE.with(|s| s.access.is_none())
}

/// A value of `HKLM\SOFTWARE\Classes\<path>` (else `HKCU\Software\Classes\<path>`) as bytes.
pub(crate) fn class_value(path: &str, name: &str) -> Option<Vec<u8>> {
    const HKCU: isize = 0x8000_0001_u32 as i32 as isize;
    const HKLM: isize = 0x8000_0002_u32 as i32 as isize;
    for (root, prefix) in [(HKLM, "SOFTWARE\\Classes\\"), (HKCU, "Software\\Classes\\")] {
        let p = wide(&alloc::format!("{prefix}{path}"));
        let n = wide(name);
        let mut key = 0isize;
        // SAFETY: NUL-terminated strings; the key is closed below.
        unsafe {
            if RegOpenKeyExW(root, p.as_ptr(), 0, 0x2_0019, &mut key) != 0 {
                continue;
            }
            let mut buf = alloc::vec![0u8; 4096];
            let (mut kind, mut len) = (0u32, 4096u32);
            let r = RegQueryValueExW(key, n.as_ptr(), core::ptr::null_mut(), &mut kind, buf.as_mut_ptr(), &mut len);
            RegCloseKey(key);
            if r == 0 {
                buf.truncate(len as usize);
                return Some(buf);
            }
        }
    }
    None
}

/// The descriptor of `EOAC_APPID`: the `AppID`'s `AccessPermission` (none: the default).
fn appid_access(appid: &Guid) -> Result<Option<Access>, Hresult> {
    let text = String::from_utf16_lossy(&appid.to_text());
    let Some(sd) = class_value(&alloc::format!("AppID\\{text}"), "AccessPermission") else { return Ok(None) };
    match parse_self_relative(&sd) {
        Some(d) if d.masks_valid() => Ok(Some(Access::Descriptor(sd))),
        _ => Err(E_INVALIDARG),
    }
}

/// The absolute descriptor `sd` as a self-relative copy, checked (rule `descriptor_forms`).
unsafe fn absolute_descriptor(sd: *const c_void) -> Result<Vec<u8>, Hresult> {
    let ptr = core::mem::size_of::<usize>();
    // SAFETY: the caller's SECURITY_DESCRIPTOR: revision, Sbz1, Control, then (absolute) the
    // owner, group, SACL and DACL pointers, pointer-aligned (`winnt.h`).
    let (control, owner, group) = unsafe {
        let b = sd as *const u8;
        (u16::from_le_bytes([*b.add(2), *b.add(3)]), *(b.add(ptr) as *const usize), *(b.add(2 * ptr) as *const usize))
    };
    if control & SE_SELF_RELATIVE != 0 {
        return Err(INVALID_SECURITY_DESCR);
    }
    if owner == 0 || group == 0 {
        return Err(E_INVALIDARG);
    }
    let mut len = 0u32;
    // SAFETY: a size query, then the copy into a buffer of that size.
    unsafe {
        MakeSelfRelativeSD(sd, core::ptr::null_mut(), &mut len);
        if len == 0 {
            return Err(E_INVALIDARG);
        }
        let mut buf = alloc::vec![0u8; len as usize];
        if MakeSelfRelativeSD(sd, buf.as_mut_ptr(), &mut len) == 0 {
            return Err(E_INVALIDARG);
        }
        Ok(buf)
    }
}

/// `CoInitializeSecurity` (sheet `CoInitializeSecurity`): needs COM in the process, works once; the
/// arguments are checked first (a refused call does not lock). A descriptor or `AppID` whose
/// entries lack `COM_RIGHTS_EXECUTE` is refused, and the process then admits nobody.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoInitializeSecurity(
    sd: *mut c_void,
    _count: i32,
    _services: *mut c_void,
    _reserved1: *mut c_void,
    authn_level: u32,
    imp_level: u32,
    _auth_list: *mut c_void,
    capabilities: u32,
    _reserved3: *mut c_void,
) -> Hresult {
    if crate::ACTIVE_APARTMENTS.load(Ordering::Acquire) == 0 {
        return CO_E_NOTINITIALIZED;
    }
    if STATE.with(|s| s.locked) {
        return RPC_E_TOO_LATE;
    }
    if let Err(hr) = check_init_args(authn_level, imp_level, capabilities) {
        return hr;
    }
    let deny_all = || STATE.with(|s| s.access = Some(Access::Descriptor(descriptor(&own_sid(), &[]))));
    let access = if capabilities & EOAC_APPID != 0 {
        if sd.is_null() {
            None
        } else {
            // SAFETY: with EOAC_APPID the argument is the AppID's GUID.
            match appid_access(unsafe { &*(sd as *const Guid) }) {
                Ok(a) => a,
                Err(hr) => {
                    deny_all();
                    return hr;
                }
            }
        }
    } else if sd.is_null() {
        Some(Access::Everyone)
    } else {
        // SAFETY: the caller's descriptor.
        let bytes = match unsafe { absolute_descriptor(sd) } {
            Ok(b) => b,
            Err(hr) => return hr,
        };
        if !parse_self_relative(&bytes).is_some_and(|d| d.masks_valid()) {
            deny_all();
            return E_INVALIDARG;
        }
        Some(Access::Descriptor(bytes))
    };
    STATE.with(|s| {
        if s.locked {
            return RPC_E_TOO_LATE;
        }
        s.locked = true;
        s.access = access;
        S_OK
    })
}

// ----- caller identity ----------------------------------------------------------------------

/// The process at the other end of a pipe connection.
pub(crate) fn pipe_client(h: Handle) -> u32 {
    let mut pid = 0u32;
    // SAFETY: a pipe handle of ours.
    unsafe { GetNamedPipeClientProcessId(h, &mut pid) };
    pid
}

/// The activation process (`comlaunch.exe`), as seen when this process last talked to it.
static ACTIVATOR: AtomicU32 = AtomicU32::new(0);

/// Note the server end of a connection to the activation pipe.
pub(crate) fn note_activator(h: Handle) {
    let mut pid = 0u32;
    // SAFETY: a pipe handle of ours.
    if unsafe { GetNamedPipeServerProcessId(h, &mut pid) } != 0 && pid != 0 {
        ACTIVATOR.store(pid, Ordering::Release);
    }
}

/// The server account of a proxy's object: `<DOMAIN>\<user>` of the exporter's process (sheet
/// `CoQueryProxyBlanket`, rule `local_proxy_defaults`). Asked once per process, kept for proxies
/// whose server has gone.
pub(crate) fn principal_of(endpoint: &str) -> Option<String> {
    static KNOWN: Locked<Vec<(u32, Option<String>)>> = Locked::new(Vec::new());
    let pid = poly_com::marshal::parse_endpoint(endpoint).map_or(super::pid(), |(p, _)| p);
    if let Some(n) = KNOWN.with(|k| k.iter().find(|e| e.0 == pid).map(|e| e.1.clone())) {
        return n;
    }
    let name = if pid == super::pid() {
        let mut t: Handle = 0;
        // SAFETY: the current-process pseudo-handle; closed below.
        if unsafe { OpenProcessToken(CURRENT_PROCESS, TOKEN_QUERY, &mut t) } != 0 {
            let n = token_account(t);
            close(t);
            n
        } else {
            None
        }
    } else {
        process_token(pid).and_then(|t| {
            let n = token_account(t);
            close(t);
            n
        })
    };
    KNOWN.with(|k| k.push((pid, name.clone())));
    name
}

/// Whether `name` names an account (a principal name of `CoSetProxyBlanket`).
pub(crate) fn names_account(name: &str) -> bool {
    let n = wide(name);
    let mut sid = [0u8; 68];
    let mut domain = [0u16; 256];
    let (mut cb, mut cd, mut kind) = (68u32, 256u32, 0u32);
    // SAFETY: a NUL-terminated name and buffers of the sizes given.
    unsafe {
        LookupAccountNameW(
            core::ptr::null(),
            n.as_ptr(),
            sid.as_mut_ptr(),
            &mut cb,
            domain.as_mut_ptr(),
            &mut cd,
            &mut kind,
        ) != 0
    }
}

// ----- the call context -----------------------------------------------------------------------

/// One incoming call, held by the thread that serves it.
struct Call {
    /// The client's process.
    pid: u32,
    caller: Caller,
    /// The client's impersonation token at the call's level (0 until needed).
    token: Handle,
    impersonating: bool,
    /// `<DOMAIN>\<user>` of the client, NUL-terminated (empty until asked).
    privs: Vec<u16>,
    /// The call's `IServerSecurity` (0 until asked); the call holds one reference.
    iss: usize,
}

static CALL_SLOT: AtomicU32 = AtomicU32::new(u32::MAX);

fn call_slot() -> Option<u32> {
    let s = CALL_SLOT.load(Ordering::Acquire);
    if s != u32::MAX {
        return Some(s);
    }
    // SAFETY: kernel32's TLS.
    let n = unsafe { TlsAlloc() };
    if n == u32::MAX {
        return None;
    }
    match CALL_SLOT.compare_exchange(u32::MAX, n, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Some(n),
        Err(other) => {
            // SAFETY: the slot just allocated, unused.
            unsafe { TlsFree(n) };
            Some(other)
        }
    }
}

/// The call this thread serves.
fn current() -> Option<*mut Call> {
    let s = CALL_SLOT.load(Ordering::Acquire);
    if s == u32::MAX {
        return None;
    }
    // SAFETY: our TLS slot.
    let p = unsafe { TlsGetValue(s) }.cast::<Call>();
    (!p.is_null()).then_some(p)
}

/// Run `f` (on the thread that serves the call) as the call from process `pid` described by
/// `caller`: the call context exists until `f` returns, then impersonation ends and the call's
/// `IServerSecurity` goes stale.
pub(crate) fn serve<R>(pid: u32, caller: Caller, f: impl FnOnce() -> R) -> R {
    let Some(slot) = call_slot() else { return f() };
    let call = Box::into_raw(Box::new(Call { pid, caller, token: 0, impersonating: false, privs: Vec::new(), iss: 0 }));
    // SAFETY: our TLS slot; a call made by an outer call is put back afterwards.
    let outer = unsafe { TlsGetValue(slot) };
    // SAFETY: as above.
    unsafe { TlsSetValue(slot, call.cast()) };
    let r = f();
    // SAFETY: as above; the call block is ours again.
    let call = unsafe {
        TlsSetValue(slot, outer);
        Box::from_raw(call)
    };
    if call.impersonating {
        // SAFETY: the serving thread reverts to its own token.
        unsafe { SetThreadToken(core::ptr::null(), 0) };
    }
    if call.iss != 0 {
        let iss = call.iss as *mut ServerSecurity;
        // SAFETY: the call's reference of its IServerSecurity.
        unsafe {
            (*iss).call.store(0, Ordering::Release);
            iss_release(iss);
        }
    }
    close(call.token);
    r
}

impl Call {
    /// The impersonation level the server's thread token gets: an activation impersonates, a
    /// direct call follows its proxy's blanket (rule `impersonation_level_follows_call`).
    fn level(&self) -> u8 {
        if self.caller.activation { 2 } else { self.caller.level.max(1) }
    }

    fn privs(&mut self) -> *const u16 {
        if self.privs.is_empty() {
            let name = process_token(self.pid)
                .and_then(|t| {
                    let n = token_account(t);
                    close(t);
                    n
                })
                .unwrap_or_default();
            self.privs = wide(&name);
        }
        self.privs.as_ptr()
    }

    fn impersonate(&mut self) -> Hresult {
        if self.token == 0 {
            match client_token(self.pid, self.level()) {
                Some(t) => self.token = t,
                None => return E_ACCESSDENIED,
            }
        }
        // SAFETY: an impersonation token of ours, on this thread.
        if unsafe { SetThreadToken(core::ptr::null(), self.token) } == 0 {
            // SAFETY: no arguments.
            return from_win32(unsafe { GetLastError() });
        }
        self.impersonating = true;
        S_OK
    }

    fn revert(&mut self) -> Hresult {
        if self.impersonating {
            // SAFETY: back to the thread's own token.
            unsafe { SetThreadToken(core::ptr::null(), 0) };
            self.impersonating = false;
        }
        S_OK
    }
}

/// The server-side blanket (rule `query_blanket`): the integers whatever happens, the strings only
/// during the call.
unsafe fn write_server_blanket(
    call: Option<*mut Call>,
    authn: *mut u32,
    authz: *mut u32,
    princ: *mut *mut u16,
    level: *mut u32,
    imp: *mut u32,
    privs: *mut *const u16,
    caps: *mut u32,
) -> Hresult {
    // SAFETY: the caller's optional out parameters; a live call.
    unsafe {
        for (p, v) in [(authn, RPC_C_AUTHN_WINNT), (authz, RPC_C_AUTHZ_NONE), (level, 6), (imp, 1), (caps, 0)] {
            if !p.is_null() {
                *p = v;
            }
        }
        let Some(call) = call else { return RPC_E_CALL_COMPLETE };
        if !princ.is_null() {
            *princ = core::ptr::null_mut();
        }
        if !privs.is_null() {
            *privs = (*call).privs();
        }
    }
    S_OK
}

// ----- IServerSecurity ----------------------------------------------------------------------

#[repr(C)]
struct ServerSecurity {
    vtbl: *const IssVtbl,
    refs: AtomicU32,
    /// The call (0 once it ended).
    call: AtomicUsize,
}

#[repr(C)]
struct IssVtbl {
    qi: unsafe extern "system" fn(*mut ServerSecurity, *const Guid, *mut *mut c_void) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut ServerSecurity) -> u32,
    release: unsafe extern "system" fn(*mut ServerSecurity) -> u32,
    query_blanket: unsafe extern "system" fn(
        *mut ServerSecurity,
        *mut u32,
        *mut u32,
        *mut *mut u16,
        *mut u32,
        *mut u32,
        *mut *const u16,
        *mut u32,
    ) -> Hresult,
    impersonate_client: unsafe extern "system" fn(*mut ServerSecurity) -> Hresult,
    revert_to_self: unsafe extern "system" fn(*mut ServerSecurity) -> Hresult,
    is_impersonating: unsafe extern "system" fn(*mut ServerSecurity) -> i32,
}

// SAFETY: a table of function pointers.
unsafe impl Sync for IssVtbl {}

static ISS: IssVtbl = IssVtbl {
    qi: iss_qi,
    add_ref: iss_add_ref,
    release: iss_release,
    query_blanket: iss_query_blanket,
    impersonate_client: iss_impersonate,
    revert_to_self: iss_revert,
    is_impersonating: iss_is_impersonating,
};

/// `IID_IServerSecurity` (`objidl.h`): {0000013E-0000-0000-C000-000000000046}.
const IID_ISERVER_SECURITY: Guid = Guid { data1: 0x13E, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

fn iss_call(this: *mut ServerSecurity) -> Option<*mut Call> {
    // SAFETY: a live object.
    let c = unsafe { (*this).call.load(Ordering::Acquire) };
    (c != 0).then_some(c as *mut Call)
}

unsafe extern "system" fn iss_qi(this: *mut ServerSecurity, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID and out pointer; a live object.
    unsafe {
        let g = *iid;
        if g == crate::IID_IUNKNOWN || g == IID_ISERVER_SECURITY {
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            *out = this.cast();
            S_OK
        } else {
            *out = core::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn iss_add_ref(this: *mut ServerSecurity) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn iss_release(this: *mut ServerSecurity) -> u32 {
    // SAFETY: a live object; the last reference frees it.
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            drop(Box::from_raw(this));
        }
        n
    }
}

unsafe extern "system" fn iss_query_blanket(
    this: *mut ServerSecurity,
    authn: *mut u32,
    authz: *mut u32,
    princ: *mut *mut u16,
    level: *mut u32,
    imp: *mut u32,
    privs: *mut *const u16,
    caps: *mut u32,
) -> Hresult {
    // SAFETY: the caller's out parameters.
    unsafe { write_server_blanket(iss_call(this), authn, authz, princ, level, imp, privs, caps) }
}

unsafe extern "system" fn iss_impersonate(this: *mut ServerSecurity) -> Hresult {
    match iss_call(this) {
        // SAFETY: the live call.
        Some(c) => unsafe { (*c).impersonate() },
        None => RPC_E_CALL_COMPLETE,
    }
}

unsafe extern "system" fn iss_revert(this: *mut ServerSecurity) -> Hresult {
    match iss_call(this) {
        // SAFETY: the live call.
        Some(c) => unsafe { (*c).revert() },
        None => S_OK,
    }
}

unsafe extern "system" fn iss_is_impersonating(this: *mut ServerSecurity) -> i32 {
    // SAFETY: the live call.
    iss_call(this).is_some_and(|c| unsafe { (*c).impersonating }) as i32
}

// ----- the exports ------------------------------------------------------------------------

/// `CoGetCallContext` (sheets `CoGetCallContext.InCall`, `CoGetCallContext`): the call's
/// `IServerSecurity` for `IID_IServerSecurity` or `IID_IUnknown` (the same object for the whole
/// call), `E_NOINTERFACE` for anything else; on a thread that serves no call
/// `RPC_E_CALL_COMPLETE` for every interface id, with or without COM, the output left as it was.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoGetCallContext(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    let Some(call) = current() else { return RPC_E_CALL_COMPLETE };
    if out.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = core::ptr::null_mut() };
    // SAFETY: the caller's IID.
    let g = unsafe { *iid };
    if g != IID_ISERVER_SECURITY && g != crate::IID_IUNKNOWN {
        return E_NOINTERFACE;
    }
    // SAFETY: this thread's live call.
    unsafe {
        if (*call).iss == 0 {
            let iss = Box::into_raw(Box::new(ServerSecurity {
                vtbl: &ISS,
                refs: AtomicU32::new(1),
                call: AtomicUsize::new(call as usize),
            }));
            (*call).iss = iss as usize;
        }
        let iss = (*call).iss as *mut ServerSecurity;
        iss_add_ref(iss);
        *out = iss.cast();
    }
    S_OK
}

/// `CoQueryClientBlanket` (sheet `CoQueryClientBlanket`): the call's blanket as the server sees
/// it; `RPC_E_CALL_COMPLETE` (outputs untouched) off a call.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CoQueryClientBlanket(
    authn: *mut u32,
    authz: *mut u32,
    princ: *mut *mut u16,
    level: *mut u32,
    imp: *mut u32,
    privs: *mut *const u16,
    caps: *mut u32,
) -> Hresult {
    let Some(call) = current() else { return RPC_E_CALL_COMPLETE };
    // SAFETY: the caller's out parameters; this thread's call.
    unsafe { write_server_blanket(Some(call), authn, authz, princ, level, imp, privs, caps) }
}

/// `CoImpersonateClient` (sheet `CoImpersonateClient`): the serving thread takes the client's
/// token at the call's impersonation level.
#[unsafe(no_mangle)]
pub extern "system" fn CoImpersonateClient() -> Hresult {
    match current() {
        // SAFETY: this thread's call.
        Some(c) => unsafe { (*c).impersonate() },
        None => RPC_E_CALL_COMPLETE,
    }
}

/// `CoRevertToSelf` (sheet `CoRevertToSelf`): ends the call's impersonation; off a call it leaves
/// any other impersonation alone.
#[unsafe(no_mangle)]
pub extern "system" fn CoRevertToSelf() -> Hresult {
    match current() {
        // SAFETY: this thread's call.
        Some(c) => unsafe { (*c).revert() },
        None => RPC_E_CALL_COMPLETE,
    }
}

/// `rpcrt4!I_RpcOpenClientProcess(NULL, access, &process)` (rpcrt4's export forwards here;
/// sheet `I_RpcOpenClientProcess`): the process of the call's immediate caller, the activation
/// process for the `CreateInstance` of an activation; `RPC_S_NO_CALL_ACTIVE` off a call, the
/// error of `OpenProcess` when it cannot be opened.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn I_RpcOpenClientProcess(_binding: *mut c_void, access: u32, out: *mut Handle) -> u32 {
    let Some(call) = current() else { return RPC_S_NO_CALL_ACTIVE };
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: this thread's call.
    let (pid, activation) = unsafe { ((*call).pid, (*call).caller.activation) };
    let activator = ACTIVATOR.load(Ordering::Acquire);
    let pid = if activation && activator != 0 { activator } else { pid };
    // SAFETY: the handle goes to the caller.
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h == 0 {
        // SAFETY: no arguments.
        return unsafe { GetLastError() };
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = h };
    0
}
