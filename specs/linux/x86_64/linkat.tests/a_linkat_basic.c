/* linkat: a second name for a file, sharing contents and inode; names
 * that exist, directories and missing sources fail. */
#include "a_sys.h"

#define D "/tmp/a_linkat_basic"

static i64 link2(const char *a, const char *b, i64 flags) {
    return syscall5(SYS_linkat, AT_FDCWD, a, AT_FDCWD, b, flags);
}

int test_main(void) {
    struct a_stat a, b;
    char buf[16];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    expect("link", link2(D "/f", D "/g", 0), 0);
    a_stat_path(D "/f", &a);
    a_stat_path(D "/g", &b);
    check("same_inode", a.st_ino == b.st_ino && a.st_dev == b.st_dev, 0);
    expect("nlink_2", a.st_nlink, 2);
    i64 fd = a_open(D "/g", O_WRONLY | O_APPEND, 0);
    a_write(fd, "def", 3);
    a_close(fd);
    fd = a_open(D "/f", O_RDONLY, 0);
    expect("write_via_link_visible", a_read(fd, buf, sizeof buf), 6);
    a_close(fd);
    expect("link_into_subdir", link2(D "/f", D "/sub/h", 0), 0);
    a_stat_path(D "/f", &a);
    expect("nlink_3", a.st_nlink, 3);
    expect("existing_target_eexist", link2(D "/f", D "/g", 0), -EEXIST);
    expect("missing_source_enoent", link2(D "/none", D "/x", 0), -ENOENT);
    expect("missing_target_dir_enoent", link2(D "/f", D "/no/x", 0), -ENOENT);
    expect("directory_source_eperm", link2(D "/sub", D "/subl", 0), -EPERM);
    expect("target_trailing_slash_enoent", link2(D "/f", D "/x/", 0), -ENOENT);
    expect("source_trailing_slash_enotdir", link2(D "/f/", D "/x", 0), -ENOTDIR);
    expect("unknown_flag_einval", link2(D "/f", D "/x", 0x1), -EINVAL);
    expect("old_link_syscall", syscall2(SYS_link, D "/f", D "/old"), 0);
    a_stat_path(D "/f", &a);
    expect("nlink_4", a.st_nlink, 4);
    /* ctime of the file changes when a link is added; the names survive
     * removal of the original. */
    expect("unlink_original", a_unlink(D "/f"), 0);
    a_stat_path(D "/g", &b);
    expect("nlink_3_after_unlink", b.st_nlink, 3);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_old", a_unlink(D "/old"), 0);
    expect("cleanup_h", a_unlink(D "/sub/h"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
