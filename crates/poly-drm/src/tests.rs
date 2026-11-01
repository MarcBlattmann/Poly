//! Tests: the uapi encoding against the headers' numbers, modes, and the
//! device driven through its ioctls the way libdrm drives a kernel, on a
//! fake hardware that records what it is asked to do.

extern crate std;

use super::*;
use crate::uapi::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::format;
use std::rc::Rc;
use std::string::ToString;
use std::vec;
use std::vec::Vec;

// --- Fakes -----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    CreateFb(u32, FbLayout),
    DestroyFb(u32),
    Upload(u32, Rect),
    Scanout(usize, Option<(u32, Rect)>),
    Flush(u32, Rect),
    Take,
    GiveBack,
    CtxCreate(u32, u32, Vec<u8>),
    CtxDestroy(u32),
    Attach(u32, u32),
    Detach(u32, u32),
    Create(u32, Option<u32>, ResourceKind),
    Destroy(u32),
    Submit(u32, Option<u32>, Vec<u8>),
    Transfer(bool, Option<u32>, u32, Transfer),
    Capset(u32, u32),
}

#[derive(Default)]
struct Fake {
    outputs: Vec<Output>,
    calls: Vec<Call>,
    live: Rc<RefCell<usize>>,
    fail_scanout: bool,
    /// 3D: what it offers, fences not signalled yet, the last fence.
    caps: GpuCaps,
    unsignalled: std::collections::BTreeSet<u64>,
    fence: u64,
    /// Fences signal only when the test says so.
    hold_fences: bool,
}

/// A buffer that counts how many exist (freed when the last clone goes).
#[derive(Debug)]
struct Mem {
    bytes: RefCell<Vec<u8>>,
    live: Rc<RefCell<usize>>,
}

impl Drop for Mem {
    fn drop(&mut self) {
        *self.live.borrow_mut() -= 1;
    }
}

impl Hardware for Fake {
    type Buffer = Rc<Mem>;

    fn probe(&mut self) -> Vec<Output> {
        self.outputs.clone()
    }

    fn alloc(&mut self, size: u64) -> Option<Rc<Mem>> {
        *self.live.borrow_mut() += 1;
        Some(Rc::new(Mem { bytes: RefCell::new(vec![0; size as usize]), live: self.live.clone() }))
    }

    fn create_fb(&mut self, fb: u32, layout: &FbLayout, _: &Rc<Mem>) -> core::result::Result<(), HwError> {
        self.calls.push(Call::CreateFb(fb, *layout));
        Ok(())
    }

    fn destroy_fb(&mut self, fb: u32, _: &Rc<Mem>) -> core::result::Result<(), HwError> {
        self.calls.push(Call::DestroyFb(fb));
        Ok(())
    }

    fn upload(&mut self, fb: u32, _: &FbLayout, rect: Rect) -> core::result::Result<(), HwError> {
        self.calls.push(Call::Upload(fb, rect));
        Ok(())
    }

    fn scanout(&mut self, index: usize, show: Option<(u32, Rect)>) -> core::result::Result<(), HwError> {
        self.calls.push(Call::Scanout(index, show));
        if self.fail_scanout { Err(HwError) } else { Ok(()) }
    }

    fn flush(&mut self, fb: u32, rect: Rect) -> core::result::Result<(), HwError> {
        self.calls.push(Call::Flush(fb, rect));
        Ok(())
    }

    fn take_display(&mut self) {
        self.calls.push(Call::Take);
    }

    fn give_back_display(&mut self) {
        self.calls.push(Call::GiveBack);
    }

    fn gpu_caps(&mut self) -> GpuCaps {
        self.caps.clone()
    }

    fn capset(&mut self, id: u32, version: u32) -> core::result::Result<Vec<u8>, HwError> {
        self.calls.push(Call::Capset(id, version));
        let info = self.caps.capsets.iter().find(|c| c.id == id).ok_or(HwError)?;
        Ok((0..info.max_size).map(|i| (i as u8) ^ id as u8).collect())
    }

    fn ctx_create(&mut self, ctx: u32, capset: u32, name: &[u8]) -> core::result::Result<(), HwError> {
        self.calls.push(Call::CtxCreate(ctx, capset, name.to_vec()));
        Ok(())
    }

    fn ctx_destroy(&mut self, ctx: u32) {
        self.calls.push(Call::CtxDestroy(ctx));
    }

    fn ctx_attach(&mut self, ctx: u32, res: u32) -> core::result::Result<(), HwError> {
        self.calls.push(Call::Attach(ctx, res));
        Ok(())
    }

    fn ctx_detach(&mut self, ctx: u32, res: u32) {
        self.calls.push(Call::Detach(ctx, res));
    }

    fn create_resource(
        &mut self,
        res: u32,
        ctx: Option<u32>,
        kind: &ResourceKind,
        _: Option<&Rc<Mem>>,
    ) -> core::result::Result<Option<Rc<Mem>>, HwError> {
        self.calls.push(Call::Create(res, ctx, *kind));
        match kind {
            // A mappable host blob: its window in the host-visible region.
            ResourceKind::Blob { mem: 2, flags, size, .. } if flags & 1 != 0 => Ok(self.alloc(*size)),
            _ => Ok(None),
        }
    }

    fn destroy_resource(&mut self, res: u32, _: Option<&Rc<Mem>>) {
        self.calls.push(Call::Destroy(res));
    }

    fn submit(&mut self, ctx: u32, ring: Option<u32>, commands: &[u8]) -> core::result::Result<u64, HwError> {
        self.calls.push(Call::Submit(ctx, ring, commands.to_vec()));
        Ok(self.next_fence())
    }

    fn transfer(
        &mut self,
        to_host: bool,
        ctx: Option<u32>,
        res: u32,
        t: &Transfer,
    ) -> core::result::Result<u64, HwError> {
        self.calls.push(Call::Transfer(to_host, ctx, res, *t));
        Ok(if ctx.is_some() { self.next_fence() } else { 0 })
    }

    fn fence_done(&mut self, fence: u64) -> bool {
        !self.unsignalled.contains(&fence)
    }
}

impl Fake {
    fn next_fence(&mut self) -> u64 {
        self.fence += 1;
        if self.hold_fences {
            self.unsignalled.insert(self.fence);
        }
        self.fence
    }
}

/// User memory: one flat area from [`BASE`].
struct User {
    bytes: Vec<u8>,
    next: usize,
}

const BASE: u64 = 0x1000_0000;

impl User {
    fn new() -> User {
        User { bytes: vec![0; 1 << 20], next: 0 }
    }

    fn alloc(&mut self, len: usize) -> u64 {
        let at = self.next;
        self.next = (self.next + len + 15) & !15;
        assert!(self.next <= self.bytes.len());
        BASE + at as u64
    }

    fn put(&mut self, data: &[u8]) -> u64 {
        let at = self.alloc(data.len().max(1));
        UserMemory::write(self, at, data).unwrap();
        at
    }

    fn get(&mut self, at: u64, len: usize) -> Vec<u8> {
        let mut v = vec![0; len];
        UserMemory::read(self, at, &mut v).unwrap();
        v
    }

    fn u32s(&mut self, at: u64, n: usize) -> Vec<u32> {
        self.get(at, n * 4).chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
    }

    fn u64s(&mut self, at: u64, n: usize) -> Vec<u64> {
        self.get(at, n * 8).chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
    }
}

impl UserMemory for User {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<()> {
        let at = addr.checked_sub(BASE).ok_or(Errno::EFAULT)? as usize;
        buf.copy_from_slice(self.bytes.get(at..at + buf.len()).ok_or(Errno::EFAULT)?);
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<()> {
        let at = addr.checked_sub(BASE).ok_or(Errno::EFAULT)? as usize;
        self.bytes.get_mut(at..at + data.len()).ok_or(Errno::EFAULT)?.copy_from_slice(data);
        Ok(())
    }
}

/// The file table: dma-buf descriptors from 100, sync files from 200.
#[derive(Default)]
struct FdTable {
    files: BTreeMap<i32, (BoId, u32)>,
    next: i32,
    fences: BTreeMap<i32, Vec<u64>>,
}

impl Fds<Rc<Mem>> for FdTable {
    fn export(&mut self, bo: BoId, _: &Rc<Mem>, _: u64, flags: u32) -> Result<i32> {
        let fd = 100 + self.next;
        self.next += 1;
        self.files.insert(fd, (bo, flags));
        Ok(fd)
    }

    fn import(&mut self, fd: i32) -> Result<BoId> {
        self.files.get(&fd).map(|f| f.0).ok_or(Errno::EBADF)
    }

    fn export_fence(&mut self, fences: &[u64]) -> Result<i32> {
        let fd = 200 + self.fences.len() as i32;
        self.fences.insert(fd, fences.to_vec());
        Ok(fd)
    }

    fn import_fence(&mut self, fd: i32) -> Result<Vec<u64>> {
        self.fences.get(&fd).cloned().ok_or(Errno::EBADF)
    }
}

/// A device on fake hardware with `outputs`, and a process to call it from.
struct T {
    dev: Device<Fake>,
    user: User,
    fds: FdTable,
    now: u64,
    privileged: bool,
}

const MS: u64 = 1_000_000;

fn output(w: u32, h: u32) -> Output {
    Output { connected: true, width: w, height: h, edid: None }
}

impl T {
    fn new(outputs: Vec<Output>) -> T {
        let hw = Fake { outputs, ..Fake::default() };
        let info = DriverInfo {
            name: "virtio_gpu",
            desc: "test device",
            date: "0",
            version: (0, 1, 0),
            unique: "pci:0000:00:02.0".to_string(),
        };
        T { dev: Device::new(hw, info), user: User::new(), fds: FdTable::default(), now: 1000 * MS, privileged: true }
    }

    fn one() -> T {
        T::new(vec![output(1024, 768)])
    }

    /// Issue `request` with argument bytes `arg` (copied in and back).
    fn call(&mut self, file: FileId, request: u32, arg: &mut [u8]) -> Result<Reply> {
        let at = self.user.put(arg);
        let mut ctx = Ctx { user: &mut self.user, fds: &mut self.fds, now_ns: self.now, privileged: self.privileged };
        let r = self.dev.ioctl(file, request, at, &mut ctx);
        let back = self.user.get(at, arg.len());
        arg.copy_from_slice(&back);
        r
    }

    fn ok(&mut self, file: FileId, request: u32, arg: &mut [u8]) {
        assert_eq!(self.call(file, request, arg), Ok(Reply::Done(0)), "request {request:#x}");
    }

    fn calls(&mut self) -> Vec<Call> {
        core::mem::take(&mut self.dev.hardware().calls)
    }

    fn resources(&mut self, file: FileId) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
        let mut r = CardRes::default().to_bytes();
        self.ok(file, ioctl::MODE_GETRESOURCES, &mut r);
        let c = CardRes::decode(&r);
        let ptrs = [c.count_fbs, c.count_crtcs, c.count_connectors, c.count_encoders]
            .map(|n| self.user.alloc(n as usize * 4 + 4));
        let mut q = CardRes {
            fb_id_ptr: ptrs[0],
            crtc_id_ptr: ptrs[1],
            connector_id_ptr: ptrs[2],
            encoder_id_ptr: ptrs[3],
            count_fbs: c.count_fbs,
            count_crtcs: c.count_crtcs,
            count_connectors: c.count_connectors,
            count_encoders: c.count_encoders,
            ..CardRes::default()
        }
        .to_bytes();
        self.ok(file, ioctl::MODE_GETRESOURCES, &mut q);
        (
            self.user.u32s(ptrs[0], c.count_fbs as usize),
            self.user.u32s(ptrs[1], c.count_crtcs as usize),
            self.user.u32s(ptrs[2], c.count_connectors as usize),
            self.user.u32s(ptrs[3], c.count_encoders as usize),
        )
    }

    /// The connector's modes and its whole reply.
    fn connector(&mut self, file: FileId, id: u32) -> (GetConnector, Vec<ModeInfo>) {
        let mut g = GetConnector { connector_id: id, ..GetConnector::default() }.to_bytes();
        self.ok(file, ioctl::MODE_GETCONNECTOR, &mut g);
        let c = GetConnector::decode(&g);
        let modes = self.user.alloc(c.count_modes as usize * ModeInfo::SIZE);
        let encs = self.user.alloc(c.count_encoders as usize * 4);
        let props = self.user.alloc(c.count_props as usize * 4);
        let values = self.user.alloc(c.count_props as usize * 8);
        let mut g = GetConnector {
            connector_id: id,
            modes_ptr: modes,
            count_modes: c.count_modes,
            encoders_ptr: encs,
            count_encoders: c.count_encoders,
            props_ptr: props,
            prop_values_ptr: values,
            count_props: c.count_props,
            ..GetConnector::default()
        }
        .to_bytes();
        self.ok(file, ioctl::MODE_GETCONNECTOR, &mut g);
        let c = GetConnector::decode(&g);
        let bytes = self.user.get(modes, c.count_modes as usize * ModeInfo::SIZE);
        (c, bytes.chunks_exact(ModeInfo::SIZE).map(ModeInfo::decode).collect())
    }

    fn dumb(&mut self, file: FileId, w: u32, h: u32) -> CreateDumb {
        let mut c = CreateDumb { width: w, height: h, bpp: 32, ..CreateDumb::default() }.to_bytes();
        self.ok(file, ioctl::MODE_CREATE_DUMB, &mut c);
        CreateDumb::decode(&c)
    }

    fn addfb2(&mut self, file: FileId, d: &CreateDumb, w: u32, h: u32) -> Result<u32> {
        let mut c = FbCmd2 { width: w, height: h, pixel_format: fourcc::XRGB8888, ..FbCmd2::default() };
        c.handles[0] = d.handle;
        c.pitches[0] = d.pitch;
        let mut b = c.to_bytes();
        self.call(file, ioctl::MODE_ADDFB2, &mut b)?;
        Ok(FbCmd2::decode(&b).fb_id)
    }

    /// Ids of CRTC 0, connector 0, encoder 0, plane 0.
    fn ids(&mut self, file: FileId) -> (u32, u32, u32) {
        let (_, crtcs, conns, encs) = self.resources(file);
        (crtcs[0], conns[0], encs[0])
    }

    fn setcrtc(&mut self, file: FileId, crtc: u32, fb: u32, conn: u32, mode: Option<ModeInfo>) -> Result<Reply> {
        let conns = self.user.put(&conn.to_le_bytes());
        let mut c = ModeCrtc {
            crtc_id: crtc,
            fb_id: fb,
            set_connectors_ptr: conns,
            count_connectors: mode.is_some() as u32,
            mode_valid: mode.is_some() as u32,
            mode: mode.unwrap_or_default(),
            ..ModeCrtc::default()
        }
        .to_bytes();
        self.call(file, ioctl::MODE_SETCRTC, &mut c)
    }

    /// A file showing a 1024x768 framebuffer on CRTC 0: (file, crtc, conn,
    /// fb, mode).
    fn showing(&mut self) -> (FileId, u32, u32, u32, ModeInfo) {
        let f = self.dev.open(Node::Primary);
        let (crtc, conn, _) = self.ids(f);
        let (_, modes) = self.connector(f, conn);
        let d = self.dumb(f, 1024, 768);
        let fb = self.addfb2(f, &d, 1024, 768).unwrap();
        assert_eq!(self.setcrtc(f, crtc, fb, conn, Some(modes[0])), Ok(Reply::Done(0)));
        (f, crtc, conn, fb, modes[0])
    }

    fn set_cap(&mut self, file: FileId, cap: u64, value: u64) -> Result<Reply> {
        let mut c = GetCap { capability: cap, value }.to_bytes();
        self.call(file, ioctl::SET_CLIENT_CAP, &mut c)
    }

    fn read(&mut self, file: FileId) -> Result<Vec<u8>> {
        self.dev.read(file, 1024, self.now)
    }

    /// The property id named `name` among `ids`.
    fn prop_named(&mut self, file: FileId, ids: &[u32], name: &str) -> u32 {
        for &id in ids {
            let mut g = GetProperty { prop_id: id, ..GetProperty::default() }.to_bytes();
            self.ok(file, ioctl::MODE_GETPROPERTY, &mut g);
            if name_str(&GetProperty::decode(&g).name) == name {
                return id;
            }
        }
        panic!("no property {name}");
    }

    fn obj_props(&mut self, file: FileId, obj: u32, ty: u32) -> (Vec<u32>, Vec<u64>) {
        let mut g = ObjGetProperties { obj_id: obj, obj_type: ty, ..ObjGetProperties::default() }.to_bytes();
        self.ok(file, ioctl::MODE_OBJ_GETPROPERTIES, &mut g);
        let n = ObjGetProperties::decode(&g).count_props as usize;
        let (p, v) = (self.user.alloc(n * 4), self.user.alloc(n * 8));
        let mut g =
            ObjGetProperties { obj_id: obj, obj_type: ty, props_ptr: p, prop_values_ptr: v, count_props: n as u32 }
                .to_bytes();
        self.ok(file, ioctl::MODE_OBJ_GETPROPERTIES, &mut g);
        (self.user.u32s(p, n), self.user.u64s(v, n))
    }

    fn atomic(&mut self, file: FileId, flags: u32, sets: &[(u32, u32, u64)], user_data: u64) -> Result<Reply> {
        let mut objs: Vec<u32> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let (mut props, mut values) = (Vec::new(), Vec::new());
        for &(obj, prop, value) in sets {
            if objs.last() != Some(&obj) {
                objs.push(obj);
                counts.push(0);
            }
            *counts.last_mut().unwrap() += 1;
            props.push(prop);
            values.push(value);
        }
        let bytes32 = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let a = Atomic {
            flags,
            count_objs: objs.len() as u32,
            objs_ptr: self.user.put(&bytes32(&objs)),
            count_props_ptr: self.user.put(&bytes32(&counts)),
            props_ptr: self.user.put(&bytes32(&props)),
            prop_values_ptr: self.user.put(&values.iter().flat_map(|x: &u64| x.to_le_bytes()).collect::<Vec<u8>>()),
            reserved: 0,
            user_data,
        };
        let mut b = a.to_bytes();
        self.call(file, ioctl::MODE_ATOMIC, &mut b)
    }
}

// --- The uapi encoding ------------------------------------------------------

#[test]
fn request_numbers_are_the_headers() {
    // _IOC(dir, 'd', nr, sizeof(struct)) for the x86-64 structure sizes.
    let expected = [
        (ioctl::VERSION, 0xc040_6400),
        (ioctl::GET_CAP, 0xc010_640c),
        (ioctl::SET_CLIENT_CAP, 0x4010_640d),
        (ioctl::SET_MASTER, 0x0000_641e),
        (ioctl::DROP_MASTER, 0x0000_641f),
        (ioctl::GEM_CLOSE, 0x4008_6409),
        (ioctl::PRIME_HANDLE_TO_FD, 0xc00c_642d),
        (ioctl::PRIME_FD_TO_HANDLE, 0xc00c_642e),
        (ioctl::WAIT_VBLANK, 0xc018_643a),
        (ioctl::MODE_GETRESOURCES, 0xc040_64a0),
        (ioctl::MODE_GETCRTC, 0xc068_64a1),
        (ioctl::MODE_SETCRTC, 0xc068_64a2),
        (ioctl::MODE_GETENCODER, 0xc014_64a6),
        (ioctl::MODE_GETCONNECTOR, 0xc050_64a7),
        (ioctl::MODE_GETPROPERTY, 0xc040_64aa),
        (ioctl::MODE_GETPROPBLOB, 0xc010_64ac),
        (ioctl::MODE_ADDFB, 0xc01c_64ae),
        (ioctl::MODE_RMFB, 0xc004_64af),
        (ioctl::MODE_PAGE_FLIP, 0xc018_64b0),
        (ioctl::MODE_DIRTYFB, 0xc018_64b1),
        (ioctl::MODE_CREATE_DUMB, 0xc020_64b2),
        (ioctl::MODE_MAP_DUMB, 0xc010_64b3),
        (ioctl::MODE_DESTROY_DUMB, 0xc004_64b4),
        (ioctl::MODE_GETPLANERESOURCES, 0xc010_64b5),
        (ioctl::MODE_GETPLANE, 0xc020_64b6),
        (ioctl::MODE_SETPLANE, 0xc030_64b7),
        (ioctl::MODE_ADDFB2, 0xc068_64b8),
        (ioctl::MODE_OBJ_GETPROPERTIES, 0xc020_64b9),
        (ioctl::MODE_OBJ_SETPROPERTY, 0xc018_64ba),
        (ioctl::MODE_ATOMIC, 0xc038_64bc),
        (ioctl::MODE_CREATEPROPBLOB, 0xc010_64bd),
        (ioctl::MODE_DESTROYPROPBLOB, 0xc004_64be),
    ];
    for (got, want) in expected {
        assert_eq!(got, want, "{got:#x} should be {want:#x}");
    }
    assert_eq!(fourcc::XRGB8888, 0x3432_5258);
    assert_eq!(fourcc::ARGB8888, 0x3432_5241);
    assert_eq!(Request::decode(ioctl::MODE_ADDFB2), Request { nr: 0xb8, ty: b'd', size: 104, dir: 3 });
}

#[test]
fn structures_encode_at_the_header_offsets() {
    let mut b = [0u8; 104];
    let mut c = FbCmd2 { fb_id: 1, width: 2, height: 3, pixel_format: 4, flags: 5, ..FbCmd2::default() };
    c.handles = [6, 7, 8, 9];
    c.modifier[3] = 0x1122_3344_5566_7788;
    c.encode(&mut b);
    assert_eq!(&b[0..4], &1u32.to_le_bytes());
    assert_eq!(&b[16..20], &5u32.to_le_bytes());
    assert_eq!(&b[20..24], &6u32.to_le_bytes());
    assert_eq!(&b[96..104], &0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(FbCmd2::decode(&b), c);
    let mut m = ModeInfo { clock: 1, hdisplay: 2, vtotal: 3, ty: 4, ..ModeInfo::default() };
    m.name = name_bytes("1024x768");
    let mut b = [0u8; 68];
    m.encode(&mut b);
    assert_eq!(&b[4..6], &2u16.to_le_bytes());
    assert_eq!(&b[20..22], &3u16.to_le_bytes());
    assert_eq!(&b[32..36], &4u32.to_le_bytes());
    assert_eq!(&b[36..44], b"1024x768");
    let crtc = ModeCrtc { mode: m, crtc_id: 9, ..ModeCrtc::default() };
    let mut b = [0u8; 104];
    crtc.encode(&mut b);
    assert_eq!(&b[12..16], &9u32.to_le_bytes());
    assert_eq!(ModeInfo::decode(&b[36..]), m);
}

// --- Modes -------------------------------------------------------------------

#[test]
fn cvt_reduced_blanking_matches_the_published_timings() {
    // VESA CVT-RB: 1920x1080 at 60 Hz is 138.5 MHz, 2080 x 1111 total;
    // 1280x800 is 71.0 MHz, 1440 x 823.
    let m = modes::cvt_rb(1920, 1080, 60);
    assert_eq!((m.clock, m.htotal, m.vtotal), (138_500, 2080, 1111));
    assert_eq!((m.hsync_start, m.hsync_end, m.vsync_start, m.vsync_end), (1968, 2000, 1083, 1088));
    assert_eq!(m.vrefresh, 60);
    assert_eq!(name_str(&m.name), "1920x1080");
    let m = modes::cvt_rb(1280, 800, 60);
    assert_eq!((m.clock, m.htotal, m.vtotal, m.vsync_end - m.vsync_start), (71_000, 1440, 823, 6));
    assert!(modes::valid(&m));
    // 1440 * 823 pixels at 71 MHz.
    assert_eq!(modes::frame_ns(&m) / 1000, 16_691);
}

/// An EDID base block with one detailed timing: 1024x768 at 65 MHz
/// (blanking 320 x 38, sync offset 24 / 3, width 136 / 6, -h -v sync),
/// 300 x 230 mm.
fn edid_1024() -> Vec<u8> {
    let mut e = vec![0u8; 128];
    e[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    e[18] = 1;
    e[19] = 4;
    e[21] = 30;
    e[22] = 23;
    let d = &mut e[54..72];
    d[0..2].copy_from_slice(&6500u16.to_le_bytes());
    d[2] = (1024 & 0xff) as u8;
    d[3] = (320 & 0xff) as u8;
    d[4] = ((1024 >> 8) << 4 | (320 >> 8)) as u8;
    d[5] = (768 & 0xff) as u8;
    d[6] = 38;
    d[7] = ((768 >> 8) << 4) as u8;
    d[8] = 24;
    d[9] = 136;
    d[10] = 3 << 4 | 6;
    d[11] = 0;
    d[12] = (300 & 0xff) as u8;
    d[13] = 230;
    d[14] = ((300 >> 8) << 4) as u8;
    d[17] = 0x18;
    let sum = e.iter().fold(0u8, |s, &x| s.wrapping_add(x));
    e[127] = 0u8.wrapping_sub(sum);
    e
}

#[test]
fn edid_detailed_timings_become_modes() {
    let got = modes::edid_modes(&edid_1024()).unwrap();
    assert_eq!(got.mm, (300, 230));
    let m = got.modes[0];
    assert_eq!((m.clock, m.hdisplay, m.hsync_start, m.hsync_end, m.htotal), (65_000, 1024, 1048, 1184, 1344));
    assert_eq!((m.vdisplay, m.vsync_start, m.vsync_end, m.vtotal), (768, 771, 777, 806));
    assert_eq!(m.flags, mode::FLAG_NHSYNC | mode::FLAG_NVSYNC);
    assert_eq!(m.ty, mode::TYPE_DRIVER | mode::TYPE_PREFERRED);
    assert_eq!(m.vrefresh, 60);
    let mut bad = edid_1024();
    bad[60] ^= 1;
    assert_eq!(modes::edid_modes(&bad), None);
    // The device's size wins when the EDID disagrees; common sizes follow.
    let (list, _) = modes::output_modes((1280, 800), Some(&edid_1024()));
    assert_eq!((list[0].hdisplay, list[0].vdisplay), (1280, 800));
    assert_ne!(list[0].ty & mode::TYPE_PREFERRED, 0);
    assert!(list.iter().skip(1).all(|m| m.ty & mode::TYPE_PREFERRED == 0));
    assert!(list.iter().any(|m| (m.hdisplay, m.vdisplay) == (1024, 768) && m.clock == 65_000));
    assert!(list.iter().all(|m| m.hdisplay <= 1280 && m.vdisplay <= 800));
}

// --- Enumeration ---------------------------------------------------------------

#[test]
fn version_unique_and_caps() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    let name = t.user.alloc(64);
    let mut v = Version { name_len: 4, name, ..Version::default() }.to_bytes();
    t.ok(f, ioctl::VERSION, &mut v);
    let v = Version::decode(&v);
    assert_eq!((v.major, v.minor, v.name_len, v.desc_len), (0, 1, 10, 11));
    // The 4 bytes asked for, no terminating NUL.
    assert_eq!(t.user.get(name, 5), b"virt\0");
    let mut c = GetCap { capability: cap::DUMB_BUFFER, value: 0 }.to_bytes();
    t.ok(f, ioctl::GET_CAP, &mut c);
    assert_eq!(GetCap::decode(&c).value, 1);
    for (cap, want) in
        [(cap::PRIME, 3), (cap::TIMESTAMP_MONOTONIC, 1), (cap::CRTC_IN_VBLANK_EVENT, 1), (cap::ADDFB2_MODIFIERS, 1)]
    {
        let mut c = GetCap { capability: cap, value: 0 }.to_bytes();
        t.ok(f, ioctl::GET_CAP, &mut c);
        assert_eq!(GetCap::decode(&c).value, want, "cap {cap}");
    }
    let mut c = GetCap { capability: 0x99, value: 0 }.to_bytes();
    assert_eq!(t.call(f, ioctl::GET_CAP, &mut c), Err(Errno::EINVAL));
    assert_eq!(t.set_cap(f, client_cap::ATOMIC, 2), Err(Errno::EINVAL));
    assert_eq!(t.set_cap(f, client_cap::WRITEBACK_CONNECTORS, 1), Err(Errno::EINVAL));
    assert_eq!(t.set_cap(f, client_cap::ATOMIC, 1), Ok(Reply::Done(0)));
    assert_eq!(t.set_cap(f, client_cap::WRITEBACK_CONNECTORS, 1), Ok(Reply::Done(0)));
    // Another ioctl type, and an unknown DRM one.
    assert_eq!(t.call(f, 0x5401, &mut [0u8; 4]), Err(Errno::ENOTTY));
    assert_eq!(t.call(f, Request::encode(3, 0x7f, 8), &mut [0u8; 8]), Err(Errno::EINVAL));
}

#[test]
fn resources_connectors_encoders_crtcs_and_planes() {
    let mut t = T::new(vec![output(1024, 768), Output::default()]);
    let f = t.dev.open(Node::Primary);
    let (fbs, crtcs, conns, encs) = t.resources(f);
    assert_eq!((fbs.len(), crtcs.len(), conns.len(), encs.len()), (0, 2, 2, 2));
    let (c, modes) = t.connector(f, conns[0]);
    assert_eq!(c.connection, mode::CONNECTED);
    assert_eq!(c.connector_type, mode::CONNECTOR_VIRTUAL);
    assert_eq!((c.connector_type_id, c.count_encoders, c.encoder_id), (1, 1, 0));
    assert_eq!((modes[0].hdisplay, modes[0].vdisplay), (1024, 768));
    assert_ne!(modes[0].ty & mode::TYPE_PREFERRED, 0);
    assert!(modes.iter().any(|m| (m.hdisplay, m.vdisplay) == (800, 600)));
    // Without the atomic cap, CRTC_ID (atomic) is not listed.
    assert_eq!(c.count_props, 4);
    let (c1, modes1) = t.connector(f, conns[1]);
    assert_eq!((c1.connection, modes1.len(), c1.connector_type_id), (mode::DISCONNECTED, 0, 2));
    let mut e = GetEncoder { encoder_id: encs[1], ..GetEncoder::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETENCODER, &mut e);
    let e = GetEncoder::decode(&e);
    assert_eq!((e.encoder_type, e.possible_crtcs, e.crtc_id), (mode::ENCODER_VIRTUAL, 2, 0));
    let mut g = ModeCrtc { crtc_id: crtcs[0], ..ModeCrtc::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETCRTC, &mut g);
    assert_eq!((ModeCrtc::decode(&g).mode_valid, ModeCrtc::decode(&g).fb_id), (0, 0));
    let mut g = ModeCrtc { crtc_id: conns[0], ..ModeCrtc::default() }.to_bytes();
    assert_eq!(t.call(f, ioctl::MODE_GETCRTC, &mut g), Err(Errno::ENOENT));
    // Planes appear with universal planes only.
    let mut r = GetPlaneRes::default().to_bytes();
    t.ok(f, ioctl::MODE_GETPLANERESOURCES, &mut r);
    assert_eq!(GetPlaneRes::decode(&r).count_planes, 0);
    t.set_cap(f, client_cap::UNIVERSAL_PLANES, 1).unwrap();
    let ptr = t.user.alloc(8);
    let mut r = GetPlaneRes { plane_id_ptr: ptr, count_planes: 2 }.to_bytes();
    t.ok(f, ioctl::MODE_GETPLANERESOURCES, &mut r);
    let planes = t.user.u32s(ptr, 2);
    let fmts = t.user.alloc(16);
    let mut p = GetPlane { plane_id: planes[0], format_type_ptr: fmts, count_format_types: 4, ..GetPlane::default() }
        .to_bytes();
    t.ok(f, ioctl::MODE_GETPLANE, &mut p);
    assert_eq!(GetPlane::decode(&p).possible_crtcs, 1);
    assert_eq!(t.user.u32s(fmts, 4), vec![fourcc::XRGB8888, fourcc::ARGB8888, fourcc::XBGR8888, fourcc::ABGR8888]);
    // The plane's type is Primary; its IN_FORMATS blob lists the formats
    // with the linear modifier.
    let (ids, values) = t.obj_props(f, planes[0], mode::OBJECT_PLANE);
    assert_eq!(ids.len(), 2, "type and IN_FORMATS without the atomic cap");
    let ty = t.prop_named(f, &ids, "type");
    assert_eq!(values[ids.iter().position(|&i| i == ty).unwrap()], mode::PLANE_PRIMARY);
    let blob = values[ids.iter().position(|&i| i == t.prop_named(f, &ids, "IN_FORMATS")).unwrap()] as u32;
    let data = t.user.alloc(128);
    let mut g = GetBlob { blob_id: blob, length: 128, data }.to_bytes();
    t.ok(f, ioctl::MODE_GETPROPBLOB, &mut g);
    let len = GetBlob::decode(&g).length as usize;
    let bytes = t.user.get(data, len);
    let h = FormatModifierBlob::decode(&bytes);
    assert_eq!((h.version, h.count_formats, h.count_modifiers), (1, 4, 1));
    let fm = FormatModifier::decode(&bytes[h.modifiers_offset as usize..]);
    assert_eq!((fm.formats, fm.modifier), (0xf, fourcc::MOD_LINEAR));
    t.set_cap(f, client_cap::ATOMIC, 1).unwrap();
    assert_eq!(t.obj_props(f, planes[0], mode::OBJECT_ANY).0.len(), 12);
    assert_eq!(t.obj_props(f, crtcs[0], mode::OBJECT_CRTC).0.len(), 2);
    assert_eq!(t.connector(f, conns[0]).0.count_props, 5);
    let mut g =
        ObjGetProperties { obj_id: crtcs[0], obj_type: mode::OBJECT_PLANE, ..ObjGetProperties::default() }.to_bytes();
    assert_eq!(t.call(f, ioctl::MODE_OBJ_GETPROPERTIES, &mut g), Err(Errno::ENOENT));
}

#[test]
fn properties_describe_themselves() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    t.set_cap(f, client_cap::ATOMIC, 1).unwrap();
    let (_, conns, _) = t.ids(f);
    let (ids, _) = t.obj_props(f, conns, mode::OBJECT_CONNECTOR);
    let dpms = t.prop_named(f, &ids, "DPMS");
    let enums = t.user.alloc(4 * 40);
    let values = t.user.alloc(4 * 8);
    let mut g = GetProperty {
        prop_id: dpms,
        values_ptr: values,
        enum_blob_ptr: enums,
        count_values: 4,
        count_enum_blobs: 4,
        ..GetProperty::default()
    }
    .to_bytes();
    t.ok(f, ioctl::MODE_GETPROPERTY, &mut g);
    let g = GetProperty::decode(&g);
    assert_eq!((g.flags, g.count_values, g.count_enum_blobs), (mode::PROP_ENUM, 4, 4));
    let e = PropertyEnum::decode(&t.user.get(enums + 3 * 40, 40));
    assert_eq!((e.value, name_str(&e.name)), (3, "Off"));
    let crtc_id = t.prop_named(f, &ids, "CRTC_ID");
    let mut g =
        GetProperty { prop_id: crtc_id, values_ptr: values, count_values: 1, ..GetProperty::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETPROPERTY, &mut g);
    assert_eq!(GetProperty::decode(&g).flags, mode::PROP_OBJECT | mode::PROP_ATOMIC);
    assert_eq!(t.user.u64s(values, 1), vec![mode::OBJECT_CRTC as u64]);
}

// --- Dumb buffers and framebuffers -------------------------------------------------

#[test]
fn dumb_buffers_map_and_go_away() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    let g = t.dev.open(Node::Primary);
    let d = t.dumb(f, 100, 10);
    assert_eq!((d.handle, d.pitch, d.size), (1, 400, 4096));
    let mut m = MapDumb { handle: d.handle, ..MapDumb::default() }.to_bytes();
    t.ok(f, ioctl::MODE_MAP_DUMB, &mut m);
    let offset = MapDumb::decode(&m).offset;
    assert_eq!(offset % 4096, 0);
    let map = t.dev.map(f, offset, 4096).unwrap();
    assert_eq!((map.start, map.size), (0, 4096));
    map.buffer.bytes.borrow_mut()[0] = 7;
    assert_eq!(t.dev.map(g, offset, 4096).err(), Some(Errno::EACCES));
    assert_eq!(t.dev.map(f, offset, 8192).err(), Some(Errno::EINVAL));
    assert_eq!(t.dev.map(f, offset + (1 << 32), 4096).err(), Some(Errno::EINVAL));
    let mut bad = CreateDumb { width: 0, height: 10, bpp: 32, ..CreateDumb::default() }.to_bytes();
    assert_eq!(t.call(f, ioctl::MODE_CREATE_DUMB, &mut bad), Err(Errno::EINVAL));
    // Destroyed: no mapping any more, but the memory the mapping holds stays.
    t.ok(f, ioctl::MODE_DESTROY_DUMB, &mut d.handle.to_le_bytes());
    assert_eq!(t.dev.map(f, offset, 4096).err(), Some(Errno::EINVAL));
    assert_eq!(*t.dev.hardware().live.borrow(), 1);
    assert_eq!(map.buffer.bytes.borrow()[0], 7);
    drop(map);
    assert_eq!(*t.dev.hardware().live.borrow(), 0);
    assert_eq!(t.call(f, ioctl::MODE_DESTROY_DUMB, &mut d.handle.to_le_bytes()), Err(Errno::EINVAL));
}

#[test]
fn framebuffers_check_their_buffer() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    let d = t.dumb(f, 64, 64);
    let fb = t.addfb2(f, &d, 64, 64).unwrap();
    assert!(
        matches!(t.calls()[..], [Call::CreateFb(id, FbLayout { width: 64, height: 64, pitch: 256, offset: 0, .. })] if id == fb)
    );
    assert_eq!(t.addfb2(f, &d, 64, 65), Err(Errno::EINVAL), "past the end of the buffer");
    assert_eq!(t.addfb2(f, &d, 65, 64), Err(Errno::EINVAL), "lines longer than the pitch");
    let mut c = FbCmd2 { width: 64, height: 64, pixel_format: fourcc::RGB565, ..FbCmd2::default() };
    c.handles[0] = d.handle;
    c.pitches[0] = 256;
    assert_eq!(t.call(f, ioctl::MODE_ADDFB2, &mut c.to_bytes()), Err(Errno::EINVAL));
    c.pixel_format = fourcc::ARGB8888;
    c.flags = mode::FB_MODIFIERS;
    c.modifier[0] = 0x0100_0000_0000_0001;
    assert_eq!(t.call(f, ioctl::MODE_ADDFB2, &mut c.to_bytes()), Err(Errno::EINVAL), "a tiled modifier");
    c.modifier[0] = fourcc::MOD_LINEAR;
    let mut b = c.to_bytes();
    t.ok(f, ioctl::MODE_ADDFB2, &mut b);
    let fb2 = FbCmd2::decode(&b).fb_id;
    // Legacy ADDFB: 32 bpp, depth 24 is XRGB8888.
    let mut a = FbCmd { width: 64, height: 64, pitch: 256, bpp: 32, depth: 24, handle: d.handle, ..FbCmd::default() }
        .to_bytes();
    t.ok(f, ioctl::MODE_ADDFB, &mut a);
    let fb3 = FbCmd::decode(&a).fb_id;
    let mut g = FbCmd2 { fb_id: fb2, ..FbCmd2::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETFB2, &mut g);
    let g = FbCmd2::decode(&g);
    assert_eq!(
        (g.pixel_format, g.flags, g.pitches[0], g.handles[0]),
        (fourcc::ARGB8888, mode::FB_MODIFIERS, 256, d.handle)
    );
    let (fbs, ..) = t.resources(f);
    assert_eq!(fbs, vec![fb, fb2, fb3]);
    // Only the owner removes a framebuffer; the buffer lives while one uses it.
    let other = t.dev.open(Node::Primary);
    assert_eq!(t.call(other, ioctl::MODE_RMFB, &mut fb.to_le_bytes()), Err(Errno::ENOENT));
    t.ok(f, ioctl::MODE_DESTROY_DUMB, &mut d.handle.to_le_bytes());
    assert_eq!(t.dev.object_counts(), (1, 3));
    for id in [fb, fb2, fb3] {
        t.ok(f, ioctl::MODE_RMFB, &mut id.to_le_bytes());
    }
    assert_eq!(t.dev.object_counts(), (0, 0));
    assert_eq!(*t.dev.hardware().live.borrow(), 0);
}

// --- Modesetting -------------------------------------------------------------------

#[test]
fn setcrtc_shows_a_framebuffer_and_takes_the_display() {
    let mut t = T::one();
    t.calls();
    let (f, crtc, conn, fb, mode) = t.showing();
    let full = Rect::new(0, 0, 1024, 768);
    let calls = t.calls();
    assert_eq!(
        calls[1..],
        [Call::Take, Call::Upload(fb, full), Call::Scanout(0, Some((fb, full))), Call::Flush(fb, full)]
    );
    assert!(t.dev.display_taken());
    let mut g = ModeCrtc { crtc_id: crtc, ..ModeCrtc::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETCRTC, &mut g);
    let g = ModeCrtc::decode(&g);
    assert_eq!((g.fb_id, g.mode_valid, g.mode.hdisplay), (fb, 1, 1024));
    let (c, _) = t.connector(f, conn);
    assert_ne!(c.encoder_id, 0);
    // A smaller mode shows the top-left corner, or another one with x, y.
    let small = modes::cvt_rb(800, 600, 60);
    let mut req = ModeCrtc {
        crtc_id: crtc,
        fb_id: fb,
        x: 100,
        y: 50,
        count_connectors: 1,
        mode_valid: 1,
        mode: small,
        ..ModeCrtc::default()
    };
    req.set_connectors_ptr = t.user.put(&conn.to_le_bytes());
    t.ok(f, ioctl::MODE_SETCRTC, &mut req.to_bytes());
    let r = Rect::new(100, 50, 800, 600);
    assert_eq!(t.calls(), [Call::Upload(fb, r), Call::Scanout(0, Some((fb, r))), Call::Flush(fb, r)]);
    req.x = 300;
    assert_eq!(t.call(f, ioctl::MODE_SETCRTC, &mut req.to_bytes()), Err(Errno::ENOSPC));
    // Not the master: no modesetting.
    let g2 = t.dev.open(Node::Primary);
    assert_eq!(t.setcrtc(g2, crtc, fb, conn, Some(mode)), Err(Errno::EACCES));
    // Off.
    assert_eq!(t.setcrtc(f, crtc, 0, conn, None), Ok(Reply::Done(0)));
    assert_eq!(t.calls(), [Call::Scanout(0, None)]);
    let _ = mode;
}

#[test]
fn page_flips_complete_at_the_next_vblank_with_an_event() {
    let mut t = T::one();
    let (f, crtc, _, fb, mode) = t.showing();
    let d2 = t.dumb(f, 1024, 768);
    let fb2 = t.addfb2(f, &d2, 1024, 768).unwrap();
    t.calls();
    let set_at = t.now;
    let frame = modes::frame_ns(&mode);
    t.now += frame / 3;
    let mut p =
        PageFlip { crtc_id: crtc, fb_id: fb2, flags: mode::PAGE_FLIP_EVENT, user_data: 0xfeed, ..PageFlip::default() }
            .to_bytes();
    t.ok(f, ioctl::MODE_PAGE_FLIP, &mut p);
    let full = Rect::new(0, 0, 1024, 768);
    assert_eq!(t.calls(), [Call::Upload(fb2, full), Call::Scanout(0, Some((fb2, full))), Call::Flush(fb2, full)]);
    // Pending until the vblank: another flip is refused, nothing to read.
    assert_eq!(t.call(f, ioctl::MODE_PAGE_FLIP, &mut p), Err(Errno::EBUSY));
    assert_eq!(t.read(f), Err(Errno::EAGAIN));
    assert!(!t.dev.readable(f, t.now));
    assert_eq!(t.dev.next_event_ns(), Some(set_at + frame));
    t.now = set_at + frame;
    assert!(t.dev.readable(f, t.now));
    assert_eq!(t.dev.ready_files(t.now), vec![f]);
    let e = t.read(f).unwrap();
    assert_eq!(e.len(), 32);
    let e = EventVblank::decode(&e);
    assert_eq!((e.ty, e.length, e.user_data, e.sequence, e.crtc_id), (event::FLIP_COMPLETE, 32, 0xfeed, 1, crtc));
    assert_eq!(e.tv_sec as u64 * 1_000_000_000 + e.tv_usec as u64 * 1000, (set_at + frame) / 1000 * 1000);
    // The flip back works now; the buffer must be the same format.
    let mut p = PageFlip { crtc_id: crtc, fb_id: fb, flags: 0, ..PageFlip::default() }.to_bytes();
    t.ok(f, ioctl::MODE_PAGE_FLIP, &mut p);
    let d3 = t.dumb(f, 1024, 768);
    let mut c = FbCmd2 { width: 1024, height: 768, pixel_format: fourcc::XBGR8888, ..FbCmd2::default() };
    (c.handles[0], c.pitches[0]) = (d3.handle, d3.pitch);
    let mut b = c.to_bytes();
    t.ok(f, ioctl::MODE_ADDFB2, &mut b);
    t.now += frame;
    let mut p = PageFlip { crtc_id: crtc, fb_id: FbCmd2::decode(&b).fb_id, ..PageFlip::default() }.to_bytes();
    assert_eq!(t.call(f, ioctl::MODE_PAGE_FLIP, &mut p), Err(Errno::EINVAL));
    // A flip aimed at vblank 10.
    let mut p = PageFlip {
        crtc_id: crtc,
        fb_id: fb2,
        flags: mode::PAGE_FLIP_EVENT | mode::PAGE_FLIP_TARGET_ABSOLUTE,
        sequence: 10,
        user_data: 1,
    }
    .to_bytes();
    t.ok(f, ioctl::MODE_PAGE_FLIP, &mut p);
    t.now = set_at + 10 * frame;
    assert_eq!(EventVblank::decode(&t.read(f).unwrap()).sequence, 10);
}

#[test]
fn vblank_waits_sleep_or_queue_an_event() {
    let mut t = T::one();
    let (f, crtc, _, _, mode) = t.showing();
    let frame = modes::frame_ns(&mode);
    let start = t.now;
    let mut w = WaitVblank { ty: vblank::RELATIVE, sequence: 2, ..WaitVblank::default() }.to_bytes();
    let r = t.call(f, ioctl::WAIT_VBLANK, &mut w);
    assert_eq!(r, Ok(Reply::Sleep { until_ns: start + 2 * frame }));
    // Rewritten to an absolute wait for vblank 2; once there, it answers.
    let req = WaitVblank::decode(&w);
    assert_eq!((req.ty, req.sequence), (vblank::ABSOLUTE, 2));
    t.now = start + 2 * frame + 5;
    t.ok(f, ioctl::WAIT_VBLANK, &mut w);
    let reply = WaitVblank::decode(&w);
    assert_eq!(reply.sequence, 2);
    assert_eq!(reply.signal * 1_000_000_000 + reply.tval_usec as u64 * 1000, (start + 2 * frame) / 1000 * 1000);
    // With an event: queued for vblank 4, the reply at once.
    let mut w = WaitVblank { ty: vblank::RELATIVE | vblank::EVENT, sequence: 2, signal: 77, ..WaitVblank::default() }
        .to_bytes();
    t.ok(f, ioctl::WAIT_VBLANK, &mut w);
    assert_eq!(WaitVblank::decode(&w).sequence, 4);
    assert_eq!(t.read(f), Err(Errno::EAGAIN));
    t.now = start + 4 * frame;
    let e = EventVblank::decode(&t.read(f).unwrap());
    assert_eq!((e.ty, e.user_data, e.sequence, e.crtc_id), (event::VBLANK, 77, 4, crtc));
    // CRTC 1 does not exist; a crtc that is off has no vblanks.
    let mut w =
        WaitVblank { ty: vblank::RELATIVE | (1 << vblank::HIGH_CRTC_SHIFT), sequence: 1, ..WaitVblank::default() }
            .to_bytes();
    assert_eq!(t.call(f, ioctl::WAIT_VBLANK, &mut w), Err(Errno::EINVAL));
    let mut s = CrtcGetSequence { crtc_id: crtc, ..CrtcGetSequence::default() }.to_bytes();
    t.ok(f, Request::encode(3, nr::CRTC_GET_SEQUENCE, 24), &mut s);
    let s = CrtcGetSequence::decode(&s);
    assert_eq!((s.active, s.sequence), (1, 4));
}

#[test]
fn events_fill_at_most_4096_bytes_and_read_whole() {
    let mut t = T::one();
    let (f, ..) = t.showing();
    for _ in 0..128 {
        let mut w =
            WaitVblank { ty: vblank::RELATIVE | vblank::EVENT, sequence: 1, ..WaitVblank::default() }.to_bytes();
        t.ok(f, ioctl::WAIT_VBLANK, &mut w);
    }
    let mut w = WaitVblank { ty: vblank::RELATIVE | vblank::EVENT, sequence: 1, ..WaitVblank::default() }.to_bytes();
    assert_eq!(t.call(f, ioctl::WAIT_VBLANK, &mut w), Err(Errno::ENOMEM));
    t.now += 100 * MS;
    assert_eq!(t.dev.read(f, 31, t.now), Err(Errno::EINVAL));
    assert_eq!(t.dev.read(f, 100, t.now).unwrap().len(), 96);
    assert_eq!(t.dev.read(f, 1 << 20, t.now).unwrap().len(), 125 * 32);
    assert_eq!(t.read(f), Err(Errno::EAGAIN));
}

#[test]
fn dirty_fb_uploads_the_clips_where_shown() {
    let mut t = T::one();
    let (f, _, _, fb, _) = t.showing();
    t.calls();
    let clips: Vec<u8> =
        [ClipRect { x1: 10, y1: 20, x2: 30, y2: 40 }, ClipRect { x1: 1000, y1: 700, x2: 2000, y2: 2000 }]
            .iter()
            .flat_map(|c| c.to_bytes())
            .collect();
    let ptr = t.user.put(&clips);
    let mut d = FbDirty { fb_id: fb, num_clips: 2, clips_ptr: ptr, ..FbDirty::default() }.to_bytes();
    t.ok(f, ioctl::MODE_DIRTYFB, &mut d);
    let (a, b) = (Rect::new(10, 20, 20, 20), Rect::new(1000, 700, 24, 68));
    assert_eq!(t.calls(), [Call::Upload(fb, a), Call::Flush(fb, a), Call::Upload(fb, b), Call::Flush(fb, b)]);
    // A framebuffer nobody shows: nothing to do.
    let d2 = t.dumb(f, 16, 16);
    let fb2 = t.addfb2(f, &d2, 16, 16).unwrap();
    t.calls();
    let mut d = FbDirty { fb_id: fb2, ..FbDirty::default() }.to_bytes();
    t.ok(f, ioctl::MODE_DIRTYFB, &mut d);
    assert!(t.calls().is_empty());
}

#[test]
fn removing_a_shown_framebuffer_turns_its_crtc_off() {
    let mut t = T::one();
    let (f, crtc, _, fb, _) = t.showing();
    t.calls();
    t.ok(f, ioctl::MODE_RMFB, &mut fb.to_le_bytes());
    assert_eq!(t.calls(), [Call::Scanout(0, None), Call::DestroyFb(fb)]);
    let mut g = ModeCrtc { crtc_id: crtc, ..ModeCrtc::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETCRTC, &mut g);
    assert_eq!(ModeCrtc::decode(&g).mode_valid, 0);
}

#[test]
fn atomic_commits_check_modeset_and_flip_with_events() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    let (crtc, conn, _) = t.ids(f);
    assert_eq!(t.atomic(f, 0, &[], 0), Err(Errno::EINVAL), "without the atomic cap");
    t.set_cap(f, client_cap::ATOMIC, 1).unwrap();
    let mut r = GetPlaneRes { plane_id_ptr: t.user.alloc(4), count_planes: 1 }.to_bytes();
    t.ok(f, ioctl::MODE_GETPLANERESOURCES, &mut r);
    let plane = t.user.u32s(GetPlaneRes::decode(&r).plane_id_ptr, 1)[0];
    let (pids, _) = t.obj_props(f, plane, mode::OBJECT_PLANE);
    let (cids, _) = t.obj_props(f, crtc, mode::OBJECT_CRTC);
    let (nids, _) = t.obj_props(f, conn, mode::OBJECT_CONNECTOR);
    let p = |t: &mut T, n: &str| t.prop_named(f, &pids, n);
    let (fb_id, crtc_id, sx, sy, sw, sh, cx, cy, cw, ch) = (
        p(&mut t, "FB_ID"),
        p(&mut t, "CRTC_ID"),
        p(&mut t, "SRC_X"),
        p(&mut t, "SRC_Y"),
        p(&mut t, "SRC_W"),
        p(&mut t, "SRC_H"),
        p(&mut t, "CRTC_X"),
        p(&mut t, "CRTC_Y"),
        p(&mut t, "CRTC_W"),
        p(&mut t, "CRTC_H"),
    );
    let active = t.prop_named(f, &cids, "ACTIVE");
    let mode_id = t.prop_named(f, &cids, "MODE_ID");
    let conn_crtc = t.prop_named(f, &nids, "CRTC_ID");
    let (_, modes) = t.connector(f, conn);
    let blob_ptr = t.user.put(&modes[0].to_bytes());
    let mut b = CreateBlob { data: blob_ptr, length: 68, blob_id: 0 }.to_bytes();
    t.ok(f, ioctl::MODE_CREATEPROPBLOB, &mut b);
    let blob = CreateBlob::decode(&b).blob_id;
    let d = t.dumb(f, 1024, 768);
    let fb = t.addfb2(f, &d, 1024, 768).unwrap();
    let d2 = t.dumb(f, 1024, 768);
    let fb2 = t.addfb2(f, &d2, 1024, 768).unwrap();
    let full = |fb: u32| {
        vec![
            (plane, fb_id, fb as u64),
            (plane, crtc_id, crtc as u64),
            (plane, sx, 0),
            (plane, sy, 0),
            (plane, sw, 1024 << 16),
            (plane, sh, 768 << 16),
            (plane, cx, 0),
            (plane, cy, 0),
            (plane, cw, 1024),
            (plane, ch, 768),
        ]
    };
    let mut set = vec![(crtc, active, 1), (crtc, mode_id, blob as u64), (conn, conn_crtc, crtc as u64)];
    set.extend(full(fb));
    t.calls();
    assert_eq!(t.atomic(f, 0, &set, 0), Err(Errno::EINVAL), "a modeset without ALLOW_MODESET");
    assert_eq!(t.atomic(f, mode::ATOMIC_TEST_ONLY | mode::ATOMIC_ALLOW_MODESET, &set, 0), Ok(Reply::Done(0)));
    assert!(t.calls().is_empty(), "a test changes nothing");
    let mut scaled = set.clone();
    scaled.push((plane, cw, 800));
    assert_eq!(t.atomic(f, mode::ATOMIC_ALLOW_MODESET, &scaled, 0), Err(Errno::ERANGE));
    let mut no_conn = set.clone();
    no_conn.retain(|s| s.0 != conn);
    assert_eq!(t.atomic(f, mode::ATOMIC_ALLOW_MODESET, &no_conn, 0), Err(Errno::EINVAL));
    assert_eq!(
        t.atomic(f, mode::ATOMIC_ALLOW_MODESET, &[(crtc, fb_id, 1)], 0),
        Err(Errno::EINVAL),
        "FB_ID is not a CRTC property"
    );
    assert_eq!(t.atomic(f, mode::ATOMIC_ALLOW_MODESET | mode::PAGE_FLIP_EVENT, &set, 5), Ok(Reply::Done(0)));
    let r = Rect::new(0, 0, 1024, 768);
    assert_eq!(t.calls(), [Call::Take, Call::Upload(fb, r), Call::Scanout(0, Some((fb, r))), Call::Flush(fb, r)]);
    // The user's blob is MODE_ID's value; destroying it keeps the mode.
    let (_, cvals) = t.obj_props(f, crtc, mode::OBJECT_CRTC);
    assert!(cvals.contains(&(blob as u64)) && cvals.contains(&1));
    t.ok(f, ioctl::MODE_DESTROYPROPBLOB, &mut blob.to_le_bytes());
    let mut g = GetBlob { blob_id: blob, ..GetBlob::default() }.to_bytes();
    t.ok(f, ioctl::MODE_GETPROPBLOB, &mut g);
    assert_eq!(GetBlob::decode(&g).length, 68);
    // A non-blocking flip while the first commit is pending: EBUSY; a
    // blocking one waits for it.
    let flip = [(plane, fb_id, fb2 as u64)];
    assert_eq!(t.atomic(f, mode::ATOMIC_NONBLOCK | mode::PAGE_FLIP_EVENT, &flip, 6), Err(Errno::EBUSY));
    let due = t.dev.next_event_ns().unwrap();
    assert_eq!(t.atomic(f, mode::PAGE_FLIP_EVENT, &flip, 6), Ok(Reply::Sleep { until_ns: due }));
    t.now = due;
    let e = EventVblank::decode(&t.read(f).unwrap());
    assert_eq!((e.ty, e.user_data, e.crtc_id), (event::FLIP_COMPLETE, 5, crtc));
    assert_eq!(t.atomic(f, mode::ATOMIC_NONBLOCK | mode::PAGE_FLIP_EVENT, &flip, 6), Ok(Reply::Done(0)));
    assert_eq!(t.calls(), [Call::Upload(fb2, r), Call::Scanout(0, Some((fb2, r))), Call::Flush(fb2, r)]);
    // Off again: ACTIVE 0 with the plane and connector let go.
    t.now += 100 * MS;
    let off = [(crtc, active, 0), (crtc, mode_id, 0), (conn, conn_crtc, 0), (plane, fb_id, 0), (plane, crtc_id, 0)];
    assert_eq!(t.atomic(f, mode::ATOMIC_ALLOW_MODESET, &off, 0), Ok(Reply::Done(0)));
    assert_eq!(t.calls(), [Call::Scanout(0, None)]);
    // The device dropped its copy of the mode blob with the state.
    let mut g = GetBlob { blob_id: blob, ..GetBlob::default() }.to_bytes();
    assert_eq!(t.call(f, ioctl::MODE_GETPROPBLOB, &mut g), Err(Errno::ENOENT));
}

#[test]
fn dpms_off_turns_the_crtc_off_and_on() {
    let mut t = T::one();
    let (f, crtc, conn, fb, _) = t.showing();
    let (ids, _) = t.obj_props(f, conn, mode::OBJECT_CONNECTOR);
    let dpms = t.prop_named(f, &ids, "DPMS");
    t.calls();
    let mut s = ConnectorSetProperty { value: mode::DPMS_OFF, prop_id: dpms, connector_id: conn }.to_bytes();
    t.ok(f, Request::encode(3, nr::MODE_SETPROPERTY, 16), &mut s);
    assert_eq!(t.calls(), [Call::Scanout(0, None)]);
    let mut s = ConnectorSetProperty { value: mode::DPMS_ON, prop_id: dpms, connector_id: conn }.to_bytes();
    t.ok(f, Request::encode(3, nr::MODE_SETPROPERTY, 16), &mut s);
    let r = Rect::new(0, 0, 1024, 768);
    assert_eq!(t.calls(), [Call::Upload(fb, r), Call::Scanout(0, Some((fb, r))), Call::Flush(fb, r)]);
    let _ = crtc;
}

// --- Files, master, render nodes, PRIME --------------------------------------------

#[test]
fn master_is_the_first_opener_until_it_lets_go() {
    let mut t = T::one();
    let a = t.dev.open(Node::Primary);
    let b = t.dev.open(Node::Primary);
    assert_eq!(t.dev.master(), Some(a));
    t.privileged = false;
    assert_eq!(t.call(b, ioctl::SET_MASTER, &mut []), Err(Errno::EBUSY));
    assert_eq!(t.call(b, ioctl::DROP_MASTER, &mut []), Err(Errno::EINVAL));
    t.ok(a, ioctl::DROP_MASTER, &mut []);
    assert_eq!(t.call(b, ioctl::SET_MASTER, &mut []), Err(Errno::EACCES), "never master, not privileged");
    t.ok(a, ioctl::SET_MASTER, &mut []);
    t.ok(a, ioctl::DROP_MASTER, &mut []);
    t.privileged = true;
    t.ok(b, ioctl::SET_MASTER, &mut []);
    assert_eq!(t.dev.master(), Some(b));
    // Authentication: the master authenticates another file's magic.
    let mut m = [0u8; 4];
    t.ok(a, Request::encode(2, nr::GET_MAGIC, 4), &mut m);
    t.ok(b, Request::encode(1, nr::AUTH_MAGIC, 4), &mut m);
    assert_eq!(t.call(a, Request::encode(1, nr::AUTH_MAGIC, 4), &mut m), Err(Errno::EACCES));
}

#[test]
fn closing_the_master_gives_the_display_back() {
    let mut t = T::one();
    let (f, _, _, fb, _) = t.showing();
    let other = t.dev.open(Node::Primary);
    t.calls();
    t.dev.close(f, t.now);
    assert_eq!(t.calls(), [Call::Scanout(0, None), Call::DestroyFb(fb), Call::GiveBack]);
    assert!(!t.dev.display_taken());
    assert_eq!(t.dev.master(), None);
    assert_eq!(t.dev.object_counts(), (0, 0));
    assert_eq!(*t.dev.hardware().live.borrow(), 0);
    // The next master shows its own picture again.
    t.ok(other, ioctl::SET_MASTER, &mut []);
    assert_eq!(t.dev.file_count(), 1);
}

#[test]
fn render_nodes_only_share_buffers() {
    let mut t = T::one();
    let card = t.dev.open(Node::Primary);
    let render = t.dev.open(Node::Render);
    assert_eq!(t.dev.master(), Some(card));
    let mut v = Version::default().to_bytes();
    t.ok(render, ioctl::VERSION, &mut v);
    let mut c = CreateDumb { width: 8, height: 8, bpp: 32, ..CreateDumb::default() }.to_bytes();
    assert_eq!(t.call(render, ioctl::MODE_CREATE_DUMB, &mut c), Err(Errno::EACCES));
    let mut r = CardRes::default().to_bytes();
    assert_eq!(t.call(render, ioctl::MODE_GETRESOURCES, &mut r), Err(Errno::EACCES));
    assert_eq!(t.call(render, ioctl::SET_MASTER, &mut []), Err(Errno::EACCES));
    // PRIME: export from the card, import on the render node, twice: one
    // handle. The object lives while the dma-buf does.
    let d = t.dumb(card, 8, 8);
    let mut p = PrimeHandle { handle: d.handle, flags: prime::CLOEXEC | prime::RDWR, fd: -1 }.to_bytes();
    t.ok(card, ioctl::PRIME_HANDLE_TO_FD, &mut p);
    let fd = PrimeHandle::decode(&p).fd;
    assert_eq!(fd, 100);
    assert_eq!(t.fds.files[&fd].1, prime::CLOEXEC | prime::RDWR);
    let mut q = PrimeHandle { fd, ..PrimeHandle::default() }.to_bytes();
    t.ok(render, ioctl::PRIME_FD_TO_HANDLE, &mut q);
    let h = PrimeHandle::decode(&q).handle;
    t.ok(render, ioctl::PRIME_FD_TO_HANDLE, &mut q);
    assert_eq!(PrimeHandle::decode(&q).handle, h);
    let mut q2 = PrimeHandle { fd, ..PrimeHandle::default() }.to_bytes();
    t.ok(card, ioctl::PRIME_FD_TO_HANDLE, &mut q2);
    assert_eq!(PrimeHandle::decode(&q2).handle, d.handle, "the same object, the same handle");
    let mut bad = PrimeHandle { fd: 7, ..PrimeHandle::default() }.to_bytes();
    assert_eq!(t.call(render, ioctl::PRIME_FD_TO_HANDLE, &mut bad), Err(Errno::EBADF));
    let mut bad = PrimeHandle { handle: d.handle, flags: 0x40, fd: -1 }.to_bytes();
    assert_eq!(t.call(card, ioctl::PRIME_HANDLE_TO_FD, &mut bad), Err(Errno::EINVAL));
    t.ok(card, ioctl::GEM_CLOSE, &mut GemClose { handle: d.handle, pad: 0 }.to_bytes());
    t.ok(render, ioctl::GEM_CLOSE, &mut GemClose { handle: h, pad: 0 }.to_bytes());
    assert_eq!(t.dev.object_counts().0, 1, "the dma-buf holds it");
    t.dev.release_dmabuf(t.fds.files[&fd].0);
    assert_eq!(t.dev.object_counts().0, 0);
    assert_eq!(*t.dev.hardware().live.borrow(), 0);
}

#[test]
fn flink_names_open_the_same_object() {
    let mut t = T::one();
    let a = t.dev.open(Node::Primary);
    let b = t.dev.open(Node::Primary);
    let d = t.dumb(a, 16, 16);
    let mut g = GemFlink { handle: d.handle, name: 0 }.to_bytes();
    t.ok(a, Request::encode(3, nr::GEM_FLINK, 8), &mut g);
    let name = GemFlink::decode(&g).name;
    let mut o = GemOpen { name, ..GemOpen::default() }.to_bytes();
    t.ok(b, Request::encode(3, nr::GEM_OPEN, 16), &mut o);
    let o = GemOpen::decode(&o);
    assert_eq!((o.handle, o.size), (1, 4096));
    t.dev.close(a, t.now);
    assert_eq!(t.dev.object_counts().0, 1);
    t.dev.close(b, t.now);
    assert_eq!(t.dev.object_counts().0, 0);
}

#[test]
fn shorter_and_longer_arguments_are_zero_extended_and_cut() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    // An old caller's 8-byte GET_CAP argument: the value is not written
    // back past what it passed.
    let mut c = [0u8; 16];
    c[0] = cap::DUMB_BUFFER as u8;
    c[8..].fill(0xaa);
    let at = t.user.put(&c);
    let mut ctx = Ctx { user: &mut t.user, fds: &mut t.fds, now_ns: t.now, privileged: true };
    assert_eq!(t.dev.ioctl(f, Request::encode(3, nr::GET_CAP, 8), at, &mut ctx), Ok(Reply::Done(0)));
    assert_eq!(&t.user.get(at + 8, 8), &[0xaa; 8]);
    // A newer caller's longer one: the extra bytes are left alone.
    let mut c = [0u8; 24];
    c[0] = cap::DUMB_BUFFER as u8;
    c[16..].fill(0xbb);
    let at = t.user.put(&c);
    let mut ctx = Ctx { user: &mut t.user, fds: &mut t.fds, now_ns: t.now, privileged: true };
    assert_eq!(t.dev.ioctl(f, Request::encode(3, nr::GET_CAP, 24), at, &mut ctx), Ok(Reply::Done(0)));
    let back = t.user.get(at, 24);
    assert_eq!((back[8], &back[16..]), (1, &[0xbb; 8][..]));
}

#[test]
fn a_failing_device_reports_eio() {
    let mut t = T::one();
    let f = t.dev.open(Node::Primary);
    let (crtc, conn, _) = t.ids(f);
    let (_, modes) = t.connector(f, conn);
    let d = t.dumb(f, 1024, 768);
    let fb = t.addfb2(f, &d, 1024, 768).unwrap();
    t.dev.hardware().fail_scanout = true;
    assert_eq!(t.setcrtc(f, crtc, fb, conn, Some(modes[0])), Err(Errno::EIO));
    assert!(format!("{:?}", t.dev).contains("virtio_gpu"));
}

// --- virtio-gpu 3D (virtgpu_drm.h, ADR 0119) ------------------------------------

use crate::uapi::virtgpu as vg;

/// A device whose hardware offers virgl, virgl2 and Venus capsets, blobs,
/// host-visible memory and context init.
fn gpu() -> T {
    let mut t = T::one();
    let caps = GpuCaps {
        virgl: true,
        blob: true,
        host_visible: true,
        context_init: true,
        capsets: vec![
            CapsetInfo { id: 1, max_version: 1, max_size: 308 },
            CapsetInfo { id: 2, max_version: 2, max_size: 1024 },
            CapsetInfo { id: 4, max_version: 0, max_size: 160 },
        ],
    };
    // The device asks once, when it is made.
    t.dev = Device::new(Fake { outputs: vec![output(1024, 768)], caps, ..Fake::default() }, t.dev.info.clone());
    t
}

impl T {
    fn getparam(&mut self, file: FileId, param: u64) -> Result<u32> {
        let at = self.user.put(&[0xff; 8]);
        let mut g = vg::GetParam { param, value: at }.to_bytes();
        self.call(file, vg::ioctl::GETPARAM, &mut g)?;
        let v = self.user.get(at, 8);
        // An int: the next 4 bytes stay as they were.
        assert_eq!(&v[4..], &[0xff; 4]);
        Ok(u32::from_le_bytes(v[..4].try_into().unwrap()))
    }

    fn create_3d(&mut self, file: FileId, width: u32, height: u32, size: u32) -> vg::ResourceCreate {
        let mut r = vg::ResourceCreate {
            target: 2,
            format: 1,
            bind: 2,
            width,
            height,
            depth: 1,
            array_size: 1,
            size,
            ..Default::default()
        }
        .to_bytes();
        self.ok(file, vg::ioctl::RESOURCE_CREATE, &mut r);
        vg::ResourceCreate::decode(&r)
    }

    fn execbuffer(
        &mut self,
        file: FileId,
        flags: u32,
        cmds: &[u8],
        handles: &[u32],
        fence_fd: i32,
        ring: u32,
    ) -> (Result<Reply>, vg::Execbuffer) {
        let command = self.user.put(cmds);
        let list: Vec<u8> = handles.iter().flat_map(|h| h.to_le_bytes()).collect();
        let bo_handles = self.user.put(&list);
        let mut e = vg::Execbuffer {
            flags,
            size: cmds.len() as u32,
            command,
            bo_handles,
            num_bo_handles: handles.len() as u32,
            fence_fd,
            ring_idx: ring,
            ..Default::default()
        }
        .to_bytes();
        let r = self.call(file, vg::ioctl::EXECBUFFER, &mut e);
        (r, vg::Execbuffer::decode(&e))
    }
}

#[test]
fn virtgpu_request_numbers_match_the_header() {
    // DRM_IOWR(DRM_COMMAND_BASE + DRM_VIRTGPU_*, struct ...), as libdrm
    // computes them for x86-64.
    assert_eq!(vg::ioctl::MAP, 0xc010_6441);
    assert_eq!(vg::ioctl::EXECBUFFER, 0xc040_6442);
    assert_eq!(vg::ioctl::GETPARAM, 0xc010_6443);
    assert_eq!(vg::ioctl::RESOURCE_CREATE, 0xc038_6444);
    assert_eq!(vg::ioctl::RESOURCE_INFO, 0xc010_6445);
    assert_eq!(vg::ioctl::TRANSFER_FROM_HOST, 0xc02c_6446);
    assert_eq!(vg::ioctl::TRANSFER_TO_HOST, 0xc02c_6447);
    assert_eq!(vg::ioctl::WAIT, 0xc008_6448);
    assert_eq!(vg::ioctl::GET_CAPS, 0xc018_6449);
    assert_eq!(vg::ioctl::RESOURCE_CREATE_BLOB, 0xc030_644a);
    assert_eq!(vg::ioctl::CONTEXT_INIT, 0xc010_644b);
}

#[test]
fn getparam_and_capsets() {
    let mut t = gpu();
    let f = t.dev.open(Node::Render);
    assert_eq!(t.getparam(f, vg::param::THREE_D_FEATURES), Ok(1));
    assert_eq!(t.getparam(f, vg::param::CAPSET_QUERY_FIX), Ok(1));
    assert_eq!(t.getparam(f, vg::param::RESOURCE_BLOB), Ok(1));
    assert_eq!(t.getparam(f, vg::param::HOST_VISIBLE), Ok(1));
    assert_eq!(t.getparam(f, vg::param::CROSS_DEVICE), Ok(0));
    assert_eq!(t.getparam(f, vg::param::CONTEXT_INIT), Ok(1));
    assert_eq!(t.getparam(f, vg::param::SUPPORTED_CAPSET_IDS), Ok(1 << 1 | 1 << 2 | 1 << 4));
    assert_eq!(t.getparam(f, 99), Err(Errno::EINVAL));
    // A 2D-only device says so.
    let mut plain = T::one();
    let p = plain.dev.open(Node::Render);
    assert_eq!(plain.getparam(p, vg::param::THREE_D_FEATURES), Ok(0));
    assert_eq!(plain.getparam(p, vg::param::SUPPORTED_CAPSET_IDS), Ok(0));

    // GET_CAPS copies at most `size` bytes of the capset, fetched once.
    let out = t.user.alloc(2048);
    let mut g = vg::GetCaps { cap_set_id: 2, cap_set_ver: 2, addr: out, size: 2048, pad: 0 }.to_bytes();
    t.ok(f, vg::ioctl::GET_CAPS, &mut g);
    let data = t.user.get(out, 1024);
    assert_eq!(data[..4], [2, 3, 0, 1]);
    let mut g = vg::GetCaps { cap_set_id: 2, cap_set_ver: 2, addr: out, size: 4, pad: 0 }.to_bytes();
    t.ok(f, vg::ioctl::GET_CAPS, &mut g);
    assert_eq!(t.calls(), [Call::Capset(2, 2)]);
    // Unknown capsets, versions above the newest, and no 3D at all.
    let mut g = vg::GetCaps { cap_set_id: 3, cap_set_ver: 0, addr: out, size: 4, pad: 0 }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::GET_CAPS, &mut g), Err(Errno::EINVAL));
    let mut g = vg::GetCaps { cap_set_id: 1, cap_set_ver: 2, addr: out, size: 4, pad: 0 }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::GET_CAPS, &mut g), Err(Errno::EINVAL));
    let mut g = vg::GetCaps { cap_set_id: 1, cap_set_ver: 1, addr: out, size: 4, pad: 0 }.to_bytes();
    assert_eq!(plain.call(p, vg::ioctl::GET_CAPS, &mut g), Err(Errno::ENOSYS));
}

#[test]
fn context_init_picks_the_capset_once() {
    let mut t = gpu();
    let f = t.dev.open(Node::Render);
    let name = t.user.put(b"vulkaninfo\0");
    let params: Vec<u8> = [
        vg::ContextSetParam { param: vg::context_param::CAPSET_ID, value: 4 },
        vg::ContextSetParam { param: vg::context_param::NUM_RINGS, value: 2 },
        vg::ContextSetParam { param: vg::context_param::POLL_RINGS_MASK, value: 0 },
        vg::ContextSetParam { param: vg::context_param::DEBUG_NAME, value: name },
    ]
    .iter()
    .flat_map(|p| p.to_bytes())
    .collect();
    let at = t.user.put(&params);
    let mut c = vg::ContextInit { num_params: 4, pad: 0, ctx_set_params: at }.to_bytes();
    t.ok(f, vg::ioctl::CONTEXT_INIT, &mut c);
    assert_eq!(t.calls(), [Call::CtxCreate(f, 4, b"vulkaninfo".to_vec())]);
    assert_eq!(t.call(f, vg::ioctl::CONTEXT_INIT, &mut c), Err(Errno::EEXIST));
    // An unsupported capset, too many rings, a mask beyond the rings, an
    // unknown parameter.
    for bad in [
        [(vg::context_param::CAPSET_ID, 3u64), (vg::context_param::NUM_RINGS, 1)],
        [(vg::context_param::CAPSET_ID, 2), (vg::context_param::NUM_RINGS, 65)],
        [(vg::context_param::NUM_RINGS, 1), (vg::context_param::POLL_RINGS_MASK, 2)],
        [(vg::context_param::CAPSET_ID, 2), (9, 0)],
    ] {
        let g = t.dev.open(Node::Render);
        let raw: Vec<u8> =
            bad.iter().flat_map(|&(param, value)| vg::ContextSetParam { param, value }.to_bytes()).collect();
        let at = t.user.put(&raw);
        let mut c = vg::ContextInit { num_params: 2, pad: 0, ctx_set_params: at }.to_bytes();
        assert_eq!(t.call(g, vg::ioctl::CONTEXT_INIT, &mut c), Err(Errno::EINVAL), "{bad:?}");
    }
    assert!(t.calls().is_empty());
    // Closing the file destroys its context.
    t.dev.close(f, t.now);
    assert_eq!(t.calls(), [Call::CtxDestroy(f)]);
}

#[test]
fn resources_map_transfer_wait_and_go_with_their_last_handle() {
    let mut t = gpu();
    t.dev.hardware().hold_fences = true;
    let f = t.dev.open(Node::Render);
    // The first resource makes the file's default (virgl) context.
    let r = t.create_3d(f, 64, 64, 64 * 64 * 4);
    assert_eq!(r.res_handle, RESOURCE_BASE + 1);
    let res = r.res_handle;
    let calls = t.calls();
    assert_eq!(calls[0], Call::CtxCreate(f, 0, b"poly".to_vec()));
    assert!(
        matches!(calls[1], Call::Create(id, Some(c), ResourceKind::ThreeD(Create3d { width: 64, height: 64, target: 2, .. })) if id == res && c == f)
    );
    assert_eq!(calls[2], Call::Attach(f, res));
    let mut i = vg::ResourceInfo { bo_handle: r.bo_handle, ..Default::default() }.to_bytes();
    t.ok(f, vg::ioctl::RESOURCE_INFO, &mut i);
    assert_eq!(
        vg::ResourceInfo::decode(&i),
        vg::ResourceInfo { bo_handle: r.bo_handle, res_handle: res, size: 16384, blob_mem: 0 }
    );
    // MAP gives the fake offset; mapping it maps the guest memory.
    let mut m = vg::Map { handle: r.bo_handle, ..Default::default() }.to_bytes();
    t.ok(f, vg::ioctl::MAP, &mut m);
    let offset = vg::Map::decode(&m).offset;
    let map = t.dev.map(f, offset, 16384).unwrap();
    assert_eq!(map.size, 16384);
    // A transfer is fenced; WAIT sleeps until the fence signals.
    let mut x =
        vg::Transfer { bo_handle: r.bo_handle, w: 64, h: 64, d: 1, stride: 256, ..Default::default() }.to_bytes();
    t.ok(f, vg::ioctl::TRANSFER_FROM_HOST, &mut x);
    let fence = t.dev.hardware().fence;
    assert!(
        matches!(&t.calls()[..], [Call::Transfer(false, Some(c), id, tr)] if *c == f && *id == res && tr.w == 64 && tr.stride == 256)
    );
    let mut w = vg::Wait { handle: r.bo_handle, flags: vg::WAIT_NOWAIT }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::WAIT, &mut w), Err(Errno::EBUSY));
    let mut w = vg::Wait { handle: r.bo_handle, flags: 0 }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::WAIT, &mut w), Ok(Reply::WaitFence { fence }));
    t.dev.hardware().unsignalled.clear();
    t.ok(f, vg::ioctl::WAIT, &mut w);
    let mut w = vg::Wait { handle: 77, flags: 0 }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::WAIT, &mut w), Err(Errno::ENOENT));
    // Shared through PRIME with another file, it lives on until both let go.
    let mut p = PrimeHandle { handle: r.bo_handle, flags: prime::RDWR, fd: -1 }.to_bytes();
    t.ok(f, ioctl::PRIME_HANDLE_TO_FD, &mut p);
    let g = t.dev.open(Node::Render);
    let mut q = PrimeHandle { fd: PrimeHandle::decode(&p).fd, ..Default::default() }.to_bytes();
    t.ok(g, ioctl::PRIME_FD_TO_HANDLE, &mut q);
    let gh = PrimeHandle::decode(&q).handle;
    // The second file's first submission makes its context and attaches.
    let (r2, _) = t.execbuffer(g, 0, &[1, 2, 3, 4], &[gh], -1, 0);
    assert_eq!(r2, Ok(Reply::Done(0)));
    let calls = t.calls();
    assert_eq!(
        calls,
        [Call::CtxCreate(g, 0, b"poly".to_vec()), Call::Attach(g, res), Call::Submit(g, None, vec![1, 2, 3, 4])]
    );
    let mut c = GemClose { handle: r.bo_handle, pad: 0 }.to_bytes();
    t.ok(f, ioctl::GEM_CLOSE, &mut c);
    drop(map);
    t.dev.release_dmabuf(BoId(1));
    assert!(t.calls().is_empty(), "the second file still has it");
    let mut c = GemClose { handle: gh, pad: 0 }.to_bytes();
    t.ok(g, ioctl::GEM_CLOSE, &mut c);
    // Detached from both contexts, then destroyed.
    let calls = t.calls();
    assert!(calls.contains(&Call::Detach(f, res)) && calls.contains(&Call::Detach(g, res)));
    assert_eq!(calls.last(), Some(&Call::Destroy(res)));
    assert_eq!(t.dev.object_counts(), (0, 0));
}

#[test]
fn execbuffer_fences_rings_and_sync_files() {
    let mut t = gpu();
    t.dev.hardware().hold_fences = true;
    let f = t.dev.open(Node::Render);
    let params: Vec<u8> = [
        vg::ContextSetParam { param: vg::context_param::CAPSET_ID, value: 2 },
        vg::ContextSetParam { param: vg::context_param::NUM_RINGS, value: 2 },
    ]
    .iter()
    .flat_map(|p| p.to_bytes())
    .collect();
    let at = t.user.put(&params);
    let mut c = vg::ContextInit { num_params: 2, pad: 0, ctx_set_params: at }.to_bytes();
    t.ok(f, vg::ioctl::CONTEXT_INIT, &mut c);
    let a = t.create_3d(f, 16, 16, 1024);
    t.calls();
    // An out-fence: a sync file for the submission's fence.
    let (r, e) = t.execbuffer(f, vg::execbuf::FENCE_FD_OUT | vg::execbuf::RING_IDX, b"cmds", &[a.bo_handle], -1, 1);
    assert_eq!(r, Ok(Reply::Done(0)));
    assert_eq!(t.calls(), [Call::Submit(f, Some(1), b"cmds".to_vec())]);
    let first = t.dev.hardware().fence;
    assert_eq!(t.fds.fences[&e.fence_fd], [first]);
    // The buffer is busy with it.
    let mut w = vg::Wait { handle: a.bo_handle, flags: vg::WAIT_NOWAIT }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::WAIT, &mut w), Err(Errno::EBUSY));
    // An in-fence that has not signalled: wait for it, nothing submitted.
    let (r, _) = t.execbuffer(f, vg::execbuf::FENCE_FD_IN, b"next", &[], e.fence_fd, 0);
    assert_eq!(r, Ok(Reply::WaitFence { fence: first }));
    assert!(t.calls().is_empty());
    t.dev.hardware().unsignalled.clear();
    let (r, _) = t.execbuffer(f, vg::execbuf::FENCE_FD_IN, b"next", &[], e.fence_fd, 0);
    assert_eq!(r, Ok(Reply::Done(0)));
    assert_eq!(t.calls(), [Call::Submit(f, None, b"next".to_vec())]);
    t.ok(f, vg::ioctl::WAIT, &mut vg::Wait { handle: a.bo_handle, flags: vg::WAIT_NOWAIT }.to_bytes());
    // A ring beyond the context's, unknown flags, sync objects, a bad handle.
    assert_eq!(t.execbuffer(f, vg::execbuf::RING_IDX, b"x", &[], -1, 2).0, Err(Errno::EINVAL));
    assert_eq!(t.execbuffer(f, 0x80, b"x", &[], -1, 0).0, Err(Errno::EINVAL));
    assert_eq!(t.execbuffer(f, 0, b"x", &[99], -1, 0).0, Err(Errno::ENOENT));
    let mut e = vg::Execbuffer { num_in_syncobjs: 1, ..Default::default() }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::EXECBUFFER, &mut e), Err(Errno::EINVAL));
    // The old, 40-byte structure (before the sync objects) still works.
    let command = t.user.put(b"old!");
    let mut old = vg::Execbuffer { size: 4, command, fence_fd: -1, ..Default::default() }.to_bytes()[..40].to_vec();
    let req = Request::encode(dir::READ | dir::WRITE, vg::nr::EXECBUFFER, 40);
    t.ok(f, req, &mut old);
    assert_eq!(t.calls(), [Call::Submit(f, None, b"old!".to_vec())]);
    // A dumb buffer is not a resource: refused by the 3D ioctls.
    let mut d = CreateDumb { width: 8, height: 8, bpp: 32, ..Default::default() }.to_bytes();
    let card = t.dev.open(Node::Primary);
    t.ok(card, ioctl::MODE_CREATE_DUMB, &mut d);
    let dh = CreateDumb::decode(&d).handle;
    assert_eq!(t.execbuffer(card, 0, b"x", &[dh], -1, 0).0, Err(Errno::EINVAL));
}

#[test]
fn blobs_guest_host_and_mappable() {
    let mut t = gpu();
    let f = t.dev.open(Node::Render);
    let blob = |mem, flags, size, blob_id, cmd: u64, cmd_size| {
        vg::ResourceCreateBlob { blob_mem: mem, blob_flags: flags, size, blob_id, cmd, cmd_size, ..Default::default() }
            .to_bytes()
    };
    // A mappable host blob made by a command stream first (Venus' way):
    // the commands go before the resource, and MAP maps its window.
    let cmd = t.user.put(b"alloc memory");
    let mut b = blob(vg::blob::MEM_HOST3D, vg::blob::FLAG_USE_MAPPABLE, 8192, 42, cmd, 12);
    t.ok(f, vg::ioctl::RESOURCE_CREATE_BLOB, &mut b);
    let rb = vg::ResourceCreateBlob::decode(&b);
    let calls = t.calls();
    assert_eq!(calls[0], Call::CtxCreate(f, 0, b"poly".to_vec()));
    assert_eq!(calls[1], Call::Submit(f, None, b"alloc memory".to_vec()));
    assert_eq!(
        calls[2],
        Call::Create(rb.res_handle, Some(f), ResourceKind::Blob { mem: 2, flags: 1, blob_id: 42, size: 8192 })
    );
    let mut m = vg::Map { handle: rb.bo_handle, ..Default::default() }.to_bytes();
    t.ok(f, vg::ioctl::MAP, &mut m);
    assert!(t.dev.map(f, vg::Map::decode(&m).offset, 8192).is_ok());
    // A host blob that is not mappable has nothing to map.
    let mut b = blob(vg::blob::MEM_HOST3D, 0, 4096, 43, 0, 0);
    t.ok(f, vg::ioctl::RESOURCE_CREATE_BLOB, &mut b);
    let mut m = vg::Map { handle: vg::ResourceCreateBlob::decode(&b).bo_handle, ..Default::default() }.to_bytes();
    assert_eq!(t.call(f, vg::ioctl::MAP, &mut m), Err(Errno::EINVAL));
    // A guest blob: guest memory, no context needed.
    let g = t.dev.open(Node::Render);
    let mut b = blob(vg::blob::MEM_GUEST, vg::blob::FLAG_USE_MAPPABLE, 5000, 0, 0, 0);
    t.ok(g, vg::ioctl::RESOURCE_CREATE_BLOB, &mut b);
    let rb = vg::ResourceCreateBlob::decode(&b);
    let mut i = vg::ResourceInfo { bo_handle: rb.bo_handle, ..Default::default() }.to_bytes();
    t.ok(g, vg::ioctl::RESOURCE_INFO, &mut i);
    assert_eq!((vg::ResourceInfo::decode(&i).size, vg::ResourceInfo::decode(&i).blob_mem), (8192, 1));
    t.calls();
    // Bad kinds, flags and sizes.
    for bad in [blob(0, 0, 4096, 0, 0, 0), blob(4, 0, 4096, 0, 0, 0), blob(1, 8, 4096, 0, 0, 0), blob(1, 0, 0, 0, 0, 0)]
    {
        let mut bad = bad;
        assert_eq!(t.call(g, vg::ioctl::RESOURCE_CREATE_BLOB, &mut bad), Err(Errno::EINVAL));
    }
    // Without blobs in the hardware: refused.
    let mut plain = T::one();
    let p = plain.dev.open(Node::Render);
    let mut b = blob(vg::blob::MEM_GUEST, 0, 4096, 0, 0, 0);
    assert_eq!(plain.call(p, vg::ioctl::RESOURCE_CREATE_BLOB, &mut b), Err(Errno::EINVAL));
}

#[test]
fn without_3d_resources_are_2d_and_transfers_go_one_way() {
    let mut t = T::one();
    let f = t.dev.open(Node::Render);
    let mut r = vg::ResourceCreate { format: 2, width: 32, height: 16, size: 2048, ..Default::default() }.to_bytes();
    t.ok(f, vg::ioctl::RESOURCE_CREATE, &mut r);
    let r = vg::ResourceCreate::decode(&r);
    assert_eq!(
        t.calls(),
        [Call::Create(r.res_handle, None, ResourceKind::Simple { format: 2, width: 32, height: 16 })]
    );
    let mut x = vg::Transfer { bo_handle: r.bo_handle, w: 32, h: 16, ..Default::default() }.to_bytes();
    t.ok(f, vg::ioctl::TRANSFER_TO_HOST, &mut x);
    assert!(matches!(&t.calls()[..], [Call::Transfer(true, None, _, _)]));
    assert_eq!(t.call(f, vg::ioctl::TRANSFER_FROM_HOST, &mut x), Err(Errno::ENOSYS));
    assert_eq!(t.execbuffer(f, 0, b"x", &[], -1, 0).0, Err(Errno::ENOSYS));
    // The 3D ioctls are render-node ioctls, open to the card node too.
    let card = t.dev.open(Node::Primary);
    assert_eq!(t.getparam(card, vg::param::THREE_D_FEATURES), Ok(0));
}
