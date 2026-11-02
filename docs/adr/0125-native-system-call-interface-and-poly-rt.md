# 0125: Native user space: Poly's own system call interface, the `poly-rt` runtime, and the plan to move the desktop onto it

**Status:** Accepted. Roadmap Phase 6, the first milestone of
[ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
(decisions 1 and 4). It extends [ADR 0001](0001-multiple-personalities-one-kernel.md)
with a fourth personality, Poly's own, and reuses the ELF loader of
[ADR 0024](0024-elf-loader.md) / [ADR 0036](0036-demand-paging-and-exec.md).

## Context

ADR 0124 decided that the desktop (compositor, shell, panels, notifications,
settings) becomes native Poly user space: pure Rust on Poly's own system
calls, in user-space processes, built with `rust-lld` and no host C
toolchain, so that it builds the same on Windows and Linux hosts. Until now
the only user-space interfaces were the guest worlds': the desktop is a set
of static glibc programs (`x86_64-unknown-linux-gnu`) on the Linux
personality. Linux apps must keep reaching the desktop (Wayland over
AF_UNIX sockets, XWayland, D-Bus), so native and Linux processes have to
share the kernel's objects.

This ADR records stage 1, which is built and tested (the interface, the
kernel side, the runtime, the build, a boot test), and the plan for stage 2
(the compositor and shell moving onto it).

## Decisions

### 1. A native program is an ELF executable with Poly's ABI note

The kernel recognises a native program by an **ELF note**: an `SHT_NOTE`
section in a `PT_NOTE` segment with owner `"Poly"` (name size 5 with its
NUL), type `NT_POLY_ABI` = 1, and a 4-byte little-endian description, the
native ABI version the program was built for. `poly-binfmt` reports such an
image as `Format::Native { arch, abi }`, whose personality is the new
`Personality::Native`; any other ELF stays Linux. The kernel runs ABI
versions `1..=ABI_VERSION` and refuses others with a reason.

Why a note:

- It is how other systems tag their ELF binaries (`.note.ABI-tag` for GNU,
  `.note.tag` / `.note.netbsd.ident` for the BSDs), tools show it
  (`llvm-readobj --notes`), and it carries a version.
- `rust-lld` writes it from an ordinary Rust static (`poly-rt` puts
  `#[link_section = ".note.poly.abi"] #[used] static ABI_NOTE`); LLVM makes
  every `.note*` section `SHT_NOTE`, and lld places notes at the start of
  the first read-only segment, right after the program headers, and keeps
  them from garbage collection. No linker script and no post-link step.
- The alternative, `EI_OSABI`: the values are assigned by the gABI's
  maintainers (64–255 are architecture-specific, none free for us on
  x86-64), lld cannot set one, so every build would need a patch step,
  and it has no room for a version.

The note must lie in the first page (`program::prepare` reads 4096 bytes to
decide the format). `cargo xtask` checks every native program it builds
with `poly-binfmt` on its first page, so a program without the note fails
the build rather than starting as a Linux program.

A native program is loaded like a **static ELF** (`user::exec::load`: a
`PT_INTERP` is refused) into a process made by `Process::new_native`, with
the ELF initial stack (argc, argv, envp, auxv): `poly-rt` reads it, and no
new loader was needed. The vDSO is mapped as into every ELF process; native
programs do not use it yet.

### 2. The calling convention

| | |
|---|---|
| instruction | `syscall` |
| call number | `rax` |
| arguments | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` |
| result | `rax`: the value, or `-code` of a native error (`-4095..=-1`) |
| clobbered | `rcx`, `r11` (the instruction itself); every other register is kept |

- The registers are the ones `syscall` leaves usable (the same as Linux's,
  for the same reason: `rcx` and `r11` hold the return address and flags).
- One register for the result, errors in the top 4095 values: one compare
  decodes it, and every value a call returns (addresses, sizes,
  nanoseconds since 1970) is far below that range.
- **Poly's own errors**, not errno: a closed set of 22 codes
  (`Unsupported`, `InvalidArgument`, `BadHandle`, `NotFound`,
  `PermissionDenied`, `AlreadyExists`, `NoMemory`, `BadAddress`,
  `WouldBlock`, `Interrupted`, `TimedOut`, `BrokenPipe`, `NotADirectory`,
  `IsADirectory`, `NoSpace`, `TooLarge`, `Busy`, `NoChild`,
  `NotExecutable`, `Io`, `TooManyHandles`, `ConnectionRefused`). The
  kernel's shared objects report Linux errno numbers (the Linux personality
  hands them out unchanged); `Error::from_errno` maps them, and anything
  without a counterpart is `Io`. An unknown code (a newer kernel's) decodes
  as `Io`.
- Strings and buffers are a pointer and a length, never NUL-terminated
  (Rust's `&[u8]`/`&str`); arrays of strings are `Str { ptr, len }` records.
- Growth: the ABI note's version says what a program expects; structures
  passed by pointer come with their size (`spawn(request, size)`), and
  flag words have reserved bits that must be 0. A new call gets a new
  number; numbers are never reused.

### 3. The first call table

| # | Call | Arguments → value |
|---|---|---|
| 0 | `exit` | `code` (low 8 bits) → never returns |
| 1 | `write` | `handle, buf, len` → bytes written |
| 2 | `read` | `handle, buf, len` → bytes read (0: end) |
| 3 | `open` | `path, path_len, flags, mode` → handle |
| 4 | `close` | `handle` |
| 5 | `dup` | `handle` → new handle on the same open file |
| 6 | `seek` | `handle, offset, whence` → position |
| 7 | `map` | `hint, len, prot, flags (0)` → address of zeroed private pages |
| 8 | `unmap` | `address, len` |
| 9 | `protect` | `address, len, prot` |
| 10 | `clock` | `id` (0 monotonic, 1 wall clock) → nanoseconds |
| 11 | `sleep` | `nanoseconds` |
| 12 | `yield` | |
| 13 | `spawn` | `request, size` → pid |
| 14 | `wait` | `pid` → exit status |
| 15 | `process_id` | → pid |

`open` flags: `READ`, `WRITE`, `CREATE`, `EXCLUSIVE`, `TRUNCATE`, `APPEND`,
`NONBLOCK`, `DIRECTORY`, checked for consistency (truncating needs writing,
exclusive needs creating, a directory is only read). `map` works in whole
pages and only hands out private anonymous memory; mapping handles comes
with stage 2. A `read` or `write` moves at most 64 KiB per call (a short
transfer, as on a pipe); `write` copies in 64 KiB chunks so no large kernel
buffer is needed. That is what a hello world, an allocator, files, time and
child processes need.

### 4. Handles are the kernel's shared open files

A native **handle** is a descriptor in the process's `fs::Files`, the same
table type Linux processes have, naming the same `OpenFile` objects: files,
pipes, sockets, DRM and evdev devices, memfds. So a socket a Linux client
connected to, or a buffer it passes with `SCM_RIGHTS`, is an ordinary
handle in a native process, with no translation layer: this is what lets
the native compositor serve Linux clients (stage 2). Reads and writes block
exactly as the Linux personality's do (`OpenFile::read_wait` with the
thread's `ThreadBlocker`), so the end of the process interrupts them.
Handles 0, 1 and 2 are standard input, output and error (the console to
begin with).

Handles are **not inherited** implicitly: `spawn` gives the child exactly
the three handles its request names (or none), and nothing else. Native
processes have no `exec`, so there is no close-on-exec to get wrong.
Stage 2 adds an explicit list of extra handles (XWayland's `-listenfd`,
`-displayfd`, `-wm` and `WAYLAND_SOCKET`).

### 5. Processes

- `spawn` starts **any program Poly runs**: it goes through
  `program::prepare`, so the child may be native, a Linux ELF or a Windows
  PE, each in its own personality. That is how the native compositor will
  launch apps. The boot test spawns the Linux nolibc hello world from a
  native program, sharing the console handle.
- The parent keeps each child (`NativeState` on its `Process`) until `wait`
  reaps it; `wait` returns `Exited(code)`, `Signaled(signal)` (a Linux
  child) or `Faulted(vector)`, encoded in one value. A wait is interrupted
  if the waiting process is ended.
- Native processes have **no signals**. `exit` ends the whole process
  (`Process::group_exit`). A CPU fault the process does not survive kills
  it (`user::fault`), and its parent sees `Faulted(14)` for a page fault.
- Children's parent id is 0 for now (the kernel's own children), as for
  programs the kernel starts.

### 6. Where the code lives

- `personalities/native` (`poly-personality-native`, host-tested): the ABI
  (`abi`: numbers, errors, flags, structures, the note), the services the
  kernel provides (`NativeKernel`), the handlers (`sys`: every argument
  checked and decoded, user memory copied in and out), the table. Its
  tests run the dispatcher against a fake kernel (writes in chunks and
  short writes, flag and path checks, page rounding, spawn requests and
  their limits, wait reaping once, the error round trip).
- `kernel/src/native`: `NativeKernel` for the calling thread on the
  kernel's objects (about 300 lines), `syscall::dispatch` routing
  `Personality::Native`, `program::prepare` loading native programs.
- `crates/poly-rt`: the runtime (below). `userland/native`: native
  programs, a workspace of their own like the kernel's.

### 7. Build: `x86_64-unknown-none`, `rust-lld`, no unstable flags

Native programs are built for the **`x86_64-unknown-none`** target, which
the pinned toolchain already installs (the kernel uses it) with prebuilt
`core` and `alloc`, and whose linker is `rust-lld`. Flags
(`userland/native/.cargo/config.toml`): `relocation-model=static`
(a position-dependent `ET_EXEC`, so the runtime needs no self-relocation),
`code-model=small` (the image sits at lld's default `0x200000`),
`--build-id=none`. No `-Zbuild-std`, no linker script, no host C compiler:
`cargo xtask build` builds them first on every host, before the kernel,
checks their ABI note, and `cargo xtask run` puts them into the initramfs
as `/bin/<name>`.

One caveat: `x86_64-unknown-none` is a kernel target and compiles floating
point as soft-float (no SSE). That is fine for stage 1; the compositor's
pixel work wants SSE2. The way there is a target of our own,
`x86_64-unknown-poly` (a target JSON: the same with `+sse2` and hard
floats), built with `-Zbuild-std` under `RUSTC_BOOTSTRAP`, confined to
xtask and cached by content hash exactly as our `ntdll.dll` is
(ADR 0071). It is taken when a measurement of the compositor shows the
cost, with the ABI unchanged.

### 8. The runtime: `poly-rt`, `no_std` + `alloc`

`poly-rt` gives a native program what `std` would:

- `_start` (`global_asm!`): reads argc, argv and envp off the initial
  stack, calls the program's `main` (named with `poly_rt::entry!(main)`),
  exits with its value;
- the ABI note;
- `sys`: one function per call (inline `asm!`);
- `io` (`print!`, `println!`, `eprintln!`: one `write` per call so lines do
  not mix with another process's), `fs` (`File`, closed on drop; `read`,
  `write`), `mem`, `time` (`Instant`, wall clock, `sleep`), `process`
  (`exit`, `id`, `Command` / `Child` with `Stdio`), `env` (`args`, `vars`,
  `var`);
- the global allocator: **`poly-heap`**, the kernel's own slab heap, over
  pages from `map`. Slabs want runs aligned to their size (up to 32 KiB);
  `map` promises pages, so such a run is mapped with room to spare and the
  ends unmapped (`heap::trim`, host-tested);
- a panic handler that writes `panic: <message>` to standard error (without
  allocating) and exits with 101.

On the host every call returns `Unsupported` and the entry point,
allocator and panic handler are left out, so `poly-rt`'s logic is tested
with the other host crates.

**`no_std` + `alloc` now, not a `std` port.** A `std` port means a Rust
target of our own with `std`'s platform layer written for Poly, built with
`-Zbuild-std` on every build (until the target is upstream); it is large
and buys little for the desktop, which uses a narrow part of `std` (below).
`poly-rt` instead grows `std`-shaped modules for what the compositor uses
(sockets, poll, handle mappings, children). A `std` port stays possible
later, on the same system calls, if third-party crates need it.

### 9. Proof on both hosts

- The boot test group `native` (CI shard `userland`): `/bin/native-hello`
  as the first process prints, reads its arguments, allocates (100 000
  numbers, 3 MiB, a 2 000-entry map, reuse), sleeps and reads both clocks,
  writes and reads a file, spawns `/bin/native-child` three times (exit 42,
  a panic: 101, a page fault: `Faulted(14)`), fails to spawn a missing
  program (`NotFound`), and spawns the Linux `/bin/hello`, which prints on
  the shared console; it reports `done, 0 failed` and exits 0. A second boot
  runs `/bin/native-child exit 7` as init and checks status 7.
- It passes natively on Windows (`cargo xtask boot-tests native`, QEMU for
  Windows on TCG) and in the Linux container; `cargo xtask run
  poly.init=/bin/native-hello` boots into it on Windows.
- `personality_routing` (the self-test) now also routes a native header.
- `ci lint` runs `cargo fmt` and `cargo clippy` on `userland/native` for
  its target, which covers `poly-rt`'s native-only code.

## Stage 2: moving the compositor and shell onto the native runtime (plan)

### What `userland/poly-compositor` uses today

The protocol, scene, shell, D-Bus, X11 window manager, DRM model, input
translation and fonts are already `no_std` + `alloc` host-tested crates
(`poly-wayland`, `poly-shell`, `poly-dbus`, `poly-x11`, `poly-drm`,
`poly-input`, `poly-font`) with no OS calls. The OS glue is in
`userland/poly-compositor` (mostly `unix.rs`, `kms.rs`, `evdev.rs`,
`fbdev.rs`, `mixer.rs`, `session.rs`, `xwayland.rs`, `desk.rs`) and
`userland/poly-notify`. The compositor is **single-threaded** on one
`poll` loop: no threads, no epoll, no timerfd or eventfd.

| Area | Today (libc / std) | Native call(s) it needs |
|---|---|---|
| Wayland, D-Bus and X11 sockets | `UnixListener::bind` (`$XDG_RUNTIME_DIR/wayland-0`, `.../bus`, `/tmp/.X11-unix/X<n>`), the abstract `@/tmp/.X11-unix/X<n>`, `UnixStream::pair`, `connect` (notify-send), `set_nonblocking` | `socket_listen(path or abstract name)`, `socket_accept`, `socket_connect`, `socket_pair`, `set_nonblocking`: on the kernel's AF_UNIX sockets (`kernel/src/linux/net`), the same objects and name space Linux clients use |
| fd passing | `sendmsg` / `recvmsg` with `SCM_RIGHTS` (up to 28 fds a message), `MSG_NOSIGNAL` | `socket_send(handle, bytes, handles)`, `socket_recv(handle, buf, handles_out)`: handles are descriptors already, so passed files land in the native table as they would in a Linux one; no `SIGPIPE` exists to suppress |
| Peer identity | `getsockopt(SO_PEERCRED)` (bus `EXTERNAL` auth), `getuid`, `getpid` | `socket_peer(handle)` → pid, uid; the native process's uid (`process_info`) |
| Waiting | `poll` over listeners, clients, evdev, the DRM card, the mixer, bus connections, Xwayland's fds, a SIGCHLD self-pipe | `poll(entries, count, timeout)` on `poly_vfs::poll` (what Linux's `poll` uses) |
| Client buffers | `mmap(PROT_READ, MAP_SHARED)` of `wl_shm` pool fds | `map_handle(handle, offset, len, prot, shared)`: the mmap path of `LinuxKernel::mmap` (files, memfds, DRM dumb buffers, fbdev) |
| Own shared memory | `memfd_create` + `F_ADD_SEALS`, reopened read-only via `/proc/self/fd` (dma-buf feedback table); the keymap as a file in the runtime dir | `memory_object(size)` → handle (a memfd), `seal`, `reopen(handle, read-only)` |
| DRM/KMS | `open("/dev/dri/card0", O_RDWR\|O_NONBLOCK)`, `ioctl` `MODE_GETRESOURCES`, `GETCONNECTOR`, `GETENCODER`, `SETCRTC`, `CREATE_DUMB`, `MAP_DUMB`, `ADDFB2`, `PAGE_FLIP` (+ event), `RMFB`, `DESTROY_DUMB`; flip events `read` from the card; `stat` → `st_rdev` of `renderD128` | `open`, `read` (have), `control(handle, request, arg)`: the device's own request numbers and structures (the DRM uapi of `poly-drm`, ADR 0108), passed to `OpenFile::ioctl`; `handle_info(handle)` → device numbers |
| Input | `/dev/input/event0..31`, `EVIOCGBIT`, `EVIOCGABS`, `EVIOCGNAME`, `EVIOCGRAB`, `input_event` reads | `open`, `read`, `control` |
| fbdev fallback | `/dev/fb0`, `FBIOGET_VSCREENINFO`, `FBIOGET_FSCREENINFO`, `FBIOPAN_DISPLAY`, `FBIO_POLY_DAMAGE`, `mmap` | `control`, `map_handle` |
| Sound settings | `/dev/snd/controlC0`, ALSA ctl `CARD_INFO`, `ELEM_INFO/READ/WRITE`, `SUBSCRIBE_EVENTS` | `control` |
| Children | `fork` + `execve` with an explicit env and a hand-made `PATH` search; launcher apps, `--exec`, Xwayland (fds kept by clearing `CLOEXEC`); `waitpid(WNOHANG)` after a SIGCHLD self-pipe; `kill(SIGTERM)` (Xwayland), `kill(pid, 0)` (X lock liveness) | `spawn` (have) with an extra-handles list; a **process handle** from `spawn` that becomes readable when the child ends (`poll` instead of SIGCHLD; the kernel's pidfd object, ADR 0111), `wait(pid, no_hang)`, `terminate(pid)`, `process_alive(pid)` |
| Files | `read_to_string` (settings, `.desktop` files, `/proc/cpuinfo`, `/proc/meminfo`, X lock), `read` (font), `write` + `rename` (settings, PPM dump), `File::create` + `sync_all`, `create_dir_all` + `set_permissions` (0700 runtime dir, 01777 X socket dir), `OpenOptions::create_new` (X lock), `read_dir`, `metadata`, `remove_file`, `sync()` | have `open`/`read`/`write`; add `rename`, `remove`, `make_dir(mode)`, `set_mode`, `read_dir`, `stat`, `sync(handle)` / `sync_all` |
| Time | `clock_gettime(CLOCK_MONOTONIC / CLOCK_REALTIME)` | `clock` (have) |
| Environment | `args`, `vars` (copied into children's env), `XDG_RUNTIME_DIR`, `POLY_DISPLAY_SCALE`, `XDG_CONFIG_HOME`, `HOME`, `POLY_XWM_DEBUG`; sets `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`, `DISPLAY` for children | `poly_rt::env` (have), `Command::env` (have) |
| System status | `uname`, `getifaddrs` (network indicator every 2 s) | `system_info` (kernel name, version, machine); the interface list through a native call over the network stack's interface table |
| Signals | `SIGUSR1` for a PPM dump, `SIGCHLD` | none: the dump becomes a command on the session bus or a file the shell watches; children through process handles |

### Native calls to add, in order

1. **Handle operations shared with every personality:** `control` (ioctl
   pass-through), `poll`, `map_handle`, `memory_object` + `seal` +
   `reopen`, `handle_info`, `set_nonblocking`, and the file calls (`rename`,
   `remove`, `make_dir`, `set_mode`, `read_dir`, `stat`, `sync`). Tested by
   a native program driving `/dev/dri/card0` (a mode set and a page flip,
   checked by screenshot as the `drm` group does) and reading evdev.
2. **AF_UNIX sockets:** listen (path and abstract names), accept, connect,
   pair, send and receive with handles, peer identity. Tested by a native
   echo server that a Linux client connects to by path and passes an fd
   (the existing C client of the `wayland` group), and the reverse.
3. **Processes:** `spawn` with extra handles, process handles readable on
   exit, `wait` without blocking, `terminate`, `process_alive`.
4. **Port the compositor.** Its OS glue moves behind one interface (the
   calls above) with a `poly-rt` implementation; `poly-compositor` and
   `poly-notify` become `no_std` + `alloc` native programs in
   `userland/native`, built by `native.rs` for every host. The `desktop`,
   `desktop-shell`, `wayland-kms`, `xwayland` and `toolkits` boot groups
   then run the native compositor. The glibc build in `compositor.rs`
   goes once they pass.
5. `cargo xtask desktop` builds and boots the desktop on Windows (the
   C test clients of the `wayland` group stay Linux-only tooling and say so
   there).

### What stays shared with Linux clients, and how

- **Wayland:** the native compositor listens on the same path,
  `$XDG_RUNTIME_DIR/wayland-0` (`/run/user/0/wayland-0`), in the kernel's
  one AF_UNIX name space; Linux clients connect with their own `connect(2)`
  through the Linux personality, and both ends are the kernel's socket
  objects. Buffers they pass (`wl_shm` memfds, dma-bufs) arrive as handles
  in the compositor's table and are mapped with `map_handle`.
- **X11:** Xwayland stays a Linux program, spawned by the native
  compositor with its sockets as extra handles; X clients reach it through
  `/tmp/.X11-unix/X<n>` and the abstract name, as today.
- **D-Bus:** the session bus stays in the compositor (`poly-dbus`) on
  `$XDG_RUNTIME_DIR/bus`; Linux clients (`notify-send`, toolkits) connect by
  path. `poly-notify` can stay a Linux client, which keeps the bridging
  tested, or become native.
- **Devices:** `/dev/dri/card0`, `/dev/input/event*`, `/dev/fb0` and the
  ALSA control device are the same device files; native `control` passes
  their own request numbers through, so the structures in `poly-drm`,
  `poly-input` and `poly-fbdev` serve both worlds.
- Windows and macOS apps reach the native compositor later through their
  own window interfaces (win32k, the window server), bridged in the kernel
  or in native helpers, not through Linux's.

## Consequences

- Four personalities: `Personality::Native` joins Linux, NT and XNU in
  `poly-abi`, the router and the unimplemented-call log
  (`syscall: pid N native/native #n not implemented`).
- `userland/native` is a workspace of its own (excluded from the host
  workspace, like `kernel`), built by `cargo xtask build`, `run` and every
  boot test run, on Windows and Linux.
- A Linux process cannot `execve` a native program yet (`ENOEXEC`); native
  programs start from `poly.init`, the kernel shell's `run`, or `spawn`.
- Panic messages carry the host's path separators (`hello\src\...` on
  Windows), the only difference between a Windows and a Linux build;
  `--remap-path-prefix` can make the bytes identical when that matters.
- Traced test runs (`poly.test.capture`) record a native process's output
  and exit, not its calls; a native call tracer comes with the call set of
  stage 2.
