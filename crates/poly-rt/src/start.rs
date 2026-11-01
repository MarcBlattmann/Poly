//! The entry point, the ABI note and the panic handler of a native program
//! (only on the native target).

use crate::abi::AbiNote;
use crate::{env, process};

/// Poly's ABI note: the kernel runs an ELF executable with it in the
/// native personality (`poly-binfmt`). A section named `.note.*` is an
/// `SHT_NOTE` section, which `rust-lld` puts into a `PT_NOTE` segment at
/// the start of the first page and keeps from garbage collection.
#[unsafe(link_section = ".note.poly.abi")]
#[used]
static ABI_NOTE: AbiNote = AbiNote::CURRENT;

// `_start`: the kernel enters here with `rsp` at `argc` (16-byte aligned)
// and every other register 0. Clear the frame pointer (the end of
// backtraces), pass the stack to `start`, keep the call aligned.
core::arch::global_asm!(
    ".globl _start",
    ".type _start, @function",
    "_start:",
    "xor ebp, ebp",
    "mov rdi, rsp",
    "and rsp, -16",
    "call {start}",
    "ud2",
    start = sym start,
);

unsafe extern "Rust" {
    /// The program's `main`, named by [`entry!`](crate::entry).
    fn __poly_rt_main() -> i32;
}

/// Read the initial stack at `sp`, run `main`, exit with its status.
///
/// # Safety
///
/// Only `_start` calls it, with the kernel's initial stack.
unsafe extern "C" fn start(sp: *const usize) -> ! {
    // SAFETY: the initial stack: argc, argv[argc], NULL, envp..., NULL,
    // valid for the life of the process.
    unsafe {
        let argc = *sp;
        let argv = sp.add(1).cast::<*const u8>();
        env::init(argc, argv, argv.add(argc + 1));
    }
    // SAFETY: `entry!` defines it with this signature.
    let code = unsafe { __poly_rt_main() };
    process::exit(code)
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    use core::fmt::Write;
    // Straight to the handle: formatting into a string could allocate, and
    // the allocator may be what panicked.
    let _ = writeln!(crate::io::Stderr, "panic: {info}");
    process::exit(101)
}
