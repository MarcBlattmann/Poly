/* fstat: timestamps: a write moves mtime and ctime forward; a chmod
 * moves ctime only; nanoseconds are in range; mtime set by utimensat is
 * reported exactly. */
#include "a_sys.h"

#define F "/tmp/a_fstat_times"

static int le(u64 as, u64 an, u64 bs, u64 bn) { return as < bs || (as == bs && an <= bn); }

int test_main(void) {
    struct a_stat a, b;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    syscall2(SYS_fstat, fd, &a);
    check("nsec_in_range", a.st_atime_nsec < 1000000000 && a.st_mtime_nsec < 1000000000 &&
                               a.st_ctime_nsec < 1000000000, 0);
    check("new_file_mtime_equals_ctime", a.st_mtime_sec == a.st_ctime_sec && a.st_mtime_nsec == a.st_ctime_nsec, 0);
    a_write(fd, "x", 1);
    syscall2(SYS_fstat, fd, &b);
    check("write_mtime_not_earlier", le(a.st_mtime_sec, a.st_mtime_nsec, b.st_mtime_sec, b.st_mtime_nsec), 0);
    check("write_ctime_not_earlier", le(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);
    struct a_timespec t[2] = {{1000000000, 123456789}, {1200000000, 987654321}};
    expect("utimensat", syscall4(SYS_utimensat, fd, 0, t, 0), 0);
    syscall2(SYS_fstat, fd, &a);
    check("atime_exact", a.st_atime_sec == 1000000000 && a.st_atime_nsec == 123456789, a.st_atime_sec);
    check("mtime_exact", a.st_mtime_sec == 1200000000 && a.st_mtime_nsec == 987654321, a.st_mtime_sec);
    check("ctime_is_now_not_set", a.st_ctime_sec > 1500000000, a.st_ctime_sec);
    syscall3(SYS_fchmodat, AT_FDCWD, F, 0600);
    syscall2(SYS_fstat, fd, &b);
    check("chmod_keeps_mtime", b.st_mtime_sec == 1200000000 && b.st_mtime_nsec == 987654321, b.st_mtime_sec);
    check("chmod_ctime_not_earlier", le(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);
    /* A read does not change mtime or ctime. */
    char c;
    syscall4(SYS_pread64, fd, &c, 1, 0);
    syscall2(SYS_fstat, fd, &a);
    check("read_keeps_mtime", a.st_mtime_sec == b.st_mtime_sec && a.st_mtime_nsec == b.st_mtime_nsec, 0);
    check("read_keeps_ctime", a.st_ctime_sec == b.st_ctime_sec && a.st_ctime_nsec == b.st_ctime_nsec, 0);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
