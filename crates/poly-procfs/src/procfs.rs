//! The `/proc` file system: a fixed tree of system files, one directory
//! per process, and per-process descriptor directories, with contents made
//! from a [`ProcSource`] on every read.
//!
//! # Inode numbers
//!
//! An inode number says what a node is, so nothing is stored per node:
//! bits 32-63 are the pid (0 outside a process directory), bits 24-31 the
//! kind of node, bits 0-23 an index (into the fixed tables, or a
//! descriptor number).
//!
//! # Directory positions
//!
//! `.` is 0 and `..` is 1; fixed entries follow at 2, 3, ...; the process
//! directories of the root are at [`PID_POS`] + pid and descriptors at 2 +
//! fd, so positions stay valid while processes come and go and descriptors
//! are opened and closed.

use crate::format;
use crate::source::{Owner, ProcSource, ProcessInfo};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{
    Attr, DeviceId, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, PROC_SUPER_MAGIC,
    RenameFlags, Result, SetAttr, Timespec,
};

/// Where the process directories start among the root's positions.
pub const PID_POS: u64 = 1 << 32;

/// The value of `/proc/sys/kernel/pid_max` (proc_sys_kernel(5): "The
/// default value for this file, 32768").
pub const PID_MAX: u32 = 32768;

/// The value of `/proc/sys/fs/nr_open`, the ceiling of `RLIMIT_NOFILE`
/// (proc_sys_fs(5): 1048576 by default).
pub const NR_OPEN: u64 = 1 << 20;

/// What a file in the fixed tree holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum G {
    Dir,
    SelfLink,
    MountsLink,
    Cpuinfo,
    Meminfo,
    Uptime,
    Loadavg,
    Version,
    Stat,
    Filesystems,
    Ostype,
    Osrelease,
    KernelVersion,
    Hostname,
    Domainname,
    PidMax,
    BootId,
    Uuid,
    EntropyAvail,
    Poolsize,
    NrOpen,
}

/// No parent: in the root.
const TOP: usize = usize::MAX;

/// The fixed tree: name, parent (an index here, or [`TOP`]), contents.
const GLOBALS: &[(&[u8], usize, G)] = &[
    (b"self", TOP, G::SelfLink),             // 0
    (b"mounts", TOP, G::MountsLink),         // 1
    (b"cpuinfo", TOP, G::Cpuinfo),           // 2
    (b"meminfo", TOP, G::Meminfo),           // 3
    (b"uptime", TOP, G::Uptime),             // 4
    (b"loadavg", TOP, G::Loadavg),           // 5
    (b"version", TOP, G::Version),           // 6
    (b"stat", TOP, G::Stat),                 // 7
    (b"filesystems", TOP, G::Filesystems),   // 8
    (b"sys", TOP, G::Dir),                   // 9
    (b"kernel", 9, G::Dir),                  // 10
    (b"ostype", 10, G::Ostype),              // 11
    (b"osrelease", 10, G::Osrelease),        // 12
    (b"version", 10, G::KernelVersion),      // 13
    (b"hostname", 10, G::Hostname),          // 14
    (b"domainname", 10, G::Domainname),      // 15
    (b"pid_max", 10, G::PidMax),             // 16
    (b"random", 10, G::Dir),                 // 17
    (b"boot_id", 17, G::BootId),             // 18
    (b"uuid", 17, G::Uuid),                  // 19
    (b"entropy_avail", 17, G::EntropyAvail), // 20
    (b"poolsize", 17, G::Poolsize),          // 21
    (b"fs", 9, G::Dir),                      // 22
    (b"nr_open", 22, G::NrOpen),             // 23
];

/// What a file in a process directory holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum E {
    Stat,
    Statm,
    Status,
    Maps,
    Cmdline,
    Environ,
    Comm,
    Exe,
    Cwd,
    Root,
    FdDir,
    FdinfoDir,
    Mounts,
    Mountinfo,
}

/// A process directory: name, contents, permission bits.
const PID_ENTRIES: &[(&[u8], E, u16)] = &[
    (b"stat", E::Stat, 0o444),
    (b"statm", E::Statm, 0o444),
    (b"status", E::Status, 0o444),
    (b"maps", E::Maps, 0o444),
    (b"cmdline", E::Cmdline, 0o444),
    (b"environ", E::Environ, 0o400),
    (b"comm", E::Comm, 0o644),
    (b"exe", E::Exe, 0o777),
    (b"cwd", E::Cwd, 0o777),
    (b"root", E::Root, 0o777),
    (b"fd", E::FdDir, 0o500),
    (b"fdinfo", E::FdinfoDir, 0o555),
    (b"mounts", E::Mounts, 0o444),
    (b"mountinfo", E::Mountinfo, 0o444),
];

/// A node of the tree, decoded from its inode number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Root,
    Global(usize),
    Pid(u32),
    Entry(u32, usize),
    Fd(u32, u32),
    FdInfo(u32, u32),
}

const K_ROOT: u64 = 1;
const K_GLOBAL: u64 = 2;
const K_PID: u64 = 3;
const K_ENTRY: u64 = 4;
const K_FD: u64 = 5;
const K_FDINFO: u64 = 6;
const INDEX_MASK: u64 = (1 << 24) - 1;

impl Node {
    fn ino(self) -> Ino {
        let (pid, kind, index) = match self {
            Node::Root => (0, K_ROOT, 0),
            Node::Global(i) => (0, K_GLOBAL, i as u64),
            Node::Pid(p) => (p, K_PID, 0),
            Node::Entry(p, i) => (p, K_ENTRY, i as u64),
            Node::Fd(p, fd) => (p, K_FD, u64::from(fd)),
            Node::FdInfo(p, fd) => (p, K_FDINFO, u64::from(fd)),
        };
        (u64::from(pid) << 32) | (kind << 24) | (index & INDEX_MASK)
    }

    fn from_ino(ino: Ino) -> Option<Node> {
        let pid = (ino >> 32) as u32;
        let index = ino & INDEX_MASK;
        Some(match (ino >> 24) & 0xff {
            K_ROOT if pid == 0 && index == 0 => Node::Root,
            K_GLOBAL if pid == 0 && (index as usize) < GLOBALS.len() => Node::Global(index as usize),
            K_PID if pid != 0 && index == 0 => Node::Pid(pid),
            K_ENTRY if pid != 0 && (index as usize) < PID_ENTRIES.len() => Node::Entry(pid, index as usize),
            K_FD if pid != 0 => Node::Fd(pid, index as u32),
            K_FDINFO if pid != 0 => Node::FdInfo(pid, index as u32),
            _ => return None,
        })
    }
}

/// The largest descriptor number an inode number can carry.
const MAX_FD: u32 = INDEX_MASK as u32;

/// procfs over a [`ProcSource`].
pub struct ProcFs {
    src: Arc<dyn ProcSource>,
}

impl core::fmt::Debug for ProcFs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProcFs").finish_non_exhaustive()
    }
}

/// A decimal number as bytes.
fn decimal(n: u64) -> Vec<u8> {
    alloc::format!("{n}").into_bytes()
}

/// A process id or descriptor number written as a name: decimal digits
/// without leading zeros (so every number has exactly one name).
fn parse_number(name: &[u8]) -> Option<u32> {
    if name.is_empty() || name.len() > 10 || (name[0] == b'0' && name.len() > 1) {
        return None;
    }
    core::str::from_utf8(name).ok()?.parse().ok()
}

/// Copies the bytes of `data` from `offset` into `buf`; how many.
pub fn copy_at(data: &[u8], offset: u64, buf: &mut [u8]) -> usize {
    let Ok(start) = usize::try_from(offset) else { return 0 };
    if start >= data.len() {
        return 0;
    }
    let n = buf.len().min(data.len() - start);
    buf[..n].copy_from_slice(&data[start..start + n]);
    n
}

impl ProcFs {
    pub fn new(src: Arc<dyn ProcSource>) -> ProcFs {
        ProcFs { src }
    }

    fn node(&self, ino: Ino) -> Result<Node> {
        Node::from_ino(ino).ok_or(Errno::ENOENT)
    }

    fn process(&self, pid: u32) -> Result<ProcessInfo> {
        self.src.process(pid).ok_or(Errno::ENOENT)
    }

    fn owner(&self, pid: u32) -> Result<Owner> {
        self.src.owner(pid).ok_or(Errno::ENOENT)
    }

    fn has_fd(&self, pid: u32, fd: u32) -> bool {
        self.src.fds(pid).contains(&fd)
    }

    fn kind(node: Node) -> FileType {
        match node {
            Node::Root | Node::Pid(_) => FileType::Directory,
            Node::Global(i) => match GLOBALS[i].2 {
                G::Dir => FileType::Directory,
                G::SelfLink | G::MountsLink => FileType::Symlink,
                _ => FileType::Regular,
            },
            Node::Entry(_, i) => match PID_ENTRIES[i].1 {
                E::FdDir | E::FdinfoDir => FileType::Directory,
                E::Exe | E::Cwd | E::Root => FileType::Symlink,
                _ => FileType::Regular,
            },
            Node::Fd(..) => FileType::Symlink,
            Node::FdInfo(..) => FileType::Regular,
        }
    }

    /// The node's parent directory.
    fn parent(node: Node) -> Node {
        match node {
            Node::Root | Node::Pid(_) => Node::Root,
            Node::Global(i) => match GLOBALS[i].1 {
                TOP => Node::Root,
                p => Node::Global(p),
            },
            Node::Entry(p, _) => Node::Pid(p),
            Node::Fd(p, _) => Node::Entry(p, entry_index(E::FdDir)),
            Node::FdInfo(p, _) => Node::Entry(p, entry_index(E::FdinfoDir)),
        }
    }

    /// The children of a directory node in listing order, from the
    /// position `pos` on, each with its position.
    fn children(&self, dir: Node, pos: u64) -> Result<Vec<(u64, Vec<u8>, Node)>> {
        let mut out = Vec::new();
        let fixed = |out: &mut Vec<(u64, Vec<u8>, Node)>, list: &mut dyn Iterator<Item = (Vec<u8>, Node)>| {
            for (i, (name, node)) in list.enumerate() {
                let at = 2 + i as u64;
                if at >= pos {
                    out.push((at, name, node));
                }
            }
        };
        match dir {
            Node::Root => {
                fixed(
                    &mut out,
                    &mut GLOBALS
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| g.1 == TOP)
                        .map(|(i, g)| (g.0.to_vec(), Node::Global(i))),
                );
                let mut pids = self.src.pids();
                pids.sort_unstable();
                pids.dedup();
                for pid in pids {
                    let at = PID_POS + u64::from(pid);
                    if pid != 0 && at >= pos {
                        out.push((at, decimal(u64::from(pid)), Node::Pid(pid)));
                    }
                }
            }
            Node::Global(d) => fixed(
                &mut out,
                &mut GLOBALS.iter().enumerate().filter(|(_, g)| g.1 == d).map(|(i, g)| (g.0.to_vec(), Node::Global(i))),
            ),
            Node::Pid(p) => {
                self.owner(p)?;
                fixed(&mut out, &mut PID_ENTRIES.iter().enumerate().map(|(i, e)| (e.0.to_vec(), Node::Entry(p, i))))
            }
            Node::Entry(p, i) if matches!(PID_ENTRIES[i].1, E::FdDir | E::FdinfoDir) => {
                self.owner(p)?;
                let mut fds = self.src.fds(p);
                fds.sort_unstable();
                fds.dedup();
                let info = PID_ENTRIES[i].1 == E::FdinfoDir;
                for fd in fds.into_iter().filter(|&fd| fd <= MAX_FD) {
                    let at = 2 + u64::from(fd);
                    if at >= pos {
                        let node = if info { Node::FdInfo(p, fd) } else { Node::Fd(p, fd) };
                        out.push((at, decimal(u64::from(fd)), node));
                    }
                }
            }
            _ => return Err(Errno::ENOTDIR),
        }
        Ok(out)
    }

    /// Whether `node` exists now (its process and descriptor too).
    fn exists(&self, node: Node) -> bool {
        match node {
            Node::Root | Node::Global(_) => true,
            Node::Pid(p) | Node::Entry(p, _) => self.src.owner(p).is_some(),
            Node::Fd(p, fd) | Node::FdInfo(p, fd) => self.has_fd(p, fd),
        }
    }

    /// The contents of a regular file.
    fn contents(&self, node: Node) -> Result<Vec<u8>> {
        let src = &*self.src;
        Ok(match node {
            Node::Global(i) => match GLOBALS[i].2 {
                G::Cpuinfo => format::cpuinfo(&src.cpus()),
                G::Meminfo => format::meminfo(&src.system()),
                G::Uptime => format::uptime(&src.system()),
                G::Loadavg => format::loadavg(&src.system()),
                G::Version => format::version(&src.kernel()),
                G::Stat => format::system_stat(&src.system(), src.cpus().len()),
                G::Filesystems => format::filesystems(&src.filesystems()),
                G::Ostype => format::line(src.kernel().ostype.as_bytes()),
                G::Osrelease => format::line(src.kernel().osrelease.as_bytes()),
                G::KernelVersion => format::line(src.kernel().version.as_bytes()),
                G::Hostname => format::line(&src.kernel().hostname),
                G::Domainname => format::line(&src.kernel().domainname),
                G::PidMax => format::line(&decimal(u64::from(PID_MAX))),
                G::BootId => format::uuid(src.boot_id()),
                G::Uuid => format::uuid(src.random_uuid()),
                G::EntropyAvail => format::line(&decimal(u64::from(src.entropy_avail()))),
                // random(4): "Linux 2.6 and later: ... It contains the
                // value 4096."
                G::Poolsize => format::line(b"4096"),
                G::NrOpen => format::line(&decimal(NR_OPEN)),
                G::Dir | G::SelfLink | G::MountsLink => return Err(Errno::EINVAL),
            },
            Node::Entry(p, i) => {
                let info = self.process(p)?;
                match PID_ENTRIES[i].1 {
                    E::Stat => format::stat(&info, &src.maps(p), &src.system()),
                    E::Statm => format::statm(&info, &src.maps(p)),
                    E::Status => format::status(&info, &src.maps(p)),
                    E::Maps => format::maps(&src.maps(p)),
                    E::Cmdline => src.cmdline(p),
                    E::Environ => src.environ(p),
                    E::Comm => format::line(format::comm(&info.comm)),
                    E::Mounts => format::mounts(&src.mounts(p)),
                    E::Mountinfo => format::mountinfo(&src.mounts(p)),
                    E::Exe | E::Cwd | E::Root | E::FdDir | E::FdinfoDir => return Err(Errno::EINVAL),
                }
            }
            Node::FdInfo(p, fd) => format::fdinfo(&src.fd(p, fd).ok_or(Errno::ENOENT)?),
            Node::Root | Node::Pid(_) => return Err(Errno::EISDIR),
            Node::Fd(..) => return Err(Errno::EINVAL),
        })
    }
}

fn entry_index(e: E) -> usize {
    PID_ENTRIES.iter().position(|x| x.1 == e).expect("every kind has an entry")
}

impl FileSystem for ProcFs {
    fn info(&self) -> FsInfo {
        // Entries come and go on their own: no lookup cache.
        FsInfo { name: "proc", read_only: true, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        Node::Root.ino()
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        let dir = self.node(dir)?;
        if Self::kind(dir) != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        if !self.exists(dir) {
            return Err(Errno::ENOENT);
        }
        if name == b".." {
            return Ok(Self::parent(dir).ino());
        }
        let found = match dir {
            Node::Root => match GLOBALS.iter().position(|g| g.1 == TOP && g.0 == name) {
                Some(i) => Some(Node::Global(i)),
                None => parse_number(name).filter(|&p| p != 0 && self.src.owner(p).is_some()).map(Node::Pid),
            },
            Node::Global(d) => GLOBALS.iter().position(|g| g.1 == d && g.0 == name).map(Node::Global),
            Node::Pid(p) => PID_ENTRIES.iter().position(|e| e.0 == name).map(|i| Node::Entry(p, i)),
            Node::Entry(p, i) => parse_number(name)
                .filter(|&fd| fd <= MAX_FD && self.has_fd(p, fd))
                .map(|fd| if PID_ENTRIES[i].1 == E::FdinfoDir { Node::FdInfo(p, fd) } else { Node::Fd(p, fd) }),
            _ => None,
        };
        found.map(Node::ino).ok_or(Errno::ENOENT)
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        let node = self.node(ino)?;
        let boot_time = self.src.boot_time();
        let boot = Timespec::new(boot_time as i64, 0);
        let (uid, gid, time) = match node {
            Node::Root | Node::Global(_) => (0, 0, boot),
            Node::Pid(p) | Node::Entry(p, _) | Node::Fd(p, _) | Node::FdInfo(p, _) => {
                let info = self.owner(p)?;
                let start = info.start_ns;
                let time =
                    Timespec::new(boot_time as i64 + (start / 1_000_000_000) as i64, (start % 1_000_000_000) as u32);
                (info.uid, info.gid, time)
            }
        };
        if !self.exists(node) {
            return Err(Errno::ENOENT);
        }
        let kind = Self::kind(node);
        let perm = match node {
            Node::Root | Node::Pid(_) => 0o555,
            Node::Global(i) => match GLOBALS[i].2 {
                G::Dir => 0o555,
                G::SelfLink | G::MountsLink => 0o777,
                _ => 0o444,
            },
            Node::Entry(_, i) => PID_ENTRIES[i].2,
            Node::Fd(p, fd) => match self.src.fd(p, fd) {
                Some(f) => 0o500 | if f.read { 0o400 } else { 0 } | if f.write { 0o200 } else { 0 },
                None => 0o700,
            },
            Node::FdInfo(..) => 0o444,
        };
        Ok(Attr {
            ino,
            kind,
            perm,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid,
            gid,
            // proc(5) files have no size: readers read to the end.
            size: 0,
            rdev: DeviceId::default(),
            atime: time,
            mtime: time,
            ctime: time,
            blocks: 0,
            blksize: 1024,
        })
    }

    fn setattr(&self, _: Ino, _: &SetAttr) -> Result<Attr> {
        Err(Errno::EROFS)
    }

    fn create(&self, _: Ino, _: &[u8], _: &NewNode) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn mkdir(&self, _: Ino, _: &[u8], _: u16, _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn symlink(&self, _: Ino, _: &[u8], _: &[u8], _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn link(&self, _: Ino, _: &[u8], _: Ino) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn unlink(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn rmdir(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn rename(&self, _: Ino, _: &[u8], _: Ino, _: &[u8], _: RenameFlags) -> Result<()> {
        Err(Errno::EROFS)
    }

    /// `/proc/self` is the caller's pid (proc_self(5)); `ENOENT` for a
    /// kernel thread, which has no process. `/proc/mounts` is
    /// `self/mounts` (proc_pid_mounts(5)). A process's `exe`, `cwd`,
    /// `root` and `fd/<n>` are what the kernel names; following them
    /// resolves that text like any link, so (unlike Linux's "magic"
    /// links) a deleted file or a pipe cannot be reopened through them.
    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        let node = self.node(ino)?;
        let src = &*self.src;
        match node {
            Node::Global(i) if GLOBALS[i].2 == G::SelfLink => {
                src.current_pid().map(|p| decimal(u64::from(p))).ok_or(Errno::ENOENT)
            }
            Node::Global(i) if GLOBALS[i].2 == G::MountsLink => Ok(b"self/mounts".to_vec()),
            Node::Entry(p, i) => {
                self.owner(p)?;
                match PID_ENTRIES[i].1 {
                    E::Exe => src.exe(p),
                    E::Cwd => src.cwd(p),
                    E::Root => src.root(p),
                    _ => return Err(Errno::EINVAL),
                }
                .ok_or(Errno::ENOENT)
            }
            Node::Fd(p, fd) => src.fd(p, fd).map(|f| f.target).ok_or(Errno::ENOENT),
            _ => Err(Errno::EINVAL),
        }
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let node = self.node(ino)?;
        if Self::kind(node) == FileType::Directory {
            return Err(Errno::EISDIR);
        }
        Ok(copy_at(&self.contents(node)?, offset, buf))
    }

    fn write(&self, _: Ino, _: u64, _: &[u8]) -> Result<usize> {
        Err(Errno::EROFS)
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        let node = self.node(dir)?;
        if Self::kind(node) != FileType::Directory {
            return Err(Errno::ENOTDIR);
        }
        if !self.exists(node) {
            return Err(Errno::ENOENT);
        }
        let dots = [(0, &b"."[..], node), (1, &b".."[..], Self::parent(node))];
        for (at, name, n) in dots {
            if at >= pos && !emit(&DirEntry { ino: n.ino(), kind: FileType::Directory, name, next: at + 1 }) {
                return Ok(());
            }
        }
        let children = self.children(node, pos)?;
        for (at, name, n) in &children {
            if !emit(&DirEntry { ino: n.ino(), kind: Self::kind(*n), name, next: at + 1 }) {
                break;
            }
        }
        Ok(())
    }

    fn statfs(&self) -> Result<FsStats> {
        Ok(FsStats {
            magic: PROC_SUPER_MAGIC,
            block_size: 4096,
            fragment_size: 4096,
            name_max: 255,
            ..FsStats::default()
        })
    }
}
