//! What the C runtime calls in `kernel32.dll` (ADR 0084: the runtime sits on
//! `kernel32`/`kernelbase` only, as the real one sits on their API sets).

pub type Handle = isize;
pub const INVALID_HANDLE_VALUE: Handle = -1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn ExitProcess(code: u32) -> !;
    pub fn GetLastError() -> u32;
    pub fn SetLastError(e: u32);
    pub fn GetStdHandle(which: u32) -> Handle;
    pub fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    pub fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, ov: *mut u8) -> i32;
    pub fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const u8,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    pub fn CloseHandle(h: Handle) -> i32;
    pub fn GetFileType(h: Handle) -> u32;
    pub fn SetFilePointerEx(h: Handle, distance: i64, new: *mut i64, method: u32) -> i32;
    pub fn FlushFileBuffers(h: Handle) -> i32;
    pub fn SetEndOfFile(h: Handle) -> i32;
    pub fn GetFileSizeEx(h: Handle, size: *mut i64) -> i32;
    pub fn GetCommandLineA() -> *const u8;
    pub fn GetCommandLineW() -> *const u16;
    pub fn GetEnvironmentStringsW() -> *mut u16;
    pub fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    pub fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    pub fn GetModuleFileNameW(module: *mut u8, buf: *mut u16, size: u32) -> u32;
    pub fn GetProcessHeap() -> *mut u8;
    pub fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    pub fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    pub fn HeapReAlloc(heap: *mut u8, flags: u32, p: *mut u8, bytes: usize) -> *mut u8;
    pub fn HeapSize(heap: *mut u8, flags: u32, p: *const u8) -> usize;
    pub fn TlsAlloc() -> u32;
    pub fn TlsGetValue(index: u32) -> *mut u8;
    pub fn TlsSetValue(index: u32, value: *mut u8) -> i32;
    pub fn InitializeCriticalSection(cs: *mut u8);
    pub fn EnterCriticalSection(cs: *mut u8);
    pub fn LeaveCriticalSection(cs: *mut u8);
    pub fn DeleteCriticalSection(cs: *mut u8);
    pub fn AcquireSRWLockExclusive(lock: *mut usize);
    pub fn ReleaseSRWLockExclusive(lock: *mut usize);
    pub fn CreateThread(
        sa: *const u8,
        stack: usize,
        start: unsafe extern "system" fn(*mut u8) -> u32,
        param: *mut u8,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
    pub fn ExitThread(code: u32) -> !;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetCurrentProcessId() -> u32;
    pub fn GetSystemTimeAsFileTime(ft: *mut u64);
    pub fn QueryPerformanceCounter(count: *mut i64) -> i32;
    pub fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
    pub fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
    pub fn WideCharToMultiByte(
        cp: u32,
        flags: u32,
        w: *const u16,
        wn: i32,
        s: *mut u8,
        n: i32,
        default: *const u8,
        used: *mut i32,
    ) -> i32;
    pub fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
    pub fn WriteConsoleW(h: Handle, buf: *const u16, chars: u32, written: *mut u32, reserved: *mut u8) -> i32;
    pub fn ReadConsoleW(h: Handle, buf: *mut u16, chars: u32, read: *mut u32, control: *mut u8) -> i32;
    pub fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    pub fn DeleteFileW(name: *const u16) -> i32;
    pub fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    pub fn CreateDirectoryW(name: *const u16, sa: *const u8) -> i32;
    pub fn RemoveDirectoryW(name: *const u16) -> i32;
    pub fn GetFileAttributesW(name: *const u16) -> u32;
    pub fn GetFullPathNameW(name: *const u16, size: u32, buf: *mut u16, part: *mut *mut u16) -> u32;
    pub fn GetCurrentDirectoryW(size: u32, buf: *mut u16) -> u32;
    pub fn SetCurrentDirectoryW(path: *const u16) -> i32;
    pub fn FindFirstFileW(pattern: *const u16, data: *mut u8) -> Handle;
    pub fn FindNextFileW(h: Handle, data: *mut u8) -> i32;
    pub fn FindClose(h: Handle) -> i32;
    pub fn Sleep(ms: u32);
}

/// `STD_INPUT_HANDLE` … (`processenv.h`).
pub const STD_HANDLES: [u32; 3] = [(-10i32) as u32, (-11i32) as u32, (-12i32) as u32];
/// `GetFileType` results (`fileapi.h`).
pub const FILE_TYPE_DISK: u32 = 1;
pub const FILE_TYPE_CHAR: u32 = 2;
pub const FILE_TYPE_PIPE: u32 = 3;
/// `HEAP_ZERO_MEMORY` (`winnt.h`).
pub const HEAP_ZERO_MEMORY: u32 = 0x8;
/// `CP_ACP` (`winnls.h`).
pub const CP_ACP: u32 = 0;
/// `FILE_BEGIN` … (`fileapi.h`).
pub const FILE_END: u32 = 2;
/// `MOVEFILE_COPY_ALLOWED` (`winbase.h`).
pub const MOVEFILE_COPY_ALLOWED: u32 = 2;
/// `INVALID_FILE_ATTRIBUTES`, `FILE_ATTRIBUTE_*` (`fileapi.h`, `winnt.h`).
pub const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
/// `ERROR_BROKEN_PIPE` (`winerror.h`).
pub const ERROR_BROKEN_PIPE: u32 = 109;
