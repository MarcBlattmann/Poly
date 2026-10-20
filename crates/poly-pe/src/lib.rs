//! PE/COFF image parsing and load planning.
//!
//! This is the host-testable half of our `ntdll` loader: everything a loader
//! decides by reading an executable, and nothing that needs an address space.
//! It parses PE32+ and PE32 images (and COFF object files' headers and
//! sections), plans how an image is laid out in memory, applies base
//! relocations, walks imports (with a resolver callback that fills the
//! import address table), exports (including forwarders), the TLS, load
//! config, exception, resource and debug directories.
//!
//! Layouts and constants come from Microsoft's public "PE Format"
//! specification (learn.microsoft.com/windows/win32/debug/pe-format, called
//! "the spec" in comments), the "x64 exception handling" page for unwind
//! data, and the public SDK header `winnt.h`, whose constant names we keep in
//! comments. The CodeView PDB 7.0 record ("RSDS") follows the public
//! `cvinfo.h` (`CV_INFO_PDB70`) from Microsoft's microsoft-pdb repository.
//!
//! # Typical use by a loader
//!
//! ```text
//! let pe = Pe::parse(file)?;                 // or Pe::parse_mapped(image)
//! let plan = pe.load_plan()?;                // regions, protections, size
//! plan.map_into(&mut image)?;                // headers + sections, zero fill
//! let info = *pe.info();                     // Copy: no borrow of `file`
//! relocate(&mut image, &info, actual_base)?; // if actual_base != preferred
//! bind_imports(&mut image, &info, |req| resolve(req))?;
//! // then: apply plan.regions() protections, run TLS callbacks, entry point
//! ```
//!
//! Functions that change an image ([`relocate`], [`bind_imports`],
//! [`bind_delay_imports`]) work in place on the *mapped* image and take an
//! [`ImageInfo`] (a copy of the headers) instead of a borrowed [`Pe`], so
//! the same buffer can be parsed before and after.
//!
//! # Robustness
//!
//! Nothing here panics or reads out of bounds, whatever the input: every
//! offset is checked, every addition that could overflow is checked, and
//! every loop is bounded by the size of the data or an explicit limit
//! ([`MAX_NAME_LEN`], [`MAX_BIND_ENTRIES`], [`MAX_RESOURCE_VISITS`]). The
//! crate is `no_std` and never allocates; lists are iterators over the
//! image bytes.

#![no_std]

mod debug;
mod exception;
mod exports;
mod headers;
mod imports;
mod loadcfg;
mod plan;
mod read;
mod reloc;
mod resource;
mod tls;
mod view;

#[cfg(test)]
mod tests;

pub use debug::{CodeView, DebugEntries, DebugEntry};
pub use exception::{RuntimeFunction, RuntimeFunctions, UnwindCode, UnwindCodes, UnwindInfo, UnwindOp, unwind_flags};
pub use exports::{Export, ExportNames, ExportTarget, Exports, ForwardTarget, Forwarder};
pub use headers::{
    Bits, CoffObject, DataDirectory, Directories, FileHeader, ImageInfo, OptionalHeader, Pe, Protection, SectionHeader,
    SectionTable, StringTable,
};
pub use imports::{
    BindError, BoundForwarder, BoundForwarders, BoundImport, BoundImports, DelayImport, DelayImports, ImportDescriptor,
    ImportDescriptors, ImportEntries, ImportEntry, ImportRequest, ImportSymbol, bind_delay_imports, bind_imports,
};
pub use loadcfg::LoadConfig;
pub use plan::{LoadPlan, Region, Regions};
pub use reloc::{Reloc, RelocBlock, RelocBlocks, RelocEntries, RelocStats, relocate};
pub use resource::{
    ResourceData, ResourceDirectory, ResourceEntries, ResourceEntry, ResourceKey, ResourceName, Utf16Str,
};
pub use tls::{TlsCallbacks, TlsDirectory};

/// Longest NUL-terminated name (DLL, import, export, forwarder, PDB path)
/// accepted, in bytes. MSVC itself truncates decorated names at 4096.
pub const MAX_NAME_LEN: usize = 4096;

/// Most import address table entries [`bind_imports`] or
/// [`bind_delay_imports`] fill in one call. Real programs have a few
/// thousand; the limit only stops hostile tables that point many
/// descriptors at one huge lookup table.
pub const MAX_BIND_ENTRIES: usize = 1 << 18;

/// Most resource directory entries [`ResourceDirectory::walk`] visits.
/// Subdirectories may be shared, so a small hostile tree can expand
/// exponentially; the walk stops with [`Error::TooMany`] instead.
pub const MAX_RESOURCE_VISITS: usize = 1 << 20;

/// Most sections an image may have. The spec: "the Windows loader limits
/// the number of sections to 96". Object files may have more.
pub const MAX_IMAGE_SECTIONS: usize = 96;

/// Why parsing or loading failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No "MZ" signature at offset 0.
    NotMz,
    /// `e_lfanew` does not point at the "PE\0\0" signature.
    NoPeSignature,
    /// A header, table or string runs past the end of the data.
    Truncated,
    /// The optional header's magic is neither PE32 (0x10b) nor PE32+ (0x20b),
    /// or it is too short for its fixed fields.
    BadOptionalHeader,
    /// Header fields contradict each other (alignment, sizes, section layout).
    BadLayout(&'static str),
    /// An RVA (relative virtual address) does not point at data that exists:
    /// outside the image, or (for a file) in a section's zero-filled tail.
    BadRva(u32),
    /// A virtual address does not fall inside the image.
    BadVa(u64),
    /// A directory's contents are malformed.
    Malformed(&'static str),
    /// A string has no NUL terminator within [`MAX_NAME_LEN`] bytes.
    BadString,
    /// More entries than a limit allows (see the `MAX_*` constants).
    TooMany,
    /// A base relocation of a type this machine does not use, or that this
    /// crate does not apply.
    UnsupportedRelocation { kind: u8, rva: u32 },
    /// The image cannot be moved: its relocations were stripped.
    RelocsStripped,
    /// The machine type is not supported for this operation.
    UnsupportedMachine(u16),
    /// The destination buffer is smaller than the image.
    BufferTooSmall,
}

/// `IMAGE_FILE_MACHINE_*` values (spec "Machine Types"; `winnt.h`).
pub mod machine {
    pub const UNKNOWN: u16 = 0x0000;
    pub const I386: u16 = 0x014c;
    pub const ARM: u16 = 0x01c0;
    pub const ARMNT: u16 = 0x01c4;
    pub const AMD64: u16 = 0x8664;
    pub const ARM64: u16 = 0xaa64;
}

/// COFF file header `Characteristics` (`IMAGE_FILE_*`; spec "Characteristics").
pub mod file_flags {
    pub const RELOCS_STRIPPED: u16 = 0x0001;
    pub const EXECUTABLE_IMAGE: u16 = 0x0002;
    pub const LARGE_ADDRESS_AWARE: u16 = 0x0020;
    pub const MACHINE_32BIT: u16 = 0x0100;
    pub const DEBUG_STRIPPED: u16 = 0x0200;
    pub const REMOVABLE_RUN_FROM_SWAP: u16 = 0x0400;
    pub const NET_RUN_FROM_SWAP: u16 = 0x0800;
    pub const SYSTEM: u16 = 0x1000;
    pub const DLL: u16 = 0x2000;
    pub const UP_SYSTEM_ONLY: u16 = 0x4000;
}

/// Optional header `DllCharacteristics` (`IMAGE_DLLCHARACTERISTICS_*`; spec
/// "DLL Characteristics").
pub mod dll_flags {
    pub const HIGH_ENTROPY_VA: u16 = 0x0020;
    pub const DYNAMIC_BASE: u16 = 0x0040;
    pub const FORCE_INTEGRITY: u16 = 0x0080;
    pub const NX_COMPAT: u16 = 0x0100;
    pub const NO_ISOLATION: u16 = 0x0200;
    pub const NO_SEH: u16 = 0x0400;
    pub const NO_BIND: u16 = 0x0800;
    pub const APPCONTAINER: u16 = 0x1000;
    pub const WDM_DRIVER: u16 = 0x2000;
    pub const GUARD_CF: u16 = 0x4000;
    pub const TERMINAL_SERVER_AWARE: u16 = 0x8000;
}

/// Optional header `Subsystem` (`IMAGE_SUBSYSTEM_*`; spec "Windows Subsystem").
pub mod subsystem {
    pub const UNKNOWN: u16 = 0;
    pub const NATIVE: u16 = 1;
    pub const WINDOWS_GUI: u16 = 2;
    pub const WINDOWS_CUI: u16 = 3;
    pub const OS2_CUI: u16 = 5;
    pub const POSIX_CUI: u16 = 7;
    pub const NATIVE_WINDOWS: u16 = 8;
    pub const WINDOWS_CE_GUI: u16 = 9;
    pub const EFI_APPLICATION: u16 = 10;
    pub const EFI_BOOT_SERVICE_DRIVER: u16 = 11;
    pub const EFI_RUNTIME_DRIVER: u16 = 12;
    pub const EFI_ROM: u16 = 13;
    pub const XBOX: u16 = 14;
    pub const WINDOWS_BOOT_APPLICATION: u16 = 16;
}

/// Section header `Characteristics` (`IMAGE_SCN_*`; spec "Section Flags").
pub mod section_flags {
    pub const TYPE_NO_PAD: u32 = 0x0000_0008;
    pub const CNT_CODE: u32 = 0x0000_0020;
    pub const CNT_INITIALIZED_DATA: u32 = 0x0000_0040;
    pub const CNT_UNINITIALIZED_DATA: u32 = 0x0000_0080;
    pub const LNK_INFO: u32 = 0x0000_0200;
    pub const LNK_REMOVE: u32 = 0x0000_0800;
    pub const LNK_COMDAT: u32 = 0x0000_1000;
    pub const GPREL: u32 = 0x0000_8000;
    /// Mask of the `IMAGE_SCN_ALIGN_*` field (objects only): value `n` in
    /// bits 20-23 means alignment `1 << (n - 1)`.
    pub const ALIGN_MASK: u32 = 0x00f0_0000;
    pub const LNK_NRELOC_OVFL: u32 = 0x0100_0000;
    pub const MEM_DISCARDABLE: u32 = 0x0200_0000;
    pub const MEM_NOT_CACHED: u32 = 0x0400_0000;
    pub const MEM_NOT_PAGED: u32 = 0x0800_0000;
    pub const MEM_SHARED: u32 = 0x1000_0000;
    pub const MEM_EXECUTE: u32 = 0x2000_0000;
    pub const MEM_READ: u32 = 0x4000_0000;
    pub const MEM_WRITE: u32 = 0x8000_0000;
}

/// Data directory indices (spec "Optional Header Data Directories";
/// `IMAGE_DIRECTORY_ENTRY_*`).
pub mod dir {
    pub const EXPORT: usize = 0;
    pub const IMPORT: usize = 1;
    pub const RESOURCE: usize = 2;
    pub const EXCEPTION: usize = 3;
    /// The certificate table. Its "RVA" is a file offset, and it is never
    /// mapped.
    pub const SECURITY: usize = 4;
    pub const BASERELOC: usize = 5;
    pub const DEBUG: usize = 6;
    pub const ARCHITECTURE: usize = 7;
    pub const GLOBALPTR: usize = 8;
    pub const TLS: usize = 9;
    pub const LOAD_CONFIG: usize = 10;
    pub const BOUND_IMPORT: usize = 11;
    pub const IAT: usize = 12;
    pub const DELAY_IMPORT: usize = 13;
    pub const COM_DESCRIPTOR: usize = 14;
    /// Number of directory slots a header can have.
    pub const COUNT: usize = 16;
}

/// Base relocation types (`IMAGE_REL_BASED_*`; spec "Base Relocation Types").
pub mod reloc_kind {
    pub const ABSOLUTE: u8 = 0;
    pub const HIGH: u8 = 1;
    pub const LOW: u8 = 2;
    pub const HIGHLOW: u8 = 3;
    pub const HIGHADJ: u8 = 4;
    pub const DIR64: u8 = 10;
}

/// Debug directory entry types (`IMAGE_DEBUG_TYPE_*`; spec "Debug Type").
pub mod debug_type {
    pub const UNKNOWN: u32 = 0;
    pub const COFF: u32 = 1;
    pub const CODEVIEW: u32 = 2;
    pub const FPO: u32 = 3;
    pub const MISC: u32 = 4;
    pub const EXCEPTION: u32 = 5;
    pub const FIXUP: u32 = 6;
    pub const OMAP_TO_SRC: u32 = 7;
    pub const OMAP_FROM_SRC: u32 = 8;
    pub const BORLAND: u32 = 9;
    pub const RESERVED10: u32 = 10;
    pub const CLSID: u32 = 11;
    pub const REPRO: u32 = 16;
    pub const EX_DLLCHARACTERISTICS: u32 = 20;
}

/// Load config `GuardFlags` (`IMAGE_GUARD_*`; spec "Load Configuration Layout").
pub mod guard_flags {
    pub const CF_INSTRUMENTED: u32 = 0x0000_0100;
    pub const CFW_INSTRUMENTED: u32 = 0x0000_0200;
    pub const CF_FUNCTION_TABLE_PRESENT: u32 = 0x0000_0400;
    pub const SECURITY_COOKIE_UNUSED: u32 = 0x0000_0800;
    pub const PROTECT_DELAYLOAD_IAT: u32 = 0x0000_1000;
    pub const DELAYLOAD_IAT_IN_ITS_OWN_SECTION: u32 = 0x0000_2000;
    pub const CF_EXPORT_SUPPRESSION_INFO_PRESENT: u32 = 0x0000_4000;
    pub const CF_ENABLE_EXPORT_SUPPRESSION: u32 = 0x0000_8000;
    pub const CF_LONGJUMP_TABLE_PRESENT: u32 = 0x0001_0000;
    /// Bits 28-31: size of the extra bytes per GuardCF function table entry.
    pub const CF_FUNCTION_TABLE_SIZE_MASK: u32 = 0xf000_0000;
}

/// Resource type IDs (`RT_*` in `winuser.h`).
pub mod resource_type {
    pub const CURSOR: u32 = 1;
    pub const BITMAP: u32 = 2;
    pub const ICON: u32 = 3;
    pub const MENU: u32 = 4;
    pub const DIALOG: u32 = 5;
    pub const STRING: u32 = 6;
    pub const FONTDIR: u32 = 7;
    pub const FONT: u32 = 8;
    pub const ACCELERATOR: u32 = 9;
    pub const RCDATA: u32 = 10;
    pub const MESSAGETABLE: u32 = 11;
    pub const GROUP_CURSOR: u32 = 12;
    pub const GROUP_ICON: u32 = 14;
    pub const VERSION: u32 = 16;
    pub const MANIFEST: u32 = 24;
}
