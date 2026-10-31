//! The PCM stream state machine (§5.14.6.6.1, "PCM Command Lifecycle")
//! and the checks on stream parameters before they go to the device.
//!
//! The specification orders the commands of a stream: SET PARAMETERS
//! (from the start, or again after PREPARE or RELEASE), PREPARE (after SET
//! PARAMETERS or RELEASE, or again), START (after PREPARE or STOP), STOP
//! (after START) and RELEASE (after PREPARE or STOP). [`StreamState`]
//! follows the device through them, and [`StreamState::path_to`] lists the
//! commands that bring a stream from where it is to where the driver
//! wants it, so a caller never sends a command the device must refuse.

use crate::proto::{PcmInfo, SetParams, format, rate};

/// A command of the lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SetParams,
    Prepare,
    Start,
    Stop,
    Release,
}

impl Command {
    /// The request code (`VIRTIO_SND_R_PCM_*`).
    pub const fn code(self) -> u32 {
        use crate::proto::code;
        match self {
            Command::SetParams => code::PCM_SET_PARAMS,
            Command::Prepare => code::PCM_PREPARE,
            Command::Start => code::PCM_START,
            Command::Stop => code::PCM_STOP,
            Command::Release => code::PCM_RELEASE,
        }
    }
}

/// Where a stream is in its lifecycle, as far as the driver knows (the
/// state after the last command the device accepted).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamState {
    /// No command yet.
    #[default]
    Initial,
    ParamsSet,
    Prepared,
    Started,
    Stopped,
    Released,
}

impl StreamState {
    /// May `cmd` be sent in this state?
    pub const fn allows(self, cmd: Command) -> bool {
        use StreamState::*;
        match cmd {
            Command::SetParams => matches!(self, Initial | ParamsSet | Prepared | Released),
            Command::Prepare => matches!(self, ParamsSet | Prepared | Released),
            Command::Start => matches!(self, Prepared | Stopped),
            Command::Stop => matches!(self, Started),
            Command::Release => matches!(self, Prepared | Stopped),
        }
    }

    /// The state once the device accepted `cmd`.
    pub const fn after(self, cmd: Command) -> StreamState {
        match cmd {
            Command::SetParams => StreamState::ParamsSet,
            Command::Prepare => StreamState::Prepared,
            Command::Start => StreamState::Started,
            Command::Stop => StreamState::Stopped,
            Command::Release => StreamState::Released,
        }
    }

    /// The commands that bring the stream from here to the point where
    /// `goal` is sent, `goal` included (at most four). Empty if the goal
    /// cannot be reached (SET PARAMETERS is never inserted: parameters are
    /// the caller's to choose).
    pub fn path_to(self, goal: Command) -> PathTo {
        use Command::*;
        let mut path = PathTo { cmds: [Stop; 4], len: 0 };
        let mut state = self;
        // A short walk: at most STOP, RELEASE, PREPARE, then the goal.
        for _ in 0..4 {
            let next = if state.allows(goal) {
                goal
            } else {
                match (state, goal) {
                    (StreamState::Started, _) => Stop,
                    (StreamState::Stopped, SetParams | Prepare) => Release,
                    (StreamState::ParamsSet | StreamState::Released, Start) => Prepare,
                    _ => return PathTo { cmds: [Stop; 4], len: 0 },
                }
            };
            path.cmds[path.len] = next;
            path.len += 1;
            state = state.after(next);
            if next == goal {
                return path;
            }
        }
        PathTo { cmds: [Stop; 4], len: 0 }
    }
}

/// A list of commands from [`StreamState::path_to`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathTo {
    cmds: [Command; 4],
    len: usize,
}

impl PathTo {
    pub fn as_slice(&self) -> &[Command] {
        &self.cmds[..self.len]
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// What a caller wants from a stream, in plain units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wanted {
    /// A [`format`] code.
    pub format: u8,
    pub rate_hz: u32,
    pub channels: u8,
    pub period_bytes: u32,
    pub buffer_bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamError {
    /// The stream does not offer this format, or it is not whole bytes.
    Format,
    /// The stream does not offer this rate (or the spec has no code for it).
    Rate,
    /// Outside the stream's channel range.
    Channels,
    /// The period is empty or not a whole number of frames.
    Period,
    /// The buffer is not a whole number (at least one) of periods.
    Buffer,
}

/// Check `want` against what the stream offers and turn it into the SET
/// PARAMETERS request (§5.14.6.6.3: the format, rate and channel count
/// must be among those the stream reported). No stream features are
/// asked for: the driver uses plain messages, not shared memory.
pub fn check_params(info: &PcmInfo, want: &Wanted) -> Result<SetParams, ParamError> {
    if !info.has_format(want.format) {
        return Err(ParamError::Format);
    }
    let sample = format::physical_bits(want.format).ok_or(ParamError::Format)? / 8;
    let code = rate::code_of(want.rate_hz).ok_or(ParamError::Rate)?;
    if !info.has_rate(code) {
        return Err(ParamError::Rate);
    }
    if want.channels < info.channels_min || want.channels > info.channels_max || want.channels == 0 {
        return Err(ParamError::Channels);
    }
    let frame = sample * want.channels as u32;
    if want.period_bytes == 0 || !want.period_bytes.is_multiple_of(frame) {
        return Err(ParamError::Period);
    }
    if want.buffer_bytes < want.period_bytes || !want.buffer_bytes.is_multiple_of(want.period_bytes) {
        return Err(ParamError::Buffer);
    }
    Ok(SetParams {
        buffer_bytes: want.buffer_bytes,
        period_bytes: want.period_bytes,
        features: 0,
        channels: want.channels,
        format: want.format,
        rate: code,
    })
}
