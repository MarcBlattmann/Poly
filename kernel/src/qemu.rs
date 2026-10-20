//! QEMU `isa-debug-exit` device, used by the automated boot test to report
//! a result through QEMU's exit status: status = (code << 1) | 1.

use crate::arch::port::outl;
use core::sync::atomic::{AtomicBool, Ordering};

const PORT: u16 = 0xf4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ExitCode {
    /// QEMU exits with status 33.
    Success = 0x10,
    /// QEMU exits with status 35.
    Failure = 0x11,
}

/// Set from the `poly.qemu_exit` kernel argument. Without it the kernel never
/// pokes the port, so the same image is safe on real hardware.
static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Exit QEMU if enabled; otherwise return.
pub fn exit(code: ExitCode) {
    if ENABLED.load(Ordering::Relaxed) {
        // SAFETY: only reached when booted under QEMU with the exit device.
        unsafe { outl(PORT, code as u32) }
    }
}
