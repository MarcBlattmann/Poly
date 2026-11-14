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

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
    fn RtlSetLastWin32Error(error: u32);
}

/// `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`): "This function is not supported on this system."
pub const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
/// `E_NOTIMPL` (`winerror.h`).
pub const E_NOTIMPL: u64 = 0x8000_4001;
/// `STATUS_NOT_IMPLEMENTED` (`ntstatus.h`).
pub const STATUS_NOT_IMPLEMENTED: u64 = 0xC000_0002;

/// What an export made by [`fail_exports!`] does when it is called: log `text` (`stub:
/// dll!name`, the line `polystub.dll` logs for an import it stands in for, ADR 0155), set the
/// last error to `ERROR_CALL_NOT_IMPLEMENTED` if the function is one that reports failure
/// through it, and return `value`.
pub fn unimplemented_export(text: &str, last_error: bool, value: u64) -> u64 {
    // SAFETY: the service reads `text.len()` bytes; the second call takes no pointer.
    unsafe {
        PolyDebugPrint(text.as_ptr(), text.len());
        if last_error {
            RtlSetLastWin32Error(ERROR_CALL_NOT_IMPLEMENTED);
        }
    }
    value
}

/// Exports that exist and fail (ADR 0155, step 9). A program's delay-load helper finds a
/// function with `LoadLibrary` and `GetProcAddress` and treats a name the DLL does not export as
/// fatal (Chrome ends itself with a breakpoint), even where it would cope with the call failing.
/// `polystub.dll` cannot stand in there (the loader rule: `GetProcAddress` of a name nobody
/// exports fails), so the DLL itself exports the name: a function that logs `stub: dll!name` and
/// fails the way ADR 0155 fixes for a function the system does not provide.
///
/// The kinds: `win32` (0 / FALSE / NULL and last error 120), `win32_minus_one` (`(DWORD)-1` and
/// last error 120, for functions whose failure value that is), `hresult` (`E_NOTIMPL`), `status`
/// (`STATUS_NOT_IMPLEMENTED`), `value(v)` (the function returns its error directly: `v`). The
/// functions take no parameters: on x64 the caller owns the argument area, so that is safe for
/// every signature.
///
/// `fail_exports!("user32", win32, [CreatePopupMenu, DestroyMenu]);`
#[macro_export]
macro_rules! fail_exports {
    ($dll:literal, win32, [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make $dll, true, 0, [$($name),*]);
    };
    ($dll:literal, win32_minus_one, [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make $dll, true, 0xFFFF_FFFF, [$($name),*]);
    };
    ($dll:literal, hresult, [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make $dll, false, $crate::E_NOTIMPL, [$($name),*]);
    };
    ($dll:literal, status, [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make $dll, false, $crate::STATUS_NOT_IMPLEMENTED, [$($name),*]);
    };
    ($dll:literal, value($value:expr), [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make $dll, false, $value, [$($name),*]);
    };
    (@make $dll:literal, $last_error:expr, $value:expr, [$($name:ident),*]) => {
        $(
            #[doc = concat!("`", stringify!($name), "`: not implemented yet; the export exists so that a delay load of it binds, and fails (ADR 0155, step 9).")]
            #[unsafe(no_mangle)]
            pub extern "system" fn $name() -> u64 {
                $crate::unimplemented_export(concat!("stub: ", $dll, "!", stringify!($name)), $last_error, $value)
            }
        )*
    };
}
