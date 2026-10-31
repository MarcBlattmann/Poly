//! Our `wininet.dll` (ADR 0095): its HTTP basics, synchronously, over our
//! `ws2_32` (the shared work in `poly-win32-inet`, the HTTP in the
//! host-tested `poly-http`). Learn's page of each function is the
//! reference; what Poly does not do yet fails plainly: the asynchronous
//! mode (`INTERNET_FLAG_ASYNC`: `ERROR_NOT_SUPPORTED`), `https`
//! (`ERROR_INTERNET_SECURITY_CHANNEL_ERROR` when the request is sent),
//! FTP and Gopher (`ERROR_NOT_SUPPORTED`, or an unrecognised scheme in a
//! URL), proxies, caching and cookies.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use poly_http::consts::*;
use poly_http::request;
use poly_http::url::{self, Scheme, UrlError};
use poly_win32_inet::{self as inet, Connect, Handle, Object, Request, Session, fail};
use poly_win32_rt as _;

const TRUE: i32 = 1;

/// WinINet's code for a failure of the shared request code.
fn wininet_error(e: u32) -> u32 {
    if e == ERROR_SECURE_FAILURE { ERROR_INTERNET_SECURITY_CHANNEL_ERROR } else { e }
}

fn ret(r: Result<(), u32>) -> i32 {
    match r {
        Ok(()) => TRUE,
        Err(e) => fail(wininet_error(e)),
    }
}

fn ret_handle(r: Result<Handle, u32>) -> Handle {
    r.unwrap_or_else(|e| {
        fail(wininet_error(e));
        0
    })
}

/// `InternetOpenW`: a session with its user agent.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InternetOpenW(
    agent: *const u16,
    _access: u32,
    _proxy: *const u16,
    _bypass: *const u16,
    flags: u32,
) -> Handle {
    if flags & INTERNET_FLAG_ASYNC != 0 {
        fail(ERROR_NOT_SUPPORTED);
        return 0;
    }
    // SAFETY: the caller's string.
    let agent = unsafe { inet::wstr(agent) };
    inet::new_handle(Object::Session(Session { agent, timeout_ms: 0 }))
}

/// `InternetConnectW`: HTTP only; port 0 is the service's default (80).
/// User name and password are not sent (no authentication yet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InternetConnectW(
    session: Handle,
    server: *const u16,
    port: u16,
    _user: *const u16,
    _password: *const u16,
    service: u32,
    _flags: u32,
    _context: usize,
) -> Handle {
    if service != INTERNET_SERVICE_HTTP {
        fail(ERROR_NOT_SUPPORTED);
        return 0;
    }
    // SAFETY: the caller's string.
    let Some(host) = (unsafe { inet::wstr(server) }).filter(|h| !h.is_empty()) else {
        fail(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let port = if port == 0 { INTERNET_DEFAULT_HTTP_PORT } else { port };
    ret_handle(inet::with_object(session, |o| {
        let s = inet::session_of(o)?.clone();
        Ok(inet::new_handle(Object::Connect(Connect { session: s, host, port })))
    }))
}

fn target(object: Option<String>) -> String {
    match object {
        None => String::from("/"),
        Some(o) if o.is_empty() => String::from("/"),
        Some(o) if o.starts_with('/') => o,
        Some(o) => alloc::format!("/{o}"),
    }
}

/// `HttpOpenRequestW`: the verb (`GET` when null), the object, and
/// `INTERNET_FLAG_SECURE`; version, referrer and accept types ignored.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HttpOpenRequestW(
    connect: Handle,
    verb: *const u16,
    object: *const u16,
    _version: *const u16,
    _referrer: *const u16,
    _accept: *const *const u16,
    flags: u32,
    _context: usize,
) -> Handle {
    // SAFETY: the caller's strings.
    let (verb, object) = unsafe { (inet::wstr(verb), inet::wstr(object)) };
    let verb = verb.filter(|v| !v.is_empty()).unwrap_or_else(|| String::from("GET"));
    ret_handle(inet::with_object(connect, |o| {
        let c = inet::connect_of(o)?;
        let r = Request::new(c, &verb, &target(object), flags & INTERNET_FLAG_SECURE != 0);
        Ok(inet::new_handle(Object::Request(alloc::boxed::Box::new(r))))
    }))
}

/// `HttpAddRequestHeadersW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HttpAddRequestHeadersW(
    req: Handle,
    headers: *const u16,
    len: u32,
    modifiers: u32,
) -> i32 {
    // SAFETY: the caller's string.
    let Some(text) = (unsafe { inet::wstr_len(headers, len) }) else { return fail(ERROR_INVALID_PARAMETER) };
    let Some(fields) = request::parse_lines(&text) else { return fail(ERROR_INVALID_PARAMETER) };
    ret(inet::with_object(req, |o| {
        let r = inet::request_of(o)?;
        request::add(&mut r.headers, fields, modifiers & HTTP_ADDREQ_FLAG_REPLACE != 0);
        Ok(())
    }))
}

fn header_fields(headers: *const u16, len: u32) -> Result<Vec<(String, String)>, u32> {
    // SAFETY: the caller's string.
    match unsafe { inet::wstr_len(headers, len) } {
        None => Ok(Vec::new()),
        Some(t) => request::parse_lines(&t).ok_or(ERROR_INVALID_PARAMETER),
    }
}

/// `HttpSendRequestW`: connect, send the head and the optional data, and
/// read the response head.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HttpSendRequestW(
    req: Handle,
    headers: *const u16,
    len: u32,
    optional: *const u8,
    optional_len: u32,
) -> i32 {
    let extra = match header_fields(headers, len) {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    let body: &[u8] = if optional.is_null() || optional_len == 0 {
        &[]
    } else {
        // SAFETY: `optional_len` bytes of the caller's.
        unsafe { core::slice::from_raw_parts(optional, optional_len as usize) }
    };
    ret(inet::with_object(req, |o| {
        let r = inet::request_of(o)?;
        r.send(&extra, body, body.len() as u64)?;
        r.receive()
    }))
}

/// `InternetOpenUrlW`: a session's request for an `http` URL, sent and
/// its head received; other schemes are `ERROR_INTERNET_UNRECOGNIZED_SCHEME`
/// (`https`: the security channel error, no TLS yet).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InternetOpenUrlW(
    session: Handle,
    url: *const u16,
    headers: *const u16,
    len: u32,
    _flags: u32,
    _context: usize,
) -> Handle {
    // SAFETY: the caller's string.
    let Some(text) = (unsafe { inet::wstr(url) }) else {
        fail(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let u = match url::parse(&text) {
        Ok(u) => u,
        Err(UrlError::Scheme) => {
            fail(ERROR_UNRECOGNIZED_SCHEME);
            return 0;
        }
        Err(UrlError::Invalid) => {
            fail(ERROR_INVALID_URL);
            return 0;
        }
    };
    let extra = match header_fields(headers, len) {
        Ok(f) => f,
        Err(e) => {
            fail(e);
            return 0;
        }
    };
    ret_handle(inet::with_object(session, |o| {
        let s = inet::session_of(o)?.clone();
        let c = Connect { session: s, host: u.host.clone(), port: u.port };
        let mut r = Request::new(&c, "GET", &u.path, u.scheme == Scheme::Https);
        r.send(&extra, &[], 0)?;
        r.receive()?;
        Ok(inet::new_handle(Object::Request(alloc::boxed::Box::new(r))))
    }))
}

/// `InternetReadFile`: body bytes (`*read` 0 at the end).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InternetReadFile(req: Handle, buf: *mut u8, n: u32, read: *mut u32) -> i32 {
    if read.is_null() || (buf.is_null() && n > 0) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    ret(inet::with_object(req, |o| {
        let r = inet::request_of(o)?;
        // SAFETY: `n` bytes of the caller's.
        let out = if n == 0 { &mut [][..] } else { unsafe { core::slice::from_raw_parts_mut(buf, n as usize) } };
        let k = r.read(out)?;
        // SAFETY: the caller's count.
        unsafe { *read = k as u32 };
        Ok(())
    }))
}

/// `InternetQueryDataAvailable`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InternetQueryDataAvailable(
    req: Handle,
    n: *mut u32,
    _flags: u32,
    _context: usize,
) -> i32 {
    ret(inet::with_object(req, |o| {
        let k = inet::request_of(o)?.available()?;
        if !n.is_null() {
            // SAFETY: the caller's count.
            unsafe { *n = k.min(u32::MAX as usize) as u32 };
        }
        Ok(())
    }))
}

/// `HttpQueryInfoW` (`poly_http::query`): for `HTTP_QUERY_CUSTOM` the
/// field's name is the buffer's contents on entry (Learn).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HttpQueryInfoW(
    req: Handle,
    level: u32,
    buf: *mut u8,
    len: *mut u32,
    index: *mut u32,
) -> i32 {
    let custom = level & !QUERY_MODIFIER_FLAGS_MASK == QUERY_CUSTOM;
    // SAFETY: the caller's buffer holds the name for a custom query.
    let name = if custom && !buf.is_null() { unsafe { inet::wstr(buf.cast::<u16>()) } } else { None };
    // SAFETY: the caller's index.
    let start = if index.is_null() { 0 } else { unsafe { *index } };
    ret(inet::with_object(req, |o| {
        let r = inet::request_of(o)?;
        let (answer, next) = r.query(level, name.as_deref(), start)?;
        // SAFETY: the caller's buffer and length.
        unsafe { inet::put_answer(&answer, buf, len)? };
        if !index.is_null() {
            // SAFETY: the caller's index.
            unsafe { *index = next };
        }
        Ok(())
    }))
}

/// `InternetCloseHandle`.
#[unsafe(no_mangle)]
pub extern "system" fn InternetCloseHandle(h: Handle) -> i32 {
    ret(inet::close_handle(h))
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn wininet_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    TRUE
}
