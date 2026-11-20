//! `ShellExecuteExW` and `ShellExecuteW` (ADR 0223): a program runs itself; a document is
//! opened by the program its type names in the registry (`HKEY_CLASSES_ROOT\.ext` → a program
//! id → `shell\<verb>\command`); a folder, or a document whose type nothing in the registry
//! opens, goes to Poly's desktop, which opens it as Files would (when the caller allows a user
//! interface: with `SEE_MASK_FLAG_NO_UI` that is the sheet's failure). The desktop is asked
//! through win32k's link ([`desktop_request`]), as "show in folder" is. Sheet:
//! `specs/windows/shell32.dll/ShellExecuteExW.yaml`.

use crate::com::nul;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_kernelbase::shellitem as si;
use poly_win32k::proto::{Reply, Request};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disp: u32,
        flags: u32,
        tpl: usize,
    ) -> usize;
    fn CloseHandle(h: usize) -> i32;
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
    fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, size: u32) -> u32;
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegGetValueW(
        key: isize,
        sub: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "win32u", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "win32u", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyUserCall(req: *const u8, len: usize, out: *mut u8, cap: usize, out_len: *mut usize) -> u32;
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
pub struct ProcessInformation {
    pub process: usize,
    pub thread: usize,
    pub pid: u32,
    pub tid: u32,
}

/// `STARTUPINFOW` (`processthreadsapi.h`): 104 bytes on x64, 68 on x86; `cb` first.
#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
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
    reserved3: *mut u8,
    input: usize,
    output: usize,
    error: usize,
}

// `shellapi.h`.
const SEE_MASK_CLASSNAME: u32 = 0x1;
const SEE_MASK_CLASSKEY: u32 = 0x3;
const SEE_MASK_IDLIST: u32 = 0x4;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const SEE_MASK_FLAG_NO_UI: u32 = 0x400;
/// `SE_ERR_*` (`shellapi.h`): what `hInstApp` holds after a failure.
const SE_ERR_FNF: usize = 2;
const SE_ERR_PNF: usize = 3;
const SE_ERR_ACCESSDENIED: usize = 5;
const SE_ERR_OOM: usize = 8;
const SE_ERR_NOASSOC: usize = 31;
/// What `hInstApp` holds after a success: any value above 32 (Learn); Windows gives 42.
const SUCCESS_INST: usize = 42;
// `winerror.h`.
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_BAD_FORMAT: u32 = 11;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NO_ASSOCIATION: u32 = 1155;
/// The last error of a type nothing opens with no user interface allowed (the sheet's
/// `no_association`: `0x80004021`, as an error code).
const NO_ASSOCIATION_NO_UI: u32 = 0x8000_4021;
/// `HKEY_CLASSES_ROOT` (`winreg.h`), sign-extended as the header's cast makes it.
const HKEY_CLASSES_ROOT: isize = 0x8000_0000_u32 as i32 as isize;
/// `RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND` (`winreg.h`).
const RRF_STRINGS: u32 = 0x2 | 0x4 | 0x1000_0000;
const REG_EXPAND_SZ: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_SHARE_ALL: u32 = 0x7;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const INVALID_HANDLE_VALUE: usize = usize::MAX;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Ask Poly's desktop to open `path` as Files would (`show` false) or to show it selected in
/// its folder in Files (`show` true): win32k's link carries the request to the compositor
/// (`Request::ShellOpen`, ADR 0223). The Win32 error when it cannot (50: no desktop).
pub fn desktop_request(path: &[u16], show: bool) -> Result<(), u32> {
    poly_win32_rt::unimplemented_export_error(
        if show {
            "shell32: asks the desktop to show a file in Files"
        } else {
            "shell32: asks the desktop to open a file"
        },
        0,
        0,
    );
    let name = nul(path);
    // SAFETY: a NUL-terminated name; the handle is only read by win32k and closed here.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_ALL,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: a plain call.
        return Err(unsafe { GetLastError() });
    }
    let req = poly_win32k::wire::encode(&Request::ShellOpen { handle: h as u64, show });
    let mut out = [0u8; 64];
    let mut len = 0usize;
    // SAFETY: our buffers, sized as passed; the handle is ours.
    let reply = unsafe {
        let s = PolyUserCall(req.as_ptr(), req.len(), out.as_mut_ptr(), out.len(), &mut len);
        CloseHandle(h);
        if s != 0 { None } else { poly_win32k::wire::decode::<Reply>(&out[..len.min(out.len())]) }
    };
    match reply {
        Some(Reply::Ok {}) => Ok(()),
        Some(Reply::Error { code }) => Err(code),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

/// A string value of `HKEY_CLASSES_ROOT\<key>` (`None` for the default value), without its NUL,
/// with `%variables%` of a `REG_EXPAND_SZ` value expanded.
fn class_value(key: &[u16], value: Option<&str>) -> Option<Vec<u16>> {
    reg_value(HKEY_CLASSES_ROOT, key, value)
}

/// `HKEY_LOCAL_MACHINE` (`winreg.h`).
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;

/// The program the machine's `App Paths` entry of the bare file name `file` names: the default
/// value of `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\<file>`, without quotes
/// (`specs/windows/shlwapi.dll/AssocQueryStringW.yaml`, rule `shellexecute_app_paths`: the entry of
/// the machine is used, the user's is not). `None` for a name with a directory part or no entry.
fn app_path(file: &[u16]) -> Option<Vec<u16>> {
    if file.is_empty() || file.iter().any(|&c| c == u16::from(b'\\') || c == u16::from(b'/') || c == u16::from(b':')) {
        return None;
    }
    let key = key_path(&[&w("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths"), file]);
    let v = reg_value(HKEY_LOCAL_MACHINE, &key, None)?;
    let q = u16::from(b'"');
    let v: Vec<u16> = v.into_iter().filter(|&c| c != q).collect();
    (!v.is_empty()).then_some(v)
}

/// A string value of `<root>\<key>` (`None` for the default value), without its NUL, with
/// `%variables%` of a `REG_EXPAND_SZ` value expanded.
fn reg_value(root: isize, key: &[u16], value: Option<&str>) -> Option<Vec<u16>> {
    let key = nul(key);
    let value = value.map(|v| nul(&w(v)));
    let value_ptr = value.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
    let mut buf = alloc::vec![0u8; 2048];
    let mut size = buf.len() as u32;
    let mut kind = 0u32;
    // SAFETY: NUL-terminated names; the buffer and its size.
    let r = unsafe { RegGetValueW(root, key.as_ptr(), value_ptr, RRF_STRINGS, &mut kind, buf.as_mut_ptr(), &mut size) };
    if r != 0 {
        return None;
    }
    let text: Vec<u16> = buf[..size as usize]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    if kind != REG_EXPAND_SZ {
        return Some(text);
    }
    let src = nul(&text);
    let mut out = alloc::vec![0u16; 4096];
    // SAFETY: a NUL-terminated string and a buffer of its size.
    let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), out.as_mut_ptr(), out.len() as u32) } as usize;
    if n == 0 || n > out.len() {
        return Some(text);
    }
    out.truncate(n - 1);
    Some(out)
}

/// `a` + `\` + `b` as a registry path.
fn key_path(parts: &[&[u16]]) -> Vec<u16> {
    let mut k = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            k.push(u16::from(b'\\'));
        }
        k.extend_from_slice(p);
    }
    k
}

/// What the registry says about a document's type.
enum Association {
    /// The command of the verb.
    Command(Vec<u16>),
    /// The type is registered, but not for this verb.
    NoVerb,
    /// Nothing is registered for the type.
    None,
}

/// The command `HKEY_CLASSES_ROOT` gives `verb` (`None`: the type's default verb, else `open`)
/// of the class `class` (an extension with its dot, or a program id).
fn association(class: &[u16], verb: Option<&[u16]>) -> Association {
    let shell = w("shell");
    let command = w("command");
    let program = if class.first() == Some(&u16::from(b'.')) {
        class_value(class, None).filter(|p| !p.is_empty())
    } else {
        Some(class.to_vec())
    };
    let mut registered = program.is_some();
    for key in program.iter().map(Vec::as_slice).chain(core::iter::once(class)) {
        let default_verb = class_value(&key_path(&[key, &shell]), None).filter(|v| !v.is_empty());
        if default_verb.is_some() {
            registered = true;
        }
        let verb = match verb {
            Some(v) => v.to_vec(),
            None => default_verb.unwrap_or_else(|| w("open")),
        };
        if let Some(cmd) = class_value(&key_path(&[key, &shell, &verb, &command]), None) {
            return Association::Command(cmd);
        }
    }
    if registered { Association::NoVerb } else { Association::None }
}

/// Start `command` (a command line) with `app` (NULL: the first word), in `dir` (NULL: ours).
fn start(app: Option<&[u16]>, command: &[u16], dir: Option<&[u16]>, show: i32) -> Result<ProcessInformation, u32> {
    let app = app.map(nul);
    let dir = dir.map(nul);
    let mut cmd = nul(command);
    // SAFETY: a zeroed STARTUPINFOW with its size and the show state.
    let mut si: StartupInfo = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<StartupInfo>() as u32;
    si.flags = 0x1; // STARTF_USESHOWWINDOW
    si.show = show as u16;
    let mut pi = ProcessInformation { process: 0, thread: 0, pid: 0, tid: 0 };
    // SAFETY: NUL-terminated strings (the command line writable), the two structures.
    let ok = unsafe {
        CreateProcessW(
            app.as_ref().map_or(core::ptr::null(), |a| a.as_ptr()),
            cmd.as_mut_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            0,
            0,
            core::ptr::null_mut(),
            dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()),
            (&mut si as *mut StartupInfo).cast(),
            &mut pi,
        )
    };
    // SAFETY: a plain call.
    if ok == 0 { Err(unsafe { GetLastError() }) } else { Ok(pi) }
}

/// The `hInstApp` of a failure with Win32 error `e`.
fn inst_of(e: u32) -> usize {
    match e {
        ERROR_FILE_NOT_FOUND => SE_ERR_FNF,
        ERROR_PATH_NOT_FOUND => SE_ERR_PNF,
        ERROR_ACCESS_DENIED => SE_ERR_ACCESSDENIED,
        ERROR_NOT_ENOUGH_MEMORY => SE_ERR_OOM,
        ERROR_BAD_FORMAT => 11,
        _ => SE_ERR_NOASSOC,
    }
}

/// The full path of `file`: a full path as it is; a relative one in `dir` if it is there, else
/// in the current directory.
fn full_path(file: &[u16], dir: Option<&[u16]>) -> Vec<u16> {
    if matches!(si::parse(file), Ok(si::Parsed::Path(_))) {
        return file.to_vec();
    }
    if let Some(d) = dir {
        let p = si::join(d, file);
        if crate::item::exists(&p).is_some() {
            return p;
        }
    }
    let name = nul(file);
    let mut buf = alloc::vec![0u16; 1024];
    // SAFETY: a NUL-terminated name and a buffer of its size.
    let n = unsafe { GetFullPathNameW(name.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), core::ptr::null_mut()) };
    if n == 0 || n as usize >= buf.len() {
        return file.to_vec();
    }
    buf.truncate(n as usize);
    buf
}

/// What [`execute`] did: the process (if one was started and the caller keeps it), or the
/// error with the `hInstApp` value that goes with it.
type Executed = Result<usize, (u32, usize)>;

fn execute(info: &ShellExecuteInfo) -> Executed {
    // SAFETY: the caller's strings (NULL allowed) and item list.
    let (verb, file, params, dir, class, pidl) = unsafe {
        (
            Some(crate::com::wstr(info.verb)).filter(|v| !v.is_empty()),
            crate::com::wstr(info.file),
            crate::com::wstr(info.params),
            Some(crate::com::wstr(info.dir)).filter(|d| !d.is_empty()),
            if info.mask & SEE_MASK_CLASSKEY == SEE_MASK_CLASSNAME { Some(crate::com::wstr(info.class)) } else { None },
            if info.mask & SEE_MASK_IDLIST != 0 && !info.id_list.is_null() {
                crate::pidl_path(info.id_list)
            } else {
                None
            },
        )
    };
    let no_ui = info.mask & SEE_MASK_FLAG_NO_UI != 0;
    let from_pidl = pidl.is_some();
    let path = match pidl {
        Some(p) => p,
        None if file.is_empty() => return Err((ERROR_FILE_NOT_FOUND, SE_ERR_FNF)),
        None => full_path(file, dir),
    };
    // A bare program name that is not there: the machine's `App Paths` entry for it.
    let path = match crate::item::exists(&path) {
        None if !from_pidl => app_path(file).unwrap_or(path),
        _ => path,
    };
    let Some(folder) = crate::item::exists(&path) else { return Err((ERROR_FILE_NOT_FOUND, SE_ERR_FNF)) };
    // A working directory that is not there is ignored (the sheet's `directory`).
    let dir = dir.filter(|d| crate::item::exists(d) == Some(true));
    let is_open = |v: Option<&[u16]>| v.is_none_or(|v| si::same_path(v, &w("open")));
    if folder {
        if !is_open(verb) && !verb.is_some_and(|v| si::same_path(v, &w("explore"))) {
            return Err((ERROR_NO_ASSOCIATION, SE_ERR_NOASSOC));
        }
        return desktop_request(&path, false).map(|()| 0).map_err(|e| (e, SE_ERR_NOASSOC));
    }
    let show = info.show;
    if class.is_none() && si::is_program(&path) {
        // `runas` asks for administrator rights (ADR 0222, [`crate::elevate`]).
        let runas = verb.is_some_and(|v| si::same_path(v, &w("runas")));
        if !is_open(verb) && !runas {
            return Err((ERROR_NO_ASSOCIATION, SE_ERR_NOASSOC));
        }
        let cmd = si::quote_for_command_line(&path, params);
        let elevated = crate::elevate::caller_elevated();
        if runas && !elevated {
            return crate::elevate::start_elevated(&path, &cmd, dir, show).map(|pi| keep(info, pi));
        }
        return match start(Some(&path), &cmd, dir, show) {
            Ok(pi) => Ok(keep(info, pi)),
            // The manifest asks for administrator rights: the Shell asks the user.
            Err(crate::elevate::ERROR_ELEVATION_REQUIRED) if !elevated => {
                crate::elevate::start_elevated(&path, &cmd, dir, show).map(|pi| keep(info, pi))
            }
            Err(e) => Err((e, inst_of(e))),
        };
    }
    let class: Vec<u16> = match class {
        Some(c) => c.to_vec(),
        None => si::extension(&path).map(<[u16]>::to_vec).unwrap_or_default(),
    };
    match association(&class, verb) {
        Association::Command(template) => {
            let cmd = si::expand_command(&template, &path, params);
            start(None, &cmd, dir, show).map(|pi| keep(info, pi)).map_err(|e| (e, inst_of(e)))
        }
        Association::NoVerb => Err((ERROR_NO_ASSOCIATION, SE_ERR_NOASSOC)),
        // Nothing of Windows' opens it: Poly's desktop does, unless no user interface may be
        // shown (where Windows would ask the user which program to use).
        Association::None if no_ui || !is_open(verb) => Err((NO_ASSOCIATION_NO_UI, SE_ERR_ACCESSDENIED)),
        Association::None => {
            desktop_request(&path, false).map(|()| 0).map_err(|_| (NO_ASSOCIATION_NO_UI, SE_ERR_ACCESSDENIED))
        }
    }
}

/// The process handle the caller keeps (`SEE_MASK_NOCLOSEPROCESS`), the others closed; with
/// `SEE_MASK_WAITFORINPUTIDLE`, after a GUI program's start-up (ADR 0245, [`crate::idle`]).
fn keep(info: &ShellExecuteInfo, pi: ProcessInformation) -> usize {
    if info.mask & crate::idle::SEE_MASK_WAITFORINPUTIDLE != 0 {
        crate::idle::wait(pi.process);
    }
    // SAFETY: the new process's handles, ours to close.
    unsafe {
        CloseHandle(pi.thread);
        if info.mask & SEE_MASK_NOCLOSEPROCESS != 0 {
            return pi.process;
        }
        CloseHandle(pi.process);
    }
    0
}

/// `ShellExecuteExW(pExecInfo)`: TRUE when the file was started; `hInstApp` above 32 then,
/// else an `SE_ERR_*` value, with the last error set.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ShellExecuteExW(info: *mut ShellExecuteInfo) -> i32 {
    if info.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return 0;
    }
    // SAFETY: the caller's structure (its size checked before anything else is read).
    unsafe {
        if (*info).size as usize != core::mem::size_of::<ShellExecuteInfo>() {
            (*info).inst_app = SE_ERR_ACCESSDENIED;
            SetLastError(ERROR_ACCESS_DENIED);
            return 0;
        }
        (*info).process = 0;
        match execute(&*info) {
            Ok(process) => {
                (*info).inst_app = SUCCESS_INST;
                (*info).process = process;
                SetLastError(0);
                1
            }
            Err((e, inst)) => {
                (*info).inst_app = inst;
                SetLastError(e);
                0
            }
        }
    }
}

/// `ShellExecuteW(hwnd, lpOperation, lpFile, lpParameters, lpDirectory, nShowCmd)`:
/// `ShellExecuteExW` with those fields; its `hInstApp`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ShellExecuteW(
    hwnd: usize,
    verb: *const u16,
    file: *const u16,
    params: *const u16,
    dir: *const u16,
    show: i32,
) -> usize {
    let mut info = ShellExecuteInfo {
        size: core::mem::size_of::<ShellExecuteInfo>() as u32,
        mask: 0,
        hwnd,
        verb,
        file,
        params,
        dir,
        show,
        inst_app: 0,
        id_list: core::ptr::null(),
        class: core::ptr::null(),
        hkey_class: 0,
        hot_key: 0,
        icon: 0,
        process: 0,
    };
    // SAFETY: a structure of ours with the caller's strings.
    unsafe { ShellExecuteExW(&mut info) };
    info.inst_app
}
