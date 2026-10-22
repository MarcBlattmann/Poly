/* Relative paths start at the working directory, which follows chdir and
 * fchdir; "." and ".." work as names, and ".." from "/" stays at "/". */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define D "/tmp/a_lchown_relative"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static void nap(void) {
    struct a_timespec t = {0, 30000000};
    syscall2(SYS_nanosleep, &t, 0);
}
static int ctime_changed(const char *p, const struct a_stat *before) {
    struct a_stat st;
    a_lstat_path(p, &st);
    return st.st_ctime_sec != before->st_ctime_sec || st.st_ctime_nsec != before->st_ctime_nsec;
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/sub/l"), 0);

    /* The test starts in "/". */
    expect("from_root_dir", lchown("tmp/a_lchown_relative/sub/f", uid, gid), 0);
    expect("from_root_dotdot", lchown("../../tmp/a_lchown_relative/sub/f", -1, -1), 0);

    expect("chdir", syscall1(SYS_chdir, D), 0);
    a_lstat_path(D "/sub/f", &st);
    nap();
    expect("rel_file", lchown("sub/f", uid, gid), 0);
    check("rel_file_changed", ctime_changed(D "/sub/f", &st), 0);
    expect("rel_dot_prefix", lchown("./sub/./f", -1, -1), 0);
    expect("rel_missing_enoent", lchown("f", -1, -1), -ENOENT);
    a_lstat_path(D "/sub/l", &st);
    struct a_stat ft;
    a_lstat_path(D "/sub/f", &ft);
    nap();
    expect("rel_link", lchown("sub/l", uid, gid), 0);
    check("rel_link_changed", ctime_changed(D "/sub/l", &st), 0);
    check("rel_link_target_untouched", !ctime_changed(D "/sub/f", &ft), 0);
    expect("rel_dot", lchown(".", -1, -1), 0);

    i64 fd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("open_sub", fd >= 0, fd);
    expect("fchdir", syscall1(SYS_fchdir, fd), 0);
    a_close(fd);
    expect("after_fchdir_file", lchown("f", -1, -1), 0);
    expect("after_fchdir_old_name_enoent", lchown("sub/f", -1, -1), -ENOENT);
    expect("after_fchdir_dotdot", lchown("../sub/l", -1, -1), 0);

    /* The working directory removed: relative names find nothing. */
    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_gone", a_rmdir(D "/gone"), 0);
    expect("removed_cwd_name_enoent", lchown("x", -1, -1), -ENOENT);
    expect("removed_cwd_dotdot", lchown("../sub/f", -1, -1), 0);

    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("root_dotdot_stays", lchown("../../../tmp/a_lchown_relative/sub/f", -1, -1), 0);
    expect("cleanup_l", a_unlink(D "/sub/l"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
