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
    fn FormatMessageW(flags: u32, src: *const u8, id: u32, lang: u32, buf: *mut u16, size: u32, args: *const u8)
    -> u32;
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

fn ends_with_ignore_case(w: &[u16], a: &[u8]) -> bool {
    w.len() >= a.len()
        && w[w.len() - a.len()..].iter().zip(a).all(|(&x, &y)| (x as u8).eq_ignore_ascii_case(&y) && x < 0x80)
}

// ----- the thread ----------------------------------------------------------------------

/// Shared with the worker thread.
struct Shared {
    go: Handle,
    done: Handle,
    cs: [u8; 40],
    srw: usize,
    counter: u32,
    seen_tid: u32,
}

static mut SHARED: Shared = Shared { go: 0, done: 0, cs: [0; 40], srw: 0, counter: 0, seen_tid: 0 };

extern "system" fn worker(p: *mut u8) -> u32 {
    let s = p as *mut Shared;
    // SAFETY: `s` is the static; the critical section and the SRW lock
    // serialise the counter.
    unsafe {
        if WaitForSingleObject((*s).go, 10_000) != WAIT_OBJECT_0 {
            return 1;
        }
        (*s).seen_tid = GetCurrentThreadId();
        for _ in 0..1000 {
            EnterCriticalSection((&raw mut (*s).cs).cast());
            (*s).counter += 1;
            LeaveCriticalSection((&raw mut (*s).cs).cast());
        }
        AcquireSRWLockExclusive(&raw mut (*s).srw);
        (*s).counter += 1;
        ReleaseSRWLockExclusive(&raw mut (*s).srw);
        SetEvent((*s).done);
    }
    0x77
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
        out(b"k32test: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}

unsafe fn errors_and_environment() {
    // SAFETY: valid arguments throughout.
    unsafe {
        SetLastError(1234);
        check(GetLastError() == 1234);
        // A failing call sets the documented error.
        let h = CreateFileW(w!("C:\\no-such-file.k32"), GENERIC_READ, 0, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_FILE_NOT_FOUND);
        let h = CreateFileW(w!("C:\\no-such-dir\\f.k32"), GENERIC_READ, 0, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_PATH_NOT_FOUND);
        let h = CreateFileW(w!("C:\\x.k32"), GENERIC_READ, 0, core::ptr::null(), 9, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_INVALID_PARAMETER);

        // The environment the kernel gave the process (ADR 0073).
        let mut buf = [0u16; 260];
        let n = GetEnvironmentVariableW(w!("temp"), buf.as_mut_ptr(), 260);
        check(n > 3 && (n as usize) < 260);
        check(GetEnvironmentVariableW(w!("TEMP"), buf.as_mut_ptr(), 2) == n + 1);
        check(GetEnvironmentVariableW(w!("POLY_K32"), buf.as_mut_ptr(), 260) == 0);
        check_error(ERROR_ENVVAR_NOT_FOUND);
        check(SetEnvironmentVariableW(w!("POLY_K32"), w!("yes")) != 0);
        check(GetEnvironmentVariableW(w!("poly_k32"), buf.as_mut_ptr(), 260) == 3);
        check(ascii_eq(&buf[..3], b"yes"));
        // The block holds it too.
        let block = GetEnvironmentStringsW();
        check(!block.is_null());
        let mut p = block as *const u16;
        let mut found = false;
        while *p != 0 {
            let e = wslice(p);
            found |= ascii_eq(e, b"POLY_K32=yes");
            p = p.add(e.len() + 1);
        }
        check(found);
        check(FreeEnvironmentStringsW(block) != 0);
        check(SetEnvironmentVariableW(w!("POLY_K32"), core::ptr::null()) != 0);
        check(GetEnvironmentVariableW(w!("POLY_K32"), buf.as_mut_ptr(), 260) == 0);
        check(SetEnvironmentVariableW(w!("A=B"), w!("x")) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // The command line: the program's path, then the argument.
        let cl = wslice(GetCommandLineW());
        check(ends_with_ignore_case(cl, b" --poly"));
        let cla = GetCommandLineA();
        check(*cla != 0);
    }
}

unsafe fn files() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // Work in the temporary directory.
        let mut temp = [0u16; 260];
        let n = GetTempPathW(260, temp.as_mut_ptr()) as usize;
        check(n > 3 && temp[n - 1] == u16::from(b'\\'));
        check(SetCurrentDirectoryW(temp.as_ptr()) != 0);
        let mut cur = [0u16; 260];
        let c = GetCurrentDirectoryW(260, cur.as_mut_ptr()) as usize;
        check(c == n - 1 && cur[..c] == temp[..c]);
        check(GetCurrentDirectoryW(1, cur.as_mut_ptr()) as usize == c + 1);
        check(SetCurrentDirectoryW(w!("C:\\no-such-dir")) == 0);

        // A directory, twice.
        let _ = DeleteFileW(w!("k32dir\\a.txt"));
        let _ = DeleteFileW(w!("k32dir\\b.txt"));
        let _ = RemoveDirectoryW(w!("k32dir"));
        check(CreateDirectoryW(w!("k32dir"), core::ptr::null()) != 0);
        check(CreateDirectoryW(w!("K32DIR"), core::ptr::null()) == 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(GetFileAttributesW(w!("k32dir")) & FILE_ATTRIBUTE_DIRECTORY != 0);

        // A file: create, write, size, seek, read, EOF.
        let h =
            CreateFileW(w!("k32dir\\a.txt"), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null(), CREATE_NEW, 0x80, 0);
        check(h != INVALID_HANDLE_VALUE);
        check_error(0);
        let text = b"hello, kernel32";
        let mut done = 0u32;
        check(WriteFile(h, text.as_ptr(), text.len() as u32, &mut done, core::ptr::null_mut()) != 0);
        check(done == text.len() as u32);
        let mut high = 7u32;
        check(GetFileSize(h, &mut high) == text.len() as u32 && high == 0);
        let mut size = 0i64;
        check(GetFileSizeEx(h, &mut size) != 0 && size == text.len() as i64);
        check(SetFilePointer(h, 0, core::ptr::null_mut(), 0) == 0);
        let mut back = [0u8; 32];
        check(ReadFile(h, back.as_mut_ptr(), 32, &mut done, core::ptr::null_mut()) != 0);
        check(done == text.len() as u32 && &back[..text.len()] == text);
        // At the end: success with no bytes.
        check(ReadFile(h, back.as_mut_ptr(), 32, &mut done, core::ptr::null_mut()) != 0 && done == 0);
        let mut pos = 0i64;
        check(SetFilePointerEx(h, -5, &mut pos, 1) != 0 && pos == 10);
        check(SetFilePointer(h, -100, core::ptr::null_mut(), 1) == u32::MAX);
        check_error(ERROR_NEGATIVE_SEEK);
        check(CloseHandle(h) != 0);
        check(CloseHandle(h) == 0);

        // CREATE_NEW on an existing file; CREATE_ALWAYS reports it existed.
        let h = CreateFileW(w!("k32dir\\a.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_NEW, 0, 0);
        check(h == INVALID_HANDLE_VALUE);
        check_error(ERROR_FILE_EXISTS);
        let h = CreateFileA(c"k32dir\\A.TXT".as_ptr().cast(), GENERIC_READ, 1, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        check(GetFileSize(h, core::ptr::null_mut()) == text.len() as u32);
        check(CloseHandle(h) != 0);
        let h = CreateFileW(w!("k32dir\\b.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        check_error(0);
        check(CloseHandle(h) != 0);
        let h = CreateFileW(w!("k32dir\\b.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        check_error(ERROR_ALREADY_EXISTS);
        check(CloseHandle(h) != 0);

        // Finding files.
        let mut data = [0u8; 592];
        let f = FindFirstFileW(w!("k32dir\\a.*"), data.as_mut_ptr());
        check(f != INVALID_HANDLE_VALUE);
        let name = wslice(data.as_ptr().add(44).cast());
        check(ascii_eq(name, b"a.txt"));
        check(u32::from_le_bytes(data[32..36].try_into().unwrap()) == text.len() as u32);
        check(FindNextFileW(f, data.as_mut_ptr()) == 0);
        check_error(ERROR_NO_MORE_FILES);
        check(FindClose(f) != 0);
        let f = FindFirstFileW(w!("k32dir\\*.*"), data.as_mut_ptr());
        check(f != INVALID_HANDLE_VALUE);
        let mut names = 1;
        while FindNextFileW(f, data.as_mut_ptr()) != 0 {
            names += 1;
        }
        // a.txt and b.txt (and "." and ".." where the volume reports them).
        check((2..=4).contains(&names));
        check(FindClose(f) != 0);
        check(FindFirstFileW(w!("k32dir\\*.none"), data.as_mut_ptr()) == INVALID_HANDLE_VALUE);
        check_error(ERROR_FILE_NOT_FOUND);

        // Full paths.
        let mut full = [0u16; 260];
        let mut part: *mut u16 = core::ptr::null_mut();
        let len = GetFullPathNameW(w!("k32dir\\..\\k32dir\\.\\a.txt"), 260, full.as_mut_ptr(), &mut part) as usize;
        check(len == c + "\\k32dir\\a.txt".len());
        check(full[..c] == cur[..c] && ascii_eq(&full[c..len], b"\\k32dir\\a.txt"));
        check(!part.is_null() && ascii_eq(wslice(part), b"a.txt"));

        // Rename, delete, remove.
        check(MoveFileExW(w!("k32dir\\a.txt"), w!("k32dir\\b.txt"), 0) == 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(MoveFileExW(w!("k32dir\\a.txt"), w!("k32dir\\b.txt"), 1) != 0);
        check(GetFileAttributesW(w!("k32dir\\a.txt")) == INVALID_FILE_ATTRIBUTES);
        check_error(ERROR_FILE_NOT_FOUND);
        check(GetFileAttributesW(w!("k32dir\\b.txt")) != INVALID_FILE_ATTRIBUTES);
        check(RemoveDirectoryW(w!("k32dir")) == 0);
        check_error(ERROR_DIR_NOT_EMPTY);
        check(DeleteFileW(w!("k32dir")) == 0);
        check(DeleteFileW(w!("k32dir\\b.txt")) != 0);
        check(DeleteFileW(w!("k32dir\\b.txt")) == 0);
        check_error(ERROR_FILE_NOT_FOUND);
        check(RemoveDirectoryW(w!("k32dir")) != 0);
        check(GetFileAttributesW(w!("k32dir")) == INVALID_FILE_ATTRIBUTES);
    }
}

unsafe fn memory() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let heap = GetProcessHeap();
        check(!heap.is_null());
        let p = HeapAlloc(heap, 0x8, 100);
        check(!p.is_null() && (p as usize).is_multiple_of(16));
        check((0..100).all(|i| *p.add(i) == 0));
        check(HeapSize(heap, 0, p) == 100);
        p.write_bytes(0xAB, 100);
        let q = HeapReAlloc(heap, 0, p, 300);
        check(!q.is_null() && *q.add(99) == 0xAB && HeapSize(heap, 0, q) == 300);
        check(HeapFree(heap, 0, q) != 0);

        let private = HeapCreate(0, 0, 0);
        check(!private.is_null());
        let r = HeapAlloc(private, 0, 5000);
        check(!r.is_null());
        check(HeapFree(private, 0, r) != 0);
        check(HeapDestroy(private) != 0);

        let l = LocalAlloc(0x40, 64);
        check(!l.is_null() && *l.add(63) == 0);
        check(LocalFree(l).is_null());
        check(LocalAlloc(0x1, 8).is_null());
        check_error(ERROR_INVALID_PARAMETER);
        let g = GlobalAlloc(0x42, 32);
        check(!g.is_null() && *g == 0);
        check(GlobalFree(g).is_null());

        // Virtual memory: commit, write, query, protect, release.
        let v = VirtualAlloc(core::ptr::null_mut(), 0x10000, 0x3000, 0x04);
        check(!v.is_null() && (v as usize).is_multiple_of(0x10000));
        v.add(0x1000).write(42);
        let mut mbi = [0u8; 48];
        check(VirtualQuery(v, mbi.as_mut_ptr(), 48) == 48);
        // MEMORY_BASIC_INFORMATION: State at 32 (MEM_COMMIT), Protect at 36.
        check(u32::from_le_bytes(mbi[32..36].try_into().unwrap()) == 0x1000);
        check(u32::from_le_bytes(mbi[36..40].try_into().unwrap()) == 0x04);
        let mut old = 0u32;
        check(VirtualProtect(v, 0x1000, 0x02, &mut old) != 0 && old == 0x04);
        check(VirtualFree(v, 0x10000, 0x8000) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(VirtualFree(v, 0, 0x8000) != 0);
    }
}

unsafe fn threads_and_sync() {
    // SAFETY: valid arguments throughout; the shared state is a static the
    // worker only touches under the locks or after the events.
    unsafe {
        let s = &raw mut SHARED;
        (*s).go = CreateEventW(core::ptr::null(), 0, 0, core::ptr::null());
        (*s).done = CreateEventW(core::ptr::null(), 1, 0, core::ptr::null());
        check((*s).go != 0 && (*s).done != 0);
        InitializeCriticalSection((&raw mut (*s).cs).cast());
        InitializeSRWLock(&raw mut (*s).srw);
        let mut tid = 0u32;
        let t = CreateThread(core::ptr::null(), 0, worker, s.cast(), 0, &mut tid);
        check(t != 0 && tid != 0 && tid != GetCurrentThreadId());
        let mut code = 0u32;
        check(GetExitCodeThread(t, &mut code) != 0 && code == 259);
        // The worker waits for `go`.
        check(WaitForSingleObject((*s).done, 20) == WAIT_TIMEOUT);
        for _ in 0..500 {
            EnterCriticalSection((&raw mut (*s).cs).cast());
            (*s).counter += 1;
            LeaveCriticalSection((&raw mut (*s).cs).cast());
        }
        check(SetEvent((*s).go) != 0);
        check(WaitForSingleObject((*s).done, 10_000) == WAIT_OBJECT_0);
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0);
        check(GetExitCodeThread(t, &mut code) != 0 && code == 0x77);
        check((*s).seen_tid == tid);
        check((*s).counter == 1501);
        // `go` was auto-reset by the worker's wait; `done` stays set.
        check(WaitForSingleObject((*s).go, 0) == WAIT_TIMEOUT);
        let both = [(*s).go, (*s).done];
        check(WaitForMultipleObjects(2, both.as_ptr(), 0, 0) == 1);
        check(WaitForMultipleObjects(2, both.as_ptr(), 1, 0) == WAIT_TIMEOUT);
        check(ResetEvent((*s).done) != 0);
        check(WaitForSingleObject((*s).done, 0) == WAIT_TIMEOUT);
        check(WaitForMultipleObjects(0, both.as_ptr(), 0, 0) == u32::MAX);
        check_error(ERROR_INVALID_PARAMETER);
        DeleteCriticalSection((&raw mut (*s).cs).cast());
        check(TryAcquireSRWLockExclusive(&raw mut (*s).srw) != 0);
        ReleaseSRWLockExclusive(&raw mut (*s).srw);
        AcquireSRWLockShared(&raw mut (*s).srw);
        check(TryAcquireSRWLockExclusive(&raw mut (*s).srw) == 0);
        ReleaseSRWLockShared(&raw mut (*s).srw);
        for h in [t, (*s).go, (*s).done] {
            check(CloseHandle(h) != 0);
        }

        // Mutexes: owned at creation, released once.
        let m = CreateMutexW(core::ptr::null(), 1, core::ptr::null());
        check(m != 0);
        check(ReleaseMutex(m) != 0);
        check(ReleaseMutex(m) == 0);
        check_error(ERROR_NOT_OWNER);
        check(WaitForSingleObject(m, 0) == WAIT_OBJECT_0);
        check(ReleaseMutex(m) != 0);
        check(CloseHandle(m) != 0);

        // Semaphores.
        let sem = CreateSemaphoreW(core::ptr::null(), 1, 2, core::ptr::null());
        check(sem != 0);
        check(WaitForSingleObject(sem, 0) == WAIT_OBJECT_0);
        check(WaitForSingleObject(sem, 0) == WAIT_TIMEOUT);
        let mut prev = -1;
        check(ReleaseSemaphore(sem, 2, &mut prev) != 0 && prev == 0);
        check(ReleaseSemaphore(sem, 1, core::ptr::null_mut()) == 0);
        check_error(ERROR_TOO_MANY_POSTS);
        check(CloseHandle(sem) != 0);
        check(CreateSemaphoreW(core::ptr::null(), 3, 2, core::ptr::null()) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // A named event: the second create opens the first.
        let a = CreateEventW(core::ptr::null(), 1, 0, w!("Local\\PolyK32Event"));
        check(a != 0);
        check_error(0);
        let b = CreateEventW(core::ptr::null(), 1, 0, w!("PolyK32Event"));
        check(b != 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(SetEvent(a) != 0);
        check(WaitForSingleObject(b, 0) == WAIT_OBJECT_0);
        check(CloseHandle(a) != 0 && CloseHandle(b) != 0);
    }
}

unsafe fn system_and_modules() {
    // SAFETY: valid arguments throughout.
    unsafe {
        check(GetCurrentProcess() == -1 && GetCurrentThread() == -2);
        check(GetCurrentProcessId() != 0 && GetCurrentThreadId() != 0);

        // Time.
        let mut freq = 0i64;
        check(QueryPerformanceFrequency(&mut freq) != 0 && freq == 10_000_000);
        let (mut c1, mut c2) = (0i64, 0i64);
        check(QueryPerformanceCounter(&mut c1) != 0);
        let t1 = GetTickCount64();
        Sleep(30);
        check(QueryPerformanceCounter(&mut c2) != 0);
        let t2 = GetTickCount64();
        check(t2 >= t1 + 20 && c2 - c1 >= 200_000);
        check(GetTickCount().wrapping_sub(t2 as u32) < 1000);
        let mut ft = 0u64;
        GetSystemTimeAsFileTime(&mut ft);
        check(ft != 0);

        // The version: this program has no manifest, so it is told
        // Windows 8 (6.2), as Learn documents for unmanifested programs.
        let v = GetVersion();
        check(v & 0xFF == 6 && (v >> 8) & 0xFF == 2 && v >> 16 == 9200);
        let mut info = [0u8; 284];
        info[..4].copy_from_slice(&284u32.to_le_bytes());
        check(GetVersionExW(info.as_mut_ptr()) != 0);
        check(u32::from_le_bytes(info[4..8].try_into().unwrap()) == 6);
        check(u32::from_le_bytes(info[16..20].try_into().unwrap()) == 2);
        info[..4].copy_from_slice(&100u32.to_le_bytes());
        check(GetVersionExW(info.as_mut_ptr()) == 0);
        let mut si = [0u8; 48];
        GetSystemInfo(si.as_mut_ptr());
        check(u32::from_le_bytes(si[4..8].try_into().unwrap()) == 4096);
        check(u32::from_le_bytes(si[32..36].try_into().unwrap()) >= 1);
        check(IsDebuggerPresent() == 0);

        // Messages.
        let mut msg = [0u16; 128];
        let n = FormatMessageW(0x1200, core::ptr::null(), 2, 0, msg.as_mut_ptr(), 128, core::ptr::null()) as usize;
        check(ascii_eq(&msg[..n], b"The system cannot find the file specified.\r\n"));
        check(FormatMessageW(0x1200, core::ptr::null(), 2, 0, msg.as_mut_ptr(), 8, core::ptr::null()) == 0);

        // Modules: this program, kernel32 (with its forwarders) and
        // kernelbase.
        let me = GetModuleHandleW(core::ptr::null());
        check(!me.is_null() && *me == b'M');
        let mut path = [0u16; 260];
        let n = GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 260) as usize;
        check(ends_with_ignore_case(&path[..n], b"\\PolyK32Test\\k32test.exe"));
        check(GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 4) == 4 && path[3] == 0);
        let k32 = GetModuleHandleW(w!("KERNEL32.DLL"));
        check(!k32.is_null());
        let kb = LoadLibraryW(w!("kernelbase"));
        check(!kb.is_null() && kb != k32);
        // A forwarded export resolves to kernelbase's code.
        let via_k32 = GetProcAddress(k32, c"GetTickCount".as_ptr().cast());
        let direct = GetProcAddress(kb, c"GetTickCount".as_ptr().cast());
        check(!via_k32.is_null() && via_k32 == direct);
        // kernel32's own export stays in kernel32.
        check(!GetProcAddress(k32, c"GlobalAlloc".as_ptr().cast()).is_null());
        check(GetProcAddress(k32, c"NoSuchFunction".as_ptr().cast()).is_null());
        check_error(ERROR_PROC_NOT_FOUND);
        check(FreeLibrary(kb) != 0);
        check(LoadLibraryW(w!("no-such-module.dll")).is_null());
        check_error(ERROR_MOD_NOT_FOUND);
        check(GetModuleHandleW(w!("no-such-module.dll")).is_null());
        check_error(ERROR_MOD_NOT_FOUND);
    }
}

// ----- the second slice (ADR 0079) -------------------------------------------------------------

const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
const STILL_ACTIVE: u32 = 259;
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_WRITE: u32 = 0x2;
const FILE_MAP_READ: u32 = 0x4;
const FILE_MAP_ALL_ACCESS: u32 = 0xF_001F;
const FILE_BEGIN: u32 = 0;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_UNICODE_TRANSLATION: u32 = 1113;
const ERROR_MAPPED_ALIGNMENT: u32 = 1132;
const ERROR_TIMEOUT: u32 = 1460;

/// A null pointer of any type.
fn np<T>() -> *const T {
    core::ptr::null()
}

/// Whether `w` contains the ASCII text `a`.
fn contains(w: &[u16], a: &[u8]) -> bool {
    w.windows(a.len()).any(|win| ascii_eq(win, a))
}

/// `s` (ASCII, may hold NULs) and a NUL into `dst`.
fn fill(dst: &mut [u16], s: &[u8]) {
    for (d, &c) in dst.iter_mut().zip(s) {
        *d = u16::from(c);
    }
    dst[s.len()] = 0;
}

/// The child `processes` starts: `--child1` checks what its creator passed
/// explicitly, `--child2` what it inherited; `--child3` and `--child4` are
/// `inheritance_and_startup`'s.
unsafe fn child(cmd: &[u16]) -> ! {
    if contains(cmd, b"--child3") {
        // SAFETY: as below.
        unsafe { child3(cmd) }
    }
    if contains(cmd, b"--child4") {
        // SAFETY: as below.
        unsafe { child4() }
    }
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut v = [0u16; 64];
        let mut cwd = [0u16; 260];
        let n = GetCurrentDirectoryW(260, cwd.as_mut_ptr()) as usize;
        if contains(cmd, b"--child1") {
            check(ascii_eq(cmd, b"\"C:\\Program Files\\PolyK32Test\\k32test.exe\" --child1 \"two words\""));
            let k = GetEnvironmentVariableW(w!("POLY_CHILD"), v.as_mut_ptr(), 64) as usize;
            check(ascii_eq(&v[..k], b"explicit"));
            // Only the block it was given.
            check(GetEnvironmentVariableW(w!("POLY_K32_INHERITED"), v.as_mut_ptr(), 64) == 0);
            check(ascii_eq(&cwd[..n], b"C:\\Windows"));
            ExitProcess(0xC0DE_0001)
        }
        check(ascii_eq(cmd, b"k32test.exe --child2"));
        let k = GetEnvironmentVariableW(w!("POLY_K32_INHERITED"), v.as_mut_ptr(), 64) as usize;
        check(ascii_eq(&v[..k], b"yes"));
        check(ascii_eq(&cwd[..n], b"C:\\Program Files\\PolyK32Test"));
        // Alive long enough for the creator to see it running.
        Sleep(300);
        ExitProcess(0xC0DE_0002)
    }
}

unsafe fn processes() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut si = [0u8; 104];
        si[..4].copy_from_slice(&104u32.to_le_bytes());
        let mut pi = [0u8; 24];
        let mut cmd = [0u16; 128];
        let handles = |pi: &[u8; 24]| {
            (
                isize::from_le_bytes(pi[0..8].try_into().unwrap()),
                isize::from_le_bytes(pi[8..16].try_into().unwrap()),
                u32::from_le_bytes(pi[16..20].try_into().unwrap()),
                u32::from_le_bytes(pi[20..24].try_into().unwrap()),
            )
        };
        check(SetEnvironmentVariableW(w!("POLY_K32_INHERITED"), w!("yes")) != 0);
        // An explicit environment, current directory and quoted program.
        fill(&mut cmd, b"\"C:\\Program Files\\PolyK32Test\\k32test.exe\" --child1 \"two words\"");
        let mut env = [0u16; 64];
        fill(&mut env, b"POLY_CHILD=explicit\0Path=C:\\Windows\\System32\0");
        let flags = CREATE_UNICODE_ENVIRONMENT;
        let ok = CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            flags,
            env.as_ptr().cast(),
            w!("C:\\Windows"),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(ok != 0);
        let (ph, th, pid, tid) = handles(&pi);
        check(ph != 0 && th != 0 && pid != 0 && pid != GetCurrentProcessId() && tid != 0);
        check(GetProcessId(ph) == pid);
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        let mut code = 0u32;
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == 0xC0DE_0001);
        check(WaitForSingleObject(th, 0) == WAIT_OBJECT_0);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);

        // A name searched for (the application's directory first), the
        // creator's environment and current directory inherited.
        check(SetCurrentDirectoryW(w!("C:\\Program Files\\PolyK32Test")) != 0);
        fill(&mut cmd, b"k32test.exe --child2");
        let ok = CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
            core::ptr::null(),
            core::ptr::null(),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(ok != 0);
        let (ph, th, _, _) = handles(&pi);
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == STILL_ACTIVE);
        check(WaitForSingleObject(ph, 0) == WAIT_TIMEOUT);
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == 0xC0DE_0002);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);

        // Failures.
        fill(&mut cmd, b"no-such-program --x");
        let r = CreateProcessW(np(), cmd.as_mut_ptr(), np(), np(), 0, 0, np(), np(), si.as_ptr(), pi.as_mut_ptr());
        check(r == 0);
        check_error(ERROR_FILE_NOT_FOUND);
        let r = CreateProcessW(
            w!("C:\\no-such.exe"),
            core::ptr::null_mut(),
            np(),
            np(),
            0,
            0,
            np(),
            np(),
            si.as_ptr(),
            pi.as_mut_ptr(),
        );
        check(r == 0);
        check_error(ERROR_FILE_NOT_FOUND);
        let r = CreateProcessW(np(), core::ptr::null_mut(), np(), np(), 0, 0, np(), np(), si.as_ptr(), pi.as_mut_ptr());
        check(r == 0);
        check_error(ERROR_INVALID_PARAMETER);
    }
}

static FLS_SEEN: AtomicU32 = AtomicU32::new(0);

extern "system" fn fls_callback(v: usize) {
    FLS_SEEN.fetch_add(v as u32, Ordering::SeqCst);
}

extern "system" fn tls_worker(p: *mut u8) -> u32 {
    let (idx, fls) = (p as u32 & 0xFFFF, p as u32 >> 16);
    // SAFETY: kernel32 calls.
    unsafe {
        if TlsGetValue(idx) != 0 {
            return 1;
        }
        if TlsSetValue(idx, 0x99) == 0 || TlsGetValue(idx) != 0x99 {
            return 2;
        }
        if FlsGetValue(fls) != 0 || FlsSetValue(fls, 0x66) == 0 {
            return 3;
        }
    }
    0x55
}

unsafe fn tls_and_fls() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let idx = TlsAlloc();
        check(idx != u32::MAX);
        SetLastError(5);
        check(TlsGetValue(idx) == 0);
        check_error(0);
        check(TlsSetValue(idx, 0x1234) != 0 && TlsGetValue(idx) == 0x1234);
        // Past the TEB's 64 slots: the expansion slots.
        let mut held = [0u32; 80];
        let mut n = 0;
        let mut big = u32::MAX;
        while n < held.len() {
            let i = TlsAlloc();
            check(i != u32::MAX);
            held[n] = i;
            n += 1;
            if i >= 64 {
                big = i;
                break;
            }
        }
        check(big != u32::MAX);
        check(TlsGetValue(big) == 0 && TlsSetValue(big, 0xABCD) != 0 && TlsGetValue(big) == 0xABCD);
        // A thread has values of its own; its FLS callback runs when it ends.
        let fls = FlsAlloc(Some(fls_callback));
        check(fls != u32::MAX && fls < 0x1_0000);
        let arg = ((fls << 16) | idx) as usize as *mut u8;
        let t = CreateThread(core::ptr::null(), 0, tls_worker, arg, 0, core::ptr::null_mut());
        check(t != 0);
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0);
        let mut code = 0u32;
        check(GetExitCodeThread(t, &mut code) != 0 && code == 0x55);
        check(CloseHandle(t) != 0);
        check(TlsGetValue(idx) == 0x1234);
        check(FLS_SEEN.load(Ordering::SeqCst) == 0x66);
        check(FlsSetValue(fls, 0x11) != 0 && FlsGetValue(fls) == 0x11);
        // Freeing an FLS index hands each value to the callback.
        check(FlsFree(fls) != 0);
        check(FLS_SEEN.load(Ordering::SeqCst) == 0x77);
        check(FlsGetValue(fls) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(FlsFree(fls) == 0);
        // Freed indices are reused, their slots cleared.
        for &i in &held[..n] {
            check(TlsFree(i) != 0);
        }
        check(TlsFree(big) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        let mut again = [0u32; 80];
        let mut found = false;
        for slot in again.iter_mut().take(n) {
            *slot = TlsAlloc();
            if *slot == big {
                found = true;
            }
        }
        check(found && TlsGetValue(big) == 0);
        for &i in &again[..n] {
            check(TlsFree(i) != 0);
        }
        check(TlsGetValue(5000) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(TlsFree(idx) != 0);
    }
}

unsafe fn code_pages_and_strings() {
    // SAFETY: valid arguments throughout.
    unsafe {
        check(GetACP() == 1252 && GetOEMCP() == 437);
        check(IsValidCodePage(65001) != 0 && IsValidCodePage(437) != 0 && IsValidCodePage(12345) == 0);
        let mut info = [0u8; 20];
        check(GetCPInfo(65001, info.as_mut_ptr()) != 0 && info[0] == 4 && info[4] == b'?');
        check(GetCPInfo(0, info.as_mut_ptr()) != 0 && info[0] == 1);
        let utf8 = b"h\xC3\xA9\xE2\x82\xAC\0";
        let null16 = core::ptr::null_mut();
        check(MultiByteToWideChar(65001, 0, utf8.as_ptr(), -1, null16, 0) == 4);
        let mut w = [0u16; 8];
        check(MultiByteToWideChar(65001, 0, utf8.as_ptr(), -1, w.as_mut_ptr(), 8) == 4);
        check(w[..4] == [0x68, 0xE9, 0x20AC, 0]);
        check(MultiByteToWideChar(65001, 0, utf8.as_ptr(), -1, w.as_mut_ptr(), 2) == 0);
        check_error(ERROR_INSUFFICIENT_BUFFER);
        check(MultiByteToWideChar(65001, 8, b"\xC0\x80".as_ptr(), 2, w.as_mut_ptr(), 8) == 0);
        check_error(ERROR_NO_UNICODE_TRANSLATION);
        check(MultiByteToWideChar(65001, 0, b"\xC0\x80".as_ptr(), 2, w.as_mut_ptr(), 8) == 2 && w[0] == 0xFFFD);
        let src = [0x68u16, 0xE9, 0x20AC, 0];
        let mut b = [0u8; 16];
        let none = core::ptr::null();
        let no_used = core::ptr::null_mut();
        check(WideCharToMultiByte(65001, 0, src.as_ptr(), -1, core::ptr::null_mut(), 0, none, no_used) == 7);
        check(WideCharToMultiByte(65001, 0, src.as_ptr(), -1, b.as_mut_ptr(), 16, none, no_used) == 7);
        check(&b[..7] == utf8);
        let mut used = 0;
        let cjk = [0x20ACu16, 0x4E2D];
        check(WideCharToMultiByte(1252, 0, cjk.as_ptr(), 2, b.as_mut_ptr(), 16, none, &mut used) == 2);
        check(b[0] == 0x80 && b[1] == b'?' && used == 1);
        check(MultiByteToWideChar(437, 0, [0x82u8].as_ptr(), 1, w.as_mut_ptr(), 8) == 1 && w[0] == 0xE9);
        check(CompareStringOrdinal(w!("abc"), -1, w!("ABC"), -1, 1) == 2);
        check(CompareStringOrdinal(w!("abc"), -1, w!("ABC"), -1, 0) == 3);
        check(CompareStringOrdinal(w!("abc"), 2, w!("abc"), 3, 0) == 1);
        check(lstrlenW(w!("hello")) == 5 && lstrlenW(core::ptr::null()) == 0);
        check(lstrcmpiW(w!("Hello"), w!("hELLO")) == 0 && lstrcmpW(w!("a"), w!("b")) < 0);
    }
}

unsafe fn file_mappings() {
    // SAFETY: valid arguments throughout; views are used inside their size.
    unsafe {
        let rw = GENERIC_READ | GENERIC_WRITE;
        let h = CreateFileW(w!("C:\\k32map.bin"), rw, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        let mut n = 0u32;
        check(WriteFile(h, b"mapped!".as_ptr(), 7, &mut n, core::ptr::null_mut()) != 0);
        // A writable mapping larger than the file grows it.
        let m = CreateFileMappingW(h, core::ptr::null(), PAGE_READWRITE, 0, 0x2000, core::ptr::null());
        check(m != 0);
        check(GetFileSize(h, core::ptr::null_mut()) == 0x2000);
        let p = MapViewOfFile(m, FILE_MAP_ALL_ACCESS, 0, 0, 0);
        check(!p.is_null());
        check(core::slice::from_raw_parts(p, 7) == b"mapped!");
        p.write(b'M');
        p.add(0x1000).write(b'Z');
        check(FlushViewOfFile(p, 0) != 0);
        // A second view shows the same pages.
        let q = MapViewOfFile(m, FILE_MAP_READ, 0, 0, 0x1000);
        check(!q.is_null() && q != p && *q == b'M');
        check(UnmapViewOfFile(q) != 0 && UnmapViewOfFile(p) != 0);
        check(UnmapViewOfFile(p) == 0);
        // The file holds what the view wrote.
        check(SetFilePointer(h, 0x1000, core::ptr::null_mut(), FILE_BEGIN) == 0x1000);
        let mut c = [0u8; 1];
        check(ReadFile(h, c.as_mut_ptr(), 1, &mut n, core::ptr::null_mut()) != 0 && n == 1 && c[0] == b'Z');
        check(MapViewOfFile(m, FILE_MAP_READ, 0, 0x1000, 0).is_null());
        check_error(ERROR_MAPPED_ALIGNMENT);
        check(CloseHandle(m) != 0);
        // A named mapping of the paging file, opened again by name.
        let a = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            core::ptr::null(),
            PAGE_READWRITE,
            0,
            0x1000,
            w!("Local\\PolyK32Map"),
        );
        check(a != 0);
        check_error(0);
        let b = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, w!("PolyK32Map"));
        check(b != 0);
        let again =
            CreateFileMappingW(INVALID_HANDLE_VALUE, core::ptr::null(), PAGE_READWRITE, 0, 0x1000, w!("PolyK32Map"));
        check(again != 0);
        check_error(ERROR_ALREADY_EXISTS);
        let pa = MapViewOfFile(a, FILE_MAP_WRITE, 0, 0, 0);
        let pb = MapViewOfFileEx(b, FILE_MAP_READ, 0, 0, 0, core::ptr::null_mut());
        check(!pa.is_null() && !pb.is_null() && pa != pb);
        pa.add(10).write(42);
        check(*pb.add(10) == 42);
        check(UnmapViewOfFile(pa) != 0 && UnmapViewOfFile(pb) != 0);
        check(CloseHandle(a) != 0 && CloseHandle(b) != 0 && CloseHandle(again) != 0);
        check(
            CreateFileMappingW(INVALID_HANDLE_VALUE, core::ptr::null(), PAGE_READWRITE, 0, 0, core::ptr::null()) == 0,
        );
        check_error(ERROR_INVALID_PARAMETER);
        check(CloseHandle(h) != 0);
        check(DeleteFileW(w!("C:\\k32map.bin")) != 0);
    }
}

/// Shared with the condition-variable worker.
struct Cond {
    lock: usize,
    cv: usize,
    ready: u32,
    cs: [u8; 40],
    cv2: usize,
    ready2: u32,
}

static mut COND: Cond = Cond { lock: 0, cv: 0, ready: 0, cs: [0; 40], cv2: 0, ready2: 0 };

extern "system" fn cond_worker(p: *mut u8) -> u32 {
    let c = p as *mut Cond;
    // SAFETY: the static, guarded by its SRW lock and critical section.
    unsafe {
        Sleep(20);
        AcquireSRWLockExclusive(&raw mut (*c).lock);
        (*c).ready = 1;
        WakeAllConditionVariable(&raw mut (*c).cv);
        ReleaseSRWLockExclusive(&raw mut (*c).lock);
        EnterCriticalSection((&raw mut (*c).cs).cast());
        (*c).ready2 = 1;
        WakeConditionVariable(&raw mut (*c).cv2);
        LeaveCriticalSection((&raw mut (*c).cs).cast());
    }
    0
}

static INIT_RUNS: AtomicU32 = AtomicU32::new(0);

extern "system" fn init_fn(_once: *mut usize, param: *mut u8, ctx: *mut *mut u8) -> i32 {
    INIT_RUNS.fetch_add(1, Ordering::SeqCst);
    if param as usize == 1 {
        return 0;
    }
    // SAFETY: the out-pointer InitOnceExecuteOnce passes.
    unsafe { ctx.write(0x1000 as *mut u8) };
    1
}

unsafe fn conditions_and_init_once() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let c = &raw mut COND;
        InitializeConditionVariable(&raw mut (*c).cv);
        InitializeConditionVariable(&raw mut (*c).cv2);
        InitializeSRWLock(&raw mut (*c).lock);
        InitializeCriticalSection((&raw mut (*c).cs).cast());
        AcquireSRWLockExclusive(&raw mut (*c).lock);
        // Nobody wakes it: a timeout, the lock held again.
        check(SleepConditionVariableSRW(&raw mut (*c).cv, &raw mut (*c).lock, 20, 0) == 0);
        check_error(ERROR_TIMEOUT);
        check(TryAcquireSRWLockExclusive(&raw mut (*c).lock) == 0);
        let t = CreateThread(core::ptr::null(), 0, cond_worker, c.cast(), 0, core::ptr::null_mut());
        check(t != 0);
        while (*c).ready == 0 {
            check(SleepConditionVariableSRW(&raw mut (*c).cv, &raw mut (*c).lock, 10_000, 0) != 0);
        }
        ReleaseSRWLockExclusive(&raw mut (*c).lock);
        EnterCriticalSection((&raw mut (*c).cs).cast());
        while (*c).ready2 == 0 {
            check(SleepConditionVariableCS(&raw mut (*c).cv2, (&raw mut (*c).cs).cast(), 10_000) != 0);
        }
        LeaveCriticalSection((&raw mut (*c).cs).cast());
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0 && CloseHandle(t) != 0);
        DeleteCriticalSection((&raw mut (*c).cs).cast());

        // One-time initialisation: a failed run lets the next one try.
        static mut ONCE: usize = 0;
        let mut ctx: *mut u8 = core::ptr::null_mut();
        check(InitOnceExecuteOnce(&raw mut ONCE, init_fn, core::ptr::without_provenance_mut(1), &mut ctx) == 0);
        check(InitOnceExecuteOnce(&raw mut ONCE, init_fn, core::ptr::null_mut(), &mut ctx) != 0);
        check(ctx as usize == 0x1000);
        ctx = core::ptr::null_mut();
        check(InitOnceExecuteOnce(&raw mut ONCE, init_fn, core::ptr::null_mut(), &mut ctx) != 0);
        check(ctx as usize == 0x1000 && INIT_RUNS.load(Ordering::SeqCst) == 2);
    }
}

unsafe fn file_information_and_times() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let rw = GENERIC_READ | GENERIC_WRITE;
        let h = CreateFileW(w!("C:\\k32info.txt"), rw, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        check(h != INVALID_HANDLE_VALUE);
        let mut n = 0u32;
        check(WriteFile(h, b"12345".as_ptr(), 5, &mut n, core::ptr::null_mut()) != 0);
        let mut info = [0u8; 52];
        check(GetFileInformationByHandle(h, info.as_mut_ptr()) != 0);
        let d = |o: usize| u32::from_le_bytes(info[o..o + 4].try_into().unwrap());
        check(d(0) & FILE_ATTRIBUTE_DIRECTORY == 0);
        check(d(32) == 0 && d(36) == 5 && d(40) >= 1);
        check(d(44) | d(48) != 0);
        // 2001-02-03 04:05:06, a Saturday.
        let st: [u16; 8] = [2001, 2, 0, 3, 4, 5, 6, 0];
        let mut ft = 0u64;
        check(SystemTimeToFileTime(st.as_ptr(), &mut ft) != 0);
        check(SetFileTime(h, core::ptr::null(), core::ptr::null(), &ft) != 0);
        let (mut c, mut a, mut wr) = (0u64, 0u64, 0u64);
        check(GetFileTime(h, &mut c, &mut a, &mut wr) != 0);
        check(wr == ft && c != 0 && a != 0);
        let mut back = [0u16; 8];
        check(FileTimeToSystemTime(&wr, back.as_mut_ptr()) != 0);
        check(back == [2001, 2, 6, 3, 4, 5, 6, 0]);
        let later = ft + 1;
        check(CompareFileTime(&ft, &wr) == 0 && CompareFileTime(&ft, &later) == -1);
        let bad: [u16; 8] = [2001, 2, 0, 30, 0, 0, 0, 0];
        check(SystemTimeToFileTime(bad.as_ptr(), &mut ft) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        let mut now = [0u16; 8];
        GetSystemTime(now.as_mut_ptr());
        check(now[0] >= 1601 && (1..=12).contains(&now[1]) && now[2] < 7);
        check(CloseHandle(h) != 0);
        check(DeleteFileW(w!("C:\\k32info.txt")) != 0);
    }
}

// ----- what 7-Zip's command line and MSVC's start-up code import (ADR 0087) ----------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn IsProcessorFeaturePresent(feature: u32) -> i32;
    fn InitializeSListHead(head: *mut u64);
    fn InterlockedPushEntrySList(head: *mut u64, entry: *mut u64) -> *mut u64;
    fn InterlockedPopEntrySList(head: *mut u64) -> *mut u64;
    fn InterlockedFlushSList(head: *mut u64) -> *mut u64;
    fn QueryDepthSList(head: *mut u64) -> u16;
    fn RtlCaptureContext(context: *mut u8);
    fn RaiseException(code: u32, flags: u32, count: u32, args: *const usize);
    fn SetUnhandledExceptionFilter(filter: usize) -> usize;
    fn UnhandledExceptionFilter(pointers: *mut u8) -> i32;
    fn FileTimeToDosDateTime(ft: *const u64, date: *mut u16, time: *mut u16) -> i32;
    fn DosDateTimeToFileTime(date: u16, time: u16, ft: *mut u64) -> i32;
    fn GetProcessTimes(p: Handle, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
    fn GlobalMemoryStatusEx(status: *mut u8) -> i32;
    fn GetLogicalDrives() -> u32;
    fn GetLogicalDriveStringsW(size: u32, buf: *mut u16) -> u32;
    fn GetDiskFreeSpaceExW(dir: *const u16, caller: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GetDiskFreeSpaceW(root: *const u16, spc: *mut u32, bps: *mut u32, free: *mut u32, total: *mut u32) -> i32;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn ResumeThread(h: Handle) -> u32;
    fn GetModuleHandleA(name: *const u8) -> *mut u8;
    fn GetStartupInfoW(info: *mut u8);
    fn GetProcessAffinityMask(p: Handle, mask: *mut u64, system: *mut u64) -> i32;
    fn SetProcessAffinityMask(p: Handle, mask: u64) -> i32;
    fn SetThreadAffinityMask(t: Handle, mask: u64) -> u64;
    fn GetLargePageMinimum() -> usize;
    fn AreFileApisANSI() -> i32;
    fn SetFileApisToOEM();
    fn SetFileApisToANSI();
    fn MoveFileW(from: *const u16, to: *const u16) -> i32;
    fn MoveFileWithProgressW(from: *const u16, to: *const u16, p: *const u8, d: *mut u8, flags: u32) -> i32;
    fn FindFirstStreamW(name: *const u16, level: u32, data: *mut u8, flags: u32) -> Handle;
    fn FindNextStreamW(find: Handle, data: *mut u8) -> i32;
    fn CreateHardLinkW(new: *const u16, existing: *const u16, sa: *const u8) -> i32;
}

const ERROR_HANDLE_EOF: u32 = 38;
/// `EXCEPTION_CONTINUE_EXECUTION`, `EXCEPTION_CONTINUE_SEARCH`,
/// `EXCEPTION_EXECUTE_HANDLER` (`excpt.h`).
const CONTINUE_EXECUTION: i32 = -1;
const CONTINUE_SEARCH: i32 = 0;
const EXECUTE_HANDLER: i32 = 1;
/// The code `RaiseException` raises here: an application code (bit 29).
const TEST_CODE: u32 = 0xE000_0087;

static RAISED: AtomicU32 = AtomicU32::new(0);

/// A vectored handler (`PVECTORED_EXCEPTION_HANDLER`): takes the test's
/// exception, noting its first parameter and that it has an address.
extern "system" fn veh(pointers: *mut u8) -> i32 {
    // SAFETY: EXCEPTION_POINTERS: the record, then the context; the
    // record's code at 0, address at 16, count at 24, parameters at 32.
    unsafe {
        let record = pointers.cast::<*const u8>().read();
        if record.cast::<u32>().read() != TEST_CODE {
            return CONTINUE_SEARCH;
        }
        let address = record.add(16).cast::<u64>().read();
        let count = record.add(24).cast::<u32>().read();
        let first = record.add(32).cast::<u64>().read();
        let ok = address != 0 && count == 2 && first == 0x7A;
        RAISED.store(if ok { 1 } else { 2 }, Ordering::SeqCst);
    }
    CONTINUE_EXECUTION
}

extern "system" fn top_filter(_pointers: *mut u8) -> i32 {
    CONTINUE_EXECUTION
}

static RESUMED: AtomicU32 = AtomicU32::new(0);

extern "system" fn resumed_worker(_p: *mut u8) -> u32 {
    RESUMED.store(1, Ordering::SeqCst);
    7
}

#[repr(C, align(16))]
struct Aligned<const N: usize>([u8; N]);

unsafe fn seven_zip_imports() {
    // SAFETY: valid arguments throughout.
    unsafe {
        // The processor: SSE2 and PAE on every x64 processor.
        check(IsProcessorFeaturePresent(10) == 1 && IsProcessorFeaturePresent(9) == 1);
        check(IsProcessorFeaturePresent(1000) == 0);

        // An interlocked list: last in, first out, with its depth.
        let mut head = Aligned([0u8; 16]);
        let mut entries = [Aligned([0u8; 16]), Aligned([0u8; 16])];
        let h = head.0.as_mut_ptr().cast::<u64>();
        InitializeSListHead(h);
        let (a, b) = (entries[0].0.as_mut_ptr().cast::<u64>(), entries[1].0.as_mut_ptr().cast::<u64>());
        check(InterlockedPushEntrySList(h, a).is_null());
        check(InterlockedPushEntrySList(h, b) == a && QueryDepthSList(h) == 2);
        check(InterlockedPopEntrySList(h) == b && QueryDepthSList(h) == 1);
        check(InterlockedFlushSList(h) == a && QueryDepthSList(h) == 0);
        check(InterlockedPopEntrySList(h).is_null());

        // RtlCaptureContext through kernel32's forwarder to ntdll: the
        // caller's stack and code, every group of registers.
        let mut ctx = Aligned([0u8; 0x4D0]);
        let local = 0u64;
        RtlCaptureContext(ctx.0.as_mut_ptr());
        let q = |o: usize| u64::from_le_bytes(ctx.0[o..o + 8].try_into().unwrap());
        let flags = u32::from_le_bytes(ctx.0[0x30..0x34].try_into().unwrap());
        let rsp = q(0x98);
        let here = &local as *const u64 as u64;
        check(flags == 0x10_000F && q(0xF8) != 0 && rsp.abs_diff(here) < 0x1_0000);
        check(u16::from_le_bytes([ctx.0[0x38], ctx.0[0x39]]) != 0);

        // RaiseException: a vectored handler (ntdll's, looked up by name)
        // sees the record and continues execution, so the call returns.
        let ntdll = GetModuleHandleW(w!("ntdll.dll"));
        let add = GetProcAddress(ntdll, c"RtlAddVectoredExceptionHandler".as_ptr().cast());
        check(!add.is_null());
        type AddVeh = unsafe extern "system" fn(u32, extern "system" fn(*mut u8) -> i32) -> *mut u8;
        let cookie = core::mem::transmute::<*mut u8, AddVeh>(add)(1, veh);
        check(!cookie.is_null());
        let args = [0x7Ausize, 0x7B];
        RaiseException(TEST_CODE, 0, 2, args.as_ptr());
        check(RAISED.load(Ordering::SeqCst) == 1);
        let remove = GetProcAddress(ntdll, c"RtlRemoveVectoredExceptionHandler".as_ptr().cast());
        type RemoveVeh = unsafe extern "system" fn(*mut u8) -> u32;
        check(!remove.is_null() && core::mem::transmute::<*mut u8, RemoveVeh>(remove)(cookie) != 0);

        // The top-level filter: installed, called by UnhandledExceptionFilter.
        check(SetUnhandledExceptionFilter(top_filter as *const () as usize) == 0);
        let mut pointers = [0u64; 2];
        check(UnhandledExceptionFilter(pointers.as_mut_ptr().cast()) == CONTINUE_EXECUTION);
        check(SetUnhandledExceptionFilter(0) == top_filter as *const () as usize);
        check(UnhandledExceptionFilter(pointers.as_mut_ptr().cast()) == EXECUTE_HANDLER);

        // MS-DOS dates: 2023-10-21 13:45:30 and back.
        let st: [u16; 8] = [2023, 10, 0, 21, 13, 45, 31, 0];
        let mut ft = 0u64;
        check(SystemTimeToFileTime(st.as_ptr(), &mut ft) != 0);
        let (mut date, mut time) = (0u16, 0u16);
        check(FileTimeToDosDateTime(&ft, &mut date, &mut time) != 0);
        check(date == (43 << 9) | (10 << 5) | 21 && time == (13 << 11) | (45 << 5) | 15);
        let mut back = 0u64;
        check(DosDateTimeToFileTime(date, time, &mut back) != 0 && ft - back == 10_000_000);
        check(DosDateTimeToFileTime(0, 0, &mut back) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // Process times: created before now.
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        check(GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) != 0);
        let mut now = 0u64;
        GetSystemTimeAsFileTime(&mut now);
        check(c != 0 && c <= now && e == 0);

        // Memory: dwLength must be set.
        let mut ms = [0u8; 64];
        ms[0] = 64;
        check(GlobalMemoryStatusEx(ms.as_mut_ptr()) != 0);
        let mq = |o: usize| u64::from_le_bytes(ms[o..o + 8].try_into().unwrap());
        check(mq(8) > 0 && mq(16) <= mq(8) && mq(40) > mq(8));
        ms[0] = 0;
        check(GlobalMemoryStatusEx(ms.as_mut_ptr()) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(GetLargePageMinimum() == 0);

        // Drives: C: and Z: (ADR 0067, 0073), and the space on C:.
        let drives = GetLogicalDrives();
        check(drives & (1 << 2) != 0 && drives & (1 << 25) != 0);
        let n = drives.count_ones();
        check(GetLogicalDriveStringsW(0, core::ptr::null_mut()) == 4 * n + 1);
        let mut list = [0u16; 128];
        check(GetLogicalDriveStringsW(128, list.as_mut_ptr()) == 4 * n);
        check(list[..4] == [u16::from(b'C'), u16::from(b':'), u16::from(b'\\'), 0] || drives & 3 != 0);
        let (mut caller, mut total, mut free) = (0u64, 0u64, 0u64);
        check(GetDiskFreeSpaceExW(w!("C:\\"), &mut caller, &mut total, &mut free) != 0);
        check(total > 0 && caller <= total && free <= total);
        let (mut spc, mut bps, mut fc, mut tc) = (0u32, 0u32, 0u32, 0u32);
        check(GetDiskFreeSpaceW(core::ptr::null(), &mut spc, &mut bps, &mut fc, &mut tc) != 0);
        check(spc * bps > 0 && tc > 0);
        check(GetDiskFreeSpaceExW(w!("C:\\no such dir"), &mut caller, &mut total, &mut free) == 0);

        // OpenEventW: a named event, and a name that does not exist.
        let ev = CreateEventW(core::ptr::null(), 1, 0, w!("k32test-7zip"));
        check(ev != 0);
        let opened = OpenEventW(0x1F_0003, 0, w!("k32test-7zip"));
        check(opened != 0 && SetEvent(opened) != 0 && WaitForSingleObject(ev, 0) == WAIT_OBJECT_0);
        check(CloseHandle(opened) != 0 && CloseHandle(ev) != 0);
        check(OpenEventW(0x1F_0003, 0, w!("k32test-no-such-event")) == 0);
        check_error(ERROR_FILE_NOT_FOUND);

        // A suspended thread runs once resumed (its suspend count was 1).
        let t = CreateThread(core::ptr::null(), 0, resumed_worker, core::ptr::null_mut(), 4, core::ptr::null_mut());
        check(t != 0);
        Sleep(10);
        check(RESUMED.load(Ordering::SeqCst) == 0);
        check(ResumeThread(t) == 1);
        check(WaitForSingleObject(t, 10_000) == WAIT_OBJECT_0 && RESUMED.load(Ordering::SeqCst) == 1);
        let mut code = 0u32;
        check(GetExitCodeThread(t, &mut code) != 0 && code == 7 && CloseHandle(t) != 0);

        // Modules, start-up information, affinity.
        check(GetModuleHandleA(c"KERNEL32.DLL".as_ptr().cast()) == GetModuleHandleW(w!("kernel32.dll")));
        check(GetModuleHandleA(core::ptr::null()) == GetModuleHandleW(core::ptr::null()));
        let mut si = [0xFFu8; 104];
        GetStartupInfoW(si.as_mut_ptr());
        check(u32::from_le_bytes(si[0..4].try_into().unwrap()) == 104 && si[60..64] == [0; 4]);
        let (mut pm, mut sm) = (0u64, 0u64);
        check(GetProcessAffinityMask(GetCurrentProcess(), &mut pm, &mut sm) != 0);
        check(pm != 0 && pm & !sm == 0);
        check(SetThreadAffinityMask(GetCurrentThread(), sm) == pm);
        check(SetProcessAffinityMask(GetCurrentProcess(), 0) == 0);
        check_error(ERROR_INVALID_PARAMETER);

        // The file API code page switch.
        check(AreFileApisANSI() != 0);
        SetFileApisToOEM();
        check(AreFileApisANSI() == 0);
        SetFileApisToANSI();
        check(AreFileApisANSI() != 0);

        // The older moves; streams (the default data stream only) and hard
        // links (ADR 0091).
        let f = CreateFileW(w!("C:\\k32move1.txt"), GENERIC_WRITE, 0, core::ptr::null(), CREATE_ALWAYS, 0, 0);
        let mut n = 0u32;
        check(f != INVALID_HANDLE_VALUE && WriteFile(f, b"five!".as_ptr(), 5, &mut n, core::ptr::null_mut()) != 0);
        check(CloseHandle(f) != 0);
        check(MoveFileW(w!("C:\\k32move1.txt"), w!("C:\\k32move2.txt")) != 0);
        check(GetFileAttributesW(w!("C:\\k32move1.txt")) == INVALID_FILE_ATTRIBUTES);
        check(
            MoveFileWithProgressW(w!("C:\\k32move2.txt"), w!("C:\\k32move3.txt"), np(), core::ptr::null_mut(), 0) != 0,
        );
        let mut data = [0u8; 600];
        let fs = FindFirstStreamW(w!("C:\\k32move3.txt"), 0, data.as_mut_ptr(), 0);
        check(fs != INVALID_HANDLE_VALUE);
        check(u64::from_le_bytes(data[0..8].try_into().unwrap()) == 5);
        let stream: [u16; 8] =
            [b':' as u16, b':' as u16, b'$' as u16, b'D' as u16, b'A' as u16, b'T' as u16, b'A' as u16, 0];
        check(data[8..24].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).eq(stream));
        check(FindNextStreamW(fs, data.as_mut_ptr()) == 0);
        check_error(ERROR_HANDLE_EOF);
        check(FindClose(fs) != 0);
        // A directory has no data stream.
        check(FindFirstStreamW(w!("C:\\"), 0, data.as_mut_ptr(), 0) == INVALID_HANDLE_VALUE);
        check_error(ERROR_HANDLE_EOF);
        check(CreateHardLinkW(w!("C:\\k32link.txt"), w!("C:\\k32move3.txt"), core::ptr::null()) != 0);
        let mut info = [0u8; 52];
        let l = CreateFileW(w!("C:\\k32link.txt"), GENERIC_READ, 7, core::ptr::null(), OPEN_EXISTING, 0, 0);
        check(l != INVALID_HANDLE_VALUE && GetFileInformationByHandle(l, info.as_mut_ptr()) != 0);
        // nNumberOfLinks, and the same bytes under the second name.
        check(u32::from_le_bytes(info[40..44].try_into().unwrap()) == 2);
        let mut five = [0u8; 5];
        check(ReadFile(l, five.as_mut_ptr(), 5, &mut n, core::ptr::null_mut()) != 0 && &five == b"five!");
        check(CloseHandle(l) != 0);
        check(CreateHardLinkW(w!("C:\\k32link.txt"), w!("C:\\k32move3.txt"), core::ptr::null()) == 0);
        check_error(ERROR_ALREADY_EXISTS);
        check(DeleteFileW(w!("C:\\k32move3.txt")) != 0);
        check(DeleteFileW(w!("C:\\k32link.txt")) != 0);

        // oleaut32 by ordinal, user32 and advapi32's RtlGenRandom, loaded.
        let ole = LoadLibraryW(w!("oleaut32.dll"));
        check(!ole.is_null());
        type AllocString = unsafe extern "system" fn(*const u16) -> *mut u16;
        type StringLen = unsafe extern "system" fn(*mut u16) -> u32;
        type FreeString = unsafe extern "system" fn(*mut u16);
        type VariantFn = unsafe extern "system" fn(*mut u8) -> u32;
        type VariantCopy = unsafe extern "system" fn(*mut u8, *const u8) -> u32;
        let by = |o: usize| GetProcAddress(ole, o as *const u8);
        check(!by(2).is_null() && !by(6).is_null() && !by(7).is_null() && !by(9).is_null() && !by(10).is_null());
        let alloc_s = core::mem::transmute::<*mut u8, AllocString>(by(2));
        let len_s = core::mem::transmute::<*mut u8, StringLen>(by(7));
        let free_s = core::mem::transmute::<*mut u8, FreeString>(by(6));
        let clear = core::mem::transmute::<*mut u8, VariantFn>(by(9));
        let copy = core::mem::transmute::<*mut u8, VariantCopy>(by(10));
        let s = alloc_s(w!("7-Zip"));
        check(!s.is_null() && len_s(s) == 5 && wslice(s) == wslice(w!("7-Zip")));
        check(alloc_s(core::ptr::null()).is_null() && len_s(core::ptr::null_mut()) == 0);
        // A VT_BSTR variant copied (the string duplicated) and cleared.
        let mut v1 = Aligned([0u8; 24]);
        let mut v2 = Aligned([0u8; 24]);
        v1.0[0] = 8;
        v1.0[8..16].copy_from_slice(&(s as u64).to_le_bytes());
        check(copy(v2.0.as_mut_ptr(), v1.0.as_ptr()) == 0 && v2.0[0] == 8);
        let dup = u64::from_le_bytes(v2.0[8..16].try_into().unwrap()) as *mut u16;
        check(dup != s && len_s(dup) == 5);
        check(clear(v2.0.as_mut_ptr()) == 0 && v2.0[0] == 0);
        check(clear(v1.0.as_mut_ptr()) == 0);
        v1.0[0] = 15;
        check(clear(v1.0.as_mut_ptr()) == 0x8002_0008);
        let _ = free_s;
        let user = LoadLibraryW(w!("user32.dll"));
        let upper = GetProcAddress(user, c"CharUpperW".as_ptr().cast());
        check(!upper.is_null());
        type CharUpper = unsafe extern "system" fn(*mut u16) -> *mut u16;
        let upper = core::mem::transmute::<*mut u8, CharUpper>(upper);
        let mut text = [u16::from(b'z'), u16::from(b'i'), u16::from(b'p'), 0];
        check(upper(text.as_mut_ptr()) == text.as_mut_ptr() && text[..3] == [0x5A, 0x49, 0x50]);
        check(upper(0x61 as *mut u16) as usize == 0x41);
        let advapi = LoadLibraryW(w!("advapi32.dll"));
        let random = GetProcAddress(advapi, c"SystemFunction036".as_ptr().cast());
        check(!random.is_null());
        type GenRandom = unsafe extern "system" fn(*mut u8, u32) -> u8;
        let mut bytes = [0u8; 32];
        check(core::mem::transmute::<*mut u8, GenRandom>(random)(bytes.as_mut_ptr(), 32) != 0);
        check(bytes.iter().any(|&b| b != 0));
    }
}

// ----- handle inheritance, STARTUPINFO, CREATE_SUSPENDED (ADR 0089) --------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetHandleInformation(h: Handle, flags: *mut u32) -> i32;
    fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32;
    fn DuplicateHandle(sp: Handle, s: Handle, tp: Handle, t: *mut Handle, access: u32, inherit: i32, opts: u32) -> i32;
    fn CreateProcessA(
        app: *const u8,
        cmd: *mut u8,
        pa: *const u8,
        ta: *const u8,
        inherit: i32,
        flags: u32,
        env: *const u8,
        cwd: *const u8,
        si: *const u8,
        pi: *mut u8,
    ) -> i32;
    fn GetStartupInfoA(info: *mut u8);
    fn GetFileType(h: Handle) -> u32;
    fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
}

/// `HANDLE_FLAG_*` (`handleapi.h`), `DUPLICATE_SAME_ACCESS` (`winnt.h`),
/// `STARTF_USESTDHANDLES` (`processthreadsapi.h`), `CREATE_SUSPENDED`
/// (`winbase.h`), the standard handle selectors (`processenv.h`),
/// `FILE_TYPE_DISK` (`fileapi.h`), `ERROR_INVALID_HANDLE` (`winerror.h`).
const HANDLE_FLAG_INHERIT: u32 = 1;
const HANDLE_FLAG_PROTECT_FROM_CLOSE: u32 = 2;
const DUPLICATE_SAME_ACCESS: u32 = 2;
const STARTF_USESTDHANDLES: u32 = 0x100;
const CREATE_SUSPENDED: u32 = 4;
const STD_INPUT_HANDLE: u32 = -10i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;
const FILE_TYPE_DISK: u32 = 1;
const ERROR_INVALID_HANDLE: u32 = 6;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE`.
const SHARE_READ_WRITE: u32 = 3;
const CHILD_STDOUT: &[u8] = b"child stdout\r\n";
const CHILD_STDERR: &[u8] = b"child stderr\r\n";

/// `s` (ASCII) appended to `buf` at `*at`.
fn push_ascii(buf: &mut [u16], at: &mut usize, s: &[u8]) {
    for &c in s {
        buf[*at] = u16::from(c);
        *at += 1;
    }
}

/// `v` in decimal appended to `buf` at `*at`.
fn push_num(buf: &mut [u16], at: &mut usize, v: u64) {
    let mut digits = [0u8; 20];
    let (mut i, mut v) = (digits.len(), v);
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    push_ascii(buf, at, &digits[i..]);
}

/// `dir` (ending in a separator) and `name`, NUL-terminated.
fn path_in(dir: &[u16], name: &[u8]) -> [u16; 300] {
    let mut p = [0u16; 300];
    p[..dir.len()].copy_from_slice(dir);
    let mut at = dir.len();
    push_ascii(&mut p, &mut at, name);
    p
}

/// The `index`th decimal number after the `--childN` flag.
fn arg_num(cmd: &[u16], index: usize) -> u64 {
    let start = cmd.windows(7).position(|w| ascii_eq(w, b"--child")).unwrap_or(0);
    let mut words = cmd[start..].split(|&c| c == u16::from(b' ')).filter(|w| !w.is_empty()).skip(1);
    let word = words.nth(index).unwrap_or(&[]);
    word.iter().fold(0u64, |n, &c| n * 10 + u64::from(c.wrapping_sub(u16::from(b'0')) & 0xF))
}

/// The first bytes of the file at `path` (up to 64).
///
/// # Safety
/// `path` is NUL-terminated.
unsafe fn read_back(path: &[u16]) -> ([u8; 64], usize) {
    let mut buf = [0u8; 64];
    let mut n = 0u32;
    // SAFETY: valid arguments.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_READ, SHARE_READ_WRITE, np(), OPEN_EXISTING, 0, 0);
        if h != INVALID_HANDLE_VALUE {
            ReadFile(h, buf.as_mut_ptr(), 64, &mut n, core::ptr::null_mut());
            CloseHandle(h);
        }
    }
    (buf, n as usize)
}

unsafe fn inheritance_and_startup() {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut temp = [0u16; 260];
        let n = GetTempPathW(260, temp.as_mut_ptr()) as usize;
        check(n > 3);
        let out_path = path_in(&temp[..n], b"k32-child-out.txt");
        let err_path = path_in(&temp[..n], b"k32-child-err.txt");
        // Inheritable through SECURITY_ATTRIBUTES.bInheritHandle.
        let mut sa = [0u8; 24];
        sa[0..4].copy_from_slice(&24u32.to_le_bytes());
        sa[16..20].copy_from_slice(&1i32.to_le_bytes());
        let out_h =
            CreateFileW(out_path.as_ptr(), GENERIC_WRITE, SHARE_READ_WRITE, sa.as_ptr(), CREATE_ALWAYS, 0x80, 0);
        check(out_h != INVALID_HANDLE_VALUE);
        let mut f = 0xFFu32;
        check(GetHandleInformation(out_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        // Made inheritable afterwards; protected from close for a while.
        let err_h = CreateFileW(err_path.as_ptr(), GENERIC_WRITE, SHARE_READ_WRITE, np(), CREATE_ALWAYS, 0x80, 0);
        check(err_h != INVALID_HANDLE_VALUE);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == 0);
        check(SetHandleInformation(err_h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) != 0);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        let protect = HANDLE_FLAG_PROTECT_FROM_CLOSE;
        check(SetHandleInformation(err_h, protect, protect | HANDLE_FLAG_INHERIT) != 0);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT | protect);
        check(CloseHandle(err_h) == 0);
        check(SetHandleInformation(err_h, protect, 0) != 0);
        check(GetHandleInformation(err_h, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        check(SetHandleInformation(err_h, 4, 4) == 0);
        check_error(ERROR_INVALID_PARAMETER);
        check(GetHandleInformation(0x7FFC, &mut f) == 0);
        check_error(ERROR_INVALID_HANDLE);
        // Handles the child must not get: enough of them that the last
        // one's value is above any the child opens for itself.
        let mut private = [0 as Handle; 48];
        for p in private.iter_mut() {
            *p = CreateEventW(np(), 1, 0, np());
            check(*p != 0);
        }
        let hidden = private[47];
        check(GetHandleInformation(hidden, &mut f) != 0 && f == 0);
        // An inheritable copy of one of them: the same event in the child.
        let mut dup: Handle = 0;
        let me = GetCurrentProcess();
        check(DuplicateHandle(me, private[0], me, &mut dup, 0, 1, DUPLICATE_SAME_ACCESS) != 0);
        check(dup != 0 && dup != private[0]);
        check(GetHandleInformation(dup, &mut f) != 0 && f == HANDLE_FLAG_INHERIT);
        check(GetHandleInformation(private[0], &mut f) != 0 && f == 0);

        // The child: standard output and error redirected to the files,
        // its standard input the creator's, a title and a position, created
        // suspended, inheriting.
        let mut si = [0u8; 104];
        si[0..4].copy_from_slice(&104u32.to_le_bytes());
        si[24..32].copy_from_slice(&(w!("poly child") as u64).to_le_bytes());
        si[32..36].copy_from_slice(&0x11u32.to_le_bytes());
        si[60..64].copy_from_slice(&STARTF_USESTDHANDLES.to_le_bytes());
        let input = GetStdHandle(STD_INPUT_HANDLE);
        for (i, h) in [input, out_h, err_h].iter().enumerate() {
            si[80 + 8 * i..88 + 8 * i].copy_from_slice(&(*h as u64).to_le_bytes());
        }
        let mut cmd = [0u16; 128];
        let mut at = 0;
        push_ascii(&mut cmd, &mut at, b"k32test.exe --child3 ");
        for (i, v) in [dup, hidden, input].into_iter().enumerate() {
            if i > 0 {
                push_ascii(&mut cmd, &mut at, b" ");
            }
            push_num(&mut cmd, &mut at, v as u64);
        }
        let mut pi = [0u8; 24];
        let flags = CREATE_SUSPENDED;
        check(
            CreateProcessW(np(), cmd.as_mut_ptr(), np(), np(), 1, flags, np(), np(), si.as_ptr(), pi.as_mut_ptr()) != 0,
        );
        let ph = isize::from_le_bytes(pi[0..8].try_into().unwrap());
        let th = isize::from_le_bytes(pi[8..16].try_into().unwrap());
        check(ph != 0 && th != 0 && u32::from_le_bytes(pi[20..24].try_into().unwrap()) != 0);
        // Suspended: it has not run yet.
        check(WaitForSingleObject(ph, 300) == WAIT_TIMEOUT);
        let mut code = 0u32;
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == STILL_ACTIVE);
        check(GetFileSize(out_h, core::ptr::null_mut()) == 0);
        check(WaitForSingleObject(private[0], 0) == WAIT_TIMEOUT);
        check(ResumeThread(th) == 1);
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        check(GetExitCodeProcess(ph, &mut code) != 0);
        if code != 0xC0DE_0003 {
            // Its report went to its standard output: the file.
            out(b"k32test: --child3 ended with ");
            out_num(code);
            out(b": ");
            let (b, n) = read_back(&out_path);
            out(&b[..n]);
            out(b"\n");
        }
        check(code == 0xC0DE_0003);
        // It set the event through its inherited copy.
        check(WaitForSingleObject(private[0], 0) == WAIT_OBJECT_0);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);
        // It wrote the files through the inherited handles.
        let (b, n) = read_back(&out_path);
        check(&b[..n] == CHILD_STDOUT);
        let (b, n) = read_back(&err_path);
        check(&b[..n] == CHILD_STDERR);
        check(CloseHandle(out_h) != 0 && CloseHandle(err_h) != 0 && CloseHandle(dup) != 0);
        for p in private {
            check(CloseHandle(p) != 0);
        }
        check(DeleteFileW(out_path.as_ptr()) != 0 && DeleteFileW(err_path.as_ptr()) != 0);

        // CreateProcessA: an ANSI environment block and a STARTUPINFOA.
        let mut si = [0u8; 104];
        si[0..4].copy_from_slice(&104u32.to_le_bytes());
        si[24..32].copy_from_slice(&(c"ansi title".as_ptr() as u64).to_le_bytes());
        let mut cmd = *b"k32test.exe --child4\0";
        let env = b"POLY_CHILD=ansi\0Path=C:\\Windows\\System32\0\0";
        check(
            CreateProcessA(np(), cmd.as_mut_ptr(), np(), np(), 0, 0, env.as_ptr(), np(), si.as_ptr(), pi.as_mut_ptr())
                != 0,
        );
        let ph = isize::from_le_bytes(pi[0..8].try_into().unwrap());
        let th = isize::from_le_bytes(pi[8..16].try_into().unwrap());
        check(WaitForSingleObject(ph, 30_000) == WAIT_OBJECT_0);
        check(GetExitCodeProcess(ph, &mut code) != 0 && code == 0xC0DE_0004);
        check(CloseHandle(th) != 0 && CloseHandle(ph) != 0);
    }
}

/// `--child3`: started suspended by `inheritance_and_startup` with
/// redirected standard output and error, inheriting; the numbers after the
/// flag are an inherited event, a handle it must not have, and its
/// creator's standard input.
unsafe fn child3(cmd: &[u16]) -> ! {
    // SAFETY: valid arguments throughout.
    unsafe {
        let (dup, hidden, input) = (arg_num(cmd, 0) as Handle, arg_num(cmd, 1) as Handle, arg_num(cmd, 2) as Handle);
        let out_h = GetStdHandle(STD_OUTPUT_HANDLE);
        let err_h = GetStdHandle(STD_ERROR_HANDLE);
        check(out_h != 0 && out_h != INVALID_HANDLE_VALUE && err_h != 0 && err_h != out_h);
        // A file, not a console: 7-Zip asks.
        check(GetFileType(out_h) == FILE_TYPE_DISK && GetFileType(err_h) == FILE_TYPE_DISK);
        let mut mode = 0u32;
        check(GetConsoleMode(out_h, &mut mode) == 0);
        check_error(ERROR_INVALID_HANDLE);
        let mut n = 0u32;
        check(WriteFile(out_h, CHILD_STDOUT.as_ptr(), CHILD_STDOUT.len() as u32, &mut n, core::ptr::null_mut()) != 0);
        check(n == CHILD_STDOUT.len() as u32);
        check(WriteFile(err_h, CHILD_STDERR.as_ptr(), CHILD_STDERR.len() as u32, &mut n, core::ptr::null_mut()) != 0);
        // The STARTUPINFO its creator passed.
        let mut si = [0xFFu8; 104];
        GetStartupInfoW(si.as_mut_ptr());
        let u32_at = |o: usize| u32::from_le_bytes(si[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(si[o..o + 8].try_into().unwrap());
        check(u32_at(0) == 104 && u32_at(60) == STARTF_USESTDHANDLES && u32_at(32) == 0x11);
        check(u64_at(16) == 0 && u64_at(24) != 0 && ascii_eq(wslice(u64_at(24) as *const u16), b"poly child"));
        check(u64_at(80) as Handle == input && u64_at(88) as Handle == out_h && u64_at(96) as Handle == err_h);
        // Its standard input: the creator's, inherited.
        check(GetStdHandle(STD_INPUT_HANDLE) == input);
        check(input == 0 || GetFileType(input) != 0);
        // The inherited copy is its creator's event; the other handle is
        // not there.
        check(SetEvent(dup) != 0);
        let mut f = 0u32;
        check(GetHandleInformation(hidden, &mut f) == 0);
        check_error(ERROR_INVALID_HANDLE);
        ExitProcess(0xC0DE_0003)
    }
}

/// `--child4`: started by `CreateProcessA` with an ANSI environment block
/// and a `STARTUPINFOA`.
unsafe fn child4() -> ! {
    // SAFETY: valid arguments throughout.
    unsafe {
        let mut v = [0u16; 64];
        let k = GetEnvironmentVariableW(w!("POLY_CHILD"), v.as_mut_ptr(), 64) as usize;
        check(ascii_eq(&v[..k], b"ansi"));
        let mut si = [0u8; 104];
        GetStartupInfoA(si.as_mut_ptr());
        let title = u64::from_le_bytes(si[24..32].try_into().unwrap()) as *const u8;
        check(!title.is_null());
        let mut len = 0;
        while *title.add(len) != 0 {
            len += 1;
        }
        check(core::slice::from_raw_parts(title, len) == b"ansi title");
        GetStartupInfoW(si.as_mut_ptr());
        let title = u64::from_le_bytes(si[24..32].try_into().unwrap()) as *const u16;
        check(!title.is_null() && ascii_eq(wslice(title), b"ansi title"));
        check(si[60..64] == [0; 4]);
        ExitProcess(0xC0DE_0004)
    }
}
