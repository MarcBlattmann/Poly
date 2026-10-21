//! The kernel's side of NT file I/O ([ADR 0067]): putting the VFS into the
//! object namespace, one worker thread per volume device that carries out
//! queued I/O request packets, and the delivery of completions.
//!
//! The decisions — what a request does, whether a lock or share mode
//! allows it, when it is cancelled — are `poly_personality_nt::io`'s,
//! host-tested. This file only acts:
//!
//! - [`attach`] gives the personality the kernel's VFS and its volume list
//!   when the NT state is first made.
//! - [`after_call`] runs after every NT system call: it wakes the workers of
//!   devices that got requests ([`IoState::take_kicks`]) and [`drain`]s
//!   completions.
//! - A worker ([`worker_main`]) takes the next request of its device
//!   (`next_work`, under the NT lock), runs the transfer on the VFS with no
//!   NT lock held, records the result (`work_done`) and drains.
//! - [`drain`] delivers each completion in the order a waiter needs: the
//!   data and the `IO_STATUS_BLOCK` are written into the requesting
//!   process's memory first (outside the NT lock: a user page fault takes
//!   the NT lock, so the NT lock is never held across user-memory access),
//!   then the event or file object is signalled and the dispatcher's wakes
//!   drained, then the APC queued on the requesting thread.
//!
//! Each NT process's memory is registered here as a `Weak` reference, so a
//! completion for a process that is gone writes nothing and nothing here
//! keeps an address space alive.
//!
//! [ADR 0067]: ../../../docs/adr/0067-nt-file-io.md
//! [`IoState::take_kicks`]: poly_personality_nt::io::IoState::take_kicks

use super::{sched, with};
use crate::sync::Mutex;
use crate::thread::{self, WaitQueue};
use crate::user::Memory;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicBool, Ordering};
use poly_ntobj::ProcessId;
use poly_personality_nt::io::{IoState, default_volumes};
use poly_personality_nt::system::NtSystem;

/// Every NT process's memory, for writing completions into it.
static MEMORY: Mutex<BTreeMap<u32, Weak<Memory>>> = Mutex::new(BTreeMap::new());

/// One device's worker.
struct Worker {
    queue: WaitQueue,
    pending: AtomicBool,
}

static WORKERS: Mutex<BTreeMap<u32, Arc<Worker>>> = Mutex::new(BTreeMap::new());

/// Put the VFS into the NT namespace (`\Device\PolyFs<N>`, `\??\C:`,
/// `\??\Z:`, `\Device\PolyPipe`). Called once, when the NT state is made.
pub(super) fn attach(sys: &mut NtSystem) {
    let vfs = crate::fs::vfs();
    let Ok(ctx) = vfs.root_context() else { return };
    let volumes = default_volumes(vfs, &ctx);
    if let Err(e) = sys.io.attach(&mut sys.objects, vfs, &volumes) {
        crate::kprintln!("nt: file I/O not attached: {:#x}", e.0);
    }
}

/// NT process `pid` runs in `memory`.
pub(super) fn register(pid: u32, memory: &Arc<Memory>) {
    MEMORY.lock().insert(pid, Arc::downgrade(memory));
}

/// NT process `pid` has ended.
pub(super) fn unregister(pid: u32) {
    MEMORY.lock().remove(&pid);
}

/// After an NT system call (or anything else that may have queued or
/// completed requests): wake the workers with work, deliver completions.
pub(super) fn after_call() {
    let kicks = with(|s| s.sys.io.take_kicks());
    for dev in kicks {
        kick(dev);
    }
    drain();
}

/// Wake device `dev`'s worker, starting it on first use.
fn kick(dev: u32) {
    let (worker, new) = {
        let mut workers = WORKERS.lock();
        match workers.get(&dev) {
            Some(w) => (Arc::clone(w), false),
            None => {
                let w = Arc::new(Worker { queue: WaitQueue::new(), pending: AtomicBool::new(true) });
                workers.insert(dev, Arc::clone(&w));
                (w, true)
            }
        }
    };
    if new {
        match thread::spawn_named("nt-io", worker_main, dev as usize) {
            Ok(tid) => {
                let _ = thread::detach(tid);
            }
            Err(_) => {
                crate::kprintln!("nt: no worker thread for device {dev}");
                WORKERS.lock().remove(&dev);
            }
        }
        return;
    }
    worker.pending.store(true, Ordering::SeqCst);
    worker.queue.wake_all();
}

/// A device's worker: run its queue, sleep until kicked, forever.
fn worker_main(arg: usize) -> usize {
    let dev = arg as u32;
    let Some(worker) = WORKERS.lock().get(&dev).cloned() else { return 0 };
    loop {
        worker.pending.store(false, Ordering::SeqCst);
        while let Some(work) = with(|s| s.sys.io.next_work(dev)) {
            let result = work.run();
            with(|s| s.sys.io.work_done(work.irp, result));
            drain();
        }
        // `next_work` may have completed requests itself (lock conflicts).
        drain();
        worker.queue.wait_until(|| worker.pending.load(Ordering::SeqCst));
    }
}

/// Deliver every queued completion.
pub(super) fn drain() {
    loop {
        let Some(c) = with(|s| s.sys.io.pop_completion()) else { break };
        let memory = MEMORY.lock().get(&c.process.0).and_then(Weak::upgrade);
        if let Some(m) = &memory {
            if let Some((addr, data)) = &c.data {
                let _ = m.write(*addr, data);
            }
            if c.iosb != 0 {
                let _ = m.write(c.iosb, &c.iosb_bytes());
            }
        }
        drop(memory);
        let apc = with(|s| {
            let apc = s.sys.io.finish(&mut s.sys.objects, &c);
            apc.filter(|(t, _)| s.processes.get(&c.process).is_some_and(|p| p.has_live_thread(*t)))
        });
        sched::drain_wakes();
        if let Some((thread, apc)) = apc {
            let _ = sched::queue_apc(thread, apc);
        }
    }
}

/// Requests in flight and file objects open (debugging, tests).
#[allow(dead_code, reason = "read by the kernel tests and the shell")]
pub fn stats() -> (usize, usize) {
    with(|s| (s.sys.io.pending_irps(), s.sys.io.file_count()))
}

/// For the kernel tests: whether the personality has the VFS.
#[allow(dead_code, reason = "read by the kernel tests")]
pub fn attached() -> bool {
    with(|s| IoState::is_attached(&s.sys.io))
}

/// For the kernel tests: whether `pid`'s memory is registered.
#[allow(dead_code, reason = "read by the kernel tests")]
pub fn registered(pid: u32) -> bool {
    MEMORY.lock().contains_key(&ProcessId(pid).0)
}
