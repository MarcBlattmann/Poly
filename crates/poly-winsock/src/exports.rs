//! The functions our `ws2_32.dll` exports (by name), for the check
//! `cargo xtask win32` makes of the built DLL (ADR 0095).
//!
//! Programs linked against `ws2_32.lib` of any Windows version bind part of
//! the library by ordinal (Chrome's `chrome.dll` imports `closesocket` as
//! `#3`); [`ORDINALS`] are the numbers those import tables use, pinned by
//! the DLL's build script.

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
    "WSADuplicateSocketW",
    "WSAEnumNetworkEvents",
    "WSAEventSelect",
    "WSAGetLastError",
    "WSAGetOverlappedResult",
    "WSAHtonl",
    "WSAHtons",
    "WSAIoctl",
    "WSALookupServiceBeginW",
    "WSALookupServiceEnd",
    "WSALookupServiceNextW",
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

/// The ordinals of the Winsock 1.1 interface and the `WSA` start-up and
/// error calls, as import tables of programs name them (`(name, ordinal)`).
/// The first 23 are the classic `wsock32` functions in alphabetical order;
/// Chrome's trace labels `closesocket (#3)`, `connect (#4)`,
/// `getsockname (#6)`, `getsockopt (#7)`, `htons (#9)`, `ioctlsocket (#10)`,
/// `ntohs (#15)`, `recv (#16)`, `sendto (#20)`, `setsockopt (#21)`,
/// `shutdown (#22)`, `WSAGetLastError (#111)` and `WSAStartup (#115)`.
pub const ORDINALS: &[(&str, u16)] = &[
    ("accept", 1),
    ("bind", 2),
    ("closesocket", 3),
    ("connect", 4),
    ("getpeername", 5),
    ("getsockname", 6),
    ("getsockopt", 7),
    ("htonl", 8),
    ("htons", 9),
    ("ioctlsocket", 10),
    ("inet_addr", 11),
    ("inet_ntoa", 12),
    ("listen", 13),
    ("ntohl", 14),
    ("ntohs", 15),
    ("recv", 16),
    ("recvfrom", 17),
    ("select", 18),
    ("send", 19),
    ("sendto", 20),
    ("setsockopt", 21),
    ("shutdown", 22),
    ("socket", 23),
    ("gethostbyname", 52),
    ("getprotobyname", 53),
    ("getprotobynumber", 54),
    ("getservbyname", 55),
    ("getservbyport", 56),
    ("gethostname", 57),
    ("WSAGetLastError", 111),
    ("WSASetLastError", 112),
    ("WSAStartup", 115),
    ("WSACleanup", 116),
    ("__WSAFDIsSet", 151),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_are_exported_names_and_unique() {
        for (i, &(name, ord)) in ORDINALS.iter().enumerate() {
            assert!(WS2_32_EXPORTS.contains(&name), "{name}");
            assert!(ORDINALS[i + 1..].iter().all(|&(n, o)| n != name && o != ord), "{name} / {ord}");
        }
    }

    #[test]
    fn the_ordinals_chrome_imports_resolve() {
        // The ordinals chrome.dll's import table names (specs/windows/apps/chrome): all of them are pinned.
        let used = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 14, 15, 16, 17, 19, 20, 21, 22, 23, 57, 111, 115];
        for o in used {
            assert!(ORDINALS.iter().any(|&(_, n)| n == o), "#{o}");
        }
    }

    #[test]
    fn sorted_and_unique() {
        assert!(WS2_32_EXPORTS.windows(2).all(|w| w[0] < w[1]));
    }
}
