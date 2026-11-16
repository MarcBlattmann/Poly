//! Our `vcruntime140.dll` (ADR 0199): the Visual C++ runtime support library that every
//! program built with Visual C++ 2015 or later imports, so that programs which expect the
//! Visual C++ redistributable to be installed run without it. A copy the program brings
//! (in its own directory) is found first and wins, as on Windows.
//!
//! - **C++ exceptions** are `poly-win32-cxxrt`'s, the code `msvcrt.dll` has had since
//!   ADR 0099 (the `msvcrt.dll` spec sheets `__CxxFrameHandler3`, `__CxxFrameHandler`,
//!   `_CxxThrowException`): `_CxxThrowException`, `__CxxFrameHandler3` and its older names,
//!   the current exception and the count of uncaught ones, `__DestructExceptionObject`,
//!   `_is_exception_typeof`, `__AdjustPointer`, `__CxxDetectRethrow`. `terminate()` is the
//!   Universal C runtime's (`ucrtbase`), whose `set_terminate` the program uses.
//! - **`type_info` and `std::exception`** helpers (`poly_cxxeh::typeinfo`): compare, hash,
//!   the copy and destruction of an exception's text.
//! - **Handlers** Learn documents: `set_unexpected` / `unexpected`, `_set_se_translator`,
//!   `__report_gsfailure` (a fail-fast with `FAST_FAIL_STACK_COOKIE_CHECK_FAILURE`).
//! - **Forwarders** (`build.rs`) to the DLLs that already have the function: the C library's
//!   memory and string functions, `setjmp`/`longjmp`, `_purecall` and `__std_terminate` to
//!   `ucrtbase`, `__C_specific_handler` to `ntdll`, the `__vcrt_` wrappers to `kernel32`.
//! - **`__CxxFrameHandler4`** (the spec `vcruntime140_1.dll/__CxxFrameHandler4.yaml`):
//!   `poly-win32-cxxrt`'s FH4 handler; `vcruntime140_1.dll` forwards its export here so that
//!   the per-thread exception state stays in one DLL.
//! - **RTTI** (`rtti`): `__RTDynamicCast`, `__RTtypeid`, `__RTCastToVoid`, the readable names
//!   of `type_info::name()` (`__std_type_info_name`, `poly_cxxeh::undname`) and
//!   `__std_type_info_hash`; `std::bad_cast` and `std::bad_typeid` are thrown with records
//!   this DLL writes into its own image.
//! - **Not yet** (fail-only, logged as `stub: vcruntime140!name`): the full name undecorator
//!   (`__unDName`, `__unDNameEx`), and the exception internals no compiler output calls
//!   directly (`__BuildCatchObject`, `__TypeMatch`, `_CreateFrameInfo`, ...): they wait for
//!   spec sheets (ADR 0199).
//!
//! The 32-bit (WOW64) `vcruntime140.dll` is a later step: x86 C++ exceptions are frame-based
//! (`fs:[0]`) and need the i686 `ucrtbase` first (ADR 0186, ADR 0199).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

poly_win32_rt::process_heap_allocator!();

use core::sync::atomic::{AtomicUsize, Ordering};
use poly_cxxeh::typeinfo::{self, TextCopy};
use poly_win32_cxxrt as cxxrt;

mod rtti;

#[cfg_attr(target_arch = "x86", link(name = "ucrtbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ucrtbase", kind = "raw-dylib"))]
unsafe extern "C" {
    #[link_name = "terminate"]
    fn ucrt_terminate() -> !;
    fn malloc(size: usize) -> *mut u8;
    fn free(p: *mut u8);
    fn strlen(s: *const u8) -> usize;
}

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RtlUnwind(frame: *mut u8, target: *mut u8, record: *mut u8, value: *mut u8);
}

/// `DLL_PROCESS_ATTACH` (`winnt.h`).
const DLL_PROCESS_ATTACH: u32 = 1;
/// `FAST_FAIL_STACK_COOKIE_CHECK_FAILURE` (`winnt.h`).
const FAST_FAIL_STACK_COOKIE_CHECK_FAILURE: u32 = 2;

fn terminate() -> ! {
    // SAFETY: the Universal C runtime's `terminate()`, which takes nothing.
    unsafe { ucrt_terminate() }
}

/// Marks this file as Poly's own copy (ADR 0199): at boot Poly refreshes a
/// `System32` copy that holds these bytes and keeps any other (Microsoft's
/// runtime a user installed). The same text as the kernel's
/// `OWN_COPY_MARKER` (`personalities/nt/src/layout/create.rs`); the kernel's
/// `build.rs` checks it is in the file.
static OWN_COPY_MARKER: &[u8] = b"Poly's own copy of a Visual C++ runtime DLL (ADR 0199)";

/// The entry point: name the Universal C runtime's `terminate()` to the C++ exception code.
#[unsafe(no_mangle)]
pub extern "system" fn vcruntime140_main(_module: *mut u8, reason: u32, _reserved: *mut u8) -> i32 {
    core::hint::black_box(OWN_COPY_MARKER.as_ptr());
    if reason == DLL_PROCESS_ATTACH {
        cxxrt::set_terminate_hook(terminate);
        rtti::attach(_module as u64);
    }
    1
}

// ----- C++ exceptions ------------------------------------------------------------------------

/// `__CxxFrameHandler3`: the language-specific handler of functions with C++ try blocks or
/// objects to destroy (FuncInfo magic `0x19930520..0x19930522`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxFrameHandler3(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's arguments.
    unsafe { cxxrt::frame_handler(record, frame, context, dc) }
}

/// `__CxxFrameHandler4`: the handler of Visual C++'s compressed ("FH4") tables. Windows has it
/// in `vcruntime140_1.dll`; Poly's `vcruntime140_1.dll` forwards its export here, so that one
/// copy of the per-thread state (catch blocks in progress, the current exception, the count of
/// uncaught exceptions) serves both handlers and `_CxxThrowException` (ADR 0199).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxFrameHandler4(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's arguments.
    unsafe { cxxrt::frame_handler4(record, frame, context, dc) }
}

/// `__CxxFrameHandler2`: an older name of the same handler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxFrameHandler2(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's arguments.
    unsafe { cxxrt::frame_handler(record, frame, context, dc) }
}

/// `__CxxFrameHandler`: the oldest name of the same handler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxFrameHandler(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's arguments.
    unsafe { cxxrt::frame_handler(record, frame, context, dc) }
}

/// `void** __current_exception()`: where this thread keeps the `EXCEPTION_RECORD*` of the
/// exception its innermost catch block handles (null outside one).
#[unsafe(no_mangle)]
pub extern "C" fn __current_exception() -> *mut *mut u8 {
    cxxrt::current_exception_slot()
}

/// `void** __current_exception_context()`: that exception's `CONTEXT*`.
#[unsafe(no_mangle)]
pub extern "C" fn __current_exception_context() -> *mut *mut u8 {
    cxxrt::current_context_slot()
}

/// `int* __processing_throw()`: this thread's count of exceptions thrown and not yet caught.
#[unsafe(no_mangle)]
pub extern "C" fn __processing_throw() -> *mut i32 {
    cxxrt::uncaught_slot()
}

/// `int __uncaught_exceptions()` (`std::uncaught_exceptions`, C++17 [uncaught.exceptions]).
#[unsafe(no_mangle)]
pub extern "C" fn __uncaught_exceptions() -> i32 {
    cxxrt::uncaught_exceptions()
}

/// `bool __uncaught_exception()` (`std::uncaught_exception`): at least one.
#[unsafe(no_mangle)]
pub extern "C" fn __uncaught_exception() -> bool {
    cxxrt::uncaught_exceptions() > 0
}

/// `void __DestructExceptionObject(EXCEPTION_RECORD*)`: destroy a C++ exception's object.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __DestructExceptionObject(record: *mut u8) {
    // SAFETY: the caller's record (or null).
    unsafe { cxxrt::destruct_exception_object(record) }
}

/// `int __CxxDetectRethrow(EXCEPTION_POINTERS*)`: whether the exception is a C++ `throw;`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __CxxDetectRethrow(pointers: *const *const u8) -> i32 {
    if pointers.is_null() {
        return 0;
    }
    // SAFETY: `EXCEPTION_POINTERS` starts with its `ExceptionRecord` (`winnt.h`).
    let record = unsafe { pointers.read_unaligned() };
    // SAFETY: the record it points at (or null).
    i32::from(unsafe { cxxrt::is_rethrow(record) })
}

/// `int _is_exception_typeof(const type_info&, EXCEPTION_POINTERS*)`: whether the C++
/// exception can be caught as that type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _is_exception_typeof(type_info: *const u8, pointers: *const *const u8) -> i32 {
    if pointers.is_null() {
        return 0;
    }
    // SAFETY: as in `__CxxDetectRethrow`.
    let record = unsafe { pointers.read_unaligned() };
    // SAFETY: the caller's record and type descriptor.
    i32::from(unsafe { cxxrt::is_exception_typeof(type_info as u64, record) })
}

/// `void* __AdjustPointer(void*, const PMD&)`: `PMD` is three `int`s, `mdisp`, `pdisp`,
/// `vdisp`, as in a catchable type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __AdjustPointer(this: *mut u8, pmd: *const i32) -> *mut u8 {
    if pmd.is_null() || this.is_null() {
        return this;
    }
    // SAFETY: the caller's three displacements.
    let (m, p, v) = unsafe { (pmd.read_unaligned(), pmd.add(1).read_unaligned(), pmd.add(2).read_unaligned()) };
    cxxrt::adjust_pointer(this as u64, m, p, v) as *mut u8
}

// ----- type_info -------------------------------------------------------------------------------

/// The decorated name of a `__std_type_info_data` (8 bytes in) without its NUL.
///
/// # Safety
/// `data` points at a `__std_type_info_data`.
unsafe fn decorated(data: *const u8) -> &'static [u8] {
    // SAFETY: a NUL-terminated name the compiler wrote.
    unsafe {
        let p = data.add(8);
        core::slice::from_raw_parts(p, strlen(p))
    }
}

/// `int __std_type_info_compare(const __std_type_info_data*, const __std_type_info_data*)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_type_info_compare(a: *const u8, b: *const u8) -> i32 {
    if a == b {
        return 0;
    }
    // SAFETY: two type descriptors' data.
    unsafe { typeinfo::compare_names(decorated(a), decorated(b)) }
}

// ----- std::exception ------------------------------------------------------------------------

/// `void __std_exception_copy(const __std_exception_data* from, __std_exception_data* to)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_exception_copy(from: *const u8, to: *mut u8) {
    // SAFETY: two `__std_exception_data` (`_What` at 0, `_DoFree` at 8).
    unsafe {
        let what = from.cast::<*const u8>().read_unaligned();
        let do_free = from.add(8).read() != 0;
        let len = if what.is_null() { 0 } else { strlen(what) };
        let (new, owned) = match typeinfo::copy_text(what.is_null(), do_free, len) {
            TextCopy::Share => (what, false),
            TextCopy::Duplicate { len } => {
                let p = malloc(len + 1);
                if p.is_null() {
                    (core::ptr::null(), false)
                } else {
                    core::ptr::copy_nonoverlapping(what, p, len + 1);
                    (p.cast_const(), true)
                }
            }
        };
        to.cast::<*const u8>().write_unaligned(new);
        to.add(8).write(u8::from(owned));
    }
}

/// `void __std_exception_destroy(__std_exception_data*)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __std_exception_destroy(data: *mut u8) {
    // SAFETY: a `__std_exception_data`.
    unsafe {
        if data.add(8).read() != 0 {
            free(data.cast::<*mut u8>().read_unaligned());
        }
        data.cast::<*const u8>().write_unaligned(core::ptr::null());
        data.add(8).write(0);
    }
}

// ----- handlers ----------------------------------------------------------------------------------

static UNEXPECTED: AtomicUsize = AtomicUsize::new(0);
static SE_TRANSLATOR: AtomicUsize = AtomicUsize::new(0);
static WINRT_OOM: AtomicUsize = AtomicUsize::new(0);

/// `unexpected_handler set_unexpected(unexpected_handler)` (Learn: `set_unexpected`): the
/// previous handler.
#[unsafe(no_mangle)]
pub extern "C" fn set_unexpected(h: usize) -> usize {
    UNEXPECTED.swap(h, Ordering::AcqRel)
}

/// `unexpected_handler _get_unexpected()`.
#[unsafe(no_mangle)]
pub extern "C" fn _get_unexpected() -> usize {
    UNEXPECTED.load(Ordering::Acquire)
}

/// `void unexpected()` (Learn: calls the handler, which by default is `terminate`; a handler
/// must not return, so `terminate` follows if it does).
#[unsafe(no_mangle)]
pub extern "C" fn unexpected() -> ! {
    let h = UNEXPECTED.load(Ordering::Acquire);
    if h != 0 {
        // SAFETY: a `void (*)()` the program installed.
        let f: unsafe extern "C" fn() = unsafe { core::mem::transmute(h) };
        // SAFETY: as above.
        unsafe { f() };
    }
    terminate()
}

/// `_se_translator_function _set_se_translator(_se_translator_function)` (Learn): kept and
/// returned; the frame handler does not call it yet (ADR 0199).
#[unsafe(no_mangle)]
pub extern "C" fn _set_se_translator(h: usize) -> usize {
    SE_TRANSLATOR.swap(h, Ordering::AcqRel)
}

/// `void _SetWinRTOutOfMemoryExceptionCallback(PGETWINRT_OOM_EXCEPTION)`: kept; Poly raises
/// no WinRT exceptions.
#[unsafe(no_mangle)]
pub extern "C" fn _SetWinRTOutOfMemoryExceptionCallback(f: usize) {
    WINRT_OOM.store(f, Ordering::Release);
}

/// `void* __GetPlatformExceptionInfo(int*)`: no WinRT exception information.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __GetPlatformExceptionInfo(is_bad_alloc: *mut i32) -> *mut u8 {
    if !is_bad_alloc.is_null() {
        // SAFETY: the caller's int.
        unsafe { is_bad_alloc.write_unaligned(0) };
    }
    core::ptr::null_mut()
}

/// `void __report_gsfailure(uintptr_t cookie)`: a `/GS` security cookie check failed; the
/// process ends at once (Learn: "/GS (Buffer Security Check)", `__fastfail`).
#[unsafe(no_mangle)]
pub extern "C" fn __report_gsfailure(_cookie: usize) -> ! {
    poly_win32_rt::fail_fast(FAST_FAIL_STACK_COOKIE_CHECK_FAILURE)
}

/// `void __telemetry_main_invoke_trigger(HINSTANCE)`: nothing to record.
#[unsafe(no_mangle)]
pub extern "C" fn __telemetry_main_invoke_trigger(_module: *mut u8) {}

/// `void __telemetry_main_return_trigger(HINSTANCE)`: nothing to record.
#[unsafe(no_mangle)]
pub extern "C" fn __telemetry_main_return_trigger(_module: *mut u8) {}

/// `void _local_unwind(void* frame, void* target)`: run the termination handlers of the
/// frames up to `frame` (`RtlUnwind`, Learn), as a `goto` or `return` out of a `__try` does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _local_unwind(frame: *mut u8, target: *mut u8) {
    // SAFETY: the compiler's frame and target in this function.
    unsafe { RtlUnwind(frame, target, core::ptr::null_mut(), core::ptr::null_mut()) }
}

// The debugger's non-local-goto hooks (`_NLG_Dispatch2`, `_NLG_Return`, ...): places a
// debugger sets breakpoints on; they do nothing.
core::arch::global_asm!(
    r#"
    .text
    .globl poly_vcrt_nlg
    .p2align 4
poly_vcrt_nlg:
    ret
"#
);

// ----- not yet (ADR 0199): fail-only ------------------------------------------------------------

poly_win32_rt::fail_exports!(
    "vcruntime140",
    value(0),
    [
        __unDName,
        __unDNameEx,
        __BuildCatchObject,
        __BuildCatchObjectHelper,
        __TypeMatch,
        __FrameUnwindFilter,
        __CxxExceptionFilter,
        __CxxQueryExceptionSize,
        __CxxRegisterExceptionObject,
        __CxxUnregisterExceptionObject,
        _CreateFrameInfo,
        _FindAndUnlinkFrame,
        _IsExceptionObjectToBeDestroyed,
    ]
);
