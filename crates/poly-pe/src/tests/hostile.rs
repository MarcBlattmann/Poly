//! Hostile inputs: whatever the bytes, nothing panics, reads out of bounds
//! or loops forever. [`parse_everything`] runs every parser and loading step.

use std::vec;

use super::builder::*;
use super::*;

fn samples() -> Vec<Vec<u8>> {
    vec![sample_exe(Bits::Pe32Plus).file, sample_exe(Bits::Pe32).file, sample_dll(Bits::Pe32Plus).file]
}

#[test]
fn samples_parse_cleanly() {
    // The baseline: the fuzzed inputs below start from valid images.
    for f in samples() {
        parse_everything(&f);
        let pe = Pe::parse(&f).unwrap();
        pe.load_plan().unwrap();
    }
}

#[test]
fn every_truncation() {
    for f in samples() {
        // Every prefix through the headers, then a stride through the rest.
        let mut n = 0;
        while n <= f.len() {
            parse_everything(&f[..n]);
            n += if n < 0x600 { 1 } else { 7 };
        }
    }
}

#[test]
fn tiny_and_degenerate_inputs() {
    for input in [&b""[..], b"M", b"MZ", b"MZ\0\0", &[0; 64], &[0xff; 64], b"PE\0\0"] {
        parse_everything(input);
    }
    // e_lfanew pointing at itself, at the last bytes, and wrapping.
    for lfanew in [0x3c, 0x40 - 4, 0xffff_fffc, 0x8000_0000] {
        let mut f = vec![0u8; 0x100];
        put_bytes(&mut f, 0, b"MZ");
        put32(&mut f, 0x3c, lfanew);
        parse_everything(&f);
    }
}

/// Random byte changes, biased towards the headers and the directory
/// tables, plus "interesting" 32-bit values.
#[test]
fn random_mutations() {
    const INTERESTING: [u32; 10] =
        [0, 1, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0xffff_fff0, 0x1000, 0x0fff, 20, 0x8000_0010];
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    for f in samples() {
        for _ in 0..700 {
            let mut m = f.clone();
            for _ in 0..1 + rng.below(6) {
                let limit = if rng.below(2) == 0 { 0x400 } else { m.len() };
                let at = rng.below(limit);
                match rng.below(3) {
                    0 => m[at] = rng.next() as u8,
                    1 => m[at] ^= 1 << rng.below(8),
                    _ => {
                        let at = at & !3;
                        if at + 4 <= m.len() {
                            let v = INTERESTING[rng.below(INTERESTING.len())];
                            m[at..at + 4].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
            parse_everything(&m);
        }
    }
}

/// Every data directory pointing at the same place: tables of different
/// kinds overlapping each other.
#[test]
fn overlapping_directories() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let dirs = NT_OFFSET + 24 + 112;
    let targets = [s.data, s.idata, s.rdata, s.didat, 0x80, 0, 1, s.text];
    for &rva in &targets {
        for size in [0, 8, 0x40, 0xffff_ffff] {
            let mut f = s.file.clone();
            for i in 0..16 {
                put32(&mut f, dirs + i * 8, rva);
                put32(&mut f, dirs + i * 8 + 4, size);
            }
            parse_everything(&f);
        }
    }
    // Each directory in turn aimed at every other directory's table.
    let real: Vec<DataDirectory> = (0..16).map(|i| pe.directories().raw(i)).collect();
    for i in 0..16 {
        for d in &real {
            let mut f = s.file.clone();
            put32(&mut f, dirs + i * 8, d.virtual_address);
            put32(&mut f, dirs + i * 8 + 4, d.size);
            parse_everything(&f);
        }
    }
}

/// Section tables that lie: raw data pointers past the end, huge sizes,
/// sections overlapping the headers and each other.
#[test]
fn lying_section_tables() {
    let f = sample_exe(Bits::Pe32Plus).file;
    let table = NT_OFFSET + 24 + 240;
    let values = [0u32, 0x200, 0x1000, 0x7fff_ffff, 0xffff_ffff, 0xffff_f000, f.len() as u32];
    for i in [0usize, 1, 4] {
        for field in [8usize, 12, 16, 20] {
            for &v in &values {
                let mut m = f.clone();
                put32(&mut m, table + i * 40 + field, v);
                parse_everything(&m);
            }
        }
    }
}

/// Absurd header values one at a time.
#[test]
fn absurd_header_fields() {
    let f = sample_exe(Bits::Pe32Plus).file;
    let opt = NT_OFFSET + 24;
    let fields32 = [opt + 16, opt + 32, opt + 36, opt + 56, opt + 60, opt + 108];
    for &at in &fields32 {
        for v in [0u32, 1, 3, 0x8000_0000, u32::MAX] {
            let mut m = f.clone();
            put32(&mut m, at, v);
            parse_everything(&m);
        }
    }
    for (at, v) in [(NT_OFFSET + 6, 0xffffu16), (NT_OFFSET + 6, 0), (NT_OFFSET + 20, 0), (NT_OFFSET + 20, 0xffff)] {
        let mut m = f.clone();
        put16(&mut m, at, v);
        parse_everything(&m);
    }
    // A string table pointer and symbol count that overflow.
    let mut m = f.clone();
    put32(&mut m, NT_OFFSET + 4 + 8, u32::MAX);
    put32(&mut m, NT_OFFSET + 4 + 12, u32::MAX);
    parse_everything(&m);
}
