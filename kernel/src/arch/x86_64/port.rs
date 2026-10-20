//! x86 I/O port access.

use core::arch::asm;

/// # Safety
/// Writing to an I/O port can have arbitrary hardware side effects.
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: caller guarantees the port write is valid.
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)) }
}

/// # Safety
/// Writing to an I/O port can have arbitrary hardware side effects.
pub unsafe fn outl(port: u16, value: u32) {
    // SAFETY: caller guarantees the port write is valid.
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags)) }
}

/// # Safety
/// Reading some I/O ports has side effects (e.g. acknowledging data).
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: caller guarantees the port read is valid.
    unsafe { asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags)) }
    value
}

/// # Safety
/// Reading some I/O ports has side effects (e.g. acknowledging data).
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    // SAFETY: caller guarantees the port read is valid.
    unsafe { asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags)) }
    value
}
