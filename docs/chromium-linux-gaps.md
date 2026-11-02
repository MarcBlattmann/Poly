# Chromium (the Linux build) on Poly: what it needs

Roadmap Phase 6: "Chromium (the Linux build) runs on the desktop with its
sandbox enabled: opens pages, renders, takes input". This is the spec
side: what Chromium does on real Linux, call by call, compared with the
coverage map and with what Poly's Linux personality has today, grouped
into batches for the implementer (`impl-linux-chromium`). Written by the
spec agent `spec-linux-chromium` from observation only (CLEAN_ROOM.md):
strace of unchanged Debian binaries, `WAYLAND_DEBUG`, screenshots; no
Chromium or Linux source was read.

- **Chromium**: Debian 13's `chromium` 156.0.8078.73-1~deb13u1
  (trixie-security) and exactly the Debian packages it depends on, pinned
  in `tools/xtask/chromium.lock`; the disk and the reference are made by
  `tools/xtask/src/chromium.rs` (module documentation).
- **Scenario tests**: `specs/linux/apps/chromium.yaml` and
  `specs/linux/apps/chromium.tests/` (8 visible, 4 held out), run by
  `cargo xtask chromium apptest`; the windowed scenario is boot group
  `chromium`.
- **Reference**: Linux 6.18.54.1 (WSL2), privileged poly-dev container,
  the Debian root entered with `pivot_root` (user-namespace sandbox) or
  `chroot` (set-user-ID sandbox), Chromium as uid 1000, traced with
  `strace -f -ff -yy` (one file per thread). Runs traced: headless
  `--dump-dom` with the user-namespace sandbox, with the set-user-ID
  sandbox, with `--no-sandbox`; `--screenshot`; through the start script;
  windowed on weston (headless back end, Wayland) and on Xvfb (X11),
  clicked and typed into with xdotool.

## The short version

- **Every one of the 130 system calls Chromium makes is green** in
  `coverage/linux/syscall/x86_64.json`. What is missing are *cases* of
  green calls (prctl options, socket options, flag combinations), **files
  in `/proc` and `/sys`**, **ioctls** that must fail the Linux way, a few
  **behaviours across calls** (pid namespaces in `SCM_CREDENTIALS`,
  chroot into a dead process's `/proc` directory, 1.4 TiB address-space
  reservations), the **compositor protocols** Chromium binds, and the
  **disk/desktop integration** (`chromium.rs` has it).
- **Which sandbox Poly gets depends on how the disk is entered.** BusyBox's
  init *chroots* into the root disk (ADR 0059). A chrooted process may not
  make a user namespace (`clone(CLONE_NEWUSER)` is `EPERM`, as Poly
  already implements: `personalities/linux/src/ns/mod.rs`, `chrooted`), so
  Chromium on Poly's disk falls back to its **set-user-ID sandbox**
  (`/usr/lib/chromium/chrome-sandbox`, mode 4755). Entered with
  `pivot_root`/`switch_root` instead, it would use its **user-namespace
  sandbox**. Both are listed (batches C and D); the scenario tests pass
  either way (stdout and status are the same), and
  `chromium-setuid-sandbox` forces the set-user-ID one everywhere
  (`--disable-namespace-sandbox`). Recommendation for the owner: keep the
  chroot (fewer moving parts: batch D only), or switch the disk's init to
  `switch_root` and implement batch C; either is a full sandbox.
- **Processes of one headless run** (about 15 processes, 250 threads):
  the browser; two `chrome_crashpad_handler`s (exec'd); the unsandboxed
  zygote (`--type=zygote --no-zygote-sandbox`) and the sandboxed zygote
  (exec'd, through `chrome-sandbox` in the set-user-ID case), from which
  the renderer and the GPU process are **forked, not exec'd**; the network
  service (`--type=utility`, exec'd as `/proc/self/exe`). Windowed adds
  nothing new at the process level.

## First run on Poly (2026-11-02, main at d2078f2d, TCG)

`cargo xtask chromium "run:..."` on the disk, as root, `poly.strace=chromium`:

- `/usr/bin/chromium --version` prints `Chromium 156.0.8078.73 built on
  Debian GNU/Linux 13 (trixie)` and exits 0, as on Linux: the start
  script, `/proc/cpuinfo`, `findmnt` and the dynamic linking of the
  330 MB binary work.
- `chromium --no-sandbox --headless=new --dump-dom file:///...hello.html`
  exits 134: both forked zygotes die of `SIGTRAP` ("killed by signal 5
  after breakpoint", Chromium's deliberate crash on a failed check) right
  after `access("/proc/N/oom_score_adj")` and `access("/proc/N/oom_adj")`
  (both `ENOENT`), `openat("/proc/", O_DIRECTORY)` and
  `newfstatat(proc_fd, "self/task/")`; the browser then logs "Did not
  receive ping from zygote child", "GPU process isn't usable. Goodbye."
  and aborts; crashpad's handler calls `ptrace` (not implemented: the
  entry `ptrace` is missing in the map) to dump the crashed children.
- **The first blocker (A0)**: the link count of `/proc/<pid>/task`. On
  Linux `stat("/proc/self/task")` gives `st_nlink` 3 for a process with one
  thread (2 plus one per thread); Chromium decides from it whether a
  process is single-threaded before it forks and sandboxes. Poly's procfs
  gives every directory `st_nlink` 2 (`poly-procfs/src/procfs.rs`), which
  reads as "no threads".

## Status legend

| Mark | Meaning |
|---|---|
| **missing** | Poly does not have it (read in Poly's code): an implementer item |
| **verify** | Poly has the call, but nothing tests this case: a visible test is needed before it can be trusted |
| **ok** | Poly has it and a spec test covers it |
| **case** | a new case of a green entry (spec sheet extension and hand-off) |
| **map+** | not in the coverage map (a `/proc` or `/sys` file, an ioctl, a socket option, a compositor protocol): a roadmap or map addition |

## Batch A: the start script and headless Chromium without a sandbox

Scenario tests: `chromium-version`, `chromium-dumpdom-root`. Order
inside the batch: top to bottom.

| # | What | How Chromium uses it (reference) | Poly | Entry |
|---|---|---|---|---|
| A0 | `st_nlink` of `/proc/<pid>/task` = 2 + the number of threads (3 when single-threaded) | `newfstatat(proc_fd, "self/task/")` before forking zygote children and before sandboxing: the single-thread check; a wrong count is a `SIGTRAP` crash (seen on Poly) | **missing** (procfs directories all have 2) | map+ (`/proc/<pid>/task`) |
| A0b | `ptrace` (attach to a crashed child) | crashpad's handler, only after a crash | **missing** (`ENOSYS`; map entry `ptrace` missing) | ptrace (missing entry; not needed while nothing crashes) |
| A1 | `/proc/cpuinfo` flags | the start script (`/usr/bin/chromium`, sh) greps `sse3\|pni`, else exits 1 | ok (`poly-procfs/cpuflags.rs` has `sse3`) | ok |
| A2 | `findmnt -bnr -o avail -T /dev/shm` | `/etc/chromium.d/dev-shm`: reads `/proc/self/mountinfo`, `statfs("/dev/shm")` (`TMPFS_MAGIC`); below 3.8 GB adds `--disable-dev-shm-usage` (shared memory then goes to `/tmp`) | verify: `/dev/shm` must be a tmpfs mount with its own line in mountinfo and a sane `f_bavail` | statfs (case) |
| A3 | `execve("/proc/self/exe", ...)` | the network service is started by exec'ing `/proc/self/exe` | verify | execve (case) |
| A4 | huge `PROT_NONE` reservations | `mmap(NULL, 1481763713024, PROT_NONE, MAP_PRIVATE\|MAP_ANONYMOUS\|MAP_NORESERVE)` (1.35 TiB, V8's sandbox) ×6, `34359738368` (32 GiB) ×12, 16 GiB ×6, 4 GiB ×6, 1 GiB ×6, per process; later `mprotect` of small parts to `PROT_READ\|PROT_WRITE` (5900×) and `MAP_FIXED` remaps inside | verify: must succeed without committing memory or page tables (47-bit user space) | mmap, mprotect (case) |
| A5 | `memfd_create("shared-memory-region", MFD_CLOEXEC\|MFD_ALLOW_SEALING\|MFD_NOEXEC_SEAL)` | ~930 per run, then `ftruncate`, `fcntl(F_ADD_SEALS, F_SEAL_SEAL\|F_SEAL_SHRINK\|F_SEAL_GROW)`, `mmap(PROT_READ\|PROT_WRITE, MAP_SHARED)`, passed with `SCM_RIGHTS` | ok (memfd green, `NOEXEC_SEAL` known) | ok |
| A6 | `memfd_create("", MFD_CLOEXEC\|MFD_ALLOW_SEALING\|MFD_HUGETLB\|MFD_NOEXEC_SEAL\|MFD_EXEC\|0x3ffffe0\|63<<MFD_HUGE_SHIFT)` | a probe, must be `EINVAL` | verify (Poly rejects unknown bits) | memfd_create (case) |
| A7 | `eventfd2(0, EFD_SEMAPHORE\|EFD_CLOEXEC\|EFD_NONBLOCK\|0xfff7f7fe)` | a probe, must be `EINVAL` (then `EFD_NONBLOCK` ones) | verify | eventfd2 (case) |
| A8 | `prctl(PR_SET_VMA, PR_SET_VMA_ANON_NAME, addr, len, name)` | ~190×, names allocator regions (`/proc/self/maps` shows `[anon:...]`) | **missing** (unknown option: `EINVAL`) | prctl (**case added**: `lc_prctl_vma_name.c`) |
| A9 | `prctl(PR_SET_PTRACER, pid)` | lets the crash handler trace the browser; 0 with Yama, `EINVAL` without (Chromium accepts that) | `EINVAL` (as Linux without Yama) | prctl (noted in the sheet) |
| A10 | `prctl(PR_GET_SPECULATION_CTRL)` / `PR_SET_SPECULATION_CTRL` | per sandboxed process; Linux here returns `PR_SPEC_PRCTL\|PR_SPEC_ENABLE` (3), set 0 | **missing** (`EINVAL`); CPU-dependent, so not a diff test | prctl (noted) |
| A11 | `prctl(PR_CAPBSET_READ, 41/42/44/48)` | probes above `CAP_LAST_CAP`: `EINVAL` | ok | ok |
| A12 | `/proc/PID/oom_score_adj` | `access(F_OK)`, then **`creat("/proc/PID/oom_score_adj")`** and write of "200"/"300" for every child (score per process type) | **missing** | map+ (`/proc/<pid>/oom_score_adj`, writable, `O_CREAT\|O_TRUNC` on an existing proc file must open it) |
| A13 | `/proc/PID/clear_refs` | opened and written "5" (resets the peak RSS) for children, ~100× | **missing** | map+ |
| A14 | `/proc/PID/task/TID/status` | ~1000 opens per run (the browser reads its children's threads) | **missing** (task dirs hold only `comm`, ADR 0120) | map+ |
| A15 | `/proc/PID/stat`, `statm`, `status`, `/proc/stat`, `/proc/meminfo`, `/proc/self/maps`, `/proc/self/mountinfo`, `/proc/mounts`, `/proc/filesystems`, `/proc/self/fd`, `/proc/self/task` | read throughout | ok | ok |
| A16 | `/proc/sys/kernel/yama/ptrace_scope` | `stat` and read ("1") | **missing** (ENOENT is what a kernel without Yama gives: acceptable) | map+ (optional) |
| A17 | `/proc/sys/kernel/cap_last_cap` | read ("40"), `fstatfs` (`PROC_SUPER_MAGIC`) | **missing** | map+ |
| A18 | `/proc/sys/fs/inotify/max_user_watches` | read | **missing** | map+ |
| A19 | `/sys/devices/system/cpu/{possible,present,online,kernel_max}` | read; `cpuN/cpufreq/cpuinfo_max_freq`, `tsc_freq_khz`, `/sys/devices/virtual/dmi/id/*`, `/sys/bus/cpu/devices/cpu0/microcode/version` `ENOENT` here too | `kernel_max` **missing**; the rest ok | map+ (`kernel_max`) |
| A20 | `inotify_init`, `inotify_add_watch` on `/etc`, `/etc/chromium`, `/etc/hosts`, `/etc/resolv.conf`, `/etc/localtime` (ENOENT for missing ones), `ioctl(FIONREAD)` on it | config watching | ok | ok |
| A21 | `socket(AF_NETLINK, SOCK_RAW\|SOCK_CLOEXEC, NETLINK_ROUTE)`, `bind`, `sendto`/`recvmsg` dumps | network change notifier | ok (netlink route green) | ok |
| A22 | `socket(AF_NETLINK, SOCK_RAW\|SOCK_CLOEXEC\|SOCK_NONBLOCK, NETLINK_KOBJECT_UEVENT)` + `setsockopt(SO_ATTACH_FILTER, {len=8 or 11})` | device monitor (udev) | **missing** (protocol and socket filter) | map+ (netlink protocol), setsockopt (case) |
| A23 | UDP probes: `socket(AF_INET, SOCK_DGRAM)`, `connect` to 127.0.0.1:53 (ok) and `[2001:4860:4860::8888]:443` (`ENETUNREACH` without IPv6 route), `getsockname`; `setsockopt(SOL_IP, IP_RECVERR, 1)` on UDP sockets (40×) | network state, DNS | `IP_RECVERR` **missing**; the rest verify | setsockopt (case) |
| A24 | `ioctl(fd, SIOCGIFNAME / SIOCETHTOOL / SIOCGIWNAME / SIOCGIWESSID)` on UDP sockets | interface type probes (`ENODEV`/`EOPNOTSUPP` here: no Wi-Fi) | **missing** (must fail like Linux, not `ENOSYS`) | map+ (socket ioctls) |
| A25 | `ioctl(fd, F2FS_IOC_GET_FEATURES)` on every SQLite file of the profile | SQLite probes for F2FS atomic writes: `ENOTTY` | verify (unknown ioctl on an ext4/tmpfs file must be `ENOTTY`) | ioctl (case) |
| A26 | `ioctl(TCGETS / TIOCGWINSZ / TIOCGPGRP)` on pipes and on `/proc/PID/mountinfo` | `ENOTTY` | ok | ok |
| A27 | `madvise(MADV_DONTNEED)` (4200×), `MADV_DODUMP`, `MADV_DONTDUMP`, `MADV_WIPEONFORK`, `0xffffffff` (`EINVAL`); `mremap(MREMAP_MAYMOVE)`; `pkey_alloc(0, 0)` = `ENOSPC`; `landlock_create_ruleset(NULL, 0, VERSION)` (7 here) | allocators, V8 | ok (all green; the landlock ABI version Poly reports is its own) | ok |
| A28 | `prlimit64(pid, RLIMIT_NPROC, NULL, bad)` = `EFAULT` (88×), `prlimit64(0, RLIMIT_DATA, {cur, max})` set in sandboxed children | resource probes, address-space limit of renderers | verify | prlimit64 (case) |
| A29 | `setpriority(PRIO_PROCESS, tid, n)`: lowering niceness is `EACCES` for uid 1000 (48×) | thread priorities | verify (`EACCES`, not `EPERM`) | setpriority (case) |
| A30 | `sched_getaffinity`, `sched_setaffinity`, `sched_getparam`, `sched_getscheduler`, `sched_yield` (845×) | thread pools | ok | ok |
| A31 | `socketpair(AF_UNIX, SOCK_SEQPACKET)` and `SOCK_STREAM`; `sendmsg`/`recvmsg` with `SCM_RIGHTS` (hundreds), `MSG_DONTWAIT`/`MSG_NOSIGNAL`; `shutdown(SHUT_RD/SHUT_WR)`; abstract addresses (`@"69d21"`) | Mojo IPC | ok | ok |
| A32 | `setsockopt(SO_PASSCRED)`, `recvmsg` with `SCM_CREDENTIALS` | the zygote learns its children's pids | ok (see D7 for pid namespaces) | ok |
| A33 | `fallocate`, `fdatasync` (286×), `flock(LOCK_EX/LOCK_UN)`, `fcntl(F_SETLK)` (397×), `rename`, `symlink` (SingletonLock/Socket/Cookie in `/tmp/org.chromium.Chromium.XXXXXX`), `unlinkat`, `getdents64` | profile, SQLite, the process singleton | ok | ok |
| A34 | `kill(child, SIGTERM)` at exit; children end "killed by SIGTERM" or exit 0; `wait4`, `waitid(P_PID)` | shutdown | ok | ok |
| A35 | `/dev/tty` `ENXIO` (no controlling terminal), `/dev/null`, `/dev/urandom` | | verify (`/dev/tty` without a controlling terminal is `ENXIO`) | open (case) |

## Batch B: rendering (screenshot, canvas, text)

Scenario test: `chromium-screenshot`.

| # | What | How Chromium uses it | Poly | Entry |
|---|---|---|---|---|
| B1 | fontconfig: `/etc/fonts/fonts.conf`, `conf.d/*` (Debian's), `access`/`readlink` of the links, `/var/cache/fontconfig/*-le64.cache-9` (written when writable), `mmap(PROT_READ, MAP_PRIVATE)` of `DejaVuSans.ttf`, `fstatfs` of font directories | text | ok | ok |
| B2 | `/dev/dri` (`openat O_DIRECTORY`), `newfstatat` of `card0`/`renderD128`, `/sys/dev/char/226:N/device/{drm,subsystem}` links; `open("/dev/dri/renderD128", O_RDWR)` `EACCES` as uid 1000 here | GPU probing (with `--disable-gpu` nothing more) | ok with the virtio GPU (ADR 0108) | ok |
| B3 | Vulkan loader: `/etc/vulkan/{icd.d,implicit_layer.d,explicit_layer.d}`, `/etc/xdg/vulkan/*` (ENOENT), `vk_swiftshader_icd.json`; "vkCreateInstance: Found no drivers!" on stderr | WebGPU/Dawn probe in the GPU process | ok (files only) | ok |
| B4 | PNG written by `--screenshot` with `write` | | ok | ok |

## Batch C: the user-namespace sandbox (a root that is not chrooted)

Scenario tests: `chromium-dumpdom-sandbox` (on a pivot_root'ed root),
`chromium-worker` (all sandboxed). Needed only if the disk is entered
with `switch_root`.

| # | What | How Chromium uses it | Poly | Entry |
|---|---|---|---|---|
| C1 | `clone(CLONE_NEWUSER\|SIGCHLD)` probe child, then `clone(child_stack, CLONE_NEWUSER\|CLONE_NEWPID\|CLONE_NEWNET\|SIGCHLD)` for the zygote (by uid 1000, single-threaded caller) | the sandbox's namespaces | verify (namespaces in clone exist; the three together by an unprivileged caller are not tested) | clone (case) |
| C2 | in the child: `access("/proc/self/setgroups")`, write "deny"; `uid_map` and `gid_map` "1000 1000 1" | identity inside | ok (unshare/setns sheets) | ok |
| C3 | `capget` (version 0 probe, `NULL` data, returns the preferred version), `capset` to `CAP_SYS_ADMIN` only, later to none | | verify | capset/capget (case) |
| C4 | `clone(CLONE_FS\|SIGCHLD)` helper that `chroot("/proc/self/fdinfo/")` and exits; the zygote's root is then an empty directory of a dead process | drops file system access | verify: chroot needs `CAP_SYS_CHROOT` in the user namespace; lookups under a root whose `/proc` directory is gone must fail (`ENOENT`) and never crash | chroot (case), proc (behaviour) |
| C5 | each renderer is pid 1 of its own pid namespace: `clone(CLONE_NEWPID\|SIGCHLD)` from the sandboxed zygote (inside the user namespace, with its capabilities) | | verify (nested pid namespaces from inside a user namespace) | clone (case) |
| C6 | `unshare(CLONE_NEWUSER)` (2×, succeeds) | probe | ok | ok |
| C7 | `/proc/self/ns/{user,pid,net}` `access(F_OK)` | | ok | ok |

## Batch D: the set-user-ID sandbox (what Poly's chrooted disk gets)

Scenario test: `chromium-setuid-sandbox` (and, on Poly's disk, every
sandboxed case).

| # | What | How Chromium uses it | Poly | Entry |
|---|---|---|---|---|
| D1 | `clone(CLONE_NEWUSER\|SIGCHLD)` from a chrooted process is `EPERM` | Chromium then chooses the set-user-ID helper | ok (`chrooted` check) | ok |
| D2 | `execve("/usr/lib/chromium/chrome-sandbox", ...)` (mode 4755, owner root, on the ext4 disk) by uid 1000 without `PR_SET_NO_NEW_PRIVS`: `getresuid` gives `[1000], [0], [0]`, full capabilities, `AT_SECURE` | the helper | verify on the disk (cred/exec.rs has set-user-ID exec; not tested from a chroot on ext4) | execve (case) |
| D3 | in the helper: `socketpair(AF_UNIX, SOCK_STREAM)`, `clone(NULL, CLONE_NEWPID\|CLONE_NEWNET\|SIGCHLD)`, `prctl(PR_SET_DUMPABLE, 0)`, `setresgid`/`setresuid(1000, 1000, 1000)`, `shutdown(SHUT_RD)`, `sendto("C")`, `waitid(P_PID, child, WEXITED)` | | verify (NEWPID with NEWNET as root) | clone (case) |
| D4 | the child (pid 1 of the new namespace): `clone(CLONE_FS\|SIGCHLD)` helper; drops to uid 1000; `execve` of the zygote; on request the helper does `chdir`/`chroot("/proc/self/fdinfo")` and exits | | verify (as C4) | chroot (case) |
| D5 | `chrome-sandbox --adjust-oom-score PID SCORE` exec'd for every child (writes another user's `/proc/PID/oom_score_adj` as root) | | **missing** (A12) | map+ |
| D6 | the helper's `rt_sigaction` of every signal to `SIG_DFL` (inherited handlers), `fcntl(F_SETFD, 0)` on its socket | | ok | ok |
| D7 | **pids across pid namespaces**: the zygote, pid 1 in its namespace, reports pids to the browser; `SCM_CREDENTIALS` and `waitid`/`wait4` results must carry the pid as seen by the *receiver's* namespace (strace shows `si_pid=N /* M in strace's PID NS */`) | the browser must know its children's real pids (it signals and reads `/proc/PID/*` of them) | verify (no test of `SCM_CREDENTIALS` across pid namespaces) | sendmsg/recvmsg (case), wait4/waitid (case) |

## Batch E: seccomp and signals (both sandboxes)

| # | What | How Chromium uses it | Poly | Entry |
|---|---|---|---|---|
| E1 | `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, NULL)` = `EFAULT` (19×); `seccomp(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, NULL)` = `EFAULT` (11×) and with `SECCOMP_FILTER_FLAG_SPEC_ALLOW` (9×) | feature probes: `EFAULT` means "supported" | verify (must be `EFAULT`, not `EINVAL`, for a NULL program with valid flags) | seccomp, prctl (case) |
| E2 | `prctl(PR_SET_NO_NEW_PRIVS, 1)`, then `seccomp(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_SPEC_ALLOW, {len=617..662})` per process type (renderer, GPU, network, ...) | the filters | ok (seccomp green; check the 662-instruction programs pass the verifier) | seccomp (case) |
| E3 | `SECCOMP_RET_TRAP`: `SIGSYS` with `si_code=SYS_SECCOMP`, `si_errno` = the filter's data (`ENXIO`, `ENOEXEC`, `ENOENT` seen), `si_syscall`, `si_arch=AUDIT_ARCH_X86_64`, `si_call_addr`; the handler (`SA_SIGINFO\|SA_NODEFER`, installed in every sandboxed process) emulates the call (`sched_getaffinity` 61×, `sched_getparam`/`sched_getscheduler` 34×, `newfstatat`, `access`, `openat` through the broker) and returns its result through `rt_sigreturn` | | ok (seccomp/mod.rs puts the data in `si_errno`) | ok |
| E4 | `SECCOMP_RET_ERRNO`: `clone3` = `ENOSYS` (glibc falls back to `clone`), `madvise` = `EPERM` (~200×), `setsockopt` = `EPERM` | | ok | ok |
| E5 | `sigaltstack` per thread, `SA_ONSTACK` handlers for SEGV/BUS/ILL/FPE/TRAP/ABRT/XCPU/XFSZ/SYS (crash reporting), `SIGRT_1` (glibc's) | | ok | ok |
| E6 | `clone3({flags=CLONE_VM\|CLONE_VFORK\|CLONE_CLEAR_SIGHAND, exit_signal=SIGCHLD, stack, stack_size})` (posix_spawn of crashpad and chrome-sandbox) | | ok (`CLONE_CLEAR_SIGHAND` handled) | ok |

## Batch F: the Wayland window (Poly's desktop)

The windowed scenario (boot group `chromium`): Chromium started from the
launcher (`/usr/local/bin/poly-chromium`, as uid 1000, sandbox on) with
`input.html`; a click anywhere in the page turns it from `#2a6fb0` to
`#b03a2a` and the title to "clicked 1"; a second click back and "clicked
2"; typing "poly" into the text field makes the title "typed poly". On
the reference (Xvfb, xdotool, 640x480 window) exactly that was seen:
pixel (400,300) `2a6fb0` → `b03a2a` → `2a6fb0`, titles "Poly Chromium
input - Chromium", "clicked 1 - Chromium", "clicked 2 - Chromium", "typed
poly - Chromium". `Ctrl+Q` does not quit Chromium; `Ctrl+Shift+Q` does.

| # | What | How Chromium uses it | Poly | Entry |
|---|---|---|---|---|
| F1 | `connect(AF_UNIX, "$XDG_RUNTIME_DIR/wayland-N")` from uid 1000 to the compositor's socket reached through a hard link in `/run/user/1000` (made by `poly-chromium`) | | ok (poly-runas does the same for VLC, ADR 0120) | ok |
| F2 | globals bound (weston 14 offered them): `wl_compositor` v4, `wl_subcompositor`, `wp_viewporter`, `zxdg_output_manager_v1` v2, `wp_presentation`, `wp_single_pixel_buffer_manager_v1`, `zwp_relative_pointer_manager_v1`, `zwp_pointer_constraints_v1`, `wl_data_device_manager` v3, `wl_shm`, `wl_output` v4, `zwp_text_input_manager_v1`, `xdg_wm_base` v5 (and a second connection from GTK: `wl_compositor` v3, `wl_subcompositor`, `zxdg_output_manager_v1`, `wl_data_device_manager`, `wl_shm`, `wl_output` v2, `xdg_wm_base`) | | Poly's compositor has all but `zxdg_output_manager_v1`, `wp_single_pixel_buffer_manager_v1`, `zwp_relative_pointer_manager_v1`, `zwp_pointer_constraints_v1`, `zwp_text_input_manager_v1`/`v3`: **missing**, optional for Chromium (it ran on weston without a seat, so the required set is the core one) | map+ (desktop: compositor protocols) |
| F3 | requests: `wl_shm.create_pool` + `create_buffer` (software frames from the GPU process's software compositor), `wl_shm_pool.resize`, `wp_viewporter.get_viewport`, `wp_presentation.feedback`, `wl_surface.frame`/`damage`/`attach`/`commit`, `set_opaque_region`, `set_input_region`, `xdg_toplevel.set_title`/`set_app_id`/`set_min_size`/`set_max_size`, `xdg_surface.set_window_geometry` | | ok (all in Poly's compositor) | ok |
| F4 | `memfd_create("wayland-cursor", ...)`; cursor theme lookups `/usr/share/icons/default/...` (ENOENT); "could not load cursor 'dnd-move'" | | ok | ok |
| F5 | XKB keymap compiled from `/usr/share/X11/xkb/{rules/evdev,keycodes/evdev,types/*,compat/*,symbols/us,pc,inet,...}` (xkb-data is on the disk) | | ok | ok |
| F6 | GTK: `libgtk-4.so.1` tried, then GTK 3 (Debian's `libgtk-3-0t64` on the disk); `gschemas.compiled` missing (GLib critical warning, harmless) | the UI toolkit integration | ok | ok |
| F7 | `mincore`, `restart_syscall` (only windowed) | | ok | ok |
| F8 | X11 instead (`--ozone-platform=x11`, through Poly's Xwayland, ADR 0118): works the same on the reference (Xvfb, traced: 154 threads); it connects to the abstract socket `@"/tmp/.X11-unix/X5"` and uses MIT-SHM: `shmget(IPC_PRIVATE, size, IPC_CREAT\|0600)`, `shmat(id, NULL, 0)`, `shmctl(IPC_RMID)`, and `writev` for requests | | ok in principle (Xwayland on the desktop; System V shm green) | ok |

## Batch G: the desktop integration (in `chromium.rs`, for the record)

| # | What | Where |
|---|---|---|
| G1 | Chromium refuses its sandbox as root: the launcher runs `poly-chromium` (setpriv to uid 1000, the Wayland socket's second name) | `chromium.rs` `LAUNCH_SCRIPT` |
| G2 | the process singleton: `SingletonLock` is a symbolic link "hostname-pid"; a stale lock from another hostname makes Chromium show "The profile appears to be in use by another Chromium process ... on another computer" (an xmessage) and exit; Poly's hostname must stay the same between boots | profile in `/home/poly/.config/chromium` |
| G3 | no D-Bus: "Failed to connect to the bus" lines, Chromium goes on | none needed |
| G4 | `/etc/machine-id` fixed; `/etc/passwd` with `poly` | `configure()` |
| G5 | the GSettings schemas are not compiled (no maintainer scripts): a GLib critical warning on stderr | could compile them at disk build |

## The coverage map and the roadmap

- **Spec entries with a case added now**: `linux/syscall/x86_64/prctl`
  (`PR_SET_VMA_ANON_NAME`, test `lc_prctl_vma_name.c`, visible; the sheet
  also records why `PR_SET_PTRACER` and the speculation controls are not
  diff tests). **prctl is green**, so the new visible test runs in
  polycheck and fails until Poly implements the option.
- **Spec entries that want a case (verify items above)**, in batch order:
  statfs (A2), execve (A3, D2), mmap/mprotect (A4), memfd_create (A6),
  eventfd2 (A7), setsockopt (A22, A23), ioctl (A25), prlimit64 (A28),
  setpriority (A29), open (A35), clone (C1, C5, D3), capget/capset (C3),
  chroot (C4, D4), recvmsg/wait4 (D7), seccomp (E1, E2).
- **Map additions** (not syscalls): the link count of `/proc/<pid>/task`
  (A0), `/proc/<pid>/oom_score_adj`,
  `/proc/<pid>/clear_refs`, `/proc/<pid>/task/<tid>/status` (and `stat`),
  `/proc/sys/kernel/cap_last_cap`, `/proc/sys/kernel/yama/ptrace_scope`
  (optional), `/proc/sys/fs/inotify/max_user_watches`,
  `/sys/devices/system/cpu/kernel_max`, netlink `NETLINK_KOBJECT_UEVENT`,
  `SO_ATTACH_FILTER`, `IP_RECVERR`, the socket ioctls `SIOCGIFNAME`,
  `SIOCETHTOOL`, `SIOCGIWNAME`, `SIOCGIWESSID`; compositor protocols
  `zxdg_output_manager_v1`, `wp_single_pixel_buffer_manager_v1`,
  `zwp_relative_pointer_manager_v1`, `zwp_pointer_constraints_v1`,
  `zwp_text_input_manager_v3`.
- **Owner decision**: chroot (set-user-ID sandbox, batch D) or
  `switch_root` (user-namespace sandbox, batch C) for the desktop disks.

## Implementer batches, in order

1. **Headless without sandbox** (batch A, B): `chromium-version`,
   `chromium-dumpdom-root`, then `chromium-screenshot` with
   `--no-sandbox` by hand. First A0 (the `/proc/<pid>/task` link count,
   what stops Chromium on Poly today), then the `/proc` files (A12-A18), `kernel_max`
   (A19), the socket pieces (A22-A24), `PR_SET_VMA` (A8) and the verify
   items A2-A7, A25, A28, A29, A35.
2. **Sandbox, set-user-ID path** (batches D, E): `chromium-setuid-sandbox`,
   then every sandboxed case on Poly's chrooted disk
   (`chromium-dumpdom-sandbox`, `chromium-dumpdom-binary`,
   `chromium-screenshot`, `chromium-worker`).
   D7 (pids across namespaces) and E1 first: without them the browser
   loses its zygote or gives up seccomp.
3. **Wayland window** (batch F): boot group `chromium` step 3 (launcher,
   window, click). Compositor protocols are optional; start with what the
   core set gives.
4. **User-namespace sandbox** (batch C), if the owner chooses
   `switch_root`.

Run them with:

```sh
cargo xtask chromium fetch                      # any host: the pinned packages
cargo xtask chromium disk                       # any host: target/poly-chromium-disk.img
cargo xtask chromium run                        # any host: the desktop, Chromium in the launcher
cargo xtask chromium apptest --target poly      # Linux host: the visible cases on Poly
cargo xtask boot-tests chromium                 # Linux host: headless + windowed on Poly
```
