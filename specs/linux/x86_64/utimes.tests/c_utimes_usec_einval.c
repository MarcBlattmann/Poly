/* utimes: a microseconds value outside [0, 999999] in either timeval is
 * EINVAL (checked as a 64-bit value); the file is left alone, ctime too.
 * The UTIME_NOW and UTIME_OMIT numbers have no special meaning here. */
#include "a_sys.h"

#define D "/tmp/c_utimes_usec_einval"

struct timeval_k {
    i64 tv_sec, tv_usec;
};

static i64 uts(const char *p, i64 as, i64 au, i64 ms, i64 mu) {
    struct timeval_k t[2] = {{as, au}, {ms, mu}};
    return syscall2(SYS_utimes, p, t);
}

int test_main(void) {
    struct a_stat before, after;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_times", uts(D "/f", 10, 250000, 20, 750000), 0);
    a_stat_path(D "/f", &before);
    expect("atime_usec_1e6", uts(D "/f", 1, 1000000, 2, 0), -EINVAL);
    expect("mtime_usec_1e6", uts(D "/f", 1, 0, 2, 1000000), -EINVAL);
    expect("atime_usec_neg", uts(D "/f", 1, -1, 2, 0), -EINVAL);
    expect("mtime_usec_neg", uts(D "/f", 1, 0, 2, -1), -EINVAL);
    expect("both_bad", uts(D "/f", 1, 2000000, 2, -5), -EINVAL);
    expect("usec_2_pow_32", uts(D "/f", 1, 0, 2, 1LL << 32), -EINVAL);
    expect("usec_2_pow_32_plus_5", uts(D "/f", 1, (1LL << 32) + 5, 2, 0), -EINVAL);
    expect("usec_min_i64", uts(D "/f", 1, (i64)(1ULL << 63), 2, 0), -EINVAL);
    expect("usec_utime_now", uts(D "/f", 1, (1 << 30) - 1, 2, (1 << 30) - 1), -EINVAL);
    expect("usec_utime_omit", uts(D "/f", 1, (1 << 30) - 2, 2, (1 << 30) - 2), -EINVAL);
    expect("usec_1e9", uts(D "/f", 1, 999999999, 2, 0), -EINVAL);
    a_stat_path(D "/f", &after);
    check("atime_kept", after.st_atime_sec == 10 && after.st_atime_nsec == 250000000, (i64)after.st_atime_nsec);
    check("mtime_kept", after.st_mtime_sec == 20 && after.st_mtime_nsec == 750000000, (i64)after.st_mtime_nsec);
    check("ctime_kept", after.st_ctime_sec == before.st_ctime_sec && after.st_ctime_nsec == before.st_ctime_nsec,
          (i64)after.st_ctime_nsec);
    expect("bounds_ok", uts(D "/f", 1, 0, 2, 999999), 0);
    a_stat_path(D "/f", &after);
    check("bounds_values", after.st_atime_sec == 1 && after.st_atime_nsec == 0 && after.st_mtime_sec == 2 &&
                               after.st_mtime_nsec == 999999000,
          (i64)after.st_mtime_nsec);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
