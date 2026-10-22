/* utime on non-regular files reached by path: a FIFO (not opened, so the
 * call does not block) and a symlink target that is a directory. */
#include "a_sys.h"

#define SYS_utime 132
#define SYS_mknodat 259
#define D "/tmp/c_utime_special"

struct utimbuf_k {
    i64 actime, modtime;
};

int test_main(void) {
    struct a_stat st;
    struct utimbuf_k t = {808, 909};
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("fifo", syscall2(SYS_utime, D "/fifo", &t), 0);
    a_stat_path(D "/fifo", &st);
    check("fifo_values", st.st_atime_sec == 808 && st.st_mtime_sec == 909, (i64)st.st_mtime_sec);
    check("fifo_still_fifo", (st.st_mode & S_IFMT) == S_IFIFO, st.st_mode);
    expect("fifo_now", syscall2(SYS_utime, D "/fifo", 0), 0);
    a_stat_path(D "/fifo", &st);
    check("fifo_now_values", st.st_mtime_sec > 909 && st.st_atime_sec == st.st_mtime_sec, (i64)st.st_mtime_sec);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/dirlink"), 0);
    expect("dirlink", syscall2(SYS_utime, D "/dirlink", &t), 0);
    a_stat_path(D "/sub", &st);
    check("dirlink_target_values", st.st_atime_sec == 808 && st.st_mtime_sec == 909, (i64)st.st_mtime_sec);
    expect("dirlink_slash", syscall2(SYS_utime, D "/dirlink/", 0), 0);
    expect("cleanup_dirlink", a_unlink(D "/dirlink"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
