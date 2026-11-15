//! CryptUnprotectMemory across processes: a block protected with SAME_PROCESS (0, Chrome's flag), CROSS_PROCESS (1)
//! and SAME_LOGON (2) by the parent is handed to a child of the same user (as hexadecimal text on its command line)
//! that tries to unprotect it with the same flag. The parent also unprotects it afterwards.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 1 of the observations, the others write the rest.)
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

// ---- crypt32 declarations (Microsoft Learn prototypes; wincrypt.h constants) ----

const CERT_STORE_PROV_MEMORY: usize = 2;
const CERT_STORE_PROV_SYSTEM_W: usize = 10;
const CERT_STORE_PROV_COLLECTION: usize = 11;
const CERT_STORE_PROV_SYSTEM_REGISTRY_W: usize = 13;
const CERT_STORE_READONLY_FLAG: u32 = 0x8000;
const CERT_STORE_OPEN_EXISTING_FLAG: u32 = 0x4000;
const CERT_STORE_CREATE_NEW_FLAG: u32 = 0x2000;
const CERT_STORE_NO_CRYPT_RELEASE_FLAG: u32 = 1;
const CERT_SYSTEM_STORE_CURRENT_USER: u32 = 0x10000;
const CERT_SYSTEM_STORE_LOCAL_MACHINE: u32 = 0x20000;
const CERT_STORE_ADD_NEW: u32 = 1;
const CERT_STORE_ADD_ALWAYS: u32 = 4;
const CERT_STORE_ADD_REPLACE_EXISTING: u32 = 3;
const CERT_STORE_ADD_USE_EXISTING: u32 = 2;
const X509_ASN_ENCODING: u32 = 1;
const CERT_FIND_ANY: u32 = 0;
const CERT_FIND_SHA1_HASH: u32 = 0x10000;
const CERT_FIND_ISSUER_NAME: u32 = 0x20004;
const CERT_FIND_SUBJECT_NAME: u32 = 0x20007;
const CERT_FIND_SUBJECT_STR_W: u32 = 0x80007;
const CERT_FIND_EXISTING: u32 = 0xD0000;
const CERT_STORE_CTRL_RESYNC: u32 = 1;
const CERT_STORE_CTRL_NOTIFY_CHANGE: u32 = 2;
const CERT_STORE_CTRL_CANCEL_NOTIFY: u32 = 4;
const CERT_CLOSE_STORE_FORCE_FLAG: u32 = 1;
const CERT_CLOSE_STORE_CHECK_FLAG: u32 = 2;
const CRYPT_E_NOT_FOUND: u32 = 0x80092004;

#[repr(C)]
struct CryptBlob {
    len: u32,
    data: *mut u8,
}

fn blob(b: &[u8]) -> CryptBlob {
    CryptBlob { len: b.len() as u32, data: b.as_ptr() as *mut u8 }
}

#[link(name = "crypt32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CertOpenStore(prov: usize, enc: u32, hprov: usize, flags: u32, para: *const c_void) -> HANDLE;
    fn CertCloseStore(h: HANDLE, flags: u32) -> BOOL;
    fn CertAddStoreToCollection(coll: HANDLE, sibling: HANDLE, flags: u32, priority: u32) -> BOOL;
    fn CertRemoveStoreFromCollection(coll: HANDLE, sibling: HANDLE);
    fn CertControlStore(h: HANDLE, flags: u32, ctrl: u32, para: *const c_void) -> BOOL;
    fn CertFindCertificateInStore(
        h: HANDLE,
        enc: u32,
        flags: u32,
        ty: u32,
        para: *const c_void,
        prev: *const c_void,
    ) -> *const c_void;
    fn CertEnumCertificatesInStore(h: HANDLE, prev: *const c_void) -> *const c_void;
    fn CertAddEncodedCertificateToStore(
        h: HANDLE,
        enc: u32,
        data: *const u8,
        len: u32,
        disp: u32,
        ctx: *mut *const c_void,
    ) -> BOOL;
    fn CertFreeCertificateContext(c: *const c_void) -> BOOL;
    fn CertDuplicateCertificateContext(c: *const c_void) -> *const c_void;
    fn CertDeleteCertificateFromStore(c: *const c_void) -> BOOL;
    fn CryptProtectMemory(data: *mut u8, len: u32, flags: u32) -> BOOL;
    fn CryptUnprotectMemory(data: *mut u8, len: u32, flags: u32) -> BOOL;
    fn CryptProtectData(
        input: *const CryptBlob,
        desc: *const u16,
        entropy: *const CryptBlob,
        reserved: *const c_void,
        prompt: *const c_void,
        flags: u32,
        out: *mut CryptBlob,
    ) -> BOOL;
    fn CryptUnprotectData(
        input: *const CryptBlob,
        desc: *mut *mut u16,
        entropy: *const CryptBlob,
        reserved: *const c_void,
        prompt: *const c_void,
        flags: u32,
        out: *mut CryptBlob,
    ) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(h: *mut c_void) -> *mut c_void;
    fn LocalSize(h: *const c_void) -> usize;
}

/// The last error as text for the crypt tests: unchanged / hex for HRESULT-like codes / decimal.
fn cerr() -> Buf<24> {
    let e = last_error();
    let mut b = Buf::<24>::new();
    if e == MARK {
        let _ = write!(b, "unchanged");
    } else if e >= 0x8000_0000 {
        let _ = write!(b, "0x{e:08X}");
    } else {
        let _ = write!(b, "{e}");
    }
    b
}

fn nn(p: *const c_void) -> u32 {
    u32::from(!p.is_null())
}

fn nnh(p: HANDLE) -> u32 {
    u32::from(!p.is_null())
}

/// Open a memory store holding the two test certificates (A and B), or only those asked for.
fn memstore(a: bool, b: bool) -> HANDLE {
    // SAFETY: plain calls with live arguments.
    unsafe {
        let s = CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, core::ptr::null());
        if a {
            CertAddEncodedCertificateToStore(
                s,
                X509_ASN_ENCODING,
                CERT_A.as_ptr(),
                CERT_A.len() as u32,
                CERT_STORE_ADD_ALWAYS,
                core::ptr::null_mut(),
            );
        }
        if b {
            CertAddEncodedCertificateToStore(
                s,
                X509_ASN_ENCODING,
                CERT_B.as_ptr(),
                CERT_B.len() as u32,
                CERT_STORE_ADD_ALWAYS,
                core::ptr::null_mut(),
            );
        }
        s
    }
}

/// Which test certificate a context is: 'A', 'B' or '?' (by its encoded bytes), '-' for NULL.
fn which(c: *const c_void) -> char {
    if c.is_null() {
        return '-';
    }
    // SAFETY: a PCCERT_CONTEXT: encoding type u32, then the encoded bytes pointer and length.
    unsafe {
        let enc = *(c as *const *const u8).add(1);
        let len = *((c as *const u32).add(4)) as usize;
        let bytes = core::slice::from_raw_parts(enc, len);
        if bytes == CERT_A {
            'A'
        } else if bytes == CERT_B {
            'B'
        } else {
            '?'
        }
    }
}

/// The subject name blob and the issuer name blob of a context's CERT_INFO (layout of wincrypt.h x64).
fn subject_blob(c: *const c_void) -> CryptBlob {
    // SAFETY: a PCCERT_CONTEXT whose pCertInfo is at byte 24; CERT_INFO's Subject at byte 80.
    unsafe {
        let info = *(c as *const *const u8).add(3);
        let len = *(info.add(80) as *const u32);
        let data = *(info.add(88) as *const *mut u8);
        CryptBlob { len, data }
    }
}

fn sha1_a() -> [u8; 20] {
    SHA1_A
}

// Two small self-signed test certificates (EC P-256, CN "Poly Test A" and "Poly Test B", made once with openssl).
const CERT_A: &[u8] = &[
    48, 130, 1, 79, 48, 129, 246, 160, 3, 2, 1, 2, 2, 2, 3, 233, 48, 10, 6, 8, 42, 134, 72, 206, 61, 4, 3, 2, 48, 22,
    49, 20, 48, 18, 6, 3, 85, 4, 3, 12, 11, 80, 111, 108, 121, 32, 84, 101, 115, 116, 32, 65, 48, 32, 23, 13, 50, 54,
    49, 49, 49, 52, 50, 51, 50, 55, 50, 54, 90, 24, 15, 50, 49, 50, 54, 49, 48, 50, 49, 50, 51, 50, 55, 50, 54, 90, 48,
    22, 49, 20, 48, 18, 6, 3, 85, 4, 3, 12, 11, 80, 111, 108, 121, 32, 84, 101, 115, 116, 32, 65, 48, 89, 48, 19, 6, 7,
    42, 134, 72, 206, 61, 2, 1, 6, 8, 42, 134, 72, 206, 61, 3, 1, 7, 3, 66, 0, 4, 208, 198, 239, 99, 167, 165, 64, 112,
    234, 81, 75, 49, 219, 236, 125, 5, 249, 95, 148, 67, 80, 19, 156, 21, 87, 226, 170, 211, 115, 206, 64, 174, 254,
    140, 14, 37, 74, 4, 5, 172, 50, 41, 125, 175, 158, 211, 75, 166, 2, 45, 248, 242, 26, 57, 10, 32, 174, 120, 29,
    165, 234, 84, 87, 54, 163, 50, 48, 48, 48, 15, 6, 3, 85, 29, 19, 1, 1, 255, 4, 5, 48, 3, 1, 1, 255, 48, 29, 6, 3,
    85, 29, 14, 4, 22, 4, 20, 1, 214, 91, 162, 31, 143, 126, 18, 230, 78, 36, 195, 49, 12, 83, 136, 123, 251, 255, 74,
    48, 10, 6, 8, 42, 134, 72, 206, 61, 4, 3, 2, 3, 72, 0, 48, 69, 2, 33, 0, 205, 17, 54, 35, 130, 53, 129, 100, 221,
    197, 61, 169, 203, 195, 106, 0, 236, 186, 122, 187, 174, 140, 164, 40, 6, 65, 102, 12, 26, 141, 33, 195, 2, 32, 37,
    142, 207, 149, 189, 122, 122, 38, 96, 36, 136, 212, 245, 144, 18, 10, 40, 14, 124, 12, 61, 110, 149, 103, 81, 196,
    252, 238, 104, 168, 247, 38,
];
const CERT_B: &[u8] = &[
    48, 130, 1, 80, 48, 129, 246, 160, 3, 2, 1, 2, 2, 2, 3, 234, 48, 10, 6, 8, 42, 134, 72, 206, 61, 4, 3, 2, 48, 22,
    49, 20, 48, 18, 6, 3, 85, 4, 3, 12, 11, 80, 111, 108, 121, 32, 84, 101, 115, 116, 32, 66, 48, 32, 23, 13, 50, 54,
    49, 49, 49, 52, 50, 51, 50, 55, 50, 55, 90, 24, 15, 50, 49, 50, 54, 49, 48, 50, 49, 50, 51, 50, 55, 50, 55, 90, 48,
    22, 49, 20, 48, 18, 6, 3, 85, 4, 3, 12, 11, 80, 111, 108, 121, 32, 84, 101, 115, 116, 32, 66, 48, 89, 48, 19, 6, 7,
    42, 134, 72, 206, 61, 2, 1, 6, 8, 42, 134, 72, 206, 61, 3, 1, 7, 3, 66, 0, 4, 37, 218, 51, 75, 146, 177, 91, 78,
    109, 118, 99, 254, 3, 236, 200, 222, 140, 246, 96, 52, 171, 234, 249, 7, 59, 235, 9, 112, 245, 41, 147, 58, 138,
    198, 140, 105, 26, 53, 158, 198, 10, 220, 228, 114, 137, 221, 235, 77, 221, 132, 251, 204, 147, 146, 228, 135, 153,
    139, 50, 95, 88, 85, 20, 94, 163, 50, 48, 48, 48, 15, 6, 3, 85, 29, 19, 1, 1, 255, 4, 5, 48, 3, 1, 1, 255, 48, 29,
    6, 3, 85, 29, 14, 4, 22, 4, 20, 120, 129, 43, 141, 23, 178, 79, 15, 26, 23, 199, 156, 87, 244, 241, 69, 186, 136,
    35, 227, 48, 10, 6, 8, 42, 134, 72, 206, 61, 4, 3, 2, 3, 73, 0, 48, 70, 2, 33, 0, 137, 237, 0, 98, 241, 8, 72, 130,
    206, 3, 12, 192, 31, 142, 123, 128, 18, 202, 142, 83, 72, 147, 69, 187, 38, 254, 174, 196, 198, 106, 163, 98, 2,
    33, 0, 237, 176, 171, 156, 237, 90, 194, 251, 78, 92, 218, 228, 205, 74, 180, 4, 30, 24, 60, 215, 11, 52, 212, 116,
    50, 138, 107, 252, 241, 126, 48, 147,
];
const SHA1_A: [u8; 20] = [131, 87, 177, 180, 2, 226, 39, 216, 169, 96, 160, 201, 203, 84, 46, 83, 89, 209, 210, 94];
const SHA1_B: [u8; 20] =
    [94, 87, 212, 106, 252, 80, 185, 118, 216, 33, 178, 27, 183, 253, 190, 175, 193, 191, 146, 103];

/// The letters of the certificates a store enumerates, in order (`AB`, `-` when it is empty).
fn enum_all(s: HANDLE) -> Buf<16> {
    let mut b = Buf::<16>::new();
    // SAFETY: contexts are passed back to the enumeration, which frees them.
    unsafe {
        let mut c = CertEnumCertificatesInStore(s, core::ptr::null());
        let mut n = 0;
        while !c.is_null() && n < 10 {
            let _ = write!(b, "{}", which(c));
            c = CertEnumCertificatesInStore(s, c);
            n += 1;
        }
    }
    if b.n == 0 {
        let _ = write!(b, "-");
    }
    b
}

/// Wait for an event handle at most `ms`; 1 when it was signalled.
fn waitev(h: HANDLE, ms: u32) -> u32 {
    // SAFETY: a live event handle.
    unsafe { u32::from(WaitForSingleObject(h, ms) == 0) }
}

main_with_child!(child, test);

fn pattern() -> [u8; 32] {
    core::array::from_fn(|i| (i as u8).wrapping_mul(5).wrapping_add(9))
}

fn child(args: &str) -> u32 {
    let h = arg_handle(args, 0);
    let mut out = ChildOut { h };
    let flag: u32 = args.split(' ').nth(1).and_then(|w| w.parse().ok()).unwrap_or(99);
    let hex = args.split(' ').nth(2).unwrap_or("");
    let mut b = [0u8; 32];
    for i in 0..32 {
        b[i] = u8::from_str_radix(hex.get(2 * i..2 * i + 2).unwrap_or("00"), 16).unwrap_or(0);
    }
    mark_error();
    // SAFETY: a live 32-byte buffer.
    let ok = unsafe { CryptUnprotectMemory(b.as_mut_ptr(), 32, flag) };
    let mut line = Buf::<200>::new();
    let _ = write!(
        line,
        "child unprotect flag {flag}: {} last error {} restored {}",
        ok,
        cerr().as_str(),
        u32::from(b == pattern())
    );
    out.put(line.as_str());
    0
}

fn test(t: &mut Test) {
    SAY_PART.store(1, core::sync::atomic::Ordering::Relaxed);
    for flag in [0u32, 1, 2] {
        let mut b = pattern();
        // SAFETY: a live 32-byte buffer.
        let ok = unsafe { CryptProtectMemory(b.as_mut_ptr(), 32, flag) };
        say!(t, "parent protect flag {flag}: {ok}");
        let mut hex = Buf::<80>::new();
        for x in b {
            let _ = write!(hex, "{x:02x}");
        }
        let rep = Report::new();
        let mut a = Buf::<200>::new();
        let _ = write!(a, "{:x} {flag} {}", rep.write as usize, hex.as_str());
        match start_child(a.as_str(), 0) {
            Some(c) => {
                let code = c.finish();
                rep.print(t);
                say!(t, "child exit code {code}");
            }
            None => {
                say!(t, "child not started");
                close(rep.read);
                close(rep.write);
            }
        }
        // SAFETY: a live 32-byte buffer.
        let back = unsafe { CryptUnprotectMemory(b.as_mut_ptr(), 32, flag) };
        say!(t, "parent unprotect flag {flag}: {back} restored {}", u32::from(b == pattern()));
    }
}
