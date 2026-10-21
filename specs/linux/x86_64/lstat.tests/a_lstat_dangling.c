/* lstat: dangling and looping links are reported as links; only a
 * trailing slash or a later component makes lstat follow them. */
#include "a_sys.h"

#define D "/tmp/a_lstat_dangling"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }
static i64 symlink(const char *t, const char *l) { return syscall3(SYS_symlinkat, t, AT_FDCWD, l); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_dangling", symlink("missing-target", D "/dang"), 0);
    expect("setup_loop_a", symlink("b", D "/a"), 0);
    expect("setup_loop_b", symlink("a", D "/b"), 0);
    expect("setup_self", symlink("self", D "/self"), 0);
    expect("dangling_ok", lstat(D "/dang", &st), 0);
    expect("dangling_is_link", st.st_mode & S_IFMT, S_IFLNK);
    expect("dangling_size", st.st_size, 14);
    expect("dangling_stat_enoent", syscall2(SYS_stat, D "/dang", &st), -ENOENT);
    expect("dangling_slash_enoent", lstat(D "/dang/", &st), -ENOENT);
    expect("dangling_child_enoent", lstat(D "/dang/x", &st), -ENOENT);
    expect("loop_ok", lstat(D "/a", &st), 0);
    expect("loop_is_link", st.st_mode & S_IFMT, S_IFLNK);
    expect("loop_size", st.st_size, 1);
    expect("self_ok", lstat(D "/self", &st), 0);
    expect("self_size", st.st_size, 4);
    expect("loop_stat_eloop", syscall2(SYS_stat, D "/a", &st), -ELOOP);
    expect("loop_slash_eloop", lstat(D "/a/", &st), -ELOOP);
    expect("loop_child_eloop", lstat(D "/a/x", &st), -ELOOP);
    expect("self_child_eloop", lstat(D "/self/x", &st), -ELOOP);
    expect("cleanup_dang", a_unlink(D "/dang"), 0);
    expect("cleanup_a", a_unlink(D "/a"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_self", a_unlink(D "/self"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
