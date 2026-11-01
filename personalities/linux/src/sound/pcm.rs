//! The runtime of one ALSA substream, playback or capture: its state
//! (`SNDRV_PCM_STATE_*`), the application and hardware pointers, the
//! software parameters, and what each `ioctl` does to them. The device
//! behind it is a [`PcmBackend`]; the runtime never blocks; the kernel
//! waits between attempts.
//!
//! Pointers count frames from PREPARE. For playback `appl` is what the
//! application wrote and `hw` what the device has taken (it moves when the
//! device hands back data: [`PcmRuntime::on_played`]); `avail = buffer -
//! (appl - hw)` frames may be written. For capture `hw` is what the device
//! has captured into the buffer ([`PcmRuntime::on_captured`]) and `appl`
//! what the application has read; `avail = hw - appl` frames may be read.
//! ALSA reports both pointers modulo the boundary, a power-of-two multiple
//! of the buffer size (`struct snd_pcm_sw_params`: "pointers wrap point").
//!
//! With mmap access the application moves `appl` itself, through
//! `SYNC_PTR` or the mapped control page ([`PcmRuntime::set_appl`]); the
//! frames are already in (or taken from) the mapped ring buffer.
//!
//! State changes follow the ALSA PCM documentation:
//!
//! ```text
//! OPEN --HW_PARAMS--> SETUP --PREPARE--> PREPARED --START, or writes reaching start_threshold--> RUNNING
//! RUNNING --avail reaches stop_threshold--> XRUN --PREPARE--> PREPARED
//! playback: RUNNING --DRAIN--> DRAINING --all played--> SETUP
//! capture:  RUNNING --DRAIN--> device stopped; DRAINING while frames are left to read, then SETUP
//! any but OPEN --DROP--> SETUP;  SETUP/PREPARED --HW_FREE--> OPEN
//! ```
//!
//! A capture stream also starts when a read asks for at least
//! `start_threshold` frames in PREPARED (as ALSA's read does). An overrun
//! is the capture side of XRUN: the buffer filled up to `stop_threshold`
//! with nobody reading.

use super::abi::{self, MmapStatus, Status, SwParams, SyncPtr, Timespec, access, state, sync_ptr};
use super::hw::HwConfig;
use crate::errno::Errno;

/// The device side of a substream. Every call is made with the runtime's
/// lock held; none may block for long.
pub trait PcmBackend {
    /// Set the device up for `cfg` (VIRTIO: SET PARAMETERS).
    fn configure(&mut self, cfg: &HwConfig) -> Result<(), Errno>;
    /// Ready the device to play (or capture) from the start of the
    /// buffer; whatever it was doing stops and is forgotten.
    fn prepare(&mut self) -> Result<(), Errno>;
    /// Start playing what has been committed, or capturing.
    fn start(&mut self) -> Result<(), Errno>;
    /// Stop now; what has not been played is dropped.
    fn stop(&mut self) -> Result<(), Errno>;
    /// Copy `data` into the ring buffer at byte `offset` (within the
    /// buffer, not wrapping).
    fn write(&mut self, offset: usize, data: &[u8]) -> Result<(), Errno>;
    /// `bytes` more bytes follow what was committed before; hand what is
    /// ready to the device if `running`.
    fn commit(&mut self, bytes: usize, running: bool);
    /// The stream drains: hand everything over, the last period padded
    /// with silence.
    fn drain(&mut self);
    /// Give up the device's resources (HW_FREE, close).
    fn release(&mut self);
    /// Capture: copy ring buffer bytes at `offset` (not wrapping) into
    /// `out`.
    fn read(&mut self, _offset: usize, _out: &mut [u8]) -> Result<(), Errno> {
        Err(Errno::EBADFD)
    }
    /// Capture: the reader has taken `bytes` more, so the device may fill
    /// them again.
    fn consumed(&mut self, _bytes: usize) {}
}

/// Which way the frames go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Playback,
    Capture,
}

/// `SNDRV_PCM_STATE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Setup,
    Prepared,
    Running,
    Xrun,
    Draining,
}

impl State {
    pub const fn code(self) -> u32 {
        match self {
            State::Open => state::OPEN,
            State::Setup => state::SETUP,
            State::Prepared => state::PREPARED,
            State::Running => state::RUNNING,
            State::Xrun => state::XRUN,
            State::Draining => state::DRAINING,
        }
    }
}

/// What `poll(2)` sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Readiness {
    pub writable: bool,
    pub readable: bool,
    pub error: bool,
}

/// One substream.
#[derive(Clone, Debug)]
pub struct PcmRuntime {
    dir: Dir,
    state: State,
    hw: Option<HwConfig>,
    sw: SwParams,
    appl: u64,
    hwp: u64,
    boundary: u64,
    trigger: Timespec,
    avail_max: u64,
    /// The user's protocol version (`SNDRV_PCM_IOCTL_USER_PVERSION`).
    pub user_version: u32,
    /// Underruns (or overruns) since open.
    pub xruns: u64,
}

impl Default for PcmRuntime {
    fn default() -> PcmRuntime {
        PcmRuntime::new()
    }
}

/// The boundary for a buffer of `buffer` frames: the largest power-of-two
/// multiple that leaves room for one more buffer below `i64::MAX`.
pub fn boundary_for(buffer: u64) -> u64 {
    if buffer == 0 {
        return 0;
    }
    let mut b = buffer;
    while b.checked_mul(2).is_some_and(|d| d <= i64::MAX as u64 - buffer) {
        b *= 2;
    }
    b
}

impl PcmRuntime {
    /// A playback substream.
    pub const fn new() -> PcmRuntime {
        PcmRuntime::with_dir(Dir::Playback)
    }

    /// A capture substream.
    pub const fn capture() -> PcmRuntime {
        PcmRuntime::with_dir(Dir::Capture)
    }

    pub const fn with_dir(dir: Dir) -> PcmRuntime {
        PcmRuntime {
            dir,
            state: State::Open,
            hw: None,
            sw: SwParams {
                tstamp_mode: 0,
                period_step: 1,
                sleep_min: 0,
                avail_min: 1,
                xfer_align: 1,
                start_threshold: 1,
                stop_threshold: 0,
                silence_threshold: 0,
                silence_size: 0,
                boundary: 0,
                proto: abi::PCM_VERSION,
                tstamp_type: 0,
            },
            appl: 0,
            hwp: 0,
            boundary: 0,
            trigger: Timespec { sec: 0, nsec: 0 },
            avail_max: 0,
            user_version: 0,
            xruns: 0,
        }
    }

    pub const fn dir(&self) -> Dir {
        self.dir
    }

    pub const fn state(&self) -> State {
        self.state
    }

    pub const fn config(&self) -> Option<HwConfig> {
        self.hw
    }

    pub const fn sw_params(&self) -> SwParams {
        self.sw
    }

    /// Frames written (playback) or read (capture) since PREPARE.
    pub const fn appl(&self) -> u64 {
        self.appl
    }

    /// Frames played (playback) or captured (capture) since PREPARE.
    pub const fn hw(&self) -> u64 {
        self.hwp
    }

    fn buffer(&self) -> u64 {
        self.hw.map_or(0, |h| h.buffer_frames as u64)
    }

    /// Is the ring buffer mapped rather than read and written?
    pub fn is_mmap(&self) -> bool {
        self.hw.is_some_and(|h| h.access == access::MMAP_INTERLEAVED)
    }

    /// Frames between the pointers: written and not played (playback),
    /// captured and not read (capture).
    pub fn queued(&self) -> u64 {
        match self.dir {
            Dir::Playback => self.appl - self.hwp,
            Dir::Capture => self.hwp - self.appl,
        }
    }

    /// Frames the application may move now: write (playback) or read
    /// (capture).
    pub fn avail(&self) -> u64 {
        match self.dir {
            Dir::Playback => self.buffer().saturating_sub(self.queued()),
            Dir::Capture => self.queued(),
        }
    }

    /// The rest of the buffer: `buffer - avail` (for playback, the frames
    /// waiting to be played).
    pub fn hw_avail(&self) -> u64 {
        self.buffer().saturating_sub(self.avail())
    }

    /// `SNDRV_PCM_IOCTL_HW_PARAMS` once [`super::hw::choose`] settled the
    /// configuration: the device is set up, the software parameters get
    /// their defaults (start at the first frame, stop when the buffer runs
    /// empty or full, wake per period), state SETUP.
    pub fn hw_params(&mut self, cfg: HwConfig, be: &mut dyn PcmBackend) -> Result<(), Errno> {
        if !matches!(self.state, State::Open | State::Setup | State::Prepared) {
            return Err(Errno::EBADFD);
        }
        be.configure(&cfg)?;
        self.hw = Some(cfg);
        self.boundary = boundary_for(cfg.buffer_frames as u64);
        self.sw.avail_min = cfg.period_frames as u64;
        self.sw.start_threshold = 1;
        self.sw.stop_threshold = cfg.buffer_frames as u64;
        self.sw.silence_threshold = 0;
        self.sw.silence_size = 0;
        self.sw.boundary = self.boundary;
        self.appl = 0;
        self.hwp = 0;
        self.state = State::Setup;
        Ok(())
    }

    /// `SNDRV_PCM_IOCTL_HW_FREE`.
    pub fn hw_free(&mut self, be: &mut dyn PcmBackend) -> Result<(), Errno> {
        match self.state {
            State::Running | State::Draining => Err(Errno::EBADFD),
            State::Open => Ok(()),
            _ => {
                be.release();
                self.hw = None;
                self.state = State::Open;
                Ok(())
            }
        }
    }

    /// `SNDRV_PCM_IOCTL_SW_PARAMS`: checked and kept; returns what the
    /// caller gets back (the boundary filled in).
    pub fn set_sw_params(&mut self, sw: &SwParams) -> Result<SwParams, Errno> {
        let Some(hw) = self.hw else { return Err(Errno::EBADFD) };
        if sw.tstamp_mode > 1 || sw.tstamp_type > abi::tstamp_type::MONOTONIC_RAW {
            return Err(Errno::EINVAL);
        }
        if sw.silence_size > self.boundary || sw.silence_threshold > hw.buffer_frames as u64 {
            return Err(Errno::EINVAL);
        }
        if sw.avail_min == 0 {
            return Err(Errno::EINVAL);
        }
        let mut s = *sw;
        s.boundary = self.boundary;
        self.sw = s;
        Ok(s)
    }

    /// `SNDRV_PCM_IOCTL_PREPARE`.
    pub fn prepare(&mut self, be: &mut dyn PcmBackend) -> Result<(), Errno> {
        if self.state == State::Open {
            return Err(Errno::EBADFD);
        }
        be.prepare()?;
        self.appl = 0;
        self.hwp = 0;
        self.avail_max = 0;
        self.state = State::Prepared;
        Ok(())
    }

    /// `SNDRV_PCM_IOCTL_RESET`: drop what is written and not played, or
    /// captured and not read.
    pub fn reset(&mut self) -> Result<(), Errno> {
        match self.state {
            State::Prepared | State::Running | State::Xrun => {
                self.appl = self.hwp;
                Ok(())
            }
            _ => Err(Errno::EBADFD),
        }
    }

    /// `SNDRV_PCM_IOCTL_START`: from PREPARED only; a playback stream with
    /// nothing written would underrun at once (`EPIPE`).
    pub fn start(&mut self, be: &mut dyn PcmBackend, now: Timespec) -> Result<(), Errno> {
        if self.state != State::Prepared {
            return Err(Errno::EBADFD);
        }
        if self.dir == Dir::Playback && self.queued() == 0 {
            return Err(Errno::EPIPE);
        }
        be.start()?;
        self.state = State::Running;
        self.trigger = now;
        Ok(())
    }

    /// `SNDRV_PCM_IOCTL_DROP`: stop at once, state SETUP.
    pub fn drop_stream(&mut self, be: &mut dyn PcmBackend, now: Timespec) -> Result<(), Errno> {
        match self.state {
            State::Open => Err(Errno::EBADFD),
            State::Setup => Ok(()),
            _ => {
                let r = be.stop();
                self.state = State::Setup;
                self.trigger = now;
                r
            }
        }
    }

    /// `SNDRV_PCM_IOCTL_DRAIN`, the first step: true when the drain is
    /// over (nothing was left to play), false if the caller must wait for
    /// the state to leave DRAINING. A capture stream stops at once; what
    /// it captured can still be read (DRAINING until then).
    pub fn drain(&mut self, be: &mut dyn PcmBackend, now: Timespec) -> Result<bool, Errno> {
        if self.dir == Dir::Capture {
            return match self.state {
                State::Open => Err(Errno::EBADFD),
                State::Setup | State::Draining => Ok(true),
                State::Prepared => {
                    self.state = State::Setup;
                    Ok(true)
                }
                State::Running | State::Xrun => {
                    let r = be.stop();
                    self.state =
                        if self.state == State::Running && self.avail() > 0 { State::Draining } else { State::Setup };
                    self.trigger = now;
                    r.map(|()| true)
                }
            };
        }
        match self.state {
            State::Open => Err(Errno::EBADFD),
            State::Setup => Ok(true),
            State::Xrun => {
                let r = be.stop();
                self.state = State::Setup;
                r.map(|()| true)
            }
            State::Prepared if self.queued() == 0 => {
                self.state = State::Setup;
                Ok(true)
            }
            State::Prepared | State::Running => {
                if self.state == State::Prepared {
                    be.start()?;
                    self.trigger = now;
                }
                self.state = State::Draining;
                be.drain();
                self.check_drained(be, now);
                Ok(self.state != State::Draining)
            }
            State::Draining => Ok(false),
        }
    }

    fn check_drained(&mut self, be: &mut dyn PcmBackend, now: Timespec) {
        if self.dir == Dir::Playback && self.state == State::Draining && self.hwp >= self.appl {
            let _ = be.stop();
            self.state = State::Setup;
            self.trigger = now;
        }
    }

    /// Playback: the device has played `total` frames since PREPARE (it
    /// only ever grows; more than was written is not believed). An
    /// underrun when `avail` reaches the stop threshold; the end of a
    /// drain when everything is played.
    pub fn on_played(&mut self, total: u64, be: &mut dyn PcmBackend, now: Timespec) {
        if self.dir != Dir::Playback || !matches!(self.state, State::Running | State::Draining) {
            return;
        }
        self.hwp = self.hwp.max(total.min(self.appl));
        self.avail_max = self.avail_max.max(self.avail());
        match self.state {
            State::Running if self.avail() >= self.sw.stop_threshold.max(1) => {
                let _ = be.stop();
                self.state = State::Xrun;
                self.trigger = now;
                self.xruns += 1;
            }
            State::Draining => self.check_drained(be, now),
            _ => {}
        }
    }

    /// Capture: the device has captured `total` frames into the buffer
    /// since PREPARE (it only ever grows, and never more than a buffer
    /// ahead of the reader). An overrun when `avail` reaches the stop
    /// threshold.
    pub fn on_captured(&mut self, total: u64, be: &mut dyn PcmBackend, now: Timespec) {
        if self.dir != Dir::Capture || self.state != State::Running {
            return;
        }
        self.hwp = self.hwp.max(total.min(self.appl + self.buffer()));
        self.avail_max = self.avail_max.max(self.avail());
        if self.avail() >= self.sw.stop_threshold.max(1) {
            let _ = be.stop();
            self.state = State::Xrun;
            self.trigger = now;
            self.xruns += 1;
        }
    }

    /// Room for the device in a capture buffer: frames it may capture
    /// before it would overwrite unread ones.
    pub fn capture_room(&self) -> u64 {
        self.buffer().saturating_sub(self.queued())
    }

    /// How many frames a write may take now: `EPIPE` after an underrun,
    /// `EBADFD` before PREPARE, while draining, or on a capture stream.
    pub fn writable(&self) -> Result<u64, Errno> {
        if self.dir != Dir::Playback {
            return Err(Errno::EBADFD);
        }
        match self.state {
            State::Xrun => Err(Errno::EPIPE),
            State::Prepared | State::Running => Ok(self.avail()),
            _ => Err(Errno::EBADFD),
        }
    }

    /// Write whole frames from `data` (as many as there is room for);
    /// starts the stream once `start_threshold` frames are waiting.
    /// Returns the frames taken (0 if there was no room).
    pub fn write(&mut self, data: &[u8], be: &mut dyn PcmBackend, now: Timespec) -> Result<u64, Errno> {
        let room = self.writable()?;
        let hw = self.hw.ok_or(Errno::EBADFD)?;
        let fb = hw.frame_bytes as u64;
        let frames = (data.len() as u64 / fb).min(room);
        if frames > 0 {
            let buffer = hw.buffer_frames as u64;
            let at = self.appl % buffer;
            let first = frames.min(buffer - at);
            let (a, b) = data[..(frames * fb) as usize].split_at((first * fb) as usize);
            be.write((at * fb) as usize, a)?;
            if !b.is_empty() {
                be.write(0, b)?;
            }
            self.appl += frames;
            be.commit((frames * fb) as usize, self.state == State::Running);
        }
        self.start_at_threshold(be, now)?;
        Ok(frames)
    }

    fn start_at_threshold(&mut self, be: &mut dyn PcmBackend, now: Timespec) -> Result<(), Errno> {
        if self.state == State::Prepared && self.queued() > 0 && self.queued() >= self.sw.start_threshold {
            self.start(be, now)?;
        }
        Ok(())
    }

    /// How many frames a read may take now: `EPIPE` after an overrun,
    /// `EBADFD` on a stream that is not capturing (or not capture).
    pub fn readable(&self) -> Result<u64, Errno> {
        if self.dir != Dir::Capture {
            return Err(Errno::EBADFD);
        }
        match self.state {
            State::Xrun => Err(Errno::EPIPE),
            State::Prepared | State::Running | State::Draining => Ok(self.avail()),
            _ => Err(Errno::EBADFD),
        }
    }

    /// Read whole frames into `out` (as many as are captured and fit). A
    /// read of at least `start_threshold` frames starts a prepared stream.
    /// Returns the frames read (0 if none were there yet); a draining
    /// stream whose last frame was read is set up.
    pub fn read(&mut self, out: &mut [u8], be: &mut dyn PcmBackend, now: Timespec) -> Result<u64, Errno> {
        self.readable()?;
        let hw = self.hw.ok_or(Errno::EBADFD)?;
        let fb = hw.frame_bytes as u64;
        let want = out.len() as u64 / fb;
        if self.state == State::Prepared && want > 0 && want >= self.sw.start_threshold {
            self.start(be, now)?;
        }
        let frames = want.min(self.avail());
        if frames > 0 {
            let buffer = hw.buffer_frames as u64;
            let at = self.appl % buffer;
            let first = frames.min(buffer - at);
            let (a, b) = out[..(frames * fb) as usize].split_at_mut((first * fb) as usize);
            be.read((at * fb) as usize, a)?;
            if !b.is_empty() {
                be.read(0, b)?;
            }
            self.appl += frames;
            be.consumed((frames * fb) as usize);
        }
        if self.state == State::Draining && self.avail() == 0 {
            self.state = State::Setup;
        }
        Ok(frames)
    }

    /// The absolute position nearest `appl` that is `wrapped` modulo the
    /// boundary.
    fn unwrap_ptr(&self, wrapped: u64) -> Option<u64> {
        if self.boundary == 0 || wrapped >= self.boundary {
            return None;
        }
        let base = self.appl - self.appl % self.boundary;
        let candidates = [base.checked_sub(self.boundary), Some(base), base.checked_add(self.boundary)];
        candidates.into_iter().flatten().map(|b| b + wrapped).min_by_key(|&v| v.abs_diff(self.appl))
    }

    /// mmap access: the application moved its pointer to `wrapped`
    /// (modulo the boundary), through `SYNC_PTR` or the control page.
    /// Playback may move it ahead up to a full buffer past `hw` (frames it
    /// put in the mapped buffer) or back to `hw`; capture up to `hw`
    /// (frames it took from the mapped buffer) or back over frames still
    /// in the buffer. A pointer outside is `EINVAL` and ignored. Moving it does not start the stream: mmap applications
    /// call `START` (as ALSA's documentation of mmap transfers says).
    pub fn set_appl(&mut self, wrapped: u64, be: &mut dyn PcmBackend) -> Result<(), Errno> {
        let hw = self.hw.ok_or(Errno::EBADFD)?;
        if !matches!(self.state, State::Prepared | State::Running | State::Draining | State::Xrun) {
            return Err(Errno::EBADFD);
        }
        let new = self.unwrap_ptr(wrapped).ok_or(Errno::EINVAL)?;
        if new == self.appl {
            return Ok(());
        }
        let buffer = hw.buffer_frames as u64;
        let ok = match self.dir {
            Dir::Playback => new >= self.hwp && new <= self.hwp + buffer,
            Dir::Capture => new <= self.hwp && new + buffer >= self.hwp,
        };
        if !ok {
            return Err(Errno::EINVAL);
        }
        let fb = hw.frame_bytes as usize;
        match self.dir {
            Dir::Playback => {
                let ahead = new.saturating_sub(self.appl);
                self.appl = new;
                if ahead > 0 {
                    be.commit(ahead as usize * fb, self.state == State::Running);
                }
            }
            Dir::Capture => {
                let read = new.saturating_sub(self.appl);
                self.appl = new;
                if read > 0 {
                    be.consumed(read as usize * fb);
                }
                if self.state == State::Draining && self.avail() == 0 {
                    self.state = State::Setup;
                }
            }
        }
        Ok(())
    }

    /// `SNDRV_PCM_IOCTL_REWIND`: move `appl` back by up to `frames`, over
    /// frames the device has not taken yet (playback) or that were read
    /// and are still in the buffer (capture). Returns how far it moved.
    pub fn rewind(&mut self, frames: u64) -> Result<u64, Errno> {
        if !matches!(self.state, State::Prepared | State::Running) {
            return Err(if self.state == State::Xrun { Errno::EPIPE } else { Errno::EBADFD });
        }
        let n = match self.dir {
            Dir::Playback => frames.min(self.queued()),
            Dir::Capture => frames.min(self.capture_room()).min(self.appl),
        };
        self.appl -= n;
        Ok(n)
    }

    /// `SNDRV_PCM_IOCTL_FORWARD`: move `appl` ahead by up to `frames`, over
    /// free room (playback: the frames are left as they are in the buffer)
    /// or captured frames (capture: skipped). Returns how far it moved.
    pub fn forward(&mut self, frames: u64, be: &mut dyn PcmBackend) -> Result<u64, Errno> {
        if !matches!(self.state, State::Prepared | State::Running) {
            return Err(if self.state == State::Xrun { Errno::EPIPE } else { Errno::EBADFD });
        }
        let fb = self.hw.map_or(0, |h| h.frame_bytes as usize);
        let n = frames.min(self.avail());
        self.appl += n;
        match self.dir {
            Dir::Playback if n > 0 => be.commit(n as usize * fb, self.state == State::Running),
            Dir::Capture if n > 0 => be.consumed(n as usize * fb),
            _ => {}
        }
        Ok(n)
    }

    /// `snd_pcm_status` now.
    pub fn status(&mut self, now: Timespec) -> Status {
        let s = Status {
            state: self.state.code(),
            trigger_tstamp: self.trigger,
            tstamp: now,
            appl_ptr: self.wrap(self.appl),
            hw_ptr: self.wrap(self.hwp),
            delay: self.delay_now(),
            avail: self.avail(),
            avail_max: self.avail_max.max(self.avail()),
            suspended_state: 0,
        };
        self.avail_max = 0;
        s
    }

    fn delay_now(&self) -> i64 {
        match self.state {
            State::Running | State::Draining | State::Prepared => self.queued() as i64,
            _ => 0,
        }
    }

    /// `SNDRV_PCM_IOCTL_DELAY`.
    pub fn delay(&self) -> Result<i64, Errno> {
        match self.state {
            State::Running | State::Draining | State::Prepared => Ok(self.queued() as i64),
            State::Xrun => Err(Errno::EPIPE),
            _ => Err(Errno::EBADFD),
        }
    }

    /// `SNDRV_PCM_IOCTL_HWSYNC`.
    pub fn hwsync(&self) -> Result<(), Errno> {
        match self.state {
            State::Running | State::Draining | State::Prepared => Ok(()),
            State::Xrun => Err(Errno::EPIPE),
            _ => Err(Errno::EBADFD),
        }
    }

    /// `v` modulo the boundary, as ALSA reports pointers.
    pub fn wrap(&self, v: u64) -> u64 {
        if self.boundary == 0 { v } else { v % self.boundary }
    }

    /// `SNDRV_PCM_IOCTL_SYNC_PTR`. Without `SYNC_PTR_APPL` the caller
    /// passes its application pointer: taken with mmap access (it moved
    /// it in the mapped buffer); with read/write access the kernel moves
    /// it (by writes and reads), so a different one is not taken. Without
    /// `SYNC_PTR_AVAIL_MIN` the caller sets `avail_min`.
    pub fn sync_ptr(&mut self, req: &SyncPtr, be: &mut dyn PcmBackend, now: Timespec) -> SyncPtr {
        if req.flags & sync_ptr::AVAIL_MIN == 0 && req.avail_min > 0 && self.hw.is_some() {
            self.sw.avail_min = req.avail_min;
        }
        if req.flags & sync_ptr::APPL == 0 && self.is_mmap() {
            let _ = self.set_appl(req.appl_ptr, be);
        }
        SyncPtr {
            flags: req.flags,
            state: self.state.code(),
            hw_ptr: self.wrap(self.hwp),
            tstamp: now,
            suspended_state: 0,
            appl_ptr: self.wrap(self.appl),
            avail_min: self.sw.avail_min,
        }
    }

    /// The mapped status page's contents now.
    pub fn mmap_status(&self, now: Timespec) -> MmapStatus {
        MmapStatus {
            state: self.state.code(),
            hw_ptr: self.wrap(self.hwp),
            tstamp: now,
            suspended_state: 0,
            audio_tstamp: now,
        }
    }

    /// Take `avail_min` from the mapped control page (0 is ignored).
    pub fn set_avail_min(&mut self, v: u64) {
        if v > 0 && self.hw.is_some() {
            self.sw.avail_min = v;
        }
    }

    /// `poll(2)`: playback is writable once `avail_min` frames are free,
    /// capture readable once `avail_min` frames are captured (or, while
    /// draining, any); an xrun or a stream that is not set up is an error
    /// (and ready, so the caller finds out).
    pub fn readiness(&self) -> Readiness {
        let enough = self.avail() >= self.sw.avail_min.max(1);
        let (w, r) = (self.dir == Dir::Playback, self.dir == Dir::Capture);
        match self.state {
            State::Prepared | State::Running => {
                Readiness { writable: w && enough, readable: r && enough, error: false }
            }
            State::Draining => Readiness { writable: false, readable: r && self.avail() > 0, error: false },
            _ => Readiness { writable: w, readable: r, error: true },
        }
    }

    /// The file is closed: stop and give everything back.
    pub fn close(&mut self, be: &mut dyn PcmBackend) {
        if !matches!(self.state, State::Open | State::Setup) {
            let _ = be.stop();
        }
        if self.state != State::Open {
            be.release();
        }
        self.state = State::Open;
        self.hw = None;
    }
}
