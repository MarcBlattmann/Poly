/* Side effects: a successful removal moves the file's change time
 * (ctime) forward and leaves mode, size, link count, inode number, mtime
 * and contents alone. Failed calls (ENODATA for a missing name,
 * EOPNOTSUPP for an unknown prefix, EACCES on a read-only file, EBADF
 * through an O_PATH descriptor) change nothing, ctime included. The
 * directory holding the file is not touched. Capabilities are dropped
 * first, so the read-only case is EACCES for every user. */
#include "a_sys.h"

#define SYS_nanosleep 35
#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_stat"

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 frm(i64 fd, const char *n) { return syscall2(SYS_fremovexattr, fd, n); }

static void nap(void) {
    i64 t[2] = { 0, 20000000 };
    syscall2(SYS_nanosleep, t, 0);
}

static int same_ctime(const struct a_stat *a, const struct a_stat *b) {
    return a->st_ctime_sec == b->st_ctime_sec && a->st_ctime_nsec == b->st_ctime_nsec;
}

int test_main(void) {
    struct a_stat a, b, da, db;
    char buf[8];
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("set", syscall5(SYS_fsetxattr, fd, "user.s", "v", 1, 0), 0);
    expect("stat_before", syscall2(SYS_fstat, fd, &a), 0);
    expect("dir_stat_before", a_stat_path(D, &da), 0);
    nap();
    expect("missing", frm(fd, "user.nope"), -ENODATA);
    expect("unknown", frm(fd, "foo.x"), -EOPNOTSUPP);
    i64 op = a_open(D "/f", O_PATH, 0);
    expect("opath", frm(op, "user.s"), -EBADF);
    a_close(op);
    expect("stat_failed", syscall2(SYS_fstat, fd, &b), 0);
    check("failed_keep_ctime", same_ctime(&a, &b), 0);
    expect("remove", frm(fd, "user.s"), 0);
    expect("stat_after", syscall2(SYS_fstat, fd, &b), 0);
    check("ctime_moved", b.st_ctime_sec > a.st_ctime_sec ||
          (b.st_ctime_sec == a.st_ctime_sec && b.st_ctime_nsec > a.st_ctime_nsec), 0);
    expect("mode_same", b.st_mode, a.st_mode);
    expect("size_same", b.st_size, 3);
    expect("nlink_same", b.st_nlink, a.st_nlink);
    expect("ino_same", b.st_ino, a.st_ino);
    check("mtime_same", b.st_mtime_sec == a.st_mtime_sec && b.st_mtime_nsec == a.st_mtime_nsec, 0);
    expect("dir_stat_after", a_stat_path(D, &db), 0);
    check("dir_ctime_same", same_ctime(&da, &db), 0);
    check("dir_mtime_same", da.st_mtime_sec == db.st_mtime_sec && da.st_mtime_nsec == db.st_mtime_nsec, 0);
    expect("set_ro", syscall5(SYS_fsetxattr, fd, "user.r", "v", 1, 0), 0);
    expect("chmod_0444", syscall2(SYS_fchmod, fd, 0444), 0);
    expect("stat_ro", syscall2(SYS_fstat, fd, &a), 0);
    nap();
    expect("ro_refused", frm(fd, "user.r"), -EACCES);
    expect("stat_ro_after", syscall2(SYS_fstat, fd, &b), 0);
    check("eacces_keep_ctime", same_ctime(&a, &b), 0);
    expect("seek", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("contents", a_read(fd, buf, sizeof buf), 3);
    check("contents_value", mem_eq(buf, "abc", 3), 0);
    a_close(fd);
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
