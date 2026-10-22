/* A successful chroot returns 0 and from then on "/" in that process is
 * the given directory: absolute paths resolve inside it, names outside it
 * are not found. The current directory is not changed (here it stays at
 * the old "/", outside the new root, so getcwd reports
 * "(unreachable)/"). Only the calling process changes: the parent still
 * has its old root. Root may chroot into a directory of mode 0 (its
 * permission override covers the search check).
 * poly-diff: needs=root (only a process with CAP_SYS_CHROOT may chroot)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define D "/tmp/a_chroot_basic"

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

static struct a_stat d0, old_root, m0;

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

static void into_d(void) {
    struct a_stat st;
    char cwd[64];
    i64 r = syscall1(SYS_chroot, D);
    expect("chroot_ok", r, 0);
    if (r != 0) return;
    expect("root_is_d", a_stat_path("/", &st) == 0 && same(&st, &d0), 1);
    expect("marker_at_root", a_stat_path("/marker", &st) == 0 ? st.st_size : -1, 2);
    expect("outside_name_enoent", a_stat_path(D "/marker", &st), -ENOENT);
    expect("tmp_enoent", a_stat_path("/tmp", &st), -ENOENT);
    expect("cwd_still_old_root", a_stat_path(".", &st) == 0 && same(&st, &old_root), 1);
    expect("getcwd_unreachable", syscall2(SYS_getcwd, cwd, sizeof cwd), 15);
    check("getcwd_text", str_eq(cwd, "(unreachable)/"), 0);
    expect("relative_from_cwd", a_stat_path("tmp/a_chroot_basic/marker", &st) == 0 ? st.st_size : -1, 2);
    i64 fd = a_open("/made", O_WRONLY | O_CREAT | O_EXCL, 0644);
    check("create_at_new_root", fd >= 0, fd);
    a_close(fd);
}

static void into_mode0(void) {
    struct a_stat st;
    i64 r = syscall1(SYS_chroot, D "/m0");
    expect("chroot_mode0_ok", r, 0);
    if (r != 0) return;
    expect("root_is_m0", a_stat_path("/", &st) == 0 && same(&st, &m0), 1);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_marker", a_make_file(D "/marker", "in", 0644), 0);
    expect("setup_m0", a_mkdir(D "/m0", 0), 0);
    expect("stat_d", a_stat_path(D, &d0), 0);
    expect("stat_m0", a_stat_path(D "/m0", &m0), 0);
    expect("stat_root", a_stat_path("/", &old_root), 0);

    expect("child_into_d", in_child(into_d), 0);
    expect("child_into_mode0", in_child(into_mode0), 0);

    expect("parent_root_unchanged", a_stat_path("/", &st) == 0 && same(&st, &old_root), 1);
    expect("parent_marker_enoent", a_stat_path("/marker", &st), -ENOENT);
    expect("made_inside_d", a_stat_path(D "/made", &st), 0);

    a_unlink(D "/made");
    expect("cleanup_marker", a_unlink(D "/marker"), 0);
    expect("cleanup_m0", a_rmdir(D "/m0"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
