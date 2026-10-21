# 0077: CI in parallel jobs, QEMU on KVM where it can, the self-test boots on TCG too

**Status:** Accepted. Changes how CI runs, not what it checks; amends the CI
part of [0056](0056-pipeline-scheduler-loop-and-home-server-deployment.md)
(which ran every boot under TCG).

## Context

`cargo xtask ci` ran as one GitHub Actions job of about 15 minutes, every step
after the other: formatting and clippy, the host tests, about fifty QEMU boots,
the coverage checks and the pipeline's `selfcheck` and `polycheck` (another
~260 boots). Every boot ran on QEMU's emulator (TCG). The repository is public
now, so GitHub's hosted runners cost nothing and up to 20 jobs run at once, and
their standard Linux runners expose `/dev/kvm`. Branch protection requires a
status check named `ci`.

A cold run on a 4-core machine (the one the change was written on, TCG only)
spends its time as follows: compiling ~9.5 minutes (clippy 1.4, host tests'
build 2, kernel 3.3, the rest in between), running the host tests 2.7 (the
ext4 and journal tests of `poly-ext2` alone 1.4), the boot tests 2.9, polycheck
1.3 (after another kernel build of 0.8: the pipeline built the kernel without
`POLY_NTDLL_DLL`, so the next `cargo xtask` build rebuilt it again).

## Decision

**QEMU runs on KVM when `/dev/kvm` can be opened, on TCG otherwise.** xtask
(`tools/xtask/src/accel.rs`) and the pipeline (`poly::accel`) pass `-accel kvm`
or `-accel tcg`. `POLY_QEMU_ACCEL=tcg` forces the emulator, `kvm` requires KVM
(an error without it), `auto` or unset chooses; a bad value stops xtask before
anything runs. The CPU model does not change with the accelerator: QEMU's
default (`qemu64`) unless a boot asks for one (`-cpu max` in one self-test
boot), so the guest sees the same features either way. What differs under KVM
is real hardware behaviour: the TSC is the host's (the kernel already treats a
hypervisor's TSC as constant-rate, ADR 0072), QEMU adds x2APIC (KVM's in-kernel
APIC), `-cpu max` is the host CPU's features (PCID among them), and
non-canonical addresses fault with #GP (the user-mode tests accept both).

**The boot tests are named groups.** `BOOT_GROUPS` in xtask lists them in the
order `cargo xtask test` runs them (`selftest-default`, `selftest-2g`,
`selftest-polled`, `selftest-max`, `smp`, `keyboard`, `shell`, `programs`,
`dynamic`, `busybox`, `rootdisk`, `net`, `ext2`, `ext4`, `missing-devices`,
`screen`, `terminal`, `faults`, `failures`). `cargo xtask boot-tests [group...]`
runs some (`selftest` names every `selftest-*`), `--list` lists them, and
`--shard <name>` runs a CI shard's.

**CI is jobs that run in parallel**, each a part of `cargo xtask ci`:
`lint` (`cargo xtask ci lint`: fmt, clippy, font tables, `global_asm!` labels,
conflict markers, the workflow check below), `host tests`, one `boot` job per
shard (`CI_BOOT_SHARDS`: `selftest`, `userland`, `devices`, `faults`, about a
minute of boots each under TCG, since every job builds the kernel first),
`coverage` (the map, the README, the loop's dry run) and `pipeline` (`selfcheck`,
`polycheck`). `cargo xtask ci` with no argument still runs every part, in the
old order; `cargo xtask ci <part>` runs one, as its job does.

**One job keeps TCG.** The matrix runs the `selftest` shard a second time with
`POLY_QEMU_ACCEL=tcg`, so what only the emulator does (no x2APIC, its own
canonical-address and SYSRET behaviour, its timing) stays tested. Locally,
`cargo xtask ci` on a machine with KVM runs that shard on TCG as well, so it
still runs everything CI does.

**The required check is an aggregate.** The last job, `ci`, needs every other
job, runs `if: always()` and fails unless all of them succeeded (a skipped
required check would count as passing). Its name is the check branch
protection requires, so the ruleset is unchanged.

**Nothing can be left out silently.** A unit test puts every boot group in
exactly one shard, and `cargo xtask ci lint` (and a unit test) checks that the
workflow runs `cargo xtask ci <part>` for every part but the boot tests, lists
exactly the shards of `CI_BOOT_SHARDS` in its matrix (`shard: [...]`), and runs
the TCG shard.

**Caches.** Each job kind has its own `Swatinem/rust-cache` key, saved only on
`main` (pull requests restore it); it holds third-party crates, not ours, which
are most of the build. The lint job saves `target/ntdll-cache` under the key
`cargo xtask ntdll --cache-key` prints (the hash xtask names the DLL by), and
the jobs that build the kernel restore it. A newer push to a pull request
cancels the older run. xtask's `pipeline` commands that build the kernel pass
`POLY_NTDLL_DLL` as `kernel_cargo` does, so polycheck's kernel is the boot
tests' (and embeds our ntdll).

## Consequences

- The wall-clock time of CI is that of its slowest job, about setup plus a
  kernel or test build plus a minute or two of tests, instead of the sum of
  everything; the runner minutes used go up (every job compiles what it
  needs), which costs nothing on a public repository but uses up to 10 of the
  20 concurrent jobs.
- The KVM path was written without a machine that has KVM; the first GitHub
  runs are its first runs. Tests that assumed emulation speed (none are known)
  would show up there, with the boot logs kept as artifacts of a failed job.
- Workspace crates are compiled in every job. Caching them (sccache, or
  restoring source mtimes so cargo's fingerprints hold) would cut most of the
  remaining time and is the next step if CI needs to be faster still.
- A runner without `/dev/kvm` still passes, on TCG, with a warning in the run's
  annotations.
