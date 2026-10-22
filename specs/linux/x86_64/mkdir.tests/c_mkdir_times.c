/* Timestamps: the new directory's access, modification and change
 * times are all equal and lie between a coarse clock reading taken
 * before the call and a precise one taken after it. The parent's
 * modification and change times move forward to that window too (and
 * equal each other); its access time does not change. A failed mkdir
 * (EEXIST) leaves the parent's times alone. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_times"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static int le(u64 as, u64 an, u64 bs, u64 bn) { return as < bs || (as == bs && an <= bn); }

int test_main(void) {
    struct a_stat c, p0, p1, p2;
    struct a_timespec before, after, pause = {0, 20000000};
    expect("setup", a_mkdir(D, 0755), 0);
    expect("parent_before", a_stat_path(D, &p0), 0);
    expect("sleep", syscall2(SYS_nanosleep, &pause, 0), 0);
    expect("clock_before", syscall2(SYS_clock_gettime, CLOCK_REALTIME_COARSE, &before), 0);
    expect("mkdir", mk(D "/n", 0755), 0);
    expect("clock_after", syscall2(SYS_clock_gettime, CLOCK_REALTIME, &after), 0);
    expect("stat_child", a_lstat_path(D "/n", &c), 0);
    expect("stat_parent", a_stat_path(D, &p1), 0);

    expect("child_atime_eq_mtime", c.st_atime_sec == c.st_mtime_sec && c.st_atime_nsec == c.st_mtime_nsec, 1);
    expect("child_ctime_eq_mtime", c.st_ctime_sec == c.st_mtime_sec && c.st_ctime_nsec == c.st_mtime_nsec, 1);
    expect("child_mtime_after_start",
           le((u64)before.tv_sec, (u64)before.tv_nsec, c.st_mtime_sec, c.st_mtime_nsec), 1);
    expect("child_mtime_before_end", le(c.st_mtime_sec, c.st_mtime_nsec, (u64)after.tv_sec, (u64)after.tv_nsec),
           1);
    expect("parent_mtime_eq_ctime", p1.st_mtime_sec == p1.st_ctime_sec && p1.st_mtime_nsec == p1.st_ctime_nsec,
           1);
    expect("parent_mtime_moved", le(p0.st_mtime_sec, p0.st_mtime_nsec, p1.st_mtime_sec, p1.st_mtime_nsec) &&
                                     !(p0.st_mtime_sec == p1.st_mtime_sec && p0.st_mtime_nsec == p1.st_mtime_nsec),
           1);
    expect("parent_mtime_in_window",
           le((u64)before.tv_sec, (u64)before.tv_nsec, p1.st_mtime_sec, p1.st_mtime_nsec) &&
               le(p1.st_mtime_sec, p1.st_mtime_nsec, (u64)after.tv_sec, (u64)after.tv_nsec),
           1);
    expect("parent_atime_unchanged", p0.st_atime_sec == p1.st_atime_sec && p0.st_atime_nsec == p1.st_atime_nsec,
           1);

    expect("sleep_again", syscall2(SYS_nanosleep, &pause, 0), 0);
    expect("mkdir_existing", mk(D "/n", 0755), -EEXIST);
    expect("stat_parent_again", a_stat_path(D, &p2), 0);
    expect("failed_parent_mtime_same", p2.st_mtime_sec == p1.st_mtime_sec && p2.st_mtime_nsec == p1.st_mtime_nsec,
           1);
    expect("failed_parent_ctime_same", p2.st_ctime_sec == p1.st_ctime_sec && p2.st_ctime_nsec == p1.st_ctime_nsec,
           1);

    expect("rm_n", a_rmdir(D "/n"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
