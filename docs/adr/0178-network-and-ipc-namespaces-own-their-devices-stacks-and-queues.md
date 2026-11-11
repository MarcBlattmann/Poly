# 0178: Network and IPC namespaces own their devices, stacks and queues

**Status:** Accepted. Roadmap Phase 8, "What Linux package formats need from
the kernel" ([ADR 0132](0132-installing-linux-apps-of-every-package-format.md)).
It continues [ADR 0168](0168-kernel-interfaces-for-linux-package-formats.md)
with the three package-format entries that needed namespaces Poly only had
by name: `netns_loopback`, the IPC half of `ns_uts_ipc`, and `sandbox_flow`
(the Chrome-style sandbox). It builds on
[ADR 0054](0054-namespaces-and-seccomp.md) (which namespace a process is in, `unshare`,
`setns`, `/proc/<pid>/ns`), [ADR 0048](0048-sockets-inet-unix-netlink.md) (sockets) and
[ADR 0028](0028-tcp-ip-stack-in-a-crate.md) (the stack never blocks).

Inputs: the spec sheets `specs/linux/pkg/{netns_loopback,ns_uts_ipc,
sandbox_flow}.yaml` and their visible tests, the manual pages
(network_namespaces(7), ipc_namespaces(7), netdevice(7), rtnetlink(7),
netlink(7), raw(7), packet(7), unix(7), mq_overview(7), proc(5) and its
sub-pages) and the constants of the UAPI headers the sheets cite. No Linux
source.

## What was there

`unshare(CLONE_NEWNET)` and `CLONE_NEWIPC` made objects with an inode
number and an owner. The IPC one held the System V tables. The network one
held nothing: every socket of every namespace was in the kernel's single
`poly-net` stack, so a sandbox's "no network" namespace could connect to
anything the host could, a new namespace's loopback was up, and abstract
Unix socket names were shared by the whole system. POSIX message queue names
were kept per namespace in a side table that never shrank. There were no
interface ioctls, no changes over rtnetlink, no raw sockets, no
`/proc/net`, no `/proc/sysvipc`, no IPC settings in `/proc/sys`, and no
message queue file system.

## Decisions

### 1. A network namespace owns its devices and the kernel's stack for them

`ns::NetNs` (personality) holds two things besides its identity:

- **Its devices**, `net::dev::Devices`: the state of `lo` (up or down, has
  127.0.0.1/8 or not). A new namespace's is down without an address; coming
  up gives it 127.0.0.1/8, which it keeps when it goes down again, and
  ::1/128 for as long as it is up. The initial namespace has one too
  (a static, up with both addresses); the kernel adds `eth0` to what it
  reports there. This is plain data with unit tests.
- **The kernel's state**, an opaque `Arc<dyn Any>` made at first use
  (`NetNs::kernel_state`): the kernel puts an `NsStack` there, a `poly-net`
  stack with no device and no addresses, whose loopback follows the
  namespace's `lo` (`Stack::set_loopback`, new: while it is down, nothing is
  routed to 127.0.0.0/8 or ::1 and a connect or send there is
  `ENETUNREACH`; binding still works, as the sheet records).

The initial namespace keeps the one stack on the network device. Nothing
is paid by a system that never makes a namespace, and a namespace that
never opens an Internet socket never gets a stack.

Why a stack per namespace and not a namespace tag on every socket of one
stack: ports, the connection table, the loopback queue and the route
lookup would all have to learn about the tag, in a crate that is otherwise
a plain TCP/IP stack; a second `Stack` gives the same isolation with no
change to any of them, and its cost is a few kilobytes.

The network thread polls every namespace stack after the interface's and
sleeps until the earliest deadline of all of them; a socket call on a
namespace stack polls it at once, as on the interface's
(`net::call_in`). Per-socket readiness notifications are filed under the
stack's number and the socket's handle (handles are per stack).

### 2. A socket belongs to the namespace it was made in

network_namespaces(7). `socket(2)` takes the calling process's network
namespace:

| Socket | What it keeps |
|---|---|
| `AF_INET`, `AF_INET6` | its stack, and the namespace (kept alive by the socket, as the sheet's side effects say) |
| `AF_UNIX` | a scope number for abstract names (the namespace's inode number, 0 for the initial one): `poly-unix` keys abstract names by scope, while path names are found through the file system by whoever reaches the path, and socket pairs and descriptors in flight cross namespaces |
| `AF_NETLINK` | the namespace: its dumps and changes are about that namespace's devices |
| raw | the namespace |

`setns` or `unshare` later does not move a socket: the interface ioctls on
an old socket still answer for its own namespace (`LinuxNet::netns`).

### 3. Devices are read and changed in one place, by three front ends

`net::netlink::Interfaces` (links, addresses, gateways) is the one
description of a namespace's devices. `LinuxNet::interfaces(ns)` makes it
(loopback from the namespace's `Devices` with the stack's loopback
counters, `eth0` from the driver in the initial namespace) and
`LinuxNet::set_link_up(ns, index, up)` is the one change. On top:

- **ioctl(2)** (`sys::netdev`, `net::dev::ifreq` / `ifconf`):
  `SIOCGIFNAME`, `SIOCGIFINDEX`, `SIOCGIFFLAGS`, `SIOCGIFMTU`,
  `SIOCGIFHWADDR`, `SIOCGIFADDR`, `SIOCGIFNETMASK`, `SIOCGIFBRDADDR`,
  `SIOCGIFDSTADDR`, `SIOCGIFMETRIC`, `SIOCGIFTXQLEN`, `SIOCGIFCONF` and
  `SIOCSIFFLAGS` (the `IFF_UP` bit decides; the other bits are the
  device's own). The other setters are `EOPNOTSUPP`.
- **rtnetlink**: besides the dumps, `RTM_NEWLINK` / `RTM_SETLINK` change
  `IFF_UP` of the link named by index or `IFLA_IFNAME`, and `RTM_NEWADDR`
  of 127.0.0.1/8 on the loopback device is accepted (sandboxes add the
  address before they bring the link up; the sheet records only the link
  change, the address request is from rtnetlink(7)). Other changes are
  `EOPNOTSUPP`.
- **procfs**: `/proc/<pid>/net/dev` and `/proc/<pid>/net/route` for the
  namespace of process `<pid>`; `/proc/net` is a link to `self/net`.

`eth0` cannot be taken down (`EOPNOTSUPP`); `lo` of the initial namespace
can, by root, like any other.

### 4. Privilege is asked of the user namespace that owns the namespace

The sheets' ownership rules, all through `ns::user::ns_capable` over the
namespace's owner:

- changing a device (ioctl setters, rtnetlink changes): `CAP_NET_ADMIN`,
  checked before the device is looked up (`EPERM`); getters and dumps are
  for anybody. For rtnetlink the sender's credentials at the time of the
  send count, not the socket's creator's.
- raw sockets (`SOCK_RAW` of `AF_INET` / `AF_INET6`, any `AF_PACKET`
  socket): `CAP_NET_RAW` (`EPERM`).
- mounting `mqueue` needs `CAP_SYS_ADMIN` over the caller's IPC namespace
  and mounting `sysfs` over its network namespace
  (`sys::admin::require_fs_owner`), besides the mount namespace's own
  check.

So `unshare(CLONE_NEWUSER)` alone gives none of it (the namespaces still
belong to the initial user namespace), and `unshare(CLONE_NEWUSER |
CLONE_NEWNET | CLONE_NEWIPC)` gives all of it.

### 5. Raw sockets can be made, and nothing more

Sandboxes and their tests make one to see whether they hold `CAP_NET_RAW`.
A raw socket is a socket file that can be made, bound, given options and
closed; a send finds no route (`ENETUNREACH`) and a receive waits for
packets that never come. Carrying packets through them is a later decision
(no test and no app here needs it).

### 6. An IPC namespace owns its queues and its limits

`ns::IpcNs` now also holds the POSIX message queue names (the side table is
gone: a namespace's queues go with it). Each of its four tables carries its
limits (`MsgLimits`, `ShmLimits`, `SemLimits`, `MqLimits`), which the table's
own checks use, so a limit set in a namespace applies there and nowhere
else. `ipc::files` makes the texts and takes the writes of:

- `/proc/sysvipc/{msg,shm,sem}`: a header line and a line per object;
- `/proc/sys/kernel/{msgmni,msgmax,msgmnb,shmmax,shmall,shmmni,sem}` and
  `/proc/sys/fs/mqueue/{queues_max,msg_max,msgsize_max,msg_default,
  msgsize_default}`, with the ranges of proc_sys_kernel(5) and
  mq_overview(7).

All of them answer for the IPC namespace of the process that reads or
writes (`ProcSource::ipc_file`, `write_ipc_file`).

### 7. The message queue file system belongs to its mounter's namespace

`poly_procfs::MqueueFs` is a synthetic file system over a `MqueueSource`:
a sticky root and one regular file per queue (the queue's mode and owner,
size in bytes of the queued messages, the status line as contents; removing
the file removes the name, creating a file makes a queue with the
namespace's defaults). `mount -t mqueue` makes one over the IPC namespace of
the mounting process and it keeps that namespace for as long as it is
mounted, so a mount made before `unshare(CLONE_NEWIPC)` goes on showing the
old namespace's queues and one made after shows the new one's, as the sheet
records. `/dev/mqueue`, a tmpfs with the right name until now, is the
initial namespace's.

## What it costs

- A stack per network namespace that opens an Internet socket (a few
  buffers of one frame each), polled by the network thread; none
  otherwise.
- `LinuxNet` grew three methods with defaults; `NetlinkSocket::request`
  takes the devices as a `LinkEnv` instead of a snapshot (the kernel
  answers on a copy of the socket, outside its lock, because the devices
  sit behind a sleeping mutex).
- `ProcSource` grew `net_file`, `ipc_file` and `write_ipc_file`.

## Not done

- Devices other than `lo` in a namespace (veth pairs, moving `eth0`): a
  namespace cannot reach the outside. Sandboxes want exactly that; containers
  with their own network will need it.
- Multicast notifications for link changes (`RTMGRP_LINK`).
- The ids in `/proc/sysvipc` are the kernel's, not translated to the
  reader's user namespace.
- A file of the message queue file system that is open while its queue's
  name is removed reads `ENOENT` afterwards (the file system keeps no
  reference of its own to an unlinked queue).

## Tests

Host: `poly-net` (loopback down and up), `poly-unix` (abstract names per
scope), `poly-procfs` (`/proc/net`, the IPC files, the message queue file
system), the personality (`net::dev`, `net::netlink`, `ipc::files`, the
socket calls for raw sockets and interface ioctls). Kernel:
`a_new_network_namespace_has_its_own_loopback` and
`an_ipc_namespace_owns_its_queues_and_settings` in `linux/ns_tests.rs`. The
entries' visible tests run on Poly with `cargo xtask pipeline polyrun`.
