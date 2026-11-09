//! Universal ("fat") files: several Mach-O files for different CPUs in one.
//!
//! The header and the slice table are big-endian (`<mach-o/fat.h>`):
//!
//! ```text
//! fat_header      { magic, nfat_arch }                      8 bytes
//! fat_arch        { cputype, cpusubtype, offset, size, align }   20 bytes each
//! fat_arch_64     { cputype, cpusubtype, offset(u64), size(u64), align, reserved }   32 bytes each
//! ```
//!
//! [`Fat::parse`] checks the whole table once: it fits the file, every slice
//! is inside the file, not empty, and no slice overlaps the table or another
//! slice. After that [`Fat::arches`] and [`Fat::slice`] cannot fail.

use crate::bytes::{slice, u32_be, u64_be};
use crate::consts::*;
use crate::error::{Error, Result};
use crate::header::{Cpu, CpuKind, FileKind};

/// Most slices accepted in a universal file. Real files have two to five;
/// the limit keeps the overlap check cheap and rejects Java class files,
/// which start with the same magic and a version number in this place.
pub const MAX_FAT_ARCHES: usize = 64;

/// One slice of a universal file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FatArch {
    /// The CPU this slice is for.
    pub cpu: Cpu,
    /// File offset of the slice.
    pub offset: u64,
    /// Size of the slice in bytes.
    pub size: u64,
    /// Alignment of the slice as a power of 2 (not enforced on `offset`).
    pub align: u32,
}

/// A parsed universal file.
#[derive(Clone, Copy, Debug)]
pub struct Fat<'a> {
    data: &'a [u8],
    count: usize,
    is64: bool,
}

impl<'a> Fat<'a> {
    /// Validates the header and the slice table of a universal file.
    pub fn parse(data: &'a [u8]) -> Result<Fat<'a>> {
        let is64 = match FileKind::detect(data)? {
            FileKind::Fat => false,
            FileKind::Fat64 => true,
            _ => return Err(Error::NotMachO),
        };
        let count = u32_be(data, 4).ok_or(Error::Truncated)? as usize;
        if count > MAX_FAT_ARCHES {
            return Err(Error::TooManyArches);
        }
        let entry = if is64 { FAT_ARCH_64_SIZE } else { FAT_ARCH_SIZE };
        let table_end = FAT_HEADER_SIZE + count * entry;
        if data.len() < table_end {
            return Err(Error::Truncated);
        }
        let fat = Fat { data, count, is64 };
        // Bounds, emptiness and alignment of every slice.
        for i in 0..count {
            let a = fat.arch_at(i).ok_or(Error::Truncated)?;
            if a.size == 0 {
                return Err(Error::EmptySlice);
            }
            if a.align >= 64 {
                return Err(Error::BadSliceAlign);
            }
            let end = a.offset.checked_add(a.size).ok_or(Error::Truncated)?;
            if end > data.len() as u64 {
                return Err(Error::Truncated);
            }
            if a.offset < table_end as u64 {
                return Err(Error::SlicesOverlap);
            }
        }
        // Pairwise overlap: [a.offset, a.end) against every later slice.
        for i in 0..count {
            let a = fat.arch_at(i).ok_or(Error::Truncated)?;
            for j in (i + 1)..count {
                let b = fat.arch_at(j).ok_or(Error::Truncated)?;
                // Both ends were checked not to overflow above.
                if a.offset < b.offset + b.size && b.offset < a.offset + a.size {
                    return Err(Error::SlicesOverlap);
                }
            }
        }
        Ok(fat)
    }

    fn arch_at(&self, i: usize) -> Option<FatArch> {
        let d = self.data;
        if self.is64 {
            let base = FAT_HEADER_SIZE + i * FAT_ARCH_64_SIZE;
            Some(FatArch {
                cpu: Cpu { cputype: u32_be(d, base)? as i32, cpusubtype: u32_be(d, base + 4)? as i32 },
                offset: u64_be(d, base + 8)?,
                size: u64_be(d, base + 16)?,
                align: u32_be(d, base + 24)?,
            })
        } else {
            let base = FAT_HEADER_SIZE + i * FAT_ARCH_SIZE;
            Some(FatArch {
                cpu: Cpu { cputype: u32_be(d, base)? as i32, cpusubtype: u32_be(d, base + 4)? as i32 },
                offset: u64::from(u32_be(d, base + 8)?),
                size: u64::from(u32_be(d, base + 12)?),
                align: u32_be(d, base + 16)?,
            })
        }
    }

    /// The number of slices.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the file has no slices.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether the slice records are 64-bit (`FAT_MAGIC_64`).
    pub fn is_64(&self) -> bool {
        self.is64
    }

    /// The slices in file order.
    pub fn arches(&self) -> FatArches<'a> {
        FatArches { fat: *self, next: 0 }
    }

    /// The bytes of a slice.
    pub fn slice(&self, arch: &FatArch) -> Result<&'a [u8]> {
        slice(self.data, arch.offset, arch.size).ok_or(Error::Truncated)
    }

    /// The first slice for exactly this CPU type and subtype (capability
    /// bits ignored).
    pub fn find(&self, cpu: Cpu) -> Option<FatArch> {
        self.arches().find(|a| a.cpu.cputype == cpu.cputype && a.cpu.subtype() == cpu.subtype())
    }

    /// The first slice for a named CPU. For [`CpuKind::Arm64`] a slice of
    /// arm64e does not count, and the other way round.
    pub fn find_kind(&self, kind: CpuKind) -> Option<FatArch> {
        self.arches().find(|a| a.cpu.kind() == kind)
    }
}

/// Iterator over the slices of a [`Fat`].
#[derive(Clone, Debug)]
pub struct FatArches<'a> {
    fat: Fat<'a>,
    next: usize,
}

impl Iterator for FatArches<'_> {
    type Item = FatArch;

    fn next(&mut self) -> Option<FatArch> {
        if self.next >= self.fat.count {
            return None;
        }
        let a = self.fat.arch_at(self.next);
        self.next += 1;
        a
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.fat.count - self.next.min(self.fat.count);
        (n, Some(n))
    }
}

/// The Mach-O bytes of `data` for a CPU: the whole file if it is thin (and
/// then it must be for that CPU kind), or the matching slice of a
/// universal file.
///
/// This is what a loader calls first on a file it was asked to run.
pub fn select_slice(data: &[u8], kind: CpuKind) -> Result<&[u8]> {
    match FileKind::detect(data)? {
        FileKind::Fat | FileKind::Fat64 => {
            let fat = Fat::parse(data)?;
            let arch = fat.find_kind(kind).ok_or(Error::NoMatchingArch)?;
            fat.slice(&arch)
        }
        FileKind::Thin64 => {
            let h = crate::header::Header::parse(data)?;
            if h.cpu.kind() == kind { Ok(data) } else { Err(Error::NoMatchingArch) }
        }
        FileKind::Thin32 => Err(Error::Unsupported32Bit),
        FileKind::ThinBigEndian => Err(Error::UnsupportedBigEndian),
    }
}

impl ExactSizeIterator for FatArches<'_> {}
