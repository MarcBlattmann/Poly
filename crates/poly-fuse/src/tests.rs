//! The requests [`FuseFs`] sends, checked against a server that runs inside
//! the caller's wait: every time a call sleeps for its answer, the test's
//! server reads one request and writes its reply.

use super::*;
use std::boxed::Box;
use std::sync::Mutex;
use std::vec;

const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const ENOSYS: i32 = 38;
const ENOENT: i32 = 2;
const EIO: i32 = 5;

/// One request as the server read it.
#[derive(Clone, Debug)]
struct Req {
    op: u32,
    node: u64,
    uid: u32,
    body: Vec<u8>,
}

impl Req {
    fn w32(&self, at: usize) -> u32 {
        u32_at(&self.body, at)
    }
    fn w64(&self, at: usize) -> u64 {
        u64_at(&self.body, at)
    }
    /// The NUL-terminated names from `at` on.
    fn names(&self, at: usize) -> Vec<Vec<u8>> {
        self.body[at..].split(|&b| b == 0).filter(|n| !n.is_empty()).map(<[u8]>::to_vec).collect()
    }
}

/// What the server answers: a body, an error (positive errno), or nothing.
type Answer = Option<core::result::Result<Vec<u8>, i32>>;
type Handler = Box<dyn FnMut(&Req) -> Answer + Send>;

struct TestEnv {
    conn: Arc<Conn>,
    handler: Mutex<Handler>,
    log: Mutex<Vec<Req>>,
    now: AtomicU64,
    /// The `INIT` reply: flags, max_write, max_pages; `None` leaves `INIT`
    /// unanswered.
    init: Mutex<Option<(u32, u32, u16)>>,
}

impl TestEnv {
    /// Serves one queued request; `false` if there is none.
    fn pump(&self) -> bool {
        let mut buf = vec![0u8; 1 << 20];
        let Ok(n) = self.conn.read(self, &mut buf, true) else { return false };
        let req = Req {
            op: u32_at(&buf, 4),
            node: u64_at(&buf, 16),
            uid: u32_at(&buf, 24),
            body: buf[IN_HEADER..n].to_vec(),
        };
        assert_eq!(u32_at(&buf, 0) as usize, n, "the header's len is the request's length");
        let unique = u64_at(&buf, 8);
        self.log.lock().unwrap().push(req.clone());
        let answer: Answer = if req.op == op::INIT {
            let Some((flags, max_write, max_pages)) = *self.init.lock().unwrap() else { return true };
            let mut o = vec![0u8; 64];
            o[0..4].copy_from_slice(&7u32.to_le_bytes());
            o[4..8].copy_from_slice(&31u32.to_le_bytes());
            o[12..16].copy_from_slice(&flags.to_le_bytes());
            o[20..24].copy_from_slice(&max_write.to_le_bytes());
            o[28..30].copy_from_slice(&max_pages.to_le_bytes());
            Some(Ok(o))
        } else {
            (self.handler.lock().unwrap())(&req)
        };
        if let Some(a) = answer {
            let (err, body) = match a {
                Ok(b) => (0i32, b),
                Err(e) => (-e, Vec::new()),
            };
            let mut out = Vec::new();
            out.extend_from_slice(&((OUT_HEADER + body.len()) as u32).to_le_bytes());
            out.extend_from_slice(&err.to_le_bytes());
            out.extend_from_slice(&unique.to_le_bytes());
            out.extend_from_slice(&body);
            assert_eq!(self.conn.write(&out), Ok(out.len()));
        }
        true
    }

    fn ops(&self) -> Vec<u32> {
        self.log.lock().unwrap().iter().map(|r| r.op).collect()
    }

    fn of(&self, opcode: u32) -> Vec<Req> {
        self.log.lock().unwrap().iter().filter(|r| r.op == opcode).cloned().collect()
    }
}

impl Env for TestEnv {
    fn now_ns(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }

    fn wait(&self, _queue: &Arc<WaitQueue>, ready: &mut dyn FnMut() -> bool) -> Result<()> {
        loop {
            if ready() {
                return Ok(());
            }
            // Nothing left to serve: the call would sleep for ever.
            if !self.pump() {
                return Err(Errno::EINTR);
            }
        }
    }

    fn caller(&self) -> Caller {
        Caller { uid: 1000, gid: 100, pid: 42, umask: 0o22 }
    }
}

/// A `struct fuse_attr`.
fn attr(ino: u64, mode: u32, size: u64, nlink: u32) -> Vec<u8> {
    let mut a = vec![0u8; 88];
    a[0..8].copy_from_slice(&ino.to_le_bytes());
    a[8..16].copy_from_slice(&size.to_le_bytes());
    a[60..64].copy_from_slice(&mode.to_le_bytes());
    a[64..68].copy_from_slice(&nlink.to_le_bytes());
    a[80..84].copy_from_slice(&4096u32.to_le_bytes());
    a
}

/// A `struct fuse_entry_out` valid for `valid` seconds.
fn entry(node: u64, mode: u32, size: u64, valid: u64) -> Vec<u8> {
    let mut e = vec![0u8; 40];
    e[0..8].copy_from_slice(&node.to_le_bytes());
    e[16..24].copy_from_slice(&valid.to_le_bytes());
    e[24..32].copy_from_slice(&valid.to_le_bytes());
    e.extend_from_slice(&attr(node, mode, size, 1));
    e
}

/// A `struct fuse_attr_out`.
fn attr_out(ino: u64, mode: u32, size: u64, valid: u64) -> Vec<u8> {
    let mut o = vec![0u8; 16];
    o[0..8].copy_from_slice(&valid.to_le_bytes());
    o.extend_from_slice(&attr(ino, mode, size, 1));
    o
}

/// A `struct fuse_open_out`.
fn open_out(fh: u64) -> Vec<u8> {
    let mut o = fh.to_le_bytes().to_vec();
    o.extend_from_slice(&[0u8; 8]);
    o
}

fn mounted(init: Option<(u32, u32, u16)>, opts: Options, handler: Handler) -> (Arc<TestEnv>, FuseFs) {
    let conn = Conn::new();
    let env = Arc::new(TestEnv {
        conn: conn.clone(),
        handler: Mutex::new(handler),
        log: Mutex::new(Vec::new()),
        now: AtomicU64::new(1_000_000_000),
        init: Mutex::new(init),
    });
    conn.bind(Caller { uid: 0, gid: 0, pid: 1, umask: 0o22 }).unwrap();
    let fs = FuseFs::new(conn, env.clone(), opts, "fuse");
    fs.finish_mount();
    (env, fs)
}

#[test]
fn a_failed_mount_ties_the_descriptor_to_it() {
    let who = Caller { uid: 0, gid: 0, pid: 1, umask: 0o22 };
    let conn = Conn::new();
    conn.fail_mount();
    assert!(conn.is_bound());
    assert_eq!(conn.bind(who), Err(Errno::ENOTCONN));
    // A connection of a live mount is not a failed one.
    let live = Conn::new();
    live.bind(who).unwrap();
    assert_eq!(live.bind(who), Err(Errno::EINVAL));
}

fn opts() -> Options {
    Options { root_mode: 0o40755, ..Options::default() }
}

/// A server with one 5-byte file, node 2, under every name looked up.
fn one_file() -> Handler {
    Box::new(|r: &Req| match r.op {
        op::LOOKUP => Some(Ok(entry(2, S_IFREG | 0o644, 5, 1000))),
        op::GETATTR => Some(Ok(attr_out(r.node, S_IFREG | 0o644, 5, 1000))),
        op::OPEN | op::OPENDIR => Some(Ok(open_out(77))),
        op::READ => {
            let (off, size) = (r.w64(8) as usize, r.w32(16) as usize);
            let data = b"hello";
            let from = off.min(data.len());
            Some(Ok(data[from..(from + size).min(data.len())].to_vec()))
        }
        op::WRITE => Some(Ok({
            let mut o = r.w32(16).to_le_bytes().to_vec();
            o.extend_from_slice(&[0u8; 4]);
            o
        })),
        op::RELEASE | op::RELEASEDIR | op::FLUSH | op::FSYNC => Some(Ok(Vec::new())),
        _ => Some(Err(ENOSYS)),
    })
}

#[test]
fn an_open_file_carries_the_servers_handle_to_read_flush_and_release() {
    let (env, fs) = mounted(Some((0, 65536, 0)), opts(), one_file());
    assert_eq!(fs.lookup(ROOT_ID, b"f"), Ok(2));
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    let h2 = fs.open_handle(2, OpenFlags::RDWR, false).unwrap().unwrap();
    assert_ne!(h, h2);
    let mut buf = [0u8; 16];
    assert_eq!(fs.read_handle(2, h, 0, &mut buf), Ok(5));
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(fs.read_handle(2, h, 3, &mut buf), Ok(2));
    assert_eq!(fs.flush_handle(2, h, true), Ok(()));
    fs.close_handle(2, h, OpenFlags::RDONLY, false);
    // RELEASE waits for nobody: the server finds it at its next read.
    assert!(env.pump());
    assert_eq!(env.ops(), [op::INIT, op::LOOKUP, op::OPEN, op::OPEN, op::READ, op::READ, op::FLUSH, op::RELEASE]);
    let opens = env.of(op::OPEN);
    assert_eq!((opens[0].node, opens[0].w32(0) & 3, opens[1].w32(0) & 3), (2, 0, 2));
    let reads = env.of(op::READ);
    assert_eq!((reads[0].w64(0), reads[0].w64(8), reads[0].w32(16)), (77, 0, 16));
    assert_eq!((reads[1].w64(0), reads[1].w64(8)), (77, 3));
    assert_eq!(env.of(op::FLUSH)[0].w64(0), 77);
    let rel = &env.of(op::RELEASE)[0];
    assert_eq!((rel.node, rel.w64(0), rel.uid), (2, 77, 1000));
    // A handle that was given up is not one any more.
    assert_eq!(fs.read_handle(2, h, 0, &mut buf), Err(Errno::EBADF));
}

#[test]
fn enosys_to_open_is_remembered_and_leaves_fh_zero_without_release() {
    let mut inner = one_file();
    let (env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(move |r: &Req| if r.op == op::OPEN { Some(Err(ENOSYS)) } else { inner(r) }),
    );
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(fs.read_handle(2, h, 0, &mut buf), Ok(5));
    assert_eq!(fs.flush_handle(2, h, true), Ok(()));
    fs.close_handle(2, h, OpenFlags::RDONLY, false);
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    fs.close_handle(2, h, OpenFlags::RDONLY, false);
    assert!(!env.pump(), "no RELEASE without an OPEN");
    assert_eq!(env.ops(), [op::INIT, op::OPEN, op::READ, op::FLUSH]);
    assert_eq!(env.of(op::READ)[0].w64(0), 0);
    assert_eq!(env.of(op::FLUSH)[0].w64(0), 0);
}

#[test]
fn an_error_reply_to_open_or_read_is_the_calls_error() {
    let mut inner = one_file();
    let fail = Arc::new(AtomicU64::new(u64::from(op::OPEN)));
    let which = fail.clone();
    let (_env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(
            move |r: &Req| {
                if u64::from(r.op) == which.load(Ordering::Relaxed) { Some(Err(EIO)) } else { inner(r) }
            },
        ),
    );
    assert_eq!(fs.open_handle(2, OpenFlags::RDONLY, false), Err(Errno::EIO));
    fail.store(u64::from(op::READ), Ordering::Relaxed);
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    assert_eq!(fs.read_handle(2, h, 0, &mut [0u8; 4]), Err(Errno::EIO));
}

#[test]
fn writes_are_split_at_max_write_and_a_short_reply_ends_them() {
    let short = Arc::new(AtomicU64::new(0));
    let cut = short.clone();
    let (env, fs) = mounted(
        Some((0, 8192, 0)),
        opts(),
        Box::new(move |r: &Req| match r.op {
            op::OPEN => Some(Ok(open_out(9))),
            op::WRITE => {
                let mut n = r.w32(16);
                assert_eq!(r.body.len(), 40 + n as usize, "the data follow the 40-byte body");
                let limit = cut.load(Ordering::Relaxed) as u32;
                if limit != 0 {
                    n = n.min(limit);
                }
                let mut o = n.to_le_bytes().to_vec();
                o.extend_from_slice(&[0u8; 4]);
                Some(Ok(o))
            }
            _ => Some(Err(ENOSYS)),
        }),
    );
    let h = fs.open_handle(2, OpenFlags::WRONLY, false).unwrap().unwrap();
    let data: Vec<u8> = (0..20000u32).map(|i| (i * 31 + 7) as u8).collect();
    assert_eq!(fs.write_handle(2, h, 100, &data), Ok(20000));
    let w = env.of(op::WRITE);
    let seen: Vec<(u64, u64, u32, u32)> = w.iter().map(|r| (r.w64(0), r.w64(8), r.w32(16), r.w32(20))).collect();
    assert_eq!(seen, [(9, 100, 8192, 0), (9, 8292, 8192, 0), (9, 16484, 3616, 0)]);
    assert_eq!(&w[1].body[40..], &data[8192..16384]);
    short.store(4, Ordering::Relaxed);
    assert_eq!(fs.write_handle(2, h, 0, b"0123456789"), Ok(4));
    assert_eq!(env.of(op::WRITE).len(), 4);
}

#[test]
fn reads_stay_within_max_read_and_max_pages() {
    let big = |r: &Req| match r.op {
        op::OPEN => Some(Ok(open_out(1))),
        op::READ => Some(Ok(vec![7u8; r.w32(16) as usize])),
        _ => Some(Err(ENOSYS)),
    };
    let (env, fs) = mounted(Some((0, 65536, 0)), Options { max_read: 4096, ..opts() }, Box::new(big));
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    let mut buf = vec![0u8; 10000];
    assert_eq!(fs.read_handle(2, h, 0, &mut buf), Ok(10000));
    let sizes: Vec<(u64, u32)> = env.of(op::READ).iter().map(|r| (r.w64(8), r.w32(16))).collect();
    assert_eq!(sizes, [(0, 4096), (4096, 4096), (8192, 1808)]);

    // FUSE_MAX_PAGES in the reply: two pages for reads and writes.
    let (env, fs) = mounted(Some((FUSE_MAX_PAGES, 65536, 2)), opts(), Box::new(big));
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    assert_eq!(fs.read_handle(2, h, 0, &mut buf), Ok(10000));
    assert!(env.of(op::READ).iter().all(|r| r.w32(16) <= 8192));
    assert_eq!(fs.conn.max_write(), 8192);

    // No limits: 131072 for a read, 65536 (max_write) for a write.
    let (env, fs) = mounted(Some((0, 65536, 0)), opts(), Box::new(big));
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    let mut buf = vec![0u8; 200_000];
    assert_eq!(fs.read_handle(2, h, 0, &mut buf), Ok(200_000));
    assert_eq!(env.of(op::READ)[0].w32(16), 131_072);
    assert_eq!(fs.conn.max_write(), 65536);
}

#[test]
fn create_opens_the_new_file_in_one_request() {
    let (env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(|r: &Req| match r.op {
            op::CREATE => {
                let mut o = entry(8, S_IFREG | 0o644, 0, 1000);
                o.extend_from_slice(&open_out(101));
                Some(Ok(o))
            }
            op::WRITE => Some(Ok(vec![3, 0, 0, 0, 0, 0, 0, 0])),
            _ => Some(Err(ENOSYS)),
        }),
    );
    let node = NewNode { kind: FileType::Regular, perm: 0o644, uid: 0, gid: 0, rdev: DeviceId::default() };
    let flags = OpenFlags::RDWR | OpenFlags::CREAT | OpenFlags::EXCL;
    let (ino, h) = fs.create_open(ROOT_ID, b"newfile", &node, flags).unwrap().unwrap();
    assert_eq!(ino, 8);
    assert_eq!(fs.write_handle(ino, h, 0, b"abc"), Ok(3));
    let c = &env.of(op::CREATE)[0];
    assert_eq!(c.node, ROOT_ID);
    assert_eq!(c.w32(0), (OpenFlags::RDWR | OpenFlags::CREAT | OpenFlags::EXCL).bits());
    assert_eq!((c.w32(4), c.w32(8)), (S_IFREG | 0o644, 0o22));
    assert_eq!(c.names(16), [b"newfile".to_vec()]);
    assert_eq!(env.of(op::WRITE)[0].w64(0), 101);
    assert_eq!(env.of(op::OPEN).len(), 0);
    // The reply's attributes are the file's: no request for them.
    assert_eq!(fs.getattr(8).map(|a| (a.kind, a.perm)), Ok((FileType::Regular, 0o644)));
    assert_eq!(fs.lookup(ROOT_ID, b"newfile"), Ok(8));
    assert_eq!(env.of(op::GETATTR).len() + env.of(op::LOOKUP).len(), 0);
}

#[test]
fn a_server_without_create_gets_mknod_then_open() {
    let (env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(|r: &Req| match r.op {
            op::MKNOD => Some(Ok(entry(8, S_IFREG | 0o600, 0, 0))),
            _ => Some(Err(ENOSYS)),
        }),
    );
    let node = NewNode { kind: FileType::Regular, perm: 0o600, uid: 0, gid: 0, rdev: DeviceId::default() };
    assert_eq!(fs.create_open(ROOT_ID, b"f", &node, OpenFlags::WRONLY | OpenFlags::CREAT), Ok(None));
    assert_eq!(fs.create(ROOT_ID, b"f", &node), Ok(8));
    assert_eq!(fs.create_open(ROOT_ID, b"g", &node, OpenFlags::WRONLY | OpenFlags::CREAT), Ok(None));
    assert_eq!(env.ops(), [op::INIT, op::CREATE, op::MKNOD]);
}

#[test]
fn names_are_made_removed_and_moved_by_their_requests() {
    let (env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(|r: &Req| match r.op {
            op::MKDIR => Some(Ok(entry(8, S_IFDIR | 0o755, 0, 1000))),
            op::MKNOD => {
                // The new node's attributes carry the device number asked for.
                let mut e = entry(9, r.w32(0), 0, 1000);
                e[40 + 76..40 + 80].copy_from_slice(&r.w32(4).to_le_bytes());
                Some(Ok(e))
            }
            op::SYMLINK => Some(Ok(entry(10, 0o120777, 7, 1000))),
            op::LINK => Some(Ok(entry(r.w64(0), S_IFREG | 0o644, 3, 1000))),
            op::LOOKUP if r.names(0)[0] == b"taken" => Some(Ok(entry(11, S_IFREG | 0o644, 0, 1000))),
            op::LOOKUP => Some(Err(ENOENT)),
            op::UNLINK | op::RMDIR | op::RENAME | op::RENAME2 => Some(Ok(Vec::new())),
            _ => Some(Err(ENOSYS)),
        }),
    );
    assert_eq!(fs.mkdir(ROOT_ID, b"d", 0o755, 0, 0), Ok(8));
    let md = &env.of(op::MKDIR)[0];
    assert_eq!((md.node, md.w32(0), md.w32(4), md.names(8)), (ROOT_ID, 0o755, 0o22, vec![b"d".to_vec()]));
    assert_eq!(fs.lookup(8, b".."), Ok(ROOT_ID));

    let chr = NewNode { kind: FileType::CharDevice, perm: 0o600, uid: 0, gid: 0, rdev: DeviceId::new(1, 3) };
    assert_eq!(fs.create(8, b"null", &chr), Ok(9));
    let mk = &env.of(op::MKNOD)[0];
    assert_eq!((mk.node, mk.w32(0), mk.w32(4), mk.w32(8)), (8, 0o020600, 0x103, 0o22));
    assert_eq!(fs.getattr(9).map(|a| a.rdev), Ok(DeviceId::new(1, 3)));

    assert_eq!(fs.symlink(ROOT_ID, b"sl", b"newfile", 0, 0), Ok(10));
    assert_eq!(env.of(op::SYMLINK)[0].names(0), [b"sl".to_vec(), b"newfile".to_vec()]);

    assert_eq!(fs.link(ROOT_ID, b"hard", 9), Ok(()));
    let li = &env.of(op::LINK)[0];
    assert_eq!((li.node, li.w64(0), li.names(8)), (ROOT_ID, 9, vec![b"hard".to_vec()]));

    assert_eq!(fs.rename(ROOT_ID, b"sl", 8, b"moved", RenameFlags::default()), Ok(()));
    let rn = &env.of(op::RENAME)[0];
    assert_eq!((rn.node, rn.w64(0), rn.names(8)), (ROOT_ID, 8, vec![b"sl".to_vec(), b"moved".to_vec()]));

    let exchange = RenameFlags { exchange: true, noreplace: false };
    assert_eq!(fs.rename(ROOT_ID, b"a", ROOT_ID, b"b", exchange), Ok(()));
    let r2 = &env.of(op::RENAME2)[0];
    assert_eq!((r2.w64(0), r2.w32(8), r2.names(16)), (ROOT_ID, 2, vec![b"a".to_vec(), b"b".to_vec()]));

    // RENAME_NOREPLACE onto a name that exists: EEXIST, and no request.
    let noreplace = RenameFlags { exchange: false, noreplace: true };
    assert_eq!(fs.rename(ROOT_ID, b"a", ROOT_ID, b"taken", noreplace), Err(Errno::EEXIST));
    assert_eq!(env.of(op::RENAME2).len(), 1);
    assert_eq!(fs.rename(ROOT_ID, b"a", ROOT_ID, b"free", noreplace), Ok(()));
    assert_eq!(env.of(op::RENAME2)[1].w32(8), 1);

    // A removed name is asked for again.
    assert_eq!(fs.lookup(ROOT_ID, b"hard"), Ok(9));
    assert_eq!(fs.unlink(ROOT_ID, b"hard"), Ok(()));
    assert_eq!(env.of(op::UNLINK)[0].names(0), [b"hard".to_vec()]);
    assert_eq!(fs.lookup(ROOT_ID, b"hard"), Err(Errno::ENOENT));
    assert_eq!(fs.rmdir(ROOT_ID, b"d"), Ok(()));
    assert_eq!(env.of(op::RMDIR)[0].names(0), [b"d".to_vec()]);
}

#[test]
fn setattr_names_the_fields_it_sets() {
    let (env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(|r: &Req| match r.op {
            op::LOOKUP => Some(Ok(entry(2, S_IFREG | 0o644, 5, 1000))),
            op::OPEN => Some(Ok(open_out(55))),
            op::SETATTR => Some(Ok(attr_out(r.node, S_IFREG | (r.w32(68) & 0o7777), r.w64(16), 1000))),
            _ => Some(Err(ENOSYS)),
        }),
    );
    assert_eq!(fs.lookup(ROOT_ID, b"f"), Ok(2));
    let a = fs.setattr(2, &SetAttr { perm: Some(0o600), ..SetAttr::default() }).unwrap();
    assert_eq!(a.perm, 0o600);
    fs.setattr(2, &SetAttr { gid: Some(7), ..SetAttr::default() }).unwrap();
    fs.truncate(2, 3).unwrap();
    let h = fs.open_handle(2, OpenFlags::RDWR, false).unwrap().unwrap();
    fs.truncate_handle(2, h, 1).unwrap();
    // The reply's attributes are the file's.
    assert_eq!(fs.getattr(2).map(|a| a.size), Ok(1));
    let at = SetTime::At(Timespec::new(1_700_000_000, 5));
    fs.setattr(2, &SetAttr { atime: Some(SetTime::Now), mtime: Some(at), ..SetAttr::default() }).unwrap();
    let s = env.of(op::SETATTR);
    assert_eq!((s[0].w32(0), s[0].w32(68)), (fattr::MODE, S_IFREG | 0o600));
    assert_eq!((s[1].w32(0), s[1].w32(80)), (fattr::GID, 7));
    assert_eq!((s[2].w32(0), s[2].w64(16)), (fattr::SIZE, 3));
    assert_eq!((s[3].w32(0), s[3].w64(8), s[3].w64(16)), (fattr::SIZE | fattr::FH, 55, 1));
    assert_eq!(s[4].w32(0), fattr::ATIME | fattr::ATIME_NOW | fattr::MTIME);
    assert_eq!((s[4].w64(40), s[4].w32(60)), (1_700_000_000, 5));
    assert!(s.iter().all(|r| r.body.len() == 88));
}

#[test]
fn o_trunc_goes_into_the_open_only_when_the_server_asked_for_it() {
    let (env, fs) = mounted(Some((0, 65536, 0)), opts(), one_file());
    assert_eq!(fs.open_truncates(), OpenTrunc::Always);
    drop((env, fs));
    let (env, fs) = mounted(Some((FUSE_ATOMIC_O_TRUNC, 65536, 0)), opts(), one_file());
    // The handshake is over once the first call went through.
    assert_eq!(fs.lookup(ROOT_ID, b"f"), Ok(2));
    assert_eq!(fs.open_truncates(), OpenTrunc::InOpen);
    fs.open_handle(2, OpenFlags::WRONLY | OpenFlags::TRUNC | OpenFlags::CREAT, false).unwrap();
    assert_eq!(env.of(op::OPEN)[0].w32(0), (OpenFlags::WRONLY | OpenFlags::TRUNC).bits());
}

#[test]
fn directories_are_listed_through_their_open() {
    let (env, fs) = mounted(
        Some((0, 65536, 0)),
        opts(),
        Box::new(|r: &Req| match r.op {
            op::OPENDIR => Some(Ok(open_out(31))),
            op::READDIR => {
                // Three entries, two per reply.
                let names: [&[u8]; 3] = [b"a", b"bb", b"ccc"];
                let mut out = Vec::new();
                for (i, n) in names.iter().enumerate().skip(r.w64(8) as usize).take(2) {
                    out.extend_from_slice(&(100 + i as u64).to_le_bytes());
                    out.extend_from_slice(&(i as u64 + 1).to_le_bytes());
                    out.extend_from_slice(&(n.len() as u32).to_le_bytes());
                    out.extend_from_slice(&8u32.to_le_bytes());
                    out.extend_from_slice(n);
                    out.resize(out.len().div_ceil(8) * 8, 0);
                }
                Some(Ok(out))
            }
            op::RELEASEDIR => Some(Ok(Vec::new())),
            _ => Some(Err(ENOSYS)),
        }),
    );
    let h = fs.open_handle(ROOT_ID, OpenFlags::RDONLY | OpenFlags::DIRECTORY, true).unwrap().unwrap();
    let mut seen = Vec::new();
    fs.readdir_handle(ROOT_ID, h, 0, &mut |e| {
        seen.push((e.ino, e.name.to_vec(), e.next));
        true
    })
    .unwrap();
    assert_eq!(seen, [(100, b"a".to_vec(), 1), (101, b"bb".to_vec(), 2), (102, b"ccc".to_vec(), 3)]);
    let offsets: Vec<(u64, u64)> = env.of(op::READDIR).iter().map(|r| (r.w64(0), r.w64(8))).collect();
    assert_eq!(offsets, [(31, 0), (31, 2), (31, 3)]);
    fs.close_handle(ROOT_ID, h, OpenFlags::RDONLY, true);
    assert!(env.pump());
    assert_eq!(env.of(op::RELEASEDIR)[0].w64(0), 31);
}

#[test]
fn fsync_says_whether_only_the_data_are_meant() {
    let (env, fs) = mounted(Some((0, 65536, 0)), opts(), one_file());
    let h = fs.open_handle(2, OpenFlags::WRONLY, false).unwrap().unwrap();
    assert_eq!(fs.sync_handle(2, h, false, false), Ok(()));
    assert_eq!(fs.sync_handle(2, h, true, false), Ok(()));
    let f = env.of(op::FSYNC);
    assert_eq!((f[0].w64(0), f[0].w32(8), f[1].w32(8)), (77, 0, 1));
}

#[test]
fn the_root_needs_no_answer_before_init_and_a_release_outlives_the_mount() {
    // No INIT reply: the root is the directory the mount options describe.
    let (env, fs) =
        mounted(None, Options { root_mode: 0o40700, user_id: 5, group_id: 6, ..Options::default() }, one_file());
    let a = fs.getattr(ROOT_ID).unwrap();
    assert_eq!((a.kind, a.perm, a.uid, a.gid), (FileType::Directory, 0o700, 5, 6));
    // Everything else waits for the handshake (here: for ever).
    assert_eq!(fs.lookup(ROOT_ID, b"f"), Err(Errno::EINTR));
    assert_eq!(env.ops(), [op::INIT]);

    // A RELEASE queued when the file system goes is still read; then ENODEV.
    let (env, fs) = mounted(Some((0, 65536, 0)), opts(), one_file());
    let h = fs.open_handle(2, OpenFlags::RDONLY, false).unwrap().unwrap();
    fs.close_handle(2, h, OpenFlags::RDONLY, false);
    let conn = fs.conn.clone();
    drop(fs);
    assert!(env.pump());
    assert_eq!(env.ops().last(), Some(&op::RELEASE));
    assert_eq!(conn.read(&*env, &mut [0u8; 8192], true), Err(Errno::ENODEV));
}

#[test]
fn mount_options_need_fd_rootmode_and_the_owner() {
    let o = parse_options(b"fd=7,rootmode=40755,user_id=5,group_id=6,max_read=65536,default_permissions,allow_other")
        .unwrap();
    assert_eq!((o.fd, o.opts.root_mode, o.opts.user_id, o.opts.group_id), (7, 0o40755, 5, 6));
    assert_eq!((o.opts.max_read, o.opts.default_permissions, o.opts.allow_other), (65536, true, true));
    assert_eq!(o.opts.root_kind(), FileType::Directory);
    assert_eq!(parse_options(b"fd=3,rootmode=040700,user_id=0,group_id=0,subtype=pk").unwrap().subtype.unwrap(), b"pk");
    for missing in [
        &b"rootmode=40000,user_id=0,group_id=0"[..],
        b"fd=3,user_id=0,group_id=0",
        b"fd=3,rootmode=40000,group_id=0",
        b"fd=3,rootmode=40000,user_id=0",
        b"",
    ] {
        assert_eq!(parse_options(missing).unwrap_err(), Errno::EINVAL, "{:?}", core::str::from_utf8(missing));
    }
}

#[test]
fn mount_options_refuse_bad_numbers_types_and_foreign_options() {
    let with = |rest: &str| parse_options(std::format!("fd=3{rest}").as_bytes()).map(|o| o.opts.root_kind());
    let ids = ",user_id=0,group_id=0";
    // Every file type but a symbolic link is a root; permission bits do not matter.
    for (mode, kind) in [
        ("100000", FileType::Regular),
        ("100644", FileType::Regular),
        ("40000", FileType::Directory),
        ("20000", FileType::CharDevice),
        ("60000", FileType::BlockDevice),
        ("10000", FileType::Fifo),
        ("140000", FileType::Socket),
    ] {
        assert_eq!(with(&std::format!(",rootmode={mode}{ids}")), Ok(kind), "{mode}");
    }
    for mode in ["0", "777", "170000", "30000", "120000", "120777", "8", "4x", "", "-40000", "+40000", "0o40000"] {
        assert_eq!(with(&std::format!(",rootmode={mode}{ids}")), Err(Errno::EINVAL), "{mode}");
    }
    for rest in [
        ",rootmode=40000,user_id=root,group_id=0",
        ",rootmode=40000,user_id=0,group_id=",
        ",rootmode=40000,user_id=-2,group_id=0",
        ",rootmode=40000,user_id=0,group_id=0,blksize=512",
        ",rootmode=40000,user_id=0,group_id=0,blksize=big",
        ",rootmode=40000,user_id=0,group_id=0,bogus=1",
        ",rootmode=40000,user_id=0,group_id=0,max_read=many",
        ",rootmode=40000,user_id=0,group_id=0,max_read=",
        ",rootmode=40000,user_id=0,group_id=0,subtype",
        ",rootmode=40000,user_id=4294967296,group_id=0",
    ] {
        assert_eq!(with(rest), Err(Errno::EINVAL), "{rest}");
    }
    assert_eq!(parse_options(b"fd=x,rootmode=40000,user_id=0,group_id=0").unwrap_err(), Errno::EINVAL);
}

#[test]
fn a_root_that_is_no_directory_shows_its_type() {
    let (_env, fs) = mounted(None, Options { root_mode: 0o100644, ..Options::default() }, one_file());
    let a = fs.walk_attr(ROOT_ID).unwrap();
    assert_eq!((a.kind, a.nlink), (FileType::Regular, 1));
}
