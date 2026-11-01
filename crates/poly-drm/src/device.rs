//! The device: its files, GEM objects, events, and the ioctl dispatcher.
//! The KMS ioctls are in `kms.rs`.
//!
//! # Files
//!
//! Every `open(2)` of a node is a file ([`Device::open`]) with its own
//! client capabilities, GEM handles (numbers from 1, one per object: a
//! second import of the same object returns the same handle), property
//! blobs, framebuffers and event queue. Closing it ([`Device::close`])
//! removes all of them; framebuffers still shown are taken off their CRTC
//! first.
//!
//! # DRM master and render nodes
//!
//! The first file opened on the primary node while there is no master
//! becomes the master; `DROP_MASTER` gives it up (the display stays as it
//! is) and `SET_MASTER` takes it when nobody has it, for a file that was
//! master before or a privileged caller. Ioctls that change what is shown
//! (`SETCRTC`, `SETPLANE`, `PAGE_FLIP`, `ATOMIC`, property sets, the cursor
//! and gamma) need master, else `EACCES`. A render node file may only use
//! `VERSION`, `GET_CAP`, `GEM_CLOSE` and the two PRIME ioctls (`EACCES`
//! for the rest). Authentication (`GET_MAGIC`, `AUTH_MAGIC`) is recorded
//! but not required by any ioctl.
//!
//! # GEM objects and dumb buffers
//!
//! An object is memory from [`Hardware::alloc`] with a count of what
//! refers to it: handles (in any file), framebuffers and dma-buf files.
//! It is freed when the count reaches 0; mappings hold the memory (a
//! clone of the buffer) on their own. `MAP_DUMB` gives the object's fake
//! mmap offset, its ID shifted left by 32: a mapping of the device file at
//! that offset maps the object, if the file has a handle to it.
//!
//! # Events
//!
//! Each file queues 32-byte events (`struct drm_event_vblank`, `struct
//! drm_event_crtc_sequence`) with the time they become readable, at most
//! 4096 bytes of them (more: `ENOMEM` for the ioctl that would queue one).
//! `read(2)` returns as many whole readable events as fit (`EAGAIN` when
//! none is readable; `EINVAL` if the first does not fit).

use crate::kms::State;
use crate::props::Prop;
use crate::uapi::{
    self, Auth, CreateDumb, GemClose, GemFlink, GemOpen, GetCap, IOCTL_BASE, MapDumb, ModeInfo, PrimeHandle, Request,
    SetVersion, Unique, Version, cap, client_cap, dir, nr,
};
use crate::{Ctx, Errno, FbLayout, Hardware, Output, Reply, Result, UserMemory};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A file of the device (an open of a node).
pub type FileId = u32;

/// A GEM object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoId(pub u32);

/// Which node a file was opened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    /// `/dev/dri/card<n>`: everything, modesetting for the master.
    Primary,
    /// `/dev/dri/renderD<128 + n>`: buffers and PRIME only.
    Render,
}

/// What `DRM_IOCTL_VERSION` and `DRM_IOCTL_GET_UNIQUE` say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriverInfo {
    pub name: &'static str,
    pub desc: &'static str,
    pub date: &'static str,
    pub version: (i32, i32, i32),
    /// The bus ID, e.g. `pci:0000:00:02.0`.
    pub unique: String,
}

/// What a mapping of the device file at some offset maps.
#[derive(Clone, Debug)]
pub struct Mapping<B> {
    pub bo: BoId,
    pub buffer: B,
    /// Where in the buffer the mapping starts.
    pub start: u64,
    /// The buffer's size.
    pub size: u64,
}

/// The most event bytes a file may have queued (Linux's 4 KiB).
pub(crate) const EVENT_SPACE: usize = 4096;
/// The largest dumb buffer.
const MAX_DUMB: u64 = 256 << 20;
/// Dumb buffers are whole pages.
const PAGE: u64 = 4096;

/// The interface version `DRM_IOCTL_SET_VERSION` reports (1.4).
const DI_VERSION: (i32, i32) = (1, 4);

/// What an ID names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Obj {
    Crtc(usize),
    Encoder(usize),
    Connector(usize),
    Plane(usize),
    Property(Prop),
    Fb,
    Blob,
}

impl Obj {
    pub(crate) fn type_code(self) -> u32 {
        use crate::uapi::mode as m;
        match self {
            Obj::Crtc(_) => m::OBJECT_CRTC,
            Obj::Encoder(_) => m::OBJECT_ENCODER,
            Obj::Connector(_) => m::OBJECT_CONNECTOR,
            Obj::Plane(_) => m::OBJECT_PLANE,
            Obj::Property(_) => m::OBJECT_PROPERTY,
            Obj::Fb => m::OBJECT_FB,
            Obj::Blob => m::OBJECT_BLOB,
        }
    }
}

/// A CRTC's vblank counter: `base_seq` at `base_ns`, one more every
/// `frame_ns` while it runs (the CRTC is active).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VblankClock {
    pub base_ns: u64,
    pub base_seq: u64,
    pub frame_ns: u64,
    pub running: bool,
}

impl VblankClock {
    pub fn seq(&self, now: u64) -> u64 {
        if !self.running {
            return self.base_seq;
        }
        self.base_seq + now.saturating_sub(self.base_ns) / self.frame_ns
    }

    /// When vblank `seq` (at least the base) happens.
    pub fn time_of(&self, seq: u64) -> u64 {
        self.base_ns + seq.saturating_sub(self.base_seq) * self.frame_ns
    }

    /// The first vblank after `now`: its number and time.
    pub fn next(&self, now: u64) -> (u64, u64) {
        let s = self.seq(now) + 1;
        (s, self.time_of(s))
    }

    pub fn start(&mut self, now: u64, frame_ns: u64) {
        self.base_seq = self.seq(now);
        self.base_ns = now;
        self.frame_ns = frame_ns.max(1);
        self.running = true;
    }

    pub fn stop(&mut self, now: u64) {
        self.base_seq = self.seq(now);
        self.base_ns = now;
        self.running = false;
    }
}

#[derive(Debug)]
pub(crate) struct Crtc {
    pub id: u32,
    pub vblank: VblankClock,
    /// When the flip or commit last queued on it completes.
    pub pending: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct Connector {
    pub id: u32,
    pub output: Output,
    pub modes: Vec<ModeInfo>,
    pub mm: (u32, u32),
    /// The `EDID` blob (0 without one).
    pub edid_blob: u32,
    pub dpms: u64,
}

#[derive(Debug)]
pub(crate) struct Fb {
    pub owner: FileId,
    pub layout: FbLayout,
    pub bo: BoId,
    pub flags: u32,
    pub modifier: u64,
    /// `bpp` and `depth` as `GETFB` reports them.
    pub bpp: u32,
    pub depth: u32,
}

#[derive(Debug)]
pub(crate) struct Blob {
    pub data: Vec<u8>,
    /// The file that made it (`CREATEPROPBLOB`); `None` for the device's
    /// own (EDID, IN_FORMATS, modes it is set to).
    pub owner: Option<FileId>,
}

#[derive(Debug)]
pub(crate) struct Bo<B> {
    pub size: u64,
    pub buffer: B,
    pub refs: u32,
    /// Its global name (`GEM_FLINK`), 0 if none.
    pub name: u32,
}

/// Client capabilities (`SET_CLIENT_CAP`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Caps {
    pub stereo: bool,
    pub universal: bool,
    pub atomic: bool,
    pub aspect: bool,
    pub writeback: bool,
    pub hotspot: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub due: u64,
    pub bytes: [u8; uapi::event::LEN],
}

#[derive(Debug)]
pub(crate) struct File {
    pub node: Node,
    pub caps: Caps,
    pub handles: BTreeMap<u32, BoId>,
    pub next_handle: u32,
    /// Sorted by `due`.
    pub events: Vec<Event>,
    pub magic: u32,
    pub authenticated: bool,
    pub was_master: bool,
    /// Whether the file asked for interface version 1.1 or later
    /// (`SET_VERSION`), which binds the bus ID: `GET_UNIQUE` reports it
    /// only then.
    pub bus_id: bool,
}

impl File {
    pub fn event_bytes(&self) -> usize {
        self.events.len() * uapi::event::LEN
    }

    pub fn queue(&mut self, e: Event) {
        let at = self.events.partition_point(|x| x.due <= e.due);
        self.events.insert(at, e);
    }
}

/// A DRM device. See the crate documentation.
pub struct Device<H: Hardware> {
    pub(crate) hw: H,
    pub(crate) info: DriverInfo,
    next_id: u32,
    pub(crate) objects: BTreeMap<u32, Obj>,
    pub(crate) prop_ids: [u32; Prop::ALL.len()],
    pub(crate) crtcs: Vec<Crtc>,
    pub(crate) encoders: Vec<u32>,
    pub(crate) connectors: Vec<Connector>,
    pub(crate) planes: Vec<u32>,
    pub(crate) state: State,
    pub(crate) fbs: BTreeMap<u32, Fb>,
    pub(crate) blobs: BTreeMap<u32, Blob>,
    pub(crate) bos: BTreeMap<BoId, Bo<H::Buffer>>,
    next_bo: u32,
    names: BTreeMap<u32, BoId>,
    next_name: u32,
    pub(crate) files: BTreeMap<FileId, File>,
    next_file: FileId,
    next_magic: u32,
    pub(crate) master: Option<FileId>,
    pub(crate) display_taken: bool,
    /// The `IN_FORMATS` blob every plane shares.
    pub(crate) in_formats: u32,
}

impl<H: Hardware> core::fmt::Debug for Device<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("name", &self.info.name)
            .field("crtcs", &self.crtcs.len())
            .field("files", &self.files.len())
            .field("master", &self.master)
            .field("framebuffers", &self.fbs.len())
            .field("objects", &self.bos.len())
            .field("display_taken", &self.display_taken)
            .finish()
    }
}

/// Ioctl permission bits.
const MASTER: u8 = 1;
const RENDER: u8 = 2;

/// The kernel's argument size, direction and permissions of request `nr`.
fn definition(n: u8) -> Option<(usize, u32, u8)> {
    use uapi::*;
    const RW: u32 = dir::READ | dir::WRITE;
    Some(match n {
        nr::VERSION => (Version::SIZE, RW, RENDER),
        nr::GET_UNIQUE => (Unique::SIZE, RW, 0),
        nr::GET_MAGIC => (Auth::SIZE, dir::READ, 0),
        nr::SET_VERSION => (SetVersion::SIZE, RW, 0),
        nr::MODESET_CTL => (8, dir::WRITE, 0),
        nr::GEM_CLOSE => (GemClose::SIZE, dir::WRITE, RENDER),
        nr::GEM_FLINK => (GemFlink::SIZE, RW, 0),
        nr::GEM_OPEN => (GemOpen::SIZE, RW, 0),
        nr::GET_CAP => (GetCap::SIZE, RW, RENDER),
        nr::SET_CLIENT_CAP => (GetCap::SIZE, dir::WRITE, 0),
        nr::AUTH_MAGIC => (Auth::SIZE, dir::WRITE, MASTER),
        nr::SET_MASTER | nr::DROP_MASTER => (0, dir::NONE, 0),
        nr::PRIME_HANDLE_TO_FD | nr::PRIME_FD_TO_HANDLE => (PrimeHandle::SIZE, RW, RENDER),
        nr::WAIT_VBLANK => (WaitVblank::SIZE, RW, 0),
        nr::CRTC_GET_SEQUENCE => (CrtcGetSequence::SIZE, RW, 0),
        nr::CRTC_QUEUE_SEQUENCE => (CrtcQueueSequence::SIZE, RW, 0),
        nr::MODE_GETRESOURCES => (CardRes::SIZE, RW, 0),
        nr::MODE_GETCRTC => (ModeCrtc::SIZE, RW, 0),
        nr::MODE_SETCRTC => (ModeCrtc::SIZE, RW, MASTER),
        nr::MODE_CURSOR => (Cursor::SIZE, RW, MASTER),
        nr::MODE_CURSOR2 => (36, RW, MASTER),
        nr::MODE_GETGAMMA => (CrtcLut::SIZE, RW, 0),
        nr::MODE_SETGAMMA => (CrtcLut::SIZE, RW, MASTER),
        nr::MODE_GETENCODER => (GetEncoder::SIZE, RW, 0),
        nr::MODE_GETCONNECTOR => (GetConnector::SIZE, RW, 0),
        nr::MODE_GETPROPERTY => (GetProperty::SIZE, RW, 0),
        nr::MODE_SETPROPERTY => (ConnectorSetProperty::SIZE, RW, MASTER),
        nr::MODE_GETPROPBLOB => (GetBlob::SIZE, RW, 0),
        nr::MODE_GETFB => (FbCmd::SIZE, RW, 0),
        nr::MODE_ADDFB => (FbCmd::SIZE, RW, 0),
        nr::MODE_RMFB => (4, RW, 0),
        nr::MODE_PAGE_FLIP => (PageFlip::SIZE, RW, MASTER),
        nr::MODE_DIRTYFB => (FbDirty::SIZE, RW, 0),
        nr::MODE_CREATE_DUMB => (CreateDumb::SIZE, RW, 0),
        nr::MODE_MAP_DUMB => (MapDumb::SIZE, RW, 0),
        nr::MODE_DESTROY_DUMB => (4, RW, 0),
        nr::MODE_GETPLANERESOURCES => (GetPlaneRes::SIZE, RW, 0),
        nr::MODE_GETPLANE => (GetPlane::SIZE, RW, 0),
        nr::MODE_SETPLANE => (SetPlane::SIZE, RW, MASTER),
        nr::MODE_ADDFB2 | nr::MODE_GETFB2 => (FbCmd2::SIZE, RW, 0),
        nr::MODE_OBJ_GETPROPERTIES => (ObjGetProperties::SIZE, RW, 0),
        nr::MODE_OBJ_SETPROPERTY => (ObjSetProperty::SIZE, RW, MASTER),
        nr::MODE_ATOMIC => (Atomic::SIZE, RW, MASTER),
        nr::MODE_CREATEPROPBLOB => (CreateBlob::SIZE, RW, 0),
        nr::MODE_DESTROYPROPBLOB => (4, RW, 0),
        _ => return None,
    })
}

/// Copy `s` to the user buffer `ptr` of `len` bytes (as much as fits, no
/// terminating NUL) and set `len` to the length of `s`.
fn copy_field(user: &mut dyn UserMemory, len: &mut u64, ptr: u64, s: &[u8]) -> Result<()> {
    let n = (*len).min(s.len() as u64) as usize;
    if n > 0 {
        user.write(ptr, &s[..n])?;
    }
    *len = s.len() as u64;
    Ok(())
}

impl<H: Hardware> Device<H> {
    /// A device on `hw`: one CRTC, encoder, connector and primary plane per
    /// output [`Hardware::probe`] reports.
    pub fn new(mut hw: H, info: DriverInfo) -> Device<H> {
        let outputs = hw.probe();
        let mut d = Device {
            hw,
            info,
            next_id: 1,
            objects: BTreeMap::new(),
            prop_ids: [0; Prop::ALL.len()],
            crtcs: Vec::new(),
            encoders: Vec::new(),
            connectors: Vec::new(),
            planes: Vec::new(),
            state: State::default(),
            fbs: BTreeMap::new(),
            blobs: BTreeMap::new(),
            bos: BTreeMap::new(),
            next_bo: 1,
            names: BTreeMap::new(),
            next_name: 1,
            files: BTreeMap::new(),
            next_file: 1,
            next_magic: 1,
            master: None,
            display_taken: false,
            in_formats: 0,
        };
        for p in Prop::ALL {
            d.prop_ids[p.index()] = d.add_object(Obj::Property(p));
        }
        for i in 0..outputs.len() {
            let crtc = d.add_object(Obj::Crtc(i));
            d.crtcs.push(Crtc {
                id: crtc,
                vblank: VblankClock { base_ns: 0, base_seq: 0, frame_ns: 16_666_667, running: false },
                pending: None,
            });
            let encoder = d.add_object(Obj::Encoder(i));
            d.encoders.push(encoder);
            let connector = d.add_object(Obj::Connector(i));
            d.connectors.push(Connector {
                id: connector,
                output: Output::default(),
                modes: Vec::new(),
                mm: (0, 0),
                edid_blob: 0,
                dpms: uapi::mode::DPMS_ON,
            });
            let plane = d.add_object(Obj::Plane(i));
            d.planes.push(plane);
        }
        d.state = State::empty(outputs.len());
        let blob = crate::kms::in_formats_blob();
        d.in_formats = d.add_blob(blob, None);
        d.apply_outputs(outputs);
        d
    }

    pub(crate) fn add_object(&mut self, o: Obj) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(id, o);
        id
    }

    pub(crate) fn add_blob(&mut self, data: Vec<u8>, owner: Option<FileId>) -> u32 {
        let id = self.add_object(Obj::Blob);
        self.blobs.insert(id, Blob { data, owner });
        id
    }

    /// A client lets go of blob `id`: removed, unless the state still uses
    /// it as a mode, in which case the device keeps it until then.
    pub(crate) fn drop_blob(&mut self, id: u32) {
        if self.state.crtcs.iter().any(|c| c.mode_blob == id) {
            if let Some(b) = self.blobs.get_mut(&id) {
                b.owner = None;
            }
        } else {
            self.remove_blob(id);
        }
    }

    pub(crate) fn remove_blob(&mut self, id: u32) {
        self.blobs.remove(&id);
        self.objects.remove(&id);
    }

    /// Take what the hardware reports about its outputs: connection, modes,
    /// EDID blobs.
    pub(crate) fn apply_outputs(&mut self, outputs: Vec<Output>) {
        for (i, out) in outputs.into_iter().enumerate().take(self.connectors.len()) {
            if self.connectors[i].output == out && !self.connectors[i].modes.is_empty() {
                continue;
            }
            let (modes, mm) = if out.connected {
                crate::modes::output_modes((out.width, out.height), out.edid.as_deref())
            } else {
                (Vec::new(), (0, 0))
            };
            let old = self.connectors[i].edid_blob;
            if old != 0 {
                self.remove_blob(old);
            }
            let edid = out.edid.clone().filter(|e| !e.is_empty());
            let blob = edid.map_or(0, |e| self.add_blob(e, None));
            let c = &mut self.connectors[i];
            c.output = out;
            c.modes = modes;
            c.mm = mm;
            c.edid_blob = blob;
        }
    }

    /// The hardware (for the kernel's own tests and statistics).
    pub fn hardware(&mut self) -> &mut H {
        &mut self.hw
    }

    /// How many CRTCs (outputs) the device has.
    pub fn crtc_count(&self) -> usize {
        self.crtcs.len()
    }

    /// Whether the device has taken the display from the console.
    pub fn display_taken(&self) -> bool {
        self.display_taken
    }

    /// The file that is DRM master, if any.
    pub fn master(&self) -> Option<FileId> {
        self.master
    }

    /// Open files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Live GEM objects and framebuffers.
    pub fn object_counts(&self) -> (usize, usize) {
        (self.bos.len(), self.fbs.len())
    }

    /// A new file on `node`. The first primary-node file while there is no
    /// master becomes master.
    pub fn open(&mut self, node: Node) -> FileId {
        let id = self.next_file;
        self.next_file += 1;
        let magic = self.next_magic;
        self.next_magic += 1;
        let master = node == Node::Primary && self.master.is_none();
        self.files.insert(
            id,
            File {
                node,
                caps: Caps::default(),
                handles: BTreeMap::new(),
                next_handle: 1,
                events: Vec::new(),
                magic,
                authenticated: master,
                was_master: master,
                bus_id: false,
            },
        );
        if master {
            self.master = Some(id);
        }
        id
    }

    /// Close `file`: its framebuffers (taken off any CRTC first), handles,
    /// blobs and events go. If it was master, or it was the last file, the
    /// outputs it drove are turned off and the console gets the display
    /// back.
    pub fn close(&mut self, file: FileId, now: u64) {
        if !self.files.contains_key(&file) {
            return;
        }
        self.tick(now);
        let fbs: Vec<u32> = self.fbs.iter().filter(|(_, f)| f.owner == file).map(|(id, _)| *id).collect();
        for fb in fbs {
            self.remove_fb(fb, now);
        }
        let blobs: Vec<u32> = self.blobs.iter().filter(|(_, b)| b.owner == Some(file)).map(|(id, _)| *id).collect();
        for b in blobs {
            self.drop_blob(b);
        }
        let handles: Vec<BoId> =
            self.files.get(&file).map(|f| f.handles.values().copied().collect()).unwrap_or_default();
        self.files.remove(&file);
        for bo in handles {
            self.unref(bo);
        }
        let was_master = self.master == Some(file);
        if was_master {
            self.master = None;
        }
        if was_master || self.files.is_empty() {
            self.restore_console(now);
        }
    }

    /// Turn off every output the device drives and give the display back
    /// to the console.
    pub(crate) fn restore_console(&mut self, now: u64) {
        if !self.display_taken {
            return;
        }
        let off = State::empty(self.crtcs.len());
        let _ = self.apply(&off, 0, now);
        self.set_state(off, now);
        self.hw.give_back_display();
        self.display_taken = false;
    }

    /// Clear flips and commits that completed by `now`.
    pub fn tick(&mut self, now: u64) {
        for c in &mut self.crtcs {
            if c.pending.is_some_and(|t| t <= now) {
                c.pending = None;
            }
        }
    }

    /// The readable events of `file` at `now`, as many whole ones as fit in
    /// `max` bytes, taken off its queue.
    pub fn read(&mut self, file: FileId, max: usize, now: u64) -> Result<Vec<u8>> {
        self.tick(now);
        let f = self.files.get_mut(&file).ok_or(Errno::EBADF)?;
        let ready = f.events.partition_point(|e| e.due <= now);
        if ready == 0 {
            return Err(Errno::EAGAIN);
        }
        let n = (max / uapi::event::LEN).min(ready);
        if n == 0 {
            return Err(Errno::EINVAL);
        }
        let mut out = Vec::with_capacity(n * uapi::event::LEN);
        for e in f.events.drain(..n) {
            out.extend_from_slice(&e.bytes);
        }
        Ok(out)
    }

    /// Whether `file` has an event to read at `now`.
    pub fn readable(&self, file: FileId, now: u64) -> bool {
        self.files.get(&file).is_some_and(|f| f.events.first().is_some_and(|e| e.due <= now))
    }

    /// When the next queued event of any file becomes readable.
    pub fn next_event_ns(&self) -> Option<u64> {
        self.files.values().filter_map(|f| f.events.first().map(|e| e.due)).min()
    }

    /// When the next event of any file that is not readable at `now`
    /// becomes readable (what a thread waking readers waits for).
    pub fn next_event_after(&self, now: u64) -> Option<u64> {
        self.files.values().filter_map(|f| f.events.iter().map(|e| e.due).find(|&d| d > now)).min()
    }

    /// The files with an event to read at `now`.
    pub fn ready_files(&self, now: u64) -> Vec<FileId> {
        self.files.iter().filter(|(_, f)| f.events.first().is_some_and(|e| e.due <= now)).map(|(id, _)| *id).collect()
    }

    /// What `mmap(2)` of `file` at `offset` (`len` bytes) maps: the GEM
    /// object whose fake offset `offset` falls in, if the file has a handle
    /// to it. `EINVAL` for an offset that names no object or a range past
    /// its end, `EACCES` for an object of another file.
    pub fn map(&self, file: FileId, offset: u64, len: u64) -> Result<Mapping<H::Buffer>> {
        let f = self.files.get(&file).ok_or(Errno::EBADF)?;
        let bo = BoId((offset >> 32) as u32);
        let start = offset & 0xffff_ffff;
        let b = self.bos.get(&bo).ok_or(Errno::EINVAL)?;
        if !f.handles.values().any(|&h| h == bo) {
            return Err(Errno::EACCES);
        }
        if len == 0 || start.checked_add(len).is_none_or(|end| end > b.size.next_multiple_of(PAGE)) {
            return Err(Errno::EINVAL);
        }
        Ok(Mapping { bo, buffer: b.buffer.clone(), start, size: b.size })
    }

    /// A dma-buf file of object `bo` was closed: drop its reference.
    pub fn release_dmabuf(&mut self, bo: BoId) {
        self.unref(bo);
    }

    /// The memory and size of object `bo` (what a dma-buf maps).
    pub fn buffer(&self, bo: BoId) -> Option<(H::Buffer, u64)> {
        self.bos.get(&bo).map(|b| (b.buffer.clone(), b.size))
    }

    pub(crate) fn unref(&mut self, bo: BoId) {
        let Some(b) = self.bos.get_mut(&bo) else { return };
        b.refs = b.refs.saturating_sub(1);
        if b.refs == 0
            && let Some(b) = self.bos.remove(&bo)
            && b.name != 0
        {
            self.names.remove(&b.name);
        }
    }

    /// The handle of `bo` in `file`, made (and counted) if it has none.
    pub(crate) fn handle_for(&mut self, file: FileId, bo: BoId) -> Result<u32> {
        let f = self.files.get_mut(&file).ok_or(Errno::EBADF)?;
        if let Some((&h, _)) = f.handles.iter().find(|(_, b)| **b == bo) {
            return Ok(h);
        }
        let h = f.next_handle;
        f.next_handle += 1;
        f.handles.insert(h, bo);
        if let Some(b) = self.bos.get_mut(&bo) {
            b.refs += 1;
        }
        Ok(h)
    }

    pub(crate) fn lookup(&self, file: FileId, handle: u32) -> Result<BoId> {
        self.files.get(&file).and_then(|f| f.handles.get(&handle)).copied().ok_or(Errno::ENOENT)
    }

    fn delete_handle(&mut self, file: FileId, handle: u32) -> Result<()> {
        let f = self.files.get_mut(&file).ok_or(Errno::EBADF)?;
        let bo = f.handles.remove(&handle).ok_or(Errno::EINVAL)?;
        self.unref(bo);
        Ok(())
    }

    pub(crate) fn file(&self, file: FileId) -> Result<&File> {
        self.files.get(&file).ok_or(Errno::EBADF)
    }

    pub(crate) fn file_mut(&mut self, file: FileId) -> Result<&mut File> {
        self.files.get_mut(&file).ok_or(Errno::EBADF)
    }

    /// Handle `ioctl(2)` request `request` with argument `arg` on `file`.
    pub fn ioctl(&mut self, file: FileId, request: u32, arg: u64, ctx: &mut Ctx<'_, H::Buffer>) -> Result<Reply> {
        self.tick(ctx.now_ns);
        let req = Request::decode(request);
        if req.ty != IOCTL_BASE {
            return Err(Errno::ENOTTY);
        }
        let (ksize, kdir, perm) = definition(req.nr).ok_or(Errno::EINVAL)?;
        let f = self.file(file)?;
        if f.node == Node::Render && perm & RENDER == 0 {
            return Err(Errno::EACCES);
        }
        if perm & MASTER != 0 && self.master != Some(file) {
            return Err(Errno::EACCES);
        }
        let mut buf = [0u8; 128];
        let arg_buf = &mut buf[..ksize];
        let n = req.size.min(ksize);
        if req.dir & kdir & dir::WRITE != 0 && n > 0 {
            ctx.user.read(arg, &mut arg_buf[..n])?;
        }
        let reply = self.dispatch(file, req.nr, arg_buf, ctx)?;
        if req.dir & kdir & dir::READ != 0 && n > 0 {
            ctx.user.write(arg, &arg_buf[..n])?;
        }
        Ok(reply)
    }

    fn dispatch(&mut self, file: FileId, n: u8, b: &mut [u8], ctx: &mut Ctx<'_, H::Buffer>) -> Result<Reply> {
        let done = |r: Result<()>| r.map(|()| Reply::Done(0));
        match n {
            nr::VERSION => done(self.version(b, ctx.user)),
            nr::GET_UNIQUE => {
                // Empty until the file binds the bus ID with interface
                // version 1.1 (libdrm's `drmSetInterfaceVersion`): libdrm's
                // `drmOpen` by driver name passes over a minor whose bus ID
                // is already set, as one some other client set up.
                let unique = if self.file(file)?.bus_id { self.info.unique.as_bytes() } else { &[] };
                let mut u = Unique::decode(b);
                copy_field(ctx.user, &mut u.len, u.ptr, unique)?;
                u.encode(b);
                Ok(Reply::Done(0))
            }
            nr::GET_MAGIC => {
                Auth { magic: self.file(file)?.magic }.encode(b);
                Ok(Reply::Done(0))
            }
            nr::AUTH_MAGIC => {
                let magic = Auth::decode(b).magic;
                let f = self.files.values_mut().find(|f| f.magic == magic && magic != 0).ok_or(Errno::EINVAL)?;
                f.authenticated = true;
                Ok(Reply::Done(0))
            }
            nr::SET_VERSION => done(self.set_version(file, b)),
            nr::MODESET_CTL => Ok(Reply::Done(0)),
            nr::GEM_CLOSE => done(self.delete_handle(file, GemClose::decode(b).handle)),
            nr::GEM_FLINK => done(self.flink(file, b)),
            nr::GEM_OPEN => done(self.gem_open(file, b)),
            nr::GET_CAP => done(self.get_cap(b)),
            nr::SET_CLIENT_CAP => done(self.set_client_cap(file, b)),
            nr::SET_MASTER => done(self.set_master(file, ctx.privileged)),
            nr::DROP_MASTER => {
                if self.master != Some(file) {
                    return Err(Errno::EINVAL);
                }
                self.master = None;
                Ok(Reply::Done(0))
            }
            nr::PRIME_HANDLE_TO_FD => done(self.prime_export(file, b, ctx)),
            nr::PRIME_FD_TO_HANDLE => {
                let mut p = PrimeHandle::decode(b);
                let bo = ctx.fds.import(p.fd)?;
                if !self.bos.contains_key(&bo) {
                    return Err(Errno::ENOENT);
                }
                p.handle = self.handle_for(file, bo)?;
                p.encode(b);
                Ok(Reply::Done(0))
            }
            nr::MODE_CREATE_DUMB => done(self.create_dumb(file, b)),
            nr::MODE_MAP_DUMB => {
                let mut m = MapDumb::decode(b);
                let bo = self.lookup(file, m.handle)?;
                m.offset = (bo.0 as u64) << 32;
                m.encode(b);
                Ok(Reply::Done(0))
            }
            nr::MODE_DESTROY_DUMB => done(self.delete_handle(file, uapi::Field::get(b, 0))),
            _ => self.kms_ioctl(file, n, b, ctx),
        }
    }

    fn version(&mut self, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut v = Version::decode(b);
        (v.major, v.minor, v.patchlevel) = self.info.version;
        copy_field(user, &mut v.name_len, v.name, self.info.name.as_bytes())?;
        copy_field(user, &mut v.date_len, v.date, self.info.date.as_bytes())?;
        copy_field(user, &mut v.desc_len, v.desc, self.info.desc.as_bytes())?;
        v.encode(b);
        Ok(())
    }

    /// `SET_VERSION`: -1 asks for nothing; interface 1.x up to 1.4 and the
    /// driver's own major version are accepted. The reply is the versions
    /// in use.
    fn set_version(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let v = SetVersion::decode(b);
        if v.di_major != -1 && (v.di_major != DI_VERSION.0 || v.di_minor < 0 || v.di_minor > DI_VERSION.1) {
            return Err(Errno::EINVAL);
        }
        if v.dd_major != -1 && (v.dd_major != self.info.version.0 || v.dd_minor < 0 || v.dd_minor > self.info.version.1)
        {
            return Err(Errno::EINVAL);
        }
        if v.di_major == 1 && v.di_minor >= 1 {
            self.file_mut(file)?.bus_id = true;
        }
        SetVersion {
            di_major: DI_VERSION.0,
            di_minor: DI_VERSION.1,
            dd_major: self.info.version.0,
            dd_minor: self.info.version.1,
        }
        .encode(b);
        Ok(())
    }

    fn get_cap(&self, b: &mut [u8]) -> Result<()> {
        let mut c = GetCap::decode(b);
        c.value = match c.capability {
            cap::DUMB_BUFFER => 1,
            cap::VBLANK_HIGH_CRTC => 1,
            cap::DUMB_PREFERRED_DEPTH => 24,
            // System memory the CPU reads at full speed: no shadow needed.
            cap::DUMB_PREFER_SHADOW => 0,
            cap::PRIME => cap::PRIME_IMPORT | cap::PRIME_EXPORT,
            cap::TIMESTAMP_MONOTONIC => 1,
            cap::ASYNC_PAGE_FLIP => 0,
            cap::CURSOR_WIDTH | cap::CURSOR_HEIGHT => 64,
            cap::ADDFB2_MODIFIERS => 1,
            cap::PAGE_FLIP_TARGET => 1,
            cap::CRTC_IN_VBLANK_EVENT => 1,
            cap::SYNCOBJ | cap::SYNCOBJ_TIMELINE => 0,
            cap::ATOMIC_ASYNC_PAGE_FLIP => 0,
            _ => return Err(Errno::EINVAL),
        };
        c.encode(b);
        Ok(())
    }

    fn set_client_cap(&mut self, file: FileId, b: &[u8]) -> Result<()> {
        let c = GetCap::decode(b);
        if c.value > 1 {
            return Err(Errno::EINVAL);
        }
        let on = c.value == 1;
        let caps = &mut self.file_mut(file)?.caps;
        match c.capability {
            client_cap::STEREO_3D => caps.stereo = on,
            client_cap::UNIVERSAL_PLANES => caps.universal = on,
            client_cap::ATOMIC => {
                // "implicitly enables DRM_CLIENT_CAP_UNIVERSAL_PLANES and
                // DRM_CLIENT_CAP_ASPECT_RATIO" (drm.h).
                caps.atomic = on;
                if on {
                    caps.universal = true;
                    caps.aspect = true;
                }
            }
            client_cap::ASPECT_RATIO => caps.aspect = on,
            client_cap::WRITEBACK_CONNECTORS => {
                if on && !caps.atomic {
                    return Err(Errno::EINVAL);
                }
                caps.writeback = on;
            }
            client_cap::CURSOR_PLANE_HOTSPOT => {
                if on && !caps.atomic {
                    return Err(Errno::EINVAL);
                }
                caps.hotspot = on;
            }
            _ => return Err(Errno::EINVAL),
        }
        Ok(())
    }

    fn set_master(&mut self, file: FileId, privileged: bool) -> Result<()> {
        match self.master {
            Some(m) if m == file => Ok(()),
            Some(_) => Err(Errno::EBUSY),
            None => {
                let f = self.file_mut(file)?;
                if f.node != Node::Primary || !(privileged || f.was_master) {
                    return Err(Errno::EACCES);
                }
                f.was_master = true;
                f.authenticated = true;
                self.master = Some(file);
                Ok(())
            }
        }
    }

    fn flink(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut g = GemFlink::decode(b);
        let bo = self.lookup(file, g.handle)?;
        let next = self.next_name;
        let o = self.bos.get_mut(&bo).ok_or(Errno::ENOENT)?;
        if o.name == 0 {
            o.name = next;
            self.next_name += 1;
            self.names.insert(next, bo);
        }
        g.name = o.name;
        g.encode(b);
        Ok(())
    }

    fn gem_open(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut g = GemOpen::decode(b);
        let bo = *self.names.get(&g.name).ok_or(Errno::ENOENT)?;
        g.handle = self.handle_for(file, bo)?;
        g.size = self.bos.get(&bo).map_or(0, |o| o.size);
        g.encode(b);
        Ok(())
    }

    /// `CREATE_DUMB`: lines of `width * bpp / 8` bytes rounded up to 4
    /// (what the device's copy takes as its stride), whole pages.
    fn create_dumb(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut c = CreateDumb::decode(b);
        if c.flags != 0 || c.width == 0 || c.height == 0 || c.bpp == 0 || c.bpp > 32 {
            return Err(Errno::EINVAL);
        }
        if c.width > crate::modes::MAX_SIZE || c.height > crate::modes::MAX_SIZE {
            return Err(Errno::EINVAL);
        }
        let pitch = (c.width as u64 * c.bpp.div_ceil(8) as u64).next_multiple_of(4);
        let size = (pitch * c.height as u64).next_multiple_of(PAGE);
        if size > MAX_DUMB {
            return Err(Errno::ENOMEM);
        }
        let buffer = self.hw.alloc(size).ok_or(Errno::ENOMEM)?;
        let bo = BoId(self.next_bo);
        self.next_bo += 1;
        self.bos.insert(bo, Bo { size, buffer, refs: 0, name: 0 });
        c.handle = self.handle_for(file, bo)?;
        c.pitch = pitch as u32;
        c.size = size;
        c.encode(b);
        Ok(())
    }

    fn prime_export(&mut self, file: FileId, b: &mut [u8], ctx: &mut Ctx<'_, H::Buffer>) -> Result<()> {
        let mut p = PrimeHandle::decode(b);
        if p.flags & !(uapi::prime::CLOEXEC | uapi::prime::RDWR) != 0 {
            return Err(Errno::EINVAL);
        }
        let bo = self.lookup(file, p.handle)?;
        let o = self.bos.get_mut(&bo).ok_or(Errno::ENOENT)?;
        o.refs += 1;
        let (buffer, size) = (o.buffer.clone(), o.size);
        match ctx.fds.export(bo, &buffer, size, p.flags) {
            Ok(fd) => {
                p.fd = fd;
                p.encode(b);
                Ok(())
            }
            Err(e) => {
                self.unref(bo);
                Err(e)
            }
        }
    }
}
