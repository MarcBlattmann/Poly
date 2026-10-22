/* chroot closes nothing: descriptors opened before the call keep working
 * and still refer to what they referred to, even outside the new root.
 * Lookups relative to such a directory descriptor are not confined: ".."
 * from it climbs above the new root. fchdir to it moves the current
 * directory outside (getcwd then has the "(unreachable)" prefix); a
 * chdir to "/" brings it back to the new root.
 * poly-diff: needs=root (only a process with CAP_SYS_CHROOT may chroot)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define D "/tmp/a_chroot_open_fds"

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

static struct a_stat tmp0, r0;

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
    char buf[16];
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 ffd = a_open(D "/outside", O_RDWR, 0);
    check("open_dir", dfd >= 0, dfd);
    check("open_file", ffd >= 0, ffd);
    expect("chdir_r", syscall1(SYS_chdir, D "/r"), 0);
    i64 r = syscall1(SYS_chroot, ".");
    expect("chroot_dot_ok", r, 0);
    if (r != 0) return;
    check("getcwd_root", cwd_is("/"), 0);
    expect("outside_path_enoent", a_stat_path(D "/outside", &st), -ENOENT);
    expect("file_fd_reads", a_read(ffd, buf, sizeof buf), 7);
    expect("file_fd_writes", a_write(ffd, "+", 1), 1);
    expect("dirfd_name", syscall4(SYS_newfstatat, dfd, "outside", &st, 0) == 0 ? st.st_size : -1, 8);
    expect("dirfd_dotdot_climbs", syscall4(SYS_newfstatat, dfd, "..", &st, 0) == 0 && same(&st, &tmp0), 1);
    i64 nfd = syscall4(SYS_openat, dfd, "new", O_WRONLY | O_CREAT | O_EXCL, 0644);
    check("dirfd_create", nfd >= 0, nfd);
    a_close(nfd);
    expect("dirfd_absolute_uses_root", syscall4(SYS_newfstatat, dfd, "/inner", &st, 0), 0);
    expect("fchdir_out", syscall1(SYS_fchdir, dfd), 0);
    check("getcwd_unreachable", cwd_is("(unreachable)" D), 0);
    expect("relative_outside", a_stat_path("outside", &st), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    check("getcwd_root_again", cwd_is("/"), 0);
    expect("cwd_is_r", a_stat_path(".", &st) == 0 && same(&st, &r0), 1);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_r", a_mkdir(D "/r", 0755), 0);
    expect("setup_inner", a_make_file(D "/r/inner", "i", 0644), 0);
    expect("setup_outside", a_make_file(D "/outside", "outside", 0644), 0);
    expect("stat_tmp", a_stat_path("/tmp", &tmp0), 0);
    expect("stat_r", a_stat_path(D "/r", &r0), 0);
    expect("child", in_child(body), 0);
    expect("write_landed", a_stat_path(D "/outside", &st) == 0 ? st.st_size : -1, 8);
    expect("created_outside", a_stat_path(D "/new", &st), 0);
    a_unlink(D "/new");
    expect("cleanup_outside", a_unlink(D "/outside"), 0);
    expect("cleanup_inner", a_unlink(D "/r/inner"), 0);
    expect("cleanup_r", a_rmdir(D "/r"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
