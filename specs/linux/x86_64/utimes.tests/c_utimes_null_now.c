/* utimess with a NULL times pointer: atime, mtime and ctime all become the
 * same current time, nanoseconds included. */
#include "a_sys.h"

#define D "/tmp/c_utimes_null_now"

/* sys/time.h: struct timeval { time_t tv_sec; suseconds_t tv_usec; } */
struct timeval_k {
    i64 tv_sec, tv_usec;
};

int test_main(void) {
    struct a_stat st;
    struct a_timespec before, after;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    struct timeval_k old[2] = {{1000, 0}, {2000, 0}};
    expect("set_old", syscall2(SYS_utimes, D "/f", old), 0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &before);
    expect("null_times", syscall2(SYS_utimes, D "/f", 0), 0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &after);
    expect("stat", a_stat_path(D "/f", &st), 0);
    check("atime_eq_mtime", st.st_atime_sec == st.st_mtime_sec && st.st_atime_nsec == st.st_mtime_nsec, 0);
    check("mtime_eq_ctime", st.st_mtime_sec == st.st_ctime_sec && st.st_mtime_nsec == st.st_ctime_nsec, 0);
    check("nsec_in_range", st.st_mtime_nsec < 1000000000, (i64)st.st_mtime_nsec);
    check("not_before_call", (i64)st.st_mtime_sec >= before.tv_sec - 1, (i64)st.st_mtime_sec - before.tv_sec);
    check("not_after_call", (i64)st.st_mtime_sec <= after.tv_sec, (i64)st.st_mtime_sec - after.tv_sec);
    expect("null_times_dir", syscall2(SYS_utimes, D, 0), 0);
    expect("stat_dir", a_stat_path(D, &st), 0);
    check("dir_atime_eq_mtime", st.st_atime_sec == st.st_mtime_sec && st.st_atime_nsec == st.st_mtime_nsec, 0);
    check("dir_current", (i64)st.st_mtime_sec >= before.tv_sec - 1, (i64)st.st_mtime_sec - before.tv_sec);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
