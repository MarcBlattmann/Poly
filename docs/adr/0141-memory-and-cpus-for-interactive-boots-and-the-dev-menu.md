# 0141: Memory and CPUs for the interactive boots, and the dev menu's resources section

**Status:** Accepted. The owner's request: "can we also extend the dev menu so
i can give more resources etc to the system etc yk?" It extends
[ADR 0133](0133-dev-menu-and-native-wayland-test-clients.md).

## Context

- Every boot had its guest size written into its command: the kernel shell
  and BusyBox 256M (`DEFAULT_MEMORY`), the desktop 512M, the root disk 1G,
  the toolkits 2G, Chromium 4G with 4 CPUs; all of them 2 CPUs
  (`QEMU_CPUS`) except Chromium. The only way to give the guest more was to
  edit the source.
- `qemu(...)` in `main.rs` is the one QEMU builder, but it hard-coded
  `-smp`, and Chromium's run added a second `-smp` after it.
- The boot tests and CI must stay deterministic: their sizes are part of
  what they check (the self-test boots check that ACPI lists as many CPUs as
  `-smp` says and that all come up).

## What the kernel does with more

Checked in the code and in the boot log (below):

- **Memory.** The frame allocator (`poly-mem`'s buddy allocator) is built
  from the firmware memory map (e820, QEMU's `-m`) at boot. It takes any
  amount: its bitmaps cover physical addresses below 2^52 (4 PiB, `MAX_PHYS`
  in `buddy.rs`) and its metadata is carved from usable RAM, so it grows
  with the address span. The kernel's direct map has room for 64 TiB
  (`DIRECT_MAP_SIZE`). So **the kernel has no practical RAM limit**; the
  limit is the host's. QEMU's q35 machine puts RAM above 2.75 GiB at
  4 GiB and up; the memory map has the second region and the allocator
  spans the hole.
- **CPUs.** The kernel starts every processor the MADT lists (the
  application processors through INIT/SIPI, `arch/x86_64/smp.rs`), up to
  `percpu::MAX_CPUS`, **16**; `-smp 32` is emulated by QEMU but the kernel
  leaves CPUs 16 and up stopped (`poly.cpus=N` / `poly.smp=off` lower it).
  The boot log says it: `smp: 4 of 4 CPUs online`. The menu never offers
  more than 16, and a unit test (`resources::the_kernels_cpu_limit_is_known`)
  fails if the kernel's constant changes without the menu following.

### Measured

`cargo xtask desktop --release` booted in QEMU (TCG, in the poly-dev
container), serial log lines:

| flags | `mem:` | `frames:` | CPUs |
|---|---|---|---|
| none | 502 MiB usable | 128651 frames (502 MiB), metadata 64 KiB | `smp: 2 of 2 CPUs online (0-1)` |
| `--memory=4G --cpus=4` | 4086 MiB usable | 1045974 frames (4085 MiB), metadata 776 KiB | `smp: 4 of 4 CPUs online (0-3)` |
| `--memory=8G --cpus=20` | 8182 MiB usable (a second region at 4 GiB) | 2094417 frames (8181 MiB), metadata 1292 KiB | `--cpus=20: the kernel starts at most 16 CPUs; using 16`, then `smp: 16 of 16 CPUs online (0-15)` |

The page cache grows with the memory (125 MiB, 1021 MiB, 2045 MiB), and the
compositor reached `desktop ready` in each.

## Decision

### One module, flags only (`tools/xtask/src/resources.rs`)

- `--memory=SIZE` (`4G`, `3072M`, a bare number is MiB as in QEMU's `-m`;
  at least 128M) and `--cpus=N` (1 or more), or `auto` for the command's own
  default. They are flags of `desktop`, `run` (so also `--busybox` and
  `--disk-root`), `chromium run` and `toolkits`, the commands that boot for a
  person. **No environment variable**, so `cargo xtask start --print`
  shows them and `--last` repeats them.
- `Resources::parse` reads them against the host's limits. What is asked for
  beyond the host is **clamped, with a note** (`==> --memory=64G: ... using
  12G`), not an error, so a saved choice made on a bigger machine still runs.
  A wrong value (`--memory=lots`, `--cpus=0`) is an error that says what is
  expected.
- `qemu_with(kernel, memory, cpus, ...)` is the one place `-m` and `-smp` are
  built; `qemu(...)` calls it with the fixed `QEMU_CPUS`, and the boot
  tests keep calling `qemu(...)`, `boot_with(...)` and the like with their own
  constants: nothing in a boot test reads `Resources`. Each interactive
  command prints what the guest got (`==> guest: 4G of memory, 4 CPUs
  (--memory=, --cpus=)`).
- Chromium's run no longer adds a second `-smp`: its default of 4 CPUs is the
  `cpus` it passes.

### What the host can give

- Total RAM: `GlobalMemoryStatusEx` on Windows, `MemTotal` of
  `/proc/meminfo` on Linux, unknown elsewhere (then nothing is greyed or
  clamped).
- The guest may have the host's RAM less a quarter (at least 1 GiB) that
  stays for the host: a guest of all of it would make the host swap. 16 GiB
  of host RAM offers up to 12G.
- CPUs: the host's logical cores, and no more than the kernel starts (16).

### The menu's resources section

A "Resources for ..." block under the entry's options, for the desktop,
Chromium, toolkits, the kernel shell, BusyBox and the root disk's bash (not
for boot tests, CI or building Chromium's disk, which keep their sizes):

- **Memory**: `auto` (shown as the entry's default, such as `= 512M, the
  default`) and presets 512M, 1G, 2G, 4G, 8G, 16G. Left/Right step through
  what the host can give; the others are drawn dim on a line under it
  with the reason ("16G greyed: this host has 16.0 GiB; 12G at most, the rest
  stays for it"). Enter types any size. A size below the entry's default is
  allowed but warned ("below the default 4G: it may not boot"): the menu
  does not know each program's real minimum.
- **CPUs**: `auto` and 1 to the host's cores (16 at most), with the line
  "this host has 8 logical cores; the kernel starts every one, up to 16"
  (or "at most 16 of them" on a bigger host). Enter types a number.
- Both are saved in `target/poly-start.json` (`memory`, `cpus`). A file
  without them, or with wrong values, gives `auto` for those two and keeps the
  rest (ADR 0133's field by field rule). A value saved on another host that
  this one cannot give is shown with a warning and clamped when Start runs.

### Not offered

- **A bigger root or Chromium disk.** The disk builders size the image to its
  contents plus fixed room (`rootdisk::image_size`) and write it whole from
  memory; a larger image means a rebuild of the disk (minutes for Chromium)
  that the menu would have to trigger and a flag nobody could take effect
  without `--fresh-disk`. Not cheap, so not invented here.
- The display size and scale (already in the menu) and the accelerator
  (already there) are kept as they are.

## Consequences

- The owner can boot the desktop with `--memory=8G --cpus=8` from the menu
  or the command line and see the kernel use it (boot log below), without
  editing source.
- The numbers a boot test uses cannot be changed from the menu, so a run of
  the tests is the same everywhere.
- The kernel's CPU limit (16) now shows in the menu. Raising `MAX_CPUS`
  needs `KERNEL_MAX_CPUS` raised with it (the test says so).
- Logic is host-tested: `resources.rs` (parsing, printing, clamping, the host's
  limits, `meminfo`) and `start.rs` (saving and loading with older files, the
  flags built per entry, stepping, typing and greying).
