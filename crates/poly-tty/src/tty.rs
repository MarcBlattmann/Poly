//! A terminal: a line discipline between a [`Driver`] (the device side: a
//! serial line, the screen, a pty master) and the processes that open it,
//! with the job control of POSIX's "General Terminal Interface" and
//! credentials(7): sessions, the controlling terminal, the foreground
//! process group.
//!
//! - **Opening** ([`Tty::open`]) gives a [`TtyFile`], the device an open
//!   file reads and writes through; the terminal counts them (the driver
//!   hears when the first opens and the last closes). A session leader
//!   without a controlling terminal that opens a terminal no session has,
//!   without `O_NOCTTY`, makes it its controlling terminal, with its
//!   process group in the foreground.
//! - **Job control.** A process of the terminal's session that is not in
//!   the foreground group and reads gets `SIGTTIN` sent to its group and
//!   the read is interrupted (restarted after the stop); `EIO` instead if
//!   it blocks or ignores `SIGTTIN` or its group is orphaned. Writing with
//!   `TOSTOP`, and changing the terminal (`tcsetattr`, `tcsetpgrp`,
//!   `tcflush`, `tcflow`, `tcdrain`, `tcsetwinsize`) at any time, does the
//!   same with `SIGTTOU`, except that a process blocking or ignoring
//!   `SIGTTOU` goes ahead.
//! - **Signals from input**: `INTR`, `QUIT` and `SUSP` go to the
//!   foreground group; a window size change sends `SIGWINCH` to it.
//! - **Hangup**: when the controlling process (the session leader) ends,
//!   the foreground group gets `SIGHUP` and the session loses the terminal
//!   ([`Ttys::process_exited`]); a pty whose master closed is hung up for
//!   good ([`Tty::hangup`]): reads return end of file, writes `EIO`.
//!
//! Locks: [`Ttys`]'s session table, then a terminal's state; both are
//! spinlocks held for memory work and the driver's `write` only, never
//! across a user copy, a block or a signal.

use crate::ioctl::*;
use crate::ldisc::{Effects, LineDiscipline, Read};
use crate::sig::{SIGCONT, SIGHUP, SIGTTIN, SIGTTOU, SIGWINCH};
use crate::termios::{TCIFLUSH, TCIOFF, TCIOFLUSH, TCION, TCOFLUSH, TCOOFF, TCOON, Termios, Winsize, cc, lflag};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_vfs::lock::{Lock, RawSpinLock};
use poly_vfs::poll::Registration;
use poly_vfs::{Blocker, CharDevice, Errno, OpenFlags, PollEvents, Result, WaitQueue, Woke};

/// Program output handled per hold of the terminal's lock.
const OUTPUT_CHUNK: usize = 256;

/// The process making the current call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub pid: i32,
    pub pgid: i32,
    pub sid: i32,
    /// Effective user and group ids (owner of a new pty).
    pub uid: u32,
    pub gid: u32,
    /// Whether it may do what needs privilege (`CAP_SYS_ADMIN`: stealing
    /// a controlling terminal, `TIOCSTI` on another terminal, opening an
    /// exclusive one).
    pub privileged: bool,
}

impl Caller {
    pub fn session_leader(&self) -> bool {
        self.pid == self.sid
    }
}

/// What terminals need from the kernel.
pub trait Host: Send + Sync {
    /// The process making the current call; `None` when the kernel itself
    /// calls (no job control then).
    fn caller(&self) -> Option<Caller>;

    /// Whether the calling thread blocks or ignores `signal`.
    fn blocks_or_ignores(&self, signal: u32) -> bool;

    /// Send `signal` to every process of group `pgid`, from the kernel (no
    /// permission check).
    fn signal_group(&self, pgid: i32, signal: u32);

    /// Send `signal` to process `pid`, from the kernel.
    fn signal_process(&self, pid: i32, signal: u32);

    /// The session of process group `pgid`, if the group exists.
    fn session_of_group(&self, pgid: i32) -> Option<i32>;

    /// Whether process group `pgid` is orphaned (POSIX: "the parent of
    /// every member is either itself a member of the group or is not a
    /// member of the group's session").
    fn group_orphaned(&self, pgid: i32) -> bool;

    /// The monotonic clock, in nanoseconds.
    fn now_ns(&self) -> u64;

    /// A blocker for the calling thread; `None` if it cannot block.
    fn blocker(&self) -> Option<Box<dyn Blocker>>;

    /// Copy `buf.len()` bytes from the caller's memory at `addr`
    /// (`EFAULT`).
    fn copy_from_user(&self, addr: u64, buf: &mut [u8]) -> Result<()>;

    /// Copy `bytes` to the caller's memory at `addr` (`EFAULT`).
    fn copy_to_user(&self, addr: u64, bytes: &[u8]) -> Result<()>;
}

/// The device side of a terminal. Its methods are called with the
/// terminal's lock held: they must not block or call back into the
/// terminal.
pub trait Driver: Send + Sync {
    /// Send `bytes` (at most [`room`](Self::room)) to the device.
    fn write(&self, bytes: &[u8]);

    /// How many bytes [`write`](Self::write) can take now; a pty master
    /// whose buffer is full takes fewer.
    fn room(&self) -> usize {
        usize::MAX
    }

    /// Output not sent yet (`TIOCOUTQ`).
    fn queued(&self) -> usize {
        0
    }

    /// Throw away output not sent yet (`TCOFLUSH`).
    fn flush(&self) {}

    /// The first open of the terminal (`true`) or its last close.
    fn opened(&self, _open: bool) {}

    /// Readers took input, so the discipline has room again (a pty
    /// master waiting to write is woken).
    fn input_drained(&self) {}

    /// A request only some devices answer (a serial port's `TIOCGSERIAL`,
    /// a virtual console's `VT_GETSTATE`, see [`crate::hw`]): the bytes the
    /// request stores at its argument, or `None` (`ENOTTY`).
    fn device_ioctl(&self, _request: u32) -> Option<alloc::vec::Vec<u8>> {
        None
    }
}

/// The terminals of one machine: what they share (the host, which
/// session has which controlling terminal).
pub struct Ttys {
    host: Arc<dyn Host>,
    sessions: Lock<RawSpinLock, Sessions>,
}

#[derive(Default)]
struct Sessions {
    /// Each session's controlling terminal.
    ctty: BTreeMap<i32, Weak<Tty>>,
    /// Processes that gave up their controlling terminal (`TIOCNOTTY`)
    /// without being their session's leader.
    detached: BTreeSet<i32>,
}

impl Sessions {
    fn of(&self, sid: i32) -> Option<Arc<Tty>> {
        self.ctty.get(&sid).and_then(Weak::upgrade)
    }
}

impl core::fmt::Debug for Ttys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ttys").finish_non_exhaustive()
    }
}

impl Ttys {
    pub fn new(host: Arc<dyn Host>) -> Arc<Ttys> {
        Arc::new(Ttys { host, sessions: Lock::new(Sessions::default()) })
    }

    pub fn host(&self) -> &dyn Host {
        &*self.host
    }

    /// The controlling terminal of the calling process (`/dev/tty`).
    pub fn controlling(&self) -> Option<Arc<Tty>> {
        let c = self.host.caller()?;
        let s = self.sessions.lock();
        if s.detached.contains(&c.pid) {
            return None;
        }
        s.of(c.sid)
    }

    /// The controlling terminal of session `sid`.
    pub fn of_session(&self, sid: i32) -> Option<Arc<Tty>> {
        self.sessions.lock().of(sid)
    }

    /// Process `pid` of session `sid` ended. If it was the session's
    /// leader (the controlling process), the terminal's foreground group
    /// gets `SIGHUP` and the session loses the terminal (POSIX `_exit`).
    pub fn process_exited(&self, pid: i32, sid: i32) {
        let tty = {
            let mut s = self.sessions.lock();
            s.detached.remove(&pid);
            if pid != sid {
                return;
            }
            let tty = s.ctty.remove(&sid).and_then(|w| w.upgrade());
            s.ctty.retain(|_, w| w.strong_count() > 0);
            tty
        };
        if let Some(tty) = tty
            && let Some(pgrp) = tty.disassociate(sid)
        {
            self.host.signal_group(pgrp, SIGHUP);
        }
    }

    fn is_detached(&self, pid: i32) -> bool {
        self.sessions.lock().detached.contains(&pid)
    }
}

/// A terminal (see the module documentation).
pub struct Tty {
    ttys: Arc<Ttys>,
    name: String,
    driver: Arc<dyn Driver>,
    state: Lock<RawSpinLock, State>,
    queue: Arc<WaitQueue>,
    opens: AtomicUsize,
    me: Weak<Tty>,
}

struct State {
    ldisc: LineDiscipline,
    winsize: Winsize,
    /// The session whose controlling terminal this is.
    session: Option<i32>,
    /// The foreground process group.
    pgrp: Option<i32>,
    hung_up: bool,
    exclusive: bool,
}

impl core::fmt::Debug for Tty {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tty").field("name", &self.name).finish_non_exhaustive()
    }
}

/// What a job control check decided.
enum Job {
    Go,
    /// Interrupted: a signal went to the caller's group.
    Signalled,
    Eio,
}

impl Tty {
    /// A terminal called `name` (`ttyS0`, `pts/3`) on `driver`.
    pub fn new(ttys: &Arc<Ttys>, name: &str, driver: Arc<dyn Driver>, termios: Termios, winsize: Winsize) -> Arc<Tty> {
        Arc::new_cyclic(|me| Tty {
            ttys: Arc::clone(ttys),
            name: String::from(name),
            driver,
            state: Lock::new(State {
                ldisc: LineDiscipline::new(termios),
                winsize,
                session: None,
                pgrp: None,
                hung_up: false,
                exclusive: false,
            }),
            queue: Arc::new(WaitQueue::new()),
            opens: AtomicUsize::new(0),
            me: me.clone(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The terminals this one shares sessions with.
    pub fn ttys(&self) -> &Arc<Ttys> {
        &self.ttys
    }

    fn host(&self) -> &dyn Host {
        self.ttys.host()
    }

    /// Where readers and writers wait.
    pub fn queue(&self) -> &Arc<WaitQueue> {
        &self.queue
    }

    /// Open files on the terminal.
    pub fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }

    pub fn termios(&self) -> Termios {
        *self.state.lock().ldisc.termios()
    }

    pub fn winsize(&self) -> Winsize {
        self.state.lock().winsize
    }

    /// The session whose controlling terminal this is, and its foreground
    /// process group.
    pub fn session(&self) -> (Option<i32>, Option<i32>) {
        let s = self.state.lock();
        (s.session, s.pgrp)
    }

    pub fn hung_up(&self) -> bool {
        self.state.lock().hung_up
    }

    /// Open the terminal: the device for one open file. `may_acquire`:
    /// this open may make it the caller's controlling terminal (not
    /// through `/dev/console` or `/dev/tty`). `EBUSY` in exclusive mode
    /// (`TIOCEXCL`) unless privileged.
    pub fn open(self: &Arc<Self>, flags: OpenFlags, may_acquire: bool) -> Result<Arc<TtyFile>> {
        let caller = self.host().caller();
        {
            let mut sessions = self.ttys.sessions.lock();
            let mut st = self.state.lock();
            if st.exclusive && !caller.is_some_and(|c| c.privileged) {
                return Err(Errno::EBUSY);
            }
            if let Some(c) = caller
                && may_acquire
                && !flags.contains(OpenFlags::NOCTTY)
                && c.session_leader()
                && sessions.of(c.sid).is_none()
                && st.session.is_none()
                && !st.hung_up
            {
                sessions.ctty.insert(c.sid, self.me.clone());
                st.session = Some(c.sid);
                st.pgrp = Some(c.pgid);
            }
        }
        if self.opens.fetch_add(1, Ordering::SeqCst) == 0 {
            self.driver.opened(true);
        }
        Ok(Arc::new(TtyFile { tty: Arc::clone(self) }))
    }

    fn closed(&self) {
        if self.opens.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.driver.opened(false);
        }
    }

    /// The session lost the terminal: returns the foreground group it had.
    fn disassociate(&self, sid: i32) -> Option<i32> {
        let mut st = self.state.lock();
        if st.session != Some(sid) {
            return None;
        }
        st.session = None;
        st.pgrp.take()
    }

    /// Whether this is the controlling terminal of `c`.
    fn is_ctty_of(&self, c: &Caller) -> bool {
        self.state.lock().session == Some(c.sid) && !self.ttys.is_detached(c.pid)
    }

    /// The job control check for the calling process on `signal`'s
    /// operation (see the module documentation). `always`: `SIGTTOU` does
    /// not depend on `TOSTOP` (changing the terminal).
    fn job_control(&self, signal: u32, always: bool) -> Job {
        let host = self.host();
        let Some(c) = host.caller() else { return Job::Go };
        let detached = self.ttys.is_detached(c.pid);
        let (session, pgrp, tostop) = {
            let st = self.state.lock();
            (st.session, st.pgrp, st.ldisc.termios().has_l(lflag::TOSTOP))
        };
        if detached || session != Some(c.sid) || pgrp.is_none() || pgrp == Some(c.pgid) {
            return Job::Go;
        }
        if signal == SIGTTOU && !always && !tostop {
            return Job::Go;
        }
        let ignored = host.blocks_or_ignores(signal);
        if signal == SIGTTOU && ignored {
            return Job::Go;
        }
        if ignored || host.group_orphaned(c.pgid) {
            return Job::Eio;
        }
        host.signal_group(c.pgid, signal);
        Job::Signalled
    }

    fn check(&self, signal: u32, always: bool) -> Result<()> {
        match self.job_control(signal, always) {
            Job::Go => Ok(()),
            // The system call layer restarts the call after the stop
            // (signal(7)), or fails with EINTR after a handler.
            Job::Signalled => Err(Errno::EINTR),
            Job::Eio => Err(Errno::EIO),
        }
    }

    /// Act on what the discipline asked for, with the state locked:
    /// output to the driver.
    fn act_locked(&self, fx: &Effects) {
        if !fx.output.is_empty() {
            self.driver.write(&fx.output);
        }
        if fx.flush_output {
            self.driver.flush();
        }
    }

    /// Act on the rest, unlocked: wake waiters, send signals to `pgrp`.
    fn act_unlocked(&self, fx: &Effects, pgrp: Option<i32>) {
        if fx.readable {
            self.queue.notify(PollEvents::READABLE);
        }
        if fx.output_started {
            self.queue.notify(PollEvents::WRITABLE);
        }
        if let Some(pgrp) = pgrp {
            for &s in &fx.signals {
                self.host().signal_group(pgrp, s);
            }
        }
    }

    // ---- input from the device -----------------------------------------------

    /// Bytes from the device (keys typed, a serial line): all of them are
    /// processed; what does not fit in the input buffer is dropped.
    pub fn receive(&self, bytes: &[u8]) {
        self.receive_up_to(bytes, false);
    }

    /// Bytes from a pty master: takes only what fits ([`input_room`]);
    /// returns how many.
    ///
    /// [`input_room`]: Self::input_room
    pub fn receive_some(&self, bytes: &[u8]) -> usize {
        self.receive_up_to(bytes, true)
    }

    fn receive_up_to(&self, bytes: &[u8], only_room: bool) -> usize {
        let now = self.host().now_ns();
        let mut fx = Effects::default();
        let (n, pgrp) = {
            let mut st = self.state.lock();
            let n = if only_room { bytes.len().min(st.ldisc.room()) } else { bytes.len() };
            st.ldisc.receive(&bytes[..n], now, &mut fx);
            self.act_locked(&fx);
            (n, st.pgrp)
        };
        self.act_unlocked(&fx, pgrp);
        n
    }

    /// How much input a pty master may write now.
    pub fn input_room(&self) -> usize {
        self.state.lock().ldisc.room()
    }

    /// Throw away input (`TCIFLUSH`).
    pub fn flush_input(&self) {
        let mut fx = Effects::default();
        {
            let mut st = self.state.lock();
            st.ldisc.flush_input(&mut fx);
            self.act_locked(&fx);
        }
        self.driver.input_drained();
    }

    // ---- reading and writing ------------------------------------------------------

    /// `read(2)` (see [`LineDiscipline::read`] for what satisfies it).
    pub fn read(&self, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        self.check(SIGTTIN, true)?;
        if buf.is_empty() {
            return Ok(0);
        }
        let host = self.host();
        let start = host.now_ns();
        let mut blocker: Option<Box<dyn Blocker>> = None;
        let mut _registered: Option<Registration> = None;
        loop {
            let mut fx = Effects::default();
            let r = {
                let mut st = self.state.lock();
                if st.hung_up {
                    return Ok(0);
                }
                let r = st.ldisc.read(buf, start, host.now_ns(), &mut fx);
                self.act_locked(&fx);
                r
            };
            match r {
                Read::Data(n) => {
                    if n > 0 {
                        self.driver.input_drained();
                    }
                    return Ok(n);
                }
                Read::Eof => return Ok(0),
                Read::Wait { until } => {
                    if nonblock {
                        return Err(Errno::EAGAIN);
                    }
                    let Some(b) = blocker.as_mut() else {
                        // Register first, then look again: input that
                        // arrives in between still wakes us.
                        let b = host.blocker().ok_or(Errno::EAGAIN)?;
                        _registered = Some(self.queue.register(PollEvents::READABLE, b.waker()));
                        blocker = Some(b);
                        continue;
                    };
                    if b.block(until) == Woke::Interrupted {
                        return Err(Errno::EINTR);
                    }
                    // The foreground group may have changed meanwhile.
                    self.check(SIGTTIN, true)?;
                }
            }
        }
    }

    /// `write(2)`: all of `buf`, waiting while output is stopped or the
    /// device has no room (unless `nonblock`). A partial count when
    /// interrupted after some bytes went out.
    pub fn write(&self, buf: &[u8], nonblock: bool) -> Result<usize> {
        self.check(SIGTTOU, false)?;
        let mut done = 0;
        let mut blocker: Option<Box<dyn Blocker>> = None;
        let mut _registered: Option<Registration> = None;
        while done < buf.len() {
            let n = {
                let mut st = self.state.lock();
                if st.hung_up {
                    return if done > 0 { Ok(done) } else { Err(Errno::EIO) };
                }
                if st.ldisc.output_stopped() {
                    0
                } else {
                    let room = self.driver.room();
                    let end = buf.len().min(done + OUTPUT_CHUNK);
                    let mut out = Vec::new();
                    let n = st.ldisc.output(&buf[done..end], room, &mut out);
                    if !out.is_empty() {
                        self.driver.write(&out);
                    }
                    n
                }
            };
            if n > 0 {
                done += n;
                continue;
            }
            if nonblock {
                return if done > 0 { Ok(done) } else { Err(Errno::EAGAIN) };
            }
            let Some(b) = blocker.as_mut() else {
                let Some(b) = self.host().blocker() else {
                    return if done > 0 { Ok(done) } else { Err(Errno::EAGAIN) };
                };
                _registered = Some(self.queue.register(PollEvents::WRITABLE, b.waker()));
                blocker = Some(b);
                continue;
            };
            if b.block(None) == Woke::Interrupted {
                return if done > 0 { Ok(done) } else { Err(Errno::EINTR) };
            }
        }
        Ok(done)
    }

    /// What the terminal is ready for (`poll(2)`).
    pub fn poll(&self) -> PollEvents {
        let st = self.state.lock();
        if st.hung_up {
            return PollEvents::READABLE | PollEvents::WRITABLE | PollEvents::HUP;
        }
        let mut ev = PollEvents::NONE;
        if st.ldisc.readable() {
            ev = ev | PollEvents::READABLE;
        }
        if !st.ldisc.output_stopped() && self.driver.room() > 0 {
            ev = ev | PollEvents::WRITABLE;
        }
        ev
    }

    /// The device made room for output (a pty master read): wake writers.
    pub fn output_room(&self) {
        self.queue.notify(PollEvents::WRITABLE);
    }

    /// The device hung up for good (the master of a pty closed): the
    /// controlling process gets `SIGHUP`, the foreground group `SIGHUP`
    /// and `SIGCONT`, the session loses the terminal; reads return end of
    /// file from now on, writes `EIO` (POSIX "Modem Disconnect"; who gets
    /// which signal is Poly's choice, to be confirmed by the diff runner).
    ///
    /// The signals are sent before the hang-up shows: a reader that sees
    /// end of file has its `SIGHUP` pending already, so it takes it on the
    /// way out of that read (ioctl's `b_ioctl_ctty` counts it right after
    /// the read returns), and a blocked reader is interrupted by the
    /// signal first and reads end of file when it tries again.
    pub fn hangup(&self) {
        let (session, pgrp) = {
            let mut sessions = self.ttys.sessions.lock();
            let mut st = self.state.lock();
            let session = st.session.take();
            if let Some(sid) = session {
                sessions.ctty.remove(&sid);
            }
            (session, st.pgrp.take())
        };
        let host = self.host();
        // The session leader gets one SIGHUP (ioctl's spec sheet), also
        // when its own group is the foreground group the signal goes to.
        if let Some(sid) = session.filter(|&sid| pgrp != Some(sid)) {
            host.signal_process(sid, SIGHUP);
        }
        if let Some(pgrp) = pgrp {
            host.signal_group(pgrp, SIGHUP);
            host.signal_group(pgrp, SIGCONT);
        }
        {
            let mut st = self.state.lock();
            st.hung_up = true;
            let mut fx = Effects::default();
            st.ldisc.flush_input(&mut fx);
        }
        self.queue.notify(PollEvents::ALL);
    }

    // ---- attributes --------------------------------------------------------------

    /// Set the attributes (`flush`: throw away input first, `TCSETSF`).
    pub fn set_termios(&self, new: Termios, flush: bool) {
        let mut fx = Effects::default();
        {
            let mut st = self.state.lock();
            st.ldisc.set_termios(new, flush, &mut fx);
            self.act_locked(&fx);
        }
        self.act_unlocked(&fx, None);
        if flush {
            self.driver.input_drained();
        }
    }

    /// Set the window size; a change sends `SIGWINCH` to the foreground
    /// group (ioctl_tty(2)).
    pub fn set_winsize(&self, ws: Winsize) {
        let pgrp = {
            let mut st = self.state.lock();
            let changed = st.winsize != ws;
            st.winsize = ws;
            if changed { st.pgrp } else { None }
        };
        if let Some(pgrp) = pgrp {
            self.host().signal_group(pgrp, SIGWINCH);
        }
    }

    /// Stop or restart output, or send `STOP` / `START` (`tcflow`).
    pub fn flow(&self, action: u32) -> Result<()> {
        let mut started = false;
        {
            let mut st = self.state.lock();
            let t = *st.ldisc.termios();
            match action {
                TCOOFF => {
                    st.ldisc.set_output_stopped(true);
                }
                TCOON => started = st.ldisc.set_output_stopped(false),
                TCIOFF | TCION => {
                    let index = if action == TCIOFF { cc::VSTOP } else { cc::VSTART };
                    if let Some(c) = t.special(index) {
                        self.driver.write(&[c]);
                    }
                }
                _ => return Err(Errno::EINVAL),
            }
        }
        if started {
            self.queue.notify(PollEvents::WRITABLE);
        }
        Ok(())
    }

    /// Bytes a read could take (`FIONREAD`).
    pub fn readable_bytes(&self) -> usize {
        self.state.lock().ldisc.readable_bytes()
    }

    /// Output not sent yet (`TIOCOUTQ`).
    pub fn queued_output(&self) -> usize {
        self.driver.queued()
    }

    /// Throw away output (`TCOFLUSH`).
    pub fn flush_output(&self) {
        let _st = self.state.lock();
        self.driver.flush();
    }

    // ---- ioctl ---------------------------------------------------------------------

    fn get<const N: usize>(&self, arg: u64) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.host().copy_from_user(arg, &mut b)?;
        Ok(b)
    }

    fn put(&self, arg: u64, bytes: &[u8]) -> Result<u64> {
        self.host().copy_to_user(arg, bytes).map(|()| 0)
    }

    fn put_int(&self, arg: u64, v: i32) -> Result<u64> {
        self.put(arg, &v.to_le_bytes())
    }

    fn get_int(&self, arg: u64) -> Result<i32> {
        self.get::<4>(arg).map(i32::from_le_bytes)
    }

    /// The terminal requests every terminal (and a pty master, for its
    /// slave) answers alike: attributes, window size, queues, flow.
    /// `job`: changes are subject to job control (not from a master).
    /// `None` if `request` is not one of them.
    pub fn common_ioctl(&self, request: u32, arg: u64, job: bool) -> Option<Result<u64>> {
        let r = match request {
            TCGETS => self.put(arg, &self.termios().to_bytes()),
            TCGETS2 => self.put(arg, &self.termios().to_bytes2()),
            TCGETA => self.put(arg, &self.termios().to_termio()),
            TCSETS | TCSETSW | TCSETSF => self.change(job, |t| {
                let new = t.termios().from_bytes(&t.get(arg)?);
                t.set_termios(new, request == TCSETSF);
                Ok(())
            }),
            TCSETS2 | TCSETSW2 | TCSETSF2 => self.change(job, |t| {
                let new = t.termios().from_bytes2(&t.get(arg)?);
                t.set_termios(new, request == TCSETSF2);
                Ok(())
            }),
            TCSETA | TCSETAW | TCSETAF => self.change(job, |t| {
                let new = t.termios().from_termio(&t.get(arg)?);
                t.set_termios(new, request == TCSETAF);
                Ok(())
            }),
            TIOCGWINSZ => self.put(arg, &self.winsize().to_bytes()),
            TIOCSWINSZ => self.change(job, |t| {
                t.set_winsize(Winsize::from_bytes(&t.get(arg)?));
                Ok(())
            }),
            TCXONC => self.change(job, |t| t.flow(arg as u32)),
            // Output goes to the device as it is written, so a drain has
            // nothing to wait for; there is no line to send a break on.
            TCSBRK | TCSBRKP => self.change(job, |_| Ok(())),
            TIOCEXCL | TIOCNXCL => {
                self.state.lock().exclusive = request == TIOCEXCL;
                Ok(0)
            }
            TIOCGEXCL => {
                let excl = self.state.lock().exclusive;
                self.put_int(arg, i32::from(excl))
            }
            TIOCGETD => self.put_int(arg, crate::termios::N_TTY as i32),
            TIOCSETD => self.get_int(arg).and_then(|d| if d == 0 { Ok(0) } else { Err(Errno::EINVAL) }),
            _ => return None,
        };
        Some(r)
    }

    /// A change of the terminal: job control first if `job` (`SIGTTOU`
    /// whatever `TOSTOP` says), then `f`.
    fn change(&self, job: bool, f: impl FnOnce(&Tty) -> Result<()>) -> Result<u64> {
        if job {
            self.check(SIGTTOU, true)?;
        }
        f(self).map(|()| 0)
    }

    /// `ioctl(2)` on the terminal (ioctl_tty(2)).
    pub fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        let host = self.host();
        match request {
            FIONREAD => self.put_int(arg, self.readable_bytes().min(i32::MAX as usize) as i32),
            TIOCOUTQ => self.put_int(arg, self.queued_output().min(i32::MAX as usize) as i32),
            TCFLSH => {
                self.check(SIGTTOU, true)?;
                match arg as u32 {
                    TCIFLUSH => self.flush_input(),
                    TCOFLUSH => self.flush_output(),
                    TCIOFLUSH => {
                        self.flush_input();
                        self.flush_output();
                    }
                    _ => return Err(Errno::EINVAL),
                }
                Ok(0)
            }
            TIOCSCTTY => self.set_controlling(arg),
            TIOCNOTTY => self.give_up_controlling(),
            TIOCGPGRP => {
                let c = host.caller().ok_or(Errno::ENOTTY)?;
                if !self.is_ctty_of(&c) {
                    return Err(Errno::ENOTTY);
                }
                let pgrp = self.state.lock().pgrp.unwrap_or(0);
                self.put_int(arg, pgrp)
            }
            TIOCSPGRP => {
                let c = host.caller().ok_or(Errno::ENOTTY)?;
                if !self.is_ctty_of(&c) {
                    return Err(Errno::ENOTTY);
                }
                self.check(SIGTTOU, true)?;
                let pgid = self.get_int(arg)?;
                if pgid < 0 {
                    return Err(Errno::EINVAL);
                }
                // tcsetpgrp(3): EPERM unless a group of the caller's session.
                if host.session_of_group(pgid) != Some(c.sid) {
                    return Err(Errno::EPERM);
                }
                self.state.lock().pgrp = Some(pgid);
                Ok(0)
            }
            TIOCGSID => {
                let c = host.caller().ok_or(Errno::ENOTTY)?;
                if !self.is_ctty_of(&c) {
                    return Err(Errno::ENOTTY);
                }
                self.put_int(arg, c.sid)
            }
            TIOCSTI => {
                let c = host.caller();
                if !c.is_some_and(|c| c.privileged || self.is_ctty_of(&c)) {
                    return Err(Errno::EPERM);
                }
                let [byte] = self.get::<1>(arg)?;
                self.receive(&[byte]);
                Ok(0)
            }
            _ => match self.common_ioctl(request, arg, true) {
                Some(result) => result,
                None => match self.driver.device_ioctl(request) {
                    Some(bytes) => self.put(arg, &bytes).map(|_| 0),
                    None => Err(Errno::ENOTTY),
                },
            },
        }
    }

    /// `TIOCSCTTY` (ioctl_tty(2)): make this the caller's controlling
    /// terminal. The caller must be a session leader without one (`EPERM`;
    /// 0 if this one is already it). A terminal another session has is
    /// taken from it only when `arg` is 1 and the caller is privileged.
    fn set_controlling(&self, arg: u64) -> Result<u64> {
        let c = self.host().caller().ok_or(Errno::EPERM)?;
        let mut sessions = self.ttys.sessions.lock();
        let mut st = self.state.lock();
        if st.session == Some(c.sid) {
            return Ok(0);
        }
        if !c.session_leader() || sessions.of(c.sid).is_some() || st.hung_up {
            return Err(Errno::EPERM);
        }
        if let Some(other) = st.session {
            if !(arg == 1 && c.privileged) {
                return Err(Errno::EPERM);
            }
            // Every process of the other session loses it.
            sessions.ctty.remove(&other);
        }
        sessions.ctty.insert(c.sid, self.me.clone());
        st.session = Some(c.sid);
        st.pgrp = Some(c.pgid);
        Ok(0)
    }

    /// `TIOCNOTTY` (ioctl_tty(2)): the caller gives up its controlling
    /// terminal. "If the process was session leader, then send SIGHUP and
    /// SIGCONT to the foreground process group and all processes in the
    /// current session lose their controlling terminal."
    fn give_up_controlling(&self) -> Result<u64> {
        let c = self.host().caller().ok_or(Errno::ENOTTY)?;
        if !self.is_ctty_of(&c) {
            return Err(Errno::ENOTTY);
        }
        if !c.session_leader() {
            self.ttys.sessions.lock().detached.insert(c.pid);
            return Ok(0);
        }
        let pgrp = {
            let mut sessions = self.ttys.sessions.lock();
            sessions.ctty.remove(&c.sid);
            let mut st = self.state.lock();
            st.session = None;
            st.pgrp.take()
        };
        if let Some(pgrp) = pgrp {
            self.host().signal_group(pgrp, SIGHUP);
            self.host().signal_group(pgrp, SIGCONT);
        }
        Ok(0)
    }
}

/// One open file's view of a terminal: the device the VFS reads, writes
/// and polls. Dropped with the open file, which closes it.
pub struct TtyFile {
    tty: Arc<Tty>,
}

impl TtyFile {
    pub fn tty(&self) -> &Arc<Tty> {
        &self.tty
    }
}

impl core::fmt::Debug for TtyFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TtyFile").field("tty", &self.tty.name).finish()
    }
}

impl Drop for TtyFile {
    fn drop(&mut self) {
        self.tty.closed();
    }
}

impl CharDevice for TtyFile {
    fn read(&self, _: u64, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        self.tty.read(buf, nonblock)
    }

    fn write(&self, _: u64, buf: &[u8], nonblock: bool) -> Result<usize> {
        self.tty.write(buf, nonblock)
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        self.tty.ioctl(request, arg)
    }

    fn poll(&self) -> PollEvents {
        self.tty.poll()
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.tty.queue)
    }
}
