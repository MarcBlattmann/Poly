//! inotify(7) on the VFS: instances, watches and the events the VFS
//! operations queue, read back in the record layout of `linux/inotify.h`.

use super::Env;
use crate::inotify::mask::*;
use crate::inotify::{Inotify, WatchKey};
use crate::*;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// One decoded event: wd, mask, cookie, name (without padding).
type Ev = (i32, u32, u32, Vec<u8>);

fn drain(i: &Inotify) -> Vec<Ev> {
    let mut buf = vec![0u8; 4096];
    let n = match i.read(&mut buf) {
        Ok(n) => n,
        Err(Errno::EAGAIN) => return Vec::new(),
        Err(e) => panic!("read: {e:?}"),
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < n {
        let word = |o: usize| u32::from_ne_bytes(buf[at + o..at + o + 4].try_into().unwrap());
        let len = word(12) as usize;
        let name = &buf[at + 16..at + 16 + len];
        let end = name.iter().position(|&b| b == 0).unwrap_or(len);
        out.push((word(0) as i32, word(4), word(8), name[..end].to_vec()));
        at += 16 + len;
    }
    out
}

fn watch(env: &Env, i: &Arc<Inotify>, path: &str, mask: u32) -> i32 {
    let node = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, path.as_bytes(), true).unwrap();
    i.add_watch(WatchKey::of_node(&node), mask).unwrap()
}

fn ev(wd: i32, mask: u32, name: &str) -> Ev {
    (wd, mask, 0, name.as_bytes().to_vec())
}

#[test]
fn empty_queue_and_record_layout() {
    let env = Env::new();
    env.mkdir("/d");
    let i = Inotify::new();
    let mut buf = [0u8; 64];
    assert_eq!(i.read(&mut buf), Err(Errno::EAGAIN));
    assert_eq!(i.read(&mut []), Err(Errno::EAGAIN));
    assert_eq!(i.poll(), PollEvents::NONE);
    assert_eq!(watch(&env, &i, "/d", IN_CREATE | IN_DELETE), 1);
    env.write("/d/abc", b"");
    env.write("/d/0123456789abcdef", b"");
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/abc").unwrap();
    assert_eq!(i.queued_bytes(), 32 + 48 + 32);
    assert_eq!(i.poll(), PollEvents::READABLE);
    // Too small for the next event: EINVAL, nothing taken.
    assert_eq!(i.read(&mut []), Err(Errno::EINVAL));
    assert_eq!(i.read(&mut buf[..31]), Err(Errno::EINVAL));
    assert_eq!(i.read(&mut buf[..32]), Ok(32));
    assert_eq!(&buf[16..20], b"abc\0");
    assert!(buf[19..32].iter().all(|&b| b == 0));
    assert_eq!(u32::from_ne_bytes(buf[12..16].try_into().unwrap()), 16);
    // Whole events only.
    let mut big = [0u8; 48 + 31];
    assert_eq!(i.read(&mut big), Ok(48));
    assert_eq!(drain(&i), vec![ev(1, IN_DELETE, "abc")]);
    assert_eq!(i.poll(), PollEvents::NONE);
}

#[test]
fn file_lifecycle_in_a_watched_directory() {
    let env = Env::new();
    env.mkdir("/d");
    let i = Inotify::new();
    watch(&env, &i, "/d", IN_ALL_EVENTS);
    let f = env.open("/d/f", OpenFlags::WRONLY | OpenFlags::CREAT).unwrap();
    f.write(b"a").unwrap();
    f.write(b"b").unwrap();
    drop(f);
    let r = env.open("/d/f", OpenFlags::RDONLY).unwrap();
    let mut b = [0u8; 4];
    r.read(&mut b).unwrap();
    drop(r);
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d/f", 0o600, true).unwrap();
    env.vfs.rename(&env.ctx, &env.root, At::Cwd, b"/d/f", At::Cwd, b"/d/g", RenameFlags::default()).unwrap();
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/g").unwrap();
    env.mkdir("/d/sub");
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/d/sub").unwrap();
    let got = drain(&i);
    let cookie = got[8].2;
    assert_ne!(cookie, 0);
    assert_eq!(
        got,
        vec![
            ev(1, IN_CREATE, "f"),
            ev(1, IN_OPEN, "f"),
            ev(1, IN_MODIFY, "f"), // two writes, merged
            ev(1, IN_CLOSE_WRITE, "f"),
            ev(1, IN_OPEN, "f"),
            ev(1, IN_ACCESS, "f"),
            ev(1, IN_CLOSE_NOWRITE, "f"),
            ev(1, IN_ATTRIB, "f"),
            (1, IN_MOVED_FROM, cookie, b"f".to_vec()),
            (1, IN_MOVED_TO, cookie, b"g".to_vec()),
            ev(1, IN_DELETE, "g"),
            ev(1, IN_CREATE | IN_ISDIR, "sub"),
            ev(1, IN_DELETE | IN_ISDIR, "sub"),
        ]
    );
}

#[test]
fn masks_replace_add_create_and_oneshot() {
    let env = Env::new();
    env.mkdir("/d");
    let i = Inotify::new();
    let churn = || {
        env.write("/d/f", b"");
        env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f").unwrap();
        drain(&i).len()
    };
    assert_eq!(watch(&env, &i, "/d", IN_CREATE), 1);
    assert_eq!(churn(), 1);
    assert_eq!(watch(&env, &i, "/d", IN_DELETE), 1);
    assert_eq!(churn(), 1);
    assert_eq!(watch(&env, &i, "/d", IN_CREATE | IN_MASK_ADD), 1);
    assert_eq!(churn(), 2);
    assert_eq!(watch(&env, &i, "/d", IN_IGNORED), 1);
    assert_eq!(churn(), 0);
    let node = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/d", true).unwrap();
    assert_eq!(i.add_watch(WatchKey::of_node(&node), IN_CREATE | IN_MASK_CREATE), Err(Errno::EEXIST));
    // The same inode through another path is the same watch.
    assert_eq!(watch(&env, &i, "/d/.", IN_CREATE | IN_DELETE | IN_ONESHOT), 1);
    env.write("/d/f", b"");
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f").unwrap();
    assert_eq!(drain(&i), vec![ev(1, IN_CREATE, "f"), ev(1, IN_IGNORED, "")]);
    assert_eq!(i.rm_watch(1), Err(Errno::EINVAL));
    assert_eq!(churn(), 0);
    // Numbers are not reused.
    assert_eq!(watch(&env, &i, "/d", IN_CREATE), 2);
}

#[test]
fn rm_watch_queues_ignored_after_pending_events() {
    let env = Env::new();
    env.mkdir("/d");
    let i = Inotify::new();
    let other = Inotify::new();
    watch(&env, &i, "/d", IN_CREATE);
    assert_eq!(watch(&env, &other, "/d", IN_CREATE), 1, "numbering is per instance");
    env.write("/d/f", b"");
    assert_eq!(i.rm_watch(1), Ok(()));
    assert_eq!(i.rm_watch(1), Err(Errno::EINVAL));
    env.write("/d/g", b"");
    assert_eq!(drain(&i), vec![ev(1, IN_CREATE, "f"), ev(1, IN_IGNORED, "")]);
    assert_eq!(drain(&other), vec![ev(1, IN_CREATE, "f"), ev(1, IN_CREATE, "g")]);
}

#[test]
fn deleting_the_watched_inode_ends_the_watch() {
    let env = Env::new();
    env.mkdir("/d");
    env.mkdir("/d/sub");
    env.write("/d/f", b"");
    env.write("/d/g", b"");
    let i = Inotify::new();
    assert_eq!(watch(&env, &i, "/d/sub", IN_DELETE_SELF), 1);
    assert_eq!(watch(&env, &i, "/d/f", IN_DELETE_SELF), 2);
    assert_eq!(watch(&env, &i, "/d/g", IN_DELETE_SELF | IN_ATTRIB), 3);
    env.vfs.rmdir(&env.ctx, &env.root, At::Cwd, b"/d/sub").unwrap();
    assert_eq!(drain(&i), vec![ev(1, IN_DELETE_SELF, ""), ev(1, IN_IGNORED, "")]);
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/f").unwrap();
    assert_eq!(drain(&i), vec![ev(2, IN_DELETE_SELF, ""), ev(2, IN_IGNORED, "")]);
    // Unlinked while open: the link count changes now, the inode goes at
    // the last close.
    let g = env.open("/d/g", OpenFlags::RDONLY).unwrap();
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/g").unwrap();
    assert_eq!(drain(&i), vec![ev(3, IN_ATTRIB, "")]);
    drop(g);
    assert_eq!(drain(&i), vec![ev(3, IN_DELETE_SELF, ""), ev(3, IN_IGNORED, "")]);
    assert_eq!(i.watch_count(), 0);
    assert_eq!(i.rm_watch(3), Err(Errno::EINVAL));
}

#[test]
fn events_on_a_watched_file_have_no_name() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/g", b"");
    let i = Inotify::new();
    assert_eq!(watch(&env, &i, "/d/g", IN_MODIFY | IN_ATTRIB), 1);
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d/g", 0o600, true).unwrap();
    let f = env.open("/d/g", OpenFlags::WRONLY).unwrap();
    f.write(b"z").unwrap();
    drop(f);
    assert_eq!(drain(&i), vec![ev(1, IN_ATTRIB, ""), ev(1, IN_MODIFY, "")]);
}

#[test]
fn closing_the_instance_removes_its_watches() {
    let env = Env::new();
    env.mkdir("/d");
    let i = Inotify::new();
    watch(&env, &i, "/d", IN_CREATE);
    let weak = Arc::downgrade(&i);
    drop(i);
    assert!(weak.upgrade().is_none());
    env.write("/d/f", b"");
}

#[test]
fn anonymous_inode_mode_has_no_type_bits() {
    let env = Env::new();
    let node = env.vfs.anon_node(FileType::Regular, 0o600, &env.root).unwrap();
    let file = OpenFile::on_object(node, Inotify::new(), OpenFlags::RDONLY);
    assert_eq!(file.stat().unwrap().mode, 0o600);
    assert_eq!(file.write(b"x"), Err(Errno::EBADF));
    assert_eq!(file.seek(5, Whence::Set), Ok(0));
    assert_eq!(file.bytes_readable(), Some(0));
}
