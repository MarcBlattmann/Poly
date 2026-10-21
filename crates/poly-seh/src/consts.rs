//! Constants of x64 SEH, named as in the public headers they come from.
//!
//! - `UNW_FLAG_*`, `UWOP_*` and the register numbers: Learn, "x64 exception
//!   handling" (the `UNWIND_INFO` and `UNWIND_CODE` tables, "Operation info"
//!   register encoding).
//! - `EXCEPTION_*` record flags and `EXCEPTION_MAXIMUM_PARAMETERS`:
//!   `winnt.h` (Learn: "EXCEPTION_RECORD").
//! - `EXCEPTION_DISPOSITION` values: `excpt.h` (Learn: "x64 exception
//!   handling", "Language-specific handler").
//! - Exception codes: `ntstatus.h` (the `EXCEPTION_*` codes of Learn's
//!   `EXCEPTION_RECORD` page and `winbase.h` are these values).

// ----- UNWIND_INFO.Flags

/// No handler.
pub const UNW_FLAG_NHANDLER: u8 = 0x0;
/// The function has an exception handler, called while looking for
/// functions that need to examine exceptions.
pub const UNW_FLAG_EHANDLER: u8 = 0x1;
/// The function has a termination handler, called when unwinding.
pub const UNW_FLAG_UHANDLER: u8 = 0x2;
/// The unwind info is not the primary one for the procedure: a chained
/// `RUNTIME_FUNCTION` follows the unwind codes.
pub const UNW_FLAG_CHAININFO: u8 = 0x4;

// ----- UNWIND_CODE.UnwindOp

pub const UWOP_PUSH_NONVOL: u8 = 0;
pub const UWOP_ALLOC_LARGE: u8 = 1;
pub const UWOP_ALLOC_SMALL: u8 = 2;
pub const UWOP_SET_FPREG: u8 = 3;
pub const UWOP_SAVE_NONVOL: u8 = 4;
pub const UWOP_SAVE_NONVOL_FAR: u8 = 5;
pub const UWOP_SAVE_XMM128: u8 = 8;
pub const UWOP_SAVE_XMM128_FAR: u8 = 9;
pub const UWOP_PUSH_MACHFRAME: u8 = 10;

// ----- integer register numbers (UNWIND_CODE operation info, frame register)

pub const RAX: u8 = 0;
pub const RCX: u8 = 1;
pub const RDX: u8 = 2;
pub const RBX: u8 = 3;
pub const RSP: u8 = 4;
pub const RBP: u8 = 5;
pub const RSI: u8 = 6;
pub const RDI: u8 = 7;
pub const R8: u8 = 8;
pub const R9: u8 = 9;
pub const R10: u8 = 10;
pub const R11: u8 = 11;
pub const R12: u8 = 12;
pub const R13: u8 = 13;
pub const R14: u8 = 14;
pub const R15: u8 = 15;

// ----- EXCEPTION_RECORD.ExceptionFlags (`winnt.h`)

pub const EXCEPTION_NONCONTINUABLE: u32 = 0x1;
pub const EXCEPTION_UNWINDING: u32 = 0x2;
pub const EXCEPTION_EXIT_UNWIND: u32 = 0x4;
pub const EXCEPTION_STACK_INVALID: u32 = 0x8;
pub const EXCEPTION_NESTED_CALL: u32 = 0x10;
pub const EXCEPTION_TARGET_UNWIND: u32 = 0x20;
pub const EXCEPTION_COLLIDED_UNWIND: u32 = 0x40;
/// `EXCEPTION_UNWIND`: every flag that marks an unwind.
pub const EXCEPTION_UNWIND: u32 = 0x66;

/// `EXCEPTION_MAXIMUM_PARAMETERS` (`winnt.h`).
pub const EXCEPTION_MAXIMUM_PARAMETERS: usize = 15;

// ----- EXCEPTION_DISPOSITION (`excpt.h`)

pub const EXCEPTION_CONTINUE_EXECUTION_DISPOSITION: u32 = 0;
pub const EXCEPTION_CONTINUE_SEARCH_DISPOSITION: u32 = 1;
pub const EXCEPTION_NESTED_EXCEPTION_DISPOSITION: u32 = 2;
pub const EXCEPTION_COLLIDED_UNWIND_DISPOSITION: u32 = 3;

// ----- exception codes (`ntstatus.h`)

pub const STATUS_GUARD_PAGE_VIOLATION: u32 = 0x8000_0001;
pub const STATUS_DATATYPE_MISALIGNMENT: u32 = 0x8000_0002;
pub const STATUS_BREAKPOINT: u32 = 0x8000_0003;
pub const STATUS_SINGLE_STEP: u32 = 0x8000_0004;
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
pub const STATUS_IN_PAGE_ERROR: u32 = 0xC000_0006;
pub const STATUS_ILLEGAL_INSTRUCTION: u32 = 0xC000_001D;
pub const STATUS_NONCONTINUABLE_EXCEPTION: u32 = 0xC000_0025;
pub const STATUS_INVALID_DISPOSITION: u32 = 0xC000_0026;
pub const STATUS_UNWIND: u32 = 0xC000_0027;
pub const STATUS_BAD_STACK: u32 = 0xC000_0028;
pub const STATUS_INVALID_UNWIND_TARGET: u32 = 0xC000_0029;
pub const STATUS_ARRAY_BOUNDS_EXCEEDED: u32 = 0xC000_008C;
pub const STATUS_FLOAT_DENORMAL_OPERAND: u32 = 0xC000_008D;
pub const STATUS_FLOAT_DIVIDE_BY_ZERO: u32 = 0xC000_008E;
pub const STATUS_FLOAT_INEXACT_RESULT: u32 = 0xC000_008F;
pub const STATUS_FLOAT_INVALID_OPERATION: u32 = 0xC000_0090;
pub const STATUS_FLOAT_OVERFLOW: u32 = 0xC000_0091;
pub const STATUS_FLOAT_STACK_CHECK: u32 = 0xC000_0092;
pub const STATUS_FLOAT_UNDERFLOW: u32 = 0xC000_0093;
pub const STATUS_INTEGER_DIVIDE_BY_ZERO: u32 = 0xC000_0094;
pub const STATUS_INTEGER_OVERFLOW: u32 = 0xC000_0095;
pub const STATUS_PRIVILEGED_INSTRUCTION: u32 = 0xC000_0096;
pub const STATUS_STACK_OVERFLOW: u32 = 0xC000_00FD;
pub const STATUS_STACK_BUFFER_OVERRUN: u32 = 0xC000_0409;
