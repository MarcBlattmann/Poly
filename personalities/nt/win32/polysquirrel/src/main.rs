//! `polysquirrel.exe`: Poly installs a Squirrel app itself (the owner's decision for batch W6,
//! `docs/windows/apps-survey.md`: no .NET Framework; ADR 0253).
//!
//! ```text
//! polysquirrel SETUP.EXE           install the app the Squirrel installer carries, then start it
//! polysquirrel --uninstall ID      remove what an install of ID made
//! ```
//!
//! What goes where is `poly_squirrel`'s ([`poly_squirrel::plan`]): the package from the
//! installer's `DATA` resource, the app in `%LOCALAPPDATA%\<id>\app-<version>`, `Update.exe` beside
//! it, the package and `RELEASES` in `packages`, a Start menu and a desktop shortcut made with the
//! shell's `ShellLink` object, and the per-user `Uninstall` key whose `UninstallString` runs this
//! program again. Poly's installer window starts it for a Squirrel installer (ADR 0230's front
//! door) and turns the new shortcuts and key into a launcher entry and an app that Files can
//! uninstall.
//!
//! It writes one line per step to its standard output (`polysquirrel: ...`) and ends with 0, or
//! with 1 after a line `polysquirrel: failed: <why>`.
//!
//! Every call is on Microsoft Learn (`GetCommandLineW`, `CreateFileW`, `GetFileSizeEx`,
//! `CreateFileMappingW`, `MapViewOfFile`, `UnmapViewOfFile`, `CloseHandle`, `WriteFile`,
//! `CreateDirectoryW`, `GetLastError`, `GetStdHandle`, `ExitProcess`, `CreateProcessW`,
//! `FindFirstFileW`, `FindNextFileW`, `FindClose`, `DeleteFileW`, `RemoveDirectoryW`,
//! `SetFileAttributesW`, `SHGetFolderPathW`, `CoInitializeEx`, `CoCreateInstance`,
//! `IShellLinkW`, `IPersistFile`, `RegCreateKeyExW`, `RegSetValueExW`, `RegGetValueW`,
//! `RegDeleteKeyW`, `RegCloseKey`); the layouts and constants are the public SDK headers'
//! (`processthreadsapi.h`, `minwinbase.h`, `shlobj_core.h`, `shobjidl_core.h`, `objidl.h`,
//! `winreg.h`, `winnt.h`, `winerror.h`).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_squirrel::{Places, Plan, Value};

poly_win32_rt::process_heap_allocator!();

type Handle = *mut c_void;
const INVALID_HANDLE: Handle = -1isize as Handle;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: Handle) -> Handle;
    fn GetFileSizeEx(file: Handle, size: *mut i64) -> i32;
    fn CreateFileMappingW(file: Handle, sa: *mut c_void, protect: u32, high: u32, low: u32, name: *const u16) -> Handle;
    fn MapViewOfFile(mapping: Handle, access: u32, high: u32, low: u32, bytes: usize) -> *mut u8;
    fn UnmapViewOfFile(base: *const u8) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn WriteFile(file: Handle, buffer: *const u8, to_write: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> i32;
    fn GetLastError() -> u32;
    fn GetStdHandle(which: u32) -> Handle;
    fn ExitProcess(code: u32) -> !;
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *mut c_void,
        thread_attributes: *mut c_void,
        inherit: i32,
        flags: u32,
        environment: *mut c_void,
        directory: *const u16,
        startup: *const StartupInfoW,
        info: *mut ProcessInformation,
    ) -> i32;
    fn FindFirstFileW(pattern: *const u16, data: *mut FindData) -> Handle;
    fn FindNextFileW(find: Handle, data: *mut FindData) -> i32;
    fn FindClose(find: Handle) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "shell32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "shell32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SHGetFolderPathW(hwnd: Handle, csidl: i32, token: Handle, flags: u32, path: *mut u16) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, context: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: isize,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        access: u32,
        sa: *mut c_void,
        out: *mut isize,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> i32;
    fn RegDeleteKeyW(key: isize, sub: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

/// `STARTUPINFOW` (processthreadsapi.h).
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    numbers: [u32; 7],
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std: [Handle; 3],
}

/// `PROCESS_INFORMATION` (processthreadsapi.h).
#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

/// `WIN32_FIND_DATAW` (minwinbase.h).
#[repr(C)]
struct FindData {
    attributes: u32,
    times_and_sizes: [u32; 10],
    name: [u16; 260],
    short_name: [u16; 14],
    rest: [u32; 3],
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

/// `CLSID_ShellLink`, `IID_IShellLinkW` (shobjidl_core.h), `IID_IPersistFile` (objidl.h).
const CLSID_SHELL_LINK: Guid = Guid(0x0002_1401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ISHELL_LINK_W: Guid = Guid(0x0002_14F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPERSIST_FILE: Guid = Guid(0x0000_010B, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const OPEN_EXISTING: u32 = 3;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const PAGE_READONLY: u32 = 2;
const FILE_MAP_READ: u32 = 4;
const ERROR_ALREADY_EXISTS: u32 = 183;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const REG_SZ: u32 = 1;
const REG_DWORD: u32 = 4;
const RRF_RT_REG_SZ: u32 = 2;
const COINIT_APARTMENTTHREADED: u32 = 2;
const CLSCTX_INPROC_SERVER: u32 = 1;
/// `CSIDL_*` (shlobj_core.h).
const CSIDL_PROGRAMS: i32 = 0x02;
const CSIDL_DESKTOPDIRECTORY: i32 = 0x10;
const CSIDL_LOCAL_APPDATA: i32 = 0x1C;
/// The largest package taken (a full app is a few hundred MB).
const MAX_PACKAGE: usize = 2 << 30;

/// `text` as UTF-16 with a NUL.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// Write `parts` and a line end to the standard output.
fn say(parts: &[&str]) {
    // SAFETY: no preconditions.
    let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if out.is_null() || out == INVALID_HANDLE {
        return;
    }
    let mut written = 0u32;
    for p in parts.iter().chain([&"\n"]) {
        // SAFETY: `p` is valid for its length.
        unsafe { WriteFile(out, p.as_ptr(), p.len() as u32, &mut written, core::ptr::null_mut()) };
    }
}

fn fail(why: &str) -> ! {
    say(&["polysquirrel: failed: ", why]);
    // SAFETY: ends the process.
    unsafe { ExitProcess(1) }
}

/// A Windows error code as text.
fn code(e: u32) -> String {
    alloc::format!("{e}")
}

/// A file mapped for reading, unmapped when dropped.
struct Mapped {
    base: *const u8,
    len: usize,
}

impl Mapped {
    fn open(path: &[u16]) -> Result<Mapped, String> {
        // SAFETY: a NUL-terminated path.
        let file = unsafe {
            CreateFileW(path.as_ptr(), GENERIC_READ, FILE_SHARE_READ, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut())
        };
        if file == INVALID_HANDLE {
            // SAFETY: no preconditions.
            return Err(alloc::format!("the installer cannot be opened ({})", code(unsafe { GetLastError() })));
        }
        let mut size = 0i64;
        // SAFETY: the open file.
        let ok = unsafe { GetFileSizeEx(file, &mut size) } != 0;
        // SAFETY: the open file; a read-only mapping of all of it.
        let mapping = unsafe { CreateFileMappingW(file, core::ptr::null_mut(), PAGE_READONLY, 0, 0, core::ptr::null()) };
        // SAFETY: the file handle, closed once (the mapping keeps the file).
        unsafe { CloseHandle(file) };
        if !ok || size <= 0 || mapping.is_null() {
            return Err("the installer cannot be read".into());
        }
        // SAFETY: the mapping just made; the whole file.
        let base = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0) };
        // SAFETY: the mapping handle, closed once (the view keeps it).
        unsafe { CloseHandle(mapping) };
        if base.is_null() {
            return Err("the installer cannot be mapped".into());
        }
        Ok(Mapped { base, len: size as usize })
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: a view of `len` readable bytes, alive until drop.
        unsafe { core::slice::from_raw_parts(self.base, self.len) }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: the view mapped in `open`, unmapped once.
        unsafe { UnmapViewOfFile(self.base) };
    }
}

/// A shell folder's path.
fn folder(csidl: i32) -> Result<String, String> {
    let mut buf = [0u16; 260];
    // SAFETY: a MAX_PATH buffer.
    let hr = unsafe { SHGetFolderPathW(core::ptr::null_mut(), csidl, core::ptr::null_mut(), 0, buf.as_mut_ptr()) };
    if hr < 0 {
        return Err(alloc::format!("SHGetFolderPathW({csidl:#x}) failed ({hr:#x})"));
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..n]))
}

fn places() -> Result<Places, String> {
    Ok(Places {
        local_app_data: folder(CSIDL_LOCAL_APPDATA)?,
        programs: folder(CSIDL_PROGRAMS)?,
        desktop: folder(CSIDL_DESKTOPDIRECTORY)?,
    })
}

/// Make `path` and the folders above it.
fn make_folders(path: &str) -> Result<(), String> {
    let with_self = alloc::format!("{path}\\x");
    for f in poly_squirrel::parent_folders(&with_self) {
        let w = wide(f);
        // SAFETY: a NUL-terminated path.
        if unsafe { CreateDirectoryW(w.as_ptr(), core::ptr::null_mut()) } == 0 {
            // SAFETY: no preconditions.
            let e = unsafe { GetLastError() };
            if e != ERROR_ALREADY_EXISTS {
                return Err(alloc::format!("{f} cannot be made ({e})"));
            }
        }
    }
    Ok(())
}

/// Write `data` to the file `path`, making its folders.
fn write_file(path: &str, data: &[u8]) -> Result<(), String> {
    if let Some((dir, _)) = path.rsplit_once('\\') {
        make_folders(dir)?;
    }
    let w = wide(path);
    // SAFETY: a NUL-terminated path.
    let f = unsafe {
        CreateFileW(w.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut())
    };
    if f == INVALID_HANDLE {
        // SAFETY: no preconditions.
        return Err(alloc::format!("{path} cannot be written ({})", unsafe { GetLastError() }));
    }
    let mut ok = true;
    for chunk in data.chunks(1 << 24) {
        let mut written = 0u32;
        // SAFETY: the open file and the chunk.
        ok &= unsafe { WriteFile(f, chunk.as_ptr(), chunk.len() as u32, &mut written, core::ptr::null_mut()) } != 0
            && written as usize == chunk.len();
    }
    // SAFETY: the file, closed once.
    unsafe { CloseHandle(f) };
    if ok { Ok(()) } else { Err(alloc::format!("{path} could not be written whole")) }
}

/// A COM object's method `index` of its vtable, as `T`.
///
/// # Safety
/// `object` is a live COM interface pointer whose vtable has a method of type `T` at `index`.
unsafe fn method<T: Copy>(object: *mut c_void, index: usize) -> T {
    // SAFETY: the caller's promise; a vtable is an array of function pointers.
    unsafe {
        let vtable = *(object as *const *const usize);
        core::mem::transmute_copy(&*vtable.add(index))
    }
}

/// A shortcut at `lnk` to `target`, started in `dir`, with `description`, through the shell's
/// `ShellLink` object (`IShellLinkW::SetPath` 20, `SetWorkingDirectory` 9, `SetDescription` 7,
/// `SetIconLocation` 17; `IPersistFile::Save` 6; `Release` 2).
fn shortcut(lnk: &str, target: &str, dir: &str, description: &str) -> Result<(), String> {
    if let Some((parent, _)) = lnk.rsplit_once('\\') {
        make_folders(parent)?;
    }
    type SetText = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
    type SetIcon = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type Query = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
    type Save = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    let mut link: *mut c_void = core::ptr::null_mut();
    // SAFETY: the class and interface ids; `link` receives the interface.
    let hr = unsafe { CoCreateInstance(&CLSID_SHELL_LINK, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut link) };
    if hr < 0 || link.is_null() {
        return Err(alloc::format!("no ShellLink object ({hr:#x})"));
    }
    let (t, d, s, l) = (wide(target), wide(dir), wide(description), wide(lnk));
    let mut file: *mut c_void = core::ptr::null_mut();
    // SAFETY: `link` is an IShellLinkW; the methods at these places of its vtable have these
    // types (shobjidl_core.h); the strings are NUL-terminated.
    let saved = unsafe {
        method::<SetText>(link, 20)(link, t.as_ptr());
        method::<SetText>(link, 9)(link, d.as_ptr());
        method::<SetText>(link, 7)(link, s.as_ptr());
        method::<SetIcon>(link, 17)(link, t.as_ptr(), 0);
        let mut hr = method::<Query>(link, 0)(link, &IID_IPERSIST_FILE, &mut file);
        if hr >= 0 && !file.is_null() {
            hr = method::<Save>(file, 6)(file, l.as_ptr(), 1);
            method::<Release>(file, 2)(file);
        }
        method::<Release>(link, 2)(link);
        hr
    };
    if saved < 0 { Err(alloc::format!("{lnk} could not be saved ({saved:#x})")) } else { Ok(()) }
}

/// Write the `Uninstall` key of the plan.
fn register(plan: &Plan) -> Result<(), String> {
    let sub = wide(&plan.key);
    let mut key = 0isize;
    // SAFETY: a NUL-terminated subkey; `key` receives the handle.
    let e = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, core::ptr::null(), 0, KEY_ALL_ACCESS, core::ptr::null_mut(), &mut key, core::ptr::null_mut())
    };
    if e != 0 {
        return Err(alloc::format!("the Uninstall key cannot be made ({e})"));
    }
    for (name, value) in &plan.values {
        let n = wide(name);
        let e = match value {
            Value::Text(t) => {
                let w = wide(t);
                // SAFETY: the open key; REG_SZ data with its NUL.
                unsafe { RegSetValueExW(key, n.as_ptr(), 0, REG_SZ, w.as_ptr().cast(), (w.len() * 2) as u32) }
            }
            Value::Number(v) => {
                // SAFETY: the open key; four bytes of REG_DWORD data.
                unsafe { RegSetValueExW(key, n.as_ptr(), 0, REG_DWORD, (v as *const u32).cast(), 4) }
            }
        };
        if e != 0 {
            // SAFETY: the key, closed once.
            unsafe { RegCloseKey(key) };
            return Err(alloc::format!("{name} cannot be set ({e})"));
        }
    }
    // SAFETY: the key, closed once.
    unsafe { RegCloseKey(key) };
    Ok(())
}

/// Start `program` in `dir` and do not wait for it.
fn start(program: &str, dir: &str) -> Result<(), String> {
    let app = wide(program);
    let mut line = wide(&alloc::format!("\"{program}\""));
    let d = wide(dir);
    // SAFETY: zeroed is a valid STARTUPINFOW.
    let mut si: StartupInfoW = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<StartupInfoW>() as u32;
    let mut pi = ProcessInformation { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), process_id: 0, thread_id: 0 };
    // SAFETY: NUL-terminated strings (the command line writable), STARTUPINFOW and
    // PROCESS_INFORMATION.
    let ok = unsafe {
        CreateProcessW(app.as_ptr(), line.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), 0, 0, core::ptr::null_mut(), d.as_ptr(), &si, &mut pi)
    };
    if ok == 0 {
        // SAFETY: no preconditions.
        return Err(alloc::format!("{program} did not start ({})", unsafe { GetLastError() }));
    }
    // SAFETY: the handles CreateProcessW returned, closed once.
    unsafe {
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
    }
    Ok(())
}

fn install(setup: &[u16]) -> Result<(), String> {
    let file = Mapped::open(setup)?;
    let data = poly_squirrel::bundle_resource(file.bytes()).ok_or("no Squirrel package in the installer")?;
    let outer = poly_zip::Archive::parse(data).map_err(|e| alloc::format!("the package bundle is damaged ({e:?})"))?;
    let bundle = poly_squirrel::bundle(&outer).ok_or("no Squirrel package in the installer")?;
    say(&["polysquirrel: unpacking ", &bundle.package.name]);
    let package = poly_zip::read(data, &bundle.package, MAX_PACKAGE).map_err(|e| alloc::format!("{}: {e:?}", bundle.package.name))?;
    let pkg = poly_zip::Archive::parse(&package).map_err(|e| alloc::format!("the package is damaged ({e:?})"))?;
    let spec_entry = poly_squirrel::spec_entry(&pkg).ok_or("the package has no .nuspec")?;
    let spec_text = poly_zip::read(&package, spec_entry, 1 << 20).map_err(|e| alloc::format!(".nuspec: {e:?}"))?;
    let spec = poly_squirrel::nuspec(&String::from_utf8_lossy(&spec_text)).ok_or("the .nuspec has no id or version")?;
    let plan = poly_squirrel::plan(&spec, &pkg, &places()?)?;
    say(&["polysquirrel: installing ", &spec.title, " ", &spec.version, " into ", &plan.app_dir]);
    for (index, path) in &plan.files {
        let entry = &pkg.entries[*index];
        let bytes = poly_zip::read(&package, entry, MAX_PACKAGE).map_err(|e| alloc::format!("{}: {e:?}", entry.name))?;
        write_file(path, &bytes)?;
    }
    if let Some(update) = &bundle.update {
        let bytes = poly_zip::read(data, update, MAX_PACKAGE).map_err(|e| alloc::format!("Update.exe: {e:?}"))?;
        write_file(&plan.update, &bytes)?;
    }
    if let Some(releases) = &bundle.releases {
        let bytes = poly_zip::read(data, releases, 1 << 20).map_err(|e| alloc::format!("RELEASES: {e:?}"))?;
        write_file(&alloc::format!("{}\\RELEASES", plan.packages), &bytes)?;
    }
    write_file(&alloc::format!("{}\\{}", plan.packages, bundle.package.name), &package)?;
    drop(package);
    // SAFETY: once, on this thread, before the shell objects are used.
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    for lnk in &plan.shortcuts {
        shortcut(lnk, &plan.program, &plan.app_dir, &spec.description)?;
    }
    register(&plan)?;
    say(&["polysquirrel: installed ", &spec.title, " ", &spec.version, "; starting ", &plan.program]);
    start(&plan.program, &plan.app_dir)
}

/// A text value of the key `sub` below `HKEY_CURRENT_USER`.
fn reg_text(sub: &[u16], name: &str) -> Option<String> {
    let n = wide(name);
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: NUL-terminated names and a buffer of `size` bytes.
    let e = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, sub.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, core::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if e != 0 {
        return None;
    }
    let len = (size as usize / 2).min(buf.len());
    let end = buf[..len].iter().position(|&c| c == 0).unwrap_or(len);
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Remove the folder `path` and everything in it.
fn remove_tree(path: &str) {
    let pattern = wide(&alloc::format!("{path}\\*"));
    // SAFETY: zeroed is a valid WIN32_FIND_DATAW.
    let mut data: FindData = unsafe { core::mem::zeroed() };
    // SAFETY: a NUL-terminated pattern and the structure.
    let find = unsafe { FindFirstFileW(pattern.as_ptr(), &mut data) };
    if find != INVALID_HANDLE {
        loop {
            let n = data.name.iter().position(|&c| c == 0).unwrap_or(data.name.len());
            let name = String::from_utf16_lossy(&data.name[..n]);
            if name != "." && name != ".." {
                let full = alloc::format!("{path}\\{name}");
                if data.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    remove_tree(&full);
                } else {
                    let w = wide(&full);
                    // SAFETY: NUL-terminated paths.
                    unsafe {
                        SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL);
                        DeleteFileW(w.as_ptr());
                    }
                }
            }
            // SAFETY: the search handle and the structure.
            if unsafe { FindNextFileW(find, &mut data) } == 0 {
                break;
            }
        }
        // SAFETY: the search handle, closed once.
        unsafe { FindClose(find) };
    }
    let w = wide(path);
    // SAFETY: a NUL-terminated path.
    unsafe { RemoveDirectoryW(w.as_ptr()) };
}

fn uninstall(id: &str) -> Result<(), String> {
    let key = poly_squirrel::uninstall_key(id);
    let sub = wide(&key);
    let title = reg_text(&sub, "DisplayName").ok_or("the app is not installed (no Uninstall key)")?;
    let publisher = reg_text(&sub, "Publisher").unwrap_or_default();
    let location = reg_text(&sub, "InstallLocation").unwrap_or_default();
    let places = places()?;
    // Only an install made here: the app's own folder in the user's local application data.
    let expected = alloc::format!("{}\\{}", places.local_app_data, poly_squirrel::file_name_safe(id));
    if !location.eq_ignore_ascii_case(&expected) {
        return Err(alloc::format!("the Uninstall key names {location}, not {expected}"));
    }
    say(&["polysquirrel: removing ", &title, " from ", &location]);
    remove_tree(&location);
    for lnk in poly_squirrel::shortcuts(&places, &title, &publisher) {
        let w = wide(&lnk);
        // SAFETY: a NUL-terminated path.
        unsafe { DeleteFileW(w.as_ptr()) };
        // The publisher's Start menu folder, if it is empty now.
        if let Some((dir, _)) = lnk.rsplit_once('\\')
            && dir.len() > places.programs.len()
        {
            let d = wide(dir);
            // SAFETY: a NUL-terminated path; fails if the folder is not empty.
            unsafe { RemoveDirectoryW(d.as_ptr()) };
        }
    }
    // SAFETY: a NUL-terminated subkey.
    let e = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, sub.as_ptr()) };
    if e != 0 {
        return Err(alloc::format!("the Uninstall key cannot be removed ({e})"));
    }
    say(&["polysquirrel: removed ", &title]);
    Ok(())
}

/// The arguments of the command line (Learn, "Parsing C++ command-line arguments", without the
/// backslash rules: paths and ids have no quotes in them).
fn arguments() -> Vec<String> {
    // SAFETY: the process's NUL-terminated command line.
    let line = unsafe {
        let p = GetCommandLineW();
        let mut n = 0;
        while !p.is_null() && *p.add(n) != 0 {
            n += 1;
        }
        if p.is_null() { &[][..] } else { core::slice::from_raw_parts(p, n) }
    };
    let text = String::from_utf16_lossy(line);
    let mut out = Vec::new();
    let (mut cur, mut quoted, mut has) = (String::new(), false, false);
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            ' ' | '\t' if !quoted => {
                if has {
                    out.push(core::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

#[unsafe(no_mangle)]
pub extern "system" fn polysquirrel_main() -> ! {
    let args = arguments();
    let result = match args.get(1..).unwrap_or(&[]) {
        [flag, id] if flag == "--uninstall" => uninstall(id),
        [setup] if !setup.starts_with("--") => install(&wide(setup)),
        _ => Err("usage: polysquirrel SETUP.EXE | polysquirrel --uninstall ID".into()),
    };
    match result {
        Ok(()) => {
            // SAFETY: ends the process.
            unsafe { ExitProcess(0) }
        }
        Err(why) => fail(&why),
    }
}
