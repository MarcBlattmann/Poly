//! Pseudo-terminals (pty(7), pts(4)): opening `/dev/ptmx` makes a pair,
//! a master the opener keeps and a slave terminal (`/dev/pts/<n>`) that a
//! program opens as its terminal.
//!
//! ```text
//!   master write ──► slave's line discipline (input, echo) ──► slave readers
//!   slave write ──► output processing ──► master buffer ──► master read
//! ```
//!
//! - **The master** ([`PtyMaster`], one per open of `/dev/ptmx`): writes
//!   are typed into the slave (taking what its input buffer has room for,
//!   then waiting); reads take what the slave wrote (and echoed), from a
//!   buffer of [`MASTER_BUFFER`] bytes; once a slave was opened and every
//!   slave file is closed again, reads fail with `EIO` and poll reports
//!   `POLLHUP`. Its ioctls: `TIOCGPTN` (the pty number), `TIOCSPTLCK` /
//!   `TIOCGPTLCK` (the slave lock, which `unlockpt` clears), `TIOCSIG`,
//!   and the slave's attributes and window size (a change sends
//!   `SIGWINCH` to the slave's foreground group).
//! - **The slave** is a [`Tty`] whose driver is the master's buffer;
//!   opening `/dev/pts/<n>` fails with `EIO` while it is locked (a new
//!   pair is locked until `unlockpt`). It is owned by the opener of the
//!   master, mode `0620`, as grantpt(3) leaves it.
//! - **Closing the master** hangs the slave up ([`Tty::hangup`]) and
//!   frees the number: `/dev/pts/<n>` goes away.
//!
//! Slave device numbers are major [`PTY_SLAVE_MAJOR`], minor the pty
//! number (to be confirmed by the diff runner); each live slave is
//! registered with the VFS under its number while its master is open.

use crate::ioctl::*;
use crate::termios::{TCIFLUSH, TCIOFLUSH, TCOFLUSH, Termios, Winsize};
use crate::tty::{Driver, Tty, Ttys};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_vfs::lock::{Lock, RawSpinLock};
use poly_vfs::poll::Registration;
use poly_vfs::{Blocker, CharDevice, DeviceId, Errno, OpenFlags, PollEvents, Result, Vfs, WaitQueue, Woke};

/// `/dev/ptmx` (pts(4): "a character file with major number 5 and minor
/// number 2").
pub const PTMX: DeviceId = DeviceId::new(5, 2);
/// The major number of pty slaves.
pub const PTY_SLAVE_MAJOR: u32 = 136;
/// Pairs at most at a time; one more open of `/dev/ptmx` fails with
/// `ENOSPC` (to be confirmed by the diff runner).
pub const MAX_PTYS: u32 = 4096;
/// Bytes of slave output the master buffers before slave writers wait.
pub const MASTER_BUFFER: usize = 64 * 1024;
/// The speed a slave reports.
const SLAVE_SPEED: u32 = 38400;

/// The pseudo-terminals of one machine.
pub struct Ptys {
    ttys: Arc<Ttys>,
    vfs: &'static Vfs,
    slots: Lock<RawSpinLock, BTreeMap<u32, Weak<PtyPair>>>,
    me: Weak<Ptys>,
}

impl core::fmt::Debug for Ptys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ptys").field("live", &self.slots.lock().len()).finish_non_exhaustive()
    }
}

impl Ptys {
    /// Pseudo-terminals whose slaves are registered with `vfs`.
    pub fn new(ttys: &Arc<Ttys>, vfs: &'static Vfs) -> Arc<Ptys> {
        Arc::new_cyclic(|me| Ptys { ttys: Arc::clone(ttys), vfs, slots: Lock::new(BTreeMap::new()), me: me.clone() })
    }

    /// The device to register as `/dev/ptmx` ([`PTMX`]).
    pub fn ptmx(&self) -> Arc<Ptmx> {
        Arc::new(Ptmx { ptys: self.me.clone() })
    }

    /// Pair `index`, if its master is open.
    pub fn get(&self, index: u32) -> Option<Arc<PtyPair>> {
        self.slots.lock().get(&index).and_then(Weak::upgrade)
    }

    /// The numbers of the live pairs, in order.
    pub fn indices(&self) -> Vec<u32> {
        self.slots.lock().iter().filter(|(_, w)| w.strong_count() > 0).map(|(&i, _)| i).collect()
    }

    /// A new pair, owned by the caller: its master.
    fn create(&self) -> Result<Arc<PtyMaster>> {
        let (uid, gid) = self.ttys.host().caller().map_or((0, 0), |c| (c.uid, c.gid));
        let mut slots = self.slots.lock();
        let index = (0..MAX_PTYS).find(|i| slots.get(i).is_none_or(|w| w.strong_count() == 0)).ok_or(Errno::ENOSPC)?;
        let master = Arc::new(MasterSide {
            buf: Lock::new(VecDeque::new()),
            queue: Arc::new(WaitQueue::new()),
            slave_open: AtomicBool::new(false),
        });
        let slave = Tty::new(
            &self.ttys,
            &format!("pts/{index}"),
            master.clone(),
            Termios::sane(SLAVE_SPEED),
            Winsize::default(),
        );
        let pair = Arc::new(PtyPair {
            index,
            ptys: self.me.clone(),
            vfs: self.vfs,
            slave,
            master,
            locked: AtomicBool::new(true),
            slave_opened: AtomicBool::new(false),
            owner: Lock::new(Owner { uid, gid, perm: 0o620 }),
        });
        let device = Arc::new(SlaveDevice { pair: Arc::downgrade(&pair) });
        self.vfs.register_char_device(pair.device(), device).map_err(|_| Errno::ENOSPC)?;
        slots.insert(index, Arc::downgrade(&pair));
        Ok(Arc::new(PtyMaster { pair, ttys: Arc::clone(&self.ttys) }))
    }
}

/// Who owns a slave, and its permissions (devpts shows them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
    pub perm: u16,
}

/// A master and slave (see the module documentation). Lives as long as
/// the master is open.
pub struct PtyPair {
    index: u32,
    ptys: Weak<Ptys>,
    vfs: &'static Vfs,
    slave: Arc<Tty>,
    master: Arc<MasterSide>,
    locked: AtomicBool,
    /// A slave was opened at least once.
    slave_opened: AtomicBool,
    owner: Lock<RawSpinLock, Owner>,
}

impl core::fmt::Debug for PtyPair {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PtyPair").field("index", &self.index).finish_non_exhaustive()
    }
}

impl PtyPair {
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The slave's device number.
    pub fn device(&self) -> DeviceId {
        DeviceId::new(PTY_SLAVE_MAJOR, self.index)
    }

    pub fn slave(&self) -> &Arc<Tty> {
        &self.slave
    }

    pub fn owner(&self) -> Owner {
        *self.owner.lock()
    }

    pub fn set_owner(&self, owner: Owner) {
        *self.owner.lock() = owner;
    }

    pub fn locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    /// Whether every slave file opened so far is closed again.
    fn slave_gone(&self) -> bool {
        self.slave_opened.load(Ordering::SeqCst) && !self.master.slave_open.load(Ordering::SeqCst)
    }
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        let _ = self.vfs.unregister_char_device(self.device());
        if let Some(ptys) = self.ptys.upgrade() {
            let mut slots = ptys.slots.lock();
            if slots.get(&self.index).is_some_and(|w| w.strong_count() == 0) {
                slots.remove(&self.index);
            }
        }
    }
}

/// The master's side: what the slave wrote, waiting to be read.
struct MasterSide {
    buf: Lock<RawSpinLock, VecDeque<u8>>,
    /// Master readers and writers wait here.
    queue: Arc<WaitQueue>,
    /// The slave is open.
    slave_open: AtomicBool,
}

impl Driver for MasterSide {
    fn write(&self, bytes: &[u8]) {
        let n = {
            let mut b = self.buf.lock();
            let n = bytes.len().min(MASTER_BUFFER - b.len());
            b.extend(&bytes[..n]);
            n
        };
        if n > 0 {
            self.queue.notify(PollEvents::READABLE);
        }
    }

    fn room(&self) -> usize {
        MASTER_BUFFER.saturating_sub(self.buf.lock().len())
    }

    fn queued(&self) -> usize {
        self.buf.lock().len()
    }

    fn flush(&self) {
        self.buf.lock().clear();
    }

    fn opened(&self, open: bool) {
        self.slave_open.store(open, Ordering::SeqCst);
        if !open {
            self.queue.notify(PollEvents::HUP | PollEvents::READABLE);
        }
    }

    fn input_drained(&self) {
        self.queue.notify(PollEvents::WRITABLE);
    }
}

/// `/dev/ptmx`: every open makes a new pair and opens its master.
pub struct Ptmx {
    ptys: Weak<Ptys>,
}

impl core::fmt::Debug for Ptmx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ptmx").finish_non_exhaustive()
    }
}

impl CharDevice for Ptmx {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn open(&self, _flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        let ptys = self.ptys.upgrade().ok_or(Errno::ENXIO)?;
        Ok(Some(ptys.create()?))
    }
}

/// `/dev/pts/<n>` as registered with the VFS: opening it opens the slave.
struct SlaveDevice {
    pair: Weak<PtyPair>,
}

impl CharDevice for SlaveDevice {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    /// `EIO` while the slave is locked (pts(4): unlockpt(3) must be called
    /// first; the error is Poly's choice, to be confirmed by the diff
    /// runner) or once its master is gone.
    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        let pair = self.pair.upgrade().ok_or(Errno::EIO)?;
        if pair.locked() {
            return Err(Errno::EIO);
        }
        let file = pair.slave.open(flags, true)?;
        pair.slave_opened.store(true, Ordering::SeqCst);
        Ok(Some(file))
    }
}

/// An open master (see the module documentation). Closing it hangs the
/// slave up.
pub struct PtyMaster {
    pair: Arc<PtyPair>,
    ttys: Arc<Ttys>,
}

impl core::fmt::Debug for PtyMaster {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PtyMaster").field("index", &self.pair.index).finish()
    }
}

impl Drop for PtyMaster {
    fn drop(&mut self) {
        self.pair.slave.hangup();
    }
}

impl PtyMaster {
    pub fn pair(&self) -> &Arc<PtyPair> {
        &self.pair
    }

    fn host(&self) -> &dyn crate::tty::Host {
        self.ttys.host()
    }

    /// Wait on the master's queue for `interest`; `EINTR` if interrupted.
    fn wait(
        &self,
        interest: PollEvents,
        blocker: &mut Option<Box<dyn Blocker>>,
        registered: &mut Option<Registration>,
    ) -> Result<()> {
        let Some(b) = blocker.as_mut() else {
            let b = self.host().blocker().ok_or(Errno::EAGAIN)?;
            *registered = Some(self.pair.master.queue.register(interest, b.waker()));
            *blocker = Some(b);
            return Ok(());
        };
        if b.block(None) == Woke::Interrupted { Err(Errno::EINTR) } else { Ok(()) }
    }

    fn put_int(&self, arg: u64, v: i32) -> Result<u64> {
        self.host().copy_to_user(arg, &v.to_le_bytes()).map(|()| 0)
    }

    fn get_int(&self, arg: u64) -> Result<i32> {
        let mut b = [0u8; 4];
        self.host().copy_from_user(arg, &mut b)?;
        Ok(i32::from_le_bytes(b))
    }
}

impl CharDevice for PtyMaster {
    fn read(&self, _: u64, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut blocker = None;
        let mut registered = None;
        loop {
            let n = {
                let mut b = self.pair.master.buf.lock();
                let n = b.len().min(buf.len());
                for (dst, src) in buf.iter_mut().zip(b.drain(..n)) {
                    *dst = src;
                }
                n
            };
            if n > 0 {
                self.pair.slave.output_room();
                return Ok(n);
            }
            if self.pair.slave_gone() {
                return Err(Errno::EIO);
            }
            if nonblock {
                return Err(Errno::EAGAIN);
            }
            self.wait(PollEvents::READABLE, &mut blocker, &mut registered)?;
        }
    }

    fn write(&self, _: u64, buf: &[u8], nonblock: bool) -> Result<usize> {
        let mut done = 0;
        let mut blocker = None;
        let mut registered = None;
        while done < buf.len() {
            let n = self.pair.slave.receive_some(&buf[done..]);
            done += n;
            if n > 0 {
                continue;
            }
            if nonblock {
                return if done > 0 { Ok(done) } else { Err(Errno::EAGAIN) };
            }
            if let Err(e) = self.wait(PollEvents::WRITABLE, &mut blocker, &mut registered) {
                return if done > 0 { Ok(done) } else { Err(e) };
            }
        }
        Ok(done)
    }

    fn poll(&self) -> PollEvents {
        let mut ev = PollEvents::NONE;
        if !self.pair.master.buf.lock().is_empty() {
            ev = ev | PollEvents::READABLE;
        }
        if self.pair.slave_gone() {
            ev = ev | PollEvents::HUP | PollEvents::READABLE;
        }
        if self.pair.slave.input_room() > 0 {
            ev = ev | PollEvents::WRITABLE;
        }
        ev
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.pair.master.queue)
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        let slave = &self.pair.slave;
        match request {
            TIOCGPTN => self.host().copy_to_user(arg, &self.pair.index.to_le_bytes()).map(|()| 0),
            TIOCSPTLCK => {
                let lock = self.get_int(arg)?;
                self.pair.locked.store(lock != 0, Ordering::SeqCst);
                Ok(0)
            }
            TIOCGPTLCK => self.put_int(arg, i32::from(self.pair.locked())),
            TIOCGPGRP => match slave.session() {
                (Some(_), Some(pgrp)) => self.put_int(arg, pgrp),
                _ => Err(Errno::ENOTTY),
            },
            TIOCGSID => match slave.session() {
                (Some(sid), _) => self.put_int(arg, sid),
                _ => Err(Errno::ENOTTY),
            },
            TIOCSIG => {
                let signal = arg as u32;
                if !(1..=64).contains(&signal) {
                    return Err(Errno::EINVAL);
                }
                if let (_, Some(pgrp)) = slave.session() {
                    self.host().signal_group(pgrp, signal);
                }
                Ok(0)
            }
            FIONREAD => {
                let n = self.pair.master.buf.lock().len();
                self.put_int(arg, n.min(i32::MAX as usize) as i32)
            }
            TIOCOUTQ => self.put_int(arg, slave.readable_bytes().min(i32::MAX as usize) as i32),
            TCFLSH => {
                let (input, output) = match arg as u32 {
                    TCIFLUSH => (true, false),
                    TCOFLUSH => (false, true),
                    TCIOFLUSH => (true, true),
                    _ => return Err(Errno::EINVAL),
                };
                if input {
                    self.pair.master.buf.lock().clear();
                    slave.output_room();
                }
                if output {
                    slave.flush_input();
                }
                Ok(0)
            }
            _ => slave.common_ioctl(request, arg, false).unwrap_or(Err(Errno::ENOTTY)),
        }
    }
}
