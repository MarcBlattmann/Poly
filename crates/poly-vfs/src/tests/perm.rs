//! Permission checks: owner/group/other classes, search permission on
//! every directory walked, root's bypass, the sticky bit, set-group-ID
//! directories, and who may chmod, chown and set times.

use super::Env;
use crate::*;

const ALICE: u32 = 1000;
const BOB: u32 = 1001;
const STAFF: u32 = 50;

fn alice() -> Cred {
    Cred::user(ALICE, ALICE)
}

fn bob() -> Cred {
    Cred::user(BOB, BOB).with_groups(&[STAFF])
}

impl Env {
    fn chmod(&self, path: &str, perm: u16) {
        self.vfs.chmod(&self.ctx, &self.root, At::Cwd, path.as_bytes(), perm, true).unwrap();
    }

    fn chown(&self, path: &str, uid: u32, gid: u32) {
        self.vfs.chown(&self.ctx, &self.root, At::Cwd, path.as_bytes(), Some(uid), Some(gid), true).unwrap();
    }

    fn open_as(&self, cred: &Cred, path: &str, flags: OpenFlags) -> Result<()> {
        self.vfs.open(&self.ctx, cred, At::Cwd, path.as_bytes(), flags, 0o644).map(|_| ())
    }

    fn access_as(&self, cred: &Cred, path: &str, want: Access) -> Result<()> {
        self.vfs.access(&self.ctx, cred, At::Cwd, path.as_bytes(), want, true)
    }
}

#[test]
fn classes_are_exclusive() {
    let env = Env::new();
    env.write("/f", b"x");
    env.chown("/f", ALICE, STAFF);
    // Owner: nothing; group: read; other: read and write.
    env.chmod("/f", 0o046);
    assert_eq!(env.open_as(&alice(), "/f", OpenFlags::RDONLY), Err(Errno::EACCES));
    assert_eq!(env.open_as(&bob(), "/f", OpenFlags::RDONLY), Ok(()));
    assert_eq!(env.open_as(&bob(), "/f", OpenFlags::WRONLY), Err(Errno::EACCES));
    let carol = Cred::user(1002, 1002);
    assert_eq!(env.open_as(&carol, "/f", OpenFlags::RDWR), Ok(()));
    // The primary group counts like a supplementary one.
    let staff_primary = Cred::user(1003, STAFF);
    assert_eq!(env.open_as(&staff_primary, "/f", OpenFlags::RDONLY), Ok(()));
    assert_eq!(env.open_as(&staff_primary, "/f", OpenFlags::WRONLY), Err(Errno::EACCES));
    // O_TRUNC needs write permission even with O_RDONLY.
    assert_eq!(env.open_as(&bob(), "/f", OpenFlags::RDONLY | OpenFlags::TRUNC), Err(Errno::EACCES));
}

#[test]
fn search_permission_on_every_directory() {
    let env = Env::new();
    env.mkdir("/locked");
    env.mkdir("/locked/inner");
    env.write("/locked/inner/f", b"x");
    env.chmod("/locked", 0o700);
    assert_eq!(env.open_as(&alice(), "/locked/inner/f", OpenFlags::RDONLY), Err(Errno::EACCES));
    assert_eq!(env.vfs.stat(&env.ctx, &alice(), At::Cwd, b"/locked/inner", true), Err(Errno::EACCES));
    // The directory itself can be stat'ed: only its parent is searched.
    assert!(env.vfs.stat(&env.ctx, &alice(), At::Cwd, b"/locked", true).is_ok());
    assert_eq!(env.vfs.stat(&env.ctx, &alice(), At::Cwd, b"/locked/.", true), Err(Errno::EACCES));
    // Readable but not searchable: listable, but names inside cannot be used.
    env.chmod("/locked", 0o744);
    assert_eq!(env.open_as(&alice(), "/locked", OpenFlags::RDONLY), Ok(()));
    assert_eq!(env.open_as(&alice(), "/locked/inner/f", OpenFlags::RDONLY), Err(Errno::EACCES));
    // Searchable but not readable: names work, listing does not.
    env.chmod("/locked", 0o711);
    assert_eq!(env.open_as(&alice(), "/locked/inner/f", OpenFlags::RDONLY), Ok(()));
    assert_eq!(env.open_as(&alice(), "/locked", OpenFlags::RDONLY), Err(Errno::EACCES));
    // chdir needs search permission on the target.
    env.chmod("/locked/inner", 0o600);
    let mut ctx = env.ctx.clone();
    assert_eq!(env.vfs.chdir(&mut ctx, &alice(), At::Cwd, b"/locked/inner"), Err(Errno::EACCES));
    // Root walks anywhere.
    env.chmod("/locked", 0o000);
    assert_eq!(env.read("/locked/inner/f").unwrap(), b"x");
}

#[test]
fn root_bypass_and_execute() {
    let env = Env::new();
    env.write("/f", b"x");
    env.chown("/f", ALICE, ALICE);
    env.chmod("/f", 0o000);
    assert_eq!(env.open_as(&env.root, "/f", OpenFlags::RDWR), Ok(()));
    // Root may execute only what has some execute bit.
    assert_eq!(env.access_as(&env.root, "/f", Access::EXEC), Err(Errno::EACCES));
    env.chmod("/f", 0o001);
    assert_eq!(env.access_as(&env.root, "/f", Access::EXEC), Ok(()));
    assert_eq!(env.access_as(&env.root, "/f", Access::READ | Access::WRITE), Ok(()));
    assert_eq!(env.access_as(&alice(), "/f", Access::READ), Err(Errno::EACCES));
    assert_eq!(env.access_as(&alice(), "/f", Access::NONE), Ok(()));
    assert_eq!(env.access_as(&alice(), "/nope", Access::NONE), Err(Errno::ENOENT));
    assert_eq!(Access::from_bits(8), Err(Errno::EINVAL));
    // open_exec: needs execute permission and a regular file.
    env.chmod("/f", 0o744);
    assert!(env.vfs.open_exec(&env.ctx, &alice(), At::Cwd, b"/f").is_ok());
    env.chmod("/f", 0o700);
    assert_eq!(env.vfs.open_exec(&env.ctx, &bob(), At::Cwd, b"/f").map(|_| ()), Err(Errno::EACCES));
    assert_eq!(env.vfs.open_exec(&env.ctx, &env.root, At::Cwd, b"/").map(|_| ()), Err(Errno::EACCES));
}

#[test]
fn changing_a_directory_needs_write_and_search() {
    let env = Env::new();
    env.mkdir("/d");
    env.write("/d/f", b"x");
    env.chown("/d", ALICE, ALICE);
    env.chmod("/d", 0o555);
    let a = alice();
    let ctx = &env.ctx;
    assert_eq!(env.open_as(&a, "/d/new", OpenFlags::WRONLY | OpenFlags::CREAT), Err(Errno::EACCES));
    assert_eq!(env.vfs.mkdir(ctx, &a, At::Cwd, b"/d/sub", 0o755), Err(Errno::EACCES));
    assert_eq!(env.vfs.unlink(ctx, &a, At::Cwd, b"/d/f"), Err(Errno::EACCES));
    assert_eq!(env.vfs.symlink(ctx, &a, b"x", At::Cwd, b"/d/l"), Err(Errno::EACCES));
    assert_eq!(env.vfs.rename(ctx, &a, At::Cwd, b"/d/f", At::Cwd, b"/d/g", RenameFlags::default()), Err(Errno::EACCES));
    // Existing names are EEXIST before any permission question.
    assert_eq!(env.vfs.mkdir(ctx, &a, At::Cwd, b"/d/f", 0o755), Err(Errno::EEXIST));
    env.chmod("/d", 0o755);
    assert_eq!(env.open_as(&a, "/d/new", OpenFlags::WRONLY | OpenFlags::CREAT), Ok(()));
    let st = env.stat("/d/new").unwrap();
    assert_eq!((st.uid, st.gid), (ALICE, ALICE));
    // A new file is opened as asked even if its mode forbids it.
    let f = env.vfs.open(ctx, &a, At::Cwd, b"/d/ro", OpenFlags::WRONLY | OpenFlags::CREAT, 0o444).unwrap();
    assert_eq!(f.write(b"ok"), Ok(2));
    assert_eq!(env.open_as(&a, "/d/ro", OpenFlags::WRONLY), Err(Errno::EACCES));
}

#[test]
fn sticky_directories() {
    let env = Env::new();
    env.mkdir("/tmp");
    env.chmod("/tmp", 0o1777);
    let ctx = &env.ctx;
    env.vfs.open(ctx, &alice(), At::Cwd, b"/tmp/a", OpenFlags::WRONLY | OpenFlags::CREAT, 0o666).unwrap();
    env.vfs.open(ctx, &bob(), At::Cwd, b"/tmp/b", OpenFlags::WRONLY | OpenFlags::CREAT, 0o666).unwrap();
    // Bob cannot remove or replace Alice's file, or move it away.
    assert_eq!(env.vfs.unlink(ctx, &bob(), At::Cwd, b"/tmp/a"), Err(Errno::EPERM));
    assert_eq!(
        env.vfs.rename(ctx, &bob(), At::Cwd, b"/tmp/b", At::Cwd, b"/tmp/a", RenameFlags::default()),
        Err(Errno::EPERM)
    );
    assert_eq!(
        env.vfs.rename(ctx, &bob(), At::Cwd, b"/tmp/a", At::Cwd, b"/tmp/c", RenameFlags::default()),
        Err(Errno::EPERM)
    );
    // His own file is fine; so is anything for root.
    env.vfs.rename(ctx, &bob(), At::Cwd, b"/tmp/b", At::Cwd, b"/tmp/c", RenameFlags::default()).unwrap();
    env.vfs.unlink(ctx, &bob(), At::Cwd, b"/tmp/c").unwrap();
    env.vfs.unlink(ctx, &env.root, At::Cwd, b"/tmp/a").unwrap();
    // Without the sticky bit, write permission on the directory is enough.
    env.vfs.open(ctx, &alice(), At::Cwd, b"/tmp/a", OpenFlags::WRONLY | OpenFlags::CREAT, 0o666).unwrap();
    env.chmod("/tmp", 0o777);
    env.vfs.unlink(ctx, &bob(), At::Cwd, b"/tmp/a").unwrap();
}

#[test]
fn set_group_id_directories() {
    let env = Env::new();
    env.mkdir("/shared");
    env.chown("/shared", 0, STAFF);
    env.chmod("/shared", 0o2777);
    let ctx = &env.ctx;
    env.vfs.open(ctx, &alice(), At::Cwd, b"/shared/f", OpenFlags::WRONLY | OpenFlags::CREAT, 0o2755).unwrap();
    let f = env.stat("/shared/f").unwrap();
    assert_eq!(f.gid, STAFF);
    // Alice is not in STAFF, so the new file loses set-group-ID.
    assert_eq!(f.perm() & S_ISGID, 0);
    env.vfs.mkdir(ctx, &alice(), At::Cwd, b"/shared/sub", 0o755).unwrap();
    let d = env.stat("/shared/sub").unwrap();
    assert_eq!(d.gid, STAFF);
    assert_eq!(d.perm(), 0o2755);
}

#[test]
fn chmod_chown_and_times() {
    let env = Env::new();
    env.write("/f", b"x");
    env.chown("/f", ALICE, ALICE);
    let node = env.vfs.lookup(&env.ctx, &env.root, At::Cwd, b"/f", true).unwrap();
    // chmod: owner or root.
    assert_eq!(node.chmod(&bob(), 0o777), Err(Errno::EPERM));
    node.chmod(&alice(), 0o2755).unwrap();
    // Alice is in the file's group, so set-group-ID stays.
    assert_eq!(node.stat().unwrap().perm(), 0o2755);
    env.chown("/f", ALICE, STAFF);
    node.chmod(&alice(), 0o2755).unwrap();
    assert_eq!(node.stat().unwrap().perm(), 0o755, "not in STAFF: set-group-ID dropped");

    // chown: only root gives a file away; the owner picks among her groups.
    assert_eq!(node.chown(&alice(), Some(BOB), None), Err(Errno::EPERM));
    assert_eq!(node.chown(&alice(), None, Some(STAFF + 1)), Err(Errno::EPERM));
    node.chown(&alice(), None, Some(ALICE)).unwrap();
    assert_eq!(node.stat().unwrap().gid, ALICE);
    assert_eq!(node.chown(&bob(), None, Some(STAFF)), Err(Errno::EPERM));
    node.chown(&alice(), Some(ALICE), None).unwrap(); // to herself: allowed
    // Changing ownership clears set-user-ID (and group-executable S_ISGID).
    node.chmod(&env.root, 0o6755).unwrap();
    node.chown(&env.root, Some(BOB), None).unwrap();
    let st = node.stat().unwrap();
    assert_eq!((st.uid, st.perm()), (BOB, 0o755));

    // Times: explicit needs ownership; "now" needs ownership or write access.
    let t = Some(SetTime::At(Timespec::new(1, 0)));
    assert_eq!(node.set_times(&alice(), t, t), Err(Errno::EPERM));
    assert_eq!(node.set_times(&alice(), Some(SetTime::Now), Some(SetTime::Now)), Err(Errno::EACCES));
    node.chmod(&env.root, 0o666).unwrap();
    node.set_times(&alice(), Some(SetTime::Now), Some(SetTime::Now)).unwrap();
    node.set_times(&bob(), t, t).unwrap();
    assert_eq!(node.stat().unwrap().mtime, Timespec::new(1, 0));
}

#[test]
fn moving_a_directory_needs_write_permission_on_it() {
    let env = Env::new();
    env.mkdir("/a");
    env.mkdir("/b");
    env.mkdir("/a/d");
    env.chown("/a", ALICE, ALICE);
    env.chown("/b", ALICE, ALICE);
    env.chmod("/a/d", 0o555);
    env.chown("/a/d", ALICE, ALICE);
    let ctx = &env.ctx;
    let f = RenameFlags::default();
    assert_eq!(env.vfs.rename(ctx, &alice(), At::Cwd, b"/a/d", At::Cwd, b"/b/d", f), Err(Errno::EACCES));
    // Within the same parent its ".." does not change.
    env.vfs.rename(ctx, &alice(), At::Cwd, b"/a/d", At::Cwd, b"/a/e", f).unwrap();
}

#[test]
fn device_nodes_need_root() {
    let env = Env::new();
    let c = DeviceId::new(1, 3);
    let ctx = &env.ctx;
    // The directory's permission comes before CAP_MKNOD (mknod's spec
    // sheet), and an existing name before both.
    assert_eq!(env.vfs.mknod(ctx, &alice(), At::Cwd, b"/null", FileType::CharDevice, 0o666, c), Err(Errno::EACCES));
    env.vfs.mknod(ctx, &env.root, At::Cwd, b"/null", FileType::CharDevice, 0o666, c).unwrap();
    assert_eq!(env.vfs.mknod(ctx, &alice(), At::Cwd, b"/null", FileType::CharDevice, 0o666, c), Err(Errno::EEXIST));
    assert_eq!(env.stat("/null").unwrap().rdev, c);
    assert_eq!(env.vfs.mknod(ctx, &env.root, At::Cwd, b"/d", FileType::Directory, 0o755, c), Err(Errno::EINVAL));
    // FIFOs and sockets: anyone who may write the directory.
    env.chmod("/", 0o777);
    assert_eq!(env.vfs.mknod(ctx, &alice(), At::Cwd, b"/zero", FileType::CharDevice, 0o666, c), Err(Errno::EPERM));
    env.vfs.mknod(ctx, &alice(), At::Cwd, b"/fifo", FileType::Fifo, 0o644, c).unwrap();
    assert_eq!(env.stat("/fifo").unwrap().rdev, DeviceId::default());
}
