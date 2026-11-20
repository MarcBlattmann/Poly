//! The kernel's side of `\Device\PolyAudio` (ADR 0244): its streams are
//! the audio server's (`crate::sound::ntstream`), and the `nt-audio`
//! worker finishes the waits the personality could not finish at once.
//!
//! The worker holds the pending waits. Whenever the mixer took frames
//! (`ntstream::PLAYED`), and every [`RECHECK_MS`] to let cancelled ones
//! go, it completes those whose stream played enough under the NT lock
//! and delivers the completions (`io::drain`).

use super::{io, with};
use crate::sound::ntstream::{self, PLAYED, PLAYED_GEN};
use crate::sync::Mutex;
use crate::thread::{self, WaitQueue};
use crate::time;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use poly_audio::Format;
use poly_audio::nt::DeviceInfo;
use poly_ntobj::NtStatus;
use poly_personality_nt::audio::{AudioWait, NtAudioStream, finish_wait};

/// How often the worker looks at its waits without the mixer moving.
const RECHECK_MS: u64 = 100;

static PENDING: Mutex<Vec<AudioWait>> = Mutex::new(Vec::new());
static KICKED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static WORKER: WaitQueue = WaitQueue::new();

pub(super) fn device() -> Option<DeviceInfo> {
    ntstream::device()
}

pub(super) fn open(format: Format, frames: u32) -> Result<Arc<dyn NtAudioStream>, NtStatus> {
    Ok(ntstream::open(format, frames)? as Arc<dyn NtAudioStream>)
}

/// Queue a wait for the worker (started on first use).
pub(super) fn pend(w: AudioWait) -> Result<(), AudioWait> {
    if !STARTED.swap(true, Ordering::SeqCst) {
        match thread::spawn_named("nt-audio", worker_main, 0) {
            Ok(tid) => {
                let _ = thread::detach(tid);
            }
            Err(_) => {
                STARTED.store(false, Ordering::SeqCst);
                return Err(w);
            }
        }
    }
    PENDING.lock().push(w);
    KICKED.store(true, Ordering::SeqCst);
    WORKER.wake_all();
    PLAYED.wake_all();
    Ok(())
}

/// Waits the worker holds (tests).
#[allow(dead_code, reason = "read by the kernel tests")]
pub fn pending() -> usize {
    PENDING.lock().len()
}

fn worker_main(_: usize) -> usize {
    loop {
        KICKED.store(false, Ordering::SeqCst);
        let seen = PLAYED_GEN.load(Ordering::SeqCst);
        let list = core::mem::take(&mut *PENDING.lock());
        let mut keep = Vec::new();
        let mut completed = false;
        for w in list {
            match w.reached() {
                Some(played) => {
                    with(|s| finish_wait(&mut s.sys, &w, played));
                    completed = true;
                }
                // Cancelled (reset, closed, CancelIo, the thread ended).
                None if !with(|s| s.sys.io.irp(w.irp).is_some()) => {}
                None => keep.push(w),
            }
        }
        if completed {
            io::drain();
        }
        let idle = {
            let mut p = PENDING.lock();
            let newer = core::mem::take(&mut *p);
            *p = keep;
            p.extend(newer);
            p.is_empty()
        };
        if idle {
            WORKER.wait_until(|| KICKED.load(Ordering::SeqCst));
        } else if !KICKED.load(Ordering::SeqCst) {
            let deadline = time::now_ns().saturating_add(RECHECK_MS * time::NANOS_PER_MILLI);
            PLAYED.wait_until_deadline(
                || PLAYED_GEN.load(Ordering::SeqCst) != seen || KICKED.load(Ordering::SeqCst),
                deadline,
            );
        }
    }
}
