/* How bind mounts and per-mount flags show in /proc/self/mountinfo. A bind of
 * a subdirectory has that directory as root field ("/sub") and the same
 * device and superblock options as the original; options are rw|ro, then
 * nosuid, nodev, noexec, then the access time mode: "relatime" by default,
 * "noatime" and/or "nodiratime" when set, nothing for MS_STRICTATIME.
 * Per-mount flags are in field 6 and do not change the superblock's options.
 * A remount of the filesystem through one path (MS_REMOUNT without MS_BIND)
 * changes the superblock options of every mount of it ("ro,..."), the
 * per-mount options only of the mount it was done on (the other mounts keep
 * "rw" in field 6 while /proc/self/mounts shows "ro" for them). A mount made on a
 * directory where another mount already is gets that mount as parent; a
 * mount on a directory inside a mount has the containing mount as parent.
 * /proc/self/mounts shows the same options (mount options then superblock
 * options without repeating rw/ro).
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mib"
#define T B "/t"

static int body(void) {
    struct pm_mi t, b, c;
    pm_mkdirs(T);
    expect("mount_t", pm_mount("pkdev", T, "tmpfs", 0, "size=1m"), 0);
    pm_mkdirs(T "/sub");
    pm_mkdirs(T "/b1");
    pm_mkdirs(T "/b2");
    pm_mkdirs(T "/b3");
    pm_mkdirs(T "/b4");
    pm_mkdirs(T "/b5");
    pm_mkdirs(T "/sub/deeper");
    expect("bind1", pm_mount(T "/sub", T "/b1", 0, MS_BIND, 0), 0);
    expect("bind1_flags", pm_mount(0, T "/b1", 0, MS_REMOUNT | MS_BIND | MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC, 0), 0);
    expect("bind2", pm_mount(T "/sub", T "/b2", 0, MS_BIND, 0), 0);
    expect("bind2_flags", pm_mount(0, T "/b2", 0, MS_REMOUNT | MS_BIND | MS_NOATIME | MS_NODIRATIME, 0), 0);
    expect("bind3", pm_mount(T "/sub", T "/b3", 0, MS_BIND, 0), 0);
    expect("bind3_flags", pm_mount(0, T "/b3", 0, MS_REMOUNT | MS_BIND | MS_STRICTATIME, 0), 0);
    expect("bind4", pm_mount(T "/sub/deeper", T "/b4", 0, MS_BIND, 0), 0);
    expect("bind4_flags", pm_mount(0, T "/b4", 0, MS_REMOUNT | MS_BIND | MS_NOATIME, 0), 0);
    expect("bind5", pm_mount(T "/sub", T "/b5", 0, MS_BIND, 0), 0);
    expect("bind5_flags", pm_mount(0, T "/b5", 0, MS_REMOUNT | MS_BIND | MS_NODIRATIME | MS_NOSUID, 0), 0);

    pm_mi_load("/proc/self/mountinfo");
    expect("t_listed", pm_mi_get(T, -1, &t), 1);
    expect("t_options", str_eq(t.opts, "rw,relatime"), 1);
    expect("t_root", str_eq(t.root, "/"), 1);

    expect("b1_listed", pm_mi_get(T "/b1", -1, &b), 1);
    expect("b1_root_is_subdir", str_eq(b.root, "/sub"), 1);
    expect("b1_options", str_eq(b.opts, "ro,nosuid,nodev,noexec,relatime"), 1);
    expect("b1_same_dev", str_eq(b.dev, t.dev), 1);
    expect("b1_same_source", str_eq(b.source, t.source), 1);
    expect("b1_same_super", str_eq(b.super, t.super), 1);
    expect("b1_super_still_rw", pm_starts(b.super, "rw,"), 1);
    expect("b1_fstype", str_eq(b.fstype, "tmpfs"), 1);
    expect("b1_parent_is_t", str_eq(b.parent, t.id), 1);
    expect("b1_own_id", str_eq(b.id, t.id), 0);

    expect("b2_listed", pm_mi_get(T "/b2", -1, &b), 1);
    expect("b2_options", str_eq(b.opts, "rw,noatime,nodiratime"), 1);
    expect("b3_listed", pm_mi_get(T "/b3", -1, &b), 1);
    expect("b3_options", str_eq(b.opts, "rw"), 1);
    expect("b4_listed", pm_mi_get(T "/b4", -1, &b), 1);
    expect("b4_root_nested", str_eq(b.root, "/sub/deeper"), 1);
    expect("b4_options", str_eq(b.opts, "rw,noatime"), 1);
    expect("b5_listed", pm_mi_get(T "/b5", -1, &b), 1);
    expect("b5_options", str_eq(b.opts, "rw,nosuid,nodiratime,relatime"), 1);

    /* mounts file: same options, mount flags first */
    pm_mi_load("/proc/self/mounts");
    struct pm_mounts_line ml;
    expect("mounts_b1", pm_mounts_get(T "/b1", -1, &ml), 1);
    expect("mounts_b1_opts_start", pm_starts(ml.opts, "ro,nosuid,nodev,noexec,relatime,"), 1);
    expect("mounts_b1_super", pm_has_item(ml.opts, "size=1024k"), 1);
    expect("mounts_b2", pm_mounts_get(T "/b2", -1, &ml), 1);
    expect("mounts_b2_opts_start", pm_starts(ml.opts, "rw,noatime,nodiratime,"), 1);
    expect("mounts_b3", pm_mounts_get(T "/b3", -1, &ml), 1);
    expect("mounts_b3_opts_start", pm_starts(ml.opts, "rw,size=1024k"), 1);

    /* a mount over a mount: the new one is the child of the covered one */
    pm_mi_load("/proc/self/mountinfo");
    pm_mi_get(T "/b1", -1, &b);
    expect("cover", pm_mount("pkcover", T "/b1", "tmpfs", 0, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    expect("covered_listed", pm_mi_get(T "/b1", 0, &b), 1);
    expect("cover_listed", pm_mi_get(T "/b1", -1, &c), 1);
    expect("cover_is_later_line", str_eq(b.id, c.id), 0);
    expect("cover_parent_is_covered", str_eq(c.parent, b.id), 1);
    expect("cover_root", str_eq(c.root, "/"), 1);
    expect("cover_options", str_eq(c.opts, "rw,relatime"), 1);
    expect("cover_source", str_eq(c.source, "pkcover"), 1);
    expect("uncover", pm_umount(T "/b1", 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    expect("uncovered_one_left", pm_mi_get(T "/b1", -1, &c), 1);
    expect("uncovered_is_the_bind", str_eq(c.id, b.id), 1);

    /* a mount inside a mount has the containing mount as parent */
    expect("inner_mount", pm_mount("pkinner", T "/b2", "tmpfs", 0, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    pm_mi_get(T "/b2", -1, &c);
    pm_mi_get(T "/b2", 0, &b);
    expect("inner_parent", str_eq(c.parent, b.id), 1);
    pm_mkdirs(T "/b2/in");
    expect("nested_mount", pm_mount("pknested", T "/b2/in", "tmpfs", 0, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    expect("nested_listed", pm_mi_get(T "/b2/in", -1, &b), 1);
    pm_mi_get(T "/b2", -1, &c);
    expect("nested_parent", str_eq(b.parent, c.id), 1);

    /* remounting the filesystem changes the superblock options of all binds */
    expect("umount_nested", pm_umount(T "/b2/in", 0), 0);
    expect("umount_inner", pm_umount(T "/b2", 0), 0);
    expect("remount_fs_ro", pm_mount(0, T "/b5", 0, MS_REMOUNT | MS_RDONLY, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    pm_mi_get(T, -1, &t);
    expect("t_super_ro", pm_starts(t.super, "ro,"), 1);
    expect("t_options_unchanged", str_eq(t.opts, "rw,relatime"), 1);
    pm_mi_get(T "/b3", -1, &b);
    expect("b3_super_ro", pm_starts(b.super, "ro,"), 1);
    expect("b3_options_unchanged", str_eq(b.opts, "rw"), 1);
    pm_mi_get(T "/b5", -1, &b);
    expect("b5_options_ro", pm_starts(b.opts, "ro,"), 1);
    expect("b5_super_ro", pm_starts(b.super, "ro,"), 1);
    pm_mi_load("/proc/self/mounts");
    struct pm_mounts_line ml2;
    expect("mounts_t_ro", pm_mounts_get(T, -1, &ml2), 1);
    expect("mounts_t_ro_opts", pm_starts(ml2.opts, "ro,relatime,"), 1);
    pm_mi_load("/proc/self/mountinfo");
    expect("remount_fs_rw", pm_mount(0, T "/b5", 0, MS_REMOUNT, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    pm_mi_get(T "/b3", -1, &b);
    expect("b3_super_rw_again", pm_starts(b.super, "rw,"), 1);

    /* sync is a superblock option */
    expect("mount_sync", pm_mount("pksync", T "/b4", "tmpfs", MS_SYNCHRONOUS | MS_NOSUID, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    pm_mi_get(T "/b4", -1, &b);
    expect("sync_options", str_eq(b.opts, "rw,nosuid,relatime"), 1);
    expect("sync_super", pm_has_item(b.super, "sync"), 1);
    expect("sync_super_rw", pm_starts(b.super, "rw"), 1);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(T);
    a_rmdir(B);
    pm_in_ns(body);
    a_rmdir(T);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
