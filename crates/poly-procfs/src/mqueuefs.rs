//! The message queue file system (mq_overview(7), "Mounting the message
//! queue filesystem"): one flat directory with a regular file per POSIX
//! message queue of an IPC namespace.
//!
//! [`MqueueFs`] stores nothing: the queues are whatever its
//! [`MqueueSource`] says they are each time it is asked. The kernel makes
//! one per mount over the IPC namespace of the process that mounts it
//! (ipc_namespaces(7); the ns_uts_ipc sheet: "a mqueue mount belongs to
//! the IPC namespace of its mounter"), so a mount made before a process
//! moved to another IPC namespace keeps showing the old namespace's
//! queues, and one made after shows the new one's (ADR 0178).
//!
//! - The root is a sticky directory, mode `1777`, owned by root.
//! - A queue is a regular file with the queue's permission bits and
//!   owner; its size is the bytes of the messages in it. Reading it gives
//!   the queue's status line (`QSIZE:... NOTIFY:... SIGNO:...
//!   NOTIFY_PID:...`); writing is `EINVAL`.
//! - Removing the file removes the queue's name, as mq_unlink(3) does.
//!   Creating a regular file makes a queue with the default attributes.
//!   Nothing else can be made here (`EPERM`).
//!
//! Inode numbers: the root is 1, a queue is [`FIRST_QUEUE_INO`] plus the
//! number its source gives it, which is never reused; directory positions
//! are inode numbers, so they stay valid while queues come and go.

use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{Attr, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, RenameFlags, Result};
use poly_vfs::{DeviceId, MQUEUE_MAGIC, SetAttr, Timespec};

/// A queue as the file system shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    /// Its name, without the leading slash of mq_open(3).
    pub name: Vec<u8>,
    /// A number that names this queue for as long as it exists and is not
    /// used again afterwards (below 2^48).
    pub id: u64,
    /// Permission bits and owner.
    pub perm: u16,
    pub uid: u32,
    pub gid: u32,
    /// Bytes of the messages queued.
    pub size: u64,
}

/// The queues of one IPC namespace, as the kernel has them. Every method
/// is called without file system locks held.
pub trait MqueueSource: Send + Sync {
    /// The named queues, in any order.
    fn queues(&self) -> Vec<QueueEntry>;

    /// The status line of queue `id`; `None` if no name leads to it any
    /// more.
    fn status(&self, id: u64) -> Option<Vec<u8>>;

    /// Remove the name `name` (the queue itself goes with its last open
    /// descriptor): `ENOENT` if there is none. The VFS checked the
    /// caller's permission on the sticky directory.
    fn unlink(&self, name: &[u8]) -> Result<()>;

    /// A new queue named `name` with the namespace's default attributes:
    /// its number. `EEXIST`, `ENOSPC` (the namespace's limit on queues),
    /// `ENAMETOOLONG`.
    fn create(&self, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<u64>;
}

/// The root directory's inode number.
const ROOT: Ino = 1;
/// The inode number of the queue with number 0.
pub const FIRST_QUEUE_INO: Ino = 2;

/// The message queue file system over a [`MqueueSource`].
pub struct MqueueFs {
    src: Arc<dyn MqueueSource>,
    /// The timestamp of every node: when the file system was made.
    time: Timespec,
}

impl core::fmt::Debug for MqueueFs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MqueueFs").finish_non_exhaustive()
    }
}

impl MqueueFs {
    /// The queues of `src`, with `time` as every node's timestamp.
    pub fn new(src: Arc<dyn MqueueSource>, time: Timespec) -> MqueueFs {
        MqueueFs { src, time }
    }

    fn queue(&self, ino: Ino) -> Result<QueueEntry> {
        let id = ino.checked_sub(FIRST_QUEUE_INO).ok_or(Errno::ENOENT)?;
        self.src.queues().into_iter().find(|q| q.id == id).ok_or(Errno::ENOENT)
    }

    fn attr(&self, ino: Ino, kind: FileType, perm: u16, uid: u32, gid: u32, size: u64) -> Attr {
        Attr {
            ino,
            kind,
            perm,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid,
            gid,
            size,
            rdev: DeviceId::default(),
            atime: self.time,
            mtime: self.time,
            ctime: self.time,
            blocks: 0,
            blksize: 4096,
        }
    }
}

impl FileSystem for MqueueFs {
    fn info(&self) -> FsInfo {
        // Queues come and go through mq_open and mq_unlink: no lookup cache.
        FsInfo { name: "mqueue", read_only: false, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        if name == b".." {
            return Ok(ROOT);
        }
        self.src.queues().iter().find(|q| q.name == name).map(|q| FIRST_QUEUE_INO + q.id).ok_or(Errno::ENOENT)
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        if ino == ROOT {
            return Ok(self.attr(ROOT, FileType::Directory, 0o1777, 0, 0, 0));
        }
        let q = self.queue(ino)?;
        Ok(self.attr(ino, FileType::Regular, q.perm, q.uid, q.gid, q.size))
    }

    /// A queue's mode and owner are fixed when it is made (Poly's
    /// choice; a truncating open changes nothing).
    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        if set.perm.is_some() || set.uid.is_some() || set.gid.is_some() {
            return Err(Errno::EPERM);
        }
        self.getattr(ino)
    }

    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        if node.kind != FileType::Regular {
            return Err(Errno::EPERM);
        }
        self.src.create(name, node.perm & 0o777, node.uid, node.gid).map(|id| FIRST_QUEUE_INO + id)
    }

    fn mkdir(&self, _: Ino, _: &[u8], _: u16, _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn symlink(&self, _: Ino, _: &[u8], _: &[u8], _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn link(&self, _: Ino, _: &[u8], _: Ino) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        self.src.unlink(name)
    }

    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        self.lookup(dir, name)?;
        Err(Errno::ENOTDIR)
    }

    fn rename(&self, _: Ino, _: &[u8], _: Ino, _: &[u8], _: RenameFlags) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn readlink(&self, _: Ino) -> Result<Vec<u8>> {
        Err(Errno::EINVAL)
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if ino == ROOT {
            return Err(Errno::EISDIR);
        }
        let id = ino.checked_sub(FIRST_QUEUE_INO).ok_or(Errno::ENOENT)?;
        let text = self.src.status(id).ok_or(Errno::ENOENT)?;
        Ok(crate::copy_at(&text, offset, buf))
    }

    fn write(&self, ino: Ino, _: u64, _: &[u8]) -> Result<usize> {
        if ino == ROOT {
            return Err(Errno::EISDIR);
        }
        Err(Errno::EINVAL)
    }

    fn truncate(&self, ino: Ino, _: u64) -> Result<()> {
        self.queue(ino).map(|_| ())
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        for (at, name) in [(0, &b"."[..]), (1, &b".."[..])] {
            if at >= pos && !emit(&DirEntry { ino: ROOT, kind: FileType::Directory, name, next: at + 1 }) {
                return Ok(());
            }
        }
        let mut queues = self.src.queues();
        queues.sort_unstable_by_key(|q| q.id);
        for q in queues {
            let ino = FIRST_QUEUE_INO + q.id;
            if ino >= pos && !emit(&DirEntry { ino, kind: FileType::Regular, name: &q.name, next: ino + 1 }) {
                break;
            }
        }
        Ok(())
    }

    fn statfs(&self) -> Result<FsStats> {
        Ok(FsStats { magic: MQUEUE_MAGIC, block_size: 4096, fragment_size: 4096, name_max: 255, ..FsStats::default() })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use alloc::string::String;
    use poly_vfs::{At, Clock, Cred, MountFlags, OpenFlags, Tmpfs, Vfs};
    use std::sync::Mutex;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> Timespec {
            Timespec::new(1_700_000_000, 0)
        }
    }

    /// Queues as a namespace would keep them: (entry, messages' text).
    #[derive(Default)]
    struct Fake {
        queues: Mutex<Vec<QueueEntry>>,
        next: Mutex<u64>,
    }

    impl Fake {
        fn add(&self, name: &str, perm: u16, uid: u32) -> u64 {
            let mut next = self.next.lock().unwrap();
            let id = *next;
            *next += 1;
            self.queues.lock().unwrap().push(QueueEntry {
                name: name.as_bytes().to_vec(),
                id,
                perm,
                uid,
                gid: uid,
                size: 0,
            });
            id
        }
    }

    impl MqueueSource for Fake {
        fn queues(&self) -> Vec<QueueEntry> {
            self.queues.lock().unwrap().clone()
        }

        fn status(&self, id: u64) -> Option<Vec<u8>> {
            let q = self.queues.lock().unwrap();
            let q = q.iter().find(|q| q.id == id)?;
            Some(alloc::format!("QSIZE:{:<10} NOTIFY:0     SIGNO:0     NOTIFY_PID:0     \n", q.size).into_bytes())
        }

        fn unlink(&self, name: &[u8]) -> Result<()> {
            let mut q = self.queues.lock().unwrap();
            let at = q.iter().position(|q| q.name == name).ok_or(Errno::ENOENT)?;
            q.remove(at);
            Ok(())
        }

        fn create(&self, name: &[u8], perm: u16, uid: u32, _gid: u32) -> Result<u64> {
            if self.queues.lock().unwrap().iter().any(|q| q.name == name) {
                return Err(Errno::EEXIST);
            }
            Ok(self.add(core::str::from_utf8(name).unwrap(), perm, uid))
        }
    }

    struct Env {
        vfs: Vfs,
        ctx: poly_vfs::FsContext,
        cred: Cred,
    }

    impl Env {
        fn new(mounts: &[(&str, &Arc<Fake>)]) -> Env {
            let vfs = Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))));
            let ctx = vfs.root_context().unwrap();
            let cred = Cred::root();
            for (dir, fake) in mounts {
                vfs.mkdir(&ctx, &cred, At::Cwd, dir.as_bytes(), 0o755).unwrap();
                let fs = Arc::new(MqueueFs::new((*fake).clone(), Timespec::new(1_700_000_000, 0)));
                vfs.mount(&ctx, &cred, At::Cwd, dir.as_bytes(), fs, MountFlags::default()).unwrap();
            }
            Env { vfs, ctx, cred }
        }

        fn list(&self, path: &str) -> Vec<String> {
            let f = self.vfs.open(&self.ctx, &self.cred, At::Cwd, path.as_bytes(), OpenFlags::RDONLY, 0).unwrap();
            let mut out = Vec::new();
            loop {
                let mut batch = Vec::new();
                // One entry per call, so positions are used between calls.
                let n = f
                    .read_dir(&mut |e| {
                        if !batch.is_empty() {
                            return false;
                        }
                        batch.push(String::from_utf8(e.name.to_vec()).unwrap());
                        true
                    })
                    .unwrap();
                if n == 0 {
                    return out;
                }
                out.append(&mut batch);
            }
        }

        fn stat(&self, path: &str) -> Result<poly_vfs::Stat> {
            self.vfs.stat(&self.ctx, &self.cred, At::Cwd, path.as_bytes(), true)
        }
    }

    /// The ns_uts_ipc sheet: a new mount lists nothing; a queue shows up as
    /// a regular file with its mode; removing it empties the listing.
    #[test]
    fn queues_are_regular_files_of_their_mode() {
        let fake = Arc::new(Fake::default());
        let e = Env::new(&[("/mq", &fake)]);
        assert_eq!(e.list("/mq"), [".", ".."]);
        let st = e.stat("/mq").unwrap();
        assert_eq!((st.kind(), st.mode & 0o7777, st.uid), (Some(FileType::Directory), 0o1777, 0));
        fake.add("pk_mq1", 0o600, 1000);
        fake.add("other", 0o644, 0);
        assert_eq!(e.list("/mq"), [".", "..", "pk_mq1", "other"]);
        let st = e.stat("/mq/pk_mq1").unwrap();
        assert_eq!((st.mode, st.uid, st.gid, st.nlink), (0o100600, 1000, 1000, 1));
        assert_eq!(e.stat("/mq/none").err(), Some(Errno::ENOENT));
        // Reading gives the status line; writing is refused.
        let f = e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/mq/pk_mq1", OpenFlags::RDWR, 0).unwrap();
        let mut buf = [0u8; 128];
        let n = f.read(&mut buf).unwrap();
        assert!(buf[..n].starts_with(b"QSIZE:0 "));
        assert_eq!(f.write(b"x"), Err(Errno::EINVAL));
        // Removing the file removes the queue's name.
        assert_eq!(e.vfs.unlink(&e.ctx, &e.cred, At::Cwd, b"/mq/pk_mq1"), Ok(()));
        assert_eq!(e.list("/mq"), [".", "..", "other"]);
        assert_eq!(e.vfs.unlink(&e.ctx, &e.cred, At::Cwd, b"/mq/pk_mq1"), Err(Errno::ENOENT));
        // A file made here is a queue; nothing else can be made.
        let made =
            e.vfs.open(&e.ctx, &e.cred, At::Cwd, b"/mq/made", OpenFlags::CREAT | OpenFlags::WRONLY, 0o640).map(|_| ());
        assert_eq!(made, Ok(()));
        assert_eq!(fake.queues().iter().find(|q| q.name == b"made").map(|q| q.perm), Some(0o640));
        assert_eq!(e.vfs.mkdir(&e.ctx, &e.cred, At::Cwd, b"/mq/dir", 0o755), Err(Errno::EPERM));
        assert_eq!(e.vfs.symlink(&e.ctx, &e.cred, b"other", At::Cwd, b"/mq/link"), Err(Errno::EPERM));
        let root = e.vfs.lookup(&e.ctx, &e.cred, At::Cwd, b"/mq", true).unwrap();
        assert_eq!(root.statfs().unwrap().magic, MQUEUE_MAGIC);
    }

    /// Two mounts over two namespaces: each shows its own queues, whoever
    /// looks.
    #[test]
    fn a_mount_shows_the_queues_of_its_own_namespace() {
        let (outer, inner) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
        let e = Env::new(&[("/a", &outer), ("/b", &inner)]);
        outer.add("q", 0o600, 0);
        assert_eq!(e.list("/a"), [".", "..", "q"]);
        assert_eq!(e.list("/b"), [".", ".."]);
        inner.add("q", 0o600, 0);
        assert_eq!(e.list("/b"), [".", "..", "q"]);
        assert_eq!(e.list("/a").len(), 3);
        inner.unlink(b"q").unwrap();
        assert_eq!(e.list("/b"), [".", ".."]);
        assert_eq!(e.list("/a"), [".", "..", "q"]);
    }

    /// A listing in pieces goes on where it stopped while queues come
    /// and go.
    #[test]
    fn positions_survive_queues_coming_and_going() {
        let fake = Arc::new(Fake::default());
        let fs = MqueueFs::new(fake.clone(), Timespec::new(0, 0));
        for n in ["a", "b", "c"] {
            fake.add(n, 0o600, 0);
        }
        let mut seen = Vec::new();
        let mut pos = 0;
        let step = |fs: &MqueueFs, seen: &mut Vec<Vec<u8>>, pos: &mut u64| {
            let mut took = false;
            fs.readdir(ROOT, *pos, &mut |e| {
                if took {
                    return false;
                }
                took = true;
                seen.push(e.name.to_vec());
                *pos = e.next;
                true
            })
            .unwrap();
            took
        };
        for _ in 0..3 {
            assert!(step(&fs, &mut seen, &mut pos));
        }
        assert_eq!(seen, [&b"."[..], b"..", b"a"]);
        fake.unlink(b"b").unwrap();
        fake.add("d", 0o600, 0);
        while step(&fs, &mut seen, &mut pos) {}
        assert_eq!(seen[3..], [&b"c"[..], b"d"]);
        assert_eq!(fs.lookup(ROOT, b"d"), Ok(FIRST_QUEUE_INO + 3));
        assert_eq!(fs.lookup(ROOT, b".."), Ok(ROOT));
        assert_eq!(fs.getattr(FIRST_QUEUE_INO + 1).err(), Some(Errno::ENOENT), "b is gone");
    }
}
