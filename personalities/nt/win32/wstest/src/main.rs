//! `wstest.exe` (ADR 0095): the program the kernel's ring-3 test
//! `nt_winsock_program_talks_over_loopback` runs. It imports from our
//! `ws2_32.dll` and `kernel32.dll` and checks Winsock's documented
//! behaviour (Learn's pages) over the loopback interface: start-up and
//! clean-up, byte order and address text, `getaddrinfo` / `GetAddrInfoW`
//! / `getnameinfo` / `gethostbyname` for `localhost`, TCP over 127.0.0.1
//! and `::1` (in one thread, and with an echo server in a second thread
//! that blocks in `accept` and `recv`), UDP with `sendto` / `recvfrom`,
//! `select`, `WSAPoll`, `ioctlsocket`, options and timeouts, non-blocking
//! connects and receives, an overlapped `WSARecv` completed by the
//! kernel's worker, and the WSA error codes of the usual mistakes.
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `wstest: step N failed (got G)` and exits with `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;
type Socket = usize;
const INVALID_SOCKET: Socket = !0;
const INVALID_HANDLE_VALUE: Handle = -1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetStdHandle(which: u32) -> Handle;
    fn WriteFile(h: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut u8) -> i32;
    fn CreateThread(
        sa: *const u8,
        stack: usize,
        start: extern "system" fn(*mut u8) -> u32,
        p: *mut u8,
        flags: u32,
        id: *mut u32,
    ) -> Handle;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn GetExitCodeThread(h: Handle, code: *mut u32) -> i32;
    fn CloseHandle(h: Handle) -> i32;
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSACleanup() -> i32;
    fn WSAGetLastError() -> i32;
    fn WSASetLastError(e: i32);
    fn socket(af: i32, ty: i32, protocol: i32) -> Socket;
    fn bind(s: Socket, name: *const u8, len: i32) -> i32;
    fn listen(s: Socket, backlog: i32) -> i32;
    fn accept(s: Socket, addr: *mut u8, len: *mut i32) -> Socket;
    fn connect(s: Socket, name: *const u8, len: i32) -> i32;
    fn send(s: Socket, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: Socket, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn sendto(s: Socket, buf: *const u8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32;
    fn recvfrom(s: Socket, buf: *mut u8, len: i32, flags: i32, from: *mut u8, fromlen: *mut i32) -> i32;
    fn closesocket(s: Socket) -> i32;
    fn shutdown(s: Socket, how: i32) -> i32;
    fn select(n: i32, r: *mut u8, w: *mut u8, e: *mut u8, timeout: *const i32) -> i32;
    fn WSAPoll(fds: *mut u8, n: u32, timeout: i32) -> i32;
    fn ioctlsocket(s: Socket, cmd: i32, arg: *mut u32) -> i32;
    fn setsockopt(s: Socket, level: i32, name: i32, value: *const u8, len: i32) -> i32;
    fn getsockopt(s: Socket, level: i32, name: i32, value: *mut u8, len: *mut i32) -> i32;
    fn getsockname(s: Socket, name: *mut u8, len: *mut i32) -> i32;
    fn getpeername(s: Socket, name: *mut u8, len: *mut i32) -> i32;
    fn getaddrinfo(node: *const u8, service: *const u8, hints: *const u8, result: *mut *mut u8) -> i32;
    fn freeaddrinfo(list: *mut u8);
    fn GetAddrInfoW(node: *const u16, service: *const u16, hints: *const u8, result: *mut *mut u8) -> i32;
    fn FreeAddrInfoW(list: *mut u8);
    fn getnameinfo(sa: *const u8, len: i32, host: *mut u8, hl: u32, serv: *mut u8, sl: u32, flags: i32) -> i32;
    fn gethostbyname(name: *const u8) -> *const u8;
    fn gethostname(name: *mut u8, len: i32) -> i32;
    fn inet_addr(s: *const u8) -> u32;
    fn inet_ntoa(a: u32) -> *const u8;
    fn inet_pton(af: i32, s: *const u8, out: *mut u8) -> i32;
    fn inet_ntop(af: i32, a: *const u8, out: *mut u8, len: usize) -> *const u8;
    fn InetPtonW(af: i32, s: *const u16, out: *mut u8) -> i32;
    fn htons(v: u16) -> u16;
    fn htonl(v: u32) -> u32;
    fn ntohs(v: u16) -> u16;
    fn WSARecv(
        s: Socket,
        bufs: *const WsaBuf,
        n: u32,
        received: *mut u32,
        flags: *mut u32,
        ov: *mut Overlapped,
        routine: *const u8,
    ) -> i32;
    fn WSASend(
        s: Socket,
        bufs: *const WsaBuf,
        n: u32,
        sent: *mut u32,
        flags: u32,
        ov: *mut Overlapped,
        routine: *const u8,
    ) -> i32;
    fn WSAGetOverlappedResult(s: Socket, ov: *mut Overlapped, n: *mut u32, wait: i32, flags: *mut u32) -> i32;
    fn WSACreateEvent() -> Handle;
    fn WSACloseEvent(h: Handle) -> i32;
    fn WSAWaitForMultipleEvents(n: u32, events: *const Handle, all: i32, ms: u32, alertable: i32) -> u32;
    fn WSAAddressToStringA(sa: *const u8, len: u32, info: *const u8, out: *mut u8, outlen: *mut u32) -> i32;
}

#[repr(C)]
struct WsaBuf {
    len: u32,
    buf: *mut u8,
}

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u64,
    event: Handle,
}

/// The exit status when every check passed.
const PASS: u32 = 0x57;

const AF_INET: i32 = 2;
const AF_INET6: i32 = 23;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_TCP: i32 = 6;
const SOL_SOCKET: i32 = 0xffff;
const SO_REUSEADDR: i32 = 4;
const SO_TYPE: i32 = 0x1008;
const SO_RCVTIMEO: i32 = 0x1006;
const SO_ERROR: i32 = 0x1007;
const TCP_NODELAY: i32 = 1;
const SD_SEND: i32 = 1;
const MSG_PEEK: i32 = 2;
const FIONBIO: i32 = 0x8004_667Eu32 as i32;
const FIONREAD: i32 = 0x4004_667F;
const POLLRDNORM: i16 = 0x100;
const POLLWRNORM: i16 = 0x10;
const NI_NUMERICHOST: i32 = 2;
const NI_NUMERICSERV: i32 = 8;
const WSAEFAULT: u32 = 10014;
const WSAEINVAL: u32 = 10022;
const WSAEWOULDBLOCK: u32 = 10035;
const WSAENOTSOCK: u32 = 10038;
const WSAEMSGSIZE: u32 = 10040;
const WSAENOPROTOOPT: u32 = 10042;
const WSAEAFNOSUPPORT: u32 = 10047;
const WSAEADDRINUSE: u32 = 10048;
const WSAENOTCONN: u32 = 10057;
const WSAETIMEDOUT: u32 = 10060;
const WSAECONNREFUSED: u32 = 10061;
const WSAVERNOTSUPPORTED: u32 = 10092;
const WSANOTINITIALISED: u32 = 10093;
const WSA_IO_PENDING: u32 = 997;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

/// A NUL-terminated ASCII literal.
macro_rules! a {
    ($s:literal) => {
        concat!($s, "\0").as_ptr()
    };
}

/// A NUL-terminated UTF-16 copy of an ASCII literal.
macro_rules! w {
    ($s:literal) => {{
        const S: &[u8] = $s.as_bytes();
        const W: [u16; S.len() + 1] = {
            let mut o = [0u16; S.len() + 1];
            let mut i = 0;
            while i < S.len() {
                o[i] = S[i] as u16;
                i += 1;
            }
            o
        };
        W.as_ptr()
    }};
}

// ----- reporting

fn out(bytes: &[u8]) {
    // SAFETY: kernel32 calls with valid buffers.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h != 0 && h != INVALID_HANDLE_VALUE {
            let mut n = 0;
            WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut n, core::ptr::null_mut());
        }
    }
}

fn out_num(mut v: u32) {
    let mut b = [0u8; 10];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out(&b[i..]);
}

static STEP: AtomicU32 = AtomicU32::new(0);

fn check_got(ok: bool, got: u32) {
    let step = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    if !ok {
        out(b"wstest: step ");
        out_num(step);
        out(b" failed (got ");
        out_num(got);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

fn last() -> u32 {
    // SAFETY: a plain ws2_32 call.
    unsafe { WSAGetLastError() as u32 }
}

/// Check `ok`, reporting the last WSA error on failure.
fn check(ok: bool) {
    check_got(ok, last());
}

/// Check that a call returning `SOCKET_ERROR` failed with `want`.
fn check_fail(r: i32, want: u32) {
    let e = last();
    check_got(r == -1 && e == want, if r == -1 { e } else { 0xFFFF });
}

// ----- addresses

fn sin(ip: [u8; 4], port: u16) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = AF_INET as u8;
    b[2..4].copy_from_slice(&port.to_be_bytes());
    b[4..8].copy_from_slice(&ip);
    b
}

fn sin6_loopback(port: u16) -> [u8; 28] {
    let mut b = [0u8; 28];
    b[0] = AF_INET6 as u8;
    b[2..4].copy_from_slice(&port.to_be_bytes());
    b[23] = 1;
    b
}

const LOOPBACK: [u8; 4] = [127, 0, 0, 1];

unsafe fn port_of(s: Socket) -> u16 {
    let mut b = [0u8; 28];
    let mut n = 28;
    // SAFETY: a local buffer.
    check(unsafe { getsockname(s, b.as_mut_ptr(), &mut n) } == 0);
    u16::from_be_bytes([b[2], b[3]])
}

fn cstr_eq(p: *const u8, want: &[u8]) -> bool {
    if p.is_null() {
        return false;
    }
    // SAFETY: a NUL-terminated string of the DLL's or ours.
    unsafe { (0..=want.len()).all(|i| *p.add(i) == if i == want.len() { 0 } else { want[i] }) }
}

// ----- the checks

unsafe fn startup() {
    let mut data = [0u8; 408];
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        // Before WSAStartup: WSANOTINITIALISED.
        check_got(socket(AF_INET, SOCK_STREAM, 0) == INVALID_SOCKET && last() == WSANOTINITIALISED, last());
        check_got(WSAStartup(0x0100, data.as_mut_ptr()) as u32 == WSAVERNOTSUPPORTED, 0);
        check_got(WSAStartup(0x0202, data.as_mut_ptr()) == 0, 1);
        check_got(u16::from_le_bytes([data[0], data[1]]) == 0x0202, u32::from(data[0]));
        check(cstr_eq(data.as_ptr().add(16), b"WinSock 2.0"));
        // A second start, matched by a second clean-up at the end.
        check(WSAStartup(0x0101, data.as_mut_ptr()) == 0);
        check(u16::from_le_bytes([data[0], data[1]]) == 0x0101);
        WSASetLastError(1234);
        check(last() == 1234);
    }
}

unsafe fn text() {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        check(htons(0x1234) == 0x3412 && ntohs(0x3412) == 0x1234 && htonl(1) == 0x0100_0000);
        check(inet_addr(a!("127.0.0.1")) == u32::from_ne_bytes(LOOPBACK));
        check(inet_addr(a!("127.1")) == u32::from_ne_bytes(LOOPBACK));
        check(inet_addr(a!("not an address")) == 0xffff_ffff);
        check(cstr_eq(inet_ntoa(u32::from_ne_bytes([10, 0, 2, 15])), b"10.0.2.15"));
        let mut b6 = [0u8; 16];
        check(inet_pton(AF_INET6, a!("2001:db8::1"), b6.as_mut_ptr()) == 1);
        check(b6[0] == 0x20 && b6[1] == 0x01 && b6[15] == 1);
        check(inet_pton(AF_INET, a!("1.2.3"), b6.as_mut_ptr()) == 0);
        check_fail(inet_pton(7, a!("1.2.3.4"), b6.as_mut_ptr()), WSAEAFNOSUPPORT);
        let mut t = [0u8; 64];
        check(cstr_eq(inet_ntop(AF_INET6, b6.as_ptr(), t.as_mut_ptr(), 64), b"2001:db8::1"));
        let mut b4 = [0u8; 4];
        check(InetPtonW(AF_INET, w!("192.168.0.1"), b4.as_mut_ptr()) == 1 && b4 == [192, 168, 0, 1]);
        let sa = sin(LOOPBACK, 8080);
        let mut n = 64u32;
        check(WSAAddressToStringA(sa.as_ptr(), 16, core::ptr::null(), t.as_mut_ptr(), &mut n) == 0);
        check(cstr_eq(t.as_ptr(), b"127.0.0.1:8080"));
    }
}

unsafe fn ai_addr(ai: *const u8) -> *const u8 {
    // SAFETY: an ADDRINFO of ws2_32's.
    unsafe { core::ptr::read_unaligned(ai.add(32).cast::<*const u8>()) }
}

unsafe fn names() {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        let mut hints = [0u8; 48];
        hints[8] = SOCK_STREAM as u8;
        let mut list: *mut u8 = core::ptr::null_mut();
        check(getaddrinfo(a!("localhost"), a!("8080"), hints.as_ptr(), &mut list) == 0);
        // IPv6 first, then IPv4, both port 8080 over TCP.
        check(!list.is_null());
        check(core::ptr::read_unaligned(list.add(4).cast::<i32>()) == AF_INET6);
        let next = core::ptr::read_unaligned(list.add(40).cast::<*mut u8>());
        check(!next.is_null());
        check(core::ptr::read_unaligned(next.add(4).cast::<i32>()) == AF_INET);
        check(core::ptr::read_unaligned(next.add(12).cast::<i32>()) == IPPROTO_TCP);
        let sa = ai_addr(next);
        check(*sa.add(2) == 0x1f && *sa.add(3) == 0x90 && *sa.add(4) == 127);
        freeaddrinfo(list);
        // AF_INET only, by service name.
        hints[4] = AF_INET as u8;
        check(getaddrinfo(a!("LOCALHOST"), a!("http"), hints.as_ptr(), &mut list) == 0);
        check(core::ptr::read_unaligned(list.add(40).cast::<*mut u8>()).is_null());
        check(*ai_addr(list).add(3) == 80);
        freeaddrinfo(list);
        // The wide form, numeric.
        check(GetAddrInfoW(w!("127.0.0.1"), w!("53"), hints.as_ptr(), &mut list) == 0);
        check(*ai_addr(list).add(3) == 53);
        FreeAddrInfoW(list);
        // A numeric IPv6 host with AF_INET: not found; no node and no service.
        check(getaddrinfo(a!("::1"), core::ptr::null(), hints.as_ptr(), &mut list) == 11001);
        check(getaddrinfo(core::ptr::null(), core::ptr::null(), hints.as_ptr(), &mut list) == 11001);
        check(list.is_null());
        // getnameinfo.
        let sa = sin(LOOPBACK, 80);
        let (mut host, mut serv) = ([0u8; 64], [0u8; 16]);
        check(getnameinfo(sa.as_ptr(), 16, host.as_mut_ptr(), 64, serv.as_mut_ptr(), 16, 0) == 0);
        check(cstr_eq(host.as_ptr(), b"localhost") && cstr_eq(serv.as_ptr(), b"http"));
        let flags = NI_NUMERICHOST | NI_NUMERICSERV;
        check(getnameinfo(sa.as_ptr(), 16, host.as_mut_ptr(), 64, serv.as_mut_ptr(), 16, flags) == 0);
        check(cstr_eq(host.as_ptr(), b"127.0.0.1") && cstr_eq(serv.as_ptr(), b"80"));
        check(getnameinfo(sa.as_ptr(), 16, host.as_mut_ptr(), 4, serv.as_mut_ptr(), 16, flags) as u32 == WSAEFAULT);
        // gethostname and gethostbyname.
        check(gethostname(host.as_mut_ptr(), 64) == 0 && host[0] != 0);
        check_fail(gethostname(host.as_mut_ptr(), 1), WSAEFAULT);
        let he = gethostbyname(a!("localhost"));
        check(!he.is_null());
        check(core::ptr::read_unaligned(he.add(16).cast::<i16>()) == 2);
        let list = core::ptr::read_unaligned(he.add(24).cast::<*const *const u8>());
        check(core::slice::from_raw_parts(*list, 4) == LOOPBACK);
    }
}

/// A listener on `127.0.0.1` (port chosen by the stack).
unsafe fn listener() -> (Socket, u16) {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        let l = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        check(l != INVALID_SOCKET);
        let sa = sin(LOOPBACK, 0);
        check(bind(l, sa.as_ptr(), 16) == 0);
        check(listen(l, 5) == 0);
        let port = port_of(l);
        check(port != 0);
        (l, port)
    }
}

unsafe fn tcp_one_thread() {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        let (l, port) = listener();
        let c = socket(AF_INET, SOCK_STREAM, 0);
        let to = sin(LOOPBACK, port);
        check(connect(c, to.as_ptr(), 16) == 0);
        let mut peer = [0u8; 16];
        let mut n = 16;
        let a = accept(l, peer.as_mut_ptr(), &mut n);
        check(a != INVALID_SOCKET && n == 16 && peer[4..8] == LOOPBACK);
        check(u16::from_be_bytes([peer[2], peer[3]]) == port_of(c));
        let mut pn = 16;
        check(getpeername(c, peer.as_mut_ptr(), &mut pn) == 0 && u16::from_be_bytes([peer[2], peer[3]]) == port);
        check(send(c, b"ping".as_ptr(), 4, 0) == 4);
        let mut avail = 0u32;
        check(ioctlsocket(a, FIONREAD, &mut avail) == 0 && avail == 4);
        // select sees it readable; WSAPoll too.
        let mut set = [0u8; 16];
        set[0] = 1;
        set[8..16].copy_from_slice(&a.to_le_bytes());
        let tv = [1i32, 0];
        check(select(0, set.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), tv.as_ptr()) == 1);
        let mut fds = [0u8; 32];
        fds[0..8].copy_from_slice(&a.to_le_bytes());
        fds[8..10].copy_from_slice(&POLLRDNORM.to_le_bytes());
        fds[16..24].copy_from_slice(&c.to_le_bytes());
        fds[24..26].copy_from_slice(&POLLWRNORM.to_le_bytes());
        check(WSAPoll(fds.as_mut_ptr(), 2, 0) == 2);
        check(i16::from_le_bytes([fds[10], fds[11]]) == POLLRDNORM);
        check(i16::from_le_bytes([fds[26], fds[27]]) == POLLWRNORM);
        let mut buf = [0u8; 16];
        check(recv(a, buf.as_mut_ptr(), 16, MSG_PEEK) == 4);
        check(recv(a, buf.as_mut_ptr(), 16, 0) == 4 && &buf[..4] == b"ping");
        // Echo back.
        check(send(a, buf.as_ptr(), 4, 0) == 4);
        check(recv(c, buf.as_mut_ptr(), 16, 0) == 4 && &buf[..4] == b"ping");
        // Non-blocking and empty: WSAEWOULDBLOCK; select times out.
        let mut on = 1u32;
        check(ioctlsocket(a, FIONBIO, &mut on) == 0);
        check_fail(recv(a, buf.as_mut_ptr(), 16, 0), WSAEWOULDBLOCK);
        set[0] = 1;
        let tv0 = [0i32, 1000];
        check(select(0, set.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), tv0.as_ptr()) == 0);
        check(set[0] == 0);
        let mut off = 0u32;
        check(ioctlsocket(a, FIONBIO, &mut off) == 0);
        // Options.
        let mut v = [0u8; 4];
        let mut vl = 4;
        check(getsockopt(a, SOL_SOCKET, SO_TYPE, v.as_mut_ptr(), &mut vl) == 0 && v == 1i32.to_le_bytes());
        check(setsockopt(c, IPPROTO_TCP, TCP_NODELAY, 1i32.to_le_bytes().as_ptr(), 4) == 0);
        vl = 4;
        check(getsockopt(c, IPPROTO_TCP, TCP_NODELAY, v.as_mut_ptr(), &mut vl) == 0 && v[0] == 1);
        check_fail(setsockopt(c, SOL_SOCKET, 0x7777, v.as_ptr(), 4), WSAENOPROTOOPT);
        vl = 4;
        check(getsockopt(c, SOL_SOCKET, SO_ERROR, v.as_mut_ptr(), &mut vl) == 0 && v == [0; 4]);
        // A receive that times out.
        check(setsockopt(a, SOL_SOCKET, SO_RCVTIMEO, 50u32.to_le_bytes().as_ptr(), 4) == 0);
        check_fail(recv(a, buf.as_mut_ptr(), 16, 0), WSAETIMEDOUT);
        // Shut down sending: the other end reads the end of the stream.
        check(shutdown(c, SD_SEND) == 0);
        check(recv(a, buf.as_mut_ptr(), 16, 0) == 0);
        for s in [a, c, l] {
            check(closesocket(s) == 0);
        }
        // A closed handle is no socket any more.
        check_fail(closesocket(a), WSAENOTSOCK);
    }
}

unsafe fn tcp_ipv6() {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        let l = socket(AF_INET6, SOCK_STREAM, 0);
        check(l != INVALID_SOCKET);
        check(bind(l, sin6_loopback(0).as_ptr(), 28) == 0);
        check(listen(l, 1) == 0);
        let port = port_of(l);
        let c = socket(AF_INET6, SOCK_STREAM, 0);
        check(connect(c, sin6_loopback(port).as_ptr(), 28) == 0);
        let a = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
        check(a != INVALID_SOCKET);
        check(send(a, b"six".as_ptr(), 3, 0) == 3);
        let mut buf = [0u8; 4];
        check(recv(c, buf.as_mut_ptr(), 4, 0) == 3 && &buf[..3] == b"six");
        // An IPv4 address on an IPv6 socket: WSAEFAULT for bind.
        let s = socket(AF_INET6, SOCK_DGRAM, 0);
        check_fail(bind(s, sin(LOOPBACK, 0).as_ptr(), 16), WSAEFAULT);
        for s in [a, c, l, s] {
            closesocket(s);
        }
    }
}

/// The echo server thread: accept one connection on the listener passed
/// in, echo until the end of the stream, exit with the byte count.
extern "system" fn echo_server(p: *mut u8) -> u32 {
    let l = p as Socket;
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        let a = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
        if a == INVALID_SOCKET {
            return 0xFFFF_FFFF;
        }
        let mut buf = [0u8; 4096];
        let mut total = 0u32;
        loop {
            let n = recv(a, buf.as_mut_ptr(), buf.len() as i32, 0);
            if n <= 0 {
                break;
            }
            let mut at = 0;
            while at < n {
                let k = send(a, buf.as_ptr().add(at as usize), n - at, 0);
                if k <= 0 {
                    return 0xFFFF_FFFE;
                }
                at += k;
            }
            total += n as u32;
        }
        closesocket(a);
        total
    }
}

/// 64 KiB through an echo server in another thread: the server blocks in
/// `accept` and `recv` until this thread connects and sends.
unsafe fn tcp_two_threads() {
    const SIZE: usize = 64 * 1024;
    // SAFETY: ws2_32 and kernel32 calls with valid buffers.
    unsafe {
        let (l, port) = listener();
        let t = CreateThread(core::ptr::null(), 0, echo_server, l as *mut u8, 0, core::ptr::null_mut());
        check(t != 0);
        let c = socket(AF_INET, SOCK_STREAM, 0);
        check(connect(c, sin(LOOPBACK, port).as_ptr(), 16) == 0);
        let mut data = [0u8; 4096];
        let mut back = [0u8; 4096];
        let mut sent = 0;
        let mut got = 0;
        let mut ok = true;
        while got < SIZE {
            if sent < SIZE {
                for (i, b) in data.iter_mut().enumerate() {
                    *b = ((sent + i) % 251) as u8;
                }
                let n = send(c, data.as_ptr(), data.len() as i32, 0);
                if n <= 0 {
                    ok = false;
                    break;
                }
                sent += n as usize;
            }
            while got < sent {
                let n = recv(c, back.as_mut_ptr(), back.len() as i32, 0);
                if n <= 0 {
                    ok = false;
                    break;
                }
                ok &= back[..n as usize].iter().enumerate().all(|(i, &b)| b == ((got + i) % 251) as u8);
                got += n as usize;
            }
            if !ok {
                break;
            }
        }
        check_got(ok && got == SIZE, got as u32);
        check(shutdown(c, SD_SEND) == 0);
        check(WaitForSingleObject(t, 20_000) == 0);
        let mut code = 0;
        check(GetExitCodeThread(t, &mut code) != 0);
        check_got(code == SIZE as u32, code);
        CloseHandle(t);
        closesocket(c);
        closesocket(l);
    }
}

unsafe fn udp() {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        let a = socket(AF_INET, SOCK_DGRAM, 0);
        let b = socket(AF_INET, SOCK_DGRAM, 0);
        check(a != INVALID_SOCKET && b != INVALID_SOCKET);
        check(bind(a, sin(LOOPBACK, 0).as_ptr(), 16) == 0);
        check(bind(b, sin(LOOPBACK, 0).as_ptr(), 16) == 0);
        let (pa, pb) = (port_of(a), port_of(b));
        let to = sin(LOOPBACK, pb);
        check(sendto(a, b"datagram".as_ptr(), 8, 0, to.as_ptr(), 16) == 8);
        let mut buf = [0u8; 32];
        let mut from = [0u8; 16];
        let mut fl = 16;
        check(recvfrom(b, buf.as_mut_ptr(), 32, 0, from.as_mut_ptr(), &mut fl) == 8);
        check(&buf[..8] == b"datagram" && fl == 16 && u16::from_be_bytes([from[2], from[3]]) == pa);
        // Too long for the buffer: WSAEMSGSIZE, the start kept.
        check(sendto(a, b"0123456789".as_ptr(), 10, 0, to.as_ptr(), 16) == 10);
        check_fail(recv(b, buf.as_mut_ptr(), 4, 0), WSAEMSGSIZE);
        check(&buf[..4] == b"0123");
        // A destination address too short: WSAEFAULT.
        check_fail(sendto(a, b"x".as_ptr(), 1, 0, to.as_ptr(), 8), WSAEFAULT);
        // Two bound to one port: WSAEADDRINUSE (without SO_REUSEADDR).
        let c = socket(AF_INET, SOCK_DGRAM, 0);
        check_fail(bind(c, sin(LOOPBACK, pb).as_ptr(), 16), WSAEADDRINUSE);
        let _ = SO_REUSEADDR;
        for s in [a, b, c] {
            closesocket(s);
        }
    }
}

unsafe fn errors() {
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        // Nobody listens: refused.
        let (l, port) = listener();
        closesocket(l);
        let c = socket(AF_INET, SOCK_STREAM, 0);
        check_fail(connect(c, sin(LOOPBACK, port).as_ptr(), 16), WSAECONNREFUSED);
        closesocket(c);
        // Unbound: getsockname is WSAEINVAL; unconnected: getpeername and
        // recv are WSAENOTCONN.
        let s = socket(AF_INET, SOCK_STREAM, 0);
        let mut b = [0u8; 16];
        let mut n = 16;
        check_fail(getsockname(s, b.as_mut_ptr(), &mut n), WSAEINVAL);
        n = 16;
        check_fail(getpeername(s, b.as_mut_ptr(), &mut n), WSAENOTCONN);
        check_fail(recv(s, b.as_mut_ptr(), 16, 0), WSAENOTCONN);
        check_fail(listen(s, 1), WSAEINVAL);
        // A non-blocking connect: WSAEWOULDBLOCK, then writable in select.
        let (l, port) = listener();
        let mut on = 1u32;
        check(ioctlsocket(s, FIONBIO, &mut on) == 0);
        check_fail(connect(s, sin(LOOPBACK, port).as_ptr(), 16), WSAEWOULDBLOCK);
        let mut set = [0u8; 16];
        set[0] = 1;
        set[8..16].copy_from_slice(&s.to_le_bytes());
        let tv = [2i32, 0];
        check(select(0, core::ptr::null_mut(), set.as_mut_ptr(), core::ptr::null_mut(), tv.as_ptr()) == 1);
        // A handle that is not a socket, and empty sets.
        check_fail(closesocket(0x7ffc), WSAENOTSOCK);
        check_fail(
            select(0, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), tv.as_ptr()),
            WSAEINVAL,
        );
        check(socket(AF_INET, 3, 0) == INVALID_SOCKET && last() == 10044);
        check(socket(5, SOCK_STREAM, 0) == INVALID_SOCKET && last() == WSAEAFNOSUPPORT);
        closesocket(s);
        closesocket(l);
    }
}

/// An overlapped receive posted before the data exists: pending, then
/// completed by the kernel's worker when the peer sends.
unsafe fn overlapped() {
    // SAFETY: ws2_32 calls with valid buffers that outlive the request.
    unsafe {
        let (l, port) = listener();
        let c = socket(AF_INET, SOCK_STREAM, 0);
        check(connect(c, sin(LOOPBACK, port).as_ptr(), 16) == 0);
        let a = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
        check(a != INVALID_SOCKET);
        let ev = WSACreateEvent();
        check(ev != 0);
        let mut buf = [0u8; 16];
        let wb = WsaBuf { len: 16, buf: buf.as_mut_ptr() };
        let mut ov = Overlapped { internal: 0, internal_high: 0, offset: 0, event: ev };
        let (mut n, mut flags) = (0u32, 0u32);
        check_fail(WSARecv(a, &wb, 1, &mut n, &mut flags, &mut ov, core::ptr::null()), WSA_IO_PENDING);
        // Not done yet.
        check(WSAGetOverlappedResult(a, &mut ov, &mut n, 0, &mut flags) == 0 && last() == 996);
        check(send(c, b"later".as_ptr(), 5, 0) == 5);
        check(WSAWaitForMultipleEvents(1, &ev, 1, 10_000, 0) == 0);
        check(WSAGetOverlappedResult(a, &mut ov, &mut n, 1, &mut flags) == 1);
        check_got(n == 5 && &buf[..5] == b"later", n);
        // An overlapped send finishes at once.
        let sb = WsaBuf { len: 3, buf: b"abc".as_ptr().cast_mut() };
        let mut ov2 = Overlapped { internal: 0, internal_high: 0, offset: 0, event: ev };
        check(WSASend(c, &sb, 1, &mut n, 0, &mut ov2, core::ptr::null()) == 0 && n == 3);
        check(recv(a, buf.as_mut_ptr(), 16, 0) == 3);
        WSACloseEvent(ev);
        for s in [a, c, l] {
            closesocket(s);
        }
    }
}

unsafe fn cleanup() {
    // SAFETY: plain ws2_32 calls.
    unsafe {
        check(WSACleanup() == 0);
        check(WSACleanup() == 0);
        check_fail(WSACleanup(), WSANOTINITIALISED);
    }
}

/// A group of checks and the name it is announced by.
type Section = (&'static [u8], unsafe fn());

#[unsafe(no_mangle)]
pub extern "system" fn wstest_main(_peb: *mut u8) -> u32 {
    out(b"wstest: start\n");
    // SAFETY: every call is a ws2_32/kernel32 export with valid arguments.
    unsafe {
        let sections: [Section; 10] = [
            (b"start-up", startup),
            (b"address text", text),
            (b"names", names),
            (b"TCP in one thread", tcp_one_thread),
            (b"TCP over IPv6", tcp_ipv6),
            (b"TCP with an echo thread", tcp_two_threads),
            (b"UDP", udp),
            (b"errors", errors),
            (b"overlapped I/O", overlapped),
            (b"clean-up", cleanup),
        ];
        for (name, run) in sections {
            out(b"wstest: ");
            out(name);
            out(b"\n");
            run();
        }
        out(b"wstest: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}
