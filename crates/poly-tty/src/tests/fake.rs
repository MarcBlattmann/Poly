//! A fake host: callers per thread, recorded signals, a process group
//! table, user memory as a byte array, blocking with a condition variable,
//! and a driver that records output.

use crate::{Caller, Driver, Host, Ttys};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use poly_vfs::{Blocker, Errno, PollEvents, Result, Waker, Woke};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

std::thread_local! {
    static CALLER: Cell<Option<Caller>> = const { Cell::new(None) };
}

/// Make the calling thread act as `c` (None: the kernel).
pub fn act_as(c: Option<Caller>) {
    CALLER.with(|x| x.set(c));
}

/// A caller: process `pid` in group `pgid` of session `sid`, root.
pub fn caller(pid: i32, pgid: i32, sid: i32) -> Caller {
    Caller { pid, pgid, sid, uid: 1000, gid: 1000, privileged: false }
}

/// Where fake user memory starts.
pub const USER: u64 = 0x10_0000;
const USER_LEN: usize = 0x1_0000;

pub struct FakeHost {
    start: Instant,
    pub signals: Mutex<Vec<(Target, u32)>>,
    /// Process group -> session.
    pub groups: Mutex<BTreeMap<i32, i32>>,
    pub orphaned: Mutex<BTreeSet<i32>>,
    /// Signals every caller blocks or ignores.
    pub ignored: Mutex<BTreeSet<u32>>,
    memory: Mutex<Vec<u8>>,
    /// Wakers of blocked threads, for interrupting them.
    blocked: Mutex<Vec<Arc<TestWaker>>>,
    /// Threads in `block` now.
    pub blocking: Arc<AtomicUsize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Group(i32),
    Process(i32),
}

impl FakeHost {
    pub fn new() -> Arc<FakeHost> {
        Arc::new(FakeHost {
            start: Instant::now(),
            signals: Mutex::new(Vec::new()),
            groups: Mutex::new(BTreeMap::new()),
            orphaned: Mutex::new(BTreeSet::new()),
            ignored: Mutex::new(BTreeSet::new()),
            memory: Mutex::new(vec![0; USER_LEN]),
            blocked: Mutex::new(Vec::new()),
            blocking: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn ttys(self: &Arc<Self>) -> Arc<Ttys> {
        Ttys::new(self.clone())
    }

    pub fn take_signals(&self) -> Vec<(Target, u32)> {
        core::mem::take(&mut *self.signals.lock().unwrap())
    }

    pub fn poke(&self, addr: u64, bytes: &[u8]) {
        let at = (addr - USER) as usize;
        self.memory.lock().unwrap()[at..at + bytes.len()].copy_from_slice(bytes);
    }

    pub fn peek(&self, addr: u64, len: usize) -> Vec<u8> {
        let at = (addr - USER) as usize;
        self.memory.lock().unwrap()[at..at + len].to_vec()
    }

    pub fn peek_int(&self, addr: u64) -> i32 {
        i32::from_le_bytes(self.peek(addr, 4).try_into().unwrap())
    }

    /// Interrupt every blocked thread (as a signal would).
    pub fn interrupt(&self) {
        for w in self.blocked.lock().unwrap().iter() {
            w.interrupted.store(true, Ordering::SeqCst);
            w.wake(PollEvents::NONE);
        }
    }

    /// Wait until `n` threads are blocked.
    pub fn wait_blocked(&self, n: usize) {
        let end = Instant::now() + Duration::from_secs(30);
        while self.blocking.load(Ordering::SeqCst) < n {
            assert!(Instant::now() < end, "threads did not block");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Host for FakeHost {
    fn caller(&self) -> Option<Caller> {
        CALLER.with(Cell::get)
    }

    fn blocks_or_ignores(&self, signal: u32) -> bool {
        self.ignored.lock().unwrap().contains(&signal)
    }

    fn signal_group(&self, pgid: i32, signal: u32) {
        self.signals.lock().unwrap().push((Target::Group(pgid), signal));
    }

    fn signal_process(&self, pid: i32, signal: u32) {
        self.signals.lock().unwrap().push((Target::Process(pid), signal));
    }

    fn session_of_group(&self, pgid: i32) -> Option<i32> {
        self.groups.lock().unwrap().get(&pgid).copied()
    }

    fn group_orphaned(&self, pgid: i32) -> bool {
        self.orphaned.lock().unwrap().contains(&pgid)
    }

    fn now_ns(&self) -> u64 {
        self.start.elapsed().as_nanos() as u64
    }

    fn blocker(&self) -> Option<Box<dyn Blocker>> {
        let waker = Arc::new(TestWaker::default());
        self.blocked.lock().unwrap().push(waker.clone());
        Some(Box::new(TestBlocker { waker, start: self.start, blocking: self.blocking.clone() }))
    }

    fn copy_from_user(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        let at = addr.checked_sub(USER).ok_or(Errno::EFAULT)? as usize;
        let m = self.memory.lock().unwrap();
        let src = m.get(at..at + buf.len()).ok_or(Errno::EFAULT)?;
        buf.copy_from_slice(src);
        Ok(())
    }

    fn copy_to_user(&self, addr: u64, bytes: &[u8]) -> Result<()> {
        let at = addr.checked_sub(USER).ok_or(Errno::EFAULT)? as usize;
        let mut m = self.memory.lock().unwrap();
        let dst = m.get_mut(at..at + bytes.len()).ok_or(Errno::EFAULT)?;
        dst.copy_from_slice(bytes);
        Ok(())
    }
}

#[derive(Default)]
pub struct TestWaker {
    woken: Mutex<bool>,
    cv: Condvar,
    interrupted: AtomicBool,
}

impl Waker for TestWaker {
    fn wake(&self, _: PollEvents) {
        *self.woken.lock().unwrap() = true;
        self.cv.notify_all();
    }
}

struct TestBlocker {
    waker: Arc<TestWaker>,
    /// The host's clock and count of blocked threads.
    start: Instant,
    blocking: Arc<AtomicUsize>,
}

impl Blocker for TestBlocker {
    fn waker(&self) -> Arc<dyn Waker> {
        self.waker.clone()
    }

    fn block(&mut self, until: Option<u64>) -> Woke {
        self.blocking.fetch_add(1, Ordering::SeqCst);
        let mut woken = self.waker.woken.lock().unwrap();
        let result = loop {
            if self.waker.interrupted.swap(false, Ordering::SeqCst) {
                break Woke::Interrupted;
            }
            if *woken {
                *woken = false;
                break Woke::Woken;
            }
            match until {
                Some(t) => {
                    let now = self.start.elapsed().as_nanos() as u64;
                    if now >= t {
                        break Woke::TimedOut;
                    }
                    woken = self.waker.cv.wait_timeout(woken, Duration::from_nanos(t - now)).unwrap().0;
                }
                None => woken = self.waker.cv.wait(woken).unwrap(),
            }
        };
        self.blocking.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

/// A driver that records what the terminal sent; `limit` bytes of room
/// at most (like a pty master), unlimited with `None`.
#[derive(Default)]
pub struct Screen {
    pub out: Mutex<Vec<u8>>,
    pub opened: Mutex<Vec<bool>>,
    pub flushed: AtomicUsize,
}

impl Screen {
    pub fn take(&self) -> Vec<u8> {
        core::mem::take(&mut *self.out.lock().unwrap())
    }

    pub fn text(&self) -> std::string::String {
        std::string::String::from_utf8_lossy(&self.take()).into_owned()
    }
}

impl Driver for Screen {
    fn write(&self, bytes: &[u8]) {
        self.out.lock().unwrap().extend_from_slice(bytes);
    }

    fn flush(&self) {
        self.flushed.fetch_add(1, Ordering::SeqCst);
    }

    fn opened(&self, open: bool) {
        self.opened.lock().unwrap().push(open);
    }
}
