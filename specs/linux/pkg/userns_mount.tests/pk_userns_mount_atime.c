/* The access-time mode of an inherited mount is locked as a whole: the three
 * settings noatime, nodiratime and relatime/strictatime together. In the user
 * namespace a bind remount may name exactly the combination the mount has, or
 * name none of the access-time flags (then nothing changes), and nothing else:
 * adding nodiratime to a mount that lacks it is EPERM just like removing it,
 * naming relatime on a mount that is nodiratime (which clears nodiratime) is
 * EPERM, and so is any other mode. A mount with nodiratime only
 * ("a"), one with noatime and nodiratime ("b"), a default one ("c": relatime)
 * and a strictatime one ("d") are tried. The lock travels with a bind copy
 * (plain or recursive), nodiratime included. umount2 of an inherited mount
 * is EINVAL with flag 0 and with MNT_FORCE, whether the mount has mounts
 * below it ("y") or not ("z"): the lock is judged before whether it is busy.
 * poly-diff: needs=root (the test starts as root and becomes uid 65534; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

#define B "/tmp/pk_umatime"
#define MS_NOATIME_ 1024
#define MS_NODIRATIME_ 2048
#define MS_RELATIME_ (1 << 21)
#define MS_STRICTATIME_ (1 << 24)
#define BR (MS_BIND | MS_REMOUNT)
#define MNT_FORCE_ 1

static i64 remount(const char *path, u64 flags) { return p_mount("", path, P_NULL, BR | flags, P_NULL); }

static int body(void) {
    expect("own_tree_private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);
    expect("mount_a", p_mount("a", B "/a", "tmpfs", MS_NODIRATIME_, "mode=0755"), 0);
    expect("mount_b", p_mount("b", B "/b", "tmpfs", MS_NOATIME_ | MS_NODIRATIME_, "mode=0755"), 0);
    expect("mount_c", p_mount("c", B "/c", "tmpfs", 0, "mode=0777"), 0);
    expect("mount_d", p_mount("d", B "/d", "tmpfs", MS_STRICTATIME_, "mode=0777"), 0);
    expect("mount_x", p_mount("x", B "/x", "tmpfs", MS_NOATIME_, "mode=0777"), 0);
    expect("mount_y", p_mount("y", B "/y", "tmpfs", 0, "mode=0777"), 0);
    expect("mkdir_in", a_mkdir(B "/y/in", 0777), 0);
    expect("mount_in", p_mount("in", B "/y/in", "tmpfs", MS_NODEV, "mode=0777"), 0);
    expect("mount_z", p_mount("z", B "/z", "tmpfs", 0, "mode=0777"), 0);
    expect("drop", p_drop_to_nobody(), 0);
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("unshare", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    expect("maps", p_write_maps(0, 65534, 65534, 1), 0);
    expect("private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);

    /* a: nodiratime only */
    expect("a_none", remount(B "/a", 0), 0);
    expect("a_nodiratime", remount(B "/a", MS_NODIRATIME_), 0);
    expect("a_noatime", remount(B "/a", MS_NOATIME_), -EPERM);
    expect("a_noatime_nodiratime", remount(B "/a", MS_NOATIME_ | MS_NODIRATIME_), -EPERM);
    expect("a_relatime_clears_nodiratime", remount(B "/a", MS_RELATIME_), -EPERM);
    expect("a_relatime_nodiratime", remount(B "/a", MS_RELATIME_ | MS_NODIRATIME_), 0);
    expect("a_strictatime", remount(B "/a", MS_STRICTATIME_), -EPERM);
    expect("a_strictatime_nodiratime", remount(B "/a", MS_STRICTATIME_ | MS_NODIRATIME_), -EPERM);

    /* b: noatime and nodiratime */
    expect("b_none", remount(B "/b", 0), 0);
    expect("b_noatime_alone", remount(B "/b", MS_NOATIME_), -EPERM);
    expect("b_nodiratime_alone", remount(B "/b", MS_NODIRATIME_), -EPERM);
    expect("b_both", remount(B "/b", MS_NOATIME_ | MS_NODIRATIME_), 0);

    /* c: the default (relatime) */
    expect("c_add_nodiratime", remount(B "/c", MS_NODIRATIME_), -EPERM);
    expect("c_relatime_add_nodiratime", remount(B "/c", MS_RELATIME_ | MS_NODIRATIME_), -EPERM);
    expect("c_none", remount(B "/c", 0), 0);
    expect("c_relatime", remount(B "/c", MS_RELATIME_), 0);
    expect("c_noatime", remount(B "/c", MS_NOATIME_), -EPERM);

    /* d: strictatime */
    expect("d_none", remount(B "/d", 0), 0);
    expect("d_strictatime", remount(B "/d", MS_STRICTATIME_), 0);
    expect("d_relatime", remount(B "/d", MS_RELATIME_), -EPERM);
    expect("d_strictatime_nodiratime", remount(B "/d", MS_STRICTATIME_ | MS_NODIRATIME_), -EPERM);

    /* umount2: locked before busy */
    expect("umount_y_flag0", syscall2(SYS_umount2, B "/y", 0), -EINVAL);
    expect("umount_in_flag0", syscall2(SYS_umount2, B "/y/in", 0), -EINVAL);
    expect("umount_z_flag0", syscall2(SYS_umount2, B "/z", 0), -EINVAL);
    expect("umount_y_force", syscall2(SYS_umount2, B "/y", MNT_FORCE_), -EINVAL);
    expect("umount_z_force", syscall2(SYS_umount2, B "/z", MNT_FORCE_), -EINVAL);

    /* copies keep the whole lock */
    expect("bind_x", p_mount(B "/x", B "/c1", P_NULL, MS_BIND, P_NULL), 0);
    expect("copy_x_relatime", remount(B "/c1", MS_RELATIME_), -EPERM);
    expect("copy_x_strictatime", remount(B "/c1", MS_STRICTATIME_), -EPERM);
    expect("copy_x_none", remount(B "/c1", 0), 0);
    expect("copy_x_noatime", remount(B "/c1", MS_NOATIME_), 0);
    expect("bind_a", p_mount(B "/a", B "/c2", P_NULL, MS_BIND, P_NULL), 0);
    expect("copy_a_none", remount(B "/c2", 0), 0);
    expect("copy_a_noatime", remount(B "/c2", MS_NOATIME_), -EPERM);
    expect("copy_a_relatime_clears_nodiratime", remount(B "/c2", MS_RELATIME_), -EPERM);
    expect("copy_a_same", remount(B "/c2", MS_RELATIME_ | MS_NODIRATIME_), 0);
    expect("rbind_c", p_mount(B "/c", B "/c3", P_NULL, MS_BIND | MS_REC, P_NULL), 0);
    expect("copy_c_noatime", remount(B "/c3", MS_NOATIME_), -EPERM);
    expect("copy_c_nodiratime", remount(B "/c3", MS_NODIRATIME_), -EPERM);
    expect("copy_c_relatime", remount(B "/c3", MS_RELATIME_), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_mkdir(B, 0755);
    const char *n[] = {"/a", "/b", "/c", "/d", "/x", "/y", "/z", "/c1", "/c2", "/c3", 0};
    for (int i = 0; n[i]; i++) {
        char p[64];
        str_cat2(p, B, n[i]);
        a_mkdir(p, 0755);
    }
    p_run("userns_mount_atime", CLONE_NEWNS, body);
    for (int i = 0; n[i]; i++) {
        char p[64];
        str_cat2(p, B, n[i]);
        a_rmdir(p);
    }
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
