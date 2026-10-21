//! Kernel-to-user exception dispatch (ADR 0068): what a CPU exception in an
//! NT thread means, how it reaches the process's user-mode exception
//! dispatcher, and the services around it (`NtRaiseException`,
//! `NtContinue`, `PolySetExceptionDispatcher`).
//!
//! On Windows the kernel hands an exception to `ntdll`'s
//! `KiUserExceptionDispatcher`, which walks the stack (`RtlDispatchException`)
//! and either resumes a context (`NtContinue`) or raises the exception again
//! as unhandled (`NtRaiseException` with `FirstChance` false), and an
//! unhandled exception ends the process with the exception code as its
//! exit status. Until our `ntdll` exists, a process names its dispatcher
//! itself with Poly's own service `PolySetExceptionDispatcher`; once the
//! kernel maps our `ntdll`, it will take the address of its
//! `KiUserExceptionDispatcher` export instead.
//!
//! # What the dispatcher is entered with (Poly's convention, ADR 0068)
//!
//! The kernel copies the `CONTEXT` and the `EXCEPTION_RECORD` to the
//! thread's stack below its RSP and enters the dispatcher as if it had been
//! called as `dispatcher(EXCEPTION_RECORD *record, CONTEXT *context)`:
//!
//! ```text
//!   context.Rsp ->  (the interrupted stack)
//!                   CONTEXT            0x4D0 bytes, 16-byte aligned
//!                   EXCEPTION_RECORD   0x98 bytes in a 0xA0 slot, 16-byte aligned
//!                   shadow space       0x20 bytes
//!   RSP ->          return address     0 (the dispatcher never returns)
//!   RCX = &EXCEPTION_RECORD, RDX = &CONTEXT, RIP = the dispatcher,
//!   RFLAGS without TF and DF; the other registers as in the CONTEXT.
//! ```
//!
//! The dispatcher ends with `NtContinue(context, FALSE)` to resume a
//! (perhaps changed) context, or `NtRaiseException(record, context, FALSE)`
//! to give up: the process then ends with the exception code. A process
//! with no dispatcher, a stack the frame cannot be written to, and a
//! `__fastfail` (`int 0x29`, Learn: "__fastfail": "a fail fast request ...
//! cannot be intercepted") end the process the same way at once.

use crate::abi;
use crate::kernel::NtKernel;
use crate::objects::Result;
use crate::vm::ADDRESS_END;
use poly_abi::SyscallRegs;
use poly_ntobj::NtStatus;
use poly_ntobj::status::{STATUS_INVALID_PARAMETER, STATUS_SUCCESS};
use poly_seh::consts::*;
use poly_seh::context::{CONTEXT_ALIGN, CONTEXT_CONTROL, CONTEXT_INTEGER, CONTEXT_SIZE};
use poly_seh::record::EXCEPTION_RECORD_SIZE;
use poly_seh::{Context, ExceptionRecord};

// ----- CPU exception vectors (Intel SDM Vol. 3A, Table 6-1)

pub const VECTOR_DE: u8 = 0;
pub const VECTOR_DB: u8 = 1;
pub const VECTOR_BP: u8 = 3;
pub const VECTOR_OF: u8 = 4;
pub const VECTOR_BR: u8 = 5;
pub const VECTOR_UD: u8 = 6;
pub const VECTOR_NM: u8 = 7;
pub const VECTOR_NP: u8 = 11;
pub const VECTOR_SS: u8 = 12;
pub const VECTOR_GP: u8 = 13;
pub const VECTOR_PF: u8 = 14;
pub const VECTOR_MF: u8 = 16;
pub const VECTOR_AC: u8 = 17;
pub const VECTOR_XM: u8 = 19;

/// The `int` vector `__fastfail` uses on x86 and x64 (Learn: "__fastfail").
pub const FAST_FAIL_VECTOR: u64 = 0x29;

/// `STATUS_END_OF_FILE` (`ntstatus.h`): the underlying status Poly reports
/// in an in-page error (a page of a file view past the file's end).
pub const STATUS_END_OF_FILE: u32 = 0xC000_0011;

/// Access-violation `ExceptionInformation[0]` values (Learn:
/// "EXCEPTION_RECORD": 0 read, 1 write, 8 a DEP violation).
pub const AV_READ: u64 = 0;
pub const AV_WRITE: u64 = 1;
pub const AV_EXECUTE: u64 = 8;

/// The address an access violation without one reports (a general
/// protection fault): to be confirmed by the spec agent.
pub const AV_UNKNOWN_ADDRESS: u64 = u64::MAX;

/// Page-fault error code bits (Intel SDM Vol. 3A §4.7).
const PF_WRITE: u64 = 1 << 1;
const PF_FETCH: u64 = 1 << 4;
/// Selector error code bit "IDT" (SDM Vol. 3A §6.13): the error names an
/// IDT gate, `int n` from ring 3 through a gate it may not use.
const ERR_IDT: u64 = 1 << 1;

/// RFLAGS.TF and RFLAGS.DF, cleared for the dispatcher (the x64 calling
/// convention requires DF clear on function entry; Learn: "x64 calling
/// convention").
const RFLAGS_TF: u32 = 1 << 8;
const RFLAGS_DF: u32 = 1 << 10;

/// A CPU exception in ring 3 that demand paging did not resolve, as the
/// kernel saw it.
#[derive(Clone, Copy, Debug)]
pub struct CpuFault<'a> {
    pub vector: u8,
    pub error_code: u64,
    /// CR2, for a page fault.
    pub address: Option<u64>,
    /// For a page fault: the exception the memory model chose
    /// (`STATUS_ACCESS_VIOLATION`, `STATUS_GUARD_PAGE_VIOLATION`,
    /// `STATUS_STACK_OVERFLOW`), or `STATUS_IN_PAGE_ERROR` when the page
    /// could not be read in.
    pub page: Option<NtStatus>,
    /// The instruction bytes at the faulting RIP, as many as could be read.
    pub code: &'a [u8],
}

/// What a CPU exception raises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Classified {
    pub record: ExceptionRecord,
    /// A fail-fast request: the process ends without any handler running.
    pub fail_fast: bool,
}

/// The instruction at `code` after its legacy and REX prefixes: (REX, the
/// operand-size prefix seen, the opcode bytes).
fn strip_prefixes(code: &[u8]) -> (u8, bool, &[u8]) {
    let mut i = 0;
    let mut opsize = false;
    while let Some(&b) = code.get(i) {
        match b {
            0x66 => opsize = true,
            0x67 | 0xF0 | 0xF2 | 0xF3 | 0x2E | 0x36 | 0x3E | 0x26 | 0x64 | 0x65 => {}
            _ => break,
        }
        i += 1;
    }
    let rex = match code.get(i) {
        Some(&b @ 0x40..=0x4F) => {
            i += 1;
            b
        }
        _ => 0,
    };
    (rex, opsize, code.get(i..).unwrap_or(&[]))
}

/// Whether the instruction at `code` is one ring 3 may not execute (Intel
/// SDM: the instructions that raise #GP at CPL > 0).
fn privileged(code: &[u8]) -> bool {
    let (_, _, op) = strip_prefixes(code);
    let modrm = |i: usize| op.get(i).map(|m| (m >> 6, (m >> 3) & 7));
    match op {
        // hlt, cli, sti, in, out, ins, outs
        [0xF4, ..] | [0xFA, ..] | [0xFB, ..] => true,
        [0xE4..=0xE7, ..] | [0xEC..=0xEF, ..] | [0x6C..=0x6F, ..] => true,
        // clts, sysret, invd, wbinvd, mov to/from CR and DR, wrmsr, rdmsr,
        // rdpmc, sysexit
        [0x0F, 0x06 | 0x07 | 0x08 | 0x09 | 0x20..=0x23 | 0x30 | 0x32 | 0x33 | 0x35, ..] => true,
        // lldt, ltr
        [0x0F, 0x00, ..] => matches!(modrm(2), Some((_, 2 | 3))),
        // swapgs, xsetbv; lgdt, lidt (memory forms), lmsw, invlpg
        [0x0F, 0x01, 0xF8, ..] | [0x0F, 0x01, 0xD1, ..] => true,
        [0x0F, 0x01, ..] => match modrm(2) {
            Some((m, 2 | 3 | 7)) => m != 3,
            Some((_, 6)) => true,
            _ => false,
        },
        _ => false,
    }
}

/// For a #DE: whether the `div`/`idiv` at `code` divided by a register
/// that is not zero (a quotient overflow, `STATUS_INTEGER_OVERFLOW`). A
/// memory divisor is not decoded (`None`).
fn divide_overflowed(code: &[u8], ctx: &Context) -> Option<bool> {
    let (rex, opsize, op) = strip_prefixes(code);
    let (&opcode, &modrm) = (op.first()?, op.get(1)?);
    if !(opcode == 0xF6 || opcode == 0xF7) || !matches!((modrm >> 3) & 7, 6 | 7) || modrm >> 6 != 3 {
        return None;
    }
    let rm = usize::from(modrm & 7);
    let reg = rm | usize::from(rex & 1) << 3;
    let value = if opcode == 0xF6 {
        // r/m8: without REX, 4-7 are AH, CH, DH, BH.
        if rex == 0 && rm >= 4 { (ctx.gpr[rm - 4] >> 8) & 0xFF } else { ctx.gpr[reg] & 0xFF }
    } else if rex & 8 != 0 {
        ctx.gpr[reg]
    } else if opsize {
        ctx.gpr[reg] & 0xFFFF
    } else {
        ctx.gpr[reg] & 0xFFFF_FFFF
    };
    Some(value != 0)
}

/// The floating-point exception the unmasked, pending flags in `status`
/// (masks in `masks`, the flags' order: invalid, denormal, divide by zero,
/// overflow, underflow, precision) raise.
fn float_status(status: u32, masks: u32, stack_fault: bool) -> u32 {
    let pending = status & !masks & 0x3F;
    match pending {
        p if p & 0x01 != 0 && stack_fault => STATUS_FLOAT_STACK_CHECK,
        p if p & 0x01 != 0 => STATUS_FLOAT_INVALID_OPERATION,
        p if p & 0x04 != 0 => STATUS_FLOAT_DIVIDE_BY_ZERO,
        p if p & 0x02 != 0 => STATUS_FLOAT_DENORMAL_OPERAND,
        p if p & 0x08 != 0 => STATUS_FLOAT_OVERFLOW,
        p if p & 0x10 != 0 => STATUS_FLOAT_UNDERFLOW,
        p if p & 0x20 != 0 => STATUS_FLOAT_INEXACT_RESULT,
        _ => STATUS_FLOAT_INVALID_OPERATION,
    }
}

/// The exception a CPU exception raises, and the context it is reported
/// with (`ctx`, the thread's registers at the fault; for a breakpoint its
/// RIP moves back onto the `int3`). Mapping from Learn's
/// `EXCEPTION_RECORD` codes and the Intel SDM's exception list; the
/// choices docs/adr/0068 lists are to be confirmed by the spec agent.
pub fn classify(fault: &CpuFault<'_>, ctx: &mut Context) -> Classified {
    let rip = ctx.rip;
    let record = |code: u32, params: &[u64]| ExceptionRecord::new(code, 0, rip, params);
    let unknown_av = || record(STATUS_ACCESS_VIOLATION, &[AV_READ, AV_UNKNOWN_ADDRESS]);
    let rec = match fault.vector {
        VECTOR_DE => match divide_overflowed(fault.code, ctx) {
            Some(true) => record(STATUS_INTEGER_OVERFLOW, &[]),
            _ => record(STATUS_INTEGER_DIVIDE_BY_ZERO, &[]),
        },
        VECTOR_DB => record(STATUS_SINGLE_STEP, &[]),
        VECTOR_BP => {
            // A trap: RIP is past the one-byte int3. Report the int3.
            ctx.rip = rip.wrapping_sub(1);
            ExceptionRecord::new(STATUS_BREAKPOINT, 0, ctx.rip, &[])
        }
        VECTOR_OF => record(STATUS_INTEGER_OVERFLOW, &[]),
        VECTOR_BR => record(STATUS_ARRAY_BOUNDS_EXCEEDED, &[]),
        VECTOR_UD | VECTOR_NM => record(STATUS_ILLEGAL_INSTRUCTION, &[]),
        VECTOR_GP | VECTOR_NP if fault.error_code & ERR_IDT != 0 => {
            if fault.error_code >> 3 == FAST_FAIL_VECTOR {
                // The fail-fast code is in RCX (Learn: "__fastfail").
                let r = ExceptionRecord::new(
                    STATUS_STACK_BUFFER_OVERRUN,
                    EXCEPTION_NONCONTINUABLE,
                    rip,
                    &[ctx.gpr[usize::from(RCX)]],
                );
                return Classified { record: r, fail_fast: true };
            }
            unknown_av()
        }
        VECTOR_GP if privileged(fault.code) => record(STATUS_PRIVILEGED_INSTRUCTION, &[]),
        VECTOR_GP | VECTOR_NP | VECTOR_SS => unknown_av(),
        VECTOR_PF => {
            let rw = if fault.error_code & PF_FETCH != 0 {
                AV_EXECUTE
            } else if fault.error_code & PF_WRITE != 0 {
                AV_WRITE
            } else {
                AV_READ
            };
            let addr = fault.address.unwrap_or(AV_UNKNOWN_ADDRESS);
            match fault.page.map(|s| s.0) {
                Some(STATUS_IN_PAGE_ERROR) => record(STATUS_IN_PAGE_ERROR, &[rw, addr, u64::from(STATUS_END_OF_FILE)]),
                Some(code @ (STATUS_GUARD_PAGE_VIOLATION | STATUS_STACK_OVERFLOW)) => record(code, &[rw, addr]),
                _ => record(STATUS_ACCESS_VIOLATION, &[rw, addr]),
            }
        }
        VECTOR_MF => {
            let fcw = u32::from(u16::from_le_bytes([ctx.flt_save[0], ctx.flt_save[1]]));
            let fsw = u32::from(u16::from_le_bytes([ctx.flt_save[2], ctx.flt_save[3]]));
            record(float_status(fsw, fcw, fsw & (1 << 6) != 0), &[])
        }
        VECTOR_XM => {
            let mxcsr = ctx.mx_csr;
            record(float_status(mxcsr, mxcsr >> 7, false), &[])
        }
        VECTOR_AC => record(STATUS_DATATYPE_MISALIGNMENT, &[]),
        _ => unknown_av(),
    };
    Classified { record: rec, fail_fast: false }
}

/// Where the dispatcher frame goes on the user stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The dispatcher's RSP (the fake return address).
    pub rsp: u64,
    pub record: u64,
    pub context: u64,
}

/// The slot the record takes: `EXCEPTION_RECORD` rounded up to 16 bytes.
pub const RECORD_SLOT: u64 = (EXCEPTION_RECORD_SIZE as u64).div_ceil(16) * 16;
/// The return-address slot and the 32-byte shadow space.
pub const CALL_SLOT: u64 = 0x28;
/// The whole frame's size at most (with alignment).
pub const FRAME_SIZE: u64 = CONTEXT_SIZE as u64 + 15 + RECORD_SLOT + CALL_SLOT;

/// Lay the dispatcher frame out below `rsp` (the interrupted stack
/// pointer). `None` if it would leave the user address space.
pub fn frame_below(rsp: u64) -> Option<Frame> {
    if rsp > ADDRESS_END {
        return None;
    }
    let context = rsp.checked_sub(CONTEXT_SIZE as u64)? & !(CONTEXT_ALIGN - 1);
    let record = context.checked_sub(RECORD_SLOT)?;
    let sp = record.checked_sub(CALL_SLOT)?;
    // Nothing of it in the first 64 KiB (never mapped).
    (sp >= 0x1_0000).then_some(Frame { rsp: sp, record, context })
}

/// The registers the dispatcher starts with (see the module docs).
pub fn dispatcher_entry(ctx: &Context, frame: &Frame, dispatcher: u64) -> Context {
    let mut entry = ctx.clone();
    entry.rip = dispatcher;
    entry.set_rsp(frame.rsp);
    entry.gpr[usize::from(RCX)] = frame.record;
    entry.gpr[usize::from(RDX)] = frame.context;
    entry.eflags &= !(RFLAGS_TF | RFLAGS_DF);
    entry
}

/// Why an exception could not reach the dispatcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undeliverable {
    /// The process has not named a dispatcher.
    NoDispatcher,
    /// The frame does not fit, or the stack cannot be written.
    BadStack,
    /// The kernel could not load the dispatcher's registers.
    NoContext(NtStatus),
}

/// Hand `record`, raised in context `ctx`, to the calling thread's process's
/// user-mode dispatcher: write the frame to its stack and make the thread
/// resume in the dispatcher.
pub fn deliver<K: NtKernel>(
    k: &mut K,
    record: &ExceptionRecord,
    ctx: &Context,
) -> core::result::Result<(), Undeliverable> {
    let dispatcher = k.with_state(|_, proc, _| proc.exception_dispatcher);
    if dispatcher == 0 {
        return Err(Undeliverable::NoDispatcher);
    }
    let frame = frame_below(ctx.rsp()).ok_or(Undeliverable::BadStack)?;
    let writes: [(u64, &[u8]); 3] =
        [(frame.context, &ctx.to_bytes()), (frame.record, &record.to_bytes()), (frame.rsp, &[0u8; CALL_SLOT as usize])];
    for (at, bytes) in writes {
        k.write_user(at, bytes).map_err(|_| Undeliverable::BadStack)?;
    }
    let entry = dispatcher_entry(ctx, &frame, dispatcher);
    k.load_context(&entry, CONTEXT_CONTROL | CONTEXT_INTEGER).map_err(Undeliverable::NoContext)
}

/// What became of a CPU exception.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultOutcome {
    /// The thread goes on in the dispatcher.
    Dispatched(ExceptionRecord),
    /// The process ends with the record's code ([`NtKernel::unhandled_exception`]
    /// was called).
    Unhandled(ExceptionRecord),
}

/// A CPU exception in the calling thread, with its registers `ctx` (filled
/// by the kernel from the trap frame, `CONTEXT_FULL | CONTEXT_SEGMENTS`):
/// classify it and deliver it, or end the process.
pub fn dispatch_fault<K: NtKernel>(k: &mut K, fault: &CpuFault<'_>, ctx: &mut Context) -> FaultOutcome {
    let c = classify(fault, ctx);
    if !c.fail_fast && deliver(k, &c.record, ctx).is_ok() {
        return FaultOutcome::Dispatched(c.record);
    }
    k.unhandled_exception(&c.record, ctx);
    FaultOutcome::Unhandled(c.record)
}

/// Read a `CONTEXT` from user memory.
fn read_context<K: NtKernel>(k: &mut K, addr: u64) -> Result<Context> {
    let mut b = alloc::vec![0u8; CONTEXT_SIZE];
    abi::read(k, addr, &mut b)?;
    Context::from_bytes(&b).ok_or(NtStatus(STATUS_ACCESS_VIOLATION))
}

/// `NtRaiseException(ExceptionRecord*, ContextRecord*, FirstChance)`: raise
/// an exception in the calling thread. First chance, it goes to the
/// dispatcher with the given context (the thread does not come back from
/// the call); second chance — the dispatcher found no handler — the process
/// ends with the exception code (no debugger port). The WDK does not
/// document the service; the parameters are Poly's reading of the
/// documented `RaiseException` / `EXCEPTION_RECORD` model (ADR 0068).
pub fn raise_exception<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [rec_ptr, ctx_ptr, first_chance] = abi::args::<K, 3>(k, regs)?;
    let mut b = [0u8; EXCEPTION_RECORD_SIZE];
    abi::read(k, rec_ptr, &mut b)?;
    let record = ExceptionRecord::from_bytes(&b).map_err(|_| STATUS_INVALID_PARAMETER)?;
    let ctx = read_context(k, ctx_ptr)?;
    if first_chance as u8 != 0 && deliver(k, &record, &ctx).is_ok() {
        return Ok(STATUS_SUCCESS);
    }
    k.unhandled_exception(&record, &ctx);
    Ok(STATUS_SUCCESS)
}

/// `NtContinue(ContextRecord*, TestAlert)`: resume the calling thread with
/// the registers of a `CONTEXT` (the groups its `ContextFlags` selects),
/// delivering a pending user APC first if `TestAlert`. Does not return to
/// the caller. A context whose RIP or RSP is not a user address is refused
/// with `STATUS_INVALID_PARAMETER` (to be confirmed by the spec agent).
pub fn continue_context<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [ctx_ptr, test_alert] = abi::args::<K, 2>(k, regs)?;
    let ctx = read_context(k, ctx_ptr)?;
    let flags = ctx.context_flags;
    if flags & CONTEXT_CONTROL == CONTEXT_CONTROL && (ctx.rip >= ADDRESS_END || ctx.rsp() >= ADDRESS_END) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    k.load_context(&ctx, flags)?;
    if test_alert as u8 != 0 {
        k.test_alert();
    }
    Ok(STATUS_SUCCESS)
}

/// `PolySetExceptionDispatcher(Dispatcher)`: name the calling process's
/// user-mode exception dispatcher (0: none; exceptions then end the
/// process). Poly's own service until our `ntdll` provides
/// `KiUserExceptionDispatcher` (ADR 0068).
pub fn set_exception_dispatcher<K: NtKernel>(k: &mut K, regs: &SyscallRegs) -> Result<NtStatus> {
    let [dispatcher] = abi::args::<K, 1>(k, regs)?;
    if dispatcher >= ADDRESS_END || (dispatcher != 0 && dispatcher < 0x1_0000) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    k.with_state(|_, proc, _| proc.exception_dispatcher = dispatcher);
    Ok(STATUS_SUCCESS)
}

/// The function table of the image mapped at `base`, from its headers
/// (`headers`: at least the first page of the image), for walking a
/// thread's stack: the PE Format's `e_lfanew` (0x3C), the PE32+ optional
/// header's `SizeOfImage` (offset 56), `NumberOfRvaAndSizes` (108) and the
/// exception directory (data directory 3, at 112 + 3 * 8). `None` if the
/// headers are not a PE32+ image with an exception directory.
pub fn image_module(base: u64, headers: &[u8]) -> Option<poly_seh::Module> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(headers.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(headers.get(at..at + 4)?.try_into().ok()?));
    let pe = u32_at(0x3C)? as usize;
    if headers.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let opt = pe.checked_add(24)?;
    if u16_at(opt)? != 0x20B || u32_at(opt + 108)? <= poly_pe::dir::EXCEPTION as u32 {
        return None;
    }
    let size = u32_at(opt + 56)?;
    let dir = opt + 112 + poly_pe::dir::EXCEPTION * 8;
    let (rva, bytes) = (u32_at(dir)?, u32_at(dir + 4)?);
    if bytes == 0 || u64::from(rva) + u64::from(bytes) > u64::from(size) {
        return None;
    }
    Some(poly_seh::Module::from_directory(base, u64::from(size), rva, bytes))
}
