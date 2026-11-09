//! `GetAdaptersAddresses`: the `IP_ADAPTER_ADDRESSES_LH` chain (x64,
//! `iptypes.h`). Learn: the function fills a caller buffer with a linked
//! list of adapters; `*SizePointer` is the size needed when the buffer is
//! too small (`ERROR_BUFFER_OVERFLOW`); `ERROR_NO_DATA` when there are no
//! adapters for the family. [`build`] makes the bytes for a buffer at
//! address `base` (the pointers inside are absolute).
//!
//! Layout of the fixed parts (x64):
//!
//! | Structure | Size | Fields (offset) |
//! |---|---:|---|
//! | `IP_ADAPTER_ADDRESSES_LH` | 448 | Length 0, IfIndex 4, Next 8, AdapterName 16, FirstUnicast 24, FirstAnycast 32, FirstMulticast 40, FirstDnsServer 48, DnsSuffix 56, Description 64, FriendlyName 72, PhysicalAddress[8] 80, PhysicalAddressLength 88, Flags 92, Mtu 96, IfType 100, OperStatus 104, Ipv6IfIndex 108, ZoneIndices[16] 112, FirstPrefix 176, TransmitLinkSpeed 184, ReceiveLinkSpeed 192, FirstWinsServer 200, FirstGateway 208, Ipv4Metric 216, Ipv6Metric 220, Luid 224, Dhcpv4Server 232, CompartmentId 248, NetworkGuid 252, ConnectionType 268, TunnelType 272, Dhcpv6Server 280, Dhcpv6ClientDuid[130] 296, DuidLength 428, Dhcpv6Iaid 432, FirstDnsSuffix 440 |
//! | `IP_ADAPTER_UNICAST_ADDRESS_LH` | 64 | Length 0, Flags 4, Next 8, Address {sockaddr 16, length 24}, PrefixOrigin 32, SuffixOrigin 36, DadState 40, ValidLifetime 44, PreferredLifetime 48, LeaseLifetime 52, OnLinkPrefixLength 56 |
//! | anycast, multicast, DNS server, gateway | 32 | Length 0, Flags/Reserved 4, Next 8, Address {sockaddr 16, length 24} |
//! | `IP_ADAPTER_PREFIX_XP` | 40 | Length 0, Flags 4, Next 8, Address {16, 24}, PrefixLength 32 |

use crate::{
    AF_INET, AF_INET6, AF_UNSPEC, ERROR_BUFFER_OVERFLOW, ERROR_INVALID_PARAMETER, ERROR_NO_DATA, IfInfo, Kind, Out,
};
use alloc::vec::Vec;

/// `GAA_FLAG_*` (`iptypes.h`).
pub const GAA_FLAG_SKIP_UNICAST: u32 = 0x1;
pub const GAA_FLAG_SKIP_ANYCAST: u32 = 0x2;
pub const GAA_FLAG_SKIP_MULTICAST: u32 = 0x4;
pub const GAA_FLAG_SKIP_DNS_SERVER: u32 = 0x8;
pub const GAA_FLAG_INCLUDE_PREFIX: u32 = 0x10;
pub const GAA_FLAG_SKIP_FRIENDLY_NAME: u32 = 0x20;
pub const GAA_FLAG_INCLUDE_WINS_INFO: u32 = 0x40;
pub const GAA_FLAG_INCLUDE_GATEWAYS: u32 = 0x80;
pub const GAA_FLAG_INCLUDE_ALL_INTERFACES: u32 = 0x100;
pub const GAA_FLAG_INCLUDE_ALL_COMPARTMENTS: u32 = 0x200;
pub const GAA_FLAG_INCLUDE_TUNNEL_BINDINGORDER: u32 = 0x400;
pub const GAA_FLAG_SKIP_DNS_INFO: u32 = 0x800;
/// Every `GAA_FLAG_*` bit; others are `ERROR_INVALID_PARAMETER`.
const GAA_ALL: u32 = 0xFFF;

pub const ADAPTER_SIZE: usize = 448;
pub const UNICAST_SIZE: usize = 64;
pub const SOCKET_ADDRESS_LIST_ENTRY_SIZE: usize = 32;
pub const PREFIX_SIZE: usize = 40;

/// `IP_ADAPTER_*` flag bits of the `Flags` union (`iptypes.h`).
const DDNS_ENABLED: u32 = 0x1;
const REGISTER_ADAPTER_SUFFIX: u32 = 0x2;
const DHCPV4_ENABLED: u32 = 0x4;
const IPV4_ENABLED: u32 = 0x80;
const IPV6_ENABLED: u32 = 0x100;

/// `IF_OPER_STATUS`: `IfOperStatusUp`, `IfOperStatusDown` (`ifdef.h`).
const OPER_UP: u32 = 1;
const OPER_DOWN: u32 = 2;

/// `IP_PREFIX_ORIGIN` / `IP_SUFFIX_ORIGIN` / `NL_DAD_STATE` (`nldef.h`).
const PREFIX_MANUAL: u32 = 1;
const PREFIX_WELL_KNOWN: u32 = 2;
const SUFFIX_MANUAL: u32 = 1;
const SUFFIX_LINK_LAYER: u32 = 4;
const DAD_PREFERRED: u32 = 4;
/// `IP_ADAPTER_ADDRESS_DNS_ELIGIBLE` (`iptypes.h`).
const ADDRESS_DNS_ELIGIBLE: u32 = 0x1;
/// `INFINITE` lifetimes.
const INFINITE: u32 = 0xFFFF_FFFF;

/// Whether `GetAdaptersAddresses(family, flags)` lists the adapter.
fn included(f: &IfInfo, family: u32) -> bool {
    match family {
        AF_INET => f.ipv4.is_some(),
        AF_INET6 => f.ipv6.is_some(),
        _ => true,
    }
}

/// The metric Windows gives an interface of this kind.
fn metric(f: &IfInfo) -> u32 {
    match f.kind {
        Kind::Loopback => 75,
        Kind::Ethernet => 25,
    }
}

/// A `SOCKET_ADDRESS` list entry's body: `[Length][Flags/Reserved][Next][sockaddr*][length]`.
fn list_entry(o: &mut Out, prev: &mut Option<usize>, first: &mut usize, sockaddr: usize, len: i32) -> usize {
    let at = o.alloc(SOCKET_ADDRESS_LIST_ENTRY_SIZE);
    o.put_u32(at, SOCKET_ADDRESS_LIST_ENTRY_SIZE as u32);
    let p = o.ptr(sockaddr);
    o.put_u64(at + 16, p);
    o.put_u32(at + 24, len as u32);
    link(o, prev, first, at);
    at
}

/// Chain entry `at` after `prev` (or make it the first).
fn link(o: &mut Out, prev: &mut Option<usize>, first: &mut usize, at: usize) {
    match prev {
        Some(p) => {
            let ptr = o.ptr(at);
            o.put_u64(*p + 8, ptr);
        }
        None => *first = o.ptr(at) as usize,
    }
    *prev = Some(at);
}

/// The bytes `GetAdaptersAddresses` writes at `base`: `Err(ERROR_NO_DATA)` when no adapter
/// qualifies, `Err(ERROR_INVALID_PARAMETER)` for a bad family or flags.
pub fn build(ifs: &[IfInfo], family: u32, flags: u32, base: u64) -> Result<Vec<u8>, u32> {
    if !matches!(family, AF_UNSPEC | AF_INET | AF_INET6) || flags & !GAA_ALL != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let chosen: Vec<&IfInfo> = ifs.iter().filter(|f| included(f, family)).collect();
    if chosen.is_empty() {
        return Err(ERROR_NO_DATA);
    }
    let mut o = Out::new(base);
    let mut prev_adapter: Option<usize> = None;
    for f in chosen {
        // The adapter first, so the chain is in order; its lists follow.
        let a = o.alloc(ADAPTER_SIZE);
        o.put_u32(a, ADAPTER_SIZE as u32);
        o.put_u32(a + 4, f.index);
        if let Some(p) = prev_adapter {
            let ptr = o.ptr(a);
            o.put_u64(p + 8, ptr);
        }
        prev_adapter = Some(a);

        let name = o.ansi(&f.adapter_name());
        let p = o.ptr(name);
        o.put_u64(a + 16, p);

        // Unicast addresses.
        if flags & GAA_FLAG_SKIP_UNICAST == 0 {
            let (mut prev, mut first) = (None, 0usize);
            let mut uni = |o: &mut Out, sock: usize, len: i32, prefix_origin, suffix_origin, plen: u8, dns: bool| {
                let at = o.alloc(UNICAST_SIZE);
                o.put_u32(at, UNICAST_SIZE as u32);
                o.put_u32(at + 4, if dns { ADDRESS_DNS_ELIGIBLE } else { 0 });
                let p = o.ptr(sock);
                o.put_u64(at + 16, p);
                o.put_u32(at + 24, len as u32);
                o.put_u32(at + 32, prefix_origin);
                o.put_u32(at + 36, suffix_origin);
                o.put_u32(at + 40, DAD_PREFERRED);
                o.put_u32(at + 44, INFINITE);
                o.put_u32(at + 48, INFINITE);
                o.put_u32(at + 52, INFINITE);
                o.put_u8(at + 56, plen);
                match prev {
                    Some(pv) => {
                        let ptr = o.ptr(at);
                        o.put_u64(pv + 8, ptr);
                    }
                    None => first = o.ptr(at) as usize,
                }
                prev = Some(at);
            };
            let dns_ok = f.kind == Kind::Ethernet;
            if family != AF_INET6
                && let Some((addr, plen)) = f.ipv4
            {
                let s = o.sockaddr_in(addr);
                uni(&mut o, s, 16, PREFIX_MANUAL, SUFFIX_MANUAL, plen, dns_ok);
            }
            if family != AF_INET
                && let Some((addr, plen)) = f.ipv6
            {
                let scope = if f.kind == Kind::Ethernet { f.index } else { 0 };
                let s = o.sockaddr_in6(addr, scope);
                let (po, so) = if f.kind == Kind::Ethernet {
                    (PREFIX_WELL_KNOWN, SUFFIX_LINK_LAYER)
                } else {
                    (PREFIX_MANUAL, SUFFIX_MANUAL)
                };
                uni(&mut o, s, 28, po, so, plen, false);
            }
            o.put_u64(a + 24, first as u64);
        }

        // Multicast: the all-hosts groups.
        if flags & GAA_FLAG_SKIP_MULTICAST == 0 && f.kind == Kind::Ethernet {
            let (mut prev, mut first) = (None, 0usize);
            if family != AF_INET6 && f.ipv4.is_some() {
                let s = o.sockaddr_in([224, 0, 0, 1]);
                list_entry(&mut o, &mut prev, &mut first, s, 16);
            }
            if family != AF_INET && f.ipv6.is_some() {
                let mut g = [0u8; 16];
                g[0] = 0xff;
                g[1] = 0x02;
                g[15] = 0x01;
                let s = o.sockaddr_in6(g, f.index);
                list_entry(&mut o, &mut prev, &mut first, s, 28);
            }
            o.put_u64(a + 40, first as u64);
        }

        // DNS servers.
        if flags & (GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_SKIP_DNS_INFO) == 0 && family != AF_INET6 {
            let (mut prev, mut first) = (None, 0usize);
            for d in &f.dns {
                let s = o.sockaddr_in(*d);
                list_entry(&mut o, &mut prev, &mut first, s, 16);
            }
            o.put_u64(a + 48, first as u64);
        }

        // DNS suffix (empty), description, friendly name.
        if flags & GAA_FLAG_SKIP_DNS_INFO == 0 {
            let s = o.wide("");
            let p = o.ptr(s);
            o.put_u64(a + 56, p);
        }
        let d = o.wide(&f.description());
        let p = o.ptr(d);
        o.put_u64(a + 64, p);
        if flags & GAA_FLAG_SKIP_FRIENDLY_NAME == 0 {
            let n = o.wide(&f.friendly_name());
            let p = o.ptr(n);
            o.put_u64(a + 72, p);
        }
        if f.has_physical_address() {
            o.put_bytes(a + 80, &f.mac);
            o.put_u32(a + 88, 6);
        }
        let mut aflags = 0;
        if f.kind == Kind::Ethernet {
            aflags |= DDNS_ENABLED | REGISTER_ADAPTER_SUFFIX;
        }
        if f.ipv4.is_some() {
            aflags |= IPV4_ENABLED;
        }
        if f.ipv6.is_some() {
            aflags |= IPV6_ENABLED;
        }
        let _ = DHCPV4_ENABLED; // Poly's addresses are static (boot setting).
        o.put_u32(a + 92, aflags);
        o.put_u32(a + 96, f.mtu);
        o.put_u32(a + 100, f.if_type());
        o.put_u32(a + 104, if f.up { OPER_UP } else { OPER_DOWN });
        o.put_u32(a + 108, if f.ipv6.is_some() { f.index } else { 0 });

        // Prefixes: the subnet and the host.
        if flags & GAA_FLAG_INCLUDE_PREFIX != 0 {
            let (mut prev, mut first) = (None, 0usize);
            if family != AF_INET6
                && let Some((addr, plen)) = f.ipv4
            {
                let mask = if plen == 0 { 0 } else { u32::MAX << (32 - u32::from(plen.min(32))) };
                let net = (u32::from_be_bytes(addr) & mask).to_be_bytes();
                for (ad, len) in [(net, plen), (addr, 32)] {
                    let s = o.sockaddr_in(ad);
                    let at = o.alloc(PREFIX_SIZE);
                    o.put_u32(at, PREFIX_SIZE as u32);
                    let p = o.ptr(s);
                    o.put_u64(at + 16, p);
                    o.put_u32(at + 24, 16);
                    o.put_u32(at + 32, u32::from(len));
                    link(&mut o, &mut prev, &mut first, at);
                }
            }
            o.put_u64(a + 176, first as u64);
        }

        o.put_u64(a + 184, f.speed);
        o.put_u64(a + 192, f.speed);

        // Gateways.
        if flags & GAA_FLAG_INCLUDE_GATEWAYS != 0 {
            let (mut prev, mut first) = (None, 0usize);
            if family != AF_INET6
                && let Some(g) = f.gateway
            {
                let s = o.sockaddr_in(g);
                list_entry(&mut o, &mut prev, &mut first, s, 16);
            }
            o.put_u64(a + 208, first as u64);
        }

        o.put_u32(a + 216, metric(f));
        o.put_u32(a + 220, metric(f));
        o.put_u64(a + 224, f.luid());
        // CompartmentId: NET_IF_COMPARTMENT_ID_PRIMARY (1). NetworkGuid: zero.
        o.put_u32(a + 248, 1);
        // ConnectionType: NET_IF_CONNECTION_DEDICATED (1); TunnelType: TUNNEL_TYPE_NONE (0).
        o.put_u32(a + 268, 1);
    }
    Ok(o.buf)
}

/// `GetAdaptersAddresses`' size rule: the bytes needed (independent of the buffer's address).
pub fn size_needed(ifs: &[IfInfo], family: u32, flags: u32) -> Result<usize, u32> {
    build(ifs, family, flags, 0).map(|b| b.len())
}

/// The whole call: `capacity` is `*SizePointer` on entry. `Ok(bytes)` to copy to the buffer,
/// or `Err((error, size))` with the `*SizePointer` to report (`ERROR_BUFFER_OVERFLOW` carries the size
/// needed).
pub fn call(ifs: &[IfInfo], family: u32, flags: u32, base: u64, capacity: u32) -> Result<Vec<u8>, (u32, u32)> {
    let need = size_needed(ifs, family, flags).map_err(|e| (e, 0))?;
    if (capacity as usize) < need {
        return Err((ERROR_BUFFER_OVERFLOW, need as u32));
    }
    build(ifs, family, flags, base).map_err(|e| (e, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    /// Walk `Next` pointers from `at` (absolute, with base 0x1000).
    fn chain(b: &[u8], first: u64, base: u64, step: usize) -> Vec<usize> {
        let mut v = Vec::new();
        let mut p = first;
        while p != 0 {
            let o = (p - base) as usize;
            v.push(o);
            p = u64_at(b, o + step);
        }
        v
    }

    #[test]
    fn the_chain_has_both_adapters_in_order() {
        let ifs = [IfInfo::loopback(), eth()];
        let base = 0x1000;
        let b = build(&ifs, AF_UNSPEC, GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_INCLUDE_PREFIX, base).unwrap();
        assert_eq!(u32_at(&b, 0), 448);
        assert_eq!(u32_at(&b, 4), 1);
        let first = base; // the first adapter is at offset 0
        let adapters = chain(&b, first, base, 8);
        assert_eq!(adapters.len(), 2);
        let e = adapters[1];
        assert_eq!(u32_at(&b, e + 4), 2);
        assert_eq!(ansi_at(&b, (u64_at(&b, e + 16) - base) as usize), "{504F4C59-0002-4000-8000-525400123456}");
        assert_eq!(wide_at(&b, (u64_at(&b, e + 72) - base) as usize), "Ethernet");
        assert_eq!(wide_at(&b, (u64_at(&b, e + 64) - base) as usize), "Poly VirtIO Ethernet Adapter");
        assert_eq!(&b[e + 80..e + 86], &[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
        assert_eq!(u32_at(&b, e + 88), 6);
        assert_eq!(u32_at(&b, e + 96), 1500);
        assert_eq!(u32_at(&b, e + 100), 6);
        assert_eq!(u32_at(&b, e + 104), 1);
        assert_eq!(u32_at(&b, e + 108), 2);
        assert_eq!(u64_at(&b, e + 184), 10_000_000_000);
        assert_eq!(u32_at(&b, e + 216), 25);
        assert_eq!(u64_at(&b, e + 224), eth().luid());
        // Unicast: IPv4 10.0.2.15/24, then the IPv6 link-local.
        let uni = chain(&b, u64_at(&b, e + 24), base, 8);
        assert_eq!(uni.len(), 2);
        let sa = (u64_at(&b, uni[0] + 16) - base) as usize;
        assert_eq!((u16_at(&b, sa), &b[sa + 4..sa + 8]), (2, &[10u8, 0, 2, 15][..]));
        assert_eq!(b[uni[0] + 56], 24);
        assert_eq!(u32_at(&b, uni[0] + 32), 1);
        let sa6 = (u64_at(&b, uni[1] + 16) - base) as usize;
        assert_eq!((u16_at(&b, sa6), b[sa6 + 8]), (23, 0xfe));
        assert_eq!(u32_at(&b, uni[1] + 24), 28);
        // DNS server and gateway.
        let dns = chain(&b, u64_at(&b, e + 48), base, 8);
        assert_eq!(dns.len(), 1);
        let ds = (u64_at(&b, dns[0] + 16) - base) as usize;
        assert_eq!(&b[ds + 4..ds + 8], &[10, 0, 2, 3]);
        let gw = chain(&b, u64_at(&b, e + 208), base, 8);
        assert_eq!(gw.len(), 1);
        let gs = (u64_at(&b, gw[0] + 16) - base) as usize;
        assert_eq!(&b[gs + 4..gs + 8], &[10, 0, 2, 2]);
        // Prefixes: 10.0.2.0/24 and the host.
        let pre = chain(&b, u64_at(&b, e + 176), base, 8);
        assert_eq!(pre.len(), 2);
        let ps = (u64_at(&b, pre[0] + 16) - base) as usize;
        assert_eq!(&b[ps + 4..ps + 8], &[10, 0, 2, 0]);
        assert_eq!(u32_at(&b, pre[0] + 32), 24);
        assert_eq!(u32_at(&b, pre[1] + 32), 32);
        // Flags: IPv4 and IPv6 enabled.
        assert_eq!(u32_at(&b, e + 92) & (IPV4_ENABLED | IPV6_ENABLED), IPV4_ENABLED | IPV6_ENABLED);
    }

    #[test]
    fn skip_flags_leave_lists_null() {
        let ifs = [eth()];
        let flags = GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_DNS_SERVER
            | GAA_FLAG_SKIP_FRIENDLY_NAME
            | GAA_FLAG_SKIP_UNICAST;
        let b = build(&ifs, AF_UNSPEC, flags, 0x2000).unwrap();
        for off in [24, 32, 40, 48, 72, 176, 208] {
            assert_eq!(u64_at(&b, off), 0, "{off}");
        }
        assert_ne!(u64_at(&b, 64), 0, "description is always there");
    }

    #[test]
    fn family_filters_adapters_and_addresses() {
        let mut nov6 = eth();
        nov6.ipv6 = None;
        let mut nov4 = IfInfo::loopback();
        nov4.ipv4 = None;
        let b = build(&[nov4.clone(), nov6.clone()], AF_INET, 0, 0x1000).unwrap();
        assert_eq!(u32_at(&b, 4), 2, "the IPv4-less adapter is not listed");
        assert_eq!(u64_at(&b, 8), 0, "and it is the only one");
        let b = build(&[nov4, nov6], AF_INET6, 0, 0x1000).unwrap();
        assert_eq!(u32_at(&b, 4), 1);
        let uni = chain(&b, u64_at(&b, 24), 0x1000, 8);
        assert_eq!(uni.len(), 1);
        let sa = (u64_at(&b, uni[0] + 16) - 0x1000) as usize;
        assert_eq!(u16_at(&b, sa), 23);
        assert_eq!(build(&[], AF_UNSPEC, 0, 0), Err(ERROR_NO_DATA));
        assert_eq!(build(&[eth()], 99, 0, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(build(&[eth()], AF_UNSPEC, 0x8000, 0), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn a_short_buffer_reports_the_size_needed() {
        let ifs = [IfInfo::loopback(), eth()];
        let need = size_needed(&ifs, AF_UNSPEC, 0).unwrap();
        assert!(need > 2 * ADAPTER_SIZE);
        assert_eq!(call(&ifs, AF_UNSPEC, 0, 0x1000, (need - 1) as u32), Err((ERROR_BUFFER_OVERFLOW, need as u32)));
        assert_eq!(call(&ifs, AF_UNSPEC, 0, 0x1000, 0), Err((ERROR_BUFFER_OVERFLOW, need as u32)));
        assert_eq!(call(&ifs, AF_UNSPEC, 0, 0x1000, need as u32).unwrap().len(), need);
        // The size does not depend on where the buffer is.
        assert_eq!(build(&ifs, AF_UNSPEC, 0, 0xdead_0000).unwrap().len(), need);
    }

    #[test]
    fn loopback_has_no_mac_and_its_addresses_are_manual() {
        let b = build(&[IfInfo::loopback()], AF_UNSPEC, 0, 0x1000).unwrap();
        assert_eq!(u32_at(&b, 88), 0);
        assert_eq!(u32_at(&b, 100), 24);
        assert_eq!(u32_at(&b, 216), 75);
        assert_eq!(wide_at(&b, (u64_at(&b, 72) - 0x1000) as usize), "Loopback Pseudo-Interface 1");
    }
}
