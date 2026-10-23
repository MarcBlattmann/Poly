/* Side effects of fsetxattr: a successful set (new or replaced attribute)
 * moves the change time forward and leaves mode, size, contents and mtime
 * alone. A refused call (EEXIST, ENODATA, EINVAL, E2BIG, EOPNOTSUPP)
 * changes no time and creates nothing. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define ENODATA 61
#define D "/tmp/a_fsetxattr_stat"

static char buf[64];

static void nap(void) {
    struct a_timespec t = { 0, 30000000 };
    syscall2(SYS_nanosleep, &t, 0);
}

static int later(const struct a_stat *a, const struct a_stat *b) {
    return b->st_ctime_sec > a->st_ctime_sec ||
           (b->st_ctime_sec == a->st_ctime_sec && b->st_ctime_nsec > a->st_ctime_nsec);
}

static int same_times(const struct a_stat *a, const struct a_stat *b) {
    return a->st_ctime_sec == b->st_ctime_sec && a->st_ctime_nsec == b->st_ctime_nsec &&
           a->st_mtime_sec == b->st_mtime_sec && a->st_mtime_nsec == b->st_mtime_nsec;
}

static i64 fd;

static i64 fset(const char *n, const char *v, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, n, v, str_len(v), flags);
}

int test_main(void) {
    struct a_stat s0, s1, s2, s3;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "contents", 0640), 0);
    fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("fstat_0", syscall2(SYS_fstat, fd, &s0), 0);
    nap();
    expect("set_new", fset("user.a", "1", 0), 0);
    expect("fstat_1", syscall2(SYS_fstat, fd, &s1), 0);
    check("new_ctime_forward", later(&s0, &s1), (i64)s1.st_ctime_nsec);
    expect("mode_same", s1.st_mode, S_IFREG | 0640);
    expect("size_same", s1.st_size, 8);
    check("mtime_same", s1.st_mtime_sec == s0.st_mtime_sec && s1.st_mtime_nsec == s0.st_mtime_nsec, 0);
    nap();
    expect("replace", fset("user.a", "22", 2), 0);
    expect("fstat_2", syscall2(SYS_fstat, fd, &s2), 0);
    check("replace_ctime_forward", later(&s1, &s2), (i64)s2.st_ctime_nsec);
    nap();
    expect("fail_eexist", fset("user.a", "3", 1), -EEXIST);
    expect("fail_enodata", fset("user.b", "3", 2), -ENODATA);
    expect("fail_einval", fset("user.b", "3", 4), -EINVAL);
    expect("fail_e2big", syscall5(SYS_fsetxattr, fd, "user.b", "3", 65537, 0), -E2BIG);
    expect("fail_eopnotsupp", fset("foo.b", "3", 0), -EOPNOTSUPP);
    expect("fstat_3", syscall2(SYS_fstat, fd, &s3), 0);
    check("fail_times_same", same_times(&s2, &s3), (i64)s3.st_ctime_nsec);
    expect("fail_value_kept", syscall4(SYS_fgetxattr, fd, "user.a", buf, sizeof buf), 2);
    expect("fail_nothing_new", syscall4(SYS_fgetxattr, fd, "user.b", buf, sizeof buf), -ENODATA);
    expect("contents_same", a_read(fd, buf, sizeof buf), 8);
    check("contents_bytes", mem_eq(buf, "contents", 8), 0);
    a_close(fd);
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
