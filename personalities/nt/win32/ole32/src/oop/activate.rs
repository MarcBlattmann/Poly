//! Finding a class object (ADR 0220): the class table of this process first, then the registry
//! (`InprocServer32` for the in-process context, `LocalServer32` / `AppID\LocalService` for the
//! local server through the activation process), else `REGDB_E_CLASSNOTREG`.
//!
//! Rules from the sheets `CoCreateInstanceEx` (`registered_factory_wins`,
//! `inproc_and_local_entries`, `contexts_local_server`, `hive_*`, `local_service`) and
//! `CoRegisterClassObject`.

use super::*;
use crate::{current_owner, release_unknown};
use alloc::string::String;
use alloc::vec::Vec;
use poly_com::activation::{ClassRegistry, Hive, command_program, find_class, local_plan};
use poly_com::classtable::{
    CLSCTX_ACTIVATE_32_BIT_SERVER, CLSCTX_ACTIVATE_64_BIT_SERVER, CLSCTX_INPROC_HANDLER, CLSCTX_INPROC_SERVER,
    CLSCTX_LOCAL_SERVER, Found,
};
use poly_com::hresult::{E_NOINTERFACE, REGDB_E_CLASSNOTREG, from_win32};
use poly_com::wire::{Plan, Request};

/// `HKEY_CURRENT_USER`, `HKEY_LOCAL_MACHINE` (`winreg.h`, sign-extended as the header defines them).
const HKCU: isize = 0x8000_0001_u32 as i32 as isize;
const HKLM: isize = 0x8000_0002_u32 as i32 as isize;
/// `KEY_READ`.
const KEY_READ: u32 = 0x2_0019;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;

/// The registry, read with `advapi32`'s functions.
struct WinRegistry;

fn root(h: Hive) -> isize {
    match h {
        Hive::CurrentUser => HKCU,
        Hive::LocalMachine => HKLM,
    }
}

impl ClassRegistry for WinRegistry {
    fn value(&self, hive: Hive, path: &str, name: Option<&str>) -> Option<String> {
        let p = wide(path);
        let n = name.map(wide);
        let mut key = 0isize;
        // SAFETY: NUL-terminated strings, live out parameters; the key is closed below.
        unsafe {
            if RegOpenKeyExW(root(hive), p.as_ptr(), 0, KEY_READ, &mut key) != 0 {
                return None;
            }
            let mut buf: Vec<u16> = alloc::vec![0; 1024];
            let mut kind = 0u32;
            let mut len = (buf.len() * 2) as u32;
            let r = RegQueryValueExW(
                key,
                n.as_ref().map_or(core::ptr::null(), |n| n.as_ptr()),
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

    fn exists(&self, hive: Hive, path: &str) -> bool {
        let p = wide(path);
        let mut key = 0isize;
        // SAFETY: a NUL-terminated path; the key is closed at once.
        unsafe {
            if RegOpenKeyExW(root(hive), p.as_ptr(), 0, KEY_READ, &mut key) != 0 {
                return false;
            }
            RegCloseKey(key);
        }
        true
    }
}

/// The machine of a program's image (`IMAGE_FILE_MACHINE_*`, PE format), if it can be read.
fn image_machine(path: &str) -> Option<u16> {
    let p = wide(path);
    // SAFETY: a NUL-terminated path; GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING; closed below.
    let h = unsafe { CreateFileW(p.as_ptr(), 0x8000_0000, 1, core::ptr::null(), 3, 0, 0) };
    if h == INVALID_HANDLE {
        return None;
    }
    let mut head = [0u8; 1024];
    let mut n = 0u32;
    // SAFETY: reads into `head`.
    let ok = unsafe { ReadFile(h, head.as_mut_ptr(), head.len() as u32, &mut n, core::ptr::null_mut()) } != 0;
    close(h);
    if !ok || (n as usize) < 0x40 || &head[..2] != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(head[0x3C..0x40].try_into().ok()?) as usize;
    if pe + 6 > n as usize || &head[pe..pe + 4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([head[pe + 4], head[pe + 5]]))
}

/// `CLSCTX_ACTIVATE_32_BIT_SERVER` / `_64_BIT_SERVER` against the server's image (sheet:
/// `LOCAL|ACTIVATE_32_BIT_SERVER` for a 64-bit server starts nothing, `REGDB_E_CLASSNOTREG`).
fn bitness_ok(ctx: u32, plan: &Plan) -> bool {
    let Plan::Command(cmd) = plan else { return true };
    let want = ctx & (CLSCTX_ACTIVATE_32_BIT_SERVER | CLSCTX_ACTIVATE_64_BIT_SERVER);
    if want == 0 || want == CLSCTX_ACTIVATE_32_BIT_SERVER | CLSCTX_ACTIVATE_64_BIT_SERVER {
        return true;
    }
    match image_machine(command_program(cmd)) {
        Some(0x8664) => want == CLSCTX_ACTIVATE_64_BIT_SERVER,
        Some(0x014C) => want == CLSCTX_ACTIVATE_32_BIT_SERVER,
        _ => true,
    }
}

/// An in-process server DLL's class object.
fn inproc_class_object(dll: &str, clsid: &Guid, iid: &Guid, ctx: u32) -> Result<*mut c_void, Hresult> {
    let p = wide(dll);
    // SAFETY: a NUL-terminated path.
    let module = unsafe { crate::LoadLibraryW(p.as_ptr()) };
    if module == 0 {
        // SAFETY: no arguments.
        let e = unsafe { GetLastError() };
        // A request that allows an in-process handler too ends as "not registered" (sheet:
        // CLSCTX_ALL gives 0x80040154 where INPROC|LOCAL gives the load error).
        return Err(if ctx & CLSCTX_INPROC_HANDLER != 0 { REGDB_E_CLASSNOTREG } else { from_win32(e) });
    }
    // SAFETY: the module just loaded; the export's documented prototype (Learn: "DllGetClassObject").
    unsafe {
        let f = crate::GetProcAddress(module, b"DllGetClassObject\0".as_ptr());
        if f == 0 {
            return Err(REGDB_E_CLASSNOTREG);
        }
        let get: unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> Hresult =
            core::mem::transmute(f);
        let mut out: *mut c_void = core::ptr::null_mut();
        let hr = get(clsid, iid, &mut out);
        if hr < 0 || out.is_null() { Err(if hr < 0 { hr } else { E_NOINTERFACE }) } else { Ok(out) }
    }
}

/// The class object of `clsid` for context `ctx`, as interface `iid` (one reference), or why not.
/// The caller has checked that COM works on the thread.
pub(crate) fn class_object(clsid: &Guid, ctx: u32, iid: &Guid) -> Result<*mut c_void, Hresult> {
    let owner = current_owner();
    match CLASSES.with(|t| t.lookup(clsid, ctx, owner)) {
        Found::Here(object) => {
            // SAFETY: a registered object (the registration holds a reference).
            let (hr, p) = unsafe { query(object as *mut c_void, iid) };
            return if p.is_null() { Err(if hr < 0 { hr } else { E_NOINTERFACE }) } else { Ok(p) };
        }
        Found::Elsewhere { owner, object } => {
            let px = proxy::local(owner, object);
            // SAFETY: the proxy's IUnknown with one reference, given back after the query.
            let (hr, p) = unsafe { query(px, iid) };
            // SAFETY: as above.
            unsafe { release_unknown(px) };
            return if p.is_null() { Err(if hr < 0 { hr } else { E_NOINTERFACE }) } else { Ok(p) };
        }
        Found::Nothing => {}
    }
    if ctx & (CLSCTX_INPROC_SERVER | CLSCTX_INPROC_HANDLER | CLSCTX_LOCAL_SERVER) == 0 {
        return Err(REGDB_E_CLASSNOTREG);
    }
    let text = String::from_utf16_lossy(&clsid.to_text());
    let entry = find_class(&WinRegistry, &text);
    if ctx & CLSCTX_INPROC_SERVER != 0 {
        if let Some(dll) = entry.as_ref().and_then(|e| e.inproc_server.clone()) {
            // The local entry is not tried after an in-process one (sheet: INPROC|LOCAL).
            return inproc_class_object(&dll, clsid, iid, ctx);
        }
        // The classes of Poly's own DLLs (the shell's file operation, taskbar and jump lists, the
        // Save dialog: ADR 0223), served as an `InprocServer32` class is.
        if let Some(dll) = poly_com::inproc::server_of(clsid) {
            return inproc_class_object(dll.trim_end_matches(char::from(0)), clsid, iid, ctx);
        }
    }
    if ctx & CLSCTX_LOCAL_SERVER == 0 {
        return Err(REGDB_E_CLASSNOTREG);
    }
    // The shell link answers a local-server request too (ShellLink.yaml); Poly's DLL serves it here.
    if entry.is_none()
        && poly_com::inproc::served_for_local_server(clsid)
        && let Some(dll) = poly_com::inproc::server_of(clsid)
    {
        return inproc_class_object(dll.trim_end_matches(char::from(0)), clsid, iid, ctx);
    }
    let plan = entry.as_ref().and_then(local_plan);
    if let Some(p) = &plan {
        if !bitness_ok(ctx, p) {
            return Err(REGDB_E_CLASSNOTREG);
        }
    }
    // Without a plan only a running server could have published the class: the activation process
    // is asked if it runs, and not started for it.
    let start = plan.is_some();
    // A server withdraws its registrations without waiting for the activation process, so a
    // factory served at that moment can be disconnected already: then the class is asked for again.
    for attempt in 0.. {
        let req = Request::Activate { clsid: *clsid, plan: plan.clone() };
        let reply = match pipe::activation(&req, start) {
            Ok(r) => r,
            Err(pipe::Failure::NoServer) => return Err(REGDB_E_CLASSNOTREG),
            Err(f) => return Err(f.hr()),
        };
        let Some(factory) = reply.obj.filter(|_| reply.hr >= 0) else {
            return Err(if reply.hr < 0 { reply.hr } else { REGDB_E_CLASSNOTREG });
        };
        let px = proxy::remote(factory);
        let alive = proxy::ping(px);
        if !alive && attempt < 3 {
            // SAFETY: the proxy's IUnknown with one reference.
            unsafe { release_unknown(px) };
            // SAFETY: a plain sleep, for the withdrawal to arrive.
            unsafe { Sleep(20) };
            continue;
        }
        // SAFETY: the proxy's IUnknown with one reference, given back after the query.
        let (hr, p) = unsafe { query(px, iid) };
        // SAFETY: as above.
        unsafe { release_unknown(px) };
        return if p.is_null() { Err(if hr < 0 { hr } else { E_NOINTERFACE }) } else { Ok(p) };
    }
    Err(REGDB_E_CLASSNOTREG)
}

/// Make one object with the class factory `f` (one reference, given back), asking for `iid`.
pub(crate) fn create_with(f: *mut c_void, outer: *mut c_void, iid: &Guid) -> Result<*mut c_void, Hresult> {
    // SAFETY: an IClassFactory pointer with one reference, given back below.
    let (hr, o) = unsafe { factory_create(f, outer, iid) };
    // SAFETY: as above.
    unsafe { release_unknown(f) };
    if o.is_null() { Err(if hr < 0 { hr } else { E_NOINTERFACE }) } else { Ok(o) }
}
