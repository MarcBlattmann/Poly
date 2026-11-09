//! The new mount API's handlers (ADR 0122): the order of their checks
//! and what reaches the kernel, against a recording [`LinuxAdmin`].

extern crate std;

use super::fake::*;
use crate::admin::{LinuxAdmin, MountRequest, Reboot, SystemName};
use crate::cred::Creds;
use crate::mountapi::uapi::*;
use crate::mountapi::{AttrChange, Command, FsContext, MountDesc, MountLink, PathAt, Value, unique_id};
use crate::{Errno, nr};
use std::boxed::Box;
use std::format;
use std::string::String;
use std::sync::{Arc, Mutex};
use std::vec::Vec;

/// What reached the kernel, as text.
type Log = Arc<Mutex<Vec<String>>>;

struct Recorder(Log);

impl Recorder {
    fn note(&self, s: String) {
        self.0.lock().unwrap().push(s);
    }
}

impl LinuxAdmin for Recorder {
    fn mount(&mut self, _: &MountRequest) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    fn umount(&mut self, _: &[u8], _: bool, _: bool) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    fn mknodat(&mut self, _: i32, _: &[u8], _: u32, _: u64, _: bool) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    fn chroot(&mut self, _: &[u8], _: bool) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    fn system_name(&self, _: SystemName) -> Vec<u8> {
        Vec::new()
    }
    fn set_system_name(&mut self, _: SystemName, _: &[u8]) -> Result<(), Errno> {
        Err(Errno::ENOSYS)
    }
    fn set_ctrl_alt_del(&mut self, _: bool) {}
    fn reboot(&mut self, _: Reboot) -> Errno {
        Errno::EIO
    }
    fn open_tree(&mut self, at: &PathAt, clone: Option<bool>, cloexec: bool) -> Result<i32, Errno> {
        self.note(format!("open_tree {at:?} {clone:?} {cloexec}"));
        Ok(5)
    }
    fn move_mount(&mut self, from: &PathAt, to: &PathAt) -> Result<(), Errno> {
        self.note(format!("move_mount {from:?} {to:?}"));
        Ok(())
    }
    fn mount_setattr(&mut self, at: &PathAt, recursive: bool, change: &AttrChange) -> Result<(), Errno> {
        self.note(format!("mount_setattr {at:?} {recursive} {change:?}"));
        Ok(())
    }
    fn fsopen(&mut self, ctx: FsContext, cloexec: bool) -> Result<i32, Errno> {
        self.note(format!("fsopen {:?} {cloexec}", ctx.purpose()));
        Ok(3)
    }
    fn fsconfig(&mut self, fd: i32, cmd: &Command<'_>) -> Result<(), Errno> {
        self.note(format!("fsconfig {fd} {cmd:?}"));
        Ok(())
    }
    fn fsmount(&mut self, fd: i32, attrs: u64, cloexec: bool) -> Result<i32, Errno> {
        self.note(format!("fsmount {fd} {attrs:#x} {cloexec}"));
        Ok(4)
    }
    fn mount_links(&mut self) -> Result<Vec<MountLink>, Errno> {
        Ok(std::vec![MountLink { id: 1, parent: None }, MountLink { id: 2, parent: Some(1) }])
    }
    fn describe_mount(&mut self, id: u64) -> Result<MountDesc, Errno> {
        if id != 2 {
            return Err(Errno::ENOENT);
        }
        Ok(MountDesc { mnt_id: 2, parent_id: 1, fs_type: b"tmpfs".to_vec(), ..MountDesc::default() })
    }
    fn pivot_root(&mut self, new_root: &[u8], put_old: &[u8]) -> Result<(), Errno> {
        self.note(format!("pivot_root {} {}", String::from_utf8_lossy(new_root), String::from_utf8_lossy(put_old)));
        Ok(())
    }
}

fn kernel(creds: Creds) -> (FakeKernel, Log) {
    let mut k = FakeKernel::new();
    *k.creds_mut() = creds;
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    k.admin = Some(Box::new(Recorder(Arc::clone(&log))));
    (k, log)
}

/// A NUL-terminated string in user memory at `at`.
fn string(k: &mut FakeKernel, at: u64, s: &[u8]) -> u64 {
    let mut b = s.to_vec();
    b.push(0);
    k.poke(at, &b);
    at
}

fn taken(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

#[test]
fn privilege_comes_first_where_needed() {
    let (mut k, log) = kernel(Creds::user(1000, 1000));
    let name = string(&mut k, DATA, b"nosuchfs");
    assert_eq!(sys(&mut k, nr::FSOPEN, &[name, 2]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::FSMOUNT, &[99, 0, 0]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::FSPICK, &[AT_FDCWD_U, name, 0x10]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::MOVE_MOUNT, &[AT_FDCWD_U, name, AT_FDCWD_U, name, 0x8000]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::PIVOT_ROOT, &[name, name]), Err(Errno::EPERM));
    // open_tree without a clone needs nothing.
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[AT_FDCWD_U, name, 0]), Ok(5));
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[AT_FDCWD_U, name, u64::from(OPEN_TREE_CLONE)]), Err(Errno::EPERM));
    // mount_setattr checks the size before the privilege.
    let attr = DATA + 0x200;
    k.poke(attr, &[0u8; 32]);
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, name, 0, attr, 8]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, name, 0, attr, 32]), Err(Errno::EPERM));
    assert_eq!(taken(&log).len(), 1);
}

#[test]
fn open_tree_and_move_mount_arguments() {
    let (mut k, log) = kernel(Creds::root());
    let p = string(&mut k, DATA, b"/tmp/d");
    let empty = string(&mut k, DATA + 0x100, b"");
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[AT_FDCWD_U, p, 0x2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[AT_FDCWD_U, p, u64::from(AT_RECURSIVE)]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[3, empty, 0]), Err(Errno::ENOENT));
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[AT_FDCWD_U, UNMAPPED, 0]), Err(Errno::EFAULT));
    let clone = u64::from(OPEN_TREE_CLONE | AT_RECURSIVE | OPEN_TREE_CLOEXEC | AT_SYMLINK_NOFOLLOW);
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[AT_FDCWD_U, p, clone]), Ok(5));
    assert_eq!(sys(&mut k, nr::OPEN_TREE, &[3, empty, u64::from(AT_EMPTY_PATH)]), Ok(5));
    assert_eq!(sys(&mut k, nr::MOVE_MOUNT, &[3, empty, AT_FDCWD_U, p, 0x8000]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MOVE_MOUNT, &[3, empty, AT_FDCWD_U, p, 0]), Err(Errno::ENOENT));
    let f = u64::from(MOVE_MOUNT_F_EMPTY_PATH);
    assert_eq!(sys(&mut k, nr::MOVE_MOUNT, &[3, empty, AT_FDCWD_U, p, f]), Ok(0));
    let log = taken(&log);
    assert_eq!(log.len(), 3);
    assert!(
        log[0].contains("path: [47, 116") && log[0].contains("follow: false") && log[0].ends_with("Some(true) true")
    );
    assert!(log[1].starts_with("open_tree PathAt { dirfd: 3, path: [], follow: true } None false"));
    assert!(log[2].starts_with("move_mount PathAt { dirfd: 3, path: [], follow: false }"));
}

#[test]
fn mount_setattr_structure_checks() {
    let (mut k, log) = kernel(Creds::root());
    let p = string(&mut k, DATA, b"/tmp/d");
    let attr = DATA + 0x200;
    let put = |k: &mut FakeKernel, words: [u64; 6]| {
        let b: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        k.poke(attr, &b);
    };
    put(&mut k, [MOUNT_ATTR_NOEXEC, 0, 0, 0, 0, 0]);
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, attr, 16]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, attr, 48]), Ok(0));
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, attr, 8192]), Err(Errno::E2BIG));
    put(&mut k, [MOUNT_ATTR_NOEXEC, 0, 0, 0, 0, 1]);
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, attr, 48]), Err(Errno::E2BIG));
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0x1, attr, 32]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, UNMAPPED, 32]), Err(Errno::EFAULT));
    put(&mut k, [1 << 30, 0, 0, 0, 0, 0]);
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, attr, 32]), Err(Errno::EINVAL));
    put(&mut k, [0, 0, 12345, 0, 0, 0]);
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, 0, attr, 32]), Err(Errno::EINVAL));
    put(&mut k, [MOUNT_ATTR_RDONLY, 0, MS_SHARED, 0, 0, 0]);
    let rec = u64::from(AT_RECURSIVE);
    assert_eq!(sys(&mut k, nr::MOUNT_SETATTR, &[AT_FDCWD_U, p, rec, attr, 32]), Ok(0));
    let log = taken(&log);
    assert_eq!(log.len(), 2);
    assert!(log[1].ends_with("true AttrChange { set: 1, clr: 0, propagation: 1048576 }"));
}

#[test]
fn fsconfig_shapes_then_descriptor() {
    let (mut k, log) = kernel(Creds::root());
    let key = string(&mut k, DATA, b"size");
    let val = string(&mut k, DATA + 0x100, b"1m");
    let c = |cmd: u32| u64::from(cmd);
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, 9, 0, 0, 0]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_SET_FLAG), 0, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_SET_FLAG), key, val, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_SET_STRING), key, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_SET_STRING), key, val, 1]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_CMD_CREATE), key, 0, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[u64::MAX, c(FSCONFIG_CMD_CREATE), 0, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_SET_STRING), key, val, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::FSCONFIG, &[3, c(FSCONFIG_CMD_RECONFIGURE), 0, 0, 0]), Ok(0));
    let log = taken(&log);
    let want = format!("fsconfig 3 {:?}", Command::Set { key: b"size", value: Value::String(b"1m") });
    assert_eq!(log, [want, String::from("fsconfig 3 Reconfigure")]);
}

#[test]
fn fsopen_fsmount_arguments() {
    let (mut k, log) = kernel(Creds::root());
    let tmpfs = string(&mut k, DATA, b"tmpfs");
    let bogus = string(&mut k, DATA + 0x100, b"nosuchfs");
    assert_eq!(sys(&mut k, nr::FSOPEN, &[tmpfs, 2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSOPEN, &[UNMAPPED, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::FSOPEN, &[bogus, 0]), Err(Errno::ENODEV));
    assert_eq!(sys(&mut k, nr::FSOPEN, &[tmpfs, u64::from(FSOPEN_CLOEXEC)]), Ok(3));
    assert_eq!(sys(&mut k, nr::FSMOUNT, &[3, 2, 0]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSMOUNT, &[3, 0, 0x30]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSMOUNT, &[3, 0, MOUNT_ATTR_IDMAP]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FSMOUNT, &[u64::MAX, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::FSMOUNT, &[3, 1, MOUNT_ATTR_NODEV | MOUNT_ATTR_NOATIME]), Ok(4));
    assert_eq!(taken(&log), ["fsopen Create true", "fsmount 3 0x14 true"]);
}

#[test]
fn statmount_and_listmount() {
    let (mut k, _log) = kernel(Creds::user(1000, 1000));
    let req = DATA;
    let buf = DATA + 0x1000;
    let put = |k: &mut FakeKernel, size: u32, spare: u32, id: u64, param: u64| {
        let mut b = Vec::new();
        b.extend_from_slice(&size.to_le_bytes());
        b.extend_from_slice(&spare.to_le_bytes());
        b.extend_from_slice(&id.to_le_bytes());
        b.extend_from_slice(&param.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        k.poke(req, &b);
    };
    put(&mut k, 24, 0, unique_id(2), STATMOUNT_FS_TYPE);
    assert_eq!(sys(&mut k, nr::STATMOUNT, &[req, buf, 4096, 0]), Ok(0));
    assert_eq!(k.peek_u64(buf + 8), STATMOUNT_FS_TYPE);
    assert_eq!(k.peek(buf + 512, 6), b"tmpfs\0");
    assert_eq!(sys(&mut k, nr::STATMOUNT, &[req, buf, 100, 0]), Err(Errno::EOVERFLOW));
    assert_eq!(sys(&mut k, nr::STATMOUNT, &[req, buf, 4096, 1]), Err(Errno::EINVAL));
    put(&mut k, 24, 0, unique_id(3), 0);
    assert_eq!(sys(&mut k, nr::STATMOUNT, &[req, buf, 4096, 0]), Err(Errno::ENOENT));
    put(&mut k, 24, 0, LSMT_ROOT, 0);
    assert_eq!(sys(&mut k, nr::STATMOUNT, &[req, buf, 4096, 0]), Err(Errno::ENOENT));
    put(&mut k, 24, 0, 5, 0);
    assert_eq!(sys(&mut k, nr::STATMOUNT, &[req, buf, 4096, 0]), Err(Errno::EINVAL));
    // listmount
    put(&mut k, 24, 0, LSMT_ROOT, 0);
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 8, 0]), Ok(2));
    assert_eq!((k.peek_u64(buf), k.peek_u64(buf + 8)), (unique_id(1), unique_id(2)));
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 8, 2]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 1_000_000_000, 0]), Err(Errno::EOVERFLOW));
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, UNMAPPED, 8, 0]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 0, 0]), Ok(0));
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[UNMAPPED, buf, 8, 0]), Err(Errno::EFAULT));
    put(&mut k, 16, 0, LSMT_ROOT, 0);
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 8, 0]), Err(Errno::EINVAL));
    put(&mut k, 24, 1, LSMT_ROOT, 0);
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 8, 0]), Err(Errno::EINVAL));
    put(&mut k, 24, 0, 0, 0);
    assert_eq!(sys(&mut k, nr::LISTMOUNT, &[req, buf, 8, 0]), Err(Errno::EINVAL));
}

/// `AT_FDCWD` as a raw argument.
const AT_FDCWD_U: u64 = -100i64 as u64;
