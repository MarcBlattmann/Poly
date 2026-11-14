//! Pseudo-terminals through a real VFS: `/dev/ptmx`, devpts on
//! `/dev/pts`, masters and slaves talking, blocking on each other, closing.

use super::fake::{FakeHost, Target, USER, act_as, caller};
use crate::devpts::DevPts;
use crate::ioctl::*;
use crate::ldisc::INPUT_BUFFER;
use crate::pty::{MASTER_BUFFER, PTMX, PTY_SLAVE_MAJOR, Ptys};
use crate::sig::{SIGCONT, SIGHUP, SIGINT, SIGWINCH};
use crate::termios::{TCIFLUSH, TCIOFLUSH, TCOFLUSH, TCOOFF, TCOON, Termios, Winsize, cc, iflag, lflag};
use crate::tty::Ttys;
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
    ttys: Arc<Ttys>,
    vfs: &'static Vfs,
    ctx: FsContext,
}

impl Env {
    fn new() -> Env {
        let host = FakeHost::new();
        let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(Epoch))))));
        let ctx = vfs.root_context().unwrap();
        let ttys = host.ttys();
        let ptys = Ptys::new(&ttys, vfs);
        let root = Cred::root();
        vfs.register_char_device(PTMX, ptys.ptmx()).unwrap();
        vfs.mkdir(&ctx, &root, At::Cwd, b"/dev", 0o755).unwrap();
        vfs.mkdir(&ctx, &root, At::Cwd, b"/dev/pts", 0o755).unwrap();
        vfs.mknod(&ctx, &root, At::Cwd, b"/dev/ptmx", FileType::CharDevice, 0o666, PTMX).unwrap();
        let devpts = Arc::new(DevPts::new(ptys.clone()));
        vfs.mount(&ctx, &root, At::Cwd, b"/dev/pts", devpts, MountFlags::default()).unwrap();
        act_as(Some(caller(10, 10, 10)));
        Env { host, ptys, ttys, vfs, ctx }
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

#[test]
fn vhangup_kills_the_open_files_but_not_the_terminal() {
    let env = Env::new();
    let (m, n) = env.master();
    env.host.groups.lock().unwrap().insert(10, 10);
    let old = env.slave(n, OpenFlags::RDWR).unwrap();
    assert_eq!(old.ioctl(TIOCGSID, USER), Ok(0), "the slave is the session's controlling terminal");
    env.host.take_signals();
    env.ttys.vhangup(10);
    // The leader is in the foreground group: one SIGHUP, and SIGCONT.
    assert_eq!(
        env.host.take_signals(),
        [(Target::Group(10), SIGHUP), (Target::Group(10), SIGCONT)],
        "the leader is in the foreground group: one SIGHUP"
    );
    // Files opened before are dead.
    assert_eq!(read_some(&old, 10), Ok(vec![]));
    assert_eq!(old.write(b"x"), Err(Errno::EIO));
    assert_eq!(old.ioctl(TIOCGSID, USER), Err(Errno::EIO));
    assert!(old.poll().contains(PollEvents::HUP));
    // The session has no controlling terminal any more.
    assert!(env.ttys.of_session(10).is_none());
    // The terminal lives: a file opened afterwards works.
    let new = env.slave(n, OpenFlags::RDWR | OpenFlags::NOCTTY).unwrap();
    assert_eq!(new.write(b"y"), Ok(1));
    assert_eq!(read_some(&m, 10), Ok(b"y".to_vec()));
    // A session without a terminal: nothing happens.
    env.ttys.vhangup(10);
    assert!(env.host.take_signals().is_empty());
}

// ---- packet mode (TIOCPKT) --------------------------------------------------

fn set_int(f: &OpenFile, env: &Env, request: u32, v: i32) -> Result<u64, Errno> {
    env.host.poke(USER, &v.to_le_bytes());
    f.ioctl(request, USER)
}

fn packet_pair(env: &Env) -> (Arc<OpenFile>, Arc<OpenFile>) {
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR | OpenFlags::NOCTTY).unwrap();
    nonblocking(&m);
    nonblocking(&s);
    assert_eq!(set_int(&m, env, TIOCPKT, 1), Ok(0));
    (m, s)
}

fn apply(f: &OpenFile, env: &Env, t: &Termios) {
    env.host.poke(USER, &t.to_bytes());
    assert_eq!(f.ioctl(TCSETS, USER), Ok(0));
}

#[test]
fn tiocpkt_switches_packet_mode_and_tiocgpkt_reports_it() {
    let env = Env::new();
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR | OpenFlags::NOCTTY).unwrap();
    assert_eq!(m.ioctl(TIOCGPKT, USER + 16), Ok(0));
    assert_eq!(env.host.peek_int(USER + 16), 0, "off at first");
    assert_eq!(set_int(&m, &env, TIOCPKT, 5), Ok(0), "any nonzero value enables");
    assert_eq!(m.ioctl(TIOCGPKT, USER + 16), Ok(0));
    assert_eq!(env.host.peek_int(USER + 16), 1);
    assert_eq!(set_int(&m, &env, TIOCPKT, 0), Ok(0));
    assert_eq!(m.ioctl(TIOCGPKT, USER + 16), Ok(0));
    assert_eq!(env.host.peek_int(USER + 16), 0);
    // Bad pointers.
    assert_eq!(m.ioctl(TIOCPKT, 0), Err(Errno::EFAULT));
    assert_eq!(m.ioctl(TIOCGPKT, 0), Err(Errno::EFAULT));
    // The slave answers neither.
    assert_eq!(s.ioctl(TIOCPKT, USER), Err(Errno::ENOTTY));
    assert_eq!(s.ioctl(TIOCGPKT, USER), Err(Errno::ENOTTY));
}

#[test]
fn packet_reads_start_with_a_zero_status_byte_before_data() {
    let env = Env::new();
    let (m, s) = packet_pair(&env);
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN), "nothing yet, no empty packet");
    assert_eq!(s.write(b"out\n"), Ok(4));
    assert_eq!(read_some(&m, 100).unwrap(), b"\0out\r\n");
    // A short buffer: the rest comes in the next packet, with its own status byte.
    assert_eq!(s.write(b"abcdef"), Ok(6));
    assert_eq!(read_some(&m, 3).unwrap(), b"\0ab");
    assert_eq!(read_some(&m, 100).unwrap(), b"\0cdef");
    // A one-byte buffer takes only the status byte and leaves the data.
    assert_eq!(s.write(b"xy"), Ok(2));
    assert_eq!(read_some(&m, 1).unwrap(), b"\0");
    assert_eq!(read_some(&m, 100).unwrap(), b"\0xy");
    // A read of nothing stays a read of nothing.
    assert_eq!(read_some(&m, 0), Ok(vec![]));
    // Writes to the master are not framed (the echo of "z" is the data).
    assert_eq!(m.write(b"z"), Ok(1));
    assert_eq!(read_some(&m, 100).unwrap(), b"\0z");
    // Packet mode off: plain data again.
    assert_eq!(set_int(&m, &env, TIOCPKT, 0), Ok(0));
    assert_eq!(s.write(b"q"), Ok(1));
    assert_eq!(read_some(&m, 100).unwrap(), b"q");
}

#[test]
fn packet_status_for_flushes() {
    let env = Env::new();
    let (m, s) = packet_pair(&env);
    // The slave flushes its input queue.
    assert_eq!(s.ioctl(TCFLSH, u64::from(TCIFLUSH)), Ok(0));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_FLUSHREAD]);
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN), "reported once");
    // Its output queue: the data not read yet is gone, the status says so.
    assert_eq!(s.write(b"lost"), Ok(4));
    assert_eq!(s.ioctl(TCFLSH, u64::from(TCOFLUSH)), Ok(0));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_FLUSHWRITE]);
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN));
    // Both at once: one byte with both bits.
    assert_eq!(s.ioctl(TCFLSH, u64::from(TCIOFLUSH)), Ok(0));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_FLUSHREAD | TIOCPKT_FLUSHWRITE]);
    // TCSETSF throws input away too.
    let t = Termios::sane(38400);
    env.host.poke(USER, &t.to_bytes());
    assert_eq!(s.ioctl(TCSETSF, USER), Ok(0));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_FLUSHREAD]);
}

#[test]
fn packet_status_for_stop_and_start() {
    let env = Env::new();
    let (m, s) = packet_pair(&env);
    // tcflow(TCOOFF / TCOON) on the slave.
    assert_eq!(s.ioctl(TCXONC, u64::from(TCOOFF)), Ok(0));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_STOP]);
    assert_eq!(s.ioctl(TCXONC, u64::from(TCOON)), Ok(0));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_START]);
    // Starting output that is not stopped says nothing.
    assert_eq!(s.ioctl(TCXONC, u64::from(TCOON)), Ok(0));
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN));
    // ^S and ^Q typed on the master.
    assert_eq!(m.write(b"\x13"), Ok(1));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_STOP]);
    assert_eq!(m.write(b"\x11"), Ok(1));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_START]);
    // Both before the read: one byte with both bits.
    s.ioctl(TCXONC, u64::from(TCOOFF)).unwrap();
    s.ioctl(TCXONC, u64::from(TCOON)).unwrap();
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_STOP | TIOCPKT_START]);
}

#[test]
fn packet_status_for_flow_control_and_attribute_changes() {
    let env = Env::new();
    let (m, s) = packet_pair(&env);
    let sane = Termios::sane(38400);
    // The same attributes again: nothing changed.
    apply(&s, &env, &sane);
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN));
    // IXON off: flow control goes away.
    let mut t = sane;
    t.iflag &= !iflag::IXON;
    apply(&s, &env, &t);
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_NOSTOP | TIOCPKT_IOCTL]);
    // IXON on again.
    apply(&s, &env, &sane);
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_DOSTOP | TIOCPKT_IOCTL]);
    // Another change: only the IOCTL bit.
    let mut t = sane;
    t.lflag &= !lflag::ECHO;
    apply(&s, &env, &t);
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_IOCTL]);
    // Another stop character: no longer ^S/^Q flow control.
    let mut t = sane;
    t.cc[cc::VSTOP] = 0x07;
    apply(&s, &env, &t);
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_NOSTOP | TIOCPKT_IOCTL]);
    // The status comes before data waiting in the buffer.
    s.write(b"d").unwrap();
    apply(&s, &env, &sane);
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_DOSTOP | TIOCPKT_IOCTL]);
    assert_eq!(read_some(&m, 100).unwrap(), b"\0d");
}

#[test]
fn outside_packet_mode_nothing_is_reported_and_switching_off_drops_the_pending_status() {
    let env = Env::new();
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR | OpenFlags::NOCTTY).unwrap();
    nonblocking(&m);
    s.ioctl(TCFLSH, u64::from(TCIFLUSH)).unwrap();
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN), "not in packet mode: no status");
    assert!(!m.poll().contains(PollEvents::IN));
    assert_eq!(set_int(&m, &env, TIOCPKT, 1), Ok(0));
    s.ioctl(TCFLSH, u64::from(TCIFLUSH)).unwrap();
    assert!(m.poll().contains(PollEvents::IN));
    assert_eq!(set_int(&m, &env, TIOCPKT, 0), Ok(0));
    assert!(!m.poll().contains(PollEvents::IN), "switched off: the pending status is gone");
    assert_eq!(read_some(&m, 100), Err(Errno::EAGAIN));
}

#[test]
fn packet_poll_is_readable_while_a_status_is_pending() {
    let env = Env::new();
    let (m, s) = packet_pair(&env);
    assert!(!m.poll().contains(PollEvents::IN));
    s.ioctl(TCXONC, u64::from(TCOOFF)).unwrap();
    assert!(m.poll().contains(PollEvents::IN));
    assert_eq!(read_some(&m, 100).unwrap(), [TIOCPKT_STOP]);
    assert!(!m.poll().contains(PollEvents::IN));
}

#[test]
fn a_blocked_packet_reader_wakes_for_a_status() {
    let env = Env::new();
    let (m, n) = env.master();
    let s = env.slave(n, OpenFlags::RDWR | OpenFlags::NOCTTY).unwrap();
    assert_eq!(set_int(&m, &env, TIOCPKT, 1), Ok(0));
    let reader = {
        let m = m.clone();
        thread::spawn(move || {
            act_as(Some(caller(10, 10, 10)));
            read_some(&m, 100)
        })
    };
    env.host.wait_blocked(1);
    assert_eq!(s.ioctl(TCXONC, u64::from(TCOOFF)), Ok(0));
    assert_eq!(reader.join().unwrap().unwrap(), [TIOCPKT_STOP]);
}

#[test]
fn packet_reads_end_with_eio_once_the_slave_is_closed() {
    let env = Env::new();
    let (m, s) = packet_pair(&env);
    s.write(b"bye").unwrap();
    drop(s);
    assert!(m.poll().contains(PollEvents::HUP));
    assert_eq!(read_some(&m, 100).unwrap(), b"\0bye");
    assert_eq!(read_some(&m, 100), Err(Errno::EIO));
}
