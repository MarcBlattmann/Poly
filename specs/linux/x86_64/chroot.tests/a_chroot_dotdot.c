/* After chroot and a chdir into the new root, ".." of the new root is
 * the new root itself: "/..", "/../..", "sub/../.." and the like never
 * leave it, and getcwd reports paths relative to it ("/", "/sub").
 * poly-diff: needs=root (only a process with CAP_SYS_CHROOT may chroot)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define D "/tmp/a_chroot_dotdot"

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

static struct a_stat d0, sub0;

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
    i64 r = syscall1(SYS_chroot, D);
    expect("chroot_ok", r, 0);
    if (r != 0) return;
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    check("getcwd_root", cwd_is("/"), 0);
    expect("dotdot_is_root", a_stat_path("..", &st) == 0 && same(&st, &d0), 1);
    expect("slash_dotdot_is_root", a_stat_path("/..", &st) == 0 && same(&st, &d0), 1);
    expect("many_dotdot_is_root", a_stat_path("/../../../..", &st) == 0 && same(&st, &d0), 1);
    expect("dotdot_then_name", a_stat_path("/../../marker", &st) == 0 ? st.st_size : -1, 1);
    expect("sub_dotdot_dotdot", a_stat_path("/sub/../..", &st) == 0 && same(&st, &d0), 1);
    expect("chdir_dotdots", syscall1(SYS_chdir, "../../.."), 0);
    check("getcwd_still_root", cwd_is("/"), 0);
    expect("chdir_sub", syscall1(SYS_chdir, "sub"), 0);
    check("getcwd_sub", cwd_is("/sub"), 0);
    expect("sub_dotdot_is_root", a_stat_path("..", &st) == 0 && same(&st, &d0), 1);
    expect("sub_up_up_marker", a_stat_path("../../../marker", &st) == 0 ? st.st_size : -1, 1);
    expect("chdir_up_up", syscall1(SYS_chdir, "../../.."), 0);
    check("getcwd_back_at_root", cwd_is("/"), 0);
    expect("cwd_is_d", a_stat_path(".", &st) == 0 && same(&st, &d0), 1);
    expect("sub_seen", a_stat_path("/sub", &st) == 0 && same(&st, &sub0), 1);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_marker", a_make_file(D "/marker", "m", 0644), 0);
    expect("stat_d", a_stat_path(D, &d0), 0);
    expect("stat_sub", a_stat_path(D "/sub", &sub0), 0);
    expect("child", in_child(body), 0);
    expect("cleanup_marker", a_unlink(D "/marker"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
