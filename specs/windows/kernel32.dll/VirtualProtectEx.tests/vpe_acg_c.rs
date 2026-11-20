//! A suspended child started with PROCESS_CREATION_MITIGATION_POLICY_PROHIBIT_DYNAMIC_CODE_ALWAYS_ON, changed from the
//! parent: executable private memory (VirtualAllocEx PAGE_EXECUTE_READWRITE, VirtualProtectEx of private memory to
//! PAGE_EXECUTE_READ), and the 16-byte rewrite of an image code page (PAGE_WRITECOPY, write, PAGE_EXECUTE_READ); then
//! the child runs its stubs. Without the policy (first lines) the same steps for comparison.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 3 of the observations, the others write the rest.)
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
        if say_keep() {
            self.line(s);
        }
    }
}

/// Which observations of this run are written: 0 all of them, 1/2/3 every third one starting at the first/second/third
/// (a test that is kept in two files writes the two halves of its observations; every call is made in both).
static SAY_PART: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SAY_N: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn say_keep() -> bool {
    let p = SAY_PART.load(core::sync::atomic::Ordering::Relaxed);
    if p == 0 {
        return true;
    }
    let n = SAY_N.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    n % 3 + 1 == p
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
            // 40 s and 15 s: CI's emulated machines are slow and loaded (a child once still ran after 10 s + 5 s there)
            if WaitForSingleObject(self.process, 40_000) != 0 {
                TerminateProcess(self.process, 0xDEAD);
                WaitForSingleObject(self.process, 15_000);
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

// memoryapi.h, processthreadsapi.h, winternl.h, libloaderapi.h, fileapi.h, winnt.h, winbase.h, ntstatus.h:
// memory of another process, sections, process information.

/// winnt.h `MEMORY_BASIC_INFORMATION` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct MBI {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

const MBI_ZERO: MBI =
    MBI { base: 0, alloc_base: 0, alloc_protect: 0, partition: 0, size: 0, state: 0, protect: 0, kind: 0 };

// winnt.h page protections, allocation types and region kinds.
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_READONLY: u32 = 0x02;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_WRITECOPY: u32 = 0x08;
const PAGE_EXECUTE: u32 = 0x10;
const PAGE_EXECUTE_READ: u32 = 0x20;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const MEM_PRIVATE: u32 = 0x20000;
const MEM_MAPPED: u32 = 0x40000;
const MEM_IMAGE: u32 = 0x100_0000;
const SEC_IMAGE: u32 = 0x100_0000;
const SECTION_ALL_ACCESS: u32 = 0xF001F;
// winbase.h process creation flags.
const CREATE_SUSPENDED: u32 = 0x4;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
// fileapi.h / winnt.h file access.
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_EXECUTE: u32 = 0x2000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_DELETE: u32 = 4;
const OPEN_EXISTING: u32 = 3;
// ntstatus.h
const STATUS_SUCCESS: i32 = 0;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
    fn GetProcAddress(module: HANDLE, name: *const u8) -> *const c_void;
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn VirtualQuery(addr: *const c_void, mbi: *mut MBI, len: usize) -> usize;
    fn VirtualQueryEx(p: HANDLE, addr: *const c_void, mbi: *mut MBI, len: usize) -> usize;
    fn VirtualProtectEx(p: HANDLE, addr: *mut c_void, size: usize, prot: u32, old: *mut u32) -> BOOL;
    fn VirtualAllocEx(p: HANDLE, addr: *mut c_void, size: usize, kind: u32, prot: u32) -> *mut c_void;
    fn VirtualFreeEx(p: HANDLE, addr: *mut c_void, size: usize, kind: u32) -> BOOL;
    fn ReadProcessMemory(p: HANDLE, addr: *const c_void, buf: *mut c_void, n: usize, got: *mut usize) -> BOOL;
    fn WriteProcessMemory(p: HANDLE, addr: *mut c_void, buf: *const c_void, n: usize, put: *mut usize) -> BOOL;
    fn ResumeThread(h: HANDLE) -> u32;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: BOOL) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disp: u32,
        flags: u32,
        tmpl: HANDLE,
    ) -> HANDLE;
    fn SetFilePointerEx(h: HANDLE, dist: i64, newpos: *mut i64, method: u32) -> BOOL;
    fn FlushInstructionCache(p: HANDLE, addr: *const c_void, n: usize) -> BOOL;
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> BOOL;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attr: usize,
        value: *const c_void,
        size: usize,
        prev: *mut c_void,
        ret: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationProcess(p: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtCreateSection(
        s: *mut HANDLE,
        access: u32,
        oa: *const c_void,
        max: *const i64,
        prot: u32,
        attrs: u32,
        file: HANDLE,
    ) -> i32;
    fn NtMapViewOfSection(
        s: HANDLE,
        p: HANDLE,
        base: *mut *mut c_void,
        zero_bits: usize,
        commit: usize,
        offset: *mut i64,
        view: *mut usize,
        inherit: u32,
        kind: u32,
        prot: u32,
    ) -> i32;
    fn NtUnmapViewOfSection(p: HANDLE, base: *mut c_void) -> i32;
    fn NtWriteVirtualMemory(p: HANDLE, addr: *mut c_void, buf: *const c_void, n: usize, put: *mut usize) -> i32;
}

fn query(addr: usize) -> MBI {
    let mut m = MBI_ZERO;
    // SAFETY: a live out structure of the size passed.
    unsafe { VirtualQuery(addr as *const c_void, &mut m, 48) };
    m
}

fn query_ex(p: HANDLE, addr: usize) -> MBI {
    let mut m = MBI_ZERO;
    // SAFETY: a live out structure of the size passed.
    unsafe { VirtualQueryEx(p, addr as *const c_void, &mut m, 48) };
    m
}

/// `type/state/protect/allocation protect` of a region as text, with names for the usual values.
fn region_text(m: &MBI) -> Buf<160> {
    let mut b = Buf::<160>::new();
    let kind = match m.kind {
        MEM_PRIVATE => "MEM_PRIVATE",
        MEM_MAPPED => "MEM_MAPPED",
        MEM_IMAGE => "MEM_IMAGE",
        0 => "0",
        _ => "other",
    };
    let _ = write!(b, "type={kind} state={:#x} protect={:#x} allocprotect={:#x}", m.state, m.protect, m.alloc_protect);
    b
}

/// `ProcessBasicInformation` (winternl.h `PROCESS_BASIC_INFORMATION`, 48 bytes on x64): (status, return length, PEB, pid, parent pid).
fn basic_info(p: HANDLE) -> (i32, u32, usize, usize, usize) {
    let mut b = [0usize; 6];
    let mut ret = 0u32;
    // SAFETY: a 48-byte out buffer and a live length.
    let s = unsafe { NtQueryInformationProcess(p, 0, b.as_mut_ptr().cast(), 48, &mut ret) };
    (s, ret, b[1], b[4], b[5])
}

/// Read `n` bytes (at most 64) of another process: (ok, bytes copied, last error, the bytes).
fn read_other(p: HANDLE, addr: usize, n: usize) -> (bool, usize, u32, [u8; 64]) {
    let mut b = [0u8; 64];
    let mut got = 0usize;
    mark_error();
    // SAFETY: the buffer holds 64 bytes, at most 64 are read.
    let ok = unsafe { ReadProcessMemory(p, addr as *const c_void, b.as_mut_ptr().cast(), n.min(64), &mut got) } != 0;
    (ok, got, last_error(), b)
}

/// Write `data` into another process: (ok, bytes copied, last error).
fn write_other(p: HANDLE, addr: usize, data: &[u8]) -> (bool, usize, u32) {
    let mut put = 0usize;
    mark_error();
    // SAFETY: `data` is a live slice; the target is another process's address.
    let ok = unsafe { WriteProcessMemory(p, addr as *mut c_void, data.as_ptr().cast(), data.len(), &mut put) } != 0;
    (ok, put, last_error())
}

/// `VirtualProtectEx`: (ok, old protection, last error).
fn protect_other(p: HANDLE, addr: usize, n: usize, prot: u32) -> (bool, u32, u32) {
    let mut old = 0xEEEEu32;
    mark_error();
    // SAFETY: a live out value; the range belongs to the target process.
    let ok = unsafe { VirtualProtectEx(p, addr as *mut c_void, n, prot, &mut old) } != 0;
    (ok, old, last_error())
}

/// The address of an export of a loaded module (`ntdll.dll`, ...), or 0.
fn export_addr(dll: &str, name: &[u8]) -> usize {
    let w = wide(dll);
    // SAFETY: NUL-terminated names; the module stays loaded.
    unsafe {
        let m = GetModuleHandleW(w.as_ptr());
        if m.is_null() {
            return 0;
        }
        GetProcAddress(m, name.as_ptr()) as usize
    }
}

/// This executable's module handle (its base).
fn own_base() -> usize {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) as usize }
}

/// The full path of this executable as a NUL-terminated UTF-16 buffer.
fn own_path() -> [u16; 600] {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) };
    exe
}

/// Start this test's own image with `--child <args>` and `CREATE_SUSPENDED` plus a process mitigation policy word
/// (`PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY`, 8 bytes); `policy` 0 starts it without the attribute.
fn start_child_policy(args: &str, policy: u64) -> Option<Child> {
    if policy == 0 {
        return start_child(args, CREATE_SUSPENDED);
    }
    let exe = own_path();
    let mut cmd = [0u16; 800];
    let mut m = 0;
    let mut push = |u: u16| {
        if m < 799 {
            cmd[m] = u;
            m += 1;
        }
    };
    push(u16::from(b'"'));
    for &u in exe.iter().take_while(|&&u| u != 0) {
        push(u);
    }
    push(u16::from(b'"'));
    for b in " --child ".bytes().chain(args.bytes()) {
        push(u16::from(b));
    }
    let mut size = 0usize;
    let mut list = [0u64; 16];
    let word = policy;
    // SAFETY: the first call asks for the size; the list buffer (128 bytes) is larger than one attribute needs.
    unsafe {
        InitializeProcThreadAttributeList(core::ptr::null_mut(), 1, 0, &mut size);
        if size > 128 || InitializeProcThreadAttributeList(list.as_mut_ptr().cast(), 1, 0, &mut size) == 0 {
            return None;
        }
        UpdateProcThreadAttribute(
            list.as_mut_ptr().cast(),
            0,
            0x20007,
            (&word as *const u64).cast(),
            8,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
    }
    /// winbase.h `STARTUPINFOEXW`.
    #[repr(C)]
    struct SIEX {
        si: STARTUPINFOW,
        list: *mut c_void,
    }
    let si = SIEX {
        si: STARTUPINFOW {
            cb: 112,
            reserved: core::ptr::null_mut(),
            desktop: core::ptr::null_mut(),
            title: core::ptr::null_mut(),
            pos: [0; 7],
            flags: 0,
            show_window: 0,
            cb_reserved2: 0,
            reserved2: core::ptr::null_mut(),
            std: [core::ptr::null_mut(); 3],
        },
        list: list.as_mut_ptr().cast(),
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
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
            core::ptr::null(),
            core::ptr::null(),
            &si.si,
            &mut pi,
        )
    };
    // SAFETY: the list was initialised above.
    unsafe { DeleteProcThreadAttributeList(list.as_mut_ptr().cast()) };
    if ok == 0 {
        return None;
    }
    Some(Child { process: pi.process, thread: pi.thread, pid: pi.pid, tid: pi.tid })
}

/// Resume a suspended child's main thread.
fn resume(c: &Child) {
    // SAFETY: the thread handle of the child this test started.
    unsafe { ResumeThread(c.thread) };
}

/// winbase.h `PROCESS_CREATION_MITIGATION_POLICY_PROHIBIT_DYNAMIC_CODE_ALWAYS_ON`.
const PROHIBIT_DYNAMIC_CODE: u64 = 1 << 36;

// This test's own code stubs (32 bytes each, on pages of their own): `s2c_stub` returns 7, `s2c_stub2` returns 9.
// They stand in for the functions a patcher rewrites in another process; no other code is copied or read.
core::arch::global_asm!(
    ".section .text",
    ".balign 4096",
    ".globl s2c_stub",
    "s2c_stub:",
    "mov eax, 7",
    "ret",
    ".fill 26, 1, 0xcc",
    ".balign 4096",
    ".globl s2c_stub2",
    "s2c_stub2:",
    "mov eax, 9",
    "ret",
    ".fill 26, 1, 0xcc",
    ".balign 4096",
);

unsafe extern "system" {
    fn s2c_stub() -> u32;
    fn s2c_stub2() -> u32;
}

/// Where the parent puts the address of the copy it made in the child (0: none).
#[unsafe(no_mangle)]
static mut S2C_THUNK: usize = 0;

/// The 16 bytes written over a stub: `mov eax, 42; ret` and filler.
const PATCH: [u8; 16] = [0xB8, 42, 0, 0, 0, 0xC3, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC];

fn stub_addr() -> usize {
    s2c_stub as usize
}

fn stub2_addr() -> usize {
    s2c_stub2 as usize
}

fn thunk_var_addr() -> usize {
    &raw const S2C_THUNK as usize
}

/// The first 32 bytes of this process's own copy of a stub.
fn own_bytes(addr: usize) -> [u8; 32] {
    let mut b = [0u8; 32];
    // SAFETY: the stubs are 32 bytes of this image's code, readable.
    unsafe { core::ptr::copy_nonoverlapping(addr as *const u8, b.as_mut_ptr(), 32) };
    b
}

/// What a child started with `run` returns: 100 * stub() + (the thunk's result, 0 without one) + 1000 * stub2() / 9.
fn run_stubs() -> u32 {
    // SAFETY: the stubs are this image's code (possibly rewritten by the parent with the bytes above); the thunk address
    // is 0 or a copy of a stub the parent wrote into executable memory of this process.
    unsafe {
        let s = s2c_stub();
        let th = core::ptr::read_volatile(&raw const S2C_THUNK);
        let t = if th == 0 { 0 } else { core::mem::transmute::<usize, extern "system" fn() -> u32>(th)() };
        let s2 = s2c_stub2();
        s * 100 + t + 1000 * (s2 / 9)
    }
}

main_with_child!(child, test);

fn child(args: &str) -> u32 {
    match args {
        "run" => run_stubs(),
        _ => 0xEEEE,
    }
}

fn steps(t: &mut Test, label: &str, policy: u64) {
    let Some(c) = start_child_policy("run", policy) else {
        say!(t, "{label}: CreateProcessW failed {}", last_error());
        return;
    };
    let p = c.process;
    mark_error();
    // SAFETY: the child's address space only.
    let x =
        unsafe { VirtualAllocEx(p, core::ptr::null_mut(), 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) }
            as usize;
    say!(
        t,
        "{label}: VirtualAllocEx PAGE_EXECUTE_READWRITE: non-null={} error={}",
        x != 0,
        if x != 0 { Buf::<24>::new() } else { err_text() }.as_str()
    );
    // SAFETY: as above.
    let rw =
        unsafe { VirtualAllocEx(p, core::ptr::null_mut(), 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) } as usize;
    say!(t, "{label}: VirtualAllocEx PAGE_READWRITE: non-null={}", rw != 0);
    let (ok, old, e) = protect_other(p, rw, 0x1000, PAGE_EXECUTE_READ);
    say!(t, "{label}: private page to PAGE_EXECUTE_READ: ok={ok} old={old:#x} error={}", if ok { 0 } else { e });
    let s = stub_addr();
    let (ok, old, e) = protect_other(p, s, 16, PAGE_WRITECOPY);
    say!(t, "{label}: image code to PAGE_WRITECOPY: ok={ok} old={old:#x} error={}", if ok { 0 } else { e });
    let (ok, put, e) = write_other(p, s, &PATCH);
    say!(t, "{label}: write 16 bytes: ok={ok} copied={put} error={}", if ok { 0 } else { e });
    let (ok, old, e) = protect_other(p, s, 16, PAGE_EXECUTE_READ);
    say!(t, "{label}: back to PAGE_EXECUTE_READ: ok={ok} old={old:#x} error={}", if ok { 0 } else { e });
    let m = query_ex(p, s);
    say!(t, "{label}: stub page {}", region_text(&m).as_str());
    let (ok, put, e) = write_other(p, stub2_addr(), &PATCH);
    say!(
        t,
        "{label}: WriteProcessMemory to the other stub without changing it first: ok={ok} copied={put} error={}",
        if ok { 0 } else { e }
    );
    resume(&c);
    let code = c.finish();
    say!(t, "{label}: child ran: exit code {code}");
}

fn test(t: &mut Test) {
    SAY_PART.store(3, core::sync::atomic::Ordering::Relaxed);
    steps(t, "no policy", 0);
    steps(t, "prohibit dynamic code", PROHIBIT_DYNAMIC_CODE);
}
