//! The virtual unwinder against hand-assembled functions and the stacks
//! their prologs leave.

use super::{BASE, Desc, Img, Mem, STACK_HIGH, e, simple_function};
use crate::consts::*;
use crate::context::Context;
use crate::info::UnwindOp;
use crate::pdata::RuntimeFunction;
use crate::unwind::{ContextPointers, Location, unwind_leaf, virtual_unwind};
use crate::{Memory, UnwindError};
use alloc::vec;

/// The stack pointer a function is entered with (pointing at its return
/// address).
const ENTRY: u64 = STACK_HIGH - 0x1000;
const RET: u64 = 0x1_4000_9999;

fn pc(f: &RuntimeFunction, offset: u32) -> u64 {
    BASE + u64::from(f.begin + offset)
}

fn unwind(
    mem: &dyn Memory,
    f: &RuntimeFunction,
    ctx: &mut Context,
    kind: u8,
) -> Result<crate::unwind::Unwound, UnwindError> {
    let control = ctx.rip;
    virtual_unwind(kind, BASE, control, f, ctx, mem, None)
}

/// The simple function's stack: caller's RBX and RSI pushed, 0x28 bytes
/// allocated below them.
fn simple_stack(mem: &mut Mem) {
    mem.put64(ENTRY, RET);
    mem.put64(ENTRY - 8, 0xB0B0); // caller's rbx
    mem.put64(ENTRY - 16, 0x5151); // caller's rsi
}

fn ctx_at(rip: u64, rsp: u64) -> Context {
    let mut c = Context::default();
    c.rip = rip;
    c.set_rsp(rsp);
    c.gpr[usize::from(RBX)] = 0x1111;
    c.gpr[usize::from(RSI)] = 0x2222;
    c
}

#[test]
fn body_undoes_every_prolog_operation() {
    let (code, d) = simple_function();
    let mut img = Img::default();
    let f = img.func(&code, &d);
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    simple_stack(&mut mem);

    let mut c = ctx_at(pc(&f, 6), ENTRY - 0x38);
    let mut ptrs = ContextPointers::default();
    let u = virtual_unwind(UNW_FLAG_NHANDLER, BASE, c.rip, &f, &mut c, &mem, Some(&mut ptrs)).unwrap();
    assert_eq!(u.location, Location::Body);
    assert_eq!(u.establisher_frame, ENTRY - 0x38);
    assert_eq!(u.handler, None);
    assert_eq!(c.rip, RET);
    assert_eq!(c.rsp(), ENTRY + 8);
    assert_eq!(c.gpr[usize::from(RBX)], 0xB0B0);
    assert_eq!(c.gpr[usize::from(RSI)], 0x5151);
    assert_eq!(ptrs.integer[usize::from(RBX)], Some(ENTRY - 8));
    assert_eq!(ptrs.integer[usize::from(RSI)], Some(ENTRY - 16));
    assert_eq!(ptrs.integer[usize::from(RAX)], None);
}

#[test]
fn prolog_undoes_only_what_ran() {
    let (code, d) = simple_function();
    let mut img = Img::default();
    let f = img.func(&code, &d);
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    simple_stack(&mut mem);
    // (offset, rsp at that point, rbx and rsi after the unwind)
    let cases = [(0, ENTRY, 0x1111, 0x2222), (1, ENTRY - 8, 0xB0B0, 0x2222), (2, ENTRY - 16, 0xB0B0, 0x5151)];
    for (offset, rsp, rbx, rsi) in cases {
        let mut c = ctx_at(pc(&f, offset), rsp);
        let u = unwind(&mem, &f, &mut c, UNW_FLAG_EHANDLER).unwrap();
        assert_eq!(u.location, Location::Prolog, "offset {offset}");
        assert_eq!((c.rip, c.rsp()), (RET, ENTRY + 8), "offset {offset}");
        assert_eq!((c.gpr[usize::from(RBX)], c.gpr[usize::from(RSI)]), (rbx, rsi), "offset {offset}");
    }
}

#[test]
fn epilog_is_simulated_from_any_of_its_instructions() {
    let (code, mut d) = simple_function();
    // A handler that must not be reported from an epilog.
    d.flags = UNW_FLAG_EHANDLER;
    d.handler = Some(0x5000);
    let mut img = Img::default();
    let f = img.func(&code, &d);
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    simple_stack(&mut mem);
    for (offset, rsp, rbx, rsi) in [
        (8, ENTRY - 0x38, 0xB0B0, 0x5151), // add rsp, 0x28
        (12, ENTRY - 16, 0xB0B0, 0x5151),  // pop rsi
        (13, ENTRY - 8, 0xB0B0, 0x2222),   // pop rbx (rsi already popped: the context's value)
        (14, ENTRY, 0x1111, 0x2222),       // ret
    ] {
        let mut c = ctx_at(pc(&f, offset), rsp);
        let u = unwind(&mem, &f, &mut c, UNW_FLAG_EHANDLER).unwrap();
        assert_eq!(u.location, Location::Epilog, "offset {offset}");
        assert_eq!(u.handler, None);
        assert_eq!((c.rip, c.rsp()), (RET, ENTRY + 8), "offset {offset}");
        assert_eq!((c.gpr[usize::from(RBX)], c.gpr[usize::from(RSI)]), (rbx, rsi), "offset {offset}");
    }
    // In the body the handler is reported, with its data address.
    let mut c = ctx_at(pc(&f, 6), ENTRY - 0x38);
    let u = unwind(&mem, &f, &mut c, UNW_FLAG_EHANDLER).unwrap();
    let h = u.handler.expect("a handler in the body");
    assert_eq!(h.routine, BASE + 0x5000);
    let info_end = BASE + u64::from(f.unwind) + 4 + 2 * 4;
    assert_eq!(h.data, info_end + 4);
    // Asked for termination handlers only: none.
    let mut c = ctx_at(pc(&f, 6), ENTRY - 0x38);
    assert_eq!(unwind(&mem, &f, &mut c, UNW_FLAG_UHANDLER).unwrap().handler, None);
}

#[test]
fn epilog_forms_ret_imm_rep_ret_and_tail_calls() {
    let d = Desc { prolog: 1, ops: vec![e(1, UnwindOp::PushNonvol { reg: RBX })], ..Desc::default() };
    let mut img = Img::default();
    // push rbx; nop; pop rbx; <terminator>
    let ret_imm = img.func(&[0x53, 0x90, 0x5B, 0xC2, 0x10, 0x00], &d);
    let rep_ret = img.func(&[0x53, 0x90, 0x5B, 0xF3, 0xC3], &d);
    // jmp rel32 out of the function: a tail call.
    let tail = img.func(&[0x53, 0x90, 0x5B, 0xE9, 0x00, 0x10, 0x00, 0x00], &d);
    // jmp [rip+0x100] with and without REX.W.
    let jmp_mem = img.func(&[0x53, 0x90, 0x5B, 0xFF, 0x25, 0x00, 0x01, 0x00, 0x00], &d);
    let rex_jmp_mem = img.func(&[0x53, 0x90, 0x5B, 0x48, 0xFF, 0x25, 0x00, 0x01, 0x00, 0x00], &d);
    // jmp rel8 back into the function: not an epilog.
    let inner_jmp = img.func(&[0x53, 0x90, 0x5B, 0xEB, 0xFB], &d);
    // jmp rax (mod 11): not a documented epilog jump.
    let jmp_reg = img.func(&[0x53, 0x90, 0x5B, 0xFF, 0xE0], &d);
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    mem.put64(ENTRY, RET);
    mem.put64(ENTRY - 8, 0xB0B0);

    for (f, extra) in [(ret_imm, 0x10), (rep_ret, 0), (tail, 0), (jmp_mem, 0), (rex_jmp_mem, 0)] {
        let mut c = ctx_at(pc(&f, 2), ENTRY - 8);
        let u = unwind(&mem, &f, &mut c, UNW_FLAG_NHANDLER).unwrap();
        assert_eq!(u.location, Location::Epilog, "{f:x?}");
        assert_eq!((c.rip, c.rsp(), c.gpr[usize::from(RBX)]), (RET, ENTRY + 8 + extra, 0xB0B0));
    }
    // Not epilogs: the body rule applies (the push is undone from RSP,
    // which here is wrong by design: the test only checks the choice).
    for f in [inner_jmp, jmp_reg] {
        let mut c = ctx_at(pc(&f, 2), ENTRY - 8);
        let u = unwind(&mem, &f, &mut c, UNW_FLAG_NHANDLER).unwrap();
        assert_eq!(u.location, Location::Body, "{f:x?}");
    }
}

#[test]
fn frame_pointer_survives_a_body_that_moved_rsp() {
    // push rbp; sub rsp, 0x40; lea rbp, [rsp+0x20]; sub rsp, 0x100 (alloca);
    // nop; lea rsp, [rbp+0x20]; pop rbp; ret
    let code = [
        0x55, // push rbp (1)
        0x48, 0x83, 0xEC, 0x40, // sub rsp, 0x40 (5)
        0x48, 0x8D, 0x6C, 0x24, 0x20, // lea rbp, [rsp+0x20] (10)
        0x48, 0x81, 0xEC, 0x00, 0x01, 0x00, 0x00, // sub rsp, 0x100 (17)
        0x90, // body (17)
        0x48, 0x8D, 0x65, 0x20, // lea rsp, [rbp+0x20] (18)
        0x5D, // pop rbp (22)
        0xC3, // ret (23)
    ];
    let d = Desc {
        prolog: 10,
        frame_reg: RBP,
        frame_off: 2,
        ops: vec![
            e(10, UnwindOp::SetFpReg),
            e(5, UnwindOp::Alloc { size: 0x40 }),
            e(1, UnwindOp::PushNonvol { reg: RBP }),
        ],
        ..Desc::default()
    };
    let mut img = Img::default();
    let f = img.func(&code, &d);
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    mem.put64(ENTRY, RET);
    mem.put64(ENTRY - 8, 0xBBBB); // caller's rbp
    let rbp = ENTRY - 8 - 0x40 + 0x20;

    // In the body, with RSP 0x100 lower than the prolog left it.
    let mut c = ctx_at(pc(&f, 17), ENTRY - 8 - 0x40 - 0x100);
    c.gpr[usize::from(RBP)] = rbp;
    let u = unwind(&mem, &f, &mut c, UNW_FLAG_NHANDLER).unwrap();
    assert_eq!(u.location, Location::Body);
    assert_eq!(u.establisher_frame, rbp - 0x20);
    assert_eq!((c.rip, c.rsp(), c.gpr[usize::from(RBP)]), (RET, ENTRY + 8, 0xBBBB));

    // In the epilog at `lea rsp, [rbp+0x20]`.
    let mut c = ctx_at(pc(&f, 18), ENTRY - 8 - 0x40 - 0x100);
    c.gpr[usize::from(RBP)] = rbp;
    let u = unwind(&mem, &f, &mut c, UNW_FLAG_NHANDLER).unwrap();
    assert_eq!(u.location, Location::Epilog);
    assert_eq!((c.rip, c.rsp(), c.gpr[usize::from(RBP)]), (RET, ENTRY + 8, 0xBBBB));

    // In the prolog before the frame pointer is set: RSP based.
    let mut c = ctx_at(pc(&f, 5), ENTRY - 8 - 0x40);
    c.gpr[usize::from(RBP)] = 0x7777; // not a frame pointer yet
    let u = unwind(&mem, &f, &mut c, UNW_FLAG_NHANDLER).unwrap();
    assert_eq!(u.location, Location::Prolog);
    assert_eq!(u.establisher_frame, ENTRY - 8 - 0x40);
    assert_eq!((c.rip, c.rsp(), c.gpr[usize::from(RBP)]), (RET, ENTRY + 8, 0xBBBB));
}

#[test]
fn saves_by_mov_xmm_and_far_forms() {
    // sub rsp, 0x48; mov [rsp+0x50], rbx; movaps [rsp+0x20], xmm6; body.
    let code = [
        0x48, 0x83, 0xEC, 0x48, // (4)
        0x48, 0x89, 0x5C, 0x24, 0x50, // (9)
        0x0F, 0x29, 0x74, 0x24, 0x20, // (14)
        0x90,
    ];
    let near = Desc {
        prolog: 14,
        ops: vec![
            e(14, UnwindOp::SaveXmm128 { reg: 6, offset: 0x20 }),
            e(9, UnwindOp::SaveNonvol { reg: RBX, offset: 0x50 }),
            e(4, UnwindOp::Alloc { size: 0x48 }),
        ],
        ..Desc::default()
    };
    // The same with the far forms, written slot by slot.
    let slot = |off: u8, op: u8, info: u8| u16::from(off) | u16::from(op) << 8 | u16::from(info) << 12;
    let far = Desc {
        raw_codes: Some(vec![
            slot(14, UWOP_SAVE_XMM128_FAR, 6),
            0x20,
            0,
            slot(9, UWOP_SAVE_NONVOL_FAR, RBX),
            0x50,
            0,
            slot(4, UWOP_ALLOC_LARGE, 1),
            0x48,
            0,
        ]),
        ..near.clone()
    };
    for d in [near, far] {
        let mut img = Img::default();
        let f = img.func(&code, &d);
        let mut mem = Mem::with_stack();
        img.install(&mut mem);
        let body = ENTRY - 0x48;
        mem.put64(ENTRY, RET);
        mem.put64(body + 0x50, 0xB0B0);
        mem.put128(body + 0x20, 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
        let mut c = ctx_at(pc(&f, 14), body);
        let mut ptrs = ContextPointers::default();
        virtual_unwind(0, BASE, c.rip, &f, &mut c, &mem, Some(&mut ptrs)).unwrap();
        assert_eq!((c.rip, c.rsp(), c.gpr[usize::from(RBX)]), (RET, ENTRY + 8, 0xB0B0));
        assert_eq!(c.xmm(6), 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
        assert_eq!(ptrs.xmm[6], Some(body + 0x20));
        assert_eq!(ptrs.integer[usize::from(RBX)], Some(body + 0x50));
    }
}

#[test]
fn large_allocations_both_forms() {
    let slot = |off: u8, op: u8, info: u8| u16::from(off) | u16::from(op) << 8 | u16::from(info) << 12;
    // sub rsp, imm32
    let code = [0x48, 0x81, 0xEC, 0x10, 0x00, 0x01, 0x00, 0x90];
    for (codes, size) in [
        (vec![slot(7, UWOP_ALLOC_LARGE, 0), 0x2002], 0x10010u64),
        (vec![slot(7, UWOP_ALLOC_LARGE, 1), 0x0010, 0x0001], 0x10010u64),
    ] {
        let d = Desc { prolog: 7, raw_codes: Some(codes), ..Desc::default() };
        let mut img = Img::default();
        let f = img.func(&code, &d);
        let mut mem = Mem::with_stack();
        img.install(&mut mem);
        mem.put64(ENTRY, RET);
        let mut c = ctx_at(pc(&f, 7), ENTRY - size);
        unwind(&mem, &f, &mut c, 0).unwrap();
        assert_eq!((c.rip, c.rsp()), (RET, ENTRY + 8));
    }
}

#[test]
fn machine_frames_with_and_without_error_code() {
    // An interrupt-style function: a machine frame, then push rbx.
    for error_code in [false, true] {
        let d = Desc {
            prolog: 1,
            ops: vec![e(1, UnwindOp::PushNonvol { reg: RBX }), e(0, UnwindOp::PushMachFrame { error_code })],
            ..Desc::default()
        };
        let mut img = Img::default();
        let f = img.func(&[0x53, 0x90, 0x5B, 0x48, 0xCF], &d);
        let mut mem = Mem::with_stack();
        img.install(&mut mem);
        let frame = ENTRY - 0x30; // the machine frame (with the error code, if any)
        let base = frame + if error_code { 8 } else { 0 };
        mem.put64(base, 0x1_4000_4444); // RIP
        mem.put64(base + 8, 0x33); // CS
        mem.put64(base + 16, 0x246); // EFLAGS
        mem.put64(base + 24, 0x7001_2340); // RSP
        mem.put64(base + 32, 0x2B); // SS
        mem.put64(frame - 8, 0xB0B0);
        let mut c = ctx_at(pc(&f, 1), frame - 8);
        unwind(&mem, &f, &mut c, 0).unwrap();
        assert_eq!((c.rip, c.rsp(), c.eflags), (0x1_4000_4444, 0x7001_2340, 0x246));
        assert_eq!((c.seg_cs, c.seg_ss, c.gpr[usize::from(RBX)]), (0x33, 0x2B, 0xB0B0));
    }
}

#[test]
fn chained_info_continues_in_the_parent() {
    let mut img = Img::default();
    // The parent: push rbx; sub rsp, 0x20; ...
    let parent_desc = Desc {
        prolog: 5,
        ops: vec![e(5, UnwindOp::Alloc { size: 0x20 }), e(1, UnwindOp::PushNonvol { reg: RBX })],
        flags: UNW_FLAG_EHANDLER,
        handler: Some(0x6000),
        ..Desc::default()
    };
    let parent = img.func(&[0x53, 0x48, 0x83, 0xEC, 0x20, 0x90, 0xC3], &parent_desc);
    // A fragment (cold code elsewhere) that pushes rsi, chained to the parent.
    let fragment_desc = Desc {
        flags: UNW_FLAG_CHAININFO,
        prolog: 1,
        ops: vec![e(1, UnwindOp::PushNonvol { reg: RSI })],
        chain: Some(parent),
        ..Desc::default()
    };
    let fragment = img.func(&[0x56, 0x90, 0x90, 0xCC], &fragment_desc);
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    mem.put64(ENTRY, RET);
    mem.put64(ENTRY - 8, 0xB0B0);
    mem.put64(ENTRY - 8 - 0x20 - 8, 0x5151);
    let mut c = ctx_at(pc(&fragment, 1), ENTRY - 8 - 0x20 - 8);
    let u = unwind(&mem, &fragment, &mut c, UNW_FLAG_EHANDLER).unwrap();
    assert_eq!((c.rip, c.rsp()), (RET, ENTRY + 8));
    assert_eq!((c.gpr[usize::from(RBX)], c.gpr[usize::from(RSI)]), (0xB0B0, 0x5151));
    // The fragment's handler is the parent's.
    assert_eq!(u.handler.map(|h| h.routine), Some(BASE + 0x6000));
}

#[test]
fn chain_loops_and_bad_data_are_errors_and_leave_the_context() {
    let mut img = Img::default();
    let code_rva = img.add_code(&[0x90; 16]);
    // Unwind info chained to an entry whose unwind info is itself.
    let at = img.xdata.len() as u32 + super::XDATA_RVA;
    let me = RuntimeFunction { begin: code_rva, end: code_rva + 16, unwind: at };
    let bytes = Img::info_bytes(&Desc { flags: UNW_FLAG_CHAININFO, chain: Some(me), ..Desc::default() });
    assert_eq!(img.add_info(&bytes), at);
    img.funcs.push(me);
    // SET_FPREG without a frame register.
    let no_fp = img.func(&[0x90, 0x90], &Desc { prolog: 1, ops: vec![e(1, UnwindOp::SetFpReg)], ..Desc::default() });
    let mut mem = Mem::with_stack();
    img.install(&mut mem);
    mem.put64(ENTRY, RET);

    let before = ctx_at(pc(&me, 4), ENTRY);
    let mut c = before.clone();
    assert_eq!(unwind(&mem, &me, &mut c, 0), Err(UnwindError::ChainTooDeep));
    assert_eq!(c, before);
    let mut c = ctx_at(pc(&no_fp, 1), ENTRY);
    assert_eq!(unwind(&mem, &no_fp, &mut c, 0), Err(UnwindError::NoFrameRegister));
    // An address outside the entry.
    let mut c = ctx_at(pc(&me, 16), ENTRY);
    assert_eq!(unwind(&mem, &me, &mut c, 0), Err(UnwindError::NotInFunction));
    // An unreadable stack: an error, the context unchanged.
    let (code, d) = simple_function();
    let mut img = Img::default();
    let f = img.func(&code, &d);
    let mut mem = Mem::default();
    img.install(&mut mem);
    let before = ctx_at(pc(&f, 6), ENTRY - 0x38);
    let mut c = before.clone();
    let mut ptrs = ContextPointers::default();
    let r = virtual_unwind(0, BASE, c.rip, &f, &mut c, &mem, Some(&mut ptrs));
    assert!(matches!(r, Err(UnwindError::Unreadable(_))));
    assert_eq!(c, before);
    assert_eq!(ptrs, ContextPointers::default());
}

#[test]
fn leaf_functions_return_through_rsp() {
    let mut mem = Mem::with_stack();
    mem.put64(ENTRY, RET);
    let mut c = ctx_at(0x1_4000_1000, ENTRY);
    unwind_leaf(&mut c, &mem).unwrap();
    assert_eq!((c.rip, c.rsp()), (RET, ENTRY + 8));
    let mut c = ctx_at(0x1_4000_1000, 0x10);
    assert!(unwind_leaf(&mut c, &mem).is_err());
}
