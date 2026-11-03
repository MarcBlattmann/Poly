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
use alloc::vec::Vec;
use poly_http::consts::*;
use poly_http::date::{self, DateTime};
use poly_http::request;
use poly_http::url::{self, Cracked, Parts};
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

// ----- URL_COMPONENTS: WinHttpCrackUrl and WinHttpCreateUrl

/// `URL_COMPONENTS` (`winhttp.h`), x64: `INTERNET_SCHEME` is an `int`,
/// `INTERNET_PORT` a `WORD`, so the structure is 104 bytes.
#[repr(C)]
#[derive(Debug)]
pub struct UrlComponents {
    pub struct_size: u32,
    pub scheme: *mut u16,
    pub scheme_length: u32,
    pub scheme_id: i32,
    pub host_name: *mut u16,
    pub host_name_length: u32,
    pub port: u16,
    pub user_name: *mut u16,
    pub user_name_length: u32,
    pub password: *mut u16,
    pub password_length: u32,
    pub url_path: *mut u16,
    pub url_path_length: u32,
    pub extra_info: *mut u16,
    pub extra_info_length: u32,
}

const _: () = assert!(core::mem::size_of::<UrlComponents>() == 104);

/// The caller's URL as units (`len` 0: NUL-terminated).
unsafe fn url_units<'a>(url: *const u16, len: u32) -> Option<&'a [u16]> {
    if url.is_null() {
        return None;
    }
    let n = if len == 0 {
        let mut n = 0;
        // SAFETY: a NUL-terminated string.
        while unsafe { *url.add(n) } != 0 {
            n += 1;
        }
        n
    } else {
        len as usize
    };
    // SAFETY: `n` units of the caller's.
    Some(unsafe { core::slice::from_raw_parts(url, n) })
}

/// One component into the caller's buffer: `ICU_ESCAPE` / `ICU_DECODE`
/// apply only when there is a buffer to write (Learn: "WinHttpCrackUrl");
/// with a null pointer and a nonzero length the pointer is set to the
/// component in the caller's own string, `source`.
unsafe fn put_component(
    raw: &[u16],
    source: *const u16,
    ptr: *mut u16,
    len: u32,
    flags: u32,
) -> Result<(*mut u16, u32), u32> {
    if ptr.is_null() {
        if len == 0 {
            return Ok((core::ptr::null_mut(), 0));
        }
        return Ok((source as *mut u16, raw.len() as u32));
    }
    let units: Vec<u16> = if flags & (ICU_ESCAPE | ICU_DECODE) == 0 {
        raw.to_vec()
    } else {
        let text: String = char::decode_utf16(raw.iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect();
        let text = if flags & ICU_DECODE != 0 { url::decode(&text) } else { text };
        let text = if flags & ICU_ESCAPE != 0 { url::escape(&text) } else { text };
        text.encode_utf16().collect()
    };
    let need = units.len() + 1;
    if (len as usize) < need {
        return Err(need as u32);
    }
    // SAFETY: the caller's buffer holds `need` units.
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
        ptr.add(units.len()).write(0);
    }
    Ok((ptr, units.len() as u32))
}

/// Store both halves of one `URL_COMPONENTS` string field back.
unsafe fn store(field: &mut *mut u16, length: &mut u32, raw: &[u16], source: *const u16, flags: u32) -> Result<(), u32> {
    // SAFETY: the caller's buffer.
    match unsafe { put_component(raw, source, *field, *length, flags) } {
        Ok((p, l)) => {
            *field = p;
            *length = l;
            Ok(())
        }
        Err(need) => {
            *length = need;
            Err(ERROR_INSUFFICIENT_BUFFER)
        }
    }
}

/// Fill every field the caller asked for; the path carries the extra
/// information when that is not asked for (Learn: "WinHttpCrackUrl").
unsafe fn fill(c: &mut UrlComponents, units: &[u16], k: &Cracked, flags: u32) -> Result<(), u32> {
    let extra_wanted = !c.extra_info.is_null() || c.extra_info_length != 0;
    let path = k.path.start..if extra_wanted { k.path.end } else { units.len() };
    let slash = [u16::from(b'/')];
    let (path_text, path_source): (&[u16], *const u16) = if path.is_empty() {
        (&slash, slash.as_ptr())
    } else {
        // SAFETY: `path` is inside `units`.
        (&units[path.clone()], unsafe { units.as_ptr().add(path.start) })
    };
    // SAFETY: every span is inside `units`, every source points into it.
    unsafe {
        store(&mut c.scheme, &mut c.scheme_length, &units[k.scheme.clone()], units.as_ptr().add(k.scheme.start), flags)?;
        store(&mut c.host_name, &mut c.host_name_length, &units[k.host.clone()], units.as_ptr().add(k.host.start), flags)?;
        store(&mut c.user_name, &mut c.user_name_length, &units[k.user.clone()], units.as_ptr().add(k.user.start), flags)?;
        store(&mut c.password, &mut c.password_length, &units[k.password.clone()], units.as_ptr().add(k.password.start), flags)?;
        store(&mut c.url_path, &mut c.url_path_length, path_text, path_source, flags)?;
        if extra_wanted {
            store(&mut c.extra_info, &mut c.extra_info_length, &units[k.extra.clone()], units.as_ptr().add(k.extra.start), flags)?;
        }
    }
    c.scheme_id = k.scheme_id;
    c.port = k.port;
    Ok(())
}

/// `WinHttpCrackUrl`: `URL_COMPONENTS` filled from `url`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpCrackUrl(url: *const u16, len: u32, flags: u32, components: *mut UrlComponents) -> i32 {
    if components.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: checked non-null; the caller owns the structure for this call.
    let c = unsafe { &mut *components };
    if c.struct_size as usize != core::mem::size_of::<UrlComponents>() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's URL.
    let Some(units) = (unsafe { url_units(url, len) }) else { return fail(ERROR_INVALID_PARAMETER) };
    let k = match url::crack(units) {
        Ok(k) => k,
        Err(url::UrlError::Scheme) => return fail(ERROR_UNRECOGNIZED_SCHEME),
        Err(url::UrlError::Invalid) => return fail(ERROR_INVALID_URL),
    };
    if flags & ICU_REJECT_USERPWD != 0 && (!k.user.is_empty() || !k.password.is_empty()) {
        return fail(ERROR_INVALID_URL);
    }
    // SAFETY: as above.
    if let Err(e) = unsafe { fill(c, units, &k, flags) } {
        return fail(e);
    }
    TRUE
}

/// A `URL_COMPONENTS` string field: `len` 0 means NUL-terminated.
unsafe fn read_component(ptr: *const u16, len: u32) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    if len == 0 {
        // SAFETY: a NUL-terminated string.
        return unsafe { inet::wstr(ptr) };
    }
    // SAFETY: `len` units of the caller's.
    let units = unsafe { core::slice::from_raw_parts(ptr, len as usize) };
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Some(char::decode_utf16(units[..end].iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect())
}

/// `WinHttpCreateUrl`: a URL from `URL_COMPONENTS`; a buffer too small (or
/// none) is `ERROR_INSUFFICIENT_BUFFER` with the characters needed, the
/// terminator included.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpCreateUrl(components: *const UrlComponents, flags: u32, out: *mut u16, len: *mut u32) -> i32 {
    if components.is_null() || len.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: checked non-null; the caller owns both for this call.
    let (c, out_len) = unsafe { (&*components, &mut *len) };
    if c.struct_size as usize != core::mem::size_of::<UrlComponents>() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's fields.
    let (scheme, host, user, password, path, extra) = unsafe {
        (
            read_component(c.scheme, c.scheme_length).unwrap_or_default(),
            read_component(c.host_name, c.host_name_length).unwrap_or_default(),
            read_component(c.user_name, c.user_name_length).unwrap_or_default(),
            read_component(c.password, c.password_length).unwrap_or_default(),
            read_component(c.url_path, c.url_path_length).unwrap_or_default(),
            read_component(c.extra_info, c.extra_info_length).unwrap_or_default(),
        )
    };
    if flags & ICU_REJECT_USERPWD != 0 && (!user.is_empty() || !password.is_empty()) {
        return fail(ERROR_INVALID_URL);
    }
    let parts = Parts {
        scheme: &scheme,
        scheme_id: c.scheme_id,
        host: &host,
        port: c.port,
        user: &user,
        password: &password,
        path: &path,
        extra: &extra,
    };
    let built = match url::create(&parts, flags & ICU_ESCAPE != 0) {
        Ok(u) => u,
        Err(url::UrlError::Scheme) => return fail(ERROR_UNRECOGNIZED_SCHEME),
        Err(url::UrlError::Invalid) => return fail(ERROR_INVALID_URL),
    };
    let units: Vec<u16> = built.encode_utf16().collect();
    let need = units.len() + 1;
    if out.is_null() || (*out_len as usize) < need {
        *out_len = need as u32;
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: the caller's buffer holds `need` units.
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len());
        out.add(units.len()).write(0);
    }
    *out_len = units.len() as u32;
    TRUE
}

// ----- HTTP dates: WinHttpTimeFromSystemTime and WinHttpTimeToSystemTime

/// `SYSTEMTIME` (`minwinbase.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

/// `WinHttpTimeFromSystemTime`: the preferred HTTP date (RFC 9110 section
/// 5.6.7) into the caller's `WINHTTP_TIME_FORMAT_BUFSIZE` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpTimeFromSystemTime(time: *const SystemTime, out: *mut u16) -> i32 {
    if time.is_null() || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: checked non-null; the caller owns it for this call.
    let t = unsafe { *time };
    let date = DateTime {
        year: t.year,
        month: t.month,
        day_of_week: t.day_of_week,
        day: t.day,
        hour: t.hour,
        minute: t.minute,
        second: t.second,
        millis: t.milliseconds,
    };
    let Some(text) = date::format_http_date(&date) else { return fail(ERROR_INVALID_PARAMETER) };
    let units: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: the caller's buffer of `WINHTTP_TIME_FORMAT_BUFSIZE` bytes,
    // longer than the 30 units written.
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len());
        out.add(units.len()).write(0);
    }
    TRUE
}

/// `WinHttpTimeToSystemTime`: any of the three HTTP date forms into
/// `SYSTEMTIME` (`wDayOfWeek` worked out from the date, `wMilliseconds`
/// zero).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WinHttpTimeToSystemTime(text: *const u16, time: *mut SystemTime) -> i32 {
    if text.is_null() || time.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's string.
    let Some(s) = (unsafe { inet::wstr(text) }) else { return fail(ERROR_INVALID_PARAMETER) };
    let Some(d) = date::parse_http_date(&s) else { return fail(ERROR_INVALID_PARAMETER) };
    // SAFETY: checked non-null.
    unsafe {
        *time = SystemTime {
            year: d.year,
            month: d.month,
            day_of_week: d.day_of_week,
            day: d.day,
            hour: d.hour,
            minute: d.minute,
            second: d.second,
            milliseconds: d.millis,
        }
    };
    TRUE
}
