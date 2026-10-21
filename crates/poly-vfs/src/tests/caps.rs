//! Privilege as capabilities (`capabilities(7)`, `path_resolution(7)`):
//! which capability each check asks for, a user 0 that dropped some, and
//! the set-user-ID / set-group-ID bits that writes, truncation and chown
//! clear.

use super::Env;
use crate::*;
use alloc::sync::Arc;

const ALICE: u32 = 1000;
const BOB: u32 = 1001;
const STAFF: u32 = 50;

fn alice() -> Cred {
    Cred::user(ALICE, ALICE)
}

fn bob() -> Cred {
    Cred::user(BOB, BOB).with_groups(&[STAFF])
}

/// User 0 without `dropped`.
fn root_without(dropped: &[u32]) -> Cred {
    let caps = dropped.iter().fold(CapSet::ALL, |c, &d| c.without(d));
    Cred::root().with_caps(caps)
}

impl Env {
    fn file_as(&self, path: &str, uid: u32, gid: u32, perm: u16) {
        self.write(path, b"data");
        self.vfs.chown(&self.ctx, &self.root, At::Cwd, path.as_bytes(), Some(uid), Some(gid), true).unwrap();
        self.vfs.chmod(&self.ctx, &self.root, At::Cwd, path.as_bytes(), perm, true).unwrap();
    }

    fn node(&self, path: &str) -> NodeRef {
        self.vfs.lookup(&self.ctx, &self.root, At::Cwd, path.as_bytes(), true).unwrap()
    }

    fn perm(&self, path: &str) -> u16 {
        self.stat(path).unwrap().perm()
    }

    fn open_file_as(&self, cred: &Cred, path: &str, flags: OpenFlags) -> Result<Arc<OpenFile>> {
        self.vfs.open(&self.ctx, cred, At::Cwd, path.as_bytes(), flags, 0o644)
    }
}

#[test]
fn capability_sets() {
    assert!(CapSet::ALL.contains(cap::LAST));
    assert!(!CapSet::ALL.contains(cap::LAST + 1));
    assert_eq!(CapSet::ALL.0, (1 << 41) - 1);
    assert!(!CapSet::ALL.without(cap::CHOWN).contains(cap::CHOWN));
    assert_eq!(CapSet::only(cap::FOWNER), CapSet(8));
    assert!(Cred::root().is_root());
    // CAP_KILL (5), which the VFS never checks: without it user 0 is still
    // less than root.
    assert!(!root_without(&[5]).is_root());
    assert!(!Cred::user(0, 0).has(cap::DAC_OVERRIDE), "user 0 without capabilities is not privileged");
}

#[test]
fn dac_override_and_read_search() {
    let env = Env::new();
    env.mkdir("/d");
    env.file_as("/d/f", ALICE, ALICE, 0o600);
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d", 0o700, true).unwrap();
    env.vfs.chown(&env.ctx, &env.root, At::Cwd, b"/d", Some(ALICE), Some(ALICE), true).unwrap();
    // User 0 without CAP_DAC_OVERRIDE and CAP_DAC_READ_SEARCH is an
    // ordinary "other" here.
    let plain = root_without(&[cap::DAC_OVERRIDE, cap::DAC_READ_SEARCH]);
    assert_eq!(env.open_file_as(&plain, "/d/f", OpenFlags::RDONLY).map(|_| ()), Err(Errno::EACCES));
    // CAP_DAC_READ_SEARCH: search the directory, read the file, not write it.
    let reader = Cred::user(BOB, BOB).with_caps(CapSet::only(cap::DAC_READ_SEARCH));
    assert!(env.open_file_as(&reader, "/d/f", OpenFlags::RDONLY).is_ok());
    assert_eq!(env.open_file_as(&reader, "/d/f", OpenFlags::WRONLY).map(|_| ()), Err(Errno::EACCES));
    assert!(env.open_file_as(&reader, "/d", OpenFlags::RDONLY | OpenFlags::DIRECTORY).is_ok());
    // It does not execute files.
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d/f", 0o700, true).unwrap();
    assert_eq!(env.vfs.access(&env.ctx, &reader, At::Cwd, b"/d/f", Access::EXEC, true), Err(Errno::EACCES));
    // CAP_DAC_OVERRIDE alone: read and write anything, execute with an x bit.
    let over = Cred::user(BOB, BOB).with_caps(CapSet::only(cap::DAC_OVERRIDE));
    assert!(env.open_file_as(&over, "/d/f", OpenFlags::RDWR).is_ok());
    assert_eq!(env.vfs.access(&env.ctx, &over, At::Cwd, b"/d/f", Access::EXEC, true), Ok(()));
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d/f", 0o600, true).unwrap();
    assert_eq!(env.vfs.access(&env.ctx, &over, At::Cwd, b"/d/f", Access::EXEC, true), Err(Errno::EACCES));
    // Creating in the directory needs write and search: override gives both.
    assert!(env.open_file_as(&over, "/d/new", OpenFlags::WRONLY | OpenFlags::CREAT).is_ok());
    assert_eq!(
        env.open_file_as(&reader, "/d/new2", OpenFlags::WRONLY | OpenFlags::CREAT).map(|_| ()),
        Err(Errno::EACCES)
    );
}

#[test]
fn fowner_acts_as_the_owner() {
    let env = Env::new();
    env.file_as("/f", ALICE, ALICE, 0o644);
    let node = env.node("/f");
    let fowner = Cred::user(BOB, BOB).with_caps(CapSet::only(cap::FOWNER));
    node.chmod(&fowner, 0o600).unwrap();
    assert_eq!(env.perm("/f"), 0o600);
    assert_eq!(node.chmod(&root_without(&[cap::FOWNER]), 0o644), Err(Errno::EPERM));
    let t = Some(SetTime::At(Timespec::new(5, 0)));
    node.set_times(&fowner, t, t).unwrap();
    assert_eq!(node.set_times(&bob(), t, t), Err(Errno::EPERM));
    // O_NOATIME: owner or CAP_FOWNER (EPERM), after the access check.
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/f", 0o666, true).unwrap();
    assert_eq!(env.open_file_as(&bob(), "/f", OpenFlags::RDONLY | OpenFlags::NOATIME).map(|_| ()), Err(Errno::EPERM));
    assert!(env.open_file_as(&alice(), "/f", OpenFlags::RDONLY | OpenFlags::NOATIME).is_ok());
    assert!(env.open_file_as(&fowner, "/f", OpenFlags::RDONLY | OpenFlags::NOATIME).is_ok());
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/f", 0o600, true).unwrap();
    assert_eq!(env.open_file_as(&bob(), "/f", OpenFlags::RDONLY | OpenFlags::NOATIME).map(|_| ()), Err(Errno::EACCES));
    // The sticky bit: CAP_FOWNER may remove anyone's file.
    env.mkdir("/tmp");
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/tmp", 0o1777, true).unwrap();
    env.open_file_as(&alice(), "/tmp/a", OpenFlags::WRONLY | OpenFlags::CREAT).unwrap();
    env.open_file_as(&alice(), "/tmp/b", OpenFlags::WRONLY | OpenFlags::CREAT).unwrap();
    assert_eq!(env.vfs.unlink(&env.ctx, &bob(), At::Cwd, b"/tmp/a"), Err(Errno::EPERM));
    // Every capability but CAP_FOWNER does not help (user 0 would: it owns
    // the directory).
    let no_fowner = Cred::user(BOB, BOB).with_caps(CapSet::ALL.without(cap::FOWNER));
    assert_eq!(env.vfs.unlink(&env.ctx, &no_fowner, At::Cwd, b"/tmp/a"), Err(Errno::EPERM));
    let bob_fowner = bob().with_caps(CapSet::only(cap::FOWNER));
    env.vfs.unlink(&env.ctx, &bob_fowner, At::Cwd, b"/tmp/a").unwrap();
    // The directory's owner may, too.
    let tmp_owner = Cred::user(0, 7);
    env.vfs.unlink(&env.ctx, &tmp_owner, At::Cwd, b"/tmp/b").unwrap();
}

#[test]
fn utimensat_rules() {
    let env = Env::new();
    env.file_as("/f", ALICE, ALICE, 0o666);
    let t = Some(SetTime::At(Timespec::new(7, 0)));
    let now = Some(SetTime::Now);
    let ctx = &env.ctx;
    let set = |cred: &Cred, a, m| env.vfs.utimens(ctx, cred, At::Cwd, b"/f", a, m, true);
    // Both "now": write permission is enough.
    assert_eq!(set(&bob(), now, now), Ok(()));
    // One "now", the other left alone: that is a change only the owner makes.
    assert_eq!(set(&bob(), now, None), Err(Errno::EPERM));
    assert_eq!(set(&bob(), t, None), Err(Errno::EPERM));
    // Both omitted: nothing to do, nothing checked.
    assert_eq!(set(&bob(), None, None), Ok(()));
    assert_eq!(set(&alice(), t, now), Ok(()));
    assert_eq!(env.stat("/f").unwrap().atime, Timespec::new(7, 0));
    // Without write permission "now" is EACCES; CAP_DAC_OVERRIDE allows it.
    env.vfs.chmod(ctx, &env.root, At::Cwd, b"/f", 0o644, true).unwrap();
    assert_eq!(set(&bob(), now, now), Err(Errno::EACCES));
    let over = bob().with_caps(CapSet::only(cap::DAC_OVERRIDE));
    assert_eq!(set(&over, now, now), Ok(()));
    assert_eq!(set(&over, t, t), Err(Errno::EPERM), "explicit times need ownership or CAP_FOWNER");
    assert_eq!(env.vfs.utimens(ctx, &bob(), At::Cwd, b"/nope", now, now, true), Err(Errno::ENOENT));
}

#[test]
fn chown_needs_cap_chown_and_chmod_keeps_setgid_with_cap_fsetid() {
    let env = Env::new();
    env.file_as("/f", ALICE, ALICE, 0o644);
    let node = env.node("/f");
    let chowner = Cred::user(BOB, BOB).with_caps(CapSet::only(cap::CHOWN));
    node.chown(&chowner, Some(BOB), Some(STAFF + 9)).unwrap();
    let st = env.stat("/f").unwrap();
    assert_eq!((st.uid, st.gid), (BOB, STAFF + 9));
    assert_eq!(node.chown(&root_without(&[cap::CHOWN]), Some(0), None), Err(Errno::EPERM));
    // The owner gives the group to one of hers; not to another.
    node.chown(&bob(), None, Some(STAFF)).unwrap();
    assert_eq!(node.chown(&bob(), None, Some(STAFF + 1)), Err(Errno::EPERM));
    // chmod by the owner outside the file's group: S_ISGID dropped, unless
    // CAP_FSETID.
    node.chown(&env.root, None, Some(STAFF + 1)).unwrap();
    node.chmod(&bob(), 0o2755).unwrap();
    assert_eq!(env.perm("/f"), 0o755);
    node.chmod(&bob().with_caps(CapSet::only(cap::FSETID)), 0o2755).unwrap();
    assert_eq!(env.perm("/f"), 0o2755);
    // Root without CAP_FSETID loses it too.
    node.chmod(&root_without(&[cap::FSETID]), 0o2755).unwrap();
    assert_eq!(env.perm("/f"), 0o755);
    // Setting the ids the file already has is allowed, and like every
    // successful chown clears the set-id bits (the spec of fchownat).
    node.chmod(&env.root, 0o6755).unwrap();
    let st = env.stat("/f").unwrap();
    node.chown(&env.root, Some(st.uid), Some(st.gid)).unwrap();
    assert_eq!(env.perm("/f"), 0o755);
    node.chmod(&env.root, 0o6755).unwrap();
    node.chown(&bob(), Some(st.uid), None).unwrap();
    node.chown(&bob(), None, None).unwrap();
    assert_eq!(env.perm("/f"), 0o755);
    assert_eq!(node.chown(&alice(), Some(st.uid), None), Err(Errno::EPERM), "not the owner");
}

#[test]
fn writes_and_truncation_clear_set_id_bits() {
    let env = Env::new();
    env.file_as("/f", ALICE, STAFF, 0o6777);
    let node = env.node("/f");
    // CAP_FSETID keeps them.
    node.clear_setid(&Cred::root()).unwrap();
    assert_eq!(env.perm("/f"), 0o6777);
    node.clear_setid(&bob()).unwrap();
    assert_eq!(env.perm("/f"), 0o777);
    // Without group execute, set-group-ID is a locking mark and stays.
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/f", 0o6767, true).unwrap();
    node.clear_setid(&alice()).unwrap();
    assert_eq!(env.perm("/f"), 0o2767);
    // Truncation by path, and O_TRUNC, do the same.
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/f", 0o6777, true).unwrap();
    env.vfs.truncate(&env.ctx, &bob(), At::Cwd, b"/f", 1).unwrap();
    assert_eq!(env.perm("/f"), 0o777);
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/f", 0o4777, true).unwrap();
    env.open_file_as(&bob(), "/f", OpenFlags::WRONLY | OpenFlags::TRUNC).unwrap();
    assert_eq!(env.perm("/f"), 0o777);
    // Root keeps them on truncation (CAP_FSETID), not on chown.
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/f", 0o4777, true).unwrap();
    env.open_file_as(&env.root, "/f", OpenFlags::WRONLY | OpenFlags::TRUNC).unwrap();
    assert_eq!(env.perm("/f"), 0o4777);
    node.chown(&env.root, Some(BOB), None).unwrap();
    assert_eq!(env.perm("/f"), 0o777);
    // Directories are left alone.
    env.mkdir("/d");
    env.vfs.chmod(&env.ctx, &env.root, At::Cwd, b"/d", 0o2775, true).unwrap();
    env.node("/d").clear_setid(&bob()).unwrap();
    assert_eq!(env.perm("/d"), 0o2775);
}

#[test]
fn mknod_mount_and_chroot_capabilities() {
    let env = Env::new();
    let c = DeviceId::new(1, 3);
    let ctx = &env.ctx;
    let no_mknod = root_without(&[cap::MKNOD]);
    assert_eq!(env.vfs.mknod(ctx, &no_mknod, At::Cwd, b"/null", FileType::CharDevice, 0o666, c), Err(Errno::EPERM));
    let mknod = Cred::user(BOB, BOB).with_caps(CapSet::only(cap::MKNOD).with(cap::DAC_OVERRIDE));
    env.vfs.mknod(ctx, &mknod, At::Cwd, b"/null", FileType::CharDevice, 0o666, c).unwrap();
    env.mkdir("/m");
    let no_admin = root_without(&[cap::SYS_ADMIN]);
    assert_eq!(env.vfs.mount(ctx, &no_admin, At::Cwd, b"/m", super::tmpfs(), MountFlags::default()), Err(Errno::EPERM));
    let mut c2 = env.ctx.clone();
    let node = env.node("/m");
    assert_eq!(c2.set_root(&root_without(&[cap::SYS_CHROOT]), node.clone()), Err(Errno::EPERM));
    c2.set_root(&Cred::user(BOB, BOB).with_caps(CapSet::only(cap::SYS_CHROOT)), node).unwrap();
}
