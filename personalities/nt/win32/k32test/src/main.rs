//! `k32test.exe` (ADR 0079): the program the kernel's ring-3 test
//! `nt_kernel32_program_runs_through_forwarders_to_kernelbase` runs. It
//! imports only from `kernel32.dll`, so every call goes through
//! `kernel32`'s API-set forwarders to `kernelbase`, and checks the
//! documented behaviour of the first two slices of the Win32 API (Learn's
//! pages). Started with `--child1` or `--child2` on its command line it
//! is the child its own `processes` check creates.
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `k32test: step N failed (last error E)` on its standard
//! output and exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;
const INVALID_HANDLE_VALUE: Handle = -1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, ov: *mut u8) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const u8,
        disp: u32,
        flags: u32,
        t: Handle,
    ) -> Handle;
    fn CreateFileA(name: *const u8, access: u32, share: u32, sa: *const u8, disp: u32, flags: u32, t: Handle)
    -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn GetFileSize(h: Handle, high: *mut u32) -> u32;
    fn GetFileSizeEx(h: Handle, size: *mut i64) -> i32;
    fn SetFilePointer(h: Handle, low: i32, high: *mut i32, method: u32) -> u32;
    fn SetFilePointerEx(h: Handle, dist: i64, new: *mut i64, method: u32) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn CreateDirectoryW(name: *const u16, sa: *const u8) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn FindFirstFileW(pattern: *const u16, data: *mut u8) -> Handle;
    fn FindNextFileW(h: Handle, data: *mut u8) -> i32;
    fn FindClose(h: Handle) -> i32;
    fn GetFullPathNameW(name: *const u16, size: u32, buf: *mut u16, part: *mut *mut u16) -> u32;
    fn GetCurrentDirectoryW(size: u32, buf: *mut u16) -> u32;
    fn SetCurrentDirectoryW(path: *const u16) -> i32;
    fn GetTempPathW(size: u32, buf: *mut u16) -> u32;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThread() -> Handle;
    fn GetCurrentThreadId() -> u32;
    fn CreateThread(
        sa: *const u8,
        stack: usize,
        start: extern "system" fn(*mut u8) -> u32,
        p: *mut u8,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
    fn GetExitCodeThread(h: Handle, code: *mut u32) -> i32;
    fn Sleep(ms: u32);
    fn GetCommandLineW() -> *const u16;
    fn GetCommandLineA() -> *const u8;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn GetModuleFileNameW(m: *mut u8, buf: *mut u16, size: u32) -> u32;
    fn LoadLibraryW(name: *const u16) -> *mut u8;
    fn GetProcAddress(m: *mut u8, name: *const u8) -> *mut u8;
    fn FreeLibrary(m: *mut u8) -> i32;
    fn VirtualAlloc(addr: *mut u8, size: usize, kind: u32, protect: u32) -> *mut u8;
    fn VirtualFree(addr: *mut u8, size: usize, kind: u32) -> i32;
    fn VirtualProtect(addr: *mut u8, size: usize, protect: u32, old: *mut u32) -> i32;
    fn VirtualQuery(addr: *const u8, info: *mut u8, len: usize) -> usize;
    fn HeapCreate(opts: u32, initial: usize, max: usize) -> *mut u8;
    fn HeapDestroy(h: *mut u8) -> i32;
    fn HeapAlloc(h: *mut u8, flags: u32, n: usize) -> *mut u8;
    fn HeapFree(h: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn HeapReAlloc(h: *mut u8, flags: u32, p: *mut u8, n: usize) -> *mut u8;
    fn HeapSize(h: *mut u8, flags: u32, p: *const u8) -> usize;
    fn GetProcessHeap() -> *mut u8;
    fn LocalAlloc(flags: u32, n: usize) -> *mut u8;
    fn LocalFree(p: *mut u8) -> *mut u8;
    fn GlobalAlloc(flags: u32, n: usize) -> *mut u8;
    fn GlobalFree(p: *mut u8) -> *mut u8;
    fn CreateEventW(sa: *const u8, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(h: Handle) -> i32;
    fn ResetEvent(h: Handle) -> i32;
    fn CreateMutexW(sa: *const u8, owner: i32, name: *const u16) -> Handle;
    fn ReleaseMutex(h: Handle) -> i32;
    fn CreateSemaphoreW(sa: *const u8, initial: i32, max: i32, name: *const u16) -> Handle;
    fn ReleaseSemaphore(h: Handle, n: i32, prev: *mut i32) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn WaitForMultipleObjects(n: u32, hs: *const Handle, all: i32, ms: u32) -> u32;
    fn InitializeCriticalSection(cs: *mut u8);
    fn EnterCriticalSection(cs: *mut u8);
    fn LeaveCriticalSection(cs: *mut u8);
    fn DeleteCriticalSection(cs: *mut u8);
    fn InitializeSRWLock(l: *mut usize);
    fn AcquireSRWLockExclusive(l: *mut usize);
    fn ReleaseSRWLockExclusive(l: *mut usize);
    fn AcquireSRWLockShared(l: *mut usize);
    fn ReleaseSRWLockShared(l: *mut usize);
    fn TryAcquireSRWLockExclusive(l: *mut usize) -> u8;
    fn GetTickCount() -> u32;
    fn GetTickCount64() -> u64;
    fn QueryPerformanceCounter(c: *mut i64) -> i32;
    fn QueryPerformanceFrequency(f: *mut i64) -> i32;
    fn GetSystemTimeAsFileTime(t: *mut u64);
    fn GetVersion() -> u32;
    fn GetVersionExW(info: *mut u8) -> i32;
    fn GetSystemInfo(info: *mut u8);
    // ADR 0114.
    fn GetActiveProcessorGroupCount() -> u16;
    fn GetActiveProcessorCount(group: u16) -> u32;
    fn SetDefaultDllDirectories(flags: u32) -> i32;
    fn FormatMessageW(flags: u32, src: *const u8, id: u32, lang: u32, buf: *mut u16, size: u32, args: *const u8)
    -> u32;
    fn FormatMessageA(flags: u32, src: *const u8, id: u32, lang: u32, buf: *mut u8, size: u32, args: *const u8) -> u32;
    fn IsDebuggerPresent() -> i32;

    // The second slice (ADR 0079).
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const u8,
        ta: *const u8,
        inherit: i32,
        flags: u32,
        env: *const u8,
        cwd: *const u16,
        si: *const u8,
        pi: *mut u8,
    ) -> i32;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
    fn GetProcessId(h: Handle) -> u32;
    fn TlsAlloc() -> u32;
    fn TlsFree(i: u32) -> i32;
    fn TlsGetValue(i: u32) -> usize;
    fn TlsSetValue(i: u32, v: usize) -> i32;
    fn FlsAlloc(cb: Option<extern "system" fn(usize)>) -> u32;
    fn FlsFree(i: u32) -> i32;
    fn FlsGetValue(i: u32) -> usize;
    fn FlsSetValue(i: u32, v: usize) -> i32;
    fn GetACP() -> u32;
    fn GetOEMCP() -> u32;
    fn IsValidCodePage(cp: u32) -> i32;
    fn GetCPInfo(cp: u32, info: *mut u8) -> i32;
    fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, d: *mut u16, dn: i32) -> i32;
    fn WideCharToMultiByte(
        cp: u32,
        flags: u32,
        s: *const u16,
        n: i32,
        d: *mut u8,
        dn: i32,
        def: *const u8,
        used: *mut i32,
    ) -> i32;
    fn CompareStringOrdinal(a: *const u16, an: i32, b: *const u16, bn: i32, ignore: i32) -> i32;
    fn lstrlenW(s: *const u16) -> i32;
    fn lstrcmpW(a: *const u16, b: *const u16) -> i32;
    fn lstrcmpiW(a: *const u16, b: *const u16) -> i32;
    fn CreateFileMappingW(f: Handle, sa: *const u8, protect: u32, hi: u32, lo: u32, name: *const u16) -> Handle;
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn MapViewOfFile(m: Handle, access: u32, hi: u32, lo: u32, n: usize) -> *mut u8;
    fn MapViewOfFileEx(m: Handle, access: u32, hi: u32, lo: u32, n: usize, base: *mut u8) -> *mut u8;
    fn UnmapViewOfFile(p: *const u8) -> i32;
    fn FlushViewOfFile(p: *const u8, n: usize) -> i32;
    fn InitializeConditionVariable(cv: *mut usize);
    fn SleepConditionVariableSRW(cv: *mut usize, l: *mut usize, ms: u32, flags: u32) -> i32;
    fn SleepConditionVariableCS(cv: *mut usize, cs: *mut u8, ms: u32) -> i32;
    fn WakeAllConditionVariable(cv: *mut usize);
    fn WakeConditionVariable(cv: *mut usize);
    fn InitOnceExecuteOnce(
        once: *mut usize,
        f: extern "system" fn(*mut usize, *mut u8, *mut *mut u8) -> i32,
        p: *mut u8,
        ctx: *mut *mut u8,
    ) -> i32;
    fn GetFileInformationByHandle(h: Handle, info: *mut u8) -> i32;
    fn GetFileTime(h: Handle, c: *mut u64, a: *mut u64, w: *mut u64) -> i32;
    fn SetFileTime(h: Handle, c: *const u64, a: *const u64, w: *const u64) -> i32;
    fn FileTimeToSystemTime(ft: *const u64, st: *mut u16) -> i32;
    fn SystemTimeToFileTime(st: *const u16, ft: *mut u64) -> i32;
    fn CompareFileTime(a: *const u64, b: *const u64) -> i32;
    fn GetSystemTime(st: *mut u16);
}

/// The exit status when every check passed.
const PASS: u32 = 0x4B;

/// A NUL-terminated UTF-16 copy of an ASCII literal.
macro_rules! w {
    ($s:literal) => {{
        const S: &[u8] = $s.as_bytes();
        const W: [u16; S.len() + 1] = {
            let mut o = [0u16; S.len() + 1];
            let mut i = 0;
            while i < S.len() {
                o[i] = S[i] as u16;
                i += 1;
            }
            o
        };
        W.as_ptr()
    }};
}

mod basic;
mod chrome2;
mod inherit;
mod sevenzip;
mod slice2;

use basic::*;
use inherit::*;
use sevenzip::*;
use slice2::*;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_DIR_NOT_EMPTY: u32 = 145;
const ERROR_ENVVAR_NOT_FOUND: u32 = 203;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_PROC_NOT_FOUND: u32 = 127;
const ERROR_NOT_OWNER: u32 = 288;
const ERROR_TOO_MANY_POSTS: u32 = 298;
const ERROR_NEGATIVE_SEEK: u32 = 131;

// ----- reporting ---------------------------------------------------------------------

fn out(bytes: &[u8]) {
    // SAFETY: kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h != 0 && h != INVALID_HANDLE_VALUE {
            let mut n = 0;
            WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut());
        }
    }
}

fn out_num(mut v: u32) {
    let mut b = [0u8; 10];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out(&b[i..]);
}

static STEP: AtomicU32 = AtomicU32::new(0);

/// Check `ok` as the next step; on failure report and exit.
fn check(ok: bool) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        // SAFETY: plain kernel32 calls.
        let e = unsafe { GetLastError() };
        out(b"k32test: step ");
        out_num(step);
        out(b" failed (last error ");
        out_num(e);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

/// `check` that the last error is `want`.
fn check_error(want: u32) {
    // SAFETY: a plain kernel32 call.
    check(unsafe { GetLastError() } == want);
}

fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: NUL-terminated strings from kernel32 or our literals.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

fn wslice<'a>(p: *const u16) -> &'a [u16] {
    // SAFETY: as above.
    unsafe { core::slice::from_raw_parts(p, wlen(p)) }
}

fn ascii_eq(w: &[u16], a: &[u8]) -> bool {
    w.len() == a.len() && w.iter().zip(a).all(|(&x, &y)| x == u16::from(y))
}

/// `FormatMessageW` / `FormatMessageA` (ADR 0101): HRESULTs, an allocated
/// buffer, errors, inserts from an argument array and a `va_list`, line
/// widths, module message tables and the ANSI forms.
fn messages() {
    const FROM_SYSTEM: u32 = 0x1000;
    const FROM_STRING: u32 = 0x400;
    const FROM_HMODULE: u32 = 0x800;
    const IGNORE: u32 = 0x200;
    const ALLOCATE: u32 = 0x100;
    const ARRAY: u32 = 0x2000;
    let null = core::ptr::null();
    let mut msg = [0u16; 256];
    let fm = |flags: u32, src: *const u8, id: u32, lang: u32, msg: &mut [u16], args: *const u8| {
        // SAFETY: a buffer of `msg.len()` units; the arguments match the
        // inserts.
        let n = unsafe { FormatMessageW(flags, src, id, lang, msg.as_mut_ptr(), msg.len() as u32, args) };
        n as usize
    };
    // An HRESULT_FROM_WIN32 value has its Win32 code's text.
    let n = fm(FROM_SYSTEM | IGNORE, null, 0x8007_0002, 0, &mut msg, null);
    check(ascii_eq(&msg[..n], b"The system cannot find the file specified.\r\n"));
    // A code from the networking range, in English (United States).
    let n = fm(FROM_SYSTEM | IGNORE, null, 10061, 0x0409, &mut msg, null);
    check(ascii_eq(&msg[..n], b"No connection could be made because the target machine actively refused it.\r\n"));
    // Unknown messages and languages.
    check(fm(FROM_SYSTEM | IGNORE, null, 0xDEAD, 0, &mut msg, null) == 0);
    check_error(317);
    check(fm(FROM_SYSTEM | IGNORE, null, 2, 0x0407, &mut msg, null) == 0);
    check_error(1815);
    // An allocated buffer (LocalAlloc'd, LocalFree'd).
    let mut p: *mut u16 = core::ptr::null_mut();
    // SAFETY: `lpBuffer` is the address of a pointer with ALLOCATE_BUFFER.
    let n = unsafe { FormatMessageW(FROM_SYSTEM | IGNORE | ALLOCATE, null, 5, 0, (&raw mut p).cast(), 0, null) };
    check(n == 19 && !p.is_null());
    // SAFETY: FormatMessageW wrote `n` units and a NUL at `p`.
    check(ascii_eq(unsafe { core::slice::from_raw_parts(p, n as usize + 1) }, b"Access is denied.\r\n\0"));
    // SAFETY: the buffer FormatMessageW allocated.
    check(unsafe { LocalFree(p.cast()) }.is_null());
    // MAX_WIDTH_MASK: the text's line break becomes a space.
    let n = fm(FROM_SYSTEM | IGNORE | 0xFF, null, 2, 0, &mut msg, null);
    check(ascii_eq(&msg[..n], b"The system cannot find the file specified. "));
    // Inserts: Learn's argument-array and va_list examples, and a system
    // message's insert.
    let (bill, bob) = (w!("Bill"), w!("Bob"));
    let array: [usize; 6] = [4, 2, bill as usize, bob as usize, 6, bill as usize];
    let n = fm(FROM_STRING | ARRAY, w!("%1!*.*s! %4 %5!*s!").cast(), 0, 0, &mut msg, array.as_ptr().cast());
    check(ascii_eq(&msg[..n], b"  Bi Bob   Bill"));
    let va = array.as_ptr();
    let n = fm(FROM_STRING, w!("%1!*.*s! %3 %4!*s!").cast(), 0, 0, &mut msg, (&raw const va).cast());
    check(ascii_eq(&msg[..n], b"  Bi Bob   Bill"));
    let one: [usize; 2] = [w!("x.exe") as usize, 0x2A];
    let n = fm(FROM_SYSTEM | ARRAY, null, 193, 0, &mut msg, one.as_ptr().cast());
    check(ascii_eq(&msg[..n], b"x.exe is not a valid Win32 application.\r\n"));
    let n = fm(FROM_STRING | ARRAY, w!("[%2!04X!]%%%n%0tail").cast(), 0, 0, &mut msg, one.as_ptr().cast());
    check(ascii_eq(&msg[..n], b"[002A]%\r\n"));
    // An insert without arguments.
    check(fm(FROM_SYSTEM, null, 193, 0, &mut msg, null) == 0);
    check_error(87);
    // This program has no message table; with FROM_SYSTEM the system's is
    // searched next.
    check(fm(FROM_HMODULE | IGNORE, null, 2, 0, &mut msg, null) == 0);
    check_error(1813);
    let n = fm(FROM_HMODULE | FROM_SYSTEM | IGNORE, null, 2, 0, &mut msg, null);
    check(ascii_eq(&msg[..n], b"The system cannot find the file specified.\r\n"));
    // FormatMessageA: ANSI text, a plain insert is ANSI, `!S!` wide.
    let mut a = [0u8; 128];
    // SAFETY: a buffer of 128 bytes.
    let n = unsafe { FormatMessageA(FROM_SYSTEM | IGNORE, null, 0x8007_0005, 0, a.as_mut_ptr(), 128, null) };
    check(&a[..n as usize + 1] == b"Access is denied.\r\n\0");
    let args: [usize; 2] = [c"abc".as_ptr() as usize, w!("def") as usize];
    // SAFETY: as above; the inserts are an ANSI and a wide string.
    let n = unsafe {
        FormatMessageA(
            FROM_STRING | ARRAY,
            c"%1 and %2!S!".as_ptr().cast(),
            0,
            0,
            a.as_mut_ptr(),
            128,
            args.as_ptr().cast(),
        )
    };
    check(&a[..n as usize] == b"abc and def");
    // SAFETY: as above, with a too small buffer.
    check(unsafe { FormatMessageA(FROM_SYSTEM | IGNORE, null, 2, 0, a.as_mut_ptr(), 4, null) } == 0);
    check_error(122);
    let mut pa: *mut u8 = core::ptr::null_mut();
    // SAFETY: an allocated ANSI buffer.
    let n = unsafe { FormatMessageA(FROM_SYSTEM | IGNORE | ALLOCATE, null, 5, 0, (&raw mut pa).cast(), 64, null) };
    // SAFETY: FormatMessageA wrote `n` bytes and a NUL at `pa`.
    check(n == 19 && unsafe { core::slice::from_raw_parts(pa, 20) } == b"Access is denied.\r\n\0");
    // SAFETY: the buffer FormatMessageA allocated.
    check(unsafe { LocalFree(pa) }.is_null());
}

fn ends_with_ignore_case(w: &[u16], a: &[u8]) -> bool {
    w.len() >= a.len()
        && w[w.len() - a.len()..].iter().zip(a).all(|(&x, &y)| (x as u8).eq_ignore_ascii_case(&y) && x < 0x80)
}

// ----- the checks ----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn k32test_main(_peb: *mut u8) -> u32 {
    // SAFETY: a kernel32 call.
    let cmd = wslice(unsafe { GetCommandLineW() });
    if contains(cmd, b"--child") {
        // SAFETY: as below.
        unsafe { child(cmd) }
    }
    out(b"k32test: start\n");
    // SAFETY: every call is a kernel32 export with valid arguments.
    unsafe {
        errors_and_environment();
        files();
        memory();
        threads_and_sync();
        system_and_modules();
        tls_and_fls();
        code_pages_and_strings();
        file_mappings();
        conditions_and_init_once();
        file_information_and_times();
        seven_zip_imports();
        processes();
        inheritance_and_startup();
        chrome2::run();
        out(b"k32test: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}
