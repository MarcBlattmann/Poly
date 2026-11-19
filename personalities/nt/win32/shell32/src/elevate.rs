//! User Account Control in the Shell (ADR 0222): `IsUserAnAdmin`, `ShellExecuteA`, and the
//! elevation `ShellExecuteExW` ([`crate::exec`], ADR 0223) asks for, with the rules of the sheets
//! (`specs/windows/shell32.dll/{IsUserAnAdmin,ShellExecuteExW.Elevation,ShellExecuteW,ShellExecuteA}.yaml`).
//!
//! - `open` (or no verb) starts a program with the caller's token. If `CreateProcessW` refuses it
//!   because its manifest requests administrator rights (`ERROR_ELEVATION_REQUIRED`), the call
//!   becomes a `runas`.
//! - `runas` from a caller that is not elevated asks Poly's consent broker
//!   (`PolyRequestElevation`, `personalities/nt/src/consent.rs`): the desktop shows its prompt.
//!   Yes gives a full token the program is started with (`CreateProcessAsUserW`); No is
//!   `ERROR_CANCELLED` with `hInstApp` 5 (`SE_ERR_ACCESSDENIED`), and nothing is started. An
//!   elevated caller starts it without a prompt.
//! - A missing file fails with 2 before any prompt, an unknown verb with 1155 / 31 ([`crate::exec`]).

use crate::exec::ProcessInformation;
use alloc::vec::Vec;
use core::ffi::c_void;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, cch: i32) -> i32;
    fn CloseHandle(h: usize) -> i32;
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateProcessAsUserW(
        token: usize,
        app: *const u16,
        cmd: *mut u16,
        pa: *mut c_void,
        ta: *mut c_void,
        inherit: i32,
        flags: u32,
        env: *mut c_void,
        dir: *const u16,
        si: *mut c_void,
        pi: *mut ProcessInformation,
    ) -> i32;
    fn OpenThreadToken(thread: usize, access: u32, as_self: i32, token: *mut usize) -> i32;
    fn OpenProcessToken(process: usize, access: u32, token: *mut usize) -> i32;
}

/// `GetTokenInformation` (declared once, in the crate root).
unsafe fn token_information(token: usize, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> i32 {
    // SAFETY: the caller's buffer of `length` bytes.
    unsafe { crate::GetTokenInformation(token as *mut u8, class, info, length, ret) }
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    /// Poly's consent broker (ADR 0222).
    fn PolyRequestElevation(path: *const u16, length: usize, token: *mut usize) -> u32;
}

/// `STARTUPINFOW` (`processthreadsapi.h`): 104 bytes on x64, 68 on x86.
#[repr(C)]
#[derive(Default)]
struct StartupInfo {
    cb: u32,
    reserved: usize,
    desktop: usize,
    title: usize,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_chars: u32,
    y_chars: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved2: u16,
    reserved3: usize,
    input: usize,
    output: usize,
    error: usize,
}

// `shellapi.h`, `winerror.h`, `ntstatus.h`, `winnt.h`.
const SE_ERR_ACCESSDENIED: usize = 5;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_CANCELLED: u32 = 1223;
/// `ERROR_ELEVATION_REQUIRED`: `CreateProcessW` of an image whose manifest asks for administrator rights.
pub const ERROR_ELEVATION_REQUIRED: u32 = 740;
const STATUS_CANCELLED: u32 = 0xC000_0120;
const STARTF_USESHOWWINDOW: u32 = 0x1;
const TOKEN_QUERY: u32 = 0x8;
const TOKEN_GROUPS: u32 = 2;
const TOKEN_IMPERSONATION_LEVEL: u32 = 9;
const TOKEN_ELEVATION: u32 = 20;
const SECURITY_IMPERSONATION: u32 = 2;
const SE_GROUP_ENABLED: u32 = 0x4;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;
/// The pseudo-handles of the current process and thread.
const CURRENT_PROCESS: usize = usize::MAX;
const CURRENT_THREAD: usize = usize::MAX - 1;
const CP_ACP: u32 = 0;
/// `BUILTIN\Administrators` (S-1-5-32-544) as bytes.
const ADMINISTRATORS: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 2, 0, 0];

/// A NUL-terminated copy.
fn nul(s: &[u16]) -> Vec<u16> {
    let mut v = s.to_vec();
    v.push(0);
    v
}

/// The token `IsUserAnAdmin` looks at: the thread's, else the process's (and whether it is the
/// thread's).
fn effective_token() -> Option<(usize, bool)> {
    let mut t = 0usize;
    // SAFETY: valid pointers; the pseudo-handles.
    unsafe {
        if OpenThreadToken(CURRENT_THREAD, TOKEN_QUERY, 1, &mut t) != 0 {
            return Some((t, true));
        }
        if OpenProcessToken(CURRENT_PROCESS, TOKEN_QUERY, &mut t) != 0 {
            return Some((t, false));
        }
    }
    None
}

fn token_u32(t: usize, class: u32) -> Option<u32> {
    let mut v = [0u8; 4];
    let mut ret = 0;
    // SAFETY: a 4-byte buffer.
    (unsafe { token_information(t, class, v.as_mut_ptr(), 4, &mut ret) } != 0).then(|| u32::from_le_bytes(v))
}

/// Whether the Administrators group of token `t` is enabled (not deny-only).
fn admins_enabled(t: usize) -> bool {
    let mut buf = alloc::vec![0u8; 2048];
    let mut ret = 0;
    // SAFETY: the buffer's length.
    if unsafe { token_information(t, TOKEN_GROUPS, buf.as_mut_ptr(), buf.len() as u32, &mut ret) } == 0 {
        return false;
    }
    let p = core::mem::size_of::<usize>();
    let count = u32::from_le_bytes(buf[0..4].try_into().unwrap_or([0; 4])) as usize;
    (0..count).any(|i| {
        // TOKEN_GROUPS: the count (padded to a pointer), then SID_AND_ATTRIBUTES (a pointer, a DWORD).
        let at = p + 2 * p * i;
        let (Some(sp), Some(attrs)) = (buf.get(at..at + p), buf.get(at + p..at + p + 4)) else { return false };
        let mut b = [0u8; 8];
        b[..p].copy_from_slice(sp);
        let sid = u64::from_le_bytes(b) as usize as *const u8;
        let attrs = u32::from_le_bytes(attrs.try_into().unwrap_or([0; 4]));
        // SAFETY: the SID lies in `buf` (the class lays them out after the array).
        let same = unsafe { core::slice::from_raw_parts(sid, ADMINISTRATORS.len()) } == ADMINISTRATORS;
        same && attrs & SE_GROUP_ENABLED != 0 && attrs & SE_GROUP_USE_FOR_DENY_ONLY == 0
    })
}

/// `IsUserAnAdmin`: TRUE when the calling thread's token (its impersonation token, else the
/// process's) has the Administrators group enabled. An impersonation token below the
/// impersonation level gives no rights: FALSE. The last error is left as it was.
#[unsafe(no_mangle)]
pub extern "system" fn IsUserAnAdmin() -> i32 {
    // SAFETY: plain calls.
    let saved = unsafe { GetLastError() };
    let answer = match effective_token() {
        Some((t, thread)) => {
            let usable = !thread || token_u32(t, TOKEN_IMPERSONATION_LEVEL).is_some_and(|l| l >= SECURITY_IMPERSONATION);
            let a = usable && admins_enabled(t);
            // SAFETY: our handle.
            unsafe { CloseHandle(t) };
            a
        }
        None => false,
    };
    // SAFETY: a plain call.
    unsafe { SetLastError(saved) };
    i32::from(answer)
}

/// Whether the caller's token (the impersonated one, else the process's) is elevated.
pub fn caller_elevated() -> bool {
    match effective_token() {
        Some((t, _)) => {
            let e = token_u32(t, TOKEN_ELEVATION).is_none_or(|e| e != 0);
            // SAFETY: our handle.
            unsafe { CloseHandle(t) };
            e
        }
        None => true,
    }
}

/// Ask the user (the consent broker) to start the program `image` elevated with command line
/// `command` in `dir` (NULL: ours), and start it with the full token for a yes.
/// `Err((last error, hInstApp))`: No is `ERROR_CANCELLED` with 5.
pub fn start_elevated(
    image: &[u16],
    command: &[u16],
    dir: Option<&[u16]>,
    show: i32,
) -> Result<ProcessInformation, (u32, usize)> {
    let mut token = 0usize;
    // SAFETY: the image's units and a local out-variable.
    let st = unsafe { PolyRequestElevation(image.as_ptr(), image.len(), &mut token) };
    if st == STATUS_CANCELLED {
        return Err((ERROR_CANCELLED, SE_ERR_ACCESSDENIED));
    }
    if st != 0 || token == 0 {
        return Err((ERROR_ACCESS_DENIED, SE_ERR_ACCESSDENIED));
    }
    let app = nul(image);
    let mut cmd = nul(command);
    let dir = dir.map(nul);
    let mut si = StartupInfo {
        cb: core::mem::size_of::<StartupInfo>() as u32,
        flags: STARTF_USESHOWWINDOW,
        show: show as u16,
        ..Default::default()
    };
    let mut pi = ProcessInformation { process: 0, thread: 0, pid: 0, tid: 0 };
    let null = core::ptr::null_mut();
    // SAFETY: NUL-terminated strings (the command line writable), local structures, the broker's token.
    let ok = unsafe {
        CreateProcessAsUserW(
            token,
            app.as_ptr(),
            cmd.as_mut_ptr(),
            null,
            null,
            0,
            0,
            null,
            dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()),
            (&mut si as *mut StartupInfo).cast(),
            &mut pi,
        )
    };
    // SAFETY: plain calls; the token handle the broker gave.
    let error = unsafe { if ok == 0 { GetLastError() } else { 0 } };
    // SAFETY: as above.
    unsafe { CloseHandle(token) };
    if ok == 0 { Err((error, SE_ERR_ACCESSDENIED)) } else { Ok(pi) }
}

/// An ANSI argument in UTF-16 (`CP_ACP`), NUL-terminated.
unsafe fn ansi(s: *const u8) -> Option<Vec<u16>> {
    if s.is_null() {
        return None;
    }
    // SAFETY: the caller's NUL-terminated string (-1: up to the NUL, which is counted).
    unsafe {
        let n = MultiByteToWideChar(CP_ACP, 0, s, -1, core::ptr::null_mut(), 0);
        let mut w = alloc::vec![0u16; n.max(1) as usize];
        let n = MultiByteToWideChar(CP_ACP, 0, s, -1, w.as_mut_ptr(), n);
        w.truncate((n.max(1) - 1) as usize);
        w.push(0);
        Some(w)
    }
}

/// `ShellExecuteA`: `ShellExecuteW` with the strings from the ANSI code page.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ShellExecuteA(
    hwnd: usize,
    verb: *const u8,
    file: *const u8,
    params: *const u8,
    dir: *const u8,
    show: i32,
) -> usize {
    // SAFETY: the caller's strings.
    let (v, f, p, d) = unsafe { (ansi(verb), ansi(file), ansi(params), ansi(dir)) };
    let ptr = |s: &Option<Vec<u16>>| s.as_ref().map_or(core::ptr::null(), |w| w.as_ptr());
    // SAFETY: NUL-terminated copies.
    unsafe { crate::exec::ShellExecuteW(hwnd, ptr(&v), ptr(&f), ptr(&p), ptr(&d), show) }
}
