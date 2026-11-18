//! The messages between `advapi32` and the manager (ADR 0221).
//!
//! On Windows `advapi32` talks to `services.exe` through RPC. On Poly the manager lives in the
//! NT personality and `advapi32` reaches it through one Poly system call,
//! `PolyServiceCall(Operation, In, InLength, Out, OutLength, ReturnLength*)`: `In` is a request
//! and `Out` receives the reply, both in the little-endian encoding of [`Writer`] and [`Reader`],
//! the same for 32-bit and 64-bit callers. Every reply starts with a Win32 error code (0 for
//! success); the rest of it is there only on success. A reply that does not fit gives
//! `STATUS_BUFFER_TOO_SMALL` and the size it needs.
//!
//! Handles are numbers the manager hands out (manager and service handles, and the event handles
//! of the waits it asks for are real NT handles in the caller's table).

use alloc::vec::Vec;

/// The operations ([`PolyServiceCall`'s first argument).
pub mod op {
    /// in: access → handle.
    pub const OPEN_MANAGER: u32 = 1;
    /// in: manager, name?, display?, access, type, start, error, path?, group?, deps?, account?,
    /// password?, wow64 → handle.
    pub const CREATE: u32 = 2;
    /// in: manager, name?, access → handle.
    pub const OPEN_SERVICE: u32 = 3;
    /// in: handle.
    pub const CLOSE: u32 = 4;
    /// in: service.
    pub const DELETE: u32 = 5;
    /// in: service, type, start, error, path?, group?, deps?, account?, password?, display?.
    pub const CHANGE: u32 = 6;
    /// in: service, level, then the level's data (see [`super::Config2`]).
    pub const CHANGE2: u32 = 7;
    /// in: service → [`super::ConfigReply`].
    pub const QUERY_CONFIG: u32 = 8;
    /// in: service, level → the level's data.
    pub const QUERY_CONFIG2: u32 = 9;
    /// in: service → [`super::Status`].
    pub const QUERY_STATUS: u32 = 10;
    /// in: service, args → event handle to wait on, then [`START_RESULT`].
    pub const START: u32 = 11;
    /// in: service → the start's result.
    pub const START_RESULT: u32 = 12;
    /// in: service, control → pending (0: done, 1: wait on the event handle that follows, then
    /// [`CONTROL_RESULT`]), [`super::Status`] when done.
    pub const CONTROL: u32 = 13;
    /// in: service → [`super::Status`] after the control.
    pub const CONTROL_RESULT: u32 = 14;
    /// in: manager, type, state, group? → count, then name, display, [`super::Status`] each.
    pub const ENUM: u32 = 15;
    /// in: handle, information → self-relative descriptor bytes.
    pub const QUERY_SECURITY: u32 = 16;
    /// in: handle, information, descriptor bytes.
    pub const SET_SECURITY: u32 = 17;
    /// The service process's dispatcher connects → name, args, event handle.
    pub const CONNECT: u32 = 32;
    /// → kind (0 nothing yet, 1 a control follows: code, event type; 2 the service stopped).
    pub const NEXT_CONTROL: u32 = 33;
    /// in: code, handler's return value.
    pub const CONTROL_DONE: u32 = 34;
    /// in: [`super::Status`] fields type, state, accepted, win32, specific, checkpoint, hint.
    pub const SET_STATUS: u32 = 35;
    /// Whether the calling process is a connected service process → 0 or 1.
    pub const IS_SERVICE: u32 = 36;
    /// `services.exe`'s loop: do what is due → event handle, milliseconds to wait (u64::MAX: no
    /// limit).
    pub const HOST: u32 = 48;
}

/// Writes a message.
#[derive(Clone, Debug, Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn new() -> Writer {
        Writer(Vec::new())
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// A string: its length in units, then the units.
    pub fn units(&mut self, s: &[u16]) -> &mut Self {
        self.u32(s.len() as u32);
        for &c in s {
            self.0.extend_from_slice(&c.to_le_bytes());
        }
        self
    }

    /// A string or NULL (length `u32::MAX`).
    pub fn opt(&mut self, s: Option<&[u16]>) -> &mut Self {
        match s {
            Some(s) => self.units(s),
            None => self.u32(u32::MAX),
        }
    }

    /// A list of strings: the count, then each.
    pub fn list(&mut self, l: &[Vec<u16>]) -> &mut Self {
        self.u32(l.len() as u32);
        for s in l {
            self.units(s);
        }
        self
    }

    /// A list or NULL (count `u32::MAX`).
    pub fn opt_list(&mut self, l: Option<&[Vec<u16>]>) -> &mut Self {
        match l {
            Some(l) => self.list(l),
            None => self.u32(u32::MAX),
        }
    }

    /// Bytes: their count, then them.
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
        self
    }

    pub fn status(&mut self, s: &Status) -> &mut Self {
        for v in [s.ty, s.state, s.accepted, s.win32, s.specific, s.checkpoint, s.hint, s.pid, s.flags] {
            self.u32(v);
        }
        self
    }
}

/// Reads a message; every read is `None` past the end.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Reader<'a> {
        Reader { b, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|s| {
            let mut a = [0u8; 8];
            a.copy_from_slice(s);
            u64::from_le_bytes(a)
        })
    }

    pub fn units(&mut self) -> Option<Vec<u16>> {
        let n = self.u32()? as usize;
        self.units_of(n)
    }

    fn units_of(&mut self, n: usize) -> Option<Vec<u16>> {
        let s = self.take(n.checked_mul(2)?)?;
        Some(s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }

    pub fn opt(&mut self) -> Option<Option<Vec<u16>>> {
        match self.u32()? {
            u32::MAX => Some(None),
            n => self.units_of(n as usize).map(Some),
        }
    }

    pub fn list(&mut self) -> Option<Vec<Vec<u16>>> {
        let n = self.u32()?;
        self.list_of(n)
    }

    fn list_of(&mut self, n: u32) -> Option<Vec<Vec<u16>>> {
        if n as usize > self.b.len() {
            return None;
        }
        (0..n).map(|_| self.units()).collect()
    }

    pub fn opt_list(&mut self) -> Option<Option<Vec<Vec<u16>>>> {
        match self.u32()? {
            u32::MAX => Some(None),
            n => self.list_of(n).map(Some),
        }
    }

    pub fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        self.take(n).map(<[u8]>::to_vec)
    }

    pub fn status(&mut self) -> Option<Status> {
        Some(Status {
            ty: self.u32()?,
            state: self.u32()?,
            accepted: self.u32()?,
            win32: self.u32()?,
            specific: self.u32()?,
            checkpoint: self.u32()?,
            hint: self.u32()?,
            pid: self.u32()?,
            flags: self.u32()?,
        })
    }
}

/// `SERVICE_STATUS_PROCESS` (`winsvc.h`): the first seven fields are `SERVICE_STATUS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub ty: u32,
    pub state: u32,
    pub accepted: u32,
    pub win32: u32,
    pub specific: u32,
    pub checkpoint: u32,
    pub hint: u32,
    pub pid: u32,
    pub flags: u32,
}

impl Status {
    /// The 36 bytes of `SERVICE_STATUS_PROCESS`.
    pub fn process_bytes(&self) -> [u8; 36] {
        let mut out = [0u8; 36];
        for (i, v) in [
            self.ty,
            self.state,
            self.accepted,
            self.win32,
            self.specific,
            self.checkpoint,
            self.hint,
            self.pid,
            self.flags,
        ]
        .into_iter()
        .enumerate()
        {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// The reply to [`op::QUERY_CONFIG`] (`QUERY_SERVICE_CONFIGW` without its layout).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigReply {
    pub ty: u32,
    pub start: u32,
    pub error: u32,
    /// The path with its environment variables expanded.
    pub path: Vec<u16>,
    pub group: Vec<u16>,
    pub tag: u32,
    pub deps: Vec<Vec<u16>>,
    pub account: Vec<u16>,
    pub display: Vec<u16>,
}

impl ConfigReply {
    pub fn write(&self, w: &mut Writer) {
        w.u32(self.ty).u32(self.start).u32(self.error).units(&self.path).units(&self.group).u32(self.tag);
        w.list(&self.deps).units(&self.account).units(&self.display);
    }

    pub fn read(r: &mut Reader<'_>) -> Option<ConfigReply> {
        Some(ConfigReply {
            ty: r.u32()?,
            start: r.u32()?,
            error: r.u32()?,
            path: r.units()?,
            group: r.units()?,
            tag: r.u32()?,
            deps: r.list()?,
            account: r.units()?,
            display: r.units()?,
        })
    }
}

/// The failure actions of `ChangeServiceConfig2W` level 2.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailureActions {
    /// Seconds without failure after which the count starts again (`INFINITE`: never).
    pub reset: u32,
    pub reboot_msg: Option<Vec<u16>>,
    pub command: Option<Vec<u16>>,
    /// `(type, delay in ms)`.
    pub actions: Vec<(u32, u32)>,
}

impl FailureActions {
    pub fn write(&self, w: &mut Writer) {
        w.u32(self.reset).opt(self.reboot_msg.as_deref()).opt(self.command.as_deref());
        w.u32(self.actions.len() as u32);
        for &(t, d) in &self.actions {
            w.u32(t).u32(d);
        }
    }

    pub fn read(r: &mut Reader<'_>) -> Option<FailureActions> {
        let reset = r.u32()?;
        let reboot_msg = r.opt()?;
        let command = r.opt()?;
        let n = r.u32()?;
        if n > 1024 {
            return None;
        }
        let actions = (0..n).map(|_| Some((r.u32()?, r.u32()?))).collect::<Option<Vec<_>>>()?;
        Some(FailureActions { reset, reboot_msg, command, actions })
    }
}

/// What `ChangeServiceConfig2W` sets, by level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Config2 {
    /// Level 1: the description (`None`: a NULL text).
    Description(Option<Vec<u16>>),
    /// Level 2: the reset period and messages; the actions only when the caller gave a list
    /// (`lpsaActions` not NULL).
    Failure { reset: u32, reboot_msg: Option<Vec<u16>>, command: Option<Vec<u16>>, actions: Option<Vec<(u32, u32)>> },
    /// Levels 3, 4, 5 and 7: one number.
    Value(u32, u32),
    /// The structure pointer was NULL.
    Nothing,
}

impl Config2 {
    /// The level's data after the level number.
    pub fn write(&self, w: &mut Writer) {
        match self {
            Config2::Nothing => {
                w.u32(0);
            }
            Config2::Description(d) => {
                w.u32(1).opt(d.as_deref());
            }
            Config2::Failure { reset, reboot_msg, command, actions } => {
                w.u32(2).u32(*reset).opt(reboot_msg.as_deref()).opt(command.as_deref());
                match actions {
                    None => {
                        w.u32(u32::MAX);
                    }
                    Some(a) => {
                        w.u32(a.len() as u32);
                        for &(t, d) in a {
                            w.u32(t).u32(d);
                        }
                    }
                }
            }
            Config2::Value(_, v) => {
                w.u32(3).u32(*v);
            }
        }
    }

    /// Read what [`Config2::write`] wrote for `level`.
    pub fn read(level: u32, r: &mut Reader<'_>) -> Option<Config2> {
        Some(match r.u32()? {
            0 => Config2::Nothing,
            1 => Config2::Description(r.opt()?),
            2 => {
                let reset = r.u32()?;
                let reboot_msg = r.opt()?;
                let command = r.opt()?;
                let actions = match r.u32()? {
                    u32::MAX => None,
                    n if n <= 1024 => Some((0..n).map(|_| Some((r.u32()?, r.u32()?))).collect::<Option<Vec<_>>>()?),
                    _ => return None,
                };
                Config2::Failure { reset, reboot_msg, command, actions }
            }
            3 => Config2::Value(level, r.u32()?),
            _ => return None,
        })
    }
}

/// A `REG_MULTI_SZ`-style list (`a\0b\0\0`) as its strings; an empty list is one empty string or
/// nothing.
pub fn split_multi(units: &[u16]) -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    for part in units.split(|&c| c == 0) {
        if part.is_empty() {
            break;
        }
        out.push(part.to_vec());
    }
    out
}

/// The strings as a `REG_MULTI_SZ` list with its final NUL (two NULs for an empty list).
pub fn join_multi(list: &[Vec<u16>]) -> Vec<u16> {
    let mut out = Vec::new();
    for s in list {
        out.extend_from_slice(s);
        out.push(0);
    }
    if list.is_empty() {
        out.push(0);
    }
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut w = Writer::new();
        w.u32(7).opt(None).opt(Some(&[65, 66])).list(&[alloc::vec![1], alloc::vec![]]).opt_list(None).bytes(&[9, 9]);
        let s = Status { state: 4, pid: 12, ..Status::default() };
        w.status(&s);
        let mut r = Reader::new(&w.0);
        assert_eq!(r.u32(), Some(7));
        assert_eq!(r.opt(), Some(None));
        assert_eq!(r.opt(), Some(Some(alloc::vec![65, 66])));
        assert_eq!(r.list(), Some(alloc::vec![alloc::vec![1], alloc::vec![]]));
        assert_eq!(r.opt_list(), Some(None));
        assert_eq!(r.bytes(), Some(alloc::vec![9, 9]));
        assert_eq!(r.status(), Some(s));
        assert_eq!(r.u32(), None);
    }

    #[test]
    fn multi_strings() {
        assert_eq!(split_multi(&[65, 0, 66, 0, 0]), alloc::vec![alloc::vec![65], alloc::vec![66]]);
        assert!(split_multi(&[0, 0]).is_empty());
        assert_eq!(join_multi(&[]), alloc::vec![0, 0]);
        assert_eq!(join_multi(&[alloc::vec![65]]), alloc::vec![65, 0, 0]);
    }

    #[test]
    fn config2_round_trips() {
        for c in [
            Config2::Nothing,
            Config2::Description(None),
            Config2::Description(Some(alloc::vec![120])),
            Config2::Failure { reset: 5, reboot_msg: None, command: None, actions: Some(alloc::vec![(1, 100)]) },
            Config2::Failure { reset: 5, reboot_msg: None, command: None, actions: None },
            Config2::Value(4, 1),
        ] {
            let mut w = Writer::new();
            c.write(&mut w);
            assert_eq!(Config2::read(4, &mut Reader::new(&w.0)), Some(c));
        }
    }
}
