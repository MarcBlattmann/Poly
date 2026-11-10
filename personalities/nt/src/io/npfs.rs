//! Named pipes (ADR 0169): the pipe file system behind `\Device\NamedPipe`,
//! reached as `\??\pipe\<name>` (`\\.\pipe\<name>` in Win32).
//!
//! # Model
//!
//! A *pipe* is a name with the properties its first instance gave it (the
//! direction, which is `ShareAccess` as `CreateNamedPipeW` sets it; the
//! instance limit; the quotas and default time-out). It has *instances*,
//! each made by a server's `NtCreateNamedPipeFile` and owned by the server
//! file object that made it. An instance is in one of four states (Learn:
//! "FILE_PIPE_LOCAL_INFORMATION": `NamedPipeState`):
//!
//! | State | Meaning | Next |
//! |---|---|---|
//! | listening | waits for a client | a client open makes it connected |
//! | connected | a client has it open | the client closing makes it closing; `FSCTL_PIPE_DISCONNECT` makes it disconnected |
//! | closing | the client closed, the server has not disconnected | `FSCTL_PIPE_DISCONNECT` |
//! | disconnected | the server disconnected | `FSCTL_PIPE_LISTEN` makes it listening again |
//!
//! A client open fails with `STATUS_PIPE_NOT_AVAILABLE` unless an instance
//! is listening (so a closing or disconnected instance stays taken until
//! the server acts, and a new instance starts listening at once). Each
//! direction of a connection is a queue of data entries ([`Chan`]): a byte
//! pipe is a stream, a message pipe keeps each write as one message. A
//! write that fits the receiving end's quota is copied into the queue and
//! completes at once; one that does not (and every non-empty write of a
//! zero quota) stays pending as an entry that holds its request, and
//! completes when the reader has taken all of it. A read takes what the
//! queue has, or waits for a write to hand it data (`STATUS_PIPE_EMPTY` in
//! no-wait mode). A message read into a buffer that is too short returns
//! the first bytes with `STATUS_BUFFER_OVERFLOW` and leaves the rest of the
//! message queued.
//!
//! Each end (file object) has its own read mode (byte or message) and
//! completion mode (wait or no-wait): [`NpEnd`]; duplicated handles share
//! the file object and so the modes. The end a client holds becomes
//! *detached* when the server disconnects: its reads and writes fail with
//! `STATUS_PIPE_DISCONNECTED` while the instance serves someone else.
//!
//! The statuses of the ends in each state come from the spec sheets
//! (`specs/windows/kernel32.dll/{ConnectNamedPipe,DisconnectNamedPipe,
//! PeekNamedPipe}.yaml`): an instance nobody is connected to fails reads
//! and writes with `STATUS_PIPE_LISTENING`, a disconnected one with
//! `STATUS_PIPE_DISCONNECTED`, a closing one fails a write with
//! `STATUS_PIPE_CLOSING` and a read, once the data is gone, with
//! `STATUS_PIPE_BROKEN`.

#![allow(
    clippy::result_large_err,
    reason = "a refused request is given back with its status so its event reference can go"
)]

use super::consts::*;
use super::irp::{Irp, IrpOp};
use super::{IoState, Target};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_ntobj::status::*;

/// An instance's state (`NamedPipeState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipeState {
    Disconnected,
    Listening,
    Connected,
    Closing,
}

impl PipeState {
    /// `FILE_PIPE_*_STATE` (`ntifs.h`).
    pub fn code(self) -> u32 {
        match self {
            PipeState::Disconnected => FILE_PIPE_DISCONNECTED_STATE,
            PipeState::Listening => FILE_PIPE_LISTENING_STATE,
            PipeState::Connected => FILE_PIPE_CONNECTED_STATE,
            PipeState::Closing => FILE_PIPE_CLOSING_STATE,
        }
    }
}

/// What a file object on a named pipe is: one end of one instance and its
/// modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NpEnd {
    pub instance: u64,
    pub server: bool,
    /// `FILE_PIPE_MESSAGE_MODE`: reads return one message at a time.
    pub message_read: bool,
    /// `FILE_PIPE_COMPLETE_OPERATION`: reads and listens do not wait.
    pub nowait: bool,
}

/// One write's data in a queue.
#[derive(Debug)]
struct Entry {
    data: Vec<u8>,
    pos: usize,
    /// The pending write holding this entry (it completes when the entry
    /// is read in full); `None` for data that fitted the quota.
    writer: Option<u64>,
}

impl Entry {
    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
}

/// What a read took.
#[derive(Debug)]
struct Taken {
    data: Vec<u8>,
    /// A message was not read in full.
    overflow: bool,
    /// Pending writes whose data is now all read: `(request, bytes)`.
    done: Vec<(u64, usize)>,
}

/// One direction of a connection.
#[derive(Debug, Default)]
pub struct Chan {
    entries: VecDeque<Entry>,
    /// Bytes of unread data that fitted the quota.
    used: usize,
    quota: usize,
    /// Reads waiting for data, oldest first.
    readers: VecDeque<u64>,
}

impl Chan {
    fn with_quota(quota: u32) -> Chan {
        Chan { quota: quota as usize, ..Chan::default() }
    }

    fn push(&mut self, e: Entry) {
        if e.writer.is_none() {
            self.used += e.remaining();
        }
        self.entries.push_back(e);
    }

    /// Unread bytes, all queued messages counted.
    fn available(&self) -> usize {
        self.entries.iter().map(Entry::remaining).sum()
    }

    /// Take up to `len` bytes: from the first message only in message
    /// mode, across messages in byte mode (a message that was not read in
    /// full keeps its rest). `None` for an empty queue.
    fn take(&mut self, len: usize, message_mode: bool) -> Option<Taken> {
        if self.entries.is_empty() {
            return None;
        }
        let mut data = Vec::new();
        let mut overflow = false;
        let mut done = Vec::new();
        if message_mode {
            let e = self.entries.front_mut().expect("not empty");
            let n = e.remaining().min(len);
            data.extend_from_slice(&e.data[e.pos..e.pos + n]);
            e.pos += n;
            if e.writer.is_none() {
                self.used -= n;
            }
            if e.remaining() == 0 {
                let e = self.entries.pop_front().expect("there");
                if let Some(w) = e.writer {
                    done.push((w, e.data.len()));
                }
            } else {
                overflow = true;
            }
        } else {
            while data.len() < len {
                let Some(e) = self.entries.front_mut() else { break };
                let n = e.remaining().min(len - data.len());
                data.extend_from_slice(&e.data[e.pos..e.pos + n]);
                e.pos += n;
                if e.writer.is_none() {
                    self.used -= n;
                }
                if e.remaining() == 0 {
                    let e = self.entries.pop_front().expect("there");
                    if let Some(w) = e.writer {
                        done.push((w, e.data.len()));
                    }
                }
            }
        }
        Some(Taken { data, overflow, done })
    }
}

/// A pipe name.
#[derive(Debug)]
pub struct NamedPipe {
    /// The name as the creator of the first instance spelled it (what
    /// `FileNameInformation` and `ObjectNameInformation` report for every
    /// handle of the pipe; the map's key is the folded name).
    pub name: Vec<u16>,
    pub share: u32,
    pub config: u32,
    pub max_instances: u32,
    pub inbound: u32,
    pub outbound: u32,
    pub timeout: i64,
    /// The instances whose server end is open.
    pub instances: Vec<u64>,
}

/// One instance.
#[derive(Debug)]
pub struct Instance {
    pub id: u64,
    pub pipe: Vec<u16>,
    /// The pipe's name as its first creator spelled it (kept here too: a
    /// client end outlives the pipe's last server end).
    pub name: Vec<u16>,
    pub state: PipeState,
    /// The instance's pipe type: messages, or a byte stream.
    pub message: bool,
    pub reject_remote: bool,
    pub server_file: Option<u64>,
    pub client_file: Option<u64>,
    pub server_pid: u32,
    pub client_pid: Option<u32>,
    pub to_server: Chan,
    pub to_client: Chan,
    /// `FSCTL_PIPE_LISTEN` requests waiting for a client.
    pub listeners: VecDeque<u64>,
    /// The server has read data the client wrote (what makes the client's
    /// user name available, `GetNamedPipeHandleStateW`).
    pub server_read: bool,
}

/// The named pipe file system.
#[derive(Debug, Default)]
pub struct Npfs {
    pipes: BTreeMap<Vec<u16>, NamedPipe>,
    instances: BTreeMap<u64, Instance>,
    next: u64,
}

/// What `NtCreateNamedPipeFile` asks for, after the service checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NpCreate {
    pub share: u32,
    pub disposition: u32,
    /// `NamedPipeType` (bit 0) and `FILE_PIPE_REJECT_REMOTE_CLIENTS` (bit 1).
    pub pipe_type: u32,
    pub read_mode: u32,
    pub completion_mode: u32,
    pub max_instances: u32,
    pub inbound: u32,
    pub outbound: u32,
    pub timeout: i64,
}

/// A failure that gives the request back (so its event reference can go).
pub type NpResult<T> = core::result::Result<T, (NtStatus, Irp)>;

/// How a request on a pipe went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpDone {
    /// It completed in the call with this status (its completion is queued).
    Done(NtStatus),
    /// It waits (request id).
    Pending(u64),
}

/// `FILE_PIPE_PEEK_BUFFER`'s fixed part (`ntifs.h`): state, bytes
/// available, number of messages, length of the first.
pub const PEEK_HEADER: usize = 16;

/// The result of looking at the state of an end for a read or a write.
fn end_error(inst: &Instance, end: &NpEnd, file: u64, write: bool, has_data: bool) -> Option<NtStatus> {
    let gone = |write: bool| {
        if write {
            Some(STATUS_PIPE_CLOSING)
        } else if has_data {
            None
        } else {
            Some(STATUS_PIPE_BROKEN)
        }
    };
    if end.server {
        match inst.state {
            PipeState::Listening => Some(STATUS_PIPE_LISTENING),
            PipeState::Disconnected => Some(STATUS_PIPE_DISCONNECTED),
            PipeState::Closing => gone(write),
            PipeState::Connected => None,
        }
    } else if inst.client_file != Some(file) {
        Some(STATUS_PIPE_DISCONNECTED)
    } else if inst.server_file.is_none() {
        gone(write)
    } else {
        None
    }
}

impl IoState {
    pub(crate) fn np_end(&self, file: u64) -> Option<NpEnd> {
        match self.files.get(&file)?.target {
            Target::NamedPipe(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn np_end_mut(&mut self, file: u64) -> Option<&mut NpEnd> {
        match &mut self.files.get_mut(&file)?.target {
            Target::NamedPipe(e) => Some(e),
            _ => None,
        }
    }

    pub fn np_instance(&self, id: u64) -> Option<&Instance> {
        self.np.instances.get(&id)
    }

    /// Whether a pipe named `key` (upcased) exists.
    pub fn np_exists(&self, key: &[u16]) -> bool {
        self.np.pipes.contains_key(key)
    }

    /// The default time-out of pipe `key` in 100 ns units (`FSCTL_PIPE_WAIT`
    /// with no time-out of its own).
    pub fn np_default_timeout(&self, key: &[u16]) -> Option<i64> {
        self.np.pipes.get(key).map(|p| p.timeout)
    }

    /// Whether an instance of pipe `key` is listening (a client open would
    /// succeed).
    pub fn np_listening(&self, key: &[u16]) -> bool {
        self.np.pipes.get(key).is_some_and(|p| {
            p.instances.iter().any(|i| self.np.instances.get(i).is_some_and(|i| i.state == PipeState::Listening))
        })
    }

    /// The name of the pipe `instance` belongs to, as its first creator
    /// spelled it (without the device or a leading backslash).
    pub fn np_name(&self, instance: u64) -> Option<&[u16]> {
        self.np.instances.get(&instance).map(|i| i.name.as_slice())
    }

    // ----- opening

    /// Make a new instance for a server open (the checks of
    /// `NtCreateNamedPipeFile`): a new name with `FILE_CREATE` or
    /// `FILE_OPEN_IF`, or a further instance of an existing one. Returns the
    /// instance and `IO_STATUS_BLOCK.Information`. The caller then makes
    /// the file object and calls [`Self::np_server_attached`].
    pub fn np_server_create(
        &mut self,
        key: &[u16],
        name: &[u16],
        c: &NpCreate,
        server_pid: u32,
    ) -> core::result::Result<(u64, u64), NtStatus> {
        let exists = self.np.pipes.contains_key(key);
        match (c.disposition, exists) {
            (FILE_CREATE, true) => return Err(STATUS_ACCESS_DENIED),
            (FILE_OPEN, false) => return Err(STATUS_OBJECT_NAME_NOT_FOUND),
            _ => {}
        }
        if exists {
            let p = &self.np.pipes[key];
            if p.share != c.share {
                return Err(STATUS_ACCESS_DENIED);
            }
            if p.max_instances != u32::MAX && p.instances.len() as u32 >= p.max_instances {
                return Err(STATUS_INSTANCE_NOT_AVAILABLE);
            }
        } else {
            // The first instance of a byte pipe cannot read in message mode;
            // a further instance of an existing name is not checked for it.
            if c.pipe_type & FILE_PIPE_MESSAGE_TYPE == 0 && c.read_mode == FILE_PIPE_MESSAGE_TYPE {
                return Err(STATUS_INVALID_PARAMETER);
            }
            let config = match c.share & 3 {
                2 => FILE_PIPE_INBOUND,
                1 => FILE_PIPE_OUTBOUND,
                _ => FILE_PIPE_FULL_DUPLEX,
            };
            self.np.pipes.insert(
                key.to_vec(),
                NamedPipe {
                    name: name.to_vec(),
                    share: c.share,
                    config,
                    max_instances: c.max_instances,
                    inbound: c.inbound,
                    outbound: c.outbound,
                    timeout: c.timeout,
                    instances: Vec::new(),
                },
            );
        }
        let (inbound, outbound, name) = {
            let p = &self.np.pipes[key];
            (p.inbound, p.outbound, p.name.clone())
        };
        self.np.next += 1;
        let id = self.np.next;
        self.np.instances.insert(
            id,
            Instance {
                id,
                pipe: key.to_vec(),
                name,
                state: PipeState::Listening,
                message: c.pipe_type & FILE_PIPE_MESSAGE_TYPE != 0,
                reject_remote: c.pipe_type & FILE_PIPE_REJECT_REMOTE_CLIENTS != 0,
                server_file: None,
                client_file: None,
                server_pid,
                client_pid: None,
                to_server: Chan::with_quota(inbound),
                to_client: Chan::with_quota(outbound),
                listeners: VecDeque::new(),
                server_read: false,
            },
        );
        self.np.pipes.get_mut(key).expect("there").instances.push(id);
        Ok((id, if exists { FILE_OPENED } else { FILE_CREATED }))
    }

    /// The server's file object `file` exists: it is the instance's server end.
    pub fn np_server_attached(&mut self, instance: u64, file: u64) {
        if let Some(i) = self.np.instances.get_mut(&instance) {
            i.server_file = Some(file);
        }
    }

    /// The server open failed after [`Self::np_server_create`]: take the instance back.
    pub fn np_server_abort(&mut self, instance: u64) {
        self.np_drop_instance(instance);
    }

    /// A client opens pipe `key` for `access`: it needs a direction the
    /// pipe allows and a listening instance. Returns the instance; the
    /// caller makes the file object and calls [`Self::np_client_attached`].
    pub fn np_client_connect(
        &mut self,
        key: &[u16],
        access: u32,
        client_pid: u32,
    ) -> core::result::Result<u64, NtStatus> {
        let p = self.np.pipes.get(key).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
        if access & FILE_READ_DATA != 0 && p.share & FILE_SHARE_READ == 0 {
            return Err(STATUS_ACCESS_DENIED);
        }
        if access & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0 && p.share & FILE_SHARE_WRITE == 0 {
            return Err(STATUS_ACCESS_DENIED);
        }
        let id = p
            .instances
            .iter()
            .copied()
            .find(|i| self.np.instances.get(i).is_some_and(|i| i.state == PipeState::Listening))
            .ok_or(STATUS_PIPE_NOT_AVAILABLE)?;
        let inst = self.np.instances.get_mut(&id).expect("listed");
        inst.state = PipeState::Connected;
        inst.client_pid = Some(client_pid);
        inst.server_read = false;
        Ok(id)
    }

    /// The client's file object exists: complete the listens waiting for it.
    pub fn np_client_attached(&mut self, instance: u64, file: u64) {
        let Some(inst) = self.np.instances.get_mut(&instance) else { return };
        inst.client_file = Some(file);
        let listeners: Vec<u64> = inst.listeners.drain(..).collect();
        for id in listeners {
            self.complete(id, STATUS_SUCCESS, 0, None);
        }
    }

    // ----- data

    /// Hand data to the reads waiting on one direction, oldest first.
    fn np_serve(&mut self, instance: u64, to_server: bool) {
        loop {
            let Some(inst) = self.np.instances.get_mut(&instance) else { return };
            let message_type = inst.message;
            let chan = if to_server { &mut inst.to_server } else { &mut inst.to_client };
            if chan.entries.is_empty() {
                return;
            }
            let Some(rid) = chan.readers.pop_front() else { return };
            let Some(irp) = self.irps.get(&rid) else { continue };
            let IrpOp::Read { len, buf, .. } = irp.op else { continue };
            let message_read = self.np_end(irp.file).is_some_and(|e| e.message_read);
            let inst = self.np.instances.get_mut(&instance).expect("there");
            let chan = if to_server { &mut inst.to_server } else { &mut inst.to_client };
            let t = chan.take(len, message_type && message_read).expect("not empty");
            if to_server {
                inst.server_read = true;
            }
            let status = if t.overflow { STATUS_BUFFER_OVERFLOW } else { STATUS_SUCCESS };
            let n = t.data.len() as u64;
            for (w, bytes) in t.done {
                self.complete(w, STATUS_SUCCESS, bytes as u64, None);
            }
            self.complete(rid, status, n, Some((buf, t.data)));
        }
    }

    /// A read on `file` (`NtReadFile`).
    pub fn np_read(&mut self, file: u64, mut irp: Irp) -> NpResult<NpDone> {
        let Some(end) = self.np_end(file) else { return Err((STATUS_INVALID_HANDLE, irp)) };
        let IrpOp::Read { len, buf, .. } = irp.op else { return Err((STATUS_INVALID_PARAMETER, irp)) };
        let Some(inst) = self.np.instances.get_mut(&end.instance) else { return Err((STATUS_PIPE_BROKEN, irp)) };
        let message_mode = inst.message && end.message_read;
        let chan = if end.server { &mut inst.to_server } else { &mut inst.to_client };
        let has_data = !chan.entries.is_empty();
        if let Some(s) = end_error(inst, &end, file, false, has_data) {
            return Err((s, irp));
        }
        let chan = if end.server { &mut inst.to_server } else { &mut inst.to_client };
        if let Some(t) = chan.take(len, message_mode) {
            if end.server {
                inst.server_read = true;
            }
            let status = if t.overflow { STATUS_BUFFER_OVERFLOW } else { STATUS_SUCCESS };
            let n = t.data.len() as u64;
            for (w, bytes) in t.done {
                self.complete(w, STATUS_SUCCESS, bytes as u64, None);
            }
            irp.sync = false;
            self.complete_now(irp, status, n, Some((buf, t.data)));
            return Ok(NpDone::Done(status));
        }
        if end.nowait {
            return Err((STATUS_PIPE_EMPTY, irp));
        }
        if len == 0 && !message_mode {
            irp.sync = false;
            self.complete_now(irp, STATUS_SUCCESS, 0, None);
            return Ok(NpDone::Done(STATUS_SUCCESS));
        }
        chan.readers.push_back(irp.id);
        Ok(NpDone::Pending(self.park(irp)))
    }

    /// Queue `data` for the other end and serve the reads waiting for it.
    /// `writer` is the pending write that owns the data, if it does not
    /// fit the quota.
    fn np_push(&mut self, instance: u64, from_server: bool, data: Vec<u8>, writer: Option<u64>) {
        let Some(inst) = self.np.instances.get_mut(&instance) else { return };
        let chan = if from_server { &mut inst.to_client } else { &mut inst.to_server };
        chan.push(Entry { data, pos: 0, writer });
        self.np_serve(instance, !from_server);
    }

    /// A write on `file` (`NtWriteFile`); it waits for the reader when the
    /// data does not fit the quota.
    pub fn np_write(&mut self, file: u64, mut irp: Irp, data: Vec<u8>) -> NpResult<NpDone> {
        let Some(end) = self.np_end(file) else { return Err((STATUS_INVALID_HANDLE, irp)) };
        let Some(inst) = self.np.instances.get_mut(&end.instance) else { return Err((STATUS_PIPE_CLOSING, irp)) };
        if let Some(s) = end_error(inst, &end, file, true, false) {
            return Err((s, irp));
        }
        let len = data.len();
        if len == 0 && !inst.message {
            irp.sync = false;
            self.complete_now(irp, STATUS_SUCCESS, 0, None);
            return Ok(NpDone::Done(STATUS_SUCCESS));
        }
        let peer = if end.server { &inst.to_client } else { &inst.to_server };
        let fits = peer.used + len <= peer.quota;
        if !fits && end.nowait {
            return Err((STATUS_PIPE_BUSY, irp));
        }
        if fits {
            self.np_push(end.instance, end.server, data, None);
            irp.sync = false;
            self.complete_now(irp, STATUS_SUCCESS, len as u64, None);
            return Ok(NpDone::Done(STATUS_SUCCESS));
        }
        let id = irp.id;
        self.park(irp);
        self.np_push(end.instance, end.server, data, Some(id));
        if self.irps.contains_key(&id) {
            return Ok(NpDone::Pending(id));
        }
        // Read at once by a waiting reader: nobody waits for it any more.
        for c in self.completions.iter_mut().filter(|c| c.irp == id) {
            c.sync = false;
        }
        Ok(NpDone::Done(STATUS_SUCCESS))
    }

    /// `FSCTL_PIPE_TRANSCEIVE`: write `input` as a message, then read the
    /// reply into the request's buffer (`irp` is a read).
    pub fn np_transceive(&mut self, file: u64, irp: Irp, input: Vec<u8>) -> NpResult<NpDone> {
        let Some(end) = self.np_end(file) else { return Err((STATUS_INVALID_HANDLE, irp)) };
        let Some(inst) = self.np.instances.get(&end.instance) else { return Err((STATUS_PIPE_CLOSING, irp)) };
        if !inst.message || !end.message_read {
            return Err((STATUS_INVALID_PIPE_STATE, irp));
        }
        // A client whose server end is closed cannot transact at all (the
        // write alone would be STATUS_PIPE_CLOSING).
        if !end.server && inst.client_file == Some(file) && inst.server_file.is_none() {
            return Err((STATUS_INVALID_PIPE_STATE, irp));
        }
        if let Some(s) = end_error(inst, &end, file, true, false) {
            return Err((s, irp));
        }
        let peer = if end.server { &inst.to_client } else { &inst.to_server };
        if peer.used + input.len() > peer.quota {
            // Only a reader waiting for it could take it (a pending write
            // is not part of a transaction).
            let readers = !peer.readers.is_empty();
            if !readers {
                return Err((STATUS_PIPE_BUSY, irp));
            }
        }
        self.np_push(end.instance, end.server, input, None);
        self.np_read(file, irp)
    }

    /// `FSCTL_PIPE_PEEK`: the pipe's header and up to `max` bytes of the
    /// first message (or of the stream), consuming nothing.
    pub fn np_peek(&self, file: u64, max: usize) -> core::result::Result<(Vec<u8>, bool), NtStatus> {
        let end = self.np_end(file).ok_or(STATUS_INVALID_HANDLE)?;
        let inst = self.np.instances.get(&end.instance).ok_or(STATUS_PIPE_BROKEN)?;
        let chan = if end.server { &inst.to_server } else { &inst.to_client };
        let has_data = !chan.entries.is_empty();
        if end.server && inst.state == PipeState::Listening {
            return Err(STATUS_INVALID_PIPE_STATE);
        }
        if let Some(s) = end_error(inst, &end, file, false, has_data) {
            return Err(s);
        }
        let available = chan.available();
        let (messages, first) =
            if inst.message { (chan.entries.len(), chan.entries.front().map_or(0, Entry::remaining)) } else { (0, 0) };
        let visible = if inst.message { first } else { available };
        let mut data = Vec::new();
        for e in &chan.entries {
            if data.len() >= max {
                break;
            }
            let n = e.remaining().min(max - data.len());
            data.extend_from_slice(&e.data[e.pos..e.pos + n]);
            if inst.message {
                break;
            }
        }
        let state = if end.server { inst.state.code() } else { FILE_PIPE_CONNECTED_STATE };
        let mut out = Vec::with_capacity(PEEK_HEADER + data.len());
        for v in [state, available as u32, messages as u32, first as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let truncated = data.len() < visible;
        out.extend_from_slice(&data);
        Ok((out, truncated))
    }

    // ----- state changes

    /// `FSCTL_PIPE_LISTEN`: waits for a client. The
    /// statuses of the other states: connected `STATUS_PIPE_CONNECTED`,
    /// closing `STATUS_PIPE_CLOSING`; a no-wait handle never waits and says
    /// `STATUS_PIPE_LISTENING` (after making a disconnected instance
    /// listen).
    pub fn np_listen(&mut self, file: u64, irp: Irp) -> NpResult<NpDone> {
        let Some(end) = self.np_end(file) else { return Err((STATUS_INVALID_HANDLE, irp)) };
        if !end.server {
            return Err((STATUS_ILLEGAL_FUNCTION, irp));
        }
        let Some(inst) = self.np.instances.get_mut(&end.instance) else { return Err((STATUS_PIPE_CLOSING, irp)) };
        match inst.state {
            PipeState::Connected => return Err((STATUS_PIPE_CONNECTED, irp)),
            PipeState::Closing => return Err((STATUS_PIPE_CLOSING, irp)),
            PipeState::Disconnected => inst.state = PipeState::Listening,
            PipeState::Listening => {}
        }
        if end.nowait {
            return Err((STATUS_PIPE_LISTENING, irp));
        }
        inst.listeners.push_back(irp.id);
        let id = irp.id;
        self.park(irp);
        // A waiting `WaitNamedPipe` is polled by its service, nothing to wake.
        Ok(NpDone::Pending(id))
    }

    /// `FSCTL_PIPE_DISCONNECT` on a server end. Unread data goes and the
    /// requests waiting on the instance fail with `STATUS_PIPE_DISCONNECTED`.
    pub fn np_disconnect(&mut self, file: u64) -> core::result::Result<(), NtStatus> {
        let end = self.np_end(file).ok_or(STATUS_INVALID_HANDLE)?;
        if !end.server {
            return Err(STATUS_ILLEGAL_FUNCTION);
        }
        let inst = self.np.instances.get_mut(&end.instance).ok_or(STATUS_PIPE_DISCONNECTED)?;
        if inst.state == PipeState::Disconnected {
            return Err(STATUS_PIPE_DISCONNECTED);
        }
        inst.state = PipeState::Disconnected;
        inst.client_file = None;
        inst.client_pid = None;
        inst.server_read = false;
        let failed = Self::np_drain(inst);
        for id in failed {
            self.complete(id, STATUS_PIPE_DISCONNECTED, 0, None);
        }
        Ok(())
    }

    /// Empty both queues; the requests that were waiting on the instance.
    fn np_drain(inst: &mut Instance) -> Vec<u64> {
        let mut ids: Vec<u64> = inst.listeners.drain(..).collect();
        for chan in [&mut inst.to_server, &mut inst.to_client] {
            ids.extend(chan.readers.drain(..));
            ids.extend(chan.entries.drain(..).filter_map(|e| e.writer));
            chan.used = 0;
        }
        ids
    }

    /// Forget the data a cancelled or completed request left in a queue.
    pub(crate) fn np_forget_irp(&mut self, file: u64, id: u64) {
        let Some(end) = self.np_end(file) else { return };
        let Some(inst) = self.np.instances.get_mut(&end.instance) else { return };
        inst.listeners.retain(|&i| i != id);
        for chan in [&mut inst.to_server, &mut inst.to_client] {
            chan.readers.retain(|&i| i != id);
            chan.entries.retain(|e| e.writer != Some(id));
        }
    }

    fn np_drop_instance(&mut self, id: u64) {
        let Some(inst) = self.np.instances.remove(&id) else { return };
        if let Some(p) = self.np.pipes.get_mut(&inst.pipe) {
            p.instances.retain(|&i| i != id);
            if p.instances.is_empty() {
                self.np.pipes.remove(&inst.pipe);
            }
        }
    }

    /// A named pipe end's last reference went (called by `file_closed`
    /// after its own requests were failed).
    pub(crate) fn np_closed(&mut self, file: u64, end: NpEnd) {
        let Some(inst) = self.np.instances.get_mut(&end.instance) else { return };
        let mut failed: Vec<(u64, NtStatus)> = Vec::new();
        if end.server {
            if inst.server_file != Some(file) {
                return;
            }
            inst.server_file = None;
            // The name loses the instance (room for another).
            let key = inst.pipe.clone();
            if let Some(p) = self.np.pipes.get_mut(&key) {
                p.instances.retain(|&i| i != end.instance);
            }
            let inst = self.np.instances.get_mut(&end.instance).expect("there");
            if inst.client_file.is_some() {
                // The client's reads end when its data is gone, its writes fail.
                failed.extend(inst.to_client.readers.drain(..).map(|i| (i, STATUS_PIPE_BROKEN)));
                failed
                    .extend(inst.to_server.entries.drain(..).filter_map(|e| e.writer).map(|i| (i, STATUS_PIPE_BROKEN)));
                inst.to_server.used = 0;
                failed.extend(inst.listeners.drain(..).map(|i| (i, STATUS_CANCELLED)));
            } else {
                self.np_drop_instance(end.instance);
            }
            let key_gone = self.np.pipes.get(&key).is_some_and(|p| p.instances.is_empty());
            if key_gone {
                self.np.pipes.remove(&key);
            }
        } else {
            if inst.client_file != Some(file) {
                // Detached by a disconnect: nothing of it is left.
                if inst.server_file.is_none() && inst.client_file.is_none() {
                    self.np_drop_instance(end.instance);
                }
                return;
            }
            inst.client_file = None;
            if inst.server_file.is_none() {
                self.np_drop_instance(end.instance);
            } else {
                inst.state = PipeState::Closing;
                failed.extend(inst.to_server.readers.drain(..).map(|i| (i, STATUS_PIPE_BROKEN)));
                failed
                    .extend(inst.to_client.entries.drain(..).filter_map(|e| e.writer).map(|i| (i, STATUS_PIPE_BROKEN)));
                inst.to_client.used = 0;
                // What the server wrote and the client did not read is gone.
                inst.to_client.entries.clear();
            }
        }
        for (id, s) in failed {
            self.complete(id, s, 0, None);
        }
    }

    // ----- information

    /// `FILE_PIPE_LOCAL_INFORMATION` (`ntifs.h`): type, configuration,
    /// maximum and current instances, quotas, bytes available, state, end.
    pub fn np_local_info(&self, file: u64) -> core::result::Result<Vec<u8>, NtStatus> {
        let end = self.np_end(file).ok_or(STATUS_INVALID_HANDLE)?;
        let inst = self.np.instances.get(&end.instance).ok_or(STATUS_PIPE_BROKEN)?;
        let (config, max, current, inbound, outbound) = match self.np.pipes.get(&inst.pipe) {
            Some(p) => (p.config, p.max_instances, p.instances.len() as u32, p.inbound, p.outbound),
            None => (FILE_PIPE_FULL_DUPLEX, 1, 0, 0, 0),
        };
        let chan = if end.server { &inst.to_server } else { &inst.to_client };
        let state = if end.server { inst.state } else { PipeState::Connected };
        let mut ty = u32::from(inst.message);
        if end.server && inst.reject_remote {
            ty |= FILE_PIPE_REJECT_REMOTE_CLIENTS;
        }
        let values =
            [ty, config, max, current, inbound, outbound, chan.available() as u32, state.code(), u32::from(end.server)];
        Ok(values.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// `FILE_PIPE_INFORMATION`: read mode and completion mode of the end.
    pub fn np_handle_info(&self, file: u64) -> core::result::Result<Vec<u8>, NtStatus> {
        let end = self.np_end(file).ok_or(STATUS_INVALID_HANDLE)?;
        let values = [u32::from(end.message_read), u32::from(end.nowait)];
        Ok(values.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// Set the end's read mode and completion mode. Message read mode needs
    /// a message pipe (`STATUS_INVALID_PARAMETER` otherwise).
    pub fn np_set_handle_info(
        &mut self,
        file: u64,
        read_mode: u32,
        completion: u32,
    ) -> core::result::Result<(), NtStatus> {
        let end = self.np_end(file).ok_or(STATUS_INVALID_HANDLE)?;
        if read_mode > FILE_PIPE_MESSAGE_TYPE || completion > FILE_PIPE_COMPLETE_OPERATION {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let message = self.np.instances.get(&end.instance).is_some_and(|i| i.message);
        if read_mode == FILE_PIPE_MESSAGE_TYPE && !message {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let e = self.np_end_mut(file).expect("checked");
        e.message_read = read_mode == FILE_PIPE_MESSAGE_TYPE;
        e.nowait = completion == FILE_PIPE_COMPLETE_OPERATION;
        Ok(())
    }

    /// `FSCTL_POLY_PIPE_PROCESSES`: client process, server process, client
    /// session, server session, flags (bit 0: a client was connected; bit 1:
    /// the server has read from the client).
    pub fn np_processes(&self, file: u64) -> core::result::Result<Vec<u8>, NtStatus> {
        let end = self.np_end(file).ok_or(STATUS_INVALID_HANDLE)?;
        let inst = self.np.instances.get(&end.instance).ok_or(STATUS_PIPE_BROKEN)?;
        let flags = u32::from(inst.client_pid.is_some()) | (u32::from(inst.server_read) << 1);
        let values = [inst.client_pid.unwrap_or(0), inst.server_pid, 0, 0, flags];
        Ok(values.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    /// The instance count and state of pipe `key`, for tests.
    pub fn np_instance_count(&self, key: &[u16]) -> usize {
        self.np.pipes.get(key).map_or(0, |p| p.instances.len())
    }
}
