//! `/bin/socket-test`: the socket system calls from ring 3 (ADR 0048), a
//! static Linux program without any C library that `cargo xtask` builds
//! like the nolibc hello (`tools/xtask/src/programs.rs`).
//!
//! Local checks, in every run:
//!
//! - TCP over 127.0.0.1 between two processes (fork): the server waits in
//!   `epoll_wait` for the connection, accepts it non-blocking, uses `poll`
//!   for every read, takes a greeting and 100 000 pattern bytes up to end
//!   of file, answers; the client checks the answers.
//! - UDP over 127.0.0.1: `sendto` / `recvfrom` with the sender's address,
//!   `FIONREAD`, `MSG_TRUNC`, a connected socket, `SO_RCVTIMEO` running out.
//! - `AF_UNIX`: a stream socket bound to a path (`S_IFSOCK` in the file
//!   system) with a client in a child process and `SO_PEERCRED`; an
//!   abstract name; datagram and sequenced-packet socket pairs keeping
//!   boundaries; `SCM_RIGHTS` passing a pipe's read end, and a write end
//!   in flight dropped with the queue of the socket that never read it.
//! - A blocked `accept` ended by a signal: `EINTR` without `SA_RESTART`,
//!   restarted (and then accepting) with it.
//! - `SIGPIPE` after `EPIPE`, not with `MSG_NOSIGNAL`; a child killed by
//!   it.
//! - `AF_NETLINK`: an `RTM_GETADDR` dump lists 127.0.0.1, an
//!   `RTM_GETLINK` dump lists `lo`.
//!
//! With `/etc/socket-test.conf` (`<address> <port>`, written by the
//! network boot test), it also pushes 256 KiB through the TCP echo server
//! there with a non-blocking socket under `poll`, sends UDP datagrams to
//! the UDP echo server on the same port, and finds the interface's
//! address in a netlink dump.
//!
//! Every check prints `socket-test: ok <name>` or `socket-test: FAIL
//! <name>: <value>`; the program ends with `socket-test: done, <n>
//! failed` and exit status `n`. Numbers and layouts from the x86-64
//! system call table and the UAPI headers (`linux/socket.h`, `linux/in.h`,
//! `linux/un.h`, `linux/netlink.h`, `linux/rtnetlink.h`, `linux/if_addr.h`,
//! `asm-generic/socket.h`, `asm/signal.h`). Written for Poly from the
//! manual pages.

#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering::SeqCst};

const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPEN: u64 = 2;
const SYS_CLOSE: u64 = 3;
const SYS_STAT: u64 = 4;
const SYS_FSTAT: u64 = 5;
const SYS_POLL: u64 = 7;
const SYS_RT_SIGACTION: u64 = 13;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_IOCTL: u64 = 16;
const SYS_SETITIMER: u64 = 38;
const SYS_GETPID: u64 = 39;
const SYS_SOCKET: u64 = 41;
const SYS_CONNECT: u64 = 42;
const SYS_ACCEPT: u64 = 43;
const SYS_SENDTO: u64 = 44;
const SYS_RECVFROM: u64 = 45;
const SYS_SENDMSG: u64 = 46;
const SYS_RECVMSG: u64 = 47;
const SYS_SHUTDOWN: u64 = 48;
const SYS_BIND: u64 = 49;
const SYS_LISTEN: u64 = 50;
const SYS_GETSOCKNAME: u64 = 51;
const SYS_GETPEERNAME: u64 = 52;
const SYS_SOCKETPAIR: u64 = 53;
const SYS_SETSOCKOPT: u64 = 54;
const SYS_GETSOCKOPT: u64 = 55;
const SYS_FORK: u64 = 57;
const SYS_WAIT4: u64 = 61;
const SYS_UNLINK: u64 = 87;
const SYS_RT_SIGPENDING: u64 = 127;
const SYS_RT_SIGTIMEDWAIT: u64 = 128;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_EXIT_GROUP: u64 = 231;
const SYS_EPOLL_WAIT: u64 = 232;
const SYS_EPOLL_CTL: u64 = 233;
const SYS_ACCEPT4: u64 = 288;
const SYS_EPOLL_CREATE1: u64 = 291;
const SYS_PIPE2: u64 = 293;

const AF_UNIX: u64 = 1;
const AF_INET: u64 = 2;
const AF_NETLINK: u64 = 16;
const SOCK_STREAM: u64 = 1;
const SOCK_DGRAM: u64 = 2;
const SOCK_RAW: u64 = 3;
const SOCK_SEQPACKET: u64 = 5;
const SOCK_NONBLOCK: u64 = 0o4000;
const SOCK_CLOEXEC: u64 = 0o2000000;
const SOL_SOCKET: u64 = 1;
const SO_REUSEADDR: u64 = 2;
const SO_TYPE: u64 = 3;
const SO_PEERCRED: u64 = 17;
const SO_RCVTIMEO: u64 = 20;
const SO_ACCEPTCONN: u64 = 30;
const MSG_PEEK: u64 = 0x2;
const MSG_TRUNC: u64 = 0x20;
const MSG_DONTWAIT: u64 = 0x40;
const MSG_NOSIGNAL: u64 = 0x4000;
const SHUT_WR: u64 = 1;
const SCM_RIGHTS: u32 = 1;
const NETLINK_ROUTE: u64 = 0;
const POLLIN: u16 = 0x1;
const POLLOUT: u16 = 0x4;
const EPOLLIN: u32 = 0x1;
const EPOLL_CTL_ADD: u64 = 1;
const FIONREAD: u64 = 0x541B;
const S_IFMT: u32 = 0o170000;
const S_IFSOCK: u32 = 0o140000;
const SIGPIPE: u64 = 13;
const SIGALRM: u64 = 14;
const SA_RESTART: u64 = 0x1000_0000;
const SA_RESTORER: u64 = 0x0400_0000;
const SIG_BLOCK: u64 = 0;
const SIG_UNBLOCK: u64 = 1;
const ITIMER_REAL: u64 = 0;
const CLOCK_MONOTONIC: u64 = 1;

const EINTR: i64 = 4;
const EAGAIN: i64 = 11;
const EPIPE: i64 = 32;
const ECONNREFUSED: i64 = 111;

const MS: u64 = 1_000_000;
/// Poll and wait timeouts: generous, for a loaded host.
const WAIT_MS: u64 = 30_000;

/// # Safety
/// The call must not break the program's memory.
unsafe fn syscall(nr: u64, a: [u64; 6]) -> i64 {
    let ret;
    // SAFETY: the x86-64 Linux convention: rcx and r11 are clobbered,
    // memory may be read and written by the kernel.
    unsafe {
        asm!("syscall", inlateout("rax") nr => ret, in("rdi") a[0], in("rsi") a[1], in("rdx") a[2],
             in("r10") a[3], in("r8") a[4], in("r9") a[5], lateout("rcx") _, lateout("r11") _,
             options(nostack));
    }
    ret
}

fn sys(nr: u64, a: &[u64]) -> i64 {
    let mut args = [0u64; 6];
    args[..a.len()].copy_from_slice(a);
    // SAFETY: every call below passes pointers to this program's own
    // variables, sized as the kernel expects.
    unsafe { syscall(nr, args) }
}

fn write_out(s: &[u8]) {
    sys(SYS_WRITE, &[1, s.as_ptr() as u64, s.len() as u64]);
}

fn write_num(n: i64) {
    let mut buf = [0u8; 24];
    let mut i = buf.len();
    let mut v = n.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if n < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    write_out(&buf[i..]);
}

static FAILED: AtomicU64 = AtomicU64::new(0);

/// Report one check: `value` is printed when it fails.
fn check(name: &[u8], ok: bool, value: i64) -> bool {
    if ok {
        write_out(b"socket-test: ok ");
        write_out(name);
    } else {
        FAILED.fetch_add(1, SeqCst);
        write_out(b"socket-test: FAIL ");
        write_out(name);
        write_out(b": ");
        write_num(value);
    }
    write_out(b"\n");
    ok
}

fn exit(code: u64) -> ! {
    loop {
        sys(SYS_EXIT_GROUP, &[code]);
    }
}

fn mono_ns() -> u64 {
    let mut t = [0i64; 2];
    sys(SYS_CLOCK_GETTIME, &[CLOCK_MONOTONIC, t.as_mut_ptr() as u64]);
    t[0] as u64 * 1_000_000_000 + t[1] as u64
}

fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8)) as u8
}

// ---- socket helpers ------------------------------------------------------------

fn sin(ip: [u8; 4], port: u16) -> [u8; 16] {
    let mut a = [0u8; 16];
    a[0..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
    a[2..4].copy_from_slice(&port.to_be_bytes());
    a[4..8].copy_from_slice(&ip);
    a
}

/// A `sockaddr_un` with `path` (abstract when it starts with NUL) and the
/// length to pass.
fn sun(path: &[u8]) -> ([u8; 110], u64) {
    let mut a = [0u8; 110];
    a[0..2].copy_from_slice(&(AF_UNIX as u16).to_le_bytes());
    a[2..2 + path.len()].copy_from_slice(path);
    let len = if path.first() == Some(&0) { 2 + path.len() } else { 2 + path.len() + 1 };
    (a, len as u64)
}

fn socket(domain: u64, ty: u64, protocol: u64) -> i64 {
    sys(SYS_SOCKET, &[domain, ty, protocol])
}

fn bind(fd: i64, addr: &[u8], len: u64) -> i64 {
    sys(SYS_BIND, &[fd as u64, addr.as_ptr() as u64, len])
}

fn connect(fd: i64, addr: &[u8], len: u64) -> i64 {
    sys(SYS_CONNECT, &[fd as u64, addr.as_ptr() as u64, len])
}

fn listen(fd: i64, backlog: u64) -> i64 {
    sys(SYS_LISTEN, &[fd as u64, backlog])
}

fn accept(fd: i64) -> i64 {
    sys(SYS_ACCEPT, &[fd as u64, 0, 0])
}

/// getsockname / getpeername into `buf`; returns the result and length.
fn name(nr: u64, fd: i64, buf: &mut [u8]) -> (i64, u32) {
    let mut len = buf.len() as u32;
    let r = sys(nr, &[fd as u64, buf.as_mut_ptr() as u64, &raw mut len as u64]);
    (r, len)
}

fn port_of(fd: i64) -> u16 {
    let mut a = [0u8; 16];
    name(SYS_GETSOCKNAME, fd, &mut a);
    u16::from_be_bytes([a[2], a[3]])
}

fn send(fd: i64, data: &[u8], flags: u64) -> i64 {
    sys(SYS_SENDTO, &[fd as u64, data.as_ptr() as u64, data.len() as u64, flags, 0, 0])
}

fn sendto(fd: i64, data: &[u8], to: &[u8], len: u64) -> i64 {
    sys(SYS_SENDTO, &[fd as u64, data.as_ptr() as u64, data.len() as u64, 0, to.as_ptr() as u64, len])
}

fn recv(fd: i64, buf: &mut [u8], flags: u64) -> i64 {
    sys(SYS_RECVFROM, &[fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, flags, 0, 0])
}

fn read(fd: i64, buf: &mut [u8]) -> i64 {
    sys(SYS_READ, &[fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64])
}

fn write(fd: i64, data: &[u8]) -> i64 {
    sys(SYS_WRITE, &[fd as u64, data.as_ptr() as u64, data.len() as u64])
}

fn close(fd: i64) {
    sys(SYS_CLOSE, &[fd as u64]);
}

fn set_int(fd: i64, level: u64, name: u64, v: i32) -> i64 {
    sys(SYS_SETSOCKOPT, &[fd as u64, level, name, &raw const v as u64, 4])
}

fn get_int(fd: i64, level: u64, name: u64) -> i64 {
    let mut v = 0i32;
    let mut len = 4u32;
    let r = sys(SYS_GETSOCKOPT, &[fd as u64, level, name, &raw mut v as u64, &raw mut len as u64]);
    if r < 0 { r } else { i64::from(v) }
}

/// poll(2) on one descriptor; the returned events (or the error).
fn poll1(fd: i64, events: u16, ms: u64) -> i64 {
    let mut p = [0u8; 8];
    p[0..4].copy_from_slice(&(fd as i32).to_le_bytes());
    p[4..6].copy_from_slice(&events.to_le_bytes());
    let r = sys(SYS_POLL, &[p.as_mut_ptr() as u64, 1, ms]);
    if r <= 0 { r } else { i64::from(u16::from_le_bytes([p[6], p[7]])) }
}

/// Write all of `data`, polling a non-blocking descriptor when it is
/// full; false on an error.
fn write_all(fd: i64, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = write(fd, data);
        if n == -EAGAIN {
            if poll1(fd, POLLOUT, WAIT_MS) <= 0 {
                return false;
            }
            continue;
        }
        if n <= 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// Read exactly `buf.len()` bytes (polling a non-blocking descriptor);
/// false on end of file or an error.
fn read_exact(fd: i64, buf: &mut [u8]) -> bool {
    let mut got = 0;
    while got < buf.len() {
        let n = read(fd, &mut buf[got..]);
        if n == -EAGAIN {
            if poll1(fd, POLLIN, WAIT_MS) <= 0 {
                return false;
            }
            continue;
        }
        if n <= 0 {
            return false;
        }
        got += n as usize;
    }
    true
}

fn fork() -> i64 {
    sys(SYS_FORK, &[])
}

/// wait4(pid): the status word, or the error.
fn wait(pid: i64) -> i64 {
    let mut status = 0i32;
    let r = sys(SYS_WAIT4, &[pid as u64, &raw mut status as u64, 0, 0]);
    if r < 0 { r } else { i64::from(status) }
}

// ---- TCP over loopback between two processes ----------------------------------------

const TCP_BYTES: usize = 100_000;

/// The client: connect, greet, check the answer, send the pattern bytes,
/// end the stream, read the final answer. Its exit code says where it
/// failed.
fn tcp_client(port: u16) -> ! {
    let s = socket(AF_INET, SOCK_STREAM, 0);
    if s < 0 {
        exit(10);
    }
    let a = sin([127, 0, 0, 1], port);
    if connect(s, &a, 16) != 0 {
        exit(11);
    }
    if !write_all(s, b"hello from child") {
        exit(12);
    }
    let mut pong = [0u8; 4];
    if !read_exact(s, &mut pong) || &pong != b"pong" {
        exit(13);
    }
    let mut chunk = [0u8; 4096];
    let mut sent = 0;
    while sent < TCP_BYTES {
        let n = chunk.len().min(TCP_BYTES - sent);
        for (i, b) in chunk[..n].iter_mut().enumerate() {
            *b = pattern(sent + i);
        }
        let mut off = 0;
        while off < n {
            let w = send(s, &chunk[off..n], MSG_NOSIGNAL);
            if w <= 0 {
                exit(14);
            }
            off += w as usize;
        }
        sent += n;
    }
    if sys(SYS_SHUTDOWN, &[s as u64, SHUT_WR]) != 0 {
        exit(15);
    }
    let mut answer = [0u8; 16];
    let mut got = 0;
    loop {
        let n = read(s, &mut answer[got..]);
        if n < 0 {
            exit(16);
        }
        if n == 0 {
            break;
        }
        got += n as usize;
    }
    if &answer[..got] != b"got it" {
        exit(17);
    }
    exit(0)
}

fn tcp_loopback() {
    let l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    check(b"tcp_socket", l >= 0, l);
    check(b"so_reuseaddr", set_int(l, SOL_SOCKET, SO_REUSEADDR, 1) == 0, 0);
    let a = sin([127, 0, 0, 1], 0);
    let r = bind(l, &a, 16);
    check(b"tcp_bind_ephemeral_port", r == 0 && port_of(l) != 0, r);
    check(b"tcp_listen", listen(l, 4) == 0, 0);
    check(b"so_acceptconn", get_int(l, SOL_SOCKET, SO_ACCEPTCONN) == 1, get_int(l, SOL_SOCKET, SO_ACCEPTCONN));
    check(b"so_type", get_int(l, SOL_SOCKET, SO_TYPE) == SOCK_STREAM as i64, get_int(l, SOL_SOCKET, SO_TYPE));
    let port = port_of(l);
    let ep = sys(SYS_EPOLL_CREATE1, &[0]);
    let mut ev = [0u8; 12];
    ev[0..4].copy_from_slice(&EPOLLIN.to_le_bytes());
    ev[4..12].copy_from_slice(&(l as u64).to_le_bytes());
    check(b"epoll_ctl_listener", sys(SYS_EPOLL_CTL, &[ep as u64, EPOLL_CTL_ADD, l as u64, ev.as_ptr() as u64]) == 0, ep);
    // A non-blocking accept with nobody there.
    let nb_l = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    let a2 = sin([127, 0, 0, 1], 0);
    bind(nb_l, &a2, 16);
    listen(nb_l, 1);
    let nb = accept(nb_l);
    check(b"nonblocking_accept_with_nobody_there", nb == -EAGAIN, nb);
    close(nb_l);
    let pid = fork();
    if pid == 0 {
        tcp_client(port);
    }
    // The connection wakes epoll_wait.
    let mut out = [0u8; 24];
    let n = sys(SYS_EPOLL_WAIT, &[ep as u64, out.as_mut_ptr() as u64, 2, WAIT_MS]);
    let events = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
    let data = u64::from_le_bytes(out[4..12].try_into().unwrap());
    check(b"epoll_wait_sees_the_connection", n == 1 && events & EPOLLIN != 0 && data == l as u64, n);
    let mut peer = [0u8; 16];
    let mut plen = 16u32;
    let c = sys(SYS_ACCEPT4, &[l as u64, peer.as_mut_ptr() as u64, &raw mut plen as u64, SOCK_NONBLOCK]);
    check(b"accept4_nonblocking", c >= 0, c);
    check(b"accept_reports_the_peer", plen == 16 && peer[4..8] == [127, 0, 0, 1], i64::from(plen));
    let mut pn = [0u8; 16];
    let (r, _) = name(SYS_GETPEERNAME, c, &mut pn);
    check(b"getpeername", r == 0 && pn[2..8] == peer[2..8], r);
    let mut hello = [0u8; 16];
    check(b"poll_then_read_the_greeting", read_exact(c, &mut hello) && &hello == b"hello from child", 0);
    check(b"write_the_answer", write_all(c, b"pong"), 0);
    // The pattern bytes, under poll, to end of file.
    let mut got = 0usize;
    let mut good = true;
    let mut buf = [0u8; 8192];
    loop {
        let n = read(c, &mut buf);
        if n == -EAGAIN {
            if poll1(c, POLLIN, WAIT_MS) <= 0 {
                good = false;
                break;
            }
            continue;
        }
        if n <= 0 {
            good &= n == 0;
            break;
        }
        for (i, &b) in buf[..n as usize].iter().enumerate() {
            good &= b == pattern(got + i);
        }
        got += n as usize;
    }
    check(b"tcp_100000_bytes_to_end_of_file", good && got == TCP_BYTES, got as i64);
    check(b"answer_after_end_of_file", write_all(c, b"got it"), 0);
    close(c);
    let status = wait(pid);
    check(b"tcp_client_process", status == 0, status);
    close(l);
    close(ep);
    // Nobody listens on port 1 of loopback.
    let s = socket(AF_INET, SOCK_STREAM, 0);
    let a = sin([127, 0, 0, 1], 1);
    let r = connect(s, &a, 16);
    check(b"connect_refused", r == -ECONNREFUSED, r);
    close(s);
}

// ---- UDP ---------------------------------------------------------------------------------

fn udp_loopback() {
    let r = socket(AF_INET, SOCK_DGRAM, 0);
    let s = socket(AF_INET, SOCK_DGRAM, 0);
    let any = sin([127, 0, 0, 1], 0);
    check(b"udp_bind", bind(r, &any, 16) == 0, r);
    let to = sin([127, 0, 0, 1], port_of(r));
    let n = sendto(s, b"datagram one", &to, 16);
    check(b"udp_sendto", n == 12, n);
    check(b"udp_poll_readable", poll1(r, POLLIN, WAIT_MS) & i64::from(POLLIN) != 0, 0);
    let mut avail = 0i32;
    sys(SYS_IOCTL, &[r as u64, FIONREAD, &raw mut avail as u64]);
    check(b"udp_fionread_is_the_datagram_size", avail == 12, i64::from(avail));
    let mut buf = [0u8; 64];
    let mut from = [0u8; 16];
    let mut flen = 16u32;
    let n = sys(
        SYS_RECVFROM,
        &[r as u64, buf.as_mut_ptr() as u64, 64, 0, from.as_mut_ptr() as u64, &raw mut flen as u64],
    );
    check(b"udp_recvfrom", n == 12 && &buf[..12] == b"datagram one", n);
    let sport = port_of(s);
    check(b"udp_sender_address", flen == 16 && u16::from_be_bytes([from[2], from[3]]) == sport, i64::from(sport));
    // A long datagram into a short buffer: MSG_TRUNC gives its length.
    sendto(s, b"twenty bytes long!!!", &to, 16);
    poll1(r, POLLIN, WAIT_MS);
    let mut small = [0u8; 8];
    let n = recv(r, &mut small, MSG_TRUNC);
    check(b"udp_msg_trunc_reports_the_length", n == 20 && &small == b"twenty b", n);
    // Connected: send / recv.
    check(b"udp_connect", connect(s, &to, 16) == 0, 0);
    send(s, b"connected", 0);
    poll1(r, POLLIN, WAIT_MS);
    let n = recv(r, &mut buf, MSG_PEEK);
    let m = recv(r, &mut buf, 0);
    check(b"udp_peek_then_read", n == 9 && m == 9 && &buf[..9] == b"connected", m);
    // SO_RCVTIMEO: an empty socket gives up with EAGAIN, not before.
    let tv: [i64; 2] = [0, 50_000];
    sys(SYS_SETSOCKOPT, &[r as u64, SOL_SOCKET, SO_RCVTIMEO, tv.as_ptr() as u64, 16]);
    let t0 = mono_ns();
    let n = recv(r, &mut buf, 0);
    let waited = mono_ns() - t0;
    check(b"so_rcvtimeo_runs_out", n == -EAGAIN && waited >= 50 * MS, n);
    let n = recv(r, &mut buf, MSG_DONTWAIT);
    check(b"msg_dontwait", n == -EAGAIN, n);
    close(r);
    close(s);
}

// ---- Unix domain sockets ---------------------------------------------------------------

const SOCK_PATH: &[u8] = b"/tmp/socket-test.sock";

fn stat_mode(path: &[u8]) -> u32 {
    let mut p = [0u8; 64];
    p[..path.len()].copy_from_slice(path);
    let mut st = [0u8; 144];
    if sys(SYS_STAT, &[p.as_ptr() as u64, st.as_mut_ptr() as u64]) != 0 {
        return 0;
    }
    u32::from_le_bytes([st[24], st[25], st[26], st[27]])
}

fn unix_stream() {
    let mut p = [0u8; 64];
    p[..SOCK_PATH.len()].copy_from_slice(SOCK_PATH);
    sys(SYS_UNLINK, &[p.as_ptr() as u64]);
    let l = socket(AF_UNIX, SOCK_STREAM, 0);
    let (addr, len) = sun(SOCK_PATH);
    let r = bind(l, &addr, len);
    check(b"unix_bind_path", r == 0, r);
    check(b"unix_socket_inode", stat_mode(SOCK_PATH) & S_IFMT == S_IFSOCK, i64::from(stat_mode(SOCK_PATH)));
    let again = socket(AF_UNIX, SOCK_STREAM, 0);
    check(b"unix_bind_taken_path", bind(again, &addr, len) == -98, bind(again, &addr, len));
    close(again);
    check(b"unix_listen", listen(l, 2) == 0, 0);
    let pid = fork();
    if pid == 0 {
        let c = socket(AF_UNIX, SOCK_STREAM, 0);
        if connect(c, &addr, len) != 0 {
            exit(20);
        }
        if !write_all(c, b"unix hello") {
            exit(21);
        }
        let mut bye = [0u8; 3];
        if !read_exact(c, &mut bye) || &bye != b"bye" {
            exit(22);
        }
        exit(0);
    }
    let mut peer = [0u8; 110];
    let mut plen = 110u32;
    let a = sys(SYS_ACCEPT, &[l as u64, peer.as_mut_ptr() as u64, &raw mut plen as u64]);
    check(b"unix_accept", a >= 0, a);
    check(b"unix_accept_unnamed_peer", plen == 2, i64::from(plen));
    let mut msg = [0u8; 10];
    check(b"unix_read", read_exact(a, &mut msg) && &msg == b"unix hello", 0);
    let mut me = [0u8; 110];
    let (r, mlen) = name(SYS_GETSOCKNAME, a, &mut me);
    check(b"unix_accepted_name_is_the_listeners", r == 0 && mlen as u64 == len && me[2..len as usize] == addr[2..len as usize], i64::from(mlen));
    let mut cred = [0i32; 3];
    let mut clen = 12u32;
    sys(SYS_GETSOCKOPT, &[a as u64, SOL_SOCKET, SO_PEERCRED, cred.as_mut_ptr() as u64, &raw mut clen as u64]);
    check(b"so_peercred_is_the_client", i64::from(cred[0]) == pid && clen == 12, i64::from(cred[0]));
    check(b"unix_write", write_all(a, b"bye"), 0);
    let status = wait(pid);
    check(b"unix_client_process", status == 0, status);
    close(a);
    close(l);
    let r = connect(socket(AF_UNIX, SOCK_STREAM, 0), &addr, len);
    check(b"unix_connect_after_close_refused", r == -ECONNREFUSED, r);
    sys(SYS_UNLINK, &[p.as_ptr() as u64]);
    // The abstract namespace: a connect completes at once (the listener
    // takes it later).
    let (abs, alen) = sun(b"\0socket-test");
    let l = socket(AF_UNIX, SOCK_STREAM, 0);
    check(b"unix_bind_abstract", bind(l, &abs, alen) == 0 && listen(l, 1) == 0, 0);
    let c = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0);
    check(b"unix_connect_abstract", connect(c, &abs, alen) == 0, 0);
    let a = accept(l);
    send(c, b"abstract", 0);
    let mut buf = [0u8; 16];
    let n = recv(a, &mut buf, 0);
    check(b"unix_abstract_data", n == 8 && &buf[..8] == b"abstract", n);
    close(a);
    let n = recv(c, &mut buf, 0);
    check(b"unix_end_of_file_after_close", n == 0, n);
    close(c);
    close(l);
}

fn socketpair(ty: u64) -> [i64; 2] {
    let mut sv = [0i32; 2];
    let r = sys(SYS_SOCKETPAIR, &[AF_UNIX, ty, 0, sv.as_mut_ptr() as u64]);
    if r != 0 {
        check(b"socketpair", false, r);
    }
    [i64::from(sv[0]), i64::from(sv[1])]
}

/// `struct msghdr` with one iovec and a control buffer.
#[repr(C)]
struct MsgHdr {
    name: u64,
    namelen: u32,
    pad: u32,
    iov: u64,
    iovlen: u64,
    control: u64,
    controllen: u64,
    flags: i32,
    pad2: u32,
}

fn msghdr(iov: &[u64; 2], control: &mut [u8]) -> MsgHdr {
    MsgHdr {
        name: 0,
        namelen: 0,
        pad: 0,
        iov: iov.as_ptr() as u64,
        iovlen: 1,
        control: control.as_mut_ptr() as u64,
        controllen: control.len() as u64,
        flags: 0,
        pad2: 0,
    }
}

/// Send `data` with one descriptor in `SCM_RIGHTS`.
fn send_fd(sock: i64, data: &[u8], fd: i64) -> i64 {
    let mut control = [0u8; 24];
    control[0..8].copy_from_slice(&20u64.to_le_bytes());
    control[8..12].copy_from_slice(&1i32.to_le_bytes());
    control[12..16].copy_from_slice(&SCM_RIGHTS.to_le_bytes());
    control[16..20].copy_from_slice(&(fd as i32).to_le_bytes());
    let iov = [data.as_ptr() as u64, data.len() as u64];
    let m = msghdr(&iov, &mut control);
    sys(SYS_SENDMSG, &[sock as u64, &raw const m as u64, 0])
}

fn unix_pairs_and_rights() {
    // Datagrams keep their boundaries.
    let d = socketpair(SOCK_DGRAM);
    send(d[0], b"a", 0);
    send(d[0], b"bb", 0);
    send(d[0], b"ccc", 0);
    let mut buf = [0u8; 16];
    let sizes = [recv(d[1], &mut buf, 0), recv(d[1], &mut buf, 0), recv(d[1], &mut buf, 0)];
    check(b"unix_dgram_boundaries", sizes == [1, 2, 3], sizes[0]);
    close(d[0]);
    close(d[1]);
    // Sequenced packets: a short buffer truncates (MSG_TRUNC in flags).
    let q = socketpair(SOCK_SEQPACKET);
    send(q[0], b"12345", 0);
    let mut small = [0u8; 3];
    let iov = [small.as_mut_ptr() as u64, 3];
    let mut m = msghdr(&iov, &mut []);
    m.control = 0;
    let n = sys(SYS_RECVMSG, &[q[1] as u64, &raw mut m as u64, 0]);
    check(b"unix_seqpacket_truncation", n == 3 && m.flags as u64 & MSG_TRUNC != 0 && &small == b"123", n);
    close(q[0]);
    close(q[1]);
    // SCM_RIGHTS: a pipe's read end goes over a stream pair.
    let s = socketpair(SOCK_STREAM);
    let mut st = [0u8; 144];
    sys(SYS_FSTAT, &[s[0] as u64, st.as_mut_ptr() as u64]);
    let mode = u32::from_le_bytes([st[24], st[25], st[26], st[27]]);
    check(b"fstat_socket_is_s_ifsock", mode & S_IFMT == S_IFSOCK, i64::from(mode));
    let mut p = [0i32; 2];
    sys(SYS_PIPE2, &[p.as_mut_ptr() as u64, 0]);
    let r = send_fd(s[0], b"x", i64::from(p[0]));
    check(b"sendmsg_scm_rights", r == 1, r);
    close(i64::from(p[0]));
    write(i64::from(p[1]), b"through the pipe");
    let mut byte = [0u8; 4];
    let iov = [byte.as_mut_ptr() as u64, 4];
    let mut control = [0u8; 64];
    let mut m = msghdr(&iov, &mut control);
    let n = sys(SYS_RECVMSG, &[s[1] as u64, &raw mut m as u64, 0]);
    let level = i32::from_le_bytes(control[8..12].try_into().unwrap());
    let ty = i32::from_le_bytes(control[12..16].try_into().unwrap());
    let fd = i64::from(i32::from_le_bytes(control[16..20].try_into().unwrap()));
    check(b"recvmsg_scm_rights", n == 1 && m.controllen >= 20 && level == 1 && ty == 1 && fd >= 0, n);
    let mut text = [0u8; 16];
    let n = read(fd, &mut text);
    check(b"passed_pipe_reads", n == 16 && &text == b"through the pipe", n);
    close(fd);
    close(i64::from(p[1]));
    // A write end in flight is dropped with the queue of a socket that
    // never received it: the read end then sees end of file.
    let mut p = [0i32; 2];
    sys(SYS_PIPE2, &[p.as_mut_ptr() as u64, 0]);
    send_fd(s[0], b"y", i64::from(p[1]));
    close(i64::from(p[1]));
    close(s[1]);
    let mut b1 = [0u8; 1];
    let n = read(i64::from(p[0]), &mut b1);
    check(b"in_flight_descriptor_released_with_the_queue", n == 0, n);
    close(i64::from(p[0]));
    close(s[0]);
}

// ---- signals ------------------------------------------------------------------------------

#[repr(C)]
struct SigAction {
    handler: u64,
    flags: u64,
    restorer: u64,
    mask: u64,
}

static ALARMS: AtomicU64 = AtomicU64::new(0);
/// A pipe's write end the handler writes a byte to (0: none).
static WAKE_FD: AtomicU64 = AtomicU64::new(0);

extern "C" fn on_alarm(_: i32) {
    ALARMS.fetch_add(1, SeqCst);
    let fd = WAKE_FD.load(SeqCst);
    if fd != 0 {
        let b = [1u8];
        sys(SYS_WRITE, &[fd, b.as_ptr() as u64, 1]);
    }
}

global_asm!(".globl socket_test_restorer", "socket_test_restorer:", "mov eax, 15", "syscall", "ud2");

unsafe extern "C" {
    fn socket_test_restorer();
}

fn handle(signal: u64, handler: u64, flags: u64) -> i64 {
    let act =
        SigAction { handler, flags: flags | SA_RESTORER, restorer: socket_test_restorer as *const () as u64, mask: 0 };
    sys(SYS_RT_SIGACTION, &[signal, &raw const act as u64, 0, 8])
}

/// A one-shot `ITIMER_REAL` in `ms` milliseconds.
fn alarm_ms(ms: u64) {
    let it: [i64; 4] = [0, 0, 0, (ms * 1000) as i64];
    sys(SYS_SETITIMER, &[ITIMER_REAL, it.as_ptr() as u64, 0]);
}

fn listener() -> (i64, [u8; 16]) {
    let l = socket(AF_INET, SOCK_STREAM, 0);
    let a = sin([127, 0, 0, 1], 0);
    bind(l, &a, 16);
    listen(l, 4);
    (l, sin([127, 0, 0, 1], port_of(l)))
}

fn interrupted_accept() {
    // Without SA_RESTART: EINTR.
    check(b"sigaction_sigalrm", handle(SIGALRM, on_alarm as *const () as u64, 0) == 0, 0);
    let (l, _) = listener();
    ALARMS.store(0, SeqCst);
    alarm_ms(50);
    let r = accept(l);
    check(b"blocked_accept_interrupted_with_eintr", r == -EINTR && ALARMS.load(SeqCst) == 1, r);
    // With SA_RESTART: the accept goes on and takes the connection the
    // child makes only after the handler ran (it waits for the handler's
    // byte on a pipe).
    let mut p = [0i32; 2];
    sys(SYS_PIPE2, &[p.as_mut_ptr() as u64, 0]);
    let (l2, to) = listener();
    let pid = fork();
    if pid == 0 {
        let mut b = [0u8; 1];
        if read(i64::from(p[0]), &mut b) != 1 {
            exit(30);
        }
        let c = socket(AF_INET, SOCK_STREAM, 0);
        exit(if connect(c, &to, 16) == 0 { 0 } else { 31 });
    }
    WAKE_FD.store(p[1] as u64, SeqCst);
    handle(SIGALRM, on_alarm as *const () as u64, SA_RESTART);
    ALARMS.store(0, SeqCst);
    alarm_ms(50);
    let r = accept(l2);
    check(b"blocked_accept_restarted_with_sa_restart", r >= 0 && ALARMS.load(SeqCst) == 1, r);
    WAKE_FD.store(0, SeqCst);
    check(b"restart_client_process", wait(pid) == 0, 0);
    close(r);
    close(l);
    close(l2);
    close(i64::from(p[0]));
    close(i64::from(p[1]));
}

extern "C" fn on_pipe(_: i32) {}

fn sigpipe() {
    // A handler while it is blocked: as init (pid 1) the program would not
    // even be sent a signal with the default action (kill(2)).
    handle(SIGPIPE, on_pipe as *const () as u64, 0);
    let mask: u64 = 1 << (SIGPIPE - 1);
    sys(SYS_RT_SIGPROCMASK, &[SIG_BLOCK, &raw const mask as u64, 0, 8]);
    let s = socketpair(SOCK_STREAM);
    close(s[1]);
    let r = send(s[0], b"x", MSG_NOSIGNAL);
    let mut pending = 0u64;
    sys(SYS_RT_SIGPENDING, &[&raw mut pending as u64, 8]);
    check(b"msg_nosignal_epipe_without_sigpipe", r == -EPIPE && pending & mask == 0, r);
    let r = send(s[0], b"x", 0);
    sys(SYS_RT_SIGPENDING, &[&raw mut pending as u64, 8]);
    check(b"epipe_raises_sigpipe", r == -EPIPE && pending & mask != 0, r);
    // Take it, then unblock.
    let zero = [0i64; 2];
    let mut info = [0u8; 128];
    let got = sys(SYS_RT_SIGTIMEDWAIT, &[&raw const mask as u64, info.as_mut_ptr() as u64, zero.as_ptr() as u64, 8]);
    check(b"sigpipe_dequeued", got == SIGPIPE as i64, got);
    sys(SYS_RT_SIGPROCMASK, &[SIG_UNBLOCK, &raw const mask as u64, 0, 8]);
    close(s[0]);
    // With the default action, a write kills the writer.
    let pid = fork();
    if pid == 0 {
        handle(SIGPIPE, 0, 0); // SIG_DFL
        let s = socketpair(SOCK_STREAM);
        close(s[1]);
        write(s[0], b"x");
        exit(40);
    }
    let status = wait(pid);
    check(b"sigpipe_kills_the_writer", status & 0x7f == SIGPIPE as i64, status);
}

// ---- netlink --------------------------------------------------------------------------------

const RTM_NEWLINK: u16 = 16;
const RTM_GETLINK: u16 = 18;
const RTM_NEWADDR: u16 = 20;
const RTM_GETADDR: u16 = 22;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 1;
const NLM_F_DUMP: u16 = 0x300;
const IFA_LOCAL: u16 = 2;
const IFLA_IFNAME: u16 = 3;

/// Send a dump request of `ty` (`body` after the header) and look at
/// every reply message until `NLMSG_DONE`: `f(type, payload)`.
fn dump(nl: i64, ty: u16, body: &[u8], seq: u32, f: &mut dyn FnMut(u16, &[u8])) -> bool {
    let mut req = [0u8; 64];
    let len = 16 + body.len();
    req[0..4].copy_from_slice(&(len as u32).to_le_bytes());
    req[4..6].copy_from_slice(&ty.to_le_bytes());
    req[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_le_bytes());
    req[8..12].copy_from_slice(&seq.to_le_bytes());
    req[16..len].copy_from_slice(body);
    let mut kernel = [0u8; 12];
    kernel[0..2].copy_from_slice(&(AF_NETLINK as u16).to_le_bytes());
    if sendto(nl, &req[..len], &kernel, 12) != len as i64 {
        return false;
    }
    let mut buf = [0u8; 16384];
    loop {
        let n = recv(nl, &mut buf, 0);
        if n <= 0 {
            return false;
        }
        let mut at = 0usize;
        while at + 16 <= n as usize {
            let mlen = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
            let mty = u16::from_le_bytes([buf[at + 4], buf[at + 5]]);
            let mseq = u32::from_le_bytes(buf[at + 8..at + 12].try_into().unwrap());
            if mlen < 16 || at + mlen > n as usize || mseq != seq {
                return false;
            }
            if mty == NLMSG_DONE {
                return true;
            }
            f(mty, &buf[at + 16..at + mlen]);
            at += (mlen + 3) & !3;
        }
    }
}

/// Every attribute `(type, data)` in `b`.
fn attrs(mut b: &[u8], f: &mut dyn FnMut(u16, &[u8])) {
    while b.len() >= 4 {
        let len = u16::from_le_bytes([b[0], b[1]]) as usize;
        let ty = u16::from_le_bytes([b[2], b[3]]);
        if len < 4 || len > b.len() {
            return;
        }
        f(ty, &b[4..len]);
        b = &b[((len + 3) & !3).min(b.len())..];
    }
}

/// Whether an `RTM_GETADDR` dump has the IPv4 address `want`.
fn has_address(nl: i64, want: [u8; 4], seq: u32) -> bool {
    let mut found = false;
    let body = [AF_INET as u8, 0, 0, 0, 0, 0, 0, 0];
    let ok = dump(nl, RTM_GETADDR, &body, seq, &mut |ty, p| {
        if ty == RTM_NEWADDR && p.len() >= 8 && p[0] == AF_INET as u8 {
            attrs(&p[8..], &mut |t, d| found |= t == IFA_LOCAL && d == want);
        }
    });
    ok && found
}

fn netlink() {
    let nl = socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE);
    check(b"netlink_socket", nl >= 0, nl);
    let mut addr = [0u8; 12];
    addr[0..2].copy_from_slice(&(AF_NETLINK as u16).to_le_bytes());
    check(b"netlink_bind", bind(nl, &addr, 12) == 0, 0);
    let mut me = [0u8; 12];
    let (r, len) = name(SYS_GETSOCKNAME, nl, &mut me);
    let pid = u32::from_le_bytes(me[4..8].try_into().unwrap());
    check(b"netlink_port_id_is_the_pid", r == 0 && len == 12 && i64::from(pid) == sys(SYS_GETPID, &[]), i64::from(pid));
    check(b"netlink_getaddr_lists_127_0_0_1", has_address(nl, [127, 0, 0, 1], 1), 0);
    let mut lo = false;
    let ok = dump(nl, RTM_GETLINK, &[0; 16], 2, &mut |ty, p| {
        if ty == RTM_NEWLINK && p.len() >= 16 {
            attrs(&p[16..], &mut |t, d| lo |= t == IFLA_IFNAME && d == b"lo\0");
        }
    });
    check(b"netlink_getlink_lists_lo", ok && lo, 0);
    close(nl);
}

// ---- the network boot: echo servers on the host ---------------------------------------------

/// `/etc/socket-test.conf`: `a.b.c.d port`.
fn remote_config() -> Option<([u8; 4], u16)> {
    let path = b"/etc/socket-test.conf\0";
    let fd = sys(SYS_OPEN, &[path.as_ptr() as u64, 0, 0]);
    if fd < 0 {
        return None;
    }
    let mut buf = [0u8; 64];
    let n = read(fd, &mut buf);
    close(fd);
    if n <= 0 {
        return None;
    }
    let mut ip = [0u8; 4];
    let mut port: u32 = 0;
    let (mut part, mut cur, mut in_port) = (0usize, 0u32, false);
    for &c in &buf[..n as usize] {
        match c {
            b'0'..=b'9' if in_port => port = port * 10 + u32::from(c - b'0'),
            b'0'..=b'9' => cur = cur * 10 + u32::from(c - b'0'),
            b'.' if !in_port && part < 3 => {
                ip[part] = cur as u8;
                part += 1;
                cur = 0;
            }
            b' ' if !in_port => {
                ip[part] = cur as u8;
                in_port = true;
            }
            _ => break,
        }
    }
    (in_port && port > 0 && port < 65536).then_some((ip, port as u16))
}

const REMOTE_BYTES: usize = 256 * 1024;

fn remote(ip: [u8; 4], port: u16) {
    // TCP: write and read at once under poll (the echo comes back while
    // we send; neither side may stall the other).
    let s = socket(AF_INET, SOCK_STREAM, 0);
    let to = sin(ip, port);
    check(b"remote_tcp_connect", connect(s, &to, 16) == 0, 0);
    let flags = 0o4000u64;
    sys(72, &[s as u64, 4, flags]); // fcntl(F_SETFL, O_NONBLOCK)
    let (mut sent, mut got, mut good) = (0usize, 0usize, true);
    let mut chunk = [0u8; 8192];
    let mut buf = [0u8; 8192];
    let start = mono_ns();
    while got < REMOTE_BYTES && mono_ns() - start < 60_000 * MS {
        let want = if sent < REMOTE_BYTES { POLLIN | POLLOUT } else { POLLIN };
        let ev = poll1(s, want, WAIT_MS);
        if ev <= 0 {
            good = false;
            break;
        }
        if ev & i64::from(POLLOUT) != 0 && sent < REMOTE_BYTES {
            let n = chunk.len().min(REMOTE_BYTES - sent);
            for (i, b) in chunk[..n].iter_mut().enumerate() {
                *b = pattern(sent + i);
            }
            let w = write(s, &chunk[..n]);
            if w > 0 {
                sent += w as usize;
            } else if w != -EAGAIN {
                good = false;
                break;
            }
        }
        if ev & i64::from(POLLIN) != 0 {
            let r = read(s, &mut buf);
            if r > 0 {
                for (i, &b) in buf[..r as usize].iter().enumerate() {
                    good &= b == pattern(got + i);
                }
                got += r as usize;
            } else if r != -EAGAIN {
                good = false;
                break;
            }
        }
    }
    check(b"remote_tcp_echo_256_kib", good && got == REMOTE_BYTES, got as i64);
    close(s);
    // UDP: a few datagrams (retried: UDP may drop).
    let u = socket(AF_INET, SOCK_DGRAM, 0);
    let tv: [i64; 2] = [2, 0];
    sys(SYS_SETSOCKOPT, &[u as u64, SOL_SOCKET, SO_RCVTIMEO, tv.as_ptr() as u64, 16]);
    let mut echoed = 0;
    for i in 0..3u8 {
        let msg = [b'u', b'd', b'p', b'0' + i];
        let mut ok = false;
        for _ in 0..5 {
            sendto(u, &msg, &to, 16);
            let mut back = [0u8; 16];
            if recv(u, &mut back, 0) == 4 && back[..4] == msg {
                ok = true;
                break;
            }
        }
        echoed += usize::from(ok);
    }
    check(b"remote_udp_echo", echoed == 3, echoed as i64);
    close(u);
    // The interface's own address in a netlink dump.
    let nl = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
    check(b"remote_netlink_lists_10_0_2_15", has_address(nl, [10, 0, 2, 15], 7), 0);
    close(nl);
    write_out(b"socket-test: remote ok\n");
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    tcp_loopback();
    udp_loopback();
    unix_stream();
    unix_pairs_and_rights();
    interrupted_accept();
    sigpipe();
    netlink();
    if let Some((ip, port)) = remote_config() {
        remote(ip, port);
    }
    let failed = FAILED.load(SeqCst);
    write_out(b"socket-test: done, ");
    write_num(failed as i64);
    write_out(b" failed\n");
    exit(failed)
}

global_asm!(".globl _start", "_start:", "xor ebp, ebp", "and rsp, -16", "call main", "ud2");

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    write_out(b"socket-test: FAIL panic\n");
    exit(99)
}
