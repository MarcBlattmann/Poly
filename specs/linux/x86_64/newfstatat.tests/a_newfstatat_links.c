/* newfstatat: link counts of files and directories as entries are added
 * and removed; hard links share an inode. */
#include "a_sys.h"

#define D "/tmp/a_newfstatat_links"

static i64 nlink(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_nlink;
}

int test_main(void) {
    struct a_stat a, b;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("empty_dir_nlink_2", nlink(D), 2);
    expect("mkdir_sub", a_mkdir(D "/sub", 0700), 0);
    expect("dir_with_subdir_nlink_3", nlink(D), 3);
    expect("file", a_make_file(D "/f", "x", 0644), 0);
    expect("file_does_not_add_dir_link", nlink(D), 3);
    expect("file_nlink_1", nlink(D "/f"), 1);
    expect("link", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/sub/g", 0), 0);
    expect("file_nlink_2", nlink(D "/f"), 2);
    a_stat_path(D "/f", &a);
    a_stat_path(D "/sub/g", &b);
    check("hard_link_same_inode", a.st_ino == b.st_ino && a.st_dev == b.st_dev, 0);
    expect("unlink_original", a_unlink(D "/f"), 0);
    expect("link_nlink_1", nlink(D "/sub/g"), 1);
    expect("unlink_link", a_unlink(D "/sub/g"), 0);
    expect("rmdir_sub", a_rmdir(D "/sub"), 0);
    expect("dir_back_to_2", nlink(D), 2);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    expect("removed_dir_enoent", nlink(D), -ENOENT);
    return finish();
}
