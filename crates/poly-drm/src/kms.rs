//! Kernel modesetting: the object getters, framebuffers, and one state
//! that legacy (`SETCRTC`, `SETPLANE`, `PAGE_FLIP`) and atomic
//! (`MODE_ATOMIC`) modesetting both change through [`Device::commit`].
//!
//! # Objects
//!
//! Output `i` of the hardware is CRTC `i`, driven by encoder `i` (type
//! VIRTUAL, `possible_crtcs` = bit `i`), feeding connector `i` (type
//! VIRTUAL, `Virtual-<i+1>`), with primary plane `i` (formats XRGB8888,
//! ARGB8888, XBGR8888, ABGR8888; linear only; no scaling: the source
//! rectangle is the CRTC's size, at the top left of the screen). There are
//! no cursor or overlay planes and no gamma tables (`gamma_size` 0).
//!
//! # The state and commits
//!
//! [`State`] is, per CRTC, `ACTIVE` and the mode (`MODE_ID`); per plane
//! `FB_ID`, `CRTC_ID` and the source and destination rectangles; per
//! connector `CRTC_ID`. A commit checks a new state as a whole (an active
//! CRTC has a mode, a connector, and its primary plane showing a
//! framebuffer over the whole mode, inside the framebuffer, unscaled), then
//! asks for `ALLOW_MODESET` if a CRTC's activity, mode or connectors
//! change, then programs the hardware: for each output whose picture
//! changed (another framebuffer or rectangle, or turned on or off) it
//! uploads the visible rectangle, sets the scanout and flushes it; outputs
//! that the commit named but whose picture stayed the same are uploaded
//! and flushed again (a client that drew into the shown buffer and commits
//! it again). A commit with `DRM_MODE_PAGE_FLIP_EVENT` queues a
//! flip-complete event per CRTC it names, readable at that CRTC's next
//! vblank; until then the CRTC has a commit pending, and another
//! non-blocking commit or legacy flip on it fails with `EBUSY` (a blocking
//! one waits: [`Reply::Sleep`]).
//!
//! The device takes changes at once: a flip is on the screen when the
//! ioctl returns, and the event says when the (virtual) vblank came.
//!
//! # Vblank
//!
//! Each CRTC counts vblanks while it is active, one per frame of its mode
//! (the mode's refresh rate: `clock * 1000 / (htotal * vtotal)` Hz), on
//! CLOCK_MONOTONIC. Event timestamps are those vblank times
//! (`DRM_CAP_TIMESTAMP_MONOTONIC` is 1).

use crate::device::{Device, Event, Fb, FileId, Obj, VblankClock};
use crate::props::{Kind, Prop};
use crate::uapi::{
    self, Atomic, CardRes, ClipRect, CreateBlob, CrtcGetSequence, CrtcLut, CrtcQueueSequence, EventCrtcSequence,
    EventVblank, FbCmd, FbCmd2, FbDirty, Field, FormatModifier, FormatModifierBlob, GetBlob, GetConnector, GetEncoder,
    GetPlane, GetPlaneRes, GetProperty, ModeCrtc, ModeInfo, ObjGetProperties, ObjSetProperty, PageFlip, PropertyEnum,
    SetPlane, WaitVblank, fourcc, mode as m, name_bytes, nr, vblank,
};
use crate::{Ctx, Errno, FbLayout, Hardware, HwError, Rect, Reply, Result, UserMemory, modes};
use alloc::vec;
use alloc::vec::Vec;

/// The formats every plane takes.
pub(crate) const FORMATS: [u32; 4] = [fourcc::XRGB8888, fourcc::ARGB8888, fourcc::XBGR8888, fourcc::ABGR8888];

/// The most properties one atomic commit may set.
const MAX_ATOMIC_PROPS: usize = 4096;
/// The largest property blob a client may create.
const MAX_BLOB: usize = 1 << 20;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CrtcState {
    pub active: bool,
    pub mode: Option<ModeInfo>,
    /// The blob `MODE_ID` reads (0 without a mode).
    pub mode_blob: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PlaneState {
    pub fb: u32,
    pub crtc: u32,
    /// `SRC_X`, `SRC_Y`, `SRC_W`, `SRC_H` (16.16).
    pub src: [u32; 4],
    /// `CRTC_X`, `CRTC_Y`, `CRTC_W`, `CRTC_H`.
    pub dst: (i32, i32, u32, u32),
}

/// What is shown where. See the module documentation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub crtcs: Vec<CrtcState>,
    pub planes: Vec<PlaneState>,
    /// Each connector's CRTC ID (0: none).
    pub conns: Vec<u32>,
}

impl State {
    /// Everything off, for `n` outputs.
    pub fn empty(n: usize) -> State {
        State { crtcs: vec![CrtcState::default(); n], planes: vec![PlaneState::default(); n], conns: vec![0; n] }
    }
}

/// How a commit goes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Commit {
    pub test_only: bool,
    pub allow_modeset: bool,
    pub nonblock: bool,
    /// Queue a flip-complete event with this `user_data` per CRTC named.
    pub event: Option<u64>,
    /// The CRTCs the commit names (bit `i`: CRTC `i`).
    pub touched: u32,
    /// A legacy flip's target vblank (`DRM_MODE_PAGE_FLIP_TARGET_*`).
    pub target: Option<u64>,
}

/// The `IN_FORMATS` blob: [`FORMATS`], each with the linear modifier only.
pub(crate) fn in_formats_blob() -> Vec<u8> {
    let formats_offset = FormatModifierBlob::SIZE as u32;
    let modifiers_offset = (formats_offset + 4 * FORMATS.len() as u32).next_multiple_of(8);
    let mut b = vec![0u8; modifiers_offset as usize + FormatModifier::SIZE];
    FormatModifierBlob {
        version: m::FORMAT_BLOB_CURRENT,
        flags: 0,
        count_formats: FORMATS.len() as u32,
        formats_offset,
        count_modifiers: 1,
        modifiers_offset,
    }
    .encode(&mut b);
    for (i, f) in FORMATS.iter().enumerate() {
        f.put(&mut b, formats_offset as usize + 4 * i);
    }
    let all = (1u64 << FORMATS.len()) - 1;
    FormatModifier { formats: all, offset: 0, pad: 0, modifier: fourcc::MOD_LINEAR }
        .encode(&mut b[modifiers_offset as usize..]);
    b
}

/// Write `ids` to the user array at `ptr` if `count` (its length) is big
/// enough, and set `count` to how many there are.
fn put_u32s(user: &mut dyn UserMemory, ptr: u64, count: &mut u32, ids: &[u32]) -> Result<()> {
    if *count as usize >= ids.len() && !ids.is_empty() {
        let bytes: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
        user.write(ptr, &bytes)?;
    }
    *count = ids.len() as u32;
    Ok(())
}

fn put_u64s(user: &mut dyn UserMemory, ptr: u64, count: u32, values: &[u64]) -> Result<()> {
    if count as usize >= values.len() && !values.is_empty() {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        user.write(ptr, &bytes)?;
    }
    Ok(())
}

fn read_u32s(user: &mut dyn UserMemory, ptr: u64, n: usize) -> Result<Vec<u32>> {
    let mut bytes = vec![0u8; n * 4];
    user.read(ptr, &mut bytes)?;
    Ok(bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn read_u64s(user: &mut dyn UserMemory, ptr: u64, n: usize) -> Result<Vec<u64>> {
    let mut bytes = vec![0u8; n * 8];
    user.read(ptr, &mut bytes)?;
    Ok(bytes.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap_or([0; 8]))).collect())
}

/// The bytes per pixel of a format the planes take.
fn cpp(format: u32) -> Option<u32> {
    FORMATS.contains(&format).then_some(4)
}

/// A vblank number from the 32 bits a request carries, near `current`.
fn widen(current: u64, low: u32) -> u64 {
    let diff = low.wrapping_sub(current as u32) as i32 as i64;
    (current as i64 + diff).max(0) as u64
}

impl<H: Hardware> Device<H> {
    fn crtc_index(&self, id: u32) -> Result<usize> {
        match self.objects.get(&id) {
            Some(Obj::Crtc(i)) => Ok(*i),
            _ => Err(Errno::ENOENT),
        }
    }

    /// What output `i` shows in state `s`: a framebuffer and its rectangle.
    pub(crate) fn target(&self, s: &State, i: usize) -> Option<(u32, Rect)> {
        let (c, p) = (s.crtcs.get(i)?, s.planes.get(i)?);
        if !c.active || p.fb == 0 || p.crtc != self.crtcs[i].id {
            return None;
        }
        Some((p.fb, Rect::new(p.src[0] >> 16, p.src[1] >> 16, p.src[2] >> 16, p.src[3] >> 16)))
    }

    /// Program the hardware for `new` (see the module documentation).
    pub(crate) fn apply(&mut self, new: &State, touched: u32, _now: u64) -> core::result::Result<(), HwError> {
        for i in 0..self.crtcs.len() {
            let old = self.target(&self.state, i);
            let next = self.target(new, i);
            match next {
                Some((fb, r)) => {
                    let Some(layout) = self.fbs.get(&fb).map(|f| f.layout) else { continue };
                    if !self.display_taken {
                        self.hw.take_display();
                        self.display_taken = true;
                    }
                    let changed = old != next;
                    if !changed && touched & (1 << i) == 0 {
                        continue;
                    }
                    self.hw.upload(fb, &layout, r)?;
                    if changed {
                        self.hw.scanout(i, Some((fb, r)))?;
                    }
                    self.hw.flush(fb, r)?;
                }
                None if old.is_some() => self.hw.scanout(i, None)?,
                None => {}
            }
        }
        Ok(())
    }

    /// Make `new` the state: start and stop vblank clocks, and free the
    /// mode blobs only the old state used.
    pub(crate) fn set_state(&mut self, new: State, now: u64) {
        for (i, crtc) in self.crtcs.iter_mut().enumerate() {
            let (o, n) = (&self.state.crtcs[i], &new.crtcs[i]);
            match (n.active, n.mode) {
                (true, Some(mode)) if !o.active || o.mode != n.mode => crtc.vblank.start(now, modes::frame_ns(&mode)),
                (false, _) if o.active => {
                    crtc.vblank.stop(now);
                    crtc.pending = None;
                }
                _ => {}
            }
        }
        let old = core::mem::replace(&mut self.state, new);
        for c in old.crtcs {
            let unused = c.mode_blob != 0 && !self.state.crtcs.iter().any(|n| n.mode_blob == c.mode_blob);
            if unused && self.blobs.get(&c.mode_blob).is_some_and(|b| b.owner.is_none()) {
                self.remove_blob(c.mode_blob);
            }
        }
    }

    fn needs_modeset(&self, new: &State, i: usize) -> bool {
        let id = self.crtcs[i].id;
        let (o, n) = (&self.state.crtcs[i], &new.crtcs[i]);
        o.active != n.active
            || o.mode != n.mode
            || self.state.conns.iter().zip(&new.conns).any(|(a, b)| (*a == id) != (*b == id))
    }

    /// Whether `new` is a state the device can show (see the module
    /// documentation).
    fn check(&self, new: &State) -> Result<()> {
        for (j, &crtc) in new.conns.iter().enumerate() {
            if crtc != 0 && crtc != self.crtcs[j].id {
                return Err(Errno::EINVAL);
            }
        }
        for i in 0..self.crtcs.len() {
            let id = self.crtcs[i].id;
            let (c, p) = (&new.crtcs[i], &new.planes[i]);
            if (p.fb == 0) != (p.crtc == 0) || (p.crtc != 0 && p.crtc != id) {
                return Err(Errno::EINVAL);
            }
            if p.fb != 0 {
                let fb = self.fbs.get(&p.fb).ok_or(Errno::ENOENT)?;
                let [x, y, w, h] = p.src.map(u64::from);
                if x + w > (fb.layout.width as u64) << 16 || y + h > (fb.layout.height as u64) << 16 {
                    return Err(Errno::ENOSPC);
                }
                let whole = p.src.iter().all(|v| v & 0xffff == 0);
                if !whole || (w >> 16, h >> 16) != (p.dst.2 as u64, p.dst.3 as u64) {
                    return Err(Errno::ERANGE);
                }
            }
            if let Some(mode) = &c.mode
                && !modes::valid(mode)
            {
                return Err(Errno::EINVAL);
            }
            if c.active {
                let mode = c.mode.ok_or(Errno::EINVAL)?;
                if p.fb == 0 || p.dst != (0, 0, mode.hdisplay as u32, mode.vdisplay as u32) {
                    return Err(Errno::EINVAL);
                }
                if !new.conns.contains(&id) {
                    return Err(Errno::EINVAL);
                }
            }
        }
        Ok(())
    }

    /// Check `new`, and unless it is a test, show it (see the module
    /// documentation).
    pub(crate) fn commit(&mut self, file: FileId, mut new: State, c: Commit, now: u64) -> Result<Reply> {
        self.check(&new)?;
        let n = self.crtcs.len();
        if !c.allow_modeset && (0..n).any(|i| self.needs_modeset(&new, i)) {
            return Err(Errno::EINVAL);
        }
        let named: Vec<usize> = (0..n).filter(|i| c.touched & (1 << i) != 0).collect();
        if c.event.is_some() && named.iter().any(|&i| !new.crtcs[i].active) {
            return Err(Errno::EINVAL);
        }
        if c.test_only {
            return Ok(Reply::Done(0));
        }
        if let Some(busy) = named.iter().filter_map(|&i| self.crtcs[i].pending).max() {
            if c.nonblock {
                return Err(Errno::EBUSY);
            }
            return Ok(Reply::Sleep { until_ns: busy });
        }
        if c.event.is_some()
            && self.file(file)?.event_bytes() + named.len() * uapi::event::LEN > crate::device::EVENT_SPACE
        {
            return Err(Errno::ENOMEM);
        }
        // A mode set without a blob of its own (legacy SETCRTC) gets one
        // the device owns, for MODE_ID to read.
        for i in 0..n {
            if let Some(mode) = new.crtcs[i].mode
                && new.crtcs[i].mode_blob == 0
            {
                new.crtcs[i].mode_blob = self.add_blob(mode.to_bytes().to_vec(), None);
            }
        }
        if self.apply(&new, c.touched, now).is_err() {
            // Whatever the hardware took stays; the state is what was asked.
            self.set_state(new, now);
            return Err(Errno::EIO);
        }
        self.set_state(new, now);
        if c.event.is_some() || c.nonblock {
            for &i in &named {
                if !self.state.crtcs[i].active {
                    continue;
                }
                let clock = self.crtcs[i].vblank;
                let (mut seq, mut due) = clock.next(now);
                if let Some(t) = c.target
                    && t > seq
                {
                    (seq, due) = (t, clock.time_of(t));
                }
                self.crtcs[i].pending = Some(due);
                if let Some(user_data) = c.event {
                    let crtc_id = self.crtcs[i].id;
                    let e = vblank_event(uapi::event::FLIP_COMPLETE, user_data, due, seq, crtc_id);
                    self.file_mut(file)?.queue(e);
                }
            }
        }
        Ok(Reply::Done(0))
    }

    /// Take framebuffer `id` off every plane (turning their CRTCs off) and
    /// destroy it.
    pub(crate) fn remove_fb(&mut self, id: u32, now: u64) {
        let using: Vec<usize> = (0..self.crtcs.len()).filter(|&i| self.state.planes[i].fb == id).collect();
        if !using.is_empty() {
            let mut new = self.state.clone();
            for &i in &using {
                let crtc = self.crtcs[i].id;
                new.planes[i] = PlaneState::default();
                new.crtcs[i] = CrtcState::default();
                for c in new.conns.iter_mut().filter(|c| **c == crtc) {
                    *c = 0;
                }
            }
            let _ = self.apply(&new, 0, now);
            self.set_state(new, now);
        }
        if let Some(fb) = self.fbs.remove(&id) {
            self.objects.remove(&id);
            if let Some(buffer) = self.bos.get(&fb.bo).and_then(|bo| bo.buffer.as_ref()) {
                // On failure the hardware keeps the memory (its contract).
                let _ = self.hw.destroy_fb(id, buffer);
            }
            self.unref(fb.bo);
        }
    }

    pub(crate) fn kms_ioctl(
        &mut self,
        file: FileId,
        n: u8,
        b: &mut [u8],
        ctx: &mut Ctx<'_, H::Buffer>,
    ) -> Result<Reply> {
        let done = |r: Result<()>| r.map(|()| Reply::Done(0));
        let now = ctx.now_ns;
        match n {
            nr::MODE_GETRESOURCES => done(self.get_resources(file, b, ctx.user)),
            nr::MODE_GETCRTC => done(self.get_crtc(b)),
            nr::MODE_SETCRTC => self.set_crtc(file, b, ctx.user, now),
            nr::MODE_CURSOR | nr::MODE_CURSOR2 => Err(Errno::ENXIO),
            nr::MODE_GETGAMMA | nr::MODE_SETGAMMA => {
                let lut = CrtcLut::decode(b);
                self.crtc_index(lut.crtc_id)?;
                Err(Errno::EINVAL)
            }
            nr::MODE_GETENCODER => done(self.get_encoder(b)),
            nr::MODE_GETCONNECTOR => done(self.get_connector(file, b, ctx.user)),
            nr::MODE_GETPROPERTY => done(self.get_property(b, ctx.user)),
            nr::MODE_SETPROPERTY => {
                let s = uapi::ConnectorSetProperty::decode(b);
                self.set_object_property(file, s.connector_id, m::OBJECT_CONNECTOR, s.prop_id, s.value, now)
            }
            nr::MODE_GETPROPBLOB => {
                let mut g = GetBlob::decode(b);
                let blob = self.blobs.get(&g.blob_id).ok_or(Errno::ENOENT)?;
                if g.length as usize >= blob.data.len() && g.data != 0 && !blob.data.is_empty() {
                    ctx.user.write(g.data, &blob.data)?;
                }
                g.length = blob.data.len() as u32;
                g.encode(b);
                Ok(Reply::Done(0))
            }
            nr::MODE_GETFB => done(self.get_fb(file, b, ctx.privileged)),
            nr::MODE_GETFB2 => done(self.get_fb2(file, b, ctx.privileged)),
            nr::MODE_ADDFB => done(self.add_fb(file, b)),
            nr::MODE_ADDFB2 => done(self.add_fb2(file, b)),
            nr::MODE_RMFB => {
                let id: u32 = Field::get(b, 0);
                match self.fbs.get(&id) {
                    Some(fb) if fb.owner == file => {}
                    _ => return Err(Errno::ENOENT),
                }
                self.remove_fb(id, now);
                Ok(Reply::Done(0))
            }
            nr::MODE_PAGE_FLIP => self.page_flip(file, b, now),
            nr::MODE_DIRTYFB => done(self.dirty_fb(b, ctx.user)),
            nr::MODE_GETPLANERESOURCES => {
                let mut r = GetPlaneRes::decode(b);
                let ids = if self.file(file)?.caps.universal { self.planes.clone() } else { Vec::new() };
                put_u32s(ctx.user, r.plane_id_ptr, &mut r.count_planes, &ids)?;
                r.encode(b);
                Ok(Reply::Done(0))
            }
            nr::MODE_GETPLANE => done(self.get_plane(b, ctx.user)),
            nr::MODE_SETPLANE => self.set_plane(file, b, now),
            nr::MODE_OBJ_GETPROPERTIES => done(self.obj_get_properties(file, b, ctx.user)),
            nr::MODE_OBJ_SETPROPERTY => {
                let s = ObjSetProperty::decode(b);
                self.set_object_property(file, s.obj_id, s.obj_type, s.prop_id, s.value, now)
            }
            nr::MODE_ATOMIC => self.atomic(file, b, ctx.user, now),
            nr::MODE_CREATEPROPBLOB => {
                let mut c = CreateBlob::decode(b);
                if c.length == 0 || c.length as usize > MAX_BLOB {
                    return Err(Errno::EINVAL);
                }
                let mut data = vec![0u8; c.length as usize];
                ctx.user.read(c.data, &mut data)?;
                c.blob_id = self.add_blob(data, Some(file));
                c.encode(b);
                Ok(Reply::Done(0))
            }
            nr::MODE_DESTROYPROPBLOB => {
                let id: u32 = Field::get(b, 0);
                match self.blobs.get(&id).map(|b| b.owner) {
                    None => return Err(Errno::ENOENT),
                    Some(owner) if owner != Some(file) => return Err(Errno::EPERM),
                    _ => {}
                }
                self.drop_blob(id);
                Ok(Reply::Done(0))
            }
            nr::WAIT_VBLANK => self.wait_vblank(file, b, now),
            nr::CRTC_GET_SEQUENCE => {
                let mut s = CrtcGetSequence::decode(b);
                let i = self.crtc_index(s.crtc_id)?;
                let clock = self.crtcs[i].vblank;
                s.active = self.state.crtcs[i].active as u32;
                s.sequence = clock.seq(now);
                s.sequence_ns = clock.time_of(s.sequence) as i64;
                s.encode(b);
                Ok(Reply::Done(0))
            }
            nr::CRTC_QUEUE_SEQUENCE => done(self.queue_sequence(file, b, now)),
            _ => Err(Errno::EINVAL),
        }
    }

    fn get_resources(&self, file: FileId, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut r = CardRes::decode(b);
        let fbs: Vec<u32> = self.fbs.iter().filter(|(_, f)| f.owner == file).map(|(id, _)| *id).collect();
        let crtcs: Vec<u32> = self.crtcs.iter().map(|c| c.id).collect();
        let connectors: Vec<u32> = self.connectors.iter().map(|c| c.id).collect();
        put_u32s(user, r.fb_id_ptr, &mut r.count_fbs, &fbs)?;
        put_u32s(user, r.crtc_id_ptr, &mut r.count_crtcs, &crtcs)?;
        put_u32s(user, r.connector_id_ptr, &mut r.count_connectors, &connectors)?;
        put_u32s(user, r.encoder_id_ptr, &mut r.count_encoders, &self.encoders)?;
        (r.min_width, r.max_width, r.min_height, r.max_height) =
            (modes::MIN_SIZE, modes::MAX_SIZE, modes::MIN_SIZE, modes::MAX_SIZE);
        r.encode(b);
        Ok(())
    }

    fn get_crtc(&self, b: &mut [u8]) -> Result<()> {
        let mut c = ModeCrtc::decode(b);
        let i = self.crtc_index(c.crtc_id)?;
        let (s, p) = (&self.state.crtcs[i], &self.state.planes[i]);
        let shown = p.crtc == c.crtc_id;
        c.fb_id = if shown { p.fb } else { 0 };
        c.x = if shown { p.src[0] >> 16 } else { 0 };
        c.y = if shown { p.src[1] >> 16 } else { 0 };
        c.gamma_size = 0;
        c.mode_valid = s.mode.is_some() as u32;
        c.mode = s.mode.unwrap_or_default();
        c.encode(b);
        Ok(())
    }

    fn set_crtc(&mut self, file: FileId, b: &mut [u8], user: &mut dyn UserMemory, now: u64) -> Result<Reply> {
        let c = ModeCrtc::decode(b);
        let i = self.crtc_index(c.crtc_id)?;
        let mut new = self.state.clone();
        for conn in new.conns.iter_mut().filter(|x| **x == c.crtc_id) {
            *conn = 0;
        }
        if c.mode_valid == 0 {
            if c.count_connectors != 0 || (c.fb_id != 0 && c.fb_id != u32::MAX) {
                return Err(Errno::EINVAL);
            }
            new.crtcs[i] = CrtcState::default();
            new.planes[i] = PlaneState::default();
        } else {
            let mode = ModeInfo { ty: c.mode.ty & !m::TYPE_PREFERRED, ..c.mode };
            if !modes::valid(&mode) || c.count_connectors == 0 {
                return Err(Errno::EINVAL);
            }
            if c.count_connectors as usize > self.connectors.len() {
                return Err(Errno::EINVAL);
            }
            let fb = if c.fb_id == u32::MAX { self.state.planes[i].fb } else { c.fb_id };
            if fb == 0 {
                return Err(Errno::EINVAL);
            }
            let f = self.fbs.get(&fb).ok_or(Errno::ENOENT)?;
            let (w, h) = (mode.hdisplay as u32, mode.vdisplay as u32);
            if c.x as u64 + w as u64 > f.layout.width as u64 || c.y as u64 + h as u64 > f.layout.height as u64 {
                return Err(Errno::ENOSPC);
            }
            for id in read_u32s(user, c.set_connectors_ptr, c.count_connectors as usize)? {
                let j = match self.objects.get(&id) {
                    Some(Obj::Connector(j)) => *j,
                    _ => return Err(Errno::ENOENT),
                };
                if j != i {
                    return Err(Errno::EINVAL);
                }
                new.conns[j] = c.crtc_id;
            }
            let keep_blob = self.state.crtcs[i].mode == Some(mode);
            new.crtcs[i] = CrtcState {
                active: true,
                mode: Some(mode),
                mode_blob: if keep_blob { self.state.crtcs[i].mode_blob } else { 0 },
            };
            new.planes[i] =
                PlaneState { fb, crtc: c.crtc_id, src: [c.x << 16, c.y << 16, w << 16, h << 16], dst: (0, 0, w, h) };
        }
        let commit = Commit { allow_modeset: true, touched: 1 << i, ..Commit::default() };
        self.commit(file, new, commit, now)
    }

    fn get_encoder(&self, b: &mut [u8]) -> Result<()> {
        let mut e = GetEncoder::decode(b);
        let i = match self.objects.get(&e.encoder_id) {
            Some(Obj::Encoder(i)) => *i,
            _ => return Err(Errno::ENOENT),
        };
        e.encoder_type = m::ENCODER_VIRTUAL;
        e.crtc_id = self.state.conns[i];
        e.possible_crtcs = 1 << i;
        e.possible_clones = 0;
        e.encode(b);
        Ok(())
    }

    /// The properties of `obj` a file with `atomic` sees, with their
    /// values now.
    fn properties(&self, obj: Obj, atomic: bool) -> Vec<(u32, u64)> {
        let list: &[Prop] = match obj {
            Obj::Connector(_) => &Prop::CONNECTOR,
            Obj::Crtc(_) => &Prop::CRTC,
            Obj::Plane(_) => &Prop::PLANE,
            _ => &[],
        };
        list.iter()
            .filter(|p| atomic || !p.atomic())
            .map(|&p| (self.prop_ids[p.index()], self.property_value(obj, p)))
            .collect()
    }

    fn property_value(&self, obj: Obj, p: Prop) -> u64 {
        let s = &self.state;
        match obj {
            Obj::Connector(j) => match p {
                Prop::Edid => self.connectors[j].edid_blob as u64,
                Prop::Dpms => self.connectors[j].dpms,
                Prop::LinkStatus => m::LINK_STATUS_GOOD,
                Prop::NonDesktop => 0,
                Prop::CrtcId => s.conns[j] as u64,
                _ => 0,
            },
            Obj::Crtc(i) => match p {
                Prop::Active => s.crtcs[i].active as u64,
                Prop::ModeId => s.crtcs[i].mode_blob as u64,
                _ => 0,
            },
            Obj::Plane(i) => {
                let pl = &s.planes[i];
                match p {
                    Prop::Type => m::PLANE_PRIMARY,
                    Prop::FbId => pl.fb as u64,
                    Prop::CrtcId => pl.crtc as u64,
                    Prop::SrcX => pl.src[0] as u64,
                    Prop::SrcY => pl.src[1] as u64,
                    Prop::SrcW => pl.src[2] as u64,
                    Prop::SrcH => pl.src[3] as u64,
                    Prop::CrtcX => pl.dst.0 as i64 as u64,
                    Prop::CrtcY => pl.dst.1 as i64 as u64,
                    Prop::CrtcW => pl.dst.2 as u64,
                    Prop::CrtcH => pl.dst.3 as u64,
                    Prop::InFormats => self.in_formats as u64,
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    fn get_connector(&mut self, file: FileId, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut g = GetConnector::decode(b);
        let j = match self.objects.get(&g.connector_id) {
            Some(Obj::Connector(j)) => *j,
            _ => return Err(Errno::ENOENT),
        };
        if g.count_modes == 0 {
            // A probe: ask the hardware again (a display may have come or gone).
            let outputs = self.hw.probe();
            self.apply_outputs(outputs);
        }
        let atomic = self.file(file)?.caps.atomic;
        let c = &self.connectors[j];
        if g.count_modes as usize >= c.modes.len() && !c.modes.is_empty() {
            let bytes: Vec<u8> = c.modes.iter().flat_map(|mode| mode.to_bytes()).collect();
            user.write(g.modes_ptr, &bytes)?;
        }
        g.count_modes = c.modes.len() as u32;
        put_u32s(user, g.encoders_ptr, &mut g.count_encoders, &[self.encoders[j]])?;
        let props = self.properties(Obj::Connector(j), atomic);
        let ids: Vec<u32> = props.iter().map(|p| p.0).collect();
        let values: Vec<u64> = props.iter().map(|p| p.1).collect();
        put_u64s(user, g.prop_values_ptr, g.count_props, &values)?;
        put_u32s(user, g.props_ptr, &mut g.count_props, &ids)?;
        let c = &self.connectors[j];
        g.encoder_id = if self.state.conns[j] != 0 { self.encoders[j] } else { 0 };
        g.connector_type = m::CONNECTOR_VIRTUAL;
        g.connector_type_id = j as u32 + 1;
        g.connection = if c.output.connected { m::CONNECTED } else { m::DISCONNECTED };
        (g.mm_width, g.mm_height) = c.mm;
        g.subpixel = m::SUBPIXEL_UNKNOWN;
        g.pad = 0;
        g.encode(b);
        Ok(())
    }

    fn get_property(&self, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut g = GetProperty::decode(b);
        let p = match self.objects.get(&g.prop_id) {
            Some(Obj::Property(p)) => *p,
            _ => return Err(Errno::ENOENT),
        };
        g.flags = p.flags();
        g.name = name_bytes(p.name());
        let (values, enums): (Vec<u64>, &[(u64, &str)]) = match p.kind() {
            Kind::Range(lo, hi) => (vec![lo, hi], &[]),
            Kind::SignedRange(lo, hi) => (vec![lo as u64, hi as u64], &[]),
            Kind::Enum(e) => (e.iter().map(|(v, _)| *v).collect(), e),
            Kind::Blob => (Vec::new(), &[]),
            Kind::Object(ty) => (vec![ty as u64], &[]),
        };
        put_u64s(user, g.values_ptr, g.count_values, &values)?;
        g.count_values = values.len() as u32;
        if g.count_enum_blobs as usize >= enums.len() && !enums.is_empty() {
            let bytes: Vec<u8> = enums
                .iter()
                .flat_map(|(v, name)| PropertyEnum { value: *v, name: name_bytes(name) }.to_bytes())
                .collect();
            user.write(g.enum_blob_ptr, &bytes)?;
        }
        g.count_enum_blobs = enums.len() as u32;
        g.encode(b);
        Ok(())
    }

    fn get_plane(&self, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut g = GetPlane::decode(b);
        let i = match self.objects.get(&g.plane_id) {
            Some(Obj::Plane(i)) => *i,
            _ => return Err(Errno::ENOENT),
        };
        let p = &self.state.planes[i];
        g.crtc_id = p.crtc;
        g.fb_id = p.fb;
        g.possible_crtcs = 1 << i;
        g.gamma_size = 0;
        put_u32s(user, g.format_type_ptr, &mut g.count_format_types, &FORMATS)?;
        g.encode(b);
        Ok(())
    }

    fn obj_get_properties(&self, file: FileId, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let mut g = ObjGetProperties::decode(b);
        let obj = *self.objects.get(&g.obj_id).ok_or(Errno::ENOENT)?;
        if g.obj_type != m::OBJECT_ANY && g.obj_type != obj.type_code() {
            return Err(Errno::ENOENT);
        }
        let props = self.properties(obj, self.file(file)?.caps.atomic);
        let ids: Vec<u32> = props.iter().map(|p| p.0).collect();
        let values: Vec<u64> = props.iter().map(|p| p.1).collect();
        put_u64s(user, g.prop_values_ptr, g.count_props, &values)?;
        put_u32s(user, g.props_ptr, &mut g.count_props, &ids)?;
        g.encode(b);
        Ok(())
    }

    /// Set property `prop` of `obj` to `value` in `new`.
    fn set_prop(&self, new: &mut State, obj: Obj, prop: Prop, value: u64) -> Result<()> {
        let attached = match obj {
            Obj::Connector(_) => Prop::CONNECTOR.contains(&prop),
            Obj::Crtc(_) => Prop::CRTC.contains(&prop),
            Obj::Plane(_) => Prop::PLANE.contains(&prop),
            _ => false,
        };
        if !attached || prop.immutable() || !prop.accepts(value) {
            return Err(Errno::EINVAL);
        }
        let v32 = value as u32;
        let crtc_or_none = |v: u32| v == 0 || matches!(self.objects.get(&v), Some(Obj::Crtc(_)));
        match (obj, prop) {
            (Obj::Plane(i), Prop::FbId) => {
                if v32 != 0 && !self.fbs.contains_key(&v32) {
                    return Err(Errno::ENOENT);
                }
                new.planes[i].fb = v32;
            }
            (Obj::Plane(i), Prop::CrtcId) => {
                if !crtc_or_none(v32) {
                    return Err(Errno::ENOENT);
                }
                new.planes[i].crtc = v32;
            }
            (Obj::Plane(i), Prop::SrcX) => new.planes[i].src[0] = v32,
            (Obj::Plane(i), Prop::SrcY) => new.planes[i].src[1] = v32,
            (Obj::Plane(i), Prop::SrcW) => new.planes[i].src[2] = v32,
            (Obj::Plane(i), Prop::SrcH) => new.planes[i].src[3] = v32,
            (Obj::Plane(i), Prop::CrtcX) => new.planes[i].dst.0 = value as i64 as i32,
            (Obj::Plane(i), Prop::CrtcY) => new.planes[i].dst.1 = value as i64 as i32,
            (Obj::Plane(i), Prop::CrtcW) => new.planes[i].dst.2 = v32,
            (Obj::Plane(i), Prop::CrtcH) => new.planes[i].dst.3 = v32,
            (Obj::Crtc(i), Prop::Active) => new.crtcs[i].active = value == 1,
            (Obj::Crtc(i), Prop::ModeId) => {
                if v32 == 0 {
                    new.crtcs[i].mode = None;
                    new.crtcs[i].mode_blob = 0;
                } else {
                    let blob = self.blobs.get(&v32).ok_or(Errno::EINVAL)?;
                    if blob.data.len() != ModeInfo::SIZE {
                        return Err(Errno::EINVAL);
                    }
                    let mode = ModeInfo::decode(&blob.data);
                    if !modes::valid(&mode) {
                        return Err(Errno::EINVAL);
                    }
                    new.crtcs[i].mode = Some(mode);
                    new.crtcs[i].mode_blob = v32;
                }
            }
            (Obj::Connector(j), Prop::CrtcId) => {
                if !crtc_or_none(v32) {
                    return Err(Errno::ENOENT);
                }
                new.conns[j] = v32;
            }
            _ => return Err(Errno::EINVAL),
        }
        Ok(())
    }

    /// The CRTCs a change to `obj` concerns, in `old` and `new` (bits).
    fn concerned(&self, obj: Obj, old: &State, new: &State) -> u32 {
        let bit = |id: u32| match self.objects.get(&id) {
            Some(Obj::Crtc(i)) => 1 << i,
            _ => 0,
        };
        match obj {
            Obj::Crtc(i) => 1 << i,
            Obj::Plane(i) => bit(old.planes[i].crtc) | bit(new.planes[i].crtc) | (1 << i),
            Obj::Connector(j) => bit(old.conns[j]) | bit(new.conns[j]),
            _ => 0,
        }
    }

    /// `SETPROPERTY` and `OBJ_SETPROPERTY`: `DPMS` (which turns the
    /// connector's CRTC on or off), or any atomic property, committed alone
    /// with modesets allowed.
    fn set_object_property(
        &mut self,
        file: FileId,
        obj_id: u32,
        ty: u32,
        prop_id: u32,
        value: u64,
        now: u64,
    ) -> Result<Reply> {
        let obj = *self.objects.get(&obj_id).ok_or(Errno::ENOENT)?;
        if ty != m::OBJECT_ANY && ty != obj.type_code() {
            return Err(Errno::ENOENT);
        }
        let prop = match self.objects.get(&prop_id) {
            Some(Obj::Property(p)) => *p,
            _ => return Err(Errno::ENOENT),
        };
        let mut new = self.state.clone();
        if let (Obj::Connector(j), Prop::Dpms) = (obj, prop) {
            if !prop.accepts(value) {
                return Err(Errno::EINVAL);
            }
            self.connectors[j].dpms = value;
            let crtc = self.state.conns[j];
            let Ok(i) = self.crtc_index(crtc) else { return Ok(Reply::Done(0)) };
            new.crtcs[i].active = value == m::DPMS_ON && new.crtcs[i].mode.is_some();
            let commit = Commit { allow_modeset: true, touched: 1 << i, ..Commit::default() };
            return self.commit(file, new, commit, now);
        }
        self.set_prop(&mut new, obj, prop, value)?;
        let touched = self.concerned(obj, &self.state, &new);
        self.commit(file, new, Commit { allow_modeset: true, touched, ..Commit::default() }, now)
    }

    fn atomic(&mut self, file: FileId, b: &mut [u8], user: &mut dyn UserMemory, now: u64) -> Result<Reply> {
        let a = Atomic::decode(b);
        if !self.file(file)?.caps.atomic || a.flags & !m::ATOMIC_FLAGS != 0 || a.reserved != 0 {
            return Err(Errno::EINVAL);
        }
        if a.flags & m::PAGE_FLIP_ASYNC != 0 {
            return Err(Errno::EINVAL);
        }
        if a.flags & m::ATOMIC_TEST_ONLY != 0 && a.flags & m::PAGE_FLIP_EVENT != 0 {
            return Err(Errno::EINVAL);
        }
        let count = a.count_objs as usize;
        if count > self.objects.len() {
            return Err(Errno::EINVAL);
        }
        let objs = read_u32s(user, a.objs_ptr, count)?;
        let counts = read_u32s(user, a.count_props_ptr, count)?;
        let total: usize = counts.iter().map(|&c| c as usize).sum();
        if total > MAX_ATOMIC_PROPS {
            return Err(Errno::EINVAL);
        }
        let props = read_u32s(user, a.props_ptr, total)?;
        let values = read_u64s(user, a.prop_values_ptr, total)?;
        let mut new = self.state.clone();
        let mut touched = 0;
        let mut at = 0;
        for (&obj_id, &n) in objs.iter().zip(&counts) {
            let obj = match self.objects.get(&obj_id) {
                Some(o @ (Obj::Crtc(_) | Obj::Connector(_) | Obj::Plane(_))) => *o,
                _ => return Err(Errno::ENOENT),
            };
            let before = new.clone();
            for k in at..at + n as usize {
                let prop = match self.objects.get(&props[k]) {
                    Some(Obj::Property(p)) => *p,
                    _ => return Err(Errno::ENOENT),
                };
                self.set_prop(&mut new, obj, prop, values[k])?;
            }
            at += n as usize;
            touched |= self.concerned(obj, &before, &new) | self.concerned(obj, &self.state, &new);
        }
        let commit = Commit {
            test_only: a.flags & m::ATOMIC_TEST_ONLY != 0,
            allow_modeset: a.flags & m::ATOMIC_ALLOW_MODESET != 0,
            nonblock: a.flags & m::ATOMIC_NONBLOCK != 0,
            event: (a.flags & m::PAGE_FLIP_EVENT != 0).then_some(a.user_data),
            touched,
            target: None,
        };
        self.commit(file, new, commit, now)
    }

    fn page_flip(&mut self, file: FileId, b: &mut [u8], now: u64) -> Result<Reply> {
        let p = PageFlip::decode(b);
        let target_flags = m::PAGE_FLIP_TARGET_ABSOLUTE | m::PAGE_FLIP_TARGET_RELATIVE;
        if p.flags & !m::PAGE_FLIP_FLAGS != 0 || p.flags & m::PAGE_FLIP_ASYNC != 0 {
            return Err(Errno::EINVAL);
        }
        if p.flags & target_flags == target_flags || (p.flags & target_flags == 0 && p.sequence != 0) {
            return Err(Errno::EINVAL);
        }
        let i = self.crtc_index(p.crtc_id)?;
        let cur = &self.state;
        if !cur.crtcs[i].active || cur.planes[i].fb == 0 {
            return Err(Errno::EINVAL);
        }
        let new_fb = self.fbs.get(&p.fb_id).ok_or(Errno::ENOENT)?;
        let old_fb = self.fbs.get(&cur.planes[i].fb).ok_or(Errno::EINVAL)?;
        if new_fb.layout.format != old_fb.layout.format {
            return Err(Errno::EINVAL);
        }
        if self.crtcs[i].pending.is_some() {
            return Err(Errno::EBUSY);
        }
        let clock = self.crtcs[i].vblank;
        let current = clock.seq(now);
        let target = match p.flags & target_flags {
            m::PAGE_FLIP_TARGET_ABSOLUTE => Some(widen(current, p.sequence)),
            m::PAGE_FLIP_TARGET_RELATIVE => Some(current + p.sequence as u64),
            _ => None,
        };
        let mut new = self.state.clone();
        new.planes[i].fb = p.fb_id;
        let commit = Commit {
            nonblock: true,
            event: (p.flags & m::PAGE_FLIP_EVENT != 0).then_some(p.user_data),
            touched: 1 << i,
            target,
            ..Commit::default()
        };
        self.commit(file, new, commit, now)
    }

    fn set_plane(&mut self, file: FileId, b: &mut [u8], now: u64) -> Result<Reply> {
        let s = SetPlane::decode(b);
        let i = match self.objects.get(&s.plane_id) {
            Some(Obj::Plane(i)) => *i,
            _ => return Err(Errno::ENOENT),
        };
        if s.flags != 0 {
            return Err(Errno::EINVAL);
        }
        let mut new = self.state.clone();
        if s.fb_id == 0 {
            new.planes[i] = PlaneState::default();
        } else {
            let c = self.crtc_index(s.crtc_id)?;
            if c != i {
                return Err(Errno::EINVAL);
            }
            if !self.fbs.contains_key(&s.fb_id) {
                return Err(Errno::ENOENT);
            }
            new.planes[i] = PlaneState {
                fb: s.fb_id,
                crtc: s.crtc_id,
                src: [s.src_x, s.src_y, s.src_w, s.src_h],
                dst: (s.crtc_x, s.crtc_y, s.crtc_w, s.crtc_h),
            };
        }
        self.commit(file, new, Commit { touched: 1 << i, ..Commit::default() }, now)
    }

    fn add_fb(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut c = FbCmd::decode(b);
        let format = match (c.bpp, c.depth) {
            (32, 24) => fourcc::XRGB8888,
            (32, 32) => fourcc::ARGB8888,
            _ => return Err(Errno::EINVAL),
        };
        let bo = self.lookup(file, c.handle)?;
        let layout = FbLayout { width: c.width, height: c.height, pitch: c.pitch, offset: 0, format };
        c.fb_id = self.create_fb(file, layout, bo, 0, fourcc::MOD_LINEAR, (c.bpp, c.depth))?;
        c.encode(b);
        Ok(())
    }

    fn add_fb2(&mut self, file: FileId, b: &mut [u8]) -> Result<()> {
        let mut c = FbCmd2::decode(b);
        if c.flags & !(m::FB_INTERLACED | m::FB_MODIFIERS) != 0 || c.flags & m::FB_INTERLACED != 0 {
            return Err(Errno::EINVAL);
        }
        if cpp(c.pixel_format).is_none() {
            return Err(Errno::EINVAL);
        }
        let modifiers = c.flags & m::FB_MODIFIERS != 0;
        if modifiers && c.modifier[0] != fourcc::MOD_LINEAR || !modifiers && c.modifier.iter().any(|&x| x != 0) {
            return Err(Errno::EINVAL);
        }
        // One plane: the others' fields are unused and must be 0.
        if c.handles[1..].iter().chain(&c.pitches[1..]).chain(&c.offsets[1..]).any(|&x| x != 0) {
            return Err(Errno::EINVAL);
        }
        let bo = self.lookup(file, c.handles[0])?;
        let layout = FbLayout {
            width: c.width,
            height: c.height,
            pitch: c.pitches[0],
            offset: c.offsets[0] as u64,
            format: c.pixel_format,
        };
        let depth = if c.pixel_format == fourcc::XRGB8888 || c.pixel_format == fourcc::XBGR8888 { 24 } else { 32 };
        c.fb_id = self.create_fb(file, layout, bo, c.flags, fourcc::MOD_LINEAR, (32, depth))?;
        c.encode(b);
        Ok(())
    }

    fn create_fb(
        &mut self,
        file: FileId,
        layout: FbLayout,
        bo: crate::BoId,
        flags: u32,
        modifier: u64,
        (bpp, depth): (u32, u32),
    ) -> Result<u32> {
        let cpp = cpp(layout.format).ok_or(Errno::EINVAL)?;
        let size_ok = |s: u32| (modes::MIN_SIZE..=modes::MAX_SIZE).contains(&s);
        if !size_ok(layout.width) || !size_ok(layout.height) {
            return Err(Errno::EINVAL);
        }
        if !layout.pitch.is_multiple_of(cpp) || (layout.pitch as u64) < layout.width as u64 * cpp as u64 {
            return Err(Errno::EINVAL);
        }
        let o = self.bos.get(&bo).ok_or(Errno::ENOENT)?;
        if layout.offset + layout.pitch as u64 * layout.height as u64 > o.size {
            return Err(Errno::EINVAL);
        }
        let buffer = o.buffer.clone().ok_or(Errno::EINVAL)?;
        let id = self.add_object(Obj::Fb);
        if self.hw.create_fb(id, &layout, &buffer).is_err() {
            self.objects.remove(&id);
            return Err(Errno::EIO);
        }
        if let Some(o) = self.bos.get_mut(&bo) {
            o.refs += 1;
        }
        self.fbs.insert(id, Fb { owner: file, layout, bo, flags, modifier, bpp, depth });
        Ok(id)
    }

    /// The handle `GETFB` hands out: the master's and a privileged
    /// caller's only (0 otherwise).
    fn fb_handle(&mut self, file: FileId, fb: u32, privileged: bool) -> Result<u32> {
        let bo = self.fbs.get(&fb).ok_or(Errno::ENOENT)?.bo;
        if self.master == Some(file) || privileged { self.handle_for(file, bo) } else { Ok(0) }
    }

    fn get_fb(&mut self, file: FileId, b: &mut [u8], privileged: bool) -> Result<()> {
        let mut c = FbCmd::decode(b);
        let handle = self.fb_handle(file, c.fb_id, privileged)?;
        let fb = self.fbs.get(&c.fb_id).ok_or(Errno::ENOENT)?;
        (c.width, c.height, c.pitch, c.bpp, c.depth) =
            (fb.layout.width, fb.layout.height, fb.layout.pitch, fb.bpp, fb.depth);
        c.handle = handle;
        c.encode(b);
        Ok(())
    }

    fn get_fb2(&mut self, file: FileId, b: &mut [u8], privileged: bool) -> Result<()> {
        let mut c = FbCmd2::decode(b);
        let handle = self.fb_handle(file, c.fb_id, privileged)?;
        let fb = self.fbs.get(&c.fb_id).ok_or(Errno::ENOENT)?;
        c.width = fb.layout.width;
        c.height = fb.layout.height;
        c.pixel_format = fb.layout.format;
        c.flags = fb.flags & m::FB_MODIFIERS;
        c.handles = [handle, 0, 0, 0];
        c.pitches = [fb.layout.pitch, 0, 0, 0];
        c.offsets = [fb.layout.offset as u32, 0, 0, 0];
        c.modifier = [if c.flags != 0 { fb.modifier } else { 0 }, 0, 0, 0];
        c.encode(b);
        Ok(())
    }

    /// `DIRTYFB`: upload and flush the clips (all of it without clips) on
    /// the outputs that show the framebuffer.
    fn dirty_fb(&mut self, b: &mut [u8], user: &mut dyn UserMemory) -> Result<()> {
        let d = FbDirty::decode(b);
        let layout = self.fbs.get(&d.fb_id).ok_or(Errno::ENOENT)?.layout;
        if d.flags & !m::FB_DIRTY_FLAGS != 0 || d.num_clips > m::FB_DIRTY_MAX_CLIPS {
            return Err(Errno::EINVAL);
        }
        let clips: Vec<Rect> = if d.num_clips == 0 {
            vec![Rect::new(0, 0, layout.width, layout.height)]
        } else {
            let mut bytes = vec![0u8; d.num_clips as usize * ClipRect::SIZE];
            user.read(d.clips_ptr, &mut bytes)?;
            bytes
                .chunks_exact(ClipRect::SIZE)
                .map(ClipRect::decode)
                .filter(|c| c.x2 > c.x1 && c.y2 > c.y1)
                .map(|c| Rect::new(c.x1 as u32, c.y1 as u32, (c.x2 - c.x1) as u32, (c.y2 - c.y1) as u32))
                .collect()
        };
        for i in 0..self.crtcs.len() {
            let Some((fb, shown)) = self.target(&self.state, i) else { continue };
            if fb != d.fb_id {
                continue;
            }
            for clip in &clips {
                let r = intersect(clip, &shown);
                if r.is_empty() {
                    continue;
                }
                self.hw.upload(fb, &layout, r).map_err(|_| Errno::EIO)?;
                self.hw.flush(fb, r).map_err(|_| Errno::EIO)?;
            }
        }
        Ok(())
    }

    /// `WAIT_VBLANK` (see the crate documentation on time).
    fn wait_vblank(&mut self, file: FileId, b: &mut [u8], now: u64) -> Result<Reply> {
        let mut w = WaitVblank::decode(b);
        let valid = vblank::TYPES_MASK | vblank::FLAGS_MASK | vblank::HIGH_CRTC_MASK;
        if w.ty & !valid != 0 || w.ty & vblank::SIGNAL != 0 {
            return Err(Errno::EINVAL);
        }
        let i = match (w.ty & vblank::HIGH_CRTC_MASK) >> vblank::HIGH_CRTC_SHIFT {
            0 if w.ty & vblank::SECONDARY != 0 => 1,
            n => n as usize,
        };
        if i >= self.crtcs.len() || !self.state.crtcs[i].active {
            return Err(Errno::EINVAL);
        }
        let clock: VblankClock = self.crtcs[i].vblank;
        let current = clock.seq(now);
        let mut target =
            if w.ty & vblank::RELATIVE != 0 { current + w.sequence as u64 } else { widen(current, w.sequence) };
        if w.ty & vblank::NEXTONMISS != 0 && target <= current {
            target = current + 1;
        }
        let passed = target <= current;
        if w.ty & vblank::EVENT != 0 {
            let f = self.file(file)?;
            if f.event_bytes() + uapi::event::LEN > crate::device::EVENT_SPACE {
                return Err(Errno::ENOMEM);
            }
            let (seq, due) = if passed { (current, now) } else { (target, clock.time_of(target)) };
            let stamp = if passed { clock.time_of(current) } else { due };
            let mut e = vblank_event(uapi::event::VBLANK, w.signal, stamp, seq, self.crtcs[i].id);
            e.due = due;
            self.file_mut(file)?.queue(e);
            w.sequence = target as u32;
            set_reply_time(&mut w, now);
            w.encode(b);
            return Ok(Reply::Done(0));
        }
        if !passed {
            // Wait, then answer the same request made absolute.
            w.ty = (w.ty & !(vblank::RELATIVE | vblank::NEXTONMISS)) | vblank::ABSOLUTE;
            w.sequence = target as u32;
            w.encode(b);
            return Ok(Reply::Sleep { until_ns: clock.time_of(target) });
        }
        w.sequence = current as u32;
        set_reply_time(&mut w, clock.time_of(current));
        w.encode(b);
        Ok(Reply::Done(0))
    }

    fn queue_sequence(&mut self, file: FileId, b: &mut [u8], now: u64) -> Result<()> {
        let mut q = CrtcQueueSequence::decode(b);
        let i = self.crtc_index(q.crtc_id)?;
        if q.flags & !(vblank::SEQUENCE_RELATIVE | vblank::SEQUENCE_NEXT_ON_MISS) != 0 {
            return Err(Errno::EINVAL);
        }
        if !self.state.crtcs[i].active {
            return Err(Errno::EINVAL);
        }
        if self.file(file)?.event_bytes() + uapi::event::LEN > crate::device::EVENT_SPACE {
            return Err(Errno::ENOMEM);
        }
        let clock = self.crtcs[i].vblank;
        let current = clock.seq(now);
        let mut target = if q.flags & vblank::SEQUENCE_RELATIVE != 0 { current + q.sequence } else { q.sequence };
        if q.flags & vblank::SEQUENCE_NEXT_ON_MISS != 0 && target <= current {
            target = current + 1;
        }
        let (seq, due) = if target <= current { (current, now) } else { (target, clock.time_of(target)) };
        let mut bytes = [0u8; uapi::event::LEN];
        EventCrtcSequence {
            ty: uapi::event::CRTC_SEQUENCE,
            length: uapi::event::LEN as u32,
            user_data: q.user_data,
            time_ns: clock.time_of(seq) as i64,
            sequence: seq,
        }
        .encode(&mut bytes);
        self.file_mut(file)?.queue(Event { due, bytes });
        q.sequence = target;
        q.encode(b);
        Ok(())
    }
}

/// The reply time of a vblank wait: seconds where the request had its
/// `signal`, microseconds after it.
fn set_reply_time(w: &mut WaitVblank, ns: u64) {
    w.signal = ns / 1_000_000_000;
    w.tval_usec = (ns % 1_000_000_000 / 1000) as i64;
}

/// A `struct drm_event_vblank` stamped `ns`, readable from `ns`.
fn vblank_event(ty: u32, user_data: u64, ns: u64, seq: u64, crtc_id: u32) -> Event {
    let mut bytes = [0u8; uapi::event::LEN];
    EventVblank {
        ty,
        length: uapi::event::LEN as u32,
        user_data,
        tv_sec: (ns / 1_000_000_000) as u32,
        tv_usec: (ns % 1_000_000_000 / 1000) as u32,
        sequence: seq as u32,
        crtc_id,
    }
    .encode(&mut bytes);
    Event { due: ns, bytes }
}

fn intersect(a: &Rect, b: &Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x as u64 + a.width as u64).min(b.x as u64 + b.width as u64);
    let y1 = (a.y as u64 + a.height as u64).min(b.y as u64 + b.height as u64);
    if x1 <= x0 as u64 || y1 <= y0 as u64 {
        return Rect::default();
    }
    Rect::new(x0, y0, (x1 - x0 as u64) as u32, (y1 - y0 as u64) as u32)
}
