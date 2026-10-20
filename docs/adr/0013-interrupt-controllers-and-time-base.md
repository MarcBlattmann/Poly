# 0013: Interrupt controllers and the time base in one host-tested crate

**Status:** Accepted

## Context

Phase 1 needs interrupts from the local APIC and I/O APIC (with the legacy
8259s masked), and a time base: a calibrated TSC, a monotonic clock and APIC
timer interrupts. These pieces depend on each other. The APIC timer is
calibrated against the HPET, the TSC against the HPET too, and the clock
falls back to the HPET when the TSC cannot be trusted. Like the PCI and
virtio code ([ADR 0009](0009-drivers-behind-access-traits.md)), most of the
work is register encoding and arithmetic that can be tested on the host.

## Decision

- **One crate, `crates/poly-irq`**, holds the local APIC, I/O APIC, 8259, HPET,
  CPUID decoding, calibration and the monotonic clock. It is x86-specific, so
  the kernel uses it only from `kernel/src/arch/x86_64/`.
- **Access traits.** MMIO reuses `poly_pci::Mmio`, so the kernel's single
  MMIO implementation serves PCI, virtio, the APICs and the HPET. The crate
  adds `Msr` (`rdmsr`/`wrmsr`) and `PortIo` (`in`/`out`). The HPET driver uses
  only 32-bit accesses, which the HPET specification allows. It reads the
  64-bit main counter high, low, high, and runs periodic timers in 32-bit
  comparator mode, so each comparator write is a single access.
- **xAPIC and x2APIC behind one interface.** `ApicRegisters` takes xAPIC
  register offsets. `XApic` maps them onto the MMIO page and `X2Apic` onto MSRs
  0x800 + offset / 16. Only the ICR and the ID width differ, and those are
  handled in the backends. `LocalApic` holds all other logic once, and a test
  checks that both backends produce identical behaviour against the same
  simulated APIC.
- **No dependency on `poly-acpi`.** Overrides and I/O APIC bases come in as
  plain numbers (`IsaOverride { bus, source, gsi, flags }`), so the kernel
  copies them out of the MADT. The two crates stay independent.
- **Time base policy.** The TSC is the clock source only when CPUID reports
  it invariant *and* calibration succeeded; otherwise the HPET (or another
  reference counter) is. `MonotonicClock::fall_back_to_reference` switches
  later without a jump. Readings never go backwards: the clock keeps the
  largest value it has handed out in one shared atomic.
- **Calibration.** A measurement brackets each reference read between two
  target reads, keeping the tightest of three. Measurements whose bracket
  uncertainty exceeds the tolerance are dropped. The rest are compared with
  their median, and those within tolerance are averaged, provided they are a
  majority of all measurements taken. The same routine calibrates the APIC
  timer, seen as an up-counter (`u32::MAX - current count`).
- **Conversion** uses a precomputed `mult`/`shift` pair (one 64×64→128
  multiply), with `shift` as large as possible. It is never above the exact
  result and at most 2 below it, and saturates instead of overflowing.

## Consequences

- The kernel's part of interrupt and timer bring-up is small: map the APIC
  page, each I/O APIC and the HPET block; implement `Msr` and `PortIo`; read
  CPUID; then call the init sequence. Everything else is unit-tested,
  including hostile cases: a stuck reference, stalls during calibration, an
  ICR that never clears, a version register claiming 256 I/O APIC inputs, and
  a 32-bit counter wrapping.
- A global atomic in the clock's read path is a contention point on many
  CPUs. Per-CPU clocks can replace it once SMP and profiling exist.
- x2APIC IDs above 255 cannot be targeted by I/O APIC redirection entries,
  which have an 8-bit destination. That needs interrupt remapping, which is
  out of scope.
