# 0009: Device drivers as host-tested crates behind access traits

**Status:** Accepted

## Context

Phase 1 brings the first device drivers: PCI enumeration, MSI/MSI-X, virtio
and then virtio-blk. Most of a driver is protocol logic (parsing
configuration space, walking capability lists, ring bookkeeping, status
handshakes), and that logic is where the bugs are. Testing it only by booting
QEMU is slow and cannot reach the error paths: a looping capability chain, a
device that reports a bogus used-ring index, a rejected feature set.

## Decision

- Bus and device protocol logic lives in `no_std`, allocation-free crates
  (`crates/poly-pci`, `crates/poly-virtio`, later one per device class), with
  no dependencies beyond each other.
- Hardware is reached only through small traits the kernel implements:
  `poly_pci::ConfigSpace` (configuration space, normally ECAM) and
  `poly_pci::Mmio` (a mapped register block). Their methods take `&self`,
  because device registers are shared state that changes on its own anyway.
- Memory shared with a device (virtqueues) is caller-provided as a pointer
  plus its bus address, so the kernel decides how DMA memory is allocated
  and mapped. The crates do the volatile accesses and memory fences.
- Unit tests simulate the device side from the specification's device
  requirements, including devices that break the rules.
- Every value read from a device is untrusted: no panics or out-of-bounds
  accesses on it, and every walk over device-provided links is bounded.

## Consequences

- `cargo test` covers drivers in milliseconds, including the hostile cases.
- The kernel side of a driver is mapping memory, implementing two traits and
  routing interrupts.
- Trait calls cost an indirection that the kernel avoids by using concrete
  types (generics), not `dyn`.
