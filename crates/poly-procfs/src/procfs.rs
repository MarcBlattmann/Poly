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
use crate::source::{IdMapFile, IpcFile, KernelName, MagicLink, NetFile, Owner, ProcSource, ProcessInfo};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::lock::{Lock, RawSpinLock};
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
    CapLastCap,
    InotifyWatches,
    InotifyInstances,
    InotifyQueued,
    ThreadSelfLink,
    NsLastPid,
    /// `/proc/net`, a link to `self/net`.
    NetLink,
    /// A file of the reader's IPC namespace.
    Ipc(IpcFile),
}

/// No parent: in the root.
const TOP: usize = usize::MAX;

/// The fixed tree: name, parent (an index here, or [`TOP`]), contents.
const GLOBALS: &[(&[u8], usize, G)] = &[
    (b"self", TOP, G::SelfLink),                                 // 0
    (b"mounts", TOP, G::MountsLink),                             // 1
    (b"cpuinfo", TOP, G::Cpuinfo),                               // 2
    (b"meminfo", TOP, G::Meminfo),                               // 3
    (b"uptime", TOP, G::Uptime),                                 // 4
    (b"loadavg", TOP, G::Loadavg),                               // 5
    (b"version", TOP, G::Version),                               // 6
    (b"stat", TOP, G::Stat),                                     // 7
    (b"filesystems", TOP, G::Filesystems),                       // 8
    (b"sys", TOP, G::Dir),                                       // 9
    (b"kernel", 9, G::Dir),                                      // 10
    (b"ostype", 10, G::Ostype),                                  // 11
    (b"osrelease", 10, G::Osrelease),                            // 12
    (b"version", 10, G::KernelVersion),                          // 13
    (b"hostname", 10, G::Hostname),                              // 14
    (b"domainname", 10, G::Domainname),                          // 15
    (b"pid_max", 10, G::PidMax),                                 // 16
    (b"random", 10, G::Dir),                                     // 17
    (b"boot_id", 17, G::BootId),                                 // 18
    (b"uuid", 17, G::Uuid),                                      // 19
    (b"entropy_avail", 17, G::EntropyAvail),                     // 20
    (b"poolsize", 17, G::Poolsize),                              // 21
    (b"fs", 9, G::Dir),                                          // 22
    (b"nr_open", 22, G::NrOpen),                                 // 23
    (b"cap_last_cap", 10, G::CapLastCap),                        // 24
    (b"inotify", 22, G::Dir),                                    // 25
    (b"max_user_watches", 25, G::InotifyWatches),                // 26
    (b"max_user_instances", 25, G::InotifyInstances),            // 27
    (b"max_queued_events", 25, G::InotifyQueued),                // 28
    (b"thread-self", TOP, G::ThreadSelfLink),                    // 29
    (b"ns_last_pid", 10, G::NsLastPid),                          // 30
    (b"net", TOP, G::NetLink),                                   // 31
    (b"sysvipc", TOP, G::Dir),                                   // 32
    (b"msg", 32, G::Ipc(IpcFile::SysvMsg)),                      // 33
    (b"shm", 32, G::Ipc(IpcFile::SysvShm)),                      // 34
    (b"sem", 32, G::Ipc(IpcFile::SysvSem)),                      // 35
    (b"msgmni", 10, G::Ipc(IpcFile::Msgmni)),                    // 36
    (b"msgmax", 10, G::Ipc(IpcFile::Msgmax)),                    // 37
    (b"msgmnb", 10, G::Ipc(IpcFile::Msgmnb)),                    // 38
    (b"shmmax", 10, G::Ipc(IpcFile::Shmmax)),                    // 39
    (b"shmall", 10, G::Ipc(IpcFile::Shmall)),                    // 40
    (b"shmmni", 10, G::Ipc(IpcFile::Shmmni)),                    // 41
    (b"sem", 10, G::Ipc(IpcFile::Sem)),                          // 42
    (b"mqueue", 22, G::Dir),                                     // 43
    (b"queues_max", 43, G::Ipc(IpcFile::MqQueuesMax)),           // 44
    (b"msg_max", 43, G::Ipc(IpcFile::MqMsgMax)),                 // 45
    (b"msgsize_max", 43, G::Ipc(IpcFile::MqMsgsizeMax)),         // 46
    (b"msg_default", 43, G::Ipc(IpcFile::MqMsgDefault)),         // 47
    (b"msgsize_default", 43, G::Ipc(IpcFile::MqMsgsizeDefault)), // 48
];

/// `/proc/sys/kernel/cap_last_cap`: the highest capability number
/// (`CAP_CHECKPOINT_RESTORE`, 40, `linux/capability.h`; proc_sys_kernel(5)).
pub const CAP_LAST_CAP: u32 = 40;

/// `/proc/sys/fs/inotify/max_queued_events`: the events an inotify queue
/// holds before `IN_Q_OVERFLOW` (poly-vfs's `inotify::MAX_QUEUED_EVENTS`).
pub const INOTIFY_MAX_QUEUED_EVENTS: u32 = 16384;

/// `/proc/sys/fs/inotify/max_user_watches` and `max_user_instances`
/// (inotify(7), "/proc interfaces"): what Poly reports. It does not count
/// watches or instances per user, so these are ceilings it never enforces.
pub const INOTIFY_MAX_USER_WATCHES: u32 = 65536;
pub const INOTIFY_MAX_USER_INSTANCES: u32 = 128;

/// The range of `/proc/<pid>/oom_score_adj` (proc_pid_oom_score_adj(5):
/// "-1000 (OOM_SCORE_ADJ_MIN) to +1000 (OOM_SCORE_ADJ_MAX)").
pub const OOM_SCORE_ADJ_MIN: i32 = -1000;
pub const OOM_SCORE_ADJ_MAX: i32 = 1000;

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
    NsDir,
    UidMap,
    GidMap,
    Setgroups,
    TaskDir,
    OomScoreAdj,
    OomScore,
    ClearRefs,
    /// `/proc/<pid>/net`: the files of the process's network namespace.
    NetDir,
}

/// The files of `/proc/<pid>/net`, in listing order.
const NET_ENTRIES: [(&[u8], NetFile); 2] = [(b"dev", NetFile::Dev), (b"route", NetFile::Route)];

impl E {
    /// The user namespace file it is, if it is one.
    fn id_map(self) -> Option<IdMapFile> {
        match self {
            E::UidMap => Some(IdMapFile::Uid),
            E::GidMap => Some(IdMapFile::Gid),
            E::Setgroups => Some(IdMapFile::Setgroups),
            _ => None,
        }
    }
}

/// The entries of `/proc/<pid>/ns`, in listing order: what namespaces(7)
/// lists. A kind's index here is what [`ProcSource`] gets. Their links'
/// texts use the type name: `pid` for `pid_for_children`, `time` for
/// `time_for_children`.
pub const NS_KINDS: [&str; 10] =
    ["ipc", "mnt", "net", "pid", "pid_for_children", "user", "uts", "cgroup", "time", "time_for_children"];

/// The indexes of `pid_for_children` and `time_for_children`, whose types
/// are `pid` and `time`.
const PID_FOR_CHILDREN: usize = 4;
const PID_KIND: usize = 3;
const TIME_FOR_CHILDREN: usize = 9;
const TIME_KIND: usize = 8;

/// The kind of namespace an entry names (`pid` for `pid_for_children`).
const fn ns_base(kind: usize) -> usize {
    match kind {
        PID_FOR_CHILDREN => PID_KIND,
        TIME_FOR_CHILDREN => TIME_KIND,
        k => k,
    }
}

fn ns_type(kind: usize) -> &'static str {
    NS_KINDS[ns_base(kind)]
}

/// A link text `<type>:[<inum>]`: the type's index and the number.
fn parse_ns_name(name: &[u8]) -> Option<(usize, u32)> {
    let open = name.iter().position(|&b| b == b':')?;
    let (ty, rest) = name.split_at(open);
    let num = rest.strip_prefix(b":[")?.strip_suffix(b"]")?;
    let kind = NS_KINDS.iter().position(|k| k.as_bytes() == ty).filter(|&k| ns_base(k) == k)?;
    Some((kind, parse_number(num)?))
}

/// The namespace an open procfs file is on, if it is one of the hidden
/// files `/proc/<pid>/ns/<kind>` links to: its inode number and kind (an
/// index into [`NS_KINDS`], never `pid_for_children` or
/// `time_for_children`).
/// The inode of the hidden file of the namespace with inode number `inum`
/// and kind `kind` (an index into [`NS_KINDS`]), in any procfs: what
/// [`namespace_file`] reads back.
pub fn namespace_file_ino(inum: u32, kind: usize) -> Ino {
    Node::NsFile(inum, kind).ino()
}

pub fn namespace_file(ino: Ino) -> Option<(u32, usize)> {
    match Node::from_ino(ino)? {
        Node::NsFile(inum, kind) => Some((inum, kind)),
        _ => None,
    }
}

/// The process whose directory `/proc/<pid>` an open procfs file is, if
/// it is one (pidfd_send_signal(2) takes such a descriptor like a pidfd).
pub fn process_dir(ino: Ino) -> Option<u32> {
    match Node::from_ino(ino)? {
        Node::Pid(pid) => Some(pid),
        _ => None,
    }
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
    (b"ns", E::NsDir, 0o511),
    (b"uid_map", E::UidMap, 0o644),
    (b"gid_map", E::GidMap, 0o644),
    (b"setgroups", E::Setgroups, 0o644),
    (b"task", E::TaskDir, 0o555),
    (b"oom_score_adj", E::OomScoreAdj, 0o644),
    (b"oom_score", E::OomScore, 0o444),
    (b"clear_refs", E::ClearRefs, 0o200),
    (b"net", E::NetDir, 0o555),
];

/// The files of `/proc/<pid>/task/<tid>`, in listing order.
const TASK_ENTRIES: [&[u8]; 3] = [b"comm", b"stat", b"status"];

/// A node of the tree, decoded from its inode number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Root,
    Global(usize),
    Pid(u32),
    Entry(u32, usize),
    Fd(u32, u32),
    FdInfo(u32, u32),
    /// `/proc/<pid>/ns/<kind>`, a link.
    Ns(u32, usize),
    /// What it names: a namespace (inode number, type), in no directory.
    NsFile(u32, usize),
    /// `/proc/<pid>/task/<tid>`: a thread of the process.
    Task(u32, u32),
    /// `/proc/<pid>/task/<tid>/comm`: the thread's name.
    TaskComm(u32, u32),
    /// `/proc/<pid>/task/<tid>/stat` and `status`: the thread's.
    TaskStat(u32, u32),
    TaskStatus(u32, u32),
    /// `/proc/<pid>/net/<file>`: an index into [`NET_ENTRIES`].
    Net(u32, usize),
}

const K_ROOT: u64 = 1;
const K_GLOBAL: u64 = 2;
const K_PID: u64 = 3;
const K_ENTRY: u64 = 4;
const K_FD: u64 = 5;
const K_FDINFO: u64 = 6;
const K_NS: u64 = 7;
/// The "pid" bits hold the namespace's inode number.
const K_NSFILE: u64 = 8;
const K_TASK: u64 = 9;
const K_TASKCOMM: u64 = 10;
const K_TASKSTAT: u64 = 11;
const K_TASKSTATUS: u64 = 12;
const K_NET: u64 = 13;
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
            Node::Ns(p, k) => (p, K_NS, k as u64),
            Node::NsFile(inum, k) => (inum, K_NSFILE, k as u64),
            Node::Task(p, t) => (p, K_TASK, u64::from(t)),
            Node::TaskComm(p, t) => (p, K_TASKCOMM, u64::from(t)),
            Node::TaskStat(p, t) => (p, K_TASKSTAT, u64::from(t)),
            Node::TaskStatus(p, t) => (p, K_TASKSTATUS, u64::from(t)),
            Node::Net(p, i) => (p, K_NET, i as u64),
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
            K_NS if pid != 0 && (index as usize) < NS_KINDS.len() => Node::Ns(pid, index as usize),
            K_NSFILE if pid != 0 && (index as usize) < NS_KINDS.len() && ns_base(index as usize) == index as usize => {
                Node::NsFile(pid, index as usize)
            }
            K_TASK if pid != 0 && index != 0 => Node::Task(pid, index as u32),
            K_TASKCOMM if pid != 0 && index != 0 => Node::TaskComm(pid, index as u32),
            K_TASKSTAT if pid != 0 && index != 0 => Node::TaskStat(pid, index as u32),
            K_TASKSTATUS if pid != 0 && index != 0 => Node::TaskStatus(pid, index as u32),
            K_NET if pid != 0 && (index as usize) < NET_ENTRIES.len() => Node::Net(pid, index as usize),
            _ => return None,
        })
    }
}

/// The largest descriptor number an inode number can carry.
const MAX_FD: u32 = INDEX_MASK as u32;

/// procfs over a [`ProcSource`].
pub struct ProcFs {
    src: Arc<dyn ProcSource>,
    /// The contents of files as their last read from the start made them,
    /// per reading process: a read further on continues in them, as a
    /// Linux procfs file read in pieces continues where it left off,
    /// instead of making the file again for every piece (a process with
    /// thousands of regions reads its `maps` 1 KiB at a time).
    snapshots: Lock<RawSpinLock, VecDeque<Snapshot>>,
}

/// A file's contents made for a read from offset 0.
struct Snapshot {
    ino: Ino,
    reader: Option<u32>,
    data: Arc<Vec<u8>>,
}

/// How many [`Snapshot`]s are kept (the oldest goes).
const SNAPSHOTS: usize = 32;

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
        ProcFs { src, snapshots: Lock::new(VecDeque::new()) }
    }

    /// The contents of `node` for a read at `offset` by the caller: made
    /// afresh at offset 0 (and kept), else the kept ones if the caller
    /// read this file from the start before.
    fn contents_at(&self, ino: Ino, node: Node, offset: u64) -> Result<Arc<Vec<u8>>> {
        let reader = self.src.current_pid();
        if offset > 0 {
            let snaps = self.snapshots.lock();
            if let Some(s) = snaps.iter().find(|s| s.ino == ino && s.reader == reader)
                && offset <= s.data.len() as u64
            {
                return Ok(s.data.clone());
            }
        }
        let data = Arc::new(self.contents(node)?);
        let mut snaps = self.snapshots.lock();
        snaps.retain(|s| !(s.ino == ino && s.reader == reader));
        if snaps.len() >= SNAPSHOTS {
            snaps.pop_front();
        }
        snaps.push_back(Snapshot { ino, reader, data: data.clone() });
        Ok(data)
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
            Node::Root | Node::Pid(_) | Node::Task(..) => FileType::Directory,
            Node::Global(i) => match GLOBALS[i].2 {
                G::Dir => FileType::Directory,
                G::SelfLink | G::MountsLink | G::ThreadSelfLink | G::NetLink => FileType::Symlink,
                _ => FileType::Regular,
            },
            Node::Entry(_, i) => match PID_ENTRIES[i].1 {
                E::FdDir | E::FdinfoDir | E::NsDir | E::TaskDir | E::NetDir => FileType::Directory,
                E::Exe | E::Cwd | E::Root => FileType::Symlink,
                _ => FileType::Regular,
            },
            Node::Fd(..) | Node::Ns(..) => FileType::Symlink,
            Node::FdInfo(..)
            | Node::NsFile(..)
            | Node::TaskComm(..)
            | Node::TaskStat(..)
            | Node::TaskStatus(..)
            | Node::Net(..) => FileType::Regular,
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
            Node::Ns(p, _) => Node::Entry(p, entry_index(E::NsDir)),
            Node::NsFile(..) => Node::Root,
            Node::Task(p, _) => Node::Entry(p, entry_index(E::TaskDir)),
            Node::TaskComm(p, t) | Node::TaskStat(p, t) | Node::TaskStatus(p, t) => Node::Task(p, t),
            Node::Net(p, _) => Node::Entry(p, entry_index(E::NetDir)),
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
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::NsDir => {
                self.owner(p)?;
                fixed(
                    &mut out,
                    &mut (0..NS_KINDS.len())
                        .filter(|&k| self.src.namespace(p, k).is_some())
                        .map(|k| (NS_KINDS[k].as_bytes().to_vec(), Node::Ns(p, k))),
                )
            }
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::NetDir => {
                self.owner(p)?;
                fixed(&mut out, &mut NET_ENTRIES.iter().enumerate().map(|(n, e)| (e.0.to_vec(), Node::Net(p, n))))
            }
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::TaskDir => {
                self.owner(p)?;
                let mut tids = self.src.threads(p);
                tids.sort_unstable();
                tids.dedup();
                for tid in tids.into_iter().filter(|&t| t != 0 && u64::from(t) <= INDEX_MASK) {
                    let at = PID_POS + u64::from(tid);
                    if at >= pos {
                        out.push((at, decimal(u64::from(tid)), Node::Task(p, tid)));
                    }
                }
            }
            Node::Task(p, t) => {
                self.owner(p)?;
                fixed(
                    &mut out,
                    &mut TASK_ENTRIES.iter().filter_map(|&name| Some((name.to_vec(), Self::task_entry(p, t, name)?))),
                )
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
            Node::Pid(p) | Node::Entry(p, _) | Node::Net(p, _) => self.src.owner(p).is_some(),
            Node::Fd(p, fd) | Node::FdInfo(p, fd) => self.has_fd(p, fd),
            Node::Ns(p, k) => self.src.namespace(p, k).is_some(),
            Node::NsFile(inum, k) => self.src.namespace_exists(inum, k),
            Node::Task(p, t) | Node::TaskComm(p, t) | Node::TaskStat(p, t) | Node::TaskStatus(p, t) => {
                self.src.threads(p).contains(&t)
            }
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
                G::CapLastCap => format::line(&decimal(u64::from(CAP_LAST_CAP))),
                G::InotifyWatches => format::line(&decimal(u64::from(INOTIFY_MAX_USER_WATCHES))),
                G::InotifyInstances => format::line(&decimal(u64::from(INOTIFY_MAX_USER_INSTANCES))),
                G::InotifyQueued => format::line(&decimal(u64::from(INOTIFY_MAX_QUEUED_EVENTS))),
                G::NsLastPid => format::line(&decimal(u64::from(src.ns_last_pid()))),
                G::Ipc(file) => src.ipc_file(file),
                G::Dir | G::SelfLink | G::MountsLink | G::ThreadSelfLink | G::NetLink => return Err(Errno::EINVAL),
            },
            Node::Entry(p, i) => {
                let info = self.process(p)?;
                match PID_ENTRIES[i].1 {
                    E::Stat => format::stat(&info, &src.size_maps(p), &src.system()),
                    E::Statm => format::statm(&info, &src.size_maps(p)),
                    E::Status => format::status(&info, &src.size_maps(p)),
                    E::Maps => format::maps(&src.maps(p)),
                    E::Cmdline => src.cmdline(p),
                    E::Environ => src.environ(p),
                    E::Comm => format::line(format::comm(&info.comm)),
                    E::Mounts => format::mounts(&src.mounts(p)),
                    E::Mountinfo => format::mountinfo(&src.mounts(p)),
                    E::UidMap => src.id_map(p, IdMapFile::Uid),
                    E::GidMap => src.id_map(p, IdMapFile::Gid),
                    E::Setgroups => src.id_map(p, IdMapFile::Setgroups),
                    E::OomScoreAdj => format::line(alloc::format!("{}", src.oom_score_adj(p)).as_bytes()),
                    // proc_pid_oom_score(5): the badness, 0..1000 with the
                    // adjustment added. Poly does not kill for memory: only
                    // the adjustment counts.
                    E::OomScore => format::line(&decimal(src.oom_score_adj(p).clamp(0, 1000) as u64)),
                    // Write-only (proc_pid_clear_refs(5)).
                    E::ClearRefs => return Err(Errno::EINVAL),
                    E::Exe | E::Cwd | E::Root | E::FdDir | E::FdinfoDir | E::NsDir | E::TaskDir | E::NetDir => {
                        return Err(Errno::EINVAL);
                    }
                }
            }
            Node::FdInfo(p, fd) => format::fdinfo(&src.fd(p, fd).ok_or(Errno::ENOENT)?),
            Node::Net(p, i) => src.net_file(p, NET_ENTRIES[i].1).ok_or(Errno::ENOENT)?,
            Node::TaskComm(p, t) => {
                let comm = match src.thread_comm(p, t) {
                    Some(c) => c,
                    None => self.process(p)?.comm,
                };
                format::line(format::comm(&comm))
            }
            Node::TaskStat(p, t) | Node::TaskStatus(p, t) => {
                let mut info = self.process(p)?;
                if let Some(comm) = src.thread_comm(p, t) {
                    info.comm = comm;
                }
                match node {
                    Node::TaskStat(..) => format::thread_stat(&info, t, &src.size_maps(p), &src.system()),
                    _ => format::thread_status(&info, t, &src.size_maps(p)),
                }
            }
            Node::Root | Node::Pid(_) | Node::Task(..) => return Err(Errno::EISDIR),
            // A namespace file has no contents (read(2) on Linux's: EINVAL).
            Node::Fd(..) | Node::Ns(..) | Node::NsFile(..) => return Err(Errno::EINVAL),
        })
    }

    /// `/proc/sys/kernel/hostname` and `domainname`, which a write sets.
    /// `/proc/sys/kernel/ns_last_pid` (pid_namespaces(7)).
    fn is_ns_last_pid(node: Node) -> bool {
        matches!(node, Node::Global(i) if GLOBALS[i].2 == G::NsLastPid)
    }

    /// A setting of the IPC namespace, which a write sets.
    fn ipc_setting(node: Node) -> Option<IpcFile> {
        match node {
            Node::Global(i) => match GLOBALS[i].2 {
                G::Ipc(file) if file.writable() => Some(file),
                _ => None,
            },
            _ => None,
        }
    }

    fn kernel_name_file(node: Node) -> Option<KernelName> {
        match node {
            Node::Global(i) => match GLOBALS[i].2 {
                G::Hostname => Some(KernelName::Host),
                G::Domainname => Some(KernelName::Domain),
                _ => None,
            },
            _ => None,
        }
    }

    /// `/proc/<pid>/task/<tid>/<name>`.
    fn task_entry(p: u32, t: u32, name: &[u8]) -> Option<Node> {
        match name {
            b"comm" => Some(Node::TaskComm(p, t)),
            b"stat" => Some(Node::TaskStat(p, t)),
            b"status" => Some(Node::TaskStatus(p, t)),
            _ => None,
        }
    }

    /// The process files a write changes: `oom_score_adj`, `clear_refs`.
    fn process_setting(node: Node) -> Option<(u32, E)> {
        match node {
            Node::Entry(p, i) if matches!(PID_ENTRIES[i].1, E::OomScoreAdj | E::ClearRefs) => {
                Some((p, PID_ENTRIES[i].1))
            }
            _ => None,
        }
    }

    /// A write to `oom_score_adj` or `clear_refs` of process `p`.
    ///
    /// - `oom_score_adj` takes a decimal number in
    ///   [`OOM_SCORE_ADJ_MIN`]..=[`OOM_SCORE_ADJ_MAX`] (surrounding white
    ///   space allowed), else `EINVAL`; the source decides who may set it.
    /// - `clear_refs` takes 1 to 5 (proc_pid_clear_refs(5): clear the
    ///   referenced and soft-dirty bits, reset the peak RSS), else
    ///   `EINVAL`. Poly keeps none of those bits, so a valid value
    ///   changes nothing.
    fn write_setting(&self, p: u32, e: E, data: &[u8]) -> Result<usize> {
        let text = core::str::from_utf8(data).map_err(|_| Errno::EINVAL)?.trim();
        let value: i64 = text.parse().map_err(|_| Errno::EINVAL)?;
        match e {
            E::OomScoreAdj => {
                let v = i32::try_from(value)
                    .ok()
                    .filter(|v| (OOM_SCORE_ADJ_MIN..=OOM_SCORE_ADJ_MAX).contains(v))
                    .ok_or(Errno::EINVAL)?;
                self.src.set_oom_score_adj(p, v)?;
            }
            _ => {
                if !(1..=5).contains(&value) {
                    return Err(Errno::EINVAL);
                }
                self.owner(p)?;
            }
        }
        Ok(data.len())
    }

    /// `st_nlink` of a node: a directory has 2 (its name and `.`), a
    /// process's `task` directory one more per thread (each thread's `..`;
    /// 3 for a single-threaded process, as Linux shows it), other files 1.
    fn link_count(&self, node: Node, kind: FileType) -> u32 {
        match node {
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::TaskDir => 2 + self.src.threads(p).len() as u32,
            _ if kind == FileType::Directory => 2,
            _ => 1,
        }
    }

    /// The writable files: a process's user namespace files.
    fn id_map_file(node: Node) -> Option<(u32, IdMapFile)> {
        match node {
            Node::Entry(p, i) => PID_ENTRIES[i].1.id_map().map(|f| (p, f)),
            _ => None,
        }
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
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::NsDir => {
                match NS_KINDS.iter().position(|k| k.as_bytes() == name) {
                    Some(k) => self.src.namespace(p, k).map(|_| Node::Ns(p, k)),
                    // What a link's text names.
                    None => parse_ns_name(name)
                        .filter(|&(k, inum)| self.src.namespace_exists(inum, k))
                        .map(|(k, inum)| Node::NsFile(inum, k)),
                }
            }
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::NetDir => {
                NET_ENTRIES.iter().position(|e| e.0 == name).map(|n| Node::Net(p, n))
            }
            Node::Entry(p, i) if PID_ENTRIES[i].1 == E::TaskDir => parse_number(name)
                .filter(|&t| t != 0 && u64::from(t) <= INDEX_MASK && self.src.threads(p).contains(&t))
                .map(|t| Node::Task(p, t)),
            Node::Task(p, t) => Self::task_entry(p, t, name),
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
            Node::Root | Node::Global(_) | Node::NsFile(..) => (0, 0, boot),
            Node::Pid(p)
            | Node::Entry(p, _)
            | Node::Fd(p, _)
            | Node::FdInfo(p, _)
            | Node::Ns(p, _)
            | Node::Task(p, _)
            | Node::TaskComm(p, _)
            | Node::TaskStat(p, _)
            | Node::TaskStatus(p, _)
            | Node::Net(p, _) => {
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
            Node::Root | Node::Pid(_) | Node::Task(..) => 0o555,
            Node::TaskComm(..) => 0o644,
            Node::TaskStat(..) | Node::TaskStatus(..) => 0o444,
            Node::Global(i) => match GLOBALS[i].2 {
                G::Dir => 0o555,
                G::SelfLink | G::MountsLink | G::ThreadSelfLink | G::NetLink => 0o777,
                G::Hostname | G::Domainname | G::NsLastPid => 0o644,
                G::Ipc(file) if file.writable() => 0o644,
                _ => 0o444,
            },
            Node::Entry(_, i) => PID_ENTRIES[i].2,
            Node::Fd(p, fd) => match self.src.fd(p, fd) {
                Some(f) => 0o500 | if f.read { 0o400 } else { 0 } | if f.write { 0o200 } else { 0 },
                None => 0o700,
            },
            Node::FdInfo(..) | Node::NsFile(..) | Node::Net(..) => 0o444,
            Node::Ns(..) => 0o777,
        };
        // A namespace file's inode number is the namespace's (namespaces(7):
        // "the inode number of the namespace", what its link shows).
        let shown_ino = match node {
            Node::NsFile(inum, _) => u64::from(inum),
            _ => ino,
        };
        Ok(Attr {
            ino: shown_ino,
            kind,
            perm,
            nlink: self.link_count(node, kind),
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
            // proc_net(5): the network namespace of whoever follows it.
            Node::Global(i) if GLOBALS[i].2 == G::NetLink => Ok(b"self/net".to_vec()),
            // proc(5): "/proc/thread-self ... the same as /proc/self/task/<tid>".
            Node::Global(i) if GLOBALS[i].2 == G::ThreadSelfLink => {
                let (pid, tid) = src.current_pid().zip(src.current_tid()).ok_or(Errno::ENOENT)?;
                Ok(alloc::format!("{pid}/task/{tid}").into_bytes())
            }
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
            Node::Ns(p, k) => {
                let inum = src.namespace(p, k).ok_or(Errno::ENOENT)?;
                // Another user's namespaces are not for the reader to follow.
                if !src.may_follow_namespaces(p) {
                    return Err(Errno::EACCES);
                }
                Ok(alloc::format!("{}:[{inum}]", ns_type(k)).into_bytes())
            }
            _ => Err(Errno::EINVAL),
        }
    }

    /// A namespace file cannot be opened for writing: `EPERM`
    /// (ioctl_nsfs's spec sheet).
    fn write_open_error(&self, ino: Ino) -> Option<Errno> {
        matches!(Node::from_ino(ino), Some(Node::NsFile(..))).then_some(Errno::EPERM)
    }

    fn writable_file(&self, ino: Ino) -> bool {
        Node::from_ino(ino).is_some_and(|n| {
            Self::id_map_file(n).is_some()
                || Self::kernel_name_file(n).is_some()
                || Self::process_setting(n).is_some()
                || Self::is_ns_last_pid(n)
                || Self::ipc_setting(n).is_some()
        })
    }

    /// A process's `exe`, `cwd`, `root`, `fd/<n>` and `ns/<kind>` name
    /// what the kernel holds, not a path of their own (proc_pid_fd(5)
    /// calls them magic links).
    fn is_magic_link(&self, ino: Ino) -> bool {
        match Node::from_ino(ino) {
            Some(Node::Fd(..) | Node::Ns(..)) => true,
            Some(Node::Entry(_, i)) => matches!(PID_ENTRIES[i].1, E::Exe | E::Cwd | E::Root),
            _ => false,
        }
    }

    fn magic_target(&self, ino: Ino) -> Option<poly_vfs::MagicTarget> {
        let (pid, link) = match Node::from_ino(ino)? {
            Node::Fd(p, fd) => (p, MagicLink::Fd(fd)),
            Node::Entry(p, i) => match PID_ENTRIES[i].1 {
                E::Exe => (p, MagicLink::Exe),
                E::Cwd => (p, MagicLink::Cwd),
                E::Root => (p, MagicLink::Root),
                _ => return None,
            },
            _ => return None,
        };
        self.src.magic_target(pid, link)
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let node = self.node(ino)?;
        if Self::kind(node) == FileType::Directory {
            return Err(Errno::EISDIR);
        }
        Ok(copy_at(&self.contents_at(ino, node, offset)?, offset, buf))
    }

    fn write(&self, ino: Ino, offset: u64, data: &[u8]) -> Result<usize> {
        let node = self.node(ino)?;
        if let Some(which) = Self::kernel_name_file(node) {
            return self.src.write_kernel_name(which, offset, data);
        }
        if Self::is_ns_last_pid(node) {
            return self.src.write_ns_last_pid(data);
        }
        if let Some(file) = Self::ipc_setting(node) {
            return self.src.write_ipc_file(file, data);
        }
        if let Some((p, e)) = Self::process_setting(node) {
            return self.write_setting(p, e, data);
        }
        match Self::id_map_file(node) {
            Some((p, file)) => {
                self.owner(p)?;
                self.src.write_id_map(p, file, offset, data)
            }
            None => Err(Errno::EROFS),
        }
    }

    fn truncate(&self, ino: Ino, _size: u64) -> Result<()> {
        // `O_TRUNC` on the writable sysctl and process files changes
        // nothing; the write that follows sets the value (Chromium opens
        // `oom_score_adj` with creat(2)).
        let node = self.node(ino)?;
        if Self::kernel_name_file(node).is_some()
            || Self::process_setting(node).is_some()
            || Self::is_ns_last_pid(node)
            || Self::ipc_setting(node).is_some()
        {
            Ok(())
        } else {
            Err(Errno::EROFS)
        }
    }

    /// Pins a node. Every lookup checked that its node exists, so this does
    /// not check again: a node pinned once (a process's root or working
    /// directory, an open file) must stay pinnable after its process is
    /// gone, as the VFS re-pins it when it copies a reference (Chromium's
    /// sandbox makes `/proc/<helper>/fdinfo` its root and the helper
    /// exits, docs/chromium-linux-gaps.md C4); reading it then fails with
    /// `ENOENT`.
    fn open(&self, ino: Ino) -> Result<()> {
        let node = self.node(ino)?;
        // A reference to a namespace file holds its namespace (setns(2)).
        if let Node::NsFile(inum, k) = node {
            self.src.pin_namespace(inum, k, true);
        }
        Ok(())
    }

    fn release(&self, ino: Ino) {
        if let Some(Node::NsFile(inum, k)) = Node::from_ino(ino) {
            self.src.pin_namespace(inum, k, false);
        }
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
