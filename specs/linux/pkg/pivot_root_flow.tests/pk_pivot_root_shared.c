/* pivot_root refuses (EINVAL) when the mount points involved are in a
 * shared peer group: new_root itself shared, put_old's mount shared, or the
 * parent mount of new_root shared. With "/" and everything below it made
 * private again, the same call works. Without CAP_SYS_ADMIN the call is
 * EPERM, before any name is looked at (a missing new_root is EPERM too).
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_pivot_root_shared"
#define NR "/tmp/pk_pivot_root_shared_n"

static i64 as_nobody_missing(void *arg) {
    (void)arg;
    return pm_pivot_root("/nonexistent-a", "/nonexistent-b");
}
static i64 as_nobody_real(void *arg) {
    const char *base = arg;
    char old[128];
    return pm_pivot_root(base, pm_path(old, base, "old"));
}

static void body(const char *base) {
    char old[128], sub[128];
    pm_path(old, base, "old");
    pm_path(sub, base, "sub");
    expect("mkdir_old", pm_mkdir(old), 0);
    expect("mkdir_sub", pm_mkdir(sub), 0);

    expect("eperm_missing_names", pm_as_nobody(as_nobody_missing, 0), -EPERM);
    expect("eperm_valid_names", pm_as_nobody(as_nobody_real, (void *)base), -EPERM);

    /* new_root's own mount is shared */
    expect("share_new_root", pm_mount(0, base, 0, MS_SHARED, 0), 0);
    expect("new_root_shared", pm_pivot_root(base, old), -EINVAL);
    expect("private_new_root", pm_mount(0, base, 0, MS_PRIVATE, 0), 0);

    /* put_old is a mount point and shared */
    expect("mount_sub", pm_mount("s", sub, "tmpfs", 0, 0), 0);
    expect("share_sub", pm_mount(0, sub, 0, MS_SHARED, 0), 0);
    expect("put_old_shared", pm_pivot_root(base, sub), -EINVAL);
    expect("private_sub", pm_mount(0, sub, 0, MS_PRIVATE, 0), 0);
    expect("umount_sub", pm_umount2(sub, 0), 0);

    /* the mount that new_root is attached to is shared */
    expect("share_everything", pm_mount(0, "/", 0, MS_REC | MS_SHARED, 0), 0);
    expect("mkdir_nr", pm_mkdir(NR), 0);
    expect("mount_nr", pm_mount("n", NR, "tmpfs", 0, 0), 0);
    expect("mkdir_nr_old", pm_mkdir(NR "/old"), 0);
    expect("parent_shared", pm_pivot_root(NR, NR "/old"), -EINVAL);
    expect("private_again", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
    expect("pivot_works_when_private", pm_pivot_root(NR, NR "/old"), 0);
    expect("old_root_below_put_old", pm_exists("/old/tmp"), 0);
}

int test_main(void) {
    int r = pm_main(BASE, body);
    a_rmdir(NR);
    return r;
}
