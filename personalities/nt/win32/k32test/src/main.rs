//! `k32test.exe` (ADR 0078): the program the kernel's ring-3 test
//! `nt_kernel32_program_runs_through_forwarders_to_kernelbase` runs. It
//! imports only from `kernel32.dll`, so every call goes through
//! `kernel32`'s API-set forwarders to `kernelbase`, and checks the
//! documented behaviour of a first slice of the Win32 API (Learn's pages).
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
    out(b"k32test: start\n");
    // SAFETY: every call is a kernel32 export with valid arguments.
    unsafe {
        errors_and_environment();
        files();
        memory();
        threads_and_sync();
        system_and_modules();
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
