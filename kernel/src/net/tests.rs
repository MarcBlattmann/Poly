//! Kernel tests of networking. The loopback ones run in every boot (no
//! device needed) and exercise the blocking sockets, the wait queues and
//! the network thread. With `poly.test.net=<port>` the boot test has
//! attached a virtio-net device on QEMU's user networking and runs a TCP
//! and UDP echo server on the host's port `<port>` (reached at 10.0.2.2,
//! the host's address under QEMU's user networking), and a host forward to
//! guest port [`LISTEN_PORT`]: then the device must be there, and real
//! traffic must flow both ways.

use super::socket::{Pinger, Socket};
use super::*;
use crate::ktest::TestResult;
use crate::{kernel_test, kprintln, thread};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Set by the boot test: the host's echo server port (TCP and UDP).
pub const TEST_NET_FLAG: &str = "poly.test.net";
/// The guest port the host connects to through its forward. Must match
/// `GUEST_PORT` in `tools/xtask`.
pub const LISTEN_PORT: u16 = 7777;
/// What the kernel prints when it waits for the host's connection. Must
/// match `LISTENING` in `tools/xtask`.
pub const LISTENING: &str = "net: test: listening for the host on port";
/// Bytes the host sends to [`LISTEN_PORT`]. Must match `HOST_BYTES` in
/// `tools/xtask`.
pub const HOST_BYTES: usize = 100_000;
const GATEWAY: Ipv4Addr = Ipv4Addr::new(10, 0, 2, 2);
/// QEMU's default MAC address for the first NIC (QEMU documentation,
/// `-nic`: "52:54:00:12:34:56").
const QEMU_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// Byte `i` of the test streams. Must match `pattern` in `tools/xtask`.
pub fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8)) as u8
}

fn secs(n: u64) -> Option<u64> {
    Some(time::now_ns() + n * time::NANOS_PER_SEC)
}

fn test_port(env: &crate::ktest::TestEnv) -> Option<u16> {
    env.boot.arg(TEST_NET_FLAG).and_then(|p| p.parse().ok())
}

/// Thread body: accept one connection on the listener at `arg` (a leaked
/// `Box<Socket>`) and echo it until the peer closes. Exit code: bytes
/// echoed, or `usize::MAX` on an error.
fn echo_server(arg: usize) -> usize {
    // SAFETY: the test passes a pointer from `Box::into_raw` and never
    // touches it again.
    let listener = unsafe { Box::from_raw(arg as *mut Socket) };
    let Ok((conn, _)) = listener.accept(secs(20)) else { return usize::MAX };
    let mut buf = vec![0u8; 8192];
    let mut total = 0;
    loop {
        match conn.recv(&mut buf, secs(20)) {
            Ok(0) => return total,
            Ok(n) => {
                if conn.send_all(&buf[..n], secs(20)).is_err() {
                    return usize::MAX;
                }
                total += n;
            }
            Err(_) => return usize::MAX,
        }
    }
}

/// Send `len` pattern bytes over `conn` in `chunk`-sized pieces, reading
/// each piece's echo back before the next, and check it.
fn exchange(conn: &Socket, len: usize, chunk: usize) -> TestResult {
    let data: Vec<u8> = (0..len).map(pattern).collect();
    let mut back = vec![0u8; chunk];
    for piece in data.chunks(chunk) {
        conn.send_all(piece, secs(20)).map_err(|_| "send failed")?;
        let mut got = 0;
        while got < piece.len() {
            match conn.recv(&mut back[got..piece.len()], secs(20)) {
                Ok(0) => return Err("the echo ended early"),
                Ok(n) => got += n,
                Err(_) => return Err("receiving the echo failed or timed out"),
            }
        }
        if back[..piece.len()] != *piece {
            return Err("the echo differs from what was sent");
        }
    }
    Ok(())
}

kernel_test! {
    /// A TCP connection over loopback between two kernel threads, both
    /// blocking: the server sleeps in `accept` until the client connects,
    /// then echoes 256 KiB.
    fn loopback_tcp_echo_between_threads() -> TestResult {
        let port = 4000;
        let any = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        let listener = Socket::new(Domain::Inet, SockType::Stream, None).map_err(|_| "socket failed")?;
        // The shell's `test` may run this again while the last run's
        // connection is still closing.
        let _ = listener.with(|s, h| s.set_reuse_addr(h, true));
        listener.bind(any).map_err(|_| "bind failed")?;
        listener.listen(4).map_err(|_| "listen failed")?;
        let server = thread::spawn(echo_server, Box::into_raw(Box::new(listener)) as usize)
            .map_err(|_| "spawn failed")?;
        // The server must be asleep in accept, not spinning.
        let start = time::now_ns();
        while thread::state(server) != Some(thread::State::Blocked) {
            if time::now_ns() - start > 5 * time::NANOS_PER_SEC {
                return Err("the server thread did not block in accept");
            }
            thread::sleep(core::time::Duration::from_millis(1));
        }
        let client = Socket::new(Domain::Inet, SockType::Stream, None).map_err(|_| "socket failed")?;
        client.connect(any, secs(10)).map_err(|_| "connect over loopback failed")?;
        if client.peer_addr() != Ok(any) || client.local_addr().is_ok_and(|a| a.port() == 0) {
            return Err("the connected socket has the wrong addresses");
        }
        exchange(&client, 256 * 1024, 16 * 1024)?;
        let ready = client.wait(Readiness::WRITABLE, secs(5)).map_err(|_| "an idle connection is not writable")?;
        if ready != client.readiness().unwrap_or_default() || client.take_error() != Ok(None) {
            return Err("readiness or the pending error of an idle connection is off");
        }
        let h = client.handle();
        client.close().map_err(|_| "close failed")?;
        if call(|s| s.readiness(h)) != Err(Errno::EBADF) {
            return Err("a closed socket's handle still works");
        }
        match thread::join(server) {
            Ok(n) if n == 256 * 1024 => Ok(()),
            Ok(usize::MAX) => Err("the echo server failed"),
            _ => Err("the echo server echoed the wrong amount"),
        }
    }
}

kernel_test! {
    fn loopback_udp_and_ping() -> TestResult {
        let lo = |port| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
        let a = Socket::new(Domain::Inet, SockType::Datagram, None).map_err(|_| "socket failed")?;
        let b = Socket::new(Domain::Inet, SockType::Datagram, None).map_err(|_| "socket failed")?;
        a.bind(lo(4001)).map_err(|_| "bind failed")?;
        b.bind(lo(4002)).map_err(|_| "bind failed")?;
        for n in [1usize, 100, 1400] {
            let msg: Vec<u8> = (0..n).map(pattern).collect();
            a.send_to(&msg, Some(lo(4002)), secs(5)).map_err(|_| "sendto failed")?;
            let mut buf = vec![0u8; 2048];
            let info = b.recv_from(&mut buf, RecvFlags::default(), secs(5)).map_err(|e| {
                kprintln!("net: test: recvfrom of a {n}-byte datagram: {e}");
                "recvfrom failed"
            })?;
            if info.from != lo(4001) || buf[..info.len] != msg[..] {
                return Err("a UDP datagram over loopback came back wrong");
            }
        }
        let ping = Pinger::new(56).map_err(|_| "no ping socket")?;
        for seq in 1..=3 {
            let (_, len) = ping.echo(Ipv4Addr::LOCALHOST, seq, time::now_ns() + 5 * time::NANOS_PER_SEC)
                .map_err(|_| "no echo reply from 127.0.0.1")?;
            if len != 64 {
                return Err("the echo reply from 127.0.0.1 has the wrong length");
            }
        }
        Ok(())
    }
}

kernel_test! {
    /// A receive with a deadline sleeps until then and fails with EAGAIN.
    fn a_blocked_receive_times_out() -> TestResult {
        let s = Socket::new(Domain::Inet, SockType::Datagram, None).map_err(|_| "socket failed")?;
        s.bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4003)).map_err(|_| "bind failed")?;
        let start = time::now_ns();
        let limit = 100 * time::NANOS_PER_MILLI;
        let mut buf = [0u8; 16];
        let r = s.recv(&mut buf, Some(start + limit));
        let took = time::now_ns() - start;
        if r != Err(Errno::EAGAIN) {
            return Err("a receive with nothing to receive did not time out with EAGAIN");
        }
        if !(limit..limit + 5 * time::NANOS_PER_SEC).contains(&took) {
            return Err("the receive did not time out at its deadline");
        }
        Ok(())
    }
}

kernel_test! {
    /// Without the test flag, there may be no device at all: the interface
    /// then has none, and the outside world is unreachable at once.
    fn device_found_and_configured(env: &TestEnv) -> TestResult {
        let info = info().ok_or("net::init has not run")?;
        if info.thread.is_none() {
            return Err("no network thread");
        }
        let has_nic = crate::pci::devices().iter().any(virtio::is_virtio_net);
        if test_port(env).is_none() {
            if has_nic {
                return Ok(());
            }
            if info.bdf.is_some() || with_interface(|i| matches!(i.dev, Device::Virtio(_))) != Some(false) {
                return Err("an interface without a device claims one");
            }
            let s = Socket::new(Domain::Inet, SockType::Stream, None).map_err(|_| "socket failed")?;
            return match s.connect(SocketAddr::new(GATEWAY.into(), 80), secs(5)) {
                Err(Errno::ENETUNREACH) => Ok(()),
                _ => Err("without a device, connecting out did not fail with ENETUNREACH"),
            };
        }
        let bdf = info.bdf.ok_or("poly.test.net is set but no virtio-net device was brought up")?;
        let dev = crate::pci::devices().iter().find(|d| d.bdf() == bdf).ok_or("the device is not in the PCI list")?;
        if dev.header.device_id != 0x1041 {
            return Err("the test NIC is not a modern-only virtio-net device (PCI device 0x1041)");
        }
        let (mac, mtu, link, failed) = with_interface(|i| (i.dev.mac(), i.dev.mtu(), i.dev.link_up(), i.dev.failed()))
            .ok_or("no interface")?;
        if mac.0 != QEMU_MAC || mtu != 1500 || !link || failed {
            return Err("the NIC's MAC, MTU or link is not what QEMU sets up");
        }
        if info.ipv4 != Some(StaticIpv4::QEMU_USER) {
            return Err("the interface does not have QEMU's user networking address");
        }
        match dev.msix() {
            Some(Ok(m)) => {
                let vector = info.vector.ok_or("the NIC has MSI-X but is not interrupt-driven")?;
                let msix = info.msix.as_ref().ok_or("no MSI-X state kept")?;
                if !interrupt_driven() || msix.is_function_masked() || !m.is_enabled(dev.config(), bdf) {
                    return Err("MSI-X is off or masked although the NIC is interrupt-driven");
                }
                match msix.entry(virtio::MSIX_ENTRY) {
                    Ok((msg, false)) if Some(msg) == crate::pci::msi_message(vector) => {}
                    _ => return Err("MSI-X entry 0 does not target the network vector"),
                }
            }
            _ => {
                if interrupt_driven() {
                    return Err("a NIC without MSI-X claims to be interrupt-driven");
                }
            }
        }
        Ok(())
    }
}

kernel_test! {
    /// ARP for QEMU's virtual router, then ICMP echo to it.
    fn ping_the_gateway(env: &TestEnv) -> TestResult {
        if test_port(env).is_none() {
            return Ok(());
        }
        let ping = Pinger::new(56).map_err(|_| "no ping socket")?;
        for seq in 1..=3 {
            // A lost request is retried: slirp answers at once, but the
            // first one waits for ARP.
            let mut ok = false;
            for _ in 0..3 {
                if let Ok((rtt, 64)) = ping.echo(GATEWAY, seq, time::now_ns() + 2 * time::NANOS_PER_SEC) {
                    kprintln!("net: test: echo reply {seq} from {GATEWAY} in {} us", rtt / 1000);
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err("no echo reply from the gateway 10.0.2.2");
            }
        }
        match with_interface(|i| i.stack.neighbor(GATEWAY.into())) {
            Some(Some(_)) => Ok(()),
            _ => Err("the gateway's MAC address is not in the ARP cache"),
        }
    }
}

kernel_test! {
    /// TCP to the host's echo server, through QEMU's user networking:
    /// 1 MiB out and back, byte for byte, on the device's interrupts.
    fn tcp_echo_through_the_host(env: &TestEnv) -> TestResult {
        let Some(port) = test_port(env) else { return Ok(()) };
        let (irqs, _) = counters();
        let start = time::now_ns();
        let s = Socket::new(Domain::Inet, SockType::Stream, None).map_err(|_| "socket failed")?;
        s.connect(SocketAddr::new(GATEWAY.into(), port), secs(10)).map_err(|_| "connect to the host's echo server failed")?;
        let len = 1024 * 1024;
        exchange(&s, len, 32 * 1024)?;
        s.shutdown(Shutdown::Write).map_err(|_| "shutdown failed")?;
        let mut buf = [0u8; 16];
        if s.recv(&mut buf, secs(10)) != Ok(0) {
            return Err("the host did not close its side after ours");
        }
        let ms = (time::now_ns() - start) / time::NANOS_PER_MILLI;
        let (irqs_after, _) = counters();
        kprintln!("net: test: {len} bytes echoed by {GATEWAY}:{port} in {ms} ms, {} interrupts", irqs_after - irqs);
        if interrupt_driven() && irqs_after == irqs {
            return Err("the transfer finished without a device interrupt");
        }
        Ok(())
    }
}

kernel_test! {
    /// UDP to the host's echo server: datagrams of several sizes come back
    /// unchanged (each is retried if lost).
    fn udp_echo_through_the_host(env: &TestEnv) -> TestResult {
        let Some(port) = test_port(env) else { return Ok(()) };
        let host = SocketAddr::new(GATEWAY.into(), port);
        let s = Socket::new(Domain::Inet, SockType::Datagram, None).map_err(|_| "socket failed")?;
        s.connect(host, None).map_err(|_| "connect failed")?;
        let mut buf = vec![0u8; 2048];
        for n in [1usize, 64, 512, 1472] {
            let msg: Vec<u8> = (0..n).map(|i| pattern(i + n)).collect();
            let mut ok = false;
            for _ in 0..3 {
                s.send(&msg, secs(5)).map_err(|_| "send failed")?;
                if let Ok(info) = s.recv_from(&mut buf, RecvFlags::default(), secs(2)) {
                    if info.from != host || buf[..info.len] != msg[..] {
                        return Err("a UDP echo came back wrong");
                    }
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err("no UDP echo from the host");
            }
        }
        Ok(())
    }
}

kernel_test! {
    /// The host connects to us through its port forward and sends
    /// [`HOST_BYTES`] pattern bytes; we check them and answer "ok <n>".
    fn accepts_a_connection_from_the_host(env: &TestEnv) -> TestResult {
        if test_port(env).is_none() {
            return Ok(());
        }
        let l = Socket::new(Domain::Inet, SockType::Stream, None).map_err(|_| "socket failed")?;
        let _ = l.with(|s, h| s.set_reuse_addr(h, true));
        l.bind(Domain::Inet.any(LISTEN_PORT)).map_err(|_| "bind failed")?;
        l.listen(1).map_err(|_| "listen failed")?;
        kprintln!("{LISTENING} {LISTEN_PORT}");
        let (c, peer) = l.accept(secs(30)).map_err(|_| "the host did not connect within 30 s")?;
        let mut got = Vec::new();
        let mut buf = vec![0u8; 8192];
        loop {
            match c.recv(&mut buf, secs(20)) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => return Err("receiving from the host failed"),
            }
        }
        let good = got.len() == HOST_BYTES && got.iter().enumerate().all(|(i, &b)| b == pattern(i));
        let answer = if good { alloc::format!("ok {}\n", got.len()) } else { alloc::format!("bad {}\n", got.len()) };
        c.send_all(answer.as_bytes(), secs(10)).map_err(|_| "answering the host failed")?;
        c.close().map_err(|_| "close failed")?;
        kprintln!("net: test: {} bytes from {peer}{}", got.len(), if good { ", as sent" } else { ", WRONG" });
        if good { Ok(()) } else { Err("the bytes from the host are not what it sends") }
    }
}
