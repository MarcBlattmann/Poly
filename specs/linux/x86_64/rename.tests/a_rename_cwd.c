/* rename with relative paths: they start at the working directory, and
 * ".." in them climbs out of it. The working directory itself can be
 * renamed: the process stays in it, getcwd reports the new path, and an
 * open descriptor on a renamed directory keeps working as a dirfd. "."
 * as either path is EBUSY. */
#include "a_sys.h"

#define D "/tmp/a_rename_cwd"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

int test_main(void) {
    struct a_stat st;
    char buf[128];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_w", a_mkdir(D "/w", 0755), 0);
    expect("setup_f", a_make_file(D "/w/f", "", 0644), 0);
    expect("setup_top", a_make_file(D "/top", "", 0644), 0);
    i64 dfd = a_open(D "/w", O_RDONLY | O_DIRECTORY, 0);
    check("open_dir", dfd >= 0, dfd);
    expect("chdir_w", syscall1(SYS_chdir, D "/w"), 0);

    expect("relative_both", rn("f", "g"), 0);
    expect("relative_moved", a_stat_path(D "/w/g", &st), 0);
    expect("relative_to_absolute", rn("g", D "/g"), 0);
    expect("absolute_to_relative", rn(D "/g", "h"), 0);
    expect("absolute_to_relative_moved", a_stat_path(D "/w/h", &st), 0);
    expect("dotdot_source", rn("../top", "top"), 0);
    expect("dotdot_moved_in", a_stat_path(D "/w/top", &st), 0);
    expect("dotdot_target", rn("top", "../top2"), 0);
    expect("dotdot_moved_out", a_stat_path(D "/top2", &st), 0);
    expect("dot_slash_prefix", rn("./h", "./sub/../i"), -ENOENT);
    expect("setup_sub", a_mkdir("sub", 0755), 0);
    expect("dot_slash_and_dotdot", rn("./h", "./sub/../i"), 0);
    expect("dot_slash_moved", a_stat_path(D "/w/i", &st), 0);

    expect("rename_cwd", rn(D "/w", D "/w2"), 0);
    i64 n = syscall2(SYS_getcwd, buf, sizeof buf);
    expect("getcwd_len", n, sizeof(D "/w2"));
    check("getcwd_new_path", n > 0 && str_eq(buf, D "/w2"), 0);
    expect("relative_after_cwd_renamed", rn("i", "j"), 0);
    expect("seen_under_new_path", a_stat_path(D "/w2/j", &st), 0);
    expect("dirfd_after_rename", syscall4(SYS_newfstatat, dfd, "j", &st, 0), 0);
    expect("renameat_via_dirfd", syscall4(SYS_renameat, dfd, "j", dfd, "k"), 0);
    expect("renameat_seen", a_stat_path(D "/w2/k", &st), 0);
    expect("dot_source", rn(".", "../w3"), -EBUSY);
    expect("dot_target", rn("k", "."), -EBUSY);

    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    a_close(dfd);
    expect("cleanup_k", a_unlink(D "/w2/k"), 0);
    expect("cleanup_sub", a_rmdir(D "/w2/sub"), 0);
    expect("cleanup_w2", a_rmdir(D "/w2"), 0);
    expect("cleanup_top2", a_unlink(D "/top2"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
