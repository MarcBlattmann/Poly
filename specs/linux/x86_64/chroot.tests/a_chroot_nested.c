/* chroot may be called again inside a chroot: the new path is resolved
 * from the current root (absolute) or the current directory (relative),
 * so roots nest. chroot("/") and chroot(".") at the root change nothing
 * visible. The previous root cannot be reached by absolute paths or "..".
 * A current directory left outside the new root is reported by getcwd
 * as "(unreachable)" followed by its full path from the real root.
 * poly-diff: needs=root (only a process with CAP_SYS_CHROOT may chroot)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define D "/tmp/a_chroot_nested"

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

static struct a_stat a0, b0, c0;

static int in_child(void (*body)(void)) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        body();
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 status = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return -1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 128 + A_WTERMSIG(status);
}

static int cwd_is(const char *want) {
    char buf[128];
    i64 r = syscall2(SYS_getcwd, buf, sizeof buf);
    return r == (i64)str_len(want) + 1 && str_eq(buf, want);
}

static void body(void) {
    struct a_stat st;
    i64 r = syscall1(SYS_chroot, D "/a");
    expect("chroot_a", r, 0);
    if (r != 0) return;
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("root_is_a", a_stat_path("/", &st) == 0 && same(&st, &a0), 1);
    expect("chroot_slash_noop", syscall1(SYS_chroot, "/"), 0);
    expect("chroot_dot_noop", syscall1(SYS_chroot, "."), 0);
    expect("root_still_a", a_stat_path("/", &st) == 0 && same(&st, &a0), 1);
    r = syscall1(SYS_chroot, "/b");
    expect("chroot_b_absolute", r, 0);
    if (r != 0) return;
    expect("root_is_b", a_stat_path("/", &st) == 0 && same(&st, &b0), 1);
    expect("b_marker", a_stat_path("/mb", &st), 0);
    expect("a_marker_gone", a_stat_path("/ma", &st), -ENOENT);
    check("getcwd_unreachable_a", cwd_is("(unreachable)" D "/a"), 0);
    expect("chdir_into_b", syscall1(SYS_chdir, "/"), 0);
    check("getcwd_root", cwd_is("/"), 0);
    expect("dotdot_stays_b", a_stat_path("..", &st) == 0 && same(&st, &b0), 1);
    r = syscall1(SYS_chroot, "c");
    expect("chroot_c_relative", r, 0);
    if (r != 0) return;
    expect("root_is_c", a_stat_path("/", &st) == 0 && same(&st, &c0), 1);
    check("getcwd_b_unreachable", cwd_is("(unreachable)" D "/a/b"), 0);
    expect("chdir_c_root", syscall1(SYS_chdir, "/"), 0);
    expect("dotdot_stays_c", a_stat_path("/../..", &st) == 0 && same(&st, &c0), 1);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_mkdir(D "/a", 0755), 0);
    expect("setup_b", a_mkdir(D "/a/b", 0755), 0);
    expect("setup_c", a_mkdir(D "/a/b/c", 0755), 0);
    expect("setup_ma", a_make_file(D "/a/ma", "", 0644), 0);
    expect("setup_mb", a_make_file(D "/a/b/mb", "", 0644), 0);
    expect("stat_a", a_stat_path(D "/a", &a0), 0);
    expect("stat_b", a_stat_path(D "/a/b", &b0), 0);
    expect("stat_c", a_stat_path(D "/a/b/c", &c0), 0);
    expect("child", in_child(body), 0);
    expect("cleanup_mb", a_unlink(D "/a/b/mb"), 0);
    expect("cleanup_ma", a_unlink(D "/a/ma"), 0);
    expect("cleanup_c", a_rmdir(D "/a/b/c"), 0);
    expect("cleanup_b", a_rmdir(D "/a/b"), 0);
    expect("cleanup_a", a_rmdir(D "/a"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
