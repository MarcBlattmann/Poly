//! `setjmp` and `longjmp` (ADR 0092), with the x64 `jmp_buf` of the public
//! `setjmp.h` (`_JUMP_BUFFER`, `_JBLEN` 16 `SETJMP_FLOAT128`s = 256
//! bytes): `Frame`, `Rbx`, `Rsp`, `Rbp`, `Rsi`, `Rdi`, `R12`…`R15`, `Rip`
//! at 8-byte steps from offset 0, `MxCsr` (u32) at 0x58, `FpCsr` (u16) at
//! 0x5C, `Spare`, then `Xmm6`…`Xmm15` (16 bytes each) from 0x60.
//!
//! **setjmp** (exported as `setjmp`, `_setjmp`, `_setjmpex` and the UCRT's
//! `__intrinsic_setjmp`/`__intrinsic_setjmpex`, as each DLL lists them)
//! saves the non-volatile registers, the caller's stack pointer and return
//! address, `MXCSR` and the x87 control word, and stores its second
//! argument (`RDX`, which MSVC's x64 code passes: the calling function's
//! frame) as `Frame`. It returns 0.
//!
//! **longjmp** (Learn: "In Microsoft C++ code on Windows, longjmp uses the
//! same stack-unwinding semantics as exception-handling code"): with a
//! nonzero `Frame` that lies on the current stack above the caller, it
//! unwinds to that frame with `RtlUnwindEx` and an exception record
//! `STATUS_LONGJUMP` (`ntstatus.h` 0x80000026) whose first parameter is the
//! `jmp_buf`, so every frame's language handler runs its termination code
//! (`__finally`, C++ destructors) on the way. The unwind resumes at a small
//! landing stub in the target frame, which then restores the registers
//! from the `jmp_buf` exactly as the plain path does. With `Frame` 0 (or
//! not on this stack) the registers are restored at once. A `value` of 0
//! returns 1 (C11 7.13.2.1, Learn). The value travels to the landing stub
//! in a thread-local slot of its own, so the unwind needs nothing from
//! `ntdll` beyond the documented `RtlUnwindEx`.
//!
//! Restoring the registers saved by `setjmp` (rather than those the unwind
//! recovers) is Poly's choice: Learn leaves "register variables"
//! unpredictable, and compilers that treat `setjmp` as returning twice
//! (GCC, Clang) expect the saved ones.

use crate::sys;
use core::sync::atomic::{AtomicU32, Ordering};

/// `STATUS_LONGJUMP` (`ntstatus.h`).
pub const STATUS_LONGJUMP: u32 = 0x8000_0026;
/// The size of `jmp_buf` on x64 (`setjmp.h`).
pub const JMP_BUF_SIZE: usize = 256;
/// `sizeof(CONTEXT)` on x64 (`winnt.h`).
const CONTEXT_SIZE: usize = 0x4D0;
/// `sizeof(EXCEPTION_RECORD)` on x64 (`winnt.h`).
const RECORD_SIZE: usize = 0x98;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCaptureContext(context: *mut u8);
    fn RtlUnwindEx(frame: u64, target: u64, record: *mut u8, value: u64, context: *mut u8, history: *mut u8);
}

/// The TLS index for the value on its way to the landing stub.
static SLOT: AtomicU32 = AtomicU32::new(u32::MAX);

fn slot() -> u32 {
    let s = SLOT.load(Ordering::Acquire);
    if s != u32::MAX {
        return s;
    }
    // SAFETY: no preconditions.
    let new = unsafe { sys::TlsAlloc() };
    match SLOT.compare_exchange(u32::MAX, new, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => new,
        Err(other) => other, // another thread won; the extra index is unused
    }
}

#[repr(C, align(16))]
struct Context([u8; CONTEXT_SIZE]);

/// `longjmp`'s unwinding path: returns only if `Frame` is not a frame of
/// this thread's stack above the caller (then the registers are restored
/// directly).
extern "C" fn unwind(buf: *mut u64, value: i32) {
    // SAFETY: `buf` is the program's jmp_buf.
    let frame = unsafe { buf.read() };
    let here = &frame as *const u64 as u64;
    let base: u64;
    // SAFETY: the TEB's NT_TIB.StackBase (`winnt.h`: offset 8) via GS.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) base, options(nostack, readonly, preserves_flags)) };
    if frame <= here || frame >= base {
        return;
    }
    let s = slot();
    // SAFETY: our own TLS slot.
    unsafe { sys::TlsSetValue(s, value as u32 as usize as *mut u8) };
    let mut record = [0u8; RECORD_SIZE];
    record[0..4].copy_from_slice(&STATUS_LONGJUMP.to_le_bytes());
    record[0x18..0x1C].copy_from_slice(&1u32.to_le_bytes()); // NumberParameters
    record[0x20..0x28].copy_from_slice(&(buf as u64).to_le_bytes()); // ExceptionInformation[0]
    let mut ctx = Context([0; CONTEXT_SIZE]);
    // SAFETY: a CONTEXT-sized, 16-byte aligned buffer; RtlUnwindEx with
    // our frame (checked above) resumes at the landing stub with RAX = buf
    // and does not return.
    unsafe {
        RtlCaptureContext(ctx.0.as_mut_ptr());
        RtlUnwindEx(
            frame,
            poly_crt_longjmp_landing as unsafe extern "C" fn() as usize as u64,
            record.as_mut_ptr(),
            buf as u64,
            ctx.0.as_mut_ptr(),
            core::ptr::null_mut(),
        );
    }
}

/// The value the landing stub returns from `setjmp`.
extern "C" fn pending() -> i32 {
    // SAFETY: our own TLS slot, set by `unwind` on this thread.
    unsafe { sys::TlsGetValue(slot()) as usize as i32 }
}

unsafe extern "C" {
    fn poly_crt_longjmp_landing();
}

core::arch::global_asm!(
    ".text",
    // setjmp(jmp_buf rcx, frame rdx): a leaf, no stack use.
    ".globl poly_crt_setjmp",
    ".p2align 4",
    "poly_crt_setjmp:",
    "mov qword ptr [rcx], rdx",
    "mov qword ptr [rcx + 0x08], rbx",
    "lea rax, [rsp + 8]",
    "mov qword ptr [rcx + 0x10], rax",
    "mov qword ptr [rcx + 0x18], rbp",
    "mov qword ptr [rcx + 0x20], rsi",
    "mov qword ptr [rcx + 0x28], rdi",
    "mov qword ptr [rcx + 0x30], r12",
    "mov qword ptr [rcx + 0x38], r13",
    "mov qword ptr [rcx + 0x40], r14",
    "mov qword ptr [rcx + 0x48], r15",
    "mov rax, qword ptr [rsp]",
    "mov qword ptr [rcx + 0x50], rax",
    "stmxcsr dword ptr [rcx + 0x58]",
    "fnstcw word ptr [rcx + 0x5C]",
    "movdqu xmmword ptr [rcx + 0x60], xmm6",
    "movdqu xmmword ptr [rcx + 0x70], xmm7",
    "movdqu xmmword ptr [rcx + 0x80], xmm8",
    "movdqu xmmword ptr [rcx + 0x90], xmm9",
    "movdqu xmmword ptr [rcx + 0xA0], xmm10",
    "movdqu xmmword ptr [rcx + 0xB0], xmm11",
    "movdqu xmmword ptr [rcx + 0xC0], xmm12",
    "movdqu xmmword ptr [rcx + 0xD0], xmm13",
    "movdqu xmmword ptr [rcx + 0xE0], xmm14",
    "movdqu xmmword ptr [rcx + 0xF0], xmm15",
    "xor eax, eax",
    "ret",
    // restore(jmp_buf rcx, value edx ≠ 0): back into the setjmp caller.
    ".p2align 4",
    "poly_crt_longjmp_restore:",
    "mov eax, edx",
    "mov rbx, qword ptr [rcx + 0x08]",
    "mov rsp, qword ptr [rcx + 0x10]",
    "mov rbp, qword ptr [rcx + 0x18]",
    "mov rsi, qword ptr [rcx + 0x20]",
    "mov rdi, qword ptr [rcx + 0x28]",
    "mov r12, qword ptr [rcx + 0x30]",
    "mov r13, qword ptr [rcx + 0x38]",
    "mov r14, qword ptr [rcx + 0x40]",
    "mov r15, qword ptr [rcx + 0x48]",
    "ldmxcsr dword ptr [rcx + 0x58]",
    "fnclex",
    "fldcw word ptr [rcx + 0x5C]",
    "movdqu xmm6, xmmword ptr [rcx + 0x60]",
    "movdqu xmm7, xmmword ptr [rcx + 0x70]",
    "movdqu xmm8, xmmword ptr [rcx + 0x80]",
    "movdqu xmm9, xmmword ptr [rcx + 0x90]",
    "movdqu xmm10, xmmword ptr [rcx + 0xA0]",
    "movdqu xmm11, xmmword ptr [rcx + 0xB0]",
    "movdqu xmm12, xmmword ptr [rcx + 0xC0]",
    "movdqu xmm13, xmmword ptr [rcx + 0xD0]",
    "movdqu xmm14, xmmword ptr [rcx + 0xE0]",
    "movdqu xmm15, xmmword ptr [rcx + 0xF0]",
    "jmp qword ptr [rcx + 0x50]",
    // The unwind resumes here in the target frame with RAX = jmp_buf.
    ".globl poly_crt_longjmp_landing",
    ".p2align 4",
    "poly_crt_longjmp_landing:",
    "mov rbx, rax",
    "and rsp, -16",
    "sub rsp, 32",
    "call {pending}",
    "mov rcx, rbx",
    "mov edx, eax",
    "jmp poly_crt_longjmp_restore",
    // longjmp(jmp_buf rcx, value edx): has unwind data, so RtlUnwindEx can
    // walk through it.
    ".globl poly_crt_longjmp",
    ".p2align 4",
    ".seh_proc poly_crt_longjmp",
    "poly_crt_longjmp:",
    "sub rsp, 40",
    ".seh_stackalloc 40",
    ".seh_endprologue",
    "test edx, edx",
    "jnz .Lpoly_longjmp_nonzero",
    "mov edx, 1",
    ".Lpoly_longjmp_nonzero:",
    "cmp qword ptr [rcx], 0",
    "je .Lpoly_longjmp_direct",
    "mov qword ptr [rsp + 48], rcx",
    "mov qword ptr [rsp + 56], rdx",
    "call {unwind}",
    "mov rcx, qword ptr [rsp + 48]",
    "mov rdx, qword ptr [rsp + 56]",
    ".Lpoly_longjmp_direct:",
    "jmp poly_crt_longjmp_restore",
    "int3",
    ".seh_endproc",
    pending = sym pending,
    unwind = sym unwind,
);

/// The `/EXPORT:` lines a DLL's `build.rs` gives the linker for the names
/// it lists (`name=poly_crt_setjmp` …).
pub const SETJMP: &str = "poly_crt_setjmp";
pub const LONGJMP: &str = "poly_crt_longjmp";
