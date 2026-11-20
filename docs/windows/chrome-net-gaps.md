# Chrome's network path on Windows: what it calls, and what Poly's sheets did not say

Track `spec-win-chrome-setup` (spec agent, 2026-11-20), for the QA finding "pages load slowly on Poly" (GitHub still blank after
14 s, BBC images about 20 s) and the implementer `chrome-qa-fixes`. Observed behaviour only; no Google or Microsoft binary was
decompiled or disassembled (CLEAN_ROOM.md).

## 1. What was recorded

Chrome for Testing 157.0.8103.38 (the build of ADR 0155), run **headless** (`--headless --no-sandbox --disable-gpu --dump-dom <url>`) with the
counting tracer kit of `spec-chrome-win-2` (ADR 0090, `tools/wintrace/polytrace-agg`: every call through Chrome's import tables of
`chrome.exe`, `chrome.dll`, `chrome_elf.dll`; first four calls of each function in full with their arguments, counters for the rest) on the Windows oracle
(homeserver2, Windows 11 26H2 build 26300.9974, SSH session, internet), two runs each of `https://github.com/` (7 and 5 s on Windows) and
`https://www.bbc.com/news` (5 s). The reduced, committed form is `specs/windows/apps/chrome/traces/net-github.trace.json` and `net-bbc.trace.json`
(`poly-chrometrace/1`, the functions both runs called, with counts and first-call rank, per process type; the network service is the type
`utility:network.mojom.NetworkService`). Kit hashes are those of the recorded build. The oracle's work directory `C:\polynet` was removed after the job.

## 2. What the network service does (the order of first calls, github.com run)

1. `CreateIoCompletionPort(NULL, concurrency 1)`, `PostQueuedCompletionStatus`, `GetQueuedCompletionStatus(.., 0 ms)`: the message pump. a handle (Mojo's pipe, from the call pattern) is
   associated with the port (`CreateIoCompletionPort(handle, port, key, 1)`).
2. `WSAStartup`; three `RegisterWaitForSingleObject(.., INFINITE, 12)`; `GetAdaptersAddresses`; `WSALookupServiceBeginW/NextW/End`;
   `WSACreateEvent` + `NotifyAddrChange(overlapped)` + a registered wait on its event (the network-change notifier).
3. A throw-away **IPv6 probe**: `WSASocketW(AF_INET6, SOCK_DGRAM, UDP, NULL, 0, WSA_FLAG_OVERLAPPED)`, `setsockopt(IPV6_V6ONLY)`,
   `setsockopt(SOL_SOCKET, SO_RANDOMIZE_PORT = 0x3005)`, `connect`, `getsockname`, `closesocket`.
4. `getaddrinfo("wpad")` three times from worker threads (proxy auto-detection), then `getaddrinfo` of each real host (17 per page; synchronous, on worker threads), `DhcpRequestParams` by `GetProcAddress`.
5. Per TCP connection (38 sockets on GitHub, 19-28 on BBC): `WSASocketW(AF_INET6, SOCK_STREAM, TCP, NULL, 0, 1)`, `ioctlsocket(FIONBIO)`,
   `setsockopt(IPPROTO_TCP, TCP_NODELAY)`, `WSAIoctl(SIO_KEEPALIVE_VALS, 12 bytes in, nothing out)`, `WSACreateEvent`, `WSAEventSelect(FD_CONNECT = 0x10)`,
   `setsockopt(SO_RANDOMIZE_PORT)`, `connect` (-1, WSAEWOULDBLOCK), `RegisterWaitForSingleObject(event, INFINITE, flags 12)`; when the event fires:
   `UnregisterWaitEx`, `WSAEnumNetworkEvents(s, event)`, `getsockname`, `recv(s, buf, 1, MSG_PEEK)` (-1, WSAEWOULDBLOCK) probes, `WSASend` with a `WSAOVERLAPPED` and event (returns 0, completed inline) +
   `WSAResetEvent`, `WSAEventSelect(FD_READ | FD_CLOSE = 0x21)`, `recv(s, buf, 4096, 0)` until WSAEWOULDBLOCK, then waits again (register, fire, unregister,
   `WSAEnumNetworkEvents`: about 11 cycles per connection); end: `shutdown(SD_SEND)`, `closesocket`, `WSACloseEvent` ×2.
6. QUIC to google hosts: `getsockopt(SO_RCVBUF / SO_SNDBUF)`, `sendto` of 1,230-byte datagrams to port 443 (315-381 calls per GitHub page), `ntohs/htons` on every datagram's address (3,400 `ntohs` per page).

Per page (GitHub / BBC): `GetQueuedCompletionStatus` 2,575-3,032 / 2,177; `PostQueuedCompletionStatus` 217-330 / 183-217; `CreateEventW` 463-505 / 277-303;
`RegisterWaitForSingleObject` 420-463 / 232-260 and `UnregisterWaitEx` the same; `WSAEnumNetworkEvents` 384-428 / 210-243; `recv` 418-534 / 430-432; `WSASend` 148-158 / 141-158;
**`WriteFile` 12,633-13,158 / 8,278-8,293 (Mojo messages on overlapped named pipes, completing inline) and `ReadFile` 499-548 / 471-508 (pending reads, 997, ended by completion packets)**.
The full table of the network service's I/O calls is in section 5.

## 3. What Chrome 157 does **not** call (in any of its processes, in these runs)

`GetQueuedCompletionStatusEx`, `SetFileCompletionNotificationModes`, `WSARecv`, `WSARecvFrom`, `WSASendTo`, `WSASendMsg/WSARecvMsg`, `WSAConnect`, `GetAddrInfoExW`,
`GetOverlappedResult`, `WSAGetOverlappedResult`, `WaitNamedPipeW`, waitable timers, timer-queue timers. (Only functions imported by Chrome's binaries are visible; names
Chrome resolves with `GetProcAddress` are listed in the trace's `objects`: for the network service `DhcpRequestParams`, `DhcpCApiInitialize`, `GetAdaptersAddresses`, `NotifyAddrChange`,
`CancelIPChangeNotify`, `ProcessPrng`, `PostThreadMessageW`, `PeekMessageW`, registry and ETW calls; no Winsock function.) Poly work on these would not help Chrome's page loads.
**The UDP receive path of QUIC was not seen** (no `recvfrom`/`WSARecvFrom` in the table; `recv` is called 418+ times and may serve connected UDP sockets too): unknown, say so before an implementer
spends time there.

## 4. Gap list: what Chrome calls and what the sheets said before this batch

Status from `coverage/` and `specs/windows/` on `main` at 2026-11-20. Rank = the order of importance for page-load speed.

| # | Call or behaviour | Per page | Before | Now |
|---|---|---|---|---|
| 1 | **overlapped `WriteFile` / `ReadFile` on named pipes + completion port** (Mojo) | 13,000 / 500 | **no sheet at all** (`WriteFile`, `ReadFile` claimed by the generic `kernel32` agent, 0 tests) | `WriteFile.OverlappedPipe`, `ReadFile.OverlappedPipe`, `CompletionPortPump` (3 entries, 9 tests) |
| 2 | **`RegisterWaitForSingleObject` flags 12 (`WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE`), hundreds live, `UnregisterWaitEx` three forms** | 460 | sheet 2/1 tests, `not_covered: WT_EXECUTEINWAITTHREAD, thousands of registered waits`; `UnregisterWaitEx` 0 tests | `RegisterWaitForSingleObject.InWaitThread` (3 tests) |
| 3 | **`WSAEventSelect` + `WSAEnumNetworkEvents` life cycle of a client socket** (edge re-arming, FD_CONNECT/READ/WRITE/CLOSE, event reset, MSG_PEEK probe, inline overlapped `WSASend`) | 38 sockets, 400 cycles | sheets of the single calls (6/3, 2/1), no scenario | `WSAEventSelect.TcpLifecycle` (3 tests) |
| 4 | `WSACreateEvent`/`WSAResetEvent`/`WSACloseEvent` | 70 | `WSAResetEvent`, `WSACloseEvent`: 0 tests | `WSAEvents` (3 tests) |
| 5 | `htons`/`ntohs`/`htonl`/`ntohl` | 3,400 `ntohs` | no sheet | `ByteOrder` (3 tests) |
| 6 | `freeaddrinfo` after `getaddrinfo` (incl. failed lookups, NULL) | 17 | 0 tests | `AddrInfoLists` (3 tests) |
| 7 | `NotifyAddrChange` + `CancelIPChangeNotify` | 1 | cancel: 0 tests | `CancelIPChangeNotify.Pending` (3 tests) |
| 8 | `getaddrinfo` of `wpad` and the hosts, synchronous on worker threads: how long a failing lookup takes | 17 + 3 | sheet 4/2 (correctness, no time) | **not specified: see the hint below** |
| 9 | UDP: `sendto` of 1,230-byte datagrams, receive path unknown | 315-381 | `sendto` sheet 2/1 | not specified (receive path unseen) |
| 10 | `WSALookupServiceNextW` / `WSALookupServiceEnd` | 1 | 0 tests | not specified (needs a name-space provider; Chrome only needs `Begin` to fail or return an empty set: `never ok` on Windows) |
| 11 | `setsockopt(SO_RANDOMIZE_PORT)`, `SIO_KEEPALIVE_VALS`, `TCP_NODELAY`, `FIONBIO`, `getsockname`, `shutdown` | per socket | all have sheets (14 rules `setsockopt`, 8 `WSAIoctl`) | covered; the scenario test uses them |
| 12 | `CryptProtectMemory`/`CryptUnprotectMemory`, `GetAdaptersAddresses`, `DhcpCApiInitialize` | 8 + 81 / 7 / 1 | green sheets | covered |

**Hints for the slowness (to check on Poly, not conclusions):**
- *The first connection waits for proxy auto-detection.* The first calls of the network service are `getaddrinfo("wpad")` ×3 and `DhcpRequestParams` before any connect (on Windows they fail at once). A name lookup
  that takes seconds to fail (a DNS timeout) on Poly would leave the page blank for exactly that long.
- *Event-select edge semantics.* `cnet_tcp` shows the exact rules (an event is signalled again only after the reader has seen WSAEWOULDBLOCK or after new data; `WSAEnumNetworkEvents` resets it). A socket
  layer that signals once and not again stalls a download until something else wakes the thread.
- *The wait thread.* All callbacks of registered waits run one after another on one wait thread (observed: 200 live waits, 1 thread; a blocked callback delays the next). Poly must neither
  serialise more than that nor run the callbacks inline in `RegisterWait`/`SetEvent`; and `UnregisterWaitEx(h, INVALID_HANDLE_VALUE)` must wait only for a *running* callback.
- *Pipe completion.* Inline completion with an event and a packet queued for every completion (also the inline ones) is what Mojo's pump counts on (`cnet_iocp`); a write that returns pending
  with 997 while there is room, or a lost packet, shows as latency of a whole message round trip.

## 5. The network service's I/O calls (github.com | bbc.com | the older S4 run, counts per page, `utility:network.mojom.NetworkService`)

The numbers live in the two trace files; a readable extract:

| first-call rank | dll | function | github | bbc | S4 (example.com + iana.org) |
|---|---|---|---|---|---|
| 78 | kernel32 | CreateEventW | 463-505 | 277-303 | 110-111 |
| 101 | kernel32 | ResetEvent | 401-413 | 318-320 | 125-127 |
| 108 | kernel32 | SetEvent | 360-361 | 230-241 | 156-163 |
| 109 | kernel32 | CreateIoCompletionPort | 10 | 10 | 12 |
| 112 | kernel32 | PostQueuedCompletionStatus | 217-330 | 183-217 | 85-89 |
| 113 | kernel32 | GetQueuedCompletionStatus | 2575-3032 | 2177-2178 | 916-921 |
| 117 | kernel32 | WriteFile | 12633-13158 | 8278-8293 | 757-761 |
| 118 | kernel32 | ReadFile | 499-548 | 471-508 | 332-335 |
| 121-126 | ws2_32 / iphlpapi | WSAStartup, GetAdaptersAddresses | 1, 7 | 1, 7 | 1, 7 |
| 123 | kernel32 | RegisterWaitForSingleObject | 420-463 | 232-260 | 66-67 |
| 147 | ws2_32 | WSACreateEvent | 71-75 | 36-54 | 17 |
| 173-179 | ws2_32 | WSASocketW, setsockopt, connect, getsockname, closesocket, getaddrinfo | 38, 111, 38, 67, 38, 17 | 19-28, 56-83, 19-28, 34-52, 19-28, 16 | 9, 24, 9, 14, 9, 7 |
| 182-188 | ws2_32 | ioctlsocket, WSAIoctl, WSAEventSelect, WSAEnumNetworkEvents, recv, WSASend | 30, 36, 65, 384-428, 418-534, 148-158 | 15-24, 18-27, 33-51, 210-243, 430-432, 141-158 | 6, 8, 14, 57, 60-63, 18 |
| 185 | kernel32 | UnregisterWaitEx | 417-460 | 229-257 | 62-63 |
| 190-199 | ws2_32 | shutdown, getsockopt, sendto, htons/ntohs | 30, 8-12, 315-381, 85-122 / 2790-3410 | 15-24, 6, 49-53, 24 / 105-119 | 6, 4, 44-48, 20 / 102-108 |
| 207 | kernel32 | CancelIoEx | 6 | 6 | 8 |

## 6. New sheets (all handed off to `chrome-qa-fixes`, recorded on homeserver2, every program run twice with identical output)

`kernel32.dll/WriteFile.OverlappedPipe`, `ReadFile.OverlappedPipe`, `CompletionPortPump`, `RegisterWaitForSingleObject.InWaitThread`; `ws2_32.dll/WSAEventSelect.TcpLifecycle`,
`WSAEvents`, `ByteOrder`, `AddrInfoLists`; `iphlpapi.dll/CancelIPChangeNotify.Pending`. Each entry is three programs of the same calls writing every third observation line
(2 visible, 1 held out). The facts an implementer would not guess:
- a write that fits completes **inline**: `TRUE`, the count in the call, the event signalled, `Internal` 0, `InternalHigh` = the count, **and a completion packet is queued** on an associated handle;
- a read with no data returns `FALSE`/997 with `Internal` = `STATUS_PENDING`; `GetOverlappedResult(.., FALSE)` before completion gives 996; the peer's write completes the read and queues its packet **before** the write's own packet;
- cancel gives 995 with a packet, the peer's close gives 109 with a packet; a write after the reader closed gives 232;
- registered waits: flags 12 callbacks run on the wait thread, **serialised**; `UnregisterWaitEx(h, NULL)` and `(h, event)` return `FALSE`/997 while the callback runs, `(h, INVALID_HANDLE_VALUE)` blocks until it returned;
- `WSAEnumNetworkEvents` resets the event; FD_READ is signalled again by the next data or after WSAEWOULDBLOCK, FD_WRITE signals at once when selected on a connected socket;
- `WSACloseEvent(INVALID_HANDLE_VALUE)` returns TRUE; `WSAResetEvent(NULL)` is FALSE with 6; `CancelIPChangeNotify` of a pending `NotifyAddrChange` signals its event.

Not on Chrome's path, not specified: `GetQueuedCompletionStatusEx` batching, `SetFileCompletionNotificationModes`, overlapped `WSARecv`/`WSARecvFrom`, `GetAddrInfoExW`, TCP fast open.
