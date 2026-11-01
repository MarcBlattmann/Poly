/* readlink(2) needs search permission on the directories walked (EACCES;
 * root passes) but nothing on the link itself; reading a link does not
 * change its mtime or ctime; a link with a target containing a NUL-free
 * multi-byte string returns the bytes as stored. */
#include "a_sys.h"

#define D "/tmp/lb_readlink_perm"

static i64 readlink_(const char *p, char *b, i64 n) { return syscall3(SYS_readlink, p, b, n); }

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    char b[64];
    struct a_stat st, st2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_l", syscall2(SYS_symlink, "\xe2\x82\xac-euro", D "/sub/l"), 0);
    expect("read_ok", readlink_(D "/sub/l", b, 64), 8);
    expect("bytes", mem_eq(b, "\xe2\x82\xac-euro", 8), 1);
    expect("chmod_rw_only", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("no_search", readlink_(D "/sub/l", b, 64), root ? 8 : -EACCES);
    expect("no_search_missing", readlink_(D "/sub/nope", b, 64), root ? -ENOENT : -EACCES);
    expect("chmod_x_only", syscall2(SYS_chmod, D "/sub", 0100), 0);
    expect("search_only_ok", readlink_(D "/sub/l", b, 64), 8);
    expect("chmod_back", syscall2(SYS_chmod, D "/sub", 0755), 0);

    /* times of the link do not change */
    expect("lstat0", a_lstat_path(D "/sub/l", &st), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("read_again", readlink_(D "/sub/l", b, 64), 8);
    expect("lstat1", a_lstat_path(D "/sub/l", &st2), 0);
    check("mtime_same", st.st_mtime_sec == st2.st_mtime_sec && st.st_mtime_nsec == st2.st_mtime_nsec, 0);
    check("ctime_same", st.st_ctime_sec == st2.st_ctime_sec && st.st_ctime_nsec == st2.st_ctime_nsec, 0);

    expect("cleanup_l", a_unlink(D "/sub/l"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
