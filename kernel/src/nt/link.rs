//! `/dev/win32k`: the desktop link's device (ADR 0128, Decision 3; ADR
//! 0131). The native compositor opens it (one opener at a time), writes
//! `FromCompositor` records and reads `ToCompositor` records
//! (`poly_win32k::link`); win32k's end is the NT personality's
//! [`Win32k`](poly_personality_nt::win32k::Win32k) in the NT state. The
//! device request `READ_BUFFER` copies rows of a surface buffer into the
//! compositor's memory.
//!
//! Reads never block (the compositor polls): with nothing to hand out a
//! read fails with `EAGAIN`. What win32k produces while an app makes a
//! call is collected after every NT system call ([`after_call`]), which
//! wakes the compositor's poll.
//!
//! The compositor usually starts before any Windows program, when the NT
//! state does not exist yet; what it writes then is kept and handed to
//! win32k after the first NT system call.
//!
//! Locks: the device's own (sleeping) locks may be held while the NT
//! state's is taken, never the other way round.

use super::with_existing;
use crate::sync::{Mutex, SpinLock};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_vfs::device::CharDevice;
use poly_vfs::{At, Cred, DeviceId, Errno, FileType, FsContext, OpenFlags, PollEvents, Vfs, WaitQueue};
use poly_win32k::link::{DEVICE_MAJOR, DEVICE_MINOR, READ_BUFFER, ReadBuffer};

/// `/dev/win32k`'s number.
pub const WIN32K: DeviceId = DeviceId::new(DEVICE_MAJOR, DEVICE_MINOR);

/// The most bytes one `READ_BUFFER` copies (a 2048-pixel-wide buffer's
/// 64 rows).
const MAX_COPY: usize = 512 * 1024;

struct Link {
    /// Whether the compositor has the device open.
    open: AtomicBool,
    /// Records for the compositor not read yet.
    out: Mutex<Vec<u8>>,
    /// What the compositor wrote before the NT state existed.
    early: Mutex<Vec<u8>>,
    queue: Arc<WaitQueue>,
}

/// The device, once installed (only cloned under this lock).
static LINK: SpinLock<Option<Arc<Link>>> = SpinLock::new(None);

fn link() -> Option<Arc<Link>> {
    LINK.lock().clone()
}

impl Link {
    /// Bring in what win32k produced (and hand it what waited for the NT
    /// state); true when there is something to read.
    fn pump(&self) -> bool {
        let mut out = self.out.lock();
        let early = core::mem::take(&mut *self.early.lock());
        let more = with_existing(|s| {
            if !early.is_empty() {
                let sys = &mut s.sys;
                sys.win32k.link_receive(&mut sys.objects, &early);
            }
            s.sys.win32k.link_output()
        });
        match more {
            Some(bytes) => out.extend_from_slice(&bytes),
            // Still no NT state: keep the greeting for later.
            None => self.early.lock().extend_from_slice(&early),
        }
        !out.is_empty()
    }
}

/// The device node: each open gets a [`LinkFile`].
struct Node;

impl CharDevice for Node {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::ENODEV)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> poly_vfs::Result<usize> {
        Err(Errno::ENODEV)
    }

    fn open(&self, _: OpenFlags) -> poly_vfs::Result<Option<Arc<dyn CharDevice>>> {
        let l = link().ok_or(Errno::ENODEV)?;
        if l.open.swap(true, Ordering::SeqCst) {
            return Err(Errno::EBUSY);
        }
        l.out.lock().clear();
        l.early.lock().clear();
        Ok(Some(Arc::new(LinkFile(l))))
    }
}

/// The compositor's open of the device; closing it disconnects.
struct LinkFile(Arc<Link>);

impl Drop for LinkFile {
    fn drop(&mut self) {
        with_existing(|s| s.sys.win32k.link_closed());
        self.0.out.lock().clear();
        self.0.early.lock().clear();
        self.0.open.store(false, Ordering::SeqCst);
    }
}

impl CharDevice for LinkFile {
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> poly_vfs::Result<usize> {
        self.0.pump();
        let mut out = self.0.out.lock();
        if out.is_empty() {
            return Err(Errno::EAGAIN);
        }
        let n = buf.len().min(out.len());
        buf[..n].copy_from_slice(&out[..n]);
        out.drain(..n);
        Ok(n)
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> poly_vfs::Result<usize> {
        let mut early = self.0.early.lock();
        let handled = early.is_empty()
            && with_existing(|s| {
                let sys = &mut s.sys;
                sys.win32k.link_receive(&mut sys.objects, buf);
            })
            .is_some();
        if !handled {
            early.extend_from_slice(buf);
        }
        drop(early);
        let readable = self.0.pump();
        // The records may have signalled a GUI thread's event (a close
        // request): its wait is satisfied, but only a system call's end
        // wakes the thread, and this is not one.
        super::sched::drain_wakes();
        if readable {
            self.0.queue.notify(PollEvents::READABLE);
        }
        Ok(buf.len())
    }

    fn ioctl(&self, request: u32, arg: u64) -> poly_vfs::Result<u64> {
        if request != READ_BUFFER {
            return Err(Errno::ENOTTY);
        }
        let mut raw = [0u8; ReadBuffer::SIZE];
        crate::user::copy_from_user(&mut raw, arg).map_err(|_| Errno::EFAULT)?;
        let r = ReadBuffer::from_bytes(&raw);
        // Copied out of the NT state first: no user memory is touched
        // while its lock is held.
        let rows = with_existing(|s| s.sys.win32k.link_rows(r.buffer, r.row, r.rows)).flatten().ok_or(Errno::EINVAL)?;
        if rows.len() > MAX_COPY {
            return Err(Errno::EINVAL);
        }
        crate::user::copy_to_user(r.dst, &rows).map_err(|_| Errno::EFAULT)?;
        Ok(rows.len() as u64)
    }

    fn poll(&self) -> PollEvents {
        if self.0.pump() { PollEvents::READABLE | PollEvents::WRITABLE } else { PollEvents::WRITABLE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.0.queue)
    }
}

/// After an NT system call: what win32k produced for the compositor wakes
/// its poll.
pub fn after_call() {
    let Some(l) = link() else { return };
    if l.open.load(Ordering::Relaxed) && l.pump() {
        l.queue.notify(PollEvents::READABLE);
    }
}

/// Register the device and make `/dev/win32k`.
pub fn install(vfs: &Vfs, ctx: &FsContext) {
    let l = Arc::new(Link {
        open: AtomicBool::new(false),
        out: Mutex::new(Vec::new()),
        early: Mutex::new(Vec::new()),
        queue: Arc::new(WaitQueue::new()),
    });
    if vfs.register_char_device(WIN32K, Arc::new(Node)).is_err() {
        return;
    }
    *LINK.lock() = Some(l);
    match vfs.mknod(ctx, &Cred::root(), At::Cwd, b"/dev/win32k", FileType::CharDevice, 0o600, WIN32K) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(e) => crate::kprintln!("win32k: /dev/win32k: {}", e.name()),
    }
}
