/* rmdir and timestamps: a successful rmdir sets the parent's mtime and
 * ctime to the time of the call (equal to each other, between a coarse
 * clock reading before and a precise one after); the parent's atime does
 * not change. A failed rmdir (ENOTEMPTY, ENOENT, ENOTDIR, EINVAL)
 * changes none of the parent's times and its link count. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_times"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }

static i64 ns(u64 s, u64 n) { return (i64)(s * 1000000000UL + n); }
static i64 clock_ns(int id) {
    struct a_timespec t;
    syscall2(SYS_clock_gettime, id, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}
static void pause_a_bit(void) {
    struct a_timespec t = {0, 20000000};
    syscall2(SYS_nanosleep, &t, 0);
}

int test_main(void) {
    struct a_stat a, b;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_x", a_mkdir(D "/x", 0755), 0);
    expect("setup_full", a_mkdir(D "/full", 0755), 0);
    expect("setup_full_f", a_make_file(D "/full/f", "", 0644), 0);
    expect("setup_file", a_make_file(D "/file", "", 0644), 0);
    pause_a_bit();

    expect("stat_before_failures", a_stat_path(D, &a), 0);
    pause_a_bit();
    expect("fail_enotempty", rm(D "/full"), -ENOTEMPTY);
    expect("fail_enoent", rm(D "/nope"), -ENOENT);
    expect("fail_enotdir", rm(D "/file"), -ENOTDIR);
    expect("fail_einval", rm(D "/x/."), -EINVAL);
    expect("stat_after_failures", a_stat_path(D, &b), 0);
    expect("failures_keep_mtime", ns(a.st_mtime_sec, a.st_mtime_nsec) == ns(b.st_mtime_sec, b.st_mtime_nsec), 1);
    expect("failures_keep_ctime", ns(a.st_ctime_sec, a.st_ctime_nsec) == ns(b.st_ctime_sec, b.st_ctime_nsec), 1);
    expect("failures_keep_nlink", a.st_nlink == b.st_nlink, 1);
    struct a_stat full;
    expect("stat_full", a_stat_path(D "/full", &full), 0);

    i64 t0 = clock_ns(CLOCK_REALTIME_COARSE);
    expect("rmdir_x", rm(D "/x"), 0);
    i64 t1 = clock_ns(CLOCK_REALTIME);
    expect("stat_after", a_stat_path(D, &b), 0);
    i64 m = ns(b.st_mtime_sec, b.st_mtime_nsec), c = ns(b.st_ctime_sec, b.st_ctime_nsec);
    expect("mtime_moved", m > ns(a.st_mtime_sec, a.st_mtime_nsec), 1);
    expect("mtime_in_window", m >= t0 && m <= t1, 1);
    expect("ctime_equals_mtime", c == m, 1);
    expect("atime_unchanged", ns(a.st_atime_sec, a.st_atime_nsec) == ns(b.st_atime_sec, b.st_atime_nsec), 1);
    expect("nlink_minus_1", b.st_nlink, a.st_nlink - 1);

    /* The sibling that failed to go is untouched. */
    struct a_stat full2;
    expect("stat_full_again", a_stat_path(D "/full", &full2), 0);
    expect("full_ctime_same", ns(full.st_ctime_sec, full.st_ctime_nsec) == ns(full2.st_ctime_sec, full2.st_ctime_nsec), 1);

    expect("cleanup", a_unlink(D "/full/f") | rm(D "/full") | a_unlink(D "/file"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
