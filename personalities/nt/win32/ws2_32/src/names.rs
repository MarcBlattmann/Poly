//! Names and addresses (Learn's page of each): `getaddrinfo` /
//! `GetAddrInfoW` / `freeaddrinfo` / `FreeAddrInfoW`, `getnameinfo` /
//! `GetNameInfoW`, `gethostbyname`, `gethostname` / `GetHostNameW`,
//! services and protocols, `inet_pton` / `inet_ntop` / `InetPtonW` /
//! `InetNtopW`, `inet_addr`, `inet_ntoa`, `WSAAddressToString*`,
//! `WSAStringToAddress*`, and byte order.
//!
//! Names that are neither numeric nor this machine's go to the stub
//! resolver: a UDP query to each DNS server `\Device\Afd` names (QEMU's
//! user network gives 10.0.2.3), two seconds each, through this DLL's own
//! sockets. The decisions are `poly_winsock::resolve`'s and
//! `poly_winsock::dns`'s.
//!
//! `gethostbyname`, `getservbyname` and `inet_ntoa` return data in
//! buffers Winsock owns; Learn makes them per thread, Poly has one per
//! process (to be revisited with thread-local storage).

use crate::lock::Locked;
use crate::sock::{self, Socket};
use crate::{Handle, afd, check_started, cstr, fail, heap_alloc, heap_free, set_error, wstr};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::net::{IpAddr, SocketAddr};
use core::sync::atomic::{AtomicU16, Ordering};
use poly_winsock::cache::{CacheLookup, DnsCache};
use poly_winsock::consts::*;
use poly_winsock::dns::{self, DnsError};
use poly_winsock::hosts;
use poly_winsock::resolve::{self, Entry, Hints, Host};
use poly_winsock::{addr, text};

// ----- the stub resolver, hosts file and cache

/// How long one server gets to answer a DNS batch.
const DNS_TIMEOUT_MS: u32 = 2000;
/// Once one query of an A+AAAA batch returned addresses, wait at most this
/// long for the second family before returning what already arrived.
const DNS_SECOND_GRACE_MS: u64 = 250;
/// Local-network fallback wait (LLMNR / mDNS / NetBIOS) for single-label and
/// `.local` names without `AI_DNS_ONLY` when DNS found no addresses.
/// Observed on Windows 11: ~1.13–1.29 s (spec bounds: 300–4000 ms).
const LOCAL_FALLBACK_WAIT_MS: u32 = 1100;
/// Largest `hosts` file read (bytes).
const MAX_HOSTS_BYTES: usize = 256 * 1024;

const HOSTS_BYTES: &[u8] = b"C:\\Windows\\System32\\drivers\\etc\\hosts\0";
const HOSTS_PATH: [u16; HOSTS_BYTES.len()] = {
    let mut w = [0u16; HOSTS_BYTES.len()];
    let mut i = 0;
    while i < HOSTS_BYTES.len() {
        w[i] = HOSTS_BYTES[i] as u16;
        i += 1;
    }
    w
};

const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_ALL: u32 = 7;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_HANDLE_VALUE: Handle = -1;

/// Process-wide DNS cache (negative NXDOMAIN + positive RR). Never locked
/// across network I/O, file I/O or sleeps so parallel lookups never block
/// one another.
static DNS_CACHE: Locked<DnsCache> = Locked::new(DnsCache::new());

/// Monotonic DNS transaction id counter so concurrent threads never collide.
static NEXT_DNS_ID: AtomicU16 = AtomicU16::new(0x5a17);

fn now_ms() -> u64 {
    // SAFETY: kernel32 query with no preconditions.
    unsafe { crate::GetTickCount64() }
}

/// Read `C:\Windows\System32\drivers\etc\hosts` fresh so live edits are seen
/// immediately by subsequent lookups.
fn read_hosts_file() -> Option<Vec<u8>> {
    // SAFETY: `HOSTS_PATH` is NUL-terminated UTF-16.
    let h = unsafe {
        crate::CreateFileW(
            HOSTS_PATH.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_ALL,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if h == INVALID_HANDLE_VALUE || h == 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    while out.len() < MAX_HOSTS_BYTES {
        let want = chunk.len().min(MAX_HOSTS_BYTES - out.len()) as u32;
        let mut got = 0u32;
        // SAFETY: `h` is our open file handle and `chunk` / `got` are live locals.
        let ok = unsafe { crate::ReadFile(h, chunk.as_mut_ptr(), want, &mut got, core::ptr::null_mut()) };
        if ok == 0 || got == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..got as usize]);
    }
    // SAFETY: `h` was opened by `CreateFileW` above and is closed once.
    unsafe { crate::CloseHandle(h) };
    Some(out)
}

/// Send all `qtypes` (1 or 2) in parallel on one UDP socket to `server:53`
/// and collect replies until all are answered, any returns `NXDOMAIN`, or the
/// deadline expires.
fn ask_batch(
    s: Socket,
    server: [u8; 4],
    name: &str,
    qtypes: &[u16],
) -> (Vec<(u16, Result<dns::Answer, DnsError>)>, bool) {
    let base = NEXT_DNS_ID.fetch_add(qtypes.len().max(1) as u16, Ordering::Relaxed);
    let (to, tolen) = addr::encode(&SocketAddr::new(IpAddr::from(server), dns::PORT));
    let mut pending: Vec<(u16, u16, Option<Result<dns::Answer, DnsError>>)> = Vec::with_capacity(qtypes.len());
    let mut sent = 0usize;
    for (i, &qt) in qtypes.iter().enumerate() {
        let id = base.wrapping_add(i as u16);
        let q = match dns::query(id, name, qt) {
            Ok(q) => q,
            Err(e) => return (alloc::vec![(qt, Err(e))], false),
        };
        // SAFETY: local buffers.
        if unsafe { sock::sendto(s, q.as_ptr(), q.len() as i32, 0, to.as_ptr(), tolen as i32) } >= 0 {
            pending.push((id, qt, None));
            sent += 1;
        } else {
            pending.push((id, qt, Some(Err(DnsError::TryAgain))));
        }
    }
    if sent == 0 {
        return (pending.into_iter().map(|(_, qt, r)| (qt, r.unwrap_or(Err(DnsError::TryAgain)))).collect(), false);
    }

    let mut deadline = now_ms().saturating_add(u64::from(DNS_TIMEOUT_MS));
    let mut got_addrs = false;
    let mut buf = [0u8; dns::MAX_UDP];
    let mut stray = 0u32;

    while pending.iter().any(|(_, _, r)| r.is_none()) {
        let now = now_ms();
        if now >= deadline {
            break;
        }
        let rem = (deadline - now).clamp(1, u64::from(u32::MAX)) as u32;
        let timeout = rem.to_le_bytes();
        // SAFETY: a local value of the option's size.
        unsafe { sock::setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, timeout.as_ptr(), 4) };
        let n = sock::recv(s, buf.as_mut_ptr(), buf.len() as i32, 0);
        if n < 0 {
            break;
        }
        let pkt = &buf[..n as usize];
        let Some(rid) = dns::reply_id(pkt) else {
            stray += 1;
            if stray >= 16 {
                break;
            }
            continue;
        };
        let Some(slot) = pending.iter_mut().find(|(id, _, r)| *id == rid && r.is_none()) else {
            stray += 1;
            if stray >= 16 {
                break;
            }
            continue;
        };
        let parsed = dns::parse(pkt, slot.0, name, slot.1);
        if parsed == Err(DnsError::NotFound) {
            // NXDOMAIN applies to every RR type of the domain: short-circuit immediately.
            return (Vec::new(), true);
        }
        if let Ok(ref ans) = parsed
            && !ans.addrs.is_empty()
            && !got_addrs
        {
            got_addrs = true;
            deadline = deadline.min(now_ms().saturating_add(DNS_SECOND_GRACE_MS));
        }
        slot.2 = Some(parsed);
    }

    let results = pending.into_iter().map(|(_, qt, r)| (qt, r.unwrap_or(Err(DnsError::TryAgain)))).collect();
    (results, false)
}

/// Resolve `name` via the `hosts` file, the DNS cache, UDP DNS queries, and
/// (for single-label / `.local` names without `AI_DNS_ONLY`) the local-network
/// fallback wait.
fn lookup_name(name: &str, h: &Hints) -> Result<(Vec<IpAddr>, Option<String>), u32> {
    // 1. Live hosts file always takes precedence over DNS and the negative cache.
    if let Some(bytes) = read_hosts_file()
        && let Some(entry) = hosts::lookup(&bytes, name)
    {
        return Ok((entry.addrs, Some(entry.canonical)));
    }
    // Reject syntactically invalid host names immediately without DNS or fallback wait.
    if !dns::is_valid_name(name) {
        return Err(resolve::dns_error_wsa(DnsError::BadName));
    }

    let types = resolve::query_types(h);
    let mut addrs = Vec::new();
    let mut canonical: Option<String> = None;
    let mut missing: Vec<u16> = Vec::with_capacity(types.len());
    let mut dns_nxdomain = false;

    // 2. Consult the in-process DNS cache under a brief lock.
    let now = now_ms();
    DNS_CACHE.with(|cache| {
        if cache.is_negative(name, now) {
            dns_nxdomain = true;
            return;
        }
        for &t in types {
            match cache.lookup(name, t, now) {
                CacheLookup::Negative => {
                    dns_nxdomain = true;
                    missing.clear();
                    return;
                }
                CacheLookup::Positive(a) => {
                    addrs.extend(a.addrs);
                    if canonical.is_none() {
                        canonical = a.canonical;
                    }
                }
                CacheLookup::Miss => missing.push(t),
            }
        }
    });

    // 3. Query DNS servers for any uncached record types (with no lock held).
    if !dns_nxdomain && !missing.is_empty() {
        let info = afd::net_info();
        for server in info.servers() {
            let s = sock::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
            if s == INVALID_SOCKET {
                break;
            }
            let (results, nx) = ask_batch(s, *server, name, &missing);
            sock::closesocket(s);
            if nx {
                DNS_CACHE.with(|c| c.insert_negative(name, now_ms()));
                addrs.clear();
                break;
            }
            let mut retry_types = Vec::new();
            let ts = now_ms();
            for (qt, res) in results {
                match res {
                    Ok(a) => {
                        DNS_CACHE.with(|c| c.insert_positive(name, qt, &a, ts));
                        addrs.extend(a.addrs);
                        if canonical.is_none() {
                            canonical = a.canonical;
                        }
                    }
                    Err(DnsError::NoData) => {}
                    Err(DnsError::NotFound) => {
                        DNS_CACHE.with(|c| c.insert_negative(name, ts));
                        addrs.clear();
                        retry_types.clear();
                        break;
                    }
                    // Timeouts / SERVFAIL are never cached.
                    Err(DnsError::TryAgain | DnsError::Fail | DnsError::BadName) => {
                        retry_types.push(qt);
                    }
                }
            }
            missing = retry_types;
            if missing.is_empty() {
                break;
            }
        }
    }

    // 4. Local-network fallback wait for single-label / `.local` names when
    // `AI_DNS_ONLY` is not set and DNS did not resolve the name. Not cached
    // and runs without any lock so parallel lookups proceed concurrently.
    if addrs.is_empty() {
        if h.flags & AI_DNS_ONLY == 0 && dns::needs_local_fallback(name) {
            // SAFETY: kernel32 sleep with no preconditions.
            unsafe { crate::Sleep(LOCAL_FALLBACK_WAIT_MS) };
        }
        return Err(resolve::dns_error_wsa(DnsError::NotFound));
    }
    Ok((addrs, canonical))
}

/// `getaddrinfo`'s answer: the entries and the canonical name.
fn resolve_all(node: Option<&str>, service: Option<&str>, h: &Hints) -> Result<(Vec<Entry>, Option<String>), u32> {
    check_started()?;
    if node.is_none() && service.is_none_or(str::is_empty) {
        return Err(EAI_NONAME);
    }
    resolve::check_hints(h, node)?;
    let port = resolve::service_port(service, h.flags)?;
    let own = afd::net_info();
    let (host, scope) = resolve::classify(node, h, own.host_name())?;
    let (addrs, canonical) = match host {
        Host::Addrs(a) => (a, None),
        Host::SelfAddrs => (resolve::self_addrs(&afd::local_addrs(), h), Some(String::from(own.host_name()))),
        Host::Lookup(name) => lookup_name(&name, h)?,
    };
    let numeric = node.is_some_and(|n| resolve::numeric_host(n).is_some());
    let entries = resolve::entries(&addrs, port, scope, h, numeric);
    if entries.is_empty() {
        return Err(EAI_NONAME);
    }
    let canonical = canonical.or_else(|| node.filter(|n| !n.is_empty()).map(String::from));
    Ok((entries, canonical))
}

unsafe fn read_hints(p: *const u8) -> Hints {
    if p.is_null() {
        return Hints::default();
    }
    // SAFETY: the caller's ADDRINFO hints.
    unsafe {
        Hints {
            flags: core::ptr::read_unaligned(p.add(ADDRINFO_FLAGS).cast::<i32>()),
            family: core::ptr::read_unaligned(p.add(ADDRINFO_FAMILY).cast::<i32>()),
            socktype: core::ptr::read_unaligned(p.add(ADDRINFO_SOCKTYPE).cast::<i32>()),
            protocol: core::ptr::read_unaligned(p.add(ADDRINFO_PROTOCOL).cast::<i32>()),
            // `ai_addrlen`, `ai_canonname`, `ai_addr` and `ai_next` must be zero (`WSANO_RECOVERY` else).
            reserved: core::ptr::read_unaligned(p.add(ADDRINFO_ADDRLEN).cast::<usize>()) != 0
                || core::ptr::read_unaligned(p.add(ADDRINFO_CANONNAME).cast::<usize>()) != 0
                || core::ptr::read_unaligned(p.add(ADDRINFO_ADDR).cast::<usize>()) != 0
                || core::ptr::read_unaligned(p.add(ADDRINFO_NEXT).cast::<usize>()) != 0,
        }
    }
}

/// Build the `ADDRINFO` list: one heap block per entry (the structure,
/// then its socket address, then the canonical name on the first if
/// asked), in either character width.
fn build_list(entries: &[Entry], canonical: Option<&str>, flags: i32, wide: bool) -> Result<*mut u8, u32> {
    let mut head: *mut u8 = core::ptr::null_mut();
    let mut tail: *mut u8 = core::ptr::null_mut();
    for (i, e) in entries.iter().enumerate() {
        let (sa, salen) = addr::encode(&e.addr);
        let name: Vec<u8> = match canonical {
            Some(c) if i == 0 && flags & AI_CANONNAME != 0 => {
                if wide {
                    c.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
                } else {
                    c.bytes().chain([0]).collect()
                }
            }
            _ => Vec::new(),
        };
        let size = ADDRINFO_SIZE + addr::MAX_SIZE + name.len();
        let p = heap_alloc(size);
        if p.is_null() {
            // SAFETY: a list this function built.
            unsafe { free_list(head) };
            return Err(EAI_MEMORY);
        }
        // SAFETY: a fresh zeroed block of `size` bytes.
        unsafe {
            core::ptr::write_unaligned(p.add(ADDRINFO_FLAGS).cast::<i32>(), e.flags);
            core::ptr::write_unaligned(p.add(ADDRINFO_FAMILY).cast::<i32>(), e.family);
            core::ptr::write_unaligned(p.add(ADDRINFO_SOCKTYPE).cast::<i32>(), e.socktype);
            core::ptr::write_unaligned(p.add(ADDRINFO_PROTOCOL).cast::<i32>(), e.protocol);
            core::ptr::write_unaligned(p.add(ADDRINFO_ADDRLEN).cast::<usize>(), salen);
            let sap = p.add(ADDRINFO_SIZE);
            core::ptr::copy_nonoverlapping(sa.as_ptr(), sap, salen);
            core::ptr::write_unaligned(p.add(ADDRINFO_ADDR).cast::<*mut u8>(), sap);
            if !name.is_empty() {
                let np = p.add(ADDRINFO_SIZE + addr::MAX_SIZE);
                core::ptr::copy_nonoverlapping(name.as_ptr(), np, name.len());
                core::ptr::write_unaligned(p.add(ADDRINFO_CANONNAME).cast::<*mut u8>(), np);
            }
            if tail.is_null() {
                head = p;
            } else {
                core::ptr::write_unaligned(tail.add(ADDRINFO_NEXT).cast::<*mut u8>(), p);
            }
        }
        tail = p;
    }
    Ok(head)
}

unsafe fn free_list(mut p: *mut u8) {
    while !p.is_null() {
        // SAFETY: a block of `build_list`.
        unsafe {
            let next = core::ptr::read_unaligned(p.add(ADDRINFO_NEXT).cast::<*mut u8>());
            heap_free(p);
            p = next;
        }
    }
}

/// `getaddrinfo`: returns the WSA error (and sets it), 0 on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getaddrinfo(
    node: *const u8,
    service: *const u8,
    hints: *const u8,
    result: *mut *mut u8,
) -> i32 {
    if result.is_null() {
        set_error(WSAEINVAL);
        return WSAEINVAL as i32;
    }
    // SAFETY: the caller's strings and hints.
    let (node, service, h) = unsafe { (cstr(node), cstr(service), read_hints(hints)) };
    let r = resolve_all(node, service, &h).and_then(|(e, c)| build_list(&e, c.as_deref(), h.flags, false));
    finish(r, result)
}

fn finish(r: Result<*mut u8, u32>, result: *mut *mut u8) -> i32 {
    match r {
        Ok(list) => {
            // SAFETY: the caller's result pointer.
            unsafe { *result = list };
            // A successful call sets the last error to 0 (observed on Windows).
            set_error(0);
            0
        }
        Err(e) => {
            // SAFETY: the caller's result pointer.
            unsafe { *result = core::ptr::null_mut() };
            set_error(e);
            e as i32
        }
    }
}

/// `GetAddrInfoW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAddrInfoW(
    node: *const u16,
    service: *const u16,
    hints: *const u8,
    result: *mut *mut u8,
) -> i32 {
    if result.is_null() {
        set_error(WSAEINVAL);
        return WSAEINVAL as i32;
    }
    // SAFETY: the caller's strings and hints.
    let (node, service, h) = unsafe { (wstr(node), wstr(service), read_hints(hints)) };
    let r = resolve_all(node.as_deref(), service.as_deref(), &h)
        .and_then(|(e, c)| build_list(&e, c.as_deref(), h.flags, true));
    finish(r, result)
}

/// `freeaddrinfo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn freeaddrinfo(list: *mut u8) {
    // SAFETY: a list of getaddrinfo's.
    unsafe { free_list(list) }
}

/// `FreeAddrInfoW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn FreeAddrInfoW(list: *mut u8) {
    // SAFETY: a list of GetAddrInfoW's.
    unsafe { free_list(list) }
}

// ----- getnameinfo

fn name_info(sa: *const u8, salen: i32, flags: i32) -> Result<(SocketAddr, String, String), u32> {
    check_started()?;
    if sa.is_null() || salen < 0 {
        return Err(WSAEFAULT);
    }
    // SAFETY: the caller's address.
    let bytes = unsafe { core::slice::from_raw_parts(sa, salen as usize) };
    let a = addr::decode(bytes).map_err(|e| if e == WSAEAFNOSUPPORT { EAI_FAMILY } else { WSAEFAULT })?;
    let host = resolve::name_of_host(&a, flags)?;
    let serv = resolve::name_of_service(a.port(), flags);
    Ok((a, host, serv))
}

/// `ERROR_INSUFFICIENT_BUFFER` (`winerror.h`): what `getnameinfo` returns for a buffer that is too short.
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// Copy `s` and a NUL to `out` of `len` units. A null buffer or a length of 0 is not written (the part
/// is not asked for); a buffer that is too short gets the first `len` characters and
/// `ERROR_INSUFFICIENT_BUFFER` (observed on Windows).
unsafe fn put_str(s: &str, out: *mut u8, len: usize, _wide: bool) -> Result<(), u32> {
    if out.is_null() || len == 0 {
        return Ok(());
    }
    let b = s.as_bytes();
    if b.len() + 1 > len {
        // SAFETY: `len` bytes of the caller's buffer.
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, len) };
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: `len` bytes of the caller's buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len());
        *out.add(b.len()) = 0;
    }
    Ok(())
}

unsafe fn put_wide(s: &str, out: *mut u16, len: usize) -> Result<(), u32> {
    if out.is_null() || len == 0 {
        return Ok(());
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() + 1 > len {
        // SAFETY: `len` units of the caller's buffer.
        unsafe { core::ptr::copy_nonoverlapping(units.as_ptr(), out, len) };
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: `len` units of the caller's buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len());
        *out.add(units.len()) = 0;
    }
    Ok(())
}

/// `getnameinfo`: returns the WSA error (and sets it), 0 on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getnameinfo(
    sa: *const u8,
    salen: i32,
    host: *mut u8,
    hostlen: u32,
    serv: *mut u8,
    servlen: u32,
    flags: i32,
) -> i32 {
    let r = name_info(sa, salen, flags).and_then(|(_, h, s)| {
        // SAFETY: the caller's buffers.
        unsafe {
            put_str(&h, host, hostlen as usize, false)?;
            put_str(&s, serv, servlen as usize, false)
        }
    });
    match r {
        Ok(()) => 0,
        Err(e) => {
            set_error(e);
            e as i32
        }
    }
}

/// `GetNameInfoW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNameInfoW(
    sa: *const u8,
    salen: i32,
    host: *mut u16,
    hostlen: u32,
    serv: *mut u16,
    servlen: u32,
    flags: i32,
) -> i32 {
    let r = name_info(sa, salen, flags).and_then(|(_, h, s)| {
        // SAFETY: the caller's buffers.
        unsafe {
            put_wide(&h, host, hostlen as usize)?;
            put_wide(&s, serv, servlen as usize)
        }
    });
    match r {
        Ok(()) => 0,
        Err(e) => {
            set_error(e);
            e as i32
        }
    }
}

// ----- gethostname, gethostbyname

/// `gethostname`: `WSAEFAULT` when the name and its NUL do not fit.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn gethostname(name: *mut u8, len: i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if name.is_null() || len < 0 {
        return fail(WSAEFAULT);
    }
    let info = afd::net_info();
    // SAFETY: the caller's buffer.
    match unsafe { put_str(info.host_name(), name, len as usize, false) } {
        Ok(()) => 0,
        // `gethostname` says `WSAEFAULT` for a buffer that is too short (`getnameinfo` the other code).
        Err(_) => fail(WSAEFAULT),
    }
}

/// `GetHostNameW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetHostNameW(name: *mut u16, len: i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if name.is_null() || len < 0 {
        return fail(WSAEFAULT);
    }
    let info = afd::net_info();
    // SAFETY: the caller's buffer.
    match unsafe { put_wide(info.host_name(), name, len as usize) } {
        Ok(()) => 0,
        Err(_) => fail(WSAEFAULT),
    }
}

/// A buffer Winsock owns (one per process).
struct Static<T>(UnsafeCell<T>);

// SAFETY: Learn makes these buffers valid until the thread's next call;
// Poly's single copy is shared, which a program calling from several
// threads at once would see as a race on the returned data only.
unsafe impl<T> Sync for Static<T> {}

/// `hostent` and what it points at: the name, an empty alias list, up to
/// 16 IPv4 addresses and their pointer list.
#[repr(C)]
struct HostBuf {
    hostent: [u8; HOSTENT_SIZE],
    name: [u8; 256],
    aliases: [usize; 1],
    addrs: [[u8; 4]; 16],
    list: [usize; 17],
}

static HOST: Static<HostBuf> = Static(UnsafeCell::new(HostBuf {
    hostent: [0; HOSTENT_SIZE],
    name: [0; 256],
    aliases: [0],
    addrs: [[0; 4]; 16],
    list: [0; 17],
}));

/// `gethostbyname` (deprecated, IPv4 only): a numeric address or a name
/// (null: this machine's).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn gethostbyname(name: *const u8) -> *mut u8 {
    if let Err(e) = check_started() {
        set_error(e);
        return core::ptr::null_mut();
    }
    let own = afd::net_info();
    // SAFETY: the caller's string.
    let name = unsafe { cstr(name) }.map(String::from).unwrap_or_else(|| String::from(own.host_name()));
    let h = Hints { family: AF_INET, ..Hints::default() };
    let addrs = match resolve::classify(Some(&name), &h, own.host_name()) {
        Ok((Host::Addrs(a), _)) => Ok(a),
        Ok((Host::SelfAddrs, _)) => Ok(resolve::self_addrs(&afd::local_addrs(), &h)),
        Ok((Host::Lookup(n), _)) => lookup_name(&n, &h).map(|(a, _)| a),
        Err(e) => Err(e),
    };
    let addrs: Vec<[u8; 4]> = match addrs {
        Ok(a) => a
            .iter()
            .filter_map(|ip| match ip {
                IpAddr::V4(v) => Some(v.octets()),
                IpAddr::V6(_) => None,
            })
            .collect(),
        Err(e) => {
            set_error(e);
            return core::ptr::null_mut();
        }
    };
    if addrs.is_empty() {
        set_error(WSAHOST_NOT_FOUND);
        return core::ptr::null_mut();
    }
    // SAFETY: Winsock's own buffer (see `Static`).
    unsafe {
        let b = &mut *HOST.0.get();
        let n = name.len().min(255);
        b.name = [0; 256];
        b.name[..n].copy_from_slice(&name.as_bytes()[..n]);
        b.aliases = [0];
        b.list = [0; 17];
        for (i, a) in addrs.iter().take(16).enumerate() {
            b.addrs[i] = *a;
            b.list[i] = b.addrs[i].as_ptr() as usize;
        }
        let he = b.hostent.as_mut_ptr();
        core::ptr::write_unaligned(he.add(HOSTENT_NAME).cast::<usize>(), b.name.as_ptr() as usize);
        core::ptr::write_unaligned(he.add(HOSTENT_ALIASES).cast::<usize>(), b.aliases.as_ptr() as usize);
        core::ptr::write_unaligned(he.add(HOSTENT_ADDRTYPE).cast::<i16>(), AF_INET as i16);
        core::ptr::write_unaligned(he.add(HOSTENT_LENGTH).cast::<i16>(), 4);
        core::ptr::write_unaligned(he.add(HOSTENT_ADDR_LIST).cast::<usize>(), b.list.as_ptr() as usize);
        he
    }
}

// ----- services and protocols

/// `servent` on x64 (`winsock2.h`: `s_name`, `s_aliases`, then `s_proto`
/// before `s_port` under `_WIN64`) and its strings.
#[repr(C)]
struct ServBuf {
    servent: [u8; 32],
    name: [u8; 32],
    proto: [u8; 8],
    aliases: [usize; 1],
}

static SERV: Static<ServBuf> =
    Static(UnsafeCell::new(ServBuf { servent: [0; 32], name: [0; 32], proto: [0; 8], aliases: [0] }));

unsafe fn servent(name: &str, port: u16, proto: &str) -> *mut u8 {
    // SAFETY: Winsock's own buffer (see `Static`).
    unsafe {
        let b = &mut *SERV.0.get();
        b.name = [0; 32];
        b.name[..name.len().min(31)].copy_from_slice(&name.as_bytes()[..name.len().min(31)]);
        b.proto = [0; 8];
        b.proto[..proto.len().min(7)].copy_from_slice(&proto.as_bytes()[..proto.len().min(7)]);
        b.aliases = [0];
        let p = b.servent.as_mut_ptr();
        core::ptr::write_unaligned(p.cast::<usize>(), b.name.as_ptr() as usize);
        core::ptr::write_unaligned(p.add(8).cast::<usize>(), b.aliases.as_ptr() as usize);
        core::ptr::write_unaligned(p.add(16).cast::<usize>(), b.proto.as_ptr() as usize);
        core::ptr::write_unaligned(p.add(24).cast::<u16>(), port.to_be());
        p
    }
}

/// `getservbyname`: Poly's table of well-known services
/// (`poly_winsock::resolve::SERVICES`), any protocol.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getservbyname(name: *const u8, proto: *const u8) -> *mut u8 {
    if let Err(e) = check_started() {
        set_error(e);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings.
    let (name, proto) = unsafe { (cstr(name), cstr(proto)) };
    match name.and_then(resolve::service_by_name) {
        // SAFETY: Winsock's own buffer.
        Some(port) => unsafe { servent(name.unwrap_or(""), port, proto.unwrap_or("tcp")) },
        None => {
            set_error(WSANO_DATA);
            core::ptr::null_mut()
        }
    }
}

/// `getservbyport` (`port` in network order).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getservbyport(port: i32, proto: *const u8) -> *mut u8 {
    if let Err(e) = check_started() {
        set_error(e);
        return core::ptr::null_mut();
    }
    let port = u16::from_be(port as u16);
    // SAFETY: the caller's string.
    let proto = unsafe { cstr(proto) };
    match resolve::service_by_port(port) {
        // SAFETY: Winsock's own buffer.
        Some(name) => unsafe { servent(name, port, proto.unwrap_or("tcp")) },
        None => {
            set_error(WSANO_DATA);
            core::ptr::null_mut()
        }
    }
}

/// `protoent` (`p_name`, `p_aliases`, `p_proto`) and its name.
#[repr(C)]
struct ProtoBuf {
    protoent: [u8; 24],
    name: [u8; 8],
    aliases: [usize; 1],
}

static PROTO: Static<ProtoBuf> = Static(UnsafeCell::new(ProtoBuf { protoent: [0; 24], name: [0; 8], aliases: [0] }));

const PROTOCOLS: &[(&str, i32)] = &[("ip", 0), ("icmp", 1), ("tcp", 6), ("udp", 17), ("ipv6", 41), ("icmpv6", 58)];

unsafe fn protoent(name: &str, number: i32) -> *mut u8 {
    // SAFETY: Winsock's own buffer.
    unsafe {
        let b = &mut *PROTO.0.get();
        b.name = [0; 8];
        b.name[..name.len()].copy_from_slice(name.as_bytes());
        b.aliases = [0];
        let p = b.protoent.as_mut_ptr();
        core::ptr::write_unaligned(p.cast::<usize>(), b.name.as_ptr() as usize);
        core::ptr::write_unaligned(p.add(8).cast::<usize>(), b.aliases.as_ptr() as usize);
        core::ptr::write_unaligned(p.add(16).cast::<i16>(), number as i16);
        p
    }
}

/// `getprotobyname`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getprotobyname(name: *const u8) -> *mut u8 {
    // SAFETY: the caller's string.
    let name = unsafe { cstr(name) };
    match PROTOCOLS.iter().find(|(n, _)| name.is_some_and(|x| x.eq_ignore_ascii_case(n))) {
        // SAFETY: Winsock's own buffer.
        Some(&(n, p)) => unsafe { protoent(n, p) },
        None => {
            set_error(WSANO_DATA);
            core::ptr::null_mut()
        }
    }
}

/// `getprotobynumber`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getprotobynumber(number: i32) -> *mut u8 {
    match PROTOCOLS.iter().find(|&&(_, p)| p == number) {
        // SAFETY: Winsock's own buffer.
        Some(&(n, p)) => unsafe { protoent(n, p) },
        None => {
            set_error(WSANO_DATA);
            core::ptr::null_mut()
        }
    }
}

// ----- address text

/// `inet_addr`: `INADDR_NONE` for a bad string (and for a null one).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn inet_addr(s: *const u8) -> u32 {
    // SAFETY: the caller's string.
    match unsafe { cstr(s) }.and_then(text::inet_addr) {
        Some(b) => u32::from_ne_bytes(b),
        None => INADDR_NONE,
    }
}

static NTOA: Static<[u8; 16]> = Static(UnsafeCell::new([0; 16]));

/// `inet_ntoa`: dotted decimal in Winsock's buffer.
#[unsafe(no_mangle)]
pub extern "system" fn inet_ntoa(a: u32) -> *mut u8 {
    let s = text::ntoa(a.to_ne_bytes());
    // SAFETY: Winsock's own buffer (see `Static`); the text is at most 15
    // bytes.
    unsafe {
        let b = &mut *NTOA.0.get();
        *b = [0; 16];
        b[..s.len()].copy_from_slice(s.as_bytes());
        b.as_mut_ptr()
    }
}

/// `inet_pton`: 1 for an address, 0 for a string that is not one, -1 with
/// `WSAEAFNOSUPPORT` for another family (Learn: "inet_pton").
#[unsafe(no_mangle)]
pub unsafe extern "system" fn inet_pton(af: i32, s: *const u8, out: *mut u8) -> i32 {
    if s.is_null() || out.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's string.
    let s = unsafe { cstr(s) }.unwrap_or("");
    // SAFETY: the caller's buffer of the family's size.
    unsafe { pton(af, s, out) }
}

unsafe fn pton(af: i32, s: &str, out: *mut u8) -> i32 {
    let bytes: Option<Vec<u8>> = match af {
        AF_INET => text::pton4(s).map(|b| b.to_vec()),
        AF_INET6 => text::pton6(s).map(|b| b.to_vec()),
        _ => return fail(WSAEAFNOSUPPORT),
    };
    match bytes {
        Some(b) => {
            // SAFETY: the caller's buffer of the family's size.
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len()) };
            1
        }
        None => 0,
    }
}

/// `InetPtonW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InetPtonW(af: i32, s: *const u16, out: *mut u8) -> i32 {
    if s.is_null() || out.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's string.
    let s = unsafe { wstr(s) }.unwrap_or_default();
    // SAFETY: the caller's buffer.
    unsafe { pton(af, &s, out) }
}

/// `inet_ntop`: the text, or null with `WSAEAFNOSUPPORT` / `WSAEINVAL`
/// (Learn: a buffer too small is `ERROR_INVALID_PARAMETER`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn inet_ntop(af: i32, a: *const u8, out: *mut u8, len: usize) -> *const u8 {
    if a.is_null() || out.is_null() {
        set_error(WSA_INVALID_PARAMETER);
        return core::ptr::null();
    }
    // SAFETY: the caller's address of the family's size.
    match unsafe { ntop(af, a) }
        .and_then(|t| unsafe { put_str(&t, out, len, false) }.map_err(|_| WSA_INVALID_PARAMETER))
    {
        Ok(()) => out,
        Err(e) => {
            set_error(e);
            core::ptr::null()
        }
    }
}

unsafe fn ntop(af: i32, a: *const u8) -> Result<String, u32> {
    let n = match af {
        AF_INET => 4,
        AF_INET6 => 16,
        _ => return Err(WSAEAFNOSUPPORT),
    };
    // SAFETY: the caller's address of the family's size.
    text::ntop(af, unsafe { core::slice::from_raw_parts(a, n) })
}

/// `InetNtopW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InetNtopW(af: i32, a: *const u8, out: *mut u16, len: usize) -> *const u16 {
    if a.is_null() || out.is_null() {
        set_error(WSA_INVALID_PARAMETER);
        return core::ptr::null();
    }
    // SAFETY: the caller's address and buffer.
    match unsafe { ntop(af, a) }.and_then(|t| unsafe { put_wide(&t, out, len) }.map_err(|_| WSA_INVALID_PARAMETER)) {
        Ok(()) => out,
        Err(e) => {
            set_error(e);
            core::ptr::null()
        }
    }
}

/// The text of a socket address: `a.b.c.d[:port]`, `[v6][:port]` (the
/// port left out when 0; an IPv6 address without a port has no brackets).
fn address_text(a: &SocketAddr) -> String {
    match (a, a.port()) {
        (SocketAddr::V4(v), 0) => alloc::format!("{}", v.ip()),
        (SocketAddr::V6(v), 0) if v.scope_id() == 0 => alloc::format!("{}", v.ip()),
        (SocketAddr::V6(v), 0) => alloc::format!("{}%{}", v.ip(), v.scope_id()),
        _ => alloc::format!("{a}"),
    }
}

unsafe fn address_to_string(sa: *const u8, salen: u32) -> Result<String, u32> {
    check_started()?;
    if sa.is_null() {
        return Err(WSAEFAULT);
    }
    // SAFETY: the caller's address.
    let a = addr::decode(unsafe { core::slice::from_raw_parts(sa, salen as usize) })
        .map_err(|e| if e == WSAEAFNOSUPPORT { WSAEINVAL } else { WSAEFAULT })?;
    Ok(address_text(&a))
}

/// `WSAAddressToStringA`: `WSAEFAULT` with the length needed when the
/// buffer is short.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAAddressToStringA(
    sa: *const u8,
    salen: u32,
    _info: *const u8,
    out: *mut u8,
    len: *mut u32,
) -> i32 {
    if len.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's address, buffer and length.
    unsafe {
        let t = match address_to_string(sa, salen) {
            Ok(t) => t,
            Err(e) => return fail(e),
        };
        let need = t.len() as u32 + 1;
        if out.is_null() || *len < need {
            *len = need;
            return fail(WSAEFAULT);
        }
        let _ = put_str(&t, out, *len as usize, false);
        *len = need;
    }
    0
}

/// `WSAAddressToStringW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAAddressToStringW(
    sa: *const u8,
    salen: u32,
    _info: *const u8,
    out: *mut u16,
    len: *mut u32,
) -> i32 {
    if len.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's address, buffer and length.
    unsafe {
        let t = match address_to_string(sa, salen) {
            Ok(t) => t,
            Err(e) => return fail(e),
        };
        let need = t.encode_utf16().count() as u32 + 1;
        if out.is_null() || *len < need {
            *len = need;
            return fail(WSAEFAULT);
        }
        let _ = put_wide(&t, out, *len as usize);
        *len = need;
    }
    0
}

/// Parse `a.b.c.d[:port]`, `v6`, `[v6]:port` for `af`.
fn parse_address(s: &str, af: i32) -> Option<SocketAddr> {
    let a = s.parse::<SocketAddr>().ok().or_else(|| {
        let ip: IpAddr = s.parse().ok()?;
        Some(SocketAddr::new(ip, 0))
    })?;
    (addr::af_of(&a) == af).then_some(a)
}

unsafe fn string_to_address(s: Option<String>, af: i32, out: *mut u8, len: *mut i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    let Some(a) = s.as_deref().and_then(|s| parse_address(s, af)) else { return fail(WSAEINVAL) };
    let (b, n) = addr::encode(&a);
    if out.is_null() || len.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's buffer and length.
    unsafe {
        if (*len as usize) < n {
            *len = n as i32;
            return fail(WSAEFAULT);
        }
        core::ptr::copy_nonoverlapping(b.as_ptr(), out, n);
        *len = n as i32;
    }
    0
}

/// `WSAStringToAddressA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAStringToAddressA(
    s: *const u8,
    af: i32,
    _info: *const u8,
    out: *mut u8,
    len: *mut i32,
) -> i32 {
    // SAFETY: the caller's string, buffer and length.
    unsafe { string_to_address(cstr(s).map(String::from), af, out, len) }
}

/// `WSAStringToAddressW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAStringToAddressW(
    s: *const u16,
    af: i32,
    _info: *const u8,
    out: *mut u8,
    len: *mut i32,
) -> i32 {
    // SAFETY: the caller's string, buffer and length.
    unsafe { string_to_address(wstr(s), af, out, len) }
}

// ----- byte order

/// `htons`.
#[unsafe(no_mangle)]
pub extern "system" fn htons(v: u16) -> u16 {
    poly_winsock::swap16(v)
}

/// `ntohs`.
#[unsafe(no_mangle)]
pub extern "system" fn ntohs(v: u16) -> u16 {
    poly_winsock::swap16(v)
}

/// `htonl`.
#[unsafe(no_mangle)]
pub extern "system" fn htonl(v: u32) -> u32 {
    poly_winsock::swap32(v)
}

/// `ntohl`.
#[unsafe(no_mangle)]
pub extern "system" fn ntohl(v: u32) -> u32 {
    poly_winsock::swap32(v)
}

fn checked_socket(s: Socket) -> Result<(), u32> {
    check_started()?;
    afd::info(s as crate::Handle).map(|_| ()).ok_or(WSAENOTSOCK)
}

/// `WSAHtons`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAHtons(s: Socket, v: u16, out: *mut u16) -> i32 {
    if out.is_null() {
        return fail(WSAEFAULT);
    }
    match checked_socket(s) {
        // SAFETY: the caller's output.
        Ok(()) => unsafe {
            *out = v.to_be();
            0
        },
        Err(e) => fail(e),
    }
}

/// `WSANtohs`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSANtohs(s: Socket, v: u16, out: *mut u16) -> i32 {
    // SAFETY: as WSAHtons.
    unsafe { WSAHtons(s, v, out) }
}

/// `WSAHtonl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAHtonl(s: Socket, v: u32, out: *mut u32) -> i32 {
    if out.is_null() {
        return fail(WSAEFAULT);
    }
    match checked_socket(s) {
        // SAFETY: the caller's output.
        Ok(()) => unsafe {
            *out = v.to_be();
            0
        },
        Err(e) => fail(e),
    }
}

/// `WSANtohl`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSANtohl(s: Socket, v: u32, out: *mut u32) -> i32 {
    // SAFETY: as WSAHtonl.
    unsafe { WSAHtonl(s, v, out) }
}
