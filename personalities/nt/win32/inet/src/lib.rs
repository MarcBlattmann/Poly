//! What our `winhttp.dll` and `wininet.dll` share (ADR 0095): their
//! handles (`HINTERNET`s: a session, a connection, a request), plain
//! HTTP/1.1 requests over our `ws2_32` sockets, and the copying of
//! strings and query answers into the caller's buffers. The HTTP itself
//! (requests, responses, header queries, URLs) is the host-tested
//! `poly-http`.
//!
//! Everything is synchronous: a request connects, sends and receives in
//! the calling thread (the asynchronous modes of both APIs are refused).
//! `https` is refused too until Poly has TLS (ADR 0095). One connection
//! per request, closed with its handle.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_http::consts::*;
use poly_http::query::{self, Answer};
use poly_http::request;
use poly_http::response::{Head, Parser};

pub type Handle = isize;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    pub fn GetLastError() -> u32;
    pub fn SetLastError(e: u32);
    fn GetProcessHeap() -> Handle;
    fn HeapAlloc(heap: Handle, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: Handle, flags: u32, mem: *mut u8) -> i32;
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(af: i32, ty: i32, protocol: i32) -> usize;
    fn connect(s: usize, name: *const u8, len: i32) -> i32;
    fn send(s: usize, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: usize, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn closesocket(s: usize) -> i32;
    fn setsockopt(s: usize, level: i32, name: i32, value: *const u8, len: i32) -> i32;
    fn getaddrinfo(node: *const u8, service: *const u8, hints: *const u8, result: *mut *mut u8) -> i32;
    fn freeaddrinfo(list: *mut u8);
}

/// `alloc` on the process heap, 16-byte aligned (both DLLs link this).
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks on x64 (Learn:
// "HeapAlloc"); stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: kernel32 exports without preconditions.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block of `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

/// Set the thread's last error and return `FALSE` / null.
pub fn fail(e: u32) -> i32 {
    // SAFETY: no preconditions.
    unsafe { SetLastError(e) };
    0
}

// ----- handles

/// A session (`WinHttpOpen`, `InternetOpenW`).
#[derive(Debug, Default, Clone)]
pub struct Session {
    pub agent: Option<String>,
    /// Receive timeout in milliseconds (0: none).
    pub timeout_ms: u32,
}

/// A connection to a server (`WinHttpConnect`, `InternetConnectW`): only
/// its name; the socket belongs to each request.
#[derive(Debug, Clone)]
pub struct Connect {
    pub session: Session,
    pub host: String,
    pub port: u16,
}

/// A request.
#[derive(Debug)]
pub struct Request {
    pub session: Session,
    pub host: String,
    pub port: u16,
    pub verb: String,
    pub path: String,
    pub secure: bool,
    pub headers: Vec<(String, String)>,
    conn: Option<Conn>,
    parser: Option<Parser>,
    /// Body bytes announced but not written yet (`WinHttpWriteData`).
    pub body_left: u64,
}

#[derive(Debug)]
pub enum Object {
    Session(Session),
    Connect(Connect),
    Request(alloc::boxed::Box<Request>),
}

/// The handles alive, so a stale or foreign value is refused rather than
/// followed.
static LIVE: Lock<Vec<usize>> = Lock::new(Vec::new());

/// A spin lock (the DLLs have no kernel32 critical sections bound here).
pub struct Lock<T> {
    taken: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

// SAFETY: access to the value is serialised by `taken`.
unsafe impl<T> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(v: T) -> Lock<T> {
        Lock { taken: AtomicBool::new(false), value: core::cell::UnsafeCell::new(v) }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.taken.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: we hold the lock.
        let r = f(unsafe { &mut *self.value.get() });
        self.taken.store(false, Ordering::Release);
        r
    }
}

/// Make a handle for `o`.
pub fn new_handle(o: Object) -> Handle {
    let p = Box::into_raw(Box::new(o)) as usize;
    LIVE.with(|l| l.push(p));
    p as Handle
}

/// Run `f` on the object behind `h` (`ERROR_INVALID_HANDLE` if it is not
/// one of ours). The caller serialises use of one handle, as both APIs
/// ask.
pub fn with_object<R>(h: Handle, f: impl FnOnce(&mut Object) -> Result<R, u32>) -> Result<R, u32> {
    if !LIVE.with(|l| l.contains(&(h as usize))) {
        return Err(ERROR_INVALID_HANDLE);
    }
    // SAFETY: a live handle is a Box this module made and has not freed.
    f(unsafe { &mut *(h as *mut Object) })
}

/// Close a handle.
pub fn close_handle(h: Handle) -> Result<(), u32> {
    let found = LIVE.with(|l| match l.iter().position(|&p| p == h as usize) {
        Some(i) => {
            l.swap_remove(i);
            true
        }
        None => false,
    });
    if !found {
        return Err(ERROR_INVALID_HANDLE);
    }
    // SAFETY: the Box this module made, now out of the live list.
    drop(unsafe { Box::from_raw(h as *mut Object) });
    Ok(())
}

pub fn session_of(o: &Object) -> Result<&Session, u32> {
    match o {
        Object::Session(s) => Ok(s),
        _ => Err(ERROR_INCORRECT_HANDLE_TYPE),
    }
}

pub fn connect_of(o: &Object) -> Result<&Connect, u32> {
    match o {
        Object::Connect(c) => Ok(c),
        _ => Err(ERROR_INCORRECT_HANDLE_TYPE),
    }
}

pub fn request_of(o: &mut Object) -> Result<&mut Request, u32> {
    match o {
        Object::Request(r) => Ok(r),
        _ => Err(ERROR_INCORRECT_HANDLE_TYPE),
    }
}

// ----- the connection

#[derive(Debug)]
struct Conn {
    s: usize,
}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: our socket.
        unsafe { closesocket(self.s) };
    }
}

static STARTED: AtomicBool = AtomicBool::new(false);

const AF_UNSPEC_STREAM_HINTS: [u8; 48] = {
    let mut h = [0u8; 48];
    h[8] = 1; // SOCK_STREAM
    h
};

impl Conn {
    /// Resolve and connect: `ERROR_*_NAME_NOT_RESOLVED` when the name has
    /// no address, `ERROR_*_CANNOT_CONNECT` when no address answers.
    fn open(host: &str, port: u16, timeout_ms: u32) -> Result<Conn, u32> {
        if !STARTED.swap(true, Ordering::SeqCst) {
            let mut data = [0u8; 408];
            // SAFETY: a WSADATA-sized buffer.
            unsafe { WSAStartup(0x0202, data.as_mut_ptr()) };
        }
        let mut node: Vec<u8> = host.bytes().collect();
        node.push(0);
        let mut service: Vec<u8> = alloc::format!("{port}").into_bytes();
        service.push(0);
        let mut list: *mut u8 = core::ptr::null_mut();
        // SAFETY: NUL-terminated strings and hints of ours.
        if unsafe { getaddrinfo(node.as_ptr(), service.as_ptr(), AF_UNSPEC_STREAM_HINTS.as_ptr(), &mut list) } != 0 {
            return Err(ERROR_NAME_NOT_RESOLVED);
        }
        let mut ai = list;
        let mut result = Err(ERROR_CANNOT_CONNECT);
        while !ai.is_null() {
            // SAFETY: an ADDRINFO list of ws2_32's (the x64 layout).
            unsafe {
                let family = core::ptr::read_unaligned(ai.add(4).cast::<i32>());
                let len = core::ptr::read_unaligned(ai.add(16).cast::<usize>());
                let addr = core::ptr::read_unaligned(ai.add(32).cast::<*const u8>());
                let s = socket(family, 1, 6);
                if s != !0 {
                    if connect(s, addr, len as i32) == 0 {
                        if timeout_ms != 0 {
                            let t = timeout_ms.to_le_bytes();
                            setsockopt(s, 0xffff, 0x1006, t.as_ptr(), 4);
                        }
                        result = Ok(Conn { s });
                        break;
                    }
                    closesocket(s);
                }
                ai = core::ptr::read_unaligned(ai.add(40).cast::<*mut u8>());
            }
        }
        // SAFETY: the list getaddrinfo made.
        unsafe { freeaddrinfo(list) };
        result
    }

    fn send_all(&self, mut data: &[u8]) -> Result<(), u32> {
        while !data.is_empty() {
            let n = data.len().min(1 << 20);
            // SAFETY: a live slice.
            let k = unsafe { send(self.s, data.as_ptr(), n as i32, 0) };
            if k <= 0 {
                return Err(ERROR_CONNECTION_ERROR);
            }
            data = &data[k as usize..];
        }
        Ok(())
    }

    /// Some bytes, or 0 at the end of the stream.
    fn recv_some(&self, buf: &mut [u8]) -> Result<usize, u32> {
        // SAFETY: a live buffer.
        let n = unsafe { recv(self.s, buf.as_mut_ptr(), buf.len() as i32, 0) };
        if n < 0 {
            // SAFETY: no preconditions.
            let e = unsafe { WSAGetLastError() } as u32;
            return Err(if e == 10060 { ERROR_TIMEOUT } else { ERROR_CONNECTION_ERROR });
        }
        Ok(n as usize)
    }
}

impl Request {
    pub fn new(c: &Connect, verb: &str, path: &str, secure: bool) -> Request {
        Request {
            session: c.session.clone(),
            host: c.host.clone(),
            port: c.port,
            verb: String::from(verb),
            path: String::from(path),
            secure,
            headers: Vec::new(),
            conn: None,
            parser: None,
            body_left: 0,
        }
    }

    /// Connect and send the head and the first `body` bytes of
    /// `total_len`. `https` fails with `ERROR_WINHTTP_SECURE_FAILURE`.
    pub fn send(&mut self, extra: &[(String, String)], body: &[u8], total_len: u64) -> Result<(), u32> {
        if self.secure {
            return Err(ERROR_SECURE_FAILURE);
        }
        if self.conn.is_some() {
            return Err(ERROR_INCORRECT_HANDLE_STATE);
        }
        let mut headers = self.headers.clone();
        request::add(&mut headers, extra.to_vec(), true);
        let default_port = if self.secure { 443 } else { 80 };
        let host = request::host_field(&self.host, self.port, default_port);
        let total = total_len.max(body.len() as u64);
        let head = request::head(&self.verb, &self.path, &host, self.session.agent.as_deref(), &headers, Some(total));
        let conn = Conn::open(&self.host, self.port, self.session.timeout_ms)?;
        conn.send_all(&head)?;
        conn.send_all(body)?;
        self.body_left = total - body.len() as u64;
        self.conn = Some(conn);
        Ok(())
    }

    /// More of the announced body.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, u32> {
        let conn = self.conn.as_ref().ok_or(ERROR_INCORRECT_HANDLE_STATE)?;
        conn.send_all(data)?;
        self.body_left = self.body_left.saturating_sub(data.len() as u64);
        Ok(data.len())
    }

    /// Read until the response head is complete.
    pub fn receive(&mut self) -> Result<(), u32> {
        if self.parser.as_ref().is_some_and(|p| p.head().is_some()) {
            return Ok(());
        }
        if self.conn.is_none() {
            return Err(ERROR_INCORRECT_HANDLE_STATE);
        }
        let head_request = self.verb.eq_ignore_ascii_case("HEAD");
        self.parser.get_or_insert_with(|| Parser::new(head_request));
        while self.parser.as_ref().is_some_and(|p| p.head().is_none()) {
            self.more()?;
        }
        Ok(())
    }

    /// One read from the connection into the parser.
    fn more(&mut self) -> Result<bool, u32> {
        let (Some(conn), Some(p)) = (self.conn.as_ref(), self.parser.as_mut()) else {
            return Err(ERROR_INCORRECT_HANDLE_STATE);
        };
        let mut buf = [0u8; 4096];
        let n = conn.recv_some(&mut buf)?;
        if n == 0 {
            p.eof()
                .map_err(|_| if p.head().is_none() { ERROR_INVALID_SERVER_RESPONSE } else { ERROR_CONNECTION_ERROR })?;
            return Ok(false);
        }
        p.feed(&buf[..n]).map_err(|_| ERROR_INVALID_SERVER_RESPONSE)?;
        Ok(true)
    }

    pub fn head(&self) -> Result<&Head, u32> {
        self.parser.as_ref().and_then(Parser::head).ok_or(ERROR_INCORRECT_HANDLE_STATE)
    }

    /// Body bytes readable without waiting, waiting for some if none are
    /// (0 at the end of the body).
    pub fn available(&mut self) -> Result<usize, u32> {
        self.receive()?;
        loop {
            let p = self.parser.as_ref().ok_or(ERROR_INCORRECT_HANDLE_STATE)?;
            if p.available() > 0 || p.is_done() {
                return Ok(p.available());
            }
            if !self.more()? {
                return Ok(self.parser.as_ref().map_or(0, Parser::available));
            }
        }
    }

    /// Read body bytes (0 at the end).
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, u32> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = self.available()?;
        if n == 0 {
            return Ok(0);
        }
        Ok(self.parser.as_mut().map_or(0, |p| p.read(buf)))
    }

    /// A header query (`poly_http::query`).
    pub fn query(&self, level: u32, name: Option<&str>, index: u32) -> Result<(Answer, u32), u32> {
        query::query(self.head()?, level, name, index)
    }
}

// ----- strings

/// A NUL-terminated UTF-16 string (`None` for null).
pub unsafe fn wstr(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated string.
    let units = unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    };
    Some(char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect())
}

/// A UTF-16 string of `len` units (`u32::MAX` / `-1`: NUL-terminated).
pub unsafe fn wstr_len(p: *const u16, len: u32) -> Option<String> {
    if p.is_null() {
        return None;
    }
    if len == u32::MAX {
        // SAFETY: as documented.
        return unsafe { wstr(p) };
    }
    // SAFETY: `len` units of the caller's.
    let units = unsafe { core::slice::from_raw_parts(p, len as usize) };
    Some(char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect())
}

/// Copy a query answer to `(buf, *len)` (bytes): text as UTF-16 with a
/// NUL (`*len` the bytes without it), a number as a DWORD. A buffer too
/// small (or null) is `ERROR_INSUFFICIENT_BUFFER` with the bytes needed,
/// the NUL included (Learn: "WinHttpQueryHeaders", "HttpQueryInfo").
pub unsafe fn put_answer(a: &Answer, buf: *mut u8, len: *mut u32) -> Result<(), u32> {
    if len.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's length and buffer.
    unsafe {
        match a {
            Answer::Number(n) => {
                if buf.is_null() || *len < 4 {
                    *len = 4;
                    return Err(ERROR_INSUFFICIENT_BUFFER);
                }
                core::ptr::copy_nonoverlapping(n.to_le_bytes().as_ptr(), buf, 4);
                *len = 4;
            }
            Answer::Text(t) => {
                let units: Vec<u16> = t.encode_utf16().collect();
                let need = (units.len() as u32 + 1) * 2;
                if buf.is_null() || *len < need {
                    *len = need;
                    return Err(ERROR_INSUFFICIENT_BUFFER);
                }
                let out = buf.cast::<u16>();
                for (i, u) in units.iter().enumerate() {
                    core::ptr::write_unaligned(out.add(i), *u);
                }
                core::ptr::write_unaligned(out.add(units.len()), 0);
                *len = need - 2;
            }
        }
    }
    Ok(())
}
