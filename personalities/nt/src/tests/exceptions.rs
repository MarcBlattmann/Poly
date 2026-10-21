//! Exception dispatch (ADR 0068): CPU exceptions classified, the dispatcher
//! frame written to the user stack, `NtRaiseException` and `NtContinue`.
//!
//! [`SehFake`] wraps the shared [`Fake`] and records what the services ask
//! of the kernel's exception side (`load_context`, `unhandled_exception`).

use super::*;
use crate::exception::*;
use crate::exec::TEB_STACK_BASE;
use poly_ntobj::status::{STATUS_ACCESS_VIOLATION as AV_STATUS, STATUS_INVALID_PARAMETER};
use poly_seh::consts::*;
use poly_seh::context::{CONTEXT_CONTROL, CONTEXT_FLOATING_POINT, CONTEXT_FULL, CONTEXT_INTEGER, CONTEXT_SIZE};
use poly_seh::record::EXCEPTION_RECORD_SIZE;
use poly_seh::{Context, ExceptionRecord};

struct SehFake {
    f: Fake,
    loaded: Vec<(Context, u32)>,
    unhandled: Option<(ExceptionRecord, Context)>,
    test_alerts: u32,
}

impl NtKernel for SehFake {
    fn read_user(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault> {
        self.f.read_user(addr, buf)
    }
    fn write_user(&mut self, addr: u64, data: &[u8]) -> Result<(), UserFault> {
        self.f.write_user(addr, data)
    }
    fn caller(&self) -> Caller {
        self.f.caller()
    }
    fn with_state<R>(&mut self, f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R) -> R {
        self.f.with_state(f)
    }
    fn terminate_process(&mut self, status: NtStatus) {
        self.f.terminate_process(status)
    }
    fn terminate_current_thread(&mut self, status: NtStatus) {
        self.f.terminate_current_thread(status)
    }
    fn create_thread(&mut self, req: ThreadCreate) -> Result<ThreadCreated, NtStatus> {
        self.f.create_thread(req)
    }
    fn terminate_thread(&mut self, thread: ThreadRef, status: NtStatus) -> Result<(), NtStatus> {
        self.f.terminate_thread(thread, status)
    }
    fn suspend_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        self.f.suspend_thread(thread)
    }
    fn resume_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        self.f.resume_thread(thread)
    }
    fn get_context(&mut self, thread: ThreadRef) -> Result<NtContext, NtStatus> {
        self.f.get_context(thread)
    }
    fn set_context(&mut self, thread: ThreadRef, ctx: &NtContext, want: u32) -> Result<(), NtStatus> {
        self.f.set_context(thread, ctx, want)
    }
    fn wait(&mut self, id: WaitId, timeout: NtTimeout, alertable: bool) -> NtStatus {
        self.f.wait(id, timeout, alertable)
    }
    fn arm_timer(
        &mut self,
        timer: poly_ntobj::ObjectId,
        due: NtTimeout,
        period: u32,
        apc: Option<Apc>,
    ) -> Result<bool, NtStatus> {
        self.f.arm_timer(timer, due, period, apc)
    }
    fn disarm_timer(&mut self, timer: poly_ntobj::ObjectId) -> Result<bool, NtStatus> {
        self.f.disarm_timer(timer)
    }
    fn queue_apc(&mut self, thread: ThreadRef, apc: Apc) -> Result<(), NtStatus> {
        self.f.queue_apc(thread, apc)
    }
    fn test_alert(&mut self) -> NtStatus {
        self.test_alerts += 1;
        self.f.test_alert()
    }
    fn delay(&mut self, timeout: NtTimeout, alertable: bool) -> NtStatus {
        self.f.delay(timeout, alertable)
    }
    fn yield_execution(&mut self) -> NtStatus {
        self.f.yield_execution()
    }
    fn resume_apc(&mut self) -> NtStatus {
        self.f.resume_apc()
    }
    fn trace(&mut self, values: [u64; 3]) {
        self.f.trace(values)
    }
    fn load_context(&mut self, ctx: &Context, flags: u32) -> Result<(), NtStatus> {
        self.loaded.push((ctx.clone(), flags));
        Ok(())
    }
    fn unhandled_exception(&mut self, record: &ExceptionRecord, ctx: &Context) {
        self.unhandled = Some((*record, ctx.clone()));
        self.f.terminate_process(NtStatus(record.code));
    }
}

const DISPATCHER: u64 = IMAGE_BASE + 0x1000;

impl SehFake {
    /// A process started from the test executable, with or without a
    /// dispatcher.
    fn new(dispatcher: u64) -> SehFake {
        let mut f = Fake::new();
        f.proc.exception_dispatcher = dispatcher;
        SehFake { f, loaded: Vec::new(), unhandled: None, test_alerts: 0 }
    }

    /// The top of the first thread's stack (its committed pages are RW).
    fn stack_top(&self) -> u64 {
        let teb = self.f.proc.threads[0].teb;
        self.f.u64_at(teb + TEB_STACK_BASE)
    }

    fn call(&mut self, service: Service, args: &[u64]) -> NtStatus {
        let mut regs = SyscallRegs { rax: u64::from(service.number()), rsp: STACK, ..Default::default() };
        let regs_args = [&mut regs.r10, &mut regs.rdx, &mut regs.r8, &mut regs.r9];
        for (slot, &a) in regs_args.into_iter().zip(args) {
            *slot = a;
        }
        assert_eq!(crate::dispatch_with(self, &mut regs), poly_abi::Outcome::Handled);
        NtStatus(regs.rax as u32)
    }

    fn context_at(&self, addr: u64) -> Context {
        Context::from_bytes(&self.f.bytes_at(addr, CONTEXT_SIZE)).unwrap()
    }

    fn record_at(&self, addr: u64) -> ExceptionRecord {
        ExceptionRecord::from_bytes(&self.f.bytes_at(addr, EXCEPTION_RECORD_SIZE)).unwrap()
    }
}

fn ctx(rip: u64, rsp: u64) -> Context {
    // IF, TF and DF set.
    let mut c = Context { context_flags: CONTEXT_FULL, rip, eflags: 0x202 | 0x100 | 0x400, ..Context::default() };
    c.set_rsp(rsp);
    for (i, r) in c.gpr.iter_mut().enumerate() {
        if i != usize::from(RSP) {
            *r = 0x1000 + i as u64;
        }
    }
    c
}

fn fault(vector: u8, error_code: u64, code: &[u8]) -> CpuFault<'_> {
    CpuFault { vector, error_code, address: None, page: None, code }
}

fn classify_one(f: &CpuFault<'_>, c: &mut Context) -> (u32, Vec<u64>, bool) {
    let r = classify(f, c);
    (r.record.code, r.record.parameters().to_vec(), r.fail_fast)
}

#[test]
fn cpu_exceptions_raise_their_documented_codes() {
    let rip = IMAGE_BASE + 0x1234;
    let mut c = ctx(rip, 0x5_0000);
    let cases: &[(u8, u64, &[u8], u32)] = &[
        (VECTOR_DB, 0, &[], STATUS_SINGLE_STEP),
        (VECTOR_OF, 0, &[], STATUS_INTEGER_OVERFLOW),
        (VECTOR_BR, 0, &[], STATUS_ARRAY_BOUNDS_EXCEEDED),
        (VECTOR_UD, 0, &[0x0F, 0x0B], STATUS_ILLEGAL_INSTRUCTION),
        (VECTOR_NM, 0, &[], STATUS_ILLEGAL_INSTRUCTION),
        (VECTOR_AC, 0, &[], STATUS_DATATYPE_MISALIGNMENT),
        // Privileged instructions: cli, hlt, in al, 0x60, wrmsr, mov cr0,
        // xsetbv, lgdt [rax], invlpg [rax], swapgs, with prefixes too.
        (VECTOR_GP, 0, &[0xFA], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0xF4], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0xE4, 0x60], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x66, 0xEF], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x0F, 0x30], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x0F, 0x22, 0xC0], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x0F, 0x01, 0xD1], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x0F, 0x01, 0x10], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x0F, 0x01, 0x38], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x0F, 0x01, 0xF8], STATUS_PRIVILEGED_INSTRUCTION),
        (VECTOR_GP, 0, &[0x48, 0x0F, 0x00, 0xD8], STATUS_PRIVILEGED_INSTRUCTION),
        // Not privileged: xgetbv, a plain load from a non-canonical address.
        (VECTOR_GP, 0, &[0x0F, 0x01, 0xD0], STATUS_ACCESS_VIOLATION),
        (VECTOR_GP, 0, &[0x48, 0x8B, 0x00], STATUS_ACCESS_VIOLATION),
        (VECTOR_GP, 0, &[], STATUS_ACCESS_VIOLATION),
        (VECTOR_SS, 0, &[], STATUS_ACCESS_VIOLATION),
        (VECTOR_NP, 0x10, &[], STATUS_ACCESS_VIOLATION),
        // `int 0x80` through a gate ring 3 may not use.
        (VECTOR_GP, 0x80 << 3 | 2, &[0xCD, 0x80], STATUS_ACCESS_VIOLATION),
        (30, 0, &[], STATUS_ACCESS_VIOLATION),
    ];
    for &(vector, err, code, want) in cases {
        let (got, params, ff) = classify_one(&fault(vector, err, code), &mut c);
        assert_eq!(got, want, "vector {vector}, code {code:x?}");
        assert!(!ff);
        if want == STATUS_ACCESS_VIOLATION {
            assert_eq!(params, vec![AV_READ, AV_UNKNOWN_ADDRESS]);
        } else {
            assert!(params.is_empty());
        }
        assert_eq!(c.rip, rip, "only a breakpoint moves RIP");
    }
}

#[test]
fn divide_errors_tell_zero_from_overflow() {
    let mut c = ctx(IMAGE_BASE + 0x10, 0x5_0000);
    let mut div = |code: &[u8], rcx: u64, rax: u64| {
        c.gpr[usize::from(RCX)] = rcx;
        c.gpr[usize::from(RAX)] = rax;
        classify_one(&fault(VECTOR_DE, 0, code), &mut c).0
    };
    // div ecx / idiv ecx
    assert_eq!(div(&[0xF7, 0xF1], 0, 0), STATUS_INTEGER_DIVIDE_BY_ZERO);
    assert_eq!(div(&[0xF7, 0xF9], 5, 0), STATUS_INTEGER_OVERFLOW);
    // 32-bit operand: only the low half of RCX counts; REX.W: all of it.
    assert_eq!(div(&[0xF7, 0xF1], 0x1_0000_0000, 0), STATUS_INTEGER_DIVIDE_BY_ZERO);
    assert_eq!(div(&[0x48, 0xF7, 0xF1], 0x1_0000_0000, 0), STATUS_INTEGER_OVERFLOW);
    // 16-bit operand.
    assert_eq!(div(&[0x66, 0xF7, 0xF1], 0x1_0000, 0), STATUS_INTEGER_DIVIDE_BY_ZERO);
    // div ah (no REX): the divisor is AH, bits 8-15 of RAX.
    assert_eq!(div(&[0xF6, 0xF4], 0, 0x0000), STATUS_INTEGER_DIVIDE_BY_ZERO);
    assert_eq!(div(&[0xF6, 0xF4], 0, 0x0300), STATUS_INTEGER_OVERFLOW);
    // A memory divisor is not decoded: divide by zero.
    assert_eq!(div(&[0xF7, 0x30], 7, 0), STATUS_INTEGER_DIVIDE_BY_ZERO);
    assert_eq!(div(&[], 7, 0), STATUS_INTEGER_DIVIDE_BY_ZERO);
}

#[test]
fn breakpoints_report_the_int3() {
    let mut c = ctx(IMAGE_BASE + 0x101, 0x5_0000);
    let r = classify(&fault(VECTOR_BP, 0, &[]), &mut c);
    assert_eq!(r.record.code, STATUS_BREAKPOINT);
    assert_eq!(r.record.address, IMAGE_BASE + 0x100);
    assert_eq!(c.rip, IMAGE_BASE + 0x100);
}

#[test]
fn page_faults_report_access_and_address() {
    let mut c = ctx(IMAGE_BASE + 0x10, 0x5_0000);
    let pf = |err: u64, page: Option<u32>| CpuFault {
        vector: VECTOR_PF,
        error_code: err,
        address: Some(0xDEAD_0000),
        page: page.map(NtStatus),
        code: &[],
    };
    let cases = [
        (0b00100, None, STATUS_ACCESS_VIOLATION, vec![AV_READ, 0xDEAD_0000]),
        (0b00110, None, STATUS_ACCESS_VIOLATION, vec![AV_WRITE, 0xDEAD_0000]),
        (0b10101, None, STATUS_ACCESS_VIOLATION, vec![AV_EXECUTE, 0xDEAD_0000]),
        (0b00110, Some(STATUS_ACCESS_VIOLATION), STATUS_ACCESS_VIOLATION, vec![AV_WRITE, 0xDEAD_0000]),
        (0b00110, Some(STATUS_GUARD_PAGE_VIOLATION), STATUS_GUARD_PAGE_VIOLATION, vec![AV_WRITE, 0xDEAD_0000]),
        (0b00110, Some(STATUS_STACK_OVERFLOW), STATUS_STACK_OVERFLOW, vec![AV_WRITE, 0xDEAD_0000]),
        (
            0b00100,
            Some(STATUS_IN_PAGE_ERROR),
            STATUS_IN_PAGE_ERROR,
            vec![AV_READ, 0xDEAD_0000, u64::from(STATUS_END_OF_FILE)],
        ),
    ];
    for (err, page, code, params) in cases {
        let (got, p, _) = classify_one(&pf(err, page), &mut c);
        assert_eq!((got, p), (code, params), "error {err:#b}, page {page:x?}");
    }
}

#[test]
fn floating_point_exceptions_follow_the_unmasked_flags() {
    let mut c = ctx(IMAGE_BASE + 0x10, 0x5_0000);
    // SSE: everything masked but divide-by-zero, which is pending.
    c.mx_csr = (0x1F80 & !(1 << 9)) | (1 << 2);
    assert_eq!(classify_one(&fault(VECTOR_XM, 0, &[]), &mut c).0, STATUS_FLOAT_DIVIDE_BY_ZERO);
    c.mx_csr = (0x1F80 & !(1 << 7)) | 1;
    assert_eq!(classify_one(&fault(VECTOR_XM, 0, &[]), &mut c).0, STATUS_FLOAT_INVALID_OPERATION);
    c.mx_csr = (0x1F80 & !(1 << 10)) | (1 << 3);
    assert_eq!(classify_one(&fault(VECTOR_XM, 0, &[]), &mut c).0, STATUS_FLOAT_OVERFLOW);
    // x87: FCW masks at bits 0-5, FSW flags; a stack fault is a stack check.
    let set_x87 = |c: &mut Context, fcw: u16, fsw: u16| {
        c.flt_save[0..2].copy_from_slice(&fcw.to_le_bytes());
        c.flt_save[2..4].copy_from_slice(&fsw.to_le_bytes());
    };
    set_x87(&mut c, 0x037F & !0x10, 0x10);
    assert_eq!(classify_one(&fault(VECTOR_MF, 0, &[]), &mut c).0, STATUS_FLOAT_UNDERFLOW);
    set_x87(&mut c, 0x037F & !0x01, 0x41);
    assert_eq!(classify_one(&fault(VECTOR_MF, 0, &[]), &mut c).0, STATUS_FLOAT_STACK_CHECK);
    set_x87(&mut c, 0x037F & !0x20, 0x20);
    assert_eq!(classify_one(&fault(VECTOR_MF, 0, &[]), &mut c).0, STATUS_FLOAT_INEXACT_RESULT);
    set_x87(&mut c, 0x037F & !0x02, 0x02);
    assert_eq!(classify_one(&fault(VECTOR_MF, 0, &[]), &mut c).0, STATUS_FLOAT_DENORMAL_OPERAND);
}

#[test]
fn fast_fail_is_noncontinuable_and_never_dispatched() {
    let mut k = SehFake::new(DISPATCHER);
    let top = k.stack_top();
    let mut c = ctx(IMAGE_BASE + 0x40, top - 0x100);
    c.gpr[usize::from(RCX)] = 7; // FAST_FAIL_FATAL_APP_EXIT-style code
    let f = fault(VECTOR_GP, FAST_FAIL_VECTOR << 3 | 2, &[0xCD, 0x29]);
    let r = classify(&f, &mut c.clone());
    assert!(r.fail_fast);
    assert_eq!(r.record.code, STATUS_STACK_BUFFER_OVERRUN);
    assert_eq!(r.record.flags, EXCEPTION_NONCONTINUABLE);
    assert_eq!(r.record.parameters(), &[7]);
    // Through #NP too (no gate at all).
    assert!(classify(&fault(VECTOR_NP, FAST_FAIL_VECTOR << 3 | 2, &[]), &mut c.clone()).fail_fast);
    match dispatch_fault(&mut k, &f, &mut c) {
        FaultOutcome::Unhandled(rec) => assert_eq!(rec.code, STATUS_STACK_BUFFER_OVERRUN),
        o => panic!("{o:?}"),
    }
    assert!(k.loaded.is_empty());
    assert_eq!(k.f.terminated, Some(NtStatus(STATUS_STACK_BUFFER_OVERRUN)));
}

#[test]
fn the_dispatcher_frame_layout() {
    let f = frame_below(0x5_0000).unwrap();
    assert_eq!(f.context, 0x5_0000 - 0x4D0);
    assert_eq!(f.context % 16, 0);
    assert_eq!(f.record, f.context - 0xA0);
    assert_eq!(f.record % 16, 0);
    assert_eq!(f.rsp, f.record - 0x28);
    // At entry RSP is 8 below a 16-byte boundary, as after a call.
    assert_eq!(f.rsp % 16, 8);
    assert!(0x5_0000 - f.rsp <= FRAME_SIZE);
    // A misaligned RSP is aligned down.
    let g = frame_below(0x5_0007).unwrap();
    assert_eq!(g.context % 16, 0);
    assert!(g.context + CONTEXT_SIZE as u64 <= 0x5_0007);
    // Too low, or not a user address.
    assert_eq!(frame_below(0x1_0400), None);
    assert_eq!(frame_below(0x100), None);
    assert_eq!(frame_below(u64::MAX), None);
    assert_eq!(frame_below(crate::vm::ADDRESS_END + 8), None);
}

#[test]
fn a_fault_is_delivered_to_the_dispatcher() {
    let mut k = SehFake::new(DISPATCHER);
    let top = k.stack_top();
    let rsp = top - 0x108;
    let mut c = ctx(IMAGE_BASE + 0x40, rsp);
    let f = CpuFault {
        vector: VECTOR_PF,
        error_code: 0b110,
        address: Some(0x10),
        page: Some(NtStatus(STATUS_ACCESS_VIOLATION)),
        code: &[0x89, 0x00],
    };
    let rec = match dispatch_fault(&mut k, &f, &mut c) {
        FaultOutcome::Dispatched(r) => r,
        o => panic!("{o:?}"),
    };
    assert_eq!(rec.code, STATUS_ACCESS_VIOLATION);
    assert_eq!(rec.address, IMAGE_BASE + 0x40);
    assert_eq!(rec.parameters(), &[AV_WRITE, 0x10]);
    // The kernel was asked to enter the dispatcher.
    let (entry, flags) = k.loaded.pop().expect("registers loaded");
    assert_eq!(flags, CONTEXT_CONTROL | CONTEXT_INTEGER);
    let frame = frame_below(rsp).unwrap();
    assert_eq!(entry.rip, DISPATCHER);
    assert_eq!(entry.rsp(), frame.rsp);
    assert_eq!(entry.gpr[usize::from(RCX)], frame.record);
    assert_eq!(entry.gpr[usize::from(RDX)], frame.context);
    assert_eq!(entry.eflags, 0x202, "TF and DF cleared");
    assert_eq!(entry.gpr[usize::from(RBX)], 0x1003, "other registers as at the fault");
    // The frame on the stack: the context and record, a zero return address.
    assert_eq!(k.context_at(frame.context), c);
    assert_eq!(k.record_at(frame.record), rec);
    assert_eq!(k.f.u64_at(frame.rsp), 0);
    assert!(k.unhandled.is_none());
    assert_eq!(k.f.terminated, None);
}

#[test]
fn undeliverable_exceptions_end_the_process_with_their_code() {
    // No dispatcher.
    let mut k = SehFake::new(0);
    let top = k.stack_top();
    let mut c = ctx(IMAGE_BASE + 0x40, top - 0x100);
    let f = fault(VECTOR_DE, 0, &[0xF7, 0xF1]);
    c.gpr[usize::from(RCX)] = 0;
    assert!(matches!(dispatch_fault(&mut k, &f, &mut c), FaultOutcome::Unhandled(_)));
    assert_eq!(k.f.terminated, Some(NtStatus(STATUS_INTEGER_DIVIDE_BY_ZERO)));
    assert_eq!(k.unhandled.as_ref().map(|u| u.1.rip), Some(IMAGE_BASE + 0x40));
    // A stack the frame cannot be written to.
    let mut k = SehFake::new(DISPATCHER);
    let mut c = ctx(IMAGE_BASE + 0x40, 0x7000_0000);
    c.gpr[usize::from(RCX)] = 0;
    assert!(matches!(dispatch_fault(&mut k, &f, &mut c), FaultOutcome::Unhandled(_)));
    assert!(k.loaded.is_empty());
    assert_eq!(k.f.terminated, Some(NtStatus(STATUS_INTEGER_DIVIDE_BY_ZERO)));
}

#[test]
fn raise_exception_first_and_second_chance() {
    let mut k = SehFake::new(DISPATCHER);
    let top = k.stack_top();
    let raised = ExceptionRecord::new(0xE000_1234, 0, IMAGE_BASE + 0x80, &[1, 2, 3]);
    let rec_ptr = k.f.put(&raised.to_bytes());
    let c = ctx(IMAGE_BASE + 0x80, top - 0x200);
    let ctx_ptr = k.f.put(&c.to_bytes());

    // First chance: delivered with the given record and context.
    assert_eq!(k.call(Service::RaiseException, &[rec_ptr, ctx_ptr, 1]), STATUS_SUCCESS);
    let (entry, _) = k.loaded.pop().expect("dispatched");
    let frame = frame_below(top - 0x200).unwrap();
    assert_eq!((entry.rip, entry.rsp()), (DISPATCHER, frame.rsp));
    assert_eq!(k.record_at(frame.record), raised);
    assert_eq!(k.context_at(frame.context), c);
    assert!(k.unhandled.is_none());

    // Second chance: the process ends with the code.
    assert_eq!(k.call(Service::RaiseException, &[rec_ptr, ctx_ptr, 0]), STATUS_SUCCESS);
    assert!(k.loaded.is_empty());
    assert_eq!(k.unhandled.as_ref().map(|u| u.0), Some(raised));
    assert_eq!(k.f.terminated, Some(NtStatus(0xE000_1234)));

    // First chance without a dispatcher: unhandled too.
    let mut k = SehFake::new(0);
    let rec_ptr = k.f.put(&raised.to_bytes());
    let ctx_ptr = k.f.put(&c.to_bytes());
    assert_eq!(k.call(Service::RaiseException, &[rec_ptr, ctx_ptr, 1]), STATUS_SUCCESS);
    assert_eq!(k.f.terminated, Some(NtStatus(0xE000_1234)));

    // Bad arguments.
    let mut k = SehFake::new(DISPATCHER);
    let mut bad = raised.to_bytes();
    bad[24] = 16; // NumberParameters > EXCEPTION_MAXIMUM_PARAMETERS
    let bad_ptr = k.f.put(&bad);
    let ctx_ptr = k.f.put(&c.to_bytes());
    assert_eq!(k.call(Service::RaiseException, &[bad_ptr, ctx_ptr, 1]), STATUS_INVALID_PARAMETER);
    assert_eq!(k.call(Service::RaiseException, &[0, ctx_ptr, 1]), AV_STATUS);
    let rec_ptr = k.f.put(&raised.to_bytes());
    assert_eq!(k.call(Service::RaiseException, &[rec_ptr, 0, 1]), AV_STATUS);
    assert!(k.loaded.is_empty() && k.unhandled.is_none());
}

#[test]
fn continue_loads_the_context_groups_it_names() {
    let mut k = SehFake::new(DISPATCHER);
    let top = k.stack_top();
    let mut c = ctx(IMAGE_BASE + 0x99, top - 0x80);
    c.gpr[usize::from(RAX)] = 0xFFFF_FFFF_0000_0001; // all 64 bits matter
    c.context_flags = CONTEXT_CONTROL | CONTEXT_INTEGER | CONTEXT_FLOATING_POINT;
    let p = k.f.put(&c.to_bytes());
    assert_eq!(k.call(Service::Continue, &[p, 0]), STATUS_SUCCESS);
    let (loaded, flags) = k.loaded.pop().unwrap();
    assert_eq!(loaded, c);
    assert_eq!(flags, c.context_flags);
    assert_eq!(k.test_alerts, 0);
    // TestAlert delivers APCs after the context is loaded.
    assert_eq!(k.call(Service::Continue, &[p, 1]), STATUS_SUCCESS);
    assert_eq!((k.loaded.len(), k.test_alerts), (1, 1));
    // A RIP or RSP that is not a user address, when the control group is
    // loaded, is refused.
    let mut bad = c.clone();
    bad.rip = 0xFFFF_8000_0000_0000;
    let p = k.f.put(&bad.to_bytes());
    assert_eq!(k.call(Service::Continue, &[p, 0]), STATUS_INVALID_PARAMETER);
    bad.context_flags = CONTEXT_INTEGER; // ... but only then
    let p = k.f.put(&bad.to_bytes());
    assert_eq!(k.call(Service::Continue, &[p, 0]), STATUS_SUCCESS);
    assert_eq!(k.call(Service::Continue, &[0, 0]), AV_STATUS);
}

#[test]
fn dispatch_fix_up_and_continue_round_trip() {
    // The whole protocol on the host: a fault reaches the dispatcher, which
    // reads the context from its stack, skips the faulting instruction and
    // continues.
    let mut k = SehFake::new(DISPATCHER);
    let top = k.stack_top();
    let mut c = ctx(IMAGE_BASE + 0x40, top - 0x100);
    c.context_flags = CONTEXT_FULL;
    let f = fault(VECTOR_UD, 0, &[0x0F, 0x0B]);
    assert!(matches!(dispatch_fault(&mut k, &f, &mut c), FaultOutcome::Dispatched(_)));
    let (entry, _) = k.loaded.pop().unwrap();
    let ctx_addr = entry.gpr[usize::from(RDX)];
    let rec = k.record_at(entry.gpr[usize::from(RCX)]);
    assert_eq!(rec.code, STATUS_ILLEGAL_INSTRUCTION);
    let mut seen = k.context_at(ctx_addr);
    seen.rip += 2; // past the ud2
    k.f.mem.write_bytes(ctx_addr, &seen.to_bytes()).unwrap();
    assert_eq!(k.call(Service::Continue, &[ctx_addr, 0]), STATUS_SUCCESS);
    let (resumed, _) = k.loaded.pop().unwrap();
    assert_eq!(resumed.rip, IMAGE_BASE + 0x42);
    assert_eq!(resumed.rsp(), top - 0x100);
    assert_eq!(resumed.gpr, c.gpr);
}

#[test]
fn image_module_reads_the_exception_directory() {
    let mut file = test_exe(true);
    assert_eq!(image_module(IMAGE_BASE, &file), None, "no exception directory");
    crate::pe_writer::set_directory(&mut file, poly_pe::dir::EXCEPTION, 0x2000, 36);
    let m = image_module(IMAGE_BASE, &file).unwrap();
    assert_eq!((m.base, m.table, m.count), (IMAGE_BASE, IMAGE_BASE + 0x2000, 3));
    assert!(m.contains(IMAGE_BASE + 0x1000) && !m.contains(IMAGE_BASE + m.size));
    // A directory past the image, truncated headers, garbage.
    crate::pe_writer::set_directory(&mut file, poly_pe::dir::EXCEPTION, 0xFFFF_0000, 36);
    assert_eq!(image_module(IMAGE_BASE, &file), None);
    assert_eq!(image_module(IMAGE_BASE, &file[..0x60]), None);
    assert_eq!(image_module(IMAGE_BASE, &[0xFF; 0x200]), None);
    assert_eq!(image_module(IMAGE_BASE, &[]), None);
}
