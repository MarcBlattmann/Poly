//! Executable format detection.
//!
//! At exec time the kernel looks at the first bytes of an image and decides
//! which personality the new process gets: ELF → Linux, PE → NT, Mach-O → XNU,
//! and an ELF with Poly's ABI note ([`NOTE_OWNER`], [`NT_POLY_ABI`]) → Poly's
//! native interface (ADR 0125).
//! This crate only *identifies* the format and its target CPU; it does not
//! load anything. Every field read here comes from the public format
//! specifications (ELF gABI, Microsoft PE/COFF spec, Apple `mach-o/loader.h`).
//!
//! The parser never panics and never reads out of bounds, whatever the input.
//! It is `no_std` so the kernel can use it directly.
//!
//! # The native ABI note
//!
//! A native Poly program is an x86-64 ELF executable with an `SHT_NOTE`
//! section in a `PT_NOTE` segment holding one note: owner `"Poly"` (name
//! size 5 with its NUL), type [`NT_POLY_ABI`], and a 4-byte little-endian
//! description, the native ABI version it was built for. This is how the BSDs
//! (`.note.tag`) and GNU (`.note.ABI-tag`) mark their binaries. The note
//! must lie in the bytes [`detect`] is given (the kernel reads the first
//! page); `poly-rt`'s linker script puts it right after the program headers.

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
    /// A native Poly program: a 64-bit little-endian ELF executable with
    /// Poly's ABI note (see the crate docs), built for native ABI version
    /// `abi`.
    Native {
        arch: Arch,
        abi: u32,
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
            Format::Native { .. } => Some(Personality::Native),
            Format::Script => None,
        }
    }
}

/// The owner of Poly's ABI note, with its NUL (the note's name).
pub const NOTE_OWNER: &[u8] = b"Poly\0";
/// The type of Poly's ABI note: its description is the native ABI version
/// (a little-endian `u32`).
pub const NT_POLY_ABI: u32 = 1;
/// `PT_NOTE` (ELF gABI, "Program Header").
const PT_NOTE: u32 = 4;

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
    if bits == Bits::B64
        && le
        && let Some(abi) = poly_abi_note(b)
    {
        return Ok(Format::Native { arch, abi });
    }
    Ok(Format::Elf { bits, arch })
}

fn u64_le_at(b: &[u8], off: usize) -> Option<u64> {
    let s: [u8; 8] = b.get(off..off.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(s))
}

/// The native ABI version in Poly's note, if a `PT_NOTE` segment of the
/// 64-bit little-endian ELF image `b` has one within `b`.
fn poly_abi_note(b: &[u8]) -> Option<u32> {
    // ELF64 header: e_phoff at 32, e_phentsize at 54, e_phnum at 56.
    let phoff = usize::try_from(u64_le_at(b, 32)?).ok()?;
    let phentsize = usize::from(u16_at(b, 54, true)?);
    let phnum = usize::from(u16_at(b, 56, true)?);
    if phentsize < 56 {
        return None;
    }
    (0..phnum).find_map(|i| {
        let ph = phoff.checked_add(i.checked_mul(phentsize)?)?;
        if u32_at(b, ph, true)? != PT_NOTE {
            return None;
        }
        let offset = usize::try_from(u64_le_at(b, ph + 8)?).ok()?;
        let size = usize::try_from(u64_le_at(b, ph + 32)?).ok()?;
        // Notes are padded to the segment's alignment: 8 for 8-aligned
        // segments (the gABI's ELF64 rule), 4 as everyone writes them.
        let align = if u64_le_at(b, ph + 48)? == 8 { 8 } else { 4 };
        notes_find(b.get(offset..offset.checked_add(size)?)?, align)
    })
}

/// Poly's ABI note in the notes `seg` (padded to `align`), if there.
fn notes_find(mut seg: &[u8], align: usize) -> Option<u32> {
    let pad = |n: usize| n.checked_add(align - 1).map(|n| n & !(align - 1));
    while seg.len() >= 12 {
        let namesz = usize::try_from(u32_at(seg, 0, true)?).ok()?;
        let descsz = usize::try_from(u32_at(seg, 4, true)?).ok()?;
        let kind = u32_at(seg, 8, true)?;
        let name_end = 12usize.checked_add(namesz)?;
        let desc_at = pad(name_end)?;
        let next = pad(desc_at.checked_add(descsz)?)?;
        let name = seg.get(12..name_end)?;
        if name == NOTE_OWNER && kind == NT_POLY_ABI && descsz == 4 {
            return u32_at(seg, desc_at, true);
        }
        seg = seg.get(next..)?;
    }
    None
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

    /// An ELF64 x86-64 header with one `PT_NOTE` segment (alignment
    /// `align`) holding `notes`.
    fn elf_with_notes(notes: &[u8], align: u64) -> Vec<u8> {
        let mut h = elf64_x86_64();
        h[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        h[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        h[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        let mut ph = std::vec![0u8; 56];
        ph[0..4].copy_from_slice(&PT_NOTE.to_le_bytes());
        ph[8..16].copy_from_slice(&120u64.to_le_bytes()); // p_offset
        ph[32..40].copy_from_slice(&(notes.len() as u64).to_le_bytes()); // p_filesz
        ph[48..56].copy_from_slice(&align.to_le_bytes());
        h.extend_from_slice(&ph);
        h.extend_from_slice(notes);
        h
    }

    fn note(name: &[u8], kind: u32, desc: &[u8], align: usize) -> Vec<u8> {
        let pad = |v: &mut Vec<u8>| v.resize(v.len().div_ceil(align) * align, 0);
        let mut n = Vec::new();
        n.extend_from_slice(&(name.len() as u32).to_le_bytes());
        n.extend_from_slice(&(desc.len() as u32).to_le_bytes());
        n.extend_from_slice(&kind.to_le_bytes());
        n.extend_from_slice(name);
        pad(&mut n);
        n.extend_from_slice(desc);
        pad(&mut n);
        n
    }

    #[test]
    fn poly_note_maps_to_native() {
        let image = elf_with_notes(&note(NOTE_OWNER, NT_POLY_ABI, &1u32.to_le_bytes(), 4), 4);
        let f = detect(&image).unwrap();
        assert_eq!(f, Format::Native { arch: Arch::X86_64, abi: 1 });
        assert_eq!(f.personality(), Some(Personality::Native));
    }

    #[test]
    fn poly_note_after_another_note_and_with_8_byte_padding() {
        let mut notes = note(b"GNU\0", 3, &[0xaa; 20], 8);
        notes.extend(note(NOTE_OWNER, NT_POLY_ABI, &7u32.to_le_bytes(), 8));
        assert_eq!(detect(&elf_with_notes(&notes, 8)).unwrap(), Format::Native { arch: Arch::X86_64, abi: 7 });
    }

    #[test]
    fn other_notes_stay_linux() {
        let elf = Format::Elf { bits: Bits::B64, arch: Arch::X86_64 };
        // GNU's ABI tag, a Poly note of another type, one with a short
        // description, and one cut off by the end of the bytes given.
        let gnu = note(b"GNU\0", 1, &[0; 16], 4);
        let other_type = note(NOTE_OWNER, 2, &1u32.to_le_bytes(), 4);
        let short = note(NOTE_OWNER, NT_POLY_ABI, &[1, 0], 4);
        for notes in [gnu, other_type, short] {
            assert_eq!(detect(&elf_with_notes(&notes, 4)).unwrap(), elf);
        }
        let mut cut = elf_with_notes(&note(NOTE_OWNER, NT_POLY_ABI, &1u32.to_le_bytes(), 4), 4);
        cut.truncate(cut.len() - 2);
        assert_eq!(detect(&cut).unwrap(), elf);
        // A note size that would overflow is no note, not a panic.
        let mut huge = note(NOTE_OWNER, NT_POLY_ABI, &1u32.to_le_bytes(), 4);
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(detect(&elf_with_notes(&huge, 4)).unwrap(), elf);
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
