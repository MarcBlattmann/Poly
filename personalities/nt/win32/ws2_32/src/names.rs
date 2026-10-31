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

use crate::sock::{self, Socket};
use crate::{afd, check_started, cstr, fail, heap_alloc, heap_free, set_error, wstr};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::net::{IpAddr, SocketAddr};
use poly_winsock::consts::*;
use poly_winsock::dns::{self, DnsError};
use poly_winsock::resolve::{self, Entry, Hints, Host};
use poly_winsock::{addr, text};

// ----- the stub resolver

/// How long one server gets to answer.
const DNS_TIMEOUT_MS: u32 = 2000;

fn dns_error(e: DnsError) -> u32 {
    match e {
        DnsError::NotFound | DnsError::BadName => WSAHOST_NOT_FOUND,
        DnsError::NoData => WSANO_DATA,
        DnsError::TryAgain => WSATRY_AGAIN,
        DnsError::Fail => WSANO_RECOVERY,
    }
}

/// Ask the servers for `name`'s records of `qtype`.
fn query(name: &str, qtype: u16) -> Result<dns::Answer, DnsError> {
    let info = afd::net_info();
    if info.servers().is_empty() {
        return Err(DnsError::TryAgain);
    }
    let mut last = DnsError::TryAgain;
    for server in info.servers() {
        let s = sock::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
        if s == INVALID_SOCKET {
            return Err(DnsError::Fail);
        }
        let r = ask(s, *server, name, qtype);
        sock::closesocket(s);
        match r {
            Ok(a) => return Ok(a),
            Err(e @ (DnsError::NotFound | DnsError::NoData)) => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn ask(s: Socket, server: [u8; 4], name: &str, qtype: u16) -> Result<dns::Answer, DnsError> {
    let timeout = DNS_TIMEOUT_MS.to_le_bytes();
    // SAFETY: a local value of the option's size.
    unsafe { sock::setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, timeout.as_ptr(), 4) };
    let id = (s as u16) ^ 0x5a17 ^ (qtype << 4);
    let q = dns::query(id, name, qtype)?;
    let (to, tolen) = addr::encode(&SocketAddr::new(IpAddr::from(server), dns::PORT));
    // SAFETY: local buffers.
    if unsafe { sock::sendto(s, q.as_ptr(), q.len() as i32, 0, to.as_ptr(), tolen as i32) } < 0 {
        return Err(DnsError::TryAgain);
    }
    let mut buf = [0u8; dns::MAX_UDP];
    for _ in 0..4 {
        let n = sock::recv(s, buf.as_mut_ptr(), buf.len() as i32, 0);
        if n < 0 {
            return Err(DnsError::TryAgain);
        }
        match dns::parse(&buf[..n as usize], id, name, qtype) {
            // Not our answer (another id): keep reading.
            Err(DnsError::Fail) if buf.len() >= 2 && u16::from_be_bytes([buf[0], buf[1]]) != id => {}
            r => return r,
        }
    }
    Err(DnsError::TryAgain)
}

/// Every address of `name` for the hints' family (IPv6 first), and the
/// canonical name.
fn lookup(name: &str, h: &Hints) -> Result<(Vec<IpAddr>, Option<String>), u32> {
    let types: &[u16] = match h.family {
        AF_INET => &[dns::TYPE_A],
        AF_INET6 => &[dns::TYPE_AAAA],
        _ => &[dns::TYPE_AAAA, dns::TYPE_A],
    };
    let mut addrs = Vec::new();
    let mut canonical = None;
    let mut first_error = None;
    for &t in types {
        match query(name, t) {
            Ok(a) => {
                addrs.extend(a.addrs);
                canonical = canonical.or(a.canonical);
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    if addrs.is_empty() {
        return Err(first_error.map_or(WSAHOST_NOT_FOUND, dns_error));
    }
    Ok((addrs, canonical))
}

/// `getaddrinfo`'s answer: the entries and the canonical name.
fn resolve_all(node: Option<&str>, service: Option<&str>, h: &Hints) -> Result<(Vec<Entry>, Option<String>), u32> {
    check_started()?;
    if node.is_none_or(str::is_empty) && service.is_none_or(str::is_empty) {
        return Err(EAI_NONAME);
    }
    resolve::check_hints(h, node)?;
    let port = resolve::service_port(service, h.flags)?;
    let own = afd::net_info();
    let (host, scope) = resolve::classify(node, h, own.host_name())?;
    let (addrs, canonical) = match host {
        Host::Addrs(a) => (a, None),
        Host::Lookup(name) => lookup(&name, h)?,
    };
    let entries = resolve::entries(&addrs, port, scope, h);
    if entries.is_empty() {
        return Err(EAI_NONAME);
    }
    let canonical = canonical.or_else(|| node.map(String::from));
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
            core::ptr::write_unaligned(p.add(ADDRINFO_FLAGS).cast::<i32>(), flags & (AI_PASSIVE | AI_CANONNAME));
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

/// Copy `s` and a NUL to `out` of `len` units: `WSAEFAULT` if it does not
/// fit (nothing to do for a null buffer).
unsafe fn put_str(s: &str, out: *mut u8, len: usize, _wide: bool) -> Result<(), u32> {
    if out.is_null() {
        return Ok(());
    }
    let b = s.as_bytes();
    if b.len() + 1 > len {
        return Err(WSAEFAULT);
    }
    // SAFETY: `len` bytes of the caller's buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(b.as_ptr(), out, b.len());
        *out.add(b.len()) = 0;
    }
    Ok(())
}

unsafe fn put_wide(s: &str, out: *mut u16, len: usize) -> Result<(), u32> {
    if out.is_null() {
        return Ok(());
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() + 1 > len {
        return Err(WSAEFAULT);
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
        Err(e) => fail(e),
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
        Err(e) => fail(e),
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
    hostent: [0; 32],
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
        Ok((Host::Lookup(n), _)) => lookup(&n, &h).map(|(a, _)| a),
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
        set_error(WSANO_DATA);
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
        core::ptr::write_unaligned(he.cast::<usize>(), b.name.as_ptr() as usize);
        core::ptr::write_unaligned(he.add(8).cast::<usize>(), b.aliases.as_ptr() as usize);
        core::ptr::write_unaligned(he.add(16).cast::<i16>(), AF_INET as i16);
        core::ptr::write_unaligned(he.add(18).cast::<i16>(), 4);
        core::ptr::write_unaligned(he.add(24).cast::<usize>(), b.list.as_ptr() as usize);
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
