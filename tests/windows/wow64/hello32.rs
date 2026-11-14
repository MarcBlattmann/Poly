//! The first 32-bit Windows program Poly runs (ADR 0186): built for `i686-pc-windows-msvc` by
//! `cargo xtask wow64`, it imports from `kernel32.dll` (and `RtlUnwind` from `ntdll.dll`), as an
//! ordinary 32-bit console program does, and checks, one line on standard output each:
//!
//! - `hello`: `GetStdHandle(STD_OUTPUT_HANDLE)` and `WriteFile` on the console;
//! - `last error`: `SetLastError` / `GetLastError` keep a value per thread;
//! - `wow64 view`: the address of a local variable is below 4 GiB, and `IsWow64Process` says
//!   the process is a 32-bit one on a 64-bit system (Learn: `IsWow64Process`);
//! - `file`: a file in `C:\\Windows\\Temp` written with `CreateFileW`/`WriteFile`, read back
//!   with `ReadFile`, its size from `GetFileSize`, then deleted;
//! - `thread`: `CreateThread` runs a second 32-bit thread with its argument, its own id and its
//!   own last error; `WaitForSingleObject` and `GetExitCodeThread` see it end with its result;
//! - `seh`: an access violation and a `RaiseException` under a frame-based handler linked at
//!   `fs:[0]` (what `__try` compiles to) reach it and continue, and `RtlUnwind` (imported from
//!   `ntdll`) unwinds an inner frame.
//!
//! It ends with `ExitProcess(0)` when every check passed, else with the number of the first
//! one that failed. Not part of the x64 test corpus (`tests/windows/*.rs`): the oracle pipeline
//! builds those for x64 only.

#![no_std]
#![no_main]

// The compiler's 32-bit helper routines (`__chkstk`, 64-bit division), which a C runtime would
// bring (ADR 0186).
extern crate poly_msvc32;

type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn ReadFile(file: Handle, buffer: *mut u8, length: u32, read: *mut u32, overlapped: *mut u8) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut u8,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn GetFileSize(file: Handle, high: *mut u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
    fn GetCurrentProcess() -> Handle;
    fn IsWow64Process(process: Handle, wow64: *mut i32) -> i32;
    fn CreateThread(
        security: *mut u8,
        stack: usize,
        start: extern "system" fn(usize) -> u32,
        parameter: usize,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeThread(thread: Handle, code: *mut u32) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn RaiseException(code: u32, flags: u32, count: u32, arguments: *const usize);
    fn ExitProcess(code: u32) -> !;
}

#[link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn RtlUnwind(target_frame: *mut u8, target_ip: *mut u8, record: *mut u8, return_value: usize);
}

/// `STD_OUTPUT_HANDLE`, `GENERIC_READ`, `GENERIC_WRITE`, `CREATE_ALWAYS`, `OPEN_EXISTING`,
/// `FILE_ATTRIBUTE_NORMAL`, `INVALID_HANDLE_VALUE` (`winbase.h`, `winnt.h`, `fileapi.h`).
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_HANDLE_VALUE: Handle = -1;

fn out(text: &[u8]) -> bool {
    let mut written = 0u32;
    // SAFETY: plain kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        WriteFile(h, text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut()) != 0
            && written == text.len() as u32
    }
}

fn last_error() -> bool {
    // SAFETY: plain kernel32 calls.
    unsafe {
        SetLastError(1234);
        let first = GetLastError();
        SetLastError(0);
        first == 1234 && GetLastError() == 0
    }
}

fn wow64_view() -> bool {
    let local = 0u32;
    let mut wow64 = 0i32;
    // SAFETY: a valid out-pointer.
    let ok = unsafe { IsWow64Process(GetCurrentProcess(), &mut wow64) };
    let address = core::hint::black_box(&local) as *const u32 as usize as u64;
    ok != 0 && wow64 != 0 && address < 1 << 32
}

/// What the second thread saw: its id and its argument, and its last error before and after it
/// set one.
static THREAD_SAW: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static THREAD_ERRORS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

extern "system" fn second_thread(parameter: usize) -> u32 {
    // SAFETY: plain kernel32 calls.
    unsafe {
        let me = GetCurrentThreadId();
        THREAD_SAW.store(me ^ parameter as u32, core::sync::atomic::Ordering::SeqCst);
        // A new thread starts with no last error, and its own.
        let before = GetLastError();
        SetLastError(7);
        THREAD_ERRORS.store(before << 16 | GetLastError(), core::sync::atomic::Ordering::SeqCst);
    }
    parameter as u32 + 1
}

/// The first step of the thread check that failed, or `None`.
fn thread_step() -> Option<&'static [u8]> {
    let mut id = 0u32;
    // SAFETY: plain kernel32 calls; the routine has the documented `ThreadProc` signature.
    unsafe {
        SetLastError(5);
        let h = CreateThread(core::ptr::null_mut(), 0, second_thread, 41, 0, &mut id);
        if h == 0 {
            return Some(b"CreateThread");
        }
        let waited = WaitForSingleObject(h, 10_000);
        let mut code = 0u32;
        let got = GetExitCodeThread(h, &mut code) != 0;
        CloseHandle(h);
        let saw = THREAD_SAW.load(core::sync::atomic::Ordering::SeqCst);
        let errors = THREAD_ERRORS.load(core::sync::atomic::Ordering::SeqCst);
        if waited != 0 {
            Some(b"WaitForSingleObject")
        } else if !got || code != 42 {
            Some(b"GetExitCodeThread")
        } else if id == 0 || id == GetCurrentThreadId() || saw != id ^ 41 {
            Some(b"thread id")
        } else if errors != 7 {
            Some(b"last error of the second thread")
        } else {
            None
        }
    }
}

fn thread() -> bool {
    match thread_step() {
        None => true,
        Some(step) => {
            out(b"thread step failed: ");
            out(step);
            out(b"\n");
            false
        }
    }
}

// ----- structured exception handling ---------------------------------------------------------
//
// A 32-bit program's handlers are frame-based: an `EXCEPTION_REGISTRATION_RECORD` (`Next`,
// `Handler`) pushed on the stack and linked at `fs:[0]`, which is what a compiler makes of
// `__try` (`winnt.h`; Learn: "Structured Exception Handling").

use core::sync::atomic::{AtomicU32, Ordering};

/// What the handler saw: the exception code, its first parameter, and the unwind calls.
static SEEN_CODE: AtomicU32 = AtomicU32::new(0);
static SEEN_PARAMETER: AtomicU32 = AtomicU32::new(0);
static UNWOUND: AtomicU32 = AtomicU32::new(0);

/// `STATUS_ACCESS_VIOLATION`, `EXCEPTION_UNWINDING` (`ntstatus.h`, `winnt.h`).
const ACCESS_VIOLATION: u32 = 0xC000_0005;
const EXCEPTION_UNWINDING: u32 = 2;
/// The program's own exception code.
const MY_CODE: u32 = 0xE000_1234;

/// An `EXCEPTION_ROUTINE` (`cdecl`): note the exception, step over the faulting 6-byte store of
/// an access violation, continue execution (`ExceptionContinueExecution`); during an unwind,
/// count the call and go on (`ExceptionContinueSearch`).
unsafe extern "C" fn handler(record: *mut u32, _frame: *mut u8, context: *mut u8, _dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's x86 EXCEPTION_RECORD (code 0, flags 4, parameters from 20) and
    // CONTEXT (`Eip` at 0xB8).
    unsafe {
        if record.add(1).read() & EXCEPTION_UNWINDING != 0 {
            UNWOUND.fetch_add(1, Ordering::SeqCst);
            return 1;
        }
        let code = record.read();
        SEEN_CODE.store(code, Ordering::SeqCst);
        SEEN_PARAMETER.store(record.add(5).read(), Ordering::SeqCst);
        if code == ACCESS_VIOLATION {
            let eip = context.add(0xB8).cast::<u32>();
            eip.write(eip.read() + 6);
        }
    }
    0
}

extern "C" fn raise_mine() {
    let parameter = [0x42usize];
    // SAFETY: a plain kernel32 call with one argument.
    unsafe { RaiseException(MY_CODE, 0, 1, parameter.as_ptr()) };
}

/// The first SEH step that failed, or `None`.
fn seh_step() -> Option<&'static [u8]> {
    // A store to address 0 under a frame whose handler steps over it.
    // SAFETY: the frame is pushed, linked, and unlinked and popped again by the same block; the
    // handler resumes after the 6-byte `mov dword ptr [eax], 1` (C7 00 01 00 00 00).
    unsafe {
        core::arch::asm!(
            "push edx",
            "mov eax, fs:[0]",
            "push eax",
            "mov fs:[0], esp",
            "xor eax, eax",
            ".byte 0xC7, 0x00, 0x01, 0x00, 0x00, 0x00",
            "pop ecx",
            "mov fs:[0], ecx",
            "add esp, 4",
            inout("edx") handler as *const () as usize => _,
            out("eax") _,
            out("ecx") _,
        );
    }
    if SEEN_CODE.load(Ordering::SeqCst) != ACCESS_VIOLATION || SEEN_PARAMETER.load(Ordering::SeqCst) != 1 {
        return Some(b"an access violation was not handled as a write");
    }
    // A software exception (`RaiseException`), its handler continuing execution: the call
    // returns.
    // SAFETY: as above; the raising function is called with the frame linked.
    unsafe {
        core::arch::asm!(
            "push edx",
            "mov eax, fs:[0]",
            "push eax",
            "mov fs:[0], esp",
            "call {raise}",
            "pop ecx",
            "mov fs:[0], ecx",
            "add esp, 4",
            inout("edx") handler as *const () as usize => _,
            raise = sym raise_mine,
            clobber_abi("C"),
        );
    }
    if SEEN_CODE.load(Ordering::SeqCst) != MY_CODE || SEEN_PARAMETER.load(Ordering::SeqCst) != 0x42 {
        return Some(b"RaiseException did not reach the handler");
    }
    // `RtlUnwind` to an outer frame: the inner frame's handler is called once, unwinding, the
    // chain is the outer frame's again, and the call returns.
    let (before, after): (u32, u32);
    // SAFETY: two frames are pushed and linked; RtlUnwind unlinks the inner one and returns
    // (`stdcall`, its 16 bytes popped); the block pops both and restores the chain.
    unsafe {
        core::arch::asm!(
            "mov ebx, fs:[0]",
            "push edx",
            "mov eax, fs:[0]",
            "push eax",
            "mov fs:[0], esp",
            "mov edi, esp",
            "push edx",
            "mov eax, fs:[0]",
            "push eax",
            "mov fs:[0], esp",
            "push 0",
            "push 0",
            "push 0",
            "push edi",
            "call {unwind}",
            "mov eax, fs:[0]",
            "sub eax, edi",
            "add esp, 8",
            "pop ecx",
            "mov fs:[0], ecx",
            "add esp, 4",
            inout("edx") handler as *const () as usize => _,
            unwind = sym RtlUnwind,
            out("ebx") before,
            out("eax") after,
            out("edi") _,
            clobber_abi("C"),
        );
    }
    if UNWOUND.load(Ordering::SeqCst) != 1 || after != 0 {
        return Some(b"RtlUnwind did not unwind the inner frame");
    }
    // SAFETY: FS is the TEB.
    let now: u32 = unsafe {
        let v: u32;
        core::arch::asm!("mov {}, fs:[0]", out(reg) v);
        v
    };
    if now != before {
        return Some(b"the frame chain was not restored");
    }
    None
}

fn seh() -> bool {
    match seh_step() {
        None => true,
        Some(step) => {
            out(b"seh step failed: ");
            out(step);
            out(b"\n");
            false
        }
    }
}

fn file() -> bool {
    let mut name = [0u16; 64];
    for (d, s) in name.iter_mut().zip("C:\\Windows\\Temp\\poly-hello32.txt".encode_utf16()) {
        *d = s;
    }
    let data = b"written by a 32-bit program\n";
    // SAFETY: plain kernel32 calls with valid buffers; `name` is NUL-terminated.
    unsafe {
        let h = CreateFileW(name.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut n = 0u32;
        let wrote = WriteFile(h, data.as_ptr(), data.len() as u32, &mut n, core::ptr::null_mut()) != 0;
        CloseHandle(h);
        if !wrote || n != data.len() as u32 {
            return false;
        }
        let h = CreateFileW(name.as_ptr(), GENERIC_READ, 0, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        let size = GetFileSize(h, core::ptr::null_mut());
        let mut back = [0u8; 64];
        let read = ReadFile(h, back.as_mut_ptr(), back.len() as u32, &mut n, core::ptr::null_mut()) != 0;
        CloseHandle(h);
        let deleted = DeleteFileW(name.as_ptr()) != 0;
        read && size == data.len() as u32 && &back[..n as usize] == data && deleted
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> u32 {
    let checks: [(&[u8], fn() -> bool); 5] = [
        (b"last error\n", last_error),
        (b"wow64 view\n", wow64_view),
        (b"file\n", file),
        (b"thread\n", thread),
        (b"seh\n", seh),
    ];
    if !out(b"hello\n") {
        // SAFETY: ends the process.
        unsafe { ExitProcess(1) }
    }
    for (i, (line, check)) in checks.iter().enumerate() {
        if !check() {
            out(b"failed: ");
            out(line);
            // SAFETY: ends the process.
            unsafe { ExitProcess(2 + i as u32) }
        }
        out(line);
    }
    // SAFETY: ends the process.
    unsafe { ExitProcess(0) }
}

/// MSVC-style code references this once floating point is used.
#[unsafe(no_mangle)]
#[used]
static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: ends the process.
    unsafe { ExitProcess(0xDEAD) }
}
