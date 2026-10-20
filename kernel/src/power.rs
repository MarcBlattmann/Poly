//! Rebooting, halting and powering off the machine, after unmounting the
//! disk file systems clean. reboot(2) (ADR 0051) and the kernel shell's
//! `reboot` come here.

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

/// Write back and unmount the disks, then power off: ACPI soft-off (S5)
/// if the firmware describes it (QEMU then exits), else halt.
pub fn power_off(out: &mut dyn Write) -> ! {
    prepare_shutdown(out);
    if let Some(off) = acpi::platform().and_then(|p| p.soft_off) {
        let _ = writeln!(
            out,
            "poweroff: ACPI S5 (SLP_TYP {}/{} to PM1a {:#x}{})",
            off.sleep_types.0,
            off.sleep_types.1,
            off.pm1a.address,
            off.pm1b.map(|b| alloc::format!(", PM1b {:#x}", b.address)).unwrap_or_default()
        );
        settle();
        arch::reset::acpi_soft_off(&off);
        wait(RESET_WAIT_NS);
        let _ = writeln!(out, "poweroff: the machine is still on; halting instead");
    } else {
        let _ = writeln!(out, "poweroff: no ACPI soft-off (S5) on this machine; halting instead");
    }
    halt_now(out)
}

/// Write back and unmount the disks, then stop: interrupts off, every
/// CPU this kernel runs halted.
pub fn halt(out: &mut dyn Write) -> ! {
    prepare_shutdown(out);
    halt_now(out)
}

fn halt_now(out: &mut dyn Write) -> ! {
    let _ = writeln!(out, "System halted.");
    settle();
    arch::cpu::halt_forever()
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
