//! User Account Control in the Shell (ADR 0222): `IsUserAnAdmin`, and `ShellExecuteExW`,
//! `ShellExecuteW` and `ShellExecuteA` for programs, with the elevation rules of the sheets
//! (`specs/windows/shell32.dll/{IsUserAnAdmin,ShellExecuteExW.Elevation,ShellExecuteW,ShellExecuteA}.yaml`).
//!
//! - `open` (or no verb) starts the program with the caller's token. If `CreateProcessW` refuses
//!   it because its manifest requests administrator rights (`ERROR_ELEVATION_REQUIRED`), the
//!   call becomes a `runas`.
//! - `runas` from a caller that is not elevated asks Poly's consent broker
//!   (`PolyRequestElevation`, `personalities/nt/src/consent.rs`): the desktop shows its prompt.
//!   Yes gives a full token the program is started with (`CreateProcessAsUserW`); No is
//!   `ERROR_CANCELLED` with `hInstApp` 5 (`SE_ERR_ACCESSDENIED`), and nothing is started. An
//!   elevated caller starts it without a prompt.
//! - A missing file fails with 2 before any prompt, an unknown verb with 1155 / 31.
//!
//! What Chrome's downloads need of `ShellExecuteExW` for documents and folders (associations,
//! the desktop's Files) is the `ShellExecuteExW` entry's (ADR 0223, `impl-chrome-dl`); it calls
//! [`gate`] for the rules here. Until it lands, a file that is not a program fails here with
//! `ERROR_NO_ASSOCIATION`.

use alloc::vec::Vec;
use core::ffi::c_void;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateProcessW(
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
    fn GetFullPathNameW(name: *const u16, size: u32, buf: *mut u16, part: *mut *mut u16) -> u32;
    fn SearchPathW(path: *const u16, name: *const u16, ext: *const u16, size: u32, buf: *mut u16, part: *mut *mut u16) -> u32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn GetCurrentDirectoryW(size: u32, buf: *mut u16) -> u32;
    fn SetCurrentDirectoryW(dir: *const u16) -> i32;
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

/// `SHELLEXECUTEINFOW` (`shellapi.h`): 112 bytes on x64, 60 on x86.
#[repr(C)]
pub struct ShellExecuteInfo {
    size: u32,
    mask: u32,
    hwnd: usize,
    verb: *const u16,
    file: *const u16,
    params: *const u16,
    dir: *const u16,
    show: i32,
    inst_app: usize,
    id_list: *const u8,
    class: *const u16,
    hkey_class: usize,
    hot_key: u32,
    icon: usize,
    process: usize,
}

#[repr(C)]
struct ProcessInformation {
    process: usize,
    thread: usize,
    pid: u32,
    tid: u32,
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

// `shellapi.h`.
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const SE_ERR_FNF: usize = 2;
const SE_ERR_ACCESSDENIED: usize = 5;
const SE_ERR_NOASSOC: usize = 31;
/// What `hInstApp` holds after a success: any value above 32 (Learn); Windows gives 42.
const SUCCESS_INST: usize = 42;
// `winerror.h`.
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_ELEVATION_REQUIRED: u32 = 740;
const ERROR_NO_ASSOCIATION: u32 = 1155;
const ERROR_CANCELLED: u32 = 1223;
const ERROR_NO_TOKEN: u32 = 1008;
/// `STATUS_CANCELLED` (`ntstatus.h`): the consent broker's No.
const STATUS_CANCELLED: u32 = 0xC000_0120;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const STARTF_USESHOWWINDOW: u32 = 0x1;
/// `TOKEN_QUERY` and the token classes `IsUserAnAdmin` reads (`winnt.h`).
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
/// `CP_ACP`.
const CP_ACP: u32 = 0;
/// `BUILTIN\Administrators` (S-1-5-32-544) as bytes.
const ADMINISTRATORS: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 2, 0, 0];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// A NUL-terminated copy.
fn nul(s: &[u16]) -> Vec<u16> {
    let mut v = s.to_vec();
    v.push(0);
    v
}

/// The units of a NUL-terminated string (`None` for NULL).
unsafe fn units<'a>(p: *const u16) -> Option<&'a [u16]> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: the caller's NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        Some(core::slice::from_raw_parts(p, n))
    }
}

fn eq_ignore_case(a: &[u16], b: &str) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.bytes()).all(|(&x, y)| x < 0x80 && (x as u8).eq_ignore_ascii_case(&y))
}

/// The token `IsUserAnAdmin` looks at: the thread's, else the process's.
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

/// Whether the process (or the impersonated) token is elevated.
fn caller_elevated() -> bool {
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

/// What a call names, resolved.
struct Target {
    /// The program's full path.
    image: Vec<u16>,
    /// The verb: `runas` (true) or `open`.
    runas: bool,
}

/// The decision of the rules before anything starts (also for `ShellExecuteExW`'s document
/// path, ADR 0223): the file must exist (else 2 / `SE_ERR_FNF`), the verb must be `open`,
/// `runas` or none (else 1155 / `SE_ERR_NOASSOC`), the file must be a program (else 1155).
/// `Err((last error, hInstApp))`.
fn gate(verb: Option<&[u16]>, file: &[u16], dir: Option<&[u16]>) -> Result<Target, (u32, usize)> {
    let image = resolve(file, dir).ok_or((ERROR_FILE_NOT_FOUND, SE_ERR_FNF))?;
    let runas = match verb {
        None => false,
        Some(v) if v.is_empty() || eq_ignore_case(v, "open") => false,
        Some(v) if eq_ignore_case(v, "runas") => true,
        Some(_) => return Err((ERROR_NO_ASSOCIATION, SE_ERR_NOASSOC)),
    };
    let lower: Vec<u16> = image.iter().map(|&c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c }).collect();
    let program = [".exe", ".com"].iter().any(|e| lower.ends_with(&wide(e)));
    if !program {
        return Err((ERROR_NO_ASSOCIATION, SE_ERR_NOASSOC));
    }
    Ok(Target { image, runas })
}

/// The full path of `file` (relative to `dir` when given), if it is a file that exists; a bare
/// name is looked for on the search path too.
fn resolve(file: &[u16], dir: Option<&[u16]>) -> Option<Vec<u16>> {
    let mut saved = [0u16; 1024];
    let dir = dir.filter(|d| !d.is_empty());
    // SAFETY: a buffer of 1024 units; NUL-terminated names.
    let restore = dir.is_some_and(|d| unsafe {
        GetCurrentDirectoryW(saved.len() as u32, saved.as_mut_ptr()) != 0 && SetCurrentDirectoryW(nul(d).as_ptr()) != 0
    });
    let name = nul(file);
    let mut buf = alloc::vec![0u16; 1024];
    // SAFETY: as above.
    let mut n = unsafe { GetFullPathNameW(name.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), core::ptr::null_mut()) }
        as usize;
    let exists = |p: &[u16]| {
        // SAFETY: a NUL-terminated name.
        let a = unsafe { GetFileAttributesW(nul(p).as_ptr()) };
        a != INVALID_FILE_ATTRIBUTES && a & FILE_ATTRIBUTE_DIRECTORY == 0
    };
    let bare = !file.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/') || c == u16::from(b':'));
    if (n == 0 || n >= buf.len() || !exists(&buf[..n])) && bare {
        let ext = nul(&wide(".exe"));
        // SAFETY: as above.
        n = unsafe {
            SearchPathW(core::ptr::null(), name.as_ptr(), ext.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), core::ptr::null_mut())
        } as usize;
    }
    if restore {
        // SAFETY: the directory saved above.
        unsafe { SetCurrentDirectoryW(saved.as_ptr()) };
    }
    (n != 0 && n < buf.len() && exists(&buf[..n])).then(|| buf[..n].to_vec())
}

/// Start `t` with `params` in `dir`, as `token` (the caller's when 0). `Err(last error)`.
fn start(t: &Target, params: Option<&[u16]>, dir: Option<&[u16]>, show: i32, token: usize) -> Result<ProcessInformation, u32> {
    let mut line = Vec::with_capacity(t.image.len() + 3);
    line.push(u16::from(b'"'));
    line.extend_from_slice(&t.image);
    line.push(u16::from(b'"'));
    if let Some(p) = params.filter(|p| !p.is_empty()) {
        line.push(u16::from(b' '));
        line.extend_from_slice(p);
    }
    line.push(0);
    let image = nul(&t.image);
    let dir = dir.filter(|d| !d.is_empty()).map(nul);
    let dir_ptr = dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr());
    let mut si = StartupInfo {
        cb: core::mem::size_of::<StartupInfo>() as u32,
        flags: STARTF_USESHOWWINDOW,
        show: show as u16,
        ..Default::default()
    };
    let mut pi = ProcessInformation { process: 0, thread: 0, pid: 0, tid: 0 };
    let si_ptr = &mut si as *mut StartupInfo as *mut c_void;
    let null = core::ptr::null_mut();
    // SAFETY: NUL-terminated strings, local structures.
    let ok = unsafe {
        if token == 0 {
            CreateProcessW(image.as_ptr(), line.as_mut_ptr(), null, null, 0, 0, null, dir_ptr, si_ptr, &mut pi)
        } else {
            CreateProcessAsUserW(token, image.as_ptr(), line.as_mut_ptr(), null, null, 0, 0, null, dir_ptr, si_ptr, &mut pi)
        }
    };
    // SAFETY: a plain call.
    if ok == 0 { Err(unsafe { GetLastError() }) } else { Ok(pi) }
}

/// Ask the user (the consent broker) to start `t` elevated, and start it with the full token
/// for a yes. `Err((last error, hInstApp))`.
fn start_elevated(t: &Target, params: Option<&[u16]>, dir: Option<&[u16]>, show: i32) -> Result<ProcessInformation, (u32, usize)> {
    let mut token = 0usize;
    // SAFETY: the image's units and a local out-variable.
    let st = unsafe { PolyRequestElevation(t.image.as_ptr(), t.image.len(), &mut token) };
    if st == STATUS_CANCELLED {
        return Err((ERROR_CANCELLED, SE_ERR_ACCESSDENIED));
    }
    if st != 0 || token == 0 {
        return Err((ERROR_ACCESS_DENIED, SE_ERR_ACCESSDENIED));
    }
    let r = start(t, params, dir, show, token);
    // SAFETY: the token handle the broker gave.
    unsafe { CloseHandle(token) };
    r.map_err(|e| (e, SE_ERR_ACCESSDENIED))
}

/// The work of the three functions: `Ok(process information)` or `Err((last error, hInstApp))`.
fn execute(verb: Option<&[u16]>, file: &[u16], params: Option<&[u16]>, dir: Option<&[u16]>, show: i32) -> Result<ProcessInformation, (u32, usize)> {
    let t = gate(verb, file, dir)?;
    let elevated = caller_elevated();
    if t.runas && !elevated {
        return start_elevated(&t, params, dir, show);
    }
    match start(&t, params, dir, show, 0) {
        Ok(pi) => Ok(pi),
        // The manifest asks for administrator rights: ShellExecute asks the user.
        Err(ERROR_ELEVATION_REQUIRED) if !elevated => start_elevated(&t, params, dir, show),
        Err(ERROR_FILE_NOT_FOUND) => Err((ERROR_FILE_NOT_FOUND, SE_ERR_FNF)),
        Err(ERROR_NO_TOKEN) => Err((ERROR_NO_TOKEN, SE_ERR_ACCESSDENIED)),
        Err(e) => Err((e, SE_ERR_ACCESSDENIED)),
    }
}

/// `ShellExecuteExW`: see the module. A `cbSize` other than the structure's is
/// `ERROR_ACCESS_DENIED`. On success `hInstApp` is above 32 and, with
/// `SEE_MASK_NOCLOSEPROCESS`, `hProcess` the new process; the last error is left alone.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ShellExecuteExW(info: *mut ShellExecuteInfo) -> i32 {
    if info.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_ACCESS_DENIED) };
        return 0;
    }
    // SAFETY: the caller's structure; its size is checked before any other field is read.
    unsafe {
        if (*info).size as usize != core::mem::size_of::<ShellExecuteInfo>() {
            SetLastError(ERROR_ACCESS_DENIED);
            return 0;
        }
        let i = &mut *info;
        i.process = 0;
        let Some(file) = units(i.file) else {
            i.inst_app = SE_ERR_FNF;
            SetLastError(ERROR_FILE_NOT_FOUND);
            return 0;
        };
        match execute(units(i.verb), file, units(i.params), units(i.dir), i.show) {
            Ok(pi) => {
                CloseHandle(pi.thread);
                if i.mask & SEE_MASK_NOCLOSEPROCESS != 0 {
                    i.process = pi.process;
                } else {
                    CloseHandle(pi.process);
                }
                i.inst_app = SUCCESS_INST;
                1
            }
            Err((error, inst)) => {
                i.inst_app = inst;
                SetLastError(error);
                0
            }
        }
    }
}

/// `ShellExecuteW`: a value above 32 when the program started, else the `SE_ERR_*` code
/// (2 missing file, 31 unknown verb, 5 declined consent).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ShellExecuteW(
    _hwnd: usize,
    verb: *const u16,
    file: *const u16,
    params: *const u16,
    dir: *const u16,
    show: i32,
) -> usize {
    // SAFETY: the caller's NUL-terminated strings.
    let (verb, file, params, dir) = unsafe { (units(verb), units(file), units(params), units(dir)) };
    let Some(file) = file else { return SE_ERR_FNF };
    match execute(verb, file, params, dir, show) {
        Ok(pi) => {
            // SAFETY: the handles CreateProcessW gave.
            unsafe {
                CloseHandle(pi.thread);
                CloseHandle(pi.process);
            }
            SUCCESS_INST
        }
        Err((error, inst)) => {
            // SAFETY: a plain call.
            unsafe { SetLastError(error) };
            inst
        }
    }
}

/// An ANSI argument in UTF-16 (`CP_ACP`).
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
    unsafe { ShellExecuteW(hwnd, ptr(&v), ptr(&f), ptr(&p), ptr(&d), show) }
}
