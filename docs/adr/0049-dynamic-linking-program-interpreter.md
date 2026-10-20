# 0049: Dynamic linking: exec loads the program interpreter next to the program; glibc's own `ld.so` runs unchanged

**Status:** Accepted. Builds on [0024](0024-elf-loader.md) (load plans,
the initial stack, the plan for `PT_INTERP`), [0036](0036-demand-paging-and-exec.md)
(memory maps, exec), [0041](0041-processes-threads-fork-exec-wait.md)
(`execve`), [0043](0043-dev-proc-sys-and-file-system-calls.md) (the file
system calls, the `rseq` decision) and [0044](0044-time-syscalls-timers-and-the-vdso.md)
(the vDSO, `AT_SYSINFO_EHDR`).

## Context

Phase 4 asks for "dynamic linking: glibc's own `ld.so` loads shared
libraries unchanged". Exec loaded static executables and static PIEs and
refused anything with `PT_INTERP`; `execve`, file mappings (private copy on
write of page-cache pages, `MAP_SHARED`), `mprotect`, the file system calls,
`/proc/self/exe` and the vDSO existed, and a static glibc "hello world" ran.
Nearly every Linux program is dynamically linked, so this is the gate to
running real software.

Sources: the System V gABI ("Program Interpreter", "Dynamic Section",
"Program Loading"), the AMD64 psABI (§3.4.1, the initial stack and the
auxiliary vector), elf(5), ld.so(8), execve(2), getauxval(3), dlopen(3),
dlsym(3), dladdr(3), madvise(2), mremap(2), rseq(2), mmap(2), the public
`<elf.h>` and the UAPI headers `asm-generic/mman-common.h`,
`asm-generic/errno-base.h` and `asm-generic/errno.h`. No Linux or C
library source was read, and nothing was run, traced or observed on a real
Linux system: the system calls glibc's loader makes were found by running
it on Poly and reading Poly's own `POLY-SYSCALL` records.

## Decision

**exec loads the interpreter; the caller opens it.**
`user::exec::load_dynamic(file, params, open_interp)` parses the program
and plans it as before (an `ET_DYN` program at `0x5555_5555_4000`, where
Linux puts PIEs with randomisation off). If it names an interpreter,
`open_interp(path)` opens it: the kernel's `program::prepare` opens it
through the VFS like the program (`Vfs::open_exec`: a regular file with
execute permission), `execve` through the calling process's files
(`Files::open_exec_at`, relative paths from the working directory). The
`user` module stays independent of the VFS, and the kernel tests hand it an
interpreter from memory. `exec::load` is `load_dynamic` with an opener that
refuses (`ExecError::Interpreter`, `ENOEXEC`). Errors follow execve(2): the
open's errno (`ENOENT`, `EACCES`, ...) for an interpreter that cannot be
opened, `ELIBBAD` for one that is not a loadable ELF image or names an
interpreter itself (`poly_elf::check_interpreter`); all are found before
anything is built.

**Where the interpreter goes.** An `ET_DYN` interpreter is placed in free
space the way an `mmap` without an address is: `poly_elf::Footprint` gives
the span of its segments and the alignment its base needs (the largest
`p_align`); the kernel reserves `size + align - page` bytes with the
region map's top-down search, gives the reservation back and maps the
segments at `Footprint::base_in(hole)`, the first aligned address in it,
so the image fits whatever its alignment. Nothing else maps into the new
memory meanwhile (it is not running), so the hole stays free. It is the
first mapping of the `mmap` area (just under the stack's 128 MiB reserve
and guard gap), the vDSO comes right below it. An `ET_EXEC` interpreter
(the gABI allows one) goes to its link addresses (`ENOMEM` if they are
taken). No address is randomised yet: every run of a program sees the same
layout, as on Linux with randomisation off, which keeps traces comparable.
Randomisation later is a different hole and base, nothing else.

**What the program and the interpreter get.** The auxiliary vector
describes the program (`AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_ENTRY`),
`AT_BASE` is the interpreter's load bias (`AuxInfo::new(&exe,
Some(&interp), ..)`), the thread starts at the interpreter's entry point
(`start_address`), the program break follows the program and the stack's
executability is the program's `PT_GNU_STACK`. `/proc/<pid>/exe` names the
program, `/proc/<pid>/maps` names both files (their mappings are
`FileSource`s). Running the interpreter as the program (`ld.so /bin/prog`,
ld.so(8)) is the exec of a static PIE: `AT_BASE` 0, the interpreter at
`0x5555_5555_4000`; it maps the program itself.

**System calls.** With the loader in place glibc's `ld.so` and C library
startup ran on Poly with what the earlier milestones built: `openat` with
`O_CLOEXEC`, `access("/etc/ld.so.preload")` and `openat` of
`/etc/ld.so.cache` failing with `ENOENT` (there is no cache; ld.so(8) then
searches `DT_RUNPATH`, the default directories, ...), `newfstatat`,
`pread64`, `mmap` of a reservation followed by `MAP_FIXED` mappings of the
segments over it (`MAP_DENYWRITE` is ignored, as mmap(2) says),
`mprotect` of the RELRO range, `arch_prctl(ARCH_SET_FS)`,
`set_tid_address`, `set_robust_list`, `prlimit64`, `getrandom`, and
`readlinkat` returning the program's path (for `$ORIGIN`, which
`/proc/self/exe` answers). One call was missing:

- **`madvise`** (`sys/madvise.rs`, its own list in the table): the thread
  library gives a finished thread's stack back with `MADV_DONTNEED`. The
  personality checks the arguments (madvise(2): unknown advice, an
  unaligned address or a wrapping range are `EINVAL`, a zero length
  succeeds) and sorts the advice: `MADV_DONTNEED` (and `_LOCKED`) drops
  the pages, `MADV_FREE` the same for private anonymous memory only
  (freeing at once is allowed: "the kernel can free these pages"),
  `MADV_REMOVE` is `EINVAL` except on a shared writable mapping, where
  it is `EOPNOTSUPP` (no file system of Poly frees a file's range yet), the
  pure hints are accepted, and advice that changes behaviour Poly does not
  have (`MADV_WIPEONFORK`, `MADV_KEEPONFORK`, `MADV_POPULATE_*`,
  `MADV_COLLAPSE`, `MADV_HWPOISON`, ...) is `EINVAL`, as on a kernel that
  predates it: accepting `MADV_WIPEONFORK` without wiping would leave, say,
  a random generator's state in a forked child. `MADV_DONTFORK` is
  accepted, but a forked child still gets a copy of the range. The kernel
  (`Memory::advise`, `user/memory/advice.rs`) drops a page the way
  unmapping does (the frame is freed, a shared frame or page-cache page
  loses a user) while the region stays, so the next access finds zeros or
  the backing object's contents. The mapped parts of a range get the
  advice; a hole makes the call fail with `ENOMEM` afterwards.

**`rseq` stays `ENOSYS`** (ADR 0043). rseq(2) lets the kernel abort a
thread's restartable sequence when it is preempted, migrated or gets a
signal; registering without ever aborting breaks that contract the first
time a thread is preempted inside a critical section. glibc's loader and
C library treat `ENOSYS` as "no rseq" and every test program runs. A real
implementation needs the abort on every return to user mode after a
preemption or signal delivery; it can come with SMP, where it matters.

**`mremap` is not implemented.** None of the test programs calls it.
glibc's `realloc` of large `mmap`ed blocks may; it is left for the
program that needs it (it means moving pages between addresses in
`Memory`, which the fork work is changing now).

**Test programs come from the host's toolchain, the C library from the
host's files.** `tools/xtask/src/dynamic.rs` builds C programs at test time
with the host's compiler, without `-static` (`tools/xtask/programs/dynamic/`):
`dyn-hello` (stdio, heap, environment, `getauxval`), `dyn-hello-nopie`
(the same as `ET_EXEC` with `PT_INTERP`), `dyn-dlopen` (`dlopen`,
`dlsym`, `dladdr`, `RTLD_NOLOAD`, `dlclose` of a plugin found through
`DT_RUNPATH` `$ORIGIN/../lib/poly` that needs libm itself), `dyn-threads`
(threads, TLS, mutexes, condition variables, `pthread_once`, key
destructors) and, with a C++ compiler, `dyn-cxx` (libstdc++ containers,
streams, exceptions through libgcc_s). The image gets the interpreter the
programs name and the `DT_NEEDED` closure of the programs and the plugin,
read with `poly_elf::Dynamic` (`DT_NEEDED`, `DT_SONAME`, `DT_RPATH`,
`DT_RUNPATH`) and searched in the object's own path, the directories of
`/etc/ld.so.conf` and the default ones (ld.so(8)); nothing is run on the
host to find them (no `ldd`). Each library goes to the directory it was
found in, with links from `/lib`, `/usr/lib`, `/lib64` and `/usr/lib64`, so
the loader's default search finds it without `/etc/ld.so.cache`. No glibc
file is committed. Without a C compiler or a shared C library the boot
tests say so and are skipped.

**poly-elf** gains `interp` (`Footprint`, `check_interpreter`,
`interpreter_options`, `is_relocatable`) and `dynamic` (`Dynamic`: the
validated dynamic array and string table of a file, never panicking on
damaged input), both host-tested, the latter also against `readelf -d` of
real programs and with the host's own `ld.so` planned at a base.
`LoadPlan::plan` plans into an existing plan: exec keeps both plans on
the heap (`Box::new(LoadPlan::EMPTY)`, then `plan`), never a 1.6 KiB copy
on the kernel stack. Kernel stacks are 16 KiB without a guard page, and
exec runs deep (a shell command, or `execve` from a system call): with a
second plan by value the shell's `run` of a dynamically linked program
overflowed its stack. The exec path's frames are now smaller than before
this change.

**The shell's `run` takes an environment**: `run NAME=value ...
<program> [args]` (leading words with `=` and a name without `/`, as
env(1) reads them), so the loader's own switches can be used on Poly.

## Consequences

- Dynamically linked glibc programs run unchanged: `dyn-hello`,
  `dyn-hello-nopie`, `dyn-dlopen`, `dyn-threads` and `dyn-cxx` as the first
  process and from the shell's `run`, and `run
  /lib64/ld-linux-x86-64.so.2 /bin/dyn-hello`; the only call without a
  handler is `rseq`. `LD_DEBUG`, `LD_LIBRARY_PATH`, `LD_BIND_NOW` and the
  loader's options (`--list`) work as ld.so(8) describes, since the loader
  is glibc's own (`run LD_DEBUG=libs /bin/dyn-hello`).
- `ExecError` has two more variants (`InterpreterOpen`, `BadInterpreter`)
  and `ImageInfo` the interpreter's placement (`InterpInfo`).
- The `LinuxKernel` trait has one more method, `madvise`, with an `ENOSYS`
  default.
- Kernel tests: a PIE and an `ET_EXEC` program naming a tiny interpreter
  of our own (`user/test_elf.rs`: an `ET_DYN` image that checks
  `AT_BASE` against its own load address, reports `AT_ENTRY` and
  `AT_PHDR` and jumps to the program) run from ring 3 and check the
  layout and the auxiliary vector; refused interpreters (missing, not
  ELF, nested) leave no frame behind; `Memory::advise` frees exactly the
  dropped frames and gives zeros, file contents and shared contents back.
- Not done yet: address space randomisation (`AT_BASE`, the PIE base and
  `mmap` are fixed), `mremap`, `rseq`, `MADV_DONTFORK` and
  `MADV_WIPEONFORK` semantics, `MADV_REMOVE` on tmpfs, `AT_SECURE`
  handling for set-user-ID programs (there are no other users yet), the
  `ioctl(TCGETS)` the C library makes on its standard output (`ENOTTY`
  until the TTY layer).
