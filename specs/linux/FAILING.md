# Failing on Poly (Linux)

What the diff runner (`cargo xtask pipeline diff --target poly`) finds,
one section per spec agent, so an implementer can be assigned. Each line
gives the first divergence; fix it and run the tests again, since what
follows a divergence often changes with it. Tests that pass go into
`tests/linux/poly-pass.txt` (corpus tests) and leave this file.

## spec-c (apps)

App tests (`tests/linux/apps/*.app` and the visible cases in
`specs/linux/apps/<app>.tests/`), compared in app mode
(docs/AGENT_PIPELINE.md, "App tests"). Only visible divergences are
listed here. Run on 2026-10-21: reference Linux 6.18.44 (Ubuntu 24.04,
coreutils 9.4, grep 3.11, sed 4.9, glibc 2.39), Poly at main c200c88
under QEMU (TCG), `--tests-dir` for each of the eleven
`specs/linux/apps/*.tests` (82 tests with the corpus: 75 pass; `libc-hello`
fails at `rseq` by decision, ADR 0043/0049; `app-ls` and the five cases
below fail). Spec sheets: `specs/linux/apps/<app>.yaml`.

Passing: **true** (2 of 2 cases), **echo** (5 of 5), **head** (5 of 5),
**grep** (7 of 7), **mkdir** (5 of 5), **cat** (6 of 6), **wc** (6 of 6),
**sort** (6 of 6), **cp** (4 of 5; `cp-p` below), **sed** (5 of 6;
`sed-i` below) and **ls** (3 of 6; `ls-la`, `ls-lS`, `ls-ld` below). The
corpus tests `app-true`, `app-echo`, `app-head`, `app-grep`, `app-sed`,
`app-mkdir`, `app-cat`, `app-sort`, `app-wc` and `app-cp` pass and are in
`tests/linux/poly-pass.txt`; `app-ls` fails (below).

Failing, by what Poly needs (first divergence, then what was seen behind
it with that call left out):

- **Directory entry order.** Linux's tmpfs returns a directory's entries
  newest first: for `ls-la`, `dangling`, `link`, `sub`, `.hidden`,
  `big.txt`, `run.sh`, `empty`, `notes.txt`, the reverse of the order the
  test defines them in. Poly returns them in another order, by all signs
  the opposite one: its `readlink` calls for the two symbolic links come
  after the last two `statx` calls (Poly's records carry no
  `getdents64` contents, so this is inferred from the call order). `ls -l`
  sorts what it prints, so stdout, stderr and the exit status match; the
  per-file calls come in another order. First divergence of `app-ls`,
  `ls-la` and `ls-lS`:
  `reference readlink("tree/dangling", <ptr>, 15) = ok, target listxattr(*, <ptr>, 152) = ok`
  (`app-ls`: `readlink("tree/link", <ptr>, 10)`).
- **Times set with `futimens` do not stick.** `cp -p` copies with
  `copy_file_range`, then sets the copy's times with
  `utimensat(4, NULL, <ptr>, 0)` and its mode with `fchmod`; Poly answers
  every call as Linux does (the whole `cp` process matches), but the `ls -l`
  step then shows the time of the copy instead of the source's
  modification time (2023-11-14 22:13:20 UTC). First divergence of `cp-p`:
  `stdout line 1: reference "-rwxr-x--- 1 root root 10 Nov 14  2023 copy.sh\n", target "-rwxr-x--- 1 root root 10 Oct 21 07:44 copy.sh\n"`.
- **`clock_gettime` is a system call on Poly under TCG.** Linux answers
  it in the vDSO (no syscall in the trace); Poly's vDSO makes the system
  call when the kernel's clock is not the TSC, which is the case under
  QEMU's TCG (ADR 0044, `Mode::Syscall`), so the call shows up in Poly's
  trace only. `ls -l` reads the time once to decide between year and time
  of day, and sed's temporary file name uses `CLOCK_MONOTONIC`. First
  divergence of `ls-ld`:
  `reference write(1, "drwxr-xr-x 2 root root 60 Nov 14  2023 tree/..., 48) = ok, target clock_gettime(0, <ptr>) = ok`
  and of `sed-i`:
  `reference [p1] openat(-100, "./sed0hx2pL", 0xc2, 0x180) = 4, target [p1] clock_gettime(1, <ptr>) = ok`.
  With it left out (`--ignore-syscall clock_gettime`) both pass
  completely, and `cp-p` and the `ls` cases above still fail as described.
  This is a property of the test machine rather than of a call Poly
  answers wrongly; it can be fixed in the kernel (the vDSO's clock under
  TCG) or by leaving the vDSO's calls out of app mode as `rseq` is.

Not caused by Poly, handled by the tooling (docs/AGENT_PIPELINE.md, "App
tests"): `rseq` (`ENOSYS` by decision, ADR 0043/0049) is left out of app
mode; the reference hides SSE4.2 from glibc (`GLIBC_TUNABLES`) so ld.so
searches libraries as on QEMU's CPU, gets an empty `/sys` and a
`/proc/filesystems` without `selinuxfs`, as a kernel without SELinux.
