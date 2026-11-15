//! The current thread id, read from the TEB, and the TEB/PEB pointers.
//! `NtCurrentTeb()` reads `gs:[0x30]` on x64 and `fs:[0x18]` in a 32-bit
//! process (`winnt.h`: `NT_TIB.Self`); `TEB.ClientId.UniqueThread` and the
//! PEB pointer are at the offsets of `poly_rtl::layout` (`winternl.h`, ADR
//! 0063, 0186).

use poly_rtl::layout::NATIVE;

/// The calling thread's id and the TEB/PEB, via the TEB.
pub struct TID;

impl TID {
    /// `TEB.ClientId.UniqueThread`.
    pub fn with(&self) -> u64 {
        // SAFETY: the TEB is valid; the id is pointer-sized.
        unsafe { ((teb() + NATIVE.teb_client_id + NATIVE.ptr) as *const usize).read() as u64 }
    }
}

/// The current TEB (`gs:[0x30]`).
#[cfg(target_arch = "x86_64")]
pub fn teb() -> u64 {
    // SAFETY: GS is the TEB.
    unsafe {
        let teb: u64;
        core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, pure, readonly));
        teb
    }
}

/// The current TEB (`fs:[0x18]`, a 32-bit thread's, ADR 0186).
#[cfg(target_arch = "x86")]
pub fn teb() -> u64 {
    // SAFETY: the kernel bases FS at the TEB of every 32-bit thread.
    unsafe {
        let teb: u32;
        core::arch::asm!("mov {}, fs:[0x18]", out(reg) teb, options(nostack, pure, readonly));
        u64::from(teb)
    }
}

/// The PEB (`TEB.ProcessEnvironmentBlock`).
pub fn peb() -> u64 {
    // SAFETY: the TEB is valid and holds the PEB pointer.
    unsafe { ((teb() + NATIVE.teb_peb) as *const usize).read() as u64 }
}
