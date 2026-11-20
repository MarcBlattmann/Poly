//! `/usr/libexec/poly-a64`: the runtime of a translated arm64 XNU process
//! (ADR 0257).
//!
//! The kernel starts it in place of an arm64 Mach-O program, in the
//! program's XNU process, with the program already mapped (`poly-a64-xnu`'s
//! `load`) and `rdi` pointing to the hand-off block. It runs the program
//! with the block JIT (or the interpreter alone, `POLY_A64_ENGINE=interp`)
//! and sends its system calls to the XNU personality as x86-64 `syscall`s,
//! the same calls an x86-64 Mac program makes.
//!
//! It is not a native Poly program (no ABI note, no `poly-rt`): its process
//! is an XNU process, so its own calls are XNU calls too (`write` for
//! messages, `exit`). Its heap and code area are mapped by the kernel at
//! fixed addresses (`poly_a64_xnu::layout`).

#![no_std]
#![no_main]

extern crate alloc;

mod heap;

use poly_a64_jit::jit::Engine;
use poly_a64_jit::mem::GuestMem;
use poly_a64_xnu::run::{self, Bridge, Host};
use poly_a64_xnu::{EngineKind, Handoff};
use poly_abi::SyscallRegs;

/// `write` and `exit` in the x86-64 convention: class 2 (BSD) in the top
/// byte of `rax`.
const SYS_WRITE: u64 = 0x200_0004;
const SYS_EXIT: u64 = 0x200_0001;

// `_start`: the kernel enters with `rdi` = the hand-off block and `rsp` at
// the ELF initial stack. Clear the frame pointer, align the stack, call
// `start`.
core::arch::global_asm!(
    ".globl _start",
    ".type _start, @function",
    "_start:",
    "xor ebp, ebp",
    "and rsp, -16",
    "call {start}",
    "ud2",
    start = sym start,
);

/// One x86-64 XNU system call; the seventh and eighth arguments go on the
/// stack above the return address slot, where the personality reads them.
fn syscall(frame: &mut SyscallRegs, stack: [u64; 2]) {
    let flags: u64;
    // SAFETY: a system call. The kernel reads the registers and, for the
    // seventh and eighth arguments, the two words written to this asm's
    // own stack space; it clobbers rcx and r11. The stack pointer is back
    // where it was at the end.
    unsafe {
        core::arch::asm!(
            "sub rsp, 32",
            "mov [rsp + 8], {a6}",
            "mov [rsp + 16], {a7}",
            "syscall",
            "pushfq",
            "pop {flags}",
            "add rsp, 32",
            a6 = in(reg) stack[0],
            a7 = in(reg) stack[1],
            flags = lateout(reg) flags,
            inout("rax") frame.rax,
            in("rdi") frame.rdi,
            in("rsi") frame.rsi,
            inout("rdx") frame.rdx,
            in("r10") frame.r10,
            in("r8") frame.r8,
            in("r9") frame.r9,
            out("rcx") _,
            out("r11") _,
        );
    }
    frame.rflags = flags;
}

/// Write `s` to standard error.
fn say(s: &str) {
    let mut f =
        SyscallRegs { rax: SYS_WRITE, rdi: 2, rsi: s.as_ptr() as u64, rdx: s.len() as u64, ..Default::default() };
    syscall(&mut f, [0, 0]);
}

fn exit(status: i32) -> ! {
    let mut f = SyscallRegs { rax: SYS_EXIT, rdi: status as u64, ..Default::default() };
    syscall(&mut f, [0, 0]);
    // The kernel does not come back from exit.
    loop {
        core::hint::spin_loop();
    }
}

/// The guest's system calls go straight to the kernel: guest addresses
/// are this process's addresses.
struct Kernel;

impl Host for Kernel {
    fn syscall(&mut self, frame: &mut SyscallRegs, stack: [u64; 2], _mem: &mut GuestMem) {
        syscall(frame, stack);
    }
}

fn now() -> u64 {
    // SAFETY: RDTSC has no preconditions in user mode on Poly.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// The hand-off block at `at`.
fn read_handoff(at: u64) -> Option<Handoff> {
    // SAFETY: the kernel mapped the block at `at` readable; its first
    // three words give its length, which is checked against its limit.
    unsafe {
        let head = &*(at as *const [u8; 24]);
        let len = Handoff::length(head)?;
        if len as u64 > poly_a64_xnu::layout::HANDOFF_SIZE {
            return None;
        }
        Handoff::from_bytes(core::slice::from_raw_parts(at as *const u8, len))
    }
}

extern "C" fn start(handoff: u64) -> ! {
    let Some(h) = read_handoff(handoff) else {
        say("poly-a64: no hand-off block (this program is started by the kernel for arm64 Mach-O programs)\n");
        exit(127);
    };
    let size = h.window_end - h.window_lo;
    // SAFETY: the window is this process's memory at the guest's own
    // addresses; the kernel mapped every region of the hand-off (and the
    // anonymous area) there, and the runtime's own memory lies outside it.
    // The guest touches only pages its flags allow.
    let mut mem = unsafe { GuestMem::from_raw(h.window_lo, size, h.window_lo as *mut u8) };
    let mut cpu = match run::guest(&h, &mut mem) {
        Ok(cpu) => cpu,
        Err(e) => {
            say(&alloc::format!("poly-a64: {}: {e}\n", h.path));
            exit(127);
        }
    };
    let mut engine = match h.engine {
        // SAFETY: the kernel mapped the code area readable, writable and
        // executable for this runtime alone; it lives as long as the process.
        EngineKind::Jit if h.code_size > 0 => unsafe { Engine::with_code(h.code as *mut u8, h.code_size as usize) },
        _ => Engine::interpreter(),
    };
    let mut bridge = Bridge::new(Kernel, &h);
    let t0 = now();
    let stop = run::run(&mut engine, &mut cpu, &mut mem, &mut bridge);
    let cycles = now().wrapping_sub(t0);
    if h.stats {
        say(&run::stats_line(h.engine, &engine, bridge.calls, cycles));
    }
    if let Some(status) = bridge.exit {
        exit(status);
    }
    say(&run::stop_message(&h.path, stop, &cpu, bridge.bad_svc));
    // The process ends with the signal a native program would get
    // (SIGSEGV, SIGBUS, SIGILL, SIGTRAP, SIGSYS).
    bridge.kill_self(run::stop_signal(stop, &mem, bridge.bad_svc), &mut mem);
    // The guest catches or ignores it: its handler cannot run in translated
    // code yet (signal frames come next), so the runtime faults on purpose
    // and the process still ends abnormally.
    // SAFETY: ud2 raises #UD, which ends this process.
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    // Formatting into a string could allocate, and the allocator may be
    // what panicked: write the pieces as they come.
    struct Stderr;
    impl core::fmt::Write for Stderr {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            say(s);
            Ok(())
        }
    }
    let _ = core::fmt::Write::write_fmt(&mut Stderr, format_args!("poly-a64: panic: {info}\n"));
    exit(101)
}
