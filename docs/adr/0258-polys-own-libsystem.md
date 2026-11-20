# 0258: Poly's own libSystem.B.dylib, v0

**Status:** Accepted. Track `impl-macos-libsystem`, the step "Our dyld and a Poly-built
`libSystem.B.dylib`" of `docs/macos/README.md` (ADR 0157's plan; dyld is ADR 0237). Inputs: the C
standard, POSIX and the macOS manual pages (`printf(3)`, `malloc(3)`, `dyld(3)`, `intro(2)`), the
System V x86-64 ABI (the `va_list`), the macOS SDK headers for constants and layouts (`<_stdio.h>`'s
`FILE`, `<sys/_pthread/_pthread_types.h>`, `<pthread/pthread_impl.h>`, `<sys/errno.h>`,
`<sys/mman.h>`, `<secure/_string.h>`), the export names, install names and versions of the SDK's
`.tbd` files (`coverage/macos/api/`), and the spec sheets of `specs/macos/bsd` and
`specs/macos/mach`. No part of Apple's libraries, dyld, libc, libpthread or XNU was read,
disassembled or run as an oracle; clang, Apple's ld and `rust-lld` were used as black boxes.

## What was there

Poly's dyld (ADR 0237) loads Mach-O programs and their dylibs, but the only libSystem was a
three-function stand-in for the dyld tests. A program linked the ordinary way against the SDK
(`/usr/lib/libSystem.B.dylib`) could not run: Poly may not ship Apple's dylibs.

## Decisions

1. **The same install names, Poly's code.** Seven Mach-O dylibs:
   `/usr/lib/libSystem.B.dylib` and the six it re-exports that a C program needs first,
   `/usr/lib/system/libsystem_kernel.dylib`, `libsystem_platform`, `libsystem_c`,
   `libsystem_malloc`, `libsystem_pthread` and `libdyld`, each with the compatibility and current
   version of the SDK's `.tbd` (compatibility 1.0.0 where the file gives none, libdyld 0). A name
   is exported by the library the SDK exports it from, so two-level namespace lookups through
   libSystem's `LC_REEXPORT_DYLIB`s find it. The other 33 libraries libSystem re-exports on macOS
   come when something needs them.
2. **Rust, built like `ntdll.dll`.** `personalities/xnu/libsystem` is a `no_std` crate, its own
   workspace, built for `x86_64-apple-darwin` with `-Zbuild-std=core,compiler_builtins` under
   `RUSTC_BOOTSTRAP=1` (ADR 0071's arrangement) and `-Ctarget-cpu=x86-64` (the target's default
   CPU assumes SSSE3, which QEMU's default CPU does not have). One build per library, each with
   that library's feature, as a static library. The logic is in the host-tested
   `crates/poly-libsystem`: the `printf` engine, `malloc`'s allocator, the `atexit` table, the C
   string and number functions and stdio buffering.
3. **The linker is `rust-lld`'s Mach-O flavour, on every host.** Each archive is linked with
   `rust-lld -flavor darwin` (LLVM's `ld64.lld`, shipped inside every Rust toolchain): `-dylib`,
   the install name and versions, `-exported_symbols_list exports/<library>.txt`,
   `-fixup_chains`, `-dead_strip`, `-platform_version macos 13.0` (Chrome's minimum, ADR 0157); a
   library that calls another links against the dylib just made, and libSystem gets
   `-reexport_library` for the six. Nothing of Apple's takes part, so the Linux container of the
   PC and a Windows host build the same files with the same command (`cargo xtask libsystem`,
   ADR 0124). Alternatives: Apple's `ld` works on a Mac only; a committed or downloaded prebuilt
   dylib is not allowed; a Poly-written Mach-O dylib writer would redo what `ld64.lld` does
   (export tries, chained fixups, re-exports) and can still come if `ld64.lld` ever falls short;
   C sources would need a host C toolchain, which CLAUDE.md rules out.
4. **Cached and checked.** As for `ntdll.dll`, the dylibs are cached under a hash of their inputs
   (`target/libsystem-cache/<hash>/` of the main checkout). Every build is read back with
   `poly-macho`: an x86-64 dylib, the install name and versions, only Poly's libraries loaded, the
   re-exports, exactly the listed exports, and every exported name one the SDK's `.tbd` of that
   library has (a host test checks the lists too). `cargo xtask ci lint` runs rustfmt and clippy
   on the crate for its target.
5. **System calls.** libsystem_kernel's wrappers are a few instructions each, with the sheets'
   convention: `rax` = 0x2000000 | number, the fourth argument in `r10`, `syscall`; carry set
   means `rax` is the errno, which goes to `errno` and the wrapper returns -1. Mach traps
   (0x1000000 | number) return their `kern_return_t`. They cover every call that has a sheet, under
   the names the SDK exports them by (`stat$INODE64`, `__psynch_mutexwait`, `__ulock_wait`,
   `mach_msg_trap` …; `$NOCANCEL` forms are the plain calls); arguments past the sixth stay on
   the stack where the caller put them. Functions the SDK declares over a trap with the same
   arguments (`mach_port_allocate`, `mach_vm_allocate`, `semaphore_signal` …) are the trap;
   `semaphore_timedwait*` split their `mach_timespec_t`. `mach_task_self_` is read at start.
6. **`errno` is a Mach-O thread-local variable** of libsystem_kernel, reached through
   `__error()`; Poly's dyld gives its descriptor the thunk (ADR 0237), so no `_tlv_bootstrap` runs.
7. **Variadic functions without unstable Rust.** `printf`, `fprintf`, `sprintf`, `snprintf`,
   `dprintf`, `asprintf`, `__sprintf_chk` and `__snprintf_chk` are short assembly trampolines that
   store the argument registers in a System V register save area, build a `va_list` over it and
   call their `v` function; the format engine reads integers and `double`s from their own
   sequences as the ABI's `va_arg` does.
8. **stdio keeps the SDK's `FILE`.** `__stdinp`, `__stdoutp`, `__stderrp` and `__sF` are 152-byte
   `struct __sFILE`s as `<_stdio.h>` lays them out; `_w` and `_r` stay 0, so the header's inline
   `putc`/`getc` macros always call `__swbuf`/`__srget`; `_flags` carries `__SERR`/`__SEOF` and
   `_file` the descriptor, which the other macros read. The buffer is Poly's, behind `_cookie`.
   `stdout` is line buffered (a terminal cannot be asked for yet), `stderr` unbuffered; `exit`
   runs the `atexit` and `__cxa_atexit` functions (last first), flushes, and calls `_exit`.
9. **dyld tells libdyld about the images.** The start code now calls every initializer with the
   address of its data block as the fifth argument (it was 0), and the block has an image table
   after the initializers (`"PolyImgs"`, the count, header, slide and path of each image, program
   first; `poly_dyld::start`). libdyld's initializer reads it, and `apple[0]`, argc, argv and
   envp: `_dyld_image_count`, `_dyld_get_image_header`/`_name`/`_vmaddr_slide`,
   `_dyld_get_prog_image_header`, `_NSGetExecutablePath`, `NXArgc`, `NXArgv`, `environ`,
   `__progname`. `dyld_stub_binder` and `_tlv_bootstrap` exist for the static linker only and
   stop the program with a message if called.
10. **malloc** hands out power-of-two blocks with a 16-byte header (size with a check value, offset
    for aligned blocks), free lists per size, never gives memory back, and stops the program on a
    `free` of a pointer it did not hand out. Arenas come from `mmap`, or from an 8 MiB zero-fill
    area of its own while the kernel has no `mmap` for Mach-O programs.
11. **v0 is single-threaded.** Nothing locks; pthread is the main thread (`pthread_self`,
    mutexes of the three kinds the SDK's initializers name, `pthread_once`, 512 keys).
    `os_unfair_lock` and `OSSpinLock` spin; `setjmp`/`sigsetjmp` do not save the signal mask.

## The libc sheets

spec-macos-5's sheets (`specs/macos/libsystem_c.dylib`, `libsystem_malloc.dylib`,
`libsystem_pthread.dylib`, `libdyld.dylib`) landed while this change was open; where they cover
what this change implements, it follows them:

- `snprintf`: positional arguments work (an unnumbered conversion after a numbered one takes the
  next number); an unknown conversion prints its letter (`%5%` pads); a format that ends inside a
  conversion prints nothing more; `%lc`/`%ls` keep a character up to 0xFF as its byte and fail
  with `EILSEQ` above; a null `%p` is `0x0`, a null `%s` `(null)` cut by the precision; `%La`
  prints the 80-bit value as it is (`0x8p-3` for 1.0).
- `malloc_size`/`malloc_good_size`: x86-64's classes (multiples of 16 up to 1024, of 512 up to
  32 KiB, then of 32 KiB); `malloc_size(malloc(n))` is `malloc_good_size(n)`; 0 for a freed block
  or an address malloc does not own, which the heap tests against its arenas before reading.
- `pthread_key_create`/`pthread_setspecific`: keys are numbered from 258, 509 of them; a deleted
  key reads NULL and is `EINVAL` to set. `pthread_mutex_trylock`: a mutex the caller holds is
  `EBUSY` to trylock (normal and error-checking), a destroyed one `EINVAL`.
- `atexit`/`__cxa_atexit`: one list of 4096 (the sheet saw 1000 succeed). `getenv`: an empty name
  or one with `=` is NULL.

The entries those sheets cover stay claimed by spec-macos-5 until their hand-off; the rest of this
change's entries are claimed by `impl-macos-libsystem`. Poly's own choices the sheets do not
show: a NaN keeps its sign; `%a` normalizes a subnormal `double`; `long double` conversions other
than `%La` go through `double`; `puts` and `fputs` return 0; `abort` sends SIGABRT and, if that
returns, exits with 134; the stack protector's canary has a zero low byte; the messages of
`__stack_chk_fail`, the `__*_chk` checks and a bad `free` are Poly's own.

## Tests

- `crates/poly-libsystem`: the format engine (every conversion, flag, size and the `va_list`
  reader), the allocator (alignment, reuse, double free, `realloc`, arenas, a random sweep), the
  `atexit` table, the string and number functions, stdio buffering and the `FILE` layout.
- `poly-dyld`: the start block with its image table, and the start code's fifth argument.
- `tools/xtask/src/libsystem.rs`: the export lists against the SDK map, the build's inputs.
- Boot test group `libsystem` (shard `userland`): `tests/macos/libsystem/hello.c`, built on a Mac
  by `generate.sh` with clang and Apple's ld against the SDK, as `/bin/mhello-libsystem` with
  Poly's seven dylibs in `/usr/lib`; it prints through `printf`, `puts`, `putchar` and `stderr`,
  uses `malloc`/`calloc`/`realloc`, `snprintf` (fortified), `asprintf`, `errno`, the number and
  string functions, `getenv`, the dyld and `crt_externs` accessors, the stack protector and
  `atexit`, and must exit 0.

## Waiting for other work

- The BSD layer in the kernel (ADR 0246) has landed: `mmap`, `getentropy`, `getpid`, `kill`,
  `read` and the other wrapped calls reach the kernel. Until then malloc used its own 8 MiB area,
  the canary was mixed from the time stamp counter and `abort` ended with status 134; those
  fallbacks stay for a kernel that answers `ENOSYS`.
- Threads (`bsdthread_create`): locks, per-thread `errno` blocks and keys.
- `sigaction` with its trampoline and `sigreturn`, `fork`, `posix_spawn`'s attribute objects.
- Running the libc sheets' visible tests on Poly (they need the macOS test harness on Poly) and the
  hand-off of their entries.
