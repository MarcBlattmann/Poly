//! `/dev/snd/*`: the card's control device and its PCM playback devices
//! as character devices (major 116, `poly_personality_linux::sound::abi`
//! has the minors).
//!
//! These only move structures between user memory and the host-tested
//! ALSA layer (`poly_personality_linux::sound`) and block where ALSA
//! blocks: `SNDRV_PCM_IOCTL_WRITEI_FRAMES` and `write(2)` until every
//! frame is in the ring buffer, `SNDRV_PCM_IOCTL_DRAIN` until the device
//! has played everything (both not with `O_NONBLOCK`, given at open).
//! They wait on the card's wait queue, which the audio thread wakes
//! whenever a period was played. A PCM device opens once at a time
//! (`EBUSY`); closing it stops the stream and gives the device back.

use super::{CARD, Card, Glue};
use crate::fs::poll::ThreadBlocker;
use crate::{fs, kprintln, time, user};
use alloc::sync::Arc;
use alloc::vec;
use poly_personality_linux::errno::Errno as LinuxErrno;
use poly_personality_linux::sound::abi::{
    self, CardInfo, PcmInfo, SwParams, SyncPtr, Timespec, Xferi, ioctl, stream, tstamp_type,
};
use poly_personality_linux::sound::hw::{self, HwParams};
use poly_personality_linux::sound::next_device;
use poly_personality_linux::sound::pcm::{PcmRuntime, State};
use poly_vfs::{At, CharDevice, Cred, DeviceId, Errno, FileType, OpenFlags, PollEvents, Result, Timeout, WaitQueue};

/// The most frames one attempt of a write copies in from user memory.
const CHUNK_BYTES: usize = 64 * 1024;

fn errno(e: LinuxErrno) -> Errno {
    Errno::from_code(i32::from(e.get())).unwrap_or(Errno::EIO)
}

fn get<const N: usize>(arg: u64) -> Result<[u8; N]> {
    let mut b = [0u8; N];
    if user::current_process().is_none() {
        return Err(Errno::EFAULT);
    }
    user::copy_from_user(&mut b, arg).map_err(|_| Errno::EFAULT)?;
    Ok(b)
}

fn put(arg: u64, bytes: &[u8]) -> Result<()> {
    if user::current_process().is_none() {
        return Err(Errno::EFAULT);
    }
    user::copy_to_user(arg, bytes).map_err(|_| Errno::EFAULT)
}

fn get_int(arg: u64) -> Result<i32> {
    get::<4>(arg).map(i32::from_le_bytes)
}

fn put_int(arg: u64, v: i32) -> Result<u64> {
    put(arg, &v.to_le_bytes()).map(|()| 0)
}

/// The timestamp a stream reports: the clock its software parameters ask
/// for (`tstamp_type`).
fn stamp(rt: &PcmRuntime) -> Timespec {
    if rt.sw_params().tstamp_type == tstamp_type::GETTIMEOFDAY {
        Timespec::from_ns(time::realtime_ns())
    } else {
        super::monotonic()
    }
}

/// Register the card's devices and make their nodes.
pub fn register(card: &Arc<Card>) {
    let vfs = fs::vfs();
    let mut devices: vec::Vec<(&[u8], DeviceId, Arc<dyn CharDevice>)> = vec::Vec::new();
    let control_path = alloc::format!("/dev/snd/controlC{CARD}");
    devices.push((
        control_path.as_bytes(),
        DeviceId::new(abi::MAJOR, abi::control_minor(CARD)),
        Arc::new(Control { card: card.clone() }),
    ));
    let pcm_paths: vec::Vec<_> =
        card.pcms.iter().map(|p| alloc::format!("/dev/snd/pcmC{CARD}D{}p", p.device)).collect();
    for (i, p) in card.pcms.iter().enumerate() {
        devices.push((
            pcm_paths[i].as_bytes(),
            DeviceId::new(abi::MAJOR, abi::playback_minor(CARD, p.device)),
            Arc::new(PcmNode { card: card.clone(), index: i }),
        ));
    }
    let Ok(mut ctx) = vfs.root_context() else { return };
    ctx.set_umask(0);
    let root = Cred::root();
    match vfs.mkdir(&ctx, &root, At::Cwd, b"/dev/snd", 0o755) {
        Ok(()) | Err(Errno::EEXIST) => {}
        Err(e) => kprintln!("sound: /dev/snd: {}", e.name()),
    }
    for (path, id, dev) in devices {
        if let Err(e) = vfs.register_char_device(id, dev) {
            kprintln!("sound: device {}:{} not registered: {}", id.major, id.minor, e.name());
            continue;
        }
        // Readable and writable by everyone: there is one user of the
        // machine's sound (Linux systems give 0660 and the audio group).
        match vfs.mknod(&ctx, &root, At::Cwd, path, FileType::CharDevice, 0o666, id) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => kprintln!("sound: {}: {}", alloc::string::String::from_utf8_lossy(path), e.name()),
        }
    }
}

/// `/dev/snd/controlC0`: what the card has.
struct Control {
    card: Arc<Card>,
}

impl Control {
    fn pcm_info(&self, device: u32, subdevice: u32, dir: i32) -> Option<[u8; abi::PCM_INFO_LEN]> {
        let p = self.card.pcms.iter().find(|p| p.device == device)?;
        if subdevice != 0 || dir != stream::PLAYBACK {
            return None;
        }
        let open = self.card.inner.lock().pcms.iter().zip(&self.card.pcms).any(|(s, d)| d.device == device && s.open);
        Some(
            PcmInfo {
                device,
                subdevice: 0,
                stream: stream::PLAYBACK,
                card: CARD as i32,
                id: "virtio-snd",
                name: &p.name,
                subname: "subdevice #0",
                subdevices_count: 1,
                subdevices_avail: if open { 0 } else { 1 },
            }
            .encode(),
        )
    }
}

impl core::fmt::Debug for Control {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

impl CharDevice for Control {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        // No control events: nothing ever subscribes to any.
        Err(Errno::EAGAIN)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        PollEvents::NONE
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        match request {
            ioctl::CTL_PVERSION => put_int(arg, abi::CTL_VERSION as i32),
            ioctl::CTL_CARD_INFO => {
                let info = CardInfo {
                    card: CARD as i32,
                    id: "Virtio",
                    driver: "poly-virtio-snd",
                    name: "Poly virtio sound",
                    longname: &self.card.longname,
                    mixername: "",
                    components: "",
                };
                put(arg, &info.encode()).map(|()| 0)
            }
            ioctl::CTL_PCM_NEXT_DEVICE => {
                let cur = get_int(arg)?;
                let devices: vec::Vec<u32> = self.card.pcms.iter().map(|p| p.device).collect();
                put_int(arg, next_device(&devices, cur))
            }
            ioctl::CTL_PCM_INFO => {
                let b = get::<{ abi::PCM_INFO_LEN }>(arg)?;
                let (device, subdevice, dir) = PcmInfo::request(&b);
                let info = self.pcm_info(device, subdevice, dir).ok_or(Errno::ENXIO)?;
                put(arg, &info).map(|()| 0)
            }
            ioctl::CTL_PCM_PREFER_SUBDEVICE => get_int(arg).map(|_| 0),
            ioctl::CTL_ELEM_LIST => {
                let mut b = get::<{ abi::ELEM_LIST_LEN }>(arg)?;
                abi::empty_elem_list(&mut b);
                put(arg, &b).map(|()| 0)
            }
            ioctl::CTL_SUBSCRIBE_EVENTS => {
                get_int(arg)?;
                put_int(arg, 0)
            }
            _ => Err(Errno::ENOTTY),
        }
    }
}

/// `/dev/snd/pcmC0D<n>p` as registered: opening it makes a [`PcmFile`].
struct PcmNode {
    card: Arc<Card>,
    index: usize,
}

impl core::fmt::Debug for PcmNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PcmNode").field("index", &self.index).finish_non_exhaustive()
    }
}

impl CharDevice for PcmNode {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EBADF)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EBADF)
    }

    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        {
            let mut g = self.card.inner.lock();
            let slot = &mut g.pcms[self.index];
            if slot.open {
                return Err(Errno::EBUSY);
            }
            slot.open = true;
        }
        Ok(Some(Arc::new(PcmFile {
            card: self.card.clone(),
            index: self.index,
            nonblock: flags.contains(OpenFlags::NONBLOCK),
        })))
    }
}

/// An open PCM playback device.
struct PcmFile {
    card: Arc<Card>,
    index: usize,
    nonblock: bool,
}

impl core::fmt::Debug for PcmFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PcmFile").field("index", &self.index).finish_non_exhaustive()
    }
}

impl Drop for PcmFile {
    fn drop(&mut self) {
        let (xruns, errors) = self.card.with_pcm(self.index, |rt, g| {
            rt.close(g);
            (rt.xruns, g.dev.errors)
        });
        let stats = {
            let mut g = self.card.inner.lock();
            g.pcms[self.index].open = false;
            g.driver.stats()
        };
        self.card.notify();
        let (interrupts, driven) = super::interrupts();
        kprintln!(
            "sound: {}: pcmC{CARD}D{}p closed: {} output messages ({} bytes, {} with errors), {} underruns, {} stream errors, {} interrupts ({})",
            self.card.bdf,
            self.card.pcms[self.index].device,
            stats.tx_messages,
            stats.tx_bytes,
            stats.tx_errors,
            xruns,
            errors,
            interrupts,
            if driven { "interrupt-driven" } else { "polled" }
        );
    }
}

impl PcmFile {
    fn with<R>(&self, f: impl FnOnce(&mut PcmRuntime, &mut Glue<'_>) -> R) -> R {
        self.card.with_pcm(self.index, f)
    }

    /// Wait (unless non-blocking) until `attempt` stops failing with
    /// `EAGAIN`, re-trying it whenever the card's queue is woken.
    fn wait<T>(&self, nonblock: bool, attempt: impl FnMut() -> Result<T>) -> Result<T> {
        let mut attempt = attempt;
        if nonblock {
            return attempt();
        }
        let (Some(process), Some(flags)) = (user::current_process(), user::current_flags()) else {
            return attempt();
        };
        let mut blocker = ThreadBlocker::new(&process, flags);
        let queue: &Arc<WaitQueue> = &self.card.queue;
        poly_vfs::poll::wait_until_done(queue, PollEvents::WRITABLE, &mut blocker, Timeout::Never, attempt)
    }

    /// One write attempt of up to `frames` frames from user memory at
    /// `buf`: `EAGAIN` with no room.
    fn write_user(&self, buf: u64, frames: u64, fb: u64) -> Result<u64> {
        let room = self.with(|rt, _| rt.writable()).map_err(errno)?;
        if room == 0 {
            return Err(Errno::EAGAIN);
        }
        let n = frames.min(room).min((CHUNK_BYTES as u64 / fb).max(1));
        let mut data = vec![0u8; (n * fb) as usize];
        if user::copy_from_user(&mut data, buf).is_err() {
            return Err(Errno::EFAULT);
        }
        let w = self.with(|rt, g| {
            let now = stamp(rt);
            rt.write(&data, g, now)
        });
        match w.map_err(errno)? {
            0 => Err(Errno::EAGAIN),
            w => Ok(w),
        }
    }

    /// `SNDRV_PCM_IOCTL_WRITEI_FRAMES`: write every frame (blocking), or
    /// as many as fit now (non-blocking); `result` gets the count.
    fn writei(&self, arg: u64) -> Result<u64> {
        let x = Xferi::decode(&get::<{ abi::XFERI_LEN }>(arg)?);
        let fb = self.with(|rt, _| rt.config().map(|c| c.frame_bytes as u64)).ok_or(Errno::EBADFD)?;
        self.with(|rt, _| rt.writable()).map_err(errno)?;
        let mut done = 0u64;
        while done < x.frames {
            let at = x.buf.wrapping_add(done * fb);
            match self.wait(self.nonblock, || self.write_user(at, x.frames - done, fb)) {
                Ok(w) => done += w,
                Err(_) if done > 0 => break,
                Err(e) => return Err(e),
            }
        }
        put(arg, &Xferi::result_bytes(done as i64)).map(|()| 0)
    }

    /// `SNDRV_PCM_IOCTL_DRAIN`.
    fn drain(&self) -> Result<u64> {
        let over = self.with(|rt, g| {
            let now = stamp(rt);
            rt.drain(g, now)
        });
        self.card.notify();
        if over.map_err(errno)? {
            return Ok(0);
        }
        if self.nonblock {
            return Err(Errno::EAGAIN);
        }
        self.wait(false, || if self.with(|rt, _| rt.state()) == State::Draining { Err(Errno::EAGAIN) } else { Ok(0) })
    }

    fn ioctl_inner(&self, request: u32, arg: u64) -> Result<u64> {
        let desc = &self.card.pcms[self.index];
        let ok = |r: core::result::Result<(), LinuxErrno>| r.map(|()| 0).map_err(errno);
        match request {
            ioctl::PCM_PVERSION => put_int(arg, abi::PCM_VERSION as i32),
            ioctl::PCM_INFO => {
                let info = PcmInfo {
                    device: desc.device,
                    subdevice: 0,
                    stream: stream::PLAYBACK,
                    card: CARD as i32,
                    id: "virtio-snd",
                    name: &desc.name,
                    subname: "subdevice #0",
                    subdevices_count: 1,
                    subdevices_avail: 0,
                };
                put(arg, &info.encode()).map(|()| 0)
            }
            ioctl::PCM_TSTAMP | ioctl::PCM_TTSTAMP => get_int(arg).map(|_| 0),
            ioctl::PCM_USER_PVERSION => {
                let v = get_int(arg)?;
                self.with(|rt, _| rt.user_version = v as u32);
                Ok(0)
            }
            ioctl::PCM_HW_REFINE => {
                let mut p = HwParams::decode(&get::<{ abi::HW_PARAMS_LEN }>(arg)?);
                hw::refine(&mut p, &desc.caps).map_err(errno)?;
                put(arg, &p.encode()).map(|()| 0)
            }
            ioctl::PCM_HW_PARAMS => {
                let mut p = HwParams::decode(&get::<{ abi::HW_PARAMS_LEN }>(arg)?);
                let cfg = hw::choose(&mut p, &desc.caps).map_err(errno)?;
                ok(self.with(|rt, g| rt.hw_params(cfg, g)))?;
                self.card.notify();
                put(arg, &p.encode()).map(|()| 0)
            }
            ioctl::PCM_HW_FREE => ok(self.with(|rt, g| rt.hw_free(g))),
            ioctl::PCM_SW_PARAMS => {
                let sw = SwParams::decode(&get::<{ abi::SW_PARAMS_LEN }>(arg)?);
                let out = self.with(|rt, _| rt.set_sw_params(&sw)).map_err(errno)?;
                self.card.notify();
                put(arg, &out.encode()).map(|()| 0)
            }
            ioctl::PCM_STATUS | ioctl::PCM_STATUS_EXT => {
                let s = self.with(|rt, _| {
                    let now = stamp(rt);
                    rt.status(now)
                });
                put(arg, &s.encode()).map(|()| 0)
            }
            ioctl::PCM_DELAY => {
                let d = self.with(|rt, _| rt.delay()).map_err(errno)?;
                put(arg, &d.to_le_bytes()).map(|()| 0)
            }
            ioctl::PCM_HWSYNC => ok(self.with(|rt, _| rt.hwsync())),
            ioctl::PCM_SYNC_PTR => {
                let req = SyncPtr::decode(&get::<{ abi::SYNC_PTR_LEN }>(arg)?);
                let r = self.with(|rt, _| {
                    let now = stamp(rt);
                    rt.sync_ptr(&req, now)
                });
                put(arg, &r.encode()).map(|()| 0)
            }
            ioctl::PCM_PREPARE => {
                let r = ok(self.with(|rt, g| rt.prepare(g)));
                self.card.notify();
                r
            }
            ioctl::PCM_RESET => ok(self.with(|rt, _| rt.reset())),
            ioctl::PCM_START => ok(self.with(|rt, g| {
                let now = stamp(rt);
                rt.start(g, now)
            })),
            ioctl::PCM_DROP => {
                let r = ok(self.with(|rt, g| {
                    let now = stamp(rt);
                    rt.drop_stream(g, now)
                }));
                self.card.notify();
                r
            }
            ioctl::PCM_DRAIN => self.drain(),
            ioctl::PCM_WRITEI_FRAMES => self.writei(arg),
            // Neither pausing nor moving the application pointer by hand:
            // the stream's info has no PAUSE, and REWIND/FORWARD move 0
            // frames.
            ioctl::PCM_PAUSE | ioctl::PCM_RESUME => Err(Errno::ENOSYS),
            ioctl::PCM_REWIND | ioctl::PCM_FORWARD => {
                get::<8>(arg)?;
                Ok(0)
            }
            ioctl::PCM_READI_FRAMES => Err(Errno::EBADFD),
            _ => Err(Errno::ENOTTY),
        }
    }
}

impl CharDevice for PcmFile {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EBADFD)
    }

    /// `write(2)` of interleaved frames (whole frames only); `EAGAIN`
    /// without room, which `write_wait` turns into blocking.
    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        let fb = self.with(|rt, _| rt.config().map(|c| c.frame_bytes as usize)).ok_or(Errno::EBADFD)?;
        if buf.len() < fb {
            return Err(Errno::EINVAL);
        }
        let w = self.with(|rt, g| {
            let now = stamp(rt);
            rt.write(buf, g, now)
        });
        match w.map_err(errno)? {
            0 => Err(Errno::EAGAIN),
            w => Ok(w as usize * fb),
        }
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        self.ioctl_inner(request, arg)
    }

    fn poll(&self) -> PollEvents {
        let r = self.with(|rt, _| rt.readiness());
        let mut e = PollEvents::NONE;
        if r.writable {
            e = e | PollEvents::WRITABLE;
        }
        if r.error {
            e = e | PollEvents::ERR;
        }
        e
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.card.queue)
    }
}
