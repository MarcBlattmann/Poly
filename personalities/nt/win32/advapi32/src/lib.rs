//! Our `advapi32.dll` (ADR 0083). The registry, token, SID, ACL and
//! security-descriptor functions are forwarders to `kernelbase` through
//! API-set contracts (written by `build.rs` from
//! `poly_kernelbase::advapi::ADVAPI32_FORWARDS`), as Learn's API locations
//! describe. What it implements itself are the functions whose locations
//! name no `kernelbase` contract: `RegDeleteKeyW/A`, the SID string
//! conversions (`sddl.h`), `GetUserNameW/A` and the privilege-name lookups.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::vec::Vec;
use poly_security::{sddl, sid, token};
use poly_win32_rt as _;

type Handle = isize;

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn LocalFree(mem: *mut u8) -> *mut u8;
    fn SetLastError(error: u32);
    fn RegDeleteKeyExW(hkey: Handle, sub: *const u16, sam: u32, reserved: u32) -> u32;
    fn RegDeleteKeyExA(hkey: Handle, sub: *const u8, sam: u32, reserved: u32) -> u32;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> i32;
}

/// `poly-security` uses `alloc`, so the DLL needs an allocator: the
/// process heap through `kernelbase`'s local memory (16-byte aligned).
struct LocalHeap;

// SAFETY: LocalAlloc returns 16-byte aligned blocks of the process heap;
// stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for LocalHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: no preconditions.
        unsafe { LocalAlloc(0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { LocalFree(ptr) };
    }
}

#[global_allocator]
static ALLOC: LocalHeap = LocalHeap;

const TRUE: i32 = 1;
const FALSE: i32 = 0;
/// `winerror.h`.
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_NO_SUCH_PRIVILEGE: u32 = 1313;
/// `LMEM_FIXED` (`minwinbase.h`).
const LMEM_FIXED: u32 = 0;

fn fail(error: u32) -> i32 {
    // SAFETY: a kernelbase export without preconditions.
    unsafe { SetLastError(error) };
    FALSE
}

/// A NUL-terminated UTF-16 string as a slice (`None` for null).
///
/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn wide<'a>(p: *const u16) -> Option<&'a [u16]> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: the caller's contract.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        Some(core::slice::from_raw_parts(p, n))
    }
}

/// A NUL-terminated ANSI string as UTF-16 (code page 1252 as in
/// `kernelbase`; ASCII here is what matters).
///
/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn ansi(p: *const u8) -> Option<Vec<u16>> {
    if p.is_null() {
        return None;
    }
    let mut v = Vec::new();
    let mut n = 0;
    // SAFETY: the caller's contract.
    unsafe {
        while *p.add(n) != 0 {
            v.push(u16::from(*p.add(n)));
            n += 1;
        }
    }
    Some(v)
}

/// Copy `bytes` into a new `LocalAlloc` block (freed by the caller with
/// `LocalFree`, as Learn's SDDL pages say).
fn local_copy(bytes: &[u8]) -> Option<*mut u8> {
    // SAFETY: no preconditions.
    let p = unsafe { LocalAlloc(LMEM_FIXED, bytes.len()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` holds `bytes.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
    Some(p)
}

/// The SID at `p` as bytes (only the header if that is not valid).
///
/// # Safety
/// `p` is a SID.
unsafe fn sid_bytes<'a>(p: *const u8) -> &'a [u8] {
    // SAFETY: the caller's contract.
    unsafe {
        let h = core::slice::from_raw_parts(p, 2);
        let len = if h[0] == sid::SID_REVISION && usize::from(h[1]) <= sid::SID_MAX_SUB_AUTHORITIES {
            sid::length_required(h[1]) as usize
        } else {
            2
        };
        core::slice::from_raw_parts(p, len)
    }
}

// ----- the registry ---------------------------------------------------------------------

/// `RegDeleteKeyW`: `RegDeleteKeyExW` without a view.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegDeleteKeyW(hkey: Handle, sub: *const u16) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { RegDeleteKeyExW(hkey, sub, 0, 0) }
}

/// `RegDeleteKeyA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegDeleteKeyA(hkey: Handle, sub: *const u8) -> u32 {
    // SAFETY: the caller's arguments.
    unsafe { RegDeleteKeyExA(hkey, sub, 0, 0) }
}

// ----- SID strings ------------------------------------------------------------------------

/// `ConvertSidToStringSidW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertSidToStringSidW(s: *const u8, out: *mut *mut u16) -> i32 {
    if s.is_null() || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's SID.
    let Some(text) = sddl::to_wide(unsafe { sid_bytes(s) }) else {
        return fail(poly_security::error::ERROR_INVALID_SID);
    };
    let bytes: Vec<u8> = text.iter().chain(&[0]).flat_map(|c| c.to_le_bytes()).collect();
    match local_copy(&bytes) {
        Some(p) => {
            // SAFETY: checked non-null.
            unsafe { out.write(p.cast()) };
            TRUE
        }
        None => fail(ERROR_NOT_ENOUGH_MEMORY),
    }
}

/// `ConvertSidToStringSidA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertSidToStringSidA(s: *const u8, out: *mut *mut u8) -> i32 {
    if s.is_null() || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's SID.
    let Some(text) = sddl::to_wide(unsafe { sid_bytes(s) }) else {
        return fail(poly_security::error::ERROR_INVALID_SID);
    };
    // The string form is ASCII.
    let bytes: Vec<u8> = text.iter().map(|&c| c as u8).chain([0]).collect();
    match local_copy(&bytes) {
        Some(p) => {
            // SAFETY: checked non-null.
            unsafe { out.write(p) };
            TRUE
        }
        None => fail(ERROR_NOT_ENOUGH_MEMORY),
    }
}

fn string_to_sid(text: Option<&[u16]>, out: *mut *mut u8) -> i32 {
    let Some(text) = text else { return fail(ERROR_INVALID_PARAMETER) };
    if out.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let Some(s) = sddl::parse(text) else { return fail(poly_security::error::ERROR_INVALID_SID) };
    match local_copy(&s.to_bytes()) {
        Some(p) => {
            // SAFETY: checked non-null.
            unsafe { out.write(p) };
            TRUE
        }
        None => fail(ERROR_NOT_ENOUGH_MEMORY),
    }
}

/// `ConvertStringSidToSidW`: the `S-R-I-S...` form or a SID string
/// constant ("BA").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertStringSidToSidW(text: *const u16, out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's string or null.
    string_to_sid(unsafe { wide(text) }, out)
}

/// `ConvertStringSidToSidA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertStringSidToSidA(text: *const u8, out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's string or null.
    let text = unsafe { ansi(text) };
    string_to_sid(text.as_deref(), out)
}

// ----- the user ---------------------------------------------------------------------------

/// The name of the process token's user: Poly's single user is the only
/// account (ADR 0066); any other SID is `ERROR_NONE_MAPPED`.
fn user_name() -> Result<&'static str, u32> {
    let mut buf = [0u8; 128];
    let mut len = 0u32;
    // SAFETY: `buf` holds 128 bytes; the process token's pseudo-handle.
    let ok = unsafe {
        GetTokenInformation(
            token::CURRENT_PROCESS_TOKEN as Handle,
            token::class::TOKEN_USER,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut len,
        )
    };
    if ok == 0 {
        return Err(poly_security::error::ERROR_NO_TOKEN);
    }
    let user = sid::Sid::from_bytes(&buf[16..]).ok_or(poly_security::error::ERROR_NONE_MAPPED)?;
    if user == poly_security::poly_user_sid() {
        Ok(poly_security::POLY_USER_NAME)
    } else {
        Err(poly_security::error::ERROR_NONE_MAPPED)
    }
}

/// `GetUserNameW`: `*size` in characters, the NUL included both ways
/// (Learn); a short buffer gets nothing, `ERROR_INSUFFICIENT_BUFFER` and the
/// size needed.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUserNameW(buf: *mut u16, size: *mut u32) -> i32 {
    if size.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let name = match user_name() {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let need = name.len() as u32 + 1;
    // SAFETY: the caller's variable and buffer of `*size` characters.
    unsafe {
        if buf.is_null() || size.read() < need {
            size.write(need);
            return fail(ERROR_INSUFFICIENT_BUFFER);
        }
        for (i, c) in name.encode_utf16().chain([0]).enumerate() {
            buf.add(i).write(c);
        }
        size.write(need);
    }
    TRUE
}

/// `GetUserNameA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetUserNameA(buf: *mut u8, size: *mut u32) -> i32 {
    if size.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let name = match user_name() {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let need = name.len() as u32 + 1;
    // SAFETY: as above, in bytes.
    unsafe {
        if buf.is_null() || size.read() < need {
            size.write(need);
            return fail(ERROR_INSUFFICIENT_BUFFER);
        }
        for (i, c) in name.bytes().chain([0]).enumerate() {
            buf.add(i).write(c);
        }
        size.write(need);
    }
    TRUE
}

// ----- privileges -----------------------------------------------------------------------

/// `LookupPrivilegeValueW` on the local system (any system name is taken
/// as this one): the `wdm.h` LUID of a `winnt.h` privilege name.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LookupPrivilegeValueW(_system: *const u16, name: *const u16, luid: *mut u64) -> i32 {
    // SAFETY: the caller's string or null.
    let Some(name) = (unsafe { wide(name) }) else { return fail(ERROR_INVALID_PARAMETER) };
    if luid.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    match token::privilege_value(name) {
        Some(v) => {
            // SAFETY: checked non-null (`LUID`: LowPart, HighPart = 0).
            unsafe { luid.write(u64::from(v)) };
            TRUE
        }
        None => fail(ERROR_NO_SUCH_PRIVILEGE),
    }
}

/// `LookupPrivilegeNameW`: `*count` in characters; a short buffer gets
/// `ERROR_INSUFFICIENT_BUFFER` and the size needed with the NUL, a success
/// the length without it (to be confirmed by the spec agent).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LookupPrivilegeNameW(
    _system: *const u16,
    luid: *const u64,
    name: *mut u16,
    count: *mut u32,
) -> i32 {
    if luid.is_null() || count.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: checked non-null.
    let v = unsafe { luid.read() };
    let Some(n) = u32::try_from(v).ok().and_then(token::privilege_name) else {
        return fail(ERROR_NO_SUCH_PRIVILEGE);
    };
    // SAFETY: the caller's variable and buffer of `*count` characters.
    unsafe {
        if name.is_null() || count.read() as usize <= n.len() {
            count.write(n.len() as u32 + 1);
            return fail(ERROR_INSUFFICIENT_BUFFER);
        }
        for (i, c) in n.encode_utf16().chain([0]).enumerate() {
            name.add(i).write(c);
        }
        count.write(n.len() as u32);
    }
    TRUE
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn advapi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    TRUE
}
