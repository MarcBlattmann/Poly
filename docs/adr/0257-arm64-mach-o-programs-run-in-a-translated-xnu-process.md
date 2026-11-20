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

The calls that change guest memory go to the kernel too (the BSD layer of ADR 0246), but the
runtime shapes them first, because guest memory must stay in the window and the window's page
flags must follow it:

- `mmap` without `MAP_FIXED` gets an address in the window's anonymous area from the runtime,
  and the kernel maps there with `MAP_FIXED`; a `MAP_FIXED` mapping must land in the window.
  Lengths and file offsets are counted in 16 KiB guest pages (the kernel's are 4 KiB): an
  unaligned file offset gives an address that many bytes into a 16 KiB page, as the mmap sheet
  says for arm64.
- The host protection asked of the kernel is the guest's read and write bits, never execute
  (guest code is only read by the translator), so the kernel's checks (a shared mapping of a
  read-only file cannot be writable) still apply; the guest's own protection, execute included,
  goes into the page flags. `MAP_JIT` memory is readable and writable for the host in both of
  its modes, which only the page flags tell apart.
- `munmap` and `mprotect` must stay in the window; the kernel does them, then the flags follow.

`exit` is reported to the runtime first (its statistics), then made.

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
the guest. The runtime prints where and why, then ends the process the way the kernel ends a
native program: `kill(getpid(), signal)` with SIGSEGV for an unmapped page, SIGBUS for one the
protection forbids (`fault_signal`), SIGTRAP for `brk`, SIGILL for an instruction, SIGSYS for
the `svc`. A guest that catches or ignores the signal cannot run its handler yet (signal frames
in translated code come later), so the runtime then faults on purpose and the process still ends
abnormally.

## Tests

- `crates/poly-a64-xnu` (every host): the hand-off block (round trip, damaged blocks), the
  layout, the settings, `runnable_cpu`, the regions' protections, the start contract, and the
  test programs loaded with `load` and run through `Bridge` with a host that answers `write`,
  `mmap`, `munmap`, `mprotect`, `getpid` and `kill` as the kernel does: under the interpreter
  everywhere and under the JIT on x86-64 hosts. The memory calls' shaping (length 0, a file
  mapping without a descriptor, `MAP_FIXED` unaligned and outside the window, `mprotect` to
  read-only, `munmap`, a fixed mapping over an unmapped page reading zero) and the signal of
  each kind of stop.
- `tests/a64/xnu`: our C sources, built by `build.sh` on a Mac and committed: (a)
  `hello-static`, `write` and `exit` with the carry flag and `EBADF` of a failed call, also as
  `hello-fat` (a universal file with only the arm64 slice); (b) `hello`, dynamically linked
  against `libsystem.c`, our stand-in for libSystem (initializers, thread-local variables,
  `malloc`, `argc`/`argv`/`envp`/`apple`); (d) `crash`, which reads address 8; (c) `heap`,
  malloc-heavy (a hash table of strings
  with `realloc`, half freed per round, a merge sort), whose expected output is the same source
  built for the Mac with its own C library (it prints nothing that depends on addresses).
- Boot test group `macho-a64` (shard `userland`): the programs (plus poly-dyld's `hello_a64`
  with its `@rpath` libgreet) as the first process on Poly; `crash` must end with signal 11
  through `kill`, not through the runtime's fallback fault; `heap 2` under the JIT and under the
  interpreter alone, both printing the native output.

On the owner's Mac under QEMU TCG (an M1 Pro running x86-64 by emulation), `heap 2` (15.3
million guest instructions) took 0.19 billion time stamp counter cycles under the JIT (236
blocks, 33 instructions left to the interpreter) and 7.6 to 9.4 billion under the interpreter:
the JIT was 31 to 48 times faster over several runs, and the whole boot 0.7 s against 7 to
10 s.

## Not in this change

- Signal handlers in translated code: a signal frame on the guest stack in the arm64 layout,
  `sigreturn`, and asynchronous signals at an instruction boundary of the run loop.
- Threads: `bsdthread_create` needs one `Engine` and `Cpu` per guest thread in the runtime,
  each with its TSD page and thread-local blocks, and a kernel thread per guest thread.
- `fork`/`execve` of a translated process.
- The commpage: arm64 macOS user space reads one at a fixed address; neither its address nor its
  layout is in the public SDK, so it needs a spec sheet before Poly maps one.
- The machine-dependent call (`x16` = `0x80000000`) for the thread pointer, `MAP_JIT`'s
  `pthread_jit_write_protect_np` through the runtime, `sys_icache_invalidate`.
- Writes by the kernel into guest pages that hold translations (a `read` into code) do not
  invalidate them yet.
- Apple's own libSystem is not run: arm64 programs link Poly's libSystem once there is one
  (ADR 0157's next step); until then only programs on our stand-in run.

## Addendum (2026-11-20): real command-line programs

Track `impl-a64-cli`. Real arm64 programs built from source and linked against the SDK the ordinary
way now run on x86-64 Poly: the Lua 5.4.7 interpreter, the SQLite 3.47.2 shell, and Poly's own C
programs with threads, signals, memory mappings and code written at run time. They load through
Poly's dyld with Poly's own libSystem (ADR 0258) in place of Apple's, and run in the translator.
Inputs as above, plus the C standard, POSIX and the macOS manual pages for the library functions,
the SDK headers for every layout used (`<runetype.h>`, `<_locale.h>`, `<time.h>`, `<sys/dirent.h>`,
`<malloc/malloc.h>`, `<pthread/pthread_impl.h>`, `<sys/signal.h>`, `<sys/_types/_ucontext.h>`,
`<mach/arm/_structs.h>`, checked with `_Static_assert`) and the spec sheets `sigaction`, `pipe`,
`ioctl`, `mmap`, `bsdthread_*`, `sys_sysctlbyname` and `ulock`. Lua and SQLite are downloaded at a
pinned version and checksum by `tests/a64/cli/build.sh` and built with clang as a black box; their
sources are not committed (THIRD_PARTY.md). No Apple binary was read or run as an oracle.

### 8. libSystem for arm64

Poly's libSystem is built for `aarch64-apple-darwin` too, and every library is a universal file
with both slices (`cargo xtask libsystem` writes the fat header; poly-dyld picks the slice). What
differs per architecture is small and in assembly: the call stubs (`svc #0x80`, the number in
`x16`, a Mach trap negated, carry for errors), `errno`'s thread-local access, `memcpy` and its
family, `setjmp` (on arm64 `setjmp`/`sigsetjmp(env, 1)` save the signal mask and
`longjmp`/`siglongjmp` restore it), the variadic functions (Apple's arm64 ABI passes variadic
arguments on the stack, one 8-byte slot each, and `va_list` is a pointer: the `printf` family and
`sscanf` are a `mov xN, sp` and a jump) and the names the SDK gives the 64-bit `stat` and `statfs`
calls (`$INODE64` on x86-64 only; `exports/<library>.<arch>.txt` hold the differences). Three more
libraries join the re-exports, as Lua and SQLite need them: `libsystem_m` (poly-libm's functions,
`fmod`, `frexp`, `ldexp` and the rounding functions exact on the bits), `libsystem_info`
(`getpwuid`, one user) and `libcompiler_rt` (`___chkstk_darwin`, a return: a thread's stack is mapped
whole). libsystem_c grows the stream functions on files (buffered reads, `ungetc`, `fseeko`,
`setvbuf`, `freopen`, `tmpfile`, `popen`/`system` through `posix_spawn` with the standard
descriptor swapped around the spawn), directories, `getcwd`, `strtod` (C syntax scanned here, the
decimal digits converted by `core`'s correctly rounded parser, hexadecimal ones rounded here),
`sscanf`, calendar time and `strftime` in the C locale (local time is UTC until Poly has a zone
database), the C locale's `_DefaultRuneLocale` tables that `<ctype.h>`'s inline functions read,
`setlocale`/`localeconv`, `strerror`, `sysconf`/`confstr`, `sigaction` (the kernel's
`__sigaction` with Poly's trampoline, which calls the handler and then `sigreturn`), `signal`,
`raise`, `__assert_rtn` and `malloc` zones (one zone, the default one, whose function pointers SQLite
calls). libsystem_pthread gets real threads: `pthread_create` maps a stack and a record and calls
`__bsdthread_create`; mutexes, condition variables, `pthread_once` and `pthread_join` wait with
`__ulock_wait`; thread-specific values are per thread. `pthread_jit_write_protect_np` reaches the
translator (section 11). The choices the libc sheets are to confirm are marked in the code; the
208 coverage entries this adds are claimed, not green.

### 9. The kernel: threads and more BSD calls

`bsdthread_register` (the first registration wins, a second is `EINVAL`), `bsdthread_create` and
`bsdthread_terminate` work for XNU processes. The register state of a new thread is Poly's
contract with its own libpthread, as no sheet fixes it: the stack pointer at the caller-made stack,
and on x86-64 `rdi` = the thread structure, `rsi` = the start function, `rdx` = its argument.
New BSD calls without sheets yet, with their POSIX meaning: `gettimeofday`, `getrusage`, `unlink`,
`rmdir`, `mkdir`, `access` (existence only: one user), `umask` and `fsync`; and `ioctl` as its sheet
has it (`EBADF`, `FIOCLEX`/`FIONCLEX`, everything else `ENOTTY`: an XNU program cannot be told its
descriptor is a terminal yet, so it reads its input as from a file).

### 10. Threads in the translated process

Each guest thread runs on a host thread the kernel starts at the runtime's own entry point. The
translator's state (engine and code cache, guest memory and page flags, the bridge) is shared and
touched only under one lock, a fair ticket lock whose waiters sleep in `__ulock_wait`: translated
code of one guest thread runs at a time, so code invalidation and the page flags stay consistent
without more machinery. A thread gives the lock up after every 65,536 blocks and around every call
that goes to the kernel (the bridge keeps the call and stops the guest; the runtime makes it
unlocked), so a call that blocks never stops the others. `bsdthread_register` and
`bsdthread_create` are the bridge's: a new thread gets its own table at `TPIDRRO_EL0` (slot 0 its
thread structure, slot k its copy of image k's thread-local template, from templates the kernel
lists in the hand-off) and starts at the registered entry with x0 = thread structure, x1 = start
function, x2 = argument. dyld's arm64 thread-local thunk now reads the calling thread's block from
that table (a descriptor's key is its slot), so `errno` and `__thread` variables are per thread.
Running guest threads in parallel (a code cache shared without one lock, page flags updated
atomically) is a later step.

### 11. Signals, faults and code written at run time

The kernel keeps a process's actions but does not call handlers yet, and a translated thread's
registers are the runtime's anyway, so the runtime delivers. It keeps the guest's actions and the
blocked set (`sigaction` and `sigprocmask` are also passed on, so the kernel knows ignored and
default actions) and delivers the signals of faults (`SIGSEGV` with `SEGV_ACCERR` and the address
for an unmapped page, `SIGBUS` with `BUS_ADRALN` for one the protection forbids, `SIGILL`,
`SIGTRAP`, `SIGSYS`: the sigaction sheet's codes) and those the process sends itself (`kill` to
its own pid or 0, `raise`): the handler runs before `kill` returns, a blocked signal waits until it
is unblocked. A handler is entered as the sheet describes, through the trampoline with (handler,
info style, signal, siginfo, ucontext) and below a frame of `siginfo_t`, `ucontext_t` and the arm64
`mcontext` (`__es`, `__ss`, `__ns`); `sigreturn` restores every register and the mask. Signals from
other processes still go to the kernel (their default actions, or pending), and the blocked set is
the process's, not each thread's.

`mmap`, `munmap` and `mprotect` go to the kernel, shaped by the bridge as in section 2.
`pthread_jit_write_protect_np` on arm64 makes Poly's machine-dependent call (`x16` = 0x80000000,
`x3` = `"POLY"`), which the runtime answers by switching `MAP_JIT` pages between writable and
executable in its page flags. Every way code can change already reaches the translator's
invalidation (ADR 0239): a write to a page with translations, a `MAP_JIT` switch to write mode, an
`mprotect` that takes execute away; `sys_icache_invalidate` has nothing to do.

### 12. Speed: where the JIT spends its time

`POLY_A64_PROFILE=1` makes the runtime count the instructions the interpreter runs, by kind and
operation, and every exit from translated code by reason (a direct branch not chained yet, an
indirect branch the cache missed, an instruction for the interpreter, a system call). On Lua's
`bench.lua` (recursion, tables, strings, closures, floating point, a sort; QEMU TCG on the owner's
Mac):

| | before | after |
|---|---|---|
| instructions left to the interpreter | 265,081 | 155,527 |
| entries into translated code | 304,266 | 146,573 |
| indirect branches the cache missed | 50,381 | 2,161 |

What changed: `RBIT`, `CLS` and `UCVTF` from a 64-bit register are translated (`Bit1` and
`IntToFp` were the top of the profile; every instruction left to the interpreter costs a round trip
out of translated code, which no chain can avoid); the indirect branch cache has 16,384 entries
instead of 4,096 (most misses were collisions); and writing NZCV, which a flag-setting instruction
does whenever a later one may leave translated code, takes 7 instructions instead of 19: `pushfq`,
then the low 16 bits of RFLAGS index a 64 KiB table per kind (after an addition, after a
subtraction) in the code area, none of which changes the host flags. With exits this rare, the time
is in translated code itself (guest registers kept in memory, a bounds and flag check on every
access); the next wins are caching guest registers in host registers and the five SIMD forms the
profile still lists (single-lane loads, vector `SCVTF`, `USHLL`, `CMTST`, `ZIP1`). Under TCG the
JIT took 16 to 22 times fewer cycles than the interpreter on a fifth of the script (the full script
does not fit the interpreter's boot time); TCG timings on a shared host vary by ±20%, so the counts
above are the reliable measure, and CI's runs on KVM print the times on real hardware.

### Tests of the addendum

- Host: `poly-libsystem` (ctype tables, `strtod` with hexadecimal rounding and ranges, calendar
  conversions over ±2,000 years with ISO weeks and `strftime`, `sscanf`, buffered reads with
  `ungetc`, `ftell`, `fseek`, `strerror`, the arm64 `va_list`); `poly-a64-xnu` (thread creation and
  the new thread's table, deferred calls, signal frames round-tripping every register,
  actions/masks/pending); `poly-personality-xnu` (`gettimeofday`, `ioctl`, `x1` for `pipe`);
  poly-dyld's start code (the fifth initializer argument, the table-reading thunk); the JIT's
  differential test generates the one-source data processing group.
- Boot test group `macho-a64-cli` (shard `userland`): libSystem's own C test program built for
  arm64; `lua hello.lua` (the language and its standard libraries over libSystem: string
  formatting, `strtod`, coroutines, errors, math, files with seeks, `os.time`/`os.date`/`os.clock`);
  an SQLite session on an in-memory database whose output must equal `session.out`; `threads.c`
  (four threads on one mutex, a condition variable queue, `pthread_once`, per-thread `errno`,
  `__thread` and keys, anonymous and file mappings, a `SA_SIGINFO` handler through `raise`, a
  blocked signal delivered on unblocking, `SIGSEGV` and `SIGBUS` caught and left with
  `siglongjmp`); `jit.c` (MAP_JIT code run, rewritten and run again, `SIGBUS` for a write in execute
  mode, `mprotect` flips, code rewritten 200 times); Lua's benchmark under the JIT with the profile,
  then under both engines, whose outputs must be equal.

### Not yet

Guest threads in parallel; signals from other processes into translated code, and per-thread
masks; `fork`/`execve` of a translated process; a time zone database; locales other than C;
terminals for XNU programs (`TIOCGETA`); x86-64 thread-local storage per thread in Poly's
libSystem (x86-64 programs get threads but share `errno`); the commpage; the SIMD forms above;
the libc sheets for the choices marked in the code.
