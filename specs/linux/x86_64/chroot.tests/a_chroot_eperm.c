/* chroot without CAP_SYS_CHROOT: the test first empties all three
 * capability sets with capset (any user may do that, so root and a normal
 * user end up the same). Then chroot of any directory that resolves and
 * may be searched is EPERM (1): "/", ".", "..", "/tmp", an absolute or
 * relative directory, trailing and doubled slashes, a symbolic link to a
 * directory. A failed call changes nothing: the current directory and the
 * root directory stay what they were. */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_chroot 161
#define D "/tmp/a_chroot_eperm"

static i64 drop_caps(void) {
    u32 hdr[2] = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3, this process */
    u32 data[6] = { 0, 0, 0, 0, 0, 0 }; /* effective, permitted, inheritable x 2 */
    return syscall2(SYS_capset, hdr, data);
}

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_dev == b->st_dev && a->st_ino == b->st_ino;
}

int test_main(void) {
    struct a_stat root0, d0, st;
    char cwd[64];
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/link"), 0);
    expect("stat_root", a_stat_path("/", &root0), 0);
    expect("stat_d", a_stat_path(D, &d0), 0);

    expect("root_eperm", syscall1(SYS_chroot, "/"), -EPERM);
    expect("dot_eperm", syscall1(SYS_chroot, "."), -EPERM);
    expect("dotdot_eperm", syscall1(SYS_chroot, ".."), -EPERM);
    expect("tmp_eperm", syscall1(SYS_chroot, "/tmp"), -EPERM);
    expect("dir_eperm", syscall1(SYS_chroot, D), -EPERM);
    expect("trailing_slash_eperm", syscall1(SYS_chroot, D "/"), -EPERM);
    expect("double_slashes_eperm", syscall1(SYS_chroot, "/tmp//a_chroot_eperm//sub//"), -EPERM);
    expect("dot_components_eperm", syscall1(SYS_chroot, D "/./sub/../sub/."), -EPERM);
    expect("symlink_to_dir_eperm", syscall1(SYS_chroot, D "/link"), -EPERM);

    expect("chdir_d", syscall1(SYS_chdir, D), 0);
    expect("relative_eperm", syscall1(SYS_chroot, "sub"), -EPERM);
    expect("relative_dot_eperm", syscall1(SYS_chroot, "."), -EPERM);
    expect("cwd_unchanged", syscall2(SYS_getcwd, cwd, sizeof cwd), (i64)sizeof D);
    check("cwd_text", str_eq(cwd, D), 0);
    expect("dot_still_d", a_stat_path(".", &st) == 0 && same(&st, &d0), 1);
    expect("root_unchanged", a_stat_path("/", &st) == 0 && same(&st, &root0), 1);
    expect("absolute_paths_work", a_stat_path(D "/sub", &st), 0);

    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_link", a_unlink(D "/link"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
