//! What the ridge and the settings window show about the system (ADR
//! 0117): the network from the kernel's interfaces (`getifaddrs`, which
//! the C library answers with the netlink requests Poly's Linux
//! personality serves), and the facts of the About page (`uname`,
//! `/proc/cpuinfo`, `/proc/meminfo`).

use poly_shell::Network;
use std::ffi::CStr;

/// An interface as `getifaddrs` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub up: bool,
    pub running: bool,
    pub loopback: bool,
    /// It has an IPv4 or IPv6 address.
    pub addressed: bool,
}

/// The interfaces, merged by name.
pub fn interfaces() -> Vec<Interface> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `list` is a valid pointer to store the list's head in.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Vec::new();
    }
    let mut out: Vec<Interface> = Vec::new();
    let mut p = list;
    while !p.is_null() {
        // SAFETY: `p` is a node of the list getifaddrs returned, not freed yet.
        let ifa = unsafe { &*p };
        // SAFETY: ifa_name is a NUL-terminated string in the node.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let flags = ifa.ifa_flags;
        // SAFETY: a non-null ifa_addr points at a sockaddr of the node.
        let family = if ifa.ifa_addr.is_null() { 0 } else { i32::from(unsafe { (*ifa.ifa_addr).sa_family }) };
        let addressed = family == libc::AF_INET || family == libc::AF_INET6;
        let entry = Interface {
            name: name.clone(),
            up: flags & libc::IFF_UP as u32 != 0,
            running: flags & libc::IFF_RUNNING as u32 != 0,
            loopback: flags & libc::IFF_LOOPBACK as u32 != 0,
            addressed,
        };
        match out.iter_mut().find(|i| i.name == name) {
            Some(i) => {
                i.addressed |= entry.addressed;
                i.up |= entry.up;
                i.running |= entry.running;
            }
            None => out.push(entry),
        }
        p = ifa.ifa_next;
    }
    // SAFETY: freeing the list getifaddrs made, once.
    unsafe { libc::freeifaddrs(list) };
    out
}

/// The network as the ridge shows it: online through the first interface
/// that is up, running and has an address (loopback aside).
pub fn network(ifs: &[Interface]) -> Network {
    match ifs.iter().find(|i| !i.loopback && i.up && i.running && i.addressed) {
        Some(i) => Network::Online(i.name.clone()),
        None => Network::Offline,
    }
}

/// The network in words, for the About page.
pub fn describe(n: &Network) -> String {
    match n {
        Network::Online(name) => format!("connected ({name})"),
        Network::Offline => "not connected".into(),
        Network::Unknown => "unknown".into(),
    }
}

/// `uname`: (system, release, version, machine).
pub fn uname() -> (String, String, String, String) {
    // SAFETY: utsname is plain old data; all zeroes is a valid value.
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `u` is a valid utsname to fill.
    if unsafe { libc::uname(&mut u) } != 0 {
        return Default::default();
    }
    let s = |f: &[libc::c_char]| {
        let bytes: Vec<u8> = f.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    (s(&u.sysname), s(&u.release), s(&u.version), s(&u.machine))
}

/// The first `model name` of `/proc/cpuinfo`'s text.
pub fn cpu_model(cpuinfo: &str) -> String {
    cpuinfo
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == "model name").map(|(_, v)| v.trim().to_string()))
        .unwrap_or_default()
}

/// `MemTotal` of `/proc/meminfo`'s text, in KiB.
pub fn mem_total_kib(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_network() {
        let lo = Interface { name: "lo".into(), up: true, running: true, loopback: true, addressed: true };
        let mut eth = Interface { name: "eth0".into(), up: true, running: true, loopback: false, addressed: false };
        assert_eq!(network(&[lo.clone(), eth.clone()]), Network::Offline, "no address yet");
        eth.addressed = true;
        assert_eq!(network(&[lo.clone(), eth.clone()]), Network::Online("eth0".into()));
        assert_eq!(network(&[lo]), Network::Offline);
        assert_eq!(describe(&Network::Online("eth0".into())), "connected (eth0)");
        // The host's own interfaces can be listed (at least loopback).
        let _ = interfaces();
    }

    #[test]
    fn proc_files() {
        assert_eq!(
            cpu_model("processor\t: 0\nmodel name\t: QEMU Virtual CPU version 2.5+\nflags: x\n"),
            "QEMU Virtual CPU version 2.5+"
        );
        assert_eq!(cpu_model(""), "");
        assert_eq!(mem_total_kib("MemTotal:        524288 kB\nMemFree: 1 kB\n"), 524_288);
        assert_eq!(mem_total_kib("nothing"), 0);
        assert!(!uname().0.is_empty());
    }
}
