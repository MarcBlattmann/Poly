/* fsopen/fsconfig/fsmount/move_mount for filesystems other than a plain
 * tmpfs, and stacking of detached mounts.
 * proc: no parameters needed; the mount shows /proc-like entries (self,
 * the caller's pid directory).
 * overlay: "lowerdir" is a colon separated list, the first entry on top;
 * with two lower directories and no upper the mount is read-only (creating
 * a file is EROFS) and shows the union (a name in both: the first lower directory
 * wins); with "upperdir" and "workdir" (SET_STRING) writes go to the upper
 * directory only.
 * Two mounts attached to the same directory stack: the later one hides the
 * earlier, umount2 of the top shows the earlier again.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsmount_types"

static i64 mount_ctx(i64 fc, u64 attrs) {
    i64 r = pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
    if (r < 0) return r;
    return pm_fsmount(fc, FSMOUNT_CLOEXEC, attrs);
}

static void body(const char *base) {
    char p[128], lo1[128], lo2[128], up[128], wk[128], mp[128], q[160];

    /* proc */
    i64 fc = pm_fsopen("proc", FSOPEN_CLOEXEC);
    check("fsopen_proc", fc >= 0, fc);
    i64 m = mount_ctx(fc, MOUNT_ATTR_NOSUID | MOUNT_ATTR_NODEV | MOUNT_ATTR_NOEXEC);
    check("fsmount_proc", m >= 0, m);
    pm_path(p, base, "proc");
    expect("mkdir_proc", pm_mkdir(p), 0);
    expect("attach_proc", pm_attach(m, p), 0);
    expect("proc_self", pm_exists(pm_path(q, p, "self")), 0);
    expect("proc_self_status", pm_exists(pm_path(q, p, "self/status")), 0);
    struct a_statfs sf;
    expect("statfs_proc", syscall2(SYS_statfs, p, &sf), 0);
    expect("proc_magic", sf.f_type, 0x9fa0);
    a_close(m);
    a_close(fc);

    /* overlay: lower directories only */
    pm_path(lo1, base, "lo1");
    pm_path(lo2, base, "lo2");
    pm_path(up, base, "up");
    pm_path(wk, base, "wk");
    pm_path(mp, base, "ov");
    expect("mkdir_lo1", pm_mkdir(lo1), 0);
    expect("mkdir_lo2", pm_mkdir(lo2), 0);
    expect("mkdir_up", pm_mkdir(up), 0);
    expect("mkdir_wk", pm_mkdir(wk), 0);
    expect("mkdir_ov", pm_mkdir(mp), 0);
    expect("only_in_lo1", a_make_file(pm_path(q, lo1, "a"), "from lo1", 0644), 0);
    expect("in_both_lo1", a_make_file(pm_path(q, lo1, "both"), "one", 0644), 0);
    expect("only_in_lo2", a_make_file(pm_path(q, lo2, "b"), "from lo2", 0644), 0);
    expect("in_both_lo2", a_make_file(pm_path(q, lo2, "both"), "two", 0644), 0);
    char lower[300];
    str_cat2(lower, lo1, ":");
    str_cpy(lower + str_len(lower), lo2);
    fc = pm_fsopen("overlay", FSOPEN_CLOEXEC);
    check("fsopen_overlay", fc >= 0, fc);
    expect("set_lowerdir", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", lower, 0), 0);
    m = mount_ctx(fc, 0);
    check("fsmount_overlay", m >= 0, m);
    expect("attach_overlay", pm_attach(m, mp), 0);
    expect("union_a", pm_exists(pm_path(q, mp, "a")), 0);
    expect("union_b", pm_exists(pm_path(q, mp, "b")), 0);
    char buf[16];
    i64 f = a_open(pm_path(q, mp, "both"), O_RDONLY, 0);
    i64 n = f < 0 ? f : a_read(f, buf, sizeof buf);
    check("first_lower_wins", n == 3 && mem_eq(buf, "one", 3), n);
    if (f >= 0) a_close(f);
    expect("overlay_without_upper_is_read_only", pm_can_create(mp), -EROFS);
    expect("overlay_magic", syscall2(SYS_statfs, mp, &sf), 0);
    expect("overlay_f_type", sf.f_type, 0x794c7630);
    a_close(m);
    a_close(fc);
    expect("detach_overlay", pm_umount2(mp, 0), 0);

    /* overlay with upper and work directories */
    fc = pm_fsopen("overlay", FSOPEN_CLOEXEC);
    expect("set_lowerdir2", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", lower, 0), 0);
    expect("set_upperdir", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", up, 0), 0);
    expect("set_workdir", pm_fsconfig(fc, FSCONFIG_SET_STRING, "workdir", wk, 0), 0);
    m = mount_ctx(fc, 0);
    check("fsmount_overlay_rw", m >= 0, m);
    expect("attach_overlay_rw", pm_attach(m, mp), 0);
    expect("create_in_overlay", a_make_file(pm_path(q, mp, "new"), "n", 0644), 0);
    expect("new_file_in_upper", pm_exists(pm_path(q, up, "new")), 0);
    expect("new_file_not_in_lower", pm_exists(pm_path(q, lo1, "new")), -ENOENT);
    a_close(m);
    a_close(fc);
    expect("detach_overlay_rw", pm_umount2(mp, 0), 0);

    /* stacking */
    pm_path(p, base, "st");
    expect("mkdir_st", pm_mkdir(p), 0);
    i64 m1 = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    expect("attach_first", pm_attach(m1, p), 0);
    expect("file_in_first", pm_touch(pm_path(q, p, "first")), 0);
    i64 m2 = pm_tmpfs_fd("2m", 0, FSMOUNT_CLOEXEC);
    expect("attach_second", pm_attach(m2, p), 0);
    expect("second_hides_first", pm_exists(pm_path(q, p, "first")), -ENOENT);
    expect("top_is_second", pm_statfs_blocks(p), 512);
    expect("umount_top", pm_umount2(p, 0), -EBUSY);
    a_close(m2);
    expect("umount_top_after_close", pm_umount2(p, 0), 0);
    expect("first_visible_again", pm_exists(pm_path(q, p, "first")), 0);
    expect("top_is_first", pm_statfs_blocks(p), 256);
}

int test_main(void) { return pm_main(BASE, body); }
