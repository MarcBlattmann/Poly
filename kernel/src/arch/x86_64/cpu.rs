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

const RFLAGS_IF: u64 = 1 << 9;

/// Whether maskable interrupts are enabled on this CPU.
pub fn interrupts_enabled() -> bool {
    let flags: u64;
    // SAFETY: pushfq; pop only reads rflags (through the stack).
    unsafe { asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags)) }
    flags & RFLAGS_IF != 0
}

/// Interrupts disabled on this CPU until dropped, then restored to how they
/// were. Nests. Held across a context switch, it belongs to the thread that
/// created it: that thread restores its own state when it resumes.
#[must_use]
#[derive(Debug)]
pub struct InterruptsOff {
    was_enabled: bool,
}

impl InterruptsOff {
    pub fn new() -> InterruptsOff {
        let was_enabled = interrupts_enabled();
        // SAFETY: cli only masks interrupts on this CPU. Not `nomem`: memory
        // accesses must not move out of the critical section.
        unsafe { asm!("cli", options(nostack)) }
        InterruptsOff { was_enabled }
    }
}

impl Drop for InterruptsOff {
    fn drop(&mut self) {
        if self.was_enabled {
            // SAFETY: interrupts were enabled when this guard was made, so
            // the code that owns this CPU's interrupt state wants them on.
            unsafe { asm!("sti", options(nostack)) }
        }
    }
}
