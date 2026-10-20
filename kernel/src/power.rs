//! Rebooting the machine, after unmounting the disk file systems clean.

use crate::{acpi, arch, block, fs, time};
use core::fmt::Write;
use poly_acpi::AddressSpace;

/// How long each reset method gets before the next one is tried.
const RESET_WAIT_NS: u64 = 500 * time::NANOS_PER_MILLI;

/// Write back the disks' caches, then reset the machine: through the ACPI
/// reset register if the FADT has one, else (or if that does nothing) the
/// 8042 keyboard controller, and finally a triple fault. Each step is
/// reported to `out` first.
pub fn reboot(out: &mut dyn Write) -> ! {
    prepare_shutdown(out);
    arch_reset(out)
}

/// What every way of going down does first: the disk file systems are
/// written back and unmounted clean (`fs::disk::shutdown`), then every
/// disk's cache is written back. A power-off (once ACPI sleep states can
/// be entered) calls this too.
pub fn prepare_shutdown(out: &mut dyn Write) {
    fs::disk::shutdown(out);
    for d in block::disks() {
        if let Err(e) = d.sync() {
            let _ = writeln!(out, "reboot: {}: writing back the cache failed: {e:?}", d.name());
        }
    }
}

/// Reset the machine, each way reported to `out` first.
fn arch_reset(out: &mut dyn Write) -> ! {
    if let Some(r) = acpi::platform().and_then(|p| p.reset) {
        let space = match r.register.space {
            AddressSpace::SystemIo => "port",
            AddressSpace::SystemMemory => "address",
            _ => "unsupported space, address",
        };
        let _ = writeln!(out, "reboot: ACPI reset register ({space} {:#x} <- {:#x})", r.register.address, r.value);
        settle();
        if arch::reset::acpi(&r) {
            wait(RESET_WAIT_NS);
        }
    }
    let _ = writeln!(out, "reboot: 8042 keyboard controller reset");
    settle();
    arch::reset::keyboard_controller();
    wait(RESET_WAIT_NS);
    let _ = writeln!(out, "reboot: triple fault");
    settle();
    arch::reset::triple_fault()
}

/// Let the message just written leave the UART before the machine resets.
fn settle() {
    wait(10 * time::NANOS_PER_MILLI);
}

fn wait(ns: u64) {
    if !time::is_running() {
        // No clock: a rough delay, long enough for the UART at any speed.
        for _ in 0..ns / 10 {
            core::hint::spin_loop();
        }
        return;
    }
    let start = time::now_ns();
    while time::now_ns() - start < ns {
        core::hint::spin_loop();
    }
}
