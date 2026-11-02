//! What the ridge and the settings window show about the system (ADR
//! 0117): the network from the kernel's interfaces (`net_interfaces`, ADR
//! 0125), and the facts of the About page (`uname` from `/proc/sys/kernel`,
//! `/proc/cpuinfo`, `/proc/meminfo`).

use crate::prelude::*;
use poly_shell::Network;

/// A network interface.
pub use poly_rt::system::Interface;

/// The interfaces.
pub fn interfaces() -> Vec<Interface> {
    poly_rt::system::interfaces()
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
    poly_rt::system::uname()
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
        assert_eq!(uname().3, "x86_64");
    }
}
