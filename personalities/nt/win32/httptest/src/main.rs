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
    fn WinHttpCrackUrl(url: *const u16, len: u32, flags: u32, components: *mut UrlComponents) -> i32;
    fn WinHttpCreateUrl(components: *const UrlComponents, flags: u32, out: *mut u16, len: *mut u32) -> i32;
    fn WinHttpTimeFromSystemTime(st: *const SystemTime, out: *mut u16) -> i32;
    fn WinHttpTimeToSystemTime(text: *const u16, st: *mut SystemTime) -> i32;
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
    fn InternetCrackUrlW(url: *const u16, len: u32, flags: u32, components: *mut UrlComponents) -> i32;
    fn InternetCreateUrlW(components: *const UrlComponents, flags: u32, out: *mut u16, len: *mut u32) -> i32;
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
const ERROR_WINHTTP_INVALID_URL: u32 = 12005;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_WINHTTP_SECURE_FAILURE: u32 = 12175;

#[repr(C)]
struct UrlComponents {
    struct_size: u32,
    scheme: *mut u16,
    scheme_length: u32,
    scheme_id: i32,
    host_name: *mut u16,
    host_name_length: u32,
    port: u16,
    user_name: *mut u16,
    user_name_length: u32,
    password: *mut u16,
    password_length: u32,
    url_path: *mut u16,
    url_path_length: u32,
    extra_info: *mut u16,
    extra_info_length: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

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

unsafe fn urls_and_dates() {
    // SAFETY: winhttp calls with valid buffers and structs.
    unsafe {
        // ----- a) Crack URL with NULL pointers, length fields = 1 -----
        let u = w!("http://user:pw@example.com:8080/a/b?c=d#e");
        let mut comp = core::mem::zeroed::<UrlComponents>();
        comp.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        comp.scheme_length = 1;
        comp.user_name_length = 1;
        comp.password_length = 1;
        comp.host_name_length = 1;
        comp.url_path_length = 1;
        comp.extra_info_length = 1;
        let r = WinHttpCrackUrl(u, 0, 0, &mut comp);
        check(r != 0);
        check_got(comp.scheme_id == 1, comp.scheme_id as u32);
        check_got(comp.port == 8080, comp.port as u32);
        check_got(comp.scheme_length == 4, comp.scheme_length);
        check_got(comp.user_name_length == 4, comp.user_name_length);
        check_got(comp.password_length == 2, comp.password_length);
        check_got(comp.host_name_length == 11, comp.host_name_length);
        check_got(comp.url_path_length == 4, comp.url_path_length);
        check_got(comp.extra_info_length == 6, comp.extra_info_length);
        check_got(comp.scheme.is_null() == false, 1u32);
        check_got(comp.host_name.is_null() == false, 1u32);
        check_got(comp.url_path.is_null() == false, 1u32);
        check_got(comp.extra_info.is_null() == false, 1u32);
        // first unit of scheme is 'h' (0x68), host is 'e' (0x65), path is '/' (0x2f), extra is '?' (0x3f)
        check_got(*comp.scheme == 0x68, (*comp.scheme).into());
        check_got(*comp.host_name == 0x65, (*comp.host_name).into());
        check_got(*comp.url_path == 0x2f, (*comp.url_path).into());
        check_got(*comp.extra_info == 0x3f, (*comp.extra_info).into());

        // ----- b) Crack URL into real buffers -----
        let mut scheme = [0u16; 8];
        let mut user = [0u16; 8];
        let mut password = [0u16; 4];
        let mut host = [0u16; 16];
        let mut path = [0u16; 8];
        let mut extra = [0u16; 12];
        let mut comp2 = core::mem::zeroed::<UrlComponents>();
        comp2.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        comp2.scheme = scheme.as_mut_ptr();
        comp2.scheme_length = scheme.len() as u32;
        comp2.user_name = user.as_mut_ptr();
        comp2.user_name_length = user.len() as u32;
        comp2.password = password.as_mut_ptr();
        comp2.password_length = password.len() as u32;
        comp2.host_name = host.as_mut_ptr();
        comp2.host_name_length = host.len() as u32;
        comp2.url_path = path.as_mut_ptr();
        comp2.url_path_length = path.len() as u32;
        comp2.extra_info = extra.as_mut_ptr();
        comp2.extra_info_length = extra.len() as u32;
        let r = WinHttpCrackUrl(u, 0, 0, &mut comp2);
        check(r != 0);
        check_got(comp2.scheme_length == 4, comp2.scheme_length);
        check_got(comp2.user_name_length == 4, comp2.user_name_length);
        check_got(comp2.password_length == 2, comp2.password_length);
        check_got(comp2.host_name_length == 11, comp2.host_name_length);
        check_got(comp2.url_path_length == 4, comp2.url_path_length);
        check_got(comp2.extra_info_length == 6, comp2.extra_info_length);
        // unit after text is NUL
        check_got(scheme[4] == 0, 1u32);
        check_got(user[4] == 0, 1u32);
        check_got(password[2] == 0, 1u32);
        check_got(host[11] == 0, 1u32);
        check_got(path[4] == 0, 1u32);
        check_got(extra[6] == 0, 1u32);

        // ----- c) Too small buffer -----
        let comp_path = [0u16; 2];
        let mut comp3 = core::mem::zeroed::<UrlComponents>();
        comp3.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        comp3.url_path = comp_path.as_ptr() as *mut u16;
        comp3.url_path_length = comp_path.len() as u32;
        let r = WinHttpCrackUrl(u, 0, 0, &mut comp3);
        check_fail(r, ERROR_INSUFFICIENT_BUFFER);
        check_got(comp3.url_path_length == 5, comp3.url_path_length);

        // ----- d) ftp://h/ unrecognized scheme -----
        let f = w!("ftp://h/");
        let mut comp4 = core::mem::zeroed::<UrlComponents>();
        comp4.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        let r = WinHttpCrackUrl(f, 0, 0, &mut comp4);
        check_fail(r, ERROR_UNRECOGNIZED_SCHEME);

        // ----- e) URL with ICU_REJECT_USERPWD flag -----
        let r = WinHttpCrackUrl(u, 0, 0x0000_4000, &mut comp4); // ICU_REJECT_USERPWD
        check_fail(r, ERROR_WINHTTP_INVALID_URL);

        // ----- f) CreateUrl from parts -----
        let mut ucreate = core::mem::zeroed::<UrlComponents>();
        ucreate.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        ucreate.scheme = w!("http") as *mut u16;
        ucreate.host_name = w!("example.com") as *mut u16;
        ucreate.port = 8080;
        ucreate.url_path = w!("/a/b") as *mut u16;
        ucreate.extra_info = w!("?c=d") as *mut u16;
        let mut len: u32 = 0;
        check_fail(WinHttpCreateUrl(&ucreate, 0, core::ptr::null_mut(), &mut len), ERROR_INSUFFICIENT_BUFFER);
        check_got(len == 32, len);
        let mut out = [0u16; 32];
        let mut out_len = 32u32;
        let r = WinHttpCreateUrl(&ucreate, 0, out.as_mut_ptr(), &mut out_len);
        check(r != 0);
        check_got(out_len == 31, out_len);
        // compare: "http://example.com:8080/a/b?c=d"
        let expected = w!("http://example.com:8080/a/b?c=d") as *const u16;
        let mut i = 0;
        while i < out_len as usize {
            let ok = *expected.add(i) == out[i];
            check_got(ok, 1u32);
            i += 1;
        }
        check_got(i == 31, 1u32);

        // ----- g) TimeFromSystemTime -----
        let st = SystemTime {
            year: 2026,
            month: 11,
            day_of_week: 0,
            day: 3,
            hour: 14,
            minute: 5,
            second: 0,
            milliseconds: 0,
        };
        let mut tbuf = [0u16; 31];
        let r = WinHttpTimeFromSystemTime(&st, tbuf.as_mut_ptr());
        check(r != 0);
        let want = b"Tue, 03 Nov 2026 14:05:00 GMT";
        let mut ok = tbuf[29] == 0;
        let mut i = 0;
        while ok && i < want.len() {
            ok = tbuf[i] == u16::from(want[i]);
            i += 1;
        }
        check_got(ok, 1u32);

        // ----- h) TimeToSystemTime -----
        let t1 = w!("Tue, 21 Nov 2000 01:06:53 GMT") as *mut u16;
        let mut st1 = core::mem::zeroed::<SystemTime>();
        let r = WinHttpTimeToSystemTime(t1, &mut st1);
        check(r != 0);
        check_got(st1.year == 2000, st1.year as u32);
        check_got(st1.month == 11, st1.month as u32);
        check_got(st1.day == 21, st1.day as u32);
        check_got(st1.day_of_week == 2, st1.day_of_week as u32);
        check_got(st1.hour == 1, st1.hour as u32);
        check_got(st1.minute == 6, st1.minute as u32);
        check_got(st1.second == 53, st1.second as u32);
        check_got(st1.milliseconds == 0, st1.milliseconds as u32);

        let t2 = w!("Sun Nov  6 08:49:37 1994") as *mut u16;
        let mut st2 = core::mem::zeroed::<SystemTime>();
        let r = WinHttpTimeToSystemTime(t2, &mut st2);
        check(r != 0);
        check_got(st2.year == 1994, st2.year as u32);
        check_got(st2.month == 11, st2.month as u32);
        check_got(st2.day == 6, st2.day as u32);
        check_got(st2.day_of_week == 0, st2.day_of_week as u32);

        let t3 = w!("nope") as *mut u16;
        let mut st3 = core::mem::zeroed::<SystemTime>();
        let r = WinHttpTimeToSystemTime(t3, &mut st3);
        check_fail(r, ERROR_INVALID_PARAMETER);
    }
}

unsafe fn wininet_urls() {
    // SAFETY: wininet calls with valid buffers and structs.
    unsafe {
        // NULL pointers, length fields 1: pointers into the caller's URL.
        let u = w!("http://user:pw@example.com:8080/a/b?c=d#e");
        let mut comp = core::mem::zeroed::<UrlComponents>();
        comp.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        comp.scheme_length = 1;
        comp.user_name_length = 1;
        comp.password_length = 1;
        comp.host_name_length = 1;
        comp.url_path_length = 1;
        comp.extra_info_length = 1;
        check(InternetCrackUrlW(u, 0, 0, &mut comp) != 0);
        check_got(comp.scheme_id == 1, comp.scheme_id as u32);
        check_got(comp.port == 8080, comp.port as u32);
        check_got(comp.scheme_length == 4, comp.scheme_length);
        check_got(comp.user_name_length == 4, comp.user_name_length);
        check_got(comp.password_length == 2, comp.password_length);
        check_got(comp.host_name_length == 11, comp.host_name_length);
        check_got(comp.url_path_length == 4, comp.url_path_length);
        check_got(comp.extra_info_length == 6, comp.extra_info_length);
        check_got(*comp.scheme == 0x68, (*comp.scheme).into()); // 'h'
        check_got(*comp.host_name == 0x65, (*comp.host_name).into()); // 'e'
        check_got(*comp.url_path == 0x2f, (*comp.url_path).into()); // '/'
        check_got(*comp.extra_info == 0x3f, (*comp.extra_info).into()); // '?'
        // ftp is no scheme for us.
        check_fail(InternetCrackUrlW(w!("ftp://h/"), 0, 0, &mut comp), ERROR_UNRECOGNIZED_SCHEME);
        // CreateUrl: size query, then the URL.
        let mut uc = core::mem::zeroed::<UrlComponents>();
        uc.struct_size = core::mem::size_of::<UrlComponents>() as u32;
        uc.scheme = w!("http") as *mut u16;
        uc.host_name = w!("example.com") as *mut u16;
        uc.port = 8080;
        uc.url_path = w!("/a/b") as *mut u16;
        uc.extra_info = w!("?c=d") as *mut u16;
        let mut len = 0u32;
        check_fail(InternetCreateUrlW(&uc, 0, core::ptr::null_mut(), &mut len), ERROR_INSUFFICIENT_BUFFER);
        check_got(len == 32, len);
        let mut out = [0u16; 32];
        let mut out_len = 32u32;
        check(InternetCreateUrlW(&uc, 0, out.as_mut_ptr(), &mut out_len) != 0);
        check_got(out_len == 31, out_len);
        let expected = w!("http://example.com:8080/a/b?c=d");
        let mut i = 0;
        while i < 31 {
            check_got(*expected.add(i) == out[i], 1u32);
            i += 1;
        }
        check_got(out[31] == 0, 1u32);
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
        urls_and_dates();
        wininet_urls();
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
