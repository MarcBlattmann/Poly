//! The screen through DRM/KMS (ADR 0116): `/dev/dri/card0` opened (the
//! first opener is DRM master, ADR 0108), the first connected connector and
//! its preferred mode, two dumb buffers as framebuffers, and page flips
//! with their completion events, so frames are paced by the display's
//! vertical blank — real vsync: frame callbacks and `wp_presentation`
//! feedback are answered when a flip completed, with its time and vblank
//! sequence.
//!
//! The scene draws into the back buffer: first the parts the back buffer
//! missed (what the frame before drew into the other buffer) are copied
//! over from the front buffer, then the frame's damage is drawn, then
//! `PAGE_FLIP` with `DRM_MODE_PAGE_FLIP_EVENT` queues it for the next
//! vblank. No frame is drawn while a flip is pending; the flip-complete
//! event (`struct drm_event_vblank`) read from the card's file ends it.
//!
//! All of it goes through [`Drm`], the card file's calls (`ioctl`, `mmap`,
//! `read`), so the tests run it against `poly-drm`'s device — the same
//! code Poly's kernel runs — in the test process.
//!
//! Sources: the MIT-licensed uapi headers `drm/drm.h`, `drm/drm_mode.h`,
//! `drm/drm_fourcc.h` (through `poly-drm`'s `uapi`), and the kernel's
//! public DRM documentation (KMS, dumb buffers, page-flip events).

use poly_drm::uapi::{
    CardRes, CreateDumb, EventVblank, FbCmd2, GetConnector, GetEncoder, MapDumb, ModeCrtc, ModeInfo, PageFlip, event,
    fourcc, ioctl, mode,
};
use poly_wayland::compose::{Framebuffer, Rect};
use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

/// The calls the backend makes on the card's file.
pub trait Drm {
    /// `ioctl(fd, request, arg)`; pointers inside `arg` are this
    /// process's.
    fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<()>;
    /// `mmap` `len` bytes at `offset` (a dumb buffer's fake offset), shared,
    /// readable and writable.
    fn map(&mut self, offset: u64, len: usize) -> io::Result<*mut u8>;
    /// Undo a [`Drm::map`].
    fn unmap(&mut self, ptr: *mut u8, len: usize);
    /// Read events without waiting: bytes read, 0 if none are there.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// The descriptor to `poll` for events, if there is one.
    fn fd(&self) -> Option<RawFd>;
}

/// `/dev/dri/card<n>`, opened.
#[derive(Debug)]
pub struct CardFile {
    fd: RawFd,
}

impl CardFile {
    pub fn open(path: &str) -> io::Result<CardFile> {
        let c = CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: a NUL-terminated path.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(CardFile { fd })
    }
}

impl Drop for CardFile {
    fn drop(&mut self) {
        crate::unix::close(self.fd);
    }
}

impl Drm for CardFile {
    fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<()> {
        loop {
            // SAFETY: `arg` is a buffer of the size `request` encodes (each
            // caller passes the structure's bytes); the pointers inside it
            // point to live buffers of the lengths given.
            let r = unsafe { libc::ioctl(self.fd, request as libc::c_ulong, arg.as_mut_ptr()) };
            if r >= 0 {
                return Ok(());
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
    }

    fn map(&mut self, offset: u64, len: usize) -> io::Result<*mut u8> {
        // SAFETY: a new shared mapping chosen by the kernel; nothing else
        // refers to that range.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                self.fd,
                offset as libc::off_t,
            )
        };
        if ptr == libc::MAP_FAILED { Err(io::Error::last_os_error()) } else { Ok(ptr.cast()) }
    }

    fn unmap(&mut self, ptr: *mut u8, len: usize) {
        // SAFETY: exactly a range `map` returned, no longer referred to.
        unsafe { libc::munmap(ptr.cast(), len) };
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: reading into a buffer of the length given.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) { Ok(0) } else { Err(e) }
    }

    fn fd(&self) -> Option<RawFd> {
        Some(self.fd)
    }
}

/// The output chosen: connector, CRTC, mode, physical size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub connector: u32,
    pub crtc: u32,
    pub mode: ModeInfo,
    /// Millimetres, 0 if unknown.
    pub mm: (u32, u32),
}

impl Output {
    pub fn size(&self) -> (u32, u32) {
        (u32::from(self.mode.hdisplay), u32::from(self.mode.vdisplay))
    }

    /// The refresh rate in millihertz, from the mode's timings.
    pub fn refresh_mhz(&self) -> u32 {
        let m = &self.mode;
        let total = u64::from(m.htotal) * u64::from(m.vtotal);
        if total == 0 {
            return m.vrefresh * 1000;
        }
        (u64::from(m.clock) * 1_000_000 / total) as u32
    }
}

/// The integer scale for a screen `px` pixels wide that is `mm`
/// millimetres wide: its density over 96 dots per inch (the density
/// logical pixels are meant for), rounded, 1 to 4; 1 if the size is not
/// known.
pub fn auto_scale(px: u32, mm: u32) -> u32 {
    if mm == 0 {
        return 1;
    }
    let dpi = u64::from(px) * 254 / (u64::from(mm) * 10);
    ((dpi + 48) / 96).clamp(1, 4) as u32
}

/// Two-pass array queries (as libdrm does): `n` 32-bit IDs.
fn ids(n: u32) -> Vec<u32> {
    vec![0; n as usize + 1]
}

/// Find the first connected connector with a mode, its preferred mode
/// (else the first) and a CRTC its encoders can drive.
pub fn probe(drm: &mut dyn Drm) -> io::Result<Output> {
    let mut r = CardRes::default().to_bytes();
    drm.ioctl(ioctl::MODE_GETRESOURCES, &mut r)?;
    let counts = CardRes::decode(&r);
    let (mut fbs, mut crtcs, mut connectors, mut encoders) =
        (ids(counts.count_fbs), ids(counts.count_crtcs), ids(counts.count_connectors), ids(counts.count_encoders));
    let mut q = CardRes {
        fb_id_ptr: fbs.as_mut_ptr() as u64,
        crtc_id_ptr: crtcs.as_mut_ptr() as u64,
        connector_id_ptr: connectors.as_mut_ptr() as u64,
        encoder_id_ptr: encoders.as_mut_ptr() as u64,
        ..counts
    }
    .to_bytes();
    drm.ioctl(ioctl::MODE_GETRESOURCES, &mut q)?;
    let res = CardRes::decode(&q);
    crtcs.truncate(res.count_crtcs.min(counts.count_crtcs) as usize);
    connectors.truncate(res.count_connectors.min(counts.count_connectors) as usize);
    for id in connectors {
        // The first call probes the connector (count_modes 0).
        let mut g = GetConnector { connector_id: id, ..GetConnector::default() }.to_bytes();
        drm.ioctl(ioctl::MODE_GETCONNECTOR, &mut g)?;
        let c = GetConnector::decode(&g);
        if c.connection != mode::CONNECTED || c.count_modes == 0 {
            continue;
        }
        let mut modes = vec![0u8; c.count_modes as usize * ModeInfo::SIZE];
        let mut encs = ids(c.count_encoders);
        let mut props = ids(c.count_props);
        let mut values = vec![0u64; c.count_props as usize + 1];
        let mut g = GetConnector {
            connector_id: id,
            modes_ptr: modes.as_mut_ptr() as u64,
            encoders_ptr: encs.as_mut_ptr() as u64,
            props_ptr: props.as_mut_ptr() as u64,
            prop_values_ptr: values.as_mut_ptr() as u64,
            count_modes: c.count_modes,
            count_encoders: c.count_encoders,
            count_props: c.count_props,
            ..GetConnector::default()
        }
        .to_bytes();
        drm.ioctl(ioctl::MODE_GETCONNECTOR, &mut g)?;
        let c = GetConnector::decode(&g);
        let modes: Vec<ModeInfo> =
            modes.chunks_exact(ModeInfo::SIZE).take(c.count_modes as usize).map(ModeInfo::decode).collect();
        let Some(chosen) = modes.iter().find(|m| m.ty & mode::TYPE_PREFERRED != 0).or(modes.first()).copied() else {
            continue;
        };
        encs.truncate(c.count_encoders as usize);
        // The CRTC: the current encoder's, else the first an encoder can
        // drive.
        let mut crtc = None;
        for e in encs.iter().copied().filter(|e| *e != 0) {
            let mut ge = GetEncoder { encoder_id: e, ..GetEncoder::default() }.to_bytes();
            drm.ioctl(ioctl::MODE_GETENCODER, &mut ge)?;
            let enc = GetEncoder::decode(&ge);
            if enc.crtc_id != 0 && e == c.encoder_id {
                crtc = Some(enc.crtc_id);
                break;
            }
            if crtc.is_none() {
                crtc = crtcs.iter().enumerate().find(|(i, _)| enc.possible_crtcs & (1 << i) != 0).map(|(_, id)| *id);
            }
        }
        let Some(crtc) = crtc else { continue };
        return Ok(Output { connector: id, crtc, mode: chosen, mm: (c.mm_width, c.mm_height) });
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "no connected connector with a mode"))
}

/// A dumb buffer as a framebuffer, mapped.
#[derive(Debug)]
struct Buffer {
    handle: u32,
    fb: u32,
    ptr: *mut u32,
    len: usize,
    /// Pixels from one row to the next.
    stride: usize,
}

/// A completed page flip: when (CLOCK_MONOTONIC) and at which vblank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flip {
    pub monotonic_ns: u64,
    pub sequence: u32,
}

/// The screen through KMS: two buffers, flipped at vblank.
#[derive(Debug)]
pub struct Kms<D: Drm> {
    drm: D,
    output: Output,
    buffers: Vec<Buffer>,
    /// The buffer shown.
    front: usize,
    /// A flip is queued and its event not read yet.
    pending: bool,
    /// Per buffer: what the other one was drawn with since this one was
    /// last drawn, to copy over before drawing into it.
    stale: Vec<Vec<Rect>>,
    /// Flips completed.
    pub flips: u64,
    /// The time of the first and the last completed flip.
    pub first_flip_ns: Option<u64>,
    pub last_flip_ns: u64,
}

impl<D: Drm> Kms<D> {
    /// Take the screen: probe, two buffers, the mode set showing the
    /// first (black).
    pub fn open(mut drm: D) -> io::Result<Kms<D>> {
        let output = probe(&mut drm)?;
        let (w, h) = output.size();
        let mut kms = Kms {
            drm,
            output,
            buffers: Vec::new(),
            front: 0,
            pending: false,
            stale: vec![Vec::new(), Vec::new()],
            flips: 0,
            first_flip_ns: None,
            last_flip_ns: 0,
        };
        for _ in 0..2 {
            let b = kms.create_buffer(w, h)?;
            kms.buffers.push(b);
        }
        let mut connector = kms.output.connector;
        let mut set = ModeCrtc {
            set_connectors_ptr: &mut connector as *mut u32 as u64,
            count_connectors: 1,
            crtc_id: kms.output.crtc,
            fb_id: kms.buffers[0].fb,
            mode_valid: 1,
            mode: kms.output.mode,
            ..ModeCrtc::default()
        }
        .to_bytes();
        kms.drm.ioctl(ioctl::MODE_SETCRTC, &mut set)?;
        Ok(kms)
    }

    fn create_buffer(&mut self, w: u32, h: u32) -> io::Result<Buffer> {
        let mut c = CreateDumb { width: w, height: h, bpp: 32, ..CreateDumb::default() }.to_bytes();
        self.drm.ioctl(ioctl::MODE_CREATE_DUMB, &mut c)?;
        let c = CreateDumb::decode(&c);
        let mut m = MapDumb { handle: c.handle, ..MapDumb::default() }.to_bytes();
        self.drm.ioctl(ioctl::MODE_MAP_DUMB, &mut m)?;
        let offset = MapDumb::decode(&m).offset;
        let len = c.size as usize;
        let ptr = self.drm.map(offset, len)?;
        let mut f = FbCmd2 {
            width: w,
            height: h,
            pixel_format: fourcc::XRGB8888,
            handles: [c.handle, 0, 0, 0],
            pitches: [c.pitch, 0, 0, 0],
            ..FbCmd2::default()
        }
        .to_bytes();
        self.drm.ioctl(ioctl::MODE_ADDFB2, &mut f)?;
        let fb = FbCmd2::decode(&f).fb_id;
        Ok(Buffer { handle: c.handle, fb, ptr: ptr.cast(), len, stride: c.pitch as usize / 4 })
    }

    pub fn output(&self) -> &Output {
        &self.output
    }

    /// Whether a flip waits for its vblank (draw nothing until it is done).
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// The descriptor to poll for the flip's event.
    pub fn fd(&self) -> Option<RawFd> {
        self.drm.fd()
    }

    fn back(&self) -> usize {
        1 - self.front
    }

    /// Bring the back buffer up to date before a frame is drawn into it:
    /// copy what it missed from the front buffer.
    pub fn begin_frame(&mut self) {
        let (back, front) = (self.back(), self.front);
        let (w, h) = self.output.size();
        let screen = Rect::new(0, 0, w as i32, h as i32);
        for r in std::mem::take(&mut self.stale[back]) {
            let r = r.intersect(&screen);
            for y in r.y..r.bottom() {
                let (src, dst) = (&self.buffers[front], &self.buffers[back]);
                // SAFETY: both rows are inside their mappings (`r` is inside
                // the screen, every row of which the buffers hold); the two
                // buffers are distinct mappings, so the ranges do not overlap.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.ptr.add(y as usize * src.stride + r.x as usize),
                        dst.ptr.add(y as usize * dst.stride + r.x as usize),
                        r.width as usize,
                    );
                }
            }
        }
    }

    /// Queue the back buffer for the next vblank (`damage`: what was drawn
    /// into it, which the other buffer now misses).
    pub fn flip(&mut self, damage: &[Rect]) -> io::Result<()> {
        if self.pending {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "a flip is pending"));
        }
        let back = self.back();
        let mut p = PageFlip {
            crtc_id: self.output.crtc,
            fb_id: self.buffers[back].fb,
            flags: mode::PAGE_FLIP_EVENT,
            user_data: back as u64,
            ..PageFlip::default()
        }
        .to_bytes();
        self.drm.ioctl(ioctl::MODE_PAGE_FLIP, &mut p)?;
        self.pending = true;
        self.front = back;
        let other = self.back();
        self.stale[other].extend_from_slice(damage);
        if self.stale[other].len() > 16 {
            let all = self.stale[other].iter().fold(Rect::default(), |a, d| a.union(d));
            self.stale[other] = vec![all];
        }
        Ok(())
    }

    /// Read the card's events: the flip that completed, if one did.
    pub fn dispatch(&mut self) -> io::Result<Option<Flip>> {
        let mut buf = [0u8; 1024];
        let mut done = None;
        loop {
            let n = self.drm.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let mut at = 0;
            while at + event::LEN <= n {
                let e = EventVblank::decode(&buf[at..]);
                if e.length < 8 {
                    break;
                }
                if e.ty == event::FLIP_COMPLETE {
                    let ns = u64::from(e.tv_sec) * 1_000_000_000 + u64::from(e.tv_usec) * 1000;
                    done = Some(Flip { monotonic_ns: ns, sequence: e.sequence });
                    self.pending = false;
                    self.flips += 1;
                    self.first_flip_ns.get_or_insert(ns);
                    self.last_flip_ns = ns;
                }
                at += e.length as usize;
            }
        }
        Ok(done)
    }

    /// The buffer on the screen, as a framebuffer to read (a picture of
    /// what is shown).
    pub fn front_view(&mut self) -> FrontView<'_, D> {
        FrontView(self)
    }
}

impl<D: Drm> Drop for Kms<D> {
    fn drop(&mut self) {
        for b in std::mem::take(&mut self.buffers) {
            self.drm.unmap(b.ptr.cast(), b.len);
            let mut fb = b.fb.to_ne_bytes();
            let _ = self.drm.ioctl(ioctl::MODE_RMFB, &mut fb);
            let mut h = b.handle.to_ne_bytes();
            let _ = self.drm.ioctl(ioctl::MODE_DESTROY_DUMB, &mut h);
        }
    }
}

impl<D: Drm> Framebuffer for Kms<D> {
    fn width(&self) -> u32 {
        self.output.size().0
    }

    fn height(&self) -> u32 {
        self.output.size().1
    }

    /// A row of the back buffer.
    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        assert!(y < self.height());
        let b = &self.buffers[self.back()];
        // SAFETY: row `y` is inside the mapping (the dumb buffer holds
        // `height` rows of `stride` pixels), which lives as long as `self`;
        // the `&mut self` borrow keeps other references to it out, and the
        // device reads the back buffer only once it is flipped.
        unsafe { std::slice::from_raw_parts_mut(b.ptr.add(y as usize * b.stride), self.output.size().0 as usize) }
    }

    /// The frame is drawn: flip to it at the next vblank.
    fn present(&mut self, damage: &[Rect]) {
        if let Err(e) = self.flip(damage) {
            eprintln!("poly-compositor: page flip: {e}");
        }
    }
}

/// The front buffer of a [`Kms`] read as a framebuffer.
pub struct FrontView<'a, D: Drm>(&'a mut Kms<D>);

impl<D: Drm> Framebuffer for FrontView<'_, D> {
    fn width(&self) -> u32 {
        self.0.width()
    }

    fn height(&self) -> u32 {
        self.0.height()
    }

    fn row_mut(&mut self, y: u32) -> &mut [u32] {
        assert!(y < self.height());
        let b = &self.0.buffers[self.0.front];
        // SAFETY: as in `Kms::row_mut`, for the front buffer; only read
        // through this view (a picture of the screen).
        unsafe { std::slice::from_raw_parts_mut(b.ptr.add(y as usize * b.stride), self.0.output.size().0 as usize) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_drm::{BoId, Output as HwOutput, Rect as HwRect};
    use poly_drm::{Ctx, Device, DriverInfo, Errno, FbLayout, Fds, FileId, Hardware, HwError, Node, UserMemory};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Hardware that only keeps buffers and counts what is shown.
    #[derive(Default)]
    struct Hw {
        shown: Vec<u32>,
        taken: bool,
    }

    impl Hardware for Hw {
        type Buffer = Rc<RefCell<Vec<u8>>>;

        fn probe(&mut self) -> Vec<HwOutput> {
            // QEMU's EDID describes a 100 dpi screen: 1280 px is 325 mm.
            vec![HwOutput { connected: true, width: 1280, height: 800, edid: None }]
        }

        fn alloc(&mut self, size: u64) -> Option<Self::Buffer> {
            Some(Rc::new(RefCell::new(vec![0; size as usize])))
        }

        fn create_fb(&mut self, _: u32, _: &FbLayout, _: &Self::Buffer) -> Result<(), HwError> {
            Ok(())
        }

        fn destroy_fb(&mut self, _: u32, _: &Self::Buffer) -> Result<(), HwError> {
            Ok(())
        }

        fn upload(&mut self, _: u32, _: &FbLayout, _: HwRect) -> Result<(), HwError> {
            Ok(())
        }

        fn scanout(&mut self, _: usize, show: Option<(u32, HwRect)>) -> Result<(), HwError> {
            if let Some((fb, _)) = show {
                self.shown.push(fb);
            }
            Ok(())
        }

        fn flush(&mut self, _: u32, _: HwRect) -> Result<(), HwError> {
            Ok(())
        }

        fn take_display(&mut self) {
            self.taken = true;
        }

        fn give_back_display(&mut self) {
            self.taken = false;
        }
    }

    /// This process's memory, for the pointers in ioctl arguments.
    struct Process;

    impl UserMemory for Process {
        fn read(&mut self, addr: u64, buf: &mut [u8]) -> poly_drm::Result<()> {
            // SAFETY: the backend passes pointers to live buffers of at
            // least the lengths the device reads (the same contract the
            // kernel's copy from user memory checks).
            unsafe { std::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len()) };
            Ok(())
        }

        fn write(&mut self, addr: u64, data: &[u8]) -> poly_drm::Result<()> {
            // SAFETY: as in `read`, for buffers the backend lets the device
            // fill.
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), addr as *mut u8, data.len()) };
            Ok(())
        }
    }

    struct NoFds;

    impl Fds<Rc<RefCell<Vec<u8>>>> for NoFds {
        fn export(&mut self, _: BoId, _: &Rc<RefCell<Vec<u8>>>, _: u64, _: u32) -> poly_drm::Result<i32> {
            Err(Errno::EINVAL)
        }

        fn import(&mut self, _: i32) -> poly_drm::Result<BoId> {
            Err(Errno::EINVAL)
        }
    }

    /// poly-drm's device in this process, as a card file; `now` is moved by
    /// the test.
    struct InProcess {
        dev: Rc<RefCell<Device<Hw>>>,
        file: FileId,
        now: Rc<RefCell<u64>>,
    }

    impl Drm for InProcess {
        fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<()> {
            let mut ctx = Ctx { user: &mut Process, fds: &mut NoFds, now_ns: *self.now.borrow(), privileged: true };
            self.dev
                .borrow_mut()
                .ioctl(self.file, request, arg.as_mut_ptr() as u64, &mut ctx)
                .map(|_| ())
                .map_err(|e| io::Error::from_raw_os_error(e.0))
        }

        fn map(&mut self, offset: u64, len: usize) -> io::Result<*mut u8> {
            let m =
                self.dev.borrow().map(self.file, offset, len as u64).map_err(|e| io::Error::from_raw_os_error(e.0))?;
            // The buffer's vector is never resized: the pointer stays valid
            // while the device keeps the buffer, which the test outlives.
            let ptr = m.buffer.borrow_mut().as_mut_ptr();
            // SAFETY: `start` is inside the buffer (the device checked it).
            Ok(unsafe { ptr.add(m.start as usize) })
        }

        fn unmap(&mut self, _: *mut u8, _: usize) {}

        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let now = *self.now.borrow();
            match self.dev.borrow_mut().read(self.file, buf.len(), now) {
                Ok(bytes) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Err(Errno::EAGAIN) => Ok(0),
                Err(e) => Err(io::Error::from_raw_os_error(e.0)),
            }
        }

        fn fd(&self) -> Option<RawFd> {
            None
        }
    }

    fn device() -> (Rc<RefCell<Device<Hw>>>, Rc<RefCell<u64>>) {
        let info = DriverInfo {
            name: "virtio_gpu",
            desc: "test",
            date: "0",
            version: (0, 1, 0),
            unique: "pci:0000:00:02.0".into(),
        };
        (Rc::new(RefCell::new(Device::new(Hw::default(), info))), Rc::new(RefCell::new(1_000_000_000)))
    }

    #[test]
    fn scales_from_the_physical_size() {
        assert_eq!(auto_scale(1280, 0), 1, "unknown");
        assert_eq!(auto_scale(1280, 325), 1, "100 dpi");
        assert_eq!(auto_scale(2560, 344), 2, "a 13-inch 2560 panel: 189 dpi");
        assert_eq!(auto_scale(3840, 344), 3, "283 dpi");
        assert_eq!(auto_scale(7680, 100), 4, "capped");
    }

    /// The backend against poly-drm's device: it takes the screen with the
    /// preferred mode, draws into the back buffer, and every flip
    /// completes at a vblank of the mode's refresh, after which the next
    /// frame may go; the back buffer gets what it missed.
    #[test]
    fn flips_are_paced_by_the_vblank_clock() {
        let (dev, now) = device();
        let file = dev.borrow_mut().open(Node::Primary);
        let drm = InProcess { dev: Rc::clone(&dev), file, now: Rc::clone(&now) };
        let mut kms = Kms::open(drm).expect("KMS");
        assert_eq!(kms.output().size(), (1280, 800));
        let refresh = kms.output().refresh_mhz();
        assert!((59_000..=76_000).contains(&refresh), "{refresh} mHz");
        assert_eq!(dev.borrow().master(), Some(file), "the first opener is master");
        assert!(dev.borrow_mut().hardware().taken, "the console yielded");
        let frame_ns = 1_000_000_000_000 / u64::from(refresh);
        // Frame 1: a red square.
        kms.begin_frame();
        for y in 10..20 {
            kms.row_mut(y)[10..20].fill(0x00ff_0000);
        }
        kms.present(&[Rect::new(10, 10, 10, 10)]);
        assert!(kms.is_pending());
        assert!(kms.flip(&[]).is_err(), "one flip at a time");
        assert_eq!(kms.dispatch().unwrap(), None, "not before the vblank");
        *now.borrow_mut() += frame_ns + 1;
        let f1 = kms.dispatch().unwrap().expect("the flip completed");
        assert!(!kms.is_pending());
        assert_eq!(kms.front_view().row_mut(15)[15], 0x00ff_0000, "shown");
        // Frame 2 draws elsewhere; the back buffer first gets the square.
        kms.begin_frame();
        assert_eq!(kms.row_mut(15)[15], 0x00ff_0000, "brought up to date");
        kms.row_mut(100)[100] = 0x0000_ff00;
        kms.present(&[Rect::new(100, 100, 1, 1)]);
        *now.borrow_mut() += frame_ns + 1;
        let f2 = kms.dispatch().unwrap().expect("the second flip");
        // A vblank apart, in sequence.
        assert_eq!(f2.sequence, f1.sequence + 1);
        let gap = f2.monotonic_ns - f1.monotonic_ns;
        assert!(gap.abs_diff(frame_ns) < 1_000_000, "{gap} ns between flips, a frame is {frame_ns}");
        let view = &mut kms.front_view();
        assert_eq!((view.row_mut(15)[15], view.row_mut(100)[100]), (0x00ff_0000, 0x0000_ff00));
        assert_eq!(kms.flips, 2);
        drop(kms);
        dev.borrow_mut().close(file, *now.borrow());
        assert!(!dev.borrow_mut().hardware().taken, "the console is back");
    }

    /// No connected output: an error, so the compositor falls back to
    /// fbdev.
    #[test]
    fn no_output_is_an_error() {
        let info = DriverInfo { name: "x", desc: "x", date: "0", version: (0, 0, 0), unique: String::new() };
        #[derive(Default)]
        struct Off(Hw);
        impl Hardware for Off {
            type Buffer = Rc<RefCell<Vec<u8>>>;
            fn probe(&mut self) -> Vec<HwOutput> {
                vec![HwOutput { connected: false, width: 0, height: 0, edid: None }]
            }
            fn alloc(&mut self, size: u64) -> Option<Self::Buffer> {
                self.0.alloc(size)
            }
            fn create_fb(&mut self, a: u32, b: &FbLayout, c: &Self::Buffer) -> Result<(), HwError> {
                self.0.create_fb(a, b, c)
            }
            fn destroy_fb(&mut self, a: u32, b: &Self::Buffer) -> Result<(), HwError> {
                self.0.destroy_fb(a, b)
            }
            fn upload(&mut self, a: u32, b: &FbLayout, c: HwRect) -> Result<(), HwError> {
                self.0.upload(a, b, c)
            }
            fn scanout(&mut self, a: usize, b: Option<(u32, HwRect)>) -> Result<(), HwError> {
                self.0.scanout(a, b)
            }
            fn flush(&mut self, a: u32, b: HwRect) -> Result<(), HwError> {
                self.0.flush(a, b)
            }
            fn take_display(&mut self) {}
            fn give_back_display(&mut self) {}
        }
        let mut dev = Device::new(Off::default(), info);
        let file = dev.open(Node::Primary);
        struct Card<'a>(&'a mut Device<Off>, FileId);
        impl Drm for Card<'_> {
            fn ioctl(&mut self, request: u32, arg: &mut [u8]) -> io::Result<()> {
                struct No;
                impl Fds<Rc<RefCell<Vec<u8>>>> for No {
                    fn export(&mut self, _: BoId, _: &Rc<RefCell<Vec<u8>>>, _: u64, _: u32) -> poly_drm::Result<i32> {
                        Err(Errno::EINVAL)
                    }
                    fn import(&mut self, _: i32) -> poly_drm::Result<BoId> {
                        Err(Errno::EINVAL)
                    }
                }
                let mut ctx = Ctx { user: &mut Process, fds: &mut No, now_ns: 0, privileged: true };
                self.0
                    .ioctl(self.1, request, arg.as_mut_ptr() as u64, &mut ctx)
                    .map(|_| ())
                    .map_err(|e| io::Error::from_raw_os_error(e.0))
            }
            fn map(&mut self, _: u64, _: usize) -> io::Result<*mut u8> {
                Err(io::ErrorKind::Unsupported.into())
            }
            fn unmap(&mut self, _: *mut u8, _: usize) {}
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Ok(0)
            }
            fn fd(&self) -> Option<RawFd> {
                None
            }
        }
        let Err(e) = Kms::open(Card(&mut dev, file)) else { panic!("an output") };
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }
}
