# Failing on Poly (Linux)

What the diff runner (`cargo xtask pipeline diff --target poly`) finds,
one section per spec agent, so an implementer can be assigned. Each line
gives the first divergence; fix it and run the tests again, since what
follows a divergence often changes with it. Tests that pass go into
`tests/linux/poly-pass.txt` (corpus tests) and leave this file.

## spec-c (apps)

App tests (`tests/linux/apps/*.app`, the visible cases in
`specs/linux/apps/<app>.tests/` and the held-out cases, which are counted
here but not named), compared in app mode (docs/AGENT_PIPELINE.md, "App tests"). Run on 2026-10-20: reference
Linux 6.18.44 (Ubuntu 24.04, coreutils 9.4, grep 3.11, sed 4.9, glibc
2.39), Poly at main 8351e77 (106 tests: 68 pass, the 37 app cases below
and `libc-hello` fail). Spec sheets: `specs/linux/apps/<app>.yaml`.

Passing: **true** (3 of 3 cases), **echo** (7 of 7), **head** (7 of 7),
**grep** (10 of 10), **mkdir** (7 of 7) and **sed** (8 of 9; `sed-i` fails
in its `cat` step, below); the corpus tests `app-true`, `app-echo`, `app-head`, `app-grep`, `app-sed` and
`app-mkdir` are in `tests/linux/poly-pass.txt`. `true`, `echo` and `head`
also match in exact mode (with only `rseq` left out).

Failing, by what Poly needs (first divergence, then what was seen behind
it with that call left out):

- **`fadvise64` is `ENOSYS`** (Linux: 0). First divergence of `app-cat`,
  `app-wc`, `app-sort`, `app-cp`, every failing `cat`, `wc`, `sort` and
  `cp` case but `cp-r` and `cp-p`, and `sed-i` (its `cat` step):
  `reference fadvise64(3, 0, 0, 2) = ok, target fadvise64(3, 0, 0, 2) = -1 ENOSYS`
  (advice 2 is `POSIX_FADV_SEQUENTIAL`, on a regular file and on stdin, a
  regular file here). With it left out (`--ignore-syscall fadvise64`),
  **wc** and **sort** pass completely (`app-wc`, `app-sort` and all 16
  cases): `fadvise64` returning 0 is all they need.
- **`copy_file_range` is `ENOSYS`** (Linux: the byte count, then 0 at end
  of file). Behind `fadvise64` in `app-cat`, `app-cp`, `cat-file`,
  `cat-dash`, `cat-stdin`, `cp-file`, `cp-overwrite`, `sed-i` and two
  held-out cases:
  `reference copy_file_range(3, NULL, 1, NULL, 9223372035781033984, 0x0) = ok, target ... = -1 ENOSYS`.
  cat copies into stdout (a regular file) this way, cp into the target.
  Note for whoever implements it: the bytes it writes to fd 1 or 2 must be
  reported in `POLY-OUT` records like those of `write` (ADR 0027 says
  "bytes the program wrote to fd 1 or 2"; the kernel records only `write`
  today, `kernel/src/linux/services.rs`), or cat's output goes missing
  from the Poly side once the call works.
- **`ioctl(fd, FIONREAD)` on a regular file is `ENOTTY`** (Linux: 0, the
  bytes left to read). Behind `fadvise64` in `cat-A`, `cat-n`, `cat-s`:
  `reference ioctl(3, 0x541b, <ptr>) = ok, target ioctl(3, 0x541b, <ptr>) = -1 ENOTTY`.
- **`ioctl(fd, FICLONE)` on tmpfs is `ENOTTY`** (Linux: `EOPNOTSUPP`). First
  divergence of `cp-r`:
  `reference [p1] ioctl(4, 0x40049409, <ptr>) = -1 EOPNOTSUPP, target [p1] ioctl(4, 0x40049409, <ptr>) = -1 ENOTTY`;
  every cp case makes this call before copying.
- **tmpfs directory sizes**: Poly reports `st_size` 4096 for a directory of
  the initramfs; Linux's tmpfs reports 40 plus 20 per entry (60 for one
  entry, 140 for five, 200 for eight), `st_blocks` 0. First divergence of
  `app-ls`, `ls-la`, `ls-ld`, `ls-lS` and one held-out case:
  `stdout line 2: reference "drwxr-xr-x 3 root root 140 Nov 14  2023 .\n", target "drwxr-xr-x 3 root root 4096 Nov 14  2023 .\n"`.
- **extended attribute calls are `ENOSYS`**: `lgetxattr` and `listxattr`
  (in `app-ls`, `ls-la`, `ls-ld`, `ls-lS`, `cp-p`), `flistxattr`
  (`cp-p`), `fgetxattr` and `fsetxattr` (`sed-i`). Linux's tmpfs answers
  `lgetxattr(path, "security.selinux")` and `fgetxattr(fd,
  "system.posix_acl_access")` with `ENODATA`, `listxattr`/`flistxattr` with
  0 and `fsetxattr(fd, "system.posix_acl_access", 28 bytes)` with 0. With
  `ENOSYS`, ls prints `ls: <name>: Function not implemented` on stderr for
  every file it lists in long format; first divergence of `cp-p`:
  `stderr line 1: reference <end of output>, target "ls: copy.sh: Function not implemented\n"`.

Not caused by Poly, handled by the tooling (docs/AGENT_PIPELINE.md, "App
tests"): `rseq` (`ENOSYS` by decision, ADR 0043/0049) is left out of app
mode; the reference hides SSE4.2 from glibc (`GLIBC_TUNABLES`) so ld.so
searches libraries as on QEMU's CPU, gets an empty `/sys` and a
`/proc/filesystems` without `selinuxfs`, as a kernel without SELinux.

## spec-d (getdents64 order on tmpfs)

The tmpfs directory order of `linux/syscall/x86_64/getdents64`
(`specs/linux/x86_64/getdents64.yaml`, section `directory_order`), handed
to impl-3. Run on 2026-10-21: `cargo xtask pipeline diff --target poly
--build-kernel --tests-dir specs/linux/x86_64/getdents64.tests --tests-dir
<held-out dir>`, reference Linux 6.18.44, Poly at 8460f5f (main merged).
12 tests: the two older visible ones (`a_getdents64_list`,
`a_getdents64_small`, which check names as a set) pass; the six new
visible ones and all 4 held-out ones fail.

Poly lists a tmpfs directory (here `/dev/shm`) in creation order, oldest
first, with d_off values going up along the listing; Linux lists newest
first (renamed entries at the front) with each record's d_off the next
entry's cookie. First divergence of each visible test (the stdout line;
the syscall divergence is the `write` of that line's check result):

- `d_getdents64_tmpfs_creation_order`: `stdout line 5: reference
  "created: . .. dd yy bb zz aa xx cc mm\n", target "created: . .. mm cc
  xx aa zz bb yy dd\n"`.
- `d_getdents64_tmpfs_multi_call`: `stdout line 5: reference "one_call: .
  .. dd yy bb zz aa xx cc mm\n", target "one_call: . .. mm cc xx aa zz bb
  yy dd\n"` (the record-per-call counts and the EINVAL for a buffer too
  small for the next record pass where the next record is the same).
- `d_getdents64_tmpfs_rename`: `stdout line 6: reference "created: . .. e
  d c b a\n", target "created: . .. a b c d e\n"`.
- `d_getdents64_tmpfs_resume`: `stdout line 5: reference "first: . .. h\n",
  target "first: . .. a\n"`.
- `d_getdents64_tmpfs_forty`: `stdout line 5: reference "all40: . .. f39
  f38 ... f00\n", target "all40: . .. f00 f01 ... f39\n"`.
- `d_getdents64_tmpfs_lseek` (syscalls compared by name): `stdout line 5:
  reference "listing: . .. f e d c b a\n", target "listing: . .. a b c d e
  f\n"`. Seeking to a returned d_off already reads the next record on
  Poly (`lseek_d_off_reads_next`, `position_is_last_d_off` pass); the
  other failed checks follow from the order: a new entry goes last on
  Poly, so ".." keeps its d_off (`d_off_kept_after_create`) and the old
  last record's d_off now leads to it (`end_position_0`: 24, one record,
  where on Linux it is the end, 2147483647, and stays 0).
