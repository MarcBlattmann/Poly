# 0152: An A64 decoder and reference interpreter (`poly-a64`), tested against `qemu-aarch64` as a black box

**Status:** Accepted. Step one of the arm64 to x86-64 translator that
[ADR 0151](0151-macos-plan-arm64-translator-and-aarch64-port.md) chose.

## Context

ADR 0151 makes an arm64 to x86-64 translator the first piece of macOS
work, so that arm64-only Mac apps run on the x86 PCs Poly targets. The
translator is a JIT in the end. Before the JIT there has to be something
that is plainly correct: a decoder for A64 instructions and an interpreter
that executes them. The interpreter is the reference the JIT is tested
against, and the fallback for instructions the JIT does not translate yet.

Two things shape the design:

- The JIT must reuse the decoder unchanged. If decoding and execution are
  mixed, the JIT would have to read raw instruction words again or copy
  the tables.
- We have no Arm hardware and no Mac to compare with, and the clean-room
  rules (`CLEAN_ROOM.md`) forbid reading any emulator's, disassembler's or
  JIT's source. Tests have to come from the Arm Architecture Reference
  Manual (DDI 0487) and from tools used as black boxes.

## Decision

### A new crate: `crates/poly-a64`

`no_std`, no dependencies, host unit tests, no `unsafe`. Modules:

| Module | Content |
|---|---|
| `inst` | `Inst`, the decoded instruction, and the enums it uses (`Cond`, `Shift`, `Extend`, `Addr`, `FpType`, ...) |
| `decode` (and `decode_simd`) | `decode(word: u32) -> Inst`, a pure function |
| `cpu` | `Cpu` (registers), `Memory` (the trait), `Outcome`, `Cpu::step`, `Cpu::execute`, `Cpu::run` |
| `exec_int`, `exec_mem`, `exec_fp`, `exec_simd` | the executors, one per instruction class |
| `fp` | scalar floating-point arithmetic on raw bit patterns |

### Decoder and interpreter are separate; the JIT reuses `Inst`

`decode` looks at the word and nothing else. `Inst` is `Copy`, holds no
pointers, and is fully decoded: register numbers, 32 or 64 bit width,
logical bitmask immediates expanded (`DecodeBitMasks` from the Arm ARM),
bitfield masks, shifted `ADD` immediates, floating-point immediates,
sign-extended branch and literal offsets in bytes, addressing modes as an
enum, condition codes as an enum. Register number 31 stays 31; the doc
comment of each variant says whether it means `SP` or `XZR`.

Allocated but unimplemented encodings decode to `Inst::Unsupported(word)`,
unallocated ones to `Inst::Undefined(word)`. Neither panics. A test decodes
three million random words and every top-ten-bit pattern.

The plan for the JIT: decode a basic block once into a run of `Inst`
values, cache the block by guest address, translate the `Inst` run to
x86-64. `Cpu::execute(inst, mem)` runs one already decoded instruction, so
the JIT can fall back to the interpreter for one instruction without
decoding it again. `Cpu::icache_flushes` counts `IC IVAU` executions: a JIT
compares it before running a cached block to notice that the program wrote
code and invalidated its instruction cache.

### Memory and system call interfaces

The interpreter never touches host memory. All accesses go through the
`Memory` trait: `read` and `write` of bytes (little-endian), `fetch`, and
`cas`. Each returns a `Result`, and a failed access becomes
`Outcome::Fault(MemFault { addr, access, alignment })`, never a panic.
The caller owns the address space (page tables, `mmap`, copy-on-write,
`MAP_JIT`); `SliceMemory` is a flat one for tests and tools.

`Cpu::step` fetches, decodes and executes one instruction and returns:

- `Continue`.
- `Svc(imm)`: the `PC` is already past the `SVC`. The caller reads the
  system call number and arguments from the registers (`x16` and the carry
  flag convention of macOS arm64, `x8` on Linux), does the call, writes the
  result, and steps on. This crate knows no system call numbers.
- `Brk(imm)`: the `PC` stays at the `BRK`.
- `Fault(MemFault)`: the `PC` stays at the instruction, registers are as
  before except that a load pair or store pair may be partly done (the
  architecture allows that).
- `Undefined(word)` and `Unsupported(word)`: the `PC` stays.

Ordinary loads and stores may be misaligned, as on macOS and Linux user
space. Exclusives, acquire/release accesses and atomics must be aligned and
fault with `alignment` set otherwise. The stack pointer's 16-byte alignment
is not checked.

### Exclusives and atomics

`LDXR` remembers address, size and value; `STXR` succeeds if the monitor is
still open (no `CLREX`, no other store-exclusive, `Cpu::clear_exclusive`
not called) and `Memory::cas` finds the remembered value still in memory.
The LSE atomics (`LDADD` family, `SWP`, `CAS`, `CASP`) are `cas` loops.
With the default `cas` this is exact for one guest thread at a time. A host
that runs guest threads in parallel overrides `cas` with a real atomic
compare-and-swap (an x86 `lock cmpxchg`) and gets correct behaviour: the
only difference from the architecture's monitor is that an intervening
store of the same value goes unnoticed, which the architecture allows for
a store-exclusive to succeed or fail in.

### Flags

`NZCV` is a `u32` in its architectural bit positions (31 to 28), so `MRS`
and `MSR` are plain copies. The interpreter computes flags eagerly with the
Arm ARM's `AddWithCarry` pseudocode, at the width of the operation (32 or
64 bits). The JIT is free to compute flags lazily, but it must produce the
same bits whenever the program reads them (`MRS`, a condition, `ADC`,
`CCMP`, a branch out of the translated block). The differential tests are
the oracle for that.

### Floating point

Scalar half (conversions only), single and double precision, round to
nearest even. Addition, subtraction, multiplication and division use the
host's IEEE operators after explicit NaN and invalid-operation handling
that follows the Arm ARM (signalling before quiet NaN, first operand before
second, default NaN positive; `FPCR.DN` is honoured). Fused multiply-add
and square root are done with integer arithmetic so they are correctly
rounded on any host. Conversions to integers use the named rounding mode
and saturate; NaN gives 0.

Not implemented, and documented in `poly_a64::fp`: `FPCR.RMode` for
arithmetic, `FRINTI`, `FRINTX` and `SCVTF` (they use nearest even),
flush-to-zero and alternative half precision, trapped exceptions, and the
`FPSR` flags other than `IOC` and `DZC` (overflow, underflow, inexact and
input-denormal are not tracked). Half-precision arithmetic is
`Unsupported`.

### What is covered

- **Integer:** add/subtract (immediate, shifted and extended register,
  with carry, flag setting), logical immediate and shifted, move wide,
  `ADR`/`ADRP`, all bitfield moves and their aliases, `EXTR`, conditional
  compare and select, all multiplies (`SMULH`, `UMULH`, long forms),
  `UDIV`/`SDIV` (zero divisor gives 0, `INT_MIN / -1` gives `INT_MIN`),
  variable shifts, `CLZ`, `CLS`, `RBIT`, `REV`, `REV16`, `REV32`.
- **Branches:** `B`, `BL`, `B.cond`, `CBZ`/`CBNZ`, `TBZ`/`TBNZ`, `BR`,
  `BLR`, `RET`.
- **System:** `SVC`, `BRK`, every hint as a no-op (`NOP`, `YIELD`, `BTI`,
  and `PACIASP`, `AUTIASP` and the other pointer-authentication hints,
  which the architecture lets an implementation without the feature treat
  as `NOP`), `MRS`/`MSR` for `NZCV`, `FPCR`, `FPSR`, `TPIDR_EL0`,
  `TPIDRRO_EL0` (read), `CNTVCT_EL0`, `CNTFRQ_EL0`, `DCZID_EL0`, barriers,
  `DC ZVA`, cache maintenance (no effect), `IC IVAU` (counted).
  `CNTVCT_EL0` returns a value the host sets and adds a step after each
  read, so a program waiting on the counter makes progress.
- **Loads and stores:** every size and addressing mode (unsigned offset,
  unscaled, pre and post index, register offset with extend and shift,
  literal), sign-extending loads, `LDTR`/`STTR`, pairs including `LDPSW`
  and the non-temporal forms, `PRFM` (a no-op), exclusives and exclusive
  pairs, `LDAR`/`STLR`/`LDAPR`/`LDLAR`, the LSE atomics, `CAS`, `CASP`, the
  `B`/`H`/`S`/`D`/`Q` register forms, and `LD1`..`LD4`/`ST1`..`ST4`
  (multiple structures, single lane, replicate, post-indexed).
- **Scalar floating point:** `FMOV` (register, general, top half,
  immediate), `FADD`, `FSUB`, `FMUL`, `FDIV`, `FNMUL`, `FSQRT`, `FABS`,
  `FNEG`, `FMIN`, `FMAX`, `FMINNM`, `FMAXNM`, `FMADD`, `FMSUB`, `FNMADD`,
  `FNMSUB`, `FCMP`, `FCMPE`, `FCCMP`, `FCSEL`, `FCVT`, `SCVTF`, `UCVTF`,
  `FCVT[NPMZA][SU]` and the fixed-point forms, `FRINT[NPMZAXI]`.
- **Advanced SIMD:** `MOVI`, `MVNI`, `ORR`/`BIC` immediate, `FMOV` vector
  immediate, `DUP`, `INS`, `UMOV`, `SMOV`, `ADD`, `SUB`, the logical and
  bit-select instructions, `CMEQ`, `CMHI`, `CMHS`, `CMGT`, `CMGE`, `CMTST`,
  compares with zero, `MUL`/`MLA`/`MLS`, `SMAX`/`SMIN`/`UMAX`/`UMIN`,
  `ADDP`, `SSHL`/`USHL`, `ADDV`, `UMAXV`, `UMINV`, `SMAXV`, `SMINV`,
  `UADDLV`, `SADDLV`, `EXT`, `TBL`, `TBX`, `UZP`, `ZIP`, `TRN`, `XTN`,
  `NOT`, `CNT`, `REV16/32/64`, `RBIT`, `NEG`, `ABS`, `CLZ`, `CLS`, shifts by
  immediate (`SHL`, `USHR`, `SSHR`, `USRA`, `SSRA`, `SLI`, `SRI`, `SHRN`,
  `USHLL`, `SSHLL`), and the scalar `D` forms of the integer ones.

### What is not covered

Decoded as `Unsupported` and executed as `Outcome::Unsupported`: SVE and
SME; the crypto extensions (AES, SHA, PMULL); `CRC32`; pointer
authentication register forms (`PACIA`, `AUTIA`, `RETAA`, ...: macOS
`arm64e` is Apple's own binaries, which Poly does not run); memory tagging;
half-precision arithmetic; vector floating-point arithmetic and by-element
forms; long, wide and narrow integer arithmetic (`UADDL`, `UMULL`,
`SQXTN`, ...); saturating and rounding vector arithmetic; the flag
manipulation extension (`RMIF`, `SETF8`); `MSR` to `PSTATE` fields. They
are added as real programs need them; the decoder already separates
"allocated, not implemented" from "undefined", so the gaps are visible
in a run.

### Testing, and `qemu-aarch64` as a black box

1. **Unit tests** (`cargo test -p poly-a64`, 51 tests). Encodings are
   test vectors made by the system assembler (`llvm-mc --triple=aarch64`,
   a black box), kept in `src/vectors.rs` with the assembly text beside each
   word and looked up by text; a few are built from field positions in the
   Arm ARM's encoding diagrams. Expected values are written by hand from
   the Arm ARM pseudocode. Edge cases: carry and overflow flags, 32-bit
   results zeroing the upper half, `SP` versus `XZR`, division edge cases,
   faults leaving state alone, alignment faults, exclusives, `NaN` rules.
   The decoder is run on millions of random words, and the interpreter
   on 600,000 random instructions with random state, to show that nothing
   panics.
2. **A differential tester** (`tools/a64diff`, `cargo xtask a64-diff`).
   A host tool, not shipped. It builds static arm64 Linux programs with
   `clang --target=aarch64-linux-gnu -nostdlib -static -fuse-ld=lld`, runs
   each under `qemu-aarch64` and under poly-a64 (a minimal static ELF64
   loader and the `write`, `exit` and `exit_group` system calls), and prints
   pass or fail and the register differences. Two kinds of programs:
   - `tests/a64/*.s`: hand-written programs for each area that end by
     writing `X0`-`X30`, `NZCV`, `FPSR` and `V0`-`V7` to standard output.
   - `a64diff fuzz`: generated programs of many one-instruction blocks.
     Each block loads a random register and memory state (with edge values
     for integers, floating point and bytes), runs one instruction built
     from the Arm ARM's encoding fields (by an encoder that does not use
     poly-a64's decoder), and writes a 1,312-byte record of all registers,
     the flags and a 512-byte memory window. The two outputs are compared
     record by record. Fifty-four families cover the classes above; branches
     and exclusives are short assembly sequences. If poly-a64 stops in a
     block, the tool notes it and resumes at the next block. If qemu rejects
     a block (a mistake in the generator), the tool finds it by bisection and
     leaves it out.
   Differences that are the platform's, not the architecture's, are removed:
   `DCZID_EL0` is read from qemu and handed to poly-a64, qemu runs with
   `-cpu max,pauth=off` (its pointer-authentication keys are random and
   poly-a64 treats the hints as `NOP`), only the `IOC` and `DZC` bits of
   `FPSR` are compared, and the programs set the flags they depend on
   because the flags at process start are unspecified (qemu starts with `Z`
   set).

   On the first full run the tool found one bug (square root of subnormal
   inputs lost precision); it is fixed and has a unit test. Afterwards
   about 200,000 blocks from eight seeds all matched. A deliberate bug
   injected in `CSINC` was caught by the first run of its family.

   The tool needs Linux (qemu-user, clang, lld, the `poly-dev` image has
   them). On any other host it says so and exits successfully, so it still
   compiles for `x86_64-pc-windows-msvc` as `cargo xtask ci lint` checks.
   It is **not part of `cargo xtask ci`**: it needs qemu-user, takes
   minutes, and a Linux CI job for it is a separate decision.

### Clean room

Inputs: the Arm Architecture Reference Manual for A-profile (DDI 0487) and
Arm's public A64 documentation. Tools used as black boxes, inputs and
outputs only: `llvm-mc` (assembling test vectors), `clang` and `lld`
(building test programs), `qemu-aarch64` (running them). Nobody read the
source of QEMU, LLVM's AArch64 code or disassembler, binutils, Unicorn,
Dynarmic, Box64, FEX-Emu, Rosetta, XNU, `dyld`, Darling or Linux's
`arch/arm64`, and no code was copied from any project. `THIRD_PARTY.md` is
unchanged.

## Consequences

- The JIT can start from `decode` and be tested record by record against
  `Cpu::step` with the same `a64diff` blocks.
- Flags, floating-point and exclusive-monitor behaviour are fixed by tests
  against an independent implementation, not only by our reading of the
  manual. Where the two disagree on something the architecture leaves
  open, the tool's exceptions above are the list.
- Programs that use anything in "not covered" stop with
  `Outcome::Unsupported(word)`, which names the instruction to add next.
- No kernel, personality or CI change: nothing here runs in Poly yet.
  The next steps are a loader for arm64 Mach-O that starts a `Cpu` (ADR
  0153's reader), the arm64 system call convention in the XNU personality,
  and the block JIT.
