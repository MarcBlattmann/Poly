/* lstat: st_nlink of files and directories as names are added and
 * removed; hard links share st_ino and st_dev; a hard link to a symbolic
 * link is still a link. */
#include "a_sys.h"

#define D "/tmp/a_lstat_nlink"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }
static i64 link(const char *a, const char *b) { return syscall5(SYS_linkat, AT_FDCWD, a, AT_FDCWD, b, 0); }

int test_main(void) {
    struct a_stat st, f;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    expect("file_nlink_1", (lstat(D "/f", &f), f.st_nlink), 1);
    expect("link_1", link(D "/f", D "/g"), 0);
    expect("file_nlink_2", (lstat(D "/f", &st), st.st_nlink), 2);
    expect("g_ok", lstat(D "/g", &st), 0);
    check("g_same_inode", st.st_ino == f.st_ino && st.st_dev == f.st_dev, 0);
    expect("g_nlink_2", st.st_nlink, 2);
    expect("link_2", link(D "/g", D "/h"), 0);
    expect("file_nlink_3", (lstat(D "/f", &st), st.st_nlink), 3);
    expect("unlink_f", a_unlink(D "/f"), 0);
    expect("f_gone", lstat(D "/f", &st), -ENOENT);
    expect("h_nlink_2", (lstat(D "/h", &st), st.st_nlink), 2);
    check("h_same_inode", st.st_ino == f.st_ino, 0);

    expect("setup_symlink", syscall3(SYS_symlinkat, "h", AT_FDCWD, D "/sl"), 0);
    expect("hardlink_to_symlink", link(D "/sl", D "/sl2"), 0);
    struct a_stat a, b;
    expect("sl_ok", lstat(D "/sl", &a), 0);
    expect("sl2_ok", lstat(D "/sl2", &b), 0);
    expect("sl2_is_link", b.st_mode & S_IFMT, S_IFLNK);
    check("sl2_same_inode", a.st_ino == b.st_ino, 0);
    expect("sl_nlink_2", a.st_nlink, 2);
    expect("h_nlink_unchanged", (lstat(D "/h", &st), st.st_nlink), 2);

    expect("dir_nlink_2", (lstat(D, &st), st.st_nlink), 2);
    expect("mkdir_a", a_mkdir(D "/a", 0755), 0);
    expect("dir_nlink_3", (lstat(D, &st), st.st_nlink), 3);
    expect("mkdir_b", a_mkdir(D "/b", 0755), 0);
    expect("dir_nlink_4", (lstat(D, &st), st.st_nlink), 4);
    expect("subdir_nlink_2", (lstat(D "/a", &st), st.st_nlink), 2);
    expect("rmdir_a", a_rmdir(D "/a"), 0);
    expect("dir_nlink_back_3", (lstat(D, &st), st.st_nlink), 3);
    expect("files_do_not_count", (a_make_file(D "/x", "", 0644), lstat(D, &st), st.st_nlink), 3);

    expect("cleanup_x", a_unlink(D "/x"), 0);
    expect("cleanup_b", a_rmdir(D "/b"), 0);
    expect("cleanup_sl", a_unlink(D "/sl"), 0);
    expect("cleanup_sl2", a_unlink(D "/sl2"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
