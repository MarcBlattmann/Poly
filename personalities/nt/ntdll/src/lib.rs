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
mod seh;
mod sys;
mod syscalls;

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

/// Run `f` with the loader and the process (exclusive). Not re-entrant: a
/// callout must not be run while this is held (it is not).
fn with_loader<R>(f: impl FnOnce(&mut Loader, &mut sys::Sys) -> R) -> R {
    use core::sync::atomic::Ordering;
    while LOADER.flag.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `flag` makes this the only accessor.
    let slot = unsafe { &mut *LOADER.loader.get() };
    let loader = slot.get_or_insert_with(|| Loader::new(Config::default()));
    let mut sys = sys::Sys;
    let r = f(loader, &mut sys);
    LOADER.flag.store(false, Ordering::Release);
    r
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

/// `LdrInitializeThunk`: where the kernel starts every thread of an `ntdll`
/// process (ADR 0071). The first thread initialises the process (the
/// process heap, the loader, the static imports and their
/// `DLL_PROCESS_ATTACH` callouts); later threads get `DLL_THREAD_ATTACH`.
/// Then it calls `RtlUserThreadStart(routine, argument)`.
///
/// Entered with RDI = the thread's start routine, RSI = its argument (the
/// PEB for the first thread), RDX = the executable's base. Poly's own
/// convention (both sides are ours).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn LdrInitializeThunk() -> ! {
    let (routine, argument, exe_base): (u64, u64, u64);
    // SAFETY: the kernel set these registers before entering us.
    unsafe {
        core::arch::asm!("", out("rdi") routine, out("rsi") argument, out("rdx") exe_base, options(nostack));
    }
    let teb = lock::teb();
    let peb = lock::peb();
    static STARTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, core::sync::atomic::Ordering::AcqRel) {
        // A later thread: TLS and DLL_THREAD_ATTACH.
        let callouts = with_loader(|l, p| l.thread_attach(p, teb)).unwrap_or_default();
        let _ = run_callouts(&callouts);
    } else {
        heap::init();
        let callouts = with_loader(|l, p| {
            l.init_ldr_data(p, peb)?;
            // ntdll itself and the executable are already mapped; register
            // them, then bind the executable.
            let ntdll_base = module_base_from_peb(peb);
            if ntdll_base != 0 {
                l.add_mapped(p, ntdll_base, &path_wide("\\??\\C:\\Windows\\System32\\ntdll.dll"), true)?;
            }
            let exe = l.add_mapped(p, exe_base, &path_wide("\\??\\C:\\image.exe"), false)?;
            l.process_init(p, exe, teb)
        });
        match callouts {
            Ok(list) => {
                if let Err(failed) = run_callouts(&list) {
                    let unload = with_loader(|l, _| l.fail_load(&poly_ldr::LoadOp::default(), failed));
                    let _ = unload;
                    // SAFETY: a static import failed: end the process.
                    unsafe { syscalls::NtTerminateProcess(syscalls::CURRENT_PROCESS, NtStatus(0xC000_0142)) };
                }
            }
            Err(status) => {
                // SAFETY: the process cannot start.
                unsafe { syscalls::NtTerminateProcess(syscalls::CURRENT_PROCESS, status) };
            }
        }
    }
    // SAFETY: hand off to the thread's start routine; it never returns here.
    unsafe { RtlUserThreadStart(routine, argument) }
}

/// `RtlUserThreadStart`: call the thread's routine and, when it returns, end
/// the thread with its result (Learn: the base thread entry).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlUserThreadStart(routine: u64, argument: u64) -> ! {
    type Start = unsafe extern "system" fn(u64) -> u32;
    // SAFETY: `routine` is the entry point the kernel gave us.
    let code = unsafe { core::mem::transmute::<u64, Start>(routine)(argument) };
    // SAFETY: never returns.
    unsafe { RtlExitUserThread(NtStatus(code)) }
}

/// `RtlExitUserThread`: `DLL_THREAD_DETACH`, free this thread's TLS, then
/// `NtTerminateThread`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlExitUserThread(status: NtStatus) -> ! {
    let teb = lock::teb();
    let callouts = with_loader(|l, _| l.thread_detach());
    let _ = run_callouts(&callouts);
    with_loader(|l, p| l.thread_exit(p, teb));
    // SAFETY: ends the current thread.
    unsafe { syscalls::NtTerminateThread(syscalls::CURRENT_THREAD, status) };
    loop {
        core::hint::spin_loop();
    }
}

/// `RtlExitUserProcess` / `LdrShutdownProcess` then `NtTerminateProcess`:
/// run every module's `DLL_PROCESS_DETACH` in reverse initialisation order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlExitUserProcess(status: NtStatus) -> ! {
    let callouts = with_loader(|l, _| l.process_detach());
    let _ = run_callouts(&callouts);
    // SAFETY: ends the process.
    unsafe { syscalls::NtTerminateProcess(syscalls::CURRENT_PROCESS, status) };
    loop {
        core::hint::spin_loop();
    }
}

/// The base of the module that holds `LdrInitializeThunk` (ntdll itself),
/// found by walking down from this function's address to the `MZ` header.
fn ntdll_base() -> u64 {
    let here = LdrInitializeThunk as *const () as u64;
    let mut page = here & !0xFFF;
    for _ in 0..0x4000 {
        // SAFETY: pages of our own image are readable; we stop at MZ.
        if unsafe { (page as *const u16).read() } == 0x5A4D {
            return page;
        }
        page = page.wrapping_sub(0x1000);
    }
    0
}

fn module_base_from_peb(_peb: u64) -> u64 {
    ntdll_base()
}

fn path_wide(s: &str) -> alloc::vec::Vec<u16> {
    s.encode_utf16().collect()
}
