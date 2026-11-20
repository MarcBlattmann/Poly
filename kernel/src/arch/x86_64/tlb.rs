//! TLB shootdowns: making other CPUs drop translations that changed.
//!
//! The protocol is `poly_smp::tlb::Mailboxes` (host-tested, ADR 0254): a
//! mailbox per CPU, any number of requests in flight, an IPI on
//! [`TLB_VECTOR`]. [`post`] sends without waiting and gives a ticket;
//! [`shoot`] posts and waits for it; [`post_quiet`] posts without an IPI.
//! A target empties its mailbox when the IPI arrives, in every wait loop, at
//! every context switch and at every timer tick. The kernel's
//! parts are here: sending the IPIs, and what a target does with a request
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
use poly_smp::tlb::Mailboxes;
pub use poly_smp::tlb::{Flush, Request};

/// A request posted to other CPUs, until they have carried it out.
pub type Ticket = poly_smp::tlb::Ticket<{ percpu::MAX_CPUS }>;

static MAILBOXES: Mailboxes<{ percpu::MAX_CPUS }> = Mailboxes::new();

/// CR3 bit 63 with CR4.PCIDE: keep the PCID's translations on the load.
const CR3_NO_FLUSH: u64 = 1 << 63;

/// Carry out a shootdown waiting for this CPU, if there is one: from the
/// IPI's handler and from every wait loop that runs with interrupts off.
pub fn service() {
    // A mailbox that overflowed: everything, global translations included.
    MAILBOXES.service(percpu::cpu_index(), apply, || invalidate_kernel(Flush::All));
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
    if let Some(t) = post(targets, req) {
        wait(&t);
    }
}

/// Post `req` to the CPUs in `targets` (this one excepted) and interrupt
/// them, without waiting: the ticket to [`wait`] for or ask about
/// ([`complete`]); `None` if there was nobody to tell.
pub fn post(targets: CpuSet, req: Request) -> Option<Ticket> {
    let _irq = InterruptsOff::new();
    MAILBOXES.post(percpu::cpu_index(), targets, req, interrupt)
}

/// [`post`] without interrupting anyone: the targets carry `req` out at
/// their next timer tick or context switch (at most a tick from now), with
/// whatever else came meanwhile, or sooner if somebody [`wait`]s for it.
pub fn post_quiet(targets: CpuSet, req: Request) -> Option<Ticket> {
    let _irq = InterruptsOff::new();
    MAILBOXES.post_quiet(percpu::cpu_index(), targets, req)
}

/// Send the shootdown IPI to the CPUs in `set`.
fn interrupt(set: CpuSet) {
    for c in set.iter() {
        if let Some(area) = percpu::cpu(c) {
            super::apic::send_ipi(area.apic_id(), TLB_VECTOR);
        }
    }
}

/// Whether every target of `t` has carried its request out.
pub fn complete(t: &Ticket) -> bool {
    MAILBOXES.complete(t)
}

/// Return once every target of `t` has carried its request out, serving
/// this CPU's own mailbox meanwhile (with interrupts off: it may hold a
/// spinlock another target spins on, and that one serves its mailbox too).
pub fn wait(t: &Ticket) {
    let _irq = InterruptsOff::new();
    // A quiet request's targets are interrupted now that someone waits.
    MAILBOXES.hurry(t, interrupt);
    MAILBOXES.wait(t, service);
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
    MAILBOXES.stats()
}
