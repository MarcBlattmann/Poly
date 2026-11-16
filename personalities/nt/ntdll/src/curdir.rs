//! The current directory (Learn: `GetCurrentDirectory`, `SetCurrentDirectory`; `ntdll`'s
//! `RtlGetCurrentDirectory_U` and `RtlSetCurrentDirectory_U`, which the MSYS2 runtime calls by
//! name at start-up). The directory lives in the process parameters (`RTL_USER_PROCESS_PARAMETERS`,
//! Poly's own `CurrentDirectory` string, ADR 0079), where `kernelbase` keeps it up to date; these
//! two read and write it, so both DLLs always see the same one.

use crate::NtStatus;
use crate::syscalls::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use poly_rtl::layout::NATIVE;

const PEB_PROCESS_PARAMETERS: u64 = NATIVE.peb_process_parameters;
const PP_CURRENT_DIRECTORY: u64 = NATIVE.pp_current_directory;
const BACKSLASH: u16 = b'\\' as u16;

/// The process parameters' address (0: none).
fn parameters() -> u64 {
    // SAFETY: the PEB is mapped and holds the parameters' pointer.
    unsafe { ((crate::lock::peb() + PEB_PROCESS_PARAMETERS) as *const usize).read() as u64 }
}

/// A directory as a program sees it: no trailing backslash unless it is a root (`C:\`).
fn normalize(mut dir: Vec<u16>) -> Vec<u16> {
    while dir.len() > 3 && dir.last() == Some(&BACKSLASH) {
        dir.pop();
    }
    dir
}

/// The directory now.
fn current() -> Vec<u16> {
    let pp = parameters();
    if pp == 0 {
        return Vec::new();
    }
    // SAFETY: a UNICODE_STRING of the parameters: `Length` in bytes at 0, the buffer pointer one pointer in.
    let text = unsafe {
        let us = pp + PP_CURRENT_DIRECTORY;
        let len = usize::from((us as *const u16).read()) / 2;
        let buf = ((us + NATIVE.ptr) as *const *const u16).read();
        if buf.is_null() || len == 0 { Vec::new() } else { core::slice::from_raw_parts(buf, len).to_vec() }
    };
    normalize(text)
}

/// Put `dir` into the parameters (in the string's own buffer when it fits, else in a new one).
fn publish(dir: &[u16]) {
    let pp = parameters();
    if pp == 0 {
        return;
    }
    // SAFETY: the parameters' UNICODE_STRING, writable process memory.
    unsafe {
        let us = pp + PP_CURRENT_DIRECTORY;
        let max = usize::from((us as *const u16).add(1).read()) / 2;
        let slot = (us + NATIVE.ptr) as *mut *mut u16;
        let mut buf = slot.read();
        if buf.is_null() || max < dir.len() + 1 {
            let mut v = alloc::vec![0u16; dir.len() + 1];
            v[..dir.len()].copy_from_slice(dir);
            buf = Box::leak(v.into_boxed_slice()).as_mut_ptr();
            slot.write(buf);
            (us as *mut u16).add(1).write(((dir.len() + 1) * 2) as u16);
        } else {
            core::ptr::copy_nonoverlapping(dir.as_ptr(), buf, dir.len());
            buf.add(dir.len()).write(0);
        }
        (us as *mut u16).write((dir.len() * 2) as u16);
    }
}

/// `RtlGetCurrentDirectory_U(bufferBytes, buffer)`: the directory's length in bytes without the NUL
/// when it and the NUL fit in `bufferBytes` (they are written); else the size it needs with the NUL,
/// and nothing is written.
///
/// # Safety
/// `buffer` holds `bufferBytes` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlGetCurrentDirectory_U(buffer_bytes: u32, buffer: *mut u16) -> u32 {
    let dir = current();
    let bytes = (dir.len() * 2) as u32;
    if buffer.is_null() || buffer_bytes < bytes + 2 {
        return bytes + 2;
    }
    // SAFETY: the caller's buffer holds the text and a NUL.
    unsafe {
        core::ptr::copy_nonoverlapping(dir.as_ptr(), buffer, dir.len());
        buffer.add(dir.len()).write(0);
    }
    bytes
}

/// `RtlSetCurrentDirectory_U(UNICODE_STRING*)`: the directory must exist (`STATUS_OBJECT_NAME_NOT_FOUND`
/// otherwise, and the current one stays).
///
/// # Safety
/// `path` is a valid `UNICODE_STRING`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlSetCurrentDirectory_U(path: *const u8) -> NtStatus {
    if path.is_null() {
        return NtStatus(0xC000_000D);
    }
    // SAFETY: the caller's UNICODE_STRING.
    let given = unsafe {
        let len = usize::from((path as *const u16).read()) / 2;
        let buf = path.add(NATIVE.ptr as usize).cast::<*const u16>().read();
        if buf.is_null() { Vec::new() } else { core::slice::from_raw_parts(buf, len).to_vec() }
    };
    // A full drive path; anything else is taken relative to the directory now.
    let drive =
        given.len() >= 2 && given[1] == u16::from(b':') && (given[0] < 0x80 && (given[0] as u8).is_ascii_alphabetic());
    let full = if drive && given.get(2) == Some(&BACKSLASH) {
        given
    } else if drive {
        // `C:dir`: relative to the drive's directory, here the current one if it is on that drive.
        let mut cur = current();
        if cur.first().map(|c| c | 0x20) != given.first().map(|c| c | 0x20) {
            cur = alloc::vec![given[0], u16::from(b':'), BACKSLASH];
        }
        if cur.last() != Some(&BACKSLASH) {
            cur.push(BACKSLASH);
        }
        cur.extend_from_slice(&given[2..]);
        cur
    } else {
        let mut cur = current();
        if cur.last() != Some(&BACKSLASH) {
            cur.push(BACKSLASH);
        }
        cur.extend_from_slice(&given);
        cur
    };
    let dir = normalize(full);
    let mut nt_path: Vec<u16> = "\\??\\".encode_utf16().collect();
    nt_path.extend_from_slice(&dir);
    match crate::sys::open_directory(&nt_path) {
        Ok(h) => {
            // SAFETY: a handle just opened.
            unsafe { NtClose(h) };
            publish(&dir);
            NtStatus(0)
        }
        Err(st) => st,
    }
}
