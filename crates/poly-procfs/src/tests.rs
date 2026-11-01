//! Host tests: procfs and a static tree mounted in a VFS, over a fake
//! source of facts, and the text of each file.

extern crate std;

use crate::format;
use crate::*;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{At, Clock, Cred, FileType, FsContext, MountFlags, OpenFlags, Timespec, Tmpfs, Vfs};
use std::string::ToString;
use std::sync::Mutex;

#[derive(Debug, Default)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1000, 0)
    }
}

/// A fake kernel: processes 1 and 42, the caller is `current`.
#[derive(Debug, Default)]
struct Fake {
    current: Mutex<Option<u32>>,
    pids: Mutex<Vec<u32>>,
    /// What was written to uid_map: (pid, offset, data).
    uid_map: Mutex<Vec<(u32, u64, Vec<u8>)>>,
}

/// Process 42's namespaces have inode numbers 100 + kind (pid 1's: 200 +
/// kind), but it has no network namespace.
fn fake_inum(pid: u32, kind: usize) -> Option<u32> {
    let base = if pid == 42 { 100 } else { 200 };
    (pid != 42 || NS_KINDS[kind] != "net").then_some(base + kind as u32)
}

const EXE: &[u8] = b"/bin/prog";

fn maps_of(pid: u32) -> Vec<MapEntry> {
    let file = |start, end, w, x, off| MapEntry {
        start,
        end,
        read: true,
        write: w,
        exec: x,
        shared: false,
        offset: off,
        dev: (0, 1),
        ino: 77,
        name: MapName::Path(EXE.to_vec()),
    };
    let mut v = std::vec![
        file(0x40_0000, 0x40_1000, false, false, 0),
        file(0x40_1000, 0x40_3000, false, true, 0x1000),
        file(0x40_3000, 0x40_4000, true, false, 0x3000),
        MapEntry {
            start: 0x50_0000,
            end: 0x52_0000,
            read: true,
            write: true,
            name: MapName::Heap,
            ..Default::default()
        },
        MapEntry {
            start: 0x7f00_0000_0000,
            end: 0x7f00_0000_2000,
            read: true,
            exec: true,
            dev: (8, 2),
            ino: 5,
            name: MapName::Path(b"/lib/libx.so".to_vec()),
            ..Default::default()
        },
        MapEntry {
            start: 0x7fff_fffd_e000,
            end: 0x7fff_ffff_f000,
            read: true,
            write: true,
            name: MapName::Stack,
            ..Default::default()
        },
    ];
    if pid == 1 {
        v.truncate(1);
    }
    v
}

impl ProcSource for Fake {
    fn current_pid(&self) -> Option<u32> {
        *self.current.lock().unwrap()
    }

    fn pids(&self) -> Vec<u32> {
        self.pids.lock().unwrap().clone()
    }

    /// Process 42 has threads 42 and 43; 43 named itself "worker".
    fn threads(&self, pid: u32) -> Vec<u32> {
        match pid {
            42 => std::vec![43, 42],
            p if self.pids.lock().unwrap().contains(&p) => std::vec![p],
            _ => Vec::new(),
        }
    }

    fn thread_comm(&self, pid: u32, tid: u32) -> Option<Vec<u8>> {
        (pid == 42 && tid == 43).then(|| b"worker".to_vec())
    }

    fn process(&self, pid: u32) -> Option<ProcessInfo> {
        if !self.pids.lock().unwrap().contains(&pid) {
            return None;
        }
        Some(ProcessInfo {
            pid,
            ppid: 1,
            pgrp: pid,
            session: pid,
            comm: b"a-very-long-program-name".to_vec(),
            state: if pid == 42 { ProcState::Running } else { ProcState::Sleeping },
            uids: [1000, 1001, 1002, 1003],
            gids: [100, 101, 102, 103],
            umask: 0o022,
            threads: 3,
            fd_slots: 64,
            start_ns: 2_500_000_000,
            user_ns: 70_000_000,
            system_ns: 30_000_000,
            resident_pages: 10,
            resident_file_pages: 4,
            rss_limit: u64::MAX,
            start_stack: 0x7fff_ffff_e000,
            start_brk: 0x50_0000,
            arg: (0x7fff_ffff_e100, 0x7fff_ffff_e110),
            env: (0x7fff_ffff_e110, 0x7fff_ffff_e120),
            exe: Some(EXE.to_vec()),
            signals: SignalSets {
                pending: 0,
                shared_pending: 1 << 9,
                blocked: 1 << 1,
                ignored: 1 << 12,
                caught: 1 << 16,
                queued: 1,
                queue_limit: 4096,
            },
            exit_code: 0,
        })
    }

    fn owner(&self, pid: u32) -> Option<Owner> {
        self.process(pid).map(|p| Owner { uid: p.uids[1], gid: p.gids[1], start_ns: p.start_ns })
    }

    fn maps(&self, pid: u32) -> Vec<MapEntry> {
        maps_of(pid)
    }

    fn cmdline(&self, _: u32) -> Vec<u8> {
        b"/bin/prog\0-x\0".to_vec()
    }

    fn environ(&self, _: u32) -> Vec<u8> {
        b"HOME=/\0".to_vec()
    }

    fn exe(&self, _: u32) -> Option<Vec<u8>> {
        Some(EXE.to_vec())
    }

    fn cwd(&self, _: u32) -> Option<Vec<u8>> {
        Some(b"/tmp".to_vec())
    }

    fn root(&self, _: u32) -> Option<Vec<u8>> {
        Some(b"/".to_vec())
    }

    fn fds(&self, pid: u32) -> Vec<u32> {
        if pid == 42 { std::vec![5, 0, 1, 2] } else { std::vec![0] }
    }

    fn fd(&self, pid: u32, fd: u32) -> Option<FdInfo> {
        self.fds(pid).contains(&fd).then(|| FdInfo {
            target: if fd == 5 { b"/tmp/data".to_vec() } else { b"/dev/console".to_vec() },
            pos: u64::from(fd) * 10,
            flags: if fd == 5 { 0o2 | format::O_CLOEXEC } else { 0o2 },
            mnt_id: 1,
            ino: 100 + u64::from(fd),
            read: true,
            write: fd != 0,
        })
    }

    fn mounts(&self, _: u32) -> Vec<MountEntry> {
        std::vec![
            MountEntry {
                id: 1,
                parent: 1,
                dev: (0, 1),
                root: b"/".to_vec(),
                mount_point: b"/".to_vec(),
                source: b"rootfs".to_vec(),
                fstype: "tmpfs".into(),
                ..Default::default()
            },
            MountEntry {
                id: 3,
                parent: 1,
                dev: (0, 3),
                root: b"/".to_vec(),
                mount_point: b"/my disk".to_vec(),
                source: b"virtio2".to_vec(),
                fstype: "ext2".into(),
                read_only: true,
                no_exec: true,
                ..Default::default()
            },
        ]
    }

    fn system(&self) -> SystemInfo {
        SystemInfo {
            uptime_ns: 12_345_678_901,
            idle_ns: 10_000_000_000,
            boot_time: 1_700_000_000,
            mem_total: 512 << 20,
            mem_free: 400 << 20,
            mem_available: 450 << 20,
            cached: 8 << 20,
            loads: [1 << 16, (1 << 16) / 2, 3 * (1 << 16) / 100],
            runnable: 1,
            threads: 12,
            last_pid: 42,
            procs_running: 1,
            cpu: CpuTimes { user: 1_000_000_000, system: 500_000_000, idle: 10_000_000_000, ..Default::default() },
            interrupts: 5000,
            context_switches: 700,
            forks: 41,
            ..Default::default()
        }
    }

    fn boot_time(&self) -> u64 {
        self.system().boot_time
    }

    fn cpus(&self) -> Vec<CpuInfo> {
        std::vec![CpuInfo {
            index: 0,
            vendor: "GenuineIntel".into(),
            model_name: "Test CPU @ 2.00GHz".into(),
            family: 6,
            model: 85,
            stepping: 7,
            cpuid_level: 13,
            khz: 2_000_123,
            apic_id: 0,
            clflush: 64,
            phys_bits: 40,
            virt_bits: 48,
            features: CpuFeatures {
                leaf1_edx: 1 | (1 << 25) | (1 << 26),
                leaf1_ecx: (1 << 19) | (1 << 20) | (1 << 25),
                leaf7_ebx: 1 << 5,
                ext1_edx: 1 << 29,
                ..Default::default()
            },
        }]
    }

    fn filesystems(&self) -> Vec<(String, bool)> {
        std::vec![("tmpfs".into(), true), ("ext2".into(), false), ("proc".into(), true)]
    }

    fn kernel(&self) -> KernelInfo {
        KernelInfo {
            ostype: "Linux".into(),
            osrelease: "6.1.0-poly".into(),
            version: "#1 Poly".into(),
            hostname: b"poly".to_vec(),
            domainname: b"(none)".to_vec(),
        }
    }

    fn random_uuid(&self) -> [u8; 16] {
        [0xff; 16]
    }

    fn boot_id(&self) -> [u8; 16] {
        core::array::from_fn(|i| i as u8)
    }

    fn entropy_avail(&self) -> u32 {
        256
    }

    fn namespace(&self, pid: u32, kind: usize) -> Option<u32> {
        if NS_KINDS[kind] == "pid_for_children" {
            return fake_inum(pid, 3);
        }
        fake_inum(pid, kind)
    }

    fn namespace_exists(&self, inum: u32, kind: usize) -> bool {
        [42, 1].iter().any(|&p| fake_inum(p, kind) == Some(inum))
    }

    fn id_map(&self, pid: u32, file: IdMapFile) -> Vec<u8> {
        match file {
            IdMapFile::Uid => alloc::format!("{:>10} {:>10} {:>10}\n", 0, pid, 1).into_bytes(),
            IdMapFile::Gid => Vec::new(),
            IdMapFile::Setgroups => b"allow\n".to_vec(),
        }
    }

    fn write_id_map(&self, pid: u32, file: IdMapFile, offset: u64, data: &[u8]) -> Result<usize, poly_vfs::Errno> {
        if file != IdMapFile::Uid {
            return Err(poly_vfs::Errno::EPERM);
        }
        self.uid_map.lock().unwrap().push((pid, offset, data.to_vec()));
        Ok(data.len())
    }
}

struct Env {
    fake: Arc<Fake>,
    vfs: Vfs,
    ctx: FsContext,
    cred: Cred,
}

fn env() -> Env {
    let fake = Arc::new(Fake::default());
    *fake.pids.lock().unwrap() = std::vec![42, 1];
    *fake.current.lock().unwrap() = Some(42);
    let vfs = Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))));
    let ctx = vfs.root_context().unwrap();
    let cred = Cred::root();
    for d in ["/proc", "/tmp", "/bin", "/sys"] {
        vfs.mkdir(&ctx, &cred, At::Cwd, d.as_bytes(), 0o755).unwrap();
    }
    let f = vfs.open(&ctx, &cred, At::Cwd, b"/tmp/data", OpenFlags::CREAT | OpenFlags::WRONLY, 0o644).unwrap();
    f.write(b"payload").unwrap();
    vfs.mount(&ctx, &cred, At::Cwd, b"/proc", Arc::new(ProcFs::new(fake.clone())), MountFlags::default()).unwrap();
    Env { fake, vfs, ctx, cred }
}

impl Env {
    fn read(&self, path: &str) -> poly_vfs::Result<Vec<u8>> {
        let f = self.vfs.open(&self.ctx, &self.cred, At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0)?;
        let mut out = Vec::new();
        let mut buf = [0u8; 7]; // small, to cross every boundary
        loop {
            match f.read(&mut buf)? {
                0 => return Ok(out),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    fn text(&self, path: &str) -> String {
        String::from_utf8(self.read(path).unwrap()).unwrap()
    }

    fn list(&self, path: &str) -> Vec<(String, FileType)> {
        let f = self.vfs.open(&self.ctx, &self.cred, At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
        let mut out = Vec::new();
        loop {
            let mut batch = Vec::new();
            // Two entries per call, so positions are used between calls.
            let n = f
                .read_dir(&mut |e| {
                    if batch.len() == 2 {
                        return false;
                    }
                    batch.push((String::from_utf8(e.name.to_vec()).unwrap(), e.kind));
                    true
                })
                .unwrap();
            if n == 0 {
                return out;
            }
            out.append(&mut batch);
        }
    }

    fn readlink(&self, path: &str) -> poly_vfs::Result<Vec<u8>> {
        self.vfs.readlink(&self.ctx, &self.cred, At::Cwd, path.as_bytes())
    }
}

fn names(list: &[(String, FileType)]) -> Vec<&str> {
    list.iter().map(|(n, _)| n.as_str()).collect()
}

#[test]
fn the_root_lists_fixed_files_then_processes_in_pid_order() {
    let e = env();
    let list = e.list("/proc");
    assert_eq!(
        names(&list),
        [
            ".",
            "..",
            "self",
            "mounts",
            "cpuinfo",
            "meminfo",
            "uptime",
            "loadavg",
            "version",
            "stat",
            "filesystems",
            "sys",
            "1",
            "42"
        ]
    );
    assert_eq!(list[2].1, FileType::Symlink);
    assert_eq!(list[11].1, FileType::Directory);
    assert_eq!(list[12].1, FileType::Directory);
}

#[test]
fn positions_survive_processes_coming_and_going() {
    let e = env();
    let f = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc", OpenFlags::RDONLY, 0).unwrap();
    let mut seen = Vec::new();
    // Read up to and including "1".
    f.read_dir(&mut |d| {
        seen.push(String::from_utf8(d.name.to_vec()).unwrap());
        d.name != b"1"
    })
    .unwrap();
    // "1" was refused, so it comes again; meanwhile 1 ends and 7 starts.
    *e.fake.pids.lock().unwrap() = std::vec![7, 42];
    let mut rest = Vec::new();
    f.read_dir(&mut |d| {
        rest.push(String::from_utf8(d.name.to_vec()).unwrap());
        true
    })
    .unwrap();
    assert_eq!(rest, ["7", "42"], "7 sorts after the position reached; 1 is gone");
}

#[test]
fn self_names_the_caller_and_leads_into_its_directory() {
    let e = env();
    assert_eq!(e.readlink("/proc/self").unwrap(), b"42");
    assert_eq!(e.readlink("/proc/mounts").unwrap(), b"self/mounts");
    assert!(e.text("/proc/self/status").contains("\nPid:\t42\n"));
    *e.fake.current.lock().unwrap() = Some(1);
    assert!(e.text("/proc/self/status").contains("\nPid:\t1\n"));
    // A kernel thread has no /proc/self.
    *e.fake.current.lock().unwrap() = None;
    assert_eq!(e.readlink("/proc/self"), Err(poly_vfs::Errno::ENOENT));
    assert_eq!(e.read("/proc/self/status"), Err(poly_vfs::Errno::ENOENT));
}

#[test]
fn process_directories_list_their_files() {
    let e = env();
    assert_eq!(
        names(&e.list("/proc/42")),
        [
            ".",
            "..",
            "stat",
            "statm",
            "status",
            "maps",
            "cmdline",
            "environ",
            "comm",
            "exe",
            "cwd",
            "root",
            "fd",
            "fdinfo",
            "mounts",
            "mountinfo",
            "ns",
            "uid_map",
            "gid_map",
            "setgroups",
            "task"
        ]
    );
    assert_eq!(names(&e.list("/proc/42/fd")), [".", "..", "0", "1", "2", "5"]);
    assert_eq!(names(&e.list("/proc/1/fdinfo")), [".", "..", "0"]);
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/42/status", true).unwrap();
    assert_eq!((st.uid, st.gid, st.mode), (1001, 101, 0o100444));
    assert_eq!(st.size, 0);
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/42", true).unwrap();
    assert_eq!(st.mtime, Timespec::new(1_700_000_002, 500_000_000), "boot time plus the start");
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/42/fd/0", false).unwrap();
    assert_eq!(st.mode, 0o120500 | 0o400, "read-only descriptor: lr-x------");
}

#[test]
fn missing_things_are_enoent() {
    let e = env();
    use poly_vfs::Errno::ENOENT;
    for path in
        ["/proc/7", "/proc/7/status", "/proc/42/nope", "/proc/42/fd/3", "/proc/42/fd/05", "/proc/0", "/proc/042"]
    {
        assert_eq!(e.read(path).err(), Some(ENOENT), "{path}");
    }
    assert_eq!(e.read("/proc/42/fd/x").err(), Some(ENOENT));
    // A directory whose process ended.
    let dir = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc/1", OpenFlags::RDONLY, 0).unwrap();
    *e.fake.pids.lock().unwrap() = std::vec![42];
    assert_eq!(dir.read_dir(&mut |_| true), Err(ENOENT));
}

#[test]
fn links_lead_to_files() {
    let e = env();
    assert_eq!(e.readlink("/proc/42/exe").unwrap(), EXE);
    assert_eq!(e.readlink("/proc/42/cwd").unwrap(), b"/tmp");
    assert_eq!(e.readlink("/proc/42/root").unwrap(), b"/");
    assert_eq!(e.readlink("/proc/42/fd/5").unwrap(), b"/tmp/data");
    // Followed, the link opens the file it names.
    assert_eq!(e.read("/proc/self/fd/5").unwrap(), b"payload");
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/self/cwd", true).unwrap();
    assert_eq!(st.kind(), Some(FileType::Directory));
}

#[test]
fn process_directories_name_their_process() {
    let e = env();
    let dir = |path: &[u8]| {
        let flags = OpenFlags::RDONLY | OpenFlags::DIRECTORY;
        let f = e.vfs.open(&e.ctx, &e.cred, At::Cwd, path, flags, 0).unwrap();
        process_dir(f.node().ino())
    };
    assert_eq!(dir(b"/proc/42"), Some(42));
    assert_eq!(dir(b"/proc/self"), Some(42), "the link leads to the caller's directory");
    assert_eq!(dir(b"/proc/1"), Some(1));
    assert_eq!(dir(b"/proc"), None);
    assert_eq!(dir(b"/proc/42/fd"), None);
    assert_eq!(dir(b"/proc/42/ns"), None);
}

#[test]
fn namespace_links_and_id_maps() {
    let e = env();
    assert_eq!(names(&e.list("/proc/42/ns")), [".", "..", "ipc", "mnt", "pid", "pid_for_children", "user", "uts"]);
    assert_eq!(e.readlink("/proc/42/ns/uts").unwrap(), b"uts:[106]");
    assert_eq!(e.readlink("/proc/42/ns/pid_for_children").unwrap(), b"pid:[103]");
    assert_eq!(e.readlink("/proc/1/ns/net").unwrap(), b"net:[202]");
    assert_eq!(e.readlink("/proc/42/ns/net"), Err(poly_vfs::Errno::ENOENT));
    // Followed, a link reaches the hidden file of its namespace: the same
    // inode from every process in that namespace.
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/42/ns/uts", true).unwrap();
    assert_eq!((st.kind(), st.mode & 0o777), (Some(FileType::Regular), 0o444));
    let f = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc/self/ns/user", OpenFlags::RDONLY, 0).unwrap();
    assert_eq!(namespace_file(f.node().ino()), Some((105, 5)));
    let by_name = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/1/ns/user:[105]", true).unwrap();
    assert_eq!(by_name.ino, f.node().ino());
    let mut buf = [0u8; 8];
    assert_eq!(f.read(&mut buf), Err(poly_vfs::Errno::EINVAL));
    assert_eq!(e.read("/proc/42/ns/net:[999]"), Err(poly_vfs::Errno::ENOENT));
    assert_eq!(e.read("/proc/42/ns/uts:[105]"), Err(poly_vfs::Errno::ENOENT), "wrong type");
    let lst = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/42/ns/uts", false).unwrap();
    assert_eq!(lst.kind(), Some(FileType::Symlink));
    assert_eq!(namespace_file(lst.ino), None);
    // The user namespace files are read through the source, and written.
    assert_eq!(e.text("/proc/42/uid_map"), "         0         42          1\n");
    assert_eq!(e.text("/proc/self/setgroups"), "allow\n");
    let flags = OpenFlags::WRONLY | OpenFlags::TRUNC;
    let w = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc/42/uid_map", flags, 0).unwrap();
    assert_eq!(w.write(b"0 1000 1\n"), Ok(9));
    assert_eq!(*e.fake.uid_map.lock().unwrap(), [(42, 0, b"0 1000 1\n".to_vec())]);
    let w = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc/42/gid_map", OpenFlags::WRONLY, 0).unwrap();
    assert_eq!(w.write(b"0 1000 1\n"), Err(poly_vfs::Errno::EPERM));
    // Other files stay read-only.
    let w = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc/42/comm", OpenFlags::WRONLY, 0);
    assert_eq!(w.err(), Some(poly_vfs::Errno::EROFS));
}

#[test]
fn everything_is_read_only() {
    let e = env();
    use poly_vfs::Errno::EROFS;
    let w = |p: &str| e.vfs.open(&e.ctx, &e.cred, At::Cwd, p.as_bytes(), OpenFlags::WRONLY, 0).err();
    assert_eq!(w("/proc/meminfo"), Some(EROFS));
    assert_eq!(e.vfs.mkdir(&e.ctx, &e.cred, At::Cwd, b"/proc/x", 0o755), Err(EROFS));
    assert_eq!(e.vfs.unlink(&e.ctx, &e.cred, At::Cwd, b"/proc/meminfo"), Err(EROFS));
    let root = e.vfs.lookup(&e.ctx, &e.cred, At::Cwd, b"/proc", true).unwrap();
    let st = root.statfs().unwrap();
    assert_eq!(st.magic, poly_vfs::PROC_SUPER_MAGIC);
    assert_eq!(st.flags & poly_vfs::ST_RDONLY, poly_vfs::ST_RDONLY);
}

#[test]
fn stat_has_52_fields() {
    let e = env();
    let text = e.text("/proc/42/stat");
    assert!(text.starts_with("42 (a-very-long-pro) R 1 42 42 0 -1 "), "{text}");
    assert!(text.ends_with('\n'));
    let after = &text[text.rfind(')').unwrap() + 2..];
    let fields: Vec<&str> = after.split_whitespace().collect();
    assert_eq!(fields.len() + 2, 52, "{text}");
    // Field n is fields[n - 3].
    let f = |n: usize| fields[n - 3];
    assert_eq!((f(14), f(15)), ("7", "3"), "utime and stime in ticks");
    assert_eq!(f(18), "20");
    assert_eq!(f(20), "3", "threads");
    assert_eq!(f(22), "250", "start time in ticks");
    assert_eq!(f(23), (0x52_0000 - 0x50_0000 + 0x4000 + 0x2000 + 0x21000).to_string(), "vsize");
    assert_eq!(f(24), "10");
    assert_eq!((f(26), f(27)), ("4198400", "4206592"), "the program's executable mapping");
    assert_eq!(f(31), (1 << 9).to_string());
    assert_eq!(f(38), "17");
    assert_eq!((f(45), f(46)), ("4206592", "4210688"), "the program's data");
    assert_eq!(f(47), (0x50_0000).to_string());
    assert_eq!(f(52), "0");
}

#[test]
fn status_reports_the_process() {
    let e = env();
    let text = e.text("/proc/42/status");
    for line in [
        "Name:\ta-very-long-pro\n",
        "Umask:\t0022\n",
        "State:\tR (running)\n",
        "Tgid:\t42\n",
        "PPid:\t1\n",
        "Uid:\t1000\t1001\t1002\t1003\n",
        "Gid:\t100\t101\t102\t103\n",
        "FDSize:\t64\n",
        "VmRSS:\t      40 kB\n",
        "RssFile:\t      16 kB\n",
        "VmStk:\t     132 kB\n",
        "VmExe:\t       8 kB\n",
        "VmLib:\t       8 kB\n",
        "Threads:\t3\n",
        "SigQ:\t1/4096\n",
        "ShdPnd:\t0000000000000200\n",
        "SigBlk:\t0000000000000002\n",
        "SigIgn:\t0000000000001000\n",
        "SigCgt:\t0000000000010000\n",
        "CapEff:\t0000000000000000\n",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    // VmData: the private writable data mapping and the heap.
    assert!(text.contains(&std::format!("VmData:\t{:>8} kB\n", (0x1000 + 0x20000) / 1024)), "{text}");
    assert_eq!(e.text("/proc/1/status").lines().next(), Some("Name:\ta-very-long-pro"));
    assert_eq!(
        e.text("/proc/42/statm"),
        std::format!(
            "{} 10 4 2 0 {} 0\n",
            (0x4000 + 0x20000 + 0x2000 + 0x21000) / 4096,
            (0x1000 + 0x20000 + 0x21000) / 4096
        )
    );
}

#[test]
fn maps_lines_are_formatted_per_proc_pid_maps() {
    let e = env();
    let text = e.text("/proc/42/maps");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 6);
    assert!(lines[0].starts_with("00400000-00401000 r--p 00000000 00:01 77 "));
    assert!(lines[0].ends_with(" /bin/prog"));
    assert_eq!(lines[0].find("/bin/prog"), Some(format::MAPS_PATH_COLUMN));
    assert!(lines[1].starts_with("00401000-00403000 r-xp 00001000 00:01 77 "));
    assert!(lines[2].starts_with("00403000-00404000 rw-p 00003000 "));
    let heap = "00500000-00520000 rw-p 00000000 00:00 0";
    assert_eq!(lines[3], std::format!("{heap}{}[heap]", " ".repeat(format::MAPS_PATH_COLUMN - heap.len())));
    assert!(lines[4].starts_with("7f0000000000-7f0000002000 r-xp 00000000 08:02 5 "));
    assert!(lines[5].starts_with("7ffffffde000-7ffffffff000 rw-p 00000000 00:00 0 ") && lines[5].ends_with("[stack]"));
    // Anonymous memory has no name; a newline in a path is escaped.
    let anon = MapEntry { start: 0x1000, end: 0x2000, read: true, ..Default::default() };
    assert_eq!(format::maps(&[anon]), b"00001000-00002000 r--p 00000000 00:00 0\n");
    let odd = MapEntry { name: MapName::Path(b"/a\nb".to_vec()), end: 0x1000, ..Default::default() };
    assert!(std::str::from_utf8(&format::maps(&[odd])).unwrap().ends_with(" /a\\012b\n"));
}

#[test]
fn cmdline_environ_and_comm() {
    let e = env();
    assert_eq!(e.read("/proc/42/cmdline").unwrap(), b"/bin/prog\0-x\0");
    assert_eq!(e.read("/proc/42/environ").unwrap(), b"HOME=/\0");
    assert_eq!(e.text("/proc/42/comm"), "a-very-long-pro\n");
}

#[test]
fn fdinfo_shows_position_flags_mount_and_inode() {
    let e = env();
    assert_eq!(e.text("/proc/42/fdinfo/5"), "pos:\t50\nflags:\t02000002\nmnt_id:\t1\nino:\t105\n");
    assert_eq!(e.text("/proc/42/fdinfo/0"), "pos:\t0\nflags:\t02\nmnt_id:\t1\nino:\t100\n");
}

#[test]
fn mounts_and_mountinfo() {
    let e = env();
    assert_eq!(e.text("/proc/self/mounts"), "rootfs / tmpfs rw 0 0\nvirtio2 /my\\040disk ext2 ro,noexec 0 0\n");
    assert_eq!(e.text("/proc/mounts"), e.text("/proc/42/mounts"));
    assert_eq!(
        e.text("/proc/42/mountinfo"),
        "1 1 0:1 / / rw - tmpfs rootfs rw\n3 1 0:3 / /my\\040disk ro,noexec - ext2 virtio2 ro\n"
    );
}

#[test]
fn system_files() {
    let e = env();
    assert_eq!(e.text("/proc/uptime"), "12.34 10.00\n");
    assert_eq!(e.text("/proc/loadavg"), "1.00 0.50 0.03 1/12 42\n");
    assert_eq!(e.text("/proc/version"), "Linux version 6.1.0-poly (poly@poly) #1 Poly\n");
    assert_eq!(e.text("/proc/filesystems"), "nodev\ttmpfs\n\text2\nnodev\tproc\n");
    let stat = e.text("/proc/stat");
    assert!(stat.starts_with("cpu 100 0 50 1000 0 0 0 0 0 0\ncpu0 100 0 50 1000 0 0 0 0 0 0\n"), "{stat}");
    for l in ["intr 5000\n", "ctxt 700\n", "btime 1700000000\n", "processes 41\n", "procs_running 1\n"] {
        assert!(stat.contains(l), "{l}");
    }
    let mem = e.text("/proc/meminfo");
    assert!(
        mem.starts_with("MemTotal:         524288 kB\nMemFree:          409600 kB\nMemAvailable:     460800 kB\n"),
        "{mem}"
    );
    assert!(mem.contains("\nSwapTotal:             0 kB\n"));
    assert!(mem.lines().all(|l| l.ends_with(" kB") && l.len() == 27), "{mem}");
}

#[test]
fn cpuinfo_names_the_cpu_and_its_flags() {
    let e = env();
    let text = e.text("/proc/cpuinfo");
    for line in [
        "processor\t: 0\n",
        "vendor_id\t: GenuineIntel\n",
        "cpu family\t: 6\n",
        "model\t\t: 85\n",
        "model name\t: Test CPU @ 2.00GHz\n",
        "stepping\t: 7\n",
        "cpu MHz\t\t: 2000.123\n",
        "flags\t\t: fpu sse sse2 lm sse4_1 sse4_2 aes avx2\n",
        "bogomips\t: 4000.24\n",
        "address sizes\t: 40 bits physical, 48 bits virtual\n",
    ] {
        assert!(text.contains(line), "{line:?} in\n{text}");
    }
    assert!(text.ends_with("\n\n"));
}

#[test]
fn sys_kernel_files() {
    let e = env();
    assert_eq!(e.text("/proc/sys/kernel/ostype"), "Linux\n");
    assert_eq!(e.text("/proc/sys/kernel/osrelease"), "6.1.0-poly\n");
    assert_eq!(e.text("/proc/sys/kernel/version"), "#1 Poly\n");
    assert_eq!(e.text("/proc/sys/kernel/hostname"), "poly\n");
    assert_eq!(e.text("/proc/sys/kernel/domainname"), "(none)\n");
    assert_eq!(e.text("/proc/sys/kernel/pid_max"), "32768\n");
    assert_eq!(e.text("/proc/sys/fs/nr_open"), "1048576\n");
    assert_eq!(e.text("/proc/sys/kernel/random/poolsize"), "4096\n");
    assert_eq!(e.text("/proc/sys/kernel/random/entropy_avail"), "256\n");
    assert_eq!(e.text("/proc/sys/kernel/random/boot_id"), "00010203-0405-4607-8809-0a0b0c0d0e0f\n");
    assert_eq!(e.text("/proc/sys/kernel/random/uuid"), "ffffffff-ffff-4fff-bfff-ffffffffffff\n");
    assert_eq!(names(&e.list("/proc/sys")), [".", "..", "kernel", "fs"]);
    // `..` climbs back to /proc and then out of the mount.
    assert_eq!(e.text("/proc/sys/kernel/../../uptime"), "12.34 10.00\n");
    let mut ctx = e.ctx.clone();
    e.vfs.chdir(&mut ctx, &e.cred, At::Cwd, b"/proc/sys/kernel/random").unwrap();
    assert_eq!(e.vfs.getcwd(&ctx).unwrap(), b"/proc/sys/kernel/random");
    e.vfs.chdir(&mut ctx, &e.cred, At::Cwd, b"/proc/42/fd").unwrap();
    assert_eq!(e.vfs.getcwd(&ctx).unwrap(), b"/proc/42/fd");
}

#[test]
fn reads_at_offsets() {
    let e = env();
    let f = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/proc/version", OpenFlags::RDONLY, 0).unwrap();
    let mut buf = [0u8; 7];
    assert_eq!(f.read_at(6, &mut buf), Ok(7));
    assert_eq!(&buf, b"version");
    assert_eq!(f.read_at(10_000, &mut buf), Ok(0));
    assert_eq!(f.seek(0, poly_vfs::Whence::End), Ok(0), "size 0: SEEK_END is the start");
}

#[test]
fn inode_numbers_are_distinct() {
    let e = env();
    let mut seen = std::collections::BTreeSet::new();
    for dir in ["/proc", "/proc/42", "/proc/1", "/proc/42/fd", "/proc/42/fdinfo", "/proc/sys/kernel/random"] {
        let f = e.vfs.open(&e.ctx, &e.cred, At::Cwd, dir.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
        f.read_dir(&mut |d| {
            if d.name != b"." && d.name != b".." {
                assert!(seen.insert(d.ino), "{dir}/{}", String::from_utf8_lossy(d.name));
                let path = std::format!("{dir}/{}", String::from_utf8_lossy(d.name));
                let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, path.as_bytes(), false).unwrap();
                assert_eq!(st.ino, d.ino, "{path}");
            }
            true
        })
        .unwrap();
    }
}

#[test]
fn uuid_sets_version_and_variant() {
    assert_eq!(format::uuid([0; 16]), b"00000000-0000-4000-8000-000000000000\n");
}

#[test]
fn flag_names_follow_the_register_order() {
    let f = CpuFeatures { leaf1_ecx: 1 | (1 << 30), leaf7_ebx: 1 << 18, ext1_ecx: 1, ..Default::default() };
    assert_eq!(cpuflags::flag_names(&f), ["sse3", "rdrand", "lahf_lm", "rdseed"]);
    assert!(cpuflags::flag_names(&CpuFeatures::default()).is_empty());
}

#[test]
fn a_static_tree_serves_sysfs() {
    let mut b = StaticFsBuilder::new("sysfs", poly_vfs::SYSFS_MAGIC, Timespec::new(5, 0));
    b.text("devices/system/cpu/online", "0\n")
        .text("devices/system/cpu/possible", "0\n")
        .text("devices/virtual/mem/null/dev", "1:3\n")
        .link("dev/char/1:3", "../../devices/virtual/mem/null")
        .file("kernel/counter", {
            let n = std::sync::atomic::AtomicU32::new(0);
            move || std::format!("{}\n", n.fetch_add(1, std::sync::atomic::Ordering::Relaxed)).into_bytes()
        });
    b.dir("block");
    let fs = b.build();
    let e = env();
    e.vfs.mount(&e.ctx, &e.cred, At::Cwd, b"/sys", fs, MountFlags::default()).unwrap();
    assert_eq!(e.text("/sys/devices/system/cpu/online"), "0\n");
    assert_eq!(e.readlink("/sys/dev/char/1:3").unwrap(), b"../../devices/virtual/mem/null");
    assert_eq!(e.text("/sys/dev/char/1:3/dev"), "1:3\n");
    // Every read call makes the contents again: the second call (which
    // finds the end) counted too.
    assert_eq!(e.text("/sys/kernel/counter"), "0\n");
    assert_eq!(e.text("/sys/kernel/counter"), "2\n");
    assert_eq!(names(&e.list("/sys")), [".", "..", "devices", "dev", "kernel", "block"]);
    assert_eq!(names(&e.list("/sys/devices/system/cpu")), [".", "..", "online", "possible"]);
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/sys/devices/system/cpu/online", true).unwrap();
    assert_eq!((st.mode, st.size), (0o100444, 4096));
    let w = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/sys/devices/system/cpu/online", OpenFlags::WRONLY, 0);
    assert_eq!(w.err(), Some(poly_vfs::Errno::EROFS));
    let root = e.vfs.lookup(&e.ctx, &e.cred, At::Cwd, b"/sys", true).unwrap();
    assert_eq!(root.statfs().unwrap().magic, poly_vfs::SYSFS_MAGIC);
    let mut ctx = e.ctx.clone();
    e.vfs.chdir(&mut ctx, &e.cred, At::Cwd, b"/sys/devices/virtual/mem").unwrap();
    assert_eq!(e.vfs.getcwd(&ctx).unwrap(), b"/sys/devices/virtual/mem");
}

#[test]
fn task_lists_threads_with_their_own_names() {
    let e = env();
    assert_eq!(names(&e.list("/proc/42/task")), [".", "..", "42", "43"]);
    assert_eq!(names(&e.list("/proc/42/task/43")), [".", "..", "comm"]);
    assert_eq!(e.text("/proc/42/task/43/comm"), "worker\n");
    assert_eq!(e.text("/proc/42/task/42/comm"), "a-very-long-pro\n", "the process's comm, 15 bytes");
    assert_eq!(e.read("/proc/42/task/44/comm"), Err(poly_vfs::Errno::ENOENT));
    assert_eq!(e.read("/proc/1/task/43/comm"), Err(poly_vfs::Errno::ENOENT));
    let st = e.vfs.stat(&e.ctx, &e.cred, At::Cwd, b"/proc/42/task/43", true).unwrap();
    assert_eq!(st.mode, 0o040555);
}
