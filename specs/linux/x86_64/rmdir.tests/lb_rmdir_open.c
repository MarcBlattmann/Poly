/* rmdir(2) of a directory that is open or is the working directory: the
 * removal succeeds; the descriptor stays valid with st_nlink 0, reading
 * its entries is ENOENT, nothing can be created in it (ENOENT); "." still
 * stats; a removed working directory can be left with chdir(".."). */
#include "a_sys.h"

#define D "/tmp/lb_rmdir_open"

static i64 rmdir_(const char *p) { return syscall1(SYS_rmdir, p); }

int test_main(void) {
    struct a_stat st;
    char dents[1024];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    i64 fd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("open_dir", fd >= 0, fd);
    expect("rmdir_open", rmdir_(D "/sub"), 0);
    expect("fstat_open", syscall2(SYS_fstat, fd, &st), 0);
    expect("fstat_nlink_0", st.st_nlink, 0);
    expect("fstat_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("getdents_enoent", syscall3(SYS_getdents64, fd, dents, sizeof dents), -ENOENT);
    expect("create_in_removed", syscall4(SYS_openat, fd, "new", O_WRONLY | O_CREAT, 0644), -ENOENT);
    expect("mkdir_in_removed", syscall3(SYS_mkdirat, fd, "new", 0755), -ENOENT);
    expect("lookup_dot", syscall4(SYS_newfstatat, fd, ".", &st, 0), 0);
    expect("close", a_close(fd), 0);

    /* the working directory */
    expect("setup_cwd", a_mkdir(D "/cwd", 0755), 0);
    expect("chdir_cwd", syscall1(SYS_chdir, D "/cwd"), 0);
    expect("rmdir_cwd_by_path", rmdir_(D "/cwd"), 0);
    expect("stat_dot", a_stat_path(".", &st), 0);
    expect("dot_nlink_0", st.st_nlink, 0);
    expect("create_in_cwd", a_open("f", O_WRONLY | O_CREAT, 0644), -ENOENT);
    expect("stat_dotdot", a_stat_path("..", &st), 0);
    expect("chdir_dotdot", syscall1(SYS_chdir, ".."), 0);
    expect("now_in_D", a_stat_path("cwd", &st), -ENOENT);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    /* removing the working directory through "." is EINVAL, by name it works */
    expect("setup_cwd2", a_mkdir(D "/cwd2", 0755), 0);
    expect("chdir_cwd2", syscall1(SYS_chdir, D "/cwd2"), 0);
    expect("rmdir_dot", rmdir_("."), -EINVAL);
    expect("rmdir_via_dotdot", rmdir_("../cwd2"), 0);
    expect("chdir_root2", syscall1(SYS_chdir, "/"), 0);

    expect("cleanup_rmdir", rmdir_(D), 0);
    return finish();
}
