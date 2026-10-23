/* lsetxattr does not follow a final symbolic link: it acts on the link
 * itself, and a link takes no user.* attribute (EPERM), whether it is live,
 * dangling, looping, points at a directory or at "." or is a second hard
 * link to the same symlink. The target is left untouched, and the link's
 * change time does not move. Zero-length values and all flag values are
 * EPERM too. */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lsetxattr_nofollow"

static char buf[64];

static i64 lset(const char *p, const char *name, i64 flags) { return syscall5(SYS_lsetxattr, p, name, "v", 1, flags); }

static void nap(void) {
    struct a_timespec t = { 0, 30000000 };
    syscall2(SYS_nanosleep, &t, 0);
}

int test_main(void) {
    struct a_stat s0, s1;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/lf"), 0);
    expect("setup_abs_link", syscall3(SYS_symlinkat, D "/f", AT_FDCWD, D "/abs"), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nope", AT_FDCWD, D "/dl"), 0);
    expect("setup_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/ls"), 0);
    expect("setup_self", syscall3(SYS_symlinkat, ".", AT_FDCWD, D "/self"), 0);
    expect("setup_hardlink", syscall5(SYS_linkat, AT_FDCWD, D "/lf", AT_FDCWD, D "/hl", 0), 0);
    expect("lstat_before", a_lstat_path(D "/lf", &s0), 0);
    nap();
    expect("link_eperm", lset(D "/lf", "user.a", 0), -EPERM);
    expect("link_create_eperm", lset(D "/lf", "user.a", 1), -EPERM);
    expect("link_replace_eperm", lset(D "/lf", "user.a", 2), -EPERM);
    expect("link_both_eperm", lset(D "/lf", "user.a", 3), -EPERM);
    expect("link_empty_value_eperm", syscall5(SYS_lsetxattr, D "/lf", "user.a", 0, 0, 0), -EPERM);
    expect("abs_link_eperm", lset(D "/abs", "user.a", 0), -EPERM);
    expect("dangling_eperm", lset(D "/dl", "user.a", 0), -EPERM);
    expect("loop_eperm", lset(D "/loop", "user.a", 0), -EPERM);
    expect("dirlink_eperm", lset(D "/ls", "user.a", 0), -EPERM);
    expect("selflink_eperm", lset(D "/self", "user.a", 0), -EPERM);
    expect("hardlink_eperm", lset(D "/hl", "user.a", 0), -EPERM);
    expect("target_untouched", syscall4(SYS_getxattr, D "/f", "user.a", buf, sizeof buf), -ENODATA);
    expect("dir_untouched", syscall4(SYS_getxattr, D "/sub", "user.a", buf, sizeof buf), -ENODATA);
    expect("link_bare", syscall4(SYS_lgetxattr, D "/lf", "user.a", buf, sizeof buf), -ENODATA);
    expect("lstat_after", a_lstat_path(D "/lf", &s1), 0);
    check("link_ctime_same", s0.st_ctime_sec == s1.st_ctime_sec && s0.st_ctime_nsec == s1.st_ctime_nsec,
          (i64)s1.st_ctime_nsec);
    check("still_link", (s1.st_mode & S_IFMT) == S_IFLNK, (i64)s1.st_mode);
    expect("missing_enoent", lset(D "/nope", "user.a", 0), -ENOENT);
    a_unlink(D "/lf");
    a_unlink(D "/hl");
    a_unlink(D "/abs");
    a_unlink(D "/dl");
    a_unlink(D "/loop");
    a_unlink(D "/ls");
    a_unlink(D "/self");
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
