/* utimensat: explicit times are stored to the nanosecond; UTIME_OMIT
 * leaves one alone; ctime is set to the current time either way. */
#include "a_sys.h"

#define D "/tmp/a_utimensat_set"

static i64 ut(const char *p, struct a_timespec *t, i64 flags) {
    return syscall4(SYS_utimensat, AT_FDCWD, p, t, flags);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    struct a_timespec t[2] = {{1000, 1}, {2000, 999999999}};
    expect("set_both", ut(D "/f", t, 0), 0);
    a_stat_path(D "/f", &st);
    check("atime_set", st.st_atime_sec == 1000 && st.st_atime_nsec == 1, st.st_atime_sec);
    check("mtime_set", st.st_mtime_sec == 2000 && st.st_mtime_nsec == 999999999, st.st_mtime_sec);
    check("ctime_is_current", st.st_ctime_sec > 1600000000, st.st_ctime_sec);
    struct a_timespec omit_a[2] = {{0, UTIME_OMIT}, {3000, 0}};
    expect("omit_atime", ut(D "/f", omit_a, 0), 0);
    a_stat_path(D "/f", &st);
    check("atime_omitted", st.st_atime_sec == 1000 && st.st_atime_nsec == 1, st.st_atime_sec);
    check("mtime_updated", st.st_mtime_sec == 3000 && st.st_mtime_nsec == 0, st.st_mtime_sec);
    struct a_timespec omit_m[2] = {{4000, 4}, {123, UTIME_OMIT}};
    expect("omit_mtime", ut(D "/f", omit_m, 0), 0);
    a_stat_path(D "/f", &st);
    check("atime_updated", st.st_atime_sec == 4000 && st.st_atime_nsec == 4, st.st_atime_sec);
    check("mtime_omitted", st.st_mtime_sec == 3000, st.st_mtime_sec);
    struct a_timespec both_omit[2] = {{0, UTIME_OMIT}, {0, UTIME_OMIT}};
    expect("omit_both", ut(D "/f", both_omit, 0), 0);
    a_stat_path(D "/f", &st);
    check("omit_both_nothing", st.st_atime_sec == 4000 && st.st_mtime_sec == 3000, 0);
    struct a_timespec zero[2] = {{0, 0}, {0, 0}};
    expect("set_epoch", ut(D "/f", zero, 0), 0);
    a_stat_path(D "/f", &st);
    check("epoch_set", st.st_atime_sec == 0 && st.st_mtime_sec == 0 && st.st_mtime_nsec == 0, st.st_mtime_sec);
    struct a_timespec neg[2] = {{-100, 0}, {-1, 500}};
    expect("set_before_epoch", ut(D "/f", neg, 0), 0);
    a_stat_path(D "/f", &st);
    check("before_epoch", (i64)st.st_atime_sec == -100 && (i64)st.st_mtime_sec == -1 && st.st_mtime_nsec == 500,
          (i64)st.st_atime_sec);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
