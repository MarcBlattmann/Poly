//! libsystem_platform: the `_platform_*` string routines (`mem.rs`), the
//! simple locks of `<os/lock.h>` and `<libkern/OSSpinLockDeprecated.h>`,
//! `setjmp`/`longjmp`, the cache-control calls and `ffs`/`fls`.
//!
//! The locks spin (with `pause`) until the word is free; there is one
//! thread in v0, so a waiter never waits for long. `setjmp` and
//! `sigsetjmp` do not save the signal mask yet (no signals in v0); the
//! buffer layout is Poly's own (the SDK only gives its size, `_JBLEN`).

use core::sync::atomic::{AtomicU32, Ordering};

fn lock(word: &AtomicU32) {
    while word.compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

fn try_lock(word: &AtomicU32) -> bool {
    word.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok()
}

/// # Safety
/// `p` points to a lock word (4 bytes, aligned).
unsafe fn word<'a>(p: *mut u32) -> &'a AtomicU32 {
    // SAFETY: the caller's contract.
    unsafe { AtomicU32::from_ptr(p) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn os_unfair_lock_lock(l: *mut u32) {
    // SAFETY: `os_unfair_lock` is a 32-bit word (`<os/lock.h>`).
    lock(unsafe { word(l) });
}

#[unsafe(no_mangle)]
unsafe extern "C" fn os_unfair_lock_trylock(l: *mut u32) -> bool {
    // SAFETY: as above.
    try_lock(unsafe { word(l) })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn os_unfair_lock_unlock(l: *mut u32) {
    // SAFETY: as above.
    unsafe { word(l) }.store(0, Ordering::Release);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn OSSpinLockLock(l: *mut u32) {
    // SAFETY: `OSSpinLock` is a 32-bit int.
    lock(unsafe { word(l) });
}

#[unsafe(no_mangle)]
unsafe extern "C" fn OSSpinLockTry(l: *mut u32) -> bool {
    // SAFETY: as above.
    try_lock(unsafe { word(l) })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn OSSpinLockUnlock(l: *mut u32) {
    // SAFETY: as above.
    unsafe { word(l) }.store(0, Ordering::Release);
}

/// x86 keeps instruction and data caches coherent: nothing to do.
#[unsafe(no_mangle)]
extern "C" fn sys_icache_invalidate(_start: *mut u8, _len: usize) {}

#[unsafe(no_mangle)]
extern "C" fn sys_dcache_flush(_start: *mut u8, _len: usize) {}

#[unsafe(no_mangle)]
extern "C" fn ffs(v: i32) -> i32 {
    if v == 0 { 0 } else { v.trailing_zeros() as i32 + 1 }
}

#[unsafe(no_mangle)]
extern "C" fn ffsl(v: i64) -> i32 {
    if v == 0 { 0 } else { v.trailing_zeros() as i32 + 1 }
}

#[unsafe(no_mangle)]
extern "C" fn ffsll(v: i64) -> i32 {
    ffsl(v)
}

#[unsafe(no_mangle)]
extern "C" fn fls(v: i32) -> i32 {
    32 - (v as u32).leading_zeros() as i32
}

#[unsafe(no_mangle)]
extern "C" fn flsl(v: i64) -> i32 {
    64 - (v as u64).leading_zeros() as i32
}

#[unsafe(no_mangle)]
extern "C" fn flsll(v: i64) -> i32 {
    flsl(v)
}

// The jump buffer: rbx, rbp, r12-r15, the stack pointer after return,
// the return address, the x87 control word and MXCSR.
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl _setjmp",
    ".globl __setjmp",
    ".globl _sigsetjmp",
    ".p2align 4",
    "_setjmp:",
    "__setjmp:",
    "_sigsetjmp:",
    "    mov [rdi], rbx",
    "    mov [rdi + 8], rbp",
    "    mov [rdi + 16], r12",
    "    mov [rdi + 24], r13",
    "    mov [rdi + 32], r14",
    "    mov [rdi + 40], r15",
    "    lea rdx, [rsp + 8]",
    "    mov [rdi + 48], rdx",
    "    mov rdx, [rsp]",
    "    mov [rdi + 56], rdx",
    "    fnstcw [rdi + 64]",
    "    stmxcsr [rdi + 68]",
    "    xor eax, eax",
    "    ret",
    ".globl _longjmp",
    ".globl __longjmp",
    ".globl _siglongjmp",
    ".p2align 4",
    "_longjmp:",
    "__longjmp:",
    "_siglongjmp:",
    "    mov eax, esi",
    "    test eax, eax",
    "    jne 2f",
    "    mov eax, 1",
    "2:",
    "    mov rbx, [rdi]",
    "    mov rbp, [rdi + 8]",
    "    mov r12, [rdi + 16]",
    "    mov r13, [rdi + 24]",
    "    mov r14, [rdi + 32]",
    "    mov r15, [rdi + 40]",
    "    fldcw [rdi + 64]",
    "    ldmxcsr [rdi + 68]",
    "    mov rsp, [rdi + 48]",
    "    jmp qword ptr [rdi + 56]",
);

// arm64 (ADR 0257): x19 to x28, the frame pointer, the link register, the
// stack pointer and d8 to d15 (the registers AAPCS64 says a call keeps),
// 168 of the SDK's 192-byte `jmp_buf`.
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    ".globl _setjmp",
    ".globl __setjmp",
    ".globl _sigsetjmp",
    ".p2align 2",
    "_setjmp:",
    "__setjmp:",
    "_sigsetjmp:",
    "    stp x19, x20, [x0]",
    "    stp x21, x22, [x0, #16]",
    "    stp x23, x24, [x0, #32]",
    "    stp x25, x26, [x0, #48]",
    "    stp x27, x28, [x0, #64]",
    "    stp x29, x30, [x0, #80]",
    "    mov x9, sp",
    "    str x9, [x0, #96]",
    "    stp d8, d9, [x0, #104]",
    "    stp d10, d11, [x0, #120]",
    "    stp d12, d13, [x0, #136]",
    "    stp d14, d15, [x0, #152]",
    "    mov w0, #0",
    "    ret",
    ".globl _longjmp",
    ".globl __longjmp",
    ".globl _siglongjmp",
    ".p2align 2",
    "_longjmp:",
    "__longjmp:",
    "_siglongjmp:",
    "    ldp x19, x20, [x0]",
    "    ldp x21, x22, [x0, #16]",
    "    ldp x23, x24, [x0, #32]",
    "    ldp x25, x26, [x0, #48]",
    "    ldp x27, x28, [x0, #64]",
    "    ldp x29, x30, [x0, #80]",
    "    ldr x9, [x0, #96]",
    "    mov sp, x9",
    "    ldp d8, d9, [x0, #104]",
    "    ldp d10, d11, [x0, #120]",
    "    ldp d12, d13, [x0, #136]",
    "    ldp d14, d15, [x0, #152]",
    "    cmp w1, #0",
    "    csinc w0, w1, wzr, ne",
    "    ret",
);
