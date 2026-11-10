//! NtQueryInformationFile(FileNameInformation) on pipe handles: the name a named pipe's server
//! end and client end report (before and after a client connects, for a second instance, for a
//! client that spelled the name in another case, for a duplicate), names of the shape Chrome's
//! crash handler uses, and the two ends of an anonymous pipe (CreatePipe).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, fileapi.h, namedpipeapi.h,
// winternl.h, ntifs.h (FILE_NAME_INFORMATION), accctrl.h, aclapi.h, sddl.h,
// securitybaseapi.h, ntstatus.h and winerror.h (constants, layouts); the
// helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;

/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// Where observation lines go: the test's output file, or (in a child process) a pipe to the parent.
trait Sink {
    fn put(&mut self, s: &str);
}

impl Sink for Test {
    fn put(&mut self, s: &str) {
        self.line(s);
    }
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<600>::new();
        let _ = write!(b, $($a)*);
        $t.put(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 299 units).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.bytes().enumerate().take(299) {
        w[i] = u16::from(c);
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<400> {
    let mut b = Buf::<400>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const PIPE_ACCESS_INBOUND: u32 = 1;
const PIPE_ACCESS_OUTBOUND: u32 = 2;
const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const PIPE_TYPE_MESSAGE: u32 = 4;
const PIPE_READMODE_MESSAGE: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_NAME_INFORMATION_CLASS: u32 = 9; // FileNameInformation (ntifs.h / wdm.h FILE_INFORMATION_CLASS)
const FILE_NAME_INFO_CLASS: u32 = 2; // FileNameInfo (minwinbase.h FILE_INFO_BY_HANDLE_CLASS)
const OBJECT_NAME_INFORMATION_CLASS: u32 = 1; // ObjectNameInformation (ntifs.h OBJECT_INFORMATION_CLASS)
const INFINITE: u32 = 0xFFFF_FFFF;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_MORE_DATA: u32 = 234;
const SENTINEL: usize = 0x1111_2222;

/// winbase.h `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: BOOL,
}

fn inheritable() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES { length: 24, security_descriptor: core::ptr::null_mut(), inherit_handle: 1 }
}

/// processthreadsapi.h `STARTUPINFOW` (104 bytes on x64).
#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    pos: [u32; 7],
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std: [HANDLE; 3],
}

/// processthreadsapi.h `PROCESS_INFORMATION`.
#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_size: u32,
        in_size: u32,
        default_timeout: u32,
        sa: *const SECURITY_ATTRIBUTES,
    ) -> HANDLE;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn DuplicateHandle(
        sp: HANDLE,
        s: HANDLE,
        tp: HANDLE,
        t: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetFileInformationByHandleEx(h: HANDLE, class: u32, buf: *mut c_void, size: u32) -> BOOL;
    fn GetNamedPipeHandleStateW(
        h: HANDLE,
        state: *mut u32,
        cur_instances: *mut u32,
        max_collection: *mut u32,
        collect_timeout: *mut u32,
        user_name: *mut u16,
        max_user_name: u32,
    ) -> BOOL;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, put: *mut u32, ov: *mut c_void) -> BOOL;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const SECURITY_ATTRIBUTES,
        ta: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn GetCommandLineW() -> *const u16;
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, n: u32) -> u32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn LocalSize(p: *mut c_void) -> usize;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationFile(
        h: HANDLE,
        iosb: *mut IoStatusBlock,
        info: *mut c_void,
        length: u32,
        class: u32,
    ) -> NTSTATUS;
    fn NtQueryObject(h: HANDLE, class: u32, info: *mut c_void, length: u32, ret_len: *mut u32) -> NTSTATUS;
}

/// 8-byte aligned scratch memory, filled with 0xAA before every query.
#[repr(C, align(8))]
struct Scratch {
    b: [u8; 800],
}

impl Scratch {
    fn new() -> Self {
        Scratch { b: [0xAA; 800] }
    }
    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.b[off], self.b[off + 1]])
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        u64::from(self.u32_at(off)) | u64::from(self.u32_at(off + 4)) << 32
    }
    /// One past the last byte that is no longer 0xAA.
    fn written(&self) -> usize {
        self.b.iter().rposition(|&x| x != 0xAA).map_or(0, |i| i + 1)
    }
    /// The UTF-16 units stored in `from..to` (whole units only).
    fn text(&self, from: usize, to: usize) -> Buf<400> {
        let mut u = [0u16; 400];
        let mut n = 0;
        let mut i = from;
        while i + 2 <= to && n < u.len() {
            u[n] = self.u16_at(i);
            n += 1;
            i += 2;
        }
        units_text(&u[..n])
    }
}

/// What a name query left behind.
struct NameResult {
    status: NTSTATUS,
    io_status: usize,
    io_info: usize,
    length_field: u32,
    written: usize,
    name: Buf<400>,
}

/// NtQueryInformationFile(FileNameInformation) with a buffer of `len` bytes (the bytes after it stay 0xAA).
fn file_name(h: HANDLE, len: usize) -> NameResult {
    let mut s = Scratch::new();
    let mut io = IoStatusBlock { status: SENTINEL, information: SENTINEL };
    // SAFETY: `s` is 800 bytes and len <= 800; `io` is a live local.
    let status =
        unsafe { NtQueryInformationFile(h, &mut io, s.b.as_mut_ptr().cast(), len as u32, FILE_NAME_INFORMATION_CLASS) };
    let written = s.written();
    let length_field = s.u32_at(0);
    let end = written.min(4 + length_field as usize);
    NameResult { status, io_status: io.status, io_info: io.information, length_field, written, name: s.text(4, end) }
}

/// One line for a FileNameInformation query.
fn say_file_name(t: &mut impl Sink, what: &str, h: HANDLE, len: usize) {
    say_file_name_as(t, what, h, len, None);
}

/// The same; with `expected`, the name and the sizes (which depend on where the test runs) are
/// only compared with it and not printed.
fn say_file_name_as(t: &mut impl Sink, what: &str, h: HANDLE, len: usize, expected: Option<&str>) {
    let r = file_name(h, len);
    let mut io = Buf::<80>::new();
    if r.io_status == SENTINEL && r.io_info == SENTINEL {
        let _ = write!(io, "iosb untouched");
    } else if expected.is_some() {
        let _ = write!(
            io,
            "iosb {:#x} info is 4 + FileNameLength: {}",
            r.io_status as u32,
            u32::from(r.io_info == 4 + r.length_field as usize)
        );
    } else {
        let _ = write!(io, "iosb {:#x} info {}", r.io_status as u32, r.io_info);
    }
    if r.written == 0 {
        say!(t, "{what} [{len}]: status {:#x} {}; buffer untouched", r.status as u32, io.as_str());
    } else if let Some(e) = expected {
        say!(
            t,
            "{what}: status {:#x} {}; FileNameLength is 2 x characters: {} name is the expected one: {}",
            r.status as u32,
            io.as_str(),
            u32::from(r.length_field as usize == 2 * e.len()),
            u32::from(r.name.as_str() == e)
        );
    } else {
        say!(
            t,
            "{what} [{len}]: status {:#x} {}; FileNameLength {} written {} name \"{}\"",
            r.status as u32,
            io.as_str(),
            r.length_field,
            r.written,
            r.name.as_str()
        );
    }
}

/// One line for GetFileInformationByHandleEx(FileNameInfo) with a buffer of `len` bytes.
fn say_name_info(t: &mut impl Sink, what: &str, h: HANDLE, len: usize) {
    say_name_info_as(t, what, h, len, None);
}

/// The same; with `expected`, the name and its length are only compared with it.
fn say_name_info_as(t: &mut impl Sink, what: &str, h: HANDLE, len: usize, expected: Option<&str>) {
    let mut s = Scratch::new();
    // SAFETY: `s` is 800 bytes and len <= 800.
    let ok = unsafe {
        SetLastError(0);
        GetFileInformationByHandleEx(h, FILE_NAME_INFO_CLASS, s.b.as_mut_ptr().cast(), len as u32)
    };
    let e = if ok == 0 { last_error() } else { 0 };
    let written = s.written();
    if written == 0 {
        say!(t, "{what} [{len}]: ret {ok} error {e}; buffer untouched");
        return;
    }
    let length_field = s.u32_at(0);
    let end = written.min(4 + length_field as usize);
    let name = s.text(4, end);
    if let Some(x) = expected {
        say!(
            t,
            "{what}: ret {ok} error {e}; FileNameLength is 2 x characters: {} name is the expected one: {}",
            u32::from(length_field as usize == 2 * x.len()),
            u32::from(name.as_str() == x)
        );
    } else {
        say!(
            t,
            "{what} [{len}]: ret {ok} error {e}; FileNameLength {length_field} written {written} name \"{}\"",
            name.as_str()
        );
    }
}

/// What NtQueryObject(ObjectNameInformation) left in a buffer of `len` bytes.
struct ObjectName {
    status: NTSTATUS,
    /// None when the call did not write it.
    return_length: Option<u32>,
    written: usize,
    length: usize,
    maximum: usize,
    /// The offset of UNICODE_STRING.Buffer inside the caller's buffer; None for NULL or elsewhere.
    buffer_at: Option<usize>,
    buffer_null: bool,
    name: Buf<400>,
}

fn object_name(h: HANDLE, len: usize) -> ObjectName {
    let mut s = Scratch::new();
    let mut ret: u32 = 0x1111_2222;
    // SAFETY: `s` is 800 bytes and len <= 800; `ret` is a live local.
    let status =
        unsafe { NtQueryObject(h, OBJECT_NAME_INFORMATION_CLASS, s.b.as_mut_ptr().cast(), len as u32, &mut ret) };
    let written = s.written();
    // UNICODE_STRING (winternl.h): Length u16 @0, MaximumLength u16 @2, Buffer pointer @8.
    let length = s.u16_at(0) as usize;
    let maximum = s.u16_at(2) as usize;
    let ptr = s.u64_at(8);
    let base = s.b.as_ptr() as u64;
    let buffer_at = if written >= 16 && ptr >= base && ptr < base + 800 { Some((ptr - base) as usize) } else { None };
    let off = buffer_at.unwrap_or(16);
    let name = if written > off { s.text(off, written.min(off + length)) } else { Buf::new() };
    ObjectName {
        status,
        return_length: if ret == 0x1111_2222 { None } else { Some(ret) },
        written,
        length,
        maximum,
        buffer_at,
        buffer_null: written >= 16 && ptr == 0,
        name,
    }
}

/// One line for NtQueryObject(ObjectNameInformation) with a buffer of `len` bytes.
fn say_object_name(t: &mut impl Sink, what: &str, h: HANDLE, len: usize) {
    let r = object_name(h, len);
    let mut rl = Buf::<40>::new();
    match r.return_length {
        None => {
            let _ = write!(rl, "untouched");
        }
        Some(n) => {
            let _ = write!(rl, "{n}");
        }
    }
    if r.written == 0 {
        say!(t, "{what} [{len}]: status {:#x} ReturnLength {}; buffer untouched", r.status as u32, rl.as_str());
        return;
    }
    let mut at = Buf::<40>::new();
    match r.buffer_at {
        Some(o) => {
            let _ = write!(at, "+{o}");
        }
        None => {
            let _ = write!(at, "{}", if r.buffer_null { "NULL" } else { "elsewhere" });
        }
    }
    say!(
        t,
        "{what} [{len}]: status {:#x} ReturnLength {}; Length {} MaximumLength {} Buffer {} written {} name \"{}\"",
        r.status as u32,
        rl.as_str(),
        r.length,
        r.maximum,
        at.as_str(),
        r.written,
        r.name.as_str()
    );
}

/// A file `name` in the test's own directory, deleted when its handle is closed.
fn scratch_file(name: &[u16]) -> HANDLE {
    let Some(path) = path_beside_image(name) else {
        return core::ptr::null_mut();
    };
    let us = path.unicode_string();
    let oa = ObjectAttributes::named(&us);
    let mut h: HANDLE = core::ptr::null_mut();
    let mut io = IoStatusBlock::new();
    // SAFETY: every pointer is to a live local; no EA buffer.
    unsafe {
        NtCreateFile(
            &mut h,
            GENERIC_READ | GENERIC_WRITE | DELETE | SYNCHRONIZE,
            &oa,
            &mut io,
            core::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            0,
            FILE_OPEN_IF,
            FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE | FILE_DELETE_ON_CLOSE,
            core::ptr::null(),
            0,
        )
    };
    h
}

/// The path a file `name` beside the test's image has on its volume: the image's directory
/// without the drive, then the name.
fn volume_path(name: &str) -> Buf<400> {
    let text = units_text(image_path());
    let s = text.as_str();
    let s = s.strip_prefix("\\??\\").unwrap_or(s);
    let s = match s.find(':') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    let dir = match s.rfind('\\') {
        Some(i) => &s[..=i],
        None => "",
    };
    let mut b = Buf::<400>::new();
    let _ = write!(b, "{dir}{name}");
    b
}

/// A named-pipe server end: `\\.\pipe\<name>`, duplex, message mode, `instances` at most.
fn pipe_server(name: &str, open_mode: u32, instances: u32, sa: *const SECURITY_ATTRIBUTES) -> HANDLE {
    let mut full = Buf::<280>::new();
    let _ = write!(full, "\\\\.\\pipe\\{name}");
    let w = wide(full.as_str());
    // SAFETY: `w` is NUL-terminated; `sa` is NULL or a live structure.
    unsafe {
        CreateNamedPipeW(w.as_ptr(), open_mode, PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE, instances, 512, 512, 0, sa)
    }
}

/// A client end opened with CreateFileW by the full path given.
fn pipe_client(path: &str) -> HANDLE {
    let w = wide(path);
    // SAFETY: `w` is NUL-terminated.
    unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    }
}

fn close(h: HANDLE) {
    if !h.is_null() && h != INVALID_HANDLE_VALUE {
        // SAFETY: a handle this test opened, closed once.
        unsafe { CloseHandle(h) };
    }
}

fn last_error() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetLastError() }
}

/// 1 for a usable handle, else 0.
fn opened(h: HANDLE) -> u32 {
    u32::from(!h.is_null() && h != INVALID_HANDLE_VALUE)
}

// ----------------------------------------------------- child processes

/// The ASCII text after ` --child ` on this process's command line.
fn child_args() -> Option<Buf<160>> {
    // SAFETY: GetCommandLineW returns a NUL-terminated string that lives as long as the process.
    let p = unsafe { GetCommandLineW() };
    let mut units = [0u16; 700];
    let mut n = 0;
    // SAFETY: reads up to the terminating NUL, bounded by `units`.
    unsafe {
        while n < units.len() && *p.add(n) != 0 {
            units[n] = *p.add(n);
            n += 1;
        }
    }
    let key = b" --child ";
    let at = (0..n.saturating_sub(key.len())).find(|&i| (0..key.len()).all(|j| units[i + j] == u16::from(key[j])))?;
    let mut out = Buf::<160>::new();
    for &u in &units[at + key.len()..n] {
        let _ = write!(out, "{}", u as u8 as char);
    }
    Some(out)
}

/// The `i`th space-separated word of `args` as a hexadecimal handle value.
fn arg_handle(args: &str, i: usize) -> HANDLE {
    let v = args.split(' ').nth(i).and_then(|w| usize::from_str_radix(w, 16).ok()).unwrap_or(0);
    v as HANDLE
}

/// Start this test's own image with `--child <args>`, handles inherited; wait for it (10 s at most).
/// Returns its exit code, or None when it could not be started (0xDEAD when it had to be ended).
fn run_child(args: &str) -> Option<u32> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut cmd = [0u16; 800];
    let mut m = 0;
    let mut push = |u: u16| {
        if m < 799 {
            cmd[m] = u;
            m += 1;
        }
    };
    push(u16::from(b'"'));
    for &u in &exe[..n] {
        push(u);
    }
    push(u16::from(b'"'));
    for b in " --child ".bytes().chain(args.bytes()) {
        push(u16::from(b));
    }
    let si = STARTUPINFOW {
        cb: 104,
        reserved: core::ptr::null_mut(),
        desktop: core::ptr::null_mut(),
        title: core::ptr::null_mut(),
        pos: [0; 7],
        flags: 0,
        show_window: 0,
        cb_reserved2: 0,
        reserved2: core::ptr::null_mut(),
        std: [core::ptr::null_mut(); 3],
    };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: `cmd` is mutable and NUL-terminated; the structures are live locals.
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            0,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return None;
    }
    let mut code = 0u32;
    // SAFETY: the handles of the process just started, closed once.
    unsafe {
        if WaitForSingleObject(pi.process, 10_000) != 0 {
            TerminateProcess(pi.process, 0xDEAD);
            WaitForSingleObject(pi.process, 5_000);
        }
        GetExitCodeProcess(pi.process, &mut code);
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
    }
    Some(code)
}

/// The child's report: lines written to an inherited anonymous pipe. The parent side.
struct Report {
    read: HANDLE,
    write: HANDLE,
}

impl Report {
    fn new() -> Report {
        let mut r = Report { read: core::ptr::null_mut(), write: core::ptr::null_mut() };
        let sa = inheritable();
        // SAFETY: live out pointers and attributes.
        unsafe { CreatePipe(&mut r.read, &mut r.write, &sa, 8192) };
        r
    }
    /// Close the write end and print what the child wrote, each line with `child: ` before it.
    fn print(self, t: &mut Test) {
        close(self.write);
        let mut all = [0u8; 8192];
        let mut n = 0usize;
        loop {
            let mut got = 0u32;
            // SAFETY: reads into the free part of `all`.
            let ok = unsafe {
                ReadFile(self.read, all.as_mut_ptr().add(n), (all.len() - n) as u32, &mut got, core::ptr::null_mut())
            };
            if ok == 0 || got == 0 {
                break;
            }
            n += got as usize;
            if n == all.len() {
                break;
            }
        }
        close(self.read);
        for line in core::str::from_utf8(&all[..n]).unwrap_or("?").split('\n') {
            if !line.is_empty() {
                say!(t, "child: {line}");
            }
        }
    }
}

/// The child side of a report: lines go into the inherited pipe.
struct ChildOut {
    h: HANDLE,
}

impl Sink for ChildOut {
    fn put(&mut self, s: &str) {
        let mut put = 0u32;
        // SAFETY: writes the bytes of `s` and a newline to the inherited pipe handle.
        unsafe {
            WriteFile(self.h, s.as_ptr(), s.len() as u32, &mut put, core::ptr::null_mut());
            WriteFile(self.h, b"\n".as_ptr(), 1, &mut put, core::ptr::null_mut());
        }
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let first = PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE;
    let s = pipe_server("PolySpec4_NameA", first, 2, core::ptr::null());
    say!(t, "server created {}", opened(s));
    say_file_name(t, "server, nobody connected", s, 528);
    let c = pipe_client("\\\\.\\pipe\\PolySpec4_NameA");
    say!(t, "client opened {}", opened(c));
    say_file_name(t, "server, connected", s, 528);
    say_file_name(t, "client", c, 528);

    let s2 = pipe_server("PolySpec4_NameA", PIPE_ACCESS_DUPLEX, 2, core::ptr::null());
    say!(t, "second instance created {}", opened(s2));
    say_file_name(t, "second instance", s2, 528);
    let c2 = pipe_client("\\\\.\\pipe\\POLYSPEC4_namea");
    say!(t, "client opened as POLYSPEC4_namea {}", opened(c2));
    say_file_name(t, "client opened as POLYSPEC4_namea", c2, 528);
    say_file_name(t, "its server end", s2, 528);

    let mut dup: HANDLE = core::ptr::null_mut();
    // SAFETY: duplicates a handle of this process into this process.
    let ok = unsafe { DuplicateHandle(GetCurrentProcess(), s, GetCurrentProcess(), &mut dup, 0, 0, 2) };
    say!(t, "duplicate made {ok}");
    say_file_name(t, "duplicate of the server end", dup, 528);
    close(dup);

    close(c);
    say_file_name(t, "server, client closed", s, 528);
    close(c2);
    close(s2);
    close(s);

    for name in
        ["crashpad_4242_ABCDEFGHIJKLMNOP", "PolySpec4.with.dots", "PolySpec4 with spaces", "PolySpec4\\sub\\leaf", "P"]
    {
        let s = pipe_server(name, first, 1, core::ptr::null());
        if opened(s) == 0 {
            say!(t, "pipe \"{name}\": not created, error {}", last_error());
            continue;
        }
        say_file_name(t, name, s, 528);
        close(s);
    }

    let (mut r, mut w): (HANDLE, HANDLE) = (core::ptr::null_mut(), core::ptr::null_mut());
    // SAFETY: live out pointers.
    let ok = unsafe { CreatePipe(&mut r, &mut w, core::ptr::null(), 0) };
    say!(t, "anonymous pipe created {ok}");
    say_file_name(t, "anonymous pipe, read end", r, 528);
    say_file_name(t, "anonymous pipe, write end", w, 528);
    close(r);
    close(w);
}
