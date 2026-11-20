# 0239: A block JIT from arm64 to x86-64, guest memory with 16 KiB pages and `MAP_JIT`, the arm64 system call convention

**Status:** Accepted. Track `impl-a64-jit`, Roadmap Phase 9 "arm64 translator: block JIT to
x86-64 on the shared decoder, the arm64 system call convention in the XNU personality, 16 KiB
pages, `MAP_JIT` and code cache invalidation". Builds on ADR 0151 (the macOS plan) and ADR 0152
(the decoder and interpreter, with its amendment: Apple silicon as the oracle). Inputs: the Arm
Architecture Reference Manual (DDI 0487), Intel's Software Developer's Manual volume 2 and AMD's
APM volume 3, the macOS SDK's public headers (`sys/mman.h` for `MAP_JIT`, `mach/arm/_structs.h`
for the signal context of the test tool), and runs of instruction streams we generated on an
Apple silicon Mac. `clang` is used as a black box (assembling test vectors on both sides). No
emulator's, translator's or JIT's source was read (QEMU, Rosetta material, FEX, box64,
Dynarmic, Unicorn, LLVM's backends), and no Apple binary was disassembled.

## Context

ADR 0151 put the translator first: arm64-only Mac programs have to run on the x86-64 PCs Poly is
for. ADR 0152 gave us a decoder that turns a word into a fully decoded `Inst` and an interpreter
that is the reference. An interpreter is tens of times slower than native code; the next step is
translating guest code to host code, in a form that can be checked against the interpreter.

Three constraints shape the design:

- **The development machines are arm64.** Translated code cannot run on them, so everything that
  can be checked without running x86-64 code is checked on the Mac (the emitter's bytes against
  the system assembler, the interpreter against the CPU), and the rest runs on x86-64 CI runners.
- **The interpreter stays the reference and the fallback.** The translator must never need to be
  complete: any instruction it does not handle, and any case it does not want to handle (a NaN
  result, an unaligned atomic, a write to code), goes to the interpreter for that one
  instruction.
- **Guest programs write code.** JavaScriptCore and every other JIT write and run code through
  `MAP_JIT` mappings, and the code cache must notice.

## Decisions

### 1. A new crate: `crates/poly-a64-jit`

`no_std` with `alloc`, host-tested. Modules: `mem` (guest memory), `x64` (the emitter),
`translate` (one block), `jit` (code cache, chaining, invalidation, run loop), `process` (loading
an arm64 test program). It runs in the translated process (ADR 0151): the host provides the
code area (readable, writable, executable) and the guest window.

### 2. Guest memory: a flat window, 16 KiB pages, a flag byte per page

`GuestMem` covers guest addresses `lo..lo + size`, backed by host memory at `host + (addr - lo)`
(owned by the value for tests, or a host reservation for a real process). The guest page size
is Apple silicon's, **16 KiB**, whatever the host's: `map`, `unmap` and `protect` take 16 KiB
multiples. One byte per page holds `R`, `W`, `X`, `JIT` (part of a `MAP_JIT` mapping) and `CODE`
(translations were made from it).

`MAP_JIT` (0x0800, SDK `sys/mman.h`) pages follow the thread's write-protect mode, as
`pthread_jit_write_protect_np` sets it on macOS: in execute mode (the default) they are `R+X`, in
write mode `R+W`. Poly's libSystem will implement that call by asking the runtime; the call
`set_jit_write_protect` is the interface.

### 3. Translated code: guest state in memory, pinned host registers

Translated code runs with `rbx` = the guest `Cpu`, `rbp` = a context, `r12` = the window's host
base, `r13` = the page flags, `r14` = the window's first address, `r15` = its size. Guest
registers live in the `Cpu` (offsets from `offset_of!`); nothing is cached in host registers
between guest instructions. That costs some speed and buys a simple invariant: the guest state
is exact at every instruction boundary, so any instruction can leave for the interpreter. A
register allocator is a later optimisation.

### 4. Memory accesses: checked in translated code, precise in the interpreter

Every translated load and store checks the guest address against the window, the page's flags
(`R`; for a store `W` and not `CODE`) and the 16 KiB page boundary, then accesses
`r12 + (addr - r14)`. If a check fails, the block leaves before the instruction has changed
anything and the run loop interprets it: the interpreter faults precisely (`MemFault` with the
first bad address) or, for a write to a code page, writes and reports the page. Guest faults
never become host faults, and no signal handler is needed. Using the host MMU instead (guest
pages mapped with matching host protections, faults from a signal handler) is a later
optimisation; the flag table stays the reference.

### 5. Flags: in the host flags within a block, written when live

A64's `NZCV` maps onto x86's flags: `add`/`sub` compute N, Z and V as A64 does, C equal after an
addition and inverted after a subtraction (A64's C is "no borrow"); `and`/`test` give A64's
`ANDS` (C and V clear). A flag-setting instruction leaves its result in the host flags. A
backwards liveness pass over the block decides whether it must be written to the `Cpu`: only if
a later instruction reads it from memory, or a possible exit (a block end, a memory access that
might go to the interpreter) comes before the next flag-setting instruction. The write uses only
instructions that keep the host flags (`setcc`, `movzx`, `lea`, `mov`), so a following
`B.cond`, `CSEL`, `CSINC`, `ADC` or `SBC` still branches or selects on the host flags directly.
Conditions the host flags cannot express for a kind (`HI` after an addition) are evaluated from
the `Cpu`'s `NZCV`.

### 6. Blocks, chaining, the indirect branch cache, the budget

A block is decoded once (the shared decoder) from its first instruction to a branch, a system
call, an instruction the translator does not handle, the end of its 16 KiB page, or 64
instructions. Each block starts with a budget check (`sub [ctx.budget], 1`), so the run loop
regains control in loops that never leave translated code.

A direct exit stores the next `PC` and jumps through a patchable `jmp rel32` to a stub that
returns to the run loop and reports the exit's site. The run loop then patches the jump to the
next block's translation (chaining) and records the link in the target block. Indirect branches
(`BR`, `BLR`, `RET` and the authenticating forms) look the target up in a 4,096-entry direct-
mapped cache of (guest PC, host address) pairs in the context before falling back to the run
loop.

### 7. Invalidation

Translating a block marks its page `CODE`. A write to such a page from translated code (which
takes the interpreter path because of the flag check), the interpreter, the loader,
`map`/`unmap`/`protect`, or a `MAP_JIT` switch to write mode records the page and clears `CODE`.
Before running anything, the run loop drops every block of a recorded page: chained jumps into
it are pointed back at their stubs, it leaves the block map, and the indirect branch cache is
cleared. `IC IVAU` and `sys_icache_invalidate` need no work of their own, because every way to
change guest code already goes through this. When the code area is full, everything is dropped.

### 8. What is translated

Integer data processing (all of it except `RBIT`, `CLS` and the flag-manipulation instructions),
every branch, `SVC` (an exit to the run loop), hints and barriers (no code: x86-64's memory order
is stronger than what A64's barriers ask for), loads and stores of every size and addressing
mode for both register files including pairs, `LDADD`/`SWP` (`lock xadd`/`xchg`) and `CAS`
(`lock cmpxchg`) with FEAT_LSE2's 16-byte rule, scalar floating point (`FADD`, `FSUB`, `FMUL`,
`FDIV`, `FNMUL`, `FSQRT`, `FABS`, `FNEG`, `FMOV`, `FCMP`, `FCSEL`, `FCVT`, `SCVTF`, `UCVTF` from
`W`, `FCVTZS` without fraction bits), fused multiply-add through FMA3 when the CPU and OS have it,
`CRC32C*` through SSE4.2, and common Advanced SIMD (`AND`, `ORR`, `EOR`, `BIC`, `ADD`, `SUB`,
`CMEQ`, `NOT`, `MOVI`, `DUP`, `UMOV`, `SMOV`, `INS`) through SSE2.

Floating point keeps A64's rules by sending the cases where x86 differs to the interpreter: a NaN
or infinite result, an unordered compare, a conversion that hits the "integer indefinite" value.
x86's default `MXCSR` (round to nearest, no flush to zero) matches A64 with `FPCR` 0 for
everything else, and the interpreter does not model the other `FPCR` modes either.

Exclusives (`LDXR`/`STXR`) and everything else stay in the interpreter.

### 9. The arm64 system call convention (`personalities/xnu/src/arm64.rs`)

`svc #0x80` with the number in `x16`: positive is a BSD call, negative a Mach trap (`-n` is trap
`n`), `0x8000_0000` the machine-dependent call; arguments in `x0`..`x7`; a BSD call reports an
error with the carry flag and the errno in `x0` and returns its result in `x0`; Mach traps return
a `kern_return_t` in `x0` and leave the flags alone. These are ABI facts (call numbers,
CLEAN_ROOM.md); the spec sheets confirm the details call by call. The call is converted to the
x86-64 frame the personality already handles (class in the top byte of `rax`) and back, so
there is one copy of every system call: `arm64::dispatch_with` runs it through the BSD layer of
ADR 0237 (`bsd::dispatch_with` and its `BsdKernel` services), `arm64::dispatch` through any
`SyscallDispatch`. The test programs' `write` and `exit` go this way. `x1` is left alone: the
calls with a second result (`fork`, `pipe`) are not implemented yet.

### 10. Test programs: our own C, built on the Mac, committed

`tests/a64/macos/*.c` are nine command-line programs (sorting, SHA-256 and CRC, floating point
including n-body and a matrix product, text processing, big integers, C11 atomics, a bytecode
interpreter with a jump table and function pointers, NEON intrinsics) with a small runtime that
makes system calls itself (`svc #0x80`) and calls nothing in libSystem. `build.sh` builds each at
`-O2` and `-O3` with Apple's toolchain (a black box) and records the real Mac's output; the 18
binaries and outputs are committed (16 to 50 KiB each), so the tests run on any host.
`process::load` maps such a program (16 KiB pages, chained rebases, the segment protections, a
stack, a return stub that calls `exit`). Programs with imports need Poly's dyld (ADR 0237).

## Tests

- **Emitter:** 15,000 instructions (every register, size and addressing form the translator
  uses, VEX-encoded FMA3, SSE) are compared byte for byte with what `clang` assembles from the
  same Intel-syntax text (`src/x64/vectors.txt`, regenerated by an ignored test on any host whose
  `clang` targets x86-64). The test found four encoding bugs on its first run.
- **Programs under the interpreter** (every host): all 18 print exactly what the Mac printed.
- **Programs under the translator** (x86-64 hosts): the same, plus counters (blocks, chained
  exits, instructions left to the interpreter).
- **Differential test** (x86-64 hosts): random sequences of one to six instructions (data
  processing, flag setters and readers, branches, loads and stores into mapped pages, scalar
  floating point with edge values, Advanced SIMD), run by the translator and by the interpreter
  from the same state; registers, flags, `FPSR`, the stop reason and memory must be equal. 30,000
  sequences in `cargo test`, 300,000 in the `a64-jit` workflow; it found two flag-tracking bugs.
- **Speed** (the `a64-jit` workflow, x86-64 runner, release build): the test programs run 7.1
  times faster translated than interpreted overall, 28 times for the bytecode interpreter, 10
  to 14 times for hashing and big integers, 2.6 to 3.7 times for floating point (vector floating
  point still goes to the interpreter).

## Not in this change

- Running a translated process in Poly: the kernel still refuses arm64 Mach-O files (ADR 0237).
  The next step is a translated-process runner in user space that maps the program with Poly's
  dyld, hands its system calls to the XNU personality through `arm64::dispatch`, and provides
  the code area and `pthread_jit_write_protect_np`.
- Guest threads (one `Engine` per thread; the code cache could be shared later), signals into
  translated code, host-MMU memory checks, a register allocator, translating exclusives, vector
  floating point and the rest of Advanced SIMD.
