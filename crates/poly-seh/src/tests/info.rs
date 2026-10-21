//! `UNWIND_INFO` parsing and the unwind code array.

use super::{Desc, Img, Mem, e};
use crate::consts::*;
use crate::info::{UnwindInfo, UnwindOp, build, decode, encode};
use crate::pdata::RuntimeFunction;
use crate::{SliceMemory, UnwindError};
use alloc::vec;
use alloc::vec::Vec;

fn slot(off: u8, op: u8, info: u8) -> u16 {
    u16::from(off) | u16::from(op) << 8 | u16::from(info) << 12
}

#[test]
fn every_documented_operation_decodes() {
    let codes = vec![
        slot(0x20, UWOP_SAVE_XMM128_FAR, 15),
        0x0010,
        0x0001, // offset 0x10010
        slot(0x1C, UWOP_SAVE_XMM128, 6),
        0x0002, // offset 2 * 16
        slot(0x18, UWOP_SAVE_NONVOL_FAR, RDI),
        0x0008,
        0x0002, // offset 0x20008
        slot(0x14, UWOP_SAVE_NONVOL, RBX),
        0x000A, // offset 10 * 8
        slot(0x10, UWOP_SET_FPREG, 0),
        slot(0x0C, UWOP_ALLOC_LARGE, 1),
        0x5678,
        0x0012, // 0x125678
        slot(0x08, UWOP_ALLOC_LARGE, 0),
        0x0200,                           // 0x200 * 8
        slot(0x04, UWOP_ALLOC_SMALL, 15), // 15 * 8 + 8 = 128
        slot(0x02, UWOP_PUSH_NONVOL, R15),
        slot(0x00, UWOP_PUSH_MACHFRAME, 1),
        slot(0x00, UWOP_PUSH_MACHFRAME, 0),
    ];
    let ops = decode(&codes).unwrap();
    let want = vec![
        e(0x20, UnwindOp::SaveXmm128 { reg: 15, offset: 0x10010 }),
        e(0x1C, UnwindOp::SaveXmm128 { reg: 6, offset: 32 }),
        e(0x18, UnwindOp::SaveNonvol { reg: RDI, offset: 0x20008 }),
        e(0x14, UnwindOp::SaveNonvol { reg: RBX, offset: 80 }),
        e(0x10, UnwindOp::SetFpReg),
        e(0x0C, UnwindOp::Alloc { size: 0x12_5678 }),
        e(0x08, UnwindOp::Alloc { size: 0x1000 }),
        e(0x04, UnwindOp::Alloc { size: 128 }),
        e(0x02, UnwindOp::PushNonvol { reg: R15 }),
        e(0x00, UnwindOp::PushMachFrame { error_code: true }),
        e(0x00, UnwindOp::PushMachFrame { error_code: false }),
    ];
    assert_eq!(ops, want);
    // Encoding picks the shortest documented form and decodes back.
    let again = decode(&encode(&want)).unwrap();
    assert_eq!(again, want);
    assert_eq!(encode(&[e(1, UnwindOp::Alloc { size: 8 })]), vec![slot(1, UWOP_ALLOC_SMALL, 0)]);
    assert_eq!(encode(&[e(1, UnwindOp::Alloc { size: 136 })]), vec![slot(1, UWOP_ALLOC_LARGE, 0), 17]);
    assert_eq!(encode(&[e(1, UnwindOp::Alloc { size: 0x8_0000 })]).len(), 3);
}

#[test]
fn bad_codes_are_errors() {
    for op in [6u8, 7, 11, 12, 13, 14, 15] {
        assert_eq!(decode(&[slot(0, op, 0)]), Err(UnwindError::BadCode { slot: 0 }), "op {op}");
    }
    // ALLOC_LARGE with operation info 2+, MACHFRAME with info 2+.
    assert_eq!(decode(&[slot(0, UWOP_ALLOC_LARGE, 2), 0, 0]), Err(UnwindError::BadCode { slot: 0 }));
    assert_eq!(
        decode(&[slot(2, UWOP_PUSH_NONVOL, 3), slot(0, UWOP_PUSH_MACHFRAME, 2)]),
        Err(UnwindError::BadCode { slot: 1 })
    );
    // Operations whose extra slots are missing.
    for codes in [
        vec![slot(0, UWOP_ALLOC_LARGE, 0)],
        vec![slot(0, UWOP_ALLOC_LARGE, 1), 0],
        vec![slot(0, UWOP_SAVE_NONVOL, 3)],
        vec![slot(0, UWOP_SAVE_NONVOL_FAR, 3), 1],
        vec![slot(0, UWOP_SAVE_XMM128, 3)],
        vec![slot(0, UWOP_SAVE_XMM128_FAR, 3), 1],
    ] {
        assert_eq!(decode(&codes), Err(UnwindError::TruncatedCodes), "{codes:x?}");
    }
    assert_eq!(decode(&[]), Ok(Vec::new()));
}

#[test]
fn header_handler_and_chain_are_read() {
    // Three code slots (padded to four), a frame register, a handler with
    // four bytes of language-specific data.
    let codes = encode(&[
        e(9, UnwindOp::SetFpReg),
        e(5, UnwindOp::Alloc { size: 0x40 }),
        e(1, UnwindOp::PushNonvol { reg: RBP }),
    ]);
    let mut trailer = 0x1234u32.to_le_bytes().to_vec();
    trailer.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    let bytes = build(UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER, 9, RBP, 2, &codes, &trailer);
    assert_eq!(bytes[0], 1 | (3 << 3));
    assert_eq!(bytes.len(), 4 + 4 * 2 + 8);
    let mem = SliceMemory { base: 0x4000, bytes: &bytes };
    let info = UnwindInfo::read(&mem, 0x4000).unwrap();
    assert_eq!((info.version, info.flags, info.prolog_size), (1, 3, 9));
    assert_eq!((info.frame_register, info.frame_offset), (RBP, 2));
    assert_eq!(info.codes, codes);
    assert_eq!(info.handler, Some(0x1234));
    assert_eq!(info.chained, None);
    // The data follows the handler RVA, after the padded code array.
    assert_eq!(info.handler_data(), Some(0x4000 + 4 + 8 + 4));
    assert_eq!(info.size(), 4 + 8 + 4);
    assert_eq!(info.ops().unwrap().len(), 3);

    // Chained info: a RUNTIME_FUNCTION after the codes.
    let parent = RuntimeFunction { begin: 0x1000, end: 0x1100, unwind: 0x8000 };
    let bytes = build(UNW_FLAG_CHAININFO, 0, 0, 0, &[slot(1, UWOP_PUSH_NONVOL, RSI)], &parent.to_bytes());
    let mem = SliceMemory { base: 0x4000, bytes: &bytes };
    let info = UnwindInfo::read(&mem, 0x4000).unwrap();
    assert_eq!(info.chained, Some(parent));
    assert_eq!(info.handler, None);
    assert_eq!(info.handler_data(), None);
    assert_eq!(info.size(), 4 + 4 + 12);
}

#[test]
fn bad_headers_are_errors() {
    let ok = build(0, 0, 0, 0, &[], &[]);
    let mem = |b: &[u8]| -> Result<UnwindInfo, UnwindError> {
        let m = SliceMemory { base: 0x4000, bytes: b };
        UnwindInfo::read(&m, 0x4000)
    };
    assert!(mem(&ok).is_ok());
    // Versions other than 1.
    for v in [0u8, 2, 3, 7] {
        let mut b = ok.clone();
        b[0] = v;
        assert_eq!(mem(&b), Err(UnwindError::BadVersion(v)));
    }
    // Undefined flag bits; chain info with a handler flag.
    let mut b = ok.clone();
    b[0] = 1 | 0x8 << 3;
    assert_eq!(mem(&b), Err(UnwindError::BadFlags(0x8)));
    let b = build(UNW_FLAG_CHAININFO | UNW_FLAG_EHANDLER, 0, 0, 0, &[], &[0u8; 12]);
    assert_eq!(mem(&b), Err(UnwindError::BadFlags(5)));
    // Misaligned.
    let m = SliceMemory { base: 0x4001, bytes: &ok };
    assert_eq!(UnwindInfo::read(&m, 0x4001), Err(UnwindError::Misaligned(0x4001)));
    // Truncated: the header, the codes, the handler RVA, the chain.
    assert!(matches!(mem(&ok[..3]), Err(UnwindError::Unreadable(_))));
    let b = build(0, 2, 0, 0, &[slot(1, 0, 3), slot(0, 0, 5)], &[]);
    assert!(matches!(mem(&b[..6]), Err(UnwindError::Unreadable(_))));
    let b = build(UNW_FLAG_EHANDLER, 0, 0, 0, &[], &[1, 2]);
    assert!(matches!(mem(&b), Err(UnwindError::Unreadable(_))));
    let b = build(UNW_FLAG_CHAININFO, 0, 0, 0, &[], &[0; 8]);
    assert!(matches!(mem(&b), Err(UnwindError::Unreadable(_))));
    // Unreadable altogether, and at the top of the address space.
    assert!(UnwindInfo::read(&Mem::default(), 0x4000).is_err());
    assert!(UnwindInfo::read(&Mem::default(), u64::MAX - 3).is_err());
}

#[test]
fn image_builder_lays_info_out_dword_aligned() {
    let mut img = Img::default();
    let d = Desc { prolog: 1, ops: vec![e(1, UnwindOp::PushNonvol { reg: RBX })], ..Desc::default() };
    let a = img.func(&[0x53, 0xC3], &d);
    let b = img.func(&[0xC3], &Desc::default());
    assert_eq!(a.unwind % 4, 0);
    assert_eq!(b.unwind % 4, 0);
    assert!(b.begin >= a.end);
}
