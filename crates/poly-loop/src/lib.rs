//! Loop devices (loop(4)), ADR 0168: a regular file (or another block
//! device) made available as a block device of major 7, and the control
//! device `/dev/loop-control` (10, 237) that adds, removes and finds them.
//!
//! - [`Loops`] is the table of devices. [`Loops::control`] is the control
//!   device; a device is registered with the VFS as block device (7, n)
//!   when it is added, so a node made with `mknod` opens it.
//! - **Binding** (`LOOP_SET_FD`, `LOOP_CONFIGURE`, `LOOP_CHANGE_FD`),
//!   **status** (`LOOP_GET_STATUS64`, `LOOP_SET_STATUS64`, capacity, block
//!   size) and the block ioctls (`BLKGETSIZE64`, `BLKROSET`, `BLKFLSBUF`,
//!   ...) are as the loop_* spec sheets give them. The device has a page
//!   cache of its own: data read through it stays until `BLKFLSBUF`, data
//!   written reaches the file at `fsync`, `BLKFLSBUF`, unbinding or
//!   `O_DIRECT`.
//! - **Use.** A device is in use while a descriptor is open on it or a file
//!   system is mounted from it ([`Loop::claim_mount`]); `LOOP_CLR_FD` then
//!   only sets `LO_FLAGS_AUTOCLEAR` and the last close or unmount unbinds.
//!
//! The kernel supplies the process's descriptors and memory through [`Env`]
//! and the VFS registration through [`Registry`].
#![no_std]
extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;
use poly_vfs::lock::{Lock, RawLock, RawSpinLock};
use poly_vfs::{CharDevice, DeviceId, Errno, FileType, OpenFile, OpenFlags, Result};

/// `LOOP_MAJOR` (`linux/major.h`).
pub const LOOP_MAJOR: u32 = 7;
/// The control device's number (`linux/miscdevice.h`, `LOOP_CTRL_MINOR`).
pub const CONTROL: DeviceId = DeviceId::new(10, 237);
/// The highest device number (`MINORMASK`).
pub const MAX_NUMBER: u32 = 0xf_ffff;

pub const LOOP_SET_FD: u32 = 0x4C00;
pub const LOOP_CLR_FD: u32 = 0x4C01;
pub const LOOP_SET_STATUS: u32 = 0x4C02;
pub const LOOP_GET_STATUS: u32 = 0x4C03;
pub const LOOP_SET_STATUS64: u32 = 0x4C04;
pub const LOOP_GET_STATUS64: u32 = 0x4C05;
pub const LOOP_CHANGE_FD: u32 = 0x4C06;
pub const LOOP_SET_CAPACITY: u32 = 0x4C07;
pub const LOOP_SET_DIRECT_IO: u32 = 0x4C08;
pub const LOOP_SET_BLOCK_SIZE: u32 = 0x4C09;
pub const LOOP_CONFIGURE: u32 = 0x4C0A;
pub const LOOP_CTL_ADD: u32 = 0x4C80;
pub const LOOP_CTL_REMOVE: u32 = 0x4C81;
pub const LOOP_CTL_GET_FREE: u32 = 0x4C82;
pub const BLKROSET: u32 = 0x125D;
pub const BLKROGET: u32 = 0x125E;
pub const BLKGETSIZE: u32 = 0x1260;
pub const BLKFLSBUF: u32 = 0x1261;
pub const BLKSSZGET: u32 = 0x1268;
pub const BLKGETSIZE64: u32 = 0x8008_1272;

pub const LO_FLAGS_READ_ONLY: u32 = 1;
pub const LO_FLAGS_AUTOCLEAR: u32 = 4;
pub const LO_FLAGS_PARTSCAN: u32 = 8;
pub const LO_FLAGS_DIRECT_IO: u32 = 16;

const SECTOR: u64 = 512;
const PAGE: u64 = 4096;
const INFO_SIZE: usize = 232;
const CONFIG_SIZE: usize = 304;
/// Pages the device caches before the oldest clean ones are dropped.
const CACHE_PAGES: usize = 4096;

/// What the kernel gives the loop code: the calling process's memory and
/// descriptors.
pub trait Env: Send + Sync {
    fn read_user(&self, addr: u64, buf: &mut [u8]) -> Result<()>;
    fn write_user(&self, addr: u64, data: &[u8]) -> Result<()>;
    /// The open file behind descriptor `fd` of the calling process
    /// (`EBADF` if there is none).
    fn file(&self, fd: i32) -> Result<Arc<OpenFile>>;
}

/// Where block devices are announced to the VFS.
pub trait Registry: Send + Sync {
    fn register(&self, id: DeviceId, dev: Arc<dyn CharDevice>) -> Result<()>;
    fn unregister(&self, id: DeviceId);
}

fn encode_dev(d: DeviceId) -> u64 {
    // `new_encode_dev` / glibc's makedev for `st_rdev`.
    let (ma, mi) = (u64::from(d.major), u64::from(d.minor));
    (mi & 0xff) | (ma << 8) | ((mi & !0xff) << 12)
}

#[derive(Default)]
struct Page {
    data: Vec<u8>,
    dirty: bool,
}

struct State {
    backing: Option<Arc<OpenFile>>,
    file_read_only: bool,
    read_only: bool,
    offset: u64,
    limit: u64,
    name: [u8; 64],
    flags: u32,
    block_size: u32,
    /// The device's size in bytes, fixed when the status or capacity is set.
    size: u64,
    pages: BTreeMap<u64, Page>,
    /// The device was mapped ([`Loop::mapped`]): a mapping shows the backing
    /// file's own pages, so the device keeps none of its own from then on.
    mapped: bool,
    opens: u32,
    mounts: u32,
    exclusive: u32,
}

impl Default for State {
    fn default() -> State {
        State {
            backing: None,
            file_read_only: false,
            read_only: false,
            offset: 0,
            limit: 0,
            name: [0; 64],
            flags: 0,
            block_size: 512,
            size: 0,
            pages: BTreeMap::new(),
            mapped: false,
            opens: 0,
            mounts: 0,
            exclusive: 0,
        }
    }
}

impl State {
    fn bound(&self) -> bool {
        self.backing.is_some()
    }
}

/// The part of its backing file a loop device shows ([`Loop::mapped`]).
#[derive(Debug)]
pub struct Mapped {
    pub file: Arc<OpenFile>,
    /// Where the device starts in the file.
    pub offset: u64,
    /// The device's size in bytes.
    pub size: u64,
    /// The device refuses writes.
    pub read_only: bool,
}

/// One loop device.
pub struct Loop<L: RawLock + Default + 'static = RawSpinLock> {
    number: u32,
    env: Arc<dyn Env>,
    state: Lock<L, State>,
}

impl<L: RawLock + Default + 'static> core::fmt::Debug for Loop<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Loop").field("number", &self.number).finish_non_exhaustive()
    }
}

/// The size of a backing file in bytes.
fn backing_size(file: &OpenFile) -> Result<u64> {
    match file.device() {
        Some(d) => Ok(d.size().unwrap_or(0)),
        None => Ok(file.stat()?.size),
    }
}

/// The device size for a backing size, offset and limit.
fn device_size(file_size: u64, offset: u64, limit: u64) -> u64 {
    let mut avail = file_size.saturating_sub(offset);
    if limit != 0 {
        avail = avail.min(limit);
    }
    avail / SECTOR * SECTOR
}

/// The fields of `struct loop_info64` the code uses.
struct Info {
    offset: u64,
    limit: u64,
    encrypt_type: u32,
    key_size: u32,
    flags: u32,
    name: [u8; 64],
}

impl Info {
    fn parse(b: &[u8]) -> Info {
        let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]));
        let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]));
        let mut name = [0u8; 64];
        name.copy_from_slice(&b[56..120]);
        Info {
            offset: u64_at(24),
            limit: u64_at(32),
            encrypt_type: u32_at(44),
            key_size: u32_at(48),
            flags: u32_at(52),
            name,
        }
    }

    fn check(&self) -> Result<()> {
        if self.encrypt_type != 0 || self.key_size > 32 {
            return Err(Errno::EINVAL);
        }
        Ok(())
    }
}

impl<L: RawLock + Default + 'static> Loop<L> {
    fn new(number: u32, env: Arc<dyn Env>) -> Arc<Loop<L>> {
        Arc::new(Loop { number, env, state: Lock::new(State { block_size: 512, ..State::default() }) })
    }

    pub fn number(&self) -> u32 {
        self.number
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(LOOP_MAJOR, self.number)
    }

    /// Whether the device is bound to a file.
    pub fn is_bound(&self) -> bool {
        self.state.lock().bound()
    }

    /// The device's size in bytes (0 when unbound).
    pub fn size_bytes(&self) -> u64 {
        self.state.lock().size
    }

    /// Whether the device refuses writes.
    pub fn is_read_only(&self) -> bool {
        let s = self.state.lock();
        s.read_only || s.file_read_only
    }

    // ----- backing I/O (state locked) -----

    fn load_page(s: &mut State, index: u64) -> Result<()> {
        if s.pages.contains_key(&index) {
            return Ok(());
        }
        let backing = s.backing.clone().ok_or(Errno::ENXIO)?;
        let mut data = vec![0u8; PAGE as usize];
        let start = index * PAGE;
        let valid = s.size.saturating_sub(start).min(PAGE) as usize;
        let mut done = 0;
        while done < valid {
            let n = backing.read_at(s.offset + start + done as u64, &mut data[done..valid])?;
            if n == 0 {
                break;
            }
            done += n;
        }
        if s.pages.len() >= CACHE_PAGES {
            let clean: Vec<u64> =
                s.pages.iter().filter(|(_, p)| !p.dirty).map(|(k, _)| *k).take(CACHE_PAGES / 2).collect();
            for k in clean {
                s.pages.remove(&k);
            }
        }
        s.pages.insert(index, Page { data, dirty: false });
        Ok(())
    }

    fn write_back(s: &mut State) -> Result<()> {
        let Some(backing) = s.backing.clone() else { return Ok(()) };
        let keys: Vec<u64> = s.pages.iter().filter(|(_, p)| p.dirty).map(|(k, _)| *k).collect();
        for k in keys {
            let start = k * PAGE;
            let valid = s.size.saturating_sub(start).min(PAGE) as usize;
            let offset = s.offset + start;
            if let Some(p) = s.pages.get_mut(&k) {
                let mut done = 0;
                while done < valid {
                    let n = backing.write_at(offset + done as u64, &p.data[done..valid])?;
                    if n == 0 {
                        return Err(Errno::EIO);
                    }
                    done += n;
                }
                p.dirty = false;
            }
        }
        Ok(())
    }

    fn drop_cache(s: &mut State) {
        s.pages.clear();
    }

    fn read_bytes(s: &mut State, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if !s.bound() || offset >= s.size {
            return Ok(0);
        }
        if s.mapped {
            // What a mapping wrote is in the file, not in pages kept here.
            Self::write_back(s)?;
            Self::drop_cache(s);
        }
        let n = (buf.len() as u64).min(s.size - offset) as usize;
        let mut done = 0;
        while done < n {
            let pos = offset + done as u64;
            let (index, within) = (pos / PAGE, (pos % PAGE) as usize);
            Self::load_page(s, index)?;
            let page = s.pages.get(&index).ok_or(Errno::EIO)?;
            let take = (PAGE as usize - within).min(n - done);
            buf[done..done + take].copy_from_slice(&page.data[within..within + take]);
            done += take;
        }
        Ok(n)
    }

    fn write_bytes(s: &mut State, offset: u64, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if !s.bound() || offset >= s.size {
            return Err(Errno::ENOSPC);
        }
        let n = (buf.len() as u64).min(s.size - offset) as usize;
        let mut done = 0;
        while done < n {
            let pos = offset + done as u64;
            let (index, within) = (pos / PAGE, (pos % PAGE) as usize);
            Self::load_page(s, index)?;
            let page = s.pages.get_mut(&index).ok_or(Errno::EIO)?;
            let take = (PAGE as usize - within).min(n - done);
            page.data[within..within + take].copy_from_slice(&buf[done..done + take]);
            page.dirty = true;
            done += take;
        }
        if s.mapped {
            // A mapping must see the write: it goes to the file at once.
            Self::write_back(s)?;
            Self::drop_cache(s);
        }
        Ok(n)
    }

    /// What a mapping of the device (mmap(2)) shows: the backing file from
    /// `offset` on, `size` bytes of it. What the device held back is
    /// written first, and from now on the device keeps no pages of its own,
    /// so reads and writes through it and through mappings agree. `None`
    /// when the device is not bound.
    pub fn mapped(&self) -> Result<Option<Mapped>> {
        let mut s = self.state.lock();
        let Some(file) = s.backing.clone() else { return Ok(None) };
        Self::write_back(&mut s)?;
        Self::drop_cache(&mut s);
        s.mapped = true;
        Ok(Some(Mapped { file, offset: s.offset, size: s.size, read_only: s.read_only || s.file_read_only }))
    }

    /// Reads for a file system mounted from the device: straight from the
    /// backing file, not through the device's own page cache.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let mut s = self.state.lock();
        let Some(backing) = s.backing.clone() else { return Ok(0) };
        if offset >= s.size {
            return Ok(0);
        }
        Self::write_back(&mut s)?;
        let n = (buf.len() as u64).min(s.size - offset) as usize;
        backing.read_at(s.offset + offset, &mut buf[..n])
    }

    // ----- binding -----

    fn recompute(s: &mut State) -> Result<()> {
        let backing = s.backing.clone().ok_or(Errno::ENXIO)?;
        s.size = device_size(backing_size(&backing)?, s.offset, s.limit);
        Ok(())
    }

    fn backing_from(&self, fd: i32) -> Result<Arc<OpenFile>> {
        let file = self.env.file(fd)?;
        if file.is_path_only() {
            return Err(Errno::EBADF);
        }
        Ok(file)
    }

    /// A device cannot be the backing file of itself (`EBADF`), and only
    /// regular files and block devices can be bound (`EINVAL`).
    fn check_kind(&self, file: &OpenFile) -> Result<()> {
        if let Some(d) = file.device()
            && let Some(h) = d.as_any().and_then(|a| a.downcast_ref::<Handle<L>>())
            && core::ptr::eq(Arc::as_ptr(&h.dev), self)
        {
            return Err(Errno::EBADF);
        }
        if matches!(file.kind(), FileType::Regular | FileType::BlockDevice) { Ok(()) } else { Err(Errno::EINVAL) }
    }

    fn bind(s: &mut State, file: Arc<OpenFile>, info: Option<&Info>, block_size: u32) -> Result<()> {
        let mut flags = 0;
        if let Some(i) = info {
            s.offset = i.offset;
            s.limit = i.limit;
            s.name = i.name;
            s.name[63] = 0;
            flags = i.flags & (LO_FLAGS_AUTOCLEAR | LO_FLAGS_PARTSCAN);
            s.read_only = i.flags & LO_FLAGS_READ_ONLY != 0;
        } else {
            s.offset = 0;
            s.limit = 0;
            s.name = [0; 64];
            s.read_only = false;
        }
        s.file_read_only = !file.writable();
        s.size = device_size(backing_size(&file)?, s.offset, s.limit);
        s.backing = Some(file);
        s.flags = flags;
        s.block_size = if block_size == 0 { 512 } else { block_size };
        Self::drop_cache(s);
        Ok(())
    }

    fn unbind(s: &mut State) {
        let _ = Self::write_back(s);
        Self::drop_cache(s);
        s.backing = None;
        s.file_read_only = false;
        s.read_only = false;
        s.offset = 0;
        s.limit = 0;
        s.name = [0; 64];
        s.flags = 0;
        s.size = 0;
        s.block_size = 512;
    }

    /// Unbinds the device if it is autoclear and nothing uses it any more.
    fn release_check(s: &mut State) {
        if s.bound() && s.flags & LO_FLAGS_AUTOCLEAR != 0 && s.opens == 0 && s.mounts == 0 {
            Self::unbind(s);
        }
    }

    /// A file system is mounted from the device until the claim is dropped;
    /// `exclusive` for a mount that refuses other exclusive openers.
    pub fn claim_mount(self: &Arc<Self>) -> Result<MountClaim<L>> {
        let mut s = self.state.lock();
        if !s.bound() {
            return Err(Errno::ENXIO);
        }
        if s.exclusive > 0 {
            return Err(Errno::EBUSY);
        }
        s.mounts += 1;
        Ok(MountClaim { dev: self.clone() })
    }

    // ----- ioctls -----

    fn user_int_out(&self, addr: u64, v: i32) -> Result<u64> {
        self.env.write_user(addr, &v.to_le_bytes())?;
        Ok(0)
    }

    fn ioctl(&self, h: &Handle<L>, request: u32, arg: u64) -> Result<u64> {
        let env = &self.env;
        match request {
            LOOP_SET_FD => {
                let file = self.backing_from(arg as i32)?;
                let mut s = self.state.lock();
                if s.bound() || h.cleared() {
                    return Err(Errno::EBUSY);
                }
                self.check_kind(&file)?;
                Self::bind(&mut s, file, None, 0)?;
                Ok(0)
            }
            LOOP_CONFIGURE => {
                if arg == 0 {
                    return Err(Errno::EFAULT);
                }
                let mut b = vec![0u8; CONFIG_SIZE];
                env.read_user(arg, &mut b)?;
                let fd = u32::from_le_bytes(b[0..4].try_into().unwrap_or([0; 4]));
                let block_size = u32::from_le_bytes(b[4..8].try_into().unwrap_or([0; 4]));
                let file = self.backing_from(fd as i32)?;
                if block_size != 0 && !valid_block_size(block_size) {
                    return Err(Errno::EINVAL);
                }
                let mut s = self.state.lock();
                if s.bound() || h.cleared() {
                    return Err(Errno::EBUSY);
                }
                self.check_kind(&file)?;
                let info = Info::parse(&b[8..8 + INFO_SIZE]);
                info.check()?;
                Self::bind(&mut s, file, Some(&info), block_size)?;
                Ok(0)
            }
            LOOP_CLR_FD => {
                let mut s = self.state.lock();
                if !s.bound() {
                    return Err(Errno::ENXIO);
                }
                if s.opens <= 1 && s.mounts == 0 {
                    Self::unbind(&mut s);
                    h.set_cleared();
                } else {
                    s.flags |= LO_FLAGS_AUTOCLEAR;
                }
                Ok(0)
            }
            LOOP_GET_STATUS64 | LOOP_GET_STATUS => {
                let s = self.state.lock();
                let backing = s.backing.clone().ok_or(Errno::ENXIO)?;
                if arg == 0 {
                    return Err(Errno::EINVAL);
                }
                let st = backing.stat()?;
                let mut out = vec![0u8; INFO_SIZE];
                if request == LOOP_GET_STATUS {
                    // The old `struct loop_info` (x86-64 layout, 160 bytes).
                    out.truncate(160);
                    out[0..4].copy_from_slice(&self.number.to_le_bytes());
                    out[20..24].copy_from_slice(&(s.offset as u32).to_le_bytes());
                    let flags = s.flags | if s.read_only || s.file_read_only { LO_FLAGS_READ_ONLY } else { 0 };
                    out[32..36].copy_from_slice(&flags.to_le_bytes());
                    out[36..100].copy_from_slice(&s.name);
                } else {
                    out[0..8].copy_from_slice(&encode_dev(st.dev).to_le_bytes());
                    out[8..16].copy_from_slice(&st.ino.to_le_bytes());
                    let rdev = if backing.kind() == FileType::BlockDevice { encode_dev(st.rdev) } else { 0 };
                    out[16..24].copy_from_slice(&rdev.to_le_bytes());
                    out[24..32].copy_from_slice(&s.offset.to_le_bytes());
                    out[32..40].copy_from_slice(&s.limit.to_le_bytes());
                    out[40..44].copy_from_slice(&self.number.to_le_bytes());
                    let flags = s.flags | if s.read_only || s.file_read_only { LO_FLAGS_READ_ONLY } else { 0 };
                    out[52..56].copy_from_slice(&flags.to_le_bytes());
                    out[56..120].copy_from_slice(&s.name);
                }
                drop(s);
                env.write_user(arg, &out)?;
                Ok(0)
            }
            LOOP_SET_STATUS64 => {
                if !self.is_bound() {
                    return Err(Errno::ENXIO);
                }
                if arg == 0 {
                    return Err(Errno::EFAULT);
                }
                let mut b = vec![0u8; INFO_SIZE];
                env.read_user(arg, &mut b)?;
                let info = Info::parse(&b);
                info.check()?;
                let mut s = self.state.lock();
                if !s.bound() {
                    return Err(Errno::ENXIO);
                }
                s.offset = info.offset;
                s.limit = info.limit;
                s.name = info.name;
                s.name[63] = 0;
                let part = s.flags & LO_FLAGS_PARTSCAN;
                s.flags =
                    (info.flags & (LO_FLAGS_AUTOCLEAR | LO_FLAGS_PARTSCAN)) | part | (s.flags & LO_FLAGS_DIRECT_IO);
                Self::write_back(&mut s)?;
                Self::recompute(&mut s)?;
                Ok(0)
            }
            LOOP_SET_STATUS => {
                if !self.is_bound() {
                    return Err(Errno::ENXIO);
                }
                Ok(0)
            }
            LOOP_SET_CAPACITY => {
                let mut s = self.state.lock();
                if !s.bound() {
                    return Err(Errno::ENXIO);
                }
                Self::recompute(&mut s)?;
                Ok(0)
            }
            LOOP_SET_DIRECT_IO => {
                let mut s = self.state.lock();
                if !s.bound() {
                    return Err(Errno::ENXIO);
                }
                if arg == 0 {
                    s.flags &= !LO_FLAGS_DIRECT_IO;
                } else {
                    s.flags |= LO_FLAGS_DIRECT_IO;
                }
                Ok(0)
            }
            LOOP_SET_BLOCK_SIZE => {
                let mut s = self.state.lock();
                if !s.bound() {
                    return Err(Errno::ENXIO);
                }
                let bs = arg as u32;
                if bs == 0 {
                    return Ok(0);
                }
                if !valid_block_size(bs) {
                    return Err(Errno::EINVAL);
                }
                s.block_size = bs;
                Ok(0)
            }
            LOOP_CHANGE_FD => {
                let file = self.backing_from(arg as i32)?;
                let mut s = self.state.lock();
                if !s.bound() {
                    return Err(Errno::ENXIO);
                }
                self.check_kind(&file)?;
                if !(s.read_only || s.file_read_only) {
                    return Err(Errno::EINVAL);
                }
                let old = s.backing.clone().ok_or(Errno::ENXIO)?;
                if backing_size(&old)? != backing_size(&file)? {
                    return Err(Errno::EINVAL);
                }
                s.backing = Some(file);
                Self::drop_cache(&mut s);
                Ok(0)
            }
            BLKROSET => {
                let mut b = [0u8; 4];
                env.read_user(arg, &mut b)?;
                self.state.lock().read_only = i32::from_le_bytes(b) != 0;
                Ok(0)
            }
            BLKROGET => {
                let ro = self.is_read_only();
                self.user_int_out(arg, i32::from(ro))
            }
            BLKGETSIZE => {
                let sectors = self.state.lock().size / SECTOR;
                env.write_user(arg, &sectors.to_le_bytes())?;
                Ok(0)
            }
            BLKGETSIZE64 => {
                let size = self.state.lock().size;
                env.write_user(arg, &size.to_le_bytes())?;
                Ok(0)
            }
            BLKSSZGET => {
                let bs = self.state.lock().block_size;
                self.user_int_out(arg, bs as i32)
            }
            BLKFLSBUF => {
                let mut s = self.state.lock();
                Self::write_back(&mut s)?;
                Self::drop_cache(&mut s);
                Ok(0)
            }
            _ => Err(Errno::ENOTTY),
        }
    }
}

/// Whether a direct transfer is not whole sectors at a sector boundary.
fn misaligned(offset: u64, len: usize) -> bool {
    len != 0 && (!offset.is_multiple_of(SECTOR) || !(len as u64).is_multiple_of(SECTOR))
}

fn valid_block_size(bs: u32) -> bool {
    bs.is_power_of_two() && (512..=8192).contains(&bs)
}

/// A file system mounted from the device holds this.
pub struct MountClaim<L: RawLock + Default + 'static = RawSpinLock> {
    dev: Arc<Loop<L>>,
}

impl<L: RawLock + Default + 'static> MountClaim<L> {
    pub fn device(&self) -> &Arc<Loop<L>> {
        &self.dev
    }
}

impl<L: RawLock + Default + 'static> core::fmt::Debug for MountClaim<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MountClaim").field("number", &self.dev.number).finish()
    }
}

impl<L: RawLock + Default + 'static> Drop for MountClaim<L> {
    fn drop(&mut self) {
        let mut s = self.dev.state.lock();
        s.mounts = s.mounts.saturating_sub(1);
        Loop::<L>::release_check(&mut s);
    }
}

/// One open of a loop device's block node.
pub struct Handle<L: RawLock + Default + 'static = RawSpinLock> {
    dev: Arc<Loop<L>>,
    direct: bool,
    exclusive: bool,
    cleared: Lock<RawSpinLock, bool>,
}

impl<L: RawLock + Default + 'static> core::fmt::Debug for Handle<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handle").field("number", &self.dev.number).finish()
    }
}

impl<L: RawLock + Default + 'static> Handle<L> {
    fn cleared(&self) -> bool {
        *self.cleared.lock()
    }

    fn set_cleared(&self) {
        *self.cleared.lock() = true;
    }
}

impl<L: RawLock + Default + 'static> Drop for Handle<L> {
    fn drop(&mut self) {
        let mut s = self.dev.state.lock();
        s.opens = s.opens.saturating_sub(1);
        if self.exclusive {
            s.exclusive = s.exclusive.saturating_sub(1);
        }
        Loop::<L>::release_check(&mut s);
    }
}

impl<L: RawLock + Default + 'static> CharDevice for Handle<L> {
    fn read(&self, offset: u64, buf: &mut [u8], _nonblock: bool) -> Result<usize> {
        if self.direct && misaligned(offset, buf.len()) {
            return Err(Errno::EINVAL);
        }
        let mut s = self.dev.state.lock();
        if self.direct {
            Loop::<L>::write_back(&mut s)?;
        }
        Loop::<L>::read_bytes(&mut s, offset, buf)
    }

    fn write(&self, offset: u64, buf: &[u8], _nonblock: bool) -> Result<usize> {
        if self.direct && misaligned(offset, buf.len()) {
            return Err(Errno::EINVAL);
        }
        let mut s = self.dev.state.lock();
        if s.bound() && (s.read_only || s.file_read_only) {
            return Err(Errno::EPERM);
        }
        let n = Loop::<L>::write_bytes(&mut s, offset, buf)?;
        if self.direct {
            Loop::<L>::write_back(&mut s)?;
        }
        Ok(n)
    }

    fn seekable(&self) -> bool {
        true
    }

    fn size(&self) -> Option<u64> {
        Some(self.dev.size_bytes())
    }

    fn sync(&self) -> Result<()> {
        Loop::<L>::write_back(&mut self.dev.state.lock())
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        self.dev.ioctl(self, request, arg)
    }

    fn as_any(&self) -> Option<&(dyn Any + Send + Sync)> {
        Some(self)
    }
}

/// The device number registered with the VFS: opening a block node of the
/// device makes a [`Handle`].
struct Node<L: RawLock + Default + 'static> {
    dev: Arc<Loop<L>>,
}

impl<L: RawLock + Default + 'static> CharDevice for Node<L> {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn seekable(&self) -> bool {
        true
    }

    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        let exclusive = flags.contains(OpenFlags::EXCL);
        let mut s = self.dev.state.lock();
        if exclusive && (s.mounts > 0 || s.exclusive > 0) {
            return Err(Errno::EBUSY);
        }
        s.opens += 1;
        if exclusive {
            s.exclusive += 1;
        }
        drop(s);
        Ok(Some(Arc::new(Handle {
            dev: self.dev.clone(),
            direct: flags.contains(OpenFlags::DIRECT),
            exclusive,
            cleared: Lock::new(false),
        })))
    }
}

/// The table of loop devices.
pub struct Loops<L: RawLock + Default + 'static = RawSpinLock> {
    env: Arc<dyn Env>,
    registry: Arc<dyn Registry>,
    devices: Lock<RawSpinLock, BTreeMap<u32, Arc<Loop<L>>>>,
}

impl<L: RawLock + Default + 'static> core::fmt::Debug for Loops<L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Loops").finish_non_exhaustive()
    }
}

impl<L: RawLock + Default + 'static> Loops<L> {
    pub fn new(env: Arc<dyn Env>, registry: Arc<dyn Registry>) -> Arc<Loops<L>> {
        Arc::new(Loops { env, registry, devices: Lock::new(BTreeMap::new()) })
    }

    /// `/dev/loop-control`.
    pub fn control(self: &Arc<Self>) -> Arc<dyn CharDevice> {
        Arc::new(Control { loops: self.clone() })
    }

    /// Device `number`, if it exists.
    pub fn device(&self, number: u32) -> Option<Arc<Loop<L>>> {
        self.devices.lock().get(&number).cloned()
    }

    /// The loop device a block device number names.
    pub fn device_of(&self, id: DeviceId) -> Option<Arc<Loop<L>>> {
        if id.major == LOOP_MAJOR { self.device(id.minor) } else { None }
    }

    fn add(&self, number: i32) -> Result<u32> {
        let mut devices = self.devices.lock();
        let n = if number < 0 {
            let mut n = 0u32;
            while devices.contains_key(&n) {
                n += 1;
            }
            n
        } else {
            let n = number as u32;
            if n > MAX_NUMBER {
                return Err(Errno::EINVAL);
            }
            if devices.contains_key(&n) {
                return Err(Errno::EEXIST);
            }
            n
        };
        if n > MAX_NUMBER {
            return Err(Errno::ENOSPC);
        }
        let dev = Loop::<L>::new(n, self.env.clone());
        self.registry.register(dev.id(), Arc::new(Node { dev: dev.clone() }))?;
        devices.insert(n, dev);
        Ok(n)
    }

    fn remove(&self, number: i32) -> Result<()> {
        if number < 0 {
            return Err(Errno::EINVAL);
        }
        let mut devices = self.devices.lock();
        let dev = devices.get(&(number as u32)).ok_or(Errno::ENODEV)?.clone();
        {
            let s = dev.state.lock();
            if s.bound() || s.opens > 0 || s.mounts > 0 {
                return Err(Errno::EBUSY);
            }
        }
        devices.remove(&(number as u32));
        self.registry.unregister(dev.id());
        Ok(())
    }

    fn get_free(&self) -> Result<u32> {
        {
            let devices = self.devices.lock();
            for (n, d) in devices.iter() {
                let s = d.state.lock();
                if !s.bound() && s.opens == 0 && s.mounts == 0 {
                    return Ok(*n);
                }
            }
        }
        self.add(-1)
    }
}

struct Control<L: RawLock + Default + 'static> {
    loops: Arc<Loops<L>>,
}

impl<L: RawLock + Default + 'static> CharDevice for Control<L> {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn ioctl(&self, request: u32, arg: u64) -> Result<u64> {
        match request {
            LOOP_CTL_ADD => self.loops.add(arg as i32).map(u64::from),
            LOOP_CTL_REMOVE => self.loops.remove(arg as i32).map(|()| 0),
            LOOP_CTL_GET_FREE => self.loops.get_free().map(u64::from),
            _ => Err(Errno::ENOSYS),
        }
    }
}

#[cfg(test)]
mod tests;
