//! The kernel side of NT exception dispatch (ADR 0068): a CPU exception in
//! an NT thread, or `NtRaiseException`, reaches the process's user-mode
//! dispatcher; `NtContinue` loads a `CONTEXT`; an unhandled exception ends
//! the process with the exception code.
//!
//! Everything that decides — the exception code and parameters, the frame
//! on the user stack, the dispatcher's registers, whether the exception can
//! be delivered at all — is `poly_personality_nt::exception`'s, host-tested.
//! Here the trap frame and FPU state become a `CONTEXT` and back, and an
//! unhandled exception's process is ended (with a backtrace of the user
//! stack in the log, walked by `poly-seh` through the image's `.pdata`).

use super::{Kernel, with};
use crate::process::{ExitStatus, Process};
use crate::sync::Mutex;
use crate::user::{self, Fault, UserRegs};
use alloc::vec::Vec;
use poly_ntobj::{NtStatus, ProcessId, ThreadRef};
use poly_personality_nt::exception::{self as nt_exception, CpuFault};
use poly_personality_nt::exec::{TEB_STACK_BASE, TEB_STACK_LIMIT};
use poly_seh::context::{
    CONTEXT_CONTROL, CONTEXT_FLOATING_POINT, CONTEXT_FULL, CONTEXT_INTEGER, CONTEXT_SEGMENTS, FLT_SAVE_MX_CSR,
    FLT_SAVE_SIZE,
};
use poly_seh::{BadAddress, Context, ExceptionRecord, Memory, Modules, StackLimits};

/// What a service or fault asked of the exception side, acted on when it
/// returns.
#[derive(Default)]
pub(super) struct Pending {
    /// RAX to return to user mode with, all 64 bits (a loaded context).
    pub rax: Option<u64>,
    /// An exception nobody handles: the process ends with its code.
    pub unhandled: Option<(ExceptionRecord, Context)>,
}

/// An unhandled exception, kept for the kernel tests: the process, the
/// code and the return addresses of the stack walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unhandled {
    pub pid: u32,
    pub code: u32,
    pub address: u64,
    pub frames: Vec<u64>,
}

static LAST_UNHANDLED: Mutex<Option<Unhandled>> = Mutex::new(None);

/// The last unhandled NT exception (kernel tests).
pub fn last_unhandled() -> Option<Unhandled> {
    LAST_UNHANDLED.lock().clone()
}

/// x87 + SSE state bits of an XSAVE header's XSTATE_BV (Intel SDM Vol. 1
/// §13.4.2), set so the legacy region of a loaded image is used.
const XSTATE_LEGACY: u64 = 0b11;
/// Where the XSAVE header (XSTATE_BV first) follows the legacy region.
const XSAVE_HEADER: usize = 512;

/// The running thread's registers as a `CONTEXT` (`CONTEXT_FULL |
/// CONTEXT_SEGMENTS`): the trap frame and the FPU legacy region.
fn current_context() -> Context {
    let r = user::registers();
    let mut c = Context {
        context_flags: CONTEXT_FULL | CONTEXT_SEGMENTS,
        seg_cs: r.cs as u16,
        seg_ss: r.ss as u16,
        // 64-bit user code has one data selector; DS, ES, FS and GS read
        // as SS (FS and GS bases are what matters, and GS is the TEB).
        seg_ds: r.ss as u16,
        seg_es: r.ss as u16,
        seg_fs: r.ss as u16,
        seg_gs: r.ss as u16,
        eflags: r.rflags as u32,
        rip: r.rip,
        ..Context::default()
    };
    c.gpr = [r.rax, r.rcx, r.rdx, r.rbx, r.rsp, r.rbp, r.rsi, r.rdi, r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15];
    if let Some((image, _)) = user::fpu_image()
        && image.len() >= FLT_SAVE_SIZE
    {
        c.flt_save.copy_from_slice(&image[..FLT_SAVE_SIZE]);
        c.mx_csr = u32::from_le_bytes(image[FLT_SAVE_MX_CSR..FLT_SAVE_MX_CSR + 4].try_into().expect("4 bytes"));
    }
    c
}

/// Apply the groups `flags` selects of `ctx` to the trap frame `r`.
fn apply(r: &mut UserRegs, ctx: &Context, flags: u32) {
    if flags & CONTEXT_CONTROL == CONTEXT_CONTROL {
        r.rip = ctx.rip;
        r.rsp = ctx.rsp();
        r.rflags = u64::from(ctx.eflags);
    }
    if flags & CONTEXT_INTEGER == CONTEXT_INTEGER {
        let g = &ctx.gpr;
        (r.rax, r.rcx, r.rdx, r.rbx, r.rbp, r.rsi, r.rdi) = (g[0], g[1], g[2], g[3], g[5], g[6], g[7]);
        (r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15) = (g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]);
    }
}

/// `NtKernel::load_context` for the running thread: the FPU legacy state
/// first (refused, changing nothing, if it would fault), then the
/// registers. Returns the RAX to return with, if the integer group was
/// loaded.
pub(super) fn load(ctx: &Context, flags: u32) -> Result<Option<u64>, NtStatus> {
    if flags & CONTEXT_FLOATING_POINT == CONTEXT_FLOATING_POINT {
        let (mut image, _) = user::fpu_image().ok_or(poly_ntobj::status::STATUS_UNSUCCESSFUL)?;
        image[..FLT_SAVE_SIZE].copy_from_slice(&ctx.flt_save);
        image[FLT_SAVE_MX_CSR..FLT_SAVE_MX_CSR + 4].copy_from_slice(&ctx.mx_csr.to_le_bytes());
        if image.len() >= XSAVE_HEADER + 8 {
            let bv = u64::from_le_bytes(image[XSAVE_HEADER..XSAVE_HEADER + 8].try_into().expect("8 bytes"));
            image[XSAVE_HEADER..XSAVE_HEADER + 8].copy_from_slice(&(bv | XSTATE_LEGACY).to_le_bytes());
        }
        user::load_fpu(&image).map_err(|_| poly_ntobj::status::STATUS_INVALID_PARAMETER)?;
    }
    let mut regs = user::registers();
    apply(&mut regs, ctx, flags);
    user::set_registers(&regs).map_err(|_| poly_ntobj::status::STATUS_UNSUCCESSFUL)?;
    Ok((flags & CONTEXT_INTEGER == CONTEXT_INTEGER).then_some(ctx.gpr[0]))
}

/// Up to 16 bytes of code at `rip`, as many as can be read.
fn code_at(rip: u64) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..16u64 {
        let mut b = [0u8; 1];
        match rip.checked_add(i) {
            Some(a) if user::copy_from_user(&mut b, a).is_ok() => out.push(b[0]),
            _ => break,
        }
    }
    out
}

/// A CPU exception the running thread of NT process `process` caused in
/// ring 3 (from `user::fault`, interrupts on). Returns `true` if the thread
/// goes on in its process's dispatcher, `false` if the process has no NT
/// state (a bare record: the caller kills the thread). An exception that
/// cannot be delivered ends the process here.
pub fn fault(process: &Process, fault: &Fault) -> bool {
    let Some(memory) = user::current_memory() else { return false };
    if !super::is_nt_process(process.pid) {
        return false;
    }
    let tid = ThreadRef(u64::from(user::current_tid().unwrap_or(process.pid)));
    // What the memory model said about this page fault (resolve_fault).
    let stashed = with(|s| s.ctl.get_mut(&tid).and_then(|c| c.page_fault.take()));
    let page = match (fault.address, fault.page) {
        (Some(_), Some(user::FaultError::Bus)) => Some(NtStatus(poly_seh::consts::STATUS_IN_PAGE_ERROR)),
        (Some(addr), _) => stashed.filter(|(at, _)| *at == addr).map(|(_, status)| status),
        _ => None,
    };
    let code = code_at(fault.rip);
    let mut ctx = current_context();
    let mut k = Kernel::new(ProcessId(process.pid), tid, memory);
    {
        let cpu =
            CpuFault { vector: fault.vector, error_code: fault.error_code, address: fault.address, page, code: &code };
        nt_exception::dispatch_fault(&mut k, &cpu, &mut ctx);
    }
    drop(code);
    finish(k);
    true
}

/// After a service or a fault, with the calling thread's [`Kernel`]: end
/// the process for an unhandled exception (never returns then; `k` and its
/// memory reference are dropped first, as the thread does not come back),
/// else give back the RAX a loaded context asked for.
pub(super) fn finish(mut k: Kernel) -> Option<u64> {
    let Some((record, ctx)) = k.seh.unhandled.take() else { return k.seh.rax.take() };
    let frames = backtrace(&k, &ctx);
    let (pid, tid) = (k.pid, k.tid);
    drop(k);
    end_unhandled(pid, tid, &record, frames)
}

/// The running thread's user memory, for walking its stack.
struct UserMemory;

impl Memory for UserMemory {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        user::copy_from_user(buf, addr).map_err(|_| BadAddress(addr))
    }
}

/// Return addresses of the stack at `ctx`, through the image's `.pdata`.
fn backtrace(k: &Kernel, ctx: &Context) -> Vec<u64> {
    let (image_base, stack) = with(|s| {
        let p = s.processes.get(&k.pid)?;
        let t = p.thread(k.tid)?;
        Some((p.image_base, t.teb))
    })
    .unwrap_or((0, 0));
    let mut headers = alloc::vec![0u8; 0x1000];
    let mut modules = Vec::new();
    if image_base != 0 && user::copy_from_user(&mut headers, image_base).is_ok() {
        modules.extend(nt_exception::image_module(image_base, &headers));
    }
    let read = |at: u64| {
        let mut b = [0u8; 8];
        user::copy_from_user(&mut b, at).ok().map(|()| u64::from_le_bytes(b))
    };
    let high = read(stack + TEB_STACK_BASE).unwrap_or(u64::MAX);
    let low = read(stack + TEB_STACK_LIMIT).map_or(0, |l| l.saturating_sub(0x10_0000));
    let target = Modules { memory: UserMemory, modules, stack: StackLimits { low, high } };
    poly_seh::dispatch::walk(&target, ctx, 32).iter().map(|f| f.pc).collect()
}

/// End the calling thread's process for an unhandled exception: its exit
/// status (and the thread object's) is the exception code, and the
/// thread's exit code (what `thread::join` sees) all 32 bits of it.
fn end_unhandled(pid: ProcessId, tid: ThreadRef, record: &ExceptionRecord, frames: Vec<u64>) -> ! {
    crate::kprintln!(
        "nt: pid {} unhandled exception {:#010x} at {:#x}, params {:x?}; frames {:x?}",
        pid.0,
        record.code,
        record.address,
        record.parameters(),
        frames
    );
    *LAST_UNHANDLED.lock() = Some(Unhandled { pid: pid.0, code: record.code, address: record.address, frames });
    let code = NtStatus(record.code);
    with(|s| s.ctl.entry(tid).or_default().exit_status = Some(code));
    let status = ExitStatus::Exited(record.code as u8);
    // The reference is dropped before the thread ends (it never returns).
    if let Some(process) = user::current_process() {
        process.group_exit(status);
    }
    user::end_thread(status, record.code as usize)
}
