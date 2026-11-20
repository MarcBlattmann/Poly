# 0255: Boots that run programs see a real CPU

**Status:** Accepted. Track `w3-dotnet-host` (batch W3 of [the Windows apps survey](../windows/apps-survey.md)).
Refines [ADR 0077](0077-ci-in-parallel-jobs-and-qemu-on-kvm.md) (QEMU on KVM) and the CPU-model note of
[ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md).

## Context

Every boot used QEMU's default CPU model (`qemu64`) unless it named another with `-cpu`, or `POLY_QEMU_CPU` was set.
`qemu64` is x86-64 baseline: no SSE4.2, no AVX. The survey of popular Windows programs found Paint.NET refusing to start
("A processor (CPU) with SSE4.2 support is required"). Poly's `IsProcessorFeaturePresent` was right: it reads CPUID,
and the guest's CPU had no SSE4.2. The .NET runtime, many games and code built for x86-64-v2 or v3 make the same check
or simply use the instructions. A real PC that runs Poly has these features. The owner's coordinator decided that the
desktop and release images boot with the host's CPU whenever an accelerator is used, and with QEMU's `max` under TCG,
and that the test tools do the same.

The survey also found that `cargo xtask win-app run` could not run a program an installer had just installed:
`--then` only reran the same executable of the program's directory.

## Decision

1. **The boots that run programs see a real CPU.** Every boot started through `qemu_with` (`desktop`, `run`,
   `chromium run`, `toolkits`, `chrome-win`, `win-app`) passes `-cpu host` on a hypervisor (KVM, WHPX; HVF on a Mac)
   and `-cpu max` under TCG (`accel::program_cpu`). QEMU 11.1 starts `-cpu host` under WHPX.
2. **The kernel boot tests keep QEMU's default model** (`qemu`), so they see the same CPU on every host and in CI.
   The self-test boots that need another model name it, as before (`selftest-max`).
3. **`POLY_QEMU_CPU` still overrides both.** `POLY_QEMU_CPU=qemu64` gives an interactive boot the boot tests' model,
   and `POLY_QEMU_CPU=host,+invtsc` adds the invariant TSC, which plain `host` leaves out under KVM.
4. **`win-app run --then-exe "PATH|ARGS"`** (repeatable, in order with `--then`) runs a program by its Windows path
   after the previous run, in the same boot and on the same disk. The launcher's stage file gets an optional line 8,
   `installed`: the launcher expands `%NAME%` in the program, command line and directory
   (`ExpandEnvironmentStringsW`), and waits within the run's time until the program's file exists
   (`GetFileAttributesW`), since an installer may finish its work in a process it started. On the desktop, each later
   run gets its own desktop file (`Winapp <name> runN`), and the step `launch:N` starts it. Its path must be a `C:\`
   path without `%NAME%`, quoted in `Exec`.

## Consequences

- Programs that need SSE4.2 or AVX start on Poly's desktop and in `win-app` without any setting.
- Interactive boots differ between hosts with different CPUs. Under TCG, `max` emulates more instructions and is
  somewhat slower. For a run that must not depend on the host's CPU, set `POLY_QEMU_CPU=qemu64`.
- The `chrome-win` boot tests now run on the host's CPU. Plain `host` has no invariant TSC under KVM, so Chrome's
  clock choice (ADR 0155) stays as it was.
- An installer survey can install and then run the installed program in one boot.
