//! The functions our `ws2_32.dll` exports (by name), for the check
//! `cargo xtask win32` makes of the built DLL (ADR 0095).
//!
//! Ordinal exports (programs linked with older import libraries bind some
//! of `ws2_32` by ordinal) are not there yet: the ordinals are
//! export-table data the coverage map does not carry so far.

/// Every export, sorted.
pub const WS2_32_EXPORTS: &[&str] = &[
    "FreeAddrInfoW",
    "GetAddrInfoW",
    "GetHostNameW",
    "GetNameInfoW",
    "InetNtopW",
    "InetPtonW",
    "WSAAddressToStringA",
    "WSAAddressToStringW",
    "WSACleanup",
    "WSACloseEvent",
    "WSAConnect",
    "WSACreateEvent",
    "WSAGetLastError",
    "WSAGetOverlappedResult",
    "WSAHtonl",
    "WSAHtons",
    "WSAIoctl",
    "WSANtohl",
    "WSANtohs",
    "WSAPoll",
    "WSARecv",
    "WSARecvFrom",
    "WSAResetEvent",
    "WSASend",
    "WSASendTo",
    "WSASetEvent",
    "WSASetLastError",
    "WSASocketA",
    "WSASocketW",
    "WSAStartup",
    "WSAStringToAddressA",
    "WSAStringToAddressW",
    "WSAWaitForMultipleEvents",
    "__WSAFDIsSet",
    "accept",
    "bind",
    "closesocket",
    "connect",
    "freeaddrinfo",
    "getaddrinfo",
    "gethostbyname",
    "gethostname",
    "getnameinfo",
    "getpeername",
    "getprotobyname",
    "getprotobynumber",
    "getservbyname",
    "getservbyport",
    "getsockname",
    "getsockopt",
    "htonl",
    "htons",
    "inet_addr",
    "inet_ntoa",
    "inet_ntop",
    "inet_pton",
    "ioctlsocket",
    "listen",
    "ntohl",
    "ntohs",
    "recv",
    "recvfrom",
    "select",
    "send",
    "sendto",
    "setsockopt",
    "shutdown",
    "socket",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_and_unique() {
        assert!(WS2_32_EXPORTS.windows(2).all(|w| w[0] < w[1]));
    }
}
