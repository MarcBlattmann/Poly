//! `iphlpapi.dll` without the machine (ADR 0155).
//!
//! Chrome's network service asks which network interfaces there are and
//! what addresses they have (`GetAdaptersAddresses`, `GetIfTable2`) and
//! to be told when that changes (`NotifyAddrChange`). Poly's kernel knows
//! its interfaces (the loopback inside the stack and the virtio-net NIC with
//! QEMU's user-network configuration); the NT personality hands them to
//! user space as a list of [`IfInfo`] over `\Device\Afd`
//! (`poly_winsock::afd::IF_LIST`, [`wire`]). Everything else is here, tested
//! on the host: the names Windows would give them ([`IfInfo::friendly_name`]
//! and the others) and the memory layouts the API fills in:
//!
//! - [`adapters`]: `IP_ADAPTER_ADDRESSES_LH` and its lists
//!   (`GetAdaptersAddresses`, `iptypes.h`).
//! - [`ifrow`]: `MIB_IF_TABLE2` / `MIB_IF_ROW2` (`GetIfTable2`, `netioapi.h`).
//! - [`oldapi`]: `IP_ADAPTER_INFO` (`GetAdaptersInfo`) and
//!   `IP_INTERFACE_INFO` (`GetInterfaceInfo`).
//!
//! Sources (clean room): Learn's pages "GetAdaptersAddresses",
//! "IP_ADAPTER_ADDRESSES_LH", "IP_ADAPTER_UNICAST_ADDRESS_LH",
//! "IP_ADAPTER_DNS_SERVER_ADDRESS_XP", "IP_ADAPTER_GATEWAY_ADDRESS_LH",
//! "IP_ADAPTER_PREFIX_XP", "GetIfTable2", "MIB_IF_TABLE2", "MIB_IF_ROW2",
//! "FreeMibTable", "GetAdaptersInfo", "IP_ADAPTER_INFO", "IP_ADDR_STRING",
//! "GetInterfaceInfo", "IP_INTERFACE_INFO", "if_indextoname",
//! "NotifyAddrChange", "CancelIPChangeNotify", "IF_TYPE", "IF_OPER_STATUS",
//! "NET_LUID_LH"; the constants and layouts of `iptypes.h`, `iphlpapi.h`,
//! `ifdef.h`, `netioapi.h`, `ipifcons.h`, `nldef.h`, `ws2def.h`,
//! `ws2ipdef.h` and `winerror.h`. No Windows binary, Wine or ReactOS source
//! was looked at.

#![no_std]

extern crate alloc;

pub mod adapters;
pub mod ifrow;
pub mod oldapi;
pub mod wire;

use alloc::string::String;
use alloc::vec::Vec;

/// `winerror.h`.
pub const NO_ERROR: u32 = 0;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_BUFFER_OVERFLOW: u32 = 111;
pub const ERROR_NO_DATA: u32 = 232;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_IO_PENDING: u32 = 997;
pub const ERROR_OPERATION_ABORTED: u32 = 995;

/// `AF_UNSPEC`, `AF_INET`, `AF_INET6` (`winsock2.h`).
pub const AF_UNSPEC: u32 = 0;
pub const AF_INET: u32 = 2;
pub const AF_INET6: u32 = 23;

/// `IF_TYPE_ETHERNET_CSMACD`, `IF_TYPE_SOFTWARE_LOOPBACK` (`ipifcons.h`).
pub const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;

/// What an interface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Loopback,
    Ethernet,
}

/// One network interface of Poly's stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfInfo {
    /// `IfIndex`: 1 is the loopback, the NICs follow.
    pub index: u32,
    pub kind: Kind,
    pub mac: [u8; 6],
    pub mtu: u32,
    pub up: bool,
    /// Link speed in bits per second.
    pub speed: u64,
    /// The IPv4 address and prefix length.
    pub ipv4: Option<([u8; 4], u8)>,
    pub gateway: Option<[u8; 4]>,
    pub dns: Vec<[u8; 4]>,
    /// The IPv6 address (`::1`, or the NIC's link-local address) and its prefix length.
    pub ipv6: Option<([u8; 16], u8)>,
}

impl IfInfo {
    /// The loopback inside the stack: 127.0.0.1/8 and ::1/128.
    pub fn loopback() -> IfInfo {
        let mut v6 = [0u8; 16];
        v6[15] = 1;
        IfInfo {
            index: 1,
            kind: Kind::Loopback,
            mac: [0; 6],
            mtu: 65535,
            up: true,
            speed: 1_073_741_824,
            ipv4: Some(([127, 0, 0, 1], 8)),
            gateway: None,
            dns: Vec::new(),
            ipv6: Some((v6, 128)),
        }
    }

    /// `IfType` (`ipifcons.h`).
    pub fn if_type(&self) -> u32 {
        match self.kind {
            Kind::Loopback => IF_TYPE_SOFTWARE_LOOPBACK,
            Kind::Ethernet => IF_TYPE_ETHERNET_CSMACD,
        }
    }

    /// `NET_LUID`: `IfType` in bits 48..64 and `NetLuidIndex` in bits 24..48 (`ifdef.h`).
    pub fn luid(&self) -> u64 {
        (u64::from(self.if_type()) << 48) | (u64::from(self.index) << 24)
    }

    /// The adapter's GUID, made from its index and address: `{504F4C59-iiii-4000-8000-mmmmmmmmmmmm}`
    /// (`504F4C59` is "POLY").
    pub fn guid(&self) -> [u8; 16] {
        let mut g = [0u8; 16];
        // Data1 = 0x504F4C59, Data2 = index, Data3 = 0x4000, Data4 = 80 00 + MAC (little-endian fields).
        g[0..4].copy_from_slice(&0x504F_4C59u32.to_le_bytes());
        g[4..6].copy_from_slice(&(self.index as u16).to_le_bytes());
        g[6..8].copy_from_slice(&0x4000u16.to_le_bytes());
        g[8] = 0x80;
        g[9] = 0x00;
        g[10..16].copy_from_slice(&self.mac);
        g
    }

    /// `AdapterName`: the GUID in braces, upper case.
    pub fn adapter_name(&self) -> String {
        guid_string(&self.guid())
    }

    /// `FriendlyName`.
    pub fn friendly_name(&self) -> String {
        match self.kind {
            Kind::Loopback => String::from("Loopback Pseudo-Interface 1"),
            // The first NIC is "Ethernet", the next "Ethernet 2", ...
            Kind::Ethernet if self.index <= 2 => String::from("Ethernet"),
            Kind::Ethernet => alloc::format!("Ethernet {}", self.index - 1),
        }
    }

    /// `Description`.
    pub fn description(&self) -> String {
        match self.kind {
            Kind::Loopback => String::from("Software Loopback Interface 1"),
            Kind::Ethernet => String::from("Poly VirtIO Ethernet Adapter"),
        }
    }

    pub fn has_physical_address(&self) -> bool {
        self.kind == Kind::Ethernet
    }
}

/// A GUID as `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`.
pub fn guid_string(g: &[u8; 16]) -> String {
    use core::fmt::Write;
    let mut s = String::with_capacity(38);
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    let _ = write!(s, "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-", g[8], g[9]);
    for b in &g[10..16] {
        let _ = write!(s, "{b:02X}");
    }
    s.push('}');
    s
}

/// Little-endian output with absolute pointers: `base` is the address
/// the finished bytes will have in the caller's buffer.
pub(crate) struct Out {
    pub buf: Vec<u8>,
    pub base: u64,
}

impl Out {
    pub fn new(base: u64) -> Out {
        Out { buf: Vec::new(), base }
    }

    /// Align the end to `to` and return its offset.
    pub fn align(&mut self, to: usize) -> usize {
        while !self.buf.len().is_multiple_of(to) {
            self.buf.push(0);
        }
        self.buf.len()
    }

    /// Reserve `n` zero bytes (aligned to 8) and return their offset.
    pub fn alloc(&mut self, n: usize) -> usize {
        let at = self.align(8);
        self.buf.resize(at + n, 0);
        at
    }

    pub fn ptr(&self, offset: usize) -> u64 {
        self.base + offset as u64
    }

    pub fn put_u8(&mut self, at: usize, v: u8) {
        self.buf[at] = v;
    }

    pub fn put_u16(&mut self, at: usize, v: u16) {
        self.buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    pub fn put_u32(&mut self, at: usize, v: u32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn put_u64(&mut self, at: usize, v: u64) {
        self.buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    pub fn put_bytes(&mut self, at: usize, v: &[u8]) {
        self.buf[at..at + v.len()].copy_from_slice(v);
    }

    /// A NUL-terminated UTF-16 string; its offset.
    pub fn wide(&mut self, s: &str) -> usize {
        let at = self.alloc((s.encode_utf16().count() + 1) * 2);
        for (i, c) in s.encode_utf16().enumerate() {
            self.put_u16(at + i * 2, c);
        }
        at
    }

    /// A NUL-terminated ANSI string; its offset.
    pub fn ansi(&mut self, s: &str) -> usize {
        let at = self.alloc(s.len() + 1);
        self.put_bytes(at, s.as_bytes());
        at
    }

    /// A `SOCKADDR_IN` (16 bytes); its offset.
    pub fn sockaddr_in(&mut self, a: [u8; 4]) -> usize {
        let at = self.alloc(16);
        self.put_u16(at, crate::AF_INET as u16);
        self.put_bytes(at + 4, &a);
        at
    }

    /// A `SOCKADDR_IN6` (28 bytes); its offset.
    pub fn sockaddr_in6(&mut self, a: [u8; 16], scope: u32) -> usize {
        let at = self.alloc(28);
        self.put_u16(at, crate::AF_INET6 as u16);
        self.put_bytes(at + 8, &a);
        self.put_u32(at + 24, scope);
        at
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub fn eth() -> IfInfo {
        let mut v6 = [0u8; 16];
        v6[0] = 0xfe;
        v6[1] = 0x80;
        v6[8..].copy_from_slice(&[0x50, 0x54, 0x00, 0xff, 0xfe, 0x12, 0x34, 0x56]);
        IfInfo {
            index: 2,
            kind: Kind::Ethernet,
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            mtu: 1500,
            up: true,
            speed: 10_000_000_000,
            ipv4: Some(([10, 0, 2, 15], 24)),
            gateway: Some([10, 0, 2, 2]),
            dns: alloc::vec![[10, 0, 2, 3]],
            ipv6: Some((v6, 64)),
        }
    }

    pub fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }
    pub fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }
    pub fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }
    pub fn wide_at(b: &[u8], at: usize) -> alloc::string::String {
        let mut v = Vec::new();
        let mut i = at;
        while u16_at(b, i) != 0 {
            v.push(u16_at(b, i));
            i += 2;
        }
        alloc::string::String::from_utf16(&v).unwrap()
    }
    pub fn ansi_at(b: &[u8], at: usize) -> alloc::string::String {
        let end = b[at..].iter().position(|&c| c == 0).unwrap();
        alloc::string::String::from_utf8(b[at..at + end].to_vec()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn names_and_ids() {
        let e = eth();
        assert_eq!(e.adapter_name(), "{504F4C59-0002-4000-8000-525400123456}");
        assert_eq!(e.friendly_name(), "Ethernet");
        assert_eq!(e.if_type(), 6);
        assert_eq!(e.luid(), (6u64 << 48) | (2 << 24));
        let l = IfInfo::loopback();
        assert_eq!(l.adapter_name(), "{504F4C59-0001-4000-8000-000000000000}");
        assert_eq!(l.friendly_name(), "Loopback Pseudo-Interface 1");
        assert_eq!(l.luid(), (24u64 << 48) | (1 << 24));
        assert!(!l.has_physical_address());
    }
}
