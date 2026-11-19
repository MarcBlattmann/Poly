//! `cmd.exe` (ADR 0240): Poly's command processor, installed in `C:\Windows\System32`.
//!
//! The language, the built-ins and the program search are `poly_cmd` (host-tested); this file
//! only answers its [`Host`] with `kernel32` calls (prototypes from Microsoft Learn; constants
//! from `winbase.h`, `fileapi.h`, `processthreadsapi.h`, `winnls.h`).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_cmd::shell::{Host, Io, Kind, OpenMode, Shell};

poly_win32_rt::process_heap_allocator!();

type Handle = isize;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *const c_void,
    inherit: i32,
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *const u16,
    desktop: *const u16,
    title: *const u16,
    dims: [u32; 7],
    flags: u32,
    show_window: u16,
    reserved2_size: u16,
    reserved2: *const u8,
    std_input: Handle,
    std_output: Handle,
    std_error: Handle,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn ReadFile(file: Handle, buf: *mut u8, len: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SecurityAttributes,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn SetFilePointerEx(file: Handle, distance: i64, new: *mut i64, method: u32) -> i32;
    fn CreateProcessW(
        app: *const u16,
        cmdline: *mut u16,
        pa: *const c_void,
        ta: *const c_void,
        inherit: i32,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const StartupInfoW,
        pi: *mut ProcessInformation,
    ) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    fn GetCurrentDirectoryW(size: u32, buf: *mut u16) -> u32;
    fn SetCurrentDirectoryW(path: *const u16) -> i32;
    fn GetFullPathNameW(name: *const u16, size: u32, buf: *mut u16, file_part: *mut *mut u16) -> u32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn CreateDirectoryW(name: *const u16, sa: *const c_void) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
    fn WriteConsoleW(h: Handle, buf: *const u16, len: u32, written: *mut u32, reserved: *const c_void) -> i32;
    fn WideCharToMultiByte(
        cp: u32,
        flags: u32,
        wide: *const u16,
        wide_len: i32,
        out: *mut u8,
        out_len: i32,
        default: *const u8,
        used_default: *mut i32,
    ) -> i32;
    fn GetTickCount() -> u32;
    fn GetLastError() -> u32;
    fn ExitProcess(code: u32) -> !;
}

const STD_HANDLES: [u32; 3] = [-10i32 as u32, -11i32 as u32, -12i32 as u32];
const INVALID_HANDLE: Handle = -1;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
const SHARE_ALL: u32 = 7;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_END: u32 = 2;
const STARTF_USESTDHANDLES: u32 = 0x100;
const INFINITE: u32 = u32::MAX;
const CP_OEMCP: u32 = 1;

/// `text` as UTF-16 with a terminating NUL.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// A NUL-terminated UTF-16 string.
///
/// # Safety
/// `p` points to a NUL-terminated string.
unsafe fn from_wide(p: *const u16) -> String {
    let mut n = 0;
    // SAFETY: the caller's string, read up to its NUL.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: `n` units were just read.
    String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(p, n) })
}

/// A string a `kernel32` call writes into a buffer and returns the length of (or the size it
/// needs, when larger than the buffer).
fn sized(mut call: impl FnMut(*mut u16, u32) -> u32) -> Option<String> {
    let mut buf = vec![0u16; 260];
    loop {
        let n = call(buf.as_mut_ptr(), buf.len() as u32) as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(String::from_utf16_lossy(&buf[..n]));
        }
        buf.resize(n + 1, 0);
    }
}

struct Win {
    std: [Handle; 3],
    /// `/u`: output to files and pipes as UTF-16.
    unicode: bool,
}

impl Host for Win {
    type File = Handle;

    fn std_handle(&self, n: u8) -> Handle {
        self.std[n as usize]
    }

    fn write(&mut self, f: Handle, text: &str) {
        if text.is_empty() {
            return;
        }
        let w: Vec<u16> = text.encode_utf16().collect();
        let mut done = 0u32;
        let mut mode = 0u32;
        // SAFETY: a handle and a local.
        if unsafe { GetConsoleMode(f, &mut mode) } != 0 {
            // SAFETY: `w` holds `w.len()` units.
            unsafe { WriteConsoleW(f, w.as_ptr(), w.len() as u32, &mut done, core::ptr::null()) };
            return;
        }
        let bytes: Vec<u8> = if self.unicode {
            w.iter().flat_map(|u| u.to_le_bytes()).collect()
        } else {
            // SAFETY: `w` holds `w.len()` units; a size query.
            let n = unsafe {
                WideCharToMultiByte(
                    CP_OEMCP,
                    0,
                    w.as_ptr(),
                    w.len() as i32,
                    core::ptr::null_mut(),
                    0,
                    core::ptr::null(),
                    core::ptr::null_mut(),
                )
            };
            let mut b = vec![0u8; n.max(0) as usize];
            // SAFETY: `b` holds `n` bytes.
            let m = unsafe {
                WideCharToMultiByte(
                    CP_OEMCP,
                    0,
                    w.as_ptr(),
                    w.len() as i32,
                    b.as_mut_ptr(),
                    n,
                    core::ptr::null(),
                    core::ptr::null_mut(),
                )
            };
            if m <= 0 {
                text.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).collect()
            } else {
                b.truncate(m as usize);
                b
            }
        };
        // SAFETY: `bytes` holds its length.
        unsafe { WriteFile(f, bytes.as_ptr(), bytes.len() as u32, &mut done, core::ptr::null_mut()) };
    }

    fn read_line(&mut self, f: Handle) -> Option<String> {
        let mut line = Vec::new();
        loop {
            let mut b = 0u8;
            let mut n = 0u32;
            // SAFETY: one byte into a local.
            let ok = unsafe { ReadFile(f, &mut b, 1, &mut n, core::ptr::null_mut()) };
            if ok == 0 || n == 0 {
                if line.is_empty() {
                    return None;
                }
                break;
            }
            if b == b'\n' {
                break;
            }
            line.push(b);
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Some(match String::from_utf8(line) {
            Ok(s) => s,
            Err(e) => e.into_bytes().iter().map(|&b| char::from(b)).collect(),
        })
    }

    fn open(&mut self, path: &str, mode: OpenMode) -> Option<Handle> {
        let name = wide(path);
        // The redirected handles are given to programs, so they are inheritable.
        let sa = SecurityAttributes {
            length: size_of::<SecurityAttributes>() as u32,
            descriptor: core::ptr::null(),
            inherit: 1,
        };
        let (access, disposition) = match mode {
            OpenMode::Read => (GENERIC_READ, OPEN_EXISTING),
            OpenMode::Write => (GENERIC_WRITE, CREATE_ALWAYS),
            OpenMode::Append => (GENERIC_WRITE, OPEN_ALWAYS),
        };
        // SAFETY: a NUL-terminated name and a local structure.
        let h = unsafe { CreateFileW(name.as_ptr(), access, SHARE_ALL, &sa, disposition, FILE_ATTRIBUTE_NORMAL, 0) };
        if h == INVALID_HANDLE || h == 0 {
            return None;
        }
        if mode == OpenMode::Append {
            // SAFETY: a handle just opened; devices (NUL) ignore it.
            unsafe { SetFilePointerEx(h, 0, core::ptr::null_mut(), FILE_END) };
        }
        Some(h)
    }

    fn close(&mut self, f: Handle) {
        // SAFETY: a handle `open` returned.
        unsafe { CloseHandle(f) };
    }

    fn var(&self, name: &str) -> Option<String> {
        let n = wide(name);
        // SAFETY: a NUL-terminated name and a buffer of the size given.
        sized(|buf, size| unsafe { GetEnvironmentVariableW(n.as_ptr(), buf, size) })
    }

    fn set_var(&mut self, name: &str, value: Option<&str>) {
        let n = wide(name);
        let v = value.map(wide);
        // SAFETY: NUL-terminated strings (or null to remove).
        unsafe { SetEnvironmentVariableW(n.as_ptr(), v.as_ref().map_or(core::ptr::null(), |v| v.as_ptr())) };
    }

    fn vars(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        // SAFETY: plain call; the block ends with an empty string.
        let block = unsafe { GetEnvironmentStringsW() };
        if block.is_null() {
            return out;
        }
        let mut p = block as *const u16;
        loop {
            // SAFETY: inside the block, which ends with an empty string.
            let entry = unsafe { from_wide(p) };
            if entry.is_empty() {
                break;
            }
            // SAFETY: past this entry and its NUL, still inside the block.
            p = unsafe { p.add(entry.encode_utf16().count() + 1) };
            // A name may start with `=` (`=C:`): the separator is the next one.
            if let Some((i, _)) = entry.char_indices().skip(1).find(|&(_, c)| c == '=') {
                out.push((String::from(&entry[..i]), String::from(&entry[i + 1..])));
            }
        }
        // SAFETY: the block GetEnvironmentStringsW returned.
        unsafe { FreeEnvironmentStringsW(block) };
        out
    }

    fn current_dir(&self) -> String {
        // SAFETY: a buffer of the size given.
        sized(|buf, size| unsafe { GetCurrentDirectoryW(size, buf) }).unwrap_or_else(|| String::from("C:\\"))
    }

    fn set_current_dir(&mut self, path: &str) -> bool {
        let p = wide(path);
        // SAFETY: a NUL-terminated path.
        unsafe { SetCurrentDirectoryW(p.as_ptr()) != 0 }
    }

    fn full_path(&self, path: &str) -> String {
        let p = wide(path);
        // SAFETY: a NUL-terminated name and a buffer of the size given.
        sized(|buf, size| unsafe { GetFullPathNameW(p.as_ptr(), size, buf, core::ptr::null_mut()) })
            .unwrap_or_else(|| String::from(path))
    }

    fn kind(&self, path: &str) -> Option<Kind> {
        let p = wide(path);
        // SAFETY: a NUL-terminated path.
        let a = unsafe { GetFileAttributesW(p.as_ptr()) };
        if a == INVALID_FILE_ATTRIBUTES {
            None
        } else if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
            Some(Kind::Dir)
        } else {
            Some(Kind::File)
        }
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let h = self.open(path, OpenMode::Read)?;
        let mut data = Vec::new();
        let mut buf = vec![0u8; 0x10000];
        loop {
            let mut n = 0u32;
            // SAFETY: a buffer of its size.
            let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) };
            if ok == 0 || n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n as usize]);
        }
        self.close(h);
        Some(data)
    }

    fn make_dir(&mut self, path: &str) -> bool {
        let p = wide(path);
        // SAFETY: a NUL-terminated path, default security.
        unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null()) != 0 }
    }

    fn delete_file(&mut self, path: &str) -> bool {
        let p = wide(path);
        // SAFETY: a NUL-terminated path.
        unsafe { DeleteFileW(p.as_ptr()) != 0 }
    }

    fn run(&mut self, program: &str, command_line: &str, io: &Io<Handle>) -> Result<u32, u32> {
        let app = wide(program);
        let mut line = wide(command_line);
        let mut si = StartupInfoW {
            cb: size_of::<StartupInfoW>() as u32,
            reserved: core::ptr::null(),
            desktop: core::ptr::null(),
            title: core::ptr::null(),
            dims: [0; 7],
            flags: 0,
            show_window: 0,
            reserved2_size: 0,
            reserved2: core::ptr::null(),
            std_input: 0,
            std_output: 0,
            std_error: 0,
        };
        if io.redirected {
            si.flags = STARTF_USESTDHANDLES;
            [si.std_input, si.std_output, si.std_error] = io.handles;
        }
        let mut pi = ProcessInformation { process: 0, thread: 0, pid: 0, tid: 0 };
        // SAFETY: NUL-terminated strings, a writable command line, local structures.
        let ok = unsafe {
            CreateProcessW(
                app.as_ptr(),
                line.as_mut_ptr(),
                core::ptr::null(),
                core::ptr::null(),
                i32::from(io.redirected),
                0,
                core::ptr::null(),
                core::ptr::null(),
                &si,
                &mut pi,
            )
        };
        if ok == 0 {
            // SAFETY: plain call.
            return Err(unsafe { GetLastError() });
        }
        let mut code = 0u32;
        // SAFETY: the handles CreateProcessW returned.
        unsafe {
            WaitForSingleObject(pi.process, INFINITE);
            GetExitCodeProcess(pi.process, &mut code);
            CloseHandle(pi.thread);
            CloseHandle(pi.process);
        }
        Ok(code)
    }

    fn random(&mut self) -> u32 {
        // SAFETY: plain call.
        let t = unsafe { GetTickCount() };
        let mut x = t ^ 0x9E37_79B9 ^ (self as *const Self as usize as u32);
        x ^= x << 13;
        x ^= x >> 17;
        x ^ (x << 5)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn cmd_main() -> ! {
    // SAFETY: the process's command line, NUL-terminated.
    let line = unsafe { from_wide(GetCommandLineW()) };
    // SAFETY: plain calls.
    let std = STD_HANDLES.map(|w| unsafe { GetStdHandle(w) });
    let unicode = poly_cmd::args::parse(poly_cmd::args::after_program_name(&line)).unicode;
    let mut shell = Shell::new(Win { std, unicode }, &line);
    let code = shell.main();
    // SAFETY: plain call.
    unsafe { ExitProcess(code as u32) }
}
