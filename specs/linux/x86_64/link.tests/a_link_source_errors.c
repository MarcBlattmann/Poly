/* link: problems with the old name. A missing name, an empty path or a
 * missing directory on the way: ENOENT; a directory (also ".", "..",
 * "/"): EPERM; a file used as a directory or a trailing slash after a
 * file: ENOTDIR. Nothing is created under the new name. */
#include "a_sys.h"

#define D "/tmp/a_link_source_errors"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("missing", syscall2(SYS_link, D "/none", D "/x"), -ENOENT);
    expect("missing_dir", syscall2(SYS_link, D "/nodir/f", D "/x"), -ENOENT);
    expect("empty", syscall2(SYS_link, "", D "/x"), -ENOENT);
    expect("directory_eperm", syscall2(SYS_link, D "/sub", D "/x"), -EPERM);
    expect("directory_slash_eperm", syscall2(SYS_link, D "/sub/", D "/x"), -EPERM);
    expect("dot_eperm", syscall2(SYS_link, D "/sub/.", D "/x"), -EPERM);
    expect("dotdot_eperm", syscall2(SYS_link, D "/sub/..", D "/x"), -EPERM);
    expect("root_eperm", syscall2(SYS_link, "/", D "/x"), -EPERM);
    expect("file_as_dir", syscall2(SYS_link, D "/f/y", D "/x"), -ENOTDIR);
    expect("file_trailing_slash", syscall2(SYS_link, D "/f/", D "/x"), -ENOTDIR);
    expect("file_trailing_dot", syscall2(SYS_link, D "/f/.", D "/x"), -ENOTDIR);
    expect("nothing_made", a_lstat_path(D "/x", &st), -ENOENT);
    a_stat_path(D "/f", &st);
    expect("nlink_1", st.st_nlink, 1);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
