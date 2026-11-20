//! `\Device\PolyAudio` (ADR 0244): the Windows side's way to the kernel's
//! audio server, for the sound DLLs (`winmm`, the audio session API) to
//! build their behaviour on. Poly's own protocol, [`poly_audio::nt`]:
//! a file object per stream, `NtDeviceIoControlFile` for the rest.
//!
//! The personality keeps a handle's [`AudioEnd`]; the stream itself is the
//! kernel's ([`NtAudioStream`], made by [`NtKernel::audio_open`]), mixed by
//! the audio server like an ALSA client and dropped with the file object.
//! A [`poly_audio::nt::WAIT`] that cannot finish at once is a pending
//! request the kernel's worker completes ([`NtKernel::audio_pend`],
//! [`finish_wait`]) when the mixer has taken enough frames.
//!
//! Nothing here is a Windows API's behaviour: what `waveOutWrite` or
//! `IAudioClient` answer is for the DLLs and their spec sheets.

use crate::abi;
use crate::io::Target;
use crate::io::consts::{STATUS_CANCELLED, STATUS_INVALID_DEVICE_REQUEST};
use crate::io::irp::IrpOp;
use crate::io::services::{Notify, wait_sync, write_iosb};
use crate::kernel::NtKernel;
use crate::system::NtSystem;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_audio::nt::{self as wire, DeviceInfo, OpenReq, StreamInfo};
use poly_audio::{Format, MAX_CHANNELS};
use poly_ntobj::status::*;
use poly_ntobj::{NtStatus, ObjectId};

#[cfg(test)]
mod tests;

type Result<T> = core::result::Result<T, NtStatus>;

/// `STATUS_NO_SUCH_DEVICE` (`ntstatus.h`): the machine has no sound card.
pub const STATUS_NO_SUCH_DEVICE: NtStatus = NtStatus(0xC000_000E);
/// `STATUS_INVALID_DEVICE_STATE` (`ntstatus.h`): a stream request before
/// [`wire::OPEN`], or a second `OPEN`.
pub const STATUS_INVALID_DEVICE_STATE: NtStatus = NtStatus(0xC000_0184);

/// A stream of the kernel's audio server.
pub trait NtAudioStream: Send + Sync + core::fmt::Debug {
    /// Append whole frames in the stream's format, as many as there is room
    /// for; returns how many it took.
    fn write(&self, data: &[u8]) -> u32;
    fn start(&self);
    fn stop(&self);
    fn reset(&self);
    fn set_gains(&self, gains: [u32; MAX_CHANNELS]);
    /// Its counters and the device's stream ([`wire::QUERY`]).
    fn info(&self) -> StreamInfo;
    fn format(&self) -> Format;
}

/// A handle on `\Device\PolyAudio`: its stream once opened.
#[derive(Debug, Default)]
pub struct AudioEnd {
    pub stream: Option<Arc<dyn NtAudioStream>>,
}

/// A [`wire::WAIT`] the kernel's worker finishes: request `irp` waits for
/// `stream` to have played `target` frames; the count goes to `out`.
#[derive(Debug)]
pub struct AudioWait {
    pub irp: u64,
    pub stream: Arc<dyn NtAudioStream>,
    pub target: u64,
    pub out: u64,
}

impl AudioWait {
    /// The played-frame count once it reached the target.
    pub fn reached(&self) -> Option<u64> {
        let played = self.stream.info().played;
        (played >= self.target).then_some(played)
    }
}

/// Complete `w` with `played` (under the NT lock; the caller delivers the
/// completion). A request already gone (cancelled, reset, closed) is left.
pub fn finish_wait(sys: &mut NtSystem, w: &AudioWait, played: u64) {
    if sys.io.irp(w.irp).is_some() {
        sys.io.complete(w.irp, STATUS_SUCCESS, 8, Some((w.out, played.to_le_bytes().to_vec())));
    }
}

/// Is file object `cookie` a `\Device\PolyAudio` handle?
pub(crate) fn is_audio(sys: &NtSystem, cookie: u64) -> bool {
    matches!(sys.io.file(cookie).map(|r| &r.target), Some(Target::Audio(_)))
}

fn end(sys: &mut NtSystem, cookie: u64) -> Result<&mut AudioEnd> {
    match sys.io.file_mut(cookie).map(|r| &mut r.target) {
        Some(Target::Audio(e)) => Ok(e),
        _ => Err(STATUS_INVALID_HANDLE),
    }
}

fn stream_of<K: NtKernel>(k: &mut K, cookie: u64) -> Result<Arc<dyn NtAudioStream>> {
    k.with_state(|sys, _, _| end(sys, cookie)?.stream.clone().ok_or(STATUS_INVALID_DEVICE_STATE))
}

/// The device's stream with this stream's counters.
fn query(k: &mut impl NtKernel, s: &Arc<dyn NtAudioStream>) -> StreamInfo {
    let mut i = s.info();
    if let Some(d) = k.audio_device() {
        i.device_rate = d.format.rate;
        i.device_period_frames = d.period_frames;
        i.device_periods = d.periods;
    }
    i
}

/// A request's result: status, `Information` and output.
struct Done {
    status: NtStatus,
    info: u64,
    output: Vec<u8>,
}

fn done(info: u64, output: Vec<u8>) -> Result<Option<Done>> {
    Ok(Some(Done { status: STATUS_SUCCESS, info, output }))
}

/// `NtDeviceIoControlFile` on a `\Device\PolyAudio` handle (`file`), the
/// arguments read: run the request and finish it.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn control<K: NtKernel>(
    k: &mut K,
    file: (ObjectId, u64),
    n: Notify,
    sync: bool,
    code: u32,
    input: &[u8],
    out: u64,
    outlen: usize,
) -> Result<NtStatus> {
    let r = match code {
        wire::DEVICE => match k.audio_device() {
            Some(d) => done(DeviceInfo::SIZE as u64, d.encode().to_vec()),
            None => Err(STATUS_NO_SUCH_DEVICE),
        },
        wire::OPEN => {
            let req = OpenReq::decode(input).ok_or(STATUS_INVALID_PARAMETER)?;
            if k.with_state(|sys, _, _| end(sys, file.1).map(|e| e.stream.is_some()))? {
                return Err(STATUS_INVALID_DEVICE_STATE);
            }
            let stream = k.audio_open(req.format, req.buffer_frames)?;
            k.with_state(|sys, _, _| end(sys, file.1).map(|e| e.stream = Some(stream)))?;
            done(0, Vec::new())
        }
        wire::WRITE => {
            let s = stream_of(k, file.1)?;
            if !input.len().is_multiple_of(s.format().frame_bytes()) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            done(u64::from(s.write(input)), Vec::new())
        }
        wire::START => {
            stream_of(k, file.1)?.start();
            done(0, Vec::new())
        }
        wire::STOP => {
            stream_of(k, file.1)?.stop();
            done(0, Vec::new())
        }
        wire::RESET => {
            let s = stream_of(k, file.1)?;
            s.reset();
            // Its waits end: the count they wait for restarted.
            k.with_state(|sys, _, _| {
                sys.io.cancel_where_status(|irp| irp.file == file.1, false, STATUS_CANCELLED);
            });
            done(0, Vec::new())
        }
        wire::SET_GAIN => {
            let g = wire::gains_of(input).ok_or(STATUS_INVALID_PARAMETER)?;
            stream_of(k, file.1)?.set_gains(g);
            done(0, Vec::new())
        }
        wire::QUERY => {
            let s = stream_of(k, file.1)?;
            done(StreamInfo::SIZE as u64, query(k, &s).encode().to_vec())
        }
        wire::WAIT => {
            let target = input.get(..8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes);
            let target = target.ok_or(STATUS_INVALID_PARAMETER)?;
            if outlen < 8 {
                return Err(STATUS_BUFFER_TOO_SMALL);
            }
            let s = stream_of(k, file.1)?;
            let played = s.info().played;
            if played >= target {
                done(8, played.to_le_bytes().to_vec())
            } else {
                return pend(k, file, n, sync, AudioWait { irp: 0, stream: s, target, out });
            }
        }
        _ => Err(STATUS_INVALID_DEVICE_REQUEST),
    };
    match r? {
        Some(d) => finish(k, file, &n, d, out, outlen),
        None => Ok(STATUS_PENDING),
    }
}

/// Write a finished request's output and status block, signal its event
/// and APC.
fn finish<K: NtKernel>(
    k: &mut K,
    file: (ObjectId, u64),
    n: &Notify,
    d: Done,
    out: u64,
    outlen: usize,
) -> Result<NtStatus> {
    if d.output.len() > outlen {
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    if !d.output.is_empty() {
        abi::write(k, out, &d.output)?;
    }
    if n.iosb != 0 {
        write_iosb(k, n.iosb, d.status, d.info)?;
    }
    let caller = k.caller();
    k.with_state(|sys, _, _| n.signal_done(sys, &caller, file, d.status, d.info));
    Ok(d.status)
}

/// A wait that cannot finish yet: a pending request for the kernel's
/// worker (a synchronous handle waits for it here). A kernel without a
/// worker leaves it pending until it is cancelled.
fn pend<K: NtKernel>(k: &mut K, file: (ObjectId, u64), n: Notify, sync: bool, mut w: AudioWait) -> Result<NtStatus> {
    let caller = k.caller();
    let irp = k.with_state(|sys, _, _| -> Result<u64> {
        n.check_event(sys, &caller)?;
        let irp = n.irp(sys, &caller, file, sync, IrpOp::Read { offset: 0, len: 8, buf: w.out, key: 0 })?;
        Ok(sys.io.park(irp))
    })?;
    w.irp = irp;
    // It may have played enough meanwhile: the worker finds it at once.
    let _ = k.audio_pend(w);
    if sync { Ok(wait_sync(k, file.0, irp).0) } else { Ok(STATUS_PENDING) }
}
