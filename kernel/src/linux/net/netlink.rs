//! `AF_NETLINK` sockets (ADR 0048): [`NetlinkFile`] is the open file of
//! a `NETLINK_ROUTE` socket. The protocol (parsing requests, building the
//! link, address and route dumps) is the personality's
//! (`poly_personality_linux::net::netlink`); this file gives it the
//! interface's facts ([`interfaces`]), port ids (netlink(7): the first
//! socket of a process gets its pid, later ones unique numbers) and a wait
//! queue that is notified when replies are queued.

use super::{NO_CREDS, bool_opt, generic_get, generic_set};
use crate::net;
use crate::sync::SpinLock;
use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use core::sync::atomic::{AtomicU32, Ordering};
use poly_net::NetDevice;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::net::netlink::{IfAddr, Interfaces, Link, LinkStats, NetlinkSocket};
use poly_personality_linux::net::{OptName, OptValue, SockAddr, SockOpt, SockType};
use poly_unix::{DEFAULT_BUFFER, SocketOptions};
use poly_vfs::{FileObject, PollEvents, WaitQueue};

/// Port ids in use.
static PORTS: SpinLock<BTreeSet<u32>> = SpinLock::new(BTreeSet::new());
/// The next port id handed out when a process's pid is taken.
static NEXT_PORT: AtomicU32 = AtomicU32::new(0xffff_f000);

/// Interface index of the loopback device and of `eth0`.
pub const LO_INDEX: u32 = 1;
pub const ETH_INDEX: u32 = 2;

#[derive(Debug)]
struct Options {
    generic: SocketOptions,
    rcv_timeout: u64,
    snd_timeout: u64,
    sndbuf: usize,
    rcvbuf: usize,
}

/// A `NETLINK_ROUTE` socket.
#[derive(Debug)]
pub struct NetlinkFile {
    ty: SockType,
    owner: u32,
    proto: SpinLock<NetlinkSocket>,
    options: SpinLock<Options>,
    queue: Arc<WaitQueue>,
}

/// The interface's facts, as `NETLINK_ROUTE` reports them: `lo` always
/// (the stack's internal loopback), `eth0` when there is a network device.
pub fn interfaces() -> Interfaces {
    let mut ifs = Interfaces::default();
    let has_device = net::info().is_some_and(|i| i.bdf.is_some());
    let _ = net::with_interface(|iface| {
        let stats = iface.stack.stats();
        ifs.links.push(Link {
            index: LO_INDEX,
            name: b"lo".to_vec(),
            loopback: true,
            mac: [0; 6],
            mtu: 65536,
            up: true,
            carrier: true,
            stats: LinkStats { rx_packets: stats.loop_packets, tx_packets: stats.loop_packets, ..LinkStats::default() },
        });
        ifs.addrs.push(IfAddr { index: LO_INDEX, addr: IpAddr::V4(Ipv4Addr::LOCALHOST), prefix_len: 8 });
        ifs.addrs.push(IfAddr { index: LO_INDEX, addr: IpAddr::V6(Ipv6Addr::LOCALHOST), prefix_len: 128 });
        if has_device {
            ifs.links.push(Link {
                index: ETH_INDEX,
                name: net::NAME.as_bytes().to_vec(),
                loopback: false,
                mac: iface.dev.mac().0,
                mtu: iface.dev.mtu() as u32,
                up: true,
                carrier: iface.dev.link_up(),
                stats: LinkStats {
                    rx_packets: stats.rx_frames,
                    tx_packets: stats.tx_frames,
                    tx_dropped: stats.tx_dropped,
                    ..LinkStats::default()
                },
            });
            let cfg = iface.stack.config();
            if let Some(c) = cfg.ipv4 {
                ifs.addrs.push(IfAddr { index: ETH_INDEX, addr: IpAddr::V4(c.addr), prefix_len: c.prefix_len });
            }
            if let Some(ll) = iface.stack.ipv6_link_local() {
                ifs.addrs.push(IfAddr { index: ETH_INDEX, addr: IpAddr::V6(ll), prefix_len: 64 });
            }
            if let Some(g) = cfg.ipv6_global {
                ifs.addrs.push(IfAddr { index: ETH_INDEX, addr: IpAddr::V6(g.addr), prefix_len: g.prefix_len });
            }
            if let Some(gw) = cfg.gateway {
                ifs.gateways.push((IpAddr::V4(gw), ETH_INDEX));
            }
            if let Some(gw) = cfg.gateway6 {
                ifs.gateways.push((IpAddr::V6(gw), ETH_INDEX));
            }
        }
    });
    ifs
}

impl NetlinkFile {
    /// A new socket of process `owner`.
    pub fn new(ty: SockType, owner: u32) -> Arc<NetlinkFile> {
        Arc::new(NetlinkFile {
            ty,
            owner,
            proto: SpinLock::new(NetlinkSocket::new()),
            options: SpinLock::new(Options {
                generic: SocketOptions { rcv_lowat: 1, ..SocketOptions::default() },
                rcv_timeout: 0,
                snd_timeout: 0,
                sndbuf: DEFAULT_BUFFER,
                rcvbuf: DEFAULT_BUFFER,
            }),
            queue: Arc::new(WaitQueue::new()),
        })
    }

    pub fn queue(&self) -> &Arc<WaitQueue> {
        &self.queue
    }

    /// `(SO_RCVTIMEO, SO_SNDTIMEO)` in nanoseconds; 0: none.
    pub fn timeouts(&self) -> (u64, u64) {
        let o = self.options.lock();
        (o.rcv_timeout, o.snd_timeout)
    }

    /// A port id for this socket: the owner's pid if free, else a fresh
    /// one. Called with the protocol state locked.
    fn autobind(&self, nl: &mut NetlinkSocket) {
        if nl.port != 0 {
            return;
        }
        let mut ports = PORTS.lock();
        let port = if ports.contains(&self.owner) || self.owner == 0 {
            loop {
                let p = NEXT_PORT.fetch_sub(1, Ordering::Relaxed);
                if p != 0 && !ports.contains(&p) {
                    break p;
                }
            }
        } else {
            self.owner
        };
        ports.insert(port);
        nl.port = port;
    }

    /// bind(2): `pid` 0 lets the kernel choose; a taken pid is
    /// `EADDRINUSE`, rebinding to another pid `EINVAL`.
    pub fn bind(&self, pid: u32, groups: u32) -> Result<(), Errno> {
        let mut nl = self.proto.lock();
        if pid == 0 {
            self.autobind(&mut nl);
        } else if nl.port != pid {
            if nl.port != 0 {
                return Err(Errno::EINVAL);
            }
            let mut ports = PORTS.lock();
            if !ports.insert(pid) {
                return Err(Errno::EADDRINUSE);
            }
            nl.port = pid;
        }
        nl.groups = groups;
        Ok(())
    }

    /// getsockname(2): the port id (0 until bound) and groups.
    pub fn local(&self) -> SockAddr {
        let nl = self.proto.lock();
        SockAddr::Netlink { pid: nl.port, groups: nl.groups }
    }

    /// A datagram to the kernel: answer it and wake readers.
    pub fn request(&self, data: &[u8]) {
        // The facts first: the interface sits behind a sleeping mutex.
        let ifs = interfaces();
        {
            let mut nl = self.proto.lock();
            self.autobind(&mut nl);
            nl.request(data, &ifs);
        }
        self.queue.notify(PollEvents::READABLE);
    }

    /// One reply datagram: `(copied, its length)`; `EAGAIN` when none.
    pub fn read(&self, buf: &mut [u8], peek: bool) -> Result<(usize, usize), poly_vfs::Errno> {
        self.proto.lock().read(buf, peek).map_err(|_| poly_vfs::Errno::EAGAIN)
    }

    pub fn setsockopt(&self, opt: SockOpt) -> Result<(), Errno> {
        let bit = |g: u32| if (1..=32).contains(&g) { Ok(1u32 << (g - 1)) } else { Err(Errno::EINVAL) };
        match opt {
            SockOpt::NetlinkAddMembership(g) => self.proto.lock().groups |= bit(g)?,
            SockOpt::NetlinkDropMembership(g) => self.proto.lock().groups &= !bit(g)?,
            SockOpt::RcvTimeout(t) => self.options.lock().rcv_timeout = t,
            SockOpt::SndTimeout(t) => self.options.lock().snd_timeout = t,
            SockOpt::SndBuf(v) => self.options.lock().sndbuf = v,
            SockOpt::RcvBuf(v) => self.options.lock().rcvbuf = v,
            SockOpt::PassCred(_) => {}
            o => {
                if !generic_set(&mut self.options.lock().generic, o) {
                    return Err(Errno::ENOPROTOOPT);
                }
            }
        }
        Ok(())
    }

    pub fn getsockopt(&self, name: OptName) -> Result<OptValue, Errno> {
        let o = self.options.lock();
        Ok(match name {
            OptName::Type => OptValue::Int(self.ty.to_linux() as i32),
            OptName::Domain => OptValue::Int(16),
            OptName::Protocol => OptValue::Int(0),
            OptName::Error | OptName::AcceptConn => OptValue::Int(0),
            OptName::PeerCred => OptValue::Ucred(NO_CREDS),
            OptName::PassCred => bool_opt(false),
            OptName::SndBuf => OptValue::Int(o.sndbuf as i32),
            OptName::RcvBuf => OptValue::Int(o.rcvbuf as i32),
            OptName::RcvTimeout => OptValue::Timeval(o.rcv_timeout),
            OptName::SndTimeout => OptValue::Timeval(o.snd_timeout),
            n => generic_get(&o.generic, n).ok_or(Errno::ENOPROTOOPT)?,
        })
    }
}

impl Drop for NetlinkFile {
    fn drop(&mut self) {
        let port = self.proto.lock().port;
        if port != 0 {
            PORTS.lock().remove(&port);
        }
    }
}

impl FileObject for NetlinkFile {
    fn read(&self, buf: &mut [u8]) -> Result<usize, poly_vfs::Errno> {
        NetlinkFile::read(self, buf, false).map(|(n, _)| n)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, poly_vfs::Errno> {
        self.request(buf);
        Ok(buf.len())
    }

    fn poll(&self) -> PollEvents {
        let ready = self.proto.lock().next_len().is_some();
        if ready { PollEvents::READABLE | PollEvents::WRITABLE } else { PollEvents::WRITABLE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn bytes_readable(&self) -> Option<usize> {
        Some(self.proto.lock().next_len().unwrap_or(0))
    }

    fn timeouts(&self) -> (Option<u64>, Option<u64>) {
        let (r, w) = NetlinkFile::timeouts(self);
        ((r != 0).then_some(r), (w != 0).then_some(w))
    }
}
