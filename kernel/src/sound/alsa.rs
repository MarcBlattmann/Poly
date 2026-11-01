//! `/dev/snd/*`: the card's control device and its PCM devices as
//! character devices (major 116, `poly_personality_linux::sound::abi`
//! has the minors).
//!
//! These only move structures between user memory and the host-tested
//! ALSA layer (`poly_personality_linux::sound`) and block where ALSA
//! blocks: `WRITEI_FRAMES` and `write(2)` until every frame is in the ring
//! buffer, `READI_FRAMES` and `read(2)` until every frame asked for was
//! captured, `DRAIN` until the server has taken everything (none of them
//! with `O_NONBLOCK`, given at open). They wait on the card's wait queue,
//! which the audio thread wakes. Each open of a PCM device is a substream
//! of its own, up to [`SUBDEVICES`] (`EBUSY` beyond); closing it stops the
//! stream.
//!
//! `mmap(2)` of an open PCM device ([`map`]) maps its ring buffer at
//! offset 0 and the status and control pages at ALSA's offsets (ADR
//! 0109). The control device keeps per-open event queues for mixer
//! changes.

use super::buffer::SharedBuf;
use super::{CARD, Card, CtlEvents, SUBDEVICES, SoftGlue};
use crate::fs::poll::ThreadBlocker;
use crate::user::memory::PageSource;
use crate::{fs, kprintln, time, user};
use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::Ordering::SeqCst;
use poly_personality_linux::errno::Errno as LinuxErrno;
use poly_personality_linux::sound::abi::{
    self, CardInfo, MmapControl, PcmInfo, SwParams, SyncPtr, Timespec, Xferi, ioctl, mmap_offset, stream, tstamp_type,
};
use poly_personality_linux::sound::ctl::{self, Controls};
use poly_personality_linux::sound::hw::{self, HwParams};
use poly_personality_linux::sound::next_device;
use poly_personality_linux::sound::pcm::{Dir, PcmRuntime, State};
use poly_vfs::{At, CharDevice, Cred, DeviceId, Errno, FileType, OpenFlags, PollEvents, Result, Timeout, WaitQueue};

/// What `mmap(2)` of a PCM device maps, and the offset into it.
type Mapping = core::result::Result<(Arc<dyn PageSource>, u64), LinuxErrno>;

/// The most frames one attempt of a write or read copies from or to user
/// memory.
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

fn node_name(p: &super::PcmDesc) -> alloc::string::String {
    alloc::format!("pcmC{CARD}D{}{}", p.device, if p.dir == Dir::Playback { 'p' } else { 'c' })
}

fn alsa_stream(d: Dir) -> i32 {
    if d == Dir::Playback { stream::PLAYBACK } else { stream::CAPTURE }
}

/// Register the card's devices and make their nodes.
pub fn register(card: &Arc<Card>) {
    let vfs = fs::vfs();
    let mut devices: vec::Vec<(alloc::string::String, DeviceId, Arc<dyn CharDevice>)> = vec::Vec::new();
    devices.push((
        alloc::format!("/dev/snd/controlC{CARD}"),
        DeviceId::new(abi::MAJOR, abi::control_minor(CARD)),
        Arc::new(ControlNode { card: card.clone() }),
    ));
    for (i, p) in card.pcms.iter().enumerate() {
        let minor = if p.dir == Dir::Playback {
            abi::playback_minor(CARD, p.device)
        } else {
            abi::capture_minor(CARD, p.device)
        };
        devices.push((
            alloc::format!("/dev/snd/{}", node_name(p)),
            DeviceId::new(abi::MAJOR, minor),
            Arc::new(PcmNode { card: card.clone(), desc: i }),
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
        match vfs.mknod(&ctx, &root, At::Cwd, path.as_bytes(), FileType::CharDevice, 0o666, id) {
            Ok(()) | Err(Errno::EEXIST) => {}
            Err(e) => kprintln!("sound: {path}: {}", e.name()),
        }
    }
}

// ---- the control device ----------------------------------------------------

/// `/dev/snd/controlC0` as registered: every open gets a [`ControlFile`].
struct ControlNode {
    card: Arc<Card>,
}

impl core::fmt::Debug for ControlNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControlNode").finish_non_exhaustive()
    }
}

impl CharDevice for ControlNode {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EBADF)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EBADF)
    }

    fn open(&self, _flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        let events = Arc::new(CtlEvents::default());
        {
            let mut g = self.card.inner.lock();
            g.listeners.retain(|w| w.strong_count() > 0);
            g.listeners.push(Arc::downgrade(&events));
        }
        Ok(Some(Arc::new(ControlFile { card: self.card.clone(), events })))
    }
}

/// An open control device: what the card has, its mixer controls, and the
/// events of their changes once subscribed.
struct ControlFile {
    card: Arc<Card>,
    events: Arc<CtlEvents>,
}

impl core::fmt::Debug for ControlFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControlFile").finish_non_exhaustive()
    }
}

impl ControlFile {
    fn pcm_info(&self, device: u32, subdevice: u32, dir: i32) -> Option<[u8; abi::PCM_INFO_LEN]> {
        let i = self.card.pcms.iter().position(|p| p.device == device && alsa_stream(p.dir) == dir)?;
        if subdevice as usize >= SUBDEVICES {
            return None;
        }
        let p = &self.card.pcms[i];
        let free = self.card.inner.lock().subs[i * SUBDEVICES..(i + 1) * SUBDEVICES].iter().filter(|s| !s.open).count();
        let subname = alloc::format!("subdevice #{subdevice}");
        Some(
            PcmInfo {
                device,
                subdevice,
                stream: dir,
                card: CARD as i32,
                id: self.card.driver,
                name: &p.name,
                subname: &subname,
                subdevices_count: SUBDEVICES as u32,
                subdevices_avail: free as u32,
            }
            .encode(),
        )
    }

    /// Queue an event for every subscribed control file and wake readers.
    fn announce(&self, id: &ctl::ElemId) {
        let e = ctl::event(id, ctl::EVENT_MASK_VALUE);
        let listeners: vec::Vec<_> = self.card.inner.lock().listeners.iter().filter_map(|w| w.upgrade()).collect();
        for l in listeners {
            if l.subscribed.load(SeqCst) {
                let mut q = l.queue.lock();
                if q.len() < 256 {
                    q.push_back(e);
                }
            }
        }
        self.card.ctl_queue.notify(PollEvents::READABLE);
    }

    fn controls<R>(&self, f: impl FnOnce(&mut Controls) -> R) -> R {
        f(&mut self.card.inner.lock().controls)
    }
}

impl CharDevice for ControlFile {
    /// Queued `snd_ctl_event`s, as many whole ones as fit; `EAGAIN` with
    /// none (the VFS blocks on the card's control queue).
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> Result<usize> {
        if buf.len() < abi::CTL_EVENT_LEN {
            return Err(Errno::EINVAL);
        }
        let mut q = self.events.queue.lock();
        let mut n = 0;
        while n + abi::CTL_EVENT_LEN <= buf.len() {
            let Some(e) = q.pop_front() else { break };
            buf[n..n + abi::CTL_EVENT_LEN].copy_from_slice(&e);
            n += abi::CTL_EVENT_LEN;
        }
        if n == 0 { Err(Errno::EAGAIN) } else { Ok(n) }
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if self.events.queue.lock().is_empty() { PollEvents::NONE } else { PollEvents::READABLE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.card.ctl_queue)
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        match request {
            ioctl::CTL_PVERSION => put_int(arg, abi::CTL_VERSION as i32),
            ioctl::CTL_CARD_INFO => {
                let info = CardInfo {
                    card: CARD as i32,
                    id: self.card.id,
                    driver: self.card.driver,
                    name: self.card.name,
                    longname: &self.card.longname,
                    mixername: "Poly software mixer",
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
            // Opens take the first free subdevice whatever is preferred.
            ioctl::CTL_PCM_PREFER_SUBDEVICE => get_int(arg).map(|_| 0),
            ioctl::CTL_ELEM_LIST => {
                let mut b = get::<{ abi::ELEM_LIST_LEN }>(arg)?;
                let (range, ids) = self.controls(|c| {
                    let r = c.list(&mut b);
                    let ids: vec::Vec<_> = r.clone().filter_map(|i| c.id(i)).collect();
                    (r, ids)
                });
                let pids = Controls::list_pointer(&b);
                for (k, id) in ids.iter().enumerate() {
                    put(pids + (k * abi::ELEM_ID_LEN) as u64, &id.encode())?;
                }
                let _ = range;
                put(arg, &b).map(|()| 0)
            }
            ioctl::CTL_ELEM_INFO => {
                let b = get::<{ abi::ELEM_INFO_LEN }>(arg)?;
                let out = self.controls(|c| c.info(&b)).map_err(errno)?;
                put(arg, &out).map(|()| 0)
            }
            ioctl::CTL_ELEM_READ => {
                let b = get::<{ abi::ELEM_VALUE_LEN }>(arg)?;
                let out = self.controls(|c| c.read(&b)).map_err(errno)?;
                put(arg, &out).map(|()| 0)
            }
            ioctl::CTL_ELEM_WRITE => {
                let b = get::<{ abi::ELEM_VALUE_LEN }>(arg)?;
                let w = self.controls(|c| c.write(&b)).map_err(errno)?;
                if let Some(id) = &w.changed {
                    self.announce(id);
                }
                put(arg, &w.reply).map(|()| 0)
            }
            // No element locks; taking one always works.
            ioctl::CTL_ELEM_LOCK | ioctl::CTL_ELEM_UNLOCK => get::<{ abi::ELEM_ID_LEN }>(arg).map(|_| 0),
            // No element has dB information.
            ioctl::CTL_TLV_READ => Err(Errno::ENXIO),
            // Always fully powered (SNDRV_CTL_POWER_D0).
            ioctl::CTL_POWER_STATE => put_int(arg, 0),
            ioctl::CTL_SUBSCRIBE_EVENTS => {
                let v = get_int(arg)?;
                if v < 0 {
                    return put_int(arg, i32::from(self.events.subscribed.load(SeqCst)));
                }
                self.events.subscribed.store(v > 0, SeqCst);
                if v == 0 {
                    self.events.queue.lock().clear();
                }
                Ok(0)
            }
            _ => Err(Errno::ENOTTY),
        }
    }
}

// ---- PCM devices -----------------------------------------------------------

/// `/dev/snd/pcmC0D<n>p` or `...c` as registered: opening it takes a free
/// substream and makes a [`PcmFile`].
struct PcmNode {
    card: Arc<Card>,
    desc: usize,
}

impl core::fmt::Debug for PcmNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PcmNode").field("desc", &self.desc).finish_non_exhaustive()
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
        let sub = {
            let mut g = self.card.inner.lock();
            let base = self.desc * SUBDEVICES;
            let k = (0..SUBDEVICES).find(|&k| !g.subs[base + k].open).ok_or(Errno::EBUSY)?;
            let s = &mut g.subs[base + k];
            s.open = true;
            // A fresh runtime for every open (its counters too).
            s.rt = PcmRuntime::with_dir(s.soft.dir);
            s.soft.status = None;
            s.soft.control = None;
            k
        };
        Ok(Some(Arc::new(PcmFile {
            card: self.card.clone(),
            desc: self.desc,
            sub,
            nonblock: flags.contains(OpenFlags::NONBLOCK),
        })))
    }
}

/// An open PCM device: one substream.
pub struct PcmFile {
    card: Arc<Card>,
    desc: usize,
    sub: usize,
    nonblock: bool,
}

impl core::fmt::Debug for PcmFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PcmFile").field("desc", &self.desc).field("sub", &self.sub).finish_non_exhaustive()
    }
}

impl Drop for PcmFile {
    fn drop(&mut self) {
        let xruns = self.with(|rt, g| {
            rt.close(g);
            rt.xruns
        });
        let stats = {
            let mut g = self.card.inner.lock();
            let s = &mut g.subs[self.index()];
            s.open = false;
            s.soft.status = None;
            s.soft.control = None;
            g.stats
        };
        self.card.notify();
        let (interrupts, driven) = super::interrupts();
        kprintln!(
            "sound: {}: {} closed: subdevice {}, {} xruns; server: {} periods out, {} in, {} device starts, {} interrupts ({})",
            self.card.bdf,
            node_name(&self.card.pcms[self.desc]),
            self.sub,
            xruns,
            stats.periods_out,
            stats.periods_in,
            stats.device_starts,
            interrupts,
            if driven { "interrupt-driven" } else { "polled" }
        );
    }
}

impl PcmFile {
    fn index(&self) -> usize {
        self.desc * SUBDEVICES + self.sub
    }

    fn dir(&self) -> Dir {
        self.card.pcms[self.desc].dir
    }

    fn with<R>(&self, f: impl FnOnce(&mut PcmRuntime, &mut SoftGlue<'_>) -> R) -> R {
        self.card.with_sub(self.index(), f)
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
        let events = PollEvents::WRITABLE | PollEvents::READABLE;
        poly_vfs::poll::wait_until_done(queue, events, &mut blocker, Timeout::Never, attempt)
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

    /// One read attempt of up to `frames` frames to user memory at `buf`:
    /// `EAGAIN` with nothing captured.
    fn read_user(&self, buf: u64, frames: u64, fb: u64) -> Result<u64> {
        let n = frames.min((CHUNK_BYTES as u64 / fb).max(1));
        let mut data = vec![0u8; (n * fb) as usize];
        let r = self.with(|rt, g| {
            let now = stamp(rt);
            rt.read(&mut data, g, now)
        });
        match r.map_err(errno)? {
            0 => Err(Errno::EAGAIN),
            got => {
                user::copy_to_user(buf, &data[..(got * fb) as usize]).map_err(|_| Errno::EFAULT)?;
                Ok(got)
            }
        }
    }

    fn frame_bytes(&self) -> Result<u64> {
        self.with(|rt, _| rt.config().map(|c| c.frame_bytes as u64)).ok_or(Errno::EBADFD)
    }

    /// `SNDRV_PCM_IOCTL_WRITEI_FRAMES` / `READI_FRAMES`: move every frame
    /// (blocking), or as many as possible now (non-blocking); `result`
    /// gets the count.
    fn xfer(&self, arg: u64, write: bool) -> Result<u64> {
        let x = Xferi::decode(&get::<{ abi::XFERI_LEN }>(arg)?);
        let fb = self.frame_bytes()?;
        if write {
            self.with(|rt, _| rt.writable()).map_err(errno)?;
        } else {
            self.with(|rt, _| rt.readable()).map_err(errno)?;
        }
        let mut done = 0u64;
        while done < x.frames {
            let at = x.buf.wrapping_add(done * fb);
            let left = x.frames - done;
            let r = if write {
                self.wait(self.nonblock, || self.write_user(at, left, fb))
            } else {
                self.wait(self.nonblock, || self.read_user(at, left, fb))
            };
            match r {
                Ok(n) => done += n,
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
        let desc = &self.card.pcms[self.desc];
        let ok = |r: core::result::Result<(), LinuxErrno>| r.map(|()| 0).map_err(errno);
        match request {
            ioctl::PCM_PVERSION => put_int(arg, abi::PCM_VERSION as i32),
            ioctl::PCM_INFO => {
                let subname = alloc::format!("subdevice #{}", self.sub);
                let info = PcmInfo {
                    device: desc.device,
                    subdevice: self.sub as u32,
                    stream: alsa_stream(desc.dir),
                    card: CARD as i32,
                    id: self.card.driver,
                    name: &desc.name,
                    subname: &subname,
                    subdevices_count: SUBDEVICES as u32,
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
                let r = self.with(|rt, g| {
                    let now = stamp(rt);
                    rt.sync_ptr(&req, g, now)
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
            ioctl::PCM_WRITEI_FRAMES => self.xfer(arg, true),
            ioctl::PCM_READI_FRAMES => self.xfer(arg, false),
            // No pausing: the stream's info has no PAUSE.
            ioctl::PCM_PAUSE | ioctl::PCM_RESUME => Err(Errno::ENOSYS),
            ioctl::PCM_REWIND => {
                let n = u64::from_le_bytes(get::<8>(arg)?);
                self.with(|rt, _| rt.rewind(n)).map_err(errno)
            }
            ioctl::PCM_FORWARD => {
                let n = u64::from_le_bytes(get::<8>(arg)?);
                self.with(|rt, g| rt.forward(n, g)).map_err(errno)
            }
            _ => Err(Errno::ENOTTY),
        }
    }

    /// `mmap(2)` at `offset` for `len` bytes: the ring buffer (offset 0,
    /// once the hardware parameters are set), or the status or control
    /// page. Returns what to map and the offset into it.
    fn map(&self, offset: u64, len: u64) -> Mapping {
        let mut g = self.card.inner.lock();
        let sub = &mut g.subs[self.index()];
        let page = crate::user::memory::PAGE_SIZE;
        match offset {
            mmap_offset::DATA => {
                if !sub.rt.is_mmap() {
                    // ALSA maps the buffer for mmap access only.
                    return Err(LinuxErrno::EINVAL);
                }
                let buf = sub.soft.buf.as_ref().ok_or(LinuxErrno::EBADFD)?;
                if len > buf.map_len() {
                    return Err(LinuxErrno::EINVAL);
                }
                Ok((buf.source(), 0))
            }
            mmap_offset::STATUS_OLD | mmap_offset::STATUS_NEW => {
                if len > page {
                    return Err(LinuxErrno::EINVAL);
                }
                if sub.soft.status.is_none() {
                    sub.soft.status = Some(SharedBuf::new(page as usize).ok_or(LinuxErrno::ENOMEM)?);
                }
                let src = sub.soft.status.as_ref().map(SharedBuf::source).ok_or(LinuxErrno::ENOMEM)?;
                super::push_pages(sub, super::monotonic());
                Ok((src, 0))
            }
            mmap_offset::CONTROL_OLD | mmap_offset::CONTROL_NEW => {
                if len > page {
                    return Err(LinuxErrno::EINVAL);
                }
                if sub.soft.control.is_none() {
                    let c = SharedBuf::new(page as usize).ok_or(LinuxErrno::ENOMEM)?;
                    let now =
                        MmapControl { appl_ptr: sub.rt.wrap(sub.rt.appl()), avail_min: sub.rt.sw_params().avail_min };
                    c.write(0, &now.encode());
                    sub.soft.last_appl = now.appl_ptr;
                    sub.soft.last_avail_min = now.avail_min;
                    sub.soft.control = Some(c);
                }
                let src = sub.soft.control.as_ref().map(SharedBuf::source).ok_or(LinuxErrno::ENOMEM)?;
                Ok((src, 0))
            }
            _ => Err(LinuxErrno::EINVAL),
        }
    }
}

impl CharDevice for PcmFile {
    /// `read(2)` of interleaved frames from a capture stream (whole frames
    /// only); `EAGAIN` with nothing captured, which `read_wait` turns into
    /// blocking.
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> Result<usize> {
        if self.dir() != Dir::Capture {
            return Err(Errno::EBADFD);
        }
        let fb = self.frame_bytes()? as usize;
        if buf.len() < fb {
            return Err(Errno::EINVAL);
        }
        let r = self.with(|rt, g| {
            let now = stamp(rt);
            rt.read(buf, g, now)
        });
        match r.map_err(errno)? {
            0 => Err(Errno::EAGAIN),
            n => Ok(n as usize * fb),
        }
    }

    /// `write(2)` of interleaved frames (whole frames only); `EAGAIN`
    /// without room, which `write_wait` turns into blocking.
    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        let fb = self.frame_bytes()? as usize;
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
        if r.readable {
            e = e | PollEvents::READABLE;
        }
        if r.error {
            e = e | PollEvents::ERR;
        }
        e
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.card.queue)
    }

    fn as_any(&self) -> Option<&(dyn core::any::Any + Send + Sync)> {
        Some(self)
    }
}

/// `mmap(2)` of a character device: `Some` if it is an open PCM device
/// (`MAP_SHARED` only, as ALSA requires), with what to map and the offset
/// into it.
pub fn map(dev: &Arc<dyn CharDevice>, offset: u64, len: u64, shared: bool) -> Option<Mapping> {
    let file = dev.as_any()?.downcast_ref::<PcmFile>()?;
    if !shared {
        return Some(Err(LinuxErrno::EINVAL));
    }
    Some(file.map(offset, len))
}
