/* mount("overlay") errors, one per cause: no lowerdir= (or no option string
 * at all) EINVAL; a lowerdir, upperdir or workdir that does not exist ENOENT;
 * a lowerdir that is not a directory EINVAL; upperdir without workdir EINVAL;
 * workdir inside the upperdir or the upperdir inside the workdir, or both the
 * same directory, EINVAL; upperdir and workdir on different filesystems
 * EINVAL; an unknown option or a bad option value EINVAL; the same directory
 * as upper and lower, a lower inside the upper, or the same lower twice
 * ELOOP; a target that is a file ENOTDIR, a missing target ENOENT. A
 * workdir without an upperdir is accepted (ignored). Nothing is mounted after
 * a failure.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovm_err"
#define L1 B "/l1"
#define L2 B "/l2"
#define U B "/u"
#define W B "/w"
#define M B "/m"

static i64 ov(const char *data) { return pm_mount("ov", M, "overlay", 0, data); }

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L1);
    pm_mkdirs(L2);
    pm_mkdirs(U);
    pm_mkdirs(W);
    pm_mkdirs(M);
    pm_mkdirs(U "/sub");
    pm_mkdirs(B "/other");
    pm_put(B, "/l1/file", "f", 0644);
    expect("tmpfs_other", pm_mount("pkother", B "/other", "tmpfs", 0, 0), 0);
    pm_mkdirs(B "/other/w");

    expect("no_data", pm_mount("ov", M, "overlay", 0, 0), -EINVAL);
    expect("empty_data", ov(""), -EINVAL);
    expect("upper_work_without_lower", ov("upperdir=" U ",workdir=" W), -EINVAL);
    expect("missing_lower", ov("lowerdir=" B "/nolower,upperdir=" U ",workdir=" W), -ENOENT);
    expect("missing_upper", ov("lowerdir=" L1 ",upperdir=" B "/noupper,workdir=" W), -ENOENT);
    expect("missing_work", ov("lowerdir=" L1 ",upperdir=" U ",workdir=" B "/nowork"), -ENOENT);
    expect("lower_is_file", ov("lowerdir=" L1 "/file,upperdir=" U ",workdir=" W), -EINVAL);
    expect("upper_without_work", ov("lowerdir=" L1 ",upperdir=" U), -EINVAL);
    expect("work_inside_upper", ov("lowerdir=" L1 ",upperdir=" U ",workdir=" U "/sub"), -EINVAL);
    expect("upper_inside_work", ov("lowerdir=" L1 ",upperdir=" U "/sub,workdir=" U), -EINVAL);
    expect("work_is_upper", ov("lowerdir=" L1 ",upperdir=" U ",workdir=" U), -EINVAL);
    expect("work_other_filesystem", ov("lowerdir=" L1 ",upperdir=" U ",workdir=" B "/other/w"), -EINVAL);
    expect("unknown_option", ov("lowerdir=" L1 ",upperdir=" U ",workdir=" W ",bogus=1"), -EINVAL);
    expect("bad_option_value", ov("lowerdir=" L1 ",upperdir=" U ",workdir=" W ",redirect_dir=maybe"), -EINVAL);
    expect("upper_is_lower", ov("lowerdir=" U ",upperdir=" U ",workdir=" W), -ELOOP);
    expect("lower_inside_upper", ov("lowerdir=" U "/sub,upperdir=" U ",workdir=" W), -ELOOP);
    expect("same_lower_twice", ov("lowerdir=" L1 ":" L2 ":" L1), -ELOOP);
    expect("target_is_file", pm_mount("ov", L1 "/file", "overlay", 0,
                                     "lowerdir=" L1 ",upperdir=" U ",workdir=" W), -ENOTDIR);
    expect("target_missing", pm_mount("ov", B "/nothing", "overlay", 0,
                                     "lowerdir=" L1 ",upperdir=" U ",workdir=" W), -ENOENT);
    pm_expect_list("nothing_mounted", M, "");
    struct a_statfs fs;
    expect("statfs_target", syscall2(SYS_statfs, M, &fs), 0);
    expect("target_still_tmpfs", fs.f_type, TMPFS_MAGIC);

    /* accepted: workdir without upperdir is ignored; a single lowerdir with an upperdir */
    expect("work_only_ignored", ov("lowerdir=" L1 ":" L2 ",workdir=" W), 0);
    expect("work_only_overlay", syscall2(SYS_statfs, M, &fs), 0);
    expect("work_only_type", fs.f_type, OVERLAYFS_SUPER_MAGIC);
    expect("work_only_umount", pm_umount(M, 0), 0);
    expect("upper_on_other_fs_than_lower",
           ov("lowerdir=" B "/other,upperdir=" U ",workdir=" W), 0);
    expect("umount2", pm_umount(M, 0), 0);
    expect("umount_other", pm_umount(B "/other", 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
