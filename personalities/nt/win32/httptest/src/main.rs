//! `httptest.exe` (ADR 0095): the program the kernel's ring-3 test
//! `nt_http_get_through_winhttp_and_wininet` runs. A second thread is a
//! tiny HTTP server on 127.0.0.1 (our `ws2_32`); the first makes GET
//! requests to it through our `winhttp.dll` and `wininet.dll` and checks
//! what Learn documents: status, header queries (numbers, custom fields,
//! raw headers, a buffer too small, a field that is not there), bodies
//! framed by `Content-Length`, chunks and the end of the connection,
//! `localhost` resolved (IPv6 refused, IPv4 taken), and the errors for
//! `https` (no TLS yet) and for other schemes. The server checks the
//! requests' `Host` and `User-Agent` fields.
//!
//! It exits through `ExitProcess(PASS)` when every check passed; a failed
//! check prints `httptest: step N failed (got G)` and exits with
//! `0x80 + N`.

#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use poly_win32_rt as _;

type Handle = isize;
type Socket = usize;
const INVALID_HANDLE_VALUE: Handle = -1;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn GetLastError() -> u32;
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
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn socket(af: i32, ty: i32, protocol: i32) -> Socket;
    fn bind(s: Socket, name: *const u8, len: i32) -> i32;
    fn listen(s: Socket, backlog: i32) -> i32;
    fn accept(s: Socket, addr: *mut u8, len: *mut i32) -> Socket;
    fn send(s: Socket, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: Socket, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn closesocket(s: Socket) -> i32;
    fn getsockname(s: Socket, name: *mut u8, len: *mut i32) -> i32;
}

#[link(name = "winhttp", kind = "raw-dylib")]
unsafe extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> Handle;
    fn WinHttpConnect(session: Handle, server: *const u16, port: u16, reserved: u32) -> Handle;
    fn WinHttpOpenRequest(
        c: Handle,
        verb: *const u16,
        object: *const u16,
        version: *const u16,
        referrer: *const u16,
        accept: *const *const u16,
        flags: u32,
    ) -> Handle;
    fn WinHttpSendRequest(
        r: Handle,
        headers: *const u16,
        len: u32,
        optional: *const u8,
        olen: u32,
        total: u32,
        ctx: usize,
    ) -> i32;
    fn WinHttpReceiveResponse(r: Handle, reserved: *mut u8) -> i32;
    fn WinHttpQueryHeaders(
        r: Handle,
        level: u32,
        name: *const u16,
        buf: *mut u8,
        len: *mut u32,
        index: *mut u32,
    ) -> i32;
    fn WinHttpQueryDataAvailable(r: Handle, n: *mut u32) -> i32;
    fn WinHttpReadData(r: Handle, buf: *mut u8, n: u32, read: *mut u32) -> i32;
    fn WinHttpCloseHandle(h: Handle) -> i32;
}

#[link(name = "wininet", kind = "raw-dylib")]
unsafe extern "system" {
    fn InternetOpenW(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> Handle;
    fn InternetConnectW(
        s: Handle,
        server: *const u16,
        port: u16,
        user: *const u16,
        password: *const u16,
        service: u32,
        flags: u32,
        ctx: usize,
    ) -> Handle;
    fn HttpOpenRequestW(
        c: Handle,
        verb: *const u16,
        object: *const u16,
        version: *const u16,
        referrer: *const u16,
        accept: *const *const u16,
        flags: u32,
        ctx: usize,
    ) -> Handle;
    fn HttpSendRequestW(r: Handle, headers: *const u16, len: u32, optional: *const u8, olen: u32) -> i32;
    fn HttpQueryInfoW(r: Handle, level: u32, buf: *mut u8, len: *mut u32, index: *mut u32) -> i32;
    fn InternetOpenUrlW(s: Handle, url: *const u16, headers: *const u16, len: u32, flags: u32, ctx: usize) -> Handle;
    fn InternetReadFile(r: Handle, buf: *mut u8, n: u32, read: *mut u32) -> i32;
    fn InternetCloseHandle(h: Handle) -> i32;
}

const PASS: u32 = 0x48;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const QUERY_CONTENT_TYPE: u32 = 1;
const QUERY_STATUS_CODE: u32 = 19;
const QUERY_RAW_HEADERS_CRLF: u32 = 22;
const QUERY_LOCATION: u32 = 33;
const QUERY_CUSTOM: u32 = 65535;
const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const INTERNET_SERVICE_HTTP: u32 = 3;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_UNRECOGNIZED_SCHEME: u32 = 12006;
const ERROR_HEADER_NOT_FOUND: u32 = 12150;
const ERROR_INTERNET_SECURITY_CHANNEL_ERROR: u32 = 12157;
const ERROR_WINHTTP_SECURE_FAILURE: u32 = 12175;

/// Connections the server answers before it ends.
const CONNECTIONS: u32 = 6;

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
        out(b"httptest: step ");
        out_num(step);
        out(b" failed (got ");
        out_num(got);
        out(b")\n");
        // SAFETY: never returns.
        unsafe { ExitProcess(0x80 + (step & 0x7F)) }
    }
}

fn check(ok: bool) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(ok, e);
}

fn check_fail(r: i32, want: u32) {
    // SAFETY: a plain kernel32 call.
    let e = unsafe { GetLastError() };
    check_got(r == 0 && e == want, if r == 0 { e } else { 0xFFFF });
}

// ----- the tiny server

/// What the server saw: bit 0 every request had `Host: 127.0.0.1:<port>`
/// or `localhost:<port>`, bit 1 the agents were ours, bit 2 a request
/// line was malformed.
static SEEN: AtomicU32 = AtomicU32::new(0b11);
static PORT: AtomicU32 = AtomicU32::new(0);

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

unsafe fn send_all(s: Socket, data: &[u8]) {
    let mut at = 0;
    while at < data.len() {
        // SAFETY: a live buffer.
        let n = unsafe { send(s, data.as_ptr().add(at), (data.len() - at) as i32, 0) };
        if n <= 0 {
            return;
        }
        at += n as usize;
    }
}

extern "system" fn server(p: *mut u8) -> u32 {
    let l = p as Socket;
    let mut served = 0;
    // SAFETY: ws2_32 calls with valid buffers.
    unsafe {
        while served < CONNECTIONS {
            let a = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
            if a == !0 {
                return 0xFFFF;
            }
            served += 1;
            let mut req = [0u8; 2048];
            let mut n = 0;
            while n < req.len() && !contains(&req[..n], b"\r\n\r\n") {
                let k = recv(a, req.as_mut_ptr().add(n), (req.len() - n) as i32, 0);
                if k <= 0 {
                    break;
                }
                n += k as usize;
            }
            let req = &req[..n];
            let port = PORT.load(Ordering::SeqCst);
            let mut host = [0u8; 32];
            let hl = fmt_host(&mut host, b"Host: 127.0.0.1:", port);
            let mut host2 = [0u8; 32];
            let hl2 = fmt_host(&mut host2, b"Host: localhost:", port);
            if !contains(req, &host[..hl]) && !contains(req, &host2[..hl2]) {
                SEEN.fetch_and(!1, Ordering::SeqCst);
            }
            if !contains(req, b"User-Agent: PolyHttpTest\r\n") {
                SEEN.fetch_and(!2, Ordering::SeqCst);
            }
            if !req.starts_with(b"GET /") || !contains(req, b" HTTP/1.1\r\n") {
                SEEN.fetch_or(4, Ordering::SeqCst);
            }
            if req.starts_with(b"GET /hello ") {
                send_all(
                    a,
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\nX-Poly: yes\r\n\r\nHello, Poly!\n",
                );
            } else if req.starts_with(b"GET /chunked ") {
                send_all(a, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\nHello, \r\n");
                send_all(a, b"7\r\nchunks!\r\n0\r\n\r\n");
            } else if req.starts_with(b"GET /close ") {
                send_all(a, b"HTTP/1.0 200 OK\r\nServer: tiny\r\n\r\nuntil the end");
            } else {
                send_all(a, b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            }
            closesocket(a);
        }
        closesocket(l);
    }
    served
}

fn fmt_host(buf: &mut [u8; 32], prefix: &[u8], port: u32) -> usize {
    buf[..prefix.len()].copy_from_slice(prefix);
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    let mut v = port;
    loop {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let d = &digits[i..];
    buf[prefix.len()..prefix.len() + d.len()].copy_from_slice(d);
    let end = prefix.len() + d.len();
    buf[end..end + 2].copy_from_slice(b"\r\n");
    end + 2
}

// ----- the clients

/// Read a body to its end through `read`.
unsafe fn body(read: impl Fn(*mut u8, u32, *mut u32) -> i32, buf: &mut [u8; 256]) -> usize {
    let mut total = 0;
    loop {
        let mut n = 0u32;
        // SAFETY: the rest of a live buffer.
        let ok = read(unsafe { buf.as_mut_ptr().add(total) }, (buf.len() - total) as u32, &mut n);
        check(ok != 0);
        if n == 0 || total + n as usize >= buf.len() {
            return total + n as usize;
        }
        total += n as usize;
    }
}

fn wide_eq(buf: &[u8], len: u32, want: &[u8]) -> bool {
    len as usize == want.len() * 2 && want.iter().enumerate().all(|(i, &c)| buf[i * 2] == c && buf[i * 2 + 1] == 0)
}

unsafe fn winhttp(port: u16) {
    // SAFETY: winhttp calls with valid buffers.
    unsafe {
        let s = WinHttpOpen(w!("PolyHttpTest"), 0, core::ptr::null(), core::ptr::null(), 0);
        check(s != 0);
        let c = WinHttpConnect(s, w!("127.0.0.1"), port, 0);
        check(c != 0);
        let r =
            WinHttpOpenRequest(c, w!("GET"), w!("/hello"), core::ptr::null(), core::ptr::null(), core::ptr::null(), 0);
        check(r != 0);
        check(WinHttpSendRequest(r, core::ptr::null(), 0, core::ptr::null(), 0, 0, 0) != 0);
        check(WinHttpReceiveResponse(r, core::ptr::null_mut()) != 0);
        let mut code = 0u32;
        let mut len = 4u32;
        let level = QUERY_STATUS_CODE | QUERY_FLAG_NUMBER;
        check(
            WinHttpQueryHeaders(r, level, core::ptr::null(), (&raw mut code).cast(), &mut len, core::ptr::null_mut())
                != 0,
        );
        check_got(code == 200, code);
        let mut b = [0u8; 256];
        len = b.len() as u32;
        check(
            WinHttpQueryHeaders(
                r,
                QUERY_CONTENT_TYPE,
                core::ptr::null(),
                b.as_mut_ptr(),
                &mut len,
                core::ptr::null_mut(),
            ) != 0,
        );
        check(wide_eq(&b, len, b"text/plain"));
        len = b.len() as u32;
        check(WinHttpQueryHeaders(r, QUERY_CUSTOM, w!("x-poly"), b.as_mut_ptr(), &mut len, core::ptr::null_mut()) != 0);
        check(wide_eq(&b, len, b"yes"));
        // The size of the raw headers, asked with no buffer.
        len = 0;
        let rc = WinHttpQueryHeaders(
            r,
            QUERY_RAW_HEADERS_CRLF,
            core::ptr::null(),
            core::ptr::null_mut(),
            &mut len,
            core::ptr::null_mut(),
        );
        check_fail(rc, ERROR_INSUFFICIENT_BUFFER);
        check_got(len > 40, len);
        len = b.len() as u32;
        let rc =
            WinHttpQueryHeaders(r, QUERY_LOCATION, core::ptr::null(), b.as_mut_ptr(), &mut len, core::ptr::null_mut());
        check_fail(rc, ERROR_HEADER_NOT_FOUND);
        let mut avail = 0u32;
        check(WinHttpQueryDataAvailable(r, &mut avail) != 0 && avail > 0);
        let n = body(|p, k, got| WinHttpReadData(r, p, k, got), &mut b);
        check_got(&b[..n] == b"Hello, Poly!\n", n as u32);
        check(WinHttpCloseHandle(r) != 0);
        // A chunked body.
        let r = WinHttpOpenRequest(
            c,
            core::ptr::null(),
            w!("chunked"),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
            0,
        );
        check(WinHttpSendRequest(r, core::ptr::null(), 0, core::ptr::null(), 0, 0, 0) != 0);
        check(WinHttpReceiveResponse(r, core::ptr::null_mut()) != 0);
        let n = body(|p, k, got| WinHttpReadData(r, p, k, got), &mut b);
        check_got(&b[..n] == b"Hello, chunks!", n as u32);
        WinHttpCloseHandle(r);
        // https: no TLS yet.
        let r = WinHttpOpenRequest(
            c,
            w!("GET"),
            w!("/"),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        check(r != 0);
        check_fail(
            WinHttpSendRequest(r, core::ptr::null(), 0, core::ptr::null(), 0, 0, 0),
            ERROR_WINHTTP_SECURE_FAILURE,
        );
        WinHttpCloseHandle(r);
        // localhost: ::1 refuses (the server listens on 127.0.0.1), 127.0.0.1 answers.
        let c2 = WinHttpConnect(s, w!("localhost"), port, 0);
        let r = WinHttpOpenRequest(
            c2,
            w!("GET"),
            w!("/missing"),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
            0,
        );
        check(WinHttpSendRequest(r, core::ptr::null(), 0, core::ptr::null(), 0, 0, 0) != 0);
        check(WinHttpReceiveResponse(r, core::ptr::null_mut()) != 0);
        len = 4;
        check(
            WinHttpQueryHeaders(r, level, core::ptr::null(), (&raw mut code).cast(), &mut len, core::ptr::null_mut())
                != 0,
        );
        check_got(code == 404, code);
        let mut avail = 1u32;
        check(WinHttpQueryDataAvailable(r, &mut avail) != 0 && avail == 0);
        for h in [r, c2, c, s] {
            check(WinHttpCloseHandle(h) != 0);
        }
        // A closed handle is gone.
        check_fail(WinHttpCloseHandle(s), 6);
    }
}

unsafe fn wininet(port: u16) {
    // SAFETY: wininet calls with valid buffers.
    unsafe {
        let s = InternetOpenW(w!("PolyHttpTest"), 0, core::ptr::null(), core::ptr::null(), 0);
        check(s != 0);
        // InternetOpenUrlW with the port in the URL.
        let mut url = [0u16; 64];
        let prefix = b"http://127.0.0.1:";
        let mut at = 0;
        for &c in prefix {
            url[at] = u16::from(c);
            at += 1;
        }
        let mut digits = [0u8; 5];
        let mut i = 5;
        let mut v = port;
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for &c in digits[i..].iter().chain(b"/hello") {
            url[at] = u16::from(c);
            at += 1;
        }
        let r = InternetOpenUrlW(s, url.as_ptr(), core::ptr::null(), 0, 0, 0);
        check(r != 0);
        let mut code = 0u32;
        let mut len = 4u32;
        let level = QUERY_STATUS_CODE | QUERY_FLAG_NUMBER;
        check(HttpQueryInfoW(r, level, (&raw mut code).cast(), &mut len, core::ptr::null_mut()) != 0);
        check_got(code == 200, code);
        let mut b = [0u8; 256];
        // A custom query names its field in the buffer.
        let name = w!("X-Poly");
        core::ptr::copy_nonoverlapping(name.cast::<u8>(), b.as_mut_ptr(), 14);
        len = b.len() as u32;
        check(HttpQueryInfoW(r, QUERY_CUSTOM, b.as_mut_ptr(), &mut len, core::ptr::null_mut()) != 0);
        check(wide_eq(&b, len, b"yes"));
        let n = body(|p, k, got| InternetReadFile(r, p, k, got), &mut b);
        check_got(&b[..n] == b"Hello, Poly!\n", n as u32);
        check(InternetCloseHandle(r) != 0);
        // InternetConnectW + HttpOpenRequestW + HttpSendRequestW: a 404.
        let c = InternetConnectW(
            s,
            w!("127.0.0.1"),
            port,
            core::ptr::null(),
            core::ptr::null(),
            INTERNET_SERVICE_HTTP,
            0,
            0,
        );
        check(c != 0);
        let r = HttpOpenRequestW(
            c,
            w!("GET"),
            w!("/missing"),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
        );
        check(r != 0);
        check(HttpSendRequestW(r, core::ptr::null(), 0, core::ptr::null(), 0) != 0);
        len = 4;
        check(HttpQueryInfoW(r, level, (&raw mut code).cast(), &mut len, core::ptr::null_mut()) != 0);
        check_got(code == 404, code);
        InternetCloseHandle(r);
        // A body that ends with the connection.
        let r = HttpOpenRequestW(
            c,
            core::ptr::null(),
            w!("/close"),
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            0,
        );
        check(HttpSendRequestW(r, core::ptr::null(), 0, core::ptr::null(), 0) != 0);
        let n = body(|p, k, got| InternetReadFile(r, p, k, got), &mut b);
        check_got(&b[..n] == b"until the end", n as u32);
        InternetCloseHandle(r);
        // https and ftp URLs.
        let r = InternetOpenUrlW(s, w!("https://127.0.0.1/"), core::ptr::null(), 0, 0, 0);
        check_got(r == 0 && GetLastError() == ERROR_INTERNET_SECURITY_CHANNEL_ERROR, GetLastError());
        let r = InternetOpenUrlW(s, w!("ftp://127.0.0.1/"), core::ptr::null(), 0, 0, 0);
        check_got(r == 0 && GetLastError() == ERROR_UNRECOGNIZED_SCHEME, GetLastError());
        check(InternetCloseHandle(c) != 0);
        check(InternetCloseHandle(s) != 0);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn httptest_main(_peb: *mut u8) -> u32 {
    out(b"httptest: start\n");
    // SAFETY: every call is an export of ours with valid arguments.
    unsafe {
        let mut data = [0u8; 408];
        check(WSAStartup(0x0202, data.as_mut_ptr()) == 0);
        let l = socket(2, 1, 6);
        check(l != !0);
        let mut sa = [0u8; 16];
        sa[0] = 2;
        sa[4..8].copy_from_slice(&[127, 0, 0, 1]);
        check(bind(l, sa.as_ptr(), 16) == 0);
        check(listen(l, 8) == 0);
        let mut n = 16;
        check(getsockname(l, sa.as_mut_ptr(), &mut n) == 0);
        let port = u16::from_be_bytes([sa[2], sa[3]]);
        PORT.store(u32::from(port), Ordering::SeqCst);
        let t = CreateThread(core::ptr::null(), 0, server, l as *mut u8, 0, core::ptr::null_mut());
        check(t != 0);
        winhttp(port);
        wininet(port);
        check(WaitForSingleObject(t, 20_000) == 0);
        let mut served = 0;
        check(GetExitCodeThread(t, &mut served) != 0);
        check_got(served == CONNECTIONS, served);
        let seen = SEEN.load(Ordering::SeqCst);
        check_got(seen == 0b11, seen);
        out(b"httptest: all ");
        out_num(STEP.load(Ordering::Relaxed));
        out(b" checks passed\n");
        ExitProcess(PASS)
    }
}
