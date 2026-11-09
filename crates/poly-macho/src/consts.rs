//! Numeric constants of the Mach-O format.
//!
//! Every group names the public SDK header it comes from. The values are the
//! ones the headers define; the comments say what they mean in one line.

// ---------------------------------------------------------------------------
// <mach-o/fat.h>
// ---------------------------------------------------------------------------

/// Magic of a universal file with 32-bit slice records (`<mach-o/fat.h>`
/// `FAT_MAGIC`). Fat headers are big-endian, whatever the slices are.
pub const FAT_MAGIC: u32 = 0xcafe_babe;
/// Magic of a universal file with 64-bit slice records (`<mach-o/fat.h>`
/// `FAT_MAGIC_64`).
pub const FAT_MAGIC_64: u32 = 0xcafe_babf;
/// Size of `struct fat_header`.
pub const FAT_HEADER_SIZE: usize = 8;
/// Size of `struct fat_arch`.
pub const FAT_ARCH_SIZE: usize = 20;
/// Size of `struct fat_arch_64`.
pub const FAT_ARCH_64_SIZE: usize = 32;

// ---------------------------------------------------------------------------
// <mach-o/loader.h>: header
// ---------------------------------------------------------------------------

/// First word of a 32-bit little-endian Mach-O file (`MH_MAGIC`).
pub const MH_MAGIC: u32 = 0xfeed_face;
/// `MH_MAGIC` as read from a big-endian 32-bit file (`MH_CIGAM`).
pub const MH_CIGAM: u32 = 0xcefa_edfe;
/// First word of a 64-bit little-endian Mach-O file (`MH_MAGIC_64`).
pub const MH_MAGIC_64: u32 = 0xfeed_facf;
/// `MH_MAGIC_64` as read from a big-endian 64-bit file (`MH_CIGAM_64`).
pub const MH_CIGAM_64: u32 = 0xcffa_edfe;
/// Size of `struct mach_header` (32-bit).
pub const MACH_HEADER_SIZE: usize = 28;
/// Size of `struct mach_header_64`.
pub const MACH_HEADER_64_SIZE: usize = 32;

/// Relocatable object file (`MH_OBJECT`).
pub const MH_OBJECT: u32 = 0x1;
/// Demand paged executable (`MH_EXECUTE`).
pub const MH_EXECUTE: u32 = 0x2;
/// Fixed VM shared library (`MH_FVMLIB`).
pub const MH_FVMLIB: u32 = 0x3;
/// Core file (`MH_CORE`).
pub const MH_CORE: u32 = 0x4;
/// Preloaded executable (`MH_PRELOAD`).
pub const MH_PRELOAD: u32 = 0x5;
/// Dynamically bound shared library (`MH_DYLIB`).
pub const MH_DYLIB: u32 = 0x6;
/// Dynamic link editor (`MH_DYLINKER`).
pub const MH_DYLINKER: u32 = 0x7;
/// Dynamically bound bundle (`MH_BUNDLE`).
pub const MH_BUNDLE: u32 = 0x8;
/// Shared library stub for static linking only (`MH_DYLIB_STUB`).
pub const MH_DYLIB_STUB: u32 = 0x9;
/// Companion file with only debug sections (`MH_DSYM`).
pub const MH_DSYM: u32 = 0xa;
/// x86-64 kernel extension (`MH_KEXT_BUNDLE`).
pub const MH_KEXT_BUNDLE: u32 = 0xb;
/// A file composed of other Mach-O files (`MH_FILESET`).
pub const MH_FILESET: u32 = 0xc;

/// No undefined references (`MH_NOUNDEFS`).
pub const MH_NOUNDEFS: u32 = 0x1;
/// Input for the dynamic linker (`MH_DYLDLINK`).
pub const MH_DYLDLINK: u32 = 0x4;
/// Undefined references bound at load time (`MH_BINDATLOAD`).
pub const MH_BINDATLOAD: u32 = 0x8;
/// Two-level namespace bindings (`MH_TWOLEVEL`).
pub const MH_TWOLEVEL: u32 = 0x80;
/// Forces flat namespace bindings (`MH_FORCE_FLAT`).
pub const MH_FORCE_FLAT: u32 = 0x100;
/// The final image contains external weak symbols (`MH_WEAK_DEFINES`).
pub const MH_WEAK_DEFINES: u32 = 0x8000;
/// The final image uses weak symbols (`MH_BINDS_TO_WEAK`).
pub const MH_BINDS_TO_WEAK: u32 = 0x1_0000;
/// All stacks may be executable (`MH_ALLOW_STACK_EXECUTION`).
pub const MH_ALLOW_STACK_EXECUTION: u32 = 0x2_0000;
/// The dylib has no re-exported dependencies to examine
/// (`MH_NO_REEXPORTED_DYLIBS`).
pub const MH_NO_REEXPORTED_DYLIBS: u32 = 0x10_0000;
/// Loaded at a random address (`MH_PIE`).
pub const MH_PIE: u32 = 0x20_0000;
/// Dead-strippable dylib (`MH_DEAD_STRIPPABLE_DYLIB`).
pub const MH_DEAD_STRIPPABLE_DYLIB: u32 = 0x40_0000;
/// Contains thread-local variable descriptors (`MH_HAS_TLV_DESCRIPTORS`).
pub const MH_HAS_TLV_DESCRIPTORS: u32 = 0x80_0000;
/// Run with a non-executable heap (`MH_NO_HEAP_EXECUTION`).
pub const MH_NO_HEAP_EXECUTION: u32 = 0x100_0000;
/// Linked for use in an application extension (`MH_APP_EXTENSION_SAFE`).
pub const MH_APP_EXTENSION_SAFE: u32 = 0x200_0000;
/// The main executable has no `__PAGEZERO` segment (`MH_IMPLICIT_PAGEZERO`).
pub const MH_IMPLICIT_PAGEZERO: u32 = 0x1000_0000;
/// The dylib is part of the dyld shared cache (`MH_DYLIB_IN_CACHE`).
pub const MH_DYLIB_IN_CACHE: u32 = 0x8000_0000;

// ---------------------------------------------------------------------------
// <mach/machine.h>
// ---------------------------------------------------------------------------

/// Mask of the architecture bits of a CPU type (`CPU_ARCH_MASK`).
pub const CPU_ARCH_MASK: i32 = 0xff00_0000_u32 as i32;
/// 64-bit ABI (`CPU_ARCH_ABI64`).
pub const CPU_ARCH_ABI64: i32 = 0x0100_0000;
/// 64-bit hardware with 32-bit types (`CPU_ARCH_ABI64_32`).
pub const CPU_ARCH_ABI64_32: i32 = 0x0200_0000;
/// 32-bit x86 (`CPU_TYPE_X86`, also `CPU_TYPE_I386`).
pub const CPU_TYPE_X86: i32 = 7;
/// x86-64 (`CPU_TYPE_X86_64`).
pub const CPU_TYPE_X86_64: i32 = CPU_TYPE_X86 | CPU_ARCH_ABI64;
/// 32-bit ARM (`CPU_TYPE_ARM`).
pub const CPU_TYPE_ARM: i32 = 12;
/// 64-bit ARM (`CPU_TYPE_ARM64`).
pub const CPU_TYPE_ARM64: i32 = CPU_TYPE_ARM | CPU_ARCH_ABI64;
/// 64-bit ARM with 32-bit pointers (`CPU_TYPE_ARM64_32`).
pub const CPU_TYPE_ARM64_32: i32 = CPU_TYPE_ARM | CPU_ARCH_ABI64_32;
/// Mask of the capability bits of a CPU subtype (`CPU_SUBTYPE_MASK`).
pub const CPU_SUBTYPE_MASK: i32 = 0xff00_0000_u32 as i32;
/// 64-bit libraries (`CPU_SUBTYPE_LIB64`), set on x86-64 slices by `lipo`.
pub const CPU_SUBTYPE_LIB64: i32 = 0x8000_0000_u32 as i32;
/// Pointer authentication with a versioned ABI (`CPU_SUBTYPE_PTRAUTH_ABI`).
pub const CPU_SUBTYPE_PTRAUTH_ABI: i32 = 0x8000_0000_u32 as i32;
/// Mask of the pointer authentication ABI version of an arm64e subtype
/// (`CPU_SUBTYPE_ARM64_PTR_AUTH_MASK`).
pub const CPU_SUBTYPE_ARM64_PTR_AUTH_MASK: i32 = 0x0f00_0000;
/// Any x86-64 (`CPU_SUBTYPE_X86_64_ALL`).
pub const CPU_SUBTYPE_X86_64_ALL: i32 = 3;
/// x86-64 Haswell feature subset (`CPU_SUBTYPE_X86_64_H`).
pub const CPU_SUBTYPE_X86_64_H: i32 = 8;
/// Any arm64 (`CPU_SUBTYPE_ARM64_ALL`).
pub const CPU_SUBTYPE_ARM64_ALL: i32 = 0;
/// arm64 v8 (`CPU_SUBTYPE_ARM64_V8`).
pub const CPU_SUBTYPE_ARM64_V8: i32 = 1;
/// arm64e, with pointer authentication (`CPU_SUBTYPE_ARM64E`).
pub const CPU_SUBTYPE_ARM64E: i32 = 2;
/// arm64 with the X1 feature set (`CPU_SUBTYPE_ARM64_X1`).
pub const CPU_SUBTYPE_ARM64_X1: i32 = 3;
/// arm64e with the X1 feature set (`CPU_SUBTYPE_ARM64E_X1`).
pub const CPU_SUBTYPE_ARM64E_X1: i32 = 12;

// ---------------------------------------------------------------------------
// <mach/vm_prot.h>
// ---------------------------------------------------------------------------

/// Read permission (`VM_PROT_READ`).
pub const VM_PROT_READ: u32 = 0x1;
/// Write permission (`VM_PROT_WRITE`).
pub const VM_PROT_WRITE: u32 = 0x2;
/// Execute permission (`VM_PROT_EXECUTE`).
pub const VM_PROT_EXECUTE: u32 = 0x4;

// ---------------------------------------------------------------------------
// <mach-o/loader.h>: load commands
// ---------------------------------------------------------------------------

/// Set in the command of load commands dyld must understand (`LC_REQ_DYLD`).
pub const LC_REQ_DYLD: u32 = 0x8000_0000;
/// 32-bit segment (`LC_SEGMENT`).
pub const LC_SEGMENT: u32 = 0x1;
/// Symbol table (`LC_SYMTAB`).
pub const LC_SYMTAB: u32 = 0x2;
/// Thread state (`LC_THREAD`).
pub const LC_THREAD: u32 = 0x4;
/// Thread state of the main thread, with a stack (`LC_UNIXTHREAD`).
pub const LC_UNIXTHREAD: u32 = 0x5;
/// Dynamic symbol table (`LC_DYSYMTAB`).
pub const LC_DYSYMTAB: u32 = 0xb;
/// Load a dynamic library (`LC_LOAD_DYLIB`).
pub const LC_LOAD_DYLIB: u32 = 0xc;
/// This image's own library identification (`LC_ID_DYLIB`).
pub const LC_ID_DYLIB: u32 = 0xd;
/// The dynamic linker to load (`LC_LOAD_DYLINKER`).
pub const LC_LOAD_DYLINKER: u32 = 0xe;
/// Identification of a dynamic linker (`LC_ID_DYLINKER`).
pub const LC_ID_DYLINKER: u32 = 0xf;
/// Load a library that may be missing (`LC_LOAD_WEAK_DYLIB`).
pub const LC_LOAD_WEAK_DYLIB: u32 = 0x18 | LC_REQ_DYLD;
/// 64-bit segment (`LC_SEGMENT_64`).
pub const LC_SEGMENT_64: u32 = 0x19;
/// The image's UUID (`LC_UUID`).
pub const LC_UUID: u32 = 0x1b;
/// A run path entry (`LC_RPATH`).
pub const LC_RPATH: u32 = 0x1c | LC_REQ_DYLD;
/// Code signature (`LC_CODE_SIGNATURE`).
pub const LC_CODE_SIGNATURE: u32 = 0x1d;
/// Segment split info (`LC_SEGMENT_SPLIT_INFO`).
pub const LC_SEGMENT_SPLIT_INFO: u32 = 0x1e;
/// Load a library and re-export it (`LC_REEXPORT_DYLIB`).
pub const LC_REEXPORT_DYLIB: u32 = 0x1f | LC_REQ_DYLD;
/// Load a library on first use (`LC_LAZY_LOAD_DYLIB`).
pub const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
/// Encrypted segment information (`LC_ENCRYPTION_INFO`).
pub const LC_ENCRYPTION_INFO: u32 = 0x21;
/// Compressed dyld information (`LC_DYLD_INFO`).
pub const LC_DYLD_INFO: u32 = 0x22;
/// Compressed dyld information, required (`LC_DYLD_INFO_ONLY`).
pub const LC_DYLD_INFO_ONLY: u32 = 0x22 | LC_REQ_DYLD;
/// Load an upward dependency (`LC_LOAD_UPWARD_DYLIB`).
pub const LC_LOAD_UPWARD_DYLIB: u32 = 0x23 | LC_REQ_DYLD;
/// Minimum macOS version (`LC_VERSION_MIN_MACOSX`).
pub const LC_VERSION_MIN_MACOSX: u32 = 0x24;
/// Minimum iOS version (`LC_VERSION_MIN_IPHONEOS`).
pub const LC_VERSION_MIN_IPHONEOS: u32 = 0x25;
/// Function start addresses (`LC_FUNCTION_STARTS`).
pub const LC_FUNCTION_STARTS: u32 = 0x26;
/// A string for dyld to treat like an environment variable (`LC_DYLD_ENVIRONMENT`).
pub const LC_DYLD_ENVIRONMENT: u32 = 0x27;
/// Entry point of the main executable (`LC_MAIN`).
pub const LC_MAIN: u32 = 0x28 | LC_REQ_DYLD;
/// Ranges of data in code (`LC_DATA_IN_CODE`).
pub const LC_DATA_IN_CODE: u32 = 0x29;
/// Source version (`LC_SOURCE_VERSION`).
pub const LC_SOURCE_VERSION: u32 = 0x2a;
/// Code signing requirements copied from linked dylibs (`LC_DYLIB_CODE_SIGN_DRS`).
pub const LC_DYLIB_CODE_SIGN_DRS: u32 = 0x2b;
/// 64-bit encrypted segment information (`LC_ENCRYPTION_INFO_64`).
pub const LC_ENCRYPTION_INFO_64: u32 = 0x2c;
/// Optimization hints (`LC_LINKER_OPTIMIZATION_HINT`).
pub const LC_LINKER_OPTIMIZATION_HINT: u32 = 0x2e;
/// Minimum tvOS version (`LC_VERSION_MIN_TVOS`).
pub const LC_VERSION_MIN_TVOS: u32 = 0x2f;
/// Minimum watchOS version (`LC_VERSION_MIN_WATCHOS`).
pub const LC_VERSION_MIN_WATCHOS: u32 = 0x30;
/// Build platform and versions (`LC_BUILD_VERSION`).
pub const LC_BUILD_VERSION: u32 = 0x32;
/// Export trie (`LC_DYLD_EXPORTS_TRIE`).
pub const LC_DYLD_EXPORTS_TRIE: u32 = 0x33 | LC_REQ_DYLD;
/// Chained fixups (`LC_DYLD_CHAINED_FIXUPS`).
pub const LC_DYLD_CHAINED_FIXUPS: u32 = 0x34 | LC_REQ_DYLD;
/// Atom info (`LC_ATOM_INFO`).
pub const LC_ATOM_INFO: u32 = 0x36;
/// Function variants (`LC_FUNCTION_VARIANTS`).
pub const LC_FUNCTION_VARIANTS: u32 = 0x37;
/// Function variant fixups (`LC_FUNCTION_VARIANT_FIXUPS`).
pub const LC_FUNCTION_VARIANT_FIXUPS: u32 = 0x38;
/// Target triple the code was compiled for (`LC_TARGET_TRIPLE`).
pub const LC_TARGET_TRIPLE: u32 = 0x39;
/// Lazy load dylib info (`LC_LAZY_LOAD_DYLIB_INFO`).
pub const LC_LAZY_LOAD_DYLIB_INFO: u32 = 0x3a;

/// Size of `struct segment_command_64`.
pub const SEGMENT_COMMAND_64_SIZE: usize = 72;
/// Size of `struct section_64`.
pub const SECTION_64_SIZE: usize = 80;
/// Size of `struct nlist_64` (`<mach-o/nlist.h>`).
pub const NLIST_64_SIZE: usize = 16;
/// Size of `struct dylib_command`.
pub const DYLIB_COMMAND_SIZE: usize = 24;
/// Size of `struct dylinker_command`, `rpath_command` and
/// `target_triple_command`: the command words and the string offset.
pub const STR_COMMAND_SIZE: usize = 12;
/// Size of `struct symtab_command`.
pub const SYMTAB_COMMAND_SIZE: usize = 24;
/// Size of `struct dysymtab_command`.
pub const DYSYMTAB_COMMAND_SIZE: usize = 80;
/// Size of `struct dyld_info_command`.
pub const DYLD_INFO_COMMAND_SIZE: usize = 48;
/// Size of `struct linkedit_data_command`.
pub const LINKEDIT_DATA_COMMAND_SIZE: usize = 16;
/// Size of `struct entry_point_command`.
pub const ENTRY_POINT_COMMAND_SIZE: usize = 24;
/// Size of `struct uuid_command`.
pub const UUID_COMMAND_SIZE: usize = 24;
/// Size of `struct build_version_command`, without its tool entries.
pub const BUILD_VERSION_COMMAND_SIZE: usize = 24;
/// Size of `struct build_tool_version`.
pub const BUILD_TOOL_VERSION_SIZE: usize = 8;
/// Size of `struct version_min_command`.
pub const VERSION_MIN_COMMAND_SIZE: usize = 16;
/// Size of `struct source_version_command`.
pub const SOURCE_VERSION_COMMAND_SIZE: usize = 16;
/// Size of `struct encryption_info_command`.
pub const ENCRYPTION_INFO_COMMAND_SIZE: usize = 20;
/// Size of `struct data_in_code_entry`.
pub const DATA_IN_CODE_ENTRY_SIZE: usize = 8;

/// The `segment_command` flag: the segment is made read-only after fixups
/// (`SG_READ_ONLY`).
pub const SG_READ_ONLY: u32 = 0x10;
/// Name of the segment that catches null references (`SEG_PAGEZERO`).
pub const SEG_PAGEZERO: &[u8] = b"__PAGEZERO";
/// Name of the traditional text segment (`SEG_TEXT`).
pub const SEG_TEXT: &[u8] = b"__TEXT";
/// Name of the segment holding what the link editor maintains (`SEG_LINKEDIT`).
pub const SEG_LINKEDIT: &[u8] = b"__LINKEDIT";

/// Mask of the section type in `section_64.flags` (`SECTION_TYPE`).
pub const SECTION_TYPE: u32 = 0x0000_00ff;
/// Regular section (`S_REGULAR`).
pub const S_REGULAR: u32 = 0x0;
/// Zero fill on demand (`S_ZEROFILL`).
pub const S_ZEROFILL: u32 = 0x1;
/// Only literal C strings (`S_CSTRING_LITERALS`).
pub const S_CSTRING_LITERALS: u32 = 0x2;
/// Non-lazy symbol pointers (`S_NON_LAZY_SYMBOL_POINTERS`).
pub const S_NON_LAZY_SYMBOL_POINTERS: u32 = 0x6;
/// Lazy symbol pointers (`S_LAZY_SYMBOL_POINTERS`).
pub const S_LAZY_SYMBOL_POINTERS: u32 = 0x7;
/// Symbol stubs (`S_SYMBOL_STUBS`).
pub const S_SYMBOL_STUBS: u32 = 0x8;
/// Function pointers for initialisation (`S_MOD_INIT_FUNC_POINTERS`).
pub const S_MOD_INIT_FUNC_POINTERS: u32 = 0x9;
/// Function pointers for termination (`S_MOD_TERM_FUNC_POINTERS`).
pub const S_MOD_TERM_FUNC_POINTERS: u32 = 0xa;
/// Zero fill on demand, may be larger than 4 GiB (`S_GB_ZEROFILL`).
pub const S_GB_ZEROFILL: u32 = 0xc;
/// Template of initial values for thread-local variables
/// (`S_THREAD_LOCAL_REGULAR`).
pub const S_THREAD_LOCAL_REGULAR: u32 = 0x11;
/// Zero filled template of initial values for thread-local variables
/// (`S_THREAD_LOCAL_ZEROFILL`).
pub const S_THREAD_LOCAL_ZEROFILL: u32 = 0x12;
/// Thread-local variable descriptors (`S_THREAD_LOCAL_VARIABLES`).
pub const S_THREAD_LOCAL_VARIABLES: u32 = 0x13;
/// 32-bit offsets to initialisers (`S_INIT_FUNC_OFFSETS`).
pub const S_INIT_FUNC_OFFSETS: u32 = 0x16;
/// The section contains only machine instructions (`S_ATTR_PURE_INSTRUCTIONS`).
pub const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
/// The section contains some machine instructions (`S_ATTR_SOME_INSTRUCTIONS`).
pub const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x0000_0400;

/// `dylib_use_command.marker` of the alternate encoding of `LC_LOAD_DYLIB`
/// (`DYLIB_USE_MARKER`).
pub const DYLIB_USE_MARKER: u32 = 0x1a74_1800;
/// The library is linked weakly (`DYLIB_USE_WEAK_LINK`).
pub const DYLIB_USE_WEAK_LINK: u32 = 0x01;
/// The library is re-exported (`DYLIB_USE_REEXPORT`).
pub const DYLIB_USE_REEXPORT: u32 = 0x02;
/// The library is an upward dependency (`DYLIB_USE_UPWARD`).
pub const DYLIB_USE_UPWARD: u32 = 0x04;
/// The library's initialisers run late (`DYLIB_USE_DELAYED_INIT`).
pub const DYLIB_USE_DELAYED_INIT: u32 = 0x08;

/// Known values of `build_version_command.platform` (`PLATFORM_*`).
pub const PLATFORM_UNKNOWN: u32 = 0;
/// macOS (`PLATFORM_MACOS`).
pub const PLATFORM_MACOS: u32 = 1;
/// iOS (`PLATFORM_IOS`).
pub const PLATFORM_IOS: u32 = 2;
/// tvOS (`PLATFORM_TVOS`).
pub const PLATFORM_TVOS: u32 = 3;
/// watchOS (`PLATFORM_WATCHOS`).
pub const PLATFORM_WATCHOS: u32 = 4;
/// bridgeOS (`PLATFORM_BRIDGEOS`).
pub const PLATFORM_BRIDGEOS: u32 = 5;
/// Mac Catalyst (`PLATFORM_MACCATALYST`).
pub const PLATFORM_MACCATALYST: u32 = 6;
/// iOS simulator (`PLATFORM_IOSSIMULATOR`).
pub const PLATFORM_IOSSIMULATOR: u32 = 7;
/// tvOS simulator (`PLATFORM_TVOSSIMULATOR`).
pub const PLATFORM_TVOSSIMULATOR: u32 = 8;
/// watchOS simulator (`PLATFORM_WATCHOSSIMULATOR`).
pub const PLATFORM_WATCHOSSIMULATOR: u32 = 9;
/// DriverKit (`PLATFORM_DRIVERKIT`).
pub const PLATFORM_DRIVERKIT: u32 = 10;
/// visionOS (`PLATFORM_VISIONOS`).
pub const PLATFORM_VISIONOS: u32 = 11;
/// visionOS simulator (`PLATFORM_VISIONOSSIMULATOR`).
pub const PLATFORM_VISIONOSSIMULATOR: u32 = 12;

/// Known values of `build_tool_version.tool` (`TOOL_*`).
pub const TOOL_CLANG: u32 = 1;
/// The Swift compiler (`TOOL_SWIFT`).
pub const TOOL_SWIFT: u32 = 2;
/// Apple's static linker (`TOOL_LD`).
pub const TOOL_LD: u32 = 3;
/// LLVM's linker (`TOOL_LLD`).
pub const TOOL_LLD: u32 = 4;

/// `INDIRECT_SYMBOL_LOCAL` of `<mach-o/loader.h>`.
pub const INDIRECT_SYMBOL_LOCAL: u32 = 0x8000_0000;
/// `INDIRECT_SYMBOL_ABS` of `<mach-o/loader.h>`.
pub const INDIRECT_SYMBOL_ABS: u32 = 0x4000_0000;

// ---------------------------------------------------------------------------
// <mach-o/loader.h>: dyld info opcodes
// ---------------------------------------------------------------------------

/// `REBASE_TYPE_POINTER`.
pub const REBASE_TYPE_POINTER: u8 = 1;
/// `REBASE_TYPE_TEXT_ABSOLUTE32`.
pub const REBASE_TYPE_TEXT_ABSOLUTE32: u8 = 2;
/// `REBASE_TYPE_TEXT_PCREL32`.
pub const REBASE_TYPE_TEXT_PCREL32: u8 = 3;
/// `REBASE_OPCODE_MASK`.
pub const REBASE_OPCODE_MASK: u8 = 0xf0;
/// `REBASE_IMMEDIATE_MASK`.
pub const REBASE_IMMEDIATE_MASK: u8 = 0x0f;
/// `REBASE_OPCODE_DONE`.
pub const REBASE_OPCODE_DONE: u8 = 0x00;
/// `REBASE_OPCODE_SET_TYPE_IMM`.
pub const REBASE_OPCODE_SET_TYPE_IMM: u8 = 0x10;
/// `REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB`.
pub const REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x20;
/// `REBASE_OPCODE_ADD_ADDR_ULEB`.
pub const REBASE_OPCODE_ADD_ADDR_ULEB: u8 = 0x30;
/// `REBASE_OPCODE_ADD_ADDR_IMM_SCALED`.
pub const REBASE_OPCODE_ADD_ADDR_IMM_SCALED: u8 = 0x40;
/// `REBASE_OPCODE_DO_REBASE_IMM_TIMES`.
pub const REBASE_OPCODE_DO_REBASE_IMM_TIMES: u8 = 0x50;
/// `REBASE_OPCODE_DO_REBASE_ULEB_TIMES`.
pub const REBASE_OPCODE_DO_REBASE_ULEB_TIMES: u8 = 0x60;
/// `REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB`.
pub const REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB: u8 = 0x70;
/// `REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB`.
pub const REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB: u8 = 0x80;

/// `BIND_TYPE_POINTER`.
pub const BIND_TYPE_POINTER: u8 = 1;
/// `BIND_TYPE_TEXT_ABSOLUTE32`.
pub const BIND_TYPE_TEXT_ABSOLUTE32: u8 = 2;
/// `BIND_TYPE_TEXT_PCREL32`.
pub const BIND_TYPE_TEXT_PCREL32: u8 = 3;
/// `BIND_SPECIAL_DYLIB_SELF`.
pub const BIND_SPECIAL_DYLIB_SELF: i32 = 0;
/// `BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE`.
pub const BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE: i32 = -1;
/// `BIND_SPECIAL_DYLIB_FLAT_LOOKUP`.
pub const BIND_SPECIAL_DYLIB_FLAT_LOOKUP: i32 = -2;
/// `BIND_SPECIAL_DYLIB_WEAK_LOOKUP`.
pub const BIND_SPECIAL_DYLIB_WEAK_LOOKUP: i32 = -3;
/// `BIND_SYMBOL_FLAGS_WEAK_IMPORT`.
pub const BIND_SYMBOL_FLAGS_WEAK_IMPORT: u8 = 0x1;
/// `BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION`.
pub const BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION: u8 = 0x8;
/// `BIND_OPCODE_MASK`.
pub const BIND_OPCODE_MASK: u8 = 0xf0;
/// `BIND_IMMEDIATE_MASK`.
pub const BIND_IMMEDIATE_MASK: u8 = 0x0f;
/// `BIND_OPCODE_DONE`.
pub const BIND_OPCODE_DONE: u8 = 0x00;
/// `BIND_OPCODE_SET_DYLIB_ORDINAL_IMM`.
pub const BIND_OPCODE_SET_DYLIB_ORDINAL_IMM: u8 = 0x10;
/// `BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB`.
pub const BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB: u8 = 0x20;
/// `BIND_OPCODE_SET_DYLIB_SPECIAL_IMM`.
pub const BIND_OPCODE_SET_DYLIB_SPECIAL_IMM: u8 = 0x30;
/// `BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM`.
pub const BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM: u8 = 0x40;
/// `BIND_OPCODE_SET_TYPE_IMM`.
pub const BIND_OPCODE_SET_TYPE_IMM: u8 = 0x50;
/// `BIND_OPCODE_SET_ADDEND_SLEB`.
pub const BIND_OPCODE_SET_ADDEND_SLEB: u8 = 0x60;
/// `BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB`.
pub const BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x70;
/// `BIND_OPCODE_ADD_ADDR_ULEB`.
pub const BIND_OPCODE_ADD_ADDR_ULEB: u8 = 0x80;
/// `BIND_OPCODE_DO_BIND`.
pub const BIND_OPCODE_DO_BIND: u8 = 0x90;
/// `BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB`.
pub const BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB: u8 = 0xa0;
/// `BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED`.
pub const BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED: u8 = 0xb0;
/// `BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB`.
pub const BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB: u8 = 0xc0;
/// `BIND_OPCODE_THREADED`.
pub const BIND_OPCODE_THREADED: u8 = 0xd0;

/// `EXPORT_SYMBOL_FLAGS_KIND_MASK`.
pub const EXPORT_SYMBOL_FLAGS_KIND_MASK: u64 = 0x03;
/// `EXPORT_SYMBOL_FLAGS_KIND_REGULAR`.
pub const EXPORT_SYMBOL_FLAGS_KIND_REGULAR: u64 = 0x00;
/// `EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL`.
pub const EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL: u64 = 0x01;
/// `EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE`.
pub const EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE: u64 = 0x02;
/// `EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION`.
pub const EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION: u64 = 0x04;
/// `EXPORT_SYMBOL_FLAGS_REEXPORT`.
pub const EXPORT_SYMBOL_FLAGS_REEXPORT: u64 = 0x08;
/// `EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER`.
pub const EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER: u64 = 0x10;
/// `EXPORT_SYMBOL_FLAGS_STATIC_RESOLVER`.
pub const EXPORT_SYMBOL_FLAGS_STATIC_RESOLVER: u64 = 0x20;

/// `DICE_KIND_DATA`.
pub const DICE_KIND_DATA: u16 = 0x0001;
/// `DICE_KIND_JUMP_TABLE8`.
pub const DICE_KIND_JUMP_TABLE8: u16 = 0x0002;
/// `DICE_KIND_JUMP_TABLE16`.
pub const DICE_KIND_JUMP_TABLE16: u16 = 0x0003;
/// `DICE_KIND_JUMP_TABLE32`.
pub const DICE_KIND_JUMP_TABLE32: u16 = 0x0004;
/// `DICE_KIND_ABS_JUMP_TABLE32`.
pub const DICE_KIND_ABS_JUMP_TABLE32: u16 = 0x0005;

// ---------------------------------------------------------------------------
// <mach-o/nlist.h>
// ---------------------------------------------------------------------------

/// Any of these bits set: a symbolic debugging entry (`N_STAB`).
pub const N_STAB: u8 = 0xe0;
/// Private external symbol (`N_PEXT`).
pub const N_PEXT: u8 = 0x10;
/// Mask for the type bits (`N_TYPE`).
pub const N_TYPE: u8 = 0x0e;
/// External symbol (`N_EXT`).
pub const N_EXT: u8 = 0x01;
/// Undefined (`N_UNDF`).
pub const N_UNDF: u8 = 0x0;
/// Absolute (`N_ABS`).
pub const N_ABS: u8 = 0x2;
/// Indirect (`N_INDR`).
pub const N_INDR: u8 = 0xa;
/// Prebound undefined (`N_PBUD`).
pub const N_PBUD: u8 = 0xc;
/// Defined in the section `n_sect` (`N_SECT`).
pub const N_SECT: u8 = 0xe;
/// Not in any section (`NO_SECT`).
pub const NO_SECT: u8 = 0;
/// Symbol is weak referenced (`N_WEAK_REF`).
pub const N_WEAK_REF: u16 = 0x0040;
/// Coalesced symbol is a weak definition (`N_WEAK_DEF`).
pub const N_WEAK_DEF: u16 = 0x0080;
/// Referenced dynamically (`REFERENCED_DYNAMICALLY`).
pub const REFERENCED_DYNAMICALLY: u16 = 0x0010;
/// Symbol is not to be dead stripped (`N_NO_DEAD_STRIP`).
pub const N_NO_DEAD_STRIP: u16 = 0x0020;
/// Symbol is a resolver function (`N_SYMBOL_RESOLVER`).
pub const N_SYMBOL_RESOLVER: u16 = 0x0100;
/// Library ordinal of a symbol defined in the image itself (`SELF_LIBRARY_ORDINAL`).
pub const SELF_LIBRARY_ORDINAL: u8 = 0x0;
/// Highest library ordinal (`MAX_LIBRARY_ORDINAL`).
pub const MAX_LIBRARY_ORDINAL: u8 = 0xfd;
/// Symbol is looked up dynamically (`DYNAMIC_LOOKUP_ORDINAL`).
pub const DYNAMIC_LOOKUP_ORDINAL: u8 = 0xfe;
/// Symbol is in the main executable (`EXECUTABLE_ORDINAL`).
pub const EXECUTABLE_ORDINAL: u8 = 0xff;

// ---------------------------------------------------------------------------
// <mach/i386/thread_status.h>, <mach/arm/thread_status.h>
// ---------------------------------------------------------------------------

/// Flavor of the x86-64 general register state (`x86_THREAD_STATE64`).
pub const X86_THREAD_STATE64: u32 = 4;
/// Number of 32-bit words of that state (`x86_THREAD_STATE64_COUNT`).
pub const X86_THREAD_STATE64_COUNT: u32 = 42;
/// Flavor of the arm64 general register state (`ARM_THREAD_STATE64`).
pub const ARM_THREAD_STATE64: u32 = 6;
/// Number of 32-bit words of that state (`ARM_THREAD_STATE64_COUNT`).
pub const ARM_THREAD_STATE64_COUNT: u32 = 68;

// ---------------------------------------------------------------------------
// <mach-o/fixup-chains.h>
// ---------------------------------------------------------------------------

/// Size of `struct dyld_chained_fixups_header`.
pub const CHAINED_FIXUPS_HEADER_SIZE: usize = 28;
/// Size of the fixed part of `struct dyld_chained_starts_in_segment`.
pub const CHAINED_STARTS_IN_SEGMENT_SIZE: usize = 22;
/// `page_start` value of a page without fixups (`DYLD_CHAINED_PTR_START_NONE`).
pub const DYLD_CHAINED_PTR_START_NONE: u16 = 0xffff;
/// `page_start` flag of a page with several starts (`DYLD_CHAINED_PTR_START_MULTI`).
pub const DYLD_CHAINED_PTR_START_MULTI: u16 = 0x8000;
/// Last entry of a list of starts (`DYLD_CHAINED_PTR_START_LAST`).
pub const DYLD_CHAINED_PTR_START_LAST: u16 = 0x8000;

/// `DYLD_CHAINED_PTR_ARM64E`: stride 8, unauthenticated target is a vm address.
pub const DYLD_CHAINED_PTR_ARM64E: u16 = 1;
/// `DYLD_CHAINED_PTR_64`: target is a vm address.
pub const DYLD_CHAINED_PTR_64: u16 = 2;
/// `DYLD_CHAINED_PTR_32`.
pub const DYLD_CHAINED_PTR_32: u16 = 3;
/// `DYLD_CHAINED_PTR_32_CACHE`.
pub const DYLD_CHAINED_PTR_32_CACHE: u16 = 4;
/// `DYLD_CHAINED_PTR_32_FIRMWARE`.
pub const DYLD_CHAINED_PTR_32_FIRMWARE: u16 = 5;
/// `DYLD_CHAINED_PTR_64_OFFSET`: target is a vm offset.
pub const DYLD_CHAINED_PTR_64_OFFSET: u16 = 6;
/// `DYLD_CHAINED_PTR_ARM64E_KERNEL` (also `DYLD_CHAINED_PTR_ARM64E_OFFSET`):
/// stride 4, unauthenticated target is a vm offset.
pub const DYLD_CHAINED_PTR_ARM64E_KERNEL: u16 = 7;
/// `DYLD_CHAINED_PTR_64_KERNEL_CACHE`.
pub const DYLD_CHAINED_PTR_64_KERNEL_CACHE: u16 = 8;
/// `DYLD_CHAINED_PTR_ARM64E_USERLAND`: stride 8, unauthenticated target is a vm offset.
pub const DYLD_CHAINED_PTR_ARM64E_USERLAND: u16 = 9;
/// `DYLD_CHAINED_PTR_ARM64E_FIRMWARE`.
pub const DYLD_CHAINED_PTR_ARM64E_FIRMWARE: u16 = 10;
/// `DYLD_CHAINED_PTR_X86_64_KERNEL_CACHE`.
pub const DYLD_CHAINED_PTR_X86_64_KERNEL_CACHE: u16 = 11;
/// `DYLD_CHAINED_PTR_ARM64E_USERLAND24`: like USERLAND with a 24-bit bind ordinal.
pub const DYLD_CHAINED_PTR_ARM64E_USERLAND24: u16 = 12;
/// `DYLD_CHAINED_PTR_ARM64E_SHARED_CACHE`.
pub const DYLD_CHAINED_PTR_ARM64E_SHARED_CACHE: u16 = 13;
/// `DYLD_CHAINED_PTR_ARM64E_SEGMENTED`.
pub const DYLD_CHAINED_PTR_ARM64E_SEGMENTED: u16 = 14;
/// `DYLD_CHAINED_PTR_ARM64E_SHARED_CACHE_V3`.
pub const DYLD_CHAINED_PTR_ARM64E_SHARED_CACHE_V3: u16 = 15;
/// `DYLD_CHAINED_PTR_SHARED_CACHE_V2`.
pub const DYLD_CHAINED_PTR_SHARED_CACHE_V2: u16 = 16;

/// `imports_format` of 32-bit imports (`DYLD_CHAINED_IMPORT`).
pub const DYLD_CHAINED_IMPORT: u32 = 1;
/// `imports_format` of 32-bit imports with a 32-bit addend (`DYLD_CHAINED_IMPORT_ADDEND`).
pub const DYLD_CHAINED_IMPORT_ADDEND: u32 = 2;
/// `imports_format` of 64-bit imports with a 64-bit addend (`DYLD_CHAINED_IMPORT_ADDEND64`).
pub const DYLD_CHAINED_IMPORT_ADDEND64: u32 = 3;
