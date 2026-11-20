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

mod acctrl;
mod etw;
mod eventlog;
mod lsa;
mod withtoken;

use alloc::vec::Vec;
use poly_security::sd::{self, SdPtr};
use poly_security::{sddl, sdstr, sid, token};
use poly_win32_rt as _;

type Handle = isize;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn LocalFree(mem: *mut u8) -> *mut u8;
    fn SetLastError(error: u32);
    fn RegDeleteKeyExW(hkey: Handle, sub: *const u16, sam: u32, reserved: u32) -> u32;
    fn RegDeleteKeyExA(hkey: Handle, sub: *const u8, sam: u32, reserved: u32) -> u32;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const u8,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
}

// ----- the CryptoAPI provider handle (random numbers only) ------------------------------------

#[cfg_attr(target_arch = "x86", link(name = "bcryptprimitives", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "bcryptprimitives", kind = "raw-dylib"))]
unsafe extern "system" {
    fn ProcessPrng(buf: *mut u8, len: usize) -> i32;
}

/// What a provider handle (`HCRYPTPROV`) points to: a live marker, so a released or invented handle is refused.
const PROVIDER_MAGIC: u32 = 0x4350_5256; // "CPRV"

/// `CryptAcquireContextW(phProv, szContainer, szProvider, dwProvType, dwFlags)`: Poly has one provider, the
/// system's random number source, for every container, provider name and type; the handle serves
/// `CryptGenRandom` and `CryptReleaseContext`. (Learn: `CRYPT_VERIFYCONTEXT`, the flag programs that need
/// only random numbers pass, takes no container; the others are accepted alike.)
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptAcquireContextW(
    prov: *mut usize,
    _container: *const u16,
    _provider: *const u16,
    _prov_type: u32,
    _flags: u32,
) -> i32 {
    if prov.is_null() {
        // SAFETY: no preconditions.
        unsafe { SetLastError(87) };
        return 0;
    }
    // SAFETY: a block of the local heap; the caller's out parameter.
    unsafe {
        let p = LocalAlloc(0, 16);
        if p.is_null() {
            SetLastError(14);
            return 0;
        }
        p.cast::<u32>().write(PROVIDER_MAGIC);
        *prov = p as usize;
    }
    1
}

/// `CryptAcquireContextA`: the same.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptAcquireContextA(
    prov: *mut usize,
    container: *const u8,
    provider: *const u8,
    prov_type: u32,
    flags: u32,
) -> i32 {
    // SAFETY: as for the wide form; the names are not read.
    unsafe { CryptAcquireContextW(prov, container.cast(), provider.cast(), prov_type, flags) }
}

/// `CryptReleaseContext(hProv, dwFlags)`: TRUE for a provider handle; `ERROR_INVALID_PARAMETER` (87) otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptReleaseContext(prov: usize, _flags: u32) -> i32 {
    let p = prov as *mut u8;
    // SAFETY: a provider handle points to a block we made; anything else is checked by the marker's address
    // being readable only for such handles, so callers pass nothing else (Learn: undefined for them).
    unsafe {
        if prov == 0 || p.cast::<u32>().read_volatile() != PROVIDER_MAGIC {
            SetLastError(87);
            return 0;
        }
        p.cast::<u32>().write_volatile(0);
        LocalFree(p);
    }
    1
}

/// `CryptGenRandom(hProv, dwLen, pbBuffer)`: `dwLen` random bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CryptGenRandom(prov: usize, len: u32, buf: *mut u8) -> i32 {
    // SAFETY: a provider handle from `CryptAcquireContext*`; the caller's buffer of `len` bytes.
    unsafe {
        if prov == 0 || buf.is_null() || (prov as *const u32).read_volatile() != PROVIDER_MAGIC {
            SetLastError(87);
            return 0;
        }
        i32::from(ProcessPrng(buf, len as usize) != 0)
    }
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

/// `RegDisablePredefinedCache`: Learn: "Instructs the system to disable the predefined registry
/// handle table cache for the current process"; the call returns `ERROR_SUCCESS` and leaves the
/// last error alone (the sheet), also when it is repeated. Poly keeps no cache of
/// `HKEY_CURRENT_USER`'s handle: a predefined key is resolved at each open, so the call only
/// has to succeed.
#[unsafe(no_mangle)]
pub extern "system" fn RegDisablePredefinedCache() -> u32 {
    0
}

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
            // SAFETY: checked non-null; the last error becomes 0
            // (ImpersonateNamedPipeClient.yaml `last_error_in_the_tests`).
            unsafe {
                out.write(p.cast());
                SetLastError(0);
            }
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
    let Some(s) = sddl::parse_bytes(text) else { return fail(poly_security::error::ERROR_INVALID_SID) };
    match local_copy(&s) {
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

// ----- security descriptor strings ------------------------------------------------------

/// `ConvertStringSecurityDescriptorToSecurityDescriptorW`: the SDDL string
/// as a self-relative descriptor in `LocalAlloc` memory (revision 1 only).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
    text: *const u16,
    revision: u32,
    out: *mut *mut u8,
    size: *mut u32,
) -> i32 {
    // SAFETY: the caller's string or null.
    let Some(text) = (unsafe { wide(text) }) else { return fail(ERROR_INVALID_PARAMETER) };
    if revision != u32::from(sd::SECURITY_DESCRIPTOR_REVISION) {
        return fail(poly_security::error::ERROR_UNKNOWN_REVISION);
    }
    match sdstr::parse(text) {
        Ok(bytes) => match local_copy(&bytes) {
            Some(p) => {
                // SAFETY: the caller's variables (the size one may be null).
                unsafe {
                    out.write(p);
                    if !size.is_null() {
                        size.write(bytes.len() as u32);
                    }
                }
                TRUE
            }
            None => fail(ERROR_NOT_ENOUGH_MEMORY),
        },
        Err(e) => {
            if !size.is_null() {
                // SAFETY: the caller's variable.
                unsafe { size.write(0) };
            }
            fail(e.win32())
        }
    }
}

/// `ConvertSecurityDescriptorToStringSecurityDescriptorW`: the SDDL text of
/// the parts `information` names, in `LocalAlloc` memory.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
    descriptor: *mut u8,
    revision: u32,
    information: u32,
    out: *mut *mut u16,
    length: *mut u32,
) -> i32 {
    if revision != u32::from(sd::SECURITY_DESCRIPTOR_REVISION) {
        return fail(poly_security::error::ERROR_UNKNOWN_REVISION);
    }
    let d = SdPtr(descriptor);
    // A NULL descriptor has no parts: the empty string.
    // SAFETY: the caller's descriptor.
    let (text, len) = if descriptor.is_null() {
        (alloc::string::String::new(), 0)
    } else {
        if unsafe { d.revision() } != sd::SECURITY_DESCRIPTOR_REVISION {
            return fail(poly_security::error::ERROR_UNKNOWN_REVISION);
        }
        // SAFETY: the caller's descriptor.
        let parts = unsafe { sd::Parts::read(d) };
        match sdstr::format(&parts, information) {
            Ok(t) => t,
            Err(e) => return fail(e.win32()),
        }
    };
    let bytes: Vec<u8> = text.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect();
    match local_copy(&bytes) {
        Some(p) => {
            // SAFETY: the caller's variables (the length one may be null).
            unsafe {
                out.write(p.cast());
                if !length.is_null() {
                    length.write(len as u32);
                }
            }
            TRUE
        }
        None => fail(ERROR_NOT_ENOUGH_MEMORY),
    }
}

// ----- security information through a handle --------------------------------------------

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQuerySecurityObject(handle: Handle, info: u32, sd: *mut u8, length: u32, needed: *mut u32) -> u32;
    fn NtSetSecurityObject(handle: Handle, info: u32, sd: *const u8) -> u32;
    fn RtlNtStatusToDosError(status: u32) -> u32;
    fn NtOpenKey(out: *mut Handle, access: u32, attributes: *const u8) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtFsControlFile(
        handle: Handle,
        event: Handle,
        apc: usize,
        apc_context: usize,
        iosb: *mut [usize; 2],
        code: u32,
        input: *const u8,
        input_length: u32,
        output: *mut u8,
        output_length: u32,
    ) -> u32;
}

/// `FSCTL_PIPE_IMPERSONATE` (`ntifs.h`).
const FSCTL_PIPE_IMPERSONATE: u32 = 0x0011_001C;

/// `ImpersonateNamedPipeClient(hNamedPipe)`: the calling thread impersonates the client of the
/// server end `hNamedPipe` (Learn; the named-pipe file system's `FSCTL_PIPE_IMPERSONATE`), at the
/// impersonation level the client allowed when it opened the pipe. `RevertToSelf` ends it.
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateNamedPipeClient(pipe: Handle) -> i32 {
    let mut iosb = [0usize; 2];
    // SAFETY: no buffers; the status block is a local.
    let status = unsafe {
        NtFsControlFile(
            pipe,
            0 as Handle,
            0,
            0,
            &mut iosb,
            FSCTL_PIPE_IMPERSONATE,
            core::ptr::null(),
            0,
            core::ptr::null_mut(),
            0,
        )
    };
    if status >= 0x8000_0000 {
        // SAFETY: plain calls.
        return fail(unsafe { RtlNtStatusToDosError(status) });
    }
    1
}

/// `SE_OBJECT_TYPE` (`accctrl.h`): the kinds `GetSecurityInfo` and
/// `SetSecurityInfo` work on through a handle here. `SE_FILE_OBJECT` and
/// `SE_KERNEL_OBJECT` are handles to kernel objects; the service, printer
/// and registry key types are other handles (`ERROR_INVALID_HANDLE`).
fn object_type_ok(ty: u32) -> Result<(), u32> {
    match ty {
        1 | 6 | 7 | 11 => Ok(()),
        2..=4 => Err(6),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

fn status_error(st: u32) -> u32 {
    // SAFETY: a pure ntdll function.
    unsafe { RtlNtStatusToDosError(st) }
}

/// `GetSecurityInfo(handle, ObjectType, SecurityInfo, **owner, **group,
/// **dacl, **sacl, **descriptor)`: returns the error code. The descriptor
/// is a `LocalAlloc`ed self-relative copy; the SID and ACL pointers point
/// into it. A part asked for needs its own pointer or the descriptor's.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSecurityInfo(
    handle: Handle,
    object_type: u32,
    info: u32,
    owner: *mut *mut u8,
    group: *mut *mut u8,
    dacl: *mut *mut u8,
    sacl: *mut *mut u8,
    descriptor: *mut *mut u8,
) -> u32 {
    // SAFETY: the caller's arguments, passed on.
    unsafe { security_info_of(handle, object_type, info, owner, group, dacl, sacl, descriptor, true) }
}

/// `GetSecurityInfo`; `check_type` off for a handle this library opened as
/// the object type says (a registry key by name).
#[allow(clippy::too_many_arguments)]
unsafe fn security_info_of(
    handle: Handle,
    object_type: u32,
    info: u32,
    owner: *mut *mut u8,
    group: *mut *mut u8,
    dacl: *mut *mut u8,
    sacl: *mut *mut u8,
    descriptor: *mut *mut u8,
    check_type: bool,
) -> u32 {
    // SAFETY: the caller's arguments, passed on.
    let rc = unsafe { get_security_info(handle, object_type, info, owner, group, dacl, sacl, descriptor, check_type) };
    if rc != 0 && handle != 0 {
        // A failed call nulls the outputs of the parts it was asked for and
        // the descriptor (the sheets).
        // SAFETY: each pointer is the caller's, written only when non-null.
        unsafe {
            for (w, p) in
                [(info & 1 != 0, owner), (info & 2 != 0, group), (info & 4 != 0, dacl), (info & 0x1F8 != 0, sacl)]
            {
                if w && !p.is_null() {
                    p.write(core::ptr::null_mut());
                }
            }
            if !descriptor.is_null() {
                descriptor.write(core::ptr::null_mut());
            }
        }
    }
    rc
}

#[allow(clippy::too_many_arguments)]
unsafe fn get_security_info(
    handle: Handle,
    object_type: u32,
    info: u32,
    owner: *mut *mut u8,
    group: *mut *mut u8,
    dacl: *mut *mut u8,
    sacl: *mut *mut u8,
    descriptor: *mut *mut u8,
    check_type: bool,
) -> u32 {
    if handle == 0 {
        return 6;
    }
    if check_type && let Err(e) = object_type_ok(object_type) {
        return e;
    }
    // A requested part has to have somewhere to go.
    let wants = [(info & 1 != 0, owner), (info & 2 != 0, group), (info & 4 != 0, dacl), (info & 0x1F8 != 0, sacl)];
    if wants.iter().any(|&(w, p)| w && p.is_null() && descriptor.is_null()) {
        return ERROR_INVALID_PARAMETER;
    }
    let mut needed = 0u32;
    // SAFETY: a size probe.
    let st = unsafe { NtQuerySecurityObject(handle, info, core::ptr::null_mut(), 0, &mut needed) };
    if st != 0 && st != 0xC000_0023 {
        return status_error(st);
    }
    // SAFETY: no preconditions.
    let sd = unsafe { LocalAlloc(LMEM_FIXED, needed.max(20) as usize) };
    if sd.is_null() {
        return ERROR_NOT_ENOUGH_MEMORY;
    }
    // SAFETY: `sd` holds `needed` bytes.
    let st = unsafe { NtQuerySecurityObject(handle, info, sd, needed.max(20), &mut needed) };
    if st != 0 {
        // SAFETY: our block.
        unsafe { LocalFree(sd) };
        return status_error(st);
    }
    // SAFETY: a self-relative descriptor of `needed` bytes at `sd`.
    unsafe {
        let off =
            |at: usize| u32::from_le_bytes([*sd.add(at), *sd.add(at + 1), *sd.add(at + 2), *sd.add(at + 3)]) as usize;
        let part = |at: usize| if off(at) == 0 { core::ptr::null_mut() } else { sd.add(off(at)) };
        if !owner.is_null() && info & 1 != 0 {
            owner.write(part(4));
        }
        if !group.is_null() && info & 2 != 0 {
            group.write(part(8));
        }
        if !dacl.is_null() && info & 4 != 0 {
            dacl.write(part(16));
        }
        if !sacl.is_null() && info & 0x1F8 != 0 {
            sacl.write(part(12));
        }
        // Without a descriptor pointer the block is never freed: the parts the
        // caller asked for point into it (Learn: free the descriptor with LocalFree).
        if !descriptor.is_null() {
            descriptor.write(sd);
        }
    }
    0
}

/// `SetSecurityInfo(handle, ObjectType, SecurityInfo, owner, group, dacl,
/// sacl)`: returns the error code. The parts become a self-relative
/// descriptor for `NtSetSecurityObject`; a NULL DACL with the DACL
/// information is stored protected.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetSecurityInfo(
    handle: Handle,
    object_type: u32,
    info: u32,
    owner: *const u8,
    group: *const u8,
    dacl: *const u8,
    sacl: *const u8,
) -> u32 {
    use poly_security::acl::{self, AclPtr};
    if handle == 0 {
        return 6;
    }
    if let Err(e) = object_type_ok(object_type) {
        return e;
    }
    if object_type != 1 && object_type != 6 {
        return if object_type == 7 || object_type == 11 { 0 } else { ERROR_INVALID_PARAMETER };
    }
    let mut parts = sd::Parts { revision: sd::SECURITY_DESCRIPTOR_REVISION, ..sd::Parts::default() };
    // SAFETY: the caller's SIDs and ACLs.
    unsafe {
        let sid_of = |p: *const u8| -> Result<Vec<u8>, u32> {
            if p.is_null() {
                return Err(ERROR_INVALID_PARAMETER);
            }
            let h = core::slice::from_raw_parts(p, 2);
            if h[0] != sid::SID_REVISION || usize::from(h[1]) > sid::SID_MAX_SUB_AUTHORITIES {
                return Err(ERROR_INVALID_PARAMETER);
            }
            Ok(core::slice::from_raw_parts(p, sid::length_required(h[1]) as usize).to_vec())
        };
        if info & 1 != 0 {
            match sid_of(owner) {
                Ok(s) => parts.owner = Some(s),
                Err(e) => return e,
            }
        }
        if info & 2 != 0 {
            match sid_of(group) {
                Ok(s) => parts.group = Some(s),
                Err(e) => return e,
            }
        }
        let acl_of = |p: *const u8| -> Result<Vec<u8>, u32> {
            if !acl::is_valid(AclPtr(p as *mut u8)) {
                return Err(poly_security::error::ERROR_INVALID_ACL);
            }
            let size = AclPtr(p as *mut u8).size();
            Ok(core::slice::from_raw_parts(p, size).to_vec())
        };
        if info & 4 != 0 {
            parts.control |= sd::SE_DACL_PRESENT;
            if dacl.is_null() {
                parts.control |= sd::SE_DACL_PROTECTED;
            } else {
                match acl_of(dacl) {
                    Ok(a) => parts.dacl = Some(a),
                    Err(e) => return e,
                }
            }
        }
        if info & 0x18 != 0 {
            parts.control |= sd::SE_SACL_PRESENT;
            if !sacl.is_null() {
                match acl_of(sacl) {
                    Ok(a) => parts.sacl = Some(a),
                    Err(e) => return e,
                }
            } else if info & 8 == 0 {
                // A NULL SACL with the label information leaves an empty
                // protected ACL (the sheets: `S:P`).
                parts.control |= sd::SE_SACL_PROTECTED;
            }
        }
    }
    let rel = parts.to_relative();
    // SAFETY: a local self-relative descriptor.
    status_error(unsafe { NtSetSecurityObject(handle, info, rel.as_ptr()) })
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
    // `TOKEN_USER` is a `SID_AND_ATTRIBUTES` (a pointer and a DWORD, padded to two pointers: 16
    // bytes in 64 bits, 8 in 32), the SID right behind it.
    let user = sid::Sid::from_bytes(&buf[2 * core::mem::size_of::<usize>()..])
        .ok_or(poly_security::error::ERROR_NONE_MAPPED)?;
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
            // SAFETY: checked non-null (`LUID`: LowPart, HighPart = 0);
            // ADR 0114: a success leaves the last error `ERROR_SUCCESS`
            // (ADR 0110's trace of Windows; Learn is silent).
            unsafe {
                luid.write(u64::from(v));
                SetLastError(0);
            }
            TRUE
        }
        None => {
            // SAFETY: checked non-null; the failed call sets the LUID to 0:0.
            unsafe { luid.write(0) };
            fail(ERROR_NO_SUCH_PRIVILEGE)
        }
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

// ----- RtlGenRandom (ADR 0087) ------------------------------------------------------------

/// `SystemFunction036`, Learn's `RtlGenRandom` (`ntsecapi.h`: "This
/// function has no associated import library"; exported by
/// `Advapi32.dll` under this name): fill the buffer with random bytes.
/// Poly's come from the kernel's generator through `ntdll!PolyRandomBytes`
/// (ADR 0206): no file is opened, so a sandboxed process whose token cannot
/// open the random device still gets them, as on Windows; TRUE when the
/// buffer was filled.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SystemFunction036(buffer: *mut u8, length: u32) -> u8 {
    if length == 0 {
        return 1;
    }
    // SAFETY: the caller's buffer of `length` bytes.
    u8::from(unsafe { PolyRandomBytes(buffer, length as usize) } >= 0)
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    /// Poly's own: random bytes of the kernel's generator (ADR 0206); an `NTSTATUS`.
    fn PolyRandomBytes(buffer: *mut u8, length: usize) -> i32;
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn advapi32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    TRUE
}

mod delayed;
mod services;

mod apps;
