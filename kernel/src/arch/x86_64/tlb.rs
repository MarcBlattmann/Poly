//! TLB shootdowns: making other CPUs drop translations that changed.
//!
//! The protocol is `poly_smp::tlb::Shootdowns` (host-tested): one request
//! at a time, a pending flag per target, an IPI on [`TLB_VECTOR`], and
//! waiting until every target has cleared its flag. The kernel's parts are
//! here: sending the IPIs, and what a target does with a request
//! ([`service`]):
//!
//! - for a user address space (named by its PML4's physical address): if
//!   that space is loaded on the CPU, `INVLPG` each page of the range, or
//!   reload CR3 without the no-flush bit (all translations of the current
//!   PCID go, SDM Vol. 3A §4.10.4.1) for a full flush; a CPU that has
//!   switched to another space meanwhile has nothing to do (with PCIDs, the
//!   space's PCID on that CPU was already forgotten by the sender, so its
//!   next load flushes; `addrspace.rs`);
//! - for kernel mappings (space 0): `INVLPG` each page (it drops global
//!   translations too), or toggle CR4.PGE for a full flush, which drops
//!   every translation including global ones (§4.10.4.1).
//!
//! **Waiting.** The sender runs with interrupts off from sending to the
//! last acknowledgement (it must not move to another CPU meanwhile), and
//! may hold a spinlock (an address space's table lock). A target with
//! interrupts off cannot take the IPI, so every place where a CPU waits
//! with interrupts off serves its own mailbox: spinning on a spinlock
//! (`sync`), waiting for a shootdown's acknowledgements, parking in
//! `smp::Exclusive` ([`super::wait_hook`]). Two CPUs that shoot at each
//! other, or one shooting while the other spins on the lock it holds, thus
//! both make progress.

use super::cpu::{self, InterruptsOff};
use super::irq::TLB_VECTOR;
use super::percpu;
use poly_paging::ADDR_MASK;
use poly_smp::CpuSet;
use poly_smp::tlb::Shootdowns;
pub use poly_smp::tlb::{Flush, Request};

static SHOOTDOWNS: Shootdowns<{ percpu::MAX_CPUS }> = Shootdowns::new();

/// CR3 bit 63 with CR4.PCIDE: keep the PCID's translations on the load.
const CR3_NO_FLUSH: u64 = 1 << 63;

/// Carry out a shootdown waiting for this CPU, if there is one: from the
/// IPI's handler and from every wait loop that runs with interrupts off.
pub fn service() {
    SHOOTDOWNS.service(percpu::cpu_index(), apply);
}

fn apply(req: &Request) {
    if req.space == 0 {
        invalidate_kernel(req.flush);
    } else if cpu::read_cr3() & ADDR_MASK == req.space {
        invalidate_current(req.flush);
    }
}

/// Drop translations of the address space loaded on this CPU.
pub(super) fn invalidate_current(flush: Flush) {
    match flush.pages() {
        Some(pages) => pages.for_each(cpu::invlpg),
        None => {
            let cr3 = cpu::read_cr3() & !CR3_NO_FLUSH;
            // SAFETY: the same tables and PCID as now; only the flush of
            // the current PCID's translations is new.
            unsafe { cpu::write_cr3(cr3) };
        }
    }
}

/// Drop translations of kernel mappings, global ones included.
fn invalidate_kernel(flush: Flush) {
    match flush.pages() {
        Some(pages) => pages.for_each(cpu::invlpg),
        None => {
            let cr4 = cpu::read_cr4();
            if cr4 & cpu::CR4_PGE != 0 {
                // SAFETY: clearing and setting PGE again flushes every TLB
                // entry, global ones too, and changes no translation.
                unsafe {
                    cpu::write_cr4(cr4 & !cpu::CR4_PGE);
                    cpu::write_cr4(cr4);
                }
            } else {
                invalidate_current(Flush::All);
            }
        }
    }
}

/// Have the CPUs in `targets` (this one excepted) carry out `req`, and
/// wait until they have. The caller has already invalidated whatever this
/// CPU needs to.
pub fn shoot(targets: CpuSet, req: Request) {
    let _irq = InterruptsOff::new();
    let me = percpu::cpu_index();
    let notify = |set: CpuSet| {
        for c in set.iter() {
            if let Some(area) = percpu::cpu(c) {
                super::apic::send_ipi(area.apic_id(), TLB_VECTOR);
            }
        }
    };
    SHOOTDOWNS.shoot(me, targets, req, notify, service);
}

/// Invalidate kernel mappings in `flush` on every CPU (after a kernel
/// mapping was removed or made less permissive). Adding mappings needs
/// none: a translation that was not present is not cached.
pub fn flush_kernel(flush: Flush) {
    let _irq = InterruptsOff::new();
    invalidate_kernel(flush);
    shoot(super::smp::online_cpus(), Request { space: 0, flush });
}

/// Shootdowns sent by any CPU, and requests carried out by targets.
pub fn stats() -> (u64, u64) {
    SHOOTDOWNS.stats()
}
