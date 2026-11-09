# 0151: macOS plan: an arm64 to x86-64 translator first, an aarch64 kernel port as groundwork

**Status:** Accepted (owner decision). Roadmap Phase 9: closes the ⚖️ item
"aarch64 kernel port or x86-64 only". The first pieces are ADR 0152 (A64
decoder and interpreter), ADR 0153 (Mach-O reader) and ADR 0154 (aarch64
boot stub).

## Context

Poly's kernel is x86-64 only, and the laptop of Phase 11 is an Intel or
AMD machine. The XNU personality so far speaks the x86-64 macOS system call
convention (`syscall`, class in the top byte of `rax`).

Mac apps are moving away from x86-64:

- Every Mac Apple sells has an arm64 CPU. Apple has said macOS 26 is the
  last release for Intel Macs.
- Universal ("fat") binaries still carry an x86-64 slice for now, but new
  apps and new versions of old ones ship arm64 only, and the share grows
  every year.
- An x86-64-only Poly could run only the x86-64 slices. That set shrinks
  while Poly is being built.

There were three ways to go:

1. **x86-64 only.** Run the x86-64 slice of fat binaries and nothing else.
   Cheapest, and the reachable part of macOS gets smaller every year.
2. **An aarch64 port of the kernel.** arm64 Mac apps run natively, but only
   on arm64 machines. It does nothing for the x86 PCs Poly targets. Apple's
   own machines would need drivers for undocumented hardware, which is a
   project of its own and out of reach under the clean-room rules.
3. **An arm64 to x86-64 translator.** arm64-only Mac apps run on x86 PCs,
   the machines Poly is for. It is the larger engineering task.

## Decision

The owner chose **both**: the translator, because it is required for
arm64-only Mac apps on x86 PCs, and the aarch64 port, as groundwork only
for now.

### Order

The translator comes first. The port is kept building and booting so the
kernel does not grow new x86 assumptions, but it gets no more than that
until the translator works.

1. **A64 decoder and interpreter** (`crates/poly-a64`, ADR 0152). The
   decoder turns an instruction word into a fully decoded `Inst` and knows
   nothing about execution, so the JIT reuses it unchanged. The interpreter
   is the reference: correct first, fast later. It is tested from the Arm
   Architecture Reference Manual and, as a black box, against
   `qemu-aarch64` running the same tiny static arm64 Linux programs.
2. **Mach-O reader** (`crates/poly-macho`, ADR 0153). Fat and thin files,
   load commands, symbols, dylib dependencies, chained fixups and the
   classic rebase and bind streams. Shared by the translator, our `dyld`
   and the port.
3. **aarch64 boot stub** (ADR 0154). Builds for `aarch64-unknown-none`,
   boots on QEMU `virt` (emulated, TCG), prints to the PL011 serial port,
   runs a small self-test, and has its own CI job. New code only under
   `kernel/src/arch/aarch64/`.
4. **An arch-portable kernel.** The x86-64 code behind `crate::arch` moves
   so a second architecture can plug in. The coordinator does this as its
   own change, not the agents of items 1 to 3.
5. Then, in this order: a block JIT from A64 to x86-64 on the decoder; the
   arm64 macOS system call convention in the XNU personality (`svc #0x80`,
   number in `x16`, carry flag for errors); our `dyld` loading arm64 Mach-O
   images into a translated process; 16 KiB page semantics; code caches that
   follow `MAP_JIT` and instruction cache invalidation, so apps with their
   own JIT (JavaScriptCore) work.

### Where the translator runs

In user space, in the translated process: the guest's registers and its
code cache live in that process, and its system calls go to the XNU
personality in the arm64 convention. The kernel stays an x86-64 kernel
that does not know A64. Later ADRs refine this.

### Clean-room limits

The Arm Architecture Reference Manual, Arm's other public specifications,
Apple's developer documentation and public SDK headers are the inputs.
`qemu-aarch64`, `clang` and `lld` may be used as black boxes (inputs and
outputs only) to build and run test programs. The source of QEMU, LLVM's
AArch64 code, Rosetta-related material, FEX-Emu, Box64, Dynarmic, Unicorn,
XNU, `dyld` and Darling is never read by implementers, and no Apple binary
is decompiled or disassembled.

## Costs

- **Translator.** The largest piece of macOS work after the frameworks.
  The interpreter is weeks of work; a JIT good enough for GUI apps is
  months. Hard parts: the NZCV flags (compute them lazily), NEON on SSE and
  AVX, floating point details (rounding modes, NaN propagation,
  flush-to-zero), the exclusive monitor and LSE atomics on x86 `lock`
  instructions, self-modifying and JIT-generated guest code, and pointer
  authentication (its hint-space instructions behave as NOPs; full `arm64e`
  is for Apple's own binaries, which Poly does not run). The memory model
  works in our favour: x86-64's ordering is stronger than arm64's, so
  barriers mostly translate to nothing.
- **Speed.** The interpreter is tens of times slower than native code. The
  JIT has to come within a small factor of native for GUI apps to be
  usable.
- **The aarch64 port.** The stub is small. The full port is not: every
  module under `kernel/src/arch/x86_64/` needs a counterpart (paging,
  exceptions, GIC, timers, SMP over PSCI, user mode and system call entry,
  FP/SIMD state, the vDSO), and the Linux personality needs the arm64
  system call table. Windows on ARM is out of scope. Real hardware means
  UEFI and ACPI arm64 machines, not Apple's.
- **CI.** One more job: the stub's boot on QEMU `virt` under TCG, a few
  seconds of boot plus a build.

## Consequences

- arm64-only Mac apps become reachable on the x86 PCs Poly targets, at the
  price of a translator.
- The XNU personality serves two system call conventions: x86-64 for x86-64
  slices, arm64 for translated code.
- The kernel keeps building and booting for aarch64, so x86-only shortcuts
  in new code show up early.
- Phase 9 gains milestones for the translator and the port in
  [ROADMAP.md](../ROADMAP.md).
