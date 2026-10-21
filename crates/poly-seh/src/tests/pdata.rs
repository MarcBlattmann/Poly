//! Function tables: parsing, validation and binary search.

use super::{Mem, Rng};
use crate::UnwindError;
use crate::pdata::{FunctionTable, RuntimeFunction, lookup};
use alloc::vec::Vec;

fn rf(begin: u32, end: u32) -> RuntimeFunction {
    RuntimeFunction { begin, end, unwind: begin + 0x8000 }
}

fn table_bytes(funcs: &[RuntimeFunction]) -> Vec<u8> {
    funcs.iter().flat_map(|f| f.to_bytes()).collect()
}

#[test]
fn entries_round_trip_their_twelve_bytes() {
    let f = RuntimeFunction { begin: 0x1000, end: 0x1234, unwind: 0x5678 };
    let b = f.to_bytes();
    assert_eq!(&b[0..4], &0x1000u32.to_le_bytes());
    assert_eq!(&b[4..8], &0x1234u32.to_le_bytes());
    assert_eq!(&b[8..12], &0x5678u32.to_le_bytes());
    assert_eq!(RuntimeFunction::from_bytes(&b), f);
}

#[test]
fn lookup_finds_the_covering_entry_or_none() {
    let funcs = [rf(0x1000, 0x1010), rf(0x1010, 0x1080), rf(0x2000, 0x2100)];
    let bytes = table_bytes(&funcs);
    let t = FunctionTable::new(&bytes).unwrap();
    assert_eq!(t.len(), 3);
    t.validate().unwrap();
    let cases = [
        (0x0FFF, None),
        (0x1000, Some(0)),
        (0x100F, Some(0)),
        (0x1010, Some(1)),
        (0x107F, Some(1)),
        (0x1080, None), // the gap
        (0x1FFF, None),
        (0x2000, Some(2)),
        (0x20FF, Some(2)),
        (0x2100, None),
        (u32::MAX, None),
    ];
    for (rva, want) in cases {
        assert_eq!(t.lookup(rva).map(|(i, _)| i), want, "rva {rva:#x}");
    }
    // The same through memory.
    let mut mem = Mem::default();
    mem.map(0x9000, bytes.len());
    mem.write(0x9000, &bytes);
    for (rva, want) in cases {
        let got = lookup(&mem, 0x9000, 3, rva).unwrap();
        assert_eq!(got, want.map(|i| (i, funcs[i])), "rva {rva:#x}");
    }
}

#[test]
fn empty_and_single_tables() {
    let t = FunctionTable::new(&[]).unwrap();
    assert!(t.is_empty());
    assert_eq!(t.lookup(0x1000), None);
    t.validate().unwrap();
    let one = table_bytes(&[rf(0x10, 0x20)]);
    let t = FunctionTable::new(&one).unwrap();
    assert_eq!(t.lookup(0x10).map(|x| x.0), Some(0));
    assert_eq!(t.lookup(0x20), None);
}

#[test]
fn malformed_tables_are_refused_or_answered_without_panicking() {
    // Not a multiple of 12 bytes.
    assert_eq!(FunctionTable::new(&[0u8; 13]).unwrap_err(), UnwindError::BadTable);
    // An entry ending before it begins, and an unsorted table.
    let bad = table_bytes(&[rf(0x20, 0x10)]);
    assert_eq!(FunctionTable::new(&bad).unwrap().validate(), Err(UnwindError::BadTable));
    let unsorted = table_bytes(&[rf(0x3000, 0x3010), rf(0x1000, 0x1010), rf(0x2000, 0x2010)]);
    let t = FunctionTable::new(&unsorted).unwrap();
    assert_eq!(t.validate(), Err(UnwindError::BadTable));
    for rva in [0, 0x1000, 0x2005, 0x3005, u32::MAX] {
        let _ = t.lookup(rva);
    }
    let overlapping = table_bytes(&[rf(0x1000, 0x1100), rf(0x1080, 0x1200)]);
    assert_eq!(FunctionTable::new(&overlapping).unwrap().validate(), Err(UnwindError::BadTable));
    // A table in memory that is cut short: an error, not a panic.
    let mut mem = Mem::default();
    mem.map(0x9000, 20);
    assert!(matches!(lookup(&mem, 0x9000, 4, 0x1000), Err(UnwindError::Unreadable(_))));
    // A table at the top of the address space.
    assert!(lookup(&mem, u64::MAX - 4, 3, 0).is_err());
}

#[test]
fn binary_search_matches_a_linear_scan() {
    let mut rng = Rng(0x5EED_1234);
    for _ in 0..300 {
        let n = rng.below(40) as usize;
        let mut funcs = Vec::new();
        let mut at = rng.below(0x100) as u32;
        for _ in 0..n {
            at += rng.below(0x20) as u32; // a gap, maybe none
            let len = 1 + rng.below(0x40) as u32;
            funcs.push(rf(at, at + len));
            at += len;
        }
        let bytes = table_bytes(&funcs);
        let t = FunctionTable::new(&bytes).unwrap();
        t.validate().unwrap();
        for _ in 0..50 {
            let rva = rng.below(u64::from(at) + 0x40) as u32;
            let want = funcs.iter().position(|f| f.contains(rva));
            assert_eq!(t.lookup(rva).map(|x| x.0), want);
        }
    }
}
