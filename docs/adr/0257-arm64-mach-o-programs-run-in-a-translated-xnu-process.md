# 0257: arm64 Mach-O programs run in a translated XNU process

**Status:** Accepted. Track `impl-a64-hello`, Roadmap Phase 9 "arm64 translator: ... arm64-only
command-line programs run on x86 PCs" and the macOS plan's step "arm64 hello through the
interpreter" (`docs/macos/README.md`). Builds on ADR 0151 (the translator runs in user space, in
the translated process), ADR 0152 (the decoder and interpreter), ADR 0237 (Poly's dyld) and
ADR 0239 (the block JIT, the arm64 system call convention). Inputs: the Arm Architecture
Reference Manual, Apple's public arm64 calling convention, the macOS SDK's headers for numbers
and layouts (`sys/syscall.h`, `sys/mman.h`, `sys/errno.h`, `mach-o/loader.h`), the spec sheets
`specs/macos/bsd/{write,exit,mmap}.yaml`, and test programs of our own built with clang, ld,
lipo and rust-lld as black boxes. No source of XNU, dyld, Rosetta-related material or any
emulator was read; no Apple binary was disassembled.

## Context

The pieces were there: a decoder and interpreter, a block JIT with guest memory on 16 KiB pages,
a dyld that links arm64 files in host tests, the arm64 system call convention converted to the
x86-64 frame. But the kernel refused arm64 Mach-O files ("arm64 needs the translator"), and
nothing put the pieces together in a process.

## Decisions

### 1. A translated process is an XNU process running Poly's runtime

An arm64 program cannot run on the CPU, so its process runs a small x86-64 program of Poly's,
the **runtime** (`userland/native/a64`, installed as `/usr/libexec/poly-a64`), with the arm64
program mapped beside it. At exec the kernel (`kernel/src/xnu/arm64.rs`):

1. loads the runtime's ELF (static, built with the native programs but without Poly's ABI note:
   it is no native program),
2. links the arm64 program with Poly's dyld (`Options::ARM64`: 16 KiB pages, libraries from
   32 GiB),
3. maps and fixes it up with `poly_a64_xnu::load` (a host-tested crate, `crates/poly-a64-xnu`),
4. maps the runtime's heap and a code area (readable, writable and executable), writes the
   hand-off block and starts the runtime with its address in `rdi`.

The process is an ordinary XNU process (`Process::new_xnu`), so every system call the runtime
makes, its own and the guest's, goes through `crate::xnu::syscall`. That is ADR 0151's "in user
space, in the translated process": the guest's registers and code cache live there, and the
kernel stays an x86-64 kernel that does not know A64.

### 2. One copy of every system call

A guest `svc #0x80` leaves the run loop for the runtime's `Bridge`, which converts it with
`arm64::to_frame` and makes a **real x86-64 `syscall`** with that frame (the seventh and eighth
arguments on the stack where the personality reads them), then writes `x0` and the carry flag
back with `arm64::from_frame`. The XNU personality sees exactly what an x86-64 Mac program
would send and does not know the caller was translated. Guest addresses are host addresses
(the program is mapped where it was linked), so pointer arguments need no change.

Kept in the runtime are only the calls that change guest memory, because the guest window's
page flags must follow them: `mmap` (anonymous memory, handed out from an area the kernel maps
at exec), `munmap` and `mprotect`; and `exit`, which the runtime reports first (statistics) and
then makes. **Until #390:** the XNU BSD layer (PR #390, ADR 0246) brings `mmap`, `munmap` and
`mprotect` to the kernel; the runtime then forwards them with addresses inside the window and
updates its flags from the result, and file mappings come with that. The runtime's stub answers
file mappings with `ENOSYS` (`EBADF` for descriptor -1, as the sheet says).

### 3. The guest window

Everything the guest can touch lies in one window, 4 GiB to 64 GiB
(`poly_a64_xnu::layout::Layout::POLY`): the program at its preferred address above the 4 GiB
`__PAGEZERO`, libraries from 32 GiB, a 2 GiB anonymous area for `mmap` at 48 GiB, a system area
at 60 GiB (the arm64 start code and its data block, one thread-local block per image, the main
thread's TSD page) and an 8 MiB stack below 64 GiB. The runtime's own memory (its program at
2 MiB, its heap, code area and the hand-off block at 1 TiB) lies outside the window, so no guest
access reaches it: the translator checks every guest access against the window and a flag byte
per 16 KiB page (ADR 0239). The kernel maps all guest memory readable and writable for the host;
what the guest may do is in the hand-off's regions, which the runtime turns into page flags
without touching the bytes (`GuestMem::set_mapped`). A program whose segments do not fit below
the anonymous area is refused with a message.

### 4. The start of the main thread

As for x86-64 (ADR 0237): a static program (`LC_UNIXTHREAD` without `LC_LOAD_DYLINKER`) starts
at its entry point with `sp` at `argc`; everything else starts in Poly's arm64 start code
(`crates/poly-dyld/stub/start_arm64.s`, embedded in `poly_dyld::start::ARM64_START`) with `sp`
at the program's header address below `argc`. The start code calls the initializers with
`(argc, argv, envp, apple, 0)`, then `main(argc, argv, envp, apple)`, then libSystem's `exit`
(the raw `exit` call without a libSystem). Thread-local descriptors point to the arm64 thunk
(`x0` = descriptor in, `x0` = key + offset out, no other register changed). `TPIDRRO_EL0` holds
the main thread's TSD page.

### 5. Universal files

A universal file runs its x86-64 slice natively when it has one, else its arm64 slice through
the translator (`poly_a64_xnu::runnable_cpu`).

### 6. Settings

`POLY_A64_ENGINE=interp` in the program's environment runs the interpreter alone;
`POLY_A64_STATS=1` makes the runtime print its counters and the time stamp counter cycles taken
when the guest exits. The kernel reads both at exec (`settings_from_env`) into the hand-off. The
kernel argument `poly.init.env=NAME=value` sets the first program's environment (boot tests).

### 7. Guest stops

A fault, `brk`, an undefined or untranslated instruction, or an `svc` other than `#0x80` stops
the guest; the runtime prints where and why, then ends the process with a fault of its own
(`ud2`), so its parent sees an abnormal end rather than an exit status the guest never chose.
**Until #390:** signals (SIGSEGV, SIGBUS, SIGILL, SIGTRAP) to a guest handler set with
`sigaction`, or ending the process with that signal.

## Tests

- `crates/poly-a64-xnu` (every host): the hand-off block (round trip, damaged blocks), the
  layout, the settings, `runnable_cpu`, the regions' protections, the start contract, and the
  test programs loaded with `load` and run through `Bridge` with a host that answers `write`:
  under the interpreter everywhere and under the JIT on x86-64 hosts. The `mmap` stub's cases
  (length 0, file mappings, `MAP_FIXED` unaligned and outside the area, `mprotect` to
  read-only, `munmap`, a fixed mapping over an unmapped page reading zero).
- `tests/a64/xnu`: our C sources, built by `build.sh` on a Mac and committed: (a)
  `hello-static`, `write` and `exit` with the carry flag and `EBADF` of a failed call, also as
  `hello-fat` (a universal file with only the arm64 slice); (b) `hello`, dynamically linked
  against `libsystem.c`, our stand-in for libSystem (initializers, thread-local variables,
  `malloc`, `argc`/`argv`/`envp`/`apple`); (c) `heap`, malloc-heavy (a hash table of strings
  with `realloc`, half freed per round, a merge sort), whose expected output is the same source
  built for the Mac with its own C library (it prints nothing that depends on addresses).
- Boot test group `macho-a64` (shard `userland`): the five programs (plus poly-dyld's
  `hello_a64` with its `@rpath` libgreet) as the first process on Poly; `heap 2` under the JIT
  and under the interpreter alone, both printing the native output.

On the owner's Mac under QEMU TCG (an M1 Pro running x86-64 by emulation), `heap 2` (15.3
million guest instructions) took 0.19 billion time stamp counter cycles under the JIT (236
blocks, 33 instructions left to the interpreter) and 7.6 to 9.4 billion under the interpreter:
the JIT was 37 to 48 times faster, and the whole boot 0.7 s against about 8 s.

## Not in this change

- Threads: `bsdthread_create` needs one `Engine` and `Cpu` per guest thread in the runtime,
  each with its TSD page and thread-local blocks, and a kernel thread per guest thread.
- Signals into translated code, `fork`/`execve` of a translated process.
- The commpage: arm64 macOS user space reads one at a fixed address; neither its address nor its
  layout is in the public SDK, so it needs a spec sheet before Poly maps one.
- The machine-dependent call (`x16` = `0x80000000`) for the thread pointer, `MAP_JIT`'s
  `pthread_jit_write_protect_np` through the runtime, `sys_icache_invalidate`.
- Writes by the kernel into guest pages that hold translations (a `read` into code) do not
  invalidate them yet.
- Apple's own libSystem is not run: arm64 programs link Poly's libSystem once there is one
  (ADR 0157's next step); until then only programs on our stand-in run.
