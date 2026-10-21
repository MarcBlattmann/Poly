//! The console host (ADR 0080): Windows consoles as NT objects, backed by
//! Poly's terminals.
//!
//! A console ([`poly_console::Console`]: input buffer, modes, code pages,
//! title, screen buffer) lives here in [`Consoles`], inside the file I/O
//! state ([`crate::io::IoState`]), because its handles are file objects:
//!
//! | File object | Opened by | Is |
//! |---|---|---|
//! | connection | `\Device\PolyConsole\New`, `\<id>`, `\Process\<pid>` | the process's attachment (its `ConsoleHandle`) |
//! | input | `Input` relative to a console handle (`CONIN$`) | the input buffer |
//! | output | `Output` relative to a console handle (`CONOUT$`) | the screen buffer |
//!
//! (names: [`poly_console::ioctl`]). `NtReadFile` on an input handle is a
//! read in the input code page (cooked or not, as the mode says);
//! `NtWriteFile` on an output handle writes in the output code page;
//! `NtQueryVolumeInformationFile` reports `FILE_DEVICE_CONSOLE`, so
//! `GetFileType` says `FILE_TYPE_CHAR`; everything else is
//! `NtDeviceIoControlFile` with Poly's codes ([`services`]). A read that
//! must wait for input is an I/O request packet parked on the console
//! ([`Wait`]) and finished when input comes ([`IoState::console_input`]).
//!
//! The kernel connects consoles to terminals: it feeds a console its
//! terminal's bytes, sends the terminal what [`IoState::take_console_output`]
//! gives, and acts on [`ConsoleEvent`]s: a console made by `AllocConsole`
//! needs a terminal, a closed one gives its terminal back, a control event
//! starts a thread at the routine `kernelbase` registered in each attached
//! process (Learn: "HandlerRoutine": "the system creates a new thread in
//! the process to execute the function").
//!
//! A process is attached while it holds a connection; a console goes away
//! when its last connection closes (Learn: "FreeConsole": "the console is
//! closed when the last process attached to it terminates or calls
//! FreeConsole"). Its other handles then fail with `STATUS_INVALID_HANDLE`.

pub mod services;
#[cfg(test)]
mod tests;

use crate::io::consts::*;
use crate::io::share::ShareRequest;
use crate::io::{Device, FileRecord, IoState, Irp, Target};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::result::Result;
use poly_console::{Console, InputRecord, ReadOutcome, ioctl};
use poly_ntobj::status::*;
use poly_ntobj::{Caller, Handle, NewObject, NtStatus, ObjectId, ObjectManager, ObjectType, ProcessId, ThreadRef};
use poly_vfs::{FileType, OpenFile};

/// What a console file object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleKind {
    Connection,
    Input,
    Output,
}

/// A console file object's target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsoleTarget {
    pub console: u32,
    pub kind: ConsoleKind,
}

/// A request waiting for input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// A text read of up to `max` units (`unicode`) or bytes into `buf`.
    Text { unicode: bool, max: usize, buf: u64 },
    /// `ReadConsoleInputW` of up to `max` records into `buf`.
    Input { max: usize, buf: u64 },
}

/// A process attached through one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attached {
    pub process: ProcessId,
    /// Where `kernelbase` wants control events delivered (0: nowhere).
    pub ctrl_routine: u64,
}

/// One console and who uses it.
#[derive(Debug)]
pub struct ConsoleEntry {
    pub console: Console,
    /// Connections by file object cookie.
    pub connections: BTreeMap<u64, Attached>,
    waiting: VecDeque<(u64, Wait)>,
    /// [`ConsoleEvent::InputWanted`] was raised.
    input_wanted: bool,
}

/// What the kernel must do for the consoles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleEvent {
    /// `AllocConsole` made console `id`: give it a terminal.
    Created { id: u32 },
    /// Console `id` is gone; `output` gives its terminal back.
    Closed { id: u32, output: Vec<u8> },
    /// Console `id` wants its terminal's input from now on (a program
    /// that can read it or take Ctrl+C attached: the kernel puts the
    /// terminal in raw mode and feeds it). Once per console.
    InputWanted { id: u32 },
    /// Start a thread at `routine(event)` in `process`.
    Ctrl { process: ProcessId, routine: u64, event: u32 },
}

/// Every console.
#[derive(Debug, Default)]
pub struct Consoles {
    map: BTreeMap<u32, ConsoleEntry>,
    next_id: u32,
    events: VecDeque<ConsoleEvent>,
}

/// A request that can finish: its status, its information and the bytes
/// for the caller's buffer.
type Finished = (NtStatus, u64, Option<(u64, Vec<u8>)>);

/// The largest single console transfer (a longer `WriteConsole` is cut by
/// `kernelbase` into pieces this big).
pub const MAX_TRANSFER: usize = 64 * 1024;

fn wide_eq(a: &[u16], b: &str) -> bool {
    a.len() == b.len() && a.iter().zip(b.bytes()).all(|(&x, y)| x < 0x80 && (x as u8).eq_ignore_ascii_case(&y))
}

fn parse_u32(s: &[u16]) -> Option<u32> {
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    s.iter().try_fold(0u32, |acc, &c| {
        let d = c.checked_sub(u16::from(b'0')).filter(|&d| d < 10)?;
        acc.checked_mul(10)?.checked_add(u32::from(d))
    })
}

impl IoState {
    /// A new console on a terminal of `width` × `height` (0: unknown).
    /// Its id.
    pub fn create_console(&mut self, width: u16, height: u16) -> u32 {
        let c = &mut self.consoles;
        c.next_id += 1;
        let id = c.next_id;
        let entry = ConsoleEntry {
            console: Console::new(width, height),
            connections: BTreeMap::new(),
            waiting: VecDeque::new(),
            input_wanted: false,
        };
        c.map.insert(id, entry);
        id
    }

    /// Console `id`'s input is needed: raise [`ConsoleEvent::InputWanted`]
    /// the first time.
    pub(crate) fn want_input(&mut self, id: u32) {
        if let Some(e) = self.consoles.map.get_mut(&id)
            && !core::mem::replace(&mut e.input_wanted, true)
        {
            self.consoles.events.push_back(ConsoleEvent::InputWanted { id });
        }
    }

    pub fn console(&self, id: u32) -> Option<&Console> {
        self.consoles.map.get(&id).map(|e| &e.console)
    }

    pub fn console_mut(&mut self, id: u32) -> Option<&mut Console> {
        self.consoles.map.get_mut(&id).map(|e| &mut e.console)
    }

    pub fn console_ids(&self) -> Vec<u32> {
        self.consoles.map.keys().copied().collect()
    }

    /// Give console `id` the size of its terminal (before it drew
    /// anything).
    pub fn resize_console(&mut self, id: u32, width: u16, height: u16) {
        if let Some(c) = self.console_mut(id)
            && !c.screen.taken_over()
        {
            c.screen = poly_console::Screen::new(width, height);
        }
    }

    /// The processes attached to console `id`.
    pub fn console_processes(&self, id: u32) -> Vec<ProcessId> {
        let mut v: Vec<ProcessId> =
            self.consoles.map.get(&id).map(|e| e.connections.values().map(|a| a.process).collect()).unwrap_or_default();
        v.sort_by_key(|p| p.0);
        v.dedup();
        v
    }

    /// Bytes from console `id`'s terminal: decoded into input, control
    /// events raised, waiting reads retried.
    pub fn console_input(&mut self, id: u32, bytes: &[u8]) {
        let Some(e) = self.consoles.map.get_mut(&id) else { return };
        e.console.feed(bytes);
        self.raise_ctrl(id);
        self.retry_console(id);
    }

    /// Records put in console `id`'s input (`WriteConsoleInputW`).
    pub fn console_records(&mut self, id: u32, records: &[InputRecord]) {
        let Some(e) = self.consoles.map.get_mut(&id) else { return };
        e.console.push_records(records);
        self.retry_console(id);
    }

    /// Control events the console raised: one per attached process that
    /// registered a routine.
    pub(crate) fn raise_ctrl(&mut self, id: u32) {
        let Some(e) = self.consoles.map.get_mut(&id) else { return };
        while let Some(event) = e.console.take_ctrl() {
            let mut seen = Vec::new();
            for a in e.connections.values() {
                if a.ctrl_routine != 0 && !seen.contains(&a.process) {
                    seen.push(a.process);
                    self.consoles.events.push_back(ConsoleEvent::Ctrl {
                        process: a.process,
                        routine: a.ctrl_routine,
                        event,
                    });
                }
            }
        }
    }

    /// `GenerateConsoleCtrlEvent` on console `id`.
    pub fn console_generate_ctrl(&mut self, id: u32, event: u32) -> Result<(), NtStatus> {
        let e = self.consoles.map.get_mut(&id).ok_or(STATUS_INVALID_HANDLE)?;
        let mut seen = Vec::new();
        for a in e.connections.values() {
            if a.ctrl_routine != 0 && !seen.contains(&a.process) {
                seen.push(a.process);
                self.consoles.events.push_back(ConsoleEvent::Ctrl {
                    process: a.process,
                    routine: a.ctrl_routine,
                    event,
                });
            }
        }
        Ok(())
    }

    /// What the kernel must do (oldest first).
    pub fn take_console_events(&mut self) -> Vec<ConsoleEvent> {
        self.consoles.events.drain(..).collect()
    }

    /// Terminal bytes per console, since the last call.
    pub fn take_console_output(&mut self) -> Vec<(u32, Vec<u8>)> {
        self.consoles
            .map
            .iter_mut()
            .filter_map(|(&id, e)| {
                let out = e.console.take_output();
                (!out.is_empty()).then_some((id, out))
            })
            .collect()
    }

    /// Try one request against console `id`: `Some` with the status, the
    /// information and the bytes for the caller's buffer once it can
    /// finish.
    fn try_wait(&mut self, id: u32, w: Wait) -> Option<Finished> {
        let e = self.consoles.map.get_mut(&id)?;
        match w {
            Wait::Text { unicode, max, buf } => match e.console.read_text(max, unicode)? {
                ReadOutcome::Units(u) => {
                    let bytes: Vec<u8> = u.iter().flat_map(|c| c.to_le_bytes()).collect();
                    Some((STATUS_SUCCESS, bytes.len() as u64, Some((buf, bytes))))
                }
                ReadOutcome::Bytes(b) => Some((STATUS_SUCCESS, b.len() as u64, Some((buf, b)))),
                ReadOutcome::Aborted => Some((STATUS_CANCELLED, 0, None)),
            },
            Wait::Input { max, buf } => {
                let records = e.console.read_input(max, true);
                if records.is_empty() {
                    return None;
                }
                let bytes: Vec<u8> = records.iter().flat_map(|r| r.to_bytes()).collect();
                Some((STATUS_SUCCESS, bytes.len() as u64, Some((buf, bytes))))
            }
        }
    }

    /// Finish what waits on console `id` and can finish now, in order. A
    /// read that cannot finish keeps the ones behind it waiting (input is
    /// taken in the order reads were made).
    pub(crate) fn retry_console(&mut self, id: u32) {
        loop {
            let Some(e) = self.consoles.map.get_mut(&id) else { return };
            let Some(&(irp, w)) = e.waiting.front() else { return };
            if self.irp(irp).is_none() {
                // Cancelled meanwhile.
                self.consoles.map.get_mut(&id).expect("there").waiting.pop_front();
                continue;
            }
            match self.try_wait(id, w) {
                Some((status, info, data)) => {
                    self.consoles.map.get_mut(&id).expect("there").waiting.pop_front();
                    self.complete(irp, status, info, data);
                }
                None => return,
            }
        }
    }

    /// A read on a console: done at once (its completion queued) or
    /// parked until input comes. Returns the parked request's id.
    pub fn console_read(&mut self, id: u32, mut irp: Irp, w: Wait) -> Result<Option<u64>, NtStatus> {
        self.want_input(id);
        let e = self.consoles.map.get(&id).ok_or(STATUS_INVALID_HANDLE)?;
        if e.waiting.is_empty()
            && let Some((status, info, data)) = self.try_wait(id, w)
        {
            irp.sync = false;
            self.complete_now(irp, status, info, data);
            return Ok(None);
        }
        let parked = self.park(irp);
        self.consoles.map.get_mut(&id).expect("checked").waiting.push_back((parked, w));
        Ok(Some(parked))
    }

    /// Bytes written to a console output handle (`WriteFile`).
    pub fn console_write_bytes(&mut self, id: u32, data: &[u8]) -> Result<u64, NtStatus> {
        let c = self.console_mut(id).ok_or(STATUS_INVALID_HANDLE)?;
        Ok(c.write_bytes(data) as u64)
    }

    /// The console target of file object `cookie`, if it is one whose
    /// console still exists.
    pub fn console_target(&self, cookie: u64) -> Result<ConsoleTarget, NtStatus> {
        match self.file(cookie).map(|f| &f.target) {
            Some(Target::Console(t)) if self.consoles.map.contains_key(&t.console) => Ok(*t),
            Some(Target::Console(_)) => Err(STATUS_INVALID_HANDLE),
            Some(_) => Err(STATUS_INVALID_DEVICE_REQUEST),
            None => Err(STATUS_INVALID_HANDLE),
        }
    }

    /// Set the control routine of connection `cookie`.
    pub fn set_ctrl_routine(&mut self, cookie: u64, routine: u64) -> Result<(), NtStatus> {
        let t = self.console_target(cookie)?;
        let e = self.consoles.map.get_mut(&t.console).ok_or(STATUS_INVALID_HANDLE)?;
        let a = e.connections.get_mut(&cookie).ok_or(STATUS_INVALID_HANDLE)?;
        a.ctrl_routine = routine;
        // Ctrl+C must reach it even when it never reads.
        self.want_input(t.console);
        Ok(())
    }

    /// Open below `\Device\PolyConsole` (`remaining`) or relative to a
    /// console file object (`relative`): the new file object.
    pub(crate) fn open_console(
        &mut self,
        om: &mut ObjectManager,
        caller: Option<&Caller>,
        thread: ThreadRef,
        params: &crate::io::create::CreateParams,
        relative: Option<ConsoleTarget>,
        remaining: &[u16],
    ) -> Result<(ObjectId, u64), NtStatus> {
        let name: &[u16] = match remaining.first() {
            Some(&c) if c == u16::from(b'\\') => &remaining[1..],
            _ => remaining,
        };
        let parts: Vec<&[u16]> = name.split(|&c| c == u16::from(b'\\')).collect();
        let (console, kind) = match (relative, parts.as_slice()) {
            (Some(t), [n]) if wide_eq(n, ioctl::INPUT) => (t.console, ConsoleKind::Input),
            (Some(t), [n]) if wide_eq(n, ioctl::OUTPUT) => (t.console, ConsoleKind::Output),
            (Some(_), _) => return Err(STATUS_OBJECT_NAME_NOT_FOUND),
            (None, [n]) if wide_eq(n, ioctl::NEW) => {
                let id = self.create_console(0, 0);
                self.consoles.events.push_back(ConsoleEvent::Created { id });
                (id, ConsoleKind::Connection)
            }
            (None, [p, pid]) if wide_eq(p, ioctl::PROCESS) => {
                let pid = parse_u32(pid).ok_or(STATUS_OBJECT_NAME_INVALID)?;
                let id = self
                    .consoles
                    .map
                    .iter()
                    .find(|(_, e)| e.connections.values().any(|a| a.process.0 == pid))
                    .map(|(&id, _)| id)
                    .ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
                (id, ConsoleKind::Connection)
            }
            (None, [n]) => {
                let id = parse_u32(n).ok_or(STATUS_OBJECT_NAME_INVALID)?;
                if !self.consoles.map.contains_key(&id) {
                    return Err(STATUS_OBJECT_NAME_NOT_FOUND);
                }
                (id, ConsoleKind::Connection)
            }
            _ => return Err(STATUS_OBJECT_NAME_NOT_FOUND),
        };
        if !self.consoles.map.contains_key(&console) {
            return Err(STATUS_INVALID_HANDLE);
        }
        let cookie = self.cookie();
        let object = om.new_object(
            NewObject::Opaque { kind: ObjectType::File, cookie, signaled: Some(false), parse: true },
            thread,
        )?;
        if kind == ConsoleKind::Connection {
            let process = caller.map_or(ProcessId(0), |c| c.process);
            self.consoles
                .map
                .get_mut(&console)
                .expect("checked")
                .connections
                .insert(cookie, Attached { process, ctrl_routine: 0 });
        }
        let rec = FileRecord {
            object,
            target: Target::Console(ConsoleTarget { console, kind }),
            kind: FileType::CharDevice,
            access: params.access,
            share: ShareRequest::new(params.access, params.share),
            options: params.options,
            position: 0,
            delete_on_close: false,
            scan: None,
        };
        self.files.insert(cookie, rec);
        Ok((object, cookie))
    }

    /// File object `cookie` on a console went: a connection detaches its
    /// process, and the last one closes the console.
    pub(crate) fn console_file_closed(&mut self, cookie: u64, t: ConsoleTarget) {
        if t.kind != ConsoleKind::Connection {
            return;
        }
        let Some(e) = self.consoles.map.get_mut(&t.console) else { return };
        e.connections.remove(&cookie);
        if !e.connections.is_empty() {
            return;
        }
        let mut e = self.consoles.map.remove(&t.console).expect("there");
        e.console.close();
        let output = e.console.take_output();
        // Reads still waiting (another process's handle) see the end.
        for (irp, _) in e.waiting {
            self.complete(irp, STATUS_END_OF_FILE, 0, None);
        }
        self.consoles.events.push_back(ConsoleEvent::Closed { id: t.console, output });
    }

    /// A file object for an open file inherited from Poly's side (a
    /// redirected standard handle: a file, a pipe, a device), with a handle
    /// in `caller`'s table.
    pub fn open_host_file(
        &mut self,
        om: &mut ObjectManager,
        caller: &Caller,
        open: Arc<OpenFile>,
        access: u32,
    ) -> Result<Handle, NtStatus> {
        let cookie = self.cookie();
        let kind = open.kind();
        let object = om.new_object(
            NewObject::Opaque { kind: ObjectType::File, cookie, signaled: Some(false), parse: false },
            caller.thread,
        )?;
        let rec = FileRecord {
            object,
            target: Target::Host(open),
            kind,
            access,
            share: ShareRequest::new(access, FILE_SHARE_READ | FILE_SHARE_WRITE),
            options: FILE_SYNCHRONOUS_IO_NONALERT,
            position: 0,
            delete_on_close: false,
            scan: None,
        };
        self.files.insert(cookie, rec);
        let h = om.open_by_pointer(caller, object, access, 0);
        let _ = om.dereference(object);
        h
    }

    /// `\Device\PolyConsole`'s cookie is registered at attach.
    pub(crate) fn add_console_device(&mut self, cookie: u64) {
        self.devices.insert(cookie, Device::Console);
    }
}

/// Open console `id` for `process` the way a console program starts on it
/// (Learn: "Creation of a Console", "Console Handles"): a connection and
/// handles to its input buffer and screen buffer (`GENERIC_READ |
/// GENERIC_WRITE`, synchronous), in `caller`'s handle table. Returns
/// `[connection, input, output, error]` (output and error are two handles
/// to the same screen buffer).
pub fn open_for_process(
    sys: &mut crate::system::NtSystem,
    b: &mut dyn crate::backend::Backend,
    caller: &Caller,
    id: u32,
) -> Result<[Handle; 4], NtStatus> {
    let mut open = |root: Option<Handle>, name: &str| -> Result<Handle, NtStatus> {
        let attrs = crate::abi::Attributes {
            root,
            name: Some(poly_ntobj::name::wide(name)),
            attributes: poly_ntobj::OBJ_CASE_INSENSITIVE,
        };
        let access = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE;
        let params = crate::io::services::check_create(access, share, FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT, 0)?;
        crate::io::services::open_in(sys, b, caller, &attrs, access, params).map(|(h, _, _)| h)
    };
    let conn = open(None, &alloc::format!("{}\\{id}", ioctl::DEVICE))?;
    let input = open(Some(conn), ioctl::INPUT)?;
    let output = open(Some(conn), ioctl::OUTPUT)?;
    let error = open(Some(conn), ioctl::OUTPUT)?;
    Ok([conn, input, output, error])
}
