//! The constants of C++ exception handling on x64.
//!
//! - The exception code and the record's first parameter: the spec
//!   (`_CxxThrowException.yaml`; Raymond Chen, "Decoding the parameters of
//!   a thrown C++ exception (0xE06D7363)", The Old New Thing).
//! - FuncInfo magic numbers, EH flags, handler adjectives, ThrowInfo
//!   attributes and catchable type properties: the spec sheets'
//!   `data_layouts` (as the compiler writes the tables).
//! - `STATUS_UNWIND_CONSOLIDATE`: `ntstatus.h`.
//! - `EXCEPTION_SOFTWARE_ORIGINATE`: `winnt.h`.

/// The code of every C++ exception (`'msc' | 0xE0000000`).
pub const CXX_EXCEPTION: u32 = 0xE06D_7363;

/// FuncInfo magic numbers: the oldest layout, then with `ESTypeList`,
/// then with `EHFlags`. Also the record's first parameter (always the
/// oldest value).
pub const MAGIC_V1: u32 = 0x1993_0520;
pub const MAGIC_V2: u32 = 0x1993_0521;
pub const MAGIC_V3: u32 = 0x1993_0522;
/// The magic number is in the low 29 bits of its field.
pub const MAGIC_MASK: u32 = 0x1FFF_FFFF;

/// FuncInfo sizes by magic number.
pub const FUNC_INFO_V1_SIZE: u64 = 32;
pub const FUNC_INFO_V2_SIZE: u64 = 36;
pub const FUNC_INFO_V3_SIZE: u64 = 40;

/// `EHFlags` (0x19930522 only): synchronous exceptions only (`/EHs`).
pub const EH_SYNCHRONOUS: u32 = 0x1;
/// `EHFlags`: the function is `noexcept`.
pub const EH_NOEXCEPT: u32 = 0x4;

/// `HandlerType.Adjectives`.
pub const ADJ_CONST: u32 = 0x1;
pub const ADJ_VOLATILE: u32 = 0x2;
pub const ADJ_UNALIGNED: u32 = 0x4;
/// The catch object is a reference (a pointer to the exception object).
pub const ADJ_REFERENCE: u32 = 0x8;
/// On `catch(...)`: C++ exceptions only.
pub const ADJ_CXX_ONLY: u32 = 0x40;

/// `ThrowInfo.attributes`: the thrown pointer's pointee is const,
/// volatile, unaligned.
pub const TI_CONST: u32 = 0x1;
pub const TI_VOLATILE: u32 = 0x2;
pub const TI_UNALIGNED: u32 = 0x4;
/// The attributes a catch clause must repeat.
pub const TI_QUALIFIERS: u32 = TI_CONST | TI_VOLATILE | TI_UNALIGNED;

/// `CatchableType.properties`: a scalar or pointer.
pub const CT_SIMPLE: u32 = 0x1;
/// The class has virtual bases: its copy function takes a third argument.
pub const CT_VIRTUAL_BASES: u32 = 0x4;
/// `std::bad_alloc`.
pub const CT_BAD_ALLOC: u32 = 0x10;

/// `STATUS_UNWIND_CONSOLIDATE` (`ntstatus.h`): the code of the record
/// msvcrt unwinds to a catch block with (ADR 0099).
pub const STATUS_UNWIND_CONSOLIDATE: u32 = 0x8000_0029;

/// `EXCEPTION_SOFTWARE_ORIGINATE` (`winnt.h`): set by `RaiseException`.
pub const EXCEPTION_SOFTWARE_ORIGINATE: u32 = 0x80;

/// The longest decorated type name compared.
pub const MAX_NAME: u64 = 4096;
