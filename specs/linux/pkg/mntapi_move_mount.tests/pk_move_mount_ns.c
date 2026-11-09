/* move_mount and mount namespaces. A detached mount is not yet in any
 * namespace: it can be attached in a namespace other than the one it was
 * made in (here a child's new namespace), and only once. A mount that is
 * attached in one namespace cannot be moved by a process of another one
 * (the descriptor is valid, the call is EINVAL). Without CAP_SYS_ADMIN
 * move_mount is EPERM (before any lookup).
 * poly-diff: needs=root
 * poly-diff: syscalls=off (several processes)
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_move_mount_ns"

static i64 g_det, g_att;
static char g_dir[128];

static void other_ns(void *arg) {
    (void)arg;
    expect("child_unshare", syscall1(SYS_unshare, CLONE_NEWNS), 0);
    expect("child_private", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    expect("attached_mount_of_other_ns", pm_move_mount(g_att, "", AT_FDCWD, g_dir, MOVE_MOUNT_F_EMPTY_PATH), -EINVAL);
    expect("detached_mount_attaches_here", pm_move_mount(g_det, "", AT_FDCWD, g_dir, MOVE_MOUNT_F_EMPTY_PATH), 0);
    expect("visible_in_child_ns", pm_statfs_blocks(g_dir), 256);
}

static i64 nobody_move(void *arg) {
    (void)arg;
    return pm_move_mount(g_det, "", AT_FDCWD, "/nonexistent-for-move-mount", MOVE_MOUNT_F_EMPTY_PATH);
}

static void body(const char *base) {
    char a[128], b[128];
    pm_path(a, base, "a");
    pm_path(b, base, "b");
    pm_path(g_dir, base, "child");
    expect("mkdir_a", pm_mkdir(a), 0);
    expect("mkdir_b", pm_mkdir(b), 0);
    expect("mkdir_child", pm_mkdir(g_dir), 0);

    /* an attached mount, and a detached one */
    i64 att = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    expect("attach_a", pm_attach(att, a), 0);
    g_att = att;
    g_det = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    check("detached", g_det >= 0, g_det);
    pm_child(other_ns, 0, 0);
    expect("parent_ns_unchanged", pm_statfs_blocks(g_dir), 1024);
    expect("detached_already_attached_elsewhere", pm_attach(g_det, b), -EINVAL);

    /* permission */
    i64 d2 = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    g_det = d2;
    expect("nobody_eperm", pm_as_nobody(nobody_move, 0), -EPERM);
    expect("root_ok_afterwards", pm_attach(d2, b), 0);
}

int test_main(void) { return pm_main(BASE, body); }
