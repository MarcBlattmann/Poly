/* A trailing slash makes the last component name a directory, so a
 * symbolic link there is followed after all: "link/" acts on the
 * directory the link points to, not on the link. A link to a file with a
 * trailing slash is ENOTDIR, a dangling one ENOENT, a self loop ELOOP. A
 * link to "/" with a slash acts on "/", which belongs to root: a
 * caller that is not root gets EPERM for its own uid there. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_trailing_slash"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static void nap(void) {
    struct a_timespec t = {0, 30000000};
    syscall2(SYS_nanosleep, &t, 0);
}
static int same_ctime(const struct a_stat *a, const struct a_stat *b) {
    return a->st_ctime_sec == b->st_ctime_sec && a->st_ctime_nsec == b->st_ctime_nsec;
}

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    struct a_stat d0, d1, l0, l1;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "dir", AT_FDCWD, D "/dl"), 0);
    expect("setup_filelink", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/fl"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "gone", AT_FDCWD, D "/gone_l"), 0);
    expect("setup_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    expect("setup_rootlink", syscall3(SYS_symlinkat, "/", AT_FDCWD, D "/rl"), 0);

    a_stat_path(D "/dir", &d0);
    a_lstat_path(D "/dl", &l0);
    nap();
    expect("dirlink_slash", lchown(D "/dl/", uid, gid), 0);
    a_stat_path(D "/dir", &d1);
    a_lstat_path(D "/dl", &l1);
    check("dirlink_slash_dir_changed", !same_ctime(&d0, &d1), 0);
    check("dirlink_slash_link_untouched", same_ctime(&l0, &l1), 0);
    nap();
    expect("dirlink_no_slash", lchown(D "/dl", uid, gid), 0);
    a_stat_path(D "/dir", &d0);
    a_lstat_path(D "/dl", &l0);
    check("dirlink_no_slash_dir_untouched", same_ctime(&d0, &d1), 0);
    check("dirlink_no_slash_link_changed", !same_ctime(&l0, &l1), 0);
    expect("dirlink_slash_dot", lchown(D "/dl/.", -1, -1), 0);
    expect("dir_slash", lchown(D "/dir/", -1, -1), 0);
    expect("dir_slashes", lchown(D "/dir//", -1, -1), 0);

    expect("file_slash_enotdir", lchown(D "/f/", -1, -1), -ENOTDIR);
    expect("filelink_slash_enotdir", lchown(D "/fl/", -1, -1), -ENOTDIR);
    expect("dangling_slash_enoent", lchown(D "/gone_l/", -1, -1), -ENOENT);
    expect("missing_slash_enoent", lchown(D "/missing/", -1, -1), -ENOENT);
    expect("loop_slash_eloop", lchown(D "/loop/", -1, -1), -ELOOP);
    expect("loop_no_slash_ok", lchown(D "/loop", -1, -1), 0);

    /* "rl" is the caller's own link; "rl/" is "/". */
    expect("rootlink_itself", lchown(D "/rl", uid, gid), 0);
    expect("rootlink_slash_minus1", lchown(D "/rl/", -1, -1), 0);
    if (root)
        check("rootlink_slash_own_uid", 1, 0);
    else
        expect("rootlink_slash_own_uid", lchown(D "/rl/", uid, -1), -EPERM);

    expect("cleanup_rl", a_unlink(D "/rl"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_gone_l", a_unlink(D "/gone_l"), 0);
    expect("cleanup_fl", a_unlink(D "/fl"), 0);
    expect("cleanup_dl", a_unlink(D "/dl"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
