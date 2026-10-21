//! Pseudo-terminals through a real VFS: `/dev/ptmx`, devpts on
//! `/dev/pts`, masters and slaves talking, blocking on each other, closing.

use super::fake::{FakeHost, Target, USER, act_as, caller};
use crate::devpts::DevPts;
use crate::ioctl::*;
use crate::ldisc::INPUT_BUFFER;
use crate::pty::{MASTER_BUFFER, PTMX, PTY_SLAVE_MAJOR, Ptys};
use crate::sig::{SIGCONT, SIGHUP, SIGINT, SIGWINCH};
use crate::termios::{Termios, Winsize, cc, lflag};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{
    At, Clock, Cred, DeviceId, Errno, FileType, FsContext, MountFlags, OpenFile, OpenFlags, PollEvents, Timespec,
    Tmpfs, Vfs,
};
use std::sync::Mutex;
use std::thread;

struct Epoch;

impl Clock for Epoch {
    fn now(&self) -> Timespec {
        Timespec::default()
    }
}

struct Env {
    host: Arc<FakeHost>,
    ptys: Arc<Ptys>,
    vfs: &'static Vfs,
    ctx: FsContext,
}

impl Env {
    fn new() -> Env {
        let host = FakeHost::new();
        let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(Epoch))))));
        let ctx = vfs.root_context().unwrap();
        let ptys = Ptys::new(&host.ttys(), vfs);
        let root = Cred::root();
        vfs.register_char_device(PTMX, ptys.ptmx()).unwrap();
        vfs.mkdir(&ctx, &root, At::Cwd, b"/dev", 0o755).unwrap();
        vfs.mkdir(&ctx, &root, At::Cwd, b"/dev/pts", 0o755).unwrap();
        vfs.mknod(&ctx, &root, At::Cwd, b"/dev/ptmx", FileType::CharDevice, 0o666, PTMX).unwrap();
        let devpts = Arc::new(DevPts::new(ptys.clone()));
        vfs.mount(&ctx, &root, At::Cwd, b"/dev/pts", devpts, MountFlags::default()).unwrap();
        act_as(Some(caller(10, 10, 10)));
        Env { host, ptys, vfs, ctx }
    }

    fn open(&self, path: &str, flags: OpenFlags) -> Result<Arc<OpenFile>, Errno> {
        self.vfs.open(&self.ctx, &Cred::root(), At::Cwd, path.as_bytes(), flags, 0)
    }

    /// A new master, unlocked, and its number.
    fn master(&self) -> (Arc<OpenFile>, u32) {
        let m = self.open("/dev/ptmx", OpenFlags::RDWR).unwrap();
        assert_eq!(m.ioctl(TIOCGPTN, USER), Ok(0));
        let n = self.host.peek_int(USER) as u32;
        self.host.poke(USER, &0i32.to_le_bytes());
        assert_eq!(m.ioctl(TIOCSPTLCK, USER), Ok(0));
        (m, n)
    }

    fn slave(&self, n: u32, flags: OpenFlags) -> Result<Arc<OpenFile>, Errno> {
        self.open(&alloc::format!("/dev/pts/{n}"), flags)
    }

    fn list(&self) -> Vec<String> {
        let dir = self.open("/dev/pts", OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap();
        let mut names = Vec::new();
        dir.read_dir(&mut |e| {
            names.push(String::from_utf8_lossy(e.name).into_owned());
            true
        })
        .unwrap();
        names
    }
}

fn read_some(f: &OpenFile, max: usize) -> Result<Vec<u8>, Errno> {
    let mut buf = vec![0u8; max];
    f.read(&mut buf).map(|n| buf[..n].to_vec())
}

fn nonblocking(f: &OpenFile) {
    f.set_flags(f.flags() | OpenFlags::NONBLOCK);
}

#[test]
fn ptmx_makes_pairs_and_devpts_lists_them() {
    let env = Env::new();
    let m0 = env.open("/dev/ptmx", OpenFlags::RDWR).unwrap();
    assert_eq!(m0.ioctl(TIOCGPTN, USER), Ok(0));
    assert_eq!(env.host.peek_int(USER), 0);
    assert_eq!(m0.stat().unwrap().rdev, PTMX, "the master is the ptmx node");
    assert_eq!(env.list(), [".", "..", "0"]);
    let st = env.vfs.stat(&env.ctx, &Cred::root(), At::Cwd, b"/dev/pts/0", true).unwrap();
    assert_eq!(st.kind(), Some(FileType::CharDevice));
    assert_eq!(st.rdev, DeviceId::new(PTY_SLAVE_MAJOR, 0));
    assert_eq!((st.perm(), st.uid, st.gid), (0o620, 1000, 1000), "owned by the opener (grantpt)");
    // Locked until unlockpt.
    assert_eq!(m0.ioctl(TIOCGPTLCK, USER), Ok(0));
    assert_eq!(env.host.peek_int(USER), 1);
    assert_eq!(env.slave(0, OpenFlags::RDWR).map(|_| ()), Err(Errno::EIO));
    env.host.poke(USER, &0i32.to_le_bytes());
    assert_eq!(m0.ioctl(TIOCSPTLCK, USER), Ok(0));
    let s0 = env.slave(0, OpenFlags::RDWR | OpenFlags::NOCTTY).unwrap();
    assert_eq!(s0.ioctl(TCGETS, USER), Ok(0), "the slave is a terminal");
    assert_eq!(s0.ioctl(TIOCGPTN, USER), Err(Errno::ENOTTY), "the master's requests are not the slave's");
    let (m1, n1) = env.master();
    assert_eq!(n1, 1);
    assert_eq!(env.list(), [".", "..", "0", "1"]);
    assert_eq!(env.slave(7, OpenFlags::RDWR).map(|_| ()), Err(Errno::ENOENT));
    // Closing a master frees its number and its node.
    drop(m0);
    assert_eq!(env.list(), [".", "..", "1"]);
    assert_eq!(env.slave(0, OpenFlags::RDWR).map(|_| ()), Err(Errno::ENOENT));
    let (_m, n) = env.master();
    assert_eq!(n, 0, "the lowest free number again");
    assert_eq!(env.ptys.indices(), [0, 1]);
    // chmod / chown on a node change the slave's owner.
    env.vfs.chmod(&env.ctx, &Cred::root(), At::Cwd, b"/dev/pts/1", 0o600, true).unwrap();
    let st = env.vfs.stat(&env.ctx, &Cred::root(), At::Cwd, b"/dev/pts/1", true).unwrap();
    assert_eq!(st.perm(), 0o600);
    drop((s0, m1));
}

#[test]
fn master_and_slave_talk() {
    let env = Env::new();
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR).unwrap();
    nonblocking(&m);
    nonblocking(&s);
    // Typed on the master: echoed back to it, a line for the slave.
    assert_eq!(m.write(b"hi\r"), Ok(3));
    assert_eq!(read_some(&s, 100).unwrap(), b"hi\n");
    assert_eq!(read_some(&m, 100).unwrap(), b"hi\r\n");
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN));
    // Written on the slave: output processing, then the master.
    assert_eq!(s.write(b"out\n"), Ok(4));
    assert_eq!(m.ioctl(FIONREAD, USER), Ok(0));
    assert_eq!(env.host.peek_int(USER), 5);
    assert!(m.poll().contains(PollEvents::IN));
    assert_eq!(read_some(&m, 100).unwrap(), b"out\r\n");
    assert!(!m.poll().contains(PollEvents::IN) && m.poll().contains(PollEvents::OUT));
    // Raw mode set through the master applies to the slave.
    let mut t = Termios::sane(38400);
    t.lflag &= !(lflag::ICANON | lflag::ECHO | lflag::ISIG);
    t.cc[cc::VMIN] = 1;
    env.host.poke(USER, &t.to_bytes());
    assert_eq!(m.ioctl(TCSETS, USER), Ok(0));
    assert_eq!(s.ioctl(TCGETS, USER + 64), Ok(0));
    assert_eq!(env.host.peek(USER + 64, Termios::SIZE), t.to_bytes());
    assert_eq!(m.write(b"\x03k"), Ok(2));
    assert_eq!(read_some(&s, 100).unwrap(), b"\x03k");
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN), "no echo");
    // Flushing from the master: its input (the slave's output) and the
    // slave's input.
    s.write(b"zz").unwrap();
    m.write(b"yy").unwrap();
    assert_eq!(m.ioctl(TCFLSH, 2), Ok(0));
    assert_eq!(read_some(&m, 10), Err(Errno::EAGAIN));
    assert_eq!(read_some(&s, 10), Err(Errno::EAGAIN));
}

#[test]
fn a_blocked_slave_reader_wakes_for_the_master() {
    let env = Env::new();
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR).unwrap();
    let reader = thread::spawn(move || {
        act_as(Some(caller(10, 10, 10)));
        read_some(&s, 100)
    });
    env.host.wait_blocked(1);
    assert_eq!(m.write(b"line\r"), Ok(5));
    assert_eq!(reader.join().unwrap().unwrap(), b"line\n");
    nonblocking(&m);
    assert_eq!(read_some(&m, 100).unwrap(), b"line\r\n", "the echo");
}

#[test]
fn writers_wait_for_room_on_either_side() {
    let env = Env::new();
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR).unwrap();
    let mut t = Termios::sane(38400);
    t.lflag &= !(lflag::ICANON | lflag::ECHO);
    env.host.poke(USER, &t.to_bytes());
    assert_eq!(s.ioctl(TCSETS, USER), Ok(0));
    // The master types more than the slave's input buffer holds.
    let big = INPUT_BUFFER + 100;
    let writer = {
        let m = m.clone();
        thread::spawn(move || {
            act_as(Some(caller(10, 10, 10)));
            m.write(&vec![b'a'; big])
        })
    };
    env.host.wait_blocked(1);
    assert!(!m.poll().contains(PollEvents::OUT));
    let mut got = 0;
    while got < big {
        got += read_some(&s, 1000).unwrap().len();
    }
    assert_eq!(writer.join().unwrap(), Ok(big));
    // The slave writes more than the master buffers.
    let big = MASTER_BUFFER + 10;
    let writer = {
        let s = s.clone();
        thread::spawn(move || {
            act_as(Some(caller(10, 10, 10)));
            s.write(&vec![b'b'; big])
        })
    };
    env.host.wait_blocked(1);
    let mut got = 0;
    while got < big {
        got += read_some(&m, 4096).unwrap().len();
    }
    assert_eq!(writer.join().unwrap(), Ok(big));
}

#[test]
fn closing_either_side() {
    let env = Env::new();
    let (m, n) = env.master();
    nonblocking(&m);
    assert_eq!(read_some(&m, 10), Err(Errno::EAGAIN), "no slave yet: nothing to read");
    let s = env.slave(n, OpenFlags::RDWR).unwrap();
    drop(s);
    assert_eq!(read_some(&m, 10), Err(Errno::EIO), "the slave closed");
    assert!(m.poll().contains(PollEvents::HUP));
    // Opened again, it is back.
    let s = env.slave(n, OpenFlags::RDWR).unwrap();
    assert_eq!(read_some(&m, 10), Err(Errno::EAGAIN));
    // The slave as a session's controlling terminal; then the master
    // closes: the session leader gets SIGHUP, the foreground group SIGHUP
    // and SIGCONT; reads see end of file, writes EIO.
    env.host.groups.lock().unwrap().insert(10, 10);
    let ctl = env.slave(n, OpenFlags::RDWR).unwrap();
    assert_eq!(ctl.ioctl(TIOCGSID, USER), Ok(0));
    assert_eq!(env.host.peek_int(USER), 10);
    assert_eq!(m.ioctl(TIOCGSID, USER), Ok(0), "also through the master");
    env.host.take_signals();
    // The signals go out before the hang-up shows: a reader that sees end
    // of file already has its SIGHUP pending.
    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let (s, seen) = (Arc::clone(&s), Arc::clone(&seen));
        *env.host.on_signal.lock().unwrap() =
            Some(Box::new(move || seen.lock().unwrap().push(s.poll().contains(PollEvents::HUP))));
    }
    drop(m);
    *env.host.on_signal.lock().unwrap() = None;
    assert_eq!(*seen.lock().unwrap(), [false, false], "not hung up while the signals are sent");
    assert_eq!(
        env.host.take_signals(),
        [(Target::Group(10), SIGHUP), (Target::Group(10), SIGCONT)],
        "the leader is in the foreground group: one SIGHUP"
    );
    assert_eq!(read_some(&s, 10), Ok(vec![]));
    assert_eq!(s.write(b"x"), Err(Errno::EIO));
    assert!(s.poll().contains(PollEvents::HUP));
    assert_eq!(env.slave(n, OpenFlags::RDWR).map(|_| ()), Err(Errno::ENOENT));
    drop(ctl);
}

#[test]
fn signals_and_window_size_through_the_master() {
    let env = Env::new();
    let (m, n) = env.master();
    env.host.groups.lock().unwrap().insert(10, 10);
    let s = env.slave(n, OpenFlags::RDWR).unwrap();
    assert_eq!(m.ioctl(TIOCGPGRP, USER), Ok(0));
    assert_eq!(env.host.peek_int(USER), 10);
    // Ctrl-C typed on the master reaches the slave's foreground group.
    m.write(b"\x03").unwrap();
    assert_eq!(env.host.take_signals(), [(Target::Group(10), SIGINT)]);
    env.host.poke(USER, &Winsize::new(30, 100).to_bytes());
    assert_eq!(m.ioctl(TIOCSWINSZ, USER), Ok(0));
    assert_eq!(env.host.take_signals(), [(Target::Group(10), SIGWINCH)]);
    assert_eq!(s.ioctl(TIOCGWINSZ, USER + 16), Ok(0));
    assert_eq!(Winsize::from_bytes(&env.host.peek(USER + 16, 8).try_into().unwrap()), Winsize::new(30, 100));
    assert_eq!(m.ioctl(TIOCSIG, 10), Ok(0));
    assert_eq!(env.host.take_signals(), [(Target::Group(10), 10)]);
    assert_eq!(m.ioctl(TIOCSIG, 0), Err(Errno::EINVAL));
}
