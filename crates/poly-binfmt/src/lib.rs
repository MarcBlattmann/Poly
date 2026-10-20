//! Executable format detection.
//!
//! At exec time the kernel looks at the first bytes of an image and decides
//! which personality the new process gets: ELF → Linux, PE → NT, Mach-O → XNU.
//! This crate only *identifies* the format and its target CPU; it does not
//! load anything. Every field read here comes from the public format
//! specifications (ELF gABI, Microsoft PE/COFF spec, Apple `mach-o/loader.h`).
//!
//! The parser never panics and never reads out of bounds, whatever the input.
//! It is `no_std` so the kernel can use it directly.

#![no_std]

pub use poly_abi::Personality;

/// CPU architecture an image was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arch {
    X86,
    X86_64,
    Arm,
    Aarch64,
    /// Recognised format, architecture we do not track.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bits {
    B32,
    B64,
}

/// The result of looking at an image header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Elf {
        bits: Bits,
        arch: Arch,
    },
    Pe {
        bits: Bits,
        arch: Arch,
    },
    MachO {
        bits: Bits,
        arch: Arch,
    },
    /// A universal ("fat") Mach-O containing `count` slices.
    MachOFat {
        count: u32,
    },
    /// `#!` script. The interpreter decides the personality; the caller
    /// resolves it and calls [`detect`] again on the interpreter.
    Script,
}

impl Format {
    /// Personality a process gets when it execs this image.
    /// `None` for scripts, which inherit from their interpreter.
    pub const fn personality(self) -> Option<Personality> {
        match self {
            Format::Elf { .. } => Some(Personality::Linux),
            Format::Pe { .. } => Some(Personality::Nt),
            Format::MachO { .. } | Format::MachOFat { .. } => Some(Personality::Xnu),
            Format::Script => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No known magic number.
    Unknown,
    /// Magic matched but the header is cut short or inconsistent.
    Malformed,
}

/// Identify an executable from (at least) its first few hundred bytes.
pub fn detect(image: &[u8]) -> Result<Format, Error> {
    if image.starts_with(b"\x7fELF") {
        return elf(image);
    }
    if image.starts_with(b"MZ") {
        return pe(image);
    }
    if image.starts_with(b"#!") {
        return Ok(Format::Script);
    }
    macho(image)
}

/// Convenience: detect and map straight to a personality.
pub fn personality_of(image: &[u8]) -> Result<Personality, Error> {
    match detect(image)?.personality() {
        Some(p) => Ok(p),
        None => Err(Error::Unknown),
    }
}

fn u16_at(b: &[u8], off: usize, le: bool) -> Option<u16> {
    let s: [u8; 2] = b.get(off..off.checked_add(2)?)?.try_into().ok()?;
    Some(if le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
}

fn u32_at(b: &[u8], off: usize, le: bool) -> Option<u32> {
    let s: [u8; 4] = b.get(off..off.checked_add(4)?)?.try_into().ok()?;
    Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
}

fn elf(b: &[u8]) -> Result<Format, Error> {
    const EI_CLASS: usize = 4;
    const EI_DATA: usize = 5;
    const E_MACHINE: usize = 18;

    let bits = match b.get(EI_CLASS) {
        Some(1) => Bits::B32,
        Some(2) => Bits::B64,
        _ => return Err(Error::Malformed),
    };
    let le = match b.get(EI_DATA) {
        Some(1) => true,
        Some(2) => false,
        _ => return Err(Error::Malformed),
    };
    let arch = match u16_at(b, E_MACHINE, le).ok_or(Error::Malformed)? {
        3 => Arch::X86,
        62 => Arch::X86_64,
        40 => Arch::Arm,
        183 => Arch::Aarch64,
        _ => Arch::Other,
    };
    Ok(Format::Elf { bits, arch })
}

fn pe(b: &[u8]) -> Result<Format, Error> {
    const E_LFANEW: usize = 0x3c;
    const COFF_HEADER_LEN: usize = 20;

    let pe_off = u32_at(b, E_LFANEW, true).ok_or(Error::Malformed)? as usize;
    // A bare "MZ" with no PE header is a DOS program, which nothing here runs.
    if b.get(pe_off..pe_off.checked_add(4).ok_or(Error::Malformed)?) != Some(b"PE\0\0") {
        return Err(Error::Malformed);
    }
    let coff = pe_off + 4;
    let arch = match u16_at(b, coff, true).ok_or(Error::Malformed)? {
        0x014c => Arch::X86,
        0x8664 => Arch::X86_64,
        0x01c4 => Arch::Arm,
        0xaa64 => Arch::Aarch64,
        _ => Arch::Other,
    };
    let bits = match u16_at(b, coff + COFF_HEADER_LEN, true).ok_or(Error::Malformed)? {
        0x10b => Bits::B32,
        0x20b => Bits::B64,
        _ => return Err(Error::Malformed),
    };
    Ok(Format::Pe { bits, arch })
}

fn macho_arch(cputype: u32) -> Arch {
    const CPU_ARCH_ABI64: u32 = 0x0100_0000;
    match cputype {
        7 => Arch::X86,
        x if x == 7 | CPU_ARCH_ABI64 => Arch::X86_64,
        12 => Arch::Arm,
        x if x == 12 | CPU_ARCH_ABI64 => Arch::Aarch64,
        _ => Arch::Other,
    }
}

fn macho(b: &[u8]) -> Result<Format, Error> {
    // Magic is stored in the file's own byte order; read it big-endian and
    // match both orders.
    let magic = u32_at(b, 0, false).ok_or(Error::Unknown)?;
    let (bits, le) = match magic {
        0xfeed_face => (Bits::B32, false),
        0xfeed_facf => (Bits::B64, false),
        0xcefa_edfe => (Bits::B32, true),
        0xcffa_edfe => (Bits::B64, true),
        0xcafe_babe | 0xcafe_babf => return macho_fat(b),
        _ => return Err(Error::Unknown),
    };
    let cputype = u32_at(b, 4, le).ok_or(Error::Malformed)?;
    Ok(Format::MachO { bits, arch: macho_arch(cputype) })
}

fn macho_fat(b: &[u8]) -> Result<Format, Error> {
    // 0xcafebabe is also the Java class-file magic. Java follows it with a
    // version number (major >= 45), a fat header with a slice count. Real
    // universal binaries have a handful of slices, so a small count is the
    // standard way to tell them apart.
    const MAX_FAT_ARCHES: u32 = 30;
    let count = u32_at(b, 4, false).ok_or(Error::Malformed)?;
    if count == 0 || count > MAX_FAT_ARCHES {
        return Err(Error::Unknown);
    }
    Ok(Format::MachOFat { count })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn elf64_x86_64() -> Vec<u8> {
        let mut h = std::vec![0u8; 64];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = 2; // 64-bit
        h[5] = 1; // little endian
        h[6] = 1; // version
        h[18..20].copy_from_slice(&62u16.to_le_bytes());
        h
    }

    fn pe64(machine: u16) -> Vec<u8> {
        let mut h = std::vec![0u8; 0x200];
        h[..2].copy_from_slice(b"MZ");
        h[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        h[0x80..0x84].copy_from_slice(b"PE\0\0");
        h[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        h[0x84 + 20..0x84 + 22].copy_from_slice(&0x20bu16.to_le_bytes());
        h
    }

    fn macho64_arm64() -> Vec<u8> {
        let mut h = std::vec![0u8; 32];
        h[..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
        h[4..8].copy_from_slice(&0x0100_000cu32.to_le_bytes());
        h
    }

    #[test]
    fn elf_maps_to_linux() {
        let f = detect(&elf64_x86_64()).unwrap();
        assert_eq!(f, Format::Elf { bits: Bits::B64, arch: Arch::X86_64 });
        assert_eq!(f.personality(), Some(Personality::Linux));
    }

    #[test]
    fn pe_maps_to_nt() {
        let f = detect(&pe64(0x8664)).unwrap();
        assert_eq!(f, Format::Pe { bits: Bits::B64, arch: Arch::X86_64 });
        assert_eq!(f.personality(), Some(Personality::Nt));
        assert_eq!(detect(&pe64(0xaa64)).unwrap(), Format::Pe { bits: Bits::B64, arch: Arch::Aarch64 });
    }

    #[test]
    fn macho_maps_to_xnu() {
        let f = detect(&macho64_arm64()).unwrap();
        assert_eq!(f, Format::MachO { bits: Bits::B64, arch: Arch::Aarch64 });
        assert_eq!(f.personality(), Some(Personality::Xnu));
    }

    #[test]
    fn macho_big_endian_magic() {
        let mut h = std::vec![0u8; 32];
        h[..4].copy_from_slice(&0xfeed_faceu32.to_be_bytes());
        h[4..8].copy_from_slice(&7u32.to_be_bytes());
        assert_eq!(detect(&h).unwrap(), Format::MachO { bits: Bits::B32, arch: Arch::X86 });
    }

    #[test]
    fn fat_macho_vs_java_class() {
        let mut fat = std::vec![0u8; 8];
        fat[..4].copy_from_slice(&0xcafe_babeu32.to_be_bytes());
        fat[4..8].copy_from_slice(&2u32.to_be_bytes());
        assert_eq!(detect(&fat).unwrap(), Format::MachOFat { count: 2 });

        // Java 8 class file: minor 0, major 52 → reads as count 52.
        let mut class = std::vec![0u8; 8];
        class[..4].copy_from_slice(&0xcafe_babeu32.to_be_bytes());
        class[6..8].copy_from_slice(&52u16.to_be_bytes());
        assert_eq!(detect(&class), Err(Error::Unknown));
    }

    #[test]
    fn script_has_no_personality() {
        let f = detect(b"#!/bin/sh\n").unwrap();
        assert_eq!(f, Format::Script);
        assert_eq!(personality_of(b"#!/bin/sh\n"), Err(Error::Unknown));
    }

    #[test]
    fn dos_stub_without_pe_header_is_malformed() {
        let mut h = pe64(0x8664);
        h[0x80..0x84].copy_from_slice(b"XX\0\0");
        assert_eq!(detect(&h), Err(Error::Malformed));
    }

    #[test]
    fn hostile_e_lfanew_does_not_overflow() {
        let mut h = pe64(0x8664);
        h[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(detect(&h), Err(Error::Malformed));
    }

    #[test]
    fn unknown_and_empty() {
        assert_eq!(detect(b""), Err(Error::Unknown));
        assert_eq!(detect(b"hello world"), Err(Error::Unknown));
    }

    /// Every prefix of every fixture must return without panicking.
    #[test]
    fn truncation_never_panics() {
        for img in [elf64_x86_64(), pe64(0x8664), macho64_arm64()] {
            for n in 0..=img.len() {
                let _ = detect(&img[..n]);
            }
        }
    }
}
