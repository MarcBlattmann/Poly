use core::arch::asm;

pub fn halt_forever() -> ! {
    loop {
        // SAFETY: cli; hlt only stops this CPU.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) }
    }
}

pub fn breakpoint() {
    // SAFETY: int3 traps to our breakpoint handler, which returns.
    unsafe { asm!("int3", options(nomem, nostack)) }
}

pub fn read_cr2() -> u64 {
    let v: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}
