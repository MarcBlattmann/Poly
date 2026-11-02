# 0126: The desktop on the native runtime: ABI version 2, Linux programs that exec native ones, and the compositor and notify-send as native programs

**Status:** Accepted. Roadmap Phase 6, the second and fourth milestones of
[ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md).
It carries out the stage 2 plan of
[ADR 0125](0125-native-system-call-interface-and-poly-rt.md) and extends
its call table (ABI version 2); ADR 0125's choices for version 1 stand.

## Context

ADR 0125 gave Poly its own system call interface, the `poly-rt` runtime
and a first native program, and planned how the compositor would move onto
it: the calls the compositor's OS glue needs (device requests, `poll`,
mappings of handles, memory objects, the file calls, AF_UNIX sockets with
handle passing, process handles), in that order, then the port itself.
This ADR records what was built and the decisions the port forced.

## Decisions

### 1. ABI version 2: 29 more calls

`poly-rt` now writes ABI version 2 into its programs; the kernel runs
versions 1 and 2. The new calls (numbers 16 to 44):

| Group | Calls | On the kernel's side |
|---|---|---|
| Devices | `control(handle, request, arg)` | `Files::ioctl`: the device's own request numbers and structures (DRM, evdev, fbdev, ALSA), so `poly-drm`, `poly-input` and `poly-fbdev` serve both worlds |
| Waiting | `poll(entries, count, timeout_ns)` | `poly_vfs::poll::poll`, what Linux's `poll` uses; the event bits are the kernel's readiness bits (POSIX's values) |
| Memory | `map_handle(handle, offset, len, prot, flags)`, `memory_object(size)`, `seal`, `truncate` | DRM buffers and dma-bufs (`drm::mapping`), the frame buffer's shadow, files and memfds (with their write seals) |
| Handles | `set_nonblocking`, `handle_info` | status flags, `fstat` |
| Files | `stat`, `rename`, `remove`, `make_dir`, `set_mode`, `read_dir`, `sync`, `make_fifo`, `pipe` | `Files`' calls; `read_dir` writes packed records (`DirRecord`) and resumes where it stopped |
| Sockets | `socket`, `socket_pair`, `bind`, `listen`, `accept`, `connect`, `send`, `recv`, `socket_peer` | `poly-unix` sockets in the one name space the Linux personality uses (`linux::net::namespace`); names are paths or abstract names; handles travel with the first byte, as `SCM_RIGHTS` |
| Processes | `process_handle(pid)`, `signal(pid, probe/terminate/kill)`, `wait(pid, NO_HANG)` | a handle object readable once the child ended (`Process::ended` notifies its queue); a Linux child gets `SIGTERM`/`SIGKILL`, a native child ends `Signaled` |
| System | `net_interfaces` | the network stack's interfaces (`linux::net::interfaces`) |

`spawn`'s request grew from 64 to 80 bytes: a list of handle pairs
(`HandleMap`, the caller's `from` as the child's `to`), which is how
Xwayland gets its `-listenfd`, `-displayfd`, `-wm` and `WAYLAND_SOCKET`
descriptors. A 64-byte (version 1) request still works. `wait` takes
flags; the first table's callers pass 0.

Every handler checks its arguments in `personalities/native` (host tests
with a fake kernel: polls copied in and out, handles passed and received,
a receive that cannot report its handles closes them, names and modes
checked, directory records resumed). The kernel side stays a mapping onto
existing objects (`kernel/src/native/mod.rs`, about 770 lines).

### 2. A Linux program can `execve` a native one

The desktop is started from a shell on the root disks (`xwayland` and
`toolkits` run `poly-compositor` from bash, and BusyBox's init may). So a
Linux process may now `execve` a native program: at the point of no
return the process's `NativeState` is activated, and from its next call
on `syscall::dispatch` routes it to the native personality. It keeps its
Linux state: its place in the process tree (its parent's `wait4` sees
how it ended), its signals with their default actions (a shell's `kill`
or `^C` still ends it; `execve` already reset the handlers), its open
descriptors as handles. The personality tag stays what it was (the tag
says how the process started); `NativeState` says how its calls are
read. This was cheaper than making the tag mutable and is all the desktop
needs.

### 3. A spawned child starts where its spawner is

`spawn` looks the program (and a Linux program's interpreter) up from the
caller's root and working directory, and the child starts with copies of
them (`program::prepare_child`, `Files::adopt_context`): on the root
disks the compositor runs inside the disk's root, and `/usr/bin/Xwayland`
is the disk's. Version 1 looked programs up from the kernel's root.

`Process::ppid` of a spawned child is the spawning process's pid. A Linux
child stays a root of the Linux process tree (its `getppid` is 0),
because the native parent is not a member of that tree; nothing in the
desktop relies on it.

### 4. No signals for the desktop: the control pipe

The compositor took `SIGUSR1` as a request for a picture of the output
(the Wayland test clients sent it to their parent). Native processes have
no signals, so the compositor makes a named pipe in the runtime
directory, `$XDG_RUNTIME_DIR/poly-compositor.ctl`, names it to its
clients in `POLY_COMPOSITOR_CONTROL`, and reads `dump` lines from it
(opened read-write, so it never reports an end). The C test clients
(`tests/linux/wayland/wl-client.c`, `wl-proto.c`) write `dump` there
instead of signalling.

`SIGCHLD` and its self-pipe are replaced by process handles: every child
the compositor starts is watched through its handle, polled with the
clients, and reaped (`wait` without hanging) when it turns readable.

### 5. The compositor's code is a host-tested library, the program a thin binary

`userland/poly-compositor` and `userland/poly-notify` are now `no_std` +
`alloc` libraries in the host workspace, tested there like before (24 and
1 tests; the tests that needed a Linux host's sockets and files moved to
the boot tests that already cover them). `userland/native/compositor` and
`userland/native/notify` are the programs, three lines each
(`poly_rt::entry!(poly_compositor::main)`). Everything the compositor
asks of the system goes through one module, `os.rs` (shared memory,
handle messages, `poll`, devices, children, the control pipe), plus
`poly-rt`'s `fs` and `net`; `io.rs` keeps the `std::io` shape of its
errors so the rest of the code reads as before. The drawing code and the
shell (`poly-shell`, `poly-gfx`, `poly-font`, `poly-wayland`) were not
touched.

On the way:

- The keymap and the dma-buf format table are sealed memory objects,
  reopened read-only through `/proc/self/fd/<n>` (the keymap was a file in
  the runtime directory, unlinked once open, because Poly had no memfds
  then).
- `uname` comes from `/proc/sys/kernel`, the network indicator from
  `net_interfaces`, the settings file is written to a temporary file,
  synced and renamed as before.
- Native processes run as uid 0 (`poly_rt::process::uid`); the session
  bus authenticates Linux clients by the peer's uid, as before.
- Device numbers handed to clients (`zwp_linux_dmabuf_v1`'s main device)
  are Linux `dev_t` values (`os::linux_dev_t`), what clients compare with
  their `stat`.

### 6. The glibc build is gone; the desktop builds on every host

`tools/xtask/src/compositor.rs` no longer builds the compositor for
`x86_64-unknown-linux-gnu` with the host's static glibc; `native.rs`
builds it with the other native programs (`/bin/poly-compositor`,
`/bin/notify-send`). `cargo xtask desktop` therefore builds and boots on a
Windows host: the C test clients (`wl-demo`, `wl-client`, which need a
Linux C toolchain) are left out of its image when they cannot be built,
and the desktop boot tests skip themselves without them as before.

Native builds remap the checkout to `/poly` and the toolchain to `/rust`
(`--remap-path-prefix` through `cargo --config`), so programs carry no
host directories. The one difference left between a Windows and a Linux
build is the separator inside workspace-relative paths of panic
locations.

### 7. Soft floats are fast enough: no target of our own yet

ADR 0125 kept a hard-float target (`x86_64-unknown-poly`) for when the
compositor's speed called for it. Measured on QEMU's emulator (TCG):

- the desktop's first frame: 62 to 68 ms;
- `wayland-kms`: 73 flips in 2 s (36 per second);
- every desktop boot group within its old timeouts.

So the compositor stays on `x86_64-unknown-none`.

### 8. Proof

| Check | Where | Result |
|---|---|---|
| `native` (`native-hello` exercises ABI version 2: directories, a named pipe, a pipe and `poll`, a memory object mapped twice, a handle passed to a native child over a socket pair, a listener by path and by abstract name, the peer's pid, a process handle, terminating a child, `uname`, the interfaces) | Windows (native), Linux container | pass |
| `wayland`, `wayland-kms` (Linux C clients against the native compositor: sockets, `SCM_RIGHTS`, SHM and dma-buf buffers, the control pipe, vsync) | Linux container | pass |
| `desktop`, `desktop-shell` (the shell, launched Linux clients, the session bus with the native `notify-send`, the mixer, settings saved and read back) | Linux container | pass |
| `xwayland`, `toolkits` (the compositor started from bash and from BusyBox's init on the root disks, Xwayland as its Linux child, xdpyinfo, xeyes, xclock, xev, xterm, GTK 3, GTK 4, Qt 6, VLC with its sound and pictures) | Linux container | pass |
| `cargo xtask desktop` | Windows (native) | builds and boots to the desktop (screenshot through QEMU's monitor) |

## Consequences

- The desktop is native user space: the compositor, the shell, the
  session bus and `notify-send` are native processes; Xwayland and the
  apps are Linux processes they start and talk to through the kernel's
  shared objects.
- A native program can be any process's `execve` target, so it can sit
  anywhere in a Linux process tree.
- Still to come: threads for native programs, credentials of their own
  (they run as root), a native call tracer, and the QMP-driven boot tests
  on Windows hosts (they use Unix sockets today).
