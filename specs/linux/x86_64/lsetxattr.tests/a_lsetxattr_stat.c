/* Side effects: a successful lsetxattr (new or replaced attribute) moves
 * the file's change time forward and leaves mode, size, contents and mtime
 * alone. A failed call (EEXIST, ENODATA, EPERM on a link) changes no
 * times. The link's own times never move when lsetxattr is refused on it,
 * and setting through an earlier dir link touches only the file. The
 * attribute belongs to the inode: another hard link sees it. */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define D "/tmp/a_lsetxattr_stat"

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

static i64 lset(const char *p, const char *n, const char *v, i64 flags) {
    return syscall5(SYS_lsetxattr, p, n, v, str_len(v), flags);
}

int test_main(void) {
    struct a_stat f0, f1, f2, l0, l1;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "contents", 0640), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, ".", AT_FDCWD, D "/self"), 0);
    expect("stat_f0", a_stat_path(D "/f", &f0), 0);
    expect("lstat_l0", a_lstat_path(D "/l", &l0), 0);
    nap();
    expect("set_new", lset(D "/self/f", "user.a", "1", 0), 0);
    expect("stat_f1", a_stat_path(D "/f", &f1), 0);
    check("ctime_forward", later(&f0, &f1), (i64)f1.st_ctime_nsec);
    expect("mode_same", f1.st_mode, S_IFREG | 0640);
    expect("size_same", f1.st_size, 8);
    check("mtime_same", f1.st_mtime_sec == f0.st_mtime_sec && f1.st_mtime_nsec == f0.st_mtime_nsec,
          (i64)f1.st_mtime_nsec);
    nap();
    expect("eexist", lset(D "/f", "user.a", "2", 1), -EEXIST);
    expect("enodata", lset(D "/f", "user.m", "2", 2), -61);
    expect("link_eperm", lset(D "/l", "user.a", "2", 0), -EPERM);
    expect("stat_f2", a_stat_path(D "/f", &f2), 0);
    check("failed_ctime_same", same_ctime(&f1, &f2), (i64)f2.st_ctime_nsec);
    expect("lstat_l1", a_lstat_path(D "/l", &l1), 0);
    check("link_ctime_same", same_ctime(&l0, &l1), (i64)l1.st_ctime_nsec);
    check("link_mtime_same", l0.st_mtime_sec == l1.st_mtime_sec && l0.st_mtime_nsec == l1.st_mtime_nsec,
          (i64)l1.st_mtime_nsec);
    nap();
    expect("replace", lset(D "/f", "user.a", "22", 0), 0);
    expect("stat_f3", a_stat_path(D "/f", &f2), 0);
    check("replace_ctime_forward", later(&f1, &f2), (i64)f2.st_ctime_nsec);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    char text[16];
    expect("contents_same", a_read(fd, text, sizeof text), 8);
    check("contents_bytes", mem_eq(text, "contents", 8), 0);
    a_close(fd);
    expect("hardlink", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/g", 0), 0);
    expect("via_hardlink", syscall4(SYS_getxattr, D "/g", "user.a", buf, sizeof buf), 2);
    expect("set_via_hardlink", lset(D "/g", "user.b", "bbb", 0), 0);
    expect("seen_via_first", syscall4(SYS_getxattr, D "/f", "user.b", buf, sizeof buf), 3);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_self", a_unlink(D "/self"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
