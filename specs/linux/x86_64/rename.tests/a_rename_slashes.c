/* rename and trailing slashes: allowed on either path when the source
 * is a directory (also onto a new name); with a non-directory source,
 * a trailing slash on either path is ENOTDIR, whether the target exists
 * or not. A symbolic link to a directory is not a directory here: with
 * a trailing slash as the source, or as the target of a directory
 * rename, it is ENOTDIR. */
#include "a_sys.h"

#define D "/tmp/a_rename_slashes"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_d", a_mkdir(D "/d", 0755), 0);
    expect("setup_e", a_mkdir(D "/e", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("setup_g", a_make_file(D "/g", "", 0644), 0);
    expect("setup_ld", syscall3(SYS_symlinkat, "e", AT_FDCWD, D "/ld"), 0);

    expect("dir_src_slash", rn(D "/d/", D "/d1"), 0);
    expect("dir_dst_slash_new", rn(D "/d1", D "/d2/"), 0);
    expect("dir_both_slashes", rn(D "/d2/", D "/d3/"), 0);
    expect("dir_many_slashes", rn(D "/d3///", D "/d4//"), 0);
    expect("dir_slash_over_empty_dir", rn(D "/d4/", D "/e/"), 0);
    expect("moved_dir_there", a_stat_path(D "/e", &st), 0);
    expect("old_dir_gone", a_stat_path(D "/d4", &st), -ENOENT);

    expect("file_src_slash", rn(D "/f/", D "/z"), -ENOTDIR);
    expect("file_dst_slash_new", rn(D "/f", D "/z/"), -ENOTDIR);
    expect("file_dst_slash_existing", rn(D "/f", D "/g/"), -ENOTDIR);
    expect("file_src_slash_dst_dir", rn(D "/f/", D "/e"), -ENOTDIR);
    expect("file_kept", a_stat_path(D "/f", &st), 0);
    expect("g_kept", a_stat_path(D "/g", &st), 0);
    expect("nothing_created", a_stat_path(D "/z", &st), -ENOENT);

    expect("dirlink_src_slash", rn(D "/ld/", D "/z"), -ENOTDIR);
    expect("dir_over_dirlink", rn(D "/e", D "/ld"), -ENOTDIR);
    expect("dir_over_dirlink_slash", rn(D "/e", D "/ld/"), -ENOTDIR);
    expect("dirlink_plain_ok", rn(D "/ld", D "/ld2"), 0);
    expect("still_a_link", a_lstat_path(D "/ld2", &st) == 0 && (st.st_mode & S_IFMT) == S_IFLNK, 1);

    expect("cleanup_ld2", a_unlink(D "/ld2"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_e", a_rmdir(D "/e"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
