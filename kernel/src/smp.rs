//! Several CPUs: bringing the others up, and what the rest of the kernel
//! asks about them. The machine-specific parts (start-up IPIs, the
//! trampoline, per-CPU tables, IPIs) are `arch::smp`; the policy of which
//! CPUs to start is `poly_smp::plan`, the scheduler's side `poly_sched`
//! (ADR 0050).
//!
//! Boot options: `poly.smp=off` keeps the kernel on the boot CPU,
//! `poly.cpus=<n>` starts at most `n` CPUs (the boot CPU included).

use crate::boot_info::BootInfo;

pub use crate::arch::smp::{Exclusive, Summary, online_cpus};
pub use poly_sched::CpuSet;

/// Start the other CPUs (after interrupts and the clock are up on the boot
/// CPU). Each joins the scheduler as it comes up.
pub fn init(boot: &BootInfo) -> Summary {
    let platform = crate::acpi::platform();
    crate::arch::smp::start_aps(platform.as_ref(), boot.arg("poly.smp"), boot.arg("poly.cpus"))
}

/// How many CPUs run the scheduler.
pub fn cpus_online() -> usize {
    online_cpus().len()
}

/// Park every other CPU until the guard is dropped (see
/// [`Exclusive`]): for code that must be the only one running, such as a
/// test comparing frame counts.
pub fn exclusive() -> Exclusive {
    Exclusive::new()
}

/// The running CPU's number.
pub fn this_cpu() -> usize {
    crate::thread::this_cpu()
}
