//! Window stations and desktops (ADR 0192): `CreateWindowStationW`, `OpenWindowStationW`,
//! `CloseWindowStation`, `GetProcessWindowStation`, `SetProcessWindowStation`,
//! `CreateDesktopW`, `OpenDesktopW`, `CloseDesktop`, `GetThreadDesktop`, `SetThreadDesktop`,
//! `GetUserObjectInformationW`, `SetUserObjectInformationW`. The objects and the rules are the NT
//! personality's (`poly_win32k::winsta`, its `winsta` module); each export sends one
//! [`WinstaOp`] and turns the answer into the return value and last error the sheets record.

use super::{call, set_last_error};
use alloc::vec::Vec;
use poly_win32k::proto::{Reply, Request};
use poly_win32k::winsta::{ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, WinstaOp, WinstaRes};

/// The text at `p` up to its NUL (nothing for a NULL pointer).
///
/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn text(p: *const u16) -> Vec<u16> {
    let mut v = Vec::new();
    if p.is_null() {
        return v;
    }
    let mut i = 0;
    // SAFETY: the caller's string.
    unsafe {
        while *p.add(i) != 0 {
            v.push(*p.add(i));
            i += 1;
        }
    }
    v
}

fn ask(op: WinstaOp) -> WinstaRes {
    match call(Request::Winsta { op }) {
        Reply::Winsta { res } => res,
        // The call itself was refused (win32k lockdown) or not understood.
        _ => WinstaRes::Err { code: ERROR_INVALID_PARAMETER },
    }
}

/// A call that gives a handle: NULL with the last error on failure.
fn handle_result(res: WinstaRes) -> usize {
    match res {
        WinstaRes::Handle { handle } => handle as usize,
        WinstaRes::HandleClear { handle } => {
            set_last_error(0);
            handle as usize
        }
        WinstaRes::Err { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

/// A call that gives `TRUE` or `FALSE` with the last error.
fn bool_result(res: WinstaRes) -> i32 {
    match res {
        WinstaRes::Done {} => 1,
        WinstaRes::Err { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateWindowStationW(name: *const u16, flags: u32, access: u32, sa: *const u8) -> usize {
    // `SECURITY_ATTRIBUTES.lpSecurityDescriptor` (offset 8 on x64).
    // SAFETY: the caller's attributes or NULL; the name is its string.
    let (sd, name) = unsafe { (sd_of(sa), text(name)) };
    handle_result(ask(WinstaOp::CreateStation { name, flags, access, sd }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenWindowStationW(name: *const u16, inherit: i32, access: u32) -> usize {
    // SAFETY: the caller's string.
    let name = unsafe { text(name) };
    handle_result(ask(WinstaOp::OpenStation { name, inherit: inherit != 0, access }))
}

#[unsafe(no_mangle)]
pub extern "system" fn CloseWindowStation(h: usize) -> i32 {
    bool_result(ask(WinstaOp::CloseStation { handle: h as u64 }))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetProcessWindowStation() -> usize {
    handle_result(ask(WinstaOp::ProcessStation {}))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetProcessWindowStation(h: usize) -> i32 {
    bool_result(ask(WinstaOp::SetProcessStation { handle: h as u64 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateDesktopW(
    name: *const u16,
    device: *const u16,
    _mode: *const u8,
    flags: u32,
    access: u32,
    sa: *const u8,
) -> usize {
    // SAFETY: the caller's attributes and string.
    let (sd, name) = unsafe { (sd_of(sa), text(name)) };
    handle_result(ask(WinstaOp::CreateDesktop { name, has_device: !device.is_null(), flags, access, sd }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenDesktopW(name: *const u16, flags: u32, inherit: i32, access: u32) -> usize {
    // SAFETY: the caller's string.
    let name = unsafe { text(name) };
    handle_result(ask(WinstaOp::OpenDesktop { name, flags, inherit: inherit != 0, access }))
}

#[unsafe(no_mangle)]
pub extern "system" fn CloseDesktop(h: usize) -> i32 {
    bool_result(ask(WinstaOp::CloseDesktop { handle: h as u64 }))
}

#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDesktop(tid: u32) -> usize {
    handle_result(ask(WinstaOp::ThreadDesktop { tid }))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetThreadDesktop(h: usize) -> i32 {
    bool_result(ask(WinstaOp::SetThreadDesktop { handle: h as u64 }))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUserObjectInformationW(
    h: usize,
    index: i32,
    info: *mut u8,
    length: u32,
    needed: *mut u32,
) -> i32 {
    // A NULL buffer has no room whatever `length` says.
    let capacity = if info.is_null() { 0 } else { length };
    match ask(WinstaOp::GetInfo { handle: h as u64, index, capacity }) {
        WinstaRes::Info { data } => {
            // SAFETY: the reply fits `length` bytes at `info`; `needed` is the caller's or NULL.
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr(), info, data.len());
                if !needed.is_null() {
                    needed.write(data.len() as u32);
                }
            }
            1
        }
        WinstaRes::Short { needed: n } => {
            if !needed.is_null() {
                // SAFETY: the caller's variable.
                unsafe { needed.write(n) };
            }
            set_last_error(ERROR_INSUFFICIENT_BUFFER);
            0
        }
        WinstaRes::Err { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetUserObjectInformationW(h: usize, index: i32, info: *const u8, length: u32) -> i32 {
    let data = if info.is_null() {
        Vec::new()
    } else {
        // SAFETY: `length` bytes at `info`, the caller's.
        unsafe { core::slice::from_raw_parts(info, length as usize) }.to_vec()
    };
    bool_result(ask(WinstaOp::SetInfo { handle: h as u64, index, data }))
}

/// The address of the descriptor in `SECURITY_ATTRIBUTES` (0 for none).
///
/// # Safety
/// `sa` is NULL or a `SECURITY_ATTRIBUTES`.
unsafe fn sd_of(sa: *const u8) -> u64 {
    if sa.is_null() {
        return 0;
    }
    // SAFETY: the caller's structure.
    unsafe { (sa.add(8) as *const u64).read_unaligned() }
}
