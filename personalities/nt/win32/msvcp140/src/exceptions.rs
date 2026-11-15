//! The throw helpers (`?_X*`, `?_Throw_Cpp_error`), the generic category's texts
//! (`?_Syserror_map`) and `std::uncaught_exception(s)`.
//!
//! A helper builds the exception object in its own frame and raises it with
//! `vcruntime140!_CxxThrowException` and the class's ThrowInfo, which lives in this image
//! (`vcruntime140` takes the image base from the ThrowInfo's address). The virtual tables,
//! RTTI records and throw descriptors are `poly_msvcp::classes`' assembly; this module has
//! the functions they name. The object layouts are in `poly_msvcp::classes`.

use poly_msvcp::classes::SYSTEM_ERROR_SIZE;
use poly_msvcp::errc;

/// `__std_exception_data` (`vcruntime_exception.h`).
#[repr(C)]
struct Data {
    what: *const u8,
    do_free: bool,
}

const NO_DATA: Data = Data { what: core::ptr::null(), do_free: false };

/// A `std::exception` (and every class here but `system_error`).
#[repr(C)]
struct Exception {
    vptr: *const u8,
    data: Data,
}

/// A `std::system_error`: a `runtime_error` and its `error_code`.
#[repr(C)]
struct SystemError {
    base: Exception,
    value: i32,
    category: *const Category,
}

const _: () = assert!(core::mem::size_of::<Exception>() == 24);
const _: () = assert!(core::mem::size_of::<SystemError>() == SYSTEM_ERROR_SIZE as usize);

#[link(name = "vcruntime140", kind = "raw-dylib")]
unsafe extern "C" {
    fn _CxxThrowException(object: *mut u8, throw_info: *const u8) -> !;
    fn __uncaught_exceptions() -> i32;
    fn __std_exception_copy(from: *const Data, to: *mut Data);
    fn __std_exception_destroy(data: *mut Data);
}

#[link(name = "ucrtbase", kind = "raw-dylib")]
unsafe extern "C" {
    fn free(p: *mut u8);
}

// The records of `poly_msvcp::classes::records_asm`.
core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/records.s")));

unsafe extern "C" {
    static poly_msvcp_vt_exception: u8;
    static poly_msvcp_vt_bad_alloc: u8;
    static poly_msvcp_vt_bad_function_call: u8;
    static poly_msvcp_vt_logic_error: u8;
    static poly_msvcp_vt_length_error: u8;
    static poly_msvcp_vt_out_of_range: u8;
    static poly_msvcp_vt_invalid_argument: u8;
    static poly_msvcp_vt_runtime_error: u8;
    static poly_msvcp_vt_overflow_error: u8;
    static poly_msvcp_vt_system_error: u8;
    static poly_msvcp_ti_bad_alloc: u8;
    static poly_msvcp_ti_bad_function_call: u8;
    static poly_msvcp_ti_length_error: u8;
    static poly_msvcp_ti_out_of_range: u8;
    static poly_msvcp_ti_invalid_argument: u8;
    static poly_msvcp_ti_runtime_error: u8;
    static poly_msvcp_ti_overflow_error: u8;
    static poly_msvcp_ti_system_error: u8;
}

// ----- the functions the records name ------------------------------------------------------

/// Virtual table slot 0, the deleting destructor: destroy, and free the memory when bit 0
/// of `flags` is set (an object the program made with `new`, whose `operator new` is the
/// C runtime's `malloc`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poly_msvcp_exception_dtor(this: *mut u8, flags: u32) -> *mut u8 {
    // SAFETY: an exception object of this DLL's classes.
    unsafe {
        poly_msvcp_exception_destroy(this);
        if flags & 1 != 0 {
            free(this);
        }
    }
    this
}

/// Virtual table slot 1, `what()`: the text, or a fixed one for an exception without text.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poly_msvcp_exception_what(this: *const u8) -> *const u8 {
    // SAFETY: an exception object of this DLL's classes.
    let what = unsafe { (*this.cast::<Exception>()).data.what };
    if what.is_null() { c"Unknown exception".as_ptr().cast() } else { what }
}

/// The ThrowInfo's destructor: give back the text the object owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poly_msvcp_exception_destroy(this: *mut u8) {
    // SAFETY: an exception object of this DLL's classes.
    unsafe { __std_exception_destroy(&raw mut (*this.cast::<Exception>()).data) }
}

/// The copy constructor of a class: its own virtual table, a copy of the text (owned if the
/// source owns it), and the bytes after the `std::exception` part (`system_error`'s code).
///
/// # Safety
/// `dst` has room for `size` bytes; `src` is an object of the class or one derived from it.
unsafe fn copy(dst: *mut u8, src: *const u8, vtable: *const u8, size: usize) -> *mut u8 {
    // SAFETY: the caller's objects.
    unsafe {
        let (d, s) = (dst.cast::<Exception>(), src.cast::<Exception>());
        (*d).vptr = vtable;
        (*d).data = NO_DATA;
        __std_exception_copy(&raw const (*s).data, &raw mut (*d).data);
        let head = core::mem::size_of::<Exception>();
        if size > head {
            core::ptr::copy_nonoverlapping(src.add(head), dst.add(head), size - head);
        }
    }
    dst
}

macro_rules! copies {
    ($($copy:ident $vtable:ident $size:expr;)*) => {$(
        /// A CatchableType's copy constructor (see `copy`).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $copy(dst: *mut u8, src: *const u8) -> *mut u8 {
            // SAFETY: the catch machinery's destination and the thrown object.
            unsafe { copy(dst, src, &raw const $vtable, $size) }
        }
    )*};
}

copies! {
    poly_msvcp_copy_exception poly_msvcp_vt_exception 24;
    poly_msvcp_copy_bad_alloc poly_msvcp_vt_bad_alloc 24;
    poly_msvcp_copy_bad_function_call poly_msvcp_vt_bad_function_call 24;
    poly_msvcp_copy_logic_error poly_msvcp_vt_logic_error 24;
    poly_msvcp_copy_length_error poly_msvcp_vt_length_error 24;
    poly_msvcp_copy_out_of_range poly_msvcp_vt_out_of_range 24;
    poly_msvcp_copy_invalid_argument poly_msvcp_vt_invalid_argument 24;
    poly_msvcp_copy_runtime_error poly_msvcp_vt_runtime_error 24;
    poly_msvcp_copy_overflow_error poly_msvcp_vt_overflow_error 24;
    poly_msvcp_copy_system_error poly_msvcp_vt_system_error SYSTEM_ERROR_SIZE as usize;
}

// ----- throwing ------------------------------------------------------------------------------

/// Throw an object of the class with that virtual table and ThrowInfo whose text is `what`:
/// copied (owned by the object) if `copied`, else a literal of this DLL.
///
/// # Safety
/// `what` is NUL-terminated or null; `vtable` and `throw_info` belong to one class of
/// `std::exception`'s size.
unsafe fn throw(vtable: *const u8, throw_info: *const u8, what: *const u8, copied: bool) -> ! {
    let mut object = Exception { vptr: vtable, data: NO_DATA };
    if copied {
        let text = Data { what, do_free: true };
        // SAFETY: the caller's text; the object's data is empty.
        unsafe { __std_exception_copy(&text, &mut object.data) };
    } else {
        object.data.what = what;
    }
    // SAFETY: the object lives in this frame, which stays while the exception is dispatched
    // and is unwound after a catch block ends.
    unsafe { _CxxThrowException((&raw mut object).cast(), throw_info) }
}

/// `[[noreturn]] void std::_Xbad_alloc()`: `std::bad_alloc`, "bad allocation".
#[unsafe(export_name = "?_Xbad_alloc@std@@YAXXZ")]
pub extern "C" fn x_bad_alloc() -> ! {
    // SAFETY: the class's records and a literal.
    unsafe {
        throw(
            &raw const poly_msvcp_vt_bad_alloc,
            &raw const poly_msvcp_ti_bad_alloc,
            c"bad allocation".as_ptr().cast(),
            false,
        )
    }
}

/// `[[noreturn]] void std::_Xbad_function_call()`: "bad function call".
#[unsafe(export_name = "?_Xbad_function_call@std@@YAXXZ")]
pub extern "C" fn x_bad_function_call() -> ! {
    // SAFETY: the class's records and a literal.
    unsafe {
        throw(
            &raw const poly_msvcp_vt_bad_function_call,
            &raw const poly_msvcp_ti_bad_function_call,
            c"bad function call".as_ptr().cast(),
            false,
        )
    }
}

macro_rules! text_throwers {
    ($($export:literal $fn:ident $vtable:ident $ti:ident;)*) => {$(
        #[doc = concat!("`[[noreturn]] void ", $export, "(const char*)`: throws with a copy of the text.")]
        #[unsafe(export_name = $export)]
        pub unsafe extern "C" fn $fn(what: *const u8) -> ! {
            // SAFETY: the class's records and the caller's text.
            unsafe { throw(&raw const $vtable, &raw const $ti, what, true) }
        }
    )*};
}

text_throwers! {
    "?_Xlength_error@std@@YAXPEBD@Z" x_length_error poly_msvcp_vt_length_error poly_msvcp_ti_length_error;
    "?_Xout_of_range@std@@YAXPEBD@Z" x_out_of_range poly_msvcp_vt_out_of_range poly_msvcp_ti_out_of_range;
    "?_Xinvalid_argument@std@@YAXPEBD@Z" x_invalid_argument poly_msvcp_vt_invalid_argument poly_msvcp_ti_invalid_argument;
    "?_Xruntime_error@std@@YAXPEBD@Z" x_runtime_error poly_msvcp_vt_runtime_error poly_msvcp_ti_runtime_error;
    "?_Xoverflow_error@std@@YAXPEBD@Z" x_overflow_error poly_msvcp_vt_overflow_error poly_msvcp_ti_overflow_error;
}

// ----- std::system_error -----------------------------------------------------------------------

/// The generic error category: a virtual table pointer and an identity word (this object's
/// address). Its layout and the rule by which a program's inline `generic_category()` compares
/// equal to it are not in a spec sheet yet; its virtual functions log and return 0.
#[repr(C)]
struct Category {
    vptr: *const [unsafe extern "C" fn() -> u64; 8],
    identity: *const Category,
}

// SAFETY: never written.
unsafe impl Sync for Category {}

struct Slots([unsafe extern "C" fn() -> u64; 8]);

// SAFETY: function pointers, never written.
unsafe impl Sync for Slots {}

unsafe extern "C" fn category_slot() -> u64 {
    poly_win32_rt::unimplemented_export(
        "stub: msvcp140!std::error_category virtual function (generic category)",
        false,
        0,
    )
}

static CATEGORY_VTABLE: Slots = Slots([category_slot; 8]);
static GENERIC_CATEGORY: Category = Category { vptr: &CATEGORY_VTABLE.0, identity: &raw const GENERIC_CATEGORY };

/// `[[noreturn]] void std::_Throw_Cpp_error(int code)`: a `std::system_error` with the
/// code's `std::errc` value in the generic category and `what()` "<text>: <text>".
#[unsafe(export_name = "?_Throw_Cpp_error@std@@YAXH@Z")]
pub extern "C" fn throw_cpp_error(code: i32) -> ! {
    let value = errc::cpp_error_value(code).unwrap_or_else(|| {
        // Not one of the sheet's codes: reported as an invalid argument.
        poly_win32_rt::unimplemented_export("msvcp140: _Throw_Cpp_error of an unknown code", false, 0);
        22
    });
    let what = errc::text(value).map_or(errc::UNKNOWN, |t| t.what);
    let mut object = SystemError {
        base: Exception {
            vptr: &raw const poly_msvcp_vt_system_error,
            data: Data { what: what.as_ptr(), do_free: false },
        },
        value,
        category: &GENERIC_CATEGORY,
    };
    // SAFETY: as in `throw`.
    unsafe { _CxxThrowException((&raw mut object).cast(), &raw const poly_msvcp_ti_system_error) }
}

/// `const char* std::_Syserror_map(int)`: the generic category's text of an `errno` value.
#[unsafe(export_name = "?_Syserror_map@std@@YAPEBDH@Z")]
pub extern "C" fn syserror_map(value: i32) -> *const u8 {
    errc::text(value).map_or(errc::UNKNOWN, |t| t.text).as_ptr()
}

// ----- uncaught exceptions ------------------------------------------------------------------

/// `int std::uncaught_exceptions() noexcept`: `vcruntime140`'s count for this thread.
#[unsafe(export_name = "?uncaught_exceptions@std@@YAHXZ")]
pub extern "C" fn uncaught_exceptions() -> i32 {
    // SAFETY: takes nothing.
    unsafe { __uncaught_exceptions() }
}

/// `bool std::uncaught_exception() noexcept`: whether that count is above 0.
#[unsafe(export_name = "?uncaught_exception@std@@YA_NXZ")]
pub extern "C" fn uncaught_exception() -> bool {
    uncaught_exceptions() > 0
}
