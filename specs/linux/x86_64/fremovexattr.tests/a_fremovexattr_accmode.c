/* fremovexattr does not care how the descriptor was opened (O_RDONLY,
 * O_WRONLY, O_RDWR, O_WRONLY|O_APPEND, a directory with O_DIRECTORY):
 * user.* removal needs write permission by the file's mode bits at the
 * time of the call. A file opened O_RDWR and then fchmod'ed to 0444 or
 * 0000 is EACCES and keeps its attribute; fchmod'ed to 0200 it works. A
 * 0555 directory is EACCES. No path is looked up: a file whose parent
 * directory became 0000 after the open still has its attribute removed.
 * Capabilities are dropped first, so root gets the same results. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_accmode"

static char buf[16];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 frm(i64 fd, const char *n) { return syscall2(SYS_fremovexattr, fd, n); }
static i64 fset(i64 fd, const char *n) { return syscall5(SYS_fsetxattr, fd, n, "v", 1, 0); }
static i64 fget(i64 fd, const char *n) { return syscall4(SYS_fgetxattr, fd, n, buf, sizeof buf); }

static void mode_case(const char *label, i64 flags) {
    i64 fd = a_open(D "/f", flags, 0);
    check(label, fd >= 0, fd);
    if (fd < 0) return;
    check(label, fset(fd, "user.m") == 0, 0);
    check(label, frm(fd, "user.m") == 0, 0);
    check(label, fget(fd, "user.m") == -ENODATA, 0);
    a_close(fd);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_subfile", a_make_file(D "/sub/g", "", 0644), 0);
    mode_case("rdonly", O_RDONLY);
    mode_case("wronly", O_WRONLY);
    mode_case("rdwr", O_RDWR);
    mode_case("wronly_append", O_WRONLY | O_APPEND);
    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("setup_open", fd >= 0, fd);
    expect("set_r", fset(fd, "user.r"), 0);
    expect("chmod_0444", syscall2(SYS_fchmod, fd, 0444), 0);
    expect("rm_0444", frm(fd, "user.r"), -EACCES);
    expect("chmod_0000", syscall2(SYS_fchmod, fd, 0), 0);
    expect("rm_0000", frm(fd, "user.r"), -EACCES);
    expect("chmod_0644_check", syscall2(SYS_fchmod, fd, 0644), 0);
    expect("still_there", fget(fd, "user.r"), 1);
    expect("chmod_0200", syscall2(SYS_fchmod, fd, 0200), 0);
    expect("rm_0200", frm(fd, "user.r"), 0);
    expect("gone_0200", frm(fd, "user.r"), -ENODATA);
    expect("chmod_0644", syscall2(SYS_fchmod, fd, 0644), 0);
    a_close(fd);
    i64 dfd = a_open(D "/sub", O_RDONLY | O_DIRECTORY, 0);
    check("setup_open_dir", dfd >= 0, dfd);
    expect("dir_set", fset(dfd, "user.d"), 0);
    expect("dir_chmod_0555", syscall2(SYS_fchmod, dfd, 0555), 0);
    expect("dir_rm_0555", frm(dfd, "user.d"), -EACCES);
    expect("dir_chmod_0755", syscall2(SYS_fchmod, dfd, 0755), 0);
    expect("dir_rm", frm(dfd, "user.d"), 0);
    i64 g = a_open(D "/sub/g", O_RDONLY, 0);
    check("setup_open_g", g >= 0, g);
    expect("g_set", fset(g, "user.g"), 0);
    expect("dir_chmod_0000", syscall2(SYS_fchmod, dfd, 0), 0);
    expect("rm_parent_0000", frm(g, "user.g"), 0);
    expect("gone_parent_0000", fget(g, "user.g"), -ENODATA);
    expect("dir_chmod_back", syscall2(SYS_fchmod, dfd, 0755), 0);
    a_close(g);
    a_close(dfd);
    a_unlink(D "/sub/g");
    a_unlink(D "/f");
    a_rmdir(D "/sub");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
