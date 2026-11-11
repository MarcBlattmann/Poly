//! The bits of a C runtime every Poly Win32 module needs (ADR 0079), since
//! they link with `/NODEFAULTLIB`:
//!
//! - `__chkstk`, the MSVC x64 stack probe the compiler calls before a frame
//!   larger than a page (Learn: "/Gs (Control Stack Checking Calls)"; RAX =
//!   the frame size, every register but R10/R11 preserved, RSP unchanged).
//!   The kernel grows a thread's stack on each guard-page fault (ADR 0063),
//!   so touching every page from the caller's RSP down is enough.
//! - `_fltused`, which MSVC-style code references once floating point is
//!   used.
//! - The panic handler: a fail-fast (`__fastfail`, `int 0x29`, Learn:
//!   "__fastfail"), which ends the process without unwinding (ADR 0068).
//!
//! Hand-written from those public contracts; the same as `ntdll`'s.

#![no_std]

use core::arch::global_asm;

global_asm!(
    r#"
    .text
    .globl __chkstk
__chkstk:
    push rcx
    push rax
    mov  rcx, rax
    lea  rax, [rsp + 24]
2:
    cmp  rcx, 0x1000
    jb   3f
    sub  rax, 0x1000
    mov  byte ptr [rax], 0
    sub  rcx, 0x1000
    jmp  2b
3:
    pop  rax
    pop  rcx
    ret
"#
);

/// MSVC emits a reference to this when floating point is used.
#[unsafe(no_mangle)]
#[used]
static _fltused: i32 = 0;

/// `FAST_FAIL_FATAL_APP_EXIT` (`winnt.h`).
pub const FAST_FAIL_FATAL_APP_EXIT: u32 = 7;

/// End the process at once (`__fastfail(code)`).
pub fn fail_fast(code: u32) -> ! {
    // SAFETY: int 0x29 with the code in ECX is the documented fail-fast; it
    // does not return.
    unsafe { core::arch::asm!("int 0x29", in("ecx") code, options(noreturn)) }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail_fast(FAST_FAIL_FATAL_APP_EXIT)
}

/// Make the process heap the module's allocator: `process_heap_allocator!();` at the top of
/// a DLL that links a crate using `alloc` (ADR 0155, step 9: the small DLLs). The blocks are
/// `HeapAlloc`'s, 16-byte aligned; a stricter alignment is refused.
#[macro_export]
macro_rules! process_heap_allocator {
    () => {
        mod process_heap {
            #[link(name = "kernel32", kind = "raw-dylib")]
            unsafe extern "system" {
                pub fn GetProcessHeap() -> *mut u8;
                pub fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
                pub fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
            }

            pub struct ProcessHeap;

            // SAFETY: HeapAlloc returns 16-byte aligned blocks of the serialized process heap;
            // stricter alignments are refused.
            unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
                unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
                    if layout.align() > 16 {
                        return core::ptr::null_mut();
                    }
                    // SAFETY: the process heap.
                    unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
                }
                unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
                    // SAFETY: a block from `alloc`.
                    unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
                }
            }

            #[global_allocator]
            static ALLOC: ProcessHeap = ProcessHeap;
        }
    };
}
