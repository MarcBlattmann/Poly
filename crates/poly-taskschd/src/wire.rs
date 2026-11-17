//! The messages between `taskschd.dll` and `schedsvc.exe` (ADR 0226, section 4): one request and
//! one reply per connection on the pipe [`PIPE`], each framed as a `u32` length (little endian)
//! and the payload (the framing of ADR 0220).
//!
//! Folder and task objects name themselves by the path text they were asked with; the scheduler
//! looks them up again on every request.

use crate::def::*;
use alloc::string::String;
use alloc::vec::Vec;

/// The scheduler's pipe.
pub const PIPE: &str = r"\\.\pipe\poly-taskschd";
/// The largest payload accepted (a definition with many triggers fits easily).
pub const MAX_FRAME: usize = 4 << 20;

/// The 4-byte length that precedes a payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 4);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn frame_len(head: [u8; 4]) -> Option<usize> {
    let n = u32::from_le_bytes(head) as usize;
    (n <= MAX_FRAME).then_some(n)
}

#[derive(Debug, Default)]
pub struct Enc(pub Vec<u8>);

impl Enc {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    pub fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    pub fn opt(&mut self, s: &Option<String>) {
        match s {
            Some(s) => {
                self.u8(1);
                self.str(s);
            }
            None => self.u8(0),
        }
    }
    pub fn strs(&mut self, v: &[String]) {
        self.u32(v.len() as u32);
        for s in v {
            self.str(s);
        }
    }
}

#[derive(Debug)]
pub struct Dec<'a> {
    b: &'a [u8],
    at: usize,
}

pub type D<T> = Result<T, ()>;

impl<'a> Dec<'a> {
    pub fn new(b: &'a [u8]) -> Dec<'a> {
        Dec { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> D<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or(())?;
        let s = self.b.get(self.at..end).ok_or(())?;
        self.at = end;
        Ok(s)
    }
    pub fn u8(&mut self) -> D<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn i16(&mut self) -> D<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().map_err(|_| ())?))
    }
    pub fn i32(&mut self) -> D<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?))
    }
    pub fn u32(&mut self) -> D<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| ())?))
    }
    pub fn i64(&mut self) -> D<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().map_err(|_| ())?))
    }
    pub fn f64(&mut self) -> D<f64> {
        Ok(f64::from_bits(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| ())?)))
    }
    pub fn bool(&mut self) -> D<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn str(&mut self) -> D<String> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        Ok(String::from(core::str::from_utf8(s).map_err(|_| ())?))
    }
    pub fn opt(&mut self) -> D<Option<String>> {
        Ok(if self.u8()? != 0 { Some(self.str()?) } else { None })
    }
    pub fn strs(&mut self) -> D<Vec<String>> {
        let n = self.u32()? as usize;
        let mut v = Vec::new();
        for _ in 0..n.min(1 << 16) {
            v.push(self.str()?);
        }
        Ok(v)
    }
}

/// A definition in full (every field, so nothing is lost between the processes).
pub fn put_definition(e: &mut Enc, d: &Definition) {
    let r = &d.reg;
    for t in [&r.description, &r.author, &r.version, &r.date, &r.documentation, &r.uri, &r.source, &r.security_descriptor] {
        e.opt(t);
    }
    let p = &d.principal;
    for t in [&p.id, &p.display_name, &p.user_id, &p.group_id] {
        e.opt(t);
    }
    e.i32(p.logon_type);
    e.i32(p.run_level);
    e.i32(p.process_token_sid_type);
    let s = &d.settings;
    e.i16(s.allow_demand_start);
    e.opt(&s.restart_interval);
    e.i32(s.restart_count);
    e.i32(s.multiple_instances);
    e.i16(s.stop_if_going_on_batteries);
    e.i16(s.disallow_start_if_on_batteries);
    e.i16(s.allow_hard_terminate);
    e.i16(s.start_when_available);
    e.i16(s.run_only_if_network_available);
    e.opt(&s.execution_time_limit);
    e.i16(s.enabled);
    e.opt(&s.delete_expired_task_after);
    e.i32(s.priority);
    e.i32(s.compatibility);
    e.i16(s.hidden);
    e.opt(&s.idle.duration);
    e.opt(&s.idle.wait_timeout);
    e.i16(s.idle.stop_on_idle_end);
    e.i16(s.idle.restart_on_idle);
    e.i16(s.run_only_if_idle);
    e.i16(s.wake_to_run);
    e.opt(&s.network.name);
    e.opt(&s.network.id);
    e.i16(s.disallow_start_on_remote_app_session);
    e.i16(s.use_unified_scheduling_engine);
    e.i16(s.volatile);
    e.u32(d.triggers.len() as u32);
    for t in &d.triggers {
        e.i32(t.kind);
        for x in [&t.id, &t.start_boundary, &t.end_boundary, &t.execution_time_limit] {
            e.opt(x);
        }
        e.i16(t.enabled);
        e.opt(&t.repetition.interval);
        e.opt(&t.repetition.duration);
        e.i16(t.repetition.stop_at_duration_end);
        e.i16(t.days_interval);
        e.i16(t.weeks_interval);
        e.i16(t.days_of_week);
        e.i32(t.days_of_month);
        e.i16(t.months_of_year);
        e.i16(t.run_on_last_day_of_month);
        e.i16(t.weeks_of_month);
        e.i16(t.run_on_last_week_of_month);
        for x in [&t.random_delay, &t.delay, &t.user_id, &t.subscription] {
            e.opt(x);
        }
        e.i32(t.state_change);
    }
    e.u32(d.actions.len() as u32);
    for a in &d.actions {
        e.i32(a.kind);
        e.i16(a.hide_app_window);
        for x in [
            &a.id,
            &a.path,
            &a.arguments,
            &a.working_directory,
            &a.class_id,
            &a.data,
            &a.server,
            &a.subject,
            &a.to,
            &a.cc,
            &a.bcc,
            &a.reply_to,
            &a.from,
            &a.body,
            &a.title,
            &a.message_body,
        ] {
            e.opt(x);
        }
    }
    e.opt(&d.context);
    e.opt(&d.data);
}

pub fn get_definition(x: &mut Dec) -> D<Definition> {
    let mut d = Definition::default();
    let r = &mut d.reg;
    for t in [
        &mut r.description,
        &mut r.author,
        &mut r.version,
        &mut r.date,
        &mut r.documentation,
        &mut r.uri,
        &mut r.source,
        &mut r.security_descriptor,
    ] {
        *t = x.opt()?;
    }
    let p = &mut d.principal;
    for t in [&mut p.id, &mut p.display_name, &mut p.user_id, &mut p.group_id] {
        *t = x.opt()?;
    }
    p.logon_type = x.i32()?;
    p.run_level = x.i32()?;
    p.process_token_sid_type = x.i32()?;
    let s = &mut d.settings;
    s.allow_demand_start = x.i16()?;
    s.restart_interval = x.opt()?;
    s.restart_count = x.i32()?;
    s.multiple_instances = x.i32()?;
    s.stop_if_going_on_batteries = x.i16()?;
    s.disallow_start_if_on_batteries = x.i16()?;
    s.allow_hard_terminate = x.i16()?;
    s.start_when_available = x.i16()?;
    s.run_only_if_network_available = x.i16()?;
    s.execution_time_limit = x.opt()?;
    s.enabled = x.i16()?;
    s.delete_expired_task_after = x.opt()?;
    s.priority = x.i32()?;
    s.compatibility = x.i32()?;
    s.hidden = x.i16()?;
    s.idle.duration = x.opt()?;
    s.idle.wait_timeout = x.opt()?;
    s.idle.stop_on_idle_end = x.i16()?;
    s.idle.restart_on_idle = x.i16()?;
    s.run_only_if_idle = x.i16()?;
    s.wake_to_run = x.i16()?;
    s.network.name = x.opt()?;
    s.network.id = x.opt()?;
    s.disallow_start_on_remote_app_session = x.i16()?;
    s.use_unified_scheduling_engine = x.i16()?;
    s.volatile = x.i16()?;
    let n = x.u32()? as usize;
    for _ in 0..n.min(4096) {
        let mut t = Trigger::new(x.i32()?);
        for f in [&mut t.id, &mut t.start_boundary, &mut t.end_boundary, &mut t.execution_time_limit] {
            *f = x.opt()?;
        }
        t.enabled = x.i16()?;
        t.repetition.interval = x.opt()?;
        t.repetition.duration = x.opt()?;
        t.repetition.stop_at_duration_end = x.i16()?;
        t.days_interval = x.i16()?;
        t.weeks_interval = x.i16()?;
        t.days_of_week = x.i16()?;
        t.days_of_month = x.i32()?;
        t.months_of_year = x.i16()?;
        t.run_on_last_day_of_month = x.i16()?;
        t.weeks_of_month = x.i16()?;
        t.run_on_last_week_of_month = x.i16()?;
        for f in [&mut t.random_delay, &mut t.delay, &mut t.user_id, &mut t.subscription] {
            *f = x.opt()?;
        }
        t.state_change = x.i32()?;
        d.triggers.push(t);
    }
    let n = x.u32()? as usize;
    for _ in 0..n.min(4096) {
        let mut a = Action::new(x.i32()?);
        a.hide_app_window = x.i16()?;
        for f in [
            &mut a.id,
            &mut a.path,
            &mut a.arguments,
            &mut a.working_directory,
            &mut a.class_id,
            &mut a.data,
            &mut a.server,
            &mut a.subject,
            &mut a.to,
            &mut a.cc,
            &mut a.bcc,
            &mut a.reply_to,
            &mut a.from,
            &mut a.body,
            &mut a.title,
            &mut a.message_body,
        ] {
            *f = x.opt()?;
        }
        d.actions.push(a);
    }
    d.context = x.opt()?;
    d.data = x.opt()?;
    Ok(d)
}

/// A request to the scheduler. `folder` is a folder object's path text, `path` a task object's.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Ping,
    GetFolder { folder: String, rel: String },
    CreateFolder { folder: String, rel: Option<String>, sddl: Option<String> },
    DeleteFolder { folder: String, rel: Option<String> },
    Folders { folder: String, flags: i32 },
    Tasks { folder: String, flags: i32 },
    GetTask { folder: String, rel: String },
    DeleteTask { folder: String, rel: Option<String>, flags: i32 },
    Register {
        folder: String,
        def: Definition,
        path: Option<String>,
        flags: i32,
        user: Option<String>,
        logon_type: i32,
        sddl: Option<String>,
        from_xml: bool,
        caller_name: String,
        caller_sid: String,
    },
    Info { path: String },
    SetEnabled { path: String, on: bool },
    Xml { path: String },
    Definition { path: String },
    Run { path: String },
    Stop { path: String, flags: i32 },
    Instances { path: String },
    Running { flags: i32 },
    StopInstance { guid: String },
    SecurityDescriptor { path: String },
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::default();
        match self {
            Request::Ping => e.u8(1),
            Request::GetFolder { folder, rel } => {
                e.u8(2);
                e.str(folder);
                e.str(rel);
            }
            Request::CreateFolder { folder, rel, sddl } => {
                e.u8(3);
                e.str(folder);
                e.opt(rel);
                e.opt(sddl);
            }
            Request::DeleteFolder { folder, rel } => {
                e.u8(4);
                e.str(folder);
                e.opt(rel);
            }
            Request::Folders { folder, flags } => {
                e.u8(5);
                e.str(folder);
                e.i32(*flags);
            }
            Request::Tasks { folder, flags } => {
                e.u8(6);
                e.str(folder);
                e.i32(*flags);
            }
            Request::GetTask { folder, rel } => {
                e.u8(7);
                e.str(folder);
                e.str(rel);
            }
            Request::DeleteTask { folder, rel, flags } => {
                e.u8(8);
                e.str(folder);
                e.opt(rel);
                e.i32(*flags);
            }
            Request::Register { folder, def, path, flags, user, logon_type, sddl, from_xml, caller_name, caller_sid } => {
                e.u8(9);
                e.str(folder);
                put_definition(&mut e, def);
                e.opt(path);
                e.i32(*flags);
                e.opt(user);
                e.i32(*logon_type);
                e.opt(sddl);
                e.bool(*from_xml);
                e.str(caller_name);
                e.str(caller_sid);
            }
            Request::Info { path } => {
                e.u8(10);
                e.str(path);
            }
            Request::SetEnabled { path, on } => {
                e.u8(11);
                e.str(path);
                e.bool(*on);
            }
            Request::Xml { path } => {
                e.u8(12);
                e.str(path);
            }
            Request::Definition { path } => {
                e.u8(13);
                e.str(path);
            }
            Request::Run { path } => {
                e.u8(14);
                e.str(path);
            }
            Request::Stop { path, flags } => {
                e.u8(15);
                e.str(path);
                e.i32(*flags);
            }
            Request::Instances { path } => {
                e.u8(16);
                e.str(path);
            }
            Request::Running { flags } => {
                e.u8(17);
                e.i32(*flags);
            }
            Request::StopInstance { guid } => {
                e.u8(18);
                e.str(guid);
            }
            Request::SecurityDescriptor { path } => {
                e.u8(19);
                e.str(path);
            }
        }
        e.0
    }

    pub fn decode(b: &[u8]) -> D<Request> {
        let mut x = Dec::new(b);
        Ok(match x.u8()? {
            1 => Request::Ping,
            2 => Request::GetFolder { folder: x.str()?, rel: x.str()? },
            3 => Request::CreateFolder { folder: x.str()?, rel: x.opt()?, sddl: x.opt()? },
            4 => Request::DeleteFolder { folder: x.str()?, rel: x.opt()? },
            5 => Request::Folders { folder: x.str()?, flags: x.i32()? },
            6 => Request::Tasks { folder: x.str()?, flags: x.i32()? },
            7 => Request::GetTask { folder: x.str()?, rel: x.str()? },
            8 => Request::DeleteTask { folder: x.str()?, rel: x.opt()?, flags: x.i32()? },
            9 => Request::Register {
                folder: x.str()?,
                def: get_definition(&mut x)?,
                path: x.opt()?,
                flags: x.i32()?,
                user: x.opt()?,
                logon_type: x.i32()?,
                sddl: x.opt()?,
                from_xml: x.bool()?,
                caller_name: x.str()?,
                caller_sid: x.str()?,
            },
            10 => Request::Info { path: x.str()? },
            11 => Request::SetEnabled { path: x.str()?, on: x.bool()? },
            12 => Request::Xml { path: x.str()? },
            13 => Request::Definition { path: x.str()? },
            14 => Request::Run { path: x.str()? },
            15 => Request::Stop { path: x.str()?, flags: x.i32()? },
            16 => Request::Instances { path: x.str()? },
            17 => Request::Running { flags: x.i32()? },
            18 => Request::StopInstance { guid: x.str()? },
            19 => Request::SecurityDescriptor { path: x.str()? },
            _ => return Err(()),
        })
    }
}

/// What `IRegisteredTask` reads in one request.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Info {
    pub state: i32,
    pub enabled: bool,
    /// `DATE`s.
    pub last_run: f64,
    pub next_run: f64,
    pub last_result: i32,
    pub missed: i32,
}

/// A running instance as `IRunningTask` shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstanceInfo {
    /// The task's canonical path and name.
    pub path: String,
    pub name: String,
    pub guid: String,
    pub pid: u32,
    pub action: String,
    pub state: i32,
}

/// The reply: the result code and whatever the request asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub hr: i32,
    pub names: Vec<String>,
    pub text: String,
    pub info: Option<Info>,
    pub def: Option<Definition>,
    pub instances: Vec<InstanceInfo>,
}

impl Reply {
    pub fn hr(hr: i32) -> Reply {
        Reply { hr, ..Reply::default() }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::default();
        e.i32(self.hr);
        e.strs(&self.names);
        e.str(&self.text);
        match &self.info {
            Some(i) => {
                e.u8(1);
                e.i32(i.state);
                e.bool(i.enabled);
                e.f64(i.last_run);
                e.f64(i.next_run);
                e.i32(i.last_result);
                e.i32(i.missed);
            }
            None => e.u8(0),
        }
        match &self.def {
            Some(d) => {
                e.u8(1);
                put_definition(&mut e, d);
            }
            None => e.u8(0),
        }
        e.u32(self.instances.len() as u32);
        for i in &self.instances {
            e.str(&i.path);
            e.str(&i.name);
            e.str(&i.guid);
            e.u32(i.pid);
            e.str(&i.action);
            e.i32(i.state);
        }
        e.0
    }

    pub fn decode(b: &[u8]) -> D<Reply> {
        let mut x = Dec::new(b);
        let hr = x.i32()?;
        let names = x.strs()?;
        let text = x.str()?;
        let info = if x.u8()? != 0 {
            Some(Info {
                state: x.i32()?,
                enabled: x.bool()?,
                last_run: x.f64()?,
                next_run: x.f64()?,
                last_result: x.i32()?,
                missed: x.i32()?,
            })
        } else {
            None
        };
        let def = if x.u8()? != 0 { Some(get_definition(&mut x)?) } else { None };
        let n = x.u32()? as usize;
        let mut instances = Vec::new();
        for _ in 0..n.min(1 << 16) {
            instances.push(InstanceInfo {
                path: x.str()?,
                name: x.str()?,
                guid: x.str()?,
                pid: x.u32()?,
                action: x.str()?,
                state: x.i32()?,
            });
        }
        Ok(Reply { hr, names, text, info, def, instances })
    }
}
