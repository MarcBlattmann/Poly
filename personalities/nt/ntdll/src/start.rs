//! Process and thread start and end (ADR 0071): `LdrInitializeThunk`,
//! `RtlUserThreadStart`, `RtlExitUserThread`, `RtlExitUserProcess`.
//!
//! The kernel–`ntdll` start convention is Poly's own (both sides are ours):
//! every thread of a process starts at `LdrInitializeThunk` with RDI = its
//! start routine, RSI = its argument, RDX = the executable's base, and RSP
//! pointing at a zero return address (as if called, 8 bytes below a 16-byte
//! boundary), so a stack walk ends there. The first thread's start routine
//! is the executable's entry point and its argument the PEB.

use crate::exports::{LOADER_LOCK, LOADER_LOCK_DEBUG};
use crate::sys::SysPlatform;
use crate::{heap, lock, params, run_callouts, syscalls, with_loader, with_slot};
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_ldr::{Config, LoadOp, Loader};
use poly_ntobj::NtStatus;

/// `STATUS_DLL_INIT_FAILED` (`ntstatus.h`).
const STATUS_DLL_INIT_FAILED: NtStatus = NtStatus(0xC000_0142);

// The entry stub: move Poly's start registers into the x64 argument
// registers and call `ntdll_thread_init`, which never returns. It has unwind
// data (`.seh_*`) so a stack walk passes through it to the zero return
// address the kernel left above it.
global_asm!(
    r#"
    .text
    .globl LdrInitializeThunk
    .p2align 4
    .seh_proc LdrInitializeThunk
LdrInitializeThunk:
    sub rsp, 0x28
    .seh_stackalloc 0x28
    .seh_endprologue
    mov rcx, rdi
    mov r8, rdx
    mov rdx, rsi
    call {init}
    int3
    .seh_endproc
"#,
    init = sym ntdll_thread_init,
);

/// Whether the process has been initialised (the first thread did it).
static STARTED: AtomicBool = AtomicBool::new(false);

/// Where `LdrInitializeThunk` goes: initialise the process (first thread)
/// or the thread, then run the thread's routine.
unsafe extern "C" fn ntdll_thread_init(routine: u64, argument: u64, exe_base: u64) -> ! {
    let teb = lock::teb();
    if STARTED.swap(true, Ordering::AcqRel) {
        // A later thread: its TLS, then DLL_THREAD_ATTACH, under the
        // loader lock.
        LOADER_LOCK.enter(&SysPlatform);
        let callouts = with_loader(|l, p| l.thread_attach(p, teb)).unwrap_or_default();
        let _ = run_callouts(&callouts);
        LOADER_LOCK.leave(&SysPlatform);
    } else if let Err(status) = process_init(teb, exe_base) {
        crate::report_load_failure("process start", status);
        // SAFETY: the process cannot start: end it with the status.
        unsafe { syscalls::NtTerminateProcess(syscalls::CURRENT_PROCESS, status) };
    }
    // SAFETY: hand off to the thread's start routine; it never returns here.
    unsafe { RtlUserThreadStart(routine, argument) }
}

unsafe extern "system" {
    /// The generated stub (`syscalls.s`).
    fn NtQuerySystemInformation(class: u32, info: *mut u8, length: u32, returned: *mut u32) -> NtStatus;
}

/// Fill in the PEB fields programs read directly (`poly_rtl::peb`, ADR 0155): the process
/// heap, the reported OS version, the processor count, the executable's subsystem and the
/// loader-lock pointer.
fn init_peb(peb: u64, exe_base: u64) {
    // SAFETY: the PEB is the process's own page; the executable's headers are mapped.
    unsafe {
        ((peb + poly_rtl::peb::PROCESS_HEAP as u64) as *mut u64).write(heap::process_heap().0 as u64);
        // SystemBasicInformation (class 0): NumberOfProcessors is the byte at offset 56.
        let mut basic = [0u8; 64];
        let mut got = 0u32;
        let processors =
            if NtQuerySystemInformation(0, basic.as_mut_ptr(), 64, &mut got).0 == 0 { u32::from(basic[56]) } else { 1 };
        let nt = exe_base + u64::from(((exe_base + 0x3c) as *const u32).read_unaligned());
        let opt = nt + 24;
        let subsystem = (
            ((opt + 68) as *const u16).read_unaligned(),
            ((opt + 48) as *const u16).read_unaligned(),
            ((opt + 50) as *const u16).read_unaligned(),
        );
        // PEB.LoaderLock points at the lock `LdrLockLoaderLock` takes and the loader holds around
        // every DllMain and TLS callback; it is set here, before any of them runs.
        LOADER_LOCK.make_static(&LOADER_LOCK_DEBUG);
        let loader_lock = (&raw const LOADER_LOCK) as u64;
        for f in poly_rtl::peb::fields(poly_rtl::peb::Facts { processors, subsystem, loader_lock }) {
            let at = (peb + f.at as u64) as *mut u8;
            core::ptr::copy_nonoverlapping(f.value.to_le_bytes().as_ptr(), at, f.width);
        }
    }
}

/// The first thread: the process heap, the loader (with the search order
/// from the process parameters), `ntdll` and the executable registered,
/// the executable's imports bound, TLS set up, and the
/// `DLL_PROCESS_ATTACH` callouts run, all under the loader lock.
fn process_init(teb: u64, exe_base: u64) -> Result<(), NtStatus> {
    heap::init();
    let peb = lock::peb();
    init_peb(peb, exe_base);
    let image = params::image_path(peb);
    let config = Config { dirs: params::dirs(peb) };
    with_slot(|slot| *slot = Some(Loader::new(config)));
    LOADER_LOCK.enter(&SysPlatform);
    let callouts = with_loader(|l, p| {
        l.init_ldr_data(p, peb)?;
        // The executable heads the load and memory order lists, then comes ntdll, as in every
        // Windows process (programs that walk `PEB.Ldr` take the first entry for the image).
        let exe = l.add_mapped(p, exe_base, &image, false)?;
        let ntdll = poly_ldr::path::wide("C:\\Windows\\System32\\ntdll.dll");
        l.add_mapped(p, ntdll_base(), &ntdll, true)?;
        l.process_init(p, exe, teb)
    });
    let result = match callouts {
        Ok(list) => match run_callouts(&list) {
            Ok(()) => Ok(()),
            Err(failed) => {
                // A static import's DllMain refused DLL_PROCESS_ATTACH: it
                // gets DLL_PROCESS_DETACH, and the process ends.
                let unload = with_loader(|l, _| l.fail_load(&LoadOp::default(), failed));
                let _ = run_callouts(&unload.callouts);
                Err(STATUS_DLL_INIT_FAILED)
            }
        },
        Err(status) => Err(status),
    };
    LOADER_LOCK.leave(&SysPlatform);
    crate::report_stubbed();
    result
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

/// `RtlExitUserThread`: `DLL_THREAD_DETACH` (under the loader lock), free
/// this thread's TLS, then `NtTerminateThread`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlExitUserThread(status: NtStatus) -> ! {
    let teb = lock::teb();
    LOADER_LOCK.enter(&SysPlatform);
    let callouts = with_loader(|l, _| l.thread_detach());
    let _ = run_callouts(&callouts);
    with_loader(|l, p| l.thread_exit(p, teb));
    LOADER_LOCK.leave(&SysPlatform);
    // SAFETY: ends the current thread.
    unsafe { syscalls::NtTerminateThread(syscalls::CURRENT_THREAD, status) };
    loop {
        core::hint::spin_loop();
    }
}

/// `RtlExitUserProcess`: run every module's `DLL_PROCESS_DETACH` in reverse
/// initialisation order under the loader lock, then `NtTerminateProcess`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RtlExitUserProcess(status: NtStatus) -> ! {
    LOADER_LOCK.enter(&SysPlatform);
    let callouts = with_loader(|l, _| l.process_detach());
    let _ = run_callouts(&callouts);
    // SAFETY: ends the process.
    unsafe { syscalls::NtTerminateProcess(syscalls::CURRENT_PROCESS, status) };
    loop {
        core::hint::spin_loop();
    }
}

unsafe extern "C" {
    /// The linker's pseudo-symbol at the image's own base (the MSVC
    /// linker's `__ImageBase`, which `lld-link` provides too).
    static __ImageBase: u8;
}

/// The base of `ntdll` itself (wherever the kernel mapped it).
fn ntdll_base() -> u64 {
    (&raw const __ImageBase) as u64
}
