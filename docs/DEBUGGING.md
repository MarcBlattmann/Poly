# Debugging the kernel

Three tools, from quickest to most thorough: the serial log, the panic
backtrace, and a debugger attached to QEMU.

## Serial log

Everything the kernel prints goes to COM1, which `cargo xtask run` connects
to your terminal, shared with QEMU's monitor: Ctrl-A C switches between the
two, Ctrl-A X quits. Anything after the command is passed on as the kernel
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

## PCI devices and the test disk

The boot log has one `pci:` line per function (address, vendor:device,
class, BARs with their sizes) and the virtio-blk bring-up:

```text
pci: 00:03.0 1af4:1042 class 01.00.00, BAR1 mem32 0xfebd5000 4 KiB, BAR4 mem64 pf 0xfe000000 16 KiB
virtio-blk: 00:03.0: MSI-X with 3 vectors, table BAR1+0x0, PBA BAR1+0x800; left disabled, polling
virtio-blk: 00:03.0: 1 MiB (2048 sectors), block size 512, features 0x100000240 of 0x10130007e54, queue 16 entries of 256
virtio-blk: sector 0: 50 4f 4c 59 00 00 00 00 08 09 0a 0b 0c 0d 0e 0f  POLY............
```

`features` is what the driver accepted, of what the device offered (bit 32
is VERSION_1; the virtio-blk bits are in VIRTIO 1.2 §5.2.3). To compare with
QEMU's own view, open the monitor (Ctrl-A C under `cargo xtask run`, or
`monitor <cmd>` from gdb) and use:

| Command | Shows |
|---|---|
| `info pci` | every function with its IDs, IRQ pin and BAR addresses as QEMU sees them |
| `info qtree` | the device tree, including the virtio-blk properties and MSI-X vector count |
| `info block` | the drives and the image file behind `disk0` |
| `info mtree` | the guest physical address map: where ECAM (`pcie-mmcfg-mmio`) and each BAR sit |

`cargo xtask run` and the boot tests attach a fresh test image,
`target/poly-test-disk.img` (1 MiB, raw), as a modern-only virtio-blk device
(`-device virtio-blk-pci,drive=disk0,disable-legacy=on`) and pass
`poly.test.disk`, which makes the `virtio_blk::*` tests require it. Every
512-byte sector starts with "POLY" and its number (le32); the rest is a
pattern of the sector and the offset (`virtio_blk::test_pattern`):

```sh
od -A x -t x1z -N 16 -j $((512 * 7)) target/poly-test-disk.img   # sector 7
cargo xtask run --no-disk                                       # boot without the disk
cargo xtask run poly.test=virtio_blk                            # only the disk tests
```

The write test changes sector 1 and puts it back, and xtask rewrites the
image before every boot anyway. To watch the device side, run QEMU by hand
with the same arguments plus QEMU trace events, e.g. `-trace
'virtio_blk_*' -trace virtio_queue_notify -trace 'pci_cfg_*'` (printed on
stderr with the log trace backend that distribution builds use).

A request that never comes back fails after a spin budget of a few seconds
with `Timeout`; the driver then resets the device and every later request
fails with `Broken`. Interrupts are not used yet: MSI-X is reported, not
enabled.

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
