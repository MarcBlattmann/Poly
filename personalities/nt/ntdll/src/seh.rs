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
        // LanguageHandler, HandlerData and ScopeIndex (`winnt.h`), which
        // `__C_specific_handler` reads (ADR 0087).
        dc[0x30..0x38].copy_from_slice(&dispatch.handler.routine.to_le_bytes());
        dc[0x38..0x40].copy_from_slice(&dispatch.handler.data.to_le_bytes());
        dc[0x48..0x4C].copy_from_slice(&dispatch.scope_index.to_le_bytes());
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
        dispatch.scope_index = u32::from_le_bytes(dc[0x48..0x4C].try_into().expect("4 bytes"));
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

/// The top-level filter of unhandled exceptions (0: none).
static UNHANDLED_FILTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// `EXCEPTION_CONTINUE_EXECUTION` (`excpt.h`).
const EXCEPTION_CONTINUE_EXECUTION: i32 = -1;

/// `RtlSetUnhandledExceptionFilter(filter)`: the function the dispatcher
/// asks about an exception no frame handler took, as
/// `filter(EXCEPTION_POINTERS*)`; `EXCEPTION_CONTINUE_EXECUTION` resumes
/// the (possibly changed) context, anything else ends the process with the
/// exception's code. Poly's kernelbase registers its
/// `UnhandledExceptionFilter` here when a program sets a top-level filter
/// (ADR 0099).
#[unsafe(no_mangle)]
pub extern "C" fn RtlSetUnhandledExceptionFilter(filter: u64) {
    UNHANDLED_FILTER.store(filter, core::sync::atomic::Ordering::Release);
}

/// `KiUserExceptionDispatcher`: the kernel transfers here on an exception
/// as if it had been called as `KiUserExceptionDispatcher(record, context)`
/// with both on this thread's stack and a zero return address (ADR 0068).
/// The vectored handlers run first, then the frame-based handlers of the
/// loaded modules' `.pdata`; a handler that takes it resumes with
/// `NtContinue`, otherwise it goes second-chance through `NtRaiseException`
/// (the kernel then ends the process with the code).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn KiUserExceptionDispatcher(record_ptr: *mut u8, context_ptr: *mut u8) -> ! {
    // SAFETY: the kernel wrote a valid record and context below our RSP.
    if unsafe { crate::veh::call_handlers(record_ptr, context_ptr) } {
        // SAFETY: resume the context a vectored handler fixed up.
        unsafe { syscalls::NtContinue(context_ptr, 0) };
    }
    // SAFETY: as above.
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
        DispatchResult::NotHandled => {
            // The top-level filter (kernelbase's `UnhandledExceptionFilter`,
            // registered when the program sets one), asked before anything
            // is unwound, as at the base of a Windows thread.
            let mut rec = record.to_bytes();
            let mut bytes = ctx.to_bytes();
            let filter = UNHANDLED_FILTER.load(core::sync::atomic::Ordering::Acquire);
            if filter != 0 {
                let mut pointers = [rec.as_mut_ptr() as u64, bytes.as_mut_ptr() as u64];
                type Filter = unsafe extern "system" fn(*mut u64) -> i32;
                // SAFETY: the function registered with
                // `RtlSetUnhandledExceptionFilter`, an
                // `LPTOP_LEVEL_EXCEPTION_FILTER`, given EXCEPTION_POINTERS.
                let r = unsafe { core::mem::transmute::<u64, Filter>(filter)(pointers.as_mut_ptr()) };
                if r == EXCEPTION_CONTINUE_EXECUTION && record.flags & 1 == 0 {
                    // SAFETY: resume the context the filter may have fixed up.
                    unsafe { syscalls::NtContinue(bytes.as_mut_ptr(), 0) };
                }
            }
            // Second chance: the kernel ends the process (ADR 0068).
            // SAFETY: a valid record and context.
            unsafe { syscalls::NtRaiseException(rec.as_ptr(), bytes.as_mut_ptr(), 0) };
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
            if rec.code == STATUS_UNWIND_CONSOLIDATE && rec.number_parameters >= 1 && rec.information[0] != 0 {
                let mut target = ContextBuf([0; CONTEXT_SIZE]);
                target.0.copy_from_slice(&resume.to_bytes());
                let mut rb = rec.to_bytes();
                // SAFETY: the record's first parameter is the callback its
                // maker named; the context and record stay in this frame,
                // above everything the callback uses.
                unsafe { poly_consolidate(target.0.as_mut_ptr(), rec.information[0], rb.as_mut_ptr()) };
            }
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

/// `STATUS_UNWIND_CONSOLIDATE` (`ntstatus.h`).
const STATUS_UNWIND_CONSOLIDATE: u32 = 0x8000_0029;

// The end of an unwind whose record is `STATUS_UNWIND_CONSOLIDATE` (ADR
// 0099; msvcrt unwinds to a C++ catch block so): instead of resuming the
// target context, `ExceptionInformation[0]` is called as
// `callback(record)` on the stack below the unwound frames (they stay in
// place), and the address it returns becomes the target's RIP, which is
// then resumed. A stack walk from inside the callback must continue in
// the target frame, not in the frames below it: the callback is called
// from `poly_consolidate_frame`, whose unwind data is a machine frame
// (`UWOP_PUSH_MACHFRAME`, Learn: "x64 exception handling") holding the
// target's RIP and RSP, and whose nonvolatile registers are the target's
// when it calls, so every frame above it unwinds to the target context.
//
// `poly_consolidate(context, callback, record)`: RCX = the target CONTEXT
// (16-byte aligned), RDX = the callback, R8 = the record. Lays the machine
// frame out below the current stack and enters the frame.
core::arch::global_asm!(
    r#"
    .text
    .globl poly_consolidate
    .p2align 4
poly_consolidate:
    and rsp, -16
    sub rsp, 0x30
    mov rax, qword ptr [rcx + 0xF8]
    mov qword ptr [rsp], rax
    mov qword ptr [rsp + 0x08], 0
    mov qword ptr [rsp + 0x10], 0
    mov rax, qword ptr [rcx + 0x98]
    mov qword ptr [rsp + 0x18], rax
    mov qword ptr [rsp + 0x20], 0
    jmp poly_consolidate_frame

    .p2align 4
    .seh_proc poly_consolidate_frame
poly_consolidate_frame:
    .seh_pushframe
    sub rsp, 0x30
    .seh_stackalloc 0x30
    .seh_endprologue
    mov qword ptr [rsp + 0x20], rcx
    mov rbx, qword ptr [rcx + 0x90]
    mov rbp, qword ptr [rcx + 0xA0]
    mov rsi, qword ptr [rcx + 0xA8]
    mov rdi, qword ptr [rcx + 0xB0]
    mov r12, qword ptr [rcx + 0xD8]
    mov r13, qword ptr [rcx + 0xE0]
    mov r14, qword ptr [rcx + 0xE8]
    mov r15, qword ptr [rcx + 0xF0]
    mov rax, rdx
    mov rcx, r8
    call rax
    mov rcx, qword ptr [rsp + 0x20]
    mov qword ptr [rcx + 0xF8], rax
    xor edx, edx
    call {cont}
    int3
    .seh_endproc
"#,
    cont = sym syscalls::NtContinue,
);

unsafe extern "C" {
    fn poly_consolidate(context: *mut u8, callback: u64, record: *mut u8) -> !;
}

// `RtlCaptureContext(CONTEXT*)` (Learn: "Retrieves a context record in the
// context of the caller"; ADR 0087): every integer register as it was at
// the call, `Rsp` and `Rip` as they are after the return, the segment
// registers, `EFlags`, `MxCsr` and the `FXSAVE` image, at `winnt.h`'s
// offsets; `ContextFlags` is `CONTEXT_CONTROL | CONTEXT_INTEGER |
// CONTEXT_SEGMENTS | CONTEXT_FLOATING_POINT`. The `CONTEXT` is 16-byte
// aligned (`DECLSPEC_ALIGN(16)`), as `FXSAVE` needs. Assembly, so that no
// prologue changes what the caller left.
core::arch::global_asm!(
    r#"
    .text
    .globl RtlCaptureContext
    .p2align 4
RtlCaptureContext:
    pushfq
    mov qword ptr [rcx + 0x78], rax
    mov qword ptr [rcx + 0x80], rcx
    mov qword ptr [rcx + 0x88], rdx
    mov qword ptr [rcx + 0x90], rbx
    mov qword ptr [rcx + 0xA0], rbp
    mov qword ptr [rcx + 0xA8], rsi
    mov qword ptr [rcx + 0xB0], rdi
    mov qword ptr [rcx + 0xB8], r8
    mov qword ptr [rcx + 0xC0], r9
    mov qword ptr [rcx + 0xC8], r10
    mov qword ptr [rcx + 0xD0], r11
    mov qword ptr [rcx + 0xD8], r12
    mov qword ptr [rcx + 0xE0], r13
    mov qword ptr [rcx + 0xE8], r14
    mov qword ptr [rcx + 0xF0], r15
    mov word ptr [rcx + 0x38], cs
    mov word ptr [rcx + 0x3A], ds
    mov word ptr [rcx + 0x3C], es
    mov word ptr [rcx + 0x3E], fs
    mov word ptr [rcx + 0x40], gs
    mov word ptr [rcx + 0x42], ss
    fxsave [rcx + 0x100]
    stmxcsr dword ptr [rcx + 0x34]
    pop rax
    mov dword ptr [rcx + 0x44], eax
    lea rax, [rsp + 8]
    mov qword ptr [rcx + 0x98], rax
    mov rax, qword ptr [rsp]
    mov qword ptr [rcx + 0xF8], rax
    mov dword ptr [rcx + 0x30], 0x10000F
    mov rax, qword ptr [rcx + 0x78]
    ret
"#
);

unsafe extern "C" {
    fn RtlCaptureContext(context: *mut u8);
}

/// A `CONTEXT` buffer with the alignment `winnt.h` declares.
#[repr(C, align(16))]
struct ContextBuf([u8; CONTEXT_SIZE]);

/// `RtlRaiseException`: raise `*record` first chance from the caller's
/// context (ADR 0087): this function's own context (`RtlCaptureContext`)
/// unwound one frame, so the exception address is the return address in
/// the caller, which `ExceptionAddress` is set to, and a handler that
/// continues execution resumes there: the call returns.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe extern "C" fn RtlRaiseException(record: *mut u8) {
    let mut buf = ContextBuf([0; CONTEXT_SIZE]);
    // SAFETY: an aligned CONTEXT buffer.
    unsafe { RtlCaptureContext(buf.0.as_mut_ptr()) };
    let mut ctx = Context::from_bytes(&buf.0).unwrap_or_default();
    if let Ok(Some(fe)) = Proc.function_entry(ctx.rip) {
        let mut caller = ctx.clone();
        if virtual_unwind(0, fe.image_base, ctx.rip, &fe.function, &mut caller, &Proc, None).is_ok() {
            ctx = caller;
        }
    }
    // SAFETY: ExceptionAddress (at 16) of the caller's record.
    unsafe { record.add(16).cast::<u64>().write_unaligned(ctx.rip) };
    let mut bytes = ctx.to_bytes();
    // SAFETY: a valid record and context; if a handler continues, the
    // kernel resumes the caller's context, so this does not return here.
    unsafe { syscalls::NtRaiseException(record, bytes.as_mut_ptr(), 1) };
    // SAFETY: NtRaiseException only comes back on a bad argument.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}

// ----- __C_specific_handler (ADR 0087; `poly_seh::scope`) -----------------------------

/// The program's filters, termination handlers and the unwind to an
/// `__except` block, called with the x64 ABI.
struct ScopeCaller {
    image_base: u64,
    frame: u64,
    record: *mut u8,
    context: *mut u8,
    dc: *mut u8,
}

impl poly_seh::scope::ScopeCalls for ScopeCaller {
    fn filter(&mut self, rva: u32) -> i32 {
        // EXCEPTION_POINTERS: the record and the context (`winnt.h`).
        let mut pointers = [self.record as u64, self.context as u64];
        type Filter = unsafe extern "system" fn(*mut u64, u64) -> i32;
        let f = self.image_base + u64::from(rva);
        // SAFETY: the compiler's filter at this address, called as its
        // handler data says.
        unsafe { core::mem::transmute::<u64, Filter>(f)(pointers.as_mut_ptr(), self.frame) }
    }

    fn termination(&mut self, rva: u32) {
        type Termination = unsafe extern "system" fn(u8, u64);
        let f = self.image_base + u64::from(rva);
        // SAFETY: as above.
        unsafe { core::mem::transmute::<u64, Termination>(f)(1, self.frame) }
    }

    fn set_scope_index(&mut self, index: u32) {
        // SAFETY: DISPATCHER_CONTEXT.ScopeIndex at 0x48.
        unsafe { self.dc.add(0x48).cast::<u32>().write_unaligned(index) };
    }

    fn unwind_to(&mut self, jump_target: u32) {
        // SAFETY: the record is valid; the return value is its code.
        let code = unsafe { self.record.cast::<u32>().read_unaligned() };
        let target = self.image_base + u64::from(jump_target);
        // SAFETY: DISPATCHER_CONTEXT.ContextRecord (0x28) and HistoryTable
        // (0x40); unwinding to the establisher frame does not return.
        unsafe {
            let ctx = self.dc.add(0x28).cast::<*mut u8>().read_unaligned();
            let history = self.dc.add(0x40).cast::<*mut u8>().read_unaligned();
            RtlUnwindEx(self.frame, target, self.record, u64::from(code), ctx, history)
        }
    }
}

/// `__C_specific_handler(ExceptionRecord, EstablisherFrame, ContextRecord,
/// DispatcherContext)`: the C compiler's language-specific handler for
/// `__try` blocks; the decisions are `poly_seh::scope`'s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __C_specific_handler(record: *mut u8, frame: u64, context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's DISPATCHER_CONTEXT (`winnt.h`): ControlPc 0,
    // ImageBase 8, TargetIp 0x20, HandlerData 0x38, ScopeIndex 0x48; the
    // record's ExceptionFlags at 4.
    let (pc, base, target, data, index, flags) = unsafe {
        (
            dc.cast::<u64>().read_unaligned(),
            dc.add(8).cast::<u64>().read_unaligned(),
            dc.add(0x20).cast::<u64>().read_unaligned(),
            dc.add(0x38).cast::<u64>().read_unaligned(),
            dc.add(0x48).cast::<u32>().read_unaligned(),
            record.add(4).cast::<u32>().read_unaligned(),
        )
    };
    let Ok(table) = poly_seh::scope::read_table(&Proc, data) else { return 1 };
    let rva = |a: u64| u32::try_from(a.wrapping_sub(base)).unwrap_or(u32::MAX);
    let mut calls = ScopeCaller { image_base: base, frame, record, context, dc };
    let target = if target == 0 { None } else { Some(rva(target)) };
    match poly_seh::scope::handle(&table, flags, rva(pc), target, index, &mut calls) {
        poly_seh::Disposition::ContinueExecution => 0,
        _ => 1,
    }
}

// `KiUserApcDispatcher`: the kernel redirects an alertable thread here
// (ADR 0065's APC entry, Poly's own convention) with R11 = the APC routine,
// RCX/RDX/R8 = its three arguments and RSP 16-byte aligned; it keeps the
// interrupted context itself. Call `routine(arg1, arg2, arg3)`, then
// `PolyResumeApc`, which restores that context (the wait returns
// `STATUS_USER_APC`). The `PolyResumeApc` stub is generated from the
// service table with the others (`build.rs`).
core::arch::global_asm!(
    r#"
    .text
    .globl KiUserApcDispatcher
    .p2align 4
    .seh_proc KiUserApcDispatcher
KiUserApcDispatcher:
    sub rsp, 0x20
    .seh_stackalloc 0x20
    .seh_endprologue
    call r11
    call PolyResumeApc
    int3
    .seh_endproc
"#
);

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
