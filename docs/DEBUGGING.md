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

The negative boot tests use these `poly.selftest.*` flags, which fault on
purpose after the in-kernel tests: `page_fault` (unmapped address),
`write_text` (write to kernel code), `exec_stack` and `exec_data` (jump into
no-execute memory) and `stack_overflow` (run the boot stack into its guard
page). Each prints the address it is about to touch first.

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

A page fault's error code says what went wrong (SDM Vol. 3A §4.7): bit 0 set
means the page was present (a protection violation, not a missing page),
bit 1 a write, bit 4 an instruction fetch. So `error 0x3` is a write to a
read-only page such as kernel code, and `error 0x11` is executing no-execute
memory (then `rip` and `cr2` are equal).

A kernel stack overflow runs into the unmapped guard page below the stack.
The page fault cannot be delivered on the full stack, so it arrives as a
double fault on its own stack, and the report says which stack overflowed:

```text
CPU exception 8 (double fault), error 0x0, rip 0xffffffff8010f07b, rsp 0xffffffff80127fb0, cr2 0xffffffff80127fa8
kernel stack overflow: hit the guard page below the boot stack
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

A thread that locks a `Mutex` it already holds panics with `deadlock: thread
N locks a mutex it already holds` and a backtrace of its stack; `cargo xtask
run poly.test=sync::deadlock poly.selftest.deadlock` shows what this looks
like. Blocking or yielding while holding a spinlock panics too (`... while
holding a spinlock`), as does blocking inside an interrupt handler. A thread
that waits on a `WaitQueue` nobody wakes is not detected: with interrupts on,
every thread being blocked is normal (they may all be sleeping), so the
kernel just idles. Attach a debugger and look at the threads' stacks, or
watch `monitor info lapic` to see whether interrupts still arrive.

## Interrupts and the timer

The boot log shows what was set up:

```text
irq: 8259 PICs remapped to vectors 0x20-0x2f, all masked
apic: local APIC id 0 version 0x14, xAPIC mode at 0xfee00000, 6 LVT entries, spurious vector 0xff, 1 LINT input(s) wired to NMI
apic: I/O APIC 0 at 0xfec00000: version 0x20, GSI 0-23, all masked
time: HPET at 0xfed00000: 100.000 MHz, 64-bit counter, 3 timers
time: TSC 2699.992 MHz, calibrated against the HPET (5 of 6 measurements, spread 14 ppm), invariant: no
time: clock source HPET (TSC not invariant)
time: APIC timer 62.499 MHz (input clock / 16), periodic tick 1000 Hz on vector 0xf0
irq: interrupts enabled
```

An interrupt on a vector without a handler prints `irq: interrupt on vector
0x..., which has no handler` (the first few times) and is acknowledged; an
APIC error prints its ESR. The QEMU monitor shows the controllers' state.
Run with `cargo xtask run --gdb` and use `monitor <cmd>` from the debugger, or
add `-monitor stdio` to a hand-written QEMU command line:

- `info lapic`: the local APIC of the current CPU: the LVT (the timer entry
  should be periodic, vector 0xf0, unmasked), the timer's initial and
  current count, and the ISR / IRR bitmaps. A vector stuck in the ISR means
  a handler never sent its EOI; one stuck in the IRR means interrupts are
  disabled (a spinlock held too long?) or a higher-priority vector is in
  service.
- `info pic`: the 8259s and the I/O APIC redirection table: every input
  should be masked except the ones a driver routed.
- `info irq`: how many interrupts each source has raised, to see whether a
  device fires at all.

`info registers` shows `RFLAGS` (bit 9, IF: interrupts enabled). In gdb,
`print poly_kernel::arch::x86_64::time::TICKS` gives the tick count, and a
breakpoint on `poly_kernel::arch::x86_64::irq::dispatch` stops at every
interrupt (expect one per millisecond from the tick).

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
- Physical memory is visible at `0xffff800000000000 + phys` (the direct
  map), e.g. `x/4gx 0xffff800000000000 + 0x100000`. `monitor info mem`
  lists every mapping with its permissions; `monitor xp /4gx 0x100000` reads
  physical memory directly.
- To find a kernel test in the debugger, break on its function: tests are
  ordinary functions named after the test (`poly_kernel::sync::spinlock_excludes_until_guard_dropped`).
