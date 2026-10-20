# Debugging the kernel

Three tools, from quickest to most thorough: the serial log, the panic
backtrace, and a debugger attached to QEMU.

## Serial log

Everything the kernel prints goes to COM1, which `cargo xtask run` connects
to your terminal. Anything after the command is passed on as the kernel
command line:

```sh
cargo xtask run                          # plain boot
cargo xtask run poly.test=sync           # run only the in-kernel tests whose name contains "sync"
cargo xtask run poly.selftest.page_fault # boot, then fault on purpose
```

To keep a copy, add `| tee serial.log`.

## In-kernel tests

Every boot runs the tests registered with `kernel_test!` (see
`kernel/src/ktest.rs`) and prints one line per test:

```text
ktest: running 6 of 6 tests
test selftest::cpu_exceptions_return ... ok
test sync::spinlock_excludes_until_guard_dropped ... ok
test result: ok. 6 passed; 0 failed; 0 filtered out
POLY-SELFTEST: PASS
```

A test is a function next to the code it checks; it returns `Ok(())` or
`Err("reason")`:

```rust
crate::kernel_test! {
    fn spinlock_excludes_until_guard_dropped() -> TestResult {
        // ...
        Ok(())
    }
}
```

Write `fn name(env: &TestEnv) -> TestResult` to get the boot info and memory
map. `cargo xtask test` fails if any test fails, if the summary line is missing
(the kernel crashed during the tests), or if the counts do not add up. Only
put tests here that need the real machine; pure logic belongs in a crate under
`crates/` with ordinary `#[test]`s.

## Panic backtraces

The kernel is built with frame pointers, so the panic handler can walk the
stack. It prints raw addresses; frame #0 is the faulting instruction for a CPU
exception (otherwise a point in the panic path), later frames are return
addresses:

```text
KERNEL PANIC: panicked at src/arch/x86_64/idt.rs:241:13:
CPU exception 14 (page fault), error 0x0, rip 0xffffffff8010776c, ...
backtrace:
  #0 0xffffffff8010776c
  #1 0xffffffff801070bc
  #2 0x00000000001000d5
```

`cargo xtask test` adds symbols as it captures the boot output. For a log you
captured yourself, or addresses copied from anywhere:

```sh
cargo xtask symbolize < serial.log          # rewrite every backtrace line in a log
cargo xtask symbolize 0xffffffff8010776c    # one or more addresses
cargo xtask symbolize --release ...         # use the release kernel
cargo xtask symbolize --elf path/to/kernel  # any other kernel ELF
```

```text
backtrace:
  #0 0xffffffff8010776c poly_kernel::selftest::trigger_page_fault+0xcc
  #1 0xffffffff801070bc kernel_entry+0x81c
  #2 0x00000000001000d5 pvh_start64+0x2b
```

Symbols come from the ELF that was built last, so symbolise before you rebuild.
For file and line numbers, use `addr2line -f -C -i -e
kernel/target/x86_64-unknown-none/debug/poly-kernel <addr>` (for frames after
#0, subtract 1 from the address to get the line of the call).

The walk stops at the bottom of the stack, after 32 frames, or at the first
frame pointer that is outside the kernel's stacks, misaligned or not moving up
the stack; the reason is printed when it is not the bottom. A corrupt stack
therefore gives a short backtrace, never a second fault.

A panic on a kernel thread walks that thread's own stack, which ends in the
code every thread starts in:

```text
  #3 0xffffffff8010560c poly_kernel::thread::panic_in_thread+0x1c
  #4 0xffffffff80105558 poly_kernel::thread::thread_start+0x18
  #5 0xffffffff8010d8db thread_trampoline+0x9
```

If every thread blocks, nothing can wake them yet (no interrupts), so the idle
thread panics with `deadlock: every thread is blocked ...` and a backtrace
through `poly_kernel::thread::idle_main`. To see where the other threads were
blocked, attach a debugger before the panic. `cargo xtask run
poly.test=thread::deadlock poly.selftest.deadlock` shows what this looks like.

## Attaching a debugger

```sh
cargo xtask run --gdb [kernel args...]
```

QEMU starts with the CPU stopped (`-S`) and a GDB server on localhost:1234
(`-s`). The kernel ELF has full debug info in the default (debug) build.

The first instructions run at physical addresses with paging off, and the
kernel's memory is only filled in by the loader once the CPU runs. Software
breakpoints set before that are lost, so stop in `kernel_entry` with a
**hardware** breakpoint first; after that, ordinary breakpoints work.

### gdb

```sh
gdb kernel/target/x86_64-unknown-none/debug/poly-kernel \
    -ex 'target remote :1234' -ex 'hbreak kernel_entry' -ex continue
```

```text
(gdb) break poly_kernel::selftest::trigger_page_fault
(gdb) continue
(gdb) bt
#0  poly_kernel::selftest::trigger_page_fault () at ...
#1  0xffffffff801070bc in poly_kernel::kernel_entry (start_info=<optimized out>) at src/main.rs:70
#2  0x00000000001000d5 in pvh_start64 ()
(gdb) info registers rip rsp cr2 cr3
(gdb) x/8gx $rsp
(gdb) monitor info mem
```

`next`, `step`, `finish` and `print` work as usual; with `opt-level = 1` some
locals show as `<optimized out>` and inlined code (such as `kprintln!`) shows
the line of the inlined function. `monitor <cmd>` runs a QEMU monitor command:
`monitor info mem` and `monitor info tlb` show the page tables.

### lldb

lldb needs to be told that the kernel runs at its link address:

```sh
lldb kernel/target/x86_64-unknown-none/debug/poly-kernel
(lldb) gdb-remote 1234
(lldb) target modules load --file poly-kernel --slide 0
(lldb) breakpoint set --hardware --name kernel_entry
(lldb) process continue
(lldb) breakpoint set --name poly_kernel::selftest::trigger_page_fault
(lldb) process continue
(lldb) bt
```

### Tips

- The kernel stops in `cli; hlt` after a panic. Interrupt the debugger (Ctrl-C)
  and `bt` still shows where it stopped; the panic message is in the serial log.
- `info registers` (gdb) / `register read` (lldb) also show the control
  registers (`cr0`, `cr2`, `cr3`, `cr4`), handy for page faults.
- To find a kernel test in the debugger, break on its function: tests are
  ordinary functions named after the test (`poly_kernel::sync::spinlock_excludes_until_guard_dropped`).
