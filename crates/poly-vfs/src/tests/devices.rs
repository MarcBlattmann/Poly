//! The character device hook.

use super::Env;
use crate::*;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

/// A console stand-in: collects writes, reads what it was given,
/// unseekable, and answers one ioctl.
#[derive(Default)]
struct Console {
    out: crate::sync::SpinMutex<Vec<u8>>,
    saw_nonblock: AtomicBool,
}

impl CharDevice for Console {
    fn read(&self, _: u64, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        self.saw_nonblock.store(nonblock, Ordering::Relaxed);
        if nonblock {
            return Err(Errno::EAGAIN);
        }
        let n = buf.len().min(3);
        buf[..n].copy_from_slice(&b"abc"[..n]);
        Ok(n)
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        self.out.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn ioctl(&self, request: u32, _: u64) -> Result<u64> {
        if request == 0x5401 { Ok(0) } else { Err(Errno::ENOTTY) }
    }
}

fn mknod(env: &Env, path: &str, id: DeviceId) {
    env.vfs.mknod(&env.ctx, &env.root, At::Cwd, path.as_bytes(), FileType::CharDevice, 0o666, id).unwrap();
}

#[test]
fn null_zero_full() {
    let env = Env::new();
    env.mkdir("/dev");
    env.vfs.register_char_device(device::NULL, Arc::new(device::Null)).unwrap();
    env.vfs.register_char_device(device::ZERO, Arc::new(device::Zero)).unwrap();
    env.vfs.register_char_device(device::FULL, Arc::new(device::Full)).unwrap();
    assert_eq!(env.vfs.register_char_device(device::NULL, Arc::new(device::Null)), Err(Errno::EBUSY));
    mknod(&env, "/dev/null", device::NULL);
    mknod(&env, "/dev/zero", device::ZERO);
    mknod(&env, "/dev/full", device::FULL);

    let null = env.open("/dev/null", OpenFlags::RDWR).unwrap();
    assert_eq!(null.write(b"gone"), Ok(4));
    assert_eq!(null.read(&mut [1u8; 8]), Ok(0));
    assert_eq!(null.seek(0, Whence::Set), Ok(0));
    assert_eq!(null.stat().unwrap().kind(), Some(FileType::CharDevice));

    let zero = env.open("/dev/zero", OpenFlags::RDONLY).unwrap();
    let mut buf = [7u8; 16];
    assert_eq!(zero.read(&mut buf), Ok(16));
    assert_eq!(buf, [0u8; 16]);
    assert_eq!(zero.read_at(1 << 40, &mut buf), Ok(16));

    let full = env.open("/dev/full", OpenFlags::WRONLY).unwrap();
    assert_eq!(full.write(b"x"), Err(Errno::ENOSPC));
    assert_eq!(full.ioctl(0x5401, 0), Err(Errno::ENOTTY));

    // Unregistered: ENXIO, also after unregistering. Open files keep theirs.
    mknod(&env, "/dev/random", device::RANDOM);
    assert_eq!(env.open("/dev/random", OpenFlags::RDONLY).map(|_| ()), Err(Errno::ENXIO));
    assert!(env.vfs.unregister_char_device(device::NULL).is_some());
    assert_eq!(env.open("/dev/null", OpenFlags::RDONLY).map(|_| ()), Err(Errno::ENXIO));
    assert_eq!(null.write(b"still"), Ok(5));
    // Block devices are not reached through the VFS yet.
    env.vfs
        .mknod(&env.ctx, &env.root, At::Cwd, b"/dev/sda", FileType::BlockDevice, 0o660, DeviceId::new(8, 0))
        .unwrap();
    assert_eq!(env.open("/dev/sda", OpenFlags::RDONLY).map(|_| ()), Err(Errno::ENXIO));
}

#[test]
fn an_unseekable_terminal_like_device() {
    let env = Env::new();
    let console = Arc::new(Console::default());
    env.vfs.register_char_device(device::CONSOLE, console.clone()).unwrap();
    mknod(&env, "/console", device::CONSOLE);
    let f = env.open("/console", OpenFlags::RDWR).unwrap();
    assert_eq!(f.write(b"hello\n"), Ok(6));
    assert_eq!(&console.out.lock()[..], b"hello\n");
    let mut buf = [0u8; 8];
    assert_eq!(f.read(&mut buf), Ok(3));
    assert_eq!(f.seek(0, Whence::Cur), Err(Errno::ESPIPE));
    assert_eq!(f.read_at(0, &mut buf), Err(Errno::ESPIPE));
    assert_eq!(f.ioctl(0x5401, 0), Ok(0));
    assert_eq!(f.ioctl(1, 0), Err(Errno::ENOTTY));
    assert!(f.device().is_some());
    // O_NONBLOCK reaches the device.
    f.set_flags(f.flags() | OpenFlags::NONBLOCK);
    assert_eq!(f.read(&mut buf), Err(Errno::EAGAIN));
    assert!(console.saw_nonblock.load(Ordering::Relaxed));
    // Regular files have no ioctls.
    env.write("/f", b"");
    assert_eq!(env.open("/f", OpenFlags::RDONLY).unwrap().ioctl(0x5401, 0), Err(Errno::ENOTTY));
}

/// A device that opens as a new device per open (as `/dev/ptmx` does),
/// counting the opens still alive; with `O_EXCL` it refuses.
#[derive(Default)]
struct Cloning {
    live: Arc<core::sync::atomic::AtomicUsize>,
}

struct Handle(Arc<core::sync::atomic::AtomicUsize>, u8);

impl Drop for Handle {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl CharDevice for Handle {
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> Result<usize> {
        buf[0] = self.1;
        Ok(1)
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        Ok(buf.len())
    }
}

impl CharDevice for Cloning {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::EIO)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::EIO)
    }

    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        if flags.contains(OpenFlags::EXCL) {
            return Err(Errno::EBUSY);
        }
        let n = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Some(Arc::new(Handle(self.live.clone(), n as u8))))
    }
}

#[test]
fn a_device_may_open_as_another() {
    let env = Env::new();
    let dev = Arc::new(Cloning::default());
    let live = dev.live.clone();
    env.vfs.register_char_device(DeviceId::new(5, 2), dev).unwrap();
    mknod(&env, "/ptmx", DeviceId::new(5, 2));
    let a = env.open("/ptmx", OpenFlags::RDWR).unwrap();
    let b = env.open("/ptmx", OpenFlags::RDWR).unwrap();
    let mut buf = [0u8; 1];
    assert_eq!((a.read(&mut buf), buf[0]), (Ok(1), 1));
    assert_eq!((b.read(&mut buf), buf[0]), (Ok(1), 2));
    assert_eq!(live.load(Ordering::SeqCst), 2);
    // The node still says what was opened.
    assert_eq!(a.stat().unwrap().rdev, DeviceId::new(5, 2));
    drop(a);
    assert_eq!(live.load(Ordering::SeqCst), 1, "the handle goes with the open file");
    assert_eq!(env.open("/ptmx", OpenFlags::RDWR | OpenFlags::EXCL).map(|_| ()), Err(Errno::EBUSY));
    drop(b);
    assert_eq!(live.load(Ordering::SeqCst), 0);
}
