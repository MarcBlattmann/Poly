//! `CreateProcessWithTokenW` (Learn, `winbase.h`; the sheet
//! `specs/windows/advapi32.dll/CreateProcessWithTokenW.yaml`): a process with a duplicate of the
//! given token as its primary token, its parent the caller, but none of the caller's handles,
//! environment or directory. The checks and their order are `poly_kernelbase::withtoken`; the
//! process is made by `kernelbase!CreateProcessAsUserW` without handle inheritance, with the
//! token user's default environment (`userenv!CreateEnvironmentBlock`) when none is given and
//! `System32` as its directory when none is given.

use alloc::vec::Vec;
use poly_kernelbase::withtoken::{self as wt, TokenFacts};

type Handle = isize;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetLastError(error: u32);
    fn CloseHandle(handle: Handle) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const u8;
    fn CreateProcessAsUserW(
        token: Handle,
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const u8,
        thread_attributes: *const u8,
        inherit_handles: i32,
        flags: u32,
        environment: *const u8,
        current_directory: *const u16,
        startup_info: *const u8,
        information: *mut u8,
    ) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> u32;
    fn NtQueryObject(handle: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> u32;
}

/// `TokenPrivileges`, `TokenType`, `TokenImpersonationLevel`, `TokenStatistics`
/// (`TOKEN_INFORMATION_CLASS`, `winnt.h`).
const TOKEN_PRIVILEGES: u32 = 3;
const TOKEN_TYPE: u32 = 8;
const TOKEN_IMPERSONATION_LEVEL: u32 = 9;
const TOKEN_STATISTICS: u32 = 10;
/// `ObjectBasicInformation` (`winternl.h`): `GrantedAccess` is the second `ULONG` of the 56 bytes.
const OBJECT_BASIC_INFORMATION: u32 = 0;
/// `CREATE_UNICODE_ENVIRONMENT` (`winbase.h`).
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
/// `sizeof(STARTUPINFOW)` (`processthreadsapi.h`): 104 bytes, 68 in a 32-bit process.
const STARTUPINFO_SIZE: usize = if core::mem::size_of::<usize>() == 8 { 104 } else { 68 };

fn fail(error: u32) -> i32 {
    // SAFETY: a plain call.
    unsafe { SetLastError(error) };
    0
}

/// A `ULONG`-sized token class of `token`, or the Win32 error of the query.
fn token_u32(token: Handle, class: u32) -> Result<u32, u32> {
    let (mut v, mut ret) = (0u32, 0u32);
    // SAFETY: a 4-byte buffer for a 4-byte class.
    let st = unsafe { NtQueryInformationToken(token, class, (&mut v as *mut u32).cast(), 4, &mut ret) };
    if st == 0 { Ok(v) } else { Err(poly_kernelbase::error::from_status(poly_kernelbase::NtStatus(st))) }
}

/// `TokenStatistics.TokenId` (`TOKEN_STATISTICS`, `winnt.h`: the first `LUID`).
fn token_id(token: Handle) -> Option<u64> {
    let mut buf = [0u64; 8];
    let mut ret = 0u32;
    // SAFETY: a 64-byte buffer (TOKEN_STATISTICS is 56).
    let st = unsafe { NtQueryInformationToken(token, TOKEN_STATISTICS, buf.as_mut_ptr().cast(), 64, &mut ret) };
    (st == 0).then_some(buf[0])
}

/// Whether the caller's own token holds `SeImpersonatePrivilege`.
fn caller_may_impersonate() -> bool {
    let mut own: Handle = 0;
    // SAFETY: an out-pointer; the handle is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), wt::TOKEN_QUERY, &mut own) } == 0 {
        return false;
    }
    let mut buf = alloc::vec![0u8; 4096];
    let mut ret = 0u32;
    // SAFETY: the buffer holds 4096 bytes.
    let st = unsafe { NtQueryInformationToken(own, TOKEN_PRIVILEGES, buf.as_mut_ptr(), 4096, &mut ret) };
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(own) };
    st == 0 && wt::holds_impersonate(&buf[..(ret as usize).min(buf.len())])
}

/// Whether `token` is the caller's own primary token (the same token, not a duplicate).
fn is_callers_primary(token: Handle) -> bool {
    let mut own: Handle = 0;
    // SAFETY: an out-pointer; the handle is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), wt::TOKEN_QUERY, &mut own) } == 0 {
        return false;
    }
    let same = token_id(own).is_some_and(|id| token_id(token) == Some(id));
    // SAFETY: the handle opened above.
    unsafe { CloseHandle(own) };
    same
}

/// Step 3 of the checks: the token handle.
fn check_token(token: Handle) -> Result<(), u32> {
    let token_type = token_u32(token, TOKEN_TYPE)?;
    let mut basic = [0u32; 14];
    let mut ret = 0u32;
    // SAFETY: a 56-byte buffer.
    let st = unsafe { NtQueryObject(token, OBJECT_BASIC_INFORMATION, basic.as_mut_ptr().cast(), 56, &mut ret) };
    let granted = if st == 0 { basic[1] } else { 0 };
    let level = if token_type == wt::TOKEN_IMPERSONATION { token_u32(token, TOKEN_IMPERSONATION_LEVEL)? } else { 0 };
    let facts = TokenFacts { granted, token_type, level, is_callers_primary: is_callers_primary(token) };
    wt::check_token(&facts)
}

/// `userenv!CreateEnvironmentBlock` / `DestroyEnvironmentBlock`, found when first needed
/// (`userenv.dll` is not a dependency of every process that loads `advapi32`).
type CreateBlock = unsafe extern "system" fn(*mut *mut u16, Handle, i32) -> i32;
type DestroyBlock = unsafe extern "system" fn(*mut u16) -> i32;

fn userenv() -> Option<(CreateBlock, DestroyBlock)> {
    let name: Vec<u16> = "userenv.dll".encode_utf16().chain([0]).collect();
    // SAFETY: a NUL-terminated name; the module stays loaded.
    let m = unsafe { LoadLibraryW(name.as_ptr()) };
    if m == 0 {
        return None;
    }
    // SAFETY: NUL-terminated names of the module's exports.
    let (c, d) = unsafe {
        (
            GetProcAddress(m, c"CreateEnvironmentBlock".as_ptr().cast()),
            GetProcAddress(m, c"DestroyEnvironmentBlock".as_ptr().cast()),
        )
    };
    if c.is_null() || d.is_null() {
        return None;
    }
    // SAFETY: the exports have these signatures (Learn, `userenv.h`).
    Some(unsafe {
        (core::mem::transmute::<*const u8, CreateBlock>(c), core::mem::transmute::<*const u8, DestroyBlock>(d))
    })
}

/// A NUL-terminated string's units, if the pointer is not null.
unsafe fn units<'a>(p: *const u16) -> Option<&'a [u16]> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: NUL-terminated per the caller's contract.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        Some(core::slice::from_raw_parts(p, n))
    }
}

/// `CreateProcessWithTokenW(hToken, dwLogonFlags, lpApplicationName, lpCommandLine,
/// dwCreationFlags, lpEnvironment, lpCurrentDirectory, lpStartupInfo, lpProcessInformation)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateProcessWithTokenW(
    token: Handle,
    logon_flags: u32,
    application: *const u16,
    command_line: *mut u16,
    flags: u32,
    environment: *const u8,
    current_directory: *const u16,
    startup_info: *const u8,
    information: *mut u8,
) -> i32 {
    // SAFETY: the caller's strings.
    let (app, cmd, cwd) = unsafe { (units(application), units(command_line), units(current_directory)) };
    if let Err(e) = wt::check_arguments(logon_flags, flags, app.is_some(), cmd.map(<[u16]>::len), token == 0) {
        return fail(e);
    }
    if !caller_may_impersonate() {
        return fail(wt::ERROR_PRIVILEGE_NOT_HELD);
    }
    if let Err(e) = check_token(token) {
        return fail(e);
    }
    // The directory: the one given, else System32.
    let mut sys = [0u16; 260];
    // SAFETY: the buffer holds 260 units.
    let n = unsafe { GetSystemDirectoryW(sys.as_mut_ptr(), sys.len() as u32) } as usize;
    let dir = wt::child_directory(cwd, &sys[..n.min(sys.len())]);
    // The environment: the one given, else the token user's.
    let mut own_block: *mut u16 = core::ptr::null_mut();
    let mut destroy: Option<DestroyBlock> = None;
    let (env, flags) = if environment.is_null() {
        match userenv() {
            // SAFETY: an out-pointer and the caller's token.
            Some((create, d)) if unsafe { create(&mut own_block, token, 0) } != 0 => {
                destroy = Some(d);
                (own_block as *const u8, flags | CREATE_UNICODE_ENVIRONMENT)
            }
            _ => (core::ptr::null(), flags),
        }
    } else {
        (environment, flags)
    };
    // A NULL STARTUPINFO is accepted: the child gets an empty one.
    let mut blank = [0u8; STARTUPINFO_SIZE];
    blank[..4].copy_from_slice(&(STARTUPINFO_SIZE as u32).to_le_bytes());
    let si = if startup_info.is_null() { blank.as_ptr() } else { startup_info };
    // SAFETY: the caller's arguments; nothing of the caller is inherited.
    let ok = unsafe {
        CreateProcessAsUserW(
            token,
            application,
            command_line,
            core::ptr::null(),
            core::ptr::null(),
            0,
            flags,
            env,
            dir.as_ptr(),
            si,
            information,
        )
    };
    if let Some(d) = destroy {
        // SAFETY: the block made above; the child has its own copy. The last error of the
        // creation is kept.
        unsafe { d(own_block) };
    }
    ok
}
