/* Inside a chroot, symbolic links are resolved against the new root: an
 * absolute target starts at the new root, ".." in a target stops there,
 * and a target naming a path that exists only outside is not found. The
 * text of a link is not changed. chroot itself follows links, so chroot
 * of a link to a directory makes that directory the root.
 * poly-diff: needs=root (only a process with CAP_SYS_CHROOT may chroot)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define D "/tmp/a_chroot_symlinks"

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

static struct a_stat target0;

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

static void body(void) {
    struct a_stat st;
    char text[64];
    i64 r = syscall1(SYS_chroot, D);
    expect("chroot_ok", r, 0);
    if (r != 0) return;
    expect("abs_link_inside", a_stat_path("/abs/f", &st) == 0 ? st.st_size : -1, 4);
    expect("up_link_stops_at_root", a_stat_path("/up/f", &st) == 0 ? st.st_size : -1, 4);
    expect("rel_link", a_stat_path("/rel/f", &st) == 0 ? st.st_size : -1, 4);
    expect("outside_target_enoent", a_stat_path("/out", &st), -ENOENT);
    expect("outside_link_itself_there", a_lstat_path("/out", &st), 0);
    r = syscall4(SYS_readlinkat, AT_FDCWD, "/abs", text, sizeof text);
    expect("readlink_unchanged", r, 7);
    check("readlink_text", r == 7 && text[0] == '/' && text[1] == 't' && text[6] == 't', 0);
    r = syscall1(SYS_chroot, "/abs");
    expect("chroot_via_abs_link", r, 0);
    if (r != 0) return;
    expect("root_is_target", a_stat_path("/", &st) == 0 && same(&st, &target0), 1);
    expect("f_at_root", a_stat_path("/f", &st) == 0 ? st.st_size : -1, 4);
    expect("abs_link_gone", a_lstat_path("/abs", &st), -ENOENT);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_target", a_mkdir(D "/target", 0755), 0);
    expect("setup_f", a_make_file(D "/target/f", "four", 0644), 0);
    expect("setup_abs", syscall3(SYS_symlinkat, "/target", AT_FDCWD, D "/abs"), 0);
    expect("setup_up", syscall3(SYS_symlinkat, "../../../../target", AT_FDCWD, D "/up"), 0);
    expect("setup_rel", syscall3(SYS_symlinkat, "target", AT_FDCWD, D "/rel"), 0);
    expect("setup_out", syscall3(SYS_symlinkat, D "/target", AT_FDCWD, D "/out"), 0);
    expect("stat_target", a_stat_path(D "/target", &target0), 0);
    expect("child", in_child(body), 0);
    expect("cleanup_out", a_unlink(D "/out"), 0);
    expect("cleanup_rel", a_unlink(D "/rel"), 0);
    expect("cleanup_up", a_unlink(D "/up"), 0);
    expect("cleanup_abs", a_unlink(D "/abs"), 0);
    expect("cleanup_f", a_unlink(D "/target/f"), 0);
    expect("cleanup_target", a_rmdir(D "/target"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
