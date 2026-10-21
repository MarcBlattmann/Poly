//! Readiness and file objects (ADR 0042): pipes (pipe(7)), event counters
//! (eventfd(2)), timers (timerfd_create(2)), epoll sets (epoll(7)) and
//! `poll(2)`, with real threads blocking where the calls block.

extern crate std;

use super::Env;
use crate::epoll::{EPOLLET, EPOLLEXCLUSIVE, EPOLLONESHOT, Epoll, EpollEvent, EpollOp};
use crate::eventfd::{EVENTFD_MAX, EventFd};
use crate::pipe::{PIPE_BUF, PIPE_CAPACITY, Pipe};
use crate::poll::{PollEntry, PollTarget};
use crate::timerfd::{TimerClock, TimerFd, TimerService, TimerSpec, TimerTarget};
use crate::*;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

// ---- a blocker made of a condition variable ---------------------------

fn now_ns() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

#[derive(Default)]
struct Flags {
    woken: bool,
    interrupted: bool,
}

#[derive(Clone, Default)]
struct Signal(Arc<(Mutex<Flags>, Condvar)>);

impl Signal {
    fn set(&self, f: impl FnOnce(&mut Flags)) {
        let (m, c) = &*self.0;
        f(&mut m.lock().unwrap());
        c.notify_all();
    }
}

impl Waker for Signal {
    fn wake(&self, _: PollEvents) {
        self.set(|f| f.woken = true);
    }
}

/// A thread's blocker; [`TestBlocker::interrupter`] interrupts it from
/// another thread (a signal).
#[derive(Clone, Default)]
struct TestBlocker {
    signal: Signal,
    blocks: Arc<Mutex<usize>>,
}

impl TestBlocker {
    fn interrupter(&self) -> Signal {
        self.signal.clone()
    }

    fn blocks(&self) -> usize {
        *self.blocks.lock().unwrap()
    }
}

impl Blocker for TestBlocker {
    fn waker(&self) -> Arc<dyn Waker> {
        Arc::new(self.signal.clone())
    }

    fn block(&mut self, until: Option<u64>) -> Woke {
        *self.blocks.lock().unwrap() += 1;
        let (m, c) = &*self.signal.0;
        let mut f = m.lock().unwrap();
        loop {
            if f.interrupted {
                return Woke::Interrupted;
            }
            if f.woken {
                f.woken = false;
                return Woke::Woken;
            }
            let wait = match until {
                Some(t) => {
                    let now = now_ns();
                    if now >= t {
                        return Woke::TimedOut;
                    }
                    Duration::from_nanos(t - now)
                }
                None => Duration::from_secs(60),
            };
            f = c.wait_timeout(f, wait).unwrap().0;
        }
    }
}

/// Wait (without a deadline worth the name) until `cond` holds: for
/// another thread to have blocked, say.
fn eventually(mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(60), "condition never held");
        thread::sleep(Duration::from_millis(1));
    }
}

// ---- pipes -------------------------------------------------------------

fn pipe_files(env: &Env, flags: OpenFlags, packet: bool) -> (Arc<OpenFile>, Arc<OpenFile>) {
    let (r, w) = Pipe::pair(packet);
    let node = env.vfs.anon_node(FileType::Fifo, 0o600, &env.root).unwrap();
    let rf = OpenFile::on_object(node.clone(), Arc::new(r), flags.union(OpenFlags::RDONLY));
    let wf = OpenFile::on_object(node, Arc::new(w), flags.union(OpenFlags::WRONLY));
    (Arc::new(rf), Arc::new(wf))
}

#[test]
fn pipe_moves_bytes_in_order_and_ends_with_eof() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    assert_eq!(w.write(b"hello, "), Ok(7));
    assert_eq!(w.write(b"pipe"), Ok(4));
    assert_eq!(r.bytes_readable(), Some(11));
    let mut buf = [0u8; 5];
    assert_eq!(r.read(&mut buf), Ok(5));
    assert_eq!(&buf, b"hello");
    let mut rest = [0u8; 64];
    assert_eq!(r.read(&mut rest), Ok(6));
    assert_eq!(&rest[..6], b", pipe");
    assert_eq!(r.read(&mut rest), Err(Errno::EAGAIN), "empty with a writer: would block");
    assert_eq!(r.read(&mut []), Ok(0));
    // Wrong direction.
    assert_eq!(r.write(b"x"), Err(Errno::EBADF));
    assert_eq!(w.read(&mut rest), Err(Errno::EBADF));
    // Unseekable.
    assert_eq!(r.seek(0, Whence::Set), Err(Errno::ESPIPE));
    assert_eq!(w.write_at(0, b"x"), Err(Errno::ESPIPE));
    drop(w);
    assert_eq!(r.read(&mut rest), Ok(0), "no writer left: end of file");
}

#[test]
fn pipe_without_readers_is_epipe() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    assert!(w.poll().contains(PollEvents::WRITABLE));
    drop(r);
    assert_eq!(w.write(b"x"), Err(Errno::EPIPE));
    assert_eq!(w.poll(), PollEvents::ERR | PollEvents::WRITABLE, "POLLERR (and POLLOUT) once the read end is closed");
    let mut blocker = TestBlocker::default();
    assert_eq!(w.write_wait(b"x", &mut blocker), Err(Errno::EPIPE));
}

#[test]
fn pipe_writes_up_to_pipe_buf_are_atomic() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    // Fill all but PIPE_BUF - 1 bytes.
    let big = vec![1u8; PIPE_CAPACITY - (PIPE_BUF - 1)];
    assert_eq!(w.write(&big), Ok(big.len()));
    assert!(!w.poll().intersects(PollEvents::WRITABLE), "a PIPE_BUF write would block");
    assert_eq!(w.write(&[2u8; PIPE_BUF]), Err(Errno::EAGAIN), "all or nothing");
    assert_eq!(w.write(&[2u8; 10]), Ok(10));
    // A longer write takes what fits.
    assert_eq!(w.write(&[3u8; PIPE_BUF + 1]), Ok(PIPE_BUF - 11));
    assert_eq!(r.bytes_readable(), Some(PIPE_CAPACITY));
    assert_eq!(w.write(&[3u8; PIPE_BUF + 1]), Err(Errno::EAGAIN), "full");
    let mut buf = vec![0u8; PIPE_CAPACITY];
    assert_eq!(r.read(&mut buf), Ok(PIPE_CAPACITY));
    assert!(w.poll().contains(PollEvents::WRITABLE));
}

#[test]
fn pipe_packet_mode_reads_one_packet_at_a_time() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK | OpenFlags::DIRECT, true);
    assert_eq!(w.write(b"one"), Ok(3));
    assert_eq!(w.write(b"second"), Ok(6));
    assert_eq!(w.write(&[7u8; PIPE_BUF + 5]), Ok(PIPE_BUF + 5), "split into two packets");
    let mut buf = [0u8; 100];
    assert_eq!(r.read(&mut buf), Ok(3));
    assert_eq!(&buf[..3], b"one");
    assert_eq!(r.read(&mut buf[..2]), Ok(2), "a short read drops the rest of the packet");
    assert_eq!(&buf[..2], b"se");
    let mut big = vec![0u8; PIPE_BUF * 2];
    assert_eq!(r.read(&mut big), Ok(PIPE_BUF));
    assert_eq!(r.read(&mut big), Ok(5));
    assert_eq!(r.read(&mut big), Err(Errno::EAGAIN));
}

#[test]
fn pipe_poll_reports_each_end() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    assert_eq!(r.poll(), PollEvents::NONE);
    assert_eq!(w.poll(), PollEvents::WRITABLE);
    w.write(b"x").unwrap();
    assert_eq!(r.poll(), PollEvents::READABLE);
    drop(w);
    assert_eq!(r.poll(), PollEvents::READABLE | PollEvents::HUP);
    let mut b = [0u8; 4];
    r.read(&mut b).unwrap();
    assert_eq!(r.poll(), PollEvents::HUP, "poll(2): the peer closed its end");
}

#[test]
fn pipe_ends_share_one_anonymous_inode() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::RDONLY, false);
    let (rs, ws) = (r.stat().unwrap(), w.stat().unwrap());
    assert_eq!(rs.ino, ws.ino);
    assert_eq!(rs.kind(), Some(FileType::Fifo));
    assert_eq!(rs.perm(), 0o600);
    assert_eq!(r.kind(), FileType::Fifo);
    assert_eq!(env.vfs.anon_inodes(), 1);
    let (r2, w2) = pipe_files(&env, OpenFlags::RDONLY, false);
    assert_ne!(r2.stat().unwrap().ino, rs.ino);
    drop((r, w, r2, w2));
    assert_eq!(env.vfs.anon_inodes(), 0, "the inodes go with the last open file");
}

#[test]
fn blocking_read_waits_for_a_writer() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::RDONLY, false);
    let queue = r.wait_queue().unwrap().clone();
    let reader = thread::spawn(move || {
        let mut blocker = TestBlocker::default();
        let mut buf = [0u8; 16];
        let n = r.read_wait(&mut buf, &mut blocker);
        (n, buf, blocker.blocks())
    });
    eventually(|| !queue.is_empty());
    assert_eq!(w.write(b"wake up"), Ok(7));
    let (n, buf, blocks) = reader.join().unwrap();
    assert_eq!(n, Ok(7));
    assert_eq!(&buf[..7], b"wake up");
    assert!(blocks >= 1);
    assert!(queue.is_empty(), "the reader's registration is gone");
}

#[test]
fn blocking_read_sees_eof_when_the_writer_goes() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::RDONLY, false);
    let queue = r.wait_queue().unwrap().clone();
    let reader = thread::spawn(move || r.read_wait(&mut [0u8; 4], &mut TestBlocker::default()));
    eventually(|| !queue.is_empty());
    drop(w);
    assert_eq!(reader.join().unwrap(), Ok(0));
}

#[test]
fn blocking_read_is_interrupted() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::RDONLY, false);
    let queue = r.wait_queue().unwrap().clone();
    let blocker = TestBlocker::default();
    let interrupt = blocker.interrupter();
    let reader = thread::spawn(move || {
        let mut blocker = blocker;
        r.read_wait(&mut [0u8; 4], &mut blocker)
    });
    eventually(|| !queue.is_empty());
    interrupt.set(|f| f.interrupted = true);
    assert_eq!(reader.join().unwrap(), Err(Errno::EINTR));
    drop(w);
}

#[test]
fn nonblocking_read_does_not_wait() {
    let env = Env::new();
    let (r, _w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let mut blocker = TestBlocker::default();
    assert_eq!(r.read_wait(&mut [0u8; 4], &mut blocker), Err(Errno::EAGAIN));
    assert_eq!(blocker.blocks(), 0);
}

#[test]
fn blocking_write_writes_everything_as_the_reader_drains() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::RDONLY, false);
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let expect = data.clone();
    let writer = thread::spawn(move || w.write_wait(&data, &mut TestBlocker::default()));
    let mut got = Vec::new();
    let mut blocker = TestBlocker::default();
    let mut buf = [0u8; 7000];
    loop {
        match r.read_wait(&mut buf, &mut blocker) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) => panic!("read: {e:?}"),
        }
    }
    assert_eq!(writer.join().unwrap(), Ok(300_000));
    assert_eq!(got, expect);
}

#[test]
fn interrupted_blocking_write_reports_the_part_written() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::RDONLY, false);
    let queue = w.wait_queue().unwrap().clone();
    let blocker = TestBlocker::default();
    let interrupt = blocker.interrupter();
    let writer = thread::spawn(move || {
        let mut blocker = blocker;
        let big = vec![0u8; PIPE_CAPACITY + 100];
        let partial = w.write_wait(&big, &mut blocker);
        // Full now: a second blocking write is interrupted before any byte.
        blocker.signal.set(|f| f.interrupted = true);
        (partial, w.write_wait(b"x", &mut blocker))
    });
    eventually(|| !queue.is_empty());
    interrupt.set(|f| f.interrupted = true);
    let (partial, second) = writer.join().unwrap();
    assert_eq!(partial, Ok(PIPE_CAPACITY));
    assert_eq!(second, Err(Errno::EINTR));
    assert_eq!(r.bytes_readable(), Some(PIPE_CAPACITY));
}

// ---- poll --------------------------------------------------------------

#[test]
fn poll_reports_ready_closed_and_ignored_entries() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let regular = env.open("/f", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    let mut entries = vec![
        PollEntry::new(PollTarget::File(r.clone()), PollEvents::IN),
        PollEntry::new(PollTarget::File(w.clone()), PollEvents::OUT),
        PollEntry::new(PollTarget::Closed, PollEvents::IN),
        PollEntry::new(PollTarget::Ignore, PollEvents::IN),
        PollEntry::new(PollTarget::File(regular), PollEvents::IN | PollEvents::OUT),
    ];
    let mut blocker = TestBlocker::default();
    assert_eq!(poll::poll(&mut entries, &mut blocker, Timeout::Now), Ok(3));
    let mut exact = vec![PollEntry::exact(PollTarget::Closed, PollEvents::NONE)];
    assert_eq!(poll::poll(&mut exact, &mut blocker, Timeout::Now), Ok(1));
    assert_eq!(exact[0].revents, PollEvents::NVAL);
    assert_eq!(entries[0].revents, PollEvents::NONE);
    assert_eq!(entries[1].revents, PollEvents::OUT);
    assert_eq!(entries[2].revents, PollEvents::NVAL);
    assert_eq!(entries[3].revents, PollEvents::NONE);
    assert_eq!(entries[4].revents, PollEvents::IN | PollEvents::OUT);
    // HUP comes unasked.
    drop((w, entries));
    let mut entries = vec![PollEntry::new(PollTarget::File(r.clone()), PollEvents::NONE)];
    assert_eq!(poll::poll(&mut entries, &mut blocker, Timeout::Now), Ok(1));
    assert_eq!(entries[0].revents, PollEvents::HUP);
    let mut exact = vec![PollEntry::exact(PollTarget::File(r), PollEvents::IN)];
    assert_eq!(poll::poll(&mut exact, &mut blocker, Timeout::Now), Ok(0), "HUP only when asked");
}

#[test]
fn poll_times_out() {
    let env = Env::new();
    let (r, _w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let mut entries = vec![PollEntry::new(PollTarget::File(r), PollEvents::IN)];
    let start = now_ns();
    let mut blocker = TestBlocker::default();
    assert_eq!(poll::poll(&mut entries, &mut blocker, Timeout::At(start + 20_000_000)), Ok(0));
    assert!(now_ns() >= start + 20_000_000);
    assert_eq!(entries[0].revents, PollEvents::NONE);
}

#[test]
fn poll_wakes_when_a_file_becomes_ready_and_can_be_interrupted() {
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let (r2, _w2) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let queue = r.wait_queue().unwrap().clone();
    let (rc, r2c) = (r.clone(), r2.clone());
    let poller = thread::spawn(move || {
        let mut entries = vec![
            PollEntry::new(PollTarget::File(r2c), PollEvents::IN),
            PollEntry::new(PollTarget::File(rc), PollEvents::IN),
        ];
        let n = poll::poll(&mut entries, &mut TestBlocker::default(), Timeout::Never);
        (n, entries[1].revents)
    });
    eventually(|| !queue.is_empty());
    w.write(b"!").unwrap();
    assert_eq!(poller.join().unwrap(), (Ok(1), PollEvents::IN));
    assert!(queue.is_empty());

    let blocker = TestBlocker::default();
    let interrupt = blocker.interrupter();
    let queue2 = r2.wait_queue().unwrap().clone();
    let poller = thread::spawn(move || {
        let mut blocker = blocker;
        let mut entries = vec![PollEntry::new(PollTarget::File(r2), PollEvents::IN)];
        poll::poll(&mut entries, &mut blocker, Timeout::Never)
    });
    eventually(|| !queue2.is_empty());
    interrupt.set(|f| f.interrupted = true);
    assert_eq!(poller.join().unwrap(), Err(Errno::EINTR));
}

// ---- event counters ----------------------------------------------------

fn object_file(env: &Env, obj: Arc<dyn FileObject>, flags: OpenFlags) -> Arc<OpenFile> {
    let node = env.vfs.anon_node(FileType::Regular, 0o600, &env.root).unwrap();
    Arc::new(OpenFile::on_object(node, obj, flags | OpenFlags::RDWR))
}

fn read_u64(f: &OpenFile) -> Result<u64> {
    let mut b = [0u8; 8];
    f.read(&mut b).map(|_| u64::from_ne_bytes(b))
}

fn write_u64(f: &OpenFile, v: u64) -> Result<usize> {
    f.write(&v.to_ne_bytes())
}

#[test]
fn eventfd_counts_and_resets() {
    let env = Env::new();
    let f = object_file(&env, Arc::new(EventFd::new(3, false)), OpenFlags::NONBLOCK);
    assert_eq!(f.poll(), PollEvents::READABLE | PollEvents::WRITABLE);
    assert_eq!(write_u64(&f, 4), Ok(8));
    assert_eq!(read_u64(&f), Ok(7));
    assert_eq!(read_u64(&f), Err(Errno::EAGAIN));
    assert_eq!(f.poll(), PollEvents::WRITABLE);
    assert_eq!(f.read(&mut [0u8; 7]), Err(Errno::EINVAL), "eventfd(2): buffer smaller than 8 bytes");
    assert_eq!(f.write(&[0u8; 7]), Err(Errno::EINVAL));
    assert_eq!(write_u64(&f, u64::MAX), Err(Errno::EINVAL));
    assert_eq!(write_u64(&f, EVENTFD_MAX), Ok(8));
    assert_eq!(f.poll(), PollEvents::READABLE, "at the maximum: not writable");
    assert_eq!(write_u64(&f, 1), Err(Errno::EAGAIN), "would overflow");
    assert_eq!(write_u64(&f, 0), Ok(8));
    assert_eq!(read_u64(&f), Ok(EVENTFD_MAX));
}

#[test]
fn eventfd_semaphore_mode_takes_one() {
    let env = Env::new();
    let f = object_file(&env, Arc::new(EventFd::new(2, true)), OpenFlags::NONBLOCK);
    assert_eq!(read_u64(&f), Ok(1));
    assert_eq!(read_u64(&f), Ok(1));
    assert_eq!(read_u64(&f), Err(Errno::EAGAIN));
}

#[test]
fn eventfd_blocking_read_waits_for_a_write() {
    let env = Env::new();
    let f = object_file(&env, Arc::new(EventFd::new(0, false)), OpenFlags::RDONLY);
    let queue = f.wait_queue().unwrap().clone();
    let g = f.clone();
    let reader = thread::spawn(move || {
        let mut b = [0u8; 8];
        g.read_wait(&mut b, &mut TestBlocker::default()).map(|_| u64::from_ne_bytes(b))
    });
    eventually(|| !queue.is_empty());
    write_u64(&f, 42).unwrap();
    assert_eq!(reader.join().unwrap(), Ok(42));
}

// ---- timers ------------------------------------------------------------

/// A service with clocks the test moves and callbacks it fires by hand.
type Armed = Vec<(u64, u64, Weak<dyn TimerTarget>)>;

#[derive(Default)]
struct FakeTimers {
    mono: Mutex<u64>,
    real: Mutex<u64>,
    armed: Mutex<Armed>,
    next_id: Mutex<u64>,
}

impl FakeTimers {
    fn advance(&self, ns: u64) {
        *self.mono.lock().unwrap() += ns;
        *self.real.lock().unwrap() += ns;
    }

    /// Fire every callback that is due.
    fn fire(&self) {
        let now = *self.mono.lock().unwrap();
        let due: Vec<Weak<dyn TimerTarget>> = {
            let mut a = self.armed.lock().unwrap();
            let due = a.iter().filter(|(_, at, _)| *at <= now).map(|(_, _, t)| t.clone()).collect();
            a.retain(|(_, at, _)| *at > now);
            due
        };
        for t in due {
            if let Some(t) = t.upgrade() {
                t.expire();
            }
        }
    }

    fn armed(&self) -> Vec<u64> {
        self.armed.lock().unwrap().iter().map(|(_, at, _)| *at).collect()
    }
}

impl TimerService for FakeTimers {
    fn monotonic_ns(&self) -> u64 {
        *self.mono.lock().unwrap()
    }

    fn realtime_ns(&self) -> u64 {
        *self.real.lock().unwrap()
    }

    fn arm(&self, at_ns: u64, target: Weak<dyn TimerTarget>) -> u64 {
        let mut id = self.next_id.lock().unwrap();
        *id += 1;
        self.armed.lock().unwrap().push((*id, at_ns, target));
        *id
    }

    fn cancel(&self, id: u64) {
        self.armed.lock().unwrap().retain(|(i, _, _)| *i != id);
    }
}

const MS: u64 = 1_000_000;

#[test]
fn timerfd_one_shot_and_interval() {
    let env = Env::new();
    let svc = Arc::new(FakeTimers::default());
    svc.advance(1_000 * MS);
    let t = TimerFd::new(TimerClock::Monotonic, svc.clone());
    let f = object_file(&env, t.clone(), OpenFlags::NONBLOCK);
    assert_eq!(t.get(), TimerSpec::default());
    assert_eq!(read_u64(&f), Err(Errno::EAGAIN));
    // One shot in 10 ms.
    let old = t.set(false, TimerSpec { value_ns: 10 * MS, interval_ns: 0 });
    assert_eq!(old, TimerSpec::default());
    assert_eq!(svc.armed(), vec![1_010 * MS]);
    svc.advance(4 * MS);
    assert_eq!(t.get(), TimerSpec { value_ns: 6 * MS, interval_ns: 0 });
    assert_eq!(f.poll(), PollEvents::NONE);
    svc.advance(6 * MS);
    assert_eq!(f.poll(), PollEvents::READABLE);
    assert_eq!(read_u64(&f), Ok(1));
    assert_eq!(read_u64(&f), Err(Errno::EAGAIN));
    assert_eq!(t.get(), TimerSpec::default(), "disarmed after its one expiration");
    // Every 5 ms from 5 ms on: 25 ms later, five expirations.
    t.set(false, TimerSpec { value_ns: 5 * MS, interval_ns: 5 * MS });
    svc.advance(25 * MS);
    assert_eq!(read_u64(&f), Ok(5));
    assert_eq!(t.get(), TimerSpec { value_ns: 5 * MS, interval_ns: 5 * MS });
    assert_eq!(f.read(&mut [0u8; 4]), Err(Errno::EINVAL));
    assert_eq!(f.write(&[0u8; 8]), Err(Errno::EINVAL));
    // Disarm; the old setting comes back.
    let old = t.set(false, TimerSpec::default());
    assert_eq!(old, TimerSpec { value_ns: 5 * MS, interval_ns: 5 * MS });
    assert!(svc.armed().is_empty(), "disarming cancels the callback");
}

#[test]
fn timerfd_absolute_times_on_both_clocks() {
    let env = Env::new();
    let svc = Arc::new(FakeTimers::default());
    *svc.real.lock().unwrap() = 1_700_000_000_000 * MS;
    svc.advance(50 * MS);
    let t = TimerFd::new(TimerClock::Realtime, svc.clone());
    let f = object_file(&env, t.clone(), OpenFlags::NONBLOCK);
    let now = svc.realtime_ns();
    t.set(true, TimerSpec { value_ns: now + 20 * MS, interval_ns: 0 });
    assert_eq!(svc.armed(), vec![70 * MS], "a monotonic callback 20 ms from now");
    assert_eq!(t.get().value_ns, 20 * MS);
    // An absolute time in the past expires at once.
    t.set(true, TimerSpec { value_ns: now - MS, interval_ns: 0 });
    assert_eq!(read_u64(&f), Ok(1));
    let m = TimerFd::new(TimerClock::Monotonic, svc.clone());
    m.set(true, TimerSpec { value_ns: 60 * MS, interval_ns: 0 });
    assert_eq!(m.get().value_ns, 10 * MS);
}

#[test]
fn timerfd_expiry_wakes_a_blocked_reader() {
    let env = Env::new();
    let svc = Arc::new(FakeTimers::default());
    let t = TimerFd::new(TimerClock::Monotonic, svc.clone());
    let f = object_file(&env, t.clone(), OpenFlags::RDONLY);
    t.set(false, TimerSpec { value_ns: 3 * MS, interval_ns: 0 });
    let queue = f.wait_queue().unwrap().clone();
    let g = f.clone();
    let reader = thread::spawn(move || {
        let mut b = [0u8; 8];
        g.read_wait(&mut b, &mut TestBlocker::default()).map(|_| u64::from_ne_bytes(b))
    });
    eventually(|| !queue.is_empty());
    svc.advance(3 * MS);
    svc.fire();
    assert_eq!(reader.join().unwrap(), Ok(1));
    drop(f);
    t.set(false, TimerSpec { value_ns: MS, interval_ns: 0 });
    let weak = Arc::downgrade(&t);
    drop(t);
    assert!(weak.upgrade().is_none());
    assert!(svc.armed().is_empty(), "a dropped timer cancels its callback");
}

// ---- epoll -------------------------------------------------------------

fn epoll_file(env: &Env) -> (Arc<Epoll>, Arc<OpenFile>) {
    let ep = Epoll::new();
    let f = object_file(env, ep.clone(), OpenFlags::RDONLY);
    (ep, f)
}

fn ev(events: PollEvents, flags: u32, data: u64) -> EpollEvent {
    EpollEvent { events: events.bits() | flags, data }
}

fn wait_now(ep: &Epoll, max: usize) -> Vec<EpollEvent> {
    ep.wait(max, &mut TestBlocker::default(), Timeout::Now).unwrap()
}

#[test]
fn epoll_level_triggered_reports_while_ready() {
    let env = Env::new();
    let (ep, _) = epoll_file(&env);
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    ep.ctl(EpollOp::Add, 3, &r, ev(PollEvents::IN, 0, 33)).unwrap();
    ep.ctl(EpollOp::Add, 4, &w, ev(PollEvents::OUT, 0, 44)).unwrap();
    assert_eq!(wait_now(&ep, 8), vec![EpollEvent { events: PollEvents::OUT.bits(), data: 44 }]);
    w.write(b"abc").unwrap();
    let got = wait_now(&ep, 8);
    assert!(got.contains(&EpollEvent { events: PollEvents::IN.bits(), data: 33 }), "{got:?}");
    assert!(got.contains(&EpollEvent { events: PollEvents::OUT.bits(), data: 44 }));
    // Still readable: reported again.
    let got = wait_now(&ep, 1);
    assert_eq!(got.len(), 1, "maxevents");
    let got2 = wait_now(&ep, 8);
    assert_eq!(got2.len(), 2);
    r.read(&mut [0u8; 8]).unwrap();
    assert_eq!(wait_now(&ep, 8), vec![EpollEvent { events: PollEvents::OUT.bits(), data: 44 }]);
}

#[test]
fn epoll_edge_triggered_reports_each_change_once() {
    let env = Env::new();
    let (ep, _) = epoll_file(&env);
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    ep.ctl(EpollOp::Add, 3, &r, ev(PollEvents::IN, EPOLLET, 1)).unwrap();
    assert!(wait_now(&ep, 8).is_empty());
    w.write(&[0u8; 2048]).unwrap();
    assert_eq!(wait_now(&ep, 8), vec![EpollEvent { events: PollEvents::IN.bits(), data: 1 }]);
    // epoll(7)'s example: half read, still data, but no new event.
    r.read(&mut [0u8; 1024]).unwrap();
    assert!(wait_now(&ep, 8).is_empty());
    w.write(b"more").unwrap();
    assert_eq!(wait_now(&ep, 8).len(), 1, "a new write is a new edge");
    assert!(wait_now(&ep, 8).is_empty());
}

#[test]
fn epoll_oneshot_until_rearmed() {
    let env = Env::new();
    let (ep, _) = epoll_file(&env);
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    ep.ctl(EpollOp::Add, 3, &r, ev(PollEvents::IN, EPOLLONESHOT, 7)).unwrap();
    w.write(b"x").unwrap();
    assert_eq!(wait_now(&ep, 8).len(), 1);
    w.write(b"y").unwrap();
    assert!(wait_now(&ep, 8).is_empty(), "disabled after one report");
    ep.ctl(EpollOp::Mod, 3, &r, ev(PollEvents::IN, EPOLLONESHOT, 8)).unwrap();
    assert_eq!(wait_now(&ep, 8), vec![EpollEvent { events: PollEvents::IN.bits(), data: 8 }]);
}

#[test]
fn epoll_ctl_errors() {
    let env = Env::new();
    let (ep, epf) = epoll_file(&env);
    let (r, _w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let regular = env.open("/f", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    let e = ev(PollEvents::IN, 0, 0);
    assert_eq!(ep.ctl(EpollOp::Add, 5, &epf, e), Err(Errno::EINVAL), "itself");
    assert_eq!(ep.ctl(EpollOp::Add, 6, &regular, e), Err(Errno::EPERM), "a regular file");
    assert_eq!(ep.ctl(EpollOp::Mod, 3, &r, e), Err(Errno::ENOENT));
    assert_eq!(ep.ctl(EpollOp::Del, 3, &r, e), Err(Errno::ENOENT));
    ep.ctl(EpollOp::Add, 3, &r, e).unwrap();
    assert_eq!(ep.ctl(EpollOp::Add, 3, &r, e), Err(Errno::EEXIST));
    // The same description under another descriptor is another entry.
    ep.ctl(EpollOp::Add, 9, &r, e).unwrap();
    assert_eq!(ep.len(), 2);
    ep.ctl(EpollOp::Del, 9, &r, e).unwrap();
    // EPOLLEXCLUSIVE: only with IN/OUT/WAKEUP/ET, never with MOD.
    let (r2, _w2) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let bad = ev(PollEvents::IN | PollEvents::PRI, EPOLLEXCLUSIVE, 0);
    assert_eq!(ep.ctl(EpollOp::Add, 4, &r2, bad), Err(Errno::EINVAL));
    ep.ctl(EpollOp::Add, 4, &r2, ev(PollEvents::IN, EPOLLEXCLUSIVE | EPOLLET, 0)).unwrap();
    assert_eq!(ep.ctl(EpollOp::Mod, 4, &r2, e), Err(Errno::EINVAL));
    assert_eq!(ep.ctl(EpollOp::Mod, 3, &r, ev(PollEvents::IN, EPOLLEXCLUSIVE, 0)), Err(Errno::EINVAL));
    assert_eq!(r.read(&mut [0u8; 1]), Err(Errno::EAGAIN));
    assert_eq!(epf.read(&mut [0u8; 8]), Err(Errno::EINVAL));
}

#[test]
fn epoll_refuses_loops_and_deep_nesting() {
    let env = Env::new();
    let sets: Vec<(Arc<Epoll>, Arc<OpenFile>)> = (0..7).map(|_| epoll_file(&env)).collect();
    let e = ev(PollEvents::IN, 0, 0);
    sets[0].0.ctl(EpollOp::Add, 1, &sets[1].1, e).unwrap();
    assert_eq!(sets[1].0.ctl(EpollOp::Add, 0, &sets[0].1, e), Err(Errno::ELOOP), "a loop");
    // A chain 1 -> 2 -> 3 -> 4 -> 5: below 6 it would nest six deep, the
    // chain from 2 on five deep, which is allowed.
    for i in 1..5 {
        sets[i].0.ctl(EpollOp::Add, 1, &sets[i + 1].1, e).unwrap();
    }
    assert_eq!(sets[6].0.ctl(EpollOp::Add, 1, &sets[1].1, e), Err(Errno::ELOOP), "too deep");
    sets[6].0.ctl(EpollOp::Add, 1, &sets[2].1, e).unwrap();
}

#[test]
fn nested_epoll_sets_pass_readiness_up() {
    let env = Env::new();
    let (outer, _) = epoll_file(&env);
    let (inner, inner_f) = epoll_file(&env);
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    inner.ctl(EpollOp::Add, 3, &r, ev(PollEvents::IN, 0, 1)).unwrap();
    outer.ctl(EpollOp::Add, 4, &inner_f, ev(PollEvents::IN, 0, 2)).unwrap();
    assert_eq!(inner_f.poll(), PollEvents::NONE);
    assert!(wait_now(&outer, 4).is_empty());
    w.write(b"x").unwrap();
    assert_eq!(inner_f.poll(), PollEvents::READABLE);
    assert_eq!(wait_now(&outer, 4), vec![EpollEvent { events: PollEvents::IN.bits(), data: 2 }]);
    assert_eq!(wait_now(&inner, 4), vec![EpollEvent { events: PollEvents::IN.bits(), data: 1 }]);
}

#[test]
fn epoll_drops_closed_files() {
    let env = Env::new();
    let (ep, _) = epoll_file(&env);
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    ep.ctl(EpollOp::Add, 3, &r, ev(PollEvents::IN, 0, 1)).unwrap();
    w.write(b"x").unwrap();
    drop(r);
    assert!(wait_now(&ep, 4).is_empty());
    assert_eq!(ep.len(), 0);
    // HUP and ERR come unasked.
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    ep.ctl(EpollOp::Add, 4, &w, ev(PollEvents::NONE, 0, 9)).unwrap();
    drop(r);
    assert_eq!(wait_now(&ep, 4), vec![EpollEvent { events: PollEvents::ERR.bits(), data: 9 }]);
}

#[test]
fn epoll_wait_blocks_until_an_event_or_the_timeout() {
    let env = Env::new();
    let (ep, _) = epoll_file(&env);
    let efd = object_file(&env, Arc::new(EventFd::new(0, false)), OpenFlags::NONBLOCK);
    ep.ctl(EpollOp::Add, 5, &efd, ev(PollEvents::IN, EPOLLET, 55)).unwrap();
    let start = now_ns();
    assert_eq!(ep.wait(4, &mut TestBlocker::default(), Timeout::At(start + 10 * MS)), Ok(vec![]));
    assert!(now_ns() >= start + 10 * MS);
    let ep2 = ep.clone();
    let waiter = thread::spawn(move || ep2.wait(4, &mut TestBlocker::default(), Timeout::Never));
    eventually(|| !ep.wait_queue().unwrap().is_empty());
    write_u64(&efd, 1).unwrap();
    assert_eq!(waiter.join().unwrap(), Ok(vec![EpollEvent { events: PollEvents::IN.bits(), data: 55 }]));

    let blocker = TestBlocker::default();
    let interrupt = blocker.interrupter();
    let ep2 = ep.clone();
    let waiter = thread::spawn(move || {
        let mut blocker = blocker;
        ep2.wait(4, &mut blocker, Timeout::Never)
    });
    eventually(|| !ep.wait_queue().unwrap().is_empty());
    interrupt.set(|f| f.interrupted = true);
    assert_eq!(waiter.join().unwrap(), Err(Errno::EINTR));
}

#[test]
fn epoll_reports_timer_expirations() {
    let env = Env::new();
    let (ep, _) = epoll_file(&env);
    let svc = Arc::new(FakeTimers::default());
    let t = TimerFd::new(TimerClock::Monotonic, svc.clone());
    let f = object_file(&env, t.clone(), OpenFlags::NONBLOCK);
    ep.ctl(EpollOp::Add, 3, &f, ev(PollEvents::IN, EPOLLET, 3)).unwrap();
    t.set(false, TimerSpec { value_ns: MS, interval_ns: MS });
    assert!(wait_now(&ep, 4).is_empty());
    svc.advance(MS);
    svc.fire();
    assert_eq!(wait_now(&ep, 4).len(), 1);
    assert_eq!(read_u64(&f), Ok(1));
    svc.advance(3 * MS);
    svc.fire();
    assert_eq!(wait_now(&ep, 4).len(), 1, "the next expiration is a new edge");
    assert_eq!(read_u64(&f), Ok(3));
}

#[test]
fn devices_and_objects_without_a_queue_are_always_ready() {
    let env = Env::new();
    env.vfs.register_char_device(device::NULL, Arc::new(device::Null)).unwrap();
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/null", FileType::CharDevice, 0o666, device::NULL).unwrap();
    let null = env.open("/null", OpenFlags::RDWR).unwrap();
    assert_eq!(null.poll(), PollEvents::READABLE | PollEvents::WRITABLE);
    assert!(null.wait_queue().is_none());
    let (ep, _) = epoll_file(&env);
    ep.ctl(EpollOp::Add, 3, &null, ev(PollEvents::IN, 0, 1)).unwrap();
    assert_eq!(wait_now(&ep, 2).len(), 1);
}

#[test]
fn pipe_room_comes_back_a_page_at_a_time() {
    use crate::pipe::{PIPE_CAPACITY as CAPACITY, PIPE_PAGE as PAGE};
    let env = Env::new();
    let (r, w) = pipe_files(&env, OpenFlags::NONBLOCK, false);
    let big = vec![7u8; 70_000];
    let mut buf = vec![0u8; 70_000];
    assert_eq!(w.write(&big), Ok(CAPACITY));
    assert_eq!(w.write(b"x"), Err(Errno::EAGAIN));
    assert_eq!(r.read(&mut buf[..100]), Ok(100));
    assert_eq!(w.write(&big[..50]), Err(Errno::EAGAIN), "a partly read page takes nothing");
    assert!(!w.poll().contains(PollEvents::WRITABLE));
    assert_eq!(r.read(&mut buf[..PAGE - 100]), Ok(PAGE - 100));
    assert!(w.poll().contains(PollEvents::WRITABLE));
    assert_eq!(w.write(&big[..PAGE]), Ok(PAGE));
    assert_eq!(w.write(b"x"), Err(Errno::EAGAIN));
    while r.read(&mut buf).is_ok_and(|n| n > 0) {}
    // The rest of the last page, then nothing: a long write is partial.
    assert_eq!(w.write(&big[..CAPACITY - 1000]), Ok(CAPACITY - 1000));
    assert_eq!(w.write(&big[..5000]), Ok(1000));
}

#[test]
fn fifos_open_as_ends_of_one_pipe() {
    use crate::pipe::PipeEnd;
    let env = Env::new();
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/f", FileType::Fifo, 0o600, DeviceId::default()).unwrap();
    let nb = OpenFlags::NONBLOCK;
    assert_eq!(env.open("/f", OpenFlags::WRONLY | nb).err(), Some(Errno::ENXIO), "no reader");
    let r = env.open("/f", OpenFlags::RDONLY | nb).unwrap();
    assert_eq!(r.kind(), FileType::Fifo);
    let mut buf = [0u8; 8];
    assert_eq!(r.read(&mut buf), Ok(0), "no writer yet: end of file");
    let w = env.open("/f", OpenFlags::WRONLY | nb).unwrap();
    assert_eq!(r.read(&mut buf), Err(Errno::EAGAIN));
    assert_eq!(w.write(b"fifo"), Ok(4));
    assert_eq!(r.read(&mut buf), Ok(4));
    assert_eq!(&buf[..4], b"fifo");
    assert_eq!(r.seek(0, Whence::Set), Err(Errno::ESPIPE));
    // A blocking open for writing finds the reader at once.
    let w2 = env.open("/f", OpenFlags::WRONLY).unwrap();
    let mut blocker = TestBlocker::default();
    assert_eq!(w2.object_as::<PipeEnd>().unwrap().wait_for_peer(&mut blocker), Ok(()));
    assert_eq!(blocker.blocks(), 0);
    drop((w, w2));
    assert_eq!(r.read(&mut buf), Ok(0), "end of file once the writers are gone");
    drop(r);
    // O_RDWR is both ends; with every end closed the data is gone.
    let rw = env.open("/f", OpenFlags::RDWR).unwrap();
    assert_eq!(rw.write(b"z"), Ok(1));
    assert_eq!(rw.read(&mut buf), Ok(1));
    assert_eq!(rw.write(b"left"), Ok(4));
    drop(rw);
    let r = env.open("/f", OpenFlags::RDONLY | nb).unwrap();
    assert_eq!(r.read(&mut buf), Ok(0));
}

#[test]
fn a_blocking_fifo_open_waits_for_the_other_side() {
    use crate::pipe::PipeEnd;
    let env = Arc::new(Env::new());
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/f", FileType::Fifo, 0o600, DeviceId::default()).unwrap();
    let r = env.open("/f", OpenFlags::RDONLY).unwrap();
    let end = r.object_as::<PipeEnd>().unwrap();
    let mut blocker = TestBlocker::default();
    let opener = {
        let env = Arc::clone(&env);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            env.open("/f", OpenFlags::WRONLY).unwrap()
        })
    };
    assert_eq!(end.wait_for_peer(&mut blocker), Ok(()));
    let _w = opener.join().unwrap();
    // Interrupted: EINTR.
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, b"/h", FileType::Fifo, 0o600, DeviceId::default()).unwrap();
    let lone = env.open("/h", OpenFlags::RDONLY).unwrap();
    let mut blocker = TestBlocker::default();
    let signal = blocker.interrupter();
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        signal.set(|f| f.interrupted = true);
    });
    assert_eq!(lone.object_as::<PipeEnd>().unwrap().wait_for_peer(&mut blocker), Err(Errno::EINTR));
    t.join().unwrap();
}
