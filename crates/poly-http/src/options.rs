//! The options of a WinHTTP handle: what `WinHttpSetOption` sets and
//! `WinHttpQueryOption` reads back, and the callback of
//! `WinHttpSetStatusCallback`.
//!
//! Each option applies to some kinds of handle (a session, a connection, a
//! request), takes a buffer of a fixed size and, for some, a range of
//! values; some are read-only (`HANDLE_TYPE`), some write-only (the value
//! is accepted but cannot be queried). The table follows the spec sheets
//! of `WinHttpSetOption` and `WinHttpSetStatusCallback`; the numbers and
//! the defaults are `winhttp.h`'s and Learn's ("Option Flags",
//! "WinHttpSetTimeouts", "WinHttpSetStatusCallback").
//!
//! Every value fits in 8 bytes, so a handle keeps them as `(option, u64)`
//! pairs. A child handle starts with a copy of its session's values.

use alloc::vec::Vec;

use crate::consts::{ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_OPTION, ERROR_INVALID_PARAMETER};

/// `ERROR_WINHTTP_INCORRECT_HANDLE_TYPE` (`winhttp.h`).
pub const ERROR_INCORRECT_HANDLE_TYPE: u32 = crate::consts::ERROR_INCORRECT_HANDLE_TYPE;
/// `ERROR_WINHTTP_OPTION_NOT_SETTABLE` (`winhttp.h`).
pub const ERROR_OPTION_NOT_SETTABLE: u32 = 12011;

// `WINHTTP_OPTION_*` (`winhttp.h`).
pub const OPTION_CALLBACK: u32 = 1;
pub const OPTION_RESOLVE_TIMEOUT: u32 = 2;
pub const OPTION_CONNECT_TIMEOUT: u32 = 3;
pub const OPTION_CONNECT_RETRIES: u32 = 4;
pub const OPTION_SEND_TIMEOUT: u32 = 5;
pub const OPTION_RECEIVE_TIMEOUT: u32 = 6;
pub const OPTION_RECEIVE_RESPONSE_TIMEOUT: u32 = 7;
pub const OPTION_HANDLE_TYPE: u32 = 9;
pub const OPTION_SECURITY_FLAGS: u32 = 31;
pub const OPTION_URL: u32 = 34;
pub const OPTION_CONTEXT_VALUE: u32 = 45;
pub const OPTION_HTTP_VERSION: u32 = 59;
pub const OPTION_CODEPAGE: u32 = 68;
pub const OPTION_MAX_CONNS_PER_SERVER: u32 = 73;
pub const OPTION_MAX_CONNS_PER_1_0_SERVER: u32 = 74;
pub const OPTION_AUTOLOGON_POLICY: u32 = 77;
pub const OPTION_ENABLE_FEATURE: u32 = 79;
pub const OPTION_SECURE_PROTOCOLS: u32 = 84;
pub const OPTION_REDIRECT_POLICY: u32 = 88;
pub const OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS: u32 = 89;
pub const OPTION_MAX_RESPONSE_HEADER_SIZE: u32 = 91;
pub const OPTION_MAX_RESPONSE_DRAIN_SIZE: u32 = 92;
pub const OPTION_REJECT_USERPWD_IN_URL: u32 = 100;
pub const OPTION_UNSAFE_HEADER_PARSING: u32 = 110;
pub const OPTION_DECOMPRESSION: u32 = 118;

/// `WINHTTP_CALLBACK_FLAG_*` of the completions only an asynchronous handle
/// reports (`winhttp.h`: `SENDREQUEST_COMPLETE`, `HEADERS_AVAILABLE`,
/// `DATA_AVAILABLE`, `READ_COMPLETE`, `WRITE_COMPLETE`, `REQUEST_ERROR`,
/// `GETPROXYFORURL_COMPLETE`).
pub const CALLBACK_FLAG_ALL_COMPLETIONS: u32 = 0x017E_0000;
/// `WINHTTP_INVALID_STATUS_CALLBACK` (`winhttp.h`): `(callback)-1`.
pub const INVALID_STATUS_CALLBACK: u64 = u64::MAX;

/// The kind of a handle; its number is `WINHTTP_HANDLE_TYPE_*` (`winhttp.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Session = 1,
    Connect = 2,
    Request = 3,
}

/// What a kind of handle may do with an option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Yes,
    /// The call fails with this error.
    No(u32),
}

use Access::{No, Yes};

const WRONG: Access = No(ERROR_INCORRECT_HANDLE_TYPE);

/// An option: its buffer size, who may set it (session, connection,
/// request), who may read it (`None`: write-only, the query fails with
/// `ERROR_INVALID_PARAMETER`), and its default.
struct Rule {
    option: u32,
    size: u32,
    set: Option<[Access; 3]>,
    query: Option<[Access; 3]>,
    default: u64,
}

/// Session and request (a connection only reads its session's value).
const SR: [Access; 3] = [Yes, WRONG, Yes];
const ALL: [Access; 3] = [Yes, Yes, Yes];
const S_ONLY: [Access; 3] = [Yes, WRONG, WRONG];
const R_ONLY: [Access; 3] = [WRONG, WRONG, Yes];

const fn rule(option: u32, size: u32, set: [Access; 3], query: Option<[Access; 3]>, default: u64) -> Rule {
    Rule { option, size, set: Some(set), query, default }
}

/// The options Poly knows; any other number is `ERROR_WINHTTP_INVALID_OPTION`.
const RULES: &[Rule] = &[
    rule(OPTION_CALLBACK, 8, ALL, Some(ALL), 0),
    // The timeouts and retries (Learn, "WinHttpSetTimeouts": resolve none,
    // connect 60 s, send and receive 30 s; the response 90 s; five tries).
    rule(OPTION_RESOLVE_TIMEOUT, 4, SR, Some(ALL), 0),
    rule(OPTION_CONNECT_TIMEOUT, 4, SR, Some(ALL), 60_000),
    rule(OPTION_CONNECT_RETRIES, 4, SR, Some(ALL), 5),
    rule(OPTION_SEND_TIMEOUT, 4, SR, Some(ALL), 30_000),
    rule(OPTION_RECEIVE_TIMEOUT, 4, SR, Some(ALL), 30_000),
    rule(OPTION_RECEIVE_RESPONSE_TIMEOUT, 4, SR, Some(ALL), 90_000),
    Rule { option: OPTION_HANDLE_TYPE, size: 4, set: None, query: Some(ALL), default: 0 },
    rule(OPTION_SECURITY_FLAGS, 4, R_ONLY, Some(R_ONLY), 0),
    Rule { option: OPTION_URL, size: 0, set: None, query: Some(R_ONLY), default: 0 },
    rule(OPTION_CONTEXT_VALUE, 8, ALL, Some(ALL), 0),
    // HTTP_VERSION_INFO: major 1 (low DWORD), minor 1.
    rule(OPTION_HTTP_VERSION, 8, ALL, Some(ALL), 0x1_0000_0001),
    rule(OPTION_CODEPAGE, 4, S_ONLY, None, 0),
    rule(OPTION_MAX_CONNS_PER_SERVER, 4, S_ONLY, Some(ALL), u32::MAX as u64),
    rule(OPTION_MAX_CONNS_PER_1_0_SERVER, 4, S_ONLY, Some(ALL), u32::MAX as u64),
    rule(OPTION_AUTOLOGON_POLICY, 4, SR, Some(SR), 0),
    rule(OPTION_ENABLE_FEATURE, 4, [No(ERROR_INVALID_PARAMETER), WRONG, Yes], None, 0),
    rule(OPTION_SECURE_PROTOCOLS, 4, S_ONLY, None, 0),
    // WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP by default.
    rule(OPTION_REDIRECT_POLICY, 4, SR, Some(SR), 1),
    rule(OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS, 4, SR, Some(SR), 10),
    rule(OPTION_MAX_RESPONSE_HEADER_SIZE, 4, SR, Some(SR), 64 * 1024),
    rule(OPTION_MAX_RESPONSE_DRAIN_SIZE, 4, SR, Some(SR), 1024 * 1024),
    rule(OPTION_REJECT_USERPWD_IN_URL, 4, SR, Some(SR), 0),
    rule(OPTION_UNSAFE_HEADER_PARSING, 4, R_ONLY, None, 0),
    rule(OPTION_DECOMPRESSION, 4, SR, None, 0),
];

fn find(option: u32) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.option == option)
}

fn allowed(a: Access) -> Result<(), u32> {
    match a {
        Yes => Ok(()),
        No(e) => Err(e),
    }
}

/// The value range of an option: `ERROR_INVALID_PARAMETER` outside it.
fn in_range(option: u32, value: u64) -> bool {
    match option {
        // NEVER, DISALLOW_HTTPS_TO_HTTP, ALWAYS.
        OPTION_REDIRECT_POLICY => value <= 2,
        // WINHTTP_DECOMPRESSION_FLAG_GZIP | _DEFLATE.
        OPTION_DECOMPRESSION => value & !3 == 0,
        _ => true,
    }
}

/// What a query answers: the bytes to copy (a string with its NUL) and the
/// length to report on success (a string's without the NUL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub bytes: Vec<u8>,
    pub length: u32,
}

/// The option values of one handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    values: Vec<(u32, u64)>,
}

impl Default for Options {
    fn default() -> Options {
        Options { values: RULES.iter().filter(|r| r.set.is_some()).map(|r| (r.option, r.default)).collect() }
    }
}

impl Options {
    /// The value of a stored option (its default until set).
    pub fn get(&self, option: u32) -> Option<u64> {
        self.values.iter().find(|v| v.0 == option).map(|v| v.1)
    }

    fn put(&mut self, option: u32, value: u64) {
        match self.values.iter_mut().find(|v| v.0 == option) {
            Some(v) => v.1 = value,
            None => self.values.push((option, value)),
        }
    }

    /// `WinHttpSetOption` on a handle of `kind`: `len` is the caller's
    /// length, `buf` its bytes (at least `min(len, 8)` of them), `None` for
    /// a NULL buffer. Errors in the order the sheet observes them: an
    /// unknown option, a read-only one, the buffer's size, the handle's
    /// kind, the value.
    pub fn set(&mut self, kind: Kind, option: u32, buf: Option<&[u8]>, len: u32) -> Result<(), u32> {
        let rule = find(option).ok_or(ERROR_INVALID_OPTION)?;
        let set = rule.set.ok_or(ERROR_OPTION_NOT_SETTABLE)?;
        let bytes = match buf {
            _ if len == 0 && buf.is_none() => return Err(ERROR_INSUFFICIENT_BUFFER),
            _ if len == 0 => return Err(ERROR_INVALID_PARAMETER),
            _ if len != rule.size => return Err(ERROR_INSUFFICIENT_BUFFER),
            None => return Err(ERROR_INVALID_PARAMETER),
            Some(b) => b,
        };
        allowed(set[kind as usize - 1])?;
        let mut raw = [0u8; 8];
        let n = (rule.size as usize).min(bytes.len());
        raw[..n].copy_from_slice(&bytes[..n]);
        let value = u64::from_le_bytes(raw);
        if !in_range(option, value) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let value = match option {
            // Security flags can only be added.
            OPTION_SECURITY_FLAGS => self.get(option).unwrap_or(0) | value,
            _ => value,
        };
        self.put(option, value);
        Ok(())
    }

    /// `WinHttpQueryOption` on a handle of `kind`; `url` is a request's URL
    /// (`WINHTTP_OPTION_URL`).
    pub fn query(&self, kind: Kind, option: u32, url: Option<&str>) -> Result<Answer, u32> {
        let rule = find(option).ok_or(ERROR_INVALID_OPTION)?;
        let query = rule.query.ok_or(ERROR_INVALID_PARAMETER)?;
        allowed(query[kind as usize - 1])?;
        let fixed = |v: u64| Answer { bytes: v.to_le_bytes()[..rule.size as usize].to_vec(), length: rule.size };
        Ok(match option {
            OPTION_HANDLE_TYPE => fixed(kind as u64),
            OPTION_URL => {
                let mut bytes: Vec<u8> = url.unwrap_or("").encode_utf16().flat_map(u16::to_le_bytes).collect();
                let length = bytes.len() as u32;
                bytes.extend_from_slice(&[0, 0]);
                Answer { bytes, length }
            }
            _ => fixed(self.get(option).unwrap_or(rule.default)),
        })
    }

    /// `WinHttpSetStatusCallback(callback, flags, reserved)`: the previous
    /// callback, or the error with which the call returns
    /// `WINHTTP_INVALID_STATUS_CALLBACK`. Flags 0 change nothing and fail;
    /// flags that name only completions (which a synchronous handle never
    /// reports) return the callback in place without replacing it.
    pub fn set_callback(&mut self, callback: u64, flags: u32, reserved: u64) -> Result<u64, u32> {
        if reserved != 0 || flags == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let previous = self.get(OPTION_CALLBACK).unwrap_or(0);
        if flags & !CALLBACK_FLAG_ALL_COMPLETIONS != 0 {
            self.put(OPTION_CALLBACK, callback);
        }
        Ok(previous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set4(o: &mut Options, kind: Kind, option: u32, v: u32) -> Result<(), u32> {
        o.set(kind, option, Some(&v.to_le_bytes()), 4)
    }

    fn get4(o: &Options, kind: Kind, option: u32) -> Result<u32, u32> {
        let a = o.query(kind, option, None)?;
        assert_eq!(a.length, 4);
        Ok(u32::from_le_bytes(a.bytes[..4].try_into().unwrap()))
    }

    #[test]
    fn session_values_round_trip() {
        let mut o = Options::default();
        for (opt, v) in [
            (OPTION_REDIRECT_POLICY, 0),
            (OPTION_REDIRECT_POLICY, 2),
            (OPTION_CONNECT_RETRIES, 7),
            (OPTION_MAX_CONNS_PER_SERVER, 12),
            (OPTION_MAX_CONNS_PER_1_0_SERVER, 3),
            (OPTION_AUTOLOGON_POLICY, 3),
            (OPTION_MAX_HTTP_AUTOMATIC_REDIRECTS, 5),
            (OPTION_MAX_RESPONSE_HEADER_SIZE, 65536),
            (OPTION_MAX_RESPONSE_DRAIN_SIZE, 1000),
            (OPTION_CONNECT_TIMEOUT, 5000),
            (OPTION_SEND_TIMEOUT, 6000),
            (OPTION_RECEIVE_TIMEOUT, 7000),
            (OPTION_RESOLVE_TIMEOUT, 1000),
            (OPTION_RECEIVE_RESPONSE_TIMEOUT, 9000),
        ] {
            assert_eq!(set4(&mut o, Kind::Session, opt, v), Ok(()), "{opt}");
            assert_eq!(get4(&o, Kind::Session, opt), Ok(v), "{opt}");
        }
    }

    #[test]
    fn ranges_write_only_and_kinds() {
        let mut o = Options::default();
        set4(&mut o, Kind::Session, OPTION_REDIRECT_POLICY, 2).unwrap();
        assert_eq!(set4(&mut o, Kind::Session, OPTION_REDIRECT_POLICY, 3), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(get4(&o, Kind::Session, OPTION_REDIRECT_POLICY), Ok(2));
        for (opt, v) in [(OPTION_SECURE_PROTOCOLS, 0x2800), (OPTION_DECOMPRESSION, 3), (OPTION_CODEPAGE, 1252)] {
            assert_eq!(set4(&mut o, Kind::Session, opt, v), Ok(()));
            assert_eq!(o.query(Kind::Session, opt, None), Err(ERROR_INVALID_PARAMETER));
        }
        assert_eq!(set4(&mut o, Kind::Session, OPTION_DECOMPRESSION, 4), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set4(&mut o, Kind::Session, OPTION_ENABLE_FEATURE, 1), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(set4(&mut o, Kind::Session, OPTION_UNSAFE_HEADER_PARSING, 1), Err(ERROR_INCORRECT_HANDLE_TYPE));
        assert_eq!(set4(&mut o, Kind::Session, OPTION_SECURITY_FLAGS, 0x3300), Err(ERROR_INCORRECT_HANDLE_TYPE));
        assert_eq!(o.query(Kind::Session, OPTION_SECURITY_FLAGS, None), Err(ERROR_INCORRECT_HANDLE_TYPE));
        assert_eq!(set4(&mut o, Kind::Session, OPTION_HANDLE_TYPE, 1), Err(ERROR_OPTION_NOT_SETTABLE));
        assert_eq!(get4(&o, Kind::Session, OPTION_HANDLE_TYPE), Ok(1));
        assert_eq!(get4(&o, Kind::Connect, OPTION_HANDLE_TYPE), Ok(2));
        assert_eq!(get4(&o, Kind::Request, OPTION_HANDLE_TYPE), Ok(3));
        assert_eq!(set4(&mut o, Kind::Session, OPTION_HTTP_VERSION, 0x10001), Err(ERROR_INSUFFICIENT_BUFFER));
        let v = o.query(Kind::Session, OPTION_HTTP_VERSION, None).unwrap();
        assert_eq!((v.length, &v.bytes[..]), (8, &[1, 0, 0, 0, 1, 0, 0, 0][..]));
        // A connection reads its session's retries but sets nothing.
        set4(&mut o, Kind::Session, OPTION_CONNECT_RETRIES, 5).unwrap();
        let mut c = o.clone();
        assert_eq!(set4(&mut c, Kind::Connect, OPTION_CONNECT_RETRIES, 2), Err(ERROR_INCORRECT_HANDLE_TYPE));
        assert_eq!(get4(&c, Kind::Connect, OPTION_CONNECT_RETRIES), Ok(5));
        assert_eq!(set4(&mut c, Kind::Connect, OPTION_REDIRECT_POLICY, 1), Err(ERROR_INCORRECT_HANDLE_TYPE));
        assert_eq!(c.query(Kind::Connect, OPTION_REDIRECT_POLICY, None), Err(ERROR_INCORRECT_HANDLE_TYPE));
    }

    #[test]
    fn request_options() {
        let mut r = Options::default();
        set4(&mut r, Kind::Request, OPTION_SECURITY_FLAGS, 0x3300).unwrap();
        set4(&mut r, Kind::Request, OPTION_SECURITY_FLAGS, 0).unwrap();
        assert_eq!(get4(&r, Kind::Request, OPTION_SECURITY_FLAGS), Ok(0x3300));
        for opt in [OPTION_DECOMPRESSION, OPTION_UNSAFE_HEADER_PARSING, OPTION_ENABLE_FEATURE] {
            assert_eq!(set4(&mut r, Kind::Request, opt, 1), Ok(()));
            assert_eq!(r.query(Kind::Request, opt, None), Err(ERROR_INVALID_PARAMETER));
        }
        assert_eq!(set4(&mut r, Kind::Request, OPTION_SECURE_PROTOCOLS, 0x800), Err(ERROR_INCORRECT_HANDLE_TYPE));
        set4(&mut r, Kind::Request, OPTION_REJECT_USERPWD_IN_URL, 1).unwrap();
        assert_eq!(get4(&r, Kind::Request, OPTION_REJECT_USERPWD_IN_URL), Ok(1));
        let url = r.query(Kind::Request, OPTION_URL, Some("http://localhost/a/b?c=d")).unwrap();
        assert_eq!((url.length, url.bytes.len()), (48, 50));
        r.set(Kind::Request, OPTION_CONTEXT_VALUE, Some(&0x5151u64.to_le_bytes()), 8).unwrap();
        assert_eq!(r.query(Kind::Request, OPTION_CONTEXT_VALUE, None).unwrap().bytes, 0x5151u64.to_le_bytes());
    }

    #[test]
    fn sizes_and_unknown_options() {
        let mut o = Options::default();
        let one = 1u64.to_le_bytes();
        assert_eq!(o.set(Kind::Session, OPTION_REDIRECT_POLICY, Some(&one), 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(o.set(Kind::Session, OPTION_REDIRECT_POLICY, Some(&one), 3), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(o.set(Kind::Session, OPTION_REDIRECT_POLICY, Some(&one), 8), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(o.set(Kind::Session, OPTION_SECURE_PROTOCOLS, Some(&one), 2), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(o.set(Kind::Session, OPTION_REDIRECT_POLICY, None, 0), Err(ERROR_INSUFFICIENT_BUFFER));
        assert_eq!(o.set(Kind::Session, OPTION_REDIRECT_POLICY, None, 4), Err(ERROR_INVALID_PARAMETER));
        for opt in [0, 200, 0xFFFF] {
            assert_eq!(o.set(Kind::Session, opt, Some(&one), 4), Err(ERROR_INVALID_OPTION));
        }
        assert_eq!(get4(&o, Kind::Session, OPTION_REDIRECT_POLICY), Ok(1));
    }

    #[test]
    fn the_callback_sequence_of_the_sheet() {
        let (a, b) = (0x1000, 0x2000);
        let mut s = Options::default();
        assert_eq!(s.set_callback(a, 0xFFFF_FFFF, 0), Ok(0));
        assert_eq!(s.set_callback(b, 0x40000, 0), Ok(a));
        assert_eq!(s.set_callback(b, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(s.set_callback(0, 0xFFFF_FFFF, 0), Ok(a));
        assert_eq!(s.set_callback(0, 0xFFFF_FFFF, 0), Ok(0));
        assert_eq!(s.set_callback(a, 0x10, 0), Ok(0));
        assert_eq!(s.query(Kind::Session, OPTION_CALLBACK, None).unwrap().bytes, a.to_le_bytes());
        assert_eq!(s.set_callback(b, 0x8000_0000, 0), Ok(a));
        assert_eq!(s.set_callback(a, 0xFFFF_FFFF, 1), Err(ERROR_INVALID_PARAMETER));
    }
}
