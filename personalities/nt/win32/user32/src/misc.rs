//! The S3 additions of ADR 0172 that need no module of their own: window
//! properties.

use super::{H, call, class_ref, set_last_error};
use poly_win32k::proto::{Reply, Request};

fn prop(hwnd: H, key: *const u16, op: u8, value: usize) -> Option<u64> {
    // SAFETY: an atom or a NUL-terminated name.
    let key = unsafe { class_ref(key) };
    match call(Request::Prop { hwnd: hwnd as u64, op, key, value: value as u64 }) {
        Reply::Value { value } => Some(value),
        Reply::Ok {} => Some(1),
        Reply::Error { code } => {
            set_last_error(code);
            None
        }
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetPropW(hwnd: H, key: *const u16, data: usize) -> i32 {
    i32::from(prop(hwnd, key, 1, data).is_some())
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetPropW(hwnd: H, key: *const u16) -> usize {
    prop(hwnd, key, 0, 0).unwrap_or(0) as usize
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RemovePropW(hwnd: H, key: *const u16) -> usize {
    prop(hwnd, key, 2, 0).unwrap_or(0) as usize
}
