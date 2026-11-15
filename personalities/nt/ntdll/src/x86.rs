//! What the i686 `ntdll` of a 32-bit process does differently (ADR 0186): its thread entry, the
//! call of a start routine, `RtlCaptureContext` for the x86 `CONTEXT`, and 32-bit structured
//! exception handling.
//!
//! The kernel starts every thread of a 32-bit process at `LdrInitializeThunk` as if it had been
//! called (`cdecl`) with the start routine, its argument and the executable's base on the stack,
//! under a zero return address (Poly's own convention, `poly_personality_nt::wow64::start_esp`),
//! with FS based at the thread's TEB.
//!
//! # Exceptions
//!
//! A 32-bit thread's exception handlers are frame-based: `NT_TIB.ExceptionList` (`fs:[0]`) heads
//! a chain of `EXCEPTION_REGISTRATION_RECORD`s (`winnt.h`: `Next`, `Handler`) on its stack, the
//! newest first, ended by -1 (Learn: "Structured Exception Handling"; the x86 `winnt.h`
//! definitions). The kernel delivers a CPU exception, or `NtRaiseException`'s, to
//! `KiUserExceptionDispatcher(record, context)` with the x86 `EXCEPTION_RECORD` and `CONTEXT` on
//! the stack (`poly_personality_nt::wow64::context`); `RtlRaiseException` dispatches in the
//! process directly. The dispatch: the vectored handlers, then each frame's handler, called as
//! `handler(record, frame, context, dispatcher_context)` (`cdecl`, the documented
//! `EXCEPTION_ROUTINE`), which answers `ExceptionContinueExecution` (0: resume the — possibly
//! changed — context, refused for a non-continuable exception) or `ExceptionContinueSearch` (1);
//! a frame outside the thread's stack ends the walk. Nobody handling it: the top-level filter,
//! then a second-chance `NtRaiseException` (the kernel ends the process with the code).
//! `RtlUnwind` calls the handlers of the frames above a target frame with `EXCEPTION_UNWINDING`,
//! unlinks them, and continues at the target address.

use crate::syscalls;

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

    .globl _poly_call_handler
_poly_call_handler:
    push ebp
    mov  ebp, esp
    push dword ptr [ebp + 24]
    push dword ptr [ebp + 20]
    push dword ptr [ebp + 16]
    push dword ptr [ebp + 12]
    call dword ptr [ebp + 8]
    mov  esp, ebp
    pop  ebp
    ret

    /* A user APC (Poly's own frame, `poly_personality_nt::wow64::apc_frame`): ESP points at
       the routine and its three arguments. Call `routine(arg1, arg2, arg3)` (stdcall), then
       PolyResumeApc, which restores the interrupted context and never returns here. */
    .globl _KiUserApcDispatcher
_KiUserApcDispatcher:
    pop  eax
    call eax
    call "_PolyResumeApc@0"
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

    /* RtlRaiseException(record): the caller's registers as they will be after the call
       returns (EIP its return address, ESP above the argument, EBP its own), then the
       dispatch, which does not come back here. */
    .globl "_RtlRaiseException@4"
"_RtlRaiseException@4":
    push ebp
    mov  ebp, esp
    sub  esp, 0x2E0
    and  esp, -16
    push esp
    call "_RtlCaptureContext@4"
    mov  eax, dword ptr [ebp + 4]
    mov  dword ptr [esp + 0xB8], eax
    lea  eax, [ebp + 12]
    mov  dword ptr [esp + 0xC4], eax
    mov  eax, dword ptr [ebp]
    mov  dword ptr [esp + 0xB4], eax
    mov  eax, esp
    push eax
    push dword ptr [ebp + 8]
    call {raise}
    int3

    /* RtlUnwind(target frame, target IP, record, return value): continue, once the frames
       above the target are unwound, at the target IP (or the return address) with the
       caller's other registers and EAX the return value. */
    .globl "_RtlUnwind@16"
"_RtlUnwind@16":
    push ebp
    mov  ebp, esp
    sub  esp, 0x2E0
    and  esp, -16
    push esp
    call "_RtlCaptureContext@4"
    mov  eax, dword ptr [ebp + 12]
    test eax, eax
    jnz  2f
    mov  eax, dword ptr [ebp + 4]
2:
    mov  dword ptr [esp + 0xB8], eax
    lea  eax, [ebp + 24]
    mov  dword ptr [esp + 0xC4], eax
    mov  eax, dword ptr [ebp]
    mov  dword ptr [esp + 0xB4], eax
    mov  eax, dword ptr [ebp + 20]
    mov  dword ptr [esp + 0xB0], eax
    mov  eax, esp
    push eax
    push dword ptr [ebp + 16]
    push dword ptr [ebp + 8]
    call {unwind}
    int3
"#,
    init = sym crate::start::ntdll_thread_init,
    raise = sym raise_in_process,
    unwind = sym unwind,
);

unsafe extern "C" {
    /// Call `routine(argument)` and return its result, with ESP restored afterwards: a
    /// `stdcall` routine pops its argument, a `cdecl` one (an executable's entry point) does
    /// not, and the call works for both.
    pub fn poly_call_start(routine: usize, argument: usize) -> u32;
    /// Call an `EXCEPTION_ROUTINE` (`handler(record, frame, context, dispatcher_context)`),
    /// with ESP restored afterwards whatever convention it pops its arguments with.
    fn poly_call_handler(handler: u32, record: *mut u8, frame: u32, context: *mut u8, dc: *mut u8) -> u32;
}

/// `EXCEPTION_RECORD` (32-bit, `winnt.h`): `ExceptionCode` 0, `ExceptionFlags` 4,
/// `ExceptionRecord` 8, `ExceptionAddress` 12, `NumberParameters` 16; 0x50 bytes.
const RECORD_CODE: usize = 0;
const RECORD_FLAGS: usize = 4;
const RECORD_NESTED: usize = 8;
const RECORD_ADDRESS: usize = 12;
const RECORD_SIZE: usize = 0x50;
/// `CONTEXT.Eip` (32-bit).
const CONTEXT_EIP: usize = 0xB8;
/// `EXCEPTION_NONCONTINUABLE`, `EXCEPTION_UNWINDING`, `EXCEPTION_EXIT_UNWIND`,
/// `EXCEPTION_STACK_INVALID` (`winnt.h`).
const EXCEPTION_NONCONTINUABLE: u32 = 0x1;
const EXCEPTION_UNWINDING: u32 = 0x2;
const EXCEPTION_EXIT_UNWIND: u32 = 0x4;
const EXCEPTION_STACK_INVALID: u32 = 0x8;
/// `ExceptionContinueExecution` (`excpt.h`'s `EXCEPTION_DISPOSITION`).
const CONTINUE_EXECUTION: u32 = 0;
/// `STATUS_NONCONTINUABLE_EXCEPTION`, `STATUS_UNWIND` (`ntstatus.h`).
const STATUS_NONCONTINUABLE_EXCEPTION: u32 = 0xC000_0025;
const STATUS_UNWIND: u32 = 0xC000_0027;
/// The end of the frame chain.
const CHAIN_END: u32 = 0xFFFF_FFFF;
/// `EXCEPTION_CONTINUE_EXECUTION` (a filter's answer, `excpt.h`).
const FILTER_CONTINUE_EXECUTION: i32 = -1;

/// # Safety
/// `p + at` is readable.
unsafe fn u32_at(p: *const u8, at: usize) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { p.add(at).cast::<u32>().read_unaligned() }
}

/// # Safety
/// `p + at` is writable.
unsafe fn set_u32(p: *mut u8, at: usize, v: u32) {
    // SAFETY: the caller's contract.
    unsafe { p.add(at).cast::<u32>().write_unaligned(v) }
}

/// The head of the calling thread's frame chain (`fs:[0]`).
fn exception_list() -> u32 {
    let v: u32;
    // SAFETY: FS is the TEB; `NT_TIB.ExceptionList` is its first field.
    unsafe { core::arch::asm!("mov {}, fs:[0]", out(reg) v, options(nostack, readonly, preserves_flags)) };
    v
}

fn set_exception_list(v: u32) {
    // SAFETY: as above; the thread's own field.
    unsafe { core::arch::asm!("mov fs:[0], {}", in(reg) v, options(nostack, preserves_flags)) };
}

/// The calling thread's stack, `[StackLimit, StackBase)` (`NT_TIB`, at 8 and 4).
fn stack_range() -> (u32, u32) {
    let teb = crate::lock::teb() as usize as *const u8;
    // SAFETY: the TEB is valid.
    unsafe { (u32_at(teb, 8), u32_at(teb, 4)) }
}

/// Whether a registration record at `frame` lies on the thread's stack.
fn frame_ok(frame: u32) -> bool {
    let (limit, base) = stack_range();
    frame & 3 == 0 && frame >= limit && frame.checked_add(8).is_some_and(|end| end <= base)
}

/// Offer the exception to the vectored handlers and then to each frame's handler: whether one
/// continues execution (with `context` as it left it).
///
/// # Safety
/// `record` and `context` are a valid x86 `EXCEPTION_RECORD` and `CONTEXT`.
unsafe fn dispatch(record: *mut u8, context: *mut u8) -> bool {
    // SAFETY: the caller's structures.
    if unsafe { crate::veh::call_handlers(record, context) } {
        return true;
    }
    let mut frame = exception_list();
    let mut steps = 0;
    while frame != CHAIN_END && steps < 4096 {
        steps += 1;
        if !frame_ok(frame) {
            // SAFETY: the caller's record.
            unsafe { set_u32(record, RECORD_FLAGS, u32_at(record, RECORD_FLAGS) | EXCEPTION_STACK_INVALID) };
            return false;
        }
        let mut dc = [0u32; 1];
        // SAFETY: a registration record on the stack: its handler takes the documented
        // arguments.
        let disposition = unsafe {
            let handler = u32_at(frame as usize as *const u8, 4);
            poly_call_handler(handler, record, frame, context, dc.as_mut_ptr().cast())
        };
        if disposition == CONTINUE_EXECUTION {
            // SAFETY: the caller's record.
            if unsafe { u32_at(record, RECORD_FLAGS) } & EXCEPTION_NONCONTINUABLE == 0 {
                return true;
            }
            // A handler may not continue a non-continuable exception: that is an exception
            // of its own (Learn: `RaiseException`), whose record names the first.
            let mut nested = [0u8; RECORD_SIZE];
            // SAFETY: a record on our stack.
            unsafe {
                set_u32(nested.as_mut_ptr(), RECORD_CODE, STATUS_NONCONTINUABLE_EXCEPTION);
                set_u32(nested.as_mut_ptr(), RECORD_FLAGS, EXCEPTION_NONCONTINUABLE);
                set_u32(nested.as_mut_ptr(), RECORD_NESTED, record as u32);
                syscalls::NtRaiseException(nested.as_ptr(), context, 0);
            }
            return false;
        }
        // SAFETY: a checked registration record: `Next` is its first field.
        frame = unsafe { u32_at(frame as usize as *const u8, 0) };
    }
    false
}

/// The top-level filter of unhandled exceptions (0: none), `RtlSetUnhandledExceptionFilter`.
static UNHANDLED_FILTER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// After the frames: the top-level filter may still continue execution; otherwise the second
/// chance, which ends the process.
///
/// # Safety
/// As for [`dispatch`].
unsafe fn unhandled(record: *mut u8, context: *mut u8) -> ! {
    let filter = UNHANDLED_FILTER.load(core::sync::atomic::Ordering::Acquire);
    // SAFETY: the caller's record.
    let continuable = unsafe { u32_at(record, RECORD_FLAGS) } & EXCEPTION_NONCONTINUABLE == 0;
    if filter != 0 {
        let mut pointers = [record as u32, context as u32];
        type Filter = unsafe extern "system" fn(*mut u32) -> i32;
        // SAFETY: an `LPTOP_LEVEL_EXCEPTION_FILTER` given `EXCEPTION_POINTERS`.
        let r = unsafe { core::mem::transmute::<usize, Filter>(filter)(pointers.as_mut_ptr()) };
        if r == FILTER_CONTINUE_EXECUTION && continuable {
            // SAFETY: resume the context the filter may have fixed up.
            unsafe { syscalls::NtContinue(context, 0) };
        }
    }
    // SAFETY: second chance: the kernel ends the process with the code.
    unsafe { syscalls::NtRaiseException(record, context, 0) };
    fail_fast()
}

/// `KiUserExceptionDispatcher(record, context)`: where the kernel resumes a 32-bit thread on an
/// exception (see the module documentation).
///
/// # Safety
/// The kernel's frame: a valid x86 `EXCEPTION_RECORD` and `CONTEXT`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn KiUserExceptionDispatcher(record: *mut u8, context: *mut u8) -> ! {
    // SAFETY: the kernel's structures.
    unsafe {
        if dispatch(record, context) {
            syscalls::NtContinue(context, 0);
        }
        unhandled(record, context)
    }
}

/// The Rust half of `RtlRaiseException`: `context` is the caller's registers after the call.
///
/// # Safety
/// `record` is a valid, writable x86 `EXCEPTION_RECORD`; `context` a full x86 `CONTEXT`.
unsafe extern "C" fn raise_in_process(record: *mut u8, context: *mut u8) -> ! {
    // SAFETY: the caller's structures.
    unsafe {
        set_u32(record, RECORD_ADDRESS, u32_at(context, CONTEXT_EIP));
        if dispatch(record, context) {
            syscalls::NtContinue(context, 0);
        }
        unhandled(record, context)
    }
}

/// The Rust half of `RtlUnwind`: call the handlers of the frames above `target` (0: every frame,
/// an exit unwind) with `EXCEPTION_UNWINDING`, unlink them, and continue with `context`.
///
/// # Safety
/// `record` is null or a valid x86 `EXCEPTION_RECORD`; `context` a full x86 `CONTEXT`.
unsafe extern "C" fn unwind(target: u32, record: *mut u8, context: *mut u8) -> ! {
    let mut own = [0u8; RECORD_SIZE];
    let record = if record.is_null() {
        // SAFETY: a record on our stack.
        unsafe {
            set_u32(own.as_mut_ptr(), RECORD_CODE, STATUS_UNWIND);
            set_u32(own.as_mut_ptr(), RECORD_ADDRESS, u32_at(context, CONTEXT_EIP));
        }
        own.as_mut_ptr()
    } else {
        record
    };
    let exit = if target == 0 { EXCEPTION_EXIT_UNWIND } else { 0 };
    // SAFETY: a valid record.
    unsafe { set_u32(record, RECORD_FLAGS, u32_at(record, RECORD_FLAGS) | EXCEPTION_UNWINDING | exit) };
    let mut frame = exception_list();
    let mut steps = 0;
    while frame != CHAIN_END && frame != target && steps < 4096 && frame_ok(frame) {
        steps += 1;
        let mut dc = [0u32; 1];
        // SAFETY: a checked registration record; its handler takes the documented arguments.
        let next = unsafe {
            let f = frame as usize as *const u8;
            poly_call_handler(u32_at(f, 4), record, frame, context, dc.as_mut_ptr().cast());
            u32_at(f, 0)
        };
        set_exception_list(next);
        frame = next;
    }
    // SAFETY: resume the caller's context (at the target address, EAX the return value).
    unsafe { syscalls::NtContinue(context, 0) };
    fail_fast()
}

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

/// A fail-fast (`__fastfail`): what is left when a context cannot be resumed.
fn fail_fast() -> ! {
    // SAFETY: int 0x29 with the code in ECX does not return.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}
