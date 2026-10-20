# Hand-off log

Append-only. One line per hand-off from a spec agent to an implementer agent.

| Date (UTC) | Entry id | Spec revision (git) | Spec agent | Implementer agent | Files handed over |
|---|---|---|---|---|---|

## Deviations

Recorded so the separation can be judged later. Each entry says what
happened, why, and what limits the risk.

- **2026-10-20, `linux/syscall/x86_64/*` (27 syscalls, commit 794ca74, merged in 39b5bdf).**
  The implementer agent was instructed by the coordinator to observe the
  real Linux host with `strace` while implementing, so one agent acted as
  both observer and implementer. It read no Linux kernel or libc source:
  inputs were man-pages section 2, the installed UAPI headers (constants and
  struct layouts) and strace output of static test programs. Every point
  where the documentation left the behaviour open (error order in mmap,
  unknown prot bits, MAP_HUGETLB, mprotect with size 0, the arch_prctl EPERM
  boundary) is marked in the handler for confirmation by the diff runner.
  From now on Linux work follows the normal split as well: observation by
  the spec agent, implementation from the spec only.
