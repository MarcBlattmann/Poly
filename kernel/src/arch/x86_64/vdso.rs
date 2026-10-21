//! The x86-64 side of the vDSO (ADR 0044): the image `build.rs` builds
//! from `vdso/vdso.rs`, and the TSC conversion it reads the clock with.
//!
//! The vDSO reads the TSC itself only while the kernel's clock does: a
//! calibrated TSC that runs at a constant rate (invariant, or a virtual
//! CPU's, as under QEMU: ADR 0013, 0072). Otherwise it makes the system
//! call.

use super::cpu::{self, InterruptsOff};
use poly_irq::time::Scale;
use poly_vdso::Conversion;

/// The vDSO image: an ELF shared object, one read-execute segment at
/// address 0, no relocations (checked by `build.rs`).
pub static IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/vdso.so"));

/// The conversion the vDSO may use: the clock's own while it reads the
/// TSC, else `None` (the vDSO makes the system call).
pub fn conversion() -> Option<Conversion> {
    let c = super::time::tsc_conversion()?;
    Some(Conversion { origin: c.origin, mult: c.mult, shift: c.shift, offset_ns: c.offset_ns })
}

/// A conversion from the calibrated TSC rate that agrees with the clock
/// now, even where the clock does not read the TSC: lets the tests run the
/// vDSO's TSC path under emulators that do not report an invariant TSC.
/// `None` if the TSC was not calibrated.
pub fn calibrated_conversion() -> Option<Conversion> {
    let hz = super::time::info()?.tsc_hz?;
    let scale = Scale::ticks_to_ns(hz)?;
    let _irq = InterruptsOff::new();
    let origin = cpu::rdtsc();
    let offset_ns = super::time::now_ns();
    Some(Conversion { origin, mult: scale.mult(), shift: scale.shift(), offset_ns })
}
