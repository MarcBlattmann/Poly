//! The current thread id, read from the TEB, and the TEB/PEB pointers.
//! `NtCurrentTeb()` reads `gs:[0x30]` (`winnt.h`: `NT_TIB.Self`);
//! `TEB.ClientId.UniqueThread` is at TEB + 0x48 and the PEB pointer at
//! TEB + 0x60 (`winternl.h`, ADR 0063).

/// The calling thread's id and the TEB/PEB, via the GS-based TEB.
pub struct TID;

impl TID {
    /// `TEB.ClientId.UniqueThread`.
    pub fn with(&self) -> u64 {
        // SAFETY: the kernel sets GS to the TEB for every NT thread.
        unsafe {
            let tid: u64;
            core::arch::asm!("mov {}, gs:[0x48]", out(reg) tid, options(nostack, pure, readonly));
            tid
        }
    }
}

/// The current TEB (`gs:[0x30]`).
pub fn teb() -> u64 {
    // SAFETY: GS is the TEB.
    unsafe {
        let teb: u64;
        core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, pure, readonly));
        teb
    }
}

/// The PEB (`TEB.ProcessEnvironmentBlock`, TEB + 0x60).
pub fn peb() -> u64 {
    // SAFETY: the TEB is valid and holds the PEB pointer at 0x60.
    unsafe { ((teb() + 0x60) as *const u64).read() }
}
