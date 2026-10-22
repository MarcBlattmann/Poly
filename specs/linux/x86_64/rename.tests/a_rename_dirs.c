/* rename of directories: the contents move with the directory and the
 * link counts of the old and new parents adjust; a directory replaces
 * an empty directory but not a non-empty one (ENOTEMPTY); a file does
 * not replace a directory (EISDIR) nor a directory a file (ENOTDIR).
 * A failed rename leaves both names as they were. */
#include "a_sys.h"

#define D "/tmp/a_rename_dirs"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }
static u64 nlink(const char *p) {
    struct a_stat st;
    return a_stat_path(p, &st) == 0 ? st.st_nlink : 999;
}
static u64 ino(const char *p) {
    struct a_stat st;
    return a_stat_path(p, &st) == 0 ? st.st_ino : 0;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_p", a_mkdir(D "/p", 0755), 0);
    expect("setup_q", a_mkdir(D "/q", 0755), 0);
    expect("setup_d", a_mkdir(D "/p/d", 0755), 0);
    expect("setup_d_child", a_mkdir(D "/p/d/child", 0755), 0);
    expect("setup_d_file", a_make_file(D "/p/d/file", "x", 0644), 0);
    expect("p_nlink_3", nlink(D "/p"), 3);
    expect("q_nlink_2", nlink(D "/q"), 2);
    u64 dino = ino(D "/p/d");

    expect("move_dir", rn(D "/p/d", D "/q/d"), 0);
    expect("old_gone", a_stat_path(D "/p/d", &st), -ENOENT);
    expect("p_nlink_2", nlink(D "/p"), 2);
    expect("q_nlink_3", nlink(D "/q"), 3);
    check("same_dir_inode", ino(D "/q/d") == dino, 0);
    expect("d_nlink_3", nlink(D "/q/d"), 3);
    expect("child_moved", a_stat_path(D "/q/d/child", &st), 0);
    expect("file_moved", a_stat_path(D "/q/d/file", &st), 0);
    check("dotdot_is_new_parent", ino(D "/q/d/..") == ino(D "/q"), 0);
    check("child_dotdot_dotdot", ino(D "/q/d/child/../..") == ino(D "/q"), 0);

    expect("rename_dir_same_parent", rn(D "/q/d", D "/q/e"), 0);
    expect("q_nlink_still_3", nlink(D "/q"), 3);

    /* Directory over an empty directory: replaced. */
    expect("setup_empty", a_mkdir(D "/p/empty", 0700), 0);
    expect("p_nlink_3_again", nlink(D "/p"), 3);
    expect("dir_over_empty_dir", rn(D "/q/e", D "/p/empty"), 0);
    check("target_is_source_inode", ino(D "/p/empty") == dino, 0);
    expect("source_gone", a_stat_path(D "/q/e", &st), -ENOENT);
    expect("p_nlink_3_after_replace", nlink(D "/p"), 3);
    expect("q_nlink_2_after_replace", nlink(D "/q"), 2);
    expect("contents_after_replace", a_stat_path(D "/p/empty/file", &st), 0);
    expect("mode_is_source", a_stat_path(D "/p/empty", &st) == 0 && st.st_mode == (S_IFDIR | 0755), 1);

    /* Directory over a non-empty directory. */
    expect("setup_q_full", a_make_file(D "/q/x", "", 0644), 0);
    expect("dir_over_nonempty", rn(D "/p/empty", D "/q"), -ENOTEMPTY);
    expect("setup_r", a_mkdir(D "/r", 0755), 0);
    expect("setup_r_sub", a_mkdir(D "/r/s", 0755), 0);
    expect("dir_over_dir_with_subdir", rn(D "/p/empty", D "/r"), -ENOTEMPTY);
    expect("source_kept_after_enotempty", a_stat_path(D "/p/empty/file", &st), 0);
    expect("target_kept_after_enotempty", a_stat_path(D "/q/x", &st), 0);

    /* Mismatched types. */
    expect("file_over_dir", rn(D "/q/x", D "/r/s"), -EISDIR);
    expect("dir_over_file", rn(D "/r/s", D "/q/x"), -ENOTDIR);
    expect("file_kept", a_stat_path(D "/q/x", &st) == 0 && (st.st_mode & S_IFMT) == S_IFREG, 1);
    expect("dir_kept", a_stat_path(D "/r/s", &st) == 0 && (st.st_mode & S_IFMT) == S_IFDIR, 1);
    expect("empty_dir_over_nonempty_dir", rn(D "/r/s", D "/q"), -ENOTEMPTY);

    expect("cleanup_x", a_unlink(D "/q/x"), 0);
    expect("cleanup_s", a_rmdir(D "/r/s"), 0);
    expect("cleanup_r", a_rmdir(D "/r"), 0);
    expect("cleanup_q", a_rmdir(D "/q"), 0);
    expect("cleanup_file", a_unlink(D "/p/empty/file"), 0);
    expect("cleanup_child", a_rmdir(D "/p/empty/child"), 0);
    expect("cleanup_empty", a_rmdir(D "/p/empty"), 0);
    expect("cleanup_p", a_rmdir(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
