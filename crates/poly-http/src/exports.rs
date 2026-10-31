//! The functions our `winhttp.dll` and `wininet.dll` export (by name),
//! for the check `cargo xtask win32` makes of the built DLLs (ADR 0095).

/// `winhttp.dll`'s exports, sorted.
pub const WINHTTP_EXPORTS: &[&str] = &[
    "WinHttpAddRequestHeaders",
    "WinHttpCheckPlatform",
    "WinHttpCloseHandle",
    "WinHttpConnect",
    "WinHttpOpen",
    "WinHttpOpenRequest",
    "WinHttpQueryDataAvailable",
    "WinHttpQueryHeaders",
    "WinHttpReadData",
    "WinHttpReceiveResponse",
    "WinHttpSendRequest",
    "WinHttpSetTimeouts",
    "WinHttpWriteData",
];

/// `wininet.dll`'s exports, sorted.
pub const WININET_EXPORTS: &[&str] = &[
    "HttpAddRequestHeadersW",
    "HttpOpenRequestW",
    "HttpQueryInfoW",
    "HttpSendRequestW",
    "InternetCloseHandle",
    "InternetConnectW",
    "InternetOpenUrlW",
    "InternetOpenW",
    "InternetQueryDataAvailable",
    "InternetReadFile",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_and_unique() {
        assert!(WINHTTP_EXPORTS.windows(2).all(|w| w[0] < w[1]));
        assert!(WININET_EXPORTS.windows(2).all(|w| w[0] < w[1]));
    }
}
