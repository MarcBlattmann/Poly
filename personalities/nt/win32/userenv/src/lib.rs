//! Our `userenv.dll` (ADR 0171). It has one export that programs import,
//! `DeriveAppContainerSidFromAppContainerName(name, &sid)`: the AppContainer
//! SID of a name, a pure function of it. The spec sheet's test vectors fix
//! the derivation: `S-1-15-2-` and the first 28 bytes of the SHA-256 of the
//! lower-cased name in UTF-16LE (no terminator), read as seven
//! little-endian 32-bit sub-authorities. A name of 1 to 64 UTF-16 units is
//! accepted, anything else is `E_INVALIDARG`. The SID is allocated with
//! `LocalAlloc` (`FreeSid` frees it).
//!
//! `RegisterGPNotification` and `UnregisterGPNotification` (Learn, `userenv.h`) are here
//! because Chrome's policy loader delay-loads them, and a delay-load that finds no export is
//! fatal for Chrome (ADR 0155). Poly has no Group Policy, so a registered event is never set.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_crypto::sha256::Sha256;
use poly_win32_cryptrt as _;

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn SetLastError(error: u32);
}

/// `ERROR_INVALID_PARAMETER` (`winerror.h`).
const ERROR_INVALID_PARAMETER: u32 = 87;

/// `RegisterGPNotification(hEvent, bMachine)`: "enables an application to receive notification
/// when there is a change in policy. When a policy change occurs, the specified event object is
/// set to the signaled state" (Learn). Policy never changes on Poly, so there is nothing to
/// remember: the registration succeeds and the event stays as it is. A null event is refused.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterGPNotification(event: isize, _machine: i32) -> i32 {
    if event == 0 {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return 0;
    }
    1
}

/// `UnregisterGPNotification(hEvent)`: the event gets no more notifications (it never got one).
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterGPNotification(event: isize) -> i32 {
    RegisterGPNotification(event, 0)
}

/// `GetProfileType(dwFlags)`: "Retrieves the type of profile loaded for the current user"
/// (Learn, `userenv.h`). The flags are 0 for a local profile (`PT_TEMPORARY`, `PT_ROAMING` and
/// `PT_MANDATORY` are the other kinds); Poly's one user has a local profile on `C:`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetProfileType(flags: *mut u32) -> i32 {
    if flags.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return 0;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { flags.write_unaligned(0) };
    1
}

const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;

/// The 40 bytes of the SID of `name` (UTF-16 units).
fn derive(name: &[u16]) -> [u8; 40] {
    let mut h = Sha256::new();
    for &u in name {
        // Lower-case what has a simple lower case (`char::to_lowercase`).
        let lower = char::from_u32(u32::from(u))
            .and_then(|c| c.to_lowercase().next())
            .map_or(u, |c| if (c as u32) <= 0xFFFF { c as u32 as u16 } else { u });
        h.update(&lower.to_le_bytes());
    }
    let digest = h.finish();
    let mut sid = [0u8; 40];
    sid[0] = 1;
    sid[1] = 8;
    sid[7] = 15;
    sid[8..12].copy_from_slice(&2u32.to_le_bytes());
    sid[12..40].copy_from_slice(&digest[..28]);
    sid
}

/// `DeriveAppContainerSidFromAppContainerName`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut u8) -> i32 {
    if sid.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out pointer.
    unsafe { sid.write(core::ptr::null_mut()) };
    if name.is_null() {
        return E_INVALIDARG;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string; at most 65 units are read.
    while n <= 64 && unsafe { *name.add(n) } != 0 {
        n += 1;
    }
    if n == 0 || n > 64 {
        return E_INVALIDARG;
    }
    // SAFETY: `n` units were just read.
    let bytes = derive(unsafe { core::slice::from_raw_parts(name, n) });
    // SAFETY: no preconditions.
    let p = unsafe { LocalAlloc(0, bytes.len()) };
    if p.is_null() {
        return E_OUTOFMEMORY;
    }
    // SAFETY: `p` holds 40 bytes; the out pointer is the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        sid.write(p);
    }
    0
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn userenv_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
