//! The dispatch and unwind walks over a three-frame stack:
//! `outer` (exception and termination handler) calls `mid` (both handlers)
//! calls `inner` (no handler), which faults.
//!
//! ```text
//!   T          return address 0 (the end of the stack)
//!   T-0x08     outer: caller's rbx
//!   T-0x28     outer's body RSP (its establisher frame)
//!   T-0x30     return address into outer
//!   T-0x38     mid: outer's rsi
//!   T-0x68     mid's body RSP
//!   T-0x70     return address into mid
//!   T-0x88     inner's body RSP: the fault
//! ```

use super::{BASE, Desc, Img, LIMITS, Mem, STACK_HIGH, e};
use crate::consts::*;
use crate::context::Context;
use crate::dispatch::{
    DispatchInfo, DispatchResult, Disposition, Handlers, Modules, StackLimits, UnwindFailure, dispatch_exception,
    unwind, unwind_record, walk,
};
use crate::info::UnwindOp;
use crate::pdata::RuntimeFunction;
use crate::record::ExceptionRecord;
use alloc::vec;
use alloc::vec::Vec;

const T: u64 = STACK_HIGH - 0x100;
const OUTER_HANDLER: u32 = 0x8000;
const MID_HANDLER: u32 = 0x8100;

struct Stack {
    target: Modules<Mem>,
    outer: RuntimeFunction,
    mid: RuntimeFunction,
    inner: RuntimeFunction,
    fault: Context,
}

fn stack() -> Stack {
    let mut img = Img::default();
    // push rbx; sub rsp, 0x20; call mid; nop; add rsp, 0x20; pop rbx; ret
    let outer_code = [0x53, 0x48, 0x83, 0xEC, 0x20, 0xE8, 0, 0, 0, 0, 0x90, 0x48, 0x83, 0xC4, 0x20, 0x5B, 0xC3];
    let outer = img.func(
        &outer_code,
        &Desc {
            flags: UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER,
            prolog: 5,
            ops: vec![e(5, UnwindOp::Alloc { size: 0x20 }), e(1, UnwindOp::PushNonvol { reg: RBX })],
            handler: Some(OUTER_HANDLER),
            ..Desc::default()
        },
    );
    // push rsi; sub rsp, 0x30; call inner; nop; ...
    let mid_code = [0x56, 0x48, 0x83, 0xEC, 0x30, 0xE8, 0, 0, 0, 0, 0x90, 0x48, 0x83, 0xC4, 0x30, 0x5E, 0xC3];
    let mid = img.func(
        &mid_code,
        &Desc {
            flags: UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER,
            prolog: 5,
            ops: vec![e(5, UnwindOp::Alloc { size: 0x30 }), e(1, UnwindOp::PushNonvol { reg: RSI })],
            handler: Some(MID_HANDLER),
            ..Desc::default()
        },
    );
    // sub rsp, 0x18; mov eax, [rax]; add rsp, 0x18; ret
    let inner_code = [0x48, 0x83, 0xEC, 0x18, 0x8B, 0x00, 0x48, 0x83, 0xC4, 0x18, 0xC3];
    let inner =
        img.func(&inner_code, &Desc { prolog: 4, ops: vec![e(4, UnwindOp::Alloc { size: 0x18 })], ..Desc::default() });
    let mut mem = Mem::with_stack();
    mem.put64(T, 0);
    mem.put64(T - 0x08, 0xB0B0);
    mem.put64(T - 0x30, BASE + u64::from(outer.begin) + 10);
    mem.put64(T - 0x38, 0x5151);
    mem.put64(T - 0x70, BASE + u64::from(mid.begin) + 10);
    let target = img.target(mem);
    let mut fault = Context::default();
    fault.rip = BASE + u64::from(inner.begin) + 4;
    fault.set_rsp(T - 0x88);
    fault.gpr[usize::from(RBX)] = 0x1111;
    fault.gpr[usize::from(RSI)] = 0x2222;
    Stack { target, outer, mid, inner, fault }
}

/// Test handlers: each call is logged, and the answer comes from a closure.
type Answer = dyn FnMut(u64, &mut ExceptionRecord, &mut Context, &DispatchInfo) -> Disposition;

struct Log {
    calls: Vec<(u64, u32, u64, u64)>, // (handler, record flags, establisher frame, frame pc)
    answer: alloc::boxed::Box<Answer>,
}

impl Handlers for Log {
    fn call(
        &mut self,
        record: &mut ExceptionRecord,
        establisher_frame: u64,
        context: &mut Context,
        dispatch: &mut DispatchInfo,
    ) -> Disposition {
        let h = dispatch.handler.routine;
        self.calls.push((h, record.flags, establisher_frame, dispatch.frame.rip));
        (self.answer)(h, record, context, dispatch)
    }
}

fn log(answer: impl FnMut(u64, &mut ExceptionRecord, &mut Context, &DispatchInfo) -> Disposition + 'static) -> Log {
    Log { calls: Vec::new(), answer: alloc::boxed::Box::new(answer) }
}

fn av(at: u64) -> ExceptionRecord {
    ExceptionRecord::new(STATUS_ACCESS_VIOLATION, 0, at, &[0, 0x10])
}

#[test]
fn search_passes_frames_without_handlers_then_one_handles() {
    let s = stack();
    let mut h = log(|h, _, ctx, _| {
        if h == BASE + u64::from(OUTER_HANDLER) {
            ctx.rip = 0x1_4000_7777; // fix up and continue
            Disposition::ContinueExecution
        } else {
            Disposition::ContinueSearch
        }
    });
    let mut rec = av(s.fault.rip);
    let mut ctx = s.fault.clone();
    let r = dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx);
    assert_eq!(r, DispatchResult::ContinueExecution);
    assert_eq!(ctx.rip, 0x1_4000_7777);
    // mid first, then outer, each with its establisher frame and its frame's pc.
    let mid_pc = BASE + u64::from(s.mid.begin) + 10;
    let outer_pc = BASE + u64::from(s.outer.begin) + 10;
    assert_eq!(
        h.calls,
        vec![
            (BASE + u64::from(MID_HANDLER), 0, T - 0x68, mid_pc),
            (BASE + u64::from(OUTER_HANDLER), 0, T - 0x28, outer_pc),
        ]
    );
    let _ = s.inner;
}

#[test]
fn nobody_handles_it() {
    let s = stack();
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    let mut rec = av(s.fault.rip);
    let mut ctx = s.fault.clone();
    assert_eq!(dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx), DispatchResult::NotHandled);
    assert_eq!(h.calls.len(), 2);
    assert_eq!(rec.flags & EXCEPTION_STACK_INVALID, 0, "the walk ended at the zero return address");
}

#[test]
fn noncontinuable_and_invalid_dispositions() {
    let s = stack();
    let mut h = log(|_, _, _, _| Disposition::ContinueExecution);
    let mut rec = av(s.fault.rip);
    rec.flags = EXCEPTION_NONCONTINUABLE;
    let mut ctx = s.fault.clone();
    assert_eq!(dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx), DispatchResult::Noncontinuable);
    for d in [Disposition::NestedException, Disposition::CollidedUnwind, Disposition::Other(7)] {
        let mut h = log(move |_, _, _, _| d);
        let mut rec = av(s.fault.rip);
        let mut ctx = s.fault.clone();
        assert_eq!(dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx), DispatchResult::InvalidDisposition);
    }
    assert_eq!(Disposition::from_u32(0), Disposition::ContinueExecution);
    assert_eq!(Disposition::from_u32(1), Disposition::ContinueSearch);
    assert_eq!(Disposition::from_u32(9), Disposition::Other(9));
}

#[test]
fn a_corrupt_stack_stops_the_search() {
    let mut s = stack();
    // mid's return address points into the void, with RSP outside the stack.
    s.target.memory.put64(T - 0x30, 0x1234);
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    let mut rec = av(s.fault.rip);
    let mut ctx = s.fault.clone();
    assert_eq!(dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx), DispatchResult::NotHandled);
    // The frame at 0x1234 is a "leaf" whose return address is at T-0x28.
    // Now limits that exclude everything.
    let mut s = stack();
    s.target.stack = StackLimits { low: 0, high: T - 0x80 };
    let mut rec = av(s.fault.rip);
    let mut ctx = s.fault.clone();
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    assert_eq!(dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx), DispatchResult::NotHandled);
    assert_ne!(rec.flags & EXCEPTION_STACK_INVALID, 0);
    assert!(h.calls.is_empty());
}

#[test]
fn a_leaf_at_the_top_is_unwound_through_rsp() {
    let mut s = stack();
    // The fault is in code without a table entry, called from inner's body:
    // its return address sits at RSP.
    let leaf_rsp = T - 0x90;
    s.target.memory.put64(leaf_rsp, BASE + u64::from(s.inner.begin) + 6);
    let mut ctx = s.fault.clone();
    ctx.rip = BASE + 0x2_8000; // inside the image, no entry
    ctx.set_rsp(leaf_rsp);
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    let mut rec = av(ctx.rip);
    assert_eq!(dispatch_exception(&s.target, &mut h, &mut rec, &mut ctx), DispatchResult::NotHandled);
    assert_eq!(h.calls.len(), 2);
}

#[test]
fn unwind_to_outer_calls_termination_handlers_and_resumes_there() {
    let s = stack();
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    let mut rec = av(s.fault.rip);
    let resumed = unwind(&s.target, &mut h, Some(T - 0x28), 0x1_4000_8888, &mut rec, 0x42, &s.fault).unwrap();
    assert_eq!(resumed.rip, 0x1_4000_8888);
    assert_eq!(resumed.gpr[usize::from(RAX)], 0x42);
    // outer's body context: its RSP, its RBX (never saved by mid or inner)
    // and the RSI mid had saved for it.
    assert_eq!(resumed.rsp(), T - 0x28);
    assert_eq!(resumed.gpr[usize::from(RBX)], 0x1111);
    assert_eq!(resumed.gpr[usize::from(RSI)], 0x5151);
    // mid unwound (not the target), then outer as the target.
    assert_eq!(h.calls.len(), 2);
    assert_eq!(h.calls[0].1, EXCEPTION_UNWINDING);
    assert_eq!(h.calls[0].2, T - 0x68);
    assert_eq!(h.calls[1].1, EXCEPTION_UNWINDING | EXCEPTION_TARGET_UNWIND);
    assert_eq!(h.calls[1].2, T - 0x28);
}

#[test]
fn unwind_failures() {
    let s = stack();
    // A target below the first frame: passed at once.
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    let mut rec = unwind_record(0);
    assert_eq!(rec.code, STATUS_UNWIND);
    let r = unwind(&s.target, &mut h, Some(T - 0x1000), 0, &mut rec, 0, &s.fault);
    assert_eq!(r, Err(UnwindFailure::InvalidUnwindTarget));
    // A target between frames: passed.
    let r = unwind(&s.target, &mut h, Some(T - 0x40), 0, &mut unwind_record(0), 0, &s.fault);
    assert_eq!(r, Err(UnwindFailure::InvalidUnwindTarget));
    // A termination handler that does not continue the search.
    let mut bad = log(|_, _, _, _| Disposition::ContinueExecution);
    let r = unwind(&s.target, &mut bad, Some(T - 0x28), 0, &mut unwind_record(0), 0, &s.fault);
    assert_eq!(r, Err(UnwindFailure::InvalidDisposition));
    // An exit unwind runs every termination handler and ends with the stack.
    let mut h = log(|_, _, _, _| Disposition::ContinueSearch);
    let mut rec = unwind_record(0);
    let r = unwind(&s.target, &mut h, None, 0, &mut rec, 0, &s.fault);
    assert_eq!(r, Err(UnwindFailure::EndOfStack));
    assert_eq!(h.calls.len(), 2);
    assert!(h.calls.iter().all(|c| c.1 & (EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND) == 6));
    // A corrupt stack.
    let mut s = stack();
    s.target.stack = LIMITS;
    s.target.memory.put64(T - 0x70, 0);
    s.target.memory.put64(T - 0x70, 0x0000_0000_0000_0008);
    let r = unwind(&s.target, &mut h, Some(T - 0x28), 0, &mut unwind_record(0), 0, &s.fault);
    assert_eq!(r, Err(UnwindFailure::BadStack));
}

#[test]
fn walk_gives_a_backtrace() {
    let s = stack();
    let frames = walk(&s.target, &s.fault, 16);
    let pcs: Vec<u64> = frames.iter().map(|f| f.pc).collect();
    assert_eq!(pcs, vec![s.fault.rip, BASE + u64::from(s.mid.begin) + 10, BASE + u64::from(s.outer.begin) + 10]);
    assert!(frames.iter().all(|f| f.function.is_some()));
    assert_eq!(walk(&s.target, &s.fault, 1).len(), 1);
    // A context outside the stack: nothing to walk.
    let mut c = s.fault.clone();
    c.set_rsp(0x10);
    assert!(walk(&s.target, &c, 16).is_empty());
}
