/* creat(2) where a directory is involved: an existing directory gives
 * EISDIR, a trailing slash gives EISDIR for a missing name or a file, and
 * "." or "/" are directories too. Nothing is created or changed. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_dirs"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);

    expect("dir_eisdir", cr(D "/sub", 0644), -EISDIR);
    expect("dir_slash_eisdir", cr(D "/sub/", 0644), -EISDIR);
    expect("dir_dot_eisdir", cr(D "/sub/.", 0644), -EISDIR);
    expect("dir_still_dir", a_stat_path(D "/sub", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFDIR);
    expect("tmp_eisdir", cr("/tmp", 0644), -EISDIR);
    expect("root_eisdir", cr("/", 0644), -EISDIR);

    expect("missing_slash_eisdir", cr(D "/new/", 0644), -EISDIR);
    expect("missing_slash_not_created", a_stat_path(D "/new", &st), -ENOENT);
    expect("file_slash_eisdir", cr(D "/f/", 0644), -EISDIR);
    expect("file_not_truncated", a_stat_path(D "/f", &st) == 0 ? (i64)st.st_size : -1, 3);

    /* Creating inside a subdirectory through ".." and "." works. */
    i64 fd = cr(D "/sub/../sub/./g", 0600);
    expect("dot_components_fd", fd, 3);
    a_close(fd);
    expect("dot_components_made_g", a_stat_path(D "/sub/g", &st), 0);
    expect("rmdir_nonempty", a_rmdir(D "/sub"), -ENOTEMPTY);

    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
