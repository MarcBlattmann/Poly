//! A small Windows installer for the front door's boot check (ADR 0230, boot
//! group `win-installer`). It does what an installer does on Windows, with
//! the documented calls only:
//!
//! 1. copies itself to `C:\Program Files\Poly Test App\poly-test-app.exe`;
//! 2. writes a shortcut to that program, with the argument `--app`, into the
//!    common Start menu (`C:\ProgramData\Microsoft\Windows\Start Menu\
//!    Programs\Poly Test App.lnk`), as [MS-SHLLINK] lays a shell link out
//!    (the header, a `LinkInfo` with the local path, the strings);
//! 3. registers the app under `HKLM\SOFTWARE\Microsoft\Windows\
//!    CurrentVersion\Uninstall\PolyTestApp` (Learn, "Uninstall Registry
//!    Key");
//! 4. starts a copy of itself with `--linger`, which waits a few seconds and
//!    ends: Poly's installer waits for what an installer started, too;
//! 5. ends with 0.
//!
//! Like Google's installer it asks for administrator rights itself: started
//! without them (`IsUserAnAdmin` is FALSE for the filtered token, ADR 0222),
//! it starts itself again with `ShellExecuteExW` and the verb `runas`, which
//! is Poly's consent prompt, waits for that copy and ends with its exit
//! code. If the user says no (`ERROR_CANCELLED`), it installs for the user
//! instead: `%LOCALAPPDATA%\Programs`, the user's Start menu (`%APPDATA%`),
//! `HKCU` (where `Program Files` or `HKLM` refuse it). With `--app` (the
//! installed app) it writes `poly-test-app.txt` next to itself and ends.
//!
//! A copy whose file name holds `admin` installs another app, "Poly Admin App" (its folder, program,
//! shortcut and key named after it), so one boot can install both (the boot test gives that copy a
//! `requireAdministrator` manifest: it is elevated before it runs and never asks itself).
//!
//! Every call is on Microsoft Learn (`GetModuleFileNameW`,
//! `GetCommandLineW`, `CreateDirectoryW`, `CopyFileW`, `CreateFileW`,
//! `WriteFile`, `CloseHandle`, `CreateProcessW`, `Sleep`,
//! `GetEnvironmentVariableW`, `GetStdHandle`, `WaitForSingleObject`,
//! `GetExitCodeProcess`, `RegCreateKeyExW`, `RegSetValueExW`, `RegCloseKey`,
//! `IsUserAnAdmin`, `ShellExecuteExW`); constants from `winnt.h`,
//! `winreg.h`, `fileapi.h`, `winbase.h`, `shellapi.h`, `winerror.h`.
#![no_std]
#![no_main]

use core::ffi::c_void;
use poly_wtest::utf16;

type HANDLE = *mut c_void;
type BOOL = i32;
type HKEY = *mut c_void;

/// `STARTUPINFOW` (processthreadsapi.h).
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

/// `PROCESS_INFORMATION` (processthreadsapi.h).
#[repr(C)]
struct ProcessInformation {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

const CREATE_ALWAYS: u32 = 2;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
/// winreg.h / winnt.h.
const HKEY_CURRENT_USER: HKEY = 0x8000_0001usize as HKEY;
const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002usize as HKEY;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const REG_SZ: u32 = 1;
/// winerror.h.
const ERROR_SUCCESS: i32 = 0;
const ERROR_CANCELLED: u32 = 1223;
/// shellapi.h, winbase.h, winuser.h.
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const INFINITE: u32 = 0xFFFF_FFFF;
const SW_SHOWNORMAL: i32 = 1;

/// `SHELLEXECUTEINFOW` (shellapi.h).
#[repr(C)]
struct ShellExecuteInfoW {
    size: u32,
    mask: u32,
    hwnd: HANDLE,
    verb: *const u16,
    file: *const u16,
    parameters: *const u16,
    directory: *const u16,
    show: i32,
    inst_app: HANDLE,
    id_list: *mut c_void,
    class: *const u16,
    key_class: HANDLE,
    hot_key: u32,
    icon_or_monitor: HANDLE,
    process: HANDLE,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleFileNameW(module: HANDLE, name: *mut u16, size: u32) -> u32;
    fn GetCommandLineW() -> *const u16;
    fn CreateDirectoryW(path: *const u16, security: *const c_void) -> BOOL;
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: BOOL) -> BOOL;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn WriteFile(file: HANDLE, buffer: *const u8, to_write: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit_handles: BOOL,
        flags: u32,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *const StartupInfoW,
        information: *mut ProcessInformation,
    ) -> BOOL;
    fn Sleep(milliseconds: u32);
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
    fn GetStdHandle(which: u32) -> HANDLE;
    fn GetLastError() -> u32;
    fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsUserAnAdmin() -> BOOL;
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: HKEY,
        sub_key: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        desired: u32,
        security: *const c_void,
        result: *mut HKEY,
        disposition: *mut u32,
    ) -> i32;
    fn RegSetValueExW(key: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    fn RegCloseKey(key: HKEY) -> i32;
}

/// A NUL-terminated UTF-16 string in a fixed buffer (no allocator here).
struct Wide {
    buf: [u16; 520],
    len: usize,
}

impl Wide {
    fn new() -> Wide {
        Wide { buf: [0; 520], len: 0 }
    }

    fn of(units: &[u16]) -> Wide {
        let mut w = Wide::new();
        w.push(units);
        w
    }

    fn push(&mut self, units: &[u16]) -> &mut Wide {
        let n = units.len().min(self.buf.len() - 1 - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&units[..n]);
        self.len += n;
        self.buf[self.len] = 0;
        self
    }

    fn units(&self) -> &[u16] {
        &self.buf[..self.len]
    }

    fn ptr(&self) -> *const u16 {
        self.buf.as_ptr()
    }
}

/// A line on standard output (the installer's log on Poly).
fn say(text: &[u8]) {
    // SAFETY: no preconditions.
    let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if out.is_null() || out == INVALID_HANDLE_VALUE {
        return;
    }
    let mut written = 0u32;
    // SAFETY: `text` is valid for its length; `written` is writable.
    unsafe {
        WriteFile(out, text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut());
        WriteFile(out, b"\n".as_ptr(), 1, &mut written, core::ptr::null_mut());
    }
}

/// This program's own path.
fn own_path() -> Wide {
    let mut w = Wide::new();
    // SAFETY: the buffer holds 519 units and a NUL.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), w.buf.as_mut_ptr(), 519) } as usize;
    w.len = n.min(519);
    w.buf[w.len] = 0;
    w
}

/// Whether the command line holds `arg`.
fn has_arg(arg: &[u16]) -> bool {
    // SAFETY: GetCommandLineW returns the process's NUL-terminated command line.
    let line = unsafe { GetCommandLineW() };
    if line.is_null() {
        return false;
    }
    let mut n = 0;
    // SAFETY: read up to the terminating NUL.
    while unsafe { *line.add(n) } != 0 && n < 32768 {
        n += 1;
    }
    // SAFETY: `n` units were just read.
    let units = unsafe { core::slice::from_raw_parts(line, n) };
    units.windows(arg.len()).any(|w| w == arg)
}

/// `%name%`'s value.
fn env(name: &[u16]) -> Wide {
    let key = Wide::of(name);
    let mut w = Wide::new();
    // SAFETY: valid strings; the buffer holds 519 units and a NUL.
    let n = unsafe { GetEnvironmentVariableW(key.ptr(), w.buf.as_mut_ptr(), 519) } as usize;
    w.len = if n < 519 { n } else { 0 };
    w.buf[w.len] = 0;
    w
}

/// Make the folder `path` and its parents (ignoring those that are there).
fn make_dirs(path: &[u16]) {
    for (i, &c) in path.iter().enumerate() {
        if (c == u16::from(b'\\') && i > 2) || i + 1 == path.len() {
            let end = if c == u16::from(b'\\') { i } else { i + 1 };
            let dir = Wide::of(&path[..end]);
            // SAFETY: a valid NUL-terminated path.
            unsafe { CreateDirectoryW(dir.ptr(), core::ptr::null()) };
        }
    }
}

/// Write `bytes` to the new file `path`.
fn write_file(path: &Wide, bytes: &[u8]) -> bool {
    // SAFETY: a valid NUL-terminated path.
    let h = unsafe {
        CreateFileW(path.ptr(), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut written = 0u32;
    // SAFETY: `bytes` is valid for its length; `h` is the file just opened.
    let ok = unsafe { WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut written, core::ptr::null_mut()) } != 0;
    // SAFETY: closed once.
    unsafe { CloseHandle(h) };
    ok && written as usize == bytes.len()
}

/// A byte buffer for the shortcut.
struct Bytes {
    buf: [u8; 2048],
    len: usize,
}

impl Bytes {
    fn put(&mut self, b: &[u8]) {
        let n = b.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
        self.len += n;
    }

    fn u16(&mut self, v: u16) {
        self.put(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }

    fn wide(&mut self, units: &[u16]) {
        for &u in units {
            self.u16(u);
        }
    }
}

/// A shell link to `target` with `args` and `description` ([MS-SHLLINK]
/// 2.1, 2.3, 2.4): `HasLinkInfo`, `HasName`, `HasArguments`, `IsUnicode`.
fn shortcut(target: &[u16], args: &[u16], description: &[u16]) -> Bytes {
    let mut b = Bytes { buf: [0; 2048], len: 0 };
    b.u32(0x4C);
    b.put(&[0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    b.u32(0x2 | 0x4 | 0x20 | 0x80);
    b.u32(FILE_ATTRIBUTE_NORMAL);
    b.put(&[0; 28]);
    b.u32(0); // IconIndex
    b.u32(1); // SW_SHOWNORMAL
    b.put(&[0; 12]);
    // LinkInfo, header 0x24: VolumeID, LocalBasePath (code page: '?' for
    // other characters), CommonPathSuffix (empty), LocalBasePathUnicode,
    // CommonPathSuffixUnicode (empty).
    let volume_at = 0x24u32;
    let volume_len = 17u32;
    let base_at = volume_at + volume_len;
    let suffix_at = base_at + target.len() as u32 + 1;
    let base_w = suffix_at + 1 + ((suffix_at + 1) & 1);
    let suffix_w = base_w + 2 * (target.len() as u32 + 1);
    let size = suffix_w + 2;
    for v in [size, 0x24, 1, volume_at, base_at, 0, suffix_at, base_w, suffix_w] {
        b.u32(v);
    }
    b.u32(0x11);
    b.u32(3); // DRIVE_FIXED
    b.u32(0x1234_5678);
    b.u32(0x10);
    b.put(&[0]);
    for &u in target {
        b.put(&[if u < 0x80 { u as u8 } else { b'?' }]);
    }
    b.put(&[0, 0]);
    if (suffix_at + 1) & 1 == 1 {
        b.put(&[0]);
    }
    b.wide(target);
    b.u16(0);
    b.u16(0);
    // StringData: NAME_STRING, COMMAND_LINE_ARGUMENTS.
    for s in [description, args] {
        b.u16(s.len() as u16);
        b.wide(s);
    }
    // The terminal block.
    b.u32(0);
    b
}

/// Set the text value `name` of `key`.
fn set(key: HKEY, name: &[u16], value: &[u16]) {
    let n = Wide::of(name);
    let v = Wide::of(value);
    // SAFETY: valid strings; the data is the value with its NUL.
    unsafe { RegSetValueExW(key, n.ptr(), 0, REG_SZ, v.ptr().cast(), 2 * (v.len as u32 + 1)) };
}

/// The names of what is installed: the app's (folder, shortcut, `DisplayName`), its program's
/// and its `Uninstall` key's.
struct Names {
    app: &'static [u16],
    program: &'static [u16],
    key: &'static [u16],
}

/// The names this copy installs under: "Poly Admin App" when its file name holds `admin`.
fn names() -> Names {
    let me = own_path();
    let file = &me.units()[me.units().iter().rposition(|&c| c == u16::from(b'\\')).map_or(0, |i| i + 1)..];
    let admin = file.windows(5).any(|w| w.iter().zip(b"admin").all(|(&c, &a)| c == u16::from(a) || c == u16::from(a.to_ascii_uppercase())));
    if admin {
        Names { app: utf16!("Poly Admin App"), program: utf16!("poly-admin-app.exe"), key: utf16!("PolyAdminApp") }
    } else {
        Names { app: utf16!("Poly Test App"), program: utf16!("poly-test-app.exe"), key: utf16!("PolyTestApp") }
    }
}

/// Install; whether everything was written.
fn install() -> bool {
    let me = own_path();
    let n = names();
    // Machine-wide first; for the user if that is refused.
    let mut machine = true;
    let mut dir = Wide::of(utf16!("C:\\Program Files\\"));
    dir.push(n.app);
    make_dirs(dir.units());
    let mut program = Wide::of(dir.units());
    program.push(utf16!("\\")).push(n.program);
    // SAFETY: valid NUL-terminated paths.
    if unsafe { CopyFileW(me.ptr(), program.ptr(), 0) } == 0 {
        machine = false;
        dir = env(utf16!("LOCALAPPDATA"));
        dir.push(utf16!("\\Programs\\")).push(n.app);
        make_dirs(dir.units());
        program = Wide::of(dir.units());
        program.push(utf16!("\\")).push(n.program);
        // SAFETY: as above.
        if unsafe { CopyFileW(me.ptr(), program.ptr(), 0) } == 0 {
            // SAFETY: no preconditions.
            let e = unsafe { GetLastError() };
            say(b"poly-test-setup: the program could not be copied");
            let _ = e;
            return false;
        }
    }
    say(if machine { b"poly-test-setup: copied to Program Files" } else { b"poly-test-setup: copied for the user" });
    let mut menu = if machine {
        Wide::of(utf16!("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs"))
    } else {
        let mut m = env(utf16!("APPDATA"));
        m.push(utf16!("\\Microsoft\\Windows\\Start Menu\\Programs"));
        m
    };
    make_dirs(menu.units());
    menu.push(utf16!("\\")).push(n.app).push(utf16!(".lnk"));
    let link = shortcut(program.units(), utf16!("--app"), utf16!("A test app of Poly's installer"));
    if !write_file(&menu, &link.buf[..link.len]) {
        say(b"poly-test-setup: the shortcut could not be written");
        return false;
    }
    say(b"poly-test-setup: shortcut written");
    let mut key: HKEY = core::ptr::null_mut();
    let mut sub = Wide::of(utf16!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\"));
    sub.push(n.key);
    let root = if machine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
    // SAFETY: valid strings; `key` is writable.
    let made = unsafe {
        RegCreateKeyExW(root, sub.ptr(), 0, core::ptr::null(), 0, KEY_ALL_ACCESS, core::ptr::null(), &mut key, core::ptr::null_mut())
    };
    if made != ERROR_SUCCESS {
        say(b"poly-test-setup: the Uninstall key could not be made");
        return false;
    }
    let mut uninstall = Wide::of(utf16!("\""));
    uninstall.push(program.units()).push(utf16!("\" --uninstall"));
    set(key, utf16!("DisplayName"), n.app);
    set(key, utf16!("DisplayVersion"), utf16!("1.0"));
    set(key, utf16!("Publisher"), utf16!("Poly"));
    set(key, utf16!("UninstallString"), uninstall.units());
    set(key, utf16!("InstallLocation"), dir.units());
    // SAFETY: the key just made, closed once.
    unsafe { RegCloseKey(key) };
    say(b"poly-test-setup: registered");
    true
}

/// Start a copy of this program with `--linger`.
fn start_lingering() {
    let me = own_path();
    let mut line = Wide::of(utf16!("\""));
    line.push(me.units()).push(utf16!("\" --linger"));
    // SAFETY: zeroed is a valid STARTUPINFOW (integers and null pointers).
    let mut si: StartupInfoW = unsafe { core::mem::zeroed() };
    si.cb = core::mem::size_of::<StartupInfoW>() as u32;
    let mut pi = ProcessInformation { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), process_id: 0, thread_id: 0 };
    // SAFETY: valid strings (the command line writable), STARTUPINFOW and
    // PROCESS_INFORMATION.
    let ok = unsafe {
        CreateProcessW(
            me.ptr(),
            line.buf.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok != 0 {
        // SAFETY: the handles CreateProcessW returned, closed once.
        unsafe {
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
        say(b"poly-test-setup: started a helper that lingers");
    } else {
        say(b"poly-test-setup: the helper could not be started");
    }
}

/// Start this program again with the verb `runas` and wait for it: its exit
/// code, or the error `ShellExecuteExW` failed with.
fn run_elevated() -> Result<u32, u32> {
    let me = own_path();
    let verb = Wide::of(utf16!("runas"));
    let params = Wide::of(utf16!("--elevated"));
    let mut info = ShellExecuteInfoW {
        size: core::mem::size_of::<ShellExecuteInfoW>() as u32,
        mask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: core::ptr::null_mut(),
        verb: verb.ptr(),
        file: me.ptr(),
        parameters: params.ptr(),
        directory: core::ptr::null(),
        show: SW_SHOWNORMAL,
        inst_app: core::ptr::null_mut(),
        id_list: core::ptr::null_mut(),
        class: core::ptr::null(),
        key_class: core::ptr::null_mut(),
        hot_key: 0,
        icon_or_monitor: core::ptr::null_mut(),
        process: core::ptr::null_mut(),
    };
    say(b"poly-test-setup: asking for administrator rights (runas)");
    // SAFETY: a valid SHELLEXECUTEINFOW whose strings outlive the call.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: no preconditions.
        return Err(unsafe { GetLastError() });
    }
    if info.process.is_null() {
        return Ok(0);
    }
    let mut code = 1u32;
    // SAFETY: the process handle ShellExecuteExW returned (SEE_MASK_NOCLOSEPROCESS), closed once.
    unsafe {
        WaitForSingleObject(info.process, INFINITE);
        GetExitCodeProcess(info.process, &mut code);
        CloseHandle(info.process);
    }
    Ok(code)
}

#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    if has_arg(utf16!("--linger")) {
        // SAFETY: no preconditions.
        unsafe { Sleep(4000) };
        say(b"poly-test-setup: the helper ends");
        poly_wtest::exit(0);
    }
    if has_arg(utf16!("--app")) {
        let me = own_path();
        let cut = me.units().iter().rposition(|&c| c == u16::from(b'\\')).map_or(0, |i| i + 1);
        let mut marker = Wide::of(&me.units()[..cut]);
        marker.push(utf16!("poly-test-app.txt"));
        let ok = write_file(&marker, b"Poly Test App ran\r\n");
        say(b"poly-test-app: started from its shortcut");
        poly_wtest::exit(if ok { 0 } else { 1 });
    }
    if has_arg(utf16!("--uninstall")) {
        say(b"poly-test-app: no uninstaller in this test program");
        poly_wtest::exit(1);
    }
    // Administrator rights, asked for as Google's installer asks: `runas`.
    // A copy started elevated never asks again (no loop if the check disagrees).
    // SAFETY: no preconditions.
    if unsafe { IsUserAnAdmin() } == 0 && !has_arg(utf16!("--elevated")) {
        match run_elevated() {
            Ok(code) => poly_wtest::exit(code),
            Err(ERROR_CANCELLED) => say(b"poly-test-setup: no administrator rights (declined); installing for the user"),
            Err(_) => say(b"poly-test-setup: no administrator rights; installing for the user"),
        }
    } else {
        say(b"poly-test-setup: running with administrator rights");
    }
    let ok = install();
    start_lingering();
    say(if ok { b"poly-test-setup: done" } else { b"poly-test-setup: failed" });
    poly_wtest::exit(if ok { 0 } else { 1 })
}
