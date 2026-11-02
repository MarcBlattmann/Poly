//! Facts about the system: the network interfaces and the kernel's name
//! and version (what `uname` gives a Linux program, read from `/proc`).

use crate::abi::InterfaceRecord;
use crate::{fs, sys};
use alloc::string::String;
use alloc::vec::Vec;

/// A network interface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub up: bool,
    pub running: bool,
    pub loopback: bool,
    /// It has an IPv4 or IPv6 address.
    pub addressed: bool,
}

/// The interfaces from `net_interfaces`'s records.
pub fn parse_interfaces(mut buf: &[u8]) -> Vec<Interface> {
    let mut out = Vec::new();
    while let Some((r, rest)) = InterfaceRecord::parse(buf) {
        out.push(Interface {
            name: String::from_utf8_lossy(r.name).into_owned(),
            up: r.flags & InterfaceRecord::UP != 0,
            running: r.flags & InterfaceRecord::RUNNING != 0,
            loopback: r.flags & InterfaceRecord::LOOPBACK != 0,
            addressed: r.flags & InterfaceRecord::ADDRESSED != 0,
        });
        buf = rest;
    }
    out
}

/// The network interfaces (none on the host).
pub fn interfaces() -> Vec<Interface> {
    let mut buf = alloc::vec![0u8; 1024];
    match sys::net_interfaces(&mut buf) {
        Ok(n) => parse_interfaces(&buf[..n]),
        Err(_) => Vec::new(),
    }
}

/// The kernel's (system name, release, version, machine), as uname(2)
/// reports them.
pub fn uname() -> (String, String, String, String) {
    let read = |f: &str| fs::read_to_string(&alloc::format!("/proc/sys/kernel/{f}")).unwrap_or_default();
    let line = |s: String| String::from(s.trim_end());
    (line(read("ostype")), line(read("osrelease")), line(read("version")), "x86_64".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_records() {
        let mut raw = Vec::new();
        InterfaceRecord { flags: InterfaceRecord::UP | InterfaceRecord::LOOPBACK, name: b"lo" }.write(&mut raw);
        InterfaceRecord { flags: InterfaceRecord::RUNNING | InterfaceRecord::ADDRESSED, name: b"eth0" }.write(&mut raw);
        let ifs = parse_interfaces(&raw);
        assert_eq!(ifs.len(), 2);
        assert!(ifs[0].up && ifs[0].loopback && !ifs[0].running);
        assert_eq!(ifs[1].name, "eth0");
        assert!(ifs[1].running && ifs[1].addressed && !ifs[1].up);
        assert!(interfaces().is_empty(), "none on the host");
    }
}
