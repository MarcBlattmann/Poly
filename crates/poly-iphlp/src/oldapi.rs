//! The older calls: `GetAdaptersInfo` (`IP_ADAPTER_INFO`, 704 bytes each,
//! `iptypes.h`), `GetInterfaceInfo` (`IP_INTERFACE_INFO`) and
//! `if_indextoname`. They list the IPv4 adapters; the loopback is not one
//! of them (Learn: `GetAdaptersInfo` returns Ethernet and similar adapters).
//!
//! `IP_ADAPTER_INFO` (x64): Next 0, ComboIndex 8, AdapterName[260] 12,
//! Description[132] 272, AddressLength 404, Address[8] 408, Index 416,
//! Type 420, DhcpEnabled 424, CurrentIpAddress 432, IpAddressList 440,
//! GatewayList 488, DhcpServer 536, HaveWins 584, PrimaryWinsServer 592,
//! SecondaryWinsServer 640, LeaseObtained 688, LeaseExpires 696.
//! `IP_ADDR_STRING` (48): Next 0, IpAddress[16] 8, IpMask[16] 24, Context 40.

use crate::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, IfInfo, Kind, NO_ERROR, Out};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

pub const ADAPTER_INFO_SIZE: usize = 704;
pub const ADDR_STRING_SIZE: usize = 48;

/// Dotted-decimal text of an IPv4 address.
pub fn dotted(a: [u8; 4]) -> String {
    let mut s = String::new();
    let _ = write!(s, "{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
    s
}

/// The netmask of a prefix length.
pub fn mask(prefix: u8) -> [u8; 4] {
    let m = if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix.min(32))) };
    m.to_be_bytes()
}

fn adapters(ifs: &[IfInfo]) -> Vec<&IfInfo> {
    ifs.iter().filter(|f| f.kind == Kind::Ethernet && f.ipv4.is_some()).collect()
}

fn addr_string(o: &mut Out, at: usize, ip: [u8; 4], netmask: [u8; 4], context: u32) {
    o.put_bytes(at + 8, dotted(ip).as_bytes());
    o.put_bytes(at + 24, dotted(netmask).as_bytes());
    o.put_u32(at + 40, context);
}

/// The `IP_ADAPTER_INFO` list for a buffer at `base`.
pub fn adapter_info(ifs: &[IfInfo], base: u64) -> Result<Vec<u8>, u32> {
    let list = adapters(ifs);
    if list.is_empty() {
        return Err(ERROR_NO_DATA);
    }
    let mut o = Out::new(base);
    let mut prev: Option<usize> = None;
    for f in list {
        let a = o.alloc(ADAPTER_INFO_SIZE);
        if let Some(p) = prev {
            let ptr = o.ptr(a);
            o.put_u64(p, ptr);
        }
        prev = Some(a);
        o.put_u32(a + 8, f.index);
        let name = f.adapter_name();
        o.put_bytes(a + 12, &name.as_bytes()[..name.len().min(259)]);
        let d = f.description();
        o.put_bytes(a + 272, &d.as_bytes()[..d.len().min(131)]);
        o.put_u32(a + 404, 6);
        o.put_bytes(a + 408, &f.mac);
        o.put_u32(a + 416, f.index);
        // MIB_IF_TYPE_ETHERNET = 6.
        o.put_u32(a + 420, f.if_type());
        o.put_u32(a + 424, 0);
        let (ip, plen) = f.ipv4.unwrap_or(([0; 4], 0));
        addr_string(&mut o, a + 440, ip, mask(plen), 1);
        // CurrentIpAddress stays NULL (Learn: only for DHCP-assigned addresses).
        if let Some(g) = f.gateway {
            addr_string(&mut o, a + 488, g, [0; 4], 0);
        }
    }
    Ok(o.buf)
}

/// `GetAdaptersInfo`'s whole call: `capacity` is `*pOutBufLen`.
pub fn adapter_info_call(ifs: &[IfInfo], base: u64, capacity: u32) -> Result<Vec<u8>, (u32, u32)> {
    let need = adapter_info(ifs, 0).map_err(|e| (e, 0))?.len();
    if (capacity as usize) < need {
        return Err((ERROR_BUFFER_OVERFLOW, need as u32));
    }
    adapter_info(ifs, base).map_err(|e| (e, 0))
}

/// `IP_ADAPTER_INDEX_MAP`: `ULONG Index; WCHAR Name[MAX_ADAPTER_NAME = 128]`.
pub const INDEX_MAP_SIZE: usize = 4 + 256;

/// `GetInterfaceInfo`: `LONG NumAdapters` then the maps, the name
/// `\DEVICE\TCPIP_{GUID}`.
pub fn interface_info(ifs: &[IfInfo]) -> Vec<u8> {
    let list = adapters(ifs);
    let mut b = alloc::vec![0u8; 4 + list.len() * INDEX_MAP_SIZE];
    b[0..4].copy_from_slice(&(list.len() as i32).to_le_bytes());
    for (i, f) in list.iter().enumerate() {
        let at = 4 + i * INDEX_MAP_SIZE;
        b[at..at + 4].copy_from_slice(&f.index.to_le_bytes());
        let name = alloc::format!("\\DEVICE\\TCPIP_{}", f.adapter_name());
        for (k, c) in name.encode_utf16().take(127).enumerate() {
            b[at + 4 + k * 2..at + 6 + k * 2].copy_from_slice(&c.to_le_bytes());
        }
    }
    b
}

/// `GetInterfaceInfo`'s call: `Err((ERROR_INSUFFICIENT_BUFFER, size))` for a short buffer.
pub fn interface_info_call(ifs: &[IfInfo], capacity: u32) -> Result<Vec<u8>, (u32, u32)> {
    let b = interface_info(ifs);
    if (capacity as usize) < b.len() {
        return Err((crate::ERROR_INSUFFICIENT_BUFFER, b.len() as u32));
    }
    Ok(b)
}

/// `if_indextoname`: the interface's name as Windows spells it
/// (`ethernet_<n>`, `loopback_0`), or `None` for an unknown index.
pub fn index_to_name(ifs: &[IfInfo], index: u32) -> Option<String> {
    let f = ifs.iter().find(|f| f.index == index)?;
    Some(match f.kind {
        Kind::Loopback => String::from("loopback_0"),
        Kind::Ethernet => alloc::format!("ethernet_{}", 32768 + f.index),
    })
}

/// `if_nametoindex` for the same names.
pub fn name_to_index(ifs: &[IfInfo], name: &str) -> u32 {
    ifs.iter().find(|f| index_to_name(ifs, f.index).as_deref() == Some(name)).map_or(0, |f| f.index)
}

#[allow(dead_code)]
const _: u32 = NO_ERROR;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[test]
    fn adapter_info_lists_the_ethernet_adapter_only() {
        let ifs = [IfInfo::loopback(), eth()];
        let b = adapter_info(&ifs, 0x4000).unwrap();
        assert_eq!(b.len(), ADAPTER_INFO_SIZE);
        assert_eq!(u64_at(&b, 0), 0);
        assert_eq!(u32_at(&b, 8), 2);
        assert_eq!(ansi_at(&b, 12), "{504F4C59-0002-4000-8000-525400123456}");
        assert_eq!(ansi_at(&b, 272), "Poly VirtIO Ethernet Adapter");
        assert_eq!(u32_at(&b, 404), 6);
        assert_eq!(&b[408..414], &eth().mac);
        assert_eq!(u32_at(&b, 416), 2);
        assert_eq!(u32_at(&b, 420), 6);
        assert_eq!(ansi_at(&b, 440 + 8), "10.0.2.15");
        assert_eq!(ansi_at(&b, 440 + 24), "255.255.255.0");
        assert_eq!(ansi_at(&b, 488 + 8), "10.0.2.2");
    }

    #[test]
    fn adapter_info_chains_two_and_checks_size() {
        let mut second = eth();
        second.index = 3;
        let ifs = [eth(), second];
        let b = adapter_info(&ifs, 0x4000).unwrap();
        assert_eq!(b.len(), 2 * ADAPTER_INFO_SIZE);
        assert_eq!(u64_at(&b, 0), 0x4000 + ADAPTER_INFO_SIZE as u64);
        assert_eq!(adapter_info_call(&ifs, 0x4000, 100), Err((ERROR_BUFFER_OVERFLOW, 1408)));
        assert_eq!(adapter_info_call(&[IfInfo::loopback()], 0, 0), Err((ERROR_NO_DATA, 0)));
        assert!(adapter_info_call(&ifs, 0x4000, 1408).is_ok());
    }

    #[test]
    fn interface_info_names_the_device() {
        let b = interface_info(&[IfInfo::loopback(), eth()]);
        assert_eq!(b.len(), 4 + INDEX_MAP_SIZE);
        assert_eq!(u32_at(&b, 0), 1);
        assert_eq!(u32_at(&b, 4), 2);
        assert_eq!(wide_at(&b, 8), "\\DEVICE\\TCPIP_{504F4C59-0002-4000-8000-525400123456}");
        assert_eq!(interface_info_call(&[eth()], 4), Err((crate::ERROR_INSUFFICIENT_BUFFER, 264)));
        assert!(interface_info_call(&[eth()], 264).is_ok());
    }

    #[test]
    fn masks_and_names() {
        assert_eq!(mask(24), [255, 255, 255, 0]);
        assert_eq!(mask(8), [255, 0, 0, 0]);
        assert_eq!(mask(0), [0; 4]);
        assert_eq!(mask(32), [255; 4]);
        let ifs = [IfInfo::loopback(), eth()];
        assert_eq!(index_to_name(&ifs, 1).as_deref(), Some("loopback_0"));
        assert_eq!(index_to_name(&ifs, 2).as_deref(), Some("ethernet_32770"));
        assert_eq!(index_to_name(&ifs, 9), None);
        assert_eq!(name_to_index(&ifs, "ethernet_32770"), 2);
        assert_eq!(name_to_index(&ifs, "nothing"), 0);
    }
}
