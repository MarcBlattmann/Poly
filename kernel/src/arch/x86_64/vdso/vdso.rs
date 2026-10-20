//! The vDSO of Linux processes on x86-64: a small ELF shared object the
//! kernel maps into every Linux process and announces with
//! `AT_SYSINFO_EHDR` (ADR 0044).
//!
//! It is not part of the kernel crate: `kernel/build.rs` compiles it with
//! the pinned `rustc` for `x86_64-unknown-none` (position independent),
//! links it with `vdso.lds` into a shared object whose only segment is
//! read and execute, keeps the symbols `vdso.ver` names (vdso(7): x86-64
//! exports `__vdso_clock_gettime`, `__vdso_getcpu`, `__vdso_gettimeofday`,
//! `__vdso_time` and `__vdso_clock_getres`, all in version `LINUX_2.6`)
//! and checks that it needs no relocations. The kernel embeds the result.
//!
//! The functions behave like the system calls of the same names: they
//! return 0 (or the time, for `time`) or `-errno`. They compute the time
//! from the TSC and the kernel's [`Snapshot`] in the data page mapped right
//! below the image (`[vvar]`, a [`Vvar`]), and make the system call
//! whenever they cannot: a clock they do not know, a snapshot that says
//! the TSC is not to be used, a null pointer (the system call then reports
//! `EFAULT`).

#![no_std]
#![no_main]
// The structures are only written, for user space to read.
#![allow(dead_code)]

use core::arch::asm;
use poly_vdso::{Snapshot, VClock, Vvar};

/// System call numbers (the x86-64 table).
const SYS_GETTIMEOFDAY: u64 = 96;
const SYS_TIME: u64 = 201;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_CLOCK_GETRES: u64 = 229;

/// `struct __kernel_timespec` (`linux/time_types.h`).
#[repr(C)]
pub struct Timespec {
    sec: i64,
    nsec: i64,
}

/// `struct __kernel_old_timeval` (`linux/time_types.h`).
#[repr(C)]
pub struct Timeval {
    sec: i64,
    usec: i64,
}

/// `struct timezone` (`linux/time.h`).
#[repr(C)]
pub struct Timezone {
    minuteswest: i32,
    dsttime: i32,
}

/// The data page: the page below the image's ELF header.
fn vvar() -> &'static Vvar {
    let ehdr: usize;
    // SAFETY: computes an address (the linker's `__ehdr_start`, the first
    // byte of the image), touching nothing.
    unsafe { asm!("lea {}, [rip + __ehdr_start]", out(reg) ehdr, options(nomem, nostack, preserves_flags, pure)) };
    // SAFETY: the kernel maps the data page, read-only, right below the
    // image for as long as the image is mapped; `Vvar` is atomics only, so
    // reading it while the kernel writes is defined (the seqlock discards
    // such reads).
    unsafe { &*((ehdr - poly_vdso::PAGE_SIZE) as *const Vvar) }
}

/// The time-stamp counter, not read before earlier instructions are done
/// (LFENCE before RDTSC, SDM Vol. 2B, RDTSC).
fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: LFENCE and RDTSC only read the counter; user mode may (the
    // kernel leaves CR4.TSD clear).
    unsafe { asm!("lfence", "rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    u64::from(hi) << 32 | u64::from(lo)
}

/// A system call with two arguments; the result as the kernel returns it.
///
/// # Safety
///
/// The call must be one whose arguments are user pointers or plain values:
/// the kernel checks them.
unsafe fn syscall2(number: u64, a: u64, b: u64) -> i64 {
    let ret: i64;
    // SAFETY: the `syscall` instruction with the Linux convention; it
    // clobbers rcx and r11 and nothing else the compiler knows of.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as i64 => ret,
            in("rdi") a,
            in("rsi") b,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        )
    };
    ret
}

/// The snapshot, if it lets the vDSO read the clock itself.
fn snapshot() -> Snapshot {
    vvar().read()
}

/// clock_gettime(2).
///
/// # Safety
///
/// `ts` must be null or point to writable memory for a `Timespec`, as for
/// the system call (a bad non-null pointer faults here instead of
/// returning `EFAULT`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vdso_clock_gettime(clock: i32, ts: *mut Timespec) -> i64 {
    if let Some(c) = VClock::from_id(clock)
        && !ts.is_null()
        && let Some(ns) = snapshot().clock_ns(c, rdtsc())
    {
        let (sec, nsec) = poly_vdso::timespec(ns);
        // SAFETY: the caller's pointer, not null.
        unsafe { ts.write(Timespec { sec, nsec }) };
        return 0;
    }
    // SAFETY: the system call checks the pointer.
    unsafe { syscall2(SYS_CLOCK_GETTIME, clock as i64 as u64, ts as u64) }
}

/// clock_getres(2): `res` may be null.
///
/// # Safety
///
/// As [`__vdso_clock_gettime`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vdso_clock_getres(clock: i32, res: *mut Timespec) -> i64 {
    let s = snapshot();
    if let Some(c) = VClock::from_id(clock)
        && s.coarse_resolution_ns != 0
    {
        if !res.is_null() {
            let (sec, nsec) = poly_vdso::timespec(s.resolution_ns(c));
            // SAFETY: the caller's pointer, not null.
            unsafe { res.write(Timespec { sec, nsec }) };
        }
        return 0;
    }
    // SAFETY: the system call checks the pointer.
    unsafe { syscall2(SYS_CLOCK_GETRES, clock as i64 as u64, res as u64) }
}

/// gettimeofday(2): either pointer may be null.
///
/// # Safety
///
/// As [`__vdso_clock_gettime`], for both pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vdso_gettimeofday(tv: *mut Timeval, tz: *mut Timezone) -> i64 {
    let s = snapshot();
    let Some(ns) = s.clock_ns(VClock::Realtime, rdtsc()) else {
        // SAFETY: the system call checks the pointers.
        return unsafe { syscall2(SYS_GETTIMEOFDAY, tv as u64, tz as u64) };
    };
    if !tv.is_null() {
        let (sec, usec) = poly_vdso::timeval(ns);
        // SAFETY: the caller's pointer, not null.
        unsafe { tv.write(Timeval { sec, usec }) };
    }
    if !tz.is_null() {
        // SAFETY: the caller's pointer, not null.
        unsafe { tz.write(Timezone { minuteswest: s.timezone.minuteswest, dsttime: s.timezone.dsttime }) };
    }
    0
}

/// time(2): the seconds of the wall clock, also stored at `t` if it is
/// not null.
///
/// # Safety
///
/// As [`__vdso_clock_gettime`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vdso_time(t: *mut i64) -> i64 {
    let Some(ns) = snapshot().clock_ns(VClock::Realtime, rdtsc()) else {
        // SAFETY: the system call checks the pointer.
        return unsafe { syscall2(SYS_TIME, t as u64, 0) };
    };
    let sec = poly_vdso::timespec(ns).0;
    if !t.is_null() {
        // SAFETY: the caller's pointer, not null.
        unsafe { t.write(sec) };
    }
    sec
}

/// getcpu(2): Poly runs Linux processes on one CPU and one NUMA node, so
/// both are 0. The third argument is unused, as on Linux since 2.6.24.
///
/// # Safety
///
/// `cpu` and `node` must each be null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __vdso_getcpu(cpu: *mut u32, node: *mut u32, _cache: *mut u8) -> i64 {
    if !cpu.is_null() {
        // SAFETY: the caller's pointer, not null.
        unsafe { cpu.write(0) };
    }
    if !node.is_null() {
        // SAFETY: the caller's pointer, not null.
        unsafe { node.write(0) };
    }
    0
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // Nothing here can panic (no indexing, no arithmetic that checks); if
    // it ever did, stop the thread with an invalid-opcode fault.
    // SAFETY: `ud2` raises #UD and never returns.
    unsafe { asm!("ud2", options(noreturn, nomem, nostack)) }
}
