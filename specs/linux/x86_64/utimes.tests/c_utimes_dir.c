/* utimess on a directory works like on a file; a trailing slash is fine on a
 * directory and ENOTDIR on a regular file. Changing a directory's times
 * does not change its children. */
#include "a_sys.h"

#define D "/tmp/c_utimes_dir"

/* sys/time.h: struct timeval { time_t tv_sec; suseconds_t tv_usec; } */
struct timeval_k {
    i64 tv_sec, tv_usec;
};

int test_main(void) {
    struct a_stat st, child_before, child_after;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    a_stat_path(D "/sub/f", &child_before);
    struct timeval_k t[2] = {{12345, 600000}, {67890, 999999}};
    expect("dir", syscall2(SYS_utimes, D "/sub", t), 0);
    a_stat_path(D "/sub", &st);
    check("dir_atime", st.st_atime_sec == 12345 && st.st_atime_nsec == 600000000, (i64)st.st_atime_sec);
    check("dir_mtime", st.st_mtime_sec == 67890 && st.st_mtime_nsec == 999999000, (i64)st.st_mtime_sec);
    check("dir_still_dir", (st.st_mode & S_IFMT) == S_IFDIR, st.st_mode);
    a_stat_path(D "/sub/f", &child_after);
    check("child_untouched", child_after.st_mtime_sec == child_before.st_mtime_sec &&
                                 child_after.st_mtime_nsec == child_before.st_mtime_nsec,
          (i64)child_after.st_mtime_sec);
    struct timeval_k t2[2] = {{1, 0}, {2, 0}};
    expect("dir_trailing_slash", syscall2(SYS_utimes, D "/sub/", t2), 0);
    a_stat_path(D "/sub", &st);
    check("dir_trailing_slash_values", st.st_atime_sec == 1 && st.st_mtime_sec == 2, (i64)st.st_mtime_sec);
    expect("dir_dot", syscall2(SYS_utimes, D "/sub/.", t), 0);
    a_stat_path(D "/sub", &st);
    check("dir_dot_values", st.st_atime_sec == 12345, (i64)st.st_atime_sec);
    expect("dir_dotdot", syscall2(SYS_utimes, D "/sub/..", t2), 0);
    a_stat_path(D, &st);
    check("dir_dotdot_values", st.st_atime_sec == 1 && st.st_mtime_sec == 2, (i64)st.st_mtime_sec);
    expect("file_trailing_slash", syscall2(SYS_utimes, D "/sub/f/", t), -ENOTDIR);
    expect("file_trailing_slash_null", syscall2(SYS_utimes, D "/sub/f/", 0), -ENOTDIR);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
