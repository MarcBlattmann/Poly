# 0154: An aarch64 boot stub in its own package, on QEMU `virt`

**Status:** Accepted. Groundwork for the aarch64 kernel port chosen in
[ADR 0151](0151-macos-plan-arm64-translator-and-aarch64-port.md); nothing in the
x86-64 kernel changes.

## Context

ADR 0151 records the owner's decision on the macOS foundations: an
arm64-to-x86-64 translator first (required), and an aarch64 kernel port as
groundwork now. The port has to start somewhere that proves the toolchain, the
CPU bring-up and the boot tests work before the kernel is made
architecture-neutral (milestone M5, which moves what is x86 in `kernel/` behind
`crate::arch`).

`poly-kernel` cannot simply gain an aarch64 binary: its `build.rs` passes the
x86 linker script to every binary, builds the x86 vDSO and embeds the Windows
DLLs, and all its dependencies would have to build for aarch64 first. The goal
of this step is smaller: boot, print, run a self-test, exit with a status, in
CI.

## Decision

**A separate package and workspace** at `kernel/src/arch/aarch64/`
(`poly-kernel-aarch64`), with its own `.cargo/config.toml` (target
`aarch64-unknown-none`), linker script and `build.rs`. It is excluded from the
host workspace by living under `kernel/`, which already is. It depends only on
shared host-tested crates that build for aarch64: `poly-mem` (the buddy frame
allocator) and `poly-heap` (the heap), the two the x86 kernel uses. No
third-party crates. Everything that touches no hardware is a library target
(`poly_a64`) of the same package, built for the host too, where its unit tests
run: a devicetree reader, `MIDR_EL1`/`ESR_ELx` decoding, page table entries,
the PL011's register model, the semihosting call block, the self-test report.
`main.rs` and its modules hold what cannot be tested on the host: the assembly,
the system registers, the MMU switch.

**Boot protocol.** `qemu-system-aarch64 -M virt -cpu cortex-a72 -kernel <ELF>`
loads the ELF's segments at their addresses and starts the boot CPU at the entry
point with `x0` holding the devicetree address (the arm64 Linux protocol QEMU
documents applies to non-ELF kernels; the QEMU documentation says the blob is at
the start of RAM for bare-metal guests, so the stub tries `x0` and then
0x4000_0000 for the magic number). The CPU starts in EL1, or in EL2 with
`-machine virt,virtualization=on`; the stub handles both and drops from EL2 to
EL1 (`HCR_EL2.RW`, timer access, no FP traps, real ID registers) and does not
support EL3. RAM starts at 0x4000_0000 and the image is linked at 0x4020_0000,
so the blob (QEMU makes it 1 MiB) stays intact. Entry: park every CPU but CPU 0,
enable FP/SIMD (`CPACR_EL1`, compiled code uses it), set the stack and
`VBAR_EL1`, zero `.bss`, call Rust.

**Devices from the devicetree.** RAM (`/memory`), the PL011 (`arm,pl011`), the
command line (`/chosen/bootargs`, QEMU's `-append`) and the PSCI conduit come
from the blob, as the QEMU documentation asks guests to ("device locations may
change between QEMU versions"). Where the blob has none, QEMU `virt`'s documented
values are the fallback (RAM at 0x4000_0000, the PL011 at 0x0900_0000). The PL011
is driven polled, 115200 8N1 with the divisors from the TRM's formula (QEMU
ignores them).

**Exit status: Arm semihosting `SYS_EXIT`** (`HLT #0xF000`, operation 0x18,
`ADP_Stopped_ApplicationExit` with the status as subcode), with
`-semihosting-config enable=on,target=native`. QEMU ends with that status, which
`cargo xtask aarch64 test` checks: 0 pass, 1 failed check, 2 panic, 3 unexpected
exception. PSCI `SYSTEM_OFF` also stops the machine but carries no status, so it
is only the fallback when semihosting is off (the `HLT` is then an exception;
the stub catches it and powers off, through the conduit the devicetree names).
Semihosting works on QEMU's emulator (TCG) only, which is what CI uses on every
runner.

**Exceptions.** A 2 KiB aligned vector table of 16 entries saves a 288-byte trap
frame and calls one Rust handler. A self-test announces the exception class it
is about to cause; that class from the stub's own code is recorded and stepped
over; anything else prints a report (vector, class, `ESR`, `ELR`, `FAR`) and
exits with status 3.

**MMU.** A single level-1 table of 1 GiB blocks, 4 KiB granule, 39-bit
addresses: everything below RAM is Device-nGnRnE (the UART, GIC and virtio live
there), the gigabytes of RAM are Normal write-back cacheable, nothing above. RWX
and coarse: real paging replaces it.

**The self-test** (`ok <name>` / `not ok <name>` lines and a
`POLY-A64-SELFTEST: PASS` verdict, like the x86 kernel's `POLY-SELFTEST`): entry
level, vectors installed, FP/SIMD, the blob found and its RAM and UART in use,
`brk` and `svc` taken and returned from, the timer frequency and the counter
advancing, the MMU on with RAM still readable and writable and an unmapped
address giving a translation fault, the buddy allocator, small and large heap
allocations freed again. `-append poly.selftest.panic` ends the boot in a
deliberate panic, to see the panic path print its message and exit 2.

**xtask and CI.** `cargo xtask aarch64 build|run|test [--release]`: pure Rust and
`rust-lld`, so it builds on Windows and Linux hosts (`ci lint` checks xtask for
Windows as before); `test` runs the library's unit tests on the host and three
boots on TCG (EL1 with two CPUs, EL2, the panic), judged on the `ok` lines (every
required name must be present), the verdict and the exit status. Without
`qemu-system-aarch64` the boots are skipped with a message. It is the CI part
`aarch64` (`cargo xtask ci aarch64`, a job that installs `qemu-system-arm`),
a requirement of the `ci` job; `ci lint` runs rustfmt and clippy on the package.
`aarch64-unknown-none` is added to `rust-toolchain.toml`.

## Consequences

- aarch64 code, its build and its boot tests exist and are kept working in CI,
  with no change to the x86-64 kernel, its build or its boot tests.
- The stub is a second copy of the little a kernel needs first (boot,
  console, exceptions, heap). **M5 folds it in:** the module split already
  follows `crate::arch` (`boot`, `cpu`, `exceptions`, `paging`, `uart`,
  `semihosting`), `poly_a64`'s pure parts move to shared crates (or stay a
  `poly-arch-aarch64` crate), the stub's `a64_main` becomes the aarch64 producer
  of `BootInfo`, and the package, its linker script and the xtask target go away
  once `poly-kernel` builds for both targets.
- Single CPU: the others are parked. The identity map is RWX 1 GiB blocks, the
  stack has no guard page, and `ESR`/`FAR` are not decoded further than the class
  and the fault status.
- Semihosting needs TCG; a KVM or HVF boot of the stub would use the PSCI
  fallback and lose the exit status.

## What's next

In order: the GIC (distributor and redistributor or CPU interface from the
devicetree) and the generic timer's interrupts for a tick; real paging (4 KiB
pages, W^X, a higher half, per-process tables with ASIDs); secondary CPUs through
PSCI `CPU_ON`; then the kernel proper: `BootInfo` from the devicetree, the
scheduler's context switch, the system call entry (`SVC`) and user mode, and the
arm64 personality entry points the translator and the Mach-O loader need.
