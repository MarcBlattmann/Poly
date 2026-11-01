//! The text of each procfs file, from the facts a [`ProcSource`] reports.
//! Formats follow the proc(5) family of manual pages, cited per function.
//!
//! [`ProcSource`]: crate::ProcSource

use crate::cpuflags::flag_names;
use crate::source::{
    CpuInfo, FdInfo, KernelInfo, MapEntry, MapName, MountEntry, NANOS_PER_TICK, ProcessInfo, SystemInfo,
};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// Page size in bytes.
pub const PAGE: u64 = 4096;

/// `O_CLOEXEC` (`asm-generic/fcntl.h`), shown in `fdinfo` flags for a
/// descriptor with close-on-exec set.
pub const O_CLOEXEC: u32 = 0o2000000;

/// A byte buffer `write!` can append to.
#[derive(Debug, Default)]
pub struct Out(pub Vec<u8>);

impl Write for Out {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

impl Out {
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
}

fn ticks(ns: u64) -> u64 {
    ns / NANOS_PER_TICK
}

/// `comm`: at most 15 bytes (TASK_COMM_LEN 16 with its NUL).
pub fn comm(name: &[u8]) -> &[u8] {
    &name[..name.len().min(15)]
}

/// Sizes derived from the mappings, in bytes, as `/proc/<pid>/status`
/// names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmSizes {
    /// Everything mapped (`VmSize`).
    pub size: u64,
    /// Private writable memory other than the stack (`VmData`).
    pub data: u64,
    /// The stack (`VmStk`).
    pub stack: u64,
    /// Locked mappings (`VmLck`).
    pub locked: u64,
    /// Executable mappings of the program file (`VmExe`).
    pub exe: u64,
    /// Executable mappings of other files (`VmLib`).
    pub lib: u64,
    /// The program's text and data ranges (`startcode`/`endcode`,
    /// `start_data`/`end_data` of `/proc/<pid>/stat`).
    pub code: (u64, u64),
    pub data_range: (u64, u64),
}

/// Adds up the mappings the way `/proc/<pid>/status` reports them.
pub fn vm_sizes(maps: &[MapEntry], exe: Option<&[u8]>) -> VmSizes {
    let mut v = VmSizes::default();
    for m in maps {
        let size = m.size();
        v.size += size;
        if m.locked {
            v.locked += size;
        }
        let of_exe = matches!((&m.name, exe), (MapName::Path(p), Some(e)) if p.as_slice() == e);
        match m.name {
            MapName::Stack => v.stack += size,
            _ if m.write && !m.shared => v.data += size,
            _ => {}
        }
        if m.exec && of_exe {
            v.exe += size;
            if v.code.0 == 0 {
                v.code = (m.start, m.end);
            } else {
                v.code.1 = m.end;
            }
        } else if m.exec && m.is_file() {
            v.lib += size;
        }
        if of_exe && m.write {
            if v.data_range.0 == 0 {
                v.data_range = (m.start, m.end);
            } else {
                v.data_range.1 = m.end;
            }
        }
    }
    v
}

/// `/proc/<pid>/stat`: 52 fields on one line (proc_pid_stat(5)). Fields
/// the kernel does not track (faults, priorities, the kernel stack
/// pointer, wait channel, swap) are 0; the scheduling policy is
/// `SCHED_OTHER` (0) and the nice value 0, so `priority` is 20.
pub fn stat(p: &ProcessInfo, maps: &[MapEntry], sys: &SystemInfo) -> Vec<u8> {
    let _ = sys;
    let v = vm_sizes(maps, p.exe.as_deref());
    let mut o = Out::default();
    let _ = write!(o, "{} (", p.pid);
    o.bytes(comm(&p.comm));
    let _ = writeln!(
        o,
        ") {} {} {} {} 0 -1 {} 0 0 0 0 {} {} 0 0 20 0 {} 0 {} {} {} {} {} {} {} 0 0 {} {} {} {} 0 0 0 17 0 0 0 0 0 0 {} {} {} {} {} {} {} {}",
        p.state.letter(),
        p.ppid,
        p.pgrp,
        p.session,
        // flags: none of the kernel's PF_* bits are meaningful here.
        0,
        ticks(p.user_ns),
        ticks(p.system_ns),
        p.threads,
        ticks(p.start_ns),
        v.size,
        p.resident_pages,
        p.rss_limit,
        v.code.0,
        v.code.1,
        p.start_stack,
        // signal, blocked, sigignore, sigcatch (decimal bitmaps of the
        // standard signals).
        p.signals.pending | p.signals.shared_pending,
        p.signals.blocked,
        p.signals.ignored,
        p.signals.caught,
        v.data_range.0,
        v.data_range.1,
        p.start_brk,
        p.arg.0,
        p.arg.1,
        p.env.0,
        p.env.1,
        p.exit_code,
    );
    o.0
}

/// `/proc/<pid>/statm`: sizes in pages (proc_pid_statm(5)): size,
/// resident, shared (file-backed resident), text, lib (0), data + stack,
/// dirty (0).
pub fn statm(p: &ProcessInfo, maps: &[MapEntry]) -> Vec<u8> {
    let v = vm_sizes(maps, p.exe.as_deref());
    let mut o = Out::default();
    let _ = writeln!(
        o,
        "{} {} {} {} 0 {} 0",
        v.size / PAGE,
        p.resident_pages,
        p.resident_file_pages,
        v.exe / PAGE,
        (v.data + v.stack) / PAGE
    );
    o.0
}

fn kb(o: &mut Out, name: &str, bytes: u64) {
    let _ = writeln!(o, "{name}:\t{:>8} kB", bytes / 1024);
}

/// `/proc/<pid>/status` (proc_pid_status(5)): the fields of the manual's
/// example, in its order, `Key:\tvalue` per line.
pub fn status(p: &ProcessInfo, maps: &[MapEntry]) -> Vec<u8> {
    let v = vm_sizes(maps, p.exe.as_deref());
    let rss = p.resident_pages * PAGE;
    let rss_file = p.resident_file_pages.min(p.resident_pages) * PAGE;
    let mut o = Out::default();
    o.bytes(b"Name:\t");
    o.bytes(comm(&p.comm));
    let [u0, u1, u2, u3] = p.uids;
    let [g0, g1, g2, g3] = p.gids;
    let _ = write!(
        o,
        "\nUmask:\t{:04o}\nState:\t{}\nTgid:\t{pid}\nNgid:\t0\nPid:\t{pid}\nPPid:\t{}\nTracerPid:\t0\n\
         Uid:\t{u0}\t{u1}\t{u2}\t{u3}\nGid:\t{g0}\t{g1}\t{g2}\t{g3}\nFDSize:\t{}\nGroups:\t\n\
         NStgid:\t{pid}\nNSpid:\t{pid}\nNSpgid:\t{}\nNSsid:\t{}\n",
        p.umask,
        p.state.status(),
        p.ppid,
        p.fd_slots,
        p.pgrp,
        p.session,
        pid = p.pid,
    );
    kb(&mut o, "VmPeak", v.size);
    kb(&mut o, "VmSize", v.size);
    kb(&mut o, "VmLck", v.locked);
    kb(&mut o, "VmPin", 0);
    kb(&mut o, "VmHWM", rss);
    kb(&mut o, "VmRSS", rss);
    kb(&mut o, "RssAnon", rss - rss_file);
    kb(&mut o, "RssFile", rss_file);
    kb(&mut o, "RssShmem", 0);
    kb(&mut o, "VmData", v.data);
    kb(&mut o, "VmStk", v.stack);
    kb(&mut o, "VmExe", v.exe);
    kb(&mut o, "VmLib", v.lib);
    kb(&mut o, "VmPTE", 0);
    kb(&mut o, "VmSwap", 0);
    let s = &p.signals;
    let _ = write!(
        o,
        "HugetlbPages:\t{:>8} kB\nCoreDumping:\t0\nThreads:\t{}\nSigQ:\t{}/{}\nSigPnd:\t{:016x}\nShdPnd:\t{:016x}\n\
         SigBlk:\t{:016x}\nSigIgn:\t{:016x}\nSigCgt:\t{:016x}\nCapInh:\t{:016x}\nCapPrm:\t{:016x}\nCapEff:\t{:016x}\n\
         CapBnd:\t{:016x}\nCapAmb:\t{:016x}\nNoNewPrivs:\t0\nSeccomp:\t0\nSeccomp_filters:\t0\n\
         Cpus_allowed:\t1\nCpus_allowed_list:\t0\nMems_allowed:\t1\nMems_allowed_list:\t0\n\
         voluntary_ctxt_switches:\t0\nnonvoluntary_ctxt_switches:\t0\n",
        0,
        p.threads,
        s.queued,
        s.queue_limit,
        s.pending,
        s.shared_pending,
        s.blocked,
        s.ignored,
        s.caught,
        0,
        // Everyone is root with every capability until users arrive:
        // capabilities(7) lists 41 of them (bits 0-40).
        if p.uids[1] == 0 { ALL_CAPS } else { 0 },
        if p.uids[1] == 0 { ALL_CAPS } else { 0 },
        ALL_CAPS,
        0,
    );
    o.0
}

/// Every capability bit (0 to `CAP_CHECKPOINT_RESTORE`, 40;
/// `linux/capability.h`).
pub const ALL_CAPS: u64 = (1 << 41) - 1;

/// Escapes what `/proc/<pid>/maps` cannot show: a newline becomes `\012`
/// (proc_pid_maps(5)).
fn map_path(o: &mut Out, path: &[u8]) {
    for &b in path {
        if b == b'\n' {
            o.bytes(b"\\012");
        } else {
            o.0.push(b);
        }
    }
}

/// The column where the path of a `/proc/<pid>/maps` line starts (the
/// fields before it padded with spaces), when the line has one.
pub const MAPS_PATH_COLUMN: usize = 73;

/// `/proc/<pid>/maps` (proc_pid_maps(5)): `start-end perms offset
/// major:minor inode` and the path or pseudo-path, addresses and offset in
/// hex (at least 8 digits), the device numbers in hex (2 digits).
pub fn maps(entries: &[MapEntry]) -> Vec<u8> {
    let mut o = Out::default();
    for m in entries {
        let line_start = o.0.len();
        let _ = write!(
            o,
            "{:08x}-{:08x} {}{}{}{} {:08x} {:02x}:{:02x} {}",
            m.start,
            m.end,
            if m.read { 'r' } else { '-' },
            if m.write { 'w' } else { '-' },
            if m.exec { 'x' } else { '-' },
            if m.shared { 's' } else { 'p' },
            m.offset,
            m.dev.0,
            m.dev.1,
            m.ino
        );
        let name: Option<&[u8]> = match &m.name {
            MapName::None => None,
            MapName::Path(p) => Some(p),
            MapName::Heap => Some(b"[heap]"),
            MapName::Stack => Some(b"[stack]"),
            MapName::Label(l) => Some(l.as_bytes()),
        };
        if let Some(name) = name {
            let used = o.0.len() - line_start;
            let pad = MAPS_PATH_COLUMN.saturating_sub(used).max(1);
            o.0.extend(core::iter::repeat_n(b' ', pad));
            map_path(&mut o, name);
        }
        o.0.push(b'\n');
    }
    o.0
}

/// `/proc/<pid>/fdinfo/<n>` (proc_pid_fdinfo(5)): `pos`, `flags` in octal
/// with a leading 0, `mnt_id`, `ino`.
pub fn fdinfo(f: &FdInfo) -> Vec<u8> {
    let mut o = Out::default();
    let _ = write!(o, "pos:\t{}\nflags:\t0{:o}\nmnt_id:\t{}\nino:\t{}\n", f.pos, f.flags, f.mnt_id, f.ino);
    o.0
}

/// Escapes a field of `/proc/mounts` the way fstab(5) describes: space,
/// tab, newline and backslash as octal (`\040`, `\011`, `\012`, `\134`).
fn mount_field(o: &mut Out, field: &[u8]) {
    for &b in field {
        match b {
            b' ' | b'\t' | b'\n' | b'\\' => {
                let _ = write!(o, "\\{b:03o}");
            }
            _ => o.0.push(b),
        }
    }
}

fn mount_options(m: &MountEntry) -> String {
    let mut s = String::from(if m.read_only { "ro" } else { "rw" });
    for (on, name) in [(m.no_suid, ",nosuid"), (m.no_dev, ",nodev"), (m.no_exec, ",noexec")] {
        if on {
            s.push_str(name);
        }
    }
    s
}

/// `/proc/<pid>/mounts` (proc_pid_mounts(5): "documented in fstab(5)"):
/// source, mount point, type, options, and the dump and pass fields (0).
pub fn mounts(entries: &[MountEntry]) -> Vec<u8> {
    let mut o = Out::default();
    for m in entries {
        mount_field(&mut o, &m.source);
        o.0.push(b' ');
        mount_field(&mut o, &m.mount_point);
        let _ = writeln!(o, " {} {} 0 0", m.fstype, mount_options(m));
    }
    o.0
}

/// `/proc/<pid>/mountinfo` (proc_pid_mountinfo(5)): id, parent id,
/// major:minor, root, mount point, per-mount options, no optional fields,
/// `-`, type, source, per-superblock options.
pub fn mountinfo(entries: &[MountEntry]) -> Vec<u8> {
    let mut o = Out::default();
    for m in entries {
        let _ = write!(o, "{} {} {}:{} ", m.id, m.parent, m.dev.0, m.dev.1);
        mount_field(&mut o, &m.root);
        o.0.push(b' ');
        mount_field(&mut o, &m.mount_point);
        let _ = write!(o, " {} - {} ", mount_options(m), m.fstype);
        mount_field(&mut o, &m.source);
        let _ = writeln!(o, " {}", if m.read_only { "ro" } else { "rw" });
    }
    o.0
}

/// `/proc/filesystems` (proc_filesystems(5)): `nodev` before the types
/// that need no device, a tab, the name.
pub fn filesystems(list: &[(String, bool)]) -> Vec<u8> {
    let mut o = Out::default();
    for (name, nodev) in list {
        let _ = writeln!(o, "{}\t{name}", if *nodev { "nodev" } else { "" });
    }
    o.0
}

/// Seconds with two decimals, from nanoseconds.
fn centis(o: &mut Out, ns: u64) {
    let c = ns / 10_000_000;
    let _ = write!(o, "{}.{:02}", c / 100, c % 100);
}

/// `/proc/uptime` (proc_uptime(5)): uptime and idle time in seconds.
pub fn uptime(s: &SystemInfo) -> Vec<u8> {
    let mut o = Out::default();
    centis(&mut o, s.uptime_ns);
    o.0.push(b' ');
    centis(&mut o, s.idle_ns);
    o.0.push(b'\n');
    o.0
}

/// A load average (16 fractional bits) with two decimals, rounded.
fn load(o: &mut Out, fixed: u64) {
    let hundredths = (fixed * 100 + (1 << 15)) >> 16;
    let _ = write!(o, "{}.{:02}", hundredths / 100, hundredths % 100);
}

/// `/proc/loadavg` (proc_loadavg(5)): three load averages, runnable/all
/// scheduling entities, the newest pid.
pub fn loadavg(s: &SystemInfo) -> Vec<u8> {
    let mut o = Out::default();
    for (i, l) in s.loads.iter().enumerate() {
        if i > 0 {
            o.0.push(b' ');
        }
        load(&mut o, *l);
    }
    let _ = writeln!(o, " {}/{} {}", s.runnable, s.threads, s.last_pid);
    o.0
}

/// `/proc/stat` (proc_stat(5)): CPU time in ticks for the system (`cpu`)
/// and each CPU (one here), then interrupts, context switches, boot time,
/// processes created, running and blocked.
pub fn system_stat(s: &SystemInfo, cpus: usize) -> Vec<u8> {
    let mut o = Out::default();
    let c = &s.cpu;
    let times = [c.user, c.nice, c.system, c.idle, c.iowait, c.irq, c.softirq, c.steal, 0, 0];
    let line = |o: &mut Out, label: &str, div: u64| {
        let _ = write!(o, "{label}");
        for t in times {
            let _ = write!(o, " {}", ticks(t) / div);
        }
        o.0.push(b'\n');
    };
    line(&mut o, "cpu", 1);
    let n = cpus.max(1) as u64;
    for i in 0..n {
        line(&mut o, &alloc::format!("cpu{i}"), n);
    }
    let _ = write!(
        o,
        "intr {}\nctxt {}\nbtime {}\nprocesses {}\nprocs_running {}\nprocs_blocked {}\n",
        s.interrupts, s.context_switches, s.boot_time, s.forks, s.procs_running, s.procs_blocked
    );
    o.0
}

/// One `/proc/meminfo` line: the name and colon padded to 16 columns, the
/// value in kB right-aligned in 8.
fn meminfo_line(o: &mut Out, name: &str, bytes: u64) {
    let label = alloc::format!("{name}:");
    let _ = writeln!(o, "{label:<15} {:>8} kB", bytes / 1024);
}

/// `/proc/meminfo` (proc_meminfo(5)). There is no swap, no high memory
/// and no vmalloc area to speak of.
pub fn meminfo(s: &SystemInfo) -> Vec<u8> {
    let mut o = Out::default();
    let lines: [(&str, u64); 25] = [
        ("MemTotal", s.mem_total),
        ("MemFree", s.mem_free),
        ("MemAvailable", s.mem_available),
        ("Buffers", s.buffers),
        ("Cached", s.cached),
        ("SwapCached", 0),
        ("Active", 0),
        ("Inactive", s.cached),
        ("Active(anon)", 0),
        ("Inactive(anon)", 0),
        ("Active(file)", 0),
        ("Inactive(file)", s.cached),
        ("Unevictable", 0),
        ("Mlocked", 0),
        ("SwapTotal", 0),
        ("SwapFree", 0),
        ("Dirty", s.dirty),
        ("Writeback", 0),
        ("Shmem", s.shmem),
        ("KernelStack", s.kernel_stack),
        ("PageTables", s.page_tables),
        ("CommitLimit", s.mem_total),
        ("VmallocTotal", 0),
        ("VmallocUsed", 0),
        ("VmallocChunk", 0),
    ];
    for (name, bytes) in lines {
        meminfo_line(&mut o, name, bytes);
    }
    o.0
}

/// `/proc/cpuinfo` (proc_cpuinfo(5): "processor" and "bogomips", the rest
/// x86's usual `key\t: value` lines), one block per CPU, separated by an
/// empty line.
pub fn cpuinfo(cpus: &[CpuInfo]) -> Vec<u8> {
    let mut o = Out::default();
    let n = cpus.len();
    for c in cpus {
        let mhz = c.khz / 1000;
        let frac = c.khz % 1000;
        let _ = write!(
            o,
            "processor\t: {}\nvendor_id\t: {}\ncpu family\t: {}\nmodel\t\t: {}\nmodel name\t: {}\nstepping\t: {}\n\
             cpu MHz\t\t: {mhz}.{frac:03}\nphysical id\t: 0\nsiblings\t: {n}\ncore id\t\t: {}\ncpu cores\t: {n}\n\
             apicid\t\t: {}\nfpu\t\t: yes\nfpu_exception\t: yes\ncpuid level\t: {}\nwp\t\t: yes\nflags\t\t:",
            c.index, c.vendor, c.family, c.model, c.model_name, c.stepping, c.index, c.apic_id, c.cpuid_level,
        );
        for f in flag_names(&c.features) {
            let _ = write!(o, " {f}");
        }
        // BogoMIPS: twice the clock in MHz, the conventional value of a
        // calibrated delay loop.
        let bogo = c.khz * 2 / 10;
        let _ = write!(
            o,
            "\nbogomips\t: {}.{:02}\nclflush size\t: {}\ncache_alignment\t: {}\naddress sizes\t: {} bits physical, {} bits virtual\npower management:\n\n",
            bogo / 100,
            bogo % 100,
            c.clflush,
            c.clflush,
            c.phys_bits,
            c.virt_bits
        );
    }
    o.0
}

/// `/proc/version` (proc_version(5)): "It includes the contents of
/// /proc/sys/kernel/ostype, /proc/sys/kernel/osrelease, and
/// /proc/sys/kernel/version", as in `Linux version 1.0.9 (who@where) #1
/// date`.
pub fn version(k: &KernelInfo) -> Vec<u8> {
    let mut o = Out::default();
    let _ = writeln!(o, "{} version {} (poly@poly) {}", k.ostype, k.osrelease, k.version);
    o.0
}

/// A line of text.
pub fn line(text: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + 1);
    v.extend_from_slice(text);
    v.push(b'\n');
    v
}

/// A UUID in its text form, `6fd5a44b-35f4-4ad4-a9b9-6b9be13e1fe9`
/// (random(4)), from 16 random bytes made a version 4 (random) UUID of
/// the RFC 4122 variant (RFC 4122 section 4.4: the version in the top
/// four bits of byte 6, the variant `10` in the top bits of byte 8).
pub fn uuid(mut bytes: [u8; 16]) -> Vec<u8> {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut o = Out::default();
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            o.0.push(b'-');
        }
        let _ = write!(o, "{b:02x}");
    }
    o.0.push(b'\n');
    o.0
}
