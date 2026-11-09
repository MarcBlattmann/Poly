//! FUSE for the kernel (ADR 0168): `/dev/fuse` (10, 229) over `poly-fuse`,
//! and the mount of a FUSE file system from an open `/dev/fuse` descriptor.

use crate::fs::poll::ThreadBlocker;
use crate::time;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use core::any::Any;
use poly_fuse::{Caller, Conn, Env};
use poly_vfs::{Blocker, CharDevice, DeviceId, Errno, OpenFlags, PollEvents, Result, Vfs, WaitQueue, Woke};

/// `/dev/fuse` (`linux/miscdevice.h`: `FUSE_MINOR` 229).
pub const FUSE_DEV: DeviceId = DeviceId::new(10, 229);

/// The clock, sleeping and the caller for a connection.
pub struct KernelEnv;

impl Env for KernelEnv {
    fn now_ns(&self) -> u64 {
        time::now_ns()
    }

    fn wait(&self, queue: &Arc<WaitQueue>, ready: &mut dyn FnMut() -> bool) -> Result<()> {
        let process = crate::user::current_process().ok_or(Errno::EINTR)?;
        let flags = crate::user::current_flags().ok_or(Errno::EINTR)?;
        let mut blocker = ThreadBlocker::new(&process, flags);
        let _registration = queue.register(PollEvents::READABLE, blocker.waker());
        loop {
            if ready() {
                return Ok(());
            }
            if blocker.block(None) == Woke::Interrupted {
                return Err(Errno::EINTR);
            }
        }
    }

    fn caller(&self) -> Caller {
        match crate::user::current_process() {
            Some(p) => {
                let c = p.files.cred();
                Caller { uid: c.uid, gid: c.gid, pid: p.pid }
            }
            None => Caller::default(),
        }
    }
}

/// An open `/dev/fuse`: one connection, bound to a mount or not yet.
#[derive(Debug)]
pub struct FuseFile {
    conn: Arc<Conn>,
}

impl FuseFile {
    pub fn conn(&self) -> &Arc<Conn> {
        &self.conn
    }
}

impl Drop for FuseFile {
    fn drop(&mut self) {
        self.conn.server_gone();
    }
}

impl CharDevice for FuseFile {
    fn read(&self, _offset: u64, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        self.conn.read(&KernelEnv, buf, nonblock)
    }

    fn write(&self, _offset: u64, buf: &[u8], _nonblock: bool) -> Result<usize> {
        self.conn.write(buf)
    }

    fn poll(&self) -> PollEvents {
        let mut e = PollEvents::WRITABLE;
        if self.conn.readable() {
            e = e | PollEvents::READABLE;
        }
        e
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(self.conn.queue())
    }

    fn as_any(&self) -> Option<&(dyn Any + Send + Sync)> {
        Some(self)
    }
}

/// The registered device: every open is a connection of its own.
#[derive(Debug)]
struct FuseDevice;

impl CharDevice for FuseDevice {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn open(&self, _flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        Ok(Some(Arc::new(FuseFile { conn: Conn::new() })))
    }
}

/// Register `/dev/fuse`; called from `fs::devices::install`.
pub fn install(vfs: &Vfs) {
    if vfs.register_char_device(FUSE_DEV, Arc::new(FuseDevice)).is_err() {
        crate::kprintln!("fs: /dev/fuse registered twice");
    }
}

/// The type names of FUSE mounts seen so far (`fuse.NAME`), kept for
/// `FsInfo::name`.
static NAMES: crate::sync::SpinLock<BTreeMap<String, &'static str>> = crate::sync::SpinLock::new(BTreeMap::new());

/// `fstype` ("fuse" or "fuse.NAME") as a static string.
pub fn type_name(fstype: &[u8]) -> &'static str {
    if fstype == b"fuse" {
        return "fuse";
    }
    let s = String::from_utf8_lossy(fstype).into_owned();
    let mut names = NAMES.lock();
    if let Some(n) = names.get(&s) {
        return n;
    }
    let leaked: &'static str = alloc::boxed::Box::leak(s.clone().into_boxed_str());
    names.insert(s, leaked);
    leaked
}
