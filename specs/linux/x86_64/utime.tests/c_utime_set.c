/* utime: explicit times are stored as whole seconds (nanoseconds become 0);
 * atime and mtime are independent; 0, negative and 32-bit edge values. */
#include "a_sys.h"

#define SYS_utime 132
#define D "/tmp/c_utime_set"

/* utime.h: struct utimbuf { time_t actime, modtime; } */
struct utimbuf_k {
    i64 actime, modtime;
};

static i64 ut(const char *p, struct utimbuf_k *t) { return syscall2(SYS_utime, p, t); }

static int times_are(const char *p, i64 a, i64 m) {
    struct a_stat st;
    if (a_stat_path(p, &st) != 0) return 0;
    return (i64)st.st_atime_sec == a && st.st_atime_nsec == 0 && (i64)st.st_mtime_sec == m && st.st_mtime_nsec == 0;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    struct a_timespec ns[2] = {{500, 123456789}, {600, 987654321}};
    expect("setup_nsec", syscall4(SYS_utimensat, AT_FDCWD, D "/f", ns, 0), 0);
    struct utimbuf_k t = {1000, 2000};
    expect("set", ut(D "/f", &t), 0);
    check("set_values_nsec_zeroed", times_are(D "/f", 1000, 2000), 0);
    struct utimbuf_k later_atime = {9000, 10};
    expect("atime_after_mtime", ut(D "/f", &later_atime), 0);
    check("atime_after_mtime_values", times_are(D "/f", 9000, 10), 0);
    struct utimbuf_k same = {777, 777};
    expect("same", ut(D "/f", &same), 0);
    check("same_values", times_are(D "/f", 777, 777), 0);
    struct utimbuf_k zero = {0, 0};
    expect("epoch", ut(D "/f", &zero), 0);
    check("epoch_values", times_are(D "/f", 0, 0), 0);
    struct utimbuf_k neg = {-1, -86400};
    expect("before_epoch", ut(D "/f", &neg), 0);
    check("before_epoch_values", times_are(D "/f", -1, -86400), 0);
    struct utimbuf_k edge = {2147483647, -2147483647 - 1};
    expect("int32_edges", ut(D "/f", &edge), 0);
    check("int32_edges_values", times_are(D "/f", 2147483647, -2147483647 - 1), 0);
    struct a_stat st;
    a_stat_path(D "/f", &st);
    check("size_unchanged", st.st_size == 4, st.st_size);
    check("mode_unchanged", (st.st_mode & 07777) == 0644, st.st_mode);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
