//! HTTP/1.1 without the machine (ADR 0095), for our `winhttp.dll` and
//! `wininet.dll`:
//!
//! - [`url`]: `http://host[:port]/path` taken apart (`WinHttpCrackUrl`,
//!   `InternetOpenUrlW`); `https` is recognised and refused by the DLLs
//!   until Poly has TLS.
//! - [`request`]: a request's bytes (RFC 9110, RFC 9112 section 3): the
//!   request line, `Host`, `User-Agent`, the caller's headers,
//!   `Content-Length`.
//! - [`response`]: an incremental response parser: interim `1xx`
//!   responses skipped, the status line and header fields, the body framed
//!   by `Transfer-Encoding: chunked`, `Content-Length`, or the end of the
//!   connection (RFC 9112 section 6.3), no body for `HEAD`, `204` and
//!   `304`.
//! - [`query`]: `WinHttpQueryHeaders` / `HttpQueryInfoW`'s information
//!   levels (`winhttp.h`, `wininet.h`: the same numbering) answered from a
//!   parsed response.
//! - [`consts`]: the WinHTTP and WinINet constants and error codes the
//!   DLLs use.
//!
//! Sources (clean room): RFC 9110 and RFC 9112; Microsoft Learn's WinHTTP
//! and WinINet reference pages (each function, "Query Info Flags",
//! "WinHTTP error codes", "Internet error codes"); the public headers
//! `winhttp.h` and `wininet.h` for constants. No Windows binary, Wine or
//! ReactOS source was looked at.

#![no_std]

extern crate alloc;

pub mod consts;
pub mod exports;
pub mod query;
pub mod request;
pub mod response;
pub mod url;
