/* What the root of a user namespace can do with the mounts it inherited,
 * beyond their own flags. Root (in a private mount namespace) prepares:
 * "x", a tmpfs mounted noexec and noatime; "y", a plain tmpfs with a
 * second tmpfs mounted nodev on "y/in". A child with uid 65534 then makes
 * unshare(CLONE_NEWUSER | CLONE_NEWNS), maps itself to root and makes the
 * mounts private.
 * Every inherited mount is locked in place, the one below another as
 * well: umount2 of x, of y/in and of y (with or without MNT_DETACH) is
 * EINVAL, and so is moving one away with MS_MOVE.
 * Flags: a flag the mount had cannot be cleared (noexec of x, nodev of
 * y/in: EPERM); a flag it did not have can be set and cleared again. The
 * access-time mode is locked on every inherited mount, whatever it was: a
 * bind remount that names another mode (MS_RELATIME for the noatime mount
 * x, MS_NOATIME or MS_STRICTATIME for y, which has the default) is EPERM,
 * while a bind remount that names no mode keeps the mode and works.
 * Copies: a recursive bind (MS_BIND | MS_REC) of an inherited mount is a
 * mount the namespace's root made, so its top can be unmounted; the copies
 * of the mounts below it stay locked onto it (umount2 of <copy>/in EINVAL;
 * the whole copy goes with MNT_DETACH, a plain umount2 of the top is
 * EBUSY because of the mount below). A plain bind of a mount that has a
 * locked mount below it is refused (EINVAL), because it would show what
 * that mount covers; a plain bind of a mount without one works, the copy
 * keeps the flag locks (clearing noexec on the copy is EPERM) and can be
 * unmounted. Mounting on top of an inherited mount works, and that new
 * mount can be unmounted again.
 * poly-diff: needs=root (the test starts as root and becomes uid 65534; creating user namespaces is a system setting)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

#define B "/tmp/pk_umin"
#define X B "/x"
#define Y B "/y"
#define C1 B "/c1"
#define C2 B "/c2"
#define C3 B "/c3"

#define MS_NOEXEC_ 8
#define MS_NOATIME_ 1024
#define MS_MOVE_ 8192
#define MS_RELATIME_ (1 << 21)
#define MS_STRICTATIME_ (1 << 24)
#define BR (MS_BIND | MS_REMOUNT)

static i64 remount(const char *path, u64 flags) { return p_mount("", path, P_NULL, BR | flags, P_NULL); }

static int body(void) {
    expect("own_tree_private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);
    expect("mount_x", p_mount("pkx", X, "tmpfs", MS_NOEXEC_ | MS_NOATIME_, "mode=0755"), 0);
    expect("mount_y", p_mount("pky", Y, "tmpfs", 0, "mode=0777"), 0);
    expect("mkdir_in", a_mkdir(Y "/in", 0777), 0);
    expect("mount_y_in", p_mount("pkin", Y "/in", "tmpfs", MS_NODEV, "mode=0777"), 0);
    expect("file_in", a_make_file(Y "/in/f", "f", 0644), 0);
    expect("drop", p_drop_to_nobody(), 0);
    syscall2(SYS_prctl, PR_SET_DUMPABLE, 1);
    expect("unshare", syscall1(SYS_unshare, CLONE_NEWUSER | CLONE_NEWNS), 0);
    expect("maps", p_write_maps(0, 65534, 65534, 1), 0);
    expect("private", p_mount("", "/", P_NULL, MS_REC | MS_PRIVATE, P_NULL), 0);

    /* locked in place */
    expect("umount_x", syscall2(SYS_umount2, X, 0), -EINVAL);
    expect("umount_x_detach", syscall2(SYS_umount2, X, MNT_DETACH), -EINVAL);
    expect("umount_submount", syscall2(SYS_umount2, Y "/in", 0), -EINVAL);
    expect("umount_submount_detach", syscall2(SYS_umount2, Y "/in", MNT_DETACH), -EINVAL);
    expect("umount_y_detach", syscall2(SYS_umount2, Y, MNT_DETACH), -EINVAL);
    expect("move_x", p_mount(X, C1, P_NULL, MS_MOVE_, P_NULL), -EINVAL);
    expect("move_submount", p_mount(Y "/in", C1, P_NULL, MS_MOVE_, P_NULL), -EINVAL);

    /* flags */
    expect("x_same_flags_no_atime_named", remount(X, MS_NOEXEC_), 0);
    expect("x_same_flags_atime_named", remount(X, MS_NOEXEC_ | MS_NOATIME_), 0);
    expect("x_clear_noexec", remount(X, MS_NOATIME_), -EPERM);
    expect("x_other_atime_mode", remount(X, MS_NOEXEC_ | MS_RELATIME_), -EPERM);
    expect("x_add_nosuid", remount(X, MS_NOEXEC_ | MS_NOSUID), 0);
    expect("x_drop_nosuid_again", remount(X, MS_NOEXEC_), 0);
    expect("in_clear_nodev", remount(Y "/in", 0), -EPERM);
    expect("in_keep_nodev_add_ro", remount(Y "/in", MS_NODEV | MS_RDONLY), 0);
    expect("in_ro_now", a_make_file(Y "/in/g", "g", 0644), -EROFS);
    expect("in_drop_ro", remount(Y "/in", MS_NODEV), 0);
    expect("y_noatime", remount(Y, MS_NOATIME_), -EPERM);
    expect("y_strictatime", remount(Y, MS_STRICTATIME_), -EPERM);
    expect("y_relatime_is_its_mode", remount(Y, MS_RELATIME_), 0);
    expect("y_no_mode_named", remount(Y, 0), 0);

    /* copies */
    expect("plain_bind_with_locked_submount", p_mount(Y, C1, P_NULL, MS_BIND, P_NULL), -EINVAL);
    expect("rbind_y", p_mount(Y, C1, P_NULL, MS_BIND | MS_REC, P_NULL), 0);
    struct a_stat st;
    expect("rbind_shows_submount_file", a_stat_path(C1 "/in/f", &st), 0);
    expect("copy_submount_locked", syscall2(SYS_umount2, C1 "/in", 0), -EINVAL);
    expect("copy_submount_keeps_nodev_lock", remount(C1 "/in", 0), -EPERM);
    expect("copy_top_busy", syscall2(SYS_umount2, C1, 0), -EBUSY);
    expect("copy_top_detach", syscall2(SYS_umount2, C1, MNT_DETACH), 0);
    expect("copy_gone", a_stat_path(C1 "/in/f", &st), -ENOENT);
    expect("plain_bind_x", p_mount(X, C2, P_NULL, MS_BIND, P_NULL), 0);
    expect("bind_copy_keeps_noexec_lock", remount(C2, 0), -EPERM);
    expect("bind_copy_same_flags", remount(C2, MS_NOEXEC_), 0);
    expect("bind_copy_umount", syscall2(SYS_umount2, C2, 0), 0);
    expect("plain_bind_submount", p_mount(Y "/in", C3, P_NULL, MS_BIND, P_NULL), 0);
    expect("bind_of_submount_umount", syscall2(SYS_umount2, C3, 0), 0);

    /* on top of an inherited mount */
    expect("own_mount_over_x", p_mount("pkown", X, "tmpfs", 0, P_NULL), 0);
    expect("own_mount_umount", syscall2(SYS_umount2, X, 0), 0);
    expect("x_below_still_locked", syscall2(SYS_umount2, X, 0), -EINVAL);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    a_mkdir(B, 0755);
    a_mkdir(X, 0755);
    a_mkdir(Y, 0755);
    a_mkdir(C1, 0755);
    a_mkdir(C2, 0755);
    a_mkdir(C3, 0755);
    p_run("userns_mount_inherited", CLONE_NEWNS, body);
    a_rmdir(X);
    a_rmdir(Y);
    a_rmdir(C1);
    a_rmdir(C2);
    a_rmdir(C3);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
