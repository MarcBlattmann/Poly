//! The credential rules, one man page paragraph at a time.

use super::*;

const ALICE: u32 = 1000;
const BOB: u32 = 1001;

fn ids(c: &Creds) -> [u32; 4] {
    [c.uid, c.euid, c.suid, c.fsuid]
}

fn gids(c: &Creds) -> [u32; 4] {
    [c.gid, c.egid, c.sgid, c.fsgid]
}

/// Real `r`, effective `e`, saved `s` user ids, with root's capabilities
/// adjusted as if set by setresuid from root (keep_caps off).
fn creds(r: u32, e: u32, s: u32) -> Creds {
    let mut c = Creds::root();
    c.set_resuid(Some(r), Some(e), Some(s)).unwrap();
    c
}

/// A login from root: groups, then group ids, then user ids; a user
/// without capabilities, root with all of them.
#[test]
fn login_sets_groups_gids_and_uids_from_root() {
    let c = Creds::login([ALICE; 3], [100; 3], &[27, 4, ALICE]).unwrap();
    assert_eq!((ids(&c), gids(&c)), ([ALICE; 4], [100; 4]));
    assert_eq!(&*c.groups, &[4, 27, ALICE]);
    assert_eq!(c.caps, Caps::NONE, "a user other than root has no capabilities");
    assert!(!c.capable(cap::DAC_OVERRIDE) && !c.capable(cap::SETUID));
    assert_eq!(c.fs_ids().caps, CapSet::EMPTY);
    // Real, effective and saved ids apart: a set-user-ID-root program's.
    let c = Creds::login([ALICE, 0, 0], [100, 100, 100], &[]).unwrap();
    assert_eq!(ids(&c), [ALICE, 0, 0, 0]);
    assert!(c.capable(cap::SETUID), "effective uid 0 keeps root's capabilities");
    let root = Creds::login([0; 3], [0; 3], &[0]).unwrap();
    assert_eq!(root.caps, Caps::ROOT);
    assert_eq!(&*root.groups, &[0]);
    assert_eq!(Creds::login([NO_ID; 3], [0; 3], &[]), Err(Errno::EINVAL));
    assert_eq!(Creds::login([ALICE; 3], [0; 3], &[NO_ID]), Err(Errno::EINVAL));
}

#[test]
fn root_and_user() {
    let root = Creds::root();
    assert_eq!(ids(&root), [0; 4]);
    assert_eq!(root.caps, Caps::ROOT);
    assert!(root.capable(cap::SETUID) && root.capable(cap::LAST));
    assert!(!root.capable(cap::LAST + 1));
    assert_eq!(Creds::default(), root);
    let u = Creds::user(ALICE, 100);
    assert_eq!((ids(&u), gids(&u)), ([ALICE; 4], [100; 4]));
    assert!(u.caps.effective.is_empty() && u.caps.permitted.is_empty());
    assert_eq!(u.caps.bounding, CapSet::ALL);
    assert_eq!(u.summary(), Credentials { uid: ALICE, euid: ALICE, gid: 100, egid: 100 });
    assert_eq!(CapSet::ALL.0, (1 << 41) - 1);
    assert_eq!(CapSet::from_words(u32::MAX, u32::MAX), CapSet::ALL, "unknown bits dropped");
    assert_eq!(CapSet::ALL.words(), (u32::MAX, 0x1ff));
}

#[test]
fn setuid_privileged_sets_all_three_and_drops_capabilities() {
    let mut c = Creds::root();
    c.set_uid(ALICE).unwrap();
    assert_eq!(ids(&c), [ALICE; 4]);
    assert_eq!((c.caps.permitted, c.caps.effective), (CapSet::EMPTY, CapSet::EMPTY), "rule 1");
    assert_eq!(c.caps.bounding, CapSet::ALL, "the bounding set stays");
    // Now unprivileged: back to root is EPERM, -1 is EINVAL.
    assert_eq!(c.set_uid(0), Err(Errno::EPERM));
    assert_eq!(c.set_uid(NO_ID), Err(Errno::EINVAL));
    assert_eq!(ids(&c), [ALICE; 4], "unchanged after errors");
}

#[test]
fn setuid_unprivileged_changes_the_effective_id_to_real_or_saved() {
    // A set-user-ID program: real ALICE, effective and saved BOB.
    let mut c = creds(ALICE, BOB, BOB);
    assert!(c.caps.permitted.is_empty());
    c.set_uid(ALICE).unwrap();
    assert_eq!(ids(&c), [ALICE, ALICE, BOB, ALICE]);
    c.set_uid(BOB).unwrap();
    assert_eq!(ids(&c), [ALICE, BOB, BOB, BOB]);
    assert_eq!(c.set_uid(7), Err(Errno::EPERM));
}

#[test]
fn set_user_id_root_program_can_drop_and_regain_root() {
    // Real ALICE, effective and saved 0: a set-user-ID-root program.
    let mut c = Creds::root();
    c.set_resuid(Some(ALICE), None, None).unwrap();
    assert_eq!(c.caps, Caps::ROOT, "one id still 0 and euid unchanged: nothing lost");
    // Temporarily drop: the effective set empties (rule 2), permitted stays.
    c.set_resuid(None, Some(ALICE), None).unwrap();
    assert_eq!(ids(&c), [ALICE, ALICE, 0, ALICE]);
    assert!(c.caps.effective.is_empty());
    assert_eq!(c.caps.permitted, CapSet::ALL);
    // ... and regain through the saved id (rule 3).
    c.set_uid(0).unwrap();
    assert_eq!(ids(&c), [ALICE, 0, 0, 0], "unprivileged path: only euid");
    // With CAP_SETUID back, set_uid now sets all three ids.
    assert_eq!(c.caps.effective, CapSet::ALL);
    c.set_uid(ALICE).unwrap();
    assert_eq!(ids(&c), [ALICE; 4]);
    assert!(c.caps.permitted.is_empty());
}

#[test]
fn setreuid_rules() {
    let mut c = creds(ALICE, BOB, 7);
    // Unprivileged: real to real or effective; effective to any of three.
    assert_eq!(c.clone().set_reuid(Some(7), None), Err(Errno::EPERM));
    assert_eq!(c.clone().set_reuid(None, Some(8)), Err(Errno::EPERM));
    // Effective set to a value other than the old real id: saved follows.
    let mut d = c.clone();
    d.set_reuid(None, Some(7)).unwrap();
    assert_eq!(ids(&d), [ALICE, 7, 7, 7]);
    // Effective set to the real id: saved unchanged.
    let mut d = c.clone();
    d.set_reuid(None, Some(ALICE)).unwrap();
    assert_eq!(ids(&d), [ALICE, ALICE, 7, ALICE]);
    // Real set: saved becomes the new effective.
    c.set_reuid(Some(BOB), Some(ALICE)).unwrap();
    assert_eq!(ids(&c), [BOB, ALICE, ALICE, ALICE]);
    // -1, -1 changes nothing.
    let before = c.clone();
    c.set_reuid(None, None).unwrap();
    assert_eq!(c, before);
    // Privileged: anything.
    let mut r = Creds::root();
    r.set_reuid(Some(5), Some(6)).unwrap();
    assert_eq!(ids(&r), [5, 6, 6, 6]);
}

#[test]
fn setresuid_rules() {
    let mut c = creds(ALICE, BOB, 7);
    assert_eq!(c.clone().set_resuid(Some(8), None, None), Err(Errno::EPERM));
    c.set_resuid(Some(7), Some(ALICE), Some(BOB)).unwrap();
    assert_eq!(ids(&c), [7, ALICE, BOB, ALICE]);
    c.set_resuid(None, None, None).unwrap();
    assert_eq!(ids(&c), [7, ALICE, BOB, ALICE]);
    let mut r = Creds::root();
    r.set_resuid(Some(1), Some(2), Some(3)).unwrap();
    assert_eq!(ids(&r), [1, 2, 3, 2]);
}

#[test]
fn keep_caps_keeps_the_permitted_set() {
    let mut c = Creds::root();
    c.keep_caps = true;
    c.set_resuid(Some(ALICE), Some(ALICE), Some(ALICE)).unwrap();
    assert_eq!(c.caps.permitted, CapSet::ALL);
    assert!(c.caps.effective.is_empty(), "rule 2 still applies: euid left 0");
    // Effective already nonzero, then the rest: effective survives.
    let mut c = Creds::root();
    c.set_resuid(None, Some(ALICE), None).unwrap();
    c.set_caps(CapSet::only(cap::KILL), CapSet::ALL, CapSet::EMPTY).unwrap();
    c.keep_caps = true;
    c.set_resuid(Some(ALICE), None, Some(ALICE)).unwrap();
    assert_eq!((c.caps.effective, c.caps.permitted), (CapSet::only(cap::KILL), CapSet::ALL));
    // Without keep_caps the same change clears both.
    let mut c = Creds::root();
    c.set_resuid(None, Some(ALICE), None).unwrap();
    c.set_caps(CapSet::only(cap::KILL), CapSet::ALL, CapSet::EMPTY).unwrap();
    c.set_resuid(Some(ALICE), None, Some(ALICE)).unwrap();
    assert!(c.caps.effective.is_empty() && c.caps.permitted.is_empty());
}

#[test]
fn file_system_ids() {
    let mut c = creds(ALICE, BOB, 7);
    // Unprivileged: real, effective, saved or current only; returns the old one.
    assert_eq!(c.set_fsuid(9), BOB);
    assert_eq!(c.fsuid, BOB);
    assert_eq!(c.set_fsuid(7), BOB);
    assert_eq!(c.set_fsuid(NO_ID), 7, "-1 reads it");
    assert_eq!(c.fsuid, 7);
    // Changing the effective id resets it.
    c.set_resuid(None, Some(ALICE), None).unwrap();
    assert_eq!(c.fsuid, ALICE);
    // Root: an fsuid away from 0 drops the file capabilities only (rule 4).
    let mut r = Creds::root();
    assert_eq!(r.set_fsuid(ALICE), 0);
    assert_eq!(r.caps.effective, CapSet::ALL.minus(CapSet::FS));
    assert!(r.capable(cap::KILL) && !r.capable(cap::DAC_OVERRIDE) && !r.capable(cap::MKNOD));
    assert_eq!(r.fs_ids().uid, ALICE);
    // Still CAP_SETUID: back to 0 restores them from the permitted set.
    assert_eq!(r.set_fsuid(0), ALICE);
    assert_eq!(r.caps.effective, CapSet::ALL);
    // Groups: no capability effects.
    let mut g = Creds::user(ALICE, 100);
    assert_eq!(g.set_fsgid(5), 100);
    assert_eq!(g.fsgid, 100);
    let mut rg = Creds::root();
    assert_eq!(rg.set_fsgid(5), 0);
    assert_eq!((rg.fsgid, rg.caps), (5, Caps::ROOT));
}

#[test]
fn group_ids_and_groups() {
    let mut r = Creds::root();
    r.set_groups(&[10, 20, 10]).unwrap();
    assert_eq!(&*r.groups, &[10, 10, 20], "sorted, duplicates kept");
    assert_eq!(r.set_groups(&[NO_ID]), Err(Errno::EINVAL));
    assert_eq!(r.set_groups(&alloc::vec![1; NGROUPS_MAX + 1]), Err(Errno::EINVAL));
    r.set_gid(100).unwrap();
    assert_eq!(gids(&r), [100; 4]);
    assert_eq!(r.caps, Caps::ROOT, "group changes leave capabilities alone");
    r.set_resgid(Some(1), Some(2), Some(3)).unwrap();
    assert_eq!(gids(&r), [1, 2, 3, 2]);
    r.set_regid(Some(4), None).unwrap();
    assert_eq!(gids(&r), [4, 2, 2, 2]);
    let mut u = Creds::user(ALICE, 100);
    assert_eq!(u.set_groups(&[]), Err(Errno::EPERM));
    assert_eq!(u.set_gid(5), Err(Errno::EPERM));
    assert_eq!(u.set_gid(NO_ID), Err(Errno::EINVAL));
    assert_eq!(u.set_resgid(Some(5), None, None), Err(Errno::EPERM));
    assert_eq!(u.set_regid(None, Some(5)), Err(Errno::EPERM));
    u.set_resgid(None, None, Some(100)).unwrap();
    // Dropping CAP_SETGID makes root unable to change groups.
    let mut d = Creds::root();
    d.set_caps(CapSet::ALL.without(cap::SETGID), CapSet::ALL, CapSet::EMPTY).unwrap();
    assert_eq!(d.set_groups(&[1]), Err(Errno::EPERM));
    assert_eq!(d.set_gid(5), Err(Errno::EPERM));
}

#[test]
fn capset_rules() {
    let mut r = Creds::root();
    // Drop from effective, keep permitted: allowed; raise back: allowed.
    r.set_caps(CapSet::EMPTY, CapSet::ALL, CapSet::EMPTY).unwrap();
    assert!(!r.capable(cap::SETPCAP));
    r.set_caps(CapSet::ALL, CapSet::ALL, CapSet::EMPTY).unwrap();
    // Effective outside permitted: EPERM.
    assert_eq!(r.set_caps(CapSet::only(cap::KILL), CapSet::only(cap::CHOWN), CapSet::EMPTY), Err(Errno::EPERM));
    // Shrink permitted; it cannot grow again.
    r.set_caps(CapSet::only(cap::KILL), CapSet::only(cap::KILL), CapSet::EMPTY).unwrap();
    assert_eq!(r.set_caps(CapSet::only(cap::KILL), CapSet::ALL, CapSet::EMPTY), Err(Errno::EPERM));
    // Inheritable: only from permitted without CAP_SETPCAP.
    assert_eq!(r.set_caps(CapSet::EMPTY, CapSet::only(cap::KILL), CapSet::only(cap::CHOWN)), Err(Errno::EPERM));
    r.set_caps(CapSet::EMPTY, CapSet::only(cap::KILL), CapSet::only(cap::KILL)).unwrap();
    // With CAP_SETPCAP anything in the bounding set.
    let mut p = Creds::root();
    p.set_caps(CapSet::only(cap::SETPCAP), CapSet::only(cap::SETPCAP), CapSet::only(cap::CHOWN)).unwrap();
    assert_eq!(p.caps.inheritable, CapSet::only(cap::CHOWN));
    let mut q = Creds::root();
    q.drop_bounding(cap::NET_RAW).unwrap();
    q.set_caps(CapSet::only(cap::SETPCAP), CapSet::only(cap::SETPCAP), CapSet::EMPTY).unwrap();
    assert_eq!(
        q.set_caps(CapSet::only(cap::SETPCAP), CapSet::only(cap::SETPCAP), CapSet::only(cap::NET_RAW)),
        Err(Errno::EPERM)
    );
}

#[test]
fn bounding_and_ambient_sets() {
    let mut r = Creds::root();
    assert_eq!(r.drop_bounding(cap::LAST + 1), Err(Errno::EINVAL));
    r.drop_bounding(cap::SYS_TIME).unwrap();
    assert!(!r.caps.bounding.contains(cap::SYS_TIME));
    assert!(r.capable(cap::SYS_TIME), "dropping from the bounding set leaves the others");
    let mut u = Creds::user(ALICE, ALICE);
    assert_eq!(u.drop_bounding(cap::SYS_TIME), Err(Errno::EPERM));
    // Ambient: permitted and inheritable first.
    assert_eq!(r.raise_ambient(cap::NET_RAW), Err(Errno::EPERM));
    r.set_caps(CapSet::ALL, CapSet::ALL, CapSet::only(cap::NET_RAW)).unwrap();
    r.raise_ambient(cap::NET_RAW).unwrap();
    assert_eq!(r.caps.ambient, CapSet::only(cap::NET_RAW));
    // Dropping it from inheritable drops it from ambient.
    let mut a = r.clone();
    a.set_caps(CapSet::ALL, CapSet::ALL, CapSet::EMPTY).unwrap();
    assert!(a.caps.ambient.is_empty());
    r.lower_ambient(cap::NET_RAW).unwrap();
    assert!(r.caps.ambient.is_empty());
    assert_eq!(r.lower_ambient(99), Err(Errno::EINVAL));
    assert_eq!(r.raise_ambient(99), Err(Errno::EINVAL));
}

#[test]
fn access_uses_the_real_ids() {
    let mut c = Creds::root();
    c.set_groups(&[7]).unwrap();
    c.set_resuid(Some(ALICE), None, None).unwrap();
    c.set_resgid(Some(100), None, None).unwrap();
    let fs = c.fs_ids();
    assert_eq!((fs.uid, fs.gid, &*fs.groups, fs.caps), (0, 0, &[7][..], CapSet::ALL));
    let a = c.access_ids();
    assert_eq!((a.uid, a.gid, &*a.groups, a.caps), (ALICE, 100, &[7][..], CapSet::EMPTY));
    // Real root checks with the permitted set.
    let mut r = Creds::root();
    r.set_caps(CapSet::EMPTY, CapSet::only(cap::DAC_OVERRIDE), CapSet::EMPTY).unwrap();
    assert_eq!(r.access_ids().caps, CapSet::only(cap::DAC_OVERRIDE));
    assert_eq!(r.fs_ids().caps, CapSet::EMPTY);
}

#[test]
fn exec_of_a_set_user_id_root_program() {
    let user = Creds::user(ALICE, 100);
    let suid_root = ExecFile { uid: 0, gid: 0, perm: 0o4755, nosuid: false };
    let (c, secure) = user.exec(&suid_root);
    assert_eq!(ids(&c), [ALICE, 0, 0, 0]);
    assert_eq!(gids(&c), [100; 4]);
    assert!(secure);
    assert_eq!((c.caps.permitted, c.caps.effective), (CapSet::ALL, CapSet::ALL), "root's capabilities");
    // The bounding set limits them.
    let mut b = user.clone();
    b.caps.bounding = CapSet::ALL.without(cap::SYS_ADMIN);
    let (c, _) = b.exec(&suid_root);
    assert!(!c.capable(cap::SYS_ADMIN) && c.capable(cap::SETUID));
    // nosuid and no_new_privs ignore the bit.
    let (c, secure) = user.exec(&ExecFile { nosuid: true, ..suid_root });
    assert_eq!((ids(&c), secure), ([ALICE; 4], false));
    assert!(c.caps.permitted.is_empty());
    let mut nnp = user.clone();
    nnp.no_new_privs = true;
    let (c, _) = nnp.exec(&suid_root);
    assert_eq!(ids(&c), [ALICE; 4]);
    assert!(c.no_new_privs, "inherited across execve");
    // Set-user-ID to another user: no capabilities.
    let (c, secure) = user.exec(&ExecFile { uid: BOB, gid: 0, perm: 0o4711, nosuid: false });
    assert_eq!((ids(&c), secure), ([ALICE, BOB, BOB, BOB], true));
    assert!(c.caps.permitted.is_empty());
}

#[test]
fn exec_of_a_set_group_id_program() {
    let user = Creds::user(ALICE, 100);
    let (c, secure) = user.exec(&ExecFile { uid: 0, gid: 50, perm: 0o2755, nosuid: false });
    assert_eq!((gids(&c), secure), ([100, 50, 50, 50], true));
    // Without group execute the bit means locking, not set-group-ID.
    let (c, secure) = user.exec(&ExecFile { uid: 0, gid: 50, perm: 0o2745, nosuid: false });
    assert_eq!((gids(&c), secure), ([100; 4], false));
}

#[test]
fn exec_as_root_and_after_dropping_root() {
    let plain = ExecFile { uid: ALICE, gid: ALICE, perm: 0o755, nosuid: false };
    // Root that emptied its effective set gets it back from the bounding set.
    let mut r = Creds::root();
    r.set_caps(CapSet::EMPTY, CapSet::ALL, CapSet::EMPTY).unwrap();
    r.keep_caps = true;
    let (c, secure) = r.exec(&plain);
    assert_eq!((c.caps, secure), (Caps::ROOT, false));
    assert!(!c.keep_caps, "cleared by execve");
    // Real root with a nonzero effective id: permitted full, effective empty.
    let mut e = Creds::root();
    e.set_resuid(None, Some(ALICE), None).unwrap();
    let (c, secure) = e.exec(&plain);
    assert_eq!(ids(&c), [0, ALICE, ALICE, ALICE]);
    assert_eq!((c.caps.permitted, c.caps.effective, secure), (CapSet::ALL, CapSet::EMPTY, true));
    // A user that kept permitted capabilities loses them at exec.
    let mut k = Creds::root();
    k.keep_caps = true;
    k.set_resuid(Some(ALICE), Some(ALICE), Some(ALICE)).unwrap();
    assert_eq!(k.caps.permitted, CapSet::ALL);
    let (c, _) = k.exec(&plain);
    assert!(c.caps.permitted.is_empty() && c.caps.effective.is_empty());
    // ... unless they are ambient.
    let mut a = Creds::root();
    a.set_caps(CapSet::ALL, CapSet::ALL, CapSet::only(cap::NET_BIND_SERVICE)).unwrap();
    a.raise_ambient(cap::NET_BIND_SERVICE).unwrap();
    a.keep_caps = true;
    a.set_resuid(Some(ALICE), Some(ALICE), Some(ALICE)).unwrap();
    assert_eq!(a.caps.ambient, CapSet::EMPTY, "rule 1 clears the ambient set");
    let mut a = Creds::user(ALICE, ALICE);
    a.caps.permitted = CapSet::only(cap::NET_BIND_SERVICE);
    a.caps.inheritable = CapSet::only(cap::NET_BIND_SERVICE);
    a.raise_ambient(cap::NET_BIND_SERVICE).unwrap();
    let (c, _) = a.exec(&plain);
    let nbs = CapSet::only(cap::NET_BIND_SERVICE);
    assert_eq!((c.caps.permitted, c.caps.effective, c.caps.ambient), (nbs, nbs, nbs));
    // A set-user-ID program clears the ambient set.
    let (c, _) = a.exec(&ExecFile { uid: BOB, perm: 0o4755, ..plain });
    assert_eq!((c.caps.permitted, c.caps.ambient), (CapSet::EMPTY, CapSet::EMPTY));
}

// ---- secure bits (the spec sheet of linux/api/pkg/seccomp_prctl) ----------

#[test]
fn every_secure_bit_can_be_set_and_higher_bits_are_eperm() {
    let mut c = Creds::root();
    assert_eq!(c.securebits(), 0);
    // Each of the twelve bits by itself, a lock also without its base bit.
    for bit in 0..12 {
        let mut one = Creds::root();
        one.set_securebits(1 << bit).unwrap();
        assert_eq!(one.securebits(), 1 << bit);
    }
    // The value is replaced as a whole.
    for v in [0x4, 0x10, 0x40, 0x100, 0x400, 0x554, 0x5, 0] {
        c.set_securebits(v).unwrap();
        assert_eq!(u64::from(c.securebits()), v);
    }
    c.set_securebits(0x5).unwrap();
    for v in [0x1000, 0x1_0000, 0x1004, 1 << 32, u64::MAX] {
        assert_eq!(c.set_securebits(v), Err(Errno::EPERM), "{v:#x}");
    }
    assert_eq!(c.securebits(), 0x5, "a refused change changes nothing");
    // KEEP_CAPS is the flag of PR_SET_KEEPCAPS.
    c.set_securebits(0x10).unwrap();
    assert!(c.keep_caps);
    c.set_keep_caps(false).unwrap();
    assert_eq!(c.securebits(), 0);
    c.set_keep_caps(true).unwrap();
    assert_eq!(c.securebits(), 0x10);
}

#[test]
fn a_lock_bit_keeps_itself_and_its_base_bit() {
    let mut c = Creds::root();
    c.set_securebits(0xc).unwrap();
    for v in [0, 0x8, 0x4] {
        assert_eq!(c.set_securebits(v), Err(Errno::EPERM), "{v:#x}");
    }
    assert_eq!(c.securebits(), 0xc);
    // Other bits may change in the same call.
    c.set_securebits(0x1c).unwrap();
    c.set_securebits(0xc).unwrap();
    // The lock alone: its base bit must stay clear.
    let mut c = Creds::root();
    c.set_securebits(0x800).unwrap();
    for v in [0xc00, 0x100, 0] {
        assert_eq!(c.set_securebits(v), Err(Errno::EPERM), "{v:#x}");
    }
    c.set_securebits(0x800).unwrap();
    c.set_securebits(0x900).unwrap();
    assert_eq!(c.securebits(), 0x900);
    // NOROOT with its lock: more bits may be added, none taken.
    let mut c = Creds::root();
    c.set_securebits(3).unwrap();
    assert_eq!(c.set_securebits(0), Err(Errno::EPERM));
    assert_eq!(c.set_securebits(2), Err(Errno::EPERM));
    c.set_securebits(0x2f).unwrap();
    assert_eq!(c.securebits(), 0x2f);
    // PR_SET_KEEPCAPS while KEEP_CAPS is locked: EPERM for both values.
    assert_eq!(c.set_keep_caps(false), Err(Errno::EPERM));
    assert_eq!(c.set_keep_caps(true), Err(Errno::EPERM));
    assert_eq!(c.securebits(), 0x2f);
    let mut c = Creds::root();
    c.set_securebits(0x30).unwrap();
    assert_eq!(c.set_keep_caps(false), Err(Errno::EPERM));
    assert_eq!(c.set_keep_caps(true), Err(Errno::EPERM));
    assert_eq!(c.securebits(), 0x30);
}

#[test]
fn without_setpcap_only_the_exec_pairs_change() {
    let mut c = Creds::root();
    c.set_securebits(0x41).unwrap();
    c.set_resgid(Some(65534), Some(65534), Some(65534)).unwrap();
    c.set_resuid(Some(65534), Some(65534), Some(65534)).unwrap();
    assert!(!c.capable(cap::SETPCAP));
    assert_eq!(c.securebits(), 0x41);
    for v in [0x41, 0, 0x40, 0x45, 0x2041, 0x540] {
        assert_eq!(c.set_securebits(v), Err(Errno::EPERM), "{v:#x}");
    }
    assert_eq!(c.securebits(), 0x41);
    c.set_securebits(0x141).unwrap();
    assert_eq!(c.set_securebits(0x141), Err(Errno::EPERM), "the value that is there already");
    c.set_securebits(0x41).unwrap();
    c.set_securebits(0xc41).unwrap();
    assert_eq!(c.set_securebits(0x41), Err(Errno::EPERM), "locked now");
    assert_eq!(c.securebits(), 0xc41);
    // PR_SET_KEEPCAPS needs no capability.
    c.set_keep_caps(true).unwrap();
    assert_eq!(c.securebits(), 0xc51);
    // With the capability the same value is accepted.
    let mut r = Creds::root();
    r.set_securebits(0x800).unwrap();
    r.set_securebits(0x800).unwrap();
}

#[test]
fn no_cap_ambient_raise_refuses_every_raise() {
    let mut c = Creds::root();
    let two = CapSet::only(cap::CHOWN).with(cap::KILL);
    c.set_caps(CapSet::ALL, CapSet::ALL, two).unwrap();
    c.raise_ambient(cap::CHOWN).unwrap();
    c.set_securebits(0x40).unwrap();
    assert_eq!(c.raise_ambient(cap::KILL), Err(Errno::EPERM));
    assert_eq!(c.raise_ambient(cap::CHOWN), Err(Errno::EPERM), "also one that is raised already");
    assert_eq!(c.raise_ambient(99), Err(Errno::EINVAL));
    // What is in the set stays, and can be lowered.
    assert_eq!(c.caps.ambient, CapSet::only(cap::CHOWN));
    c.lower_ambient(cap::CHOWN).unwrap();
    assert!(c.caps.ambient.is_empty());
    c.set_securebits(0).unwrap();
    c.raise_ambient(cap::KILL).unwrap();
    // Fork children inherit the bits (the credentials are copied).
    c.set_securebits(0xc0).unwrap();
    let mut child = c.clone();
    assert_eq!(child.securebits(), 0xc0);
    assert_eq!(child.set_securebits(0), Err(Errno::EPERM));
    assert_eq!(child.raise_ambient(cap::CHOWN), Err(Errno::EPERM));
}

#[test]
fn leaving_uid_0_under_the_secure_bits() {
    let leave = |bits: u64| {
        let mut c = Creds::root();
        c.set_securebits(bits).unwrap();
        c.set_resuid(Some(65534), Some(65534), Some(65534)).unwrap();
        c
    };
    // Normally both sets become empty, and a capset cannot bring one back.
    let mut c = leave(0);
    assert!(c.caps.permitted.is_empty() && c.caps.effective.is_empty());
    assert_eq!(c.set_caps(CapSet::only(cap::KILL), CapSet::only(cap::KILL), CapSet::EMPTY), Err(Errno::EPERM));
    // NO_SETUID_FIXUP: both stay as they were.
    let c = leave(0x4);
    assert_eq!((c.caps.permitted, c.caps.effective), (CapSet::ALL, CapSet::ALL));
    // KEEP_CAPS: the permitted set stays, the effective one is emptied and
    // can be filled again; the bit is still set.
    let mut c = leave(0x10);
    assert_eq!((c.caps.permitted, c.caps.effective), (CapSet::ALL, CapSet::EMPTY));
    c.set_caps(CapSet::only(cap::KILL), CapSet::ALL, CapSet::EMPTY).unwrap();
    assert_eq!(c.securebits(), 0x10);
    // setfsuid under NO_SETUID_FIXUP leaves the effective set alone too.
    let mut c = Creds::root();
    c.set_securebits(0x4).unwrap();
    c.set_fsuid(65534);
    assert_eq!(c.caps.effective, CapSet::ALL);
}

#[test]
fn noroot_gives_root_nothing_at_exec() {
    let plain = ExecFile { uid: 0, gid: 0, perm: 0o755, nosuid: false };
    let mut c = Creds::root();
    c.set_securebits(0x1).unwrap();
    let (after, _) = c.exec(&plain);
    assert!(after.caps.permitted.is_empty() && after.caps.effective.is_empty());
    assert_eq!(after.securebits(), 0x1, "kept across execve");
    let (after, _) = Creds::root().exec(&plain);
    assert_eq!(after.caps.permitted, CapSet::ALL);
}
