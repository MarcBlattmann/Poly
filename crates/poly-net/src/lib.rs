//! Poly's TCP/IP stack: Ethernet II, ARP, IPv4 (with fragment
//! reassembly), ICMP, UDP, TCP, and IPv6 basics (ICMPv6 echo, Neighbor
//! Discovery, link-local addressing), with a nonblocking socket API.
//!
//! Written from the IETF RFCs cited in each module; no other stack's code
//! was consulted (`CLEAN_ROOM.md`).
//!
//! # Shape
//!
//! - A driver implements [`NetDevice`]: send a frame, hand over received
//!   frames, report its MAC address and MTU. [`Loopback`] is one.
//! - A [`Stack`] owns everything else: addresses, neighbour caches,
//!   reassembly, sockets. It never blocks and has no clock of its own:
//!   [`Stack::poll`] takes the current time, does all due work and returns
//!   the next deadline.
//! - Sockets are handles into the stack. Calls mirror the BSD socket API
//!   (`socket`, `bind`, `listen`, `accept`, `connect`, `send`, `recv`,
//!   `shutdown`, `close`), are always nonblocking, and fail with Linux
//!   [`Errno`] values. [`Stack::readiness`] gives poll/epoll flags. A kernel
//!   implements blocking calls by waiting for readiness.
//!
//! # Memory
//!
//! The stack allocates (it is meant for the kernel heap): socket buffers
//! once per connection at their configured size, queued datagrams,
//! reassembly buffers and packets waiting for address resolution. Every one
//! of these is bounded by configuration ([`Config`]); nothing grows with
//! untrusted input beyond those limits. Frames move through four
//! interface-sized buffers allocated with the stack.
//!
//! # Policies worth knowing
//!
//! - **No outgoing fragmentation.** TCP sets DF and sizes segments from the
//!   MTU (and lowers them on ICMP "fragmentation needed", RFC 1191).
//!   Datagrams that would not fit the MTU fail with `EMSGSIZE`, like Linux
//!   with `IP_PMTUDISC_DO`. Incoming IPv4 fragments are reassembled
//!   ([`reassembly`]); IPv6 fragments are dropped.
//! - **IPv6 sockets are IPv6-only** (`IPV6_V6ONLY`); no SLAAC, no DAD, no
//!   router discovery: addresses and routers are configured.
//! - **One interface** plus internal loopback: packets to 127.0.0.0/8, ::1
//!   and the stack's own addresses never reach the device.
//! - **Not implemented:** SACK, ECN, urgent data, IP options (ignored on
//!   receive), multicast group membership beyond all-nodes, raw sockets,
//!   DHCP.

#![no_std]

extern crate alloc;

pub mod device;
pub mod errno;
pub mod neighbor;
pub mod reassembly;
mod siphash;
pub mod socket;
mod stack;
pub mod tcp;
pub mod time;
pub mod wire;

pub use device::{Loopback, NetDevice, TxError};
pub use errno::Errno;
pub use neighbor::NeighborConfig;
pub use reassembly::{ReassemblyConfig, ReassemblyStats};
pub use socket::{Domain, Protocol, Readiness, RecvFlags, RecvInfo, Shutdown, SockType, SocketHandle};
pub use stack::{Config, Ipv4Cidr, Ipv6Cidr, Stack, Stats, TcpInfo};
pub use tcp::{TcpConfig, TcpState, TcpStats};
pub use time::Instant;
pub use wire::ethernet::MacAddr;

#[cfg(test)]
mod sim;
#[cfg(test)]
mod tests;
