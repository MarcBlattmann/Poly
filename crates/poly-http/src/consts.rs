//! WinHTTP (`winhttp.h`) and WinINet (`wininet.h`) constants. Both
//! headers give their errors the same base (12000) and their query levels
//! the same numbers.

// ----- errors (winhttp.h: WINHTTP_ERROR_BASE + n; wininet.h: INTERNET_ERROR_BASE + n)

pub const ERROR_TIMEOUT: u32 = 12002;
pub const ERROR_INTERNAL_ERROR: u32 = 12004;
pub const ERROR_INVALID_URL: u32 = 12005;
pub const ERROR_UNRECOGNIZED_SCHEME: u32 = 12006;
pub const ERROR_NAME_NOT_RESOLVED: u32 = 12007;
pub const ERROR_INVALID_OPTION: u32 = 12009;
pub const ERROR_OPERATION_CANCELLED: u32 = 12017;
pub const ERROR_INCORRECT_HANDLE_TYPE: u32 = 12018;
pub const ERROR_INCORRECT_HANDLE_STATE: u32 = 12019;
pub const ERROR_CANNOT_CONNECT: u32 = 12029;
pub const ERROR_CONNECTION_ERROR: u32 = 12030;
pub const ERROR_HEADER_NOT_FOUND: u32 = 12150;
pub const ERROR_INVALID_SERVER_RESPONSE: u32 = 12152;
pub const ERROR_INVALID_QUERY_REQUEST: u32 = 12154;
/// `ERROR_WINHTTP_SECURE_FAILURE`: what an `https` request fails with
/// while Poly has no TLS.
pub const ERROR_SECURE_FAILURE: u32 = 12175;
/// `ERROR_INTERNET_SECURITY_CHANNEL_ERROR` (`wininet.h`): WinINet's
/// answer to an `https` request while Poly has no TLS (12175 means
/// something else there).
pub const ERROR_INTERNET_SECURITY_CHANNEL_ERROR: u32 = 12157;

/// `winerror.h`.
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_NOT_SUPPORTED: u32 = 50;

// ----- query levels and flags

pub const QUERY_CONTENT_TYPE: u32 = 1;
pub const QUERY_CONTENT_LENGTH: u32 = 5;
pub const QUERY_DATE: u32 = 9;
pub const QUERY_VERSION: u32 = 18;
pub const QUERY_STATUS_CODE: u32 = 19;
pub const QUERY_STATUS_TEXT: u32 = 20;
pub const QUERY_RAW_HEADERS: u32 = 21;
pub const QUERY_RAW_HEADERS_CRLF: u32 = 22;
pub const QUERY_LOCATION: u32 = 33;
pub const QUERY_SERVER: u32 = 37;
pub const QUERY_CUSTOM: u32 = 65535;
pub const QUERY_FLAG_REQUEST_HEADERS: u32 = 0x8000_0000;
pub const QUERY_FLAG_SYSTEMTIME: u32 = 0x4000_0000;
pub const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
pub const QUERY_MODIFIER_FLAGS_MASK: u32 = 0xF000_0000;

// ----- WinHTTP

pub const WINHTTP_FLAG_ASYNC: u32 = 0x1000_0000;
pub const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
pub const INTERNET_DEFAULT_PORT: u16 = 0;
pub const INTERNET_DEFAULT_HTTP_PORT: u16 = 80;
pub const INTERNET_DEFAULT_HTTPS_PORT: u16 = 443;
pub const WINHTTP_ADDREQ_FLAG_ADD: u32 = 0x2000_0000;
pub const WINHTTP_ADDREQ_FLAG_REPLACE: u32 = 0x8000_0000;
/// `WINHTTP_OPTION_*` read back by `WinHttpQueryOption`.
pub const WINHTTP_OPTION_CONNECT_TIMEOUT: u32 = 3;
pub const WINHTTP_OPTION_RECEIVE_TIMEOUT: u32 = 6;
pub const WINHTTP_OPTION_SEND_TIMEOUT: u32 = 5;

// ----- WinINet

pub const INTERNET_SERVICE_HTTP: u32 = 3;
pub const INTERNET_FLAG_SECURE: u32 = 0x0080_0000;
pub const INTERNET_FLAG_ASYNC: u32 = 0x1000_0000;
pub const HTTP_ADDREQ_FLAG_ADD: u32 = 0x2000_0000;
pub const HTTP_ADDREQ_FLAG_REPLACE: u32 = 0x8000_0000;
