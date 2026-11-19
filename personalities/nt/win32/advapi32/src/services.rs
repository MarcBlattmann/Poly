//! The service functions (ADR 0221): the calls of programs that manage services and of the service
//! processes themselves. Each sends one message of `poly_scm::wire` to the service control manager
//! in the NT personality through `ntdll!PolyServiceCall` and lays the reply out in the caller's
//! structures (`winsvc.h`, native pointer width). Waits (`StartServiceW` until the service's process
//! connected, `ControlService` until its handler returned, a dispatcher until a control comes) are
//! waits on the event handles the manager hands out.
//!
//! Behaviour from the spec sheets of the 20 entries (`specs/windows/advapi32.dll/`, SERVICES.md).

use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use poly_scm::consts::*;
use poly_scm::wire::{Config2, ConfigReply, FailureActions, Reader, Status, Writer, join_multi, op};

type ScHandle = usize;
type Bool = i32;

const PTR: usize = core::mem::size_of::<usize>();
/// `STATUS_BUFFER_TOO_SMALL`.
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023_u32 as i32;
/// `STATUS_TIMEOUT`.
const STATUS_TIMEOUT: i32 = 0x102;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    /// Poly's own: one message to the service control manager (ADR 0221); an `NTSTATUS`.
    fn PolyServiceCall(op: u32, input: *const u8, in_len: usize, out: *mut u8, out_len: usize, ret: *mut u32) -> i32;
    fn NtWaitForSingleObject(handle: isize, alertable: u8, timeout: *const i64) -> i32;
    fn NtClose(handle: isize) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetLastError(error: u32);
    fn GetComputerNameW(buf: *mut u16, size: *mut u32) -> i32;
    fn CreateThread(
        attrs: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> isize;
}

fn fail(e: u32) -> Bool {
    // SAFETY: sets the thread's last error.
    unsafe { SetLastError(e) };
    0
}

fn fail_h(e: u32) -> ScHandle {
    fail(e);
    0
}

fn succeed() -> Bool {
    // SAFETY: sets the thread's last error.
    unsafe { SetLastError(0) };
    1
}

/// Send `req` as operation `operation`: the reply after its error code, or the error.
fn call(operation: u32, req: &Writer) -> Result<Vec<u8>, u32> {
    let mut out = vec![0u8; 16 * 1024];
    loop {
        let mut ret = 0u32;
        // SAFETY: the request and the reply buffer live for the call; `ret` is a live out value.
        let st =
            unsafe { PolyServiceCall(operation, req.0.as_ptr(), req.0.len(), out.as_mut_ptr(), out.len(), &mut ret) };
        if st == STATUS_BUFFER_TOO_SMALL && (ret as usize) > out.len() {
            // Only the replies of queries grow (the others are a few bytes): asking again is safe.
            out = vec![0u8; ret as usize];
            continue;
        }
        if st < 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let n = (ret as usize).min(out.len());
        let mut r = Reader::new(&out[..n]);
        return match r.u32() {
            Some(0) => Ok(out[4..n].to_vec()),
            Some(e) => Err(e),
            None => Err(ERROR_INVALID_PARAMETER),
        };
    }
}

/// A NUL-terminated string at `p` (`None` for NULL).
fn wstr(p: *const u16) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    let mut v = Vec::new();
    let mut i = 0;
    // SAFETY: the caller's NUL-terminated string.
    unsafe {
        while *p.add(i) != 0 && i < 0x8000 {
            v.push(*p.add(i));
            i += 1;
        }
    }
    Some(v)
}

/// A double-NUL-terminated list at `p` (`None` for NULL).
fn multi(p: *const u16) -> Option<Vec<Vec<u16>>> {
    if p.is_null() {
        return None;
    }
    let mut out = Vec::new();
    let mut at = p;
    loop {
        let s = wstr(at).unwrap_or_default();
        if s.is_empty() {
            break;
        }
        // SAFETY: the next string of the caller's list starts after this one's NUL.
        at = unsafe { at.add(s.len() + 1) };
        out.push(s);
        if out.len() > 4096 {
            break;
        }
    }
    Some(out)
}

fn handle_of(r: Result<Vec<u8>, u32>) -> ScHandle {
    match r {
        Ok(b) => match Reader::new(&b).u64() {
            Some(h) => {
                succeed();
                h as usize
            }
            None => fail_h(ERROR_INVALID_PARAMETER),
        },
        Err(e) => fail_h(e),
    }
}

fn done(r: Result<Vec<u8>, u32>) -> Bool {
    match r {
        Ok(_) => succeed(),
        Err(e) => fail(e),
    }
}

fn eq_ascii(s: &[u16], t: &str) -> bool {
    s.len() == t.len() && s.iter().zip(t.bytes()).all(|(&a, b)| a < 0x80 && (a as u8).eq_ignore_ascii_case(&b))
}

// ----- the manager

/// `OpenSCManagerW(lpMachineName, lpDatabaseName, dwDesiredAccess)`: the machine may be NULL, empty,
/// "." or this computer's name (with or without `\\`); the database NULL or `ServicesActive`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> ScHandle {
    if let Some(m) = wstr(machine).filter(|m| !m.is_empty() && !eq_ascii(m, ".")) {
        let mut me = [0u16; 64];
        let mut n = 64u32;
        // SAFETY: a 64-unit buffer and its length.
        unsafe { GetComputerNameW(me.as_mut_ptr(), &mut n) };
        let bare = if m.starts_with(&[92, 92]) { &m[2..] } else { &m[..] };
        let mine = &me[..n as usize];
        let same = bare.len() == mine.len()
            && bare
                .iter()
                .zip(mine)
                .all(|(&a, &b)| a == b || (a < 0x80 && b < 0x80 && (a as u8).eq_ignore_ascii_case(&(b as u8))));
        if !same {
            // RPC_S_SERVER_UNAVAILABLE: Poly reaches no other machine.
            return fail_h(1722);
        }
    }
    if let Some(d) = wstr(database) {
        if eq_ascii(&d, "ServicesFailed") {
            return fail_h(ERROR_DATABASE_DOES_NOT_EXIST);
        }
        if !eq_ascii(&d, "ServicesActive") {
            return fail_h(ERROR_INVALID_NAME);
        }
    }
    let mut w = Writer::new();
    w.u32(access);
    handle_of(call(op::OPEN_MANAGER, &w))
}

/// `CreateServiceW`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn CreateServiceW(
    scm: ScHandle,
    name: *const u16,
    display: *const u16,
    access: u32,
    ty: u32,
    start: u32,
    error: u32,
    path: *const u16,
    group: *const u16,
    tag: *mut u32,
    deps: *const u16,
    account: *const u16,
    password: *const u16,
) -> ScHandle {
    let mut w = Writer::new();
    w.u64(scm as u64).opt(wstr(name).as_deref()).opt(wstr(display).as_deref());
    w.u32(access).u32(ty).u32(start).u32(error);
    w.opt(wstr(path).as_deref()).opt(wstr(group).as_deref()).opt_list(multi(deps).as_deref());
    w.opt(wstr(account).as_deref()).opt(wstr(password).as_deref()).u32(u32::from(cfg!(target_arch = "x86")));
    let h = handle_of(call(op::CREATE, &w));
    if h != 0 && !tag.is_null() {
        // SAFETY: the caller's tag; Poly has no load-order tags.
        unsafe { tag.write(0) };
    }
    h
}

/// `OpenServiceW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenServiceW(scm: ScHandle, name: *const u16, access: u32) -> ScHandle {
    let mut w = Writer::new();
    w.u64(scm as u64).opt(wstr(name).as_deref()).u32(access);
    handle_of(call(op::OPEN_SERVICE, &w))
}

/// `CloseServiceHandle`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CloseServiceHandle(h: ScHandle) -> Bool {
    let mut w = Writer::new();
    w.u64(h as u64);
    done(call(op::CLOSE, &w))
}

/// `DeleteService`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteService(h: ScHandle) -> Bool {
    let mut w = Writer::new();
    w.u64(h as u64);
    done(call(op::DELETE, &w))
}

/// `ChangeServiceConfigW`: `SERVICE_NO_CHANGE` and NULL leave a field as it is.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn ChangeServiceConfigW(
    h: ScHandle,
    ty: u32,
    start: u32,
    error: u32,
    path: *const u16,
    group: *const u16,
    tag: *mut u32,
    deps: *const u16,
    account: *const u16,
    password: *const u16,
    display: *const u16,
) -> Bool {
    let mut w = Writer::new();
    w.u64(h as u64).u32(ty).u32(start).u32(error);
    w.opt(wstr(path).as_deref()).opt(wstr(group).as_deref()).opt_list(multi(deps).as_deref());
    w.opt(wstr(account).as_deref()).opt(wstr(password).as_deref()).opt(wstr(display).as_deref());
    let r = done(call(op::CHANGE, &w));
    if r != 0 && !tag.is_null() {
        // SAFETY: the caller's tag.
        unsafe { tag.write(0) };
    }
    r
}

/// `SERVICE_FAILURE_ACTIONSW`.
#[repr(C)]
struct FailureActionsW {
    reset: u32,
    reboot_msg: *const u16,
    command: *const u16,
    count: u32,
    actions: *const [u32; 2],
}

/// `ChangeServiceConfig2W(hService, dwInfoLevel, lpInfo)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ChangeServiceConfig2W(h: ScHandle, level: u32, info: *const c_void) -> Bool {
    let data = if info.is_null() {
        Config2::Nothing
    } else {
        // SAFETY: the structure of the level, as winsvc.h lays it out.
        unsafe {
            match level {
                SERVICE_CONFIG_DESCRIPTION => Config2::Description(wstr(*(info as *const *const u16))),
                SERVICE_CONFIG_FAILURE_ACTIONS => {
                    let f = &*(info as *const FailureActionsW);
                    let actions = (!f.actions.is_null()).then(|| {
                        (0..f.count.min(1024) as usize)
                            .map(|i| {
                                let a = *f.actions.add(i);
                                (a[0], a[1])
                            })
                            .collect()
                    });
                    Config2::Failure {
                        reset: f.reset,
                        reboot_msg: wstr(f.reboot_msg),
                        command: wstr(f.command),
                        actions,
                    }
                }
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO
                | SERVICE_CONFIG_FAILURE_ACTIONS_FLAG
                | SERVICE_CONFIG_SERVICE_SID_INFO
                | SERVICE_CONFIG_PRESHUTDOWN_INFO => Config2::Value(level, *(info as *const u32)),
                _ => Config2::Nothing,
            }
        }
    };
    let mut w = Writer::new();
    w.u64(h as u64).u32(level);
    data.write(&mut w);
    done(call(op::CHANGE2, &w))
}

/// Lays out strings after a structure in the caller's buffer.
struct Layout {
    base: *mut u8,
    at: usize,
}

impl Layout {
    /// Copy `s` and its NUL; returns its address.
    unsafe fn put(&mut self, s: &[u16]) -> usize {
        let p = self.base as usize + self.at;
        for (i, &c) in s.iter().chain(core::iter::once(&0)).enumerate() {
            // SAFETY: the caller checked the buffer holds the size computed for these strings.
            unsafe { (self.base.add(self.at + 2 * i) as *mut u16).write_unaligned(c) };
        }
        self.at += 2 * (s.len() + 1);
        p
    }

    /// Copy units as they are (a list with its NULs).
    unsafe fn put_raw(&mut self, s: &[u16]) -> usize {
        let p = self.base as usize + self.at;
        for (i, &c) in s.iter().enumerate() {
            // SAFETY: as in `put`.
            unsafe { (self.base.add(self.at + 2 * i) as *mut u16).write_unaligned(c) };
        }
        self.at += 2 * s.len();
        p
    }
}

/// Write a pointer-sized value at `at`.
unsafe fn put_ptr(base: *mut u8, at: usize, v: usize) {
    // SAFETY: inside the caller's buffer (checked by the caller).
    unsafe { (base.add(at) as *mut usize).write_unaligned(v) };
}

unsafe fn put_u32(base: *mut u8, at: usize, v: u32) {
    // SAFETY: inside the caller's buffer (checked by the caller).
    unsafe { (base.add(at) as *mut u32).write_unaligned(v) };
}

/// `QueryServiceConfigW`: a `QUERY_SERVICE_CONFIGW` (64 bytes on x64, 36 on x86) and its strings.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryServiceConfigW(h: ScHandle, buf: *mut u8, size: u32, needed: *mut u32) -> Bool {
    if needed.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    let mut w = Writer::new();
    w.u64(h as u64);
    let cfg = match call(op::QUERY_CONFIG, &w)
        .and_then(|b| ConfigReply::read(&mut Reader::new(&b)).ok_or(ERROR_INVALID_PARAMETER))
    {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    let head = if PTR == 8 { 64 } else { 36 };
    let deps = join_multi(&cfg.deps);
    let total = head
        + 2 * (cfg.path.len() + 1)
        + 2 * (cfg.group.len() + 1)
        + 2 * deps.len()
        + 2 * (cfg.account.len() + 1)
        + 2 * (cfg.display.len() + 1);
    if buf.is_null() || (size as usize) < total {
        // SAFETY: checked above.
        unsafe { needed.write(total as u32) };
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: the buffer holds `total` bytes; the offsets are QUERY_SERVICE_CONFIGW's for the width.
    unsafe {
        let mut l = Layout { base: buf, at: head };
        let path = l.put(&cfg.path);
        let group = l.put(&cfg.group);
        let deps_at = l.put_raw(&deps);
        let account = l.put(&cfg.account);
        let display = l.put(&cfg.display);
        put_u32(buf, 0, cfg.ty);
        put_u32(buf, 4, cfg.start);
        put_u32(buf, 8, cfg.error);
        if PTR == 8 {
            put_u32(buf, 12, 0);
            put_ptr(buf, 16, path);
            put_ptr(buf, 24, group);
            put_u32(buf, 32, cfg.tag);
            put_u32(buf, 36, 0);
            put_ptr(buf, 40, deps_at);
            put_ptr(buf, 48, account);
            put_ptr(buf, 56, display);
        } else {
            put_ptr(buf, 12, path);
            put_ptr(buf, 16, group);
            put_u32(buf, 20, cfg.tag);
            put_ptr(buf, 24, deps_at);
            put_ptr(buf, 28, account);
            put_ptr(buf, 32, display);
        }
        needed.write(total as u32);
    }
    succeed()
}

/// `QueryServiceConfig2W`: the structure of the level and what it points to.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryServiceConfig2W(
    h: ScHandle,
    level: u32,
    buf: *mut u8,
    size: u32,
    needed: *mut u32,
) -> Bool {
    if needed.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    let mut w = Writer::new();
    w.u64(h as u64).u32(level);
    let reply = match call(op::QUERY_CONFIG2, &w) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let mut r = Reader::new(&reply);
    enum Shape {
        Description(Option<Vec<u16>>),
        Failure(FailureActions),
        Value(u32),
        Pointer,
        Triggers,
    }
    let shape = match level {
        SERVICE_CONFIG_DESCRIPTION => Shape::Description(r.opt().flatten()),
        SERVICE_CONFIG_FAILURE_ACTIONS => Shape::Failure(FailureActions::read(&mut r).unwrap_or_default()),
        SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO => Shape::Pointer,
        SERVICE_CONFIG_TRIGGER_INFO => Shape::Triggers,
        _ => Shape::Value(r.u32().unwrap_or(0)),
    };
    let s = |x: &Option<Vec<u16>>| x.as_ref().map_or(0, |v| 2 * (v.len() + 1));
    let total = match &shape {
        Shape::Description(d) => PTR + s(d),
        Shape::Failure(f) => 5 * PTR + 8 * f.actions.len() + s(&f.reboot_msg) + s(&f.command),
        Shape::Value(_) => 4,
        Shape::Pointer => PTR,
        Shape::Triggers => 3 * PTR,
    };
    if buf.is_null() || (size as usize) < total {
        // SAFETY: checked above.
        unsafe { needed.write(total as u32) };
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: the buffer holds `total` bytes, laid out as the level's structure.
    unsafe {
        match &shape {
            Shape::Description(d) => {
                let mut l = Layout { base: buf, at: PTR };
                let p = d.as_ref().map_or(0, |d| l.put(d));
                put_ptr(buf, 0, p);
            }
            Shape::Failure(f) => {
                // reset, reboot message, command, count, actions: 0/8/16/24/32 on x64, 0/4/8/12/16 on x86.
                let head = 5 * PTR;
                let acts = buf as usize + head;
                for (i, &(t, d)) in f.actions.iter().enumerate() {
                    put_u32(buf, head + 8 * i, t);
                    put_u32(buf, head + 8 * i + 4, d);
                }
                let mut l = Layout { base: buf, at: head + 8 * f.actions.len() };
                let reboot = f.reboot_msg.as_ref().map_or(0, |m| l.put(m));
                let command = f.command.as_ref().map_or(0, |m| l.put(m));
                put_u32(buf, 0, f.reset);
                if PTR == 8 {
                    put_u32(buf, 4, 0);
                }
                put_ptr(buf, PTR, reboot);
                put_ptr(buf, 2 * PTR, command);
                put_u32(buf, 3 * PTR, f.actions.len() as u32);
                if PTR == 8 {
                    put_u32(buf, 3 * PTR + 4, 0);
                }
                put_ptr(buf, 4 * PTR, if f.actions.is_empty() { 0 } else { acts });
            }
            Shape::Value(v) => put_u32(buf, 0, *v),
            Shape::Pointer => put_ptr(buf, 0, 0),
            Shape::Triggers => {
                for i in 0..3 {
                    put_ptr(buf, i * PTR, 0);
                }
            }
        }
        needed.write(total as u32);
    }
    succeed()
}

fn status_of(h: ScHandle) -> Result<Status, u32> {
    let mut w = Writer::new();
    w.u64(h as u64);
    call(op::QUERY_STATUS, &w).and_then(|b| Reader::new(&b).status().ok_or(ERROR_INVALID_PARAMETER))
}

/// `QueryServiceStatusEx(hService, InfoLevel, lpBuffer, cbBufSize, pcbBytesNeeded)`: level
/// `SC_STATUS_PROCESS_INFO` (0) only, 36 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryServiceStatusEx(
    h: ScHandle,
    level: u32,
    buf: *mut u8,
    size: u32,
    needed: *mut u32,
) -> Bool {
    if needed.is_null() || buf.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    let s = match status_of(h) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    if level != 0 {
        return fail(ERROR_INVALID_LEVEL);
    }
    // SAFETY: checked above.
    unsafe { needed.write(36) };
    if size < 36 {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    let bytes = s.process_bytes();
    // SAFETY: the caller's buffer holds at least 36 bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, 36) };
    succeed()
}

/// Copy the 28 bytes of `SERVICE_STATUS`.
unsafe fn put_status(out: *mut u8, s: &Status) {
    let bytes = s.process_bytes();
    // SAFETY: the caller's SERVICE_STATUS.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, 28) };
}

/// `QueryServiceStatus(hService, lpServiceStatus)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryServiceStatus(h: ScHandle, out: *mut u8) -> Bool {
    if out.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    match status_of(h) {
        // SAFETY: the caller's structure.
        Ok(s) => unsafe {
            put_status(out, &s);
            succeed()
        },
        Err(e) => fail(e),
    }
}

/// Wait on `event` for at most `ms` milliseconds and close it: false when the time ran out.
fn wait_and_close(event: u64, ms: u64) -> bool {
    let timeout: i64 = -((ms as i64) * 10_000);
    // SAFETY: an event handle the manager made in this process; the timeout lives for the call.
    let st = unsafe { NtWaitForSingleObject(event as isize, 0, &timeout) };
    // SAFETY: the same handle, closed once.
    unsafe { NtClose(event as isize) };
    st != STATUS_TIMEOUT
}

/// `StartServiceW(hService, dwNumServiceArgs, lpServiceArgVectors)`: returns when the service's
/// process has connected (or the start failed).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn StartServiceW(h: ScHandle, argc: u32, argv: *const *const u16) -> Bool {
    let args: Vec<Vec<u16>> = if argv.is_null() {
        Vec::new()
    } else {
        // SAFETY: `argc` string pointers.
        (0..argc.min(1024) as usize).map(|i| wstr(unsafe { *argv.add(i) }).unwrap_or_default()).collect()
    };
    let mut w = Writer::new();
    w.u64(h as u64).list(&args);
    let event = match call(op::START, &w).map(|b| Reader::new(&b).u64().unwrap_or(0)) {
        Ok(e) => e,
        Err(e) => return fail(e),
    };
    // The manager decides within its 30 seconds; a little more for the host's turn.
    if !wait_and_close(event, START_TIMEOUT_MS + 5000) {
        return fail(ERROR_SERVICE_REQUEST_TIMEOUT);
    }
    let mut w = Writer::new();
    w.u64(h as u64);
    done(call(op::START_RESULT, &w))
}

/// `ControlService(hService, dwControl, lpServiceStatus)`: the status after the handler returned.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ControlService(h: ScHandle, control: u32, out: *mut u8) -> Bool {
    if out.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    let mut w = Writer::new();
    w.u64(h as u64).u32(control);
    let reply = match call(op::CONTROL, &w) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let mut r = Reader::new(&reply);
    let status = match r.u32() {
        Some(0) => r.status(),
        Some(1) => {
            let event = r.u64().unwrap_or(0);
            if !wait_and_close(event, CONTROL_TIMEOUT_MS) {
                return fail(ERROR_SERVICE_REQUEST_TIMEOUT);
            }
            let mut w = Writer::new();
            w.u64(h as u64);
            match call(op::CONTROL_RESULT, &w) {
                Ok(b) => Reader::new(&b).status(),
                Err(e) => return fail(e),
            }
        }
        _ => None,
    };
    match status {
        // SAFETY: the caller's structure.
        Some(s) => unsafe {
            put_status(out, &s);
            succeed()
        },
        None => fail(ERROR_INVALID_PARAMETER),
    }
}

/// `EnumServicesStatusExW(hSCManager, InfoLevel, dwServiceType, dwServiceState, lpServices,
/// cbBufSize, pcbBytesNeeded, lpServicesReturned, lpResumeHandle, pszGroupName)`:
/// `ENUM_SERVICE_STATUS_PROCESSW` entries from the start of the buffer, their strings from its end.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn EnumServicesStatusExW(
    scm: ScHandle,
    level: u32,
    ty: u32,
    state: u32,
    buf: *mut u8,
    size: u32,
    needed: *mut u32,
    returned: *mut u32,
    resume: *mut u32,
    group: *const u16,
) -> Bool {
    if level != 0 {
        return fail(ERROR_INVALID_LEVEL);
    }
    if needed.is_null() || returned.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    // SAFETY: checked above.
    unsafe {
        needed.write(0);
        returned.write(0);
    }
    let mut w = Writer::new();
    w.u64(scm as u64).u32(ty).u32(state).opt(wstr(group).as_deref());
    let reply = match call(op::ENUM, &w) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let mut r = Reader::new(&reply);
    let n = r.u32().unwrap_or(0);
    let mut list = Vec::new();
    for _ in 0..n {
        let (Some(name), Some(display), Some(s)) = (r.units(), r.units(), r.status()) else { break };
        list.push((name, display, s));
    }
    // The entry: two pointers and SERVICE_STATUS_PROCESS, aligned to a pointer.
    let entry = (2 * PTR + 36).div_ceil(PTR) * PTR;
    // SAFETY: the caller's resume handle, if any.
    let first = if resume.is_null() { 0 } else { unsafe { *resume as usize } };
    let rest = list.get(first..).unwrap_or(&[]);
    let cost = |e: &(Vec<u16>, Vec<u16>, Status)| entry + 2 * (e.0.len() + 1) + 2 * (e.1.len() + 1);
    let mut used = 0usize;
    let mut fit = 0usize;
    for e in rest {
        if buf.is_null() || used + cost(e) > size as usize {
            break;
        }
        used += cost(e);
        fit += 1;
    }
    let mut strings_end = size as usize;
    for (i, e) in rest[..fit].iter().enumerate() {
        // SAFETY: the entries and strings counted above fit in the caller's buffer.
        unsafe {
            let at = i * entry;
            strings_end -= 2 * (e.0.len() + 1);
            let name = Layout { base: buf, at: strings_end }.put(&e.0);
            strings_end -= 2 * (e.1.len() + 1);
            let display = Layout { base: buf, at: strings_end }.put(&e.1);
            put_ptr(buf, at, name);
            put_ptr(buf, at + PTR, display);
            let b = e.2.process_bytes();
            core::ptr::copy_nonoverlapping(b.as_ptr(), buf.add(at + 2 * PTR), 36);
        }
    }
    let remaining: usize = rest[fit..].iter().map(cost).sum();
    // SAFETY: checked above.
    unsafe {
        returned.write(fit as u32);
        needed.write(remaining as u32);
        if !resume.is_null() {
            resume.write(if remaining == 0 { 0 } else { (first + fit) as u32 });
        }
    }
    if remaining == 0 { succeed() } else { fail(ERROR_MORE_DATA) }
}

/// `QueryServiceObjectSecurity(hService, dwSecurityInformation, lpSecurityDescriptor, cbBufSize,
/// pcbBytesNeeded)`: a self-relative descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryServiceObjectSecurity(
    h: ScHandle,
    info: u32,
    sd: *mut u8,
    size: u32,
    needed: *mut u32,
) -> Bool {
    if needed.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    let mut w = Writer::new();
    w.u64(h as u64).u32(info);
    let bytes = match call(op::QUERY_SECURITY, &w).map(|b| Reader::new(&b).bytes().unwrap_or_default()) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    // SAFETY: checked above.
    unsafe { needed.write(bytes.len() as u32) };
    if sd.is_null() || (size as usize) < bytes.len() {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: the caller's buffer holds the descriptor.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), sd, bytes.len()) };
    succeed()
}

/// `SetServiceObjectSecurity(hService, dwSecurityInformation, lpSecurityDescriptor)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetServiceObjectSecurity(h: ScHandle, info: u32, sd: *const u8) -> Bool {
    if sd.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    // SAFETY: the caller's self-relative descriptor; its length from its own offsets.
    let bytes = unsafe {
        let n = poly_security::sd::length(poly_security::sd::SdPtr(sd as *mut u8));
        core::slice::from_raw_parts(sd, n).to_vec()
    };
    let mut w = Writer::new();
    w.u64(h as u64).u32(info).bytes(&bytes);
    done(call(op::SET_SECURITY, &w))
}

// ----- the service process

/// Whether this process connected to the manager as a service.
static CONNECTED: AtomicBool = AtomicBool::new(false);
static HANDLER: AtomicUsize = AtomicUsize::new(0);
static HANDLER_EX: AtomicUsize = AtomicUsize::new(0);
static CONTEXT: AtomicUsize = AtomicUsize::new(0);
/// What `RegisterServiceCtrlHandler(Ex)W` hands out: the status handle `SetServiceStatus` takes.
static STATUS_HANDLE: u8 = 0;

type ServiceMain = unsafe extern "system" fn(u32, *mut *mut u16);
type Handler = unsafe extern "system" fn(u32);
type HandlerEx = unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32;

/// `SERVICE_TABLE_ENTRYW`.
#[repr(C)]
pub struct TableEntry {
    name: *const u16,
    proc_: Option<ServiceMain>,
}

/// What the service main's thread gets.
struct MainStart {
    main: ServiceMain,
    argc: u32,
    argv: *mut *mut u16,
}

unsafe extern "system" fn main_thread(p: *mut c_void) -> u32 {
    // SAFETY: the box `StartServiceCtrlDispatcherW` made for this thread.
    let start = unsafe { alloc::boxed::Box::from_raw(p as *mut MainStart) };
    // SAFETY: the service's own main function with the arguments the manager gave.
    unsafe { (start.main)(start.argc, start.argv) };
    0
}

/// `StartServiceCtrlDispatcherW(lpServiceStartTable)`: connects the process to the manager (1063
/// when the manager did not start it, 1056 after a first attempt), starts the service main function
/// on a thread of its own, then calls the handler with each control until the service reports
/// STOPPED.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn StartServiceCtrlDispatcherW(table: *const TableEntry) -> Bool {
    let reply = match call(op::CONNECT, &Writer::new()) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let mut r = Reader::new(&reply);
    let (Some(_name), Some(argv), Some(event)) = (r.units(), r.list(), r.u64()) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    // SAFETY: the caller's table, ended by a NULL entry (an own-process service uses its first).
    let main = if table.is_null() { None } else { unsafe { (*table).proc_ } };
    let Some(main) = main else {
        // SAFETY: the event the manager made for this process.
        unsafe { NtClose(event as isize) };
        return fail(ERROR_INVALID_PARAMETER);
    };
    CONNECTED.store(true, Ordering::SeqCst);
    // The arguments live as long as the process (the service may keep them).
    let mut ptrs: Vec<*mut u16> = argv
        .into_iter()
        .map(|mut a| {
            a.push(0);
            alloc::boxed::Box::leak(a.into_boxed_slice()).as_mut_ptr()
        })
        .collect();
    let argc = ptrs.len() as u32;
    ptrs.push(core::ptr::null_mut());
    let argv = alloc::boxed::Box::leak(ptrs.into_boxed_slice()).as_mut_ptr();
    let start = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(MainStart { main, argc, argv }));
    // SAFETY: a new thread running the service main with its box.
    let th = unsafe { CreateThread(core::ptr::null(), 0, main_thread, start as *mut c_void, 0, core::ptr::null_mut()) };
    if th == 0 {
        // SAFETY: the box was not handed to a thread.
        drop(unsafe { alloc::boxed::Box::from_raw(start) });
        return 0;
    }
    // SAFETY: the thread's handle; the thread runs on.
    unsafe { NtClose(th) };
    loop {
        let reply = call(op::NEXT_CONTROL, &Writer::new()).unwrap_or_else(|_| 2u32.to_le_bytes().to_vec());
        let mut r = Reader::new(&reply);
        match r.u32() {
            Some(0) => {
                // SAFETY: the dispatcher's event, waited on without a time limit.
                unsafe { NtWaitForSingleObject(event as isize, 0, core::ptr::null()) };
            }
            Some(1) => {
                let code = r.u32().unwrap_or(0);
                let ev = r.u32().unwrap_or(0);
                let ex = HANDLER_EX.load(Ordering::SeqCst);
                let plain = HANDLER.load(Ordering::SeqCst);
                if ex != 0 {
                    // SAFETY: the extended handler the service registered, with its context.
                    let f: HandlerEx = unsafe { core::mem::transmute(ex) };
                    unsafe { f(code, ev, core::ptr::null_mut(), CONTEXT.load(Ordering::SeqCst) as *mut c_void) };
                } else if plain != 0 {
                    // SAFETY: the handler the service registered.
                    let f: Handler = unsafe { core::mem::transmute(plain) };
                    unsafe { f(code) };
                }
                let _ = call(op::CONTROL_DONE, &Writer::new());
            }
            _ => break,
        }
    }
    // SAFETY: the dispatcher's event, closed once.
    unsafe { NtClose(event as isize) };
    succeed()
}

fn register(handler: usize, ex: bool, context: usize) -> ScHandle {
    if !CONNECTED.load(Ordering::SeqCst) {
        // Not a service process: NULL (the sheet: the last error is not meaningful).
        return fail_h(ERROR_INVALID_HANDLE);
    }
    if ex {
        CONTEXT.store(context, Ordering::SeqCst);
        HANDLER_EX.store(handler, Ordering::SeqCst);
    } else {
        HANDLER.store(handler, Ordering::SeqCst);
    }
    succeed();
    &STATUS_HANDLE as *const u8 as usize
}

/// `RegisterServiceCtrlHandlerW(lpServiceName, lpHandlerProc)`: the name does not matter for an
/// own-process service.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterServiceCtrlHandlerW(_name: *const u16, handler: Option<Handler>) -> ScHandle {
    register(handler.map_or(0, |f| f as usize), false, 0)
}

/// `RegisterServiceCtrlHandlerExW(lpServiceName, lpHandlerProc, lpContext)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterServiceCtrlHandlerExW(
    _name: *const u16,
    handler: Option<HandlerEx>,
    context: *mut c_void,
) -> ScHandle {
    register(handler.map_or(0, |f| f as usize), true, context as usize)
}

/// `SetServiceStatus(hServiceStatus, lpServiceStatus)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetServiceStatus(h: ScHandle, status: *const u32) -> Bool {
    if h == 0 || h != &STATUS_HANDLE as *const u8 as usize {
        return fail(ERROR_INVALID_HANDLE);
    }
    if status.is_null() {
        return fail(ERROR_INVALID_ADDRESS);
    }
    let mut w = Writer::new();
    for i in 0..7 {
        // SAFETY: the caller's SERVICE_STATUS (seven DWORDs).
        w.u32(unsafe { *status.add(i) });
    }
    done(call(op::SET_STATUS, &w))
}
