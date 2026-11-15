# 0195: Winsock for Chrome's network stage: dual-stack sockets, overlapped `AcceptEx` / `ConnectEx`, and Windows' connection errors on Poly's stack

**Status:** Accepted. Builds on [ADR 0095](0095-winsock-on-polys-sockets.md) (`ws2_32` over
`\Device\Afd`), [ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)
(the stage ladder; `WSAEventSelect`, `WSADuplicateSocketW`) and
[ADR 0067](0067-nt-file-io.md) / [ADR 0169](0169-named-pipes-completion-ports-jobs-and-process-attributes-for-chrome.md)
(I/O requests and completion ports).

## Context

Chrome's network service (stage S4) talks to the network the way the spec sheets of
`spec-chrome-win-s4` observed on Windows: an `AF_INET6` socket with `IPV6_V6ONLY` turned off,
non-blocking, `WSAEventSelect`ed, `WSAIoctl(SIO_KEEPALIVE_VALS)`, `WSADuplicateSocketW` to hand it
over, and `WSARecvMsg` for UDP. Poly's stack has IPv6 sockets that carry only IPv6, errors that a
socket reports once (Linux's rule), and nothing for `AcceptEx` / `ConnectEx` (no `mswsock.dll`).
The observed differences were 155 of 183 tests of the sheets.

Sources (clean room): the spec sheets and visible tests of `specs/windows/{ws2_32,mswsock}.dll`,
Learn's Winsock reference ("Dual-Stack Sockets", "WSAEventSelect", "AcceptEx", "ConnectEx",
"SIO_KEEPALIVE_VALS", "SIO_GET_EXTENSION_FUNCTION_POINTER", "Winsock Error Codes"), `mswsock.h`,
`ws2ipdef.h`, `mstcpip.h`, `winsock2.h`.

## Decision

### Dual-stack sockets live in the personality

The stack's IPv6 sockets stay IPv6-only ([ADR 0028](0028-tcp-ip-stack-in-a-crate.md)). An endpoint
of family `AF_INET6` with `IPV6_V6ONLY` off (a flag of the endpoint, default on as on Windows)
uses an **IPv4 stack socket** when the program says IPv4:

- `connect` or `bind` to an IPv4-mapped address (`::ffff:a.b.c.d`) makes an IPv4 socket that
  replaces the endpoint's (its local port and kept options move over) and marks the endpoint
  *mapped*: every address it reports (`getsockname`, `getpeername`, `accept`, `recvfrom`) is the
  IPv4-mapped form; datagram destinations are unmapped. With `IPV6_V6ONLY` on, such an address is
  `WSAEADDRNOTAVAIL`; a 16-byte `AF_INET` address on an `AF_INET6` socket is `WSAEFAULT`.
- `bind` of `::` on a dual-stack stream socket also makes an IPv4 **twin** bound to the same port;
  `listen` listens on both, `accept` and `select` / `WSAPoll` look at both (a connection from the
  twin makes a mapped endpoint).
- A dual-stack datagram socket that is bound and unconnected only receives IPv6 (no twin); one that
  connects or sends to an IPv4 address switches to IPv4.

### Windows' connection errors on top of Linux' ones

The stack reports an error once; Winsock keeps failing the same way. The endpoint keeps a *broken*
status: a reset by the peer is `WSAECONNRESET` for every later `recv` and `send` (and discards
queued data), a reset after the peer had closed its side (its FIN came) or one we sent is
`WSAECONNABORTED`; a connect that failed keeps `SO_ERROR` (not cleared by reading) while the
socket is `WSAENOTCONN`; `shutdown(SD_SEND)` makes `send` `WSAESHUTDOWN`; `shutdown(SD_RECEIVE)`
makes the stack reset the connection when data is queued or arrives (`ECONNABORTED`,
`Stack::set_rd_shut_aborts`); closing a socket under a blocked `recv` aborts the connection
(`Stack::reset_connection`); `FIONREAD` of a datagram socket is the total queued
(`Stack::queued_bytes`). `FD_CLOSE` after a reset carries `WSAECONNABORTED` while `recv` says
`WSAECONNRESET` (Windows' own, observed).

### `WSAEventSelect` is evaluated where the program looks

The helper thread of ADR 0155 stays, with the observed rules in `poly_winsock::events`: `FD_READ` is
recorded when data *arrives* (also while earlier data is unread) and again after any `recv` that
leaves data; `FD_WRITE` once when selected and after a blocked send; `FD_CLOSE` comes with the data
that was sent before it, never after a local `SD_RECEIVE`; selecting again only evaluates events
that are new in the mask. `WSAEnumNetworkEvents` first looks at the socket itself (a zero-timeout
`POLL`), so it never answers with what a polling thread had not yet seen, and writes only the error
slots of events that happened.

### Overlapped accept and connect are requests of the device

`\Device\Afd` gets two requests, [`ACCEPT_EX`](../../crates/poly-winsock/src/afd.rs) and
`CONNECT_EX`, which always pend: the kernel's `nt-afd` worker retries them when the stack changes
and completes the request packet, so the event, the status block and a completion port get their
completion as for every asynchronous request. A finished `AcceptEx` also *places* the accepted
connection into the endpoint of the socket the program made (applied under the NT lock just before
the completion; `afd::services::place`), which tells its addresses only after
`SO_UPDATE_ACCEPT_CONTEXT` (as on Windows). `ConnectEx` sends its data after the connection.

`mswsock.dll` is new: `AcceptEx`, `GetAcceptExSockaddrs`, `ConnectEx`, `DisconnectEx`,
`TransmitFile` (refused: `WSAEOPNOTSUPP`). `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` returns
the DLL's exports for those GUIDs and `ws2_32`'s own `WSARecvMsg` / `WSASendMsg` for theirs.

### Smaller rules the sheets observed

`WSASocketW` / `socket` follow the matrix of the sheet (`check_socket`: `AF_UNSPEC` needs a
protocol, `AF_UNIX` streams are made, a wrong protocol is `WSAEPROTONOSUPPORT`, unknown flags
`WSAEPROTOTYPE`, a group `WSAEINVAL`; success leaves the last error 0; handles are inheritable
unless `WSA_FLAG_NO_HANDLE_INHERIT`). `getsockopt` returns options in their own size (one byte for
`SO_KEEPALIVE`, `TCP_NODELAY`, ...), refuses a short buffer with `WSAEFAULT` and sets the length 0
for an unknown option. `SIO_KEEPALIVE_VALS`, `SIO_UDP_CONNRESET`, `SIO_BASE_HANDLE` work.
`WSAStartup` clamps a request to what its major version has (1.1, 2.2) and fills `iMaxSockets` /
`iMaxUdpDg` for Winsock 1. `WSAPoll` reports the end of a stream as `POLLHUP` without
`POLLRDNORM`, and a failed connect (error and hang-up) to a poll for writing only; `select` puts a
failed connect in the except set alone. `WSASocketW` from a `WSADuplicateSocketW` record makes a
handle of its own to the duplicated socket, blocking again.

## Consequences

- New: `personalities/nt/win32/mswsock`, `ws2_32/src/ext.rs`; `Endpoint` fields (`v6only`,
  `mapped`, `twin`, `broken`, `conn_error`, `connected`, `peer`, `needs_update`, `shut_rd`,
  `shut_wr`); `PendOp::{Accept, Connect}` and `Placement`; `NtSocket::{disconnect, abort,
  established}`; `poly-net`: `Stack::{set_rd_shut_aborts, reset_connection, queued_bytes}`.
- Left: dual-stack datagram sockets receiving both families while unconnected, `TransmitFile`,
  `WSAAsyncSelect`, per-handle blocking state for a socket shared between processes (a duplicate is
  made blocking when it is taken up), out-of-band data.

## Where stage 4 stands

`cargo xtask chrome-win run --stage 4 --release` (a new rung of the ladder: the spec's `s4.html`
with an `http://example.com/` and an `https://www.iana.org/` frame, headless, `--no-sandbox`, QEMU's
user networking) boots Chrome, which resolves both names, connects with event-selected non-blocking
sockets (`WSAEventSelect`, `WSAEnumNetworkEvents`, `SIO_KEEPALIVE_VALS`, `WSASend` with an
`OVERLAPPED`), verifies the TLS certificate with its own store and prints the DOM the spec recorded
on Windows (831 bytes, SHA-256 `568f0bdf…2330be`, `http=1 https=1`). Four of five runs passed; the
fifth ended `0xffff7002` like the stage 1 runs that ended that way (ADR 0155's addendum).
