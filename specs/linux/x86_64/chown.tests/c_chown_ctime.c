/* A successful chown updates the file's ctime to the current time, even
 * with -1 for both ids; mtime, atime, size and contents stay. */
#include "a_sys.h"

#define D "/tmp/c_chown_ctime"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static void nap(void) {
    i64 ts[2] = {0, 30000000};
    syscall2(SYS_nanosleep, ts, 0);
}

int test_main(void) {
    struct a_stat a, b;
    i64 now[2];
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    a_stat_path(D "/f", &a);
    nap();
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, now);
    expect("noop_chown", ch(D "/f", -1, -1), 0);
    a_stat_path(D "/f", &b);
    check("ctime_advanced", b.st_ctime_sec > a.st_ctime_sec ||
          (b.st_ctime_sec == a.st_ctime_sec && b.st_ctime_nsec > a.st_ctime_nsec), 0);
    check("ctime_not_before_call", (i64)b.st_ctime_sec >= now[0] - 1, 0);
    check("mtime_kept", b.st_mtime_sec == a.st_mtime_sec && b.st_mtime_nsec == a.st_mtime_nsec, 0);
    check("atime_kept", b.st_atime_sec == a.st_atime_sec && b.st_atime_nsec == a.st_atime_nsec, 0);
    check("size_kept", b.st_size == 3, b.st_size);
    a = b;
    nap();
    expect("same_ids_chown", ch(D "/f", uid, gid), 0);
    a_stat_path(D "/f", &b);
    check("ctime_advanced_again", b.st_ctime_sec > a.st_ctime_sec ||
          (b.st_ctime_sec == a.st_ctime_sec && b.st_ctime_nsec > a.st_ctime_nsec), 0);
    a = b;
    expect("failed_chown", ch(D "/f/x", -1, -1), -ENOTDIR);
    a_stat_path(D "/f", &b);
    check("failed_ctime_kept", b.st_ctime_sec == a.st_ctime_sec && b.st_ctime_nsec == a.st_ctime_nsec, 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("contents_kept", a_read(fd, buf, sizeof buf), 3);
    check("contents_bytes", buf[0] == 'a' && buf[1] == 'b' && buf[2] == 'c', buf[0]);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
