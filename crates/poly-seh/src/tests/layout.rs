//! The `winnt.h` byte layouts of `CONTEXT`, `EXCEPTION_RECORD` and
//! `DISPATCHER_CONTEXT`.

use crate::consts::*;
use crate::context::*;
use crate::record::*;

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

#[test]
fn context_fields_sit_at_their_winnt_offsets_and_round_trip() {
    let mut c = Context::default();
    c.p_home = [1, 2, 3, 4, 5, 6];
    c.context_flags = CONTEXT_FULL;
    c.mx_csr = 0x1F80;
    c.seg_cs = 0x33;
    c.seg_ds = 0x2B;
    c.seg_es = 0x2C;
    c.seg_fs = 0x53;
    c.seg_gs = 0x2D;
    c.seg_ss = 0x2E;
    c.eflags = 0x246;
    c.dr = [0x10, 0x11, 0x12, 0x13, 0x16, 0x17];
    for (i, r) in c.gpr.iter_mut().enumerate() {
        *r = 0x1000 + i as u64;
    }
    c.rip = 0x1_4000_1234;
    c.set_xmm(0, 0xAAAA);
    c.set_xmm(15, 0xFFFF);
    c.vector_register[0] = 0x77;
    c.vector_control = 0x21;
    c.debug_control = 0x22;
    c.last_branch_to_rip = 0x23;
    c.last_branch_from_rip = 0x24;
    c.last_exception_to_rip = 0x25;
    c.last_exception_from_rip = 0x26;
    let b = c.to_bytes();
    assert_eq!(b.len(), CONTEXT_SIZE);
    assert_eq!(CONTEXT_SIZE, 1232);
    assert_eq!(u64_at(&b, 0x00), 1);
    assert_eq!(u64_at(&b, 0x28), 6);
    assert_eq!(u32::from_le_bytes(b[0x30..0x34].try_into().unwrap()), 0x10000B);
    assert_eq!(u32::from_le_bytes(b[0x34..0x38].try_into().unwrap()), 0x1F80);
    assert_eq!(u16::from_le_bytes([b[0x38], b[0x39]]), 0x33);
    assert_eq!(u16::from_le_bytes([b[0x42], b[0x43]]), 0x2E);
    assert_eq!(u32::from_le_bytes(b[0x44..0x48].try_into().unwrap()), 0x246);
    assert_eq!(u64_at(&b, 0x48), 0x10); // Dr0
    assert_eq!(u64_at(&b, 0x70), 0x17); // Dr7
    assert_eq!(u64_at(&b, 0x78), 0x1000); // Rax
    assert_eq!(u64_at(&b, 0x90), 0x1003); // Rbx
    assert_eq!(u64_at(&b, 0x98), 0x1004); // Rsp
    assert_eq!(u64_at(&b, 0xF0), 0x100F); // R15
    assert_eq!(u64_at(&b, 0xF8), 0x1_4000_1234); // Rip
    assert_eq!(u64_at(&b, 0x1A0), 0xAAAA); // Xmm0
    assert_eq!(u64_at(&b, 0x290), 0xFFFF); // Xmm15
    assert_eq!(b[0x300], 0x77); // VectorRegister[0]
    assert_eq!(u64_at(&b, 0x4A0), 0x21);
    assert_eq!(u64_at(&b, 0x4C8), 0x26);
    assert_eq!(Context::from_bytes(&b), Some(c.clone()));
    assert_eq!(Context::from_bytes(&b[..CONTEXT_SIZE - 1]), None);
    assert_eq!((c.rsp(), c.xmm(15)), (0x1004, 0xFFFF));
    assert_eq!(CONTEXT_ALL, 0x10001F);
}

#[test]
fn exception_records_round_trip_and_refuse_too_many_parameters() {
    let r = ExceptionRecord::new(STATUS_ACCESS_VIOLATION, EXCEPTION_NONCONTINUABLE, 0x1_4000_0010, &[1, 0xDEAD]);
    assert_eq!(r.parameters(), &[1, 0xDEAD]);
    let b = r.to_bytes();
    assert_eq!(b.len(), 0x98);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0xC000_0005);
    assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 1);
    assert_eq!(u64_at(&b, 16), 0x1_4000_0010);
    assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 2);
    assert_eq!(u64_at(&b, 32), 1);
    assert_eq!(u64_at(&b, 40), 0xDEAD);
    assert_eq!(ExceptionRecord::from_bytes(&b), Ok(r));
    let mut too_many = b;
    too_many[24] = 16;
    assert_eq!(ExceptionRecord::from_bytes(&too_many), Err(BadRecord));
    assert_eq!(ExceptionRecord::from_bytes(&b[..0x97]), Err(BadRecord));
    // At most 15 parameters are kept.
    let r = ExceptionRecord::new(1, 0, 0, &[7; 20]);
    assert_eq!(r.number_parameters, 15);
}

#[test]
fn dispatcher_context_layout() {
    let d = DispatcherContext {
        control_pc: 1,
        image_base: 2,
        function_entry: 3,
        establisher_frame: 4,
        target_ip: 5,
        context_record: 6,
        language_handler: 7,
        handler_data: 8,
        history_table: 9,
        scope_index: 10,
    };
    let b = d.to_bytes();
    assert_eq!(b.len(), 0x50);
    for i in 0..9 {
        assert_eq!(u64_at(&b, 8 * i), i as u64 + 1);
    }
    assert_eq!(u32::from_le_bytes(b[0x48..0x4C].try_into().unwrap()), 10);
}
