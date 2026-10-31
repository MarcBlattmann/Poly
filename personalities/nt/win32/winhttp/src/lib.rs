//! Our `winhttp.dll` (ADR 0095): plain HTTP/1.1 requests, synchronously,
//! over our `ws2_32` (the shared work in `poly-win32-inet`, the HTTP in
//! the host-tested `poly-http`). Learn's page of each function is the
//! reference; what Poly does not do yet fails plainly: the asynchronous
//! mode (`WINHTTP_FLAG_ASYNC`: `ERROR_NOT_SUPPORTED`), `https`
//! (`WINHTTP_FLAG_SECURE`: `ERROR_WINHTTP_SECURE_FAILURE` when the request
//! is sent), proxies (ignored: requests go to the server).

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

use alloc::string::String;
use poly_http::consts::*;
use poly_http::request;
use poly_win32_inet::{self as inet, Connect, Handle, Object, Request, Session, fail};
use poly_win32_rt as _;

const TRUE: i32 = 1;

fn ret(r: Result<(), u32>) -> i32 {
    match r {
        Ok(()) => TRUE,
        Err(e) => fail(e),
    }
}

fn ret_handle(r: Result<Handle, u32>) -> Handle {
    r.unwrap_or_else(|e| {
        fail(e);
        0
    })
}

/// `WinHttpOpen`: a session with its user agent.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpOpen(
    agent: *const u16,
    _access: u32,
    _proxy: *const u16,
    _bypass: *const u16,
    flags: u32,
) -> Handle {
    if flags & WINHTTP_FLAG_ASYNC != 0 {
        fail(ERROR_NOT_SUPPORTED);
        return 0;
    }
    // SAFETY: the caller's string.
    let agent = unsafe { inet::wstr(agent) };
    inet::new_handle(Object::Session(Session { agent, timeout_ms: 0 }))
}

/// `WinHttpConnect`: the server's name and port (`INTERNET_DEFAULT_PORT`:
/// 80).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpConnect(session: Handle, server: *const u16, port: u16, _reserved: u32) -> Handle {
    // SAFETY: the caller's string.
    let Some(host) = (unsafe { inet::wstr(server) }).filter(|h| !h.is_empty()) else {
        fail(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let port = if port == INTERNET_DEFAULT_PORT { INTERNET_DEFAULT_HTTP_PORT } else { port };
    ret_handle(inet::with_object(session, |o| {
        let s = inet::session_of(o)?.clone();
        Ok(inet::new_handle(Object::Connect(Connect { session: s, host, port })))
    }))
}

/// The object name as a request target (`/` when absent).
pub fn target(object: Option<String>) -> String {
    match object {
        None => String::from("/"),
        Some(o) if o.is_empty() => String::from("/"),
        Some(o) if o.starts_with('/') || o.starts_with('*') || o.contains("://") => o,
        Some(o) => alloc::format!("/{o}"),
    }
}

/// `WinHttpOpenRequest`: the verb (`GET` when null), the object, and
/// `WINHTTP_FLAG_SECURE`. The version, referrer and accept types are
/// ignored (HTTP/1.1 always).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpOpenRequest(
    connect: Handle,
    verb: *const u16,
    object: *const u16,
    _version: *const u16,
    _referrer: *const u16,
    _accept: *const *const u16,
    flags: u32,
) -> Handle {
    // SAFETY: the caller's strings.
    let (verb, object) = unsafe { (inet::wstr(verb), inet::wstr(object)) };
    let verb = verb.filter(|v| !v.is_empty()).unwrap_or_else(|| String::from("GET"));
    ret_handle(inet::with_object(connect, |o| {
        let c = inet::connect_of(o)?;
        let r = Request::new(c, &verb, &target(object), flags & WINHTTP_FLAG_SECURE != 0);
        Ok(inet::new_handle(Object::Request(alloc::boxed::Box::new(r))))
    }))
}

/// `WinHttpAddRequestHeaders`: header lines added (or replacing fields
/// of the same name with `WINHTTP_ADDREQ_FLAG_REPLACE`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpAddRequestHeaders(
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
        request::add(&mut r.headers, fields, modifiers & WINHTTP_ADDREQ_FLAG_REPLACE != 0);
        Ok(())
    }))
}

/// `WinHttpSendRequest`: connect and send the head, the optional data,
/// and announce `total` bytes of body (the rest through
/// `WinHttpWriteData`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpSendRequest(
    req: Handle,
    headers: *const u16,
    len: u32,
    optional: *const u8,
    optional_len: u32,
    total: u32,
    _context: usize,
) -> i32 {
    // SAFETY: the caller's string.
    let extra = match unsafe { inet::wstr_len(headers, len) } {
        None => alloc::vec::Vec::new(),
        Some(t) => match request::parse_lines(&t) {
            Some(f) => f,
            None => return fail(ERROR_INVALID_PARAMETER),
        },
    };
    let body: &[u8] = if optional.is_null() || optional_len == 0 {
        &[]
    } else {
        // SAFETY: `optional_len` bytes of the caller's.
        unsafe { core::slice::from_raw_parts(optional, optional_len as usize) }
    };
    ret(inet::with_object(req, |o| inet::request_of(o)?.send(&extra, body, u64::from(total))))
}

/// `WinHttpWriteData`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpWriteData(req: Handle, buf: *const u8, n: u32, written: *mut u32) -> i32 {
    if buf.is_null() && n > 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: `n` bytes of the caller's.
    let data = if n == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(buf, n as usize) } };
    ret(inet::with_object(req, |o| {
        let k = inet::request_of(o)?.write(data)?;
        if !written.is_null() {
            // SAFETY: the caller's count.
            unsafe { *written = k as u32 };
        }
        Ok(())
    }))
}

/// `WinHttpReceiveResponse`: wait for the response head.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpReceiveResponse(req: Handle, _reserved: *mut u8) -> i32 {
    ret(inet::with_object(req, |o| inet::request_of(o)?.receive()))
}

/// `WinHttpQueryHeaders` (`poly_http::query`): the answer in the caller's
/// buffer, `ERROR_INSUFFICIENT_BUFFER` with the size needed,
/// `ERROR_WINHTTP_HEADER_NOT_FOUND`; `*index` advanced for repeated
/// fields.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpQueryHeaders(
    req: Handle,
    level: u32,
    name: *const u16,
    buf: *mut u8,
    len: *mut u32,
    index: *mut u32,
) -> i32 {
    // SAFETY: the caller's string and index.
    let (name, start) = unsafe { (inet::wstr(name), if index.is_null() { 0 } else { *index }) };
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

/// `WinHttpQueryDataAvailable`: body bytes readable now, waiting for some
/// if none (0 at the end).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpQueryDataAvailable(req: Handle, n: *mut u32) -> i32 {
    ret(inet::with_object(req, |o| {
        let k = inet::request_of(o)?.available()?;
        if !n.is_null() {
            // SAFETY: the caller's count.
            unsafe { *n = k.min(u32::MAX as usize) as u32 };
        }
        Ok(())
    }))
}

/// `WinHttpReadData`: body bytes (0 read at the end).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpReadData(req: Handle, buf: *mut u8, n: u32, read: *mut u32) -> i32 {
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

/// `WinHttpSetTimeouts`: the receive timeout is kept (on a session for
/// its new requests, on a request for itself before it is sent); the
/// others are accepted.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSetTimeouts(h: Handle, resolve: i32, connect: i32, send: i32, receive: i32) -> i32 {
    if [resolve, connect, send, receive].iter().any(|&t| t < -1) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let ms = if receive <= 0 { 0 } else { receive as u32 };
    ret(inet::with_object(h, |o| {
        match o {
            Object::Session(s) => s.timeout_ms = ms,
            Object::Request(r) => r.session.timeout_ms = ms,
            Object::Connect(_) => return Err(ERROR_INCORRECT_HANDLE_TYPE),
        }
        Ok(())
    }))
}

/// `WinHttpCloseHandle`.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpCloseHandle(h: Handle) -> i32 {
    ret(inet::close_handle(h))
}

/// `WinHttpCheckPlatform`: WinHTTP is supported.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpCheckPlatform() -> i32 {
    TRUE
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn winhttp_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    TRUE
}
