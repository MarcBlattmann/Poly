# 0051: BusyBox as init: BusyBox taken at test time, mount(2) over the VFS, ACPI soft-off without AML, signals init waits for

**Status:** Accepted. Builds on [0037](0037-vfs-and-initramfs-in-the-kernel.md)
(the VFS, the initramfs), [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md)
(`poly.init`), [0040](0040-page-cache-and-ext2-on-virtio-blk.md) (ext2 on
virtio-blk), [0041](0041-processes-threads-fork-exec-wait.md) (processes),
[0043](0043-dev-proc-sys-and-file-system-calls.md) (`/proc`, `/sys`, the
`LinuxFs` trait), [0045](0045-credentials-and-capabilities.md)
(capabilities) and [0046](0046-terminals-line-discipline-ptys-job-control.md)
(terminals, job control).

## Context

Phase 4's milestone is "BusyBox shell boots as init and runs
interactively": BusyBox's `init` as process 1, its `rcS` mounting the file
systems, a shell with job control on the console (serial or screen), the
everyday applets, and `poweroff` / `reboot` ending the machine. Processes,
pipes, terminals with job control, `/proc`, credentials and dynamic linking
were there; mount(2), reboot(2), sethostname(2), mknod(2), the priority
and scheduling calls and sendfile(2) were not, and the machine could reset
but not power off.

BusyBox is GPL-2.0. It may be run as a test program, never copied into
the repository, and (CLEAN_ROOM.md) neither its source nor a real Linux
system may be used to learn what it expects: what it needs was found by
running it on Poly and reading Poly's own logs (`not implemented` lines,
the new `poly.strace` records, the applets' error messages).

Sources: mount(2), umount2(2), mknod(2), chroot(2), sethostname(2),
setdomainname(2), uname(2), reboot(2), getpriority(2), sched(7),
sched_setscheduler(2), sched_setaffinity(2), sendfile(2), kill(2),
sigwaitinfo(2), tmpfs(5), proc(5), fstab(5), mount(8), ioctl_tty(2),
ioctl_console(2), setserial(8), bootparam(7); the UAPI headers
`linux/mount.h`, `linux/fs.h`, `linux/reboot.h`, `linux/utsname.h`,
`linux/resource.h`, `linux/sched.h`, `linux/serial.h`, `linux/vt.h`,
`linux/kd.h`, `linux/stat.h`; the ACPI 6.5 specification (§4.8.2.1 ACPI
mode, §4.8.3.2.1 PM1 control registers, §5.2.9 FADT, §7.4.2 `\_Sx`,
§16.1.7 entering S5, §20.2 the AML grammar); BusyBox's own documentation
of the inittab format. No Linux, C library or BusyBox source was read and
nothing was run or observed on a real Linux system.

## Decision

**BusyBox comes from the host, or from the pinned release, at test time.**
`tools/xtask/src/busybox.rs` takes, in order, `POLY_BUSYBOX=<file>`, the
host's statically linked `/bin/busybox` or `/usr/bin/busybox` (checked:
an ELF without `PT_INTERP`; Debian's and Ubuntu's `busybox-static`, which
the CI workflow installs with QEMU), or a build of BusyBox 1.36.1 cached
under `target/busybox/`: downloaded with `curl`, its SHA-256 checked
against the value published next to it, `make defconfig` with
`CONFIG_STATIC=y` (and without `tc`, which newer kernel headers break),
built with the host's compiler (`POLY_BUSYBOX=build` forces this). Without
any of them the boot test says why and is skipped. A dynamically linked
BusyBox would work too (ADR 0049), but would need the host's C library in
the image; a static one keeps the image to one file.

**The image is BusyBox, links and configuration, made by xtask.**
`initramfs::busybox_image`: `/bin/busybox`, a symbolic link per applet
from a fixed list (`BUSYBOX_APPLETS`; one the binary lacks says "applet
not found"), `/etc/inittab` (`rcS` at sysinit, `cttyhack /bin/sh -l`
respawned on the console, `reboot` on Ctrl-Alt-Del, `umount -a -r` at
shutdown), `/etc/init.d/rcS` (mounts what the kernel did not: procfs,
sysfs, devpts and `/dev/shm` are mounted at boot already, the tmpfs on
`/tmp` is new; the host name from `/etc/hostname`), `/etc/profile`
(`PATH`, `HOME`, `PS1='\u@\h:\w\$ '`), `/etc/fstab`, `/etc/passwd`,
`/etc/group`, `/etc/shells`, `/var/run` → `/run`. A fixed list rather
than `busybox --install` at boot keeps the image and the boot's output the
same on every run. `cargo xtask run --busybox` boots it with
`poly.init=/sbin/init` (`console=tty1` for the screen).

**mount(2) is checked in the personality and done over the VFS.** A new
services trait, `LinuxAdmin` (`personalities/linux/src/admin.rs`, reached
through `LinuxKernel::admin`, `None` by default), like `LinuxFs` keeps the
shared trait small. The handlers (`sys/admin.rs`) check privileges
(`CAP_SYS_ADMIN`, `CAP_MKNOD` for device nodes, `CAP_SYS_CHROOT`,
`CAP_SYS_BOOT`), strip the old `MS_MGC_VAL` magic, decide the operation
in mount(2)'s order (remount, bind, propagation, move, new) into a
`MountRequest`, and check a new mount's `data` per type (`mount_options`:
the options tmpfs(5), proc(5) and mount(8) list; unknown ones `EINVAL`).
The kernel (`kernel/src/linux/admin.rs`, `fs/mounts.rs`,
`fs/files/mounts.rs`) mounts:

- `proc`, `sysfs`: a new instance over the same kernel source, so a second
  mount shows the same processes; `devpts`: a view of the one set of
  ptys; `tmpfs` (and `ramfs`): a new tmpfs whose root takes `mode`
  (default 01777), `uid` and `gid`.
- `ext2`: the source must name a block device (`ENOTBLK`); its number
  says which disk. The disks get nodes at boot: `/dev/vda`, `/dev/vdb`,
  ... for the virtio-blk functions in PCI order (`virtio0`, ...; major
  254, Poly's choice where Linux's is dynamic, 16 minors per disk) and
  `/dev/ram0` (1, 0). The disk comes up on first use and is mounted
  behind the page cache like `poly.mount` disks (`disk::mount_flags`);
  unmounting it writes back and marks the volume clean, and is refused
  while busy, `MNT_DETACH` or not.
- Remounts change a mount's flags in place: `poly_vfs::Mount`'s flags are
  an atomic now and `Vfs::remount` sets them (a volume that is read-only
  itself stays so: `EROFS`). Bind mounts use `Vfs::bind`. Propagation
  changes are accepted for a mount point and do nothing (every Poly mount
  is private). `MS_MOVE` is `EINVAL` for now.
- The source string is remembered per mount for `/proc/mounts`
  (`scratch /m tmpfs rw 0 0`).

**reboot(2) and powering off.** The handler checks the magic numbers
(`EINVAL`), the command and `CAP_SYS_BOOT`; `CAD_ON` / `CAD_OFF` are
recorded; `RESTART` / `RESTART2` print "Restarting system." and reset as
the kernel shell's `reboot` does; `HALT` prints "System halted." and stops
the CPU; `POWER_OFF` prints "Power down." and enters ACPI S5; kexec and
hibernation are `EINVAL`. Every one first writes back and unmounts the
disk file systems (`power::prepare_shutdown`), which reboot(2) does not
promise but costs nothing. S5 needs the FADT's PM1a/PM1b control blocks
(now parsed, with `ACPI_ENABLE`) and the sleep type values of `\_S5`,
which only the DSDT's AML declares. Rather than an AML interpreter
(Phase 11), `poly_acpi::aml::sleep_types` finds the declaration by the
grammar of §20.2: `NameOp` (with an optional root or parent prefix),
`_S5_`, a `DefPackage` whose first two elements are integer constants.
That is how firmware, QEMU's included, declares it; a computed `\_S5` is
not found and power-off halts instead. The kernel switches the machine
into ACPI mode if `SCI_EN` is clear, then writes `SLP_TYPx | SLP_EN` to
PM1b and PM1a (other bits kept); QEMU then exits with status 0.

**Signals init waits for are kept.** kill(2) says init only gets the
signals it has handlers for, which the personality enforced by dropping
the others at send time. BusyBox's init blocks its signals and takes them
with `rt_sigtimedwait`, whose wait unblocks them for its duration; so
`poweroff`'s `SIGUSR2` was dropped. Now a signal init blocks, or waits for
in `rt_sigtimedwait` (`LinuxThread::sigwait`), stays pending; the same
rule keeps a default-ignored signal a thread waits for (`SIGCHLD`) from
being discarded during the wait.

**What else BusyBox needed**, each from Poly's logs:

- `cttyhack` finds the console's real terminal with `TIOCGSERIAL` (a
  serial port's `struct serial_struct`) and `VT_GETSTATE` (a virtual
  console's `struct vt_stat`); init asks `VT_OPENQRY`. `poly_tty::Driver`
  gets `device_ioctl`, answered from `poly_tty::hw` (COM1 as a 16550A at
  0x3f8, IRQ 4; `tty1` as virtual console 1; `KDGKBTYPE`).
- sethostname(2) / setdomainname(2) (at most 64 bytes, kept by the kernel,
  reported by uname(2) and `/proc/sys/kernel/{hostname,domainname}`);
  mknod(2) / mknodat(2) (the 32-bit `dev` decoded as makedev(3) encodes
  it; a zero type is a regular file); chroot(2).
- getpriority(2) / setpriority(2) (a nice value per process, inherited by
  fork; the raw call returns `20 - nice`; `EACCES` / `EPERM` as the man
  page says) and the sched(7) calls: the policy is always `SCHED_OTHER`,
  real-time policies are `EPERM` (accepting them would promise behaviour
  Poly lacks), the affinity mask is CPU 0. The scheduler does not use the
  nice value yet. `sched_yield` succeeds.
- sendfile(2) (`cat`): chunked through a kernel buffer, at `*offset` or
  the input's file offset; a short write gives back what was not written
  where the input can seek.

**Debugging aids.** `poly.strace` (or `poly.strace=<comm>`) writes one
`strace: pid N name(args) = ret` line per system call of every Linux
process, string arguments decoded, without changing the processes'
terminals (unlike `poly.test.capture`). Each unimplemented call is logged
once per boot (an interactive shell's console would otherwise fill with
`rseq`), and the kernel logs a process stopped, continued or killed by an
everyday signal only with `poly.strace` (those whose default is a core
dump always).

**Tests.** The boot test (`cargo xtask test`, alone `cargo xtask
busybox-test`) boots BusyBox's init on the serial console with the test
disks attached and types into its shell through QEMU's standard input,
waiting for each prompt with generous deadlines: `stty`, `echo`, `ls /`,
`/proc/meminfo`, a pipeline, a file in `/tmp`, `sleep 0.2`, a background
job with `jobs` and `kill %1`, Ctrl-C (`$?` 130), Ctrl-Z / `bg` / `fg`,
`ps`, `mount`, a tmpfs mounted with options and unmounted, the ext2 test
disk mounted from `/dev/vdc`, `hostname` and `uname -a`, `id`, `exit`
(init starts a new shell), and `poweroff` ending QEMU with status 0.
Host tests cover every new handler against a recording `LinuxAdmin`, the
option parser, `Vfs::remount`, the FADT fields and the `\_S5` scan;
kernel tests (`linux::admin_tests`) mount, remount, bind and unmount the
memory file systems, make device nodes and FIFOs, `chroot`, set the host
name and make the harmless reboot calls from ring 3.

## Consequences

- BusyBox runs as init on the serial console and on the screen
  (`console=tty1`) with job control, and powers the machine off.
- `LinuxKernel` has one more method (`admin`), `LinuxProcess` a `nice`
  field and `LinuxThread` a `sigwait` set; `poly_vfs::Mount`'s flags are
  read with `flags()`.
- Not done yet: `MS_MOVE` (switch_root), mount propagation, `MNT_EXPIRE`,
  other disk file systems than ext2 (and partitions), remounting a disk
  volume read-write after it was mounted read-only, Ctrl-Alt-Del from the
  keyboard (the setting is recorded only), a scheduler that uses nice
  values, `ioprio_get` / `ioprio_set`, `copy_file_range`, `/proc/<pid>/comm`
  of the first program (it names the file the path resolved to, not the
  path), more than one virtual console.
- Choices the documents leave open (check orders, the virtio-blk major, a
  tmpfs root's default mode, `VT_OPENQRY` with no free console, the
  priority of a group) are marked "to be confirmed by the diff runner"
  in the code or here.
