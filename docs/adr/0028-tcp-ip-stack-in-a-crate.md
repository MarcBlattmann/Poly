# 0028: Our own TCP/IP stack as a host-tested crate, driven by poll(now)

**Status:** Accepted

## Context

Phase 4 needs networking: a virtio-net driver and a TCP/IP stack (IPv4,
IPv6, TCP, UDP, ICMP) under the Linux socket calls (`AF_INET`,
`AF_INET6`), and later under the NT and XNU personalities. A TCP stack is
mostly protocol logic and timers, and its bugs show up only under loss,
reordering and odd timing, which booting QEMU cannot produce on demand.
Clean-room rules (`CLEAN_ROOM.md`) rule out porting an existing stack; the
IETF RFCs are public specifications and may be implemented directly.

## Decision

- **`crates/poly-net`** holds the whole stack: wire formats, ARP and NDP,
  IPv4 reassembly, ICMP, UDP, TCP and the socket layer. It is `no_std`,
  has no dependencies, and is written from the RFCs cited in each module.
- **Drivers plug in through `NetDevice`** (ADR 0009 style): MAC address,
  MTU, `transmit(frame)`, `receive(buf)`, `can_transmit()`. Frames are
  copied between the driver's memory and the stack's buffers, so the
  driver owns its DMA memory outright. `Loopback` is the in-crate device.
- **No clock, no threads.** `Stack::poll(dev, now)` receives, runs timers,
  sends, and returns the next deadline. Socket calls only queue work; the
  kernel calls `poll` after them, when the device interrupts, and at the
  deadline. Tests drive simulated time the same way.
- **The stack allocates** from the kernel heap, unlike the block layer:
  per-connection buffers are sized per socket (`SO_SNDBUF`/`SO_RCVBUF`,
  128 KiB by default) and allocated once when a connection starts;
  datagram queues, reassembly buffers and packets waiting for address
  resolution are `Vec`s. Every one of these is bounded by `Config`, so
  untrusted input cannot grow memory past the configured limits. Caller-
  provided tables (as in `poly-block`) would force fixed socket counts and
  buffer sizes at boot, which socket APIs do not have.
- **Socket API**: handles with generations (stale handles fail with
  `EBADF`), BSD-shaped calls that never block (`EAGAIN`, `EINPROGRESS`),
  readiness flags with a mapping to Linux `poll` bits, errors as Linux
  errno values from the UAPI headers. Blocking, signals and `SIGPIPE` are
  the personality's business.
- **One interface plus internal loopback.** Packets to 127.0.0.0/8, ::1 and
  the stack's own addresses are looped back inside the stack. Several
  interfaces mean several stacks for now.
- **Policies** where the RFCs leave room:
  - No outgoing fragmentation: TCP sets DF and lowers its MSS on ICMP
    "fragmentation needed" (RFC 1191); oversized datagrams fail with
    `EMSGSIZE`. Incoming IPv4 fragments are reassembled with limits on
    datagrams, bytes, pieces and time (30 s); a fragment that overlaps
    with different data poisons its datagram (RFC 1858, RFC 5722's rule
    applied to IPv4). IPv6 fragments are dropped.
  - IPv6 sockets are IPv6-only; IPv6 addresses and routers are configured
    (link-local from the MAC by EUI-64); no SLAAC, DAD or router discovery.
  - TCP follows RFC 9293 with the RFC 5961 defences, RFC 6298 RTO,
    RFC 5681 congestion control with RFC 6582 NewReno, RFC 7323 window
    scaling and timestamps (PAWS); no SACK, ECN or urgent data. The
    minimum RTO defaults to RFC 6298's 1 s. Initial sequence numbers follow
    RFC 6528 with SipHash-2-4 as the keyed function; the kernel must pass
    a random secret in `Config::secret`.
  - The listen backlog bounds connections waiting for `accept`, half-open
    ones included; SYNs beyond it are dropped and retried by the client.
  - Closing with unread data, or receiving data after closing, sends RST
    (RFC 1122 4.2.2.13). Orphaned connections finish on their own, with a
    60 s limit in FIN-WAIT-2.
  - ICMP errors are rate-limited (token bucket) and never sent about ICMP
    errors, non-initial fragments, or broadcast/multicast packets.
- **Testing** is a deterministic simulation: two stacks joined by a link
  with seeded loss, duplication, corruption, delay and jitter, simulated
  time jumping from deadline to deadline. Megabyte transfers must arrive
  byte-exact under all of these; fuzz tests throw random and mutated
  frames (with valid checksums, to reach the deeper parsers) at busy
  stacks and check they still work.

## Consequences

- The protocol is tested on the host in about a second, including hours of
  simulated time (keepalives, TIME-WAIT, retransmission give-up).
- The kernel side is a virtio-net driver implementing `NetDevice`, a lock
  around the `Stack`, a thread or timer that calls `poll` at its deadline,
  and wait queues woken when readiness changes. The Linux personality
  translates `sockaddr`s, flags and socket options to this API.
- Frames are copied up to four times on the way out. Zero-copy (driver
  buffers lent to the stack) can come later behind the same trait.
- Connection lookup is a linear scan over sockets; a hash index is due
  once there are more than a few hundred sockets.
