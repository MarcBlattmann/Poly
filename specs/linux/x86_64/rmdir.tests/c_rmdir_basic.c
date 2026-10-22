/* rmdir(path) (syscall 84) removes an empty directory and returns 0: the
 * name is gone at once (ENOENT for stat, open and a second rmdir), the
 * parent's link count drops by one, and the name can be used again. Only
 * directories made with mkdir and holding nothing but "." and ".." count
 * as empty; a directory whose last entry was removed is empty again. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_basic"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }

int test_main(void) {
    struct a_stat st, parent;
    expect("setup_parent", a_mkdir(D, 0755), 0);
    expect("parent_stat_before", a_stat_path(D, &parent), 0);
    u64 nlink_before = parent.st_nlink;

    expect("mkdir_a", a_mkdir(D "/a", 0755), 0);
    expect("parent_nlink_plus_1", a_stat_path(D, &parent) == 0 ? (i64)parent.st_nlink : -1, (i64)nlink_before + 1);
    expect("rmdir_returns_0", rm(D "/a"), 0);
    expect("stat_gone", a_lstat_path(D "/a", &st), -ENOENT);
    expect("open_gone", a_open(D "/a", O_RDONLY | O_DIRECTORY, 0), -ENOENT);
    expect("parent_nlink_back", a_stat_path(D, &parent) == 0 ? (i64)parent.st_nlink : -1, (i64)nlink_before);
    expect("rmdir_again_enoent", rm(D "/a"), -ENOENT);

    /* The name can be reused, as a directory or as a file. */
    expect("mkdir_a_again", a_mkdir(D "/a", 0700), 0);
    expect("rmdir_a_again", rm(D "/a"), 0);
    expect("file_a", a_make_file(D "/a", "x", 0644), 0);
    expect("unlink_file_a", a_unlink(D "/a"), 0);

    /* Emptied directories can be removed. */
    expect("mkdir_b", a_mkdir(D "/b", 0755), 0);
    expect("file_in_b", a_make_file(D "/b/f", "x", 0644), 0);
    expect("mkdir_in_b", a_mkdir(D "/b/s", 0755), 0);
    expect("b_not_empty", rm(D "/b"), -ENOTEMPTY);
    expect("rm_b_f", a_unlink(D "/b/f"), 0);
    expect("rm_b_s", rm(D "/b/s"), 0);
    expect("b_nlink_2", a_stat_path(D "/b", &st) == 0 ? (i64)st.st_nlink : -1, 2);
    expect("rmdir_emptied_b", rm(D "/b"), 0);

    /* Nested: inner first, then outer. */
    expect("mkdir_c", a_mkdir(D "/c", 0755), 0);
    expect("mkdir_c_d", a_mkdir(D "/c/d", 0755), 0);
    expect("mkdir_c_d_e", a_mkdir(D "/c/d/e", 0755), 0);
    expect("outer_not_empty", rm(D "/c"), -ENOTEMPTY);
    expect("rm_e", rm(D "/c/d/e"), 0);
    expect("rm_d", rm(D "/c/d"), 0);
    expect("rm_c", rm(D "/c"), 0);

    /* The directory's own mode does not matter. */
    expect("mkdir_mode_0", a_mkdir(D "/z", 0), 0);
    expect("rmdir_mode_0", rm(D "/z"), 0);
    expect("mkdir_sticky", a_mkdir(D "/t", 01777), 0);
    expect("rmdir_sticky_itself", rm(D "/t"), 0);

    expect("parent_nlink_final", a_stat_path(D, &parent) == 0 ? (i64)parent.st_nlink : -1, (i64)nlink_before);
    expect("cleanup_rmdir", rm(D), 0);
    expect("cleanup_gone", a_lstat_path(D, &st), -ENOENT);
    return finish();
}
