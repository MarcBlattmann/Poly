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

// The i686 build's `__chkstk` (whose 32-bit contract differs) and 64-bit arithmetic helpers
// come from `poly-msvc32` (ADR 0186).
extern crate poly_msvc32;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
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
            #[cfg_attr(
                target_arch = "x86",
                link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")
            )]
            #[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
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

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn PolyDebugPrint(text: *const u8, length: usize) -> u32;
    fn RtlSetLastWin32Error(error: u32);
    fn NtTerminateProcess(process: isize, status: u32) -> u32;
}

/// The bytes a 32-bit `stdcall` function pops (`x86_signatures.txt`, ADR 0186 and 0222): the
/// table and its lookup are `poly-stubs`'s, the same file compiled here.
#[path = "../../../../../crates/poly-stubs/src/x86slots.rs"]
#[doc(hidden)]
pub mod x86slots;

/// The table of 32-bit signatures, for [`fail_exports!`]'s i686 stubs.
#[doc(hidden)]
pub const X86_SIGNATURES: &str = include_str!("../../../../../crates/poly-stubs/src/x86_signatures.txt");

/// What the 32-bit stub of a fail-only export whose arguments' size is unknown does (ADR 0222): it
/// cannot return (`stdcall`: the callee pops what the caller pushed, and nobody knows how much), so
/// it logs `stub: dll!name` and ends the process with `STATUS_NOT_IMPLEMENTED`, as the 32-bit
/// `polystub.dll` does (ADR 0186). Returning anyway left the stack wrong and the program jumped to
/// a stack address (Google's installer, survey stop 1).
pub fn unimplemented_export_fatal(text: &str) -> ! {
    // SAFETY: the service reads `text.len()` bytes; -1 is the current process.
    unsafe {
        PolyDebugPrint(text.as_ptr(), text.len());
        NtTerminateProcess(-1, STATUS_NOT_IMPLEMENTED as u32);
    }
    loop {
        core::hint::spin_loop();
    }
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
    unimplemented_export_error(text, if last_error { ERROR_CALL_NOT_IMPLEMENTED } else { 0 }, value)
}

/// [`unimplemented_export`] with the last error given (0: leave it alone): the functions of a
/// family that Learn documents with an error of its own (`APPMODEL_ERROR_NO_PACKAGE`,
/// `WSAEOPNOTSUPP`).
pub fn unimplemented_export_error(text: &str, last_error: u32, value: u64) -> u64 {
    // SAFETY: the service reads `text.len()` bytes; the second call takes no pointer.
    unsafe {
        PolyDebugPrint(text.as_ptr(), text.len());
        if last_error != 0 {
            RtlSetLastWin32Error(last_error);
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
/// (`STATUS_NOT_IMPLEMENTED`), `value(v)` (the function returns its error directly: `v`), `invalid_handle`
/// (`(HANDLE)-1` or `(INT_PTR)-1` and last error 120), `error(code, v)` (`v` with the last error `code`). The
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
    ($dll:literal, invalid_handle, [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make $dll, true, u64::MAX, [$($name),*]);
    };
    ($dll:literal, error($code:expr, $value:expr), [$($name:ident),* $(,)?]) => {
        $crate::fail_exports!(@make_error $dll, $code, $value, [$($name),*]);
    };
    (@make_error $dll:literal, $code:expr, $value:expr, [$($name:ident),*]) => {
        $(
            $crate::fail_exports!(@one $dll, $name,
                $crate::unimplemented_export_error(concat!("stub: ", $dll, "!", stringify!($name)), $code, $value));
        )*
    };
    (@make $dll:literal, $last_error:expr, $value:expr, [$($name:ident),*]) => {
        $(
            $crate::fail_exports!(@one $dll, $name,
                $crate::unimplemented_export(concat!("stub: ", $dll, "!", stringify!($name)), $last_error, $value));
        )*
    };
    // One export. On x64 the caller owns the argument area, so a function without parameters is
    // right for every signature. A 32-bit `stdcall` function pops its arguments itself (ADR 0186):
    // the i686 export is a naked stub that calls the body and returns with `ret` popping the bytes
    // `x86_signatures.txt` gives the function, or, for a function the table does not know, a body
    // that ends the process ([`unimplemented_export_fatal`]) instead of returning wrong (ADR 0222).
    (@one $dll:literal, $name:ident, $body:expr) => {
        #[cfg(not(target_arch = "x86"))]
        #[doc = concat!("`", stringify!($name), "`: not implemented yet; the export exists so that a program binds it, and it fails (ADR 0155).")]
        #[unsafe(no_mangle)]
        pub extern "system" fn $name() -> u64 {
            $body
        }

        #[cfg(target_arch = "x86")]
        #[allow(non_snake_case)]
        #[doc(hidden)]
        pub mod $name {
            /// The bytes the stub pops (-1: unknown, the body ends the process).
            pub const BYTES: i32 = $crate::x86slots::stack_bytes($crate::X86_SIGNATURES, $dll, stringify!($name));
            /// What the stub does before it returns.
            pub extern "C" fn body() -> u64 {
                if BYTES < 0 {
                    $crate::unimplemented_export_fatal(concat!(
                        "stub: ", $dll, "!", stringify!($name),
                        " (32-bit, the size of its arguments is unknown: the process ends)"
                    ));
                }
                $body
            }
        }

        #[cfg(target_arch = "x86")]
        #[doc = concat!("`", stringify!($name), "`: not implemented yet; the export exists so that a program binds it, and it fails (ADR 0155, 0222).")]
        #[unsafe(no_mangle)]
        #[unsafe(naked)]
        pub extern "system" fn $name() {
            core::arch::naked_asm!(
                "call {body}",
                "pop ecx",
                "add esp, {bytes}",
                "jmp ecx",
                body = sym $name::body,
                bytes = const if $name::BYTES < 0 { 0 } else { $name::BYTES },
            )
        }
    };
}
