extern crate std;

use super::*;
use poly_vfs::lock::RawSpinLock;
use poly_vfs::{At, Clock, Cred, FsContext, Timespec, Tmpfs, Vfs, Whence};
use std::sync::Mutex;
use std::vec;
use std::vec::Vec;

struct Tick;
impl Clock for Tick {
    fn now(&self) -> Timespec {
        Timespec::new(1, 0)
    }
}

struct World {
    vfs: Arc<Vfs>,
    ctx: FsContext,
    root: Cred,
    files: Mutex<Vec<Arc<OpenFile>>>,
    mem: Mutex<Vec<u8>>,
}

struct TestEnv(Arc<World>);

impl Env for TestEnv {
    fn read_user(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        let m = self.0.mem.lock().unwrap();
        buf.copy_from_slice(m.get(addr as usize..addr as usize + buf.len()).ok_or(Errno::EFAULT)?);
        Ok(())
    }
    fn write_user(&self, addr: u64, data: &[u8]) -> Result<()> {
        let mut m = self.0.mem.lock().unwrap();
        m.get_mut(addr as usize..addr as usize + data.len()).ok_or(Errno::EFAULT)?.copy_from_slice(data);
        Ok(())
    }
    fn file(&self, fd: i32) -> Result<Arc<OpenFile>> {
        self.0.files.lock().unwrap().get(fd as usize).cloned().ok_or(Errno::EBADF)
    }
}

struct Reg(Arc<Vfs>);
impl Registry for Reg {
    fn register(&self, id: DeviceId, dev: Arc<dyn CharDevice>) -> Result<()> {
        self.0.register_block_device(id, dev)
    }
    fn unregister(&self, id: DeviceId) {
        self.0.unregister_block_device(id);
    }
}

fn world() -> (Arc<World>, Arc<Loops<RawSpinLock>>) {
    let fs = Arc::new(Tmpfs::new(Arc::new(Tick)));
    let vfs = Arc::new(Vfs::new(fs));
    let ctx = vfs.root_context().unwrap();
    let w = Arc::new(World {
        vfs: vfs.clone(),
        ctx,
        root: Cred::root(),
        files: Mutex::new(Vec::new()),
        mem: Mutex::new(vec![0; 4096]),
    });
    let loops = Loops::new(Arc::new(TestEnv(w.clone())), Arc::new(Reg(vfs)));
    (w, loops)
}

fn open(w: &World, path: &str, flags: OpenFlags) -> Result<Arc<OpenFile>> {
    w.vfs.open(&w.ctx, &w.root, At::Cwd, path.as_bytes(), flags, 0o644)
}

fn fd(w: &World, f: Arc<OpenFile>) -> u64 {
    let mut v = w.files.lock().unwrap();
    v.push(f);
    v.len() as u64 - 1
}

#[test]
fn bind_read_write_unbind() {
    let (w, loops) = world();
    let ctl = loops.control();
    assert_eq!(ctl.ioctl(LOOP_CTL_ADD, 5), Ok(5));
    assert_eq!(ctl.ioctl(LOOP_CTL_ADD, 5), Err(Errno::EEXIST));
    assert_eq!(ctl.ioctl(LOOP_CTL_ADD, 0x100000), Err(Errno::EINVAL));
    assert_eq!(ctl.ioctl(0x4C00, 0), Err(Errno::ENOSYS));
    let img = open(&w, "/img", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    let data: Vec<u8> = (0..1700u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(img.write(&data), Ok(1700));
    w.vfs.mknod(&w.ctx, &w.root, At::Cwd, b"/l5", FileType::BlockDevice, 0o600, DeviceId::new(7, 5)).unwrap();
    let dev = open(&w, "/l5", OpenFlags::RDWR).unwrap();
    // Unbound: empty, writes fail.
    let mut buf = [0u8; 8];
    assert_eq!(dev.read(&mut buf), Ok(0));
    assert_eq!(dev.write(b"x"), Err(Errno::ENOSPC));
    assert_eq!(dev.ioctl(LOOP_GET_STATUS64, 0), Err(Errno::ENXIO));
    let imgfd = fd(&w, img.clone());
    assert_eq!(dev.ioctl(LOOP_SET_FD, imgfd), Ok(0));
    assert_eq!(dev.ioctl(LOOP_SET_FD, imgfd), Err(Errno::EBUSY));
    assert_eq!(dev.ioctl(BLKGETSIZE64, 0), Ok(0));
    assert_eq!(u64::from_le_bytes(w.mem.lock().unwrap()[0..8].try_into().unwrap()), 1536);
    assert_eq!(dev.seek(0, Whence::End), Ok(1536));
    assert_eq!(dev.seek(1500, Whence::Set), Ok(1500));
    let mut big = [0u8; 100];
    assert_eq!(dev.read(&mut big), Ok(36));
    assert_eq!(&big[..36], &data[1500..1536]);
    assert_eq!(dev.read(&mut big), Ok(0));
    dev.seek(1500, Whence::Set).unwrap();
    assert_eq!(dev.write(&[7u8; 100]), Ok(36));
    assert_eq!(dev.write(b"x"), Err(Errno::ENOSPC));
    // Not in the file until it is synced.
    let mut check = [0u8; 4];
    assert_eq!(img.read_at(1500, &mut check), Ok(4));
    assert_eq!(&check, &data[1500..1504]);
    assert_eq!(dev.sync(), Ok(()));
    assert_eq!(img.read_at(1500, &mut check), Ok(4));
    assert_eq!(check, [7; 4]);
    // Removing a bound device is busy; unbinding it is not.
    assert_eq!(ctl.ioctl(LOOP_CTL_REMOVE, 5), Err(Errno::EBUSY));
    assert_eq!(dev.ioctl(LOOP_CLR_FD, 0), Ok(0));
    assert_eq!(dev.ioctl(LOOP_CLR_FD, 0), Err(Errno::ENXIO));
    assert_eq!(dev.ioctl(LOOP_SET_FD, imgfd), Err(Errno::EBUSY));
    drop(dev);
    assert_eq!(ctl.ioctl(LOOP_CTL_REMOVE, 5), Ok(0));
    assert_eq!(ctl.ioctl(LOOP_CTL_REMOVE, 5), Err(Errno::ENODEV));
    assert_eq!(ctl.ioctl(LOOP_CTL_REMOVE, u64::MAX), Err(Errno::EINVAL));
}

#[test]
fn offset_and_limit() {
    assert_eq!(device_size(1700, 0, 0), 1536);
    assert_eq!(device_size(511, 0, 0), 0);
    assert_eq!(device_size(65536, 4096, 6144), 6144);
    assert_eq!(device_size(5000, 4096, 6144), 512);
    assert_eq!(device_size(100, 200, 0), 0);
}

#[test]
fn autoclear_on_last_close() {
    let (w, loops) = world();
    let ctl = loops.control();
    ctl.ioctl(LOOP_CTL_ADD, 9).unwrap();
    let img = open(&w, "/img", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    img.write(&[1u8; 2048]).unwrap();
    w.vfs.mknod(&w.ctx, &w.root, At::Cwd, b"/l9", FileType::BlockDevice, 0o600, DeviceId::new(7, 9)).unwrap();
    let a = open(&w, "/l9", OpenFlags::RDWR).unwrap();
    let b = open(&w, "/l9", OpenFlags::RDWR).unwrap();
    let imgfd = fd(&w, img);
    assert_eq!(a.ioctl(LOOP_SET_FD, imgfd), Ok(0));
    // Another descriptor is open: only autoclear is set.
    assert_eq!(a.ioctl(LOOP_CLR_FD, 0), Ok(0));
    assert!(loops.device(9).unwrap().is_bound());
    drop(a);
    assert!(loops.device(9).unwrap().is_bound());
    drop(b);
    assert!(!loops.device(9).unwrap().is_bound());
    assert_eq!(ctl.ioctl(LOOP_CTL_REMOVE, 9), Ok(0));
}

#[test]
fn a_mapped_device_shows_the_backing_file_and_keeps_no_pages() {
    let (w, loops) = world();
    let ctl = loops.control();
    assert_eq!(ctl.ioctl(LOOP_CTL_ADD, 6), Ok(6));
    let l6 = loops.device(6).unwrap();
    // Not bound: nothing to map.
    assert!(l6.mapped().unwrap().is_none());
    let img = open(&w, "/img6", OpenFlags::RDWR | OpenFlags::CREAT).unwrap();
    assert_eq!(img.write(&[1u8; 2048]), Ok(2048));
    w.vfs.mknod(&w.ctx, &w.root, At::Cwd, b"/l6", FileType::BlockDevice, 0o600, DeviceId::new(7, 6)).unwrap();
    let dev = open(&w, "/l6", OpenFlags::RDWR).unwrap();
    assert_eq!(dev.ioctl(LOOP_SET_FD, fd(&w, img.clone())), Ok(0));
    // A write the device still holds back is in the file once it is mapped.
    assert_eq!(dev.write_at(0, &[2u8; 4]), Ok(4));
    let mut check = [0u8; 4];
    assert_eq!(img.read_at(0, &mut check), Ok(4));
    assert_eq!(check, [1; 4]);
    let m = l6.mapped().unwrap().unwrap();
    assert_eq!((m.offset, m.size, m.read_only), (0, 2048, false));
    assert_eq!(m.file.read_at(0, &mut check), Ok(4));
    assert_eq!(check, [2; 4]);
    // From now on the device and the file agree at once, both ways: what a
    // mapping changes in the file is what the device reads, and a write
    // through the device is in the file without a sync.
    assert_eq!(img.write_at(8, &[9u8; 4]), Ok(4));
    assert_eq!(dev.read_at(8, &mut check), Ok(4));
    assert_eq!(check, [9; 4]);
    assert_eq!(dev.write_at(16, &[5u8; 4]), Ok(4));
    assert_eq!(img.read_at(16, &mut check), Ok(4));
    assert_eq!(check, [5; 4]);
}
