//! Network commands: `ifconfig` (and `ip`), `ping`, `nc`.

use super::commands::CommandResult;
use super::{Console, sgr};
use crate::net::socket::{Pinger, Socket};
use crate::net::{self, Device, Domain, Shutdown, SockType};
use crate::{thread, time};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use core::fmt::Write;
use core::net::{Ipv4Addr, SocketAddr};
use poly_net::NetDevice;

macro_rules! out {
    ($con:expr, $($arg:tt)*) => {{
        let _ = writeln!($con, $($arg)*);
    }};
}

/// `ifconfig` / `ip`: the interface, its addresses and counters.
pub fn ifconfig(con: &mut dyn Console, args: &[String]) -> CommandResult {
    if !args.is_empty() {
        return Err("takes no arguments".into());
    }
    let info = net::info().ok_or("networking is not set up")?;
    let (irqs, polls) = net::counters();
    let shown = net::with_interface(|i| {
        let mode = match (info.bdf, info.vector) {
            (None, _) => String::from("no device (loopback only)"),
            (Some(bdf), Some(v)) => format!("virtio-net {bdf}, interrupt-driven (vector {v:#x})"),
            (Some(bdf), None) => format!("virtio-net {bdf}, polled"),
        };
        let link = if i.dev.failed() {
            "failed"
        } else if i.dev.link_up() {
            "up"
        } else {
            "down"
        };
        out!(con, "{}{}{}: {mode}, link {link}, MTU {}", sgr::BOLD, net::NAME, sgr::RESET, i.dev.mtu());
        if info.bdf.is_some() {
            out!(con, "  ether {}", i.dev.mac());
        }
        match info.ipv4 {
            Some(s) => {
                let mut line = format!("  inet {}/{}", s.addr.addr, s.addr.prefix_len);
                if let Some(g) = s.gateway {
                    let _ = write!(line, "  gateway {g}");
                }
                if let Some(d) = net::dns_server() {
                    let _ = write!(line, "  dns {d}");
                }
                out!(con, "{line}");
            }
            None if info.bdf.is_some() => out!(con, "  inet none (boot with {}=...)", net::SETTING_KEY),
            None => {}
        }
        if let Some(ll) = i.stack.ipv6_link_local() {
            out!(con, "  inet6 {ll}/64");
        }
        if let Device::Virtio(d) = &i.dev {
            let s = d.stats();
            out!(
                con,
                "  rx {} frames, {} bytes, {} dropped, {} errors",
                s.rx_packets,
                s.rx_bytes,
                s.rx_dropped,
                s.rx_errors
            );
            out!(con, "  tx {} frames, {} bytes, {} busy", s.tx_packets, s.tx_bytes, s.tx_busy);
        }
        let s = i.stack.stats();
        out!(
            con,
            "  stack: {} sockets; {} ARP requests, {} ARP replies out; {} malformed, {} bad checksums, {} ignored; {} TCP resets out",
            i.stack.socket_count(),
            s.arp_requests_out,
            s.arp_replies_out,
            s.rx_malformed,
            s.rx_checksum,
            s.rx_ignored,
            s.tcp_rst_out
        );
        out!(con, "  {irqs} interrupts, {polls} polls");
    });
    shown.ok_or_else(|| String::from("networking is not set up"))?;
    out!(con, "{}lo{}: loopback 127.0.0.1/8 ::1 (inside the stack)", sgr::BOLD, sgr::RESET);
    Ok(())
}

fn parse_ipv4(s: &str) -> Result<Ipv4Addr, String> {
    s.parse().map_err(|_| format!("'{s}' is not an IPv4 address"))
}

/// `ping [-c count] <ipv4>`: ICMP echo, one request a second.
pub fn ping(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let (count, dst) = match args {
        [dst] => (4, dst),
        [c, n, dst] if c == "-c" => (n.parse::<u16>().map_err(|_| "-c needs a count from 1 to 65535")?, dst),
        _ => return Err("usage: ping [-c count] <ipv4>".into()),
    };
    if count == 0 {
        return Err("-c needs a count from 1 to 65535".into());
    }
    let dst = parse_ipv4(dst)?;
    let pinger = Pinger::new(56).map_err(|e| format!("socket: {e}"))?;
    out!(con, "PING {dst}: 56 data bytes");
    let mut received = 0u32;
    let mut rtts = (u64::MAX, 0u64, 0u64);
    for seq in 1..=count {
        let start = time::now_ns();
        match pinger.echo(dst, seq, start + time::NANOS_PER_SEC) {
            Ok((rtt, len)) => {
                received += 1;
                rtts = (rtts.0.min(rtt), rtts.1.max(rtt), rtts.2 + rtt);
                out!(
                    con,
                    "{len} bytes from {dst}: icmp_seq={seq} time={}.{:03} ms",
                    rtt / 1_000_000,
                    rtt / 1000 % 1000
                );
            }
            Err(e) if e == net::Errno::EAGAIN => out!(con, "no reply for icmp_seq={seq}"),
            Err(e) => return Err(format!("{dst}: {e}")),
        }
        if seq < count {
            thread::sleep_until(start + time::NANOS_PER_SEC);
        }
    }
    let loss = (u32::from(count) - received) * 100 / u32::from(count);
    out!(con, "--- {dst} ping statistics ---");
    out!(con, "{count} packets transmitted, {received} received, {loss}% packet loss");
    if received > 0 {
        let us = |ns: u64| format!("{}.{:03}", ns / 1_000_000, ns / 1000 % 1000);
        out!(con, "rtt min/avg/max = {}/{}/{} ms", us(rtts.0), us(rtts.2 / u64::from(received)), us(rtts.1));
    }
    Ok(())
}

/// `nc <ipv4> <port> [text...]`: connect over TCP, send the text and a
/// newline, close our side and print what comes back until the peer closes
/// (or 5 seconds of silence).
pub fn nc(con: &mut dyn Console, args: &[String]) -> CommandResult {
    let [ip, port, text @ ..] = args else { return Err("usage: nc <ipv4> <port> [text...]".into()) };
    let ip = parse_ipv4(ip)?;
    let port: u16 = port.parse().map_err(|_| format!("'{port}' is not a port"))?;
    let peer = SocketAddr::new(ip.into(), port);
    let s = Socket::new(Domain::Inet, SockType::Stream, None).map_err(|e| format!("socket: {e}"))?;
    let soon = || Some(time::now_ns() + 5 * time::NANOS_PER_SEC);
    s.connect(peer, soon()).map_err(|e| format!("connect to {peer}: {e}"))?;
    if !text.is_empty() {
        let line = text.join(" ") + "\n";
        s.send_all(line.as_bytes(), soon()).map_err(|e| format!("send: {e}"))?;
    }
    s.shutdown(Shutdown::Write).map_err(|e| format!("shutdown: {e}"))?;
    let mut buf = vec![0u8; 4096];
    loop {
        match s.recv(&mut buf, soon()) {
            Ok(0) => break,
            Ok(n) => {
                let _ = con.write_str(&String::from_utf8_lossy(&buf[..n]));
            }
            Err(e) if e == net::Errno::EAGAIN => {
                out!(con, "(no more data for 5 s)");
                break;
            }
            Err(e) => return Err(format!("recv: {e}")),
        }
    }
    s.close().map_err(|e| e.to_string())
}
