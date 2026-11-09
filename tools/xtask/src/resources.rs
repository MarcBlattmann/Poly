//! What an interactive boot gives its virtual machine: memory and CPUs
//! (`--memory=SIZE`, `--cpus=N` on `cargo xtask desktop`, `run`, `chromium
//! run` and `toolkits`; the resources section of `cargo xtask start`, ADR 0141).
//!
//! The flags are the only way in (no environment variable), so
//! `start --print` and `--last` show and repeat them. The boot tests never
//! read them: their sizes are fixed, so a run is the same on every host.
//!
//! Each command has defaults of its own (the desktop 512M, Chromium 4G and
//! 4 CPUs, ...); [`Resources`] only holds what was asked for and the
//! command says what it falls back to.

use crate::Result;

/// The smallest guest memory taken: the kernel and a shell need about this.
pub const MIN_MEMORY_MIB: u64 = 128;

/// The most CPUs the kernel starts: `percpu::MAX_CPUS`. Asked for more,
/// QEMU emulates them, but the kernel leaves the rest stopped (the boot
/// log says `smp: 16 of 32 CPUs online`). A test keeps this in step with
/// the kernel's constant.
pub const KERNEL_MAX_CPUS: u32 = 16;

/// The memory sizes the menu offers (the kernel takes any size from
/// [`MIN_MEMORY_MIB`]: its frame allocator spans up to 4 PiB of physical
/// addresses, ADR 0141).
pub const MEMORY_PRESETS: [&str; 6] = ["512M", "1G", "2G", "4G", "8G", "16G"];

/// The share of the host's memory kept for the host: a guest of all of it
/// would make the host swap.
const HOST_RESERVE_DIVISOR: u64 = 4;
/// But at least this much (MiB) stays for it.
const HOST_RESERVE_MIN_MIB: u64 = 1024;

/// What was asked for; `None` is "the command's own default".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Resources {
    /// Guest memory in MiB.
    pub memory_mib: Option<u64>,
    /// Virtual CPUs (`-smp`).
    pub cpus: Option<u32>,
}

/// `4G`, `512M`, `3072M` (a number and `M` or `G`, either case, `MiB` and
/// `GiB` too; a bare number is MiB, as QEMU's `-m`) as MiB.
pub fn parse_memory(v: &str) -> Option<u64> {
    let v = v.trim();
    let lower = v.to_ascii_lowercase();
    let digits = lower.trim_end_matches(|c: char| !c.is_ascii_digit());
    let unit = lower[digits.len()..].trim();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let mib = match unit {
        "" | "m" | "mb" | "mib" => n,
        "g" | "gb" | "gib" => n.checked_mul(1024)?,
        _ => return None,
    };
    (MIN_MEMORY_MIB..=(1 << 32)).contains(&mib).then_some(mib)
}

/// MiB as the shortest of `4G` and `3072M`.
pub fn format_memory(mib: u64) -> String {
    if mib.is_multiple_of(1024) { format!("{}G", mib / 1024) } else { format!("{mib}M") }
}

/// A memory size for people: `15.8 GiB`.
pub fn gib(mib: u64) -> String {
    format!("{:.1} GiB", mib as f64 / 1024.0)
}

/// The flags this module reads.
pub fn is_flag(arg: &str) -> bool {
    arg.starts_with("--memory=") || arg.starts_with("--cpus=")
}

/// What this host can give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostResources {
    /// Its physical memory in MiB, if it can be found out.
    pub memory_mib: Option<u64>,
    /// Its logical cores.
    pub cpus: u32,
}

impl HostResources {
    pub fn probe() -> HostResources {
        HostResources {
            memory_mib: host_memory_mib(),
            cpus: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
        }
    }

    /// The most guest memory (MiB) offered: the host's, less what stays
    /// for the host. `None` if the host's is not known.
    pub fn memory_limit_mib(&self) -> Option<u64> {
        self.memory_mib.map(|total| {
            let reserve = (total / HOST_RESERVE_DIVISOR).max(HOST_RESERVE_MIN_MIB);
            total.saturating_sub(reserve).max(MIN_MEMORY_MIB)
        })
    }

    /// The most CPUs offered: the host's cores, and no more than the
    /// kernel starts.
    pub fn cpu_limit(&self) -> u32 {
        self.cpus.clamp(1, KERNEL_MAX_CPUS)
    }

    /// Why `mib` of memory is not offered, if it is not.
    pub fn memory_refusal(&self, mib: u64) -> Option<String> {
        let (total, limit) = (self.memory_mib?, self.memory_limit_mib()?);
        (mib > limit).then(|| {
            format!(
                "{} is more than this host can give: it has {} and keeps a quarter (at least 1 GiB) for itself, so {} at most",
                format_memory(mib),
                gib(total),
                format_memory(limit / 128 * 128)
            )
        })
    }

    /// Why `n` CPUs are not offered, if they are not.
    pub fn cpu_refusal(&self, n: u32) -> Option<String> {
        if n > self.cpus {
            Some(format!("{n} CPUs are more than this host's {} logical cores", self.cpus))
        } else if n > KERNEL_MAX_CPUS {
            Some(format!("the kernel starts at most {KERNEL_MAX_CPUS} CPUs"))
        } else {
            None
        }
    }
}

/// The host's physical memory in MiB: `GlobalMemoryStatusEx` on Windows,
/// `/proc/meminfo` on Linux, unknown elsewhere.
pub fn host_memory_mib() -> Option<u64> {
    #[cfg(windows)]
    {
        win::total_mib()
    }
    #[cfg(target_os = "linux")]
    {
        meminfo_mib(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// `MemTotal` of a `/proc/meminfo` text, in MiB.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn meminfo_mib(text: &str) -> Option<u64> {
    let line = text.lines().find_map(|l| l.strip_prefix("MemTotal:"))?;
    let kib: u64 = line.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kib / 1024)
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    }

    pub fn total_mib() -> Option<u64> {
        let mut s = MemoryStatusEx {
            length: size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `s` is a valid MEMORYSTATUSEX (repr(C), same layout as
        // the SDK's sysinfoapi.h) with `dwLength` set, as the call requires;
        // it writes only inside it.
        let ok = unsafe { GlobalMemoryStatusEx(&mut s) };
        (ok != 0).then_some(s.total_phys >> 20)
    }
}

impl Resources {
    /// `--memory=` and `--cpus=` in `args` (`auto` or missing: the
    /// command's default), asked for more than `host` can give: the most
    /// it can, and a note saying so (printed by [`Resources::from_args`]).
    pub fn parse(args: &[String], host: &HostResources) -> Result<(Resources, Vec<String>)> {
        let mut r = Resources::default();
        let mut notes = Vec::new();
        for a in args {
            if let Some(v) = a.strip_prefix("--memory=") {
                r.memory_mib = if v == "auto" {
                    None
                } else {
                    let mib = parse_memory(v).ok_or(format!(
                        "--memory={v}: expected auto or a size from {MIN_MEMORY_MIB}M, such as 4G or 3072M"
                    ))?;
                    match (host.memory_refusal(mib), host.memory_limit_mib()) {
                        (Some(why), Some(limit)) => {
                            let limit = limit / 128 * 128;
                            notes.push(format!("--memory={v}: {why}; using {}", format_memory(limit)));
                            Some(limit)
                        }
                        _ => Some(mib),
                    }
                };
            } else if let Some(v) = a.strip_prefix("--cpus=") {
                r.cpus = if v == "auto" {
                    None
                } else {
                    let n = v
                        .parse::<u32>()
                        .ok()
                        .filter(|n| *n >= 1)
                        .ok_or(format!("--cpus={v}: expected auto or a number of CPUs from 1"))?;
                    match host.cpu_refusal(n) {
                        Some(why) => {
                            notes.push(format!("--cpus={v}: {why}; using {}", host.cpu_limit()));
                            Some(host.cpu_limit())
                        }
                        None => Some(n),
                    }
                };
            }
        }
        Ok((r, notes))
    }

    /// [`Resources::parse`] for this host, the notes printed.
    pub fn from_args(args: &[String]) -> Result<Resources> {
        let (r, notes) = Resources::parse(args, &HostResources::probe())?;
        for n in notes {
            eprintln!("==> {n}");
        }
        Ok(r)
    }

    /// The guest memory as QEMU's `-m` takes it: this, or `default`.
    pub fn memory(&self, default: &str) -> String {
        self.memory_mib.map_or_else(|| default.to_string(), format_memory)
    }

    /// The virtual CPUs: this, or `default`.
    pub fn cpus_or(&self, default: u32) -> u32 {
        self.cpus.unwrap_or(default)
    }

    /// What a boot prints: the memory and CPUs, and whether they were asked for.
    pub fn describe(&self, default_memory: &str, default_cpus: u32) -> String {
        let asked = if *self == Resources::default() { "" } else { " (--memory=, --cpus=)" };
        format!("==> guest: {} of memory, {} CPUs{asked}", self.memory(default_memory), self.cpus_or(default_cpus))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(memory_mib: Option<u64>, cpus: u32) -> HostResources {
        HostResources { memory_mib, cpus }
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn memory_sizes_parse_and_print() {
        assert_eq!(parse_memory("4G"), Some(4096));
        assert_eq!(parse_memory("4g"), Some(4096));
        assert_eq!(parse_memory("512M"), Some(512));
        assert_eq!(parse_memory("512"), Some(512));
        assert_eq!(parse_memory(" 3072MiB "), Some(3072));
        assert_eq!(parse_memory("2GiB"), Some(2048));
        for bad in ["", "G", "4.5G", "-1G", "4T", "64M", "0", "4 G x", "99999999999999999999G"] {
            assert_eq!(parse_memory(bad), None, "{bad:?}");
        }
        assert_eq!(format_memory(4096), "4G");
        assert_eq!(format_memory(3072), "3G");
        assert_eq!(format_memory(1536), "1536M");
        assert_eq!(format_memory(512), "512M");
        for p in MEMORY_PRESETS {
            assert_eq!(format_memory(parse_memory(p).unwrap()), p);
        }
        assert_eq!(gib(16200), "15.8 GiB");
    }

    #[test]
    fn meminfo_is_read() {
        let text = "MemTotal:       16384000 kB\nMemFree:         1000 kB\n";
        assert_eq!(meminfo_mib(text), Some(16000));
        assert_eq!(meminfo_mib("MemFree: 1 kB\n"), None);
        assert_eq!(meminfo_mib("MemTotal: lots kB\n"), None);
    }

    #[test]
    fn the_host_keeps_a_share_for_itself() {
        let h = host(Some(16 * 1024), 8);
        // A quarter of 16 GiB stays: 12 GiB at most.
        assert_eq!(h.memory_limit_mib(), Some(12 * 1024));
        assert!(h.memory_refusal(12 * 1024).is_none());
        let why = h.memory_refusal(16 * 1024).unwrap();
        assert!(why.contains("16G") && why.contains("16.0 GiB") && why.contains("12G"), "{why}");
        // A small host keeps at least 1 GiB.
        assert_eq!(host(Some(2048), 2).memory_limit_mib(), Some(1024));
        // A tiny one still offers the least memory.
        assert_eq!(host(Some(512), 1).memory_limit_mib(), Some(MIN_MEMORY_MIB));
        // Unknown: nothing is refused.
        assert_eq!(host(None, 4).memory_limit_mib(), None);
        assert!(host(None, 4).memory_refusal(1 << 20).is_none());
    }

    #[test]
    fn cpus_are_the_hosts_and_the_kernels_at_most() {
        let h = host(None, 8);
        assert_eq!(h.cpu_limit(), 8);
        assert!(h.cpu_refusal(8).is_none());
        assert!(h.cpu_refusal(9).unwrap().contains("8 logical cores"));
        let big = host(None, 64);
        assert_eq!(big.cpu_limit(), KERNEL_MAX_CPUS);
        assert!(big.cpu_refusal(17).unwrap().contains("at most 16"));
        assert_eq!(host(None, 0).cpu_limit(), 1);
    }

    #[test]
    fn flags_are_parsed_and_clamped() {
        let h = host(Some(16 * 1024), 8);
        let (r, notes) = Resources::parse(&args(&["desktop", "--release", "poly.x=1"]), &h).unwrap();
        assert_eq!((r, notes.len()), (Resources::default(), 0));
        let (r, notes) = Resources::parse(&args(&["--memory=4G", "--cpus=4"]), &h).unwrap();
        assert_eq!((r.memory_mib, r.cpus, notes.len()), (Some(4096), Some(4), 0));
        let (r, _) = Resources::parse(&args(&["--memory=auto", "--cpus=auto"]), &h).unwrap();
        assert_eq!(r, Resources::default());
        // More than the host has: the most it can, and a note.
        let (r, notes) = Resources::parse(&args(&["--memory=64G", "--cpus=32"]), &h).unwrap();
        assert_eq!((r.memory_mib, r.cpus), (Some(12 * 1024), Some(8)));
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("--memory=64G") && notes[0].contains("using 12G"), "{notes:?}");
        assert!(notes[1].contains("using 8"), "{notes:?}");
        // On a host whose memory is unknown, what is asked for is taken.
        let (r, notes) = Resources::parse(&args(&["--memory=64G"]), &host(None, 8)).unwrap();
        assert_eq!((r.memory_mib, notes.len()), (Some(65536), 0));
        // Wrong values are errors that say what is expected.
        for bad in ["--memory=lots", "--memory=64M", "--memory=", "--cpus=0", "--cpus=two", "--cpus=-1"] {
            let e = Resources::parse(&args(&[bad]), &h).unwrap_err();
            assert!(e.starts_with(bad.split('=').next().unwrap()) && e.contains("expected"), "{bad}: {e}");
        }
    }

    #[test]
    fn defaults_fill_what_was_not_asked() {
        let none = Resources::default();
        assert_eq!(none.memory("512M"), "512M");
        assert_eq!(none.cpus_or(2), 2);
        assert_eq!(none.describe("512M", 2), "==> guest: 512M of memory, 2 CPUs");
        let r = Resources { memory_mib: Some(3072), cpus: Some(6) };
        assert_eq!(r.memory("512M"), "3G");
        assert_eq!(r.cpus_or(2), 6);
        assert_eq!(r.describe("512M", 2), "==> guest: 3G of memory, 6 CPUs (--memory=, --cpus=)");
        let only_cpus = Resources { memory_mib: None, cpus: Some(1) };
        assert_eq!(only_cpus.memory("4G"), "4G");
    }

    #[test]
    fn is_flag_knows_only_its_own() {
        assert!(is_flag("--memory=1G") && is_flag("--cpus=2"));
        assert!(!is_flag("--memory") && !is_flag("--release") && !is_flag("poly.cpus=2"));
    }

    /// The kernel's limit on CPUs is the one the menu shows.
    #[test]
    fn the_kernels_cpu_limit_is_known() {
        let src = std::fs::read_to_string(crate::root().join("kernel/src/arch/x86_64/percpu.rs")).unwrap();
        let line = src.lines().find(|l| l.contains("pub const MAX_CPUS: usize")).expect("percpu::MAX_CPUS");
        assert!(line.contains(&format!("= {KERNEL_MAX_CPUS};")), "kernel: {line}; xtask: {KERNEL_MAX_CPUS}");
    }
}
