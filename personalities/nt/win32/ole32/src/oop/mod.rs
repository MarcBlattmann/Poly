//! COM across apartments and processes (ADR 0220): class registrations, the server count,
//! initialisation spies, proxies, the object exporter and activation through `comlaunch.exe`.
//!
//! - [`entries`]: the exported functions of batch B2a.
//! - [`activate`]: `CoGetClassObject` and `CoCreateInstance[Ex]` — the class table, the
//!   registry (`InprocServer32`, `LocalServer32`, `AppID\LocalService`), the activation process.
//! - [`proxy`]: the proxy objects (`IUnknown`, `IClassFactory`, `IMarshal`) for another apartment
//!   of this process or another process.
//! - [`exporter`]: this process's named pipe serving calls on the objects it handed out.
//! - [`apt`]: running a call in an apartment (an STA's message-only window, an MTA thread).
//! - [`pipe`]: one request and one reply over a named pipe; starting `comlaunch.exe`.
//! - [`security`]: `CoInitializeSecurity`, the access check of incoming calls, the call context
//!   (`CoGetCallContext`, `IServerSecurity`, impersonation) and `I_RpcOpenClientProcess` (B2c).
//!
//! The decisions are `poly_com::{classtable, activation, spy, wire}`'s (host-tested).

pub(crate) mod activate;
pub(crate) mod apt;
pub(crate) mod channel;
pub(crate) mod entries;
pub(crate) mod exporter;
pub(crate) mod guard;
pub(crate) mod marshal;
pub(crate) mod pipe;
pub(crate) mod proxy;
pub(crate) mod security;

use crate::Locked;
use core::ffi::c_void;
use poly_com::classtable::{ClassTable, ServerCount};
use poly_com::guid::Guid;
use poly_com::hresult::Hresult;
use poly_com::spy::Spies;

pub(crate) type Handle = isize;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub(crate) fn CreateThread(
        security: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        arg: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> Handle;
    pub(crate) fn CreateEventW(security: *const c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    pub(crate) fn SetEvent(event: Handle) -> i32;
    pub(crate) fn Sleep(ms: u32);
    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn GetCurrentProcessId() -> u32;
    pub(crate) fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        security: *const c_void,
    ) -> Handle;
    pub(crate) fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    pub(crate) fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    pub(crate) fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    pub(crate) fn ReadFile(file: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    pub(crate) fn WriteFile(file: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    pub(crate) fn CreateMutexW(security: *const c_void, owner: i32, name: *const u16) -> Handle;
    pub(crate) fn ReleaseMutex(mutex: Handle) -> i32;
    pub(crate) fn GetModuleHandleW(name: *const u16) -> usize;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    pub(crate) fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32, out: *mut isize) -> i32;
    pub(crate) fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> i32;
    pub(crate) fn RegCloseKey(key: isize) -> i32;
}

/// `INVALID_HANDLE_VALUE`.
pub(crate) const INVALID_HANDLE: Handle = -1;

/// The registrations of the process.
pub(crate) static CLASSES: Locked<ClassTable> = Locked::new(ClassTable::new());
/// The server count (`CoAddRefServerProcess`).
pub(crate) static COUNT: Locked<ServerCount> = Locked::new(ServerCount::new());
/// The initialisation spies.
pub(crate) static SPIES: Locked<Spies> = Locked::new(Spies::new());

/// `IID_IClassFactory` (`unknwn.h`): {00000001-0000-0000-C000-000000000046}.
pub(crate) const IID_ICLASS_FACTORY: Guid =
    Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// `IID_IInitializeSpy` (`objidl.h`): {00000034-0000-0000-C000-000000000046}.
pub(crate) const IID_IINITIALIZE_SPY: Guid =
    Guid { data1: 0x34, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// `RPC_E_DISCONNECTED` (`winerror.h`): the object's server let go of it.
pub(crate) const RPC_E_DISCONNECTED: Hresult = 0x8001_0108_u32 as i32;
/// `HRESULT_FROM_WIN32(RPC_S_SERVER_UNAVAILABLE)`: the object's process is gone.
pub(crate) const RPC_S_SERVER_UNAVAILABLE: Hresult = 0x8007_06BA_u32 as i32;
/// `HRESULT_FROM_WIN32(RPC_S_CALL_FAILED)`: the object's process ended during the call (sheets
/// `CoUnmarshalInterface`, `GlobalOptions.ExceptionHandling`).
pub(crate) const RPC_S_CALL_FAILED: Hresult = 0x8007_06BE_u32 as i32;

/// `IUnknown::QueryInterface` of a COM object.
pub(crate) unsafe fn query(p: *mut c_void, iid: &Guid) -> (Hresult, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: the caller's COM object; slot 0 of every vtable.
    let hr = unsafe {
        let vtbl =
            *(p as *const *const unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult);
        (*vtbl)(p, iid, &mut out)
    };
    if hr < 0 {
        out = core::ptr::null_mut();
    }
    (hr, out)
}

/// `IClassFactory::CreateInstance` (slot 3).
pub(crate) unsafe fn factory_create(f: *mut c_void, outer: *mut c_void, iid: &Guid) -> (Hresult, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: an IClassFactory pointer.
    let hr = unsafe {
        let vtbl = *(f as *const *const usize);
        let create: unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> Hresult =
            core::mem::transmute(*vtbl.add(3));
        create(f, outer, iid, &mut out)
    };
    if hr < 0 {
        out = core::ptr::null_mut();
    }
    (hr, out)
}

/// `IClassFactory::LockServer` (slot 4).
pub(crate) unsafe fn factory_lock(f: *mut c_void, lock: bool) -> Hresult {
    // SAFETY: an IClassFactory pointer.
    unsafe {
        let vtbl = *(f as *const *const usize);
        let l: unsafe extern "system" fn(*mut c_void, i32) -> Hresult = core::mem::transmute(*vtbl.add(4));
        l(f, i32::from(lock))
    }
}

/// A NUL-terminated UTF-16 copy of `s`.
pub(crate) fn wide(s: &str) -> alloc::vec::Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// This process's id.
pub(crate) fn pid() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetCurrentProcessId() }
}

/// Close a kernel handle.
pub(crate) fn close(h: Handle) {
    if h != 0 && h != INVALID_HANDLE {
        // SAFETY: a handle this module opened, closed once.
        unsafe { crate::CloseHandle(h as usize) };
    }
}
