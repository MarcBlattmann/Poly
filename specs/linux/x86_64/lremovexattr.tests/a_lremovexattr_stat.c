/* lremovexattr side effects: removing a file's user.* attribute moves its
 * change time (ctime) and leaves mode, size, link count, inode and mtime
 * alone. A failed call changes nothing: ENODATA, EOPNOTSUPP on the file;
 * EPERM / EOPNOTSUPP on a symbolic link leave both the link's and the
 * target's ctime as they were. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_stat"
#define F D "/f"
#define L D "/l"

static struct a_stat s0, s1, l0, l1;

static void nap(void) {
    struct { i64 s, ns; } t = { 0, 30000000 };
    syscall2(SYS_nanosleep, &t, 0);
}

static int same_ctime(struct a_stat *a, struct a_stat *b) {
    return a->st_ctime_sec == b->st_ctime_sec && a->st_ctime_nsec == b->st_ctime_nsec;
}
static int ctime_later(void) {
    return s1.st_ctime_sec > s0.st_ctime_sec || (s1.st_ctime_sec == s0.st_ctime_sec && s1.st_ctime_nsec > s0.st_ctime_nsec);
}
static int same_rest(void) {
    return s0.st_mode == s1.st_mode && s0.st_size == s1.st_size && s0.st_nlink == s1.st_nlink &&
           s0.st_mtime_sec == s1.st_mtime_sec && s0.st_mtime_nsec == s1.st_mtime_nsec && s0.st_ino == s1.st_ino;
}
static i64 lrm(const char *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "hello", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, L), 0);
    expect("set_a", syscall5(SYS_setxattr, F, "user.a", "v", 1, 0), 0);
    expect("set_b", syscall5(SYS_setxattr, F, "user.b", "v", 1, 0), 0);
    expect("stat0", a_stat_path(F, &s0), 0);
    nap();
    expect("remove_a", lrm(F, "user.a"), 0);
    expect("stat1", a_stat_path(F, &s1), 0);
    check("ctime_moved", ctime_later(), 0);
    check("rest_unchanged", same_rest(), 0);
    expect("size_5", s1.st_size, 5);
    s0 = s1;
    expect("lstat_link0", a_lstat_path(L, &l0), 0);
    nap();
    expect("missing_enodata", lrm(F, "user.a"), -ENODATA);
    expect("unknown_eopnotsupp", lrm(F, "foo.a"), -EOPNOTSUPP);
    expect("link_eperm", lrm(L, "user.b"), -EPERM);
    expect("link_unknown_eopnotsupp", lrm(L, "foo.a"), -EOPNOTSUPP);
    expect("link_acl_eopnotsupp", lrm(L, "system.posix_acl_access"), -EOPNOTSUPP);
    expect("stat2", a_stat_path(F, &s1), 0);
    expect("lstat_link1", a_lstat_path(L, &l1), 0);
    check("file_ctime_kept", same_ctime(&s0, &s1), 0);
    check("file_rest_kept", same_rest(), 0);
    check("link_ctime_kept", same_ctime(&l0, &l1), 0);
    check("link_is_link", (l1.st_mode & 0170000) == 0120000, l1.st_mode);
    expect("b_still_there", lrm(F, "user.b"), 0);
    a_unlink(L);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
