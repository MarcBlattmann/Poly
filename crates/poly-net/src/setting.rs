//! A static interface configuration written as one word, the way a kernel
//! command line carries it:
//!
//! ```text
//! 10.0.2.15/24,gw=10.0.2.2,dns=10.0.2.3    address/prefix, router, name server (both optional)
//! qemu                                     the same: QEMU's user networking ("slirp") defaults
//! off                                      leave the device alone
//! ```
//!
//! The QEMU defaults are those of QEMU's documentation for `-netdev user`:
//! guest 10.0.2.15 on 10.0.2.0/24, the host (router) at 10.0.2.2 and the
//! DNS server at 10.0.2.3.

use crate::stack::Ipv4Cidr;
use core::net::Ipv4Addr;

/// An IPv4 configuration without DHCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticIpv4 {
    pub addr: Ipv4Cidr,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Option<Ipv4Addr>,
}

/// What the setting asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// Do not bring the device up.
    Off,
    Static(StaticIpv4),
}

impl StaticIpv4 {
    /// QEMU's user networking: 10.0.2.15/24, router 10.0.2.2, DNS 10.0.2.3.
    pub const QEMU_USER: StaticIpv4 = StaticIpv4 {
        addr: Ipv4Cidr { addr: Ipv4Addr::new(10, 0, 2, 15), prefix_len: 24 },
        gateway: Some(Ipv4Addr::new(10, 0, 2, 2)),
        dns: Some(Ipv4Addr::new(10, 0, 2, 3)),
    };
}

/// Why a setting was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingError {
    /// Not `<a.b.c.d>/<prefix>`, or a prefix above 32.
    BadAddress,
    /// A `gw=` or `dns=` value that is not an IPv4 address.
    BadOption,
    /// A part that is neither `gw=` nor `dns=`.
    UnknownOption,
    /// A router outside the interface's subnet (it could never be reached).
    GatewayOffLink,
    /// The network or broadcast address of the subnet, a multicast,
    /// loopback or unspecified address as the interface address.
    UnusableAddress,
}

/// Parse a setting (see the module documentation).
pub fn parse(s: &str) -> Result<Setting, SettingError> {
    match s {
        "off" => return Ok(Setting::Off),
        "qemu" => return Ok(Setting::Static(StaticIpv4::QEMU_USER)),
        _ => {}
    }
    let mut parts = s.split(',');
    let first = parts.next().unwrap_or("");
    let (ip, prefix) = first.split_once('/').ok_or(SettingError::BadAddress)?;
    let ip: Ipv4Addr = ip.parse().map_err(|_| SettingError::BadAddress)?;
    let prefix: u8 = prefix.parse().map_err(|_| SettingError::BadAddress)?;
    if prefix > 32 {
        return Err(SettingError::BadAddress);
    }
    let addr = Ipv4Cidr::new(ip, prefix);
    let subnet_edge = prefix < 31 && (ip == addr.broadcast() || u32::from(ip) & !mask(prefix) == 0);
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || ip.is_broadcast() || subnet_edge {
        return Err(SettingError::UnusableAddress);
    }
    let mut cfg = StaticIpv4 { addr, gateway: None, dns: None };
    for part in parts {
        let (key, value) = part.split_once('=').ok_or(SettingError::UnknownOption)?;
        let value: Ipv4Addr = match key {
            "gw" | "dns" => value.parse().map_err(|_| SettingError::BadOption)?,
            _ => return Err(SettingError::UnknownOption),
        };
        if key == "gw" {
            if !addr.contains(value) {
                return Err(SettingError::GatewayOffLink);
            }
            cfg.gateway = Some(value);
        } else {
            cfg.dns = Some(value);
        }
    }
    Ok(Setting::Static(cfg))
}

fn mask(prefix: u8) -> u32 {
    if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
        Ipv4Addr::new(a, b, c, d)
    }

    #[test]
    fn full_and_short_forms() {
        let full = parse("10.0.2.15/24,gw=10.0.2.2,dns=10.0.2.3").unwrap();
        assert_eq!(full, Setting::Static(StaticIpv4::QEMU_USER));
        assert_eq!(parse("qemu").unwrap(), full);
        assert_eq!(parse("off").unwrap(), Setting::Off);
        let bare = parse("192.168.7.9/16").unwrap();
        assert_eq!(
            bare,
            Setting::Static(StaticIpv4 { addr: Ipv4Cidr::new(ip(192, 168, 7, 9), 16), gateway: None, dns: None })
        );
        let dns_only = parse("172.16.0.5/12,dns=8.8.8.8").unwrap();
        let Setting::Static(s) = dns_only else { panic!() };
        assert_eq!((s.gateway, s.dns), (None, Some(ip(8, 8, 8, 8))));
        // Point-to-point prefixes have no network or broadcast address.
        assert!(parse("10.0.0.0/31").is_ok());
        assert!(parse("10.0.0.1/32,gw=10.0.0.1").is_ok());
    }

    #[test]
    fn bad_settings() {
        use SettingError::*;
        for (s, e) in [
            ("", BadAddress),
            ("10.0.2.15", BadAddress),
            ("10.0.2.15/33", BadAddress),
            ("10.0.2/24", BadAddress),
            ("10.0.2.15/x", BadAddress),
            ("on", BadAddress),
            ("10.0.2.15/24,gw=10.0.3.1", GatewayOffLink),
            ("10.0.2.15/24,gw=10.0.2", BadOption),
            ("10.0.2.15/24,dns=", BadOption),
            ("10.0.2.15/24,mtu=1400", UnknownOption),
            ("10.0.2.15/24,gw", UnknownOption),
            ("10.0.2.0/24", UnusableAddress),
            ("10.0.2.255/24", UnusableAddress),
            ("127.0.0.1/8", UnusableAddress),
            ("224.0.0.1/4", UnusableAddress),
            ("0.0.0.0/0", UnusableAddress),
        ] {
            assert_eq!(parse(s), Err(e), "{s:?}");
        }
    }
}
