//! Poly's own `libSystem.B.dylib` and the sub-libraries it re-exports
//! (ADR 0258): `libsystem_kernel`, `libsystem_platform`, `libsystem_c`,
//! `libsystem_malloc`, `libsystem_pthread` and `libdyld`, each a Mach-O
//! dylib with Apple's install name, so programs linked against the macOS
//! SDK load them in place of Apple's. `cargo xtask libsystem` builds this
//! crate once per library (one feature each) for `x86_64-apple-darwin` and
//! `aarch64-apple-darwin` (ADR 0257: arm64 programs run translated) and
//! links every build with `rust-lld -flavor darwin`; the export lists are
//! in `exports/`.
//!
//! Inputs: the C standard, POSIX and the macOS manual pages for what the
//! functions do, the macOS SDK headers for constants and layouts, the
//! export names of the SDK's `.tbd` files (`coverage/macos/api/`), and the
//! spec sheets of `specs/macos/` for the system calls. No part of Apple's
//! libraries, dyld or XNU was read or disassembled.
//!
//! v0 is single-threaded: nothing takes a lock yet (threads come with
//! `bsdthread_create`, ADR 0246).

#![no_std]

mod mem;

#[cfg(feature = "c")]
mod c;
#[cfg(feature = "compiler_rt")]
mod compiler_rt;
#[cfg(feature = "info")]
mod info;
#[cfg(feature = "m")]
mod m;
#[cfg(feature = "dyld")]
mod dyld;
#[cfg(feature = "kernel")]
mod kernel;
#[cfg(feature = "malloc")]
mod malloc;
#[cfg(feature = "platform")]
mod platform;
#[cfg(feature = "pthread")]
mod pthread;
#[cfg(feature = "system")]
mod system;

/// A library-wide variable. v0 has one thread, so nothing locks it; each
/// user takes it once per call.
#[allow(dead_code)]
struct Global<T>(core::cell::UnsafeCell<T>);

// SAFETY: one thread in v0 (see the crate documentation).
unsafe impl<T> Sync for Global<T> {}

#[allow(dead_code)]
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(core::cell::UnsafeCell::new(v))
    }

    /// The value.
    ///
    /// # Safety
    /// No other reference to it is alive (one thread, one use per call).
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self) -> &mut T {
        // SAFETY: the caller's contract.
        unsafe { &mut *self.0.get() }
    }
}

#[cfg(not(feature = "kernel"))]
unsafe extern "C" {
    /// libsystem_kernel's `__error()`: the calling thread's `errno`.
    fn __error() -> *mut i32;
}

/// Set `errno`.
#[allow(dead_code)]
fn set_errno(e: i32) {
    #[cfg(feature = "kernel")]
    use kernel::__error;
    // SAFETY: `__error` returns the calling thread's errno variable.
    unsafe { *__error() = e };
}

/// The current `errno`.
#[allow(dead_code)]
fn errno() -> i32 {
    #[cfg(feature = "kernel")]
    use kernel::__error;
    // SAFETY: as for `set_errno`.
    unsafe { *__error() }
}

/// A raw BSD call with up to three arguments; `Err(errno)` on failure.
/// Used where a library must not depend on another one (the panic path).
#[allow(dead_code)]
#[cfg(target_arch = "x86_64")]
fn raw3(n: u64, a: u64, b: u64, c: u64) -> Result<u64, u64> {
    let rax: u64;
    let cf: u8;
    // SAFETY: a Darwin system call: arguments in rdi, rsi, rdx; rcx and
    // r11 are clobbered by `syscall`; the carry flag tells failure.
    unsafe {
        core::arch::asm!(
            "syscall",
            "setc {cf}",
            cf = out(reg_byte) cf,
            inlateout("rax") poly_libsystem::sys::unix(n) => rax,
            in("rdi") a,
            in("rsi") b,
            inlateout("rdx") c => _,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    if cf != 0 { Err(rax) } else { Ok(rax) }
}

/// [`raw3`] on arm64: `svc #0x80`, the number in `x16`.
#[allow(dead_code)]
#[cfg(target_arch = "aarch64")]
fn raw3(n: u64, a: u64, b: u64, c: u64) -> Result<u64, u64> {
    let x0: u64;
    let cf: u64;
    // SAFETY: a Darwin system call: arguments in x0..x2, the number in
    // x16; x1 may come back changed (a second result); the carry flag
    // tells failure.
    unsafe {
        core::arch::asm!(
            "svc #0x80",
            "cset {cf}, cs",
            cf = out(reg) cf,
            inlateout("x0") a => x0,
            inlateout("x1") b => _,
            in("x2") c,
            in("x16") n,
            options(nostack),
        );
    }
    if cf != 0 { Err(x0) } else { Ok(x0) }
}

/// Print `msg` on stderr and end the process with SIGABRT (or status 134
/// if the signal cannot be sent). What `abort` does, without depending on
/// libsystem_c.
#[allow(dead_code)]
fn die(msg: &[u8]) -> ! {
    let _ = raw3(4, 2, msg.as_ptr() as u64, msg.len() as u64);
    if let Ok(pid) = raw3(20, 0, 0, 0) {
        let _ = raw3(37, pid, poly_libsystem::sys::SIGABRT as u64, 1);
    }
    let _ = raw3(1, 134, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    die(b"poly libSystem: internal error\n")
}
