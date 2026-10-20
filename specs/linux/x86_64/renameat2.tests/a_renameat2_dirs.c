/* renameat2: the type rules for directories (EISDIR, ENOTDIR,
 * ENOTEMPTY), moving into its own subtree (EINVAL), "." and "..". */
#include "a_sys.h"

#define D "/tmp/a_renameat2_dirs"

static i64 rename2(const char *a, const char *b, i64 flags) {
    return syscall5(SYS_renameat2, AT_FDCWD, a, AT_FDCWD, b, flags);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_d1", a_mkdir(D "/d1", 0700), 0);
    expect("setup_d2", a_mkdir(D "/d2", 0700), 0);
    expect("setup_full", a_mkdir(D "/full", 0700), 0);
    expect("setup_full_f", a_make_file(D "/full/f", "", 0644), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);

    expect("file_over_dir_eisdir", rename2(D "/f", D "/d1", 0), -EISDIR);
    expect("dir_over_file_enotdir", rename2(D "/d1", D "/f", 0), -ENOTDIR);
    expect("dir_over_nonempty_enotempty", rename2(D "/d1", D "/full", 0), -ENOTEMPTY);
    expect("dir_over_empty_dir", rename2(D "/d1", D "/d2", 0), 0);
    expect("replaced_dir_source_gone", a_stat_path(D "/d1", &st), -ENOENT);
    expect("into_own_subdir_einval", rename2(D "/full", D "/full/sub", 0), -EINVAL);
    expect("dot_source_ebusy", rename2(D "/d2/.", D "/x", 0), -EBUSY);
    expect("dotdot_target_ebusy", rename2(D "/f", D "/d2/..", 0), -EBUSY);
    expect("trailing_slash_on_file_enotdir", rename2(D "/f/", D "/g", 0), -ENOTDIR);
    expect("trailing_slash_on_dir", rename2(D "/d2/", D "/d3/", 0), 0);
    expect("moved_with_slashes", a_stat_path(D "/d3", &st), 0);
    expect("file_to_trailing_slash_enotdir", rename2(D "/f", D "/g/", 0), -ENOTDIR);
    /* The parent's link count follows a directory moved out of it. */
    expect("move_dir_into_full", rename2(D "/d3", D "/full/d3", 0), 0);
    a_stat_path(D "/full", &st);
    expect("full_nlink_3", st.st_nlink, 3);
    a_stat_path(D, &st);
    expect("base_nlink_3", st.st_nlink, 3);

    expect("cleanup_d3", a_rmdir(D "/full/d3"), 0);
    expect("cleanup_full_f", a_unlink(D "/full/f"), 0);
    expect("cleanup_full", a_rmdir(D "/full"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
