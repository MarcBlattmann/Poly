/* Propagation types inside one mount namespace. A bind of a shared mount is a
 * peer of it: a mount or umount made below one appears below the other. After
 * MS_PRIVATE on the copy nothing propagates either way; after MS_SLAVE on the
 * copy it still receives events from the original but sends none back; MS_SLAVE
 * on a private mount changes nothing. MS_SHARED|MS_REC (and the other types
 * with MS_REC) apply to the submounts too; without MS_REC only the named
 * mount. MS_UNBINDABLE makes bind mounts of that mount EINVAL (until it is
 * made private again) and mounts below it do not propagate. A propagation
 * type needs a mount point (EINVAL otherwise), exactly one type per call
 * (EINVAL for two), and the calls ignore the filesystem type and source.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mpp"
#define T B "/t"
#define T2 B "/t2"

static int body(void) {
    expect("tmpfs", pm_tmpfs(T), 0);
    pm_mkdirs(T2);
    pm_mkdirs(B "/plain");
    pm_mkdirs(T "/m1");
    pm_mkdirs(T "/m2");
    pm_mkdirs(T "/m3");
    pm_mkdirs(T "/m4");
    pm_mkdirs(T "/sub");

    /* errors */
    expect("shared_not_mountpoint", pm_mount(0, B "/plain", 0, MS_SHARED, 0), -EINVAL);
    expect("private_not_mountpoint", pm_mount(0, B "/plain", 0, MS_PRIVATE, 0), -EINVAL);
    expect("slave_not_mountpoint", pm_mount(0, B "/plain", 0, MS_SLAVE, 0), -EINVAL);
    expect("two_types_einval", pm_mount(0, T, 0, MS_SHARED | MS_PRIVATE, 0), -EINVAL);
    expect("slave_and_shared_einval", pm_mount(0, T, 0, MS_SLAVE | MS_SHARED, 0), -EINVAL);
    expect("missing_target", pm_mount(0, B "/nothing", 0, MS_SHARED, 0), -ENOENT);
    expect("slave_on_private_ok", pm_mount(0, T, 0, MS_SLAVE, 0), 0);
    expect("type_ignores_source", pm_mount("whatever", T, "nofs", MS_PRIVATE, 0), 0);

    /* shared peers */
    expect("make_shared", pm_mount(0, T, 0, MS_SHARED, 0), 0);
    expect("bind_peer", pm_mount(T, T2, 0, MS_BIND, 0), 0);
    expect("mount_in_t", pm_mount("pk1", T "/m1", "tmpfs", 0, 0), 0);
    pm_put(T, "/m1/from_t", "t", 0644);
    pm_expect_file("seen_in_peer", T2 "/m1/from_t", "t");
    expect("mount_in_t2", pm_mount("pk2", T2 "/m2", "tmpfs", 0, 0), 0);
    pm_put(T2, "/m2/from_t2", "t2", 0644);
    pm_expect_file("seen_in_original", T "/m2/from_t2", "t2");
    expect("umount_in_t", pm_umount(T "/m1", 0), 0);
    expect("umount_seen_in_peer", pm_exists(T2 "/m1/from_t"), 0);
    expect("umount_in_t2", pm_umount(T2 "/m2", 0), 0);
    expect("umount_seen_in_original", pm_exists(T "/m2/from_t2"), 0);

    /* private copy */
    expect("peer_private", pm_mount(0, T2, 0, MS_PRIVATE, 0), 0);
    expect("mount_after_private", pm_mount("pk3", T "/m3", "tmpfs", 0, 0), 0);
    pm_put(T, "/m3/f", "3", 0644);
    expect("not_propagated_to_private", pm_exists(T2 "/m3/f"), 0);
    expect("mount_in_private", pm_mount("pk4", T2 "/m4", "tmpfs", 0, 0), 0);
    pm_put(T2, "/m4/f", "4", 0644);
    expect("private_sends_nothing", pm_exists(T "/m4/f"), 0);
    expect("umount_m4", pm_umount(T2 "/m4", 0), 0);
    expect("umount_m3", pm_umount(T "/m3", 0), 0);

    /* slave copy: receives, does not send */
    expect("remake_shared", pm_mount(0, T, 0, MS_SHARED, 0), 0);
    expect("umount_t2", pm_umount(T2, 0), 0);
    expect("rebind_peer", pm_mount(T, T2, 0, MS_BIND, 0), 0);
    expect("peer_slave", pm_mount(0, T2, 0, MS_SLAVE, 0), 0);
    expect("mount_for_slave", pm_mount("pk5", T "/m1", "tmpfs", 0, 0), 0);
    pm_put(T, "/m1/master", "m", 0644);
    pm_expect_file("slave_receives", T2 "/m1/master", "m");
    expect("mount_in_slave", pm_mount("pk6", T2 "/m2", "tmpfs", 0, 0), 0);
    pm_put(T2, "/m2/slave", "s", 0644);
    expect("slave_sends_nothing", pm_exists(T "/m2/slave"), 0);
    expect("umount_in_master", pm_umount(T "/m1", 0), 0);
    expect("slave_receives_umount", pm_exists(T2 "/m1/master"), 0);
    expect("umount_in_slave", pm_umount(T2 "/m2", 0), 0);
    expect("slave_umount_kept_master", pm_exists(T "/m2"), 1);

    /* MS_REC reaches submounts */
    expect("sub_mount", pm_mount("pksub", T "/sub", "tmpfs", 0, 0), 0);
    pm_mkdirs(T "/sub/inner");
    expect("umount_t2_for_rec", pm_umount(T2, MNT_DETACH), 0);
    expect("private_rec", pm_mount(0, T, 0, MS_PRIVATE | MS_REC, 0), 0);
    expect("shared_without_rec", pm_mount(0, T, 0, MS_SHARED, 0), 0);
    expect("bind_sub", pm_mount(T "/sub", B "/plain", 0, MS_BIND, 0), 0);
    expect("mount_below_copy_of_sub", pm_mount("pkx", B "/plain/inner", "tmpfs", 0, 0), 0);
    pm_put(B, "/plain/inner/x", "x", 0644);
    expect("sub_private_no_propagation", pm_exists(T "/sub/inner/x"), 0);
    expect("umount_inner", pm_umount(B "/plain/inner", 0), 0);
    expect("umount_plain_bind", pm_umount(B "/plain", 0), 0);
    expect("shared_rec", pm_mount(0, T, 0, MS_SHARED | MS_REC, 0), 0);
    expect("bind_sub_again", pm_mount(T "/sub", B "/plain", 0, MS_BIND, 0), 0);
    expect("mount_below_copy_again", pm_mount("pky", B "/plain/inner", "tmpfs", 0, 0), 0);
    pm_put(B, "/plain/inner/y", "y", 0644);
    pm_expect_file("sub_shared_propagates", T "/sub/inner/y", "y");
    expect("umount_inner_again", pm_umount(B "/plain/inner", 0), 0);
    expect("umount_plain_again", pm_umount(B "/plain", 0), 0);

    /* unbindable */
    expect("make_unbindable", pm_mount(0, T, 0, MS_UNBINDABLE, 0), 0);
    expect("bind_unbindable", pm_mount(T, T2, 0, MS_BIND, 0), -EINVAL);
    expect("bind_unbindable_rec", pm_mount(T, T2, 0, MS_BIND | MS_REC, 0), -EINVAL);
    expect("unbindable_sub_unaffected", pm_mount(T "/sub", T2, 0, MS_BIND, 0), 0);
    expect("umount_t2_unb", pm_umount(T2, 0), 0);
    expect("back_to_private", pm_mount(0, T, 0, MS_PRIVATE, 0), 0);
    expect("bind_private_ok", pm_mount(T, T2, 0, MS_BIND, 0), 0);
    expect("umount_t2_last", pm_umount(T2, 0), 0);
    expect("unbindable_rec", pm_mount(0, T, 0, MS_UNBINDABLE | MS_REC, 0), 0);
    expect("sub_unbindable_too", pm_mount(T "/sub", T2, 0, MS_BIND, 0), -EINVAL);
    expect("umount_sub", pm_umount(T "/sub", 0), 0);
    expect("umount_t", pm_umount(T, 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B "/t");
    a_rmdir(B);
    pm_in_ns(body);
    a_rmdir(B "/t2");
    a_rmdir(B "/plain");
    a_rmdir(B "/t");
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
