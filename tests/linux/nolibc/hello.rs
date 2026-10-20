//! A static Linux "hello world" without any C library, built with nothing
//! but the Rust toolchain the kernel uses (`cargo xtask` compiles it with
//! `rustc --target x86_64-unknown-none` into a static, non-PIE ELF
//! executable; see `tools/xtask/src/programs.rs`).
//!
//! It makes the first Linux system calls Poly implements (roadmap Phase 3),
//! each through the raw x86-64 `syscall` instruction (numbers from
//! `asm/unistd_64.h`, constants from the UAPI headers):
//!
//! 1. `brk(0)`, then `brk` one page further, and writes to the new page;
//! 2. `mmap` of an anonymous page, writes to it, `munmap`;
//! 3. `arch_prctl(ARCH_SET_FS)` to a word it reads back through `%fs`;
//! 4. `write(1, "Hello, world!\n")`;
//! 5. `exit_group(0)`, or `exit_group(1)` with a message on stderr if a
//!    step failed.
//!
//! Written for Poly from the public x86-64 psABI and the Linux man pages.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};

const SYS_WRITE: u64 = 1;
const SYS_MMAP: u64 = 9;
const SYS_MUNMAP: u64 = 11;
const SYS_BRK: u64 = 12;
const SYS_ARCH_PRCTL: u64 = 158;
const SYS_EXIT_GROUP: u64 = 231;

const PROT_READ: u64 = 0x1;
const PROT_WRITE: u64 = 0x2;
const MAP_PRIVATE: u64 = 0x02;
const MAP_ANONYMOUS: u64 = 0x20;
const ARCH_SET_FS: u64 = 0x1002;
const PAGE: u64 = 4096;

/// A system call with up to six arguments; returns `rax` (`-errno` on
/// failure).
///
/// # Safety
/// The call must not break the program's memory (an `munmap` of memory in
/// use, say).
unsafe fn syscall(nr: u64, a: [u64; 6]) -> u64 {
    let ret;
    // SAFETY: the x86-64 Linux convention: rcx and r11 are clobbered,
    // memory may be read and written by the kernel.
    unsafe {
        asm!("syscall", inlateout("rax") nr => ret, in("rdi") a[0], in("rsi") a[1], in("rdx") a[2],
             in("r10") a[3], in("r8") a[4], in("r9") a[5], lateout("rcx") _, lateout("r11") _,
             options(nostack));
    }
    ret
}

fn write(fd: u64, s: &[u8]) {
    // SAFETY: the buffer is valid for its length.
    unsafe { syscall(SYS_WRITE, [fd, s.as_ptr() as u64, s.len() as u64, 0, 0, 0]) };
}

fn exit(code: u64) -> ! {
    loop {
        // SAFETY: ends the program.
        unsafe { syscall(SYS_EXIT_GROUP, [code, 0, 0, 0, 0, 0]) };
    }
}

fn fail(what: &[u8]) -> ! {
    write(2, b"hello: ");
    write(2, what);
    write(2, b" failed\n");
    exit(1)
}

/// Something `%fs` points at.
static mut TLS: [u64; 2] = [0x5a5a_1234, 0];

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // SAFETY (all blocks below): each call maps, uses and unmaps memory
    // this program owns alone; nothing else refers to it.
    unsafe {
        // The program break: query, grow by a page, use it.
        let start = syscall(SYS_BRK, [0; 6]);
        if start == 0 || start > (1 << 47) {
            fail(b"brk(0)");
        }
        let grown = syscall(SYS_BRK, [start + PAGE, 0, 0, 0, 0, 0]);
        if grown != start + PAGE {
            fail(b"brk(grow)");
        }
        let heap = start as *mut u64;
        heap.write_volatile(42);
        if heap.read_volatile() != 42 {
            fail(b"writing the heap");
        }

        // An anonymous mapping.
        let prot = PROT_READ | PROT_WRITE;
        let page = syscall(SYS_MMAP, [0, PAGE, prot, MAP_PRIVATE | MAP_ANONYMOUS, u64::MAX, 0]);
        if page > u64::MAX - 4095 || page % PAGE != 0 {
            fail(b"mmap");
        }
        let p = page as *mut u64;
        if p.read_volatile() != 0 {
            fail(b"zeroed mmap memory");
        }
        p.write_volatile(7);
        if syscall(SYS_MUNMAP, [page, PAGE, 0, 0, 0, 0]) != 0 {
            fail(b"munmap");
        }

        // Thread-local storage the way a C library sets it up.
        let tls = &raw mut TLS;
        if syscall(SYS_ARCH_PRCTL, [ARCH_SET_FS, tls as u64, 0, 0, 0, 0]) != 0 {
            fail(b"arch_prctl(ARCH_SET_FS)");
        }
        let through_fs: u64;
        asm!("mov {}, qword ptr fs:[0]", out(reg) through_fs, options(nostack, readonly));
        if through_fs != 0x5a5a_1234 {
            fail(b"reading through %fs");
        }
    }
    write(1, b"Hello, world!\n");
    exit(0)
}

// The entry point: the kernel starts the program with `rsp` at `argc`
// (16-byte aligned, psABI §3.4.1); `main` gets a normal call frame.
global_asm!(".globl _start", "_start:", "xor ebp, ebp", "and rsp, -16", "call main", "ud2");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    fail(b"a panic")
}
