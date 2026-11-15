//! A 32-bit program whose image carries a safe exception handler table (ADR 0186, stage 4: the
//! SafeSEH check of the i686 `ntdll`). Its load configuration (`__load_config_used`, which the
//! linker points the load configuration directory at) names one handler, `good_handler`, in
//! `SEHandlerTable`/`SEHandlerCount` (the PE format's "Load Configuration Structure": a table of
//! RVAs).
//!
//! - run without an argument: a frame linked at `fs:[0]` with `good_handler` (in the table)
//!   receives a `RaiseException` and continues; the program prints `safeseh` and ends with 0;
//! - run with `bad` at the end of the command line: the frame's handler is `bad_handler`, which
//!   the table does not list. The system must not call it: the exception becomes
//!   `STATUS_INVALID_EXCEPTION_HANDLER` (0xC00001A5), which ends the process (an exit code of 3
//!   means the exception returned, 4 that `bad_handler` ran).

#![no_std]
#![no_main]

extern crate poly_msvc32;

type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut u8) -> i32;
    fn ExitProcess(code: u32) -> !;
    fn RaiseException(code: u32, flags: u32, count: u32, arguments: *const usize);
    fn GetCommandLineW() -> *const u16;
}

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
/// The software exception code the test raises (a customer code: bit 29 set).
const MY_CODE: u32 = 0xE000_5AFE;

static RAN_GOOD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static RAN_BAD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// IMAGE_LOAD_CONFIG_DIRECTORY32 (`winnt.h`) up to `SEHandlerCount`: 72 bytes, what Visual C++'s
// `loadcfg.c` gives every image. `___safe_se_handler_table` and `___safe_se_handler_count` are
// the linker's own symbols (the VA of the sorted table of RVAs it builds from the `.safeseh`
// directives of the objects, and its length); with `/safeseh` it also leaves `NO_SEH` clear.
core::arch::global_asm!(
    ".section .rdata,\"dr\"",
    ".p2align 2",
    ".globl __load_config_used",
    "__load_config_used:",
    ".long 72",
    ".long 0",
    ".short 0, 0",
    ".long 0, 0, 0",
    ".long 0, 0, 0, 0, 0, 0",
    ".long 0",
    ".short 0, 0",
    ".long 0",
    ".long 0",
    ".long ___safe_se_handler_table",
    ".long ___safe_se_handler_count",
    ".text",
    ".safeseh _good_handler",
);

fn out(text: &[u8]) {
    let mut written = 0u32;
    // SAFETY: plain kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        WriteFile(h, text.as_ptr(), text.len() as u32, &mut written, core::ptr::null_mut());
    }
}

/// A frame handler (`EXCEPTION_DISPOSITION handler(record, frame, context, dispatcher)`): in the
/// table. It continues execution.
#[unsafe(no_mangle)]
pub extern "C" fn good_handler(_record: *mut u8, _frame: u32, _context: *mut u8, _dispatcher: *mut u8) -> u32 {
    RAN_GOOD.store(1, core::sync::atomic::Ordering::SeqCst);
    // ExceptionContinueExecution.
    0
}

/// A frame handler the table does not list.
#[unsafe(no_mangle)]
pub extern "C" fn bad_handler(_record: *mut u8, _frame: u32, _context: *mut u8, _dispatcher: *mut u8) -> u32 {
    RAN_BAD.store(1, core::sync::atomic::Ordering::SeqCst);
    0
}

/// Whether the command line ends with `bad`.
fn wants_bad() -> bool {
    // SAFETY: the process's NUL-terminated command line.
    unsafe {
        let line = GetCommandLineW();
        let mut n = 0;
        while line.add(n).read() != 0 {
            n += 1;
        }
        n >= 3 && line.add(n - 3).read() == b'b' as u16 && line.add(n - 2).read() == b'a' as u16 && line.add(n - 1).read() == b'd' as u16
    }
}

/// Link a frame with `handler`, raise `MY_CODE`, unlink the frame.
fn raise_under(handler: usize) {
    // SAFETY: the frame is pushed, linked and unlinked and popped again by the same block.
    unsafe {
        core::arch::asm!(
            "push edx",
            "mov eax, fs:[0]",
            "push eax",
            "mov fs:[0], esp",
            "push 0",
            "push 0",
            "push 0",
            "push {code}",
            "call {raise}",
            "pop ecx",
            "mov fs:[0], ecx",
            "add esp, 4",
            inout("edx") handler => _,
            code = const MY_CODE,
            raise = sym RaiseException,
            clobber_abi("C"),
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> u32 {
    if wants_bad() {
        raise_under(bad_handler as *const () as usize);
        // Not reached: the invalid handler ends the process.
        // SAFETY: ends the process.
        unsafe { ExitProcess(if RAN_BAD.load(core::sync::atomic::Ordering::SeqCst) != 0 { 4 } else { 3 }) }
    }
    raise_under(good_handler as *const () as usize);
    if RAN_GOOD.load(core::sync::atomic::Ordering::SeqCst) == 0 {
        // SAFETY: ends the process.
        unsafe { ExitProcess(2) }
    }
    out(b"safeseh\n");
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
