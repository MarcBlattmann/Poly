//! The Mach-O file header and CPU descriptions.

use crate::bytes::{u32_be, u32_le};
use crate::consts::*;
use crate::error::{Error, Result};

/// The CPU a Mach-O file or a universal slice is for: a CPU type and a
/// CPU subtype as stored in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpu {
    /// `cputype`: `CPU_TYPE_*` of `<mach/machine.h>`.
    pub cputype: i32,
    /// `cpusubtype` as stored: the subtype in the low 24 bits and the
    /// capability bits in the high 8 ([`CPU_SUBTYPE_MASK`]).
    pub cpusubtype: i32,
}

/// The CPUs this project cares about, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuKind {
    /// x86-64.
    X86_64,
    /// arm64, without pointer authentication.
    Arm64,
    /// arm64e: arm64 with pointer authentication.
    Arm64e,
    /// Anything else (32-bit ARM, i386, arm64_32, PowerPC, ...).
    Other,
}

impl Cpu {
    /// The x86-64 CPU with the usual subtype.
    pub const X86_64: Cpu = Cpu { cputype: CPU_TYPE_X86_64, cpusubtype: CPU_SUBTYPE_X86_64_ALL };
    /// The arm64 CPU with the usual subtype.
    pub const ARM64: Cpu = Cpu { cputype: CPU_TYPE_ARM64, cpusubtype: CPU_SUBTYPE_ARM64_ALL };
    /// The arm64e CPU.
    pub const ARM64E: Cpu = Cpu { cputype: CPU_TYPE_ARM64, cpusubtype: CPU_SUBTYPE_ARM64E };

    /// The subtype with the capability bits masked off.
    pub const fn subtype(&self) -> i32 {
        self.cpusubtype & !CPU_SUBTYPE_MASK
    }

    /// The capability bits of the subtype: the high 8 bits.
    ///
    /// `lipo` sets `CPU_SUBTYPE_LIB64` on x86-64 slices; on arm64e the
    /// bits 24..28 hold the pointer authentication ABI version.
    pub const fn capabilities(&self) -> u32 {
        (self.cpusubtype & CPU_SUBTYPE_MASK) as u32
    }

    /// The pointer authentication ABI version of an arm64 subtype
    /// (`CPU_SUBTYPE_ARM64_PTR_AUTH_VERSION`), 0 if none.
    pub const fn ptrauth_version(&self) -> u32 {
        ((self.cpusubtype & CPU_SUBTYPE_ARM64_PTR_AUTH_MASK) >> 24) as u32
    }

    /// Which of the named CPUs this is.
    pub const fn kind(&self) -> CpuKind {
        match self.cputype {
            CPU_TYPE_X86_64 => CpuKind::X86_64,
            CPU_TYPE_ARM64 => {
                let sub = self.subtype();
                if sub == CPU_SUBTYPE_ARM64E || sub == CPU_SUBTYPE_ARM64E_X1 { CpuKind::Arm64e } else { CpuKind::Arm64 }
            }
            _ => CpuKind::Other,
        }
    }
}

/// `filetype` of a Mach-O header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    /// `MH_OBJECT`.
    Object,
    /// `MH_EXECUTE`.
    Execute,
    /// `MH_DYLIB`.
    Dylib,
    /// `MH_DYLINKER`.
    Dylinker,
    /// `MH_BUNDLE`.
    Bundle,
    /// `MH_DYLIB_STUB`.
    DylibStub,
    /// `MH_DSYM`.
    Dsym,
    /// `MH_KEXT_BUNDLE`.
    KextBundle,
    /// `MH_FILESET`.
    Fileset,
    /// `MH_CORE`.
    Core,
    /// Any other value (`MH_FVMLIB`, `MH_PRELOAD`, GPU types, unknown).
    Other(u32),
}

/// What the first word of a file says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A universal file with 32-bit slice records.
    Fat,
    /// A universal file with 64-bit slice records.
    Fat64,
    /// A 64-bit little-endian Mach-O file.
    Thin64,
    /// A 32-bit little-endian Mach-O file (recognised, not read).
    Thin32,
    /// A big-endian Mach-O file (recognised, not read).
    ThinBigEndian,
}

impl FileKind {
    /// Reads the first word of `data`.
    pub fn detect(data: &[u8]) -> Result<FileKind> {
        // Fat magics are big-endian in the file; thin magics little-endian.
        let be = u32_be(data, 0).ok_or(Error::Truncated)?;
        let le = u32_le(data, 0).ok_or(Error::Truncated)?;
        if be == FAT_MAGIC {
            Ok(FileKind::Fat)
        } else if be == FAT_MAGIC_64 {
            Ok(FileKind::Fat64)
        } else if le == MH_MAGIC_64 {
            Ok(FileKind::Thin64)
        } else if le == MH_MAGIC {
            Ok(FileKind::Thin32)
        } else if le == MH_CIGAM || le == MH_CIGAM_64 {
            Ok(FileKind::ThinBigEndian)
        } else {
            Err(Error::NotMachO)
        }
    }
}

/// `struct mach_header_64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// `magic`: always [`MH_MAGIC_64`] for a parsed header.
    pub magic: u32,
    /// `cputype` and `cpusubtype`.
    pub cpu: Cpu,
    /// `filetype`: `MH_*`.
    pub filetype: u32,
    /// `ncmds`: the number of load commands.
    pub ncmds: u32,
    /// `sizeofcmds`: the size of all load commands in bytes.
    pub sizeofcmds: u32,
    /// `flags`: `MH_*`.
    pub flags: u32,
    /// `reserved`.
    pub reserved: u32,
}

impl Header {
    /// Reads a 64-bit little-endian header from the start of `data`.
    ///
    /// Only the header is checked (magic, length); the load commands are
    /// checked by [`MachO::parse`](crate::MachO::parse).
    pub fn parse(data: &[u8]) -> Result<Header> {
        match FileKind::detect(data)? {
            FileKind::Thin64 => {}
            FileKind::Thin32 => return Err(Error::Unsupported32Bit),
            FileKind::ThinBigEndian => return Err(Error::UnsupportedBigEndian),
            FileKind::Fat | FileKind::Fat64 => return Err(Error::FatFile),
        }
        let w = |i: usize| u32_le(data, i * 4).ok_or(Error::Truncated);
        Ok(Header {
            magic: w(0)?,
            cpu: Cpu { cputype: w(1)? as i32, cpusubtype: w(2)? as i32 },
            filetype: w(3)?,
            ncmds: w(4)?,
            sizeofcmds: w(5)?,
            flags: w(6)?,
            reserved: w(7)?,
        })
    }

    /// `filetype` as an enum.
    pub const fn file_type(&self) -> FileType {
        match self.filetype {
            MH_OBJECT => FileType::Object,
            MH_EXECUTE => FileType::Execute,
            MH_DYLIB => FileType::Dylib,
            MH_DYLINKER => FileType::Dylinker,
            MH_BUNDLE => FileType::Bundle,
            MH_DYLIB_STUB => FileType::DylibStub,
            MH_DSYM => FileType::Dsym,
            MH_KEXT_BUNDLE => FileType::KextBundle,
            MH_FILESET => FileType::Fileset,
            MH_CORE => FileType::Core,
            other => FileType::Other(other),
        }
    }

    /// Whether `MH_PIE` is set: the main executable may be loaded at any
    /// address.
    pub const fn is_pie(&self) -> bool {
        self.flags & MH_PIE != 0
    }

    /// Whether the image uses the two-level namespace (`MH_TWOLEVEL`).
    pub const fn is_two_level(&self) -> bool {
        self.flags & MH_TWOLEVEL != 0
    }
}
