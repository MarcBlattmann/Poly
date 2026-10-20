# 0039: Networking in the kernel: virtio-net in a host-tested crate, one stack behind a sleeping mutex, a network thread, readiness generations for blocking sockets

**Status:** Accepted. Builds on [ADR 0028](0028-tcp-ip-stack-in-a-crate.md)
(the stack and its `NetDevice` / `poll(now)` interface),
[ADR 0009](0009-drivers-behind-access-traits.md),
[ADR 0015](0015-pci-and-virtio-in-the-kernel.md) and
[ADR 0018](0018-msi-x-and-interrupt-driven-disk-io.md) (PCI, DMA, MSI-X).

## Context

Phase 4 lists "Networking: virtio-net driver, own TCP/IP stack (IPv4,
IPv6, TCP, UDP, ICMP)". The stack exists as `poly-net` (ADR 0028), which
leaves the kernel a driver implementing `NetDevice`, a lock around the
`Stack`, something that calls `poll` on interrupts and at the deadline it
returns, randomness for its secret, an address configuration, and a way
for threads to block on sockets. The Linux socket system calls are a
later milestone; they need blocking calls with timeouts and a way to wait
on readiness (for `poll`, `select`, `epoll`).

## Decision

**The virtio-net driver is a host-tested crate, `poly-virtio-net`,** like
`poly-virtio-blk`: allocation-free, register access through `poly_pci::Mmio`,
queue memory, packet buffers and tables from the caller. It follows VIRTIO
1.2 §5.1 and implements `poly_net::NetDevice`.

- Features: VERSION_1 (required), MAC (else a locally administered address
  from caller-provided random bytes, §5.1.4.2), STATUS (link state), MTU
  (only when the buffers can receive a frame of that size; the stack is
  given at most 1500), MRG_RXBUF, EVENT_IDX, ACCESS_PLATFORM. Never
  checksum or segmentation offloads, the control queue, several queue
  pairs or hash reports: every packet carries full checksums, which the
  stack computes and checks anyway, and one queue pair is enough for one
  CPU.
- One queue pair (receive queue 0, transmit queue 1). Every packet buffer
  is 2 KiB with one descriptor; the 12-byte header and the frame share it
  (VERSION_1 implies any descriptor layout). Frames are copied between
  these buffers and the stack's (as ADR 0028 intends), so the device only
  ever sees memory the driver owns, and buffers are recycled at once.
- Receive buffers all go to the device before DRIVER_OK and each goes back
  as soon as its frame is copied out; a packet spread over several buffers
  (MRG_RXBUF) is gathered, and held until all its buffers are back.
- Transmit completions are reclaimed when the next frame is sent; transmit
  interrupts are asked for only while every transmit buffer is in flight.
- Everything from the device is checked; a broken queue protocol resets the
  device and the driver refuses all work after.

The kernel side (`kernel/src/net/virtio.rs`) is discovery, mapping, a reset
before bus mastering, 128 + 128 buffers from `DmaBuffer`s, and MSI-X entry
0 for both queues and configuration changes. Without MSI-X the device is
polled every 10 ms.

**One interface with one stack behind a sleeping `Mutex`.** The stack and
its device live together in `net::Interface`, behind `sync::Mutex`, not a
spinlock: a poll may handle hundreds of frames and allocate, and nothing
that runs in interrupt context needs the stack. Without a virtio-net device
the stack runs on a stand-in device that drops everything, so loopback
works in every boot and the system calls always have a stack.

**A network thread drives the stack.** The MSI-X handler only bumps a
counter and wakes the thread. The thread polls, re-arms the device's
interrupts (and polls again if work arrived in between, which raises no
interrupt), and sleeps until the counter moves or the deadline `poll`
returned (`WaitQueue::wait_until_deadline`). Socket calls (`net::call`) run
the operation and then `poll` at once, so data leaves without waiting for
the thread, and kick the thread only when the stack's deadline is now
earlier than the one it sleeps towards.

**Blocking through a readiness generation.** One counter, `CHANGED`, moves
whenever socket readiness may have changed: when a poll moved packets
(device frames or packets through the stack's internal loopback, counted by
a new `Stats::loop_packets`), ran a due timer, or saw the link change, and
on `shutdown` / `close`. It moves while the interface is locked. A blocking
call tries its operation and, on `EAGAIN`, sleeps on one wait queue until
the counter differs from the value read (under the same lock) right after
its attempt (`call_and_generation`, `wait_for_change`), or its deadline. No
change after an attempt can be missed, and a poll that moved nothing (a
socket call that only checked readiness, say) wakes nobody, so two waiting
threads cannot wake each other forever. The same pair serves `poll(2)`
over many sockets later: check all, wait for a change from the generation
the check returned. `net::socket::Socket` wraps a handle with blocking
calls and optional deadlines, and closes it when dropped.

**Configuration from the command line.** `poly.net=<addr>/<prefix>[,gw=<addr>][,dns=<addr>]`,
`poly.net=qemu` (QEMU's documented user-networking addresses) or
`poly.net=off`, parsed by `poly_net::setting` (host-tested). No DHCP yet.

**Randomness**: the stack's secret and the fallback MAC address come from
`arch::user::random_bytes` (RDRAND when present, else the TSC through a
mixer), as `AT_RANDOM` does.

**Tests against real traffic.** Besides the host tests (the driver against
a simulated device written from the specification's device requirements,
including a `poly-net` stack running ARP, ping and TCP over it), the kernel
tests run TCP, UDP and ping over loopback in every boot, and with
`poly.test.net=<port>` exchange real traffic with QEMU's user networking:
ping to the virtual router 10.0.2.2 (QEMU's documentation says it answers),
1 MiB through a TCP echo server and datagrams through a UDP echo server
that `cargo xtask` runs on the host (reached through 10.0.2.2), and a
connection from the host through a port forward. One boot is
interrupt-driven, one polled; a third types `ifconfig`, `ping` and `nc`
into the shell.

## Consequences

- Networking works in the kernel and from the shell; the socket system
  calls are a translation layer over `net::socket` and `net::call`.
- Frames are copied once more in the driver (device buffer to stack buffer
  and back), on top of the stack's own copies. Lending driver buffers to
  the stack can come later behind the same trait.
- Every readiness change wakes every blocked socket caller, which then
  checks its own socket: fine for the few sockets a kernel test or shell
  has; per-socket wait queues (the stack would report which sockets
  changed) are due once many threads block on sockets.
- Blocking waits cannot be interrupted yet: signals (`EINTR`) need a wait a
  signal can end, which the socket system calls will add. `SIGPIPE` is the
  personality's (the stack returns `EPIPE`).
- The stack holds back loopback datagrams while the device cannot transmit
  (its output loop checks the device first), so a full transmit queue can
  delay local UDP; harmless with 128 transmit buffers, worth fixing in
  `poly-net`.
- One device, one queue pair, one CPU: more devices mean more stacks (ADR
  0028); SMP needs a destination policy for MSI-X and possibly per-CPU
  queues (VIRTIO_NET_F_MQ).
- No IPv4 address without `poly.net=`: DHCP is future work.
