//! Copying to and from user memory, safely: [`copy_from_user`],
//! [`copy_to_user`] and [`strncpy_from_user`].
//!
//! A user pointer may be anything: unmapped, read-only, pointing into the
//! kernel. Each function first checks that the whole range lies in the
//! user half (below [`USER_END`]), so the kernel can never be made to read
//! or write its own memory on a process's behalf. The copy itself then runs
//! in a small assembly routine that may fault: if a page is not mapped, or
//! a write hits a read-only page (CR0.WP makes ring 0 honour that), the
//! page-fault handler finds the faulting instruction in [`FIXUPS`] and
//! resumes at the routine's fixup label instead of panicking, and the
//! function returns [`Efault`].
//!
//! **SMAP.** With CR4.SMAP the kernel faults on every access to a user page
//! unless RFLAGS.AC is set, so a stray kernel pointer into user memory
//! cannot be exploited. The routines run between `STAC` and `CLAC`
//! ([`UserAccess`]); interrupt entry clears AC (`idt.rs`) and the
//! interrupted copy gets it back with its RFLAGS on return. Without SMAP
//! (QEMU's default CPU) the instructions do not exist and are skipped; the
//! range checks still apply.
//!
//! **Demand paging.** A fault in a copy routine may be a page that belongs
//! to the thread's memory map but is not present yet. [`page_fault`] asks
//! `user::page_fault` to bring it in first and retries the instruction
//! (`rep movsb` resumes where it stopped); only a fault it cannot resolve
//! goes to the fixup. That needs interrupts on (resolving may block on the
//! memory map's mutex, later on a disk read), so a copy made with
//! interrupts off (under a spinlock) does not fault pages in and fails
//! with [`Efault`] on a page that is not present yet. A copy must never be
//! made while holding the memory map's mutex.

use super::addrspace::USER_END;
use super::cpu;
use super::idt::TrapFrame;
use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicBool, Ordering};

/// A user pointer did not lead to accessible user memory: EFAULT in Linux
/// terms, STATUS_ACCESS_VIOLATION in NT's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Efault;

/// Whether CR4.SMAP is set (read by the interrupt entry, `idt.rs`).
pub(super) static SMAP: AtomicBool = AtomicBool::new(false);

global_asm!(
    r#"
    .section .text.user_copy, "ax", @progbits

    /* usize user_copy_bytes(dst: rdi, src: rsi, len: rdx): copy len bytes;
       returns the number of bytes NOT copied (0 on success). rep movsb
       keeps rcx at the bytes still to go when it faults. */
    .global user_copy_bytes
user_copy_bytes:
    mov rcx, rdx
    rep movsb
    xor eax, eax
    ret
    .global user_copy_bytes_fixup
user_copy_bytes_fixup:
    mov rax, rcx
    ret

    /* isize user_strncpy(dst: rdi, src: rsi, max: rdx): copy bytes up to
       and including the first NUL, at most max; returns the length of the
       string (the NUL's index), max if there was no NUL in the first max
       bytes, or -1 on a fault. */
    .global user_strncpy
user_strncpy:
    xor eax, eax
2:
    cmp rax, rdx
    je 3f
    movzx ecx, byte ptr [rsi + rax]
    mov byte ptr [rdi + rax], cl
    test cl, cl
    je 3f
    inc rax
    jmp 2b
3:
    ret
    .global user_strncpy_fixup
user_strncpy_fixup:
    mov rax, -1
    ret
    /* u64 user_cmpxchg32(addr: rdi, expected: esi, new: edx): an atomic
       compare-exchange on the user word; returns the value found (32 bits),
       or u64::MAX on a fault. */
    .global user_cmpxchg32
user_cmpxchg32:
    mov eax, esi
    lock cmpxchg dword ptr [rdi], edx
    ret
    .global user_cmpxchg32_fixup
user_cmpxchg32_fixup:
    mov rax, -1
    ret
    .global user_copy_end
user_copy_end:
"#
);

unsafe extern "sysv64" {
    fn user_copy_bytes(dst: *mut u8, src: *const u8, len: usize) -> usize;
    fn user_strncpy(dst: *mut u8, src: *const u8, max: usize) -> isize;
    fn user_cmpxchg32(addr: *mut u32, expected: u32, new: u32) -> u64;
    static user_cmpxchg32_fixup: u8;
    static user_copy_bytes_fixup: u8;
    static user_strncpy_fixup: u8;
    static user_copy_end: u8;
}

/// One routine that may fault on user memory: faults at `start..end`
/// resume at `fixup`.
struct Fixup {
    start: u64,
    end: u64,
    fixup: u64,
}

/// The exception table: every user-copy routine and where its faults go.
/// Each routine's fixup label directly follows its body.
fn fixups() -> [Fixup; 3] {
    let addr = |p: *const u8| p as u64;
    [
        Fixup {
            start: user_copy_bytes as *const () as u64,
            end: addr(&raw const user_copy_bytes_fixup),
            fixup: addr(&raw const user_copy_bytes_fixup),
        },
        Fixup {
            start: user_strncpy as *const () as u64,
            end: addr(&raw const user_strncpy_fixup),
            fixup: addr(&raw const user_strncpy_fixup),
        },
        Fixup {
            start: user_cmpxchg32 as *const () as u64,
            end: addr(&raw const user_cmpxchg32_fixup),
            fixup: addr(&raw const user_cmpxchg32_fixup),
        },
    ]
}

/// RFLAGS.IF.
const RFLAGS_IF: u64 = 1 << 9;

/// A page fault in ring 0. If it hit user memory inside a user-copy
/// routine, either bring the page in (demand paging) and retry, or resume
/// at the routine's fixup; returns `true` then. Returns `false` for every
/// other fault, which is the kernel's own.
pub(super) fn page_fault(frame: &mut TrapFrame) -> bool {
    let addr = cpu::read_cr2();
    let Some(fixup) = fixup(frame.rip, addr) else { return false };
    if frame.rflags & RFLAGS_IF != 0
        && let Some(access) = super::user::fault_access(frame.error_code)
    {
        // SAFETY: the interrupted copy ran with interrupts on, so it held
        // no spinlock and no `InterruptsOff` that this would break; CR2 has
        // been read. The trap frame is on this thread's stack, so blocking
        // or being preempted here is like doing so in the copy itself.
        unsafe { cpu::enable_interrupts() };
        let resolved = crate::user::page_fault_in(addr, access, true).is_ok();
        cpu::disable_interrupts();
        if resolved {
            return true;
        }
    }
    frame.rip = fixup;
    true
}

/// Where to resume after a page fault at `rip` on address `addr` in ring 0,
/// if it hit user memory inside a user-copy routine. Faults on kernel
/// addresses (a bad kernel buffer) are bugs and stay fatal.
fn fixup(rip: u64, addr: u64) -> Option<u64> {
    if addr >= USER_END {
        return None;
    }
    let end = (&raw const user_copy_end) as u64;
    let table = fixups();
    debug_assert!(table.iter().all(|f| f.start < f.end && f.end <= end));
    table.iter().find(|f| (f.start..f.end).contains(&rip)).map(|f| f.fixup)
}

/// RFLAGS.AC set (with SMAP) while alive: ring 0 may touch user pages.
struct UserAccess(());

impl UserAccess {
    fn open() -> UserAccess {
        if SMAP.load(Ordering::Relaxed) {
            // SAFETY: STAC exists (CR4.SMAP is only set when CPUID reports
            // SMAP) and only sets RFLAGS.AC. Not `nomem`: the copy must stay
            // between STAC and CLAC.
            unsafe { asm!("stac", options(nostack)) }
        }
        UserAccess(())
    }
}

impl Drop for UserAccess {
    fn drop(&mut self) {
        if SMAP.load(Ordering::Relaxed) {
            // SAFETY: as in `open`; CLAC only clears RFLAGS.AC.
            unsafe { asm!("clac", options(nostack)) }
        }
    }
}

/// Whether `[addr, addr + len)` lies entirely in the mappable user range.
pub fn is_user_range(addr: u64, len: u64) -> bool {
    addr.checked_add(len).is_some_and(|end| end <= USER_END)
}

/// Copy `dst.len()` bytes from the user address `src` (in the address
/// space that is loaded) into `dst`. On [`Efault`] part of `dst` may have
/// been written.
pub fn copy_from_user(dst: &mut [u8], src: u64) -> Result<(), Efault> {
    if !is_user_range(src, dst.len() as u64) {
        return Err(Efault);
    }
    if dst.is_empty() {
        return Ok(());
    }
    let _access = UserAccess::open();
    // SAFETY: `dst` is a valid kernel buffer of that length; `src` is a user
    // range, and any fault reading it is caught by the fixup.
    let left = unsafe { user_copy_bytes(dst.as_mut_ptr(), src as *const u8, dst.len()) };
    if left == 0 { Ok(()) } else { Err(Efault) }
}

/// Copy `src` to the user address `dst` (in the address space that is
/// loaded). On [`Efault`] part of it may have been written.
pub fn copy_to_user(dst: u64, src: &[u8]) -> Result<(), Efault> {
    if !is_user_range(dst, src.len() as u64) {
        return Err(Efault);
    }
    if src.is_empty() {
        return Ok(());
    }
    let _access = UserAccess::open();
    // SAFETY: as in `copy_from_user`, the other way round; writes to a
    // read-only or missing user page fault and are caught.
    let left = unsafe { user_copy_bytes(dst as *mut u8, src.as_ptr(), src.len()) };
    if left == 0 { Ok(()) } else { Err(Efault) }
}

/// Atomically compare the aligned 32-bit word at the user address `addr`
/// (in the address space that is loaded) with `expected` and, if equal,
/// replace it with `new` (`LOCK CMPXCHG`, SDM Vol. 2A): another CPU running
/// a thread of the same process sees either the old or the new word, never
/// a mix of a read and a write. Returns the value it found. A page that is
/// not present is brought in first when interrupts are on, like for a copy.
pub fn cmpxchg_user_u32(addr: u64, expected: u32, new: u32) -> Result<u32, Efault> {
    if !addr.is_multiple_of(4) || !is_user_range(addr, 4) {
        return Err(Efault);
    }
    let _access = UserAccess::open();
    // SAFETY: an aligned user address; a fault on it is caught by the
    // fixup (the routine is in the exception table).
    let found = unsafe { user_cmpxchg32(addr as *mut u32, expected, new) };
    u32::try_from(found).map_err(|_| Efault)
}

/// Copy the NUL-terminated string at the user address `src` into `dst`,
/// NUL included, reading at most `dst.len()` bytes. Returns the string's
/// length (without the NUL, which is at `dst[len]`), or `dst.len()` if no
/// NUL came within `dst.len()` bytes: the string is longer than the buffer
/// and `dst` holds its start, unterminated. Reading stops at the end of
/// user space, which counts as a fault.
pub fn strncpy_from_user(dst: &mut [u8], src: u64) -> Result<usize, Efault> {
    if src >= USER_END {
        return Err(Efault);
    }
    // Stop at the end of user space rather than ever reading past it.
    let room = (USER_END - src).min(dst.len() as u64) as usize;
    let len = {
        let _access = UserAccess::open();
        // SAFETY: `dst` is valid for `room` bytes; `[src, src + room)` is a
        // user range, and faults reading it are caught.
        unsafe { user_strncpy(dst.as_mut_ptr(), src as *const u8, room) }
    };
    match usize::try_from(len) {
        Err(_) => Err(Efault),
        Ok(n) if n == room && room < dst.len() => Err(Efault),
        Ok(n) => Ok(n),
    }
}
