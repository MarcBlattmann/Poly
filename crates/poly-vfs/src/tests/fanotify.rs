//! fanotify(7) on the VFS: groups, marks and the events the VFS operations
//! queue, read back in the layout of `linux/fanotify.h`.

use super::Env;
use crate::fanotify::bits::*;
use crate::fanotify::{Fanotify, Hooks, MarkKey, MarkOp, set_hooks};
use crate::*;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// The test hooks: pid 42; the "descriptor" of an event is its object's
/// inode number, so tests can tell which object an event names.
fn hooks() {
    fn pid() -> i32 {
        42
    }
    fn open(node: &NodeRef, _flags: OpenFlags) -> Result<i32> {
        Ok(node.ino() as i32)
    }
    set_hooks(Hooks { actor_pid: pid, open_event_fd: open });
}

/// One decoded event: event_len, mask, fd, pid.
type Ev = (u32, u64, i32, i32);

fn drain(g: &Fanotify) -> Vec<Ev> {
    let mut buf = vec![0u8; 4096];
    let n = match g.read(&mut buf) {
        Ok(n) => n,
        Err(Errno::EAGAIN) => return Vec::new(),
        Err(e) => panic!("read: {e:?}"),
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < n {
        let len = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        assert_eq!(buf[at + 4], 3, "version");
        assert_eq!(u16::from_le_bytes(buf[at + 6..at + 8].try_into().unwrap()), 24);
        let mask = u64::from_le_bytes(buf[at + 8..at + 16].try_into().unwrap());
        let fd = i32::from_le_bytes(buf[at + 16..at + 20].try_into().unwrap());
        let pid = i32::from_le_bytes(buf[at + 20..at + 24].try_into().unwrap());
        out.push((len, mask, fd, pid));
        at += len as usize;
    }
    out
}

fn node(env: &Env, path: &str) -> NodeRef {
    env.vfs.lookup(&env.ctx, &env.root, At::Cwd, path.as_bytes(), true).unwrap()
}

fn mark(env: &Env, g: &Arc<Fanotify>, path: &str, mask: u32) {
    g.mark(MarkKey::inode(&node(env, path)), MarkOp::Add, mask, false).unwrap();
}

fn touch(env: &Env, path: &str, flags: OpenFlags) {
    drop(env.open(path, flags).unwrap());
}

#[test]
fn file_events_merge_and_layout() {
    hooks();
    let env = Env::new();
    env.write("/f", b"");
    let ino = env.ino("/f").unwrap() as i32;
    let g = Fanotify::new(FAN_NONBLOCK, OpenFlags::RDONLY, true);
    let mut buf = [0u8; 64];
    assert_eq!(g.read(&mut buf), Err(Errno::EAGAIN));
    mark(&env, &g, "/f", FAN_OPEN | FAN_MODIFY | FAN_CLOSE_WRITE | FAN_CLOSE_NOWRITE);
    let f = env.open("/f", OpenFlags::RDWR).unwrap();
    assert_eq!(f.write(b"x"), Ok(1));
    drop(f);
    assert_eq!(g.read(&mut buf[..23]), Err(Errno::EINVAL), "too small for one event");
    assert_eq!(drain(&g), vec![(24, u64::from(FAN_OPEN | FAN_MODIFY | FAN_CLOSE_WRITE), ino, 42)]);
    touch(&env, "/f", OpenFlags::RDONLY);
    assert_eq!(drain(&g), vec![(24, u64::from(FAN_OPEN | FAN_CLOSE_NOWRITE), ino, 42)]);
    assert_eq!(g.write(b"x"), Err(Errno::EINVAL));
}

#[test]
fn remove_flush_and_drop() {
    hooks();
    let env = Env::new();
    env.write("/f", b"");
    let g = Fanotify::new(0, OpenFlags::RDONLY, true);
    mark(&env, &g, "/f", FAN_OPEN | FAN_CLOSE_WRITE | FAN_CLOSE_NOWRITE);
    let key = MarkKey::inode(&node(&env, "/f"));
    g.mark(key, MarkOp::Remove, FAN_OPEN | FAN_CLOSE_NOWRITE, false).unwrap();
    touch(&env, "/f", OpenFlags::RDONLY);
    assert!(drain(&g).is_empty());
    touch(&env, "/f", OpenFlags::WRONLY);
    assert_eq!(drain(&g).iter().map(|e| e.1).collect::<Vec<_>>(), vec![u64::from(FAN_CLOSE_WRITE)]);
    g.flush(false, false);
    assert_eq!(g.mark_count(), 0);
    assert_eq!(g.mark(key, MarkOp::Remove, FAN_OPEN, false), Err(Errno::ENOENT));
    touch(&env, "/f", OpenFlags::WRONLY);
    assert!(drain(&g).is_empty());
}

#[test]
fn directory_children_ondir_and_ignore() {
    hooks();
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"");
    env.write("/d/g", b"");
    let f = env.ino("/d/f").unwrap() as i32;
    let d = env.ino("/d").unwrap() as i32;
    let g = Fanotify::new(0, OpenFlags::RDONLY, true);
    mark(&env, &g, "/d", FAN_OPEN | FAN_EVENT_ON_CHILD);
    touch(&env, "/d/f", OpenFlags::RDONLY);
    assert_eq!(drain(&g), vec![(24, u64::from(FAN_OPEN), f, 42)]);
    touch(&env, "/d", OpenFlags::RDONLY | OpenFlags::DIRECTORY);
    assert!(drain(&g).is_empty(), "the directory's own open needs FAN_ONDIR");
    mark(&env, &g, "/d", FAN_ONDIR);
    touch(&env, "/d", OpenFlags::RDONLY | OpenFlags::DIRECTORY);
    assert_eq!(drain(&g), vec![(24, u64::from(FAN_OPEN), d, 42)]);
    g.mark(MarkKey::inode(&node(&env, "/d/g")), MarkOp::Add, FAN_OPEN, true).unwrap();
    touch(&env, "/d/g", OpenFlags::RDONLY);
    assert!(drain(&g).is_empty(), "ignored");
    touch(&env, "/d/f", OpenFlags::RDONLY);
    assert_eq!(drain(&g).len(), 1);
}

#[test]
fn quiet_files_cause_no_events() {
    hooks();
    let env = Env::new();
    env.write("/f", b"");
    let g = Fanotify::new(0, OpenFlags::RDONLY, true);
    mark(&env, &g, "/f", FAN_OPEN | FAN_ACCESS | FAN_CLOSE_NOWRITE);
    let f = env.vfs.open_node_quiet(&node(&env, "/f"), OpenFlags::RDONLY | OpenFlags::LARGEFILE).unwrap();
    let mut b = [0u8; 4];
    let _ = f.read(&mut b);
    assert_eq!(f.flags(), OpenFlags::RDONLY | OpenFlags::LARGEFILE);
    drop(f);
    assert!(drain(&g).is_empty());
}

#[test]
fn fid_groups_report_entries_with_info_records() {
    hooks();
    let env = Env::new();
    env.mkdir("/d");
    let g = Fanotify::new(FAN_REPORT_FID, OpenFlags::RDONLY, true);
    mark(&env, &g, "/d", FAN_CREATE | FAN_DELETE);
    env.write("/d/h", b"");
    env.vfs.unlink(&env.ctx, &env.root, At::Cwd, b"/d/h").unwrap();
    let evs = drain(&g);
    assert_eq!(evs.len(), 1, "both name the directory: merged");
    let (len, mask, fd, _) = evs[0];
    assert_eq!(fd, FAN_NOFD);
    assert_eq!(mask, u64::from(FAN_CREATE | FAN_DELETE));
    assert_eq!(len, 24 + 4 + 8 + 8 + 12, "the info record holds tmpfs's 12-byte handle");
    // A subdirectory entry needs FAN_ONDIR.
    env.mkdir("/d/sub");
    assert!(drain(&g).is_empty());
}

#[test]
fn mount_marks_see_everything_on_the_mount() {
    hooks();
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"");
    let g = Fanotify::new(0, OpenFlags::RDONLY, true);
    g.mark(MarkKey::mount(&node(&env, "/")), MarkOp::Add, FAN_OPEN, false).unwrap();
    touch(&env, "/d/f", OpenFlags::RDONLY);
    assert_eq!(drain(&g).len(), 1);
    g.flush(true, false);
    touch(&env, "/d/f", OpenFlags::RDONLY);
    assert!(drain(&g).is_empty());
}
