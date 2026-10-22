/* symlink: a relative linkpath is resolved from the working directory;
 * relative link text is resolved later, from the directory holding the
 * link, not from the working directory of the caller. A working directory
 * that was removed accepts no new names (ENOENT). */
#include "a_sys.h"

#define D "/tmp/c_symlink_cwd"

static char buf[64];

int test_main(void) {
    struct a_stat st, want;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("chdir_d", syscall1(SYS_chdir, D), 0);

    expect("relative_name", syscall2(SYS_symlink, "f", "rl"), 0);
    expect("made_in_cwd", a_lstat_path(D "/rl", &st), 0);
    expect("made_is_link", st.st_mode & S_IFMT, S_IFLNK);
    expect("dot_slash_name", syscall2(SYS_symlink, "f", "./dl"), 0);
    expect("dot_slash_made", a_lstat_path(D "/dl", &st), 0);
    expect("sub_name", syscall2(SYS_symlink, "f", "sub/lf"), 0);
    expect("sub_made", syscall3(SYS_readlink, D "/sub/lf", buf, sizeof buf), 1);
    /* "f" in sub/lf means sub/f, which does not exist. */
    expect("text_relative_to_link_dir", a_stat_path(D "/sub/lf", &st), -ENOENT);
    expect("sub_up_name", syscall2(SYS_symlink, "../f", "sub/up"), 0);
    expect("sub_up_resolves", a_stat_path(D "/sub/up", &st), 0);
    expect("stat_f", a_stat_path(D "/f", &want), 0);
    check("sub_up_is_f", st.st_ino == want.st_ino, 0);

    /* The working directory of the caller does not matter when the link is
     * followed. */
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("rl_from_sub", a_stat_path(D "/rl", &st), 0);
    check("rl_is_f", st.st_ino == want.st_ino, 0);
    expect("dotdot_name", syscall2(SYS_symlink, "f", "../pl"), 0);
    expect("dotdot_made", a_stat_path(D "/pl", &st), 0);
    check("pl_is_f", st.st_ino == want.st_ino, 0);

    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_gone", a_rmdir(D "/gone"), 0);
    expect("removed_cwd_enoent", syscall2(SYS_symlink, "x", "l"), -ENOENT);
    expect("removed_cwd_dot_enoent", syscall2(SYS_symlink, "x", "./l"), -ENOENT);
    expect("removed_cwd_absolute_ok", syscall2(SYS_symlink, "x", D "/abs"), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    const char *names[] = {D "/rl", D "/dl", D "/sub/lf", D "/sub/up", D "/pl", D "/abs", D "/f"};
    int ok = 1;
    for (int i = 0; i < 7; i++) ok &= a_unlink(names[i]) == 0;
    ok &= a_rmdir(D "/sub") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
