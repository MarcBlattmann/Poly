/* lchown(2): path errors and their order, relative paths, search
 * permission on the way (root passes it), ctime of the link. */
#include "a_sys.h"

#define D "/tmp/lb_lchown_paths"
#define SYS_lchown 94

static i64 lchown_(const char *p, i64 u, i64 g) { return syscall3(SYS_lchown, p, u, g); }

static char buf[8192];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_stat st, st2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner_link", syscall2(SYS_symlink, "../f", D "/sub/l"), 0);
    expect("setup_loop1", syscall2(SYS_symlink, "loop2", D "/loop1"), 0);
    expect("setup_loop2", syscall2(SYS_symlink, "loop1", D "/loop2"), 0);

    expect("missing_enoent", lchown_(D "/nope", -1, -1), -ENOENT);
    expect("missing_dir_enoent", lchown_(D "/nope/l", -1, -1), -ENOENT);
    expect("empty_enoent", lchown_("", -1, -1), -ENOENT);
    expect("file_as_dir_enotdir", lchown_(D "/f/x", -1, -1), -ENOTDIR);
    expect("loop_last_ok", lchown_(D "/loop1", -1, -1), 0);
    expect("loop_middle_eloop", lchown_(D "/loop1/x", -1, -1), -ELOOP);
    expect("null_efault", lchown_((const char *)0, -1, -1), -EFAULT);
    expect("addr1_efault", lchown_((const char *)1, -1, -1), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'l';
    p[256] = 0;
    expect("name_256_enametoolong", lchown_(buf, -1, -1), -ENAMETOOLONG);
    p[255] = 0;
    expect("name_255_enoent", lchown_(buf, -1, -1), -ENOENT);
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("path_4096_enametoolong", lchown_(buf, -1, -1), -ENAMETOOLONG);
    p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 'l';
    p[256] = 0;
    expect("order_missing_before_long", lchown_(buf, -1, -1), -ENOENT);

    expect("chdir", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative_link", lchown_("l", -1, -1), 0);
    expect("relative_dotdot", lchown_("../loop2", -1, -1), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    /* ctime of the link itself changes; the target's does not */
    expect("lstat_l0", a_lstat_path(D "/sub/l", &st), 0);
    expect("stat_f0", a_stat_path(D "/f", &st2), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("lchown_link", lchown_(D "/sub/l", -1, -1), 0);
    struct a_stat st3, st4;
    expect("lstat_l1", a_lstat_path(D "/sub/l", &st3), 0);
    expect("stat_f1", a_stat_path(D "/f", &st4), 0);
    check("link_ctime_advanced", st3.st_ctime_sec > st.st_ctime_sec ||
                                     (st3.st_ctime_sec == st.st_ctime_sec && st3.st_ctime_nsec > st.st_ctime_nsec), 0);
    check("target_ctime_same", st4.st_ctime_sec == st2.st_ctime_sec && st4.st_ctime_nsec == st2.st_ctime_nsec, 0);

    expect("sub_0600", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("through_closed", lchown_(D "/sub/l", -1, -1), root ? 0 : -EACCES);
    expect("sub_0755", syscall2(SYS_chmod, D "/sub", 0755), 0);

    expect("cleanup_loop2", a_unlink(D "/loop2"), 0);
    expect("cleanup_loop1", a_unlink(D "/loop1"), 0);
    expect("cleanup_inner_link", a_unlink(D "/sub/l"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
