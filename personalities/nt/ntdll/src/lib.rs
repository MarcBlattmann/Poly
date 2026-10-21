//! Our `ntdll.dll` (ADR 0071): a real PE DLL, built from Rust for
//! `x86_64-pc-windows-msvc` and linked by `rust-lld`. The hard logic lives
//! in the host-tested crates [`poly_ldr`] (the loader), [`poly_rtl`] (the
//! heap, strings, the status table, critical sections, the version) and
//! [`poly_seh`] (x64 unwinding); this crate is the glue: the exported
//! `Rtl*`, `Heap*`, `Ldr*` and `Ki*` entry points, the system-call stubs,
//! the process heap as the global allocator, and the loader behind the
//! loader lock.
//!
//! It never calls user code while it holds the loader lock's state: a load
//! returns the TLS callbacks and `DllMain`s to run as a list, and they are
//! run with the lock held but no borrow of the loader, so a `DllMain` may
//! call back into the loader (Learn: "Dynamic-Link Library Best Practices").

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

mod chkstk;
mod exports;
mod heap;
mod lock;
mod params;
mod seh;
mod start;
mod sys;
mod syscalls;
mod veh;

use poly_ldr::{Callout, CalloutKind, Config, Loader};
use poly_ntobj::NtStatus;

#[global_allocator]
static ALLOC: heap::ProcessHeapAlloc = heap::ProcessHeapAlloc;

/// MSVC emits a reference to this when floating-point is used.
#[unsafe(no_mangle)]
#[used]
static _fltused: i32 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // A panic in ntdll is a fail-fast (`__fastfail`, int 0x29, Learn:
    // "__fastfail"): a non-continuable stack-buffer-overrun the kernel does
    // not dispatch (ADR 0068).
    // SAFETY: int 0x29 with the fail-fast code in ECX never returns.
    unsafe { core::arch::asm!("int 0x29", in("ecx") 2u32, options(noreturn)) }
}

/// The loader behind a spin lock (the loader's own state; the loader lock
/// critical section apps see is [`exports::LOADER_LOCK`]). The access is
/// brief and never runs user code, so a spin lock is enough.
struct LoaderCell {
    flag: core::sync::atomic::AtomicBool,
    loader: core::cell::UnsafeCell<Option<Loader>>,
}

// SAFETY: access is serialized by `flag`.
unsafe impl Sync for LoaderCell {}

static LOADER: LoaderCell =
    LoaderCell { flag: core::sync::atomic::AtomicBool::new(false), loader: core::cell::UnsafeCell::new(None) };

/// Run `f` with the loader slot (exclusive).
fn with_slot<R>(f: impl FnOnce(&mut Option<Loader>) -> R) -> R {
    use core::sync::atomic::Ordering;
    while LOADER.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only accessor.
    let r = f(unsafe { &mut *LOADER.loader.get() });
    LOADER.flag.store(false, Ordering::Release);
    r
}

/// Run `f` with the loader and the process (exclusive). Not re-entrant: a
/// callout must not be run while this is held (it is not).
fn with_loader<R>(f: impl FnOnce(&mut Loader, &mut sys::Sys) -> R) -> R {
    with_slot(|slot| {
        let loader = slot.get_or_insert_with(|| Loader::new(Config::default()));
        f(loader, &mut sys::Sys)
    })
}

/// Call one callout with the Microsoft x64 ABI: `routine(base, reason,
/// reserved)` for an entry point, `routine(base, reason, reserved)` for a
/// TLS callback (the same three arguments, Learn: `PIMAGE_TLS_CALLBACK`).
/// Returns the `BOOL` the entry point returned (TLS callbacks return void;
/// their result is ignored).
fn run_callout(c: &Callout) -> i32 {
    type Fn3 = unsafe extern "system" fn(u64, u32, u64) -> i32;
    // SAFETY: the loader resolved `routine` to code in a mapped image; the
    // ABI is the documented `DllMain` / TLS-callback signature.
    let r = unsafe { core::mem::transmute::<u64, Fn3>(c.routine)(c.base, c.reason, c.reserved) };
    match c.kind {
        CalloutKind::Entry => r,
        CalloutKind::Tls => 1,
    }
}

/// Run a list of attach/detach callouts. For `DLL_PROCESS_ATTACH`, a
/// `DllMain` that returns `FALSE` stops the run and the failing module's id
/// is returned; `ntdll`'s caller then cleans up with
/// [`Loader::fail_load`](poly_ldr::Loader::fail_load).
fn run_callouts(callouts: &[Callout]) -> Result<(), usize> {
    for c in callouts {
        let ok = run_callout(c);
        if c.kind == CalloutKind::Entry && c.reason == poly_ldr::reason::PROCESS_ATTACH && ok == 0 {
            return Err(c.module);
        }
    }
    Ok(())
}
