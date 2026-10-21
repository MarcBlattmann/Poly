/* stat: relative paths start at the working directory; the working
 * directory can be stat'ed as "." even after it was removed, but names
 * inside it can no longer be found. */
#include "a_sys.h"

#define D "/tmp/a_stat_relative"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

int test_main(void) {
    struct a_stat s, d, t;
    a_unlink(D "/sub/f");
    a_rmdir(D "/sub");
    a_rmdir(D "/gone");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "12345", 0644), 0);

    /* Tests start in "/". */
    expect("relative_from_root", st("tmp/a_stat_relative/sub/f", &s), 0);
    expect("relative_from_root_size", s.st_size, 5);

    expect("chdir", syscall1(SYS_chdir, D), 0);
    st(D, &d);
    expect("dot", st(".", &t), 0);
    check("dot_is_cwd", t.st_ino == d.st_ino && t.st_dev == d.st_dev, 0);
    expect("relative_name", st("sub/f", &t), 0);
    check("relative_same_inode", t.st_ino == s.st_ino, 0);
    expect("dot_slash", st("./sub/./f", &t), 0);
    expect("dotdot_back", st("../a_stat_relative/sub/f", &t), 0);
    check("dotdot_same_inode", t.st_ino == s.st_ino, 0);
    expect("relative_missing", st("nope", &t), -ENOENT);
    expect("relative_enotdir", st("sub/f/", &t), -ENOTDIR);

    expect("chdir_sub", syscall1(SYS_chdir, "sub"), 0);
    expect("bare_name", st("f", &t), 0);
    check("bare_name_same_inode", t.st_ino == s.st_ino, 0);
    expect("dotdot", st("..", &t), 0);
    check("dotdot_is_parent", t.st_ino == d.st_ino, 0);
    expect("absolute_ignores_cwd", st(D, &t), 0);
    check("absolute_same", t.st_ino == d.st_ino, 0);

    /* Removed working directory. */
    expect("mkdir_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_cwd", a_rmdir(D "/gone"), 0);
    expect("removed_cwd_dot", st(".", &t), 0);
    expect("removed_cwd_nlink_0", t.st_nlink, 0);
    expect("removed_cwd_is_dir", t.st_mode & S_IFMT, S_IFDIR);
    expect("removed_cwd_name_enoent", st("x", &t), -ENOENT);
    expect("removed_cwd_dotdot", st("..", &t), 0);
    check("removed_cwd_dotdot_is_parent", t.st_ino == d.st_ino, 0);

    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
