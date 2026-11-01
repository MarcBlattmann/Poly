/* chroot(path) as root makes the directory the calling process's root
 * directory for path resolution: absolute paths start there, ".." at it
 * stays at it, absolute symlink targets resolve inside it, and what lies
 * outside is no longer reachable by name (ENOENT). The working directory
 * is not changed. A child forked afterwards inherits the root; the
 * parent (which did not call chroot) keeps its own. Each case runs in a
 * child process, since a chroot cannot be undone.
 * poly-diff: needs=root (chroot needs CAP_SYS_CHROOT)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define DIR "/tmp/ld_chroot_root"

static struct a_stat st;

static int cwd_is(const char *want) {
    char b[256];
    i64 r = syscall2(SYS_getcwd, b, sizeof b);
    return r == (i64)str_len(want) + 1 && str_eq(b, want);
}

static int run_child(int (*fn)(void)) {
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(fn() ? 1 : 0);
    i32 status = -1;
    if (syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return -1;
    return status;
}

static int grandchild(void) {
    expect("inherited_root", a_stat_path("/sub/file", &st), 0);
    expect("inherited_outside_enoent", a_stat_path(DIR, &st), -ENOENT);
    return failed;
}

static int inside(void) {
    expect("chdir_dir", syscall1(SYS_chdir, DIR), 0);
    expect("chroot_dot", syscall1(SYS_chroot, "."), 0);
    check("cwd_is_slash", cwd_is("/"), 0);
    expect("absolute_inside", a_stat_path("/sub/file", &st), 0);
    expect("relative_inside", a_stat_path("sub/file", &st), 0);
    expect("outside_enoent", a_stat_path(DIR "/sub/file", &st), -ENOENT);
    expect("dotdot_stays", a_stat_path("/../../sub/file", &st), 0);
    expect("relative_dotdot_stays", a_stat_path("../../../sub/file", &st), 0);
    expect("absolute_symlink_inside", a_stat_path("/abslink/file", &st), 0);
    expect("chdir_sub", syscall1(SYS_chdir, "sub"), 0);
    check("cwd_sub", cwd_is("/sub"), 0);
    expect("chdir_up_past_root", syscall1(SYS_chdir, "../../.."), 0);
    check("cwd_back_at_slash", cwd_is("/"), 0);
    expect("create_inside", a_make_file("/made", "m", 0644), 0);
    expect("grandchild", run_child(grandchild), 0);
    return failed;
}

static int keep_cwd(void) {
    /* The working directory stays where it was: "/" of the old root. */
    expect("chroot_abs", syscall1(SYS_chroot, DIR), 0);
    expect("relative_from_old_cwd", a_stat_path("tmp/ld_chroot_root/sub/file", &st), 0);
    expect("absolute_new_root", a_stat_path("/sub/file", &st), 0);
    expect("chdir_slash", syscall1(SYS_chdir, "/"), 0);
    expect("old_path_now_enoent", a_stat_path("tmp/ld_chroot_root/sub/file", &st), -ENOENT);
    check("cwd_slash", cwd_is("/"), 0);
    return failed;
}

static void cleanup(void) {
    a_unlink(DIR "/sub/file");
    a_unlink(DIR "/abslink");
    a_unlink(DIR "/made");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    expect("file", a_make_file(DIR "/sub/file", "x", 0644), 0);
    syscall2(SYS_symlink, "/sub", DIR "/abslink");
    a_block_sigchld();
    expect("child_inside", run_child(inside), 0);
    expect("made_in_dir", a_stat_path(DIR "/made", &st), 0);
    expect("child_keep_cwd", run_child(keep_cwd), 0);
    expect("parent_root_unchanged", a_stat_path(DIR "/sub/file", &st), 0);
    expect("parent_tmp_visible", a_stat_path("/tmp", &st), 0);
    cleanup();
    return finish();
}
