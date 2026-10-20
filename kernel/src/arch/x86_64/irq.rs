//! External interrupts: the vector map, handler registration, dispatch and
//! end-of-interrupt.
//!
//! ```text
//! 0x00-0x1f  CPU exceptions (idt.rs)
//! 0x20-0x2f  legacy 8259s, remapped here and masked: only a spurious IRQ 7 / 15 can arrive
//! 0x30-0xef  devices: allocate_vector (I/O APIC routes, MSI, MSI-X)
//! 0xf0       local APIC timer: the scheduler tick
//! 0xf1-0xfd  reserved for IPIs (SMP)
//! 0xfe       local APIC error
//! 0xff       local APIC spurious interrupt (never gets an EOI)
//! ```
//!
//! The local APIC delivers the highest pending priority class (vector / 16)
//! first, so the tick and IPIs outrank devices. Handlers never nest: every
//! IDT entry is an interrupt gate.
//!
//! # Drivers
//!
//! [`allocate_vector`]`(handler, data)` reserves a device vector and installs
//! `handler`, which is then called as `handler(data)` for every interrupt on
//! it, with interrupts disabled, before the EOI. Then route something to the
//! vector: [`msi_message`] gives the address / data pair for an MSI or MSI-X
//! entry, [`route_isa_irq`] and [`route_gsi`] program an I/O APIC input.
//! Handlers must be short and must not block (no `Mutex`, `sleep`, `join`);
//! waking a thread through a `WaitQueue` is fine. [`free_vector`] releases
//! the vector once the device can no longer raise it.

use super::apic::{self, RouteError};
use crate::kprintln;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use poly_irq::{MsiMessage, Polarity, Trigger};

pub const PIC_BASE: u8 = 0x20;
pub const FIRST_DEVICE_VECTOR: u8 = 0x30;
pub const LAST_DEVICE_VECTOR: u8 = 0xef;
pub const TIMER_VECTOR: u8 = 0xf0;
pub const ERROR_VECTOR: u8 = 0xfe;
/// Bits 3:0 are hardwired to 1 on some older APICs, so 0xff works on all.
pub const SPURIOUS_VECTOR: u8 = 0xff;

/// An interrupt handler: called with the `data` given to [`allocate_vector`].
pub type Handler = fn(usize);

/// One vector's registration. `handler` is 0 when free, [`CLAIMED`] while
/// [`allocate_vector`] fills in `data`, else a [`Handler`].
struct Slot {
    handler: AtomicUsize,
    data: AtomicUsize,
}

const CLAIMED: usize = 1;

static SLOTS: [Slot; 256] = [const { Slot { handler: AtomicUsize::new(0), data: AtomicUsize::new(0) } }; 256];

/// Interrupts on the local APIC's spurious vector.
static SPURIOUS: AtomicU64 = AtomicU64::new(0);
/// Interrupts on a vector nobody has registered.
static UNHANDLED: AtomicU64 = AtomicU64::new(0);
/// Interrupts on the 8259 vectors.
static LEGACY: AtomicU64 = AtomicU64::new(0);
/// Local APIC error interrupts.
static ERRORS: AtomicU64 = AtomicU64::new(0);
/// Interrupts taken on each vector (32-255; the exceptions are not counted).
/// TODO(smp): per CPU, summed when read.
static COUNTS: [AtomicU64; 256] = [const { AtomicU64::new(0) }; 256];
/// Handlers running on this CPU (0 or 1: they do not nest).
/// TODO(smp): per CPU.
static DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Reserve a device vector and install `handler(data)` on it. `None` when
/// every device vector is taken.
pub fn allocate_vector(handler: Handler, data: usize) -> Option<u8> {
    for vector in FIRST_DEVICE_VECTOR..=LAST_DEVICE_VECTOR {
        let slot = &SLOTS[usize::from(vector)];
        if slot.handler.compare_exchange(0, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            slot.data.store(data, Ordering::Relaxed);
            // Publishes `data` to `dispatch`, which loads the handler first.
            slot.handler.store(handler as usize, Ordering::Release);
            return Some(vector);
        }
    }
    None
}

/// Uninstall the handler of a vector from [`allocate_vector`] and make the
/// vector available again. An interrupt still pending on it is counted as
/// unhandled. False if it was not allocated.
pub fn free_vector(vector: u8) -> bool {
    if !(FIRST_DEVICE_VECTOR..=LAST_DEVICE_VECTOR).contains(&vector) {
        return false;
    }
    let old = SLOTS[usize::from(vector)].handler.swap(0, Ordering::AcqRel);
    old > CLAIMED
}

/// The MSI / MSI-X message that raises `vector` on the boot CPU (fixed
/// delivery, physical destination, edge). `None` before the local APIC is
/// set up or if its ID does not fit an MSI address (above 255).
pub fn msi_message(vector: u8) -> Option<MsiMessage> {
    MsiMessage::fixed(apic::interrupt_destination()?, vector).ok()
}

/// Allocate a vector for `handler(data)` and route ISA IRQ `irq` to it
/// through the I/O APIC, with the polarity and trigger mode the MADT gives.
pub fn route_isa_irq(irq: u8, handler: Handler, data: usize) -> Result<u8, RouteError> {
    let route = apic::isa_route(irq).ok_or(RouteError::NoSuchGsi)?;
    route_gsi(route.gsi, route.polarity, route.trigger, handler, data)
}

/// Allocate a vector for `handler(data)` and route global system interrupt
/// `gsi` to it. Undo with [`unroute_gsi`].
pub fn route_gsi(
    gsi: u32,
    polarity: Polarity,
    trigger: Trigger,
    handler: Handler,
    data: usize,
) -> Result<u8, RouteError> {
    let vector = allocate_vector(handler, data).ok_or(RouteError::NotReady)?;
    match apic::route_gsi(gsi, vector, polarity, trigger) {
        Ok(()) => Ok(vector),
        Err(e) => {
            free_vector(vector);
            Err(e)
        }
    }
}

/// Mask `gsi` and free the vector [`route_gsi`] gave it.
pub fn unroute_gsi(gsi: u32, vector: u8) -> Result<(), RouteError> {
    apic::mask_gsi(gsi)?;
    free_vector(vector);
    Ok(())
}

/// Raise `vector` on this CPU through the local APIC (an IPI to self).
/// False before the local APIC is set up or for a reserved vector.
pub fn send_self_ipi(vector: u8) -> bool {
    apic::send_self_ipi(vector)
}

/// Whether this CPU is running an interrupt handler (blocking is not
/// allowed there). False again by the time the timer preempts a thread.
pub fn in_interrupt() -> bool {
    DEPTH.load(Ordering::Relaxed) != 0
}

/// Counters for the boot log and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub spurious: u64,
    pub unhandled: u64,
    pub legacy: u64,
    pub errors: u64,
}

/// Interrupts taken on `vector` since boot.
pub fn count(vector: u8) -> u64 {
    COUNTS[usize::from(vector)].load(Ordering::Relaxed)
}

/// Whether a device vector has a handler from [`allocate_vector`].
pub fn is_allocated(vector: u8) -> bool {
    (FIRST_DEVICE_VECTOR..=LAST_DEVICE_VECTOR).contains(&vector)
        && SLOTS[usize::from(vector)].handler.load(Ordering::Relaxed) > CLAIMED
}

pub fn stats() -> Stats {
    Stats {
        spurious: SPURIOUS.load(Ordering::Relaxed),
        unhandled: UNHANDLED.load(Ordering::Relaxed),
        legacy: LEGACY.load(Ordering::Relaxed),
        errors: ERRORS.load(Ordering::Relaxed),
    }
}

/// Called by the IDT stub of every vector from 32 up, with interrupts
/// disabled, on the interrupted thread's stack.
pub(super) fn dispatch(vector: u8) {
    DEPTH.fetch_add(1, Ordering::Relaxed);
    COUNTS[usize::from(vector)].fetch_add(1, Ordering::Relaxed);
    match vector {
        // Not an interrupt at all: the APIC withdrew it after starting to
        // deliver it, nothing is in service, and an EOI would end the wrong
        // one (SDM §11.9).
        SPURIOUS_VECTOR => {
            SPURIOUS.fetch_add(1, Ordering::Relaxed);
        }
        PIC_BASE..FIRST_DEVICE_VECTOR => {
            LEGACY.fetch_add(1, Ordering::Relaxed);
            apic::legacy_interrupt(vector - PIC_BASE);
        }
        TIMER_VECTOR => {
            super::time::on_tick();
            apic::eoi();
            DEPTH.fetch_sub(1, Ordering::Relaxed);
            // After the EOI, on this thread's own stack (see preempt_tick):
            // may switch threads and return much later.
            crate::thread::on_timer_tick();
            return;
        }
        ERROR_VECTOR => {
            let esr = apic::local_apic().map_or(0, |a| a.error_status().0);
            if ERRORS.fetch_add(1, Ordering::Relaxed) < 8 {
                kprintln!("apic: error interrupt, ESR {esr:#x}");
            }
            apic::eoi();
        }
        v => {
            let slot = &SLOTS[usize::from(v)];
            let handler = slot.handler.load(Ordering::Acquire);
            if handler > CLAIMED {
                // SAFETY: values above CLAIMED are only ever stored by
                // `allocate_vector`, from a `Handler`.
                let handler = unsafe { core::mem::transmute::<usize, Handler>(handler) };
                handler(slot.data.load(Ordering::Relaxed));
            } else if UNHANDLED.fetch_add(1, Ordering::Relaxed) < 8 {
                kprintln!("irq: interrupt on vector {v:#x}, which has no handler");
            }
            apic::eoi();
        }
    }
    DEPTH.fetch_sub(1, Ordering::Relaxed);
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;
use core::sync::atomic::Ordering::SeqCst;

/// Bit `v - 64 * i` of word `i`: vectors whose handler saw its own number.
static SEEN: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static MISMATCHED: AtomicU64 = AtomicU64::new(0);

fn note_vector(expected: usize) {
    let me = DEPTH.load(Ordering::Relaxed);
    if me != 1 {
        MISMATCHED.fetch_add(1, SeqCst);
    }
    SEEN[expected / 64].fetch_or(1 << (expected % 64), SeqCst);
}

fn seen(v: u8) -> bool {
    SEEN[usize::from(v) / 64].load(SeqCst) & (1 << (v % 64)) != 0
}

/// Wait up to a second for something an interrupt does, with interrupts on.
fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let start = crate::time::now_ns();
    while crate::time::now_ns() - start < crate::time::NANOS_PER_SEC {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    done()
}

kernel_test! {
    fn every_device_vector_reaches_its_handler() -> TestResult {
        // Each vector's handler gets its own number as `data`; a self-IPI on
        // each must arrive at that handler, which checks the IDT stub pushed
        // the right vector.
        for word in &SEEN {
            word.store(0, SeqCst);
        }
        MISMATCHED.store(0, SeqCst);
        let mut allocated = [false; 256];
        let result = (|| {
            for v in FIRST_DEVICE_VECTOR..=LAST_DEVICE_VECTOR {
                if SLOTS[usize::from(v)].handler.load(SeqCst) != 0 {
                    continue; // in use by a driver
                }
                let got = allocate_vector(note_vector, usize::from(v)).ok_or("could not allocate a free vector")?;
                if got != v {
                    return Err("allocate_vector skipped a free vector");
                }
                allocated[usize::from(v)] = true;
                if !is_allocated(v) {
                    return Err("an allocated vector does not say so");
                }
                let before = count(v);
                if !send_self_ipi(v) {
                    return Err("self-IPI refused");
                }
                if !wait_for(|| seen(v)) {
                    crate::kprintln!("irq: vector {v:#x} did not arrive");
                    return Err("a self-IPI did not reach the vector's handler");
                }
                if count(v) != before + 1 {
                    return Err("the vector's interrupt count did not go up by one");
                }
            }
            if MISMATCHED.load(SeqCst) != 0 {
                return Err("a handler ran outside interrupt context");
            }
            Ok(())
        })();
        for v in FIRST_DEVICE_VECTOR..=LAST_DEVICE_VECTOR {
            if allocated[usize::from(v)] && !free_vector(v) {
                return Err("free_vector did not find an allocated vector");
            }
        }
        result
    }
}

kernel_test! {
    fn vector_allocation_runs_out_and_frees() -> TestResult {
        let mut taken = [0u8; 256];
        let mut n = 0;
        while let Some(v) = allocate_vector(note_vector, 0) {
            if !(FIRST_DEVICE_VECTOR..=LAST_DEVICE_VECTOR).contains(&v) || taken[..n].contains(&v) {
                return Err("allocate_vector handed out a reserved or duplicate vector");
            }
            taken[n] = v;
            n += 1;
        }
        let all_freed = taken[..n].iter().all(|&v| free_vector(v));
        if n == 0 || !all_freed {
            return Err("no vector to allocate, or one could not be freed");
        }
        if free_vector(taken[0]) || free_vector(TIMER_VECTOR) {
            return Err("free_vector accepted a vector that is not allocated");
        }
        let again = allocate_vector(note_vector, 0).ok_or("a freed vector could not be allocated again")?;
        free_vector(again);
        Ok(())
    }
}

kernel_test! {
    fn unregistered_and_spurious_vectors_are_survived() -> TestResult {
        let before = stats();
        // A real interrupt on a vector with no handler: counted, and
        // acknowledged, or the APIC would hold back everything at or below
        // its priority from now on.
        let v = allocate_vector(note_vector, 0).ok_or("no free vector")?;
        free_vector(v);
        if !send_self_ipi(v) {
            return Err("self-IPI refused");
        }
        if !wait_for(|| stats().unhandled > before.unhandled) {
            return Err("an interrupt on an unregistered vector was not counted");
        }
        if apic::local_apic().is_some_and(|a| a.is_in_service(v)) {
            return Err("an unregistered vector was left in service (no EOI)");
        }
        // The spurious vector: counted, no EOI. (Raised here by software;
        // the CPU enters the same stub.)
        // SAFETY: vector 0xff has an interrupt gate whose handler only
        // counts and returns.
        unsafe { core::arch::asm!("int 0xff", options(nomem, nostack)) };
        if stats().spurious <= before.spurious {
            return Err("the spurious vector was not counted");
        }
        // Interrupts still flow: the timer keeps ticking.
        let ticks = crate::time::ticks();
        if !wait_for(|| crate::time::ticks() > ticks) {
            return Err("timer interrupts stopped after an unhandled interrupt");
        }
        Ok(())
    }
}

kernel_test! {
    fn msi_messages_target_the_boot_cpu() -> TestResult {
        let id = apic::interrupt_destination().ok_or("no local APIC")?;
        let m = msi_message(0x41).ok_or("no MSI message for a device vector")?;
        if m.address != 0xfee0_0000 | (u64::from(id) << 12) || m.data != 0x41 {
            return Err("MSI message does not target the boot CPU's APIC");
        }
        if msi_message(3).is_some() {
            return Err("MSI message for an exception vector");
        }
        Ok(())
    }
}

/// The 8254 PIT's channel 0 data and mode/command ports (Intel 8254
/// Programmable Interval Timer datasheet), for the ISA routing test.
const PIT_CHANNEL0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;
/// Its input clock, 1.193182 MHz.
const PIT_HZ: u32 = 1_193_182;

static PIT_INTERRUPTS: AtomicU64 = AtomicU64::new(0);

fn count_pit_interrupt(_: usize) {
    PIT_INTERRUPTS.fetch_add(1, Ordering::Relaxed);
}

/// Program PIT channel 0: `mode` 2 (rate generator) or 0 (one shot, which
/// leaves it quiet), lobyte/hibyte access, binary count.
fn program_pit(mode: u8, count: u16) {
    // SAFETY: the PIT's ports; channel 0 drives only ISA IRQ 0, which is
    // masked everywhere except while the test below routes it.
    unsafe {
        super::port::outb(PIT_COMMAND, 0b0011_0000 | mode << 1);
        super::port::outb(PIT_CHANNEL0, count as u8);
        super::port::outb(PIT_CHANNEL0, (count >> 8) as u8);
    }
}

kernel_test! {
    /// ISA IRQ 0 (the PIT) through the I/O APIC input the MADT's override
    /// names (GSI 2 on PCs and QEMU).
    fn isa_irq_arrives_on_its_overridden_gsi() -> TestResult {
        let route = apic::isa_route(0).ok_or("ISA IRQ 0 has no route")?;
        PIT_INTERRUPTS.store(0, SeqCst);
        program_pit(2, (PIT_HZ / 1000) as u16);
        let vector = route_isa_irq(0, count_pit_interrupt, 0).map_err(|_| "could not route ISA IRQ 0")?;
        let result = (|| {
            let entry = apic::gsi_entry(route.gsi).ok_or("no redirection entry for IRQ 0's GSI")?;
            if entry.vector() != vector || entry.is_masked() || entry.trigger() != route.trigger {
                return Err("the redirection entry does not match the route");
            }
            let start = crate::time::now_ns();
            while PIT_INTERRUPTS.load(SeqCst) < 2 {
                if crate::time::now_ns() - start > crate::time::NANOS_PER_SEC {
                    return Err("no PIT interrupt arrived through the I/O APIC");
                }
                core::hint::spin_loop();
            }
            crate::kprintln!("irq: ISA IRQ 0 -> GSI {} -> vector {vector:#x}: delivered", route.gsi);
            Ok(())
        })();
        unroute_gsi(route.gsi, vector).map_err(|_| "could not mask IRQ 0 again")?;
        program_pit(0, 0);
        result
    }
}
