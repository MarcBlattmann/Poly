/* link: problems with the new name. Its directory missing: ENOENT; a
 * file used as its directory: ENOTDIR; an empty path: ENOENT; a
 * trailing slash on a name that does not exist: ENOENT. The source's
 * link count does not change. */
#include "a_sys.h"

#define D "/tmp/a_link_target_errors"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("missing_dir", syscall2(SYS_link, D "/f", D "/nodir/x"), -ENOENT);
    expect("missing_dirs", syscall2(SYS_link, D "/f", D "/a/b/c"), -ENOENT);
    expect("file_as_dir", syscall2(SYS_link, D "/f", D "/f/x"), -ENOTDIR);
    expect("empty", syscall2(SYS_link, D "/f", ""), -ENOENT);
    expect("trailing_slash", syscall2(SYS_link, D "/f", D "/x/"), -ENOENT);
    expect("trailing_slash_dot", syscall2(SYS_link, D "/f", D "/x/."), -ENOENT);
    expect("not_made", a_lstat_path(D "/x", &st), -ENOENT);
    a_stat_path(D "/f", &st);
    expect("nlink_still_1", st.st_nlink, 1);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
