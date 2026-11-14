/* mount(2) of "fuse": which complaint comes first, and what a failed mount
 * leaves behind. Options that are wrong in themselves (a descriptor that is
 * not an open /dev/fuse, a missing or non-numeric fd=, a rootmode that is not
 * an octal file type, user_id= or group_id= missing or not decimal) are EINVAL
 * before the fit of the root's type to the mount point is judged, so a root
 * that would be ENOTDIR over this mount point is EINVAL when anything else is
 * wrong. A flag option (default_permissions, allow_other) given a value, even
 * 0 or an empty one, is EINVAL; so is an unknown option. A mount that fails
 * with EINVAL leaves the /dev/fuse descriptor free for a good mount; one that
 * fails with ENOTDIR does not: the descriptor stays tied to the failed
 * mount, and a new mount with it is ENOTCONN (-107), also on a mount point
 * where the root would fit. Every other check uses a new descriptor.
 * (mount(8) fuse options, linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off (the descriptor numbers and option strings vary)
 */
#include "p_fuse.h"

#define D "/tmp/pk_fuse_mord"
#define F "/tmp/pk_fuse_mord_file"
#define ENOTCONN_ 107

static i64 mnt_fd(i64 fd, const char *target, const char *rest) {
    char opts[200];
    char *p = str_cpy(opts, "fd=");
    a_utoa(p, (u64)fd);
    p += str_len(p);
    str_cpy(p, rest);
    return syscall5(SYS_mount, "pk", target, "fuse", MS_NOSUID | MS_NODEV, opts);
}

/* the whole option string is given as it is */
static i64 try_all(const char *target, const char *all) {
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    char opts[200];
    str_cpy(opts, all);
    i64 r = syscall5(SYS_mount, "pk", target, "fuse", MS_NOSUID | MS_NODEV, opts);
    if (r == 0) syscall2(SYS_umount2, target, MNT_DETACH);
    a_close(fd);
    return r;
}

/* a new descriptor and the given options after fd= */
static i64 try_rest(const char *target, const char *rest) {
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    i64 r = mnt_fd(fd, target, rest);
    if (r == 0) syscall2(SYS_umount2, target, MNT_DETACH);
    a_close(fd);
    return r;
}

#define IDS ",user_id=0,group_id=0"

int test_main(void) {
    expect("setup", fz_setup(D), 0);
    a_unlink(F);
    expect("make_file", a_make_file(F, "x", 0644), 0);

    /* wrong in itself, before the type fits */
    expect("closed_fd", try_all(D, "fd=999,rootmode=40000" IDS), -EINVAL);
    expect("closed_fd_regular_over_dir", try_all(D, "fd=999,rootmode=100000" IDS), -EINVAL);
    expect("fd_text_regular_over_dir", try_all(D, "fd=abc,rootmode=100000" IDS), -EINVAL);
    expect("fd_empty_regular_over_dir", try_all(D, "fd=,rootmode=100000" IDS), -EINVAL);
    expect("fd_missing_regular_over_dir", try_all(D, "rootmode=100000" IDS), -EINVAL);
    expect("fd_stdin_regular_over_dir", try_all(D, "fd=0,rootmode=100000" IDS), -EINVAL);
    expect("fd_stdin_dir_over_file", try_all(F, "fd=0,rootmode=40000" IDS), -EINVAL);
    expect("closed_fd_dir_over_file", try_all(F, "fd=999,rootmode=40000" IDS), -EINVAL);
    expect("bad_user_regular_over_dir", try_rest(D, ",rootmode=100000,user_id=x,group_id=0"), -EINVAL);
    expect("missing_group_regular_over_dir", try_rest(D, ",rootmode=100000,user_id=0"), -EINVAL);
    expect("missing_user_dir_over_file", try_rest(F, ",rootmode=40000,group_id=0"), -EINVAL);
    expect("missing_rootmode_over_dir", try_rest(D, IDS), -EINVAL);
    expect("rootmode_zero_over_dir", try_rest(D, ",rootmode=0" IDS), -EINVAL);

    /* the options as such */
    expect("plain", try_rest(D, ",rootmode=40000" IDS), 0);
    expect("default_permissions", try_rest(D, ",rootmode=40000" IDS ",default_permissions"), 0);
    expect("allow_other", try_rest(D, ",rootmode=40000" IDS ",allow_other"), 0);
    expect("both_flags", try_rest(D, ",rootmode=40000" IDS ",allow_other,default_permissions"), 0);
    expect("default_permissions_1", try_rest(D, ",rootmode=40000" IDS ",default_permissions=1"), -EINVAL);
    expect("default_permissions_0", try_rest(D, ",rootmode=40000" IDS ",default_permissions=0"), -EINVAL);
    expect("default_permissions_empty", try_rest(D, ",rootmode=40000" IDS ",default_permissions="), -EINVAL);
    expect("allow_other_x", try_rest(D, ",rootmode=40000" IDS ",allow_other=x"), -EINVAL);
    expect("allow_other_1", try_rest(D, ",rootmode=40000" IDS ",allow_other=1"), -EINVAL);
    expect("allow_other_empty", try_rest(D, ",rootmode=40000" IDS ",allow_other="), -EINVAL);
    expect("unknown_flag", try_rest(D, ",rootmode=40000" IDS ",nosuchoption"), -EINVAL);
    expect("unknown_value", try_rest(D, ",rootmode=40000" IDS ",nosuchoption=1"), -EINVAL);

    /* what a failed mount leaves */
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    expect("einval_mount", mnt_fd(fd, D, ",rootmode=0" IDS), -EINVAL);
    expect("then_good_with_same_fd", mnt_fd(fd, D, ",rootmode=40000" IDS), 0);
    syscall2(SYS_umount2, D, MNT_DETACH);
    a_close(fd);

    fd = a_open("/dev/fuse", O_RDWR, 0);
    expect("enotdir_mount", mnt_fd(fd, D, ",rootmode=100000" IDS), -ENOTDIR);
    expect("then_good_same_fd_is_enotconn", mnt_fd(fd, D, ",rootmode=40000" IDS), -ENOTCONN_);
    expect("then_good_other_mount_point", mnt_fd(fd, F, ",rootmode=100000" IDS), -ENOTCONN_);
    a_close(fd);

    fd = a_open("/dev/fuse", O_RDWR, 0);
    expect("enotdir_mount_file", mnt_fd(fd, F, ",rootmode=40000" IDS), -ENOTDIR);
    expect("then_good_file_same_fd", mnt_fd(fd, F, ",rootmode=100000" IDS), -ENOTCONN_);
    a_close(fd);

    fd = a_open("/dev/fuse", O_RDWR, 0);
    expect("einval_bad_user", mnt_fd(fd, D, ",rootmode=40000,user_id=x,group_id=0"), -EINVAL);
    expect("then_good_after_bad_user", mnt_fd(fd, D, ",rootmode=40000" IDS), 0);
    syscall2(SYS_umount2, D, MNT_DETACH);
    a_close(fd);

    a_unlink(F);
    fz_cleanup();
    return finish();
}
