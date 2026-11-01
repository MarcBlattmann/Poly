/* renameat(2): the order in which problems of the two (path, dirfd) pairs
 * are reported: oldpath's string, olddirfd and directories; newpath's
 * string, newdirfd and directories; then oldpath's last name, newpath's
 * last name; then the rename itself. */
#include "a_sys.h"

#define D "/tmp/lb_renameat_order"

static i64 renameat_(i64 ofd, const char *a, i64 nfd, const char *b) { return syscall4(SYS_renameat, ofd, a, nfd, b); }

static char lng[512], lng2[512], huge[4200];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    i64 d = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 file = a_open(D "/f", O_RDONLY, 0);
    check("opened", d >= 0 && file >= 0, file);
    char *p = str_cpy(lng, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'q';
    p[256] = 0;
    for (int i = 0; i < 256; i++) lng2[i] = 'q';
    lng2[256] = 0;
    for (int i = 0; i < 4096; i++) huge[i] = '/';
    huge[4096] = 0;

    /* old string beats everything of the new pair */
    expect("old_efault_vs_bad_newfd", renameat_(d, (const char *)1, 9999, "x"), -EFAULT);
    expect("old_huge_vs_bad_newfd", renameat_(d, huge, 9999, "x"), -ENAMETOOLONG);
    expect("old_empty_vs_bad_newfd", renameat_(d, "", 9999, "x"), -ENOENT);
    /* olddirfd beats the new pair */
    expect("bad_oldfd_vs_new_efault", renameat_(9999, "f", d, (const char *)1), -EBADF);
    expect("file_oldfd_vs_bad_newfd", renameat_(file, "f", 9999, "x"), -ENOTDIR);
    /* old directories before the new pair */
    expect("old_missing_dir_vs_bad_newfd", renameat_(d, "nope/x", 9999, "x"), -ENOENT);
    expect("old_notdir_vs_new_huge", renameat_(d, "f/x", d, huge), -ENOTDIR);
    /* the old last name is judged after the new pair's directories */
    expect("old_missing_last_vs_bad_newfd", renameat_(d, "missing", 9999, "x"), -EBADF);
    expect("old_missing_last_vs_new_huge", renameat_(d, "missing", d, huge), -ENAMETOOLONG);
    expect("old_missing_last_vs_new_missing_dir", renameat_(d, "missing", d, "nope/x"), -ENOENT);
    expect("old_long_last_vs_new_notdir", renameat_(d, lng2, d, "f/x"), -ENOTDIR);
    expect("old_long_last_vs_new_missing_dir", renameat_(AT_FDCWD, lng, d, "nope/x"), -ENOENT);
    /* old last name before new last name */
    expect("old_missing_vs_new_long", renameat_(d, "missing", d, lng2), -ENOENT);
    expect("old_long_vs_new_long", renameat_(d, lng2, d, lng2), -ENAMETOOLONG);
    expect("old_ok_vs_new_long", renameat_(d, "f", d, lng2), -ENAMETOOLONG);
    /* "." / ".." last components */
    expect("old_dot_vs_new_missing_dir", renameat_(d, "sub/.", d, "nope/x"), -ENOENT);
    expect("old_missing_vs_new_dot", renameat_(d, "missing", d, "sub/.."), -EBUSY);
    /* then the rename's own errors */
    expect("dir_over_file", renameat_(d, "sub", d, "f"), -ENOTDIR);
    expect("file_over_dir", renameat_(d, "f", d, "sub"), -EISDIR);

    expect("close_file", a_close(file), 0);
    expect("close_d", a_close(d), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
