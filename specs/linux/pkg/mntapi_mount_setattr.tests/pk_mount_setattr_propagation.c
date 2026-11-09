/* The propagation field of struct mount_attr sets the propagation type of
 * the mount like mount(2) with MS_SHARED (1<<20), MS_SLAVE (1<<19),
 * MS_PRIVATE (1<<18) or MS_UNBINDABLE (1<<17); 0 changes nothing; two types
 * together, or any other value, are EINVAL; it can be combined with
 * attr_set/attr_clr in one call and with AT_RECURSIVE.
 * Effects seen here: a bind mount of a shared mount joins its peer group, so
 * a mount made below one of the two shows up below the other; after
 * MS_SLAVE the bind mount still receives mounts made below the master but
 * not the other way round; after MS_PRIVATE it receives none; and an
 * unbindable mount cannot be bind mounted (EINVAL). A bind mount of a
 * private mount is not a peer.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_mount_setattr_propagation"

static i64 prop(const char *path, u64 flags, u64 p) {
    struct pm_mount_attr a = {0, 0, p, 0};
    return pm_mount_setattr(AT_FDCWD, path, flags, &a, sizeof a);
}

static void body(const char *base) {
    char x[128], y[128], z[128], q[200], r[200];
    pm_path(x, base, "x");
    pm_path(y, base, "y");
    pm_path(z, base, "z");
    expect("mkdir_x", pm_mkdir(x), 0);
    expect("mkdir_y", pm_mkdir(y), 0);
    expect("mkdir_z", pm_mkdir(z), 0);
    expect("mount_x", pm_mount("x", x, "tmpfs", 0, "size=1m"), 0);
    expect("dirs_in_x", pm_mkdir(pm_path(q, x, "m1")) | pm_mkdir(pm_path(q, x, "m2")) | pm_mkdir(pm_path(q, x, "m3")), 0);

    /* accepted values and bad ones */
    expect("shared", prop(x, 0, MS_SHARED), 0);
    expect("slave", prop(x, 0, MS_SLAVE), 0);
    expect("private", prop(x, 0, MS_PRIVATE), 0);
    expect("unbindable", prop(x, 0, MS_UNBINDABLE), 0);
    expect("zero_changes_nothing", prop(x, 0, 0), 0);
    expect("shared_and_slave", prop(x, 0, MS_SHARED | MS_SLAVE), -EINVAL);
    expect("private_and_unbindable", prop(x, 0, MS_PRIVATE | MS_UNBINDABLE), -EINVAL);
    expect("value_1", prop(x, 0, 1), -EINVAL);
    expect("value_rec", prop(x, 0, MS_REC), -EINVAL);
    expect("value_bind", prop(x, 0, MS_BIND), -EINVAL);
    expect("value_shared_with_extra", prop(x, 0, MS_SHARED | 1), -EINVAL);
    expect("recursive_private", prop(x, AT_RECURSIVE, MS_PRIVATE), 0);

    /* unbindable cannot be bound */
    expect("make_unbindable", prop(x, 0, MS_UNBINDABLE), 0);
    expect("bind_of_unbindable", pm_mount(x, y, 0, MS_BIND, 0), -EINVAL);
    expect("open_tree_clone_of_unbindable", pm_open_tree(AT_FDCWD, x, OPEN_TREE_CLONE), -EINVAL);

    /* a private mount's bind mount is not a peer */
    expect("make_private", prop(x, 0, MS_PRIVATE), 0);
    expect("bind_private", pm_mount(x, y, 0, MS_BIND, 0), 0);
    expect("mount_in_x", pm_mount("m", pm_path(q, x, "m1"), "tmpfs", 0, 0), 0);
    expect("file_below_private_x", pm_touch(pm_path(q, x, "m1/f")), 0);
    expect("not_seen_below_y", pm_exists(pm_path(r, y, "m1/f")), -ENOENT);
    expect("umount_y", pm_umount2(y, 0), 0);
    expect("umount_m1", pm_umount2(pm_path(q, x, "m1"), 0), 0);

    /* shared: peers */
    expect("make_shared", prop(x, 0, MS_SHARED), 0);
    expect("bind_shared", pm_mount(x, y, 0, MS_BIND, 0), 0);
    expect("mount_below_x", pm_mount("m", pm_path(q, x, "m1"), "tmpfs", 0, 0), 0);
    expect("file_below_x", pm_touch(pm_path(q, x, "m1/f")), 0);
    expect("seen_below_y", pm_exists(pm_path(r, y, "m1/f")), 0);
    expect("mount_below_y", pm_mount("m", pm_path(q, y, "m2"), "tmpfs", 0, 0), 0);
    expect("file_below_y", pm_touch(pm_path(q, y, "m2/g")), 0);
    expect("seen_below_x", pm_exists(pm_path(r, x, "m2/g")), 0);

    /* slave: y receives, x does not */
    expect("y_becomes_slave", prop(y, 0, MS_SLAVE), 0);
    expect("mount_below_x_again", pm_mount("m", pm_path(q, x, "m3"), "tmpfs", 0, 0), 0);
    expect("file_below_x_again", pm_touch(pm_path(q, x, "m3/h")), 0);
    expect("slave_receives", pm_exists(pm_path(r, y, "m3/h")), 0);
    expect("mkdir_n_in_y", pm_mkdir(pm_path(q, y, "n")), 0);
    expect("mount_below_y_slave", pm_mount("m", pm_path(q, y, "n"), "tmpfs", 0, 0), 0);
    expect("file_below_y_slave", pm_touch(pm_path(q, y, "n/k")), 0);
    expect("master_does_not_receive", pm_exists(pm_path(r, x, "n/k")), -ENOENT);

    /* private: y receives nothing new */
    expect("y_becomes_private", prop(y, 0, MS_PRIVATE), 0);
    expect("mkdir_p_in_x", pm_mkdir(pm_path(q, x, "p")), 0);
    expect("mount_below_x_third", pm_mount("m", pm_path(q, x, "p"), "tmpfs", 0, 0), 0);
    expect("file_below_x_third", pm_touch(pm_path(q, x, "p/i")), 0);
    expect("private_receives_no_more", pm_exists(pm_path(r, y, "p/i")), -ENOENT);
    expect("empty_dir_arrived_with_files_of_x", pm_exists(pm_path(r, y, "p")), 0);

    /* recursive on a tree, together with other attributes */
    struct pm_mount_attr a = {MOUNT_ATTR_NODEV, 0, MS_PRIVATE, 0};
    expect("recursive_with_flags", pm_mount_setattr(AT_FDCWD, x, AT_RECURSIVE, &a, sizeof a), 0);
    expect("flags_applied", pm_mntflags(x) & ST_NODEV, ST_NODEV);
    expect("flags_applied_below", pm_mntflags(pm_path(q, x, "m1")) & ST_NODEV, ST_NODEV);
    expect("bind_of_private_child", pm_mount(pm_path(q, x, "m1"), z, 0, MS_BIND, 0), 0);
}

int test_main(void) { return pm_main(BASE, body); }
