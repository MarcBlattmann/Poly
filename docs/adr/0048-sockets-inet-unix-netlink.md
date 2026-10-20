# 0048: Sockets: socket files over the stack, Unix sockets in a crate, netlink in the personality, per-socket wake-ups

**Status:** Accepted. Builds on [ADR 0028](0028-tcp-ip-stack-in-a-crate.md)
and [0039](0039-networking-in-the-kernel.md) (the stack in the kernel),
[0042](0042-readiness-pipes-poll-epoll-and-event-files.md) (file objects,
wait queues, `poll` / `epoll`), [0025](0025-linux-syscall-table-and-kernel-services-trait.md),
[0031](0031-linux-signals-and-futexes.md) (restart codes, `SIGPIPE`),
[0041](0041-processes-threads-fork-exec-wait.md) (fork shares open files)
and [0043](0043-dev-proc-sys-and-file-system-calls.md) (the file system
calls, the services-trait pattern).

## Context

Phase 4 lists "Sockets: `AF_INET`, `AF_INET6`, `AF_UNIX` (incl. fd
passing), `AF_NETLINK` basics". The stack (`poly-net`) ran in the kernel
behind `net::call`, with blocking only for the kernel's own sockets
(one readiness generation that woke every waiter); file objects, wait
queues and `poll` / `epoll` existed for pipes and event files; the
personality had signals and restart codes. Sockets need all of it: a
descriptor whose readiness `poll` and `epoll` see, blocking calls a
signal can end, timeouts, `SIGPIPE`, Unix sockets with names in the file
system and descriptor passing, and enough `NETLINK_ROUTE` for `ip`,
`getifaddrs`, `if_nameindex` and `getaddrinfo`.

Sources: socket(2), socketpair(2), bind(2), listen(2), accept(2),
connect(2), getsockname(2), getpeername(2), send(2), recv(2), sendmmsg(2),
recvmmsg(2), shutdown(2), getsockopt(2), socket(7), ip(7), ipv6(7),
tcp(7), udp(7), unix(7), netlink(7), rtnetlink(7), cmsg(3), signal(7),
hosts(5), resolv.conf(5); layouts and constants from the public headers
`asm-generic/socket.h`, `linux/socket.h`, `linux/net.h`, `linux/in.h`,
`linux/in6.h`, `linux/un.h`, `linux/tcp.h`, `linux/sockios.h`,
`linux/netlink.h`, `linux/rtnetlink.h`, `linux/if_link.h`,
`linux/if_addr.h`, `linux/if.h`, `linux/if_arp.h` and the C library's
`sys/socket.h` for `MSG_*`. No Linux or C library source was read and
nothing was observed on a real system; points the pages leave open are
marked "to be confirmed by the diff runner" in the code.

## Decision

**Sockets are file objects.** Each socket descriptor is an `OpenFile` on
an anonymous `S_IFSOCK` inode whose object is the socket: `InetSocket`
(`kernel/src/net/file.rs`) for `AF_INET` / `AF_INET6`,
`poly_unix::UnixFile` for `AF_UNIX`, `NetlinkFile`
(`kernel/src/linux/net/netlink.rs`) for `AF_NETLINK`. Their operations
never block; each has a wait queue. So `read`, `write`, `readv`,
`writev`, `close`, `fstat`, `poll`, `select` and `epoll` work on sockets
unchanged, and `fork` shares them like any file.

**Per-socket wake-ups from the stack.** `poly-net` gained
`Stack::take_changed`: every open socket whose readiness or queued input
(bytes, datagrams, pending connections) differs from what the previous
call saw. The kernel calls it after every poll of the stack (the network
thread's and the one each socket call makes), with the interface
locked, and notifies the wait queues registered for those sockets
(`net::notify`) once the lock is released. A waiter registers on its
socket's queue and then tries again (ADR 0042's pattern), and every
attempt polls and collects under the lock, so no change is missed; a
change of one socket wakes only that socket's waiters. Counting queued
input as well as readiness flags makes edge-triggered `epoll` report
more data arriving on a socket that was readable already. The kernel's
own blocking sockets (`net::socket::Socket`, the shell and kernel tests)
keep ADR 0039's generation.

**Options the stack did not have** are small additions to `poly-net`:
per-socket TTL / hop limit and TOS (`IP_TTL`, `IPV6_UNICAST_HOPS`,
`IP_TOS`, applied to the packets the socket sends), keepalive timing
(`TCP_KEEPIDLE`, `TCP_KEEPINTVL`, `TCP_KEEPCNT`), queue sizes
(`FIONREAD` / `SIOCINQ`, `SIOCOUTQ`), the boolean options read back, and
`abort` (close with a reset, for `SO_LINGER` with a zero timeout). Options
the stack does not act on are kept by the socket object and reported
back: `SO_REUSEPORT` (accepted; ports are not shared among such
sockets), `SO_LINGER` with a positive time (close does not wait: the
connection finishes in the background), `SO_OOBINLINE`, `SO_PRIORITY`,
`SO_RCVLOWAT` (readiness ignores it), `TCP_MAXSEG`, `IPV6_V6ONLY`
(IPv6 sockets carry only IPv6 whatever it says, ADR 0028).

**Unix sockets are a host-tested crate, `poly-unix`.** `UnixSocket` is
one socket of `SOCK_STREAM`, `SOCK_DGRAM` or `SOCK_SEQPACKET`; a
`Namespace` holds the abstract names and the sockets bound to paths,
keyed by the device and inode number of the socket inode `bind` makes
with `Vfs::mknod` (mode `0777` less the umask; `EADDRINUSE` if the path
exists). `connect` and `sendto` resolve a path through the VFS and need
write permission on the inode (unix(7)); a socket closes its name, not
the inode (unlinking is the program's job). Every send queues one packet
at the receiver: bytes, the sender's address, credentials (explicit
`SCM_CREDENTIALS`, checked by the personality, or the sender's), and the
open files of `SCM_RIGHTS` (references to open file descriptions).
Datagram and sequenced-packet receives take one packet (truncating);
stream receives cross packets but stop after one that carried
descriptors, the barrier unix(7) describes, and before one whose
credentials differ while `SO_PASSCRED` is on. `listen` queues server
ends that `connect` creates; a full backlog makes `connect` wait
(`EAGAIN` non-blocking), and `accept` wakes the waiting connectors.
`socketpair`, autobind (a NUL and five hexadecimal digits),
`SO_PEERCRED`, `SO_PASSCRED`, shutdown in each direction, end of file,
`EPIPE`, `ECONNRESET` (a socket closed with unread data resets its peer,
after the peer read what was queued), `ECONNREFUSED`, `EPROTOTYPE` and
`EPERM` (a connected datagram socket takes datagrams from its peer only)
are all there, with host tests on a real VFS over tmpfs.

**Descriptors in flight.** An open file in a queue is counted in flight
until it is received (and installed as a descriptor, or dropped if the
receiver's control buffer or descriptor limit has no room: `MSG_CTRUNC`)
or the queue is dropped: when its socket closes, the files go with it.
Closing is the last close of the socket's open file description
(`UnixFile`'s `Drop`). Sockets that hold each other (a socket sent over
itself, two sockets in each other's queues) never get that last close;
`Namespace::collect` finds them: a Unix socket's open file whose every
reference is a queue entry is a candidate, candidates reachable from a
queue a process can still receive from are kept, the queues of the rest
are purged. The kernel runs it after every `close` while anything is in
flight (`linux::net::after_close`); references held during the scan only
make files look reachable.

**Netlink is the personality's protocol over the kernel's facts.**
`poly_personality_linux::net::netlink` parses requests and builds the
`RTM_NEWLINK`, `RTM_NEWADDR` and `RTM_NEWROUTE` dumps (with
`NLM_F_MULTI` and `NLMSG_DONE`), single-link answers, acknowledgements and
`NLMSG_ERROR` replies (`EOPNOTSUPP` for anything that would change
state: it is read-only), packing replies into datagrams of at most 8 KiB.
The kernel's `NetlinkFile` gives it an `Interfaces` snapshot (`lo` with
127.0.0.1/8 and ::1, `eth0` when there is a device with its MAC, MTU,
carrier, counters, IPv4 address, link-local IPv6 address and gateways);
routes are derived from those. Port ids follow netlink(7): the process's
pid for its first socket, unique numbers after. Multicast groups can be
joined; no notifications are sent (nothing changes at run time).

**The personality owns the calls and the rules.** `net::LinuxNet`
(reached with `LinuxKernel::net()`, `None` by default: `ENOSYS`) has the
socket operations on decoded types and kernel buffers. `sys/socket.rs`
(registered as its own group, `socket::CALLS`) checks
`(domain, type, protocol)` and flags, decodes and encodes `sockaddr_in`,
`sockaddr_in6`, `sockaddr_un` (paths to the first NUL, abstract names by
length, the lengths unix(7) gives back) and `sockaddr_nl`, `msghdr`,
`mmsghdr`, control messages (`CMSG_*` layout from cmsg(3)), option values
(`int`, `struct timeval`, `struct linger`, `struct ucred`; `SO_SNDBUF` /
`SO_RCVBUF` doubled as socket(7) says), moves data between iovecs and
kernel buffers (streams in 64 KiB chunks, the control data with the
first), and decides:

- a blocking call a signal ended restarts under `SA_RESTART` unless the
  socket has a timeout in that direction (`SO_RCVTIMEO` for `accept` and
  receives, `SO_SNDTIMEO` for `connect` and sends): then `EINTR`
  (signal(7)); `read` / `write` on a socket with a timeout follow the
  same rule;
- `EPIPE` from a send raises `SIGPIPE` unless `MSG_NOSIGNAL`;
- an interrupted blocking `connect` that is restarted goes on waiting
  (the kernel remembers it was connecting) instead of failing with
  `EALREADY`; one whose `SO_SNDTIMEO` ran out reports `EINPROGRESS`.

Calls: `socket`, `socketpair`, `bind`, `listen`, `accept`, `accept4`,
`connect`, `getsockname`, `getpeername`, `sendto`, `recvfrom`, `sendmsg`,
`recvmsg`, `sendmmsg`, `recvmmsg`, `shutdown`, `setsockopt`,
`getsockopt`, and `ioctl` `FIONREAD` / `SIOCINQ` / `SIOCOUTQ`.

**Blocking and timeouts in the kernel.** `linux/net.rs` implements
`LinuxNet` for the running thread: it finds the socket object behind a
descriptor (`ENOTSOCK` otherwise), and every call that may wait uses
`poly_vfs::poll::wait_until_done` on the socket's queue with the
thread's `ThreadBlocker`, a deadline from the socket's timeout, or no
wait for `O_NONBLOCK` / `MSG_DONTWAIT`. `FileObject` gained
`timeouts()`, and `OpenFile` `read_wait_until` / `write_wait_until`, so
`read` and `write` honour `SO_RCVTIMEO` / `SO_SNDTIMEO` too.

**Name resolution is user space's.** The initramfs has `/etc/hosts` and
`/etc/resolv.conf` naming QEMU's user-networking DNS server 10.0.2.3.

## Consequences

- Host tests: `poly-unix` (streams, datagrams, sequenced packets, names
  in both namespaces with permission checks, backlog waits, barriers,
  credentials, descriptors released with queues, cycles collected),
  `poly-net` (change reports, queue sizes, TTL / TOS on the wire,
  keepalive timing, resets), the personality (every call against a fake
  `LinuxNet`: argument rules, address and message encodings, control
  messages, options, chunked sends, `SIGPIPE`, restarts; the netlink
  dumps). Kernel tests: per-socket wake-ups; `/bin/socket-test` from ring
  3 (TCP between two processes with `epoll` and `poll`, UDP, Unix sockets
  with a path, an abstract name, pairs and `SCM_RIGHTS`, a blocked
  `accept` interrupted and restarted, `SIGPIPE` and `MSG_NOSIGNAL`,
  netlink). Boot tests: `/bin/socket-test` as the first process, and on
  virtio-net against the host's TCP and UDP echo servers.
- Not done yet: `SO_REUSEPORT` port sharing, a lingering close, urgent
  data (`MSG_OOB`), `SO_RCVLOWAT` in readiness, error queues, socket
  timestamps, `SO_BINDTODEVICE`, IPv4-mapped IPv6 (dual-stack) sockets,
  raw and packet sockets, netlink notifications and changes, other
  netlink families, `SCM_SECURITY`, `/proc/net`. A datagram written with
  `write(2)` above 64 KiB is split by the personality's bounce buffers
  (`sendto` and `sendmsg` send it whole). `read` / `write` on a Unix
  socket use the credentials the socket was created with.
- Descriptors in flight that only unreachable cycles hold are freed at
  the next `close` of any descriptor, not at a process's exit.
- `Stack::take_changed` costs one look at every socket per poll while any
  socket file exists; a change list kept by the stack itself can replace
  it once there are many sockets.
