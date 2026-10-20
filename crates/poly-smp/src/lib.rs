//! Multiprocessor logic the kernel needs to bring up and coordinate several
//! CPUs, without touching any of them:
//!
//! - [`plan`]: which processors to start, from the ACPI MADT's list, the
//!   boot processor's APIC ID and the `poly.smp=` / `poly.cpus=` options.
//! - [`trampoline`]: where the real-mode start-up code may live below 1 MiB,
//!   and the parameter block (temporary GDT, far pointers, control register
//!   values, the stack and entry point) the kernel fills in for each
//!   application processor (AP) before it sends the start-up IPIs.
//! - [`startup`]: the INIT-SIPI-SIPI sequence with the delays the Intel SDM
//!   (Vol. 3A §9.4.4.1) and the MultiProcessor Specification (appendix B.4)
//!   give, over a small trait the kernel implements with its local APIC.
//! - [`tlb`]: batching of TLB invalidations into one range or a full flush,
//!   and the mailbox protocol of a TLB shootdown: one request at a time,
//!   acknowledged by every target, with waiters that keep serving their own
//!   mailbox so two CPUs shooting at each other cannot deadlock.
//!
//! The scheduler's side (per-CPU run queues) is `poly-sched`; this crate
//! reuses its [`CpuSet`].
//!
//! Sources: Intel SDM Vol. 3A ch. 9 ("Processor Management and
//! Initialization": §9.4 multiple-processor initialization, §9.8-9.9 mode
//! switching), ch. 4 (§4.10.4 TLB invalidation, §4.10.5 propagation of
//! paging-structure changes to multiple processors) and ch. 11 (APIC, IPIs);
//! the MultiProcessor Specification 1.4, appendix B; ACPI 6.5 §5.2.12 (MADT).

#![no_std]

extern crate alloc;

pub mod plan;
pub mod startup;
pub mod tlb;
pub mod trampoline;

pub use poly_sched::{CpuSet, MAX_CPUS};
