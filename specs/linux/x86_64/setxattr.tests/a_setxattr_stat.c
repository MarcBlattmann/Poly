/* setxattr side effects: a successful set (new or replaced attribute)
 * moves the change time forward; mode, size, contents, mtime and atime stay
 * as they were. A failed set (EEXIST, ENODATA) changes nothing. The
 * attribute belongs to the inode: it is seen through another hard link and
 * stays across a rename. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define D "/tmp/a_setxattr_stat"

static char buf[64];

static void nap(void) {
    struct a_timespec t = { 0, 30000000 };
    syscall2(SYS_nanosleep, &t, 0);
}

static int later(const struct a_stat *a, const struct a_stat *b) {
    return b->st_ctime_sec > a->st_ctime_sec ||
           (b->st_ctime_sec == a->st_ctime_sec && b->st_ctime_nsec > a->st_ctime_nsec);
}

static int same_ctime(const struct a_stat *a, const struct a_stat *b) {
    return a->st_ctime_sec == b->st_ctime_sec && a->st_ctime_nsec == b->st_ctime_nsec;
}

int test_main(void) {
    struct a_stat s0, s1, s2;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "contents", 0640), 0);
    expect("stat_before", a_stat_path(D "/f", &s0), 0);
    nap();
    expect("set_new", syscall5(SYS_setxattr, D "/f", "user.a", "1", 1, 0), 0);
    expect("stat_after", a_stat_path(D "/f", &s1), 0);
    check("ctime_forward", later(&s0, &s1), (i64)s1.st_ctime_nsec);
    expect("mode_same", s1.st_mode, S_IFREG | 0640);
    expect("size_same", s1.st_size, 8);
    check("mtime_same", s1.st_mtime_sec == s0.st_mtime_sec && s1.st_mtime_nsec == s0.st_mtime_nsec,
          (i64)s1.st_mtime_nsec);
    check("atime_same", s1.st_atime_sec == s0.st_atime_sec && s1.st_atime_nsec == s0.st_atime_nsec,
          (i64)s1.st_atime_nsec);
    nap();
    expect("create_existing_eexist", syscall5(SYS_setxattr, D "/f", "user.a", "2", 1, 1), -EEXIST);
    expect("replace_missing_enodata", syscall5(SYS_setxattr, D "/f", "user.m", "2", 1, 2), -61);
    expect("stat_failed", a_stat_path(D "/f", &s2), 0);
    check("failed_ctime_same", same_ctime(&s1, &s2), (i64)s2.st_ctime_nsec);
    nap();
    expect("replace", syscall5(SYS_setxattr, D "/f", "user.a", "22", 2, 0), 0);
    expect("stat_replaced", a_stat_path(D "/f", &s2), 0);
    check("replace_ctime_forward", later(&s1, &s2), (i64)s2.st_ctime_nsec);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    char text[16];
    expect("contents_same", a_read(fd, text, sizeof text), 8);
    check("contents_bytes", mem_eq(text, "contents", 8), 0);
    a_close(fd);
    expect("link", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/g", 0), 0);
    expect("via_other_link", syscall4(SYS_getxattr, D "/g", "user.a", buf, sizeof buf), 2);
    expect("set_via_other_link", syscall5(SYS_setxattr, D "/g", "user.b", "bbb", 3, 0), 0);
    expect("seen_via_first", syscall4(SYS_getxattr, D "/f", "user.b", buf, sizeof buf), 3);
    expect("rename", syscall4(SYS_renameat, AT_FDCWD, D "/f", AT_FDCWD, D "/h"), 0);
    expect("kept_after_rename", syscall4(SYS_getxattr, D "/h", "user.b", buf, sizeof buf), 3);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
