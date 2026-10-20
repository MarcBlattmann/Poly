//! Network boot tests: the kernel on a virtio-net device under QEMU's user
//! networking ("slirp", `-netdev user`), talking to servers that xtask runs
//! on the host.
//!
//! Under user networking the guest is 10.0.2.15, and 10.0.2.2 is the host:
//! a connection from the guest to 10.0.2.2:<port> reaches the host's
//! 127.0.0.1:<port> (QEMU documentation, "Network options", `-netdev
//! user`). So xtask runs a TCP and a UDP echo server on one host port and
//! tells the kernel the port (`poly.test.net=<port>`); a host forward
//! (`hostfwd=tcp:127.0.0.1:<h>-:7777`) lets the host connect to the
//! guest's port 7777 as well. The kernel's `net::tests` then
//!
//! - ping 10.0.2.2 (ARP, then ICMP echo, which QEMU answers itself),
//! - push 1 MiB through the TCP echo server and check every byte,
//! - send UDP datagrams of several sizes through the UDP echo server,
//! - accept the connection the host makes through the forward once the
//!   kernel prints that it listens, check the 100 000 bytes the host sends
//!   and answer "ok 100000", which the host checks too,
//!
//! and xtask checks that the echo servers really saw the traffic. One boot
//! is interrupt-driven (MSI-X), one polled (`vectors=0`). A third types
//! `ifconfig`, `ping` and `nc` into the shell. A fourth runs
//! `/bin/socket-test` as the first process with `/etc/socket-test.conf`
//! naming the echo servers: a user program connects over TCP through the
//! socket system calls, pushes 256 KiB through the echo server under
//! `poll`, sends UDP datagrams, and finds 10.0.2.15 in a netlink dump
//! (ADR 0048).

use super::{
    Devices, PASS_MARKER, QEMU_SUCCESS, Result, SHELL_FLAG, Symbols, boot_typing, boot_with, console, initramfs, ktest,
    programs, root,
};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The kernel's test flag: the host's echo port (`net::tests::TEST_NET_FLAG`).
const TEST_NET_FLAG: &str = "poly.test.net";
/// The guest port the host connects to (`net::tests::LISTEN_PORT`).
const GUEST_PORT: u16 = 7777;
/// What the kernel prints while it waits for the host (`net::tests::LISTENING`).
const LISTENING: &str = "net: test: listening for the host on port";
/// Bytes the host sends to the guest (`net::tests::HOST_BYTES`).
const HOST_BYTES: usize = 100_000;
/// Bytes the kernel's TCP test pushes through the echo server.
const TCP_BYTES: u64 = 1024 * 1024;

/// Byte `i` of the test streams (`net::tests::pattern`).
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8)) as u8
}

/// A TCP and a UDP echo server on one port of 127.0.0.1, counting what
/// they echo. They run until xtask exits.
struct EchoServers {
    port: u16,
    tcp_bytes: Arc<AtomicU64>,
    udp_datagrams: Arc<AtomicU64>,
}

impl EchoServers {
    fn start() -> Result<EchoServers> {
        // The UDP socket must get the TCP listener's port number; another
        // program may hold it already, so try a few.
        let (tcp, udp) = (0..20)
            .find_map(|_| {
                let tcp = TcpListener::bind("127.0.0.1:0").ok()?;
                let port = tcp.local_addr().ok()?.port();
                let udp = UdpSocket::bind(("127.0.0.1", port)).ok()?;
                Some((tcp, udp))
            })
            .ok_or("cannot bind the echo servers on 127.0.0.1")?;
        let port = udp.local_addr().map_err(|e| e.to_string())?.port();
        let tcp_bytes = Arc::new(AtomicU64::new(0));
        let udp_datagrams = Arc::new(AtomicU64::new(0));
        let count = Arc::clone(&tcp_bytes);
        thread::spawn(move || {
            for conn in tcp.incoming().map_while(std::result::Result::ok) {
                let count = Arc::clone(&count);
                thread::spawn(move || echo_tcp(conn, &count));
            }
        });
        let count = Arc::clone(&udp_datagrams);
        thread::spawn(move || {
            let mut buf = [0u8; 65536];
            while let Ok((n, from)) = udp.recv_from(&mut buf) {
                if udp.send_to(&buf[..n], from).is_ok() {
                    count.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Ok(EchoServers { port, tcp_bytes, udp_datagrams })
    }
}

/// Echo one connection until the peer closes its side, then close ours.
fn echo_tcp(mut conn: TcpStream, count: &AtomicU64) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(60)));
    let mut buf = [0u8; 65536];
    while let Ok(n @ 1..) = conn.read(&mut buf) {
        if conn.write_all(&buf[..n]).is_err() {
            return;
        }
        count.fetch_add(n as u64, Ordering::Relaxed);
    }
    let _ = conn.shutdown(Shutdown::Both);
}

/// A free TCP port on 127.0.0.1 for QEMU's host forward (free a moment
/// ago; QEMU fails to start if someone takes it in between).
fn free_port() -> Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(l.local_addr().map_err(|e| e.to_string())?.port())
}

/// QEMU arguments for a virtio-net device on user networking, forwarding
/// the host's `host_port` (if any) to the guest's [`GUEST_PORT`].
pub fn nic_args(host_port: Option<u16>, polled: bool) -> Vec<String> {
    let fwd = host_port.map_or(String::new(), |p| format!(",hostfwd=tcp:127.0.0.1:{p}-:{GUEST_PORT}"));
    let vectors = if polled { ",vectors=0" } else { "" };
    vec![
        "-netdev".into(),
        format!("user,id=net0{fwd}"),
        "-device".into(),
        format!("virtio-net-pci,netdev=net0,disable-legacy=on{vectors}"),
    ]
}

/// Connect to the guest through the forward (retrying while QEMU's
/// listener and the guest come up), send [`HOST_BYTES`] pattern bytes,
/// close our side and read the guest's answer.
fn talk_to_guest(port: u16) -> Result<String> {
    let start = Instant::now();
    let mut conn = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(c) => break c,
            Err(e) if start.elapsed() > Duration::from_secs(20) => return Err(format!("connect: {e}")),
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    };
    conn.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
    let data: Vec<u8> = (0..HOST_BYTES).map(pattern).collect();
    conn.write_all(&data).map_err(|e| format!("send: {e}"))?;
    conn.shutdown(Shutdown::Write).map_err(|e| e.to_string())?;
    let mut answer = String::new();
    conn.read_to_string(&mut answer).map_err(|e| format!("read: {e}"))?;
    Ok(answer)
}

/// Boot with the NIC (interrupt-driven, or polled), run the kernel's
/// network tests against the host's servers, and check both ends.
fn traffic_test(kernel: &Path, syms: &Arc<Symbols>, echo: &EchoServers, polled: bool) -> Result {
    let how = if polled { "polled (no MSI-X)" } else { "interrupt-driven (MSI-X)" };
    eprintln!("==> boot test: virtio-net {how}: ping, TCP and UDP echo through the host, a connection from the host");
    let host_port = free_port()?;
    let append = format!("poly.qemu_exit poly.test=net:: poly.net=qemu {TEST_NET_FLAG}={}", echo.port);
    let (tcp_before, udp_before) = (echo.tcp_bytes.load(Ordering::Relaxed), echo.udp_datagrams.load(Ordering::Relaxed));
    let answer: Arc<Mutex<Option<Result<String>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&answer);
    let mut started = false;
    let on_line = move |line: &str| {
        if !started && line.contains(LISTENING) {
            started = true;
            let slot = Arc::clone(&slot);
            thread::spawn(move || *slot.lock().unwrap() = Some(talk_to_guest(host_port)));
        }
    };
    let qemu_args = nic_args(Some(host_port), polled);
    let b = boot_with(kernel, syms, super::DEFAULT_MEMORY, &append, Devices::None, &qemu_args, None, on_line)?;
    let report = ktest::parse(&b.output);
    report.check()?;
    match b.status {
        Some(QEMU_SUCCESS) if b.output.contains(PASS_MARKER) => {}
        s => return Err(format!("the network boot ({how}) ended with status {s:?}")),
    }
    let mode = if polled { "; polled every " } else { "; queues on MSI-X entry 0 -> vector 0x" };
    let checks = [
        (
            "the NIC came up with QEMU's MAC address",
            "virtio-net: 00:".to_string(),
            "MAC 52:54:00:12:34:56, MTU 1500, link up",
        ),
        (how, "net: eth0: virtio-net ".into(), mode),
        (
            "the address came from poly.net=qemu",
            "net: eth0: inet ".into(),
            "10.0.2.15/24 gateway 10.0.2.2 dns 10.0.2.3",
        ),
        ("10.0.2.2 answered pings", "net: test: echo reply 3 from 10.0.2.2".into(), ""),
        ("1 MiB went through the TCP echo server", format!("net: test: {TCP_BYTES} bytes echoed by 10.0.2.2:"), ""),
        ("the host's bytes arrived intact", format!("net: test: {HOST_BYTES} bytes from 10.0.2.2:"), ", as sent"),
    ];
    for (what, start, rest) in checks {
        if !b.output.lines().any(|l| l.contains(&start) && l.contains(rest)) {
            return Err(format!("network boot ({how}): {what}: no line with \"{start}\"...\"{rest}\""));
        }
    }
    let host = answer.lock().unwrap().take().ok_or("the kernel never listened for the host")?;
    let host = host.map_err(|e| format!("the host's connection to the guest: {e}"))?;
    if host != format!("ok {HOST_BYTES}\n") {
        return Err(format!("the guest answered the host with {host:?}"));
    }
    let tcp = echo.tcp_bytes.load(Ordering::Relaxed) - tcp_before;
    let udp = echo.udp_datagrams.load(Ordering::Relaxed) - udp_before;
    if tcp < TCP_BYTES || udp < 4 {
        return Err(format!(
            "the host's echo servers saw {tcp} TCP bytes and {udp} datagrams; expected the tests' traffic"
        ));
    }
    eprintln!(
        "==> the host echoed {tcp} TCP bytes and {udp} UDP datagrams; the guest took {HOST_BYTES} bytes from the host"
    );
    Ok(())
}

/// Type network commands into the shell and check what they print.
fn shell_test(kernel: &Path, syms: &Arc<Symbols>, echo: &EchoServers) -> Result {
    eprintln!("==> boot test: ifconfig, ping and nc in the shell, on the virtio-net device");
    let append = format!("poly.qemu_exit {SHELL_FLAG} poly.test=net::tests::device_found poly.net=qemu");
    let nc = format!("nc 10.0.2.2 {} hello from poly\r", echo.port);
    let commands = ["ifconfig\r", "ping -c 2 10.0.2.2\r", nc.as_str(), "ping -c 1 10.0.2.99\r", "reboot\r"];
    let steps: Vec<_> = commands.iter().map(|&c| (console::PROMPT, c)).collect();
    let b = boot_typing(kernel, syms, &append, Devices::None, &nic_args(None, false), &steps)?;
    let ex = console::exchanges(&b.output);
    if ex.len() < commands.len() {
        return Err(format!(
            "the shell answered {} of {} network commands (status {:?})",
            ex.len(),
            commands.len(),
            b.status
        ));
    }
    let out = |i: usize| ex[i].output;
    let checks = [
        (
            "ifconfig shows the interface",
            [
                "eth0: virtio-net ",
                "link up, MTU 1500",
                "ether 52:54:00:12:34:56",
                "inet 10.0.2.15/24  gateway 10.0.2.2  dns 10.0.2.3",
                "inet6 fe80::5054:ff:fe12:3456/64",
            ]
            .iter()
            .all(|s| out(0).contains(s)),
        ),
        (
            "ping gets replies from 10.0.2.2",
            out(1).contains("64 bytes from 10.0.2.2: icmp_seq=1 ")
                && out(1).contains("2 packets transmitted, 2 received, 0% packet loss"),
        ),
        ("nc gets the echo", out(2) == "hello from poly\n"),
        (
            "ping reports a host that does not answer",
            out(3).contains("1 packets transmitted, 0 received, 100% packet loss"),
        ),
    ];
    for (what, ok) in checks {
        if !ok {
            return Err(format!("network shell commands: {what} failed"));
        }
    }
    if b.status != Some(0) {
        return Err(format!("reboot did not reset the machine (status {:?})", b.status));
    }
    Ok(())
}

/// `/bin/socket-test` as the first process, talking to the host's echo
/// servers through the socket system calls.
fn socket_program_test(kernel: &Path, syms: &Arc<Symbols>, echo: &EchoServers) -> Result {
    eprintln!("==> boot test: a user program uses TCP, UDP and netlink sockets on virtio-net (/bin/socket-test)");
    let elf = programs::socket_test()?;
    let conf = format!("10.0.2.2 {}\n", echo.port);
    let image =
        initramfs::programs_image_with_files(&[("socket-test", &elf)], &[("etc/socket-test.conf", conf.as_bytes())]);
    let image = initramfs::write(&root(), "poly-socket-test-initramfs.cpio", &image)?;
    let (tcp_before, udp_before) = (echo.tcp_bytes.load(Ordering::Relaxed), echo.udp_datagrams.load(Ordering::Relaxed));
    let mut args = nic_args(None, false);
    args.extend(["-initrd".into(), image.display().to_string()]);
    let append = "poly.qemu_exit poly.net=qemu poly.init=/bin/socket-test";
    let b = boot_with(kernel, syms, super::DEFAULT_MEMORY, append, Devices::None, &args, None, |_| {})?;
    if let Some(line) = b.output.lines().find(|l| l.contains("socket-test: FAIL") || l.contains(" not implemented")) {
        return Err(format!("/bin/socket-test on the NIC: {line}"));
    }
    let ok = b.status == Some(QEMU_SUCCESS)
        && b.output.contains("socket-test: remote ok")
        && b.output.contains(programs::SOCKET_TEST_OUTPUT.trim_end())
        && b.output.contains("POLY-EXIT: pid=1 code=0");
    if !ok {
        return Err(format!("/bin/socket-test on the NIC did not pass (QEMU status {:?})", b.status));
    }
    let tcp = echo.tcp_bytes.load(Ordering::Relaxed) - tcp_before;
    let udp = echo.udp_datagrams.load(Ordering::Relaxed) - udp_before;
    if tcp < 256 * 1024 || udp < 3 {
        return Err(format!("the host's echo servers saw {tcp} TCP bytes and {udp} datagrams from /bin/socket-test"));
    }
    eprintln!("==> the host echoed {tcp} TCP bytes and {udp} UDP datagrams for /bin/socket-test");
    Ok(())
}

/// All network boot tests.
pub fn boot_tests(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let echo = EchoServers::start()?;
    traffic_test(kernel, syms, &echo, false)?;
    traffic_test(kernel, syms, &echo, true)?;
    shell_test(kernel, syms, &echo)?;
    socket_program_test(kernel, syms, &echo)
}
