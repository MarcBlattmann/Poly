/* rmdir of the working directory, by its absolute name, succeeds. The
 * process stays in the removed directory: getcwd fails with ENOENT,
 * rmdir(".") is EINVAL, creating anything relative to it is ENOENT, a
 * missing name is ENOENT, ".." still leads to the old parent (so
 * "../y" can be removed and chdir("..") works). Removing a directory
 * that is the working directory of a child process works too, and the
 * child sees the same.
 * poly-diff: syscalls=off (a child process)
 */
#include "a_sys.h"

#define D "/tmp/c_rmdir_cwd"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }
static char cwd[256];

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_w", a_mkdir(D "/w", 0755), 0);
    expect("setup_y", a_mkdir(D "/y", 0755), 0);
    expect("chdir_w", syscall1(SYS_chdir, D "/w"), 0);
    expect("getcwd_before", syscall2(SYS_getcwd, cwd, sizeof cwd), (i64)sizeof(D "/w"));

    expect("rmdir_cwd", rm(D "/w"), 0);
    expect("gone", a_lstat_path(D "/w", &st), -ENOENT);
    expect("getcwd_enoent", syscall2(SYS_getcwd, cwd, sizeof cwd), -ENOENT);
    expect("stat_dot", a_stat_path(".", &st), 0);
    expect("dot_nlink_0", st.st_nlink, 0);
    expect("rmdir_dot_einval", rm("."), -EINVAL);
    expect("rmdir_missing_enoent", rm("nope"), -ENOENT);
    expect("mkdir_rel_enoent", a_mkdir("new", 0755), -ENOENT);
    expect("create_rel_enoent", a_make_file("f", "x", 0644), -ENOENT);
    expect("rmdir_dotdot_enotempty", rm(".."), -ENOTEMPTY);
    expect("rmdir_dotdot_y", rm("../y"), 0);
    expect("y_gone", a_lstat_path(D "/y", &st), -ENOENT);
    expect("chdir_dotdot", syscall1(SYS_chdir, ".."), 0);
    expect("getcwd_parent", syscall2(SYS_getcwd, cwd, sizeof cwd), (i64)sizeof(D));
    expect("getcwd_parent_text", str_eq(cwd, D), 1);

    /* A child's working directory. */
    expect("setup_c", a_mkdir(D "/c", 0755), 0);
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    sh[2] = 0;
    a_block_sigchld();
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        syscall1(SYS_chdir, D "/c");
        sh[0] = 1;
        while (sh[1] == 0) a_relax();
        sh[2] = (i32)syscall2(SYS_getcwd, cwd, sizeof cwd);
        sh[3] = (i32)a_mkdir("z", 0755);
        exit_group(0);
    }
    expect("child_ready", a_spin_until(sh, 1, A_SPIN_LIMIT), 1);
    expect("rmdir_child_cwd", rm(D "/c"), 0);
    sh[1] = 1;
    i32 status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_getcwd_enoent", sh[2], -ENOENT);
    expect("child_mkdir_enoent", sh[3], -ENOENT);

    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
