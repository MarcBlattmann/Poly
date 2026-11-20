//! Exceptions in the calls this process serves for others (sheet
//! `GlobalOptions.ExceptionHandling`): by default COM catches an exception the server's code raises
//! inside a served call, the server keeps running and the caller gets `RPC_E_SERVERFAULT`;
//! `IGlobalOptions::Set(COMGLB_EXCEPTION_HANDLING, ...)` lets access violations, or every
//! exception, go on (the process ends with it, as an unhandled exception ends it).
//!
//! [`guarded`] runs a call inside a frame with an exception handler of its own: an x64 language
//! handler (`.seh_handler`, Learn "x64 exception handling") or an x86 registration record on the
//! thread's chain (`fs:[0]`, listed as a safe handler). When the setting says the exception is
//! caught, the handler unwinds the frames above it (`RtlUnwindEx` / `RtlUnwind`, Learn) and the
//! guard returns the exception's code.

use core::ffi::c_void;

/// `RPC_E_SERVERFAULT` (`winerror.h`): the server raised an exception in the call.
pub(crate) const RPC_E_SERVERFAULT: poly_com::hresult::Hresult = 0x8001_0105_u32 as i32;

/// `EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND | EXCEPTION_TARGET_UNWIND | EXCEPTION_COLLIDED_UNWIND`
/// (`winnt.h`): the handler is called for an unwind, not a search.
const EXCEPTION_UNWIND: u32 = 0x2 | 0x4 | 0x20 | 0x40;
/// `ExceptionContinueSearch` (`excpt.h`).
const CONTINUE_SEARCH: u32 = 1;

/// Run `f` guarded: `Err(code)` when an exception the process's setting catches left it (the
/// frames it left were unwound, `f` did not finish).
pub(crate) fn guarded(f: impl FnOnce()) -> Result<(), u32> {
    let mut f = Some(f);
    let mut call = || {
        if let Some(f) = f.take() {
            f();
        }
    };
    let mut dynf: &mut dyn FnMut() = &mut call;
    // SAFETY: `trampoline` gets a pointer to `dynf`, which lives on this frame for the whole call.
    let code = unsafe { poly_ole_guarded(trampoline as usize, (&raw mut dynf).cast()) };
    if code == 0 { Ok(()) } else { Err(code) }
}

/// The guarded call's body: the closure `arg` points at.
unsafe extern "system" fn trampoline(arg: *mut c_void) {
    // SAFETY: `guarded` passes a pointer to its `&mut dyn FnMut()`.
    unsafe { (*arg.cast::<&mut dyn FnMut()>())() };
}

/// Whether the exception `code` is caught (the process's setting).
fn caught(code: u32) -> bool {
    poly_com::globalopts::catches(crate::globalopts::exception_handling(), code)
}

unsafe extern "C" {
    /// `routine(argument)` inside the guard's frame; 0, or the code of an exception caught.
    fn poly_ole_guarded(routine: usize, argument: *mut c_void) -> u32;
}

// ----- x64 ----------------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RtlUnwindEx(frame: u64, target_ip: u64, record: *mut u8, value: u64, context: *mut u8, history: *mut u8) -> !;
}

/// The guard's frame handler (`EXCEPTION_ROUTINE`): during the search, an exception the setting
/// catches is unwound to the guard, which returns its code.
///
/// # Safety
/// Called by the exception dispatcher with valid arguments.
#[cfg(target_arch = "x86_64")]
unsafe extern "C" fn guard_handler(record: *mut u8, frame: u64, _context: *mut u8, dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's `EXCEPTION_RECORD` (`ExceptionCode` 0, `ExceptionFlags` 4) and
    // `DISPATCHER_CONTEXT` (`ContextRecord` 0x28, `HistoryTable` 0x40, `winnt.h`).
    unsafe {
        let code = record.cast::<u32>().read_unaligned();
        let flags = record.add(4).cast::<u32>().read_unaligned();
        if flags & EXCEPTION_UNWIND != 0 || !caught(code) {
            return CONTINUE_SEARCH;
        }
        let context = dc.add(0x28).cast::<*mut u8>().read_unaligned();
        let history = dc.add(0x40).cast::<*mut u8>().read_unaligned();
        RtlUnwindEx(frame, poly_ole_guard_landing as usize as u64, record, u64::from(code), context, history)
    }
}

#[cfg(target_arch = "x86_64")]
unsafe extern "C" {
    /// The address the unwind continues at: the guard's return, RAX the exception's code.
    fn poly_ole_guard_landing();
}

// `poly_ole_guarded(routine, argument)`: the `nop` keeps the call's return address out of the
// epilog (Learn: no handler is called for a frame in its epilog); an unwind to the landing has the
// guard's RSP (its establisher frame) and the code in EAX.
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
    .text
    .globl poly_ole_guarded
    .globl poly_ole_guard_landing
    .p2align 4
    .seh_proc poly_ole_guarded
poly_ole_guarded:
    .seh_handler {handler}, @except
    sub rsp, 0x28
    .seh_stackalloc 0x28
    .seh_endprologue
    mov rax, rcx
    mov rcx, rdx
    call rax
    nop
    xor eax, eax
poly_ole_guard_landing:
    add rsp, 0x28
    ret
    .seh_endproc
"#,
    handler = sym guard_handler,
);

// ----- x86 ----------------------------------------------------------------------------------

#[cfg(target_arch = "x86")]
#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RtlUnwind(frame: *mut u8, target_ip: usize, record: *mut u8, value: usize);
}

#[cfg(target_arch = "x86")]
unsafe extern "C" {
    /// Where the unwind continues: EAX the guard's registration record.
    fn poly_ole_guard_landing();
}

/// The guard's handler (`EXCEPTION_ROUTINE`, `cdecl`): `frame` is the guard's registration record,
/// with the slot for the caught code 8 bytes above it.
///
/// # Safety
/// Called by the exception dispatcher with valid arguments.
#[cfg(target_arch = "x86")]
unsafe extern "C" fn guard_handler(record: *mut u8, frame: *mut u8, _context: *mut u8, _dc: *mut u8) -> u32 {
    // SAFETY: the dispatcher's 32-bit `EXCEPTION_RECORD` (`ExceptionCode` 0, `ExceptionFlags` 4) and
    // the guard's frame (below).
    unsafe {
        let code = record.cast::<u32>().read_unaligned();
        let flags = record.add(4).cast::<u32>().read_unaligned();
        if flags & EXCEPTION_UNWIND != 0 || !caught(code) {
            return CONTINUE_SEARCH;
        }
        frame.add(8).cast::<u32>().write_unaligned(code);
        // The frames above the guard's are unwound; the landing gets the record's address.
        RtlUnwind(frame, poly_ole_guard_landing as usize, record, frame as usize);
    }
    // `RtlUnwind` continued at the landing.
    CONTINUE_SEARCH
}

// `_poly_ole_guarded(routine, argument)` (`cdecl`): EBP, EBX, ESI, EDI saved; [EBP-16] the caught
// code; [EBP-24] the registration record (`Next`, `Handler`) linked at `fs:[0]`. The routine may
// pop its argument or not (ESP is set back from EBP). The landing has the record's address in EAX
// and nothing else of the guard's registers.
#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    r#"
    .text
    .safeseh {handler}
    .globl _poly_ole_guarded
    .globl _poly_ole_guard_landing
    .p2align 4
_poly_ole_guarded:
    push ebp
    mov  ebp, esp
    push ebx
    push esi
    push edi
    push 0
    mov  eax, offset {handler}
    push eax
    push dword ptr fs:[0]
    mov  dword ptr fs:[0], esp
    push dword ptr [ebp + 12]
    call dword ptr [ebp + 8]
    lea  esp, [ebp - 24]
    xor  eax, eax
    jmp  2f
_poly_ole_guard_landing:
    mov  esp, eax
    lea  ebp, [esp + 24]
    mov  eax, dword ptr [ebp - 16]
2:
    mov  ecx, dword ptr [esp]
    mov  dword ptr fs:[0], ecx
    lea  esp, [ebp - 12]
    pop  edi
    pop  esi
    pop  ebx
    pop  ebp
    ret
"#,
    handler = sym guard_handler,
);
