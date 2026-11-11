//! DuplicateHandle's access argument in its other forms: zero, MAXIMUM_ALLOWED, generic rights,
//! bits the object type does not have; with DUPLICATE_SAME_ACCESS and DUPLICATE_CLOSE_SOURCE;
//! and the status NtDuplicateObject gives for the request DuplicateHandle refuses.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

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

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; characters outside the BMP are not used).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<600> {
    let mut b = Buf::<600>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    let low = |c: u16| if (0x41..=0x5a).contains(&c) { c + 0x20 } else { c };
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| low(x) == low(y))
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const INFINITE: u32 = 0xFFFF_FFFF;
/// A value no call sets as last error: "unchanged" when it is still there afterwards.
const MARK: u32 = 0xDEAD;

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
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn Sleep(ms: u32);
    fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, put: *mut u32, ov: *mut c_void) -> BOOL;
    fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
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

fn mark_error() {
    // SAFETY: sets this thread's last error.
    unsafe { SetLastError(MARK) };
}

/// The last error as text: `unchanged` when it still is the mark set before the call.
fn err_text() -> Buf<24> {
    let e = last_error();
    let mut b = Buf::<24>::new();
    if e == MARK {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{e}");
    }
    b
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
    let units = wstr_at(p, 700);
    let n = units.len();
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

/// A child process: this test's own image started with `--child <args>`.
struct Child {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

/// Start this test's own image with `--child <args>` (handles inherited, creation `flags`).
fn start_child(args: &str, flags: u32) -> Option<Child> {
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
            flags,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(Child { process: pi.process, thread: pi.thread, pid: pi.pid, tid: pi.tid })
}

impl Child {
    /// Wait for the child (10 s at most, then it is ended with 0xDEAD); close its handles; its exit code.
    fn finish(self) -> u32 {
        let mut code = 0u32;
        // SAFETY: the handles of the process this test started, closed once.
        unsafe {
            if WaitForSingleObject(self.process, 10_000) != 0 {
                TerminateProcess(self.process, 0xDEAD);
                WaitForSingleObject(self.process, 5_000);
            }
            GetExitCodeProcess(self.process, &mut code);
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
        code
    }
    /// End the child now and close its handles.
    fn kill(self) {
        // SAFETY: the handles of the process this test started, closed once.
        unsafe {
            TerminateProcess(self.process, 0xDEAD);
            WaitForSingleObject(self.process, 5_000);
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

/// Start this test's own image with `--child <args>` and wait for it; its exit code.
fn run_child(args: &str) -> Option<u32> {
    start_child(args, 0).map(Child::finish)
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

/// The entry point of a test that starts its own image again: a process started with
/// `--child <args>` runs `$child(args) -> u32` (its exit code) and never opens the observation
/// file (the parent holds it); the other one runs `$test` like `poly_wtest::main!`.
macro_rules! main_with_child {
    ($child:path, $test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_args() {
                exit($child(a.as_str()))
            }
            if open_output().is_err() {
                exit(EXIT_NO_OUTPUT);
            }
            let mut t = Test::new();
            $test(&mut t);
            let code = t.finish();
            exit(code)
        }
    };
}

// handleapi.h (DuplicateHandle), memoryapi.h / winbase.h (file mappings), synchapi.h,
// processthreadsapi.h, securitybaseapi.h (security descriptors and ACLs), winnt.h (access masks,
// SECURITY_DESCRIPTOR, ACL, SID), winternl.h (NtQueryObject, PUBLIC_OBJECT_BASIC_INFORMATION).
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DuplicateHandle(
        sp: HANDLE,
        s: HANDLE,
        tp: HANDLE,
        t: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    fn CreateFileMappingW(
        file: HANDLE,
        sa: *const SECURITY_ATTRIBUTES,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u16,
    ) -> HANDLE;
    fn MapViewOfFile(mapping: HANDLE, access: u32, off_high: u32, off_low: u32, bytes: usize) -> *mut c_void;
    fn UnmapViewOfFile(base: *const c_void) -> BOOL;
    fn CreateMutexW(sa: *const SECURITY_ATTRIBUTES, owned: BOOL, name: *const u16) -> HANDLE;
    fn CreateSemaphoreW(sa: *const SECURITY_ATTRIBUTES, initial: i32, maximum: i32, name: *const u16) -> HANDLE;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn OpenThread(access: u32, inherit: BOOL, tid: u32) -> HANDLE;
    fn ResumeThread(h: HANDLE) -> u32;
    fn CreateThread(
        sa: *const SECURITY_ATTRIBUTES,
        stack: usize,
        start: extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> HANDLE;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitializeSecurityDescriptor(sd: *mut c_void, revision: u32) -> BOOL;
    fn SetSecurityDescriptorDacl(sd: *mut c_void, present: BOOL, dacl: *const c_void, defaulted: BOOL) -> BOOL;
    fn InitializeAcl(acl: *mut c_void, length: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAce(acl: *mut c_void, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn DuplicateTokenEx(
        token: HANDLE,
        access: u32,
        sa: *const SECURITY_ATTRIBUTES,
        level: u32,
        kind: u32,
        new: *mut HANDLE,
    ) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: *mut c_void, length: u32, ret_len: *mut u32) -> NTSTATUS;
    fn NtDuplicateObject(
        sp: HANDLE,
        s: HANDLE,
        tp: HANDLE,
        t: *mut HANDLE,
        access: u32,
        attributes: u32,
        options: u32,
    ) -> NTSTATUS;
}

const DUPLICATE_CLOSE_SOURCE: u32 = 1;
const DUPLICATE_SAME_ACCESS: u32 = 2;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const SECTION_QUERY: u32 = 0x0001;
const SECTION_MAP_WRITE: u32 = 0x0002;
const SECTION_MAP_READ: u32 = 0x0004;
const SECTION_MAP_EXECUTE: u32 = 0x0008;
const SECTION_EXTEND_SIZE: u32 = 0x0010;
const SECTION_ALL_ACCESS: u32 = 0x000F_001F;
const EVENT_MODIFY_STATE: u32 = 0x0002;
const EVENT_ALL_ACCESS: u32 = 0x001F_0003;
const MUTEX_ALL_ACCESS: u32 = 0x001F_0001;
const PROCESS_TERMINATE: u32 = 0x0001;
const PROCESS_DUP_HANDLE: u32 = 0x0040;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_ALL_ACCESS: u32 = 0x001F_FFFF;
const THREAD_TERMINATE: u32 = 0x0001;
const THREAD_SUSPEND_RESUME: u32 = 0x0002;
const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x0800;
const THREAD_ALL_ACCESS: u32 = 0x001F_FFFF;
const TOKEN_DUPLICATE: u32 = 0x0002;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_ALL_ACCESS: u32 = 0x000F_01FF;
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const CREATE_SUSPENDED: u32 = 4;
const UNTOUCHED: usize = 0x1111_2222;

/// winternl.h `PUBLIC_OBJECT_BASIC_INFORMATION` (56 bytes).
#[repr(C)]
struct BasicInfo {
    attributes: u32,
    granted: u32,
    handles: u32,
    pointers: u32,
    reserved: [u32; 10],
}

/// The access a handle was granted (NtQueryObject, ObjectBasicInformation).
fn granted(h: HANDLE) -> u32 {
    let mut b = BasicInfo { attributes: 0, granted: 0xFFFF_FFFF, handles: 0, pointers: 0, reserved: [0; 10] };
    let mut n = 0u32;
    // SAFETY: a 56-byte structure and a live length.
    unsafe { NtQueryObject(h, 0, (&raw mut b).cast(), 56, &mut n) };
    b.granted
}

/// The SID of Everyone, S-1-1-0 (winnt.h `SID`: revision, count, authority, one sub-authority).
static EVERYONE: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];

/// What a security descriptor's DACL is.
#[derive(Clone, Copy)]
enum Dacl {
    /// Present and NULL: everybody may do everything.
    Null,
    /// Present and empty: nobody is allowed anything.
    Empty,
    /// One entry: Everyone is allowed this mask.
    Everyone(u32),
}

/// An absolute security descriptor (winnt.h: 40 bytes on x64) with its ACL behind it.
#[repr(C, align(8))]
struct Sd {
    sd: [u8; 40],
    acl: [u8; 64],
}

impl Sd {
    fn new() -> Sd {
        Sd { sd: [0; 40], acl: [0; 64] }
    }
    /// Fill it in place (the descriptor points at its own ACL) and return attributes that name it.
    fn attributes(&mut self, dacl: Dacl) -> SECURITY_ATTRIBUTES {
        let sd = self.sd.as_mut_ptr().cast::<c_void>();
        let acl = self.acl.as_mut_ptr().cast::<c_void>();
        // SAFETY: both buffers are large enough (SECURITY_DESCRIPTOR_MIN_LENGTH 40; an ACL with one entry 28).
        unsafe {
            InitializeSecurityDescriptor(sd, 1);
            match dacl {
                Dacl::Null => {
                    SetSecurityDescriptorDacl(sd, 1, core::ptr::null(), 0);
                }
                Dacl::Empty => {
                    InitializeAcl(acl, 8, 2);
                    SetSecurityDescriptorDacl(sd, 1, acl, 0);
                }
                Dacl::Everyone(mask) => {
                    InitializeAcl(acl, 64, 2);
                    AddAccessAllowedAce(acl, 2, mask, EVERYONE.as_ptr().cast());
                    SetSecurityDescriptorDacl(sd, 1, acl, 0);
                }
            }
        }
        SECURITY_ATTRIBUTES { length: 24, security_descriptor: sd, inherit_handle: 0 }
    }
}

fn me() -> HANDLE {
    // SAFETY: no arguments.
    unsafe { GetCurrentProcess() }
}

/// A duplicate of `src` in this process with exactly `access` (no options), or NULL.
fn lower(src: HANDLE, access: u32) -> HANDLE {
    let mut out: HANDLE = core::ptr::null_mut();
    // SAFETY: live handles and out pointer.
    unsafe { DuplicateHandle(me(), src, me(), &mut out, access, 0, 0) };
    out
}

/// One line: DuplicateHandle(this process, `src`, this process, &out, `access`, FALSE, `options`):
/// the result, then the new handle's granted access, or the last error and what is in `out`.
fn try_dup(t: &mut Test, what: &str, src: HANDLE, access: u32, options: u32) {
    let mut out = UNTOUCHED as HANDLE;
    mark_error();
    // SAFETY: live handles and out pointer.
    let ok = unsafe { DuplicateHandle(me(), src, me(), &mut out, access, 0, options) };
    if ok != 0 {
        say!(t, "{what}: access {access:#x} options {options} -> 1, granted {:#x}", granted(out));
        close(out);
    } else {
        let left = if out as usize == UNTOUCHED {
            "untouched"
        } else if out.is_null() {
            "NULL"
        } else {
            "something else"
        };
        say!(
            t,
            "{what}: access {access:#x} options {options} -> 0, last error {}, out handle {left}",
            err_text().as_str()
        );
    }
}

/// `try_dup` for each access in `list` (no options).
fn try_each(t: &mut Test, what: &str, src: HANDLE, list: &[u32]) {
    for &a in list {
        try_dup(t, what, src, a, 0);
    }
}

/// A page-file backed section of one page, read-write, unnamed.
fn section(sa: *const SECURITY_ATTRIBUTES) -> HANDLE {
    // SAFETY: `sa` is NULL or live attributes.
    unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, sa, PAGE_READWRITE, 0, 4096, core::ptr::null()) }
}

poly_wtest::main!(test);

const READ_ONLY: u32 = SECTION_QUERY | SECTION_MAP_READ;

fn forms(t: &mut Test, what: &str, low: HANDLE) {
    try_each(
        t,
        what,
        low,
        &[
            0,
            MAXIMUM_ALLOWED,
            MAXIMUM_ALLOWED | SECTION_QUERY,
            MAXIMUM_ALLOWED | SECTION_MAP_WRITE,
            GENERIC_READ,
            GENERIC_WRITE,
            GENERIC_EXECUTE,
            GENERIC_ALL,
            0x0000_FFE0,
            SYNCHRONIZE,
        ],
    );
    try_dup(t, what, low, 0, DUPLICATE_SAME_ACCESS);
    try_dup(t, what, low, GENERIC_ALL, DUPLICATE_SAME_ACCESS);
    try_dup(t, what, low, MAXIMUM_ALLOWED, DUPLICATE_SAME_ACCESS);
}

fn test(t: &mut Test) {
    // A section nobody is allowed anything on, through a read-only handle.
    let mut sd = Sd::new();
    let sa = sd.attributes(Dacl::Empty);
    let full = section(&sa);
    let low = lower(full, READ_ONLY);
    say!(t, "section, empty DACL: read-only duplicate granted {:#x}", granted(low));
    forms(t, "section, empty DACL, read-only handle", low);

    // The native call for the request Chrome makes, and for one that is allowed.
    let mut out = UNTOUCHED as HANDLE;
    // SAFETY: live handles and out pointer.
    let s = unsafe { NtDuplicateObject(me(), low, me(), &mut out, SECTION_MAP_WRITE, 0, 0) };
    say!(
        t,
        "NtDuplicateObject(read-only handle, SECTION_MAP_WRITE) = {:#x}; out handle {}",
        s as u32,
        if out as usize == UNTOUCHED {
            "untouched"
        } else if out.is_null() {
            "NULL"
        } else {
            "set"
        }
    );
    let mut out = UNTOUCHED as HANDLE;
    // SAFETY: as above.
    let s = unsafe { NtDuplicateObject(me(), low, me(), &mut out, SECTION_QUERY, 0, 0) };
    say!(t, "NtDuplicateObject(read-only handle, SECTION_QUERY) = {:#x}; granted {:#x}", s as u32, granted(out));
    close(out);

    // DUPLICATE_CLOSE_SOURCE with a request that is refused: is the source handle still open afterwards?
    let victim = lower(full, READ_ONLY);
    let mut out = UNTOUCHED as HANDLE;
    mark_error();
    // SAFETY: live handles and out pointer.
    let ok = unsafe { DuplicateHandle(me(), victim, me(), &mut out, SECTION_MAP_WRITE, 0, DUPLICATE_CLOSE_SOURCE) };
    let e = err_text();
    say!(
        t,
        "refused request with DUPLICATE_CLOSE_SOURCE: -> {ok} last error {}; the source handle afterwards: granted {:#x}",
        e.as_str(),
        granted(victim)
    );
    // SAFETY: closes the source if the call left it open; reports whether it was.
    let still = unsafe { CloseHandle(victim) };
    say!(t, "closing the source handle afterwards: {still}");
    close(low);
    close(full);

    // The same forms on a section everybody may read, through a query-only handle.
    let mut sd = Sd::new();
    let sa = sd.attributes(Dacl::Everyone(READ_ONLY));
    let full = section(&sa);
    let low = lower(full, SECTION_QUERY);
    say!(t, "section, DACL allows Everyone QUERY|MAP_READ: query-only duplicate granted {:#x}", granted(low));
    forms(t, "section, Everyone may read, query-only handle", low);
    close(low);
    close(full);

    // And on a section made with no security attributes.
    let full = section(core::ptr::null());
    let low = lower(full, SECTION_QUERY);
    forms(t, "section, no security attributes, query-only handle", low);
    close(low);
    close(full);
}
