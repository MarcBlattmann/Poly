/* rename(2) of directories: contents move along; an empty directory can
 * be replaced, a non-empty one not (ENOTEMPTY); a directory may not
 * replace a file (ENOTDIR) nor a file a directory (EISDIR); a directory
 * may not move into itself (EINVAL); link counts of both parents adjust
 * and ".." follows; trailing slashes are fine on directories. */
#include "a_sys.h"

#define D "/tmp/lb_rename_dirs"

static i64 rename_(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }
static i64 ino_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_ino;
}
static i64 nlink_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_nlink;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_p1", a_mkdir(D "/p1", 0755), 0);
    expect("setup_p2", a_mkdir(D "/p2", 0755), 0);
    expect("setup_d", a_mkdir(D "/p1/d", 0755), 0);
    expect("setup_d_file", a_make_file(D "/p1/d/inside", "x", 0644), 0);
    expect("setup_d_sub", a_mkdir(D "/p1/d/sub", 0755), 0);
    i64 ino_d = ino_of(D "/p1/d");
    expect("p1_nlink_3", nlink_of(D "/p1"), 3);
    expect("p2_nlink_2", nlink_of(D "/p2"), 2);

    expect("move_dir", rename_(D "/p1/d", D "/p2/d"), 0);
    expect("moved_inode", ino_of(D "/p2/d"), ino_d);
    expect("contents_moved", a_lstat_path(D "/p2/d/inside", &st), 0);
    expect("p1_nlink_2", nlink_of(D "/p1"), 2);
    expect("p2_nlink_3", nlink_of(D "/p2"), 3);
    expect("dotdot_is_p2", ino_of(D "/p2/d/.."), ino_of(D "/p2"));
    expect("d_nlink_3", nlink_of(D "/p2/d"), 3);

    /* trailing slashes on directories */
    expect("trailing_slashes", rename_(D "/p2/d/", D "/p2/e/"), 0);
    expect("trailing_one_side", rename_(D "/p2/e", D "/p2/d/"), 0);

    /* replace an empty directory */
    expect("setup_empty", a_mkdir(D "/p1/empty", 0700), 0);
    i64 efd = a_open(D "/p1/empty", O_RDONLY | O_DIRECTORY, 0);
    check("open_empty", efd >= 0, efd);
    expect("replace_empty_dir", rename_(D "/p2/d", D "/p1/empty"), 0);
    expect("replace_empty_inode", ino_of(D "/p1/empty"), ino_d);
    expect("replaced_fd_nlink_0", syscall2(SYS_fstat, efd, &st) == 0 && st.st_nlink == 0, 1);
    expect("close_empty", a_close(efd), 0);
    expect("nlinks_after_replace", nlink_of(D "/p1") * 10 + nlink_of(D "/p2"), 32);

    /* non-empty target, kinds mismatch, own subtree */
    expect("setup_full", a_mkdir(D "/full", 0755), 0);
    expect("setup_full_f", a_make_file(D "/full/f", "x", 0644), 0);
    expect("onto_non_empty", rename_(D "/p1/empty", D "/full"), -ENOTEMPTY);
    expect("setup_file", a_make_file(D "/file", "x", 0644), 0);
    expect("dir_over_file", rename_(D "/p1/empty", D "/file"), -ENOTDIR);
    expect("file_over_dir", rename_(D "/file", D "/p2"), -EISDIR);
    expect("file_over_dir_slash_enotdir", rename_(D "/file", D "/p2/"), -ENOTDIR);
    expect("into_itself", rename_(D "/p1", D "/p1/x"), -EINVAL);
    expect("into_own_subdir", rename_(D "/p1", D "/p1/empty/x"), -EINVAL);
    expect("subdir_over_ancestor", rename_(D "/p1/empty", D "/p1"), -ENOTEMPTY);
    expect("ancestor_over_own_empty_subdir", rename_(D "/p1", D "/p1/empty/sub"), -EINVAL);
    expect("onto_itself_dir", rename_(D "/p1", D "/p1"), 0);
    expect("onto_itself_slash", rename_(D "/p1/", D "/p1"), 0);
    expect("dir_still_there", ino_of(D "/p1/empty"), ino_d);

    expect("cleanup_file", a_unlink(D "/file"), 0);
    expect("cleanup_full_f", a_unlink(D "/full/f"), 0);
    expect("cleanup_full", a_rmdir(D "/full"), 0);
    expect("cleanup_inside", a_unlink(D "/p1/empty/inside"), 0);
    expect("cleanup_sub", a_rmdir(D "/p1/empty/sub"), 0);
    expect("cleanup_empty", a_rmdir(D "/p1/empty"), 0);
    expect("cleanup_p1", a_rmdir(D "/p1"), 0);
    expect("cleanup_p2", a_rmdir(D "/p2"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
