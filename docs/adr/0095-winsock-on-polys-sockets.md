# 0095: Winsock on Poly's sockets: `ws2_32.dll` over `\Device\Afd`, an NT device in front of the kernel's TCP/IP stack

**Status:** Accepted. Delivers the Phase 5 roadmap item "Winsock
(`ws2_32`) on our sockets; `winhttp` / `wininet` basics" in two slices:
`ws2_32` first, then `winhttp` / `wininet` ("WinHTTP and WinINet" below). Builds on [ADR 0028](0028-tcp-ip-stack-in-a-crate.md) (the
`poly-net` stack), [ADR 0048](0048-sockets-inet-unix-netlink.md) (the
kernel's Internet sockets), [ADR 0067](0067-nt-file-io.md) (NT file I/O,
IRPs), [ADR 0079](0079-our-kernelbase-and-kernel32.md) (the Win32
workspace) and [ADR 0080](0080-console-host.md) (a device of Poly's own
with Poly's own control codes).

## Context

Every networked Windows program reaches the network through Winsock
(`ws2_32.dll`), and most of them through name resolution
(`getaddrinfo`) first. Poly already has a TCP/IP stack with IPv4, IPv6,
TCP, UDP and loopback (`poly-net`), driven by the kernel's network thread,
and blocking socket files on top of it for the Linux personality. What
was missing is a way for NT processes to use them, and the DLL.

On Windows, `ws2_32` is a thin layer over a kernel driver (AFD) reached
through file handles: a `SOCKET` *is* a file handle, which is why
`ReadFile`, `WriteFile`, `CloseHandle`-style handle duplication and
waiting on a socket handle all work. The driver's control interface is
not documented, so Poly cannot and need not reproduce it; only the
behaviour Learn documents at the Winsock API matters.

Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's Winsock reference
(each function's page; "Winsock Error Codes"; "SOL_SOCKET Socket
Options", "IPPROTO_TCP Socket Options", "IPPROTO_IPV6 Socket Options";
"WSAPOLLFD"; "addrinfo"; "sockaddr"; "OVERLAPPED"; "Defining I/O Control
Codes"); the public SDK headers' constants and layouts (`winsock2.h`,
`ws2def.h`, `ws2ipdef.h`, `ws2tcpip.h`, `winerror.h`, `ntstatus.h`,
`winioctl.h`, `minwinbase.h`); RFC 1035 and RFC 3596 (DNS messages), RFC
4291 and RFC 5952 (IPv6 text), RFC 6724 (address order), IANA's service
name registry (RFC 6335) for well-known ports; the coverage map's export
names. No Windows binary, Wine or ReactOS source was looked at.

## Decision

### Sockets are file objects on `\Device\Afd`

The NT personality gets a device `\Device\Afd` (beside `\Device\PolyFs*`,
`\Device\PolyPipe` and `\Device\PolyConsole`). `NtCreateFile` of
`\Device\Afd\Endpoint` (or the device itself) makes an **endpoint**: a file
object whose target is `Target::Socket(Endpoint)`, at first without a
socket. Everything else is `NtDeviceIoControlFile` with **Poly's own
codes** (`poly_winsock::afd`): `CTL_CODE(FILE_DEVICE_NETWORK, 0x800 + n,
METHOD_NEITHER, FILE_ANY_ACCESS)`, the function range Learn leaves to
vendors, little-endian buffers with fixed offsets, socket addresses as the
program's own `SOCKADDR_IN` / `SOCKADDR_IN6` bytes, and data buffers passed
by address and length (read and written in the caller's memory directly).

| Request | Does |
|---|---|
| `SOCKET` | make the kernel socket (family, type, protocol) |
| `BIND`, `LISTEN`, `CONNECT`, `SHUTDOWN` | the calls of the same name |
| `ACCEPT` | the next connection, put into another fresh endpoint whose handle the request names (so the device never makes handles itself) |
| `SEND`, `RECV` | data, with a destination / the sender for datagrams, `MSG_PEEK`, `MSG_WAITALL`, and an *overlapped* flag |
| `NAME` | `getsockname` / `getpeername` |
| `SET_OPTION`, `GET_OPTION` | options by Winsock's own level and name |
| `SET_NONBLOCKING`, `READABLE` | `FIONBIO`, `FIONREAD` |
| `POLL` | wait for any of a list of socket handles (`select`, `WSAPoll`) |
| `NET_INFO` | the DNS servers and the host name |
| `INFO` | what the endpoint is |

Why a device rather than new system services: a socket handle then is a
file handle in every respect the NT layer already has — the handle table,
`NtClose`, `NtDuplicateObject`, waiting on it, `NtCancelIoFile`, the I/O
request packets and their completion through events, status blocks and
APCs (ADR 0067), and `NtReadFile` / `NtWriteFile`, which on a socket
receive and send as on Windows. The protocol is private between our
`ws2_32` and the personality (like the console's, ADR 0080), so it can
change with both.

### The kernel's side: `NtSocket` over the Linux personality's sockets

The personality does not know the stack. It asks the kernel through four
new `NtKernel` methods (with defaults, so host fakes and kernels without
networking still work): `socket_create`, which returns an
`Arc<dyn NtSocket>`; `socket_poll` (several sockets at once);
`socket_pend` (an overlapped request to finish later); and `net_info`.
`NtSocket` is the personality's trait of socket calls with a `SockWait`
(not at all, for ever, or a number of milliseconds).

The kernel implements it (`kernel/src/nt/afd.rs`) over `net::file::
InetSocket`, the objects behind Linux socket descriptors (ADR 0048): the
same stack calls, the same wait queues notified by the network thread, the
same `ThreadBlocker` that parks the calling thread and ends the wait when
its process ends. A call that may wait runs in the calling thread with no
NT lock held. Closing the last handle *releases* the socket (waits on it
end with `EINTR`); it closes when the last reference goes, a TCP
connection finishing in the background or resetting under `SO_LINGER`
zero, as for Linux.

### Overlapped I/O

`WSASend`, `WSARecv`, `WSASendTo` and `WSARecvFrom` with an `OVERLAPPED`
issue the request with the `OVERLAPPED` as its `IO_STATUS_BLOCK`
(`Internal` / `InternalHigh` have the status block's places) and its
`hEvent` as the event. What can finish at once does (status block written,
event set, APC queued). What cannot becomes an IRP kept *waiting* and is
handed to the kernel's **`nt-afd` worker**, a kernel thread that retries
every queued request whenever the stack's readiness generation moves (and
every 50 ms, to drop cancelled ones), completes the IRP under the NT lock
and delivers the completion (data and status block into the requesting
process, the event, the APC) through the same path as the volume workers.
Closing the socket, `CancelIo` or the thread's end complete it with
`STATUS_CANCELLED`. A completion routine runs as the request's APC through
a small trampoline in `ws2_32` that turns the status block into the
routine's arguments. Requests with more than one buffer, and
`WSARecvFrom`'s address, are done synchronously and reported as completed.
I/O completion ports are not modelled yet (ADR 0067), so neither is
`AcceptEx` / `ConnectEx` (`SIO_GET_EXTENSION_FUNCTION_POINTER` is
`WSAEINVAL`).

### Errors: errno, `NTSTATUS`, WSA codes

The stack speaks Linux error numbers; requests complete with an
`NTSTATUS`; `ws2_32` sets a WSA code (`WSAGetLastError` is the thread's
last error, as Learn says). Both mappings are in `poly_winsock::error`,
Poly's own (each status the `ntstatus.h` value with the nearest documented
meaning) and checked by a host test to round-trip to the WSA code Learn
describes for each situation (`ECONNREFUSED` → `STATUS_CONNECTION_REFUSED`
→ `WSAECONNREFUSED`, `EAGAIN` → `STATUS_CANT_WAIT` → `WSAEWOULDBLOCK`, a
truncated datagram → `STATUS_BUFFER_OVERFLOW` → `WSAEMSGSIZE`, ...).
Errors decided before the stack sees a request are the personality's:
`bind` with the other family or a short address (`WSAEFAULT`), `connect`
with the other family (`WSAEAFNOSUPPORT`) or to the unspecified address /
port 0 (`WSAEADDRNOTAVAIL`), `listen` before `bind`, `accept` without
`listen` and `getsockname` before `bind` (`WSAEINVAL`), `listen` on a
datagram socket (`WSAEOPNOTSUPP`), an unknown option (`WSAENOPROTOOPT`),
a blocking receive whose `SO_RCVTIMEO` ran out (`WSAETIMEDOUT`).
`ws2_32` checks `socket`'s arguments itself (`WSAEAFNOSUPPORT`,
`WSAESOCKTNOSUPPORT`, `WSAEPROTOTYPE`, `WSAEPROTONOSUPPORT`) and that
`WSAStartup` was called (`WSANOTINITIALISED`).

### `ws2_32.dll`

A new DLL in the Win32 workspace (68 exports by name,
`poly_winsock::exports`, checked by `cargo xtask win32`): `WSAStartup` /
`WSACleanup` (version negotiation up to 2.2, `WSADATA` in its `_WIN64`
layout, a start count), `socket`, `WSASocketA/W`, `bind`, `listen`,
`accept`, `connect`, `WSAConnect`, `send`, `recv`, `sendto`, `recvfrom`,
`WSASend`, `WSARecv`, `WSASendTo`, `WSARecvFrom`,
`WSAGetOverlappedResult`, `closesocket` (`WSAENOTSOCK` for a handle that
is no socket), `shutdown`, `select`, `__WSAFDIsSet`, `WSAPoll`,
`ioctlsocket` / `WSAIoctl` (`FIONBIO`, `FIONREAD`, `SIOCATMARK`),
`getsockopt` / `setsockopt` (below), `getsockname`, `getpeername`,
`getaddrinfo`, `GetAddrInfoW`, `freeaddrinfo`, `FreeAddrInfoW`,
`getnameinfo`, `GetNameInfoW`, `gethostbyname`, `gethostname`,
`GetHostNameW`, `getservbyname`, `getservbyport`, `getprotobyname`,
`getprotobynumber`, `inet_pton`, `inet_ntop`, `InetPtonW`, `InetNtopW`,
`inet_addr`, `inet_ntoa`, `WSAAddressToStringA/W`, `WSAStringToAddressA/W`,
`htons` / `htonl` / `ntohs` / `ntohl` and their `WSA*` forms,
`WSAGetLastError` / `WSASetLastError`, and the event functions
(`WSACreateEvent` ... `WSAWaitForMultipleEvents`, Win32 events).

Options: `SO_REUSEADDR`, `SO_KEEPALIVE`, `SO_BROADCAST`, `SO_LINGER` /
`SO_DONTLINGER`, `SO_SNDBUF` / `SO_RCVBUF` (reported as set),
`SO_RCVTIMEO` / `SO_SNDTIMEO` (milliseconds, kept by the endpoint),
`SO_TYPE`, `SO_ERROR` (as a WSA code), `SO_ACCEPTCONN`, `TCP_NODELAY`,
`TCP_KEEPALIVE` / `TCP_KEEPINTVL` / `TCP_KEEPCNT`, `IP_TTL`,
`IPV6_UNICAST_HOPS`; `SO_DEBUG`, `SO_DONTROUTE`, `SO_OOBINLINE`,
`SO_EXCLUSIVEADDRUSE`, `IP_TOS` are stored and reported only.
`IPV6_V6ONLY` reads 1: the stack's IPv6 sockets carry only IPv6 (no
dual-stack sockets yet).

### Name resolution

`getaddrinfo`'s decisions are host-tested in `poly_winsock::resolve`:
hints checked, the service a decimal port or a well-known name, numeric
hosts (IPv4, IPv6 with a numeric scope), `localhost` and the machine's own
name answered with the loopback addresses (no `hosts` file yet), the empty
host as the unspecified (`AI_PASSIVE`) or loopback addresses; one entry
per address with the hint's socket type and the type's protocol; IPv6
before IPv4 (RFC 6724's default policy prefers `::1`). Other names go to a
**stub resolver** in `ws2_32` (messages in `poly_winsock::dns`): one UDP
query per record type (`AAAA`, `A`) to each DNS server `NET_INFO` names,
two seconds each, `CNAME`s followed within the answer, no TCP retry, no
cache, no search domains. The server is the kernel's `poly.net=...,dns=`
(10.0.2.3 with `poly.net=qemu`); without one a lookup is `WSATRY_AGAIN`.
`getnameinfo` gives `localhost` for loopback and the numeric form
otherwise (no reverse lookups yet). `gethostbyname` and friends return
data in one buffer per process (Learn: per thread).

### Tests

- Host: `poly-winsock` (constants, address layouts, address text, the
  error tables, the ioctl buffers, DNS queries and answers with
  compression and loops, `getaddrinfo` / `getnameinfo` decisions,
  `select` / `WSAPoll` bits, the export list); the personality's
  `afd::tests` drive the services against the fake kernel with sockets of
  a `poly-net` stack of their own: endpoints by name, TCP over 127.0.0.1
  and `::1`, UDP and truncation, the personality's errors, options,
  `POLL`, a receive timeout, overlapped requests, `NtReadFile` /
  `NtWriteFile` on a socket, `NET_INFO`.
- Ring 3 (`kernel_test!` `nt_winsock_program_talks_over_loopback`, every
  boot): `wstest.exe` through `ws2_32.dll` — start-up and clean-up
  counts, address text, `getaddrinfo` / `GetAddrInfoW` / `getnameinfo` /
  `gethostbyname` of `localhost`, TCP over 127.0.0.1 in one thread and
  64 KiB through an echo server in a second thread that blocks in `accept`
  and `recv`, TCP over `::1`, UDP with `recvfrom` and `WSAEMSGSIZE`,
  `select`, `WSAPoll`, `FIONBIO` / `FIONREAD`, options, a receive timeout,
  a refused connect, a non-blocking connect finished through `select`, an
  overlapped `WSARecv` posted before the data and completed by the
  `nt-afd` worker, and the error codes of the usual mistakes (159
  checks, all passing).

### WinHTTP and WinINet (second slice)

Sources, in addition: RFC 9110 and RFC 9112 (HTTP/1.1 semantics and
message syntax), RFC 3986 (URLs), Learn's WinHTTP and WinINet reference
pages ("Query Info Flags", "WinHTTP error codes", "Internet error
codes"), and the constants of `winhttp.h` and `wininet.h`.

- **`poly-http`** (host-tested): URLs taken apart (`http` / `https`,
  IPv6 literals, user information, the fragment dropped); a request's head
  (request line, `Host` with the port only when it is not the default,
  `User-Agent`, the caller's header lines replacing fields of the same
  name, `Content-Length`); an incremental response parser (interim `1xx`
  responses skipped, folded fields joined, the body framed as RFC 9112
  section 6.3 says: none for `HEAD`, `204` and `304`, `chunked` as the
  last coding, `Content-Length` with equal repeated values, else until the
  connection closes); and the header queries both APIs share (`winhttp.h`
  and `wininet.h` give the levels the same numbers): status code, text,
  version, raw headers with CRLFs or NULs, the field of each numbered
  level, `CUSTOM` by name, the index of repeated fields, `FLAG_NUMBER`.
- **`poly-win32-inet`**, a library both DLLs link: handles (a session, a
  connection, a request; checked against a list of live handles, so a
  stale value fails with `ERROR_INVALID_HANDLE`), one connection per
  request over our `ws2_32` (`getaddrinfo`, then each address until one
  answers, so `localhost` falls back from `::1` to 127.0.0.1), and the
  copying of answers into the caller's buffer (UTF-16 with a NUL, or a
  DWORD; a buffer too small is `ERROR_INSUFFICIENT_BUFFER` with the bytes
  needed, NUL included).
- **`winhttp.dll`**: `WinHttpOpen`, `WinHttpConnect` (port 0: 80),
  `WinHttpOpenRequest`, `WinHttpAddRequestHeaders`, `WinHttpSendRequest`,
  `WinHttpWriteData`, `WinHttpReceiveResponse`, `WinHttpQueryHeaders`,
  `WinHttpQueryDataAvailable`, `WinHttpReadData`, `WinHttpSetTimeouts`
  (the receive timeout), `WinHttpCloseHandle`, `WinHttpCheckPlatform`.
- **`wininet.dll`**: `InternetOpenW`, `InternetConnectW` (HTTP only),
  `HttpOpenRequestW`, `HttpAddRequestHeadersW`, `HttpSendRequestW` (sends
  and reads the response head), `InternetOpenUrlW`, `InternetReadFile`,
  `InternetQueryDataAvailable`, `HttpQueryInfoW` (a `CUSTOM` query names
  its field in the buffer, as Learn says), `InternetCloseHandle`.
- **Synchronous only**: the asynchronous modes (`WINHTTP_FLAG_ASYNC`,
  `INTERNET_FLAG_ASYNC`) fail with `ERROR_NOT_SUPPORTED`. Proxies are
  ignored, no authentication, cookies, caching, redirects or keep-alive
  reuse.
- **No TLS yet** (it needs the cryptography workstream): a WinHTTP request
  with `WINHTTP_FLAG_SECURE` fails when sent with
  `ERROR_WINHTTP_SECURE_FAILURE` (12175); a WinINet request with
  `INTERNET_FLAG_SECURE`, or `InternetOpenUrlW` of an `https` URL, with
  `ERROR_INTERNET_SECURITY_CHANNEL_ERROR` (12157, since 12175 means
  something else in `wininet.h`). Other schemes are
  `ERROR_INTERNET_UNRECOGNIZED_SCHEME`.
- **Tests**: `poly-http`'s host tests (URLs, heads, the parser fed byte by
  byte, chunks split anywhere, until-close bodies, malformed responses,
  queries); and the ring-3 `kernel_test!`
  `nt_http_get_through_winhttp_and_wininet` (every boot): `httptest.exe`
  runs a tiny HTTP server in a second thread on 127.0.0.1 (our `ws2_32`)
  and makes GET requests to it through both DLLs — status and header
  queries (a number, a custom field, the raw headers' size with no
  buffer, a field that is not there), bodies framed by `Content-Length`,
  chunks and the end of the connection, `localhost`, a 404, the `https`
  and `ftp` errors, a closed handle — while the server checks every
  request's `Host` and `User-Agent` (70 checks, all passing).

## To be confirmed by the spec agent against real Windows

1. `inet_addr` with trailing white space (ours: accepted) and `inet_pton`
   with leading zeros (ours: refused).
2. `getaddrinfo`'s order (ours: IPv6 first), its entries for a zero socket
   type (ours: one per address, type and protocol 0), and the protocol
   filled for a given type; `getnameinfo` of 127.0.0.1 (ours:
   `localhost`).
3. `WSACleanup` with sockets open (ours: they stay open), and `WSAStartup`
   with versions between 1.0 and 2.2 (ours: the request itself).
4. `getsockopt` of a BOOL into one byte (ours: allowed), `SO_SNDBUF` read
   back (ours: as set), `SO_ERROR` after it was read (ours: cleared).
5. `recv` on a stream socket that never connected (ours: `WSAENOTCONN`),
   `send` after `shutdown(SD_SEND)` (ours: `WSAESHUTDOWN`).
6. `WinHttpQueryHeaders` / `HttpQueryInfoW`'s length after a buffer too
   small (ours: bytes with the NUL) and after success (ours: without);
   the header fields WinHTTP adds by itself (ours: `Host`, `User-Agent`,
   `Content-Length` only); what `InternetOpenUrlW` of an `https` URL
   reports where TLS is missing (Windows always has it).

## Consequences

- New: `crates/poly-winsock`; `personalities/nt/src/afd` (`Device::Afd`,
  `Target::Socket`, the services, `NtSocket`); `kernel/src/nt/afd.rs`
  (the kernel socket, polling, the `nt-afd` worker);
  `personalities/nt/win32/ws2_32` and `wstest`; `kernel/src/nt/
  test_winsock.rs`.
- Shared files changed additively: the workspace members, the Win32
  workspace members and `OUTPUTS` / inputs / checks of
  `tools/xtask/src/win32.rs`, `kernel/build.rs`, the kernel's embedded
  modules and the layout's install list (`ws2_32.dll` was already a known
  DLL), `NtKernel`, the NT `Target` and `Device` enums, the dispatch of
  `NtDeviceIoControlFile` / `NtReadFile` / `NtWriteFile`, and the fake
  kernel of the personality's tests.
- A fix found on the way: a thread made by `NtCreateThreadEx` got a stack
  that did not grow (only the first thread's allocation was marked as a
  stack), so its first frame past the initial commit raised
  `STATUS_GUARD_PAGE_VIOLATION`; `exec::create_thread` now marks it too
  (host test `a_second_threads_stack_grows_too`; the echo thread of
  `wstest.exe`, with a 4 KiB buffer, needs it). Also seen: an unhandled
  exception whose user stack the kernel cannot walk overflows the kernel
  stack in `nt::exception::backtrace` (a double fault instead of the
  process ending); left for its own change.
- The second slice adds `crates/poly-http`, `personalities/nt/win32/
  {inet,winhttp,wininet,httptest}` and `kernel/src/nt/test_http.rs`, and
  the same shared lists again (`winhttp.dll` and `wininet.dll` installed
  in `System32`).
- Coverage: the 68 `ws2_32.dll` entries above, the 13 `winhttp.dll` and
  the 10 `wininet.dll` entries are claimed by `impl-winsock`.
- Left: I/O completion ports and the extension functions (`AcceptEx`,
  `ConnectEx`, `TransmitFile`), `WSAEventSelect` / `WSAAsyncSelect`,
  `WSAAccept`'s condition, dual-stack sockets, raw and ICMP sockets,
  out-of-band data, ordinal exports, the `hosts` file, reverse lookups,
  per-thread result buffers; for HTTP, TLS (`https`), the asynchronous
  modes, proxies, authentication, redirects, cookies, connection reuse,
  `WinHttpCrackUrl` and the ANSI forms of WinINet.
