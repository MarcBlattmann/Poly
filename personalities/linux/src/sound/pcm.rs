//! The runtime of one ALSA playback substream: its state
//! (`SNDRV_PCM_STATE_*`), the application and hardware pointers, the
//! software parameters, and what each `ioctl` does to them. The device
//! behind it is a [`PcmBackend`]; the runtime never blocks; the kernel
//! waits between attempts.
//!
//! Pointers count frames from PREPARE. `appl` is what the application
//! wrote, `hw` what the device has played (it moves when the device hands
//! back data: [`PcmRuntime::on_played`]). For playback `avail = buffer -
//! (appl - hw)` frames may be written. ALSA reports both pointers modulo
//! the boundary, a power-of-two multiple of the buffer size (`struct
//! snd_pcm_sw_params`: "pointers wrap point").
//!
//! State changes follow the ALSA PCM documentation:
//!
//! ```text
//! OPEN --HW_PARAMS--> SETUP --PREPARE--> PREPARED --START or writes reaching start_threshold--> RUNNING
//! RUNNING --avail reaches stop_threshold--> XRUN --PREPARE--> PREPARED
//! RUNNING --DRAIN--> DRAINING --all played--> SETUP
//! any but OPEN --DROP--> SETUP;  SETUP/PREPARED --HW_FREE--> OPEN
//! ```

use super::abi::{self, Status, SwParams, SyncPtr, Timespec, state, sync_ptr};
use super::hw::HwConfig;
use crate::errno::Errno;

/// The device side of a substream. Every call is made with the runtime's
/// lock held; none may block for long.
pub trait PcmBackend {
    /// Set the device up for `cfg` (VIRTIO: SET PARAMETERS).
    fn configure(&mut self, cfg: &HwConfig) -> Result<(), Errno>;
    /// Ready the device to play from the start of the buffer; whatever it
    /// was playing stops and is forgotten.
    fn prepare(&mut self) -> Result<(), Errno>;
    /// Start playing what has been committed.
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
    pub error: bool,
}

/// One playback substream.
#[derive(Clone, Debug)]
pub struct PcmRuntime {
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
    /// Underruns since open.
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
    pub const fn new() -> PcmRuntime {
        PcmRuntime {
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

    pub const fn state(&self) -> State {
        self.state
    }

    pub const fn config(&self) -> Option<HwConfig> {
        self.hw
    }

    pub const fn sw_params(&self) -> SwParams {
        self.sw
    }

    /// Frames written since PREPARE.
    pub const fn appl(&self) -> u64 {
        self.appl
    }

    /// Frames played since PREPARE.
    pub const fn hw(&self) -> u64 {
        self.hwp
    }

    fn buffer(&self) -> u64 {
        self.hw.map_or(0, |h| h.buffer_frames as u64)
    }

    /// Frames written and not played.
    pub fn hw_avail(&self) -> u64 {
        self.appl - self.hwp
    }

    /// Frames that may be written.
    pub fn avail(&self) -> u64 {
        self.buffer().saturating_sub(self.hw_avail())
    }

    /// `SNDRV_PCM_IOCTL_HW_PARAMS` once [`super::hw::choose`] settled the
    /// configuration: the device is set up, the software parameters get
    /// their defaults (start at the first frame written, stop when the
    /// buffer runs empty, wake per period), state SETUP.
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

    /// `SNDRV_PCM_IOCTL_RESET`: drop what is written and not played.
    pub fn reset(&mut self) -> Result<(), Errno> {
        match self.state {
            State::Prepared | State::Running | State::Xrun => {
                self.appl = self.hwp;
                Ok(())
            }
            _ => Err(Errno::EBADFD),
        }
    }

    /// `SNDRV_PCM_IOCTL_START`: from PREPARED only; with nothing written
    /// a playback stream would underrun at once (`EPIPE`).
    pub fn start(&mut self, be: &mut dyn PcmBackend, now: Timespec) -> Result<(), Errno> {
        if self.state != State::Prepared {
            return Err(Errno::EBADFD);
        }
        if self.hw_avail() == 0 {
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
    /// the state to leave DRAINING.
    pub fn drain(&mut self, be: &mut dyn PcmBackend, now: Timespec) -> Result<bool, Errno> {
        match self.state {
            State::Open => Err(Errno::EBADFD),
            State::Setup => Ok(true),
            State::Xrun => {
                let r = be.stop();
                self.state = State::Setup;
                r.map(|()| true)
            }
            State::Prepared if self.hw_avail() == 0 => {
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
        if self.state == State::Draining && self.hwp >= self.appl {
            let _ = be.stop();
            self.state = State::Setup;
            self.trigger = now;
        }
    }

    /// The device has played `total` frames since PREPARE (it only ever
    /// grows; more than was written is not believed). An underrun when
    /// `avail` reaches the stop threshold; the end of a drain when
    /// everything is played.
    pub fn on_played(&mut self, total: u64, be: &mut dyn PcmBackend, now: Timespec) {
        if !matches!(self.state, State::Running | State::Draining) {
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

    /// How many frames a write may take now: `EPIPE` after an underrun,
    /// `EBADFD` before PREPARE or while draining.
    pub fn writable(&self) -> Result<u64, Errno> {
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
        if self.state == State::Prepared && self.hw_avail() > 0 && self.hw_avail() >= self.sw.start_threshold {
            self.start(be, now)?;
        }
        Ok(frames)
    }

    /// `snd_pcm_status` now.
    pub fn status(&mut self, now: Timespec) -> Status {
        let s = Status {
            state: self.state.code(),
            trigger_tstamp: self.trigger,
            tstamp: now,
            appl_ptr: self.wrap(self.appl),
            hw_ptr: self.wrap(self.hwp),
            delay: match self.state {
                State::Running | State::Draining | State::Prepared => self.hw_avail() as i64,
                _ => 0,
            },
            avail: self.avail(),
            avail_max: self.avail_max.max(self.avail()),
            suspended_state: 0,
        };
        self.avail_max = 0;
        s
    }

    /// `SNDRV_PCM_IOCTL_DELAY`.
    pub fn delay(&self) -> Result<i64, Errno> {
        match self.state {
            State::Running | State::Draining | State::Prepared => Ok(self.hw_avail() as i64),
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

    fn wrap(&self, v: u64) -> u64 {
        if self.boundary == 0 { v } else { v % self.boundary }
    }

    /// `SNDRV_PCM_IOCTL_SYNC_PTR`. Without `SYNC_PTR_APPL` the caller
    /// passes its application pointer; with read/write access the kernel
    /// moves it (by writes), so a different value is not taken. Without
    /// `SYNC_PTR_AVAIL_MIN` the caller sets `avail_min`.
    pub fn sync_ptr(&mut self, req: &SyncPtr, now: Timespec) -> SyncPtr {
        if req.flags & sync_ptr::AVAIL_MIN == 0 && req.avail_min > 0 && self.hw.is_some() {
            self.sw.avail_min = req.avail_min;
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

    /// `poll(2)`: writable once `avail_min` frames are free while the
    /// stream is prepared or running; an underrun or a stream that is not
    /// set up is an error (and "writable", so a writer finds out).
    pub fn readiness(&self) -> Readiness {
        match self.state {
            State::Prepared | State::Running => {
                Readiness { writable: self.avail() >= self.sw.avail_min.max(1), error: false }
            }
            State::Draining => Readiness { writable: false, error: false },
            _ => Readiness { writable: true, error: true },
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
