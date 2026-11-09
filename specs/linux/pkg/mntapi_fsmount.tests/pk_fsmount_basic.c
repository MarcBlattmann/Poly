/* fsmount(fs_fd, flags, attr_flags) turns a created filesystem context into
 * a detached mount and returns a descriptor for its root directory: the
 * lowest free descriptor, with FD_CLOEXEC only when FSMOUNT_CLOEXEC (1) was
 * given. fstat on it shows the root directory (a tmpfs created with
 * mode=0711 gives 040711), fstatfs the filesystem. The mount is not part of
 * the namespace until move_mount attaches it: files can be made through
 * the descriptor with openat/mkdirat, fchdir into it works, and they appear
 * at the mount point after the attach and are the same files. Closing the
 * descriptor of a mount that was never attached drops the filesystem; the
 * context descriptor can be closed right after fsmount without
 * consequence for the mount. A second fsmount on the same context is EBUSY.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsmount_basic"

static void body(const char *base) {
    char p[128];
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("size", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "1m", 0), 0);
    expect("mode", pm_fsconfig(fc, FSCONFIG_SET_STRING, "mode", "0711", 0), 0);
    expect("create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    i64 m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("fsmount_cloexec", m >= 3 && m != fc, m);
    expect("cloexec_set", syscall2(SYS_fcntl, m, F_GETFD), FD_CLOEXEC);
    expect("second_fsmount", pm_fsmount(fc, FSMOUNT_CLOEXEC, 0), -EBUSY);
    expect("close_context", a_close(fc), 0);

    struct a_stat st;
    expect("fstat_root", syscall2(SYS_fstat, m, &st), 0);
    expect("root_is_directory_0711", st.st_mode, 040711);
    struct a_statfs sf;
    expect("fstatfs", pm_fd_statfs(m, &sf), 0);
    expect("fstatfs_tmpfs", sf.f_type, TMPFS_MAGIC);
    expect("fstatfs_blocks", (i64)sf.f_blocks, 256);

    /* files through the descriptor, before the mount is anywhere */
    i64 f = syscall4(SYS_openat, m, "made-through-fd", O_CREAT | O_WRONLY, 0644);
    check("openat_on_mount_fd", f >= 0, f);
    expect("write_data", a_write(f, "hello", 5), 5);
    a_close(f);
    expect("mkdirat_on_mount_fd", syscall3(SYS_mkdirat, m, "dir", 0755), 0);
    expect("fchdir_into_mount", syscall1(SYS_fchdir, m), 0);
    expect("relative_name_in_mount", pm_exists("made-through-fd"), 0);
    expect("back_to_root", syscall1(SYS_chdir, "/"), 0);

    /* attach it: the same files appear */
    pm_path(p, base, "m");
    expect("mkdir_m", pm_mkdir(p), 0);
    expect("nothing_there_yet", pm_exists(pm_path(p, base, "m/made-through-fd")), -ENOENT);
    expect("attach", pm_attach(m, pm_path(p, base, "m")), 0);
    expect("file_visible", pm_exists(pm_path(p, base, "m/made-through-fd")), 0);
    expect("dir_visible", pm_exists(pm_path(p, base, "m/dir")), 0);
    char buf[16];
    i64 rf = a_open(pm_path(p, base, "m/made-through-fd"), O_RDONLY, 0);
    i64 n = rf < 0 ? rf : a_read(rf, buf, sizeof buf);
    check("content", n == 5 && mem_eq(buf, "hello", 5), n);
    if (rf >= 0) a_close(rf);
    expect("statfs_at_mount_point", pm_statfs_blocks(pm_path(p, base, "m")), 256);
    expect("create_through_path", pm_touch(pm_path(p, base, "m/second")), 0);
    expect("visible_through_fd", syscall4(SYS_newfstatat, m, "second", &st, 0), 0);
    expect("visible_at_path", pm_exists(pm_path(p, base, "m/second")), 0);
    a_close(m);
    expect("mount_stays_after_close", pm_exists(pm_path(p, base, "m/second")), 0);
    expect("detach", pm_umount2(pm_path(p, base, "m"), 0), 0);
    expect("gone_after_umount", pm_exists(pm_path(p, base, "m/second")), -ENOENT);

    /* a mount that was never attached disappears with its descriptor */
    i64 m2 = pm_tmpfs_fd("1m", 0, FSMOUNT_CLOEXEC);
    check("second_mount", m2 >= 3, m2);
    expect("cloexec_set_again", syscall2(SYS_fcntl, m2, F_GETFD), FD_CLOEXEC);
    expect("close_unattached", a_close(m2), 0);
    i64 m3 = pm_tmpfs_fd("1m", 0, 0);
    check("fsmount_no_cloexec", m3 >= 3, m3);
    expect("no_cloexec", syscall2(SYS_fcntl, m3, F_GETFD), 0);
}

int test_main(void) { return pm_main(BASE, body); }
