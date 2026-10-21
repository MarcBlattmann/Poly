/* lstat: a symbolic link has timestamps of its own, set with utimensat
 * and AT_SYMLINK_NOFOLLOW, separate from its target's; lstat itself
 * changes no timestamp. */
#include "a_sys.h"

#define D "/tmp/a_lstat_times"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }

int test_main(void) {
    struct a_stat st, a, b;
    struct a_timespec ts[2];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    ts[0].tv_sec = 1000000000; ts[0].tv_nsec = 123456789;
    ts[1].tv_sec = 1100000000; ts[1].tv_nsec = 987654321;
    expect("set_link_times", syscall4(SYS_utimensat, AT_FDCWD, D "/l", ts, AT_SYMLINK_NOFOLLOW), 0);
    ts[0].tv_sec = 1200000000; ts[0].tv_nsec = 5;
    ts[1].tv_sec = 1300000000; ts[1].tv_nsec = 7;
    expect("set_file_times", syscall4(SYS_utimensat, AT_FDCWD, D "/f", ts, 0), 0);
    expect("lstat_link", lstat(D "/l", &st), 0);
    expect("link_atime", st.st_atime_sec, 1000000000);
    expect("link_atime_nsec", st.st_atime_nsec, 123456789);
    expect("link_mtime", st.st_mtime_sec, 1100000000);
    expect("link_mtime_nsec", st.st_mtime_nsec, 987654321);
    check("link_ctime_nsec_range", st.st_ctime_nsec < 1000000000, st.st_ctime_nsec);
    check("link_ctime_recent", st.st_ctime_sec > 1600000000, 0);
    expect("lstat_file", lstat(D "/f", &st), 0);
    expect("file_atime", st.st_atime_sec, 1200000000);
    expect("file_atime_nsec", st.st_atime_nsec, 5);
    expect("file_mtime", st.st_mtime_sec, 1300000000);
    expect("file_mtime_nsec", st.st_mtime_nsec, 7);
    expect("stat_link_is_file_times", (syscall2(SYS_stat, D "/l", &st), st.st_mtime_sec), 1300000000);
    /* Repeated lstat calls see the same times: lstat does not touch them. */
    lstat(D "/l", &a);
    lstat(D "/l", &b);
    lstat(D "/l", &b);
    check("link_times_stable",
          a.st_atime_sec == b.st_atime_sec && a.st_atime_nsec == b.st_atime_nsec && a.st_ctime_sec == b.st_ctime_sec &&
              a.st_ctime_nsec == b.st_ctime_nsec,
          0);
    expect("file_atime_untouched", (lstat(D "/f", &st), st.st_atime_sec), 1200000000);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
