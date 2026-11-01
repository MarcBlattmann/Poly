/* chroot changes only the root used for name resolution; it does not
 * close or confine descriptors or the working directory. Following a
 * symlink to a directory is fine (chroot of "link" -> "sub" chroots to
 * sub). A process holding a descriptor of a directory outside the new
 * root can fchdir there; ".." from a working directory outside the root
 * climbs up to the real root (the new root is only a barrier when ".."
 * starts at it), and chroot(".") there undoes the confinement. getcwd
 * shows a working directory outside the root as "(unreachable)"
 * followed by its path from the real (initial) root.
 * chroot(".") with the working directory at the root is allowed. Each
 * case runs in a child process.
 * poly-diff: needs=root (chroot needs CAP_SYS_CHROOT)
 */
#include "a_sys.h"

#define SYS_chroot 161
#define DIR "/tmp/ld_chroot_escape"

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

static int escape(void) {
    i64 out = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("outside_fd_3", out, 3);
    expect("chroot_via_symlink", syscall1(SYS_chroot, DIR "/link"), 0);
    expect("new_root_is_sub", a_stat_path("/file", &st), 0);
    expect("dir_hidden", a_stat_path(DIR, &st), -ENOENT);
    expect("fchdir_outside", syscall1(SYS_fchdir, out), 0);
    check("cwd_unreachable_tmp", cwd_is("(unreachable)/tmp"), 0);
    expect("relative_from_outside_cwd", a_stat_path("ld_chroot_escape/sub/file", &st), 0);
    expect("climb_to_real_root", syscall1(SYS_chdir, "../../.."), 0);
    check("cwd_unreachable_slash", cwd_is("(unreachable)/"), 0);
    expect("chroot_dot_escapes", syscall1(SYS_chroot, "."), 0);
    check("cwd_slash_again", cwd_is("/"), 0);
    expect("real_path_visible", a_stat_path(DIR "/sub/file", &st), 0);
    return failed;
}

static int nested(void) {
    expect("chdir_dir", syscall1(SYS_chdir, DIR), 0);
    expect("chroot_dot", syscall1(SYS_chroot, "."), 0);
    expect("chroot_dot_again", syscall1(SYS_chroot, "."), 0);
    expect("chroot_relative_sub", syscall1(SYS_chroot, "sub"), 0);
    /* The working directory (old root) is now outside the root; getcwd
     * names it by its path from the real root. */
    check("cwd_outside", cwd_is("(unreachable)" DIR), 0);
    expect("relative_still_old", a_stat_path("sub/file", &st), 0);
    expect("absolute_new", a_stat_path("/file", &st), 0);
    expect("absolute_old_enoent", a_stat_path("/sub/file", &st), -ENOENT);
    expect("chroot_missing_enoent", syscall1(SYS_chroot, "/none"), -ENOENT);
    expect("chroot_file_enotdir", syscall1(SYS_chroot, "/file"), -ENOTDIR);
    return failed;
}

static void cleanup(void) {
    a_unlink(DIR "/sub/file");
    a_unlink(DIR "/link");
    a_rmdir(DIR "/sub");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_mkdir(DIR "/sub", 0755);
    expect("file", a_make_file(DIR "/sub/file", "x", 0644), 0);
    syscall2(SYS_symlink, "sub", DIR "/link");
    a_block_sigchld();
    expect("child_escape", run_child(escape), 0);
    expect("child_nested", run_child(nested), 0);
    cleanup();
    return finish();
}
