//! Exceptions and interlocked lists (ADR 0087): `RaiseException`, the
//! top-level filter (`SetUnhandledExceptionFilter`,
//! `UnhandledExceptionFilter`), and the interlocked singly linked lists
//! (`poly_kernelbase::slist`) that MSVC's start-up code initialises.

use crate::nt;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_kernelbase::slist::Header;

// ----- RaiseException --------------------------------------------------------------

/// `EXCEPTION_NONCONTINUABLE`, `EXCEPTION_MAXIMUM_PARAMETERS` (`winnt.h`).
const EXCEPTION_NONCONTINUABLE: u32 = 1;
const EXCEPTION_MAXIMUM_PARAMETERS: usize = 15;
/// `EXCEPTION_SOFTWARE_ORIGINATE` (`winnt.h`): every exception
/// `RaiseException` raises carries it (observed on Windows 11 25H2:
/// `_CxxThrowException.yaml`, `record_flags`).
const EXCEPTION_SOFTWARE_ORIGINATE: u32 = 0x80;
/// `sizeof(EXCEPTION_RECORD)` on x64 (`winnt.h`): code, flags, the nested
/// record, the address, the parameter count and 15 parameters.
const EXCEPTION_RECORD_SIZE: usize = 152;

/// `RaiseException`: an `EXCEPTION_RECORD` with the code, the
/// `EXCEPTION_NONCONTINUABLE` flag if given, and up to
/// `EXCEPTION_MAXIMUM_PARAMETERS` arguments (Learn: the count "must not
/// exceed" it; Poly takes the first 15; "ignored if lpArguments is NULL"),
/// raised by `ntdll`'s `RtlRaiseException`, which fills in the address.
/// Returns if a handler continues execution.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RaiseException(code: u32, flags: u32, count: u32, args: *const usize) {
    let mut r = [0u8; EXCEPTION_RECORD_SIZE];
    r[0..4].copy_from_slice(&code.to_le_bytes());
    let flags = (flags & EXCEPTION_NONCONTINUABLE) | EXCEPTION_SOFTWARE_ORIGINATE;
    r[4..8].copy_from_slice(&flags.to_le_bytes());
    let n = if args.is_null() { 0 } else { (count as usize).min(EXCEPTION_MAXIMUM_PARAMETERS) };
    r[24..28].copy_from_slice(&(n as u32).to_le_bytes());
    for i in 0..n {
        // SAFETY: the caller's array of `count` arguments.
        let v = unsafe { args.add(i).read_unaligned() } as u64;
        r[32 + i * 8..40 + i * 8].copy_from_slice(&v.to_le_bytes());
    }
    // SAFETY: a valid record.
    unsafe { nt::RtlRaiseException(r.as_ptr()) };
}

// ----- the top-level filter ------------------------------------------------------------

/// The filter `SetUnhandledExceptionFilter` installed (0: none).
static TOP_FILTER: AtomicUsize = AtomicUsize::new(0);

/// `EXCEPTION_EXECUTE_HANDLER`, `EXCEPTION_CONTINUE_SEARCH` (`excpt.h`).
const EXCEPTION_EXECUTE_HANDLER: i32 = 1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlSetUnhandledExceptionFilter(filter: usize);
}

/// `SetUnhandledExceptionFilter`: the previous filter. `ntdll`'s dispatcher
/// is told to ask `UnhandledExceptionFilter` about exceptions no frame
/// handler takes (ADR 0099), as the base of a Windows thread does.
#[unsafe(no_mangle)]
pub extern "system" fn SetUnhandledExceptionFilter(filter: usize) -> usize {
    // SAFETY: registers a function with the documented filter signature.
    unsafe { RtlSetUnhandledExceptionFilter(UnhandledExceptionFilter as *const () as usize) };
    TOP_FILTER.swap(filter, Ordering::AcqRel)
}

/// `UnhandledExceptionFilter`: Learn: with a debugger attached,
/// `EXCEPTION_CONTINUE_SEARCH`; otherwise the top-level filter decides,
/// and when it has none (or it says `EXCEPTION_CONTINUE_SEARCH`) the
/// function returns `EXCEPTION_EXECUTE_HANDLER` — Poly shows no
/// "Application Error" box, as with `SEM_NOGPFAULTERRORBOX`. Poly's
/// `ntdll` does not call this for an exception nothing handled yet (the
/// process ends second-chance, ADR 0068); programs that call it from
/// their own `__except` get this behaviour.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnhandledExceptionFilter(pointers: *mut u8) -> i32 {
    if crate::misc::IsDebuggerPresent() != 0 {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let filter = TOP_FILTER.load(Ordering::Acquire);
    if filter != 0 {
        type Filter = unsafe extern "system" fn(*mut u8) -> i32;
        // SAFETY: the program installed a filter with this signature.
        let r = unsafe { core::mem::transmute::<usize, Filter>(filter)(pointers) };
        if r != EXCEPTION_CONTINUE_SEARCH {
            return r;
        }
    }
    EXCEPTION_EXECUTE_HANDLER
}

// ----- interlocked singly linked lists ----------------------------------------------------

/// `LOCK CMPXCHG16B` on the 16-byte aligned header at `p`: replace `old`
/// with `new`; the current value when they differ.
///
/// # Safety
/// `p` is a valid, 16-byte aligned `SLIST_HEADER`.
unsafe fn exchange(p: *mut u64, old: (u64, u64), new: (u64, u64)) -> Result<(), (u64, u64)> {
    let (mut lo, mut hi) = old;
    let ok: u8;
    // SAFETY: the caller's contract; RBX cannot be an asm operand, so the
    // new low quadword travels in another register and is swapped in and
    // back around the instruction.
    unsafe {
        core::arch::asm!(
            "xchg {new_lo}, rbx",
            "lock cmpxchg16b [{p}]",
            "sete {ok}",
            "xchg {new_lo}, rbx",
            p = in(reg) p,
            new_lo = inout(reg) new.0 => _,
            ok = out(reg_byte) ok,
            inout("rax") lo,
            inout("rdx") hi,
            in("rcx") new.1,
            options(nostack),
        );
    }
    if ok != 0 { Ok(()) } else { Err((lo, hi)) }
}

/// The header's two quadwords as they are now (any value; the exchange
/// checks it).
///
/// # Safety
/// `p` is a valid `SLIST_HEADER`.
unsafe fn load(p: *mut u64) -> (u64, u64) {
    // SAFETY: the caller's contract.
    unsafe { (core::ptr::read_volatile(p), core::ptr::read_volatile(p.add(1))) }
}

/// `InitializeSListHead`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitializeSListHead(head: *mut u64) {
    // SAFETY: the caller's SLIST_HEADER.
    unsafe {
        head.write_volatile(0);
        head.add(1).write_volatile(0);
    }
}

/// `InterlockedPushEntrySList`: the previous first entry.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InterlockedPushEntrySList(head: *mut u64, entry: *mut u64) -> *mut u64 {
    // SAFETY: the caller's header and 16-byte aligned entry.
    unsafe {
        let mut cur = load(head);
        loop {
            let h = Header::unpack(cur.0, cur.1);
            entry.write_volatile(h.next);
            match exchange(head, cur, h.pushed(entry as u64).pack()) {
                Ok(()) => return h.next as *mut u64,
                Err(now) => cur = now,
            }
        }
    }
}

/// `InterlockedPopEntrySList`: the first entry, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InterlockedPopEntrySList(head: *mut u64) -> *mut u64 {
    // SAFETY: the caller's header; its entries are valid while listed.
    unsafe {
        let mut cur = load(head);
        loop {
            let h = Header::unpack(cur.0, cur.1);
            if h.next == 0 {
                return core::ptr::null_mut();
            }
            let following = (h.next as *const u64).read_volatile();
            let Some(new) = h.popped(following) else { return core::ptr::null_mut() };
            match exchange(head, cur, new.pack()) {
                Ok(()) => return h.next as *mut u64,
                Err(now) => cur = now,
            }
        }
    }
}

/// `InterlockedFlushSList`: every entry, as a list, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InterlockedFlushSList(head: *mut u64) -> *mut u64 {
    // SAFETY: the caller's header.
    unsafe {
        let mut cur = load(head);
        loop {
            let h = Header::unpack(cur.0, cur.1);
            if h.next == 0 {
                return core::ptr::null_mut();
            }
            match exchange(head, cur, h.flushed().pack()) {
                Ok(()) => return h.next as *mut u64,
                Err(now) => cur = now,
            }
        }
    }
}

/// `QueryDepthSList`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryDepthSList(head: *mut u64) -> u16 {
    // SAFETY: the caller's header.
    unsafe { core::ptr::read_volatile(head) as u16 }
}
