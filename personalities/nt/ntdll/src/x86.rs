//! What the i686 `ntdll` of a 32-bit process does differently (ADR 0186): its thread entry, the
//! call of a start routine, `RtlCaptureContext` for the x86 `CONTEXT`, and the exception exports
//! of the first milestone.
//!
//! The kernel starts every thread of a 32-bit process at `LdrInitializeThunk` as if it had been
//! called (`cdecl`) with the start routine, its argument and the executable's base on the stack,
//! under a zero return address (Poly's own convention, `poly_personality_nt::wow64::start_esp`),
//! with FS based at the thread's TEB.
//!
//! Exceptions are not dispatched to 32-bit code yet (ADR 0186's staged plan): the kernel ends a
//! 32-bit process on a CPU exception, and `RtlRaiseException` ends it with the code, as an
//! unhandled exception would.

use crate::syscalls;
use poly_ntobj::NtStatus;

core::arch::global_asm!(
    r#"
    .text
    .globl _LdrInitializeThunk
_LdrInitializeThunk:
    jmp {init}

    .globl _poly_call_start
_poly_call_start:
    push ebp
    mov  ebp, esp
    push dword ptr [ebp + 12]
    call dword ptr [ebp + 8]
    mov  esp, ebp
    pop  ebp
    ret

    .globl _KiUserApcDispatcher
_KiUserApcDispatcher:
    int3

    .globl "_RtlCaptureContext@4"
"_RtlCaptureContext@4":
    push ebx
    mov  ebx, dword ptr [esp + 8]
    mov  dword ptr [ebx + 0xB0], eax
    mov  dword ptr [ebx + 0xAC], ecx
    mov  dword ptr [ebx + 0xA8], edx
    mov  eax, dword ptr [esp]
    mov  dword ptr [ebx + 0xA4], eax
    mov  dword ptr [ebx + 0xA0], esi
    mov  dword ptr [ebx + 0x9C], edi
    mov  dword ptr [ebx + 0xB4], ebp
    mov  eax, dword ptr [esp + 4]
    mov  dword ptr [ebx + 0xB8], eax
    lea  eax, [esp + 12]
    mov  dword ptr [ebx + 0xC4], eax
    pushfd
    pop  eax
    mov  dword ptr [ebx + 0xC0], eax
    xor  eax, eax
    mov  ax, cs
    mov  dword ptr [ebx + 0xBC], eax
    mov  ax, ss
    mov  dword ptr [ebx + 0xC8], eax
    mov  ax, ds
    mov  dword ptr [ebx + 0x98], eax
    mov  ax, es
    mov  dword ptr [ebx + 0x94], eax
    mov  ax, fs
    mov  dword ptr [ebx + 0x90], eax
    mov  ax, gs
    mov  dword ptr [ebx + 0x8C], eax
    mov  dword ptr [ebx], 0x10007
    mov  eax, dword ptr [ebx + 0xB0]
    pop  ebx
    ret  4
"#,
    init = sym crate::start::ntdll_thread_init,
);

unsafe extern "C" {
    /// Call `routine(argument)` and return its result, with ESP restored afterwards: a
    /// `stdcall` routine pops its argument, a `cdecl` one (an executable's entry point) does
    /// not, and the call works for both.
    pub fn poly_call_start(routine: usize, argument: usize) -> u32;
}

/// `EXCEPTION_RECORD.ExceptionCode` and `.ExceptionAddress` (`winnt.h`, 32-bit: 0 and 12).
const RECORD_CODE: usize = 0;
const RECORD_ADDRESS: usize = 12;

/// End the process with an exception's code: what an exception nobody handles does.
fn unhandled(code: u32) -> ! {
    crate::debug_print(&alloc::format!("wow64: unhandled exception {code:#x} (32-bit dispatch is not there yet)"));
    // SAFETY: ends the process.
    unsafe { syscalls::NtTerminateProcess(syscalls::CURRENT_PROCESS, NtStatus(code)) };
    loop {
        core::hint::spin_loop();
    }
}

/// `KiUserExceptionDispatcher` (the export the kernel looks up; not entered yet for a 32-bit
/// process, ADR 0186): the exception is not handled.
///
/// # Safety
/// `record` is a valid `EXCEPTION_RECORD`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn KiUserExceptionDispatcher(record: *mut u8, _context: *mut u8) -> ! {
    // SAFETY: the caller's record.
    unhandled(unsafe { record.add(RECORD_CODE).cast::<u32>().read_unaligned() })
}

/// `RtlRaiseException(ExceptionRecord)`: without 32-bit dispatch, the exception goes unhandled.
///
/// # Safety
/// `record` is a valid, writable `EXCEPTION_RECORD`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlRaiseException(record: *mut u8) {
    // SAFETY: the caller's record.
    let code = unsafe {
        record.add(RECORD_ADDRESS).cast::<u32>().write_unaligned(0);
        record.add(RECORD_CODE).cast::<u32>().read_unaligned()
    };
    unhandled(code)
}

/// The top-level filter of unhandled exceptions (kept for when 32-bit dispatch comes).
static UNHANDLED_FILTER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// `RtlSetUnhandledExceptionFilter(filter)`.
#[unsafe(no_mangle)]
pub extern "system" fn RtlSetUnhandledExceptionFilter(filter: usize) {
    UNHANDLED_FILTER.store(filter, core::sync::atomic::Ordering::Release);
}

/// `RtlPcToFileHeader`: the base of the image containing `pc`.
///
/// # Safety
/// `base` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlPcToFileHeader(pc: *mut u8, base: *mut *mut u8) -> *mut u8 {
    let b = crate::with_loader(|l, _| l.find(pc as u64)).map_or(0, |m| m.base);
    if !base.is_null() {
        // SAFETY: a valid out-pointer.
        unsafe { base.write(b as usize as *mut u8) };
    }
    b as usize as *mut u8
}

/// `RtlImageNtHeader`: the `IMAGE_NT_HEADERS` of a mapped image.
///
/// # Safety
/// `base` is null or a mapped image.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlImageNtHeader(base: *mut u8) -> *mut u8 {
    if base.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: a mapped image begins with the DOS header.
    unsafe {
        if (base as *const u16).read() != 0x5A4D {
            return core::ptr::null_mut();
        }
        let nt = base.add((base.add(0x3C) as *const u32).read() as usize);
        if (nt as *const u32).read() != 0x0000_4550 {
            return core::ptr::null_mut();
        }
        nt
    }
}

/// `RtlCaptureStackBackTrace`: no 32-bit stack walk yet; nothing captured.
#[unsafe(no_mangle)]
pub extern "system" fn RtlCaptureStackBackTrace(_skip: u32, _count: u32, _frames: *mut *mut u8, hash: *mut u32) -> u16 {
    if !hash.is_null() {
        // SAFETY: the caller's ULONG.
        unsafe { hash.write(0) };
    }
    0
}
