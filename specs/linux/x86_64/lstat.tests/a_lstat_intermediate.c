/* lstat: links in the middle of a path are followed, only the last name
 * is not; a trailing slash (or "/.") makes the last link be followed too;
 * ".." after a followed link goes up from the link's target. */
#include "a_sys.h"

#define D "/tmp/a_lstat_intermediate"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }
static i64 symlink(const char *t, const char *l) { return syscall3(SYS_symlinkat, t, AT_FDCWD, l); }

int test_main(void) {
    struct a_stat st, dir, sub, inner, file, lnk;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner", a_mkdir(D "/sub/inner", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/inner/f", "12345", 0644), 0);
    expect("setup_rel_link", symlink("sub/inner", D "/rel"), 0);
    expect("setup_abs_link", symlink(D "/sub/inner", D "/abs"), 0);
    expect("setup_flink", symlink("sub/inner/f", D "/flink"), 0);
    expect("setup_chain", symlink("rel", D "/chain"), 0);
    lstat(D, &dir);
    lstat(D "/sub", &sub);
    lstat(D "/sub/inner", &inner);
    lstat(D "/sub/inner/f", &file);
    expect("lstat_rel", lstat(D "/rel", &lnk), 0);
    expect("rel_is_link", lnk.st_mode & S_IFMT, S_IFLNK);
    expect("rel_size", lnk.st_size, 9);
    expect("through_rel", lstat(D "/rel/f", &st), 0);
    check("through_rel_is_file", st.st_ino == file.st_ino && st.st_size == 5, 0);
    expect("through_abs", lstat(D "/abs/f", &st), 0);
    check("through_abs_is_file", st.st_ino == file.st_ino, 0);
    expect("abs_size", (lstat(D "/abs", &st), st.st_size), (i64)str_len(D "/sub/inner"));
    expect("rel_slash", lstat(D "/rel/", &st), 0);
    check("rel_slash_is_target", st.st_ino == inner.st_ino && (st.st_mode & S_IFMT) == S_IFDIR, 0);
    expect("rel_dot", lstat(D "/rel/.", &st), 0);
    check("rel_dot_is_target", st.st_ino == inner.st_ino, 0);
    expect("rel_dotdot", lstat(D "/rel/..", &st), 0);
    check("rel_dotdot_is_sub", st.st_ino == sub.st_ino, 0);
    expect("flink_is_link", (lstat(D "/flink", &st), st.st_mode & S_IFMT), S_IFLNK);
    expect("flink_slash_enotdir", lstat(D "/flink/", &st), -ENOTDIR);
    expect("chain_is_link", (lstat(D "/chain", &st), st.st_mode & S_IFMT), S_IFLNK);
    expect("chain_size", st.st_size, 3);
    check("chain_own_inode", st.st_ino != lnk.st_ino, 0);
    expect("chain_slash", lstat(D "/chain/", &st), 0);
    check("chain_slash_is_inner", st.st_ino == inner.st_ino, 0);
    expect("chain_child", lstat(D "/chain/f", &st), 0);
    check("chain_child_is_file", st.st_ino == file.st_ino, 0);
    expect("dir_double_slash", lstat(D "//sub///inner//", &st), 0);
    check("double_slash_is_inner", st.st_ino == inner.st_ino, 0);
    expect("cleanup_chain", a_unlink(D "/chain"), 0);
    expect("cleanup_flink", a_unlink(D "/flink"), 0);
    expect("cleanup_abs", a_unlink(D "/abs"), 0);
    expect("cleanup_rel", a_unlink(D "/rel"), 0);
    expect("cleanup_f", a_unlink(D "/sub/inner/f"), 0);
    expect("cleanup_inner", a_rmdir(D "/sub/inner"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
