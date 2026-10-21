//! The structured-exception-handling exports (ADR 0068, 0071), over
//! [`poly_seh`]: function-entry lookup, the virtual unwinder, the two
//! dispatch passes, and `KiUserExceptionDispatcher` / `KiUserApcDispatcher`.
//!
//! The crate reads memory and calls language-specific handlers through
//! traits; here they read this process's memory with raw pointers and call
//! the handler routines with the documented x64 ABI
//! (`handler(record, establisher_frame, context, dispatcher_context)`,
//! Learn: "Language-specific handler").

#[allow(unused_imports)]
use crate::lock;
use crate::{syscalls, with_loader};
#[allow(unused_imports)]
use poly_ntobj::NtStatus;
use poly_seh::context::CONTEXT_SIZE;
use poly_seh::dispatch::{
    DispatchInfo, DispatchResult, Disposition, FunctionEntry, Handlers, StackLimits, Target, dispatch_exception, unwind,
};
use poly_seh::pdata::{self, RuntimeFunction};
use poly_seh::record::EXCEPTION_RECORD_SIZE;
use poly_seh::{BadAddress, Context, ExceptionRecord, Memory, UnwindError, virtual_unwind};

/// This process as a [`Target`]: raw-pointer memory, loader function tables,
/// the current thread's stack limits from the TEB (`NT_TIB.StackBase` at
/// TEB + 0x08, `StackLimit` at TEB + 0x10).
struct Proc;

impl Memory for Proc {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), BadAddress> {
        if addr == 0 {
            return Err(BadAddress(addr));
        }
        // SAFETY: SEH reads only mapped memory of this process; a bad
        // address would fault, which the kernel turns into a nested
        // exception. The addresses come from unwind data and the stack.
        unsafe { core::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len()) };
        Ok(())
    }
}

impl Target for Proc {
    fn function_entry(&self, pc: u64) -> Result<Option<FunctionEntry>, UnwindError> {
        let Some(m) = with_loader(|l, _| l.find(pc)) else { return Ok(None) };
        let (rva, bytes) = m.exception;
        if bytes == 0 {
            return Ok(None);
        }
        let Ok(off) = u32::try_from(pc - m.base) else { return Ok(None) };
        let table = m.base + u64::from(rva);
        let count = bytes as usize / RuntimeFunction::SIZE;
        Ok(pdata::lookup(self, table, count, off)?.map(|(i, function)| FunctionEntry {
            image_base: m.base,
            function,
            address: table + (i * RuntimeFunction::SIZE) as u64,
        }))
    }

    fn stack_limits(&self) -> StackLimits {
        let teb = crate::lock::teb();
        // SAFETY: the TEB holds the stack base and limit.
        let (base, limit) = unsafe { (((teb + 0x08) as *const u64).read(), ((teb + 0x10) as *const u64).read()) };
        StackLimits { low: limit, high: base }
    }
}

/// Calls language-specific handlers with the x64 ABI.
struct CallHandlers;

impl Handlers for CallHandlers {
    fn call(
        &mut self,
        record: &mut ExceptionRecord,
        establisher_frame: u64,
        context: &mut Context,
        dispatch: &mut DispatchInfo,
    ) -> Disposition {
        // DISPATCHER_CONTEXT the handler may read (ControlPc, ImageBase,
        // FunctionEntry, EstablisherFrame, ...). We pass a minimal one.
        let mut dc = [0u8; 0x50];
        dc[0..8].copy_from_slice(&dispatch.control_pc.to_le_bytes());
        dc[8..16].copy_from_slice(&dispatch.function.image_base.to_le_bytes());
        dc[16..24].copy_from_slice(&dispatch.function.address.to_le_bytes());
        dc[24..32].copy_from_slice(&establisher_frame.to_le_bytes());
        dc[32..40].copy_from_slice(&dispatch.target_ip.to_le_bytes());
        let mut ctx_bytes = context.to_bytes();
        dc[40..48].copy_from_slice(&(ctx_bytes.as_mut_ptr() as u64).to_le_bytes());
        let mut rec_bytes = record.to_bytes();
        type HandlerFn = unsafe extern "system" fn(*mut u8, u64, *mut u8, *mut u8) -> u32;
        let handler = dispatch.handler.routine;
        // SAFETY: the unwinder resolved `handler` to a frame handler in a
        // mapped image; the ABI is the documented one.
        let disp = unsafe {
            core::mem::transmute::<u64, HandlerFn>(handler)(
                rec_bytes.as_mut_ptr(),
                establisher_frame,
                ctx_bytes.as_mut_ptr(),
                dc.as_mut_ptr(),
            )
        };
        // A handler that fixed the context up wrote into `ctx_bytes`.
        if let Some(c) = Context::from_bytes(&ctx_bytes) {
            *context = c;
        }
        if let Ok(r) = ExceptionRecord::from_bytes(&rec_bytes) {
            *record = r;
        }
        Disposition::from_u32(disp)
    }
}

/// `RtlLookupFunctionEntry(ControlPc, ImageBase*, HistoryTable*)`: the
/// `RUNTIME_FUNCTION` covering `pc`, with its image base.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlLookupFunctionEntry(pc: u64, image_base: *mut u64, _history: *mut u8) -> *const u8 {
    match Proc.function_entry(pc) {
        Ok(Some(fe)) => {
            if !image_base.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { image_base.write(fe.image_base) };
            }
            fe.address as *const u8
        }
        _ => {
            if !image_base.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { image_base.write(0) };
            }
            core::ptr::null()
        }
    }
}

/// `RtlVirtualUnwind`: unwind one frame, updating `*context` to the
/// caller's and returning the handler address (0 for none).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlVirtualUnwind(
    handler_type: u32,
    image_base: u64,
    control_pc: u64,
    function: *const u8,
    context: *mut u8,
    handler_data: *mut *mut u8,
    establisher: *mut u64,
    _pointers: *mut u8,
) -> u64 {
    // SAFETY: a valid CONTEXT and RUNTIME_FUNCTION (the RtlVirtualUnwind
    // contract).
    let mut ctx = match unsafe { read_context(context) } {
        Some(c) => c,
        None => return 0,
    };
    // SAFETY: a 12-byte RUNTIME_FUNCTION.
    let func = unsafe { RuntimeFunction::from_bytes(&*(function as *const [u8; 12])) };
    let r = virtual_unwind(handler_type as u8, image_base, control_pc, &func, &mut ctx, &Proc, None);
    match r {
        Ok(u) => {
            // SAFETY: a valid out CONTEXT.
            unsafe { write_context(context, &ctx) };
            if !establisher.is_null() {
                // SAFETY: a valid out-pointer.
                unsafe { establisher.write(u.establisher_frame) };
            }
            match u.handler {
                Some(h) => {
                    if !handler_data.is_null() {
                        // SAFETY: a valid out-pointer.
                        unsafe { handler_data.write(h.data as *mut u8) };
                    }
                    h.routine
                }
                None => 0,
            }
        }
        Err(_) => 0,
    }
}

/// `KiUserExceptionDispatcher`: the kernel transfers here on a fault with
/// RCX = `&EXCEPTION_RECORD`, RDX = `&CONTEXT` on this thread's stack (ADR
/// 0068). Dispatch the exception; a handler that takes it resumes with
/// `NtContinue`, otherwise it goes second-chance through `NtRaiseException`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn KiUserExceptionDispatcher() -> ! {
    let (rcx, rdx): (u64, u64);
    // SAFETY: the kernel set RCX and RDX before entering us.
    unsafe { core::arch::asm!("", out("rcx") rcx, out("rdx") rdx, options(nostack)) };
    let record_ptr = rcx as *mut u8;
    let context_ptr = rdx as *mut u8;
    // SAFETY: the kernel wrote a valid record and context below our RSP.
    let mut record = unsafe { read_record(record_ptr) };
    // SAFETY: as above.
    let mut ctx = unsafe { read_context(context_ptr) }.unwrap_or_default();
    let result = dispatch_exception(&Proc, &mut CallHandlers, &mut record, &mut ctx);
    match result {
        DispatchResult::ContinueExecution => {
            let mut bytes = ctx.to_bytes();
            // SAFETY: NtContinue resumes the (fixed-up) context; never returns.
            unsafe { syscalls::NtContinue(bytes.as_mut_ptr(), 0) };
        }
        _ => {
            let rec = record.to_bytes();
            let mut bytes = ctx.to_bytes();
            // Second chance: the kernel ends the process (ADR 0068).
            // SAFETY: a valid record and context.
            unsafe { syscalls::NtRaiseException(rec.as_ptr(), bytes.as_mut_ptr(), 0) };
        }
    }
    // SAFETY: neither syscall returns; if one does, fail fast.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}

/// `RtlUnwindEx`: unwind to `target_frame` / `target_ip`, calling
/// termination handlers, then resume there with `return_value` via
/// `NtContinue`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUnwindEx(
    target_frame: u64,
    target_ip: u64,
    record: *mut u8,
    return_value: u64,
    context: *mut u8,
    _history: *mut u8,
) -> ! {
    // SAFETY: a valid CONTEXT.
    let ctx = unsafe { read_context(context) }.unwrap_or_default();
    let mut rec = if record.is_null() {
        poly_seh::dispatch::unwind_record(ctx.rip)
    } else {
        // SAFETY: a valid record when non-null.
        unsafe { read_record(record) }
    };
    let frame = if target_frame == 0 { None } else { Some(target_frame) };
    match unwind(&Proc, &mut CallHandlers, frame, target_ip, &mut rec, return_value, &ctx) {
        Ok(resume) => {
            let mut bytes = resume.to_bytes();
            // SAFETY: resume the target frame; never returns.
            unsafe { syscalls::NtContinue(bytes.as_mut_ptr(), 0) };
        }
        Err(_) => {
            // SAFETY: a bad unwind fails fast.
            unsafe { core::arch::asm!("int 0x29", in("ecx") 3u32, options(noreturn)) }
        }
    }
    // SAFETY: NtContinue does not return.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}

/// `RtlCaptureContext`: fill `*context` with the caller's registers. A
/// partial capture (integer and control registers), enough for an unwind
/// to start; the legacy FP area is left zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlCaptureContext(context: *mut u8) {
    let mut ctx = Context::default();
    // SAFETY: read the caller's saved registers from this frame.
    unsafe {
        let rsp: u64;
        let rbp: u64;
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem));
        core::arch::asm!("mov {}, rbp", out(reg) rbp, options(nostack, nomem));
        ctx.set_rsp(rsp + 8); // the caller's RSP: past our return address
        ctx.rip = (rsp as *const u64).read(); // our return address
        ctx.gpr[5] = rbp; // RBP
    }
    // SAFETY: a valid CONTEXT buffer.
    unsafe { write_context(context, &ctx) };
}

/// `RtlRaiseException`: capture the context and raise `*record` first
/// chance.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlRaiseException(record: *const u8) -> ! {
    let mut ctx = Context::default();
    // SAFETY: capture this frame's RSP/RIP.
    unsafe {
        let rsp: u64;
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, nomem));
        ctx.set_rsp(rsp);
        ctx.rip = RtlRaiseException as *const () as u64;
    }
    let mut bytes = ctx.to_bytes();
    // SAFETY: a valid record and context.
    unsafe { syscalls::NtRaiseException(record, bytes.as_mut_ptr(), 1) };
    // SAFETY: if the exception was handled the kernel resumes elsewhere; a
    // return here is a fail-fast.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}

/// `KiUserApcDispatcher`: entered by the kernel with the APC routine and
/// its arguments; call it, then resume the interrupted context with
/// `NtContinue`. The kernel places the interrupted `CONTEXT` on the stack
/// (Poly's convention mirrors ADR 0065's trampoline).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn KiUserApcDispatcher() -> ! {
    let (routine, a1, a2, a3, ctx): (u64, u64, u64, u64, u64);
    // SAFETY: the kernel set these registers (ADR 0065 APC entry).
    unsafe {
        core::arch::asm!(
            "", out("rcx") a1, out("rdx") a2, out("r8") a3, out("r9") ctx, out("r11") routine,
            options(nostack)
        );
    }
    type Apc = unsafe extern "system" fn(u64, u64, u64);
    // SAFETY: the kernel resolved `routine` to the APC routine.
    unsafe { core::mem::transmute::<u64, Apc>(routine)(a1, a2, a3) };
    // SAFETY: resume the interrupted context.
    unsafe { syscalls::NtContinue(ctx as *mut u8, 0) };
    // SAFETY: NtContinue does not return.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}

/// `RtlPcToFileHeader`: the base of the image containing `pc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlPcToFileHeader(pc: *mut u8, base: *mut *mut u8) -> *mut u8 {
    let b = with_loader(|l, _| l.find(pc as u64)).map_or(0, |m| m.base);
    if !base.is_null() {
        // SAFETY: a valid out-pointer.
        unsafe { base.write(b as *mut u8) };
    }
    b as *mut u8
}

/// `RtlImageNtHeader`: the `IMAGE_NT_HEADERS` of a mapped image (the `MZ`
/// then `e_lfanew` then the `PE\0\0` signature).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlImageNtHeader(base: *mut u8) -> *mut u8 {
    if base.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: a mapped image begins with the DOS header.
    unsafe {
        if (base as *const u16).read() != 0x5A4D {
            return core::ptr::null_mut();
        }
        let e_lfanew = (base.add(0x3C) as *const u32).read() as usize;
        let nt = base.add(e_lfanew);
        if (nt as *const u32).read() != 0x0000_4550 {
            return core::ptr::null_mut();
        }
        nt
    }
}

// ----- CONTEXT / EXCEPTION_RECORD byte helpers ------------------------------

/// Read a `CONTEXT` from `ptr`.
///
/// # Safety
/// `ptr` is null or a valid `CONTEXT` of [`CONTEXT_SIZE`] bytes.
unsafe fn read_context(ptr: *const u8) -> Option<Context> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the caller's contract.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, CONTEXT_SIZE) };
    Context::from_bytes(bytes)
}

/// Write a `CONTEXT` to `ptr`.
///
/// # Safety
/// `ptr` is a valid `CONTEXT` buffer of [`CONTEXT_SIZE`] bytes.
unsafe fn write_context(ptr: *mut u8, ctx: &Context) {
    let bytes = ctx.to_bytes();
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
}

/// Read an `EXCEPTION_RECORD` from `ptr`.
///
/// # Safety
/// `ptr` is a valid `EXCEPTION_RECORD` of [`EXCEPTION_RECORD_SIZE`] bytes.
unsafe fn read_record(ptr: *const u8) -> ExceptionRecord {
    // SAFETY: the caller's contract.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, EXCEPTION_RECORD_SIZE) };
    ExceptionRecord::from_bytes(bytes).unwrap_or_else(|_| ExceptionRecord::new(0xC000_0005, 0, 0, &[]))
}
