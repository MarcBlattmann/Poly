/* Attributes are per inode and shared by every descriptor and name of
 * it. A removal through a dup'd descriptor (after the original is
 * closed), through a descriptor opened via a symbolic link (removes the
 * target's attribute), through a reopen of /proc/self/fd/N, or by a
 * forked child through an inherited descriptor is seen at once through
 * the other descriptors, the hard link and getxattr by path. */
#include "a_sys.h"

#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define SYS_fremovexattr 199
#define ENODATA 61
#define D "/tmp/a_fremovexattr_shared"

static char buf[64];

static i64 frm(i64 fd, const char *n) { return syscall2(SYS_fremovexattr, fd, n); }
static i64 fset(i64 fd, const char *n) { return syscall5(SYS_fsetxattr, fd, n, "v", 1, 0); }
static i64 get(const char *p, const char *n) { return syscall4(SYS_getxattr, p, n, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_hardlink", syscall2(SYS_link, D "/f", D "/h"), 0);
    expect("setup_symlink", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("setup_open", fd >= 0, fd);
    expect("set_dup", fset(fd, "user.dup"), 0);
    expect("set_link", fset(fd, "user.link"), 0);
    expect("set_proc", fset(fd, "user.proc"), 0);
    expect("set_child", fset(fd, "user.child"), 0);
    i64 d2 = syscall1(SYS_dup, fd);
    check("setup_dup", d2 >= 0, d2);
    i64 d3 = syscall1(SYS_dup, fd);
    check("setup_dup2", d3 >= 0, d3);
    a_close(fd);
    expect("dup_remove", frm(d2, "user.dup"), 0);
    expect("dup_seen_other", syscall4(SYS_fgetxattr, d3, "user.dup", buf, sizeof buf), -ENODATA);
    expect("dup_seen_hardlink", get(D "/h", "user.dup"), -ENODATA);
    i64 lf = a_open(D "/l", O_RDONLY, 0);
    check("setup_open_symlink", lf >= 0, lf);
    expect("symlink_fd_remove", frm(lf, "user.link"), 0);
    expect("symlink_fd_target", get(D "/f", "user.link"), -ENODATA);
    a_close(lf);
    char path[40];
    str_cpy(path, "/proc/self/fd/");
    a_utoa(path + str_len(path), (u64)d2);
    i64 re = a_open(path, O_RDONLY, 0);
    check("setup_reopen", re >= 0, re);
    expect("reopen_remove", frm(re, "user.proc"), 0);
    expect("reopen_seen", syscall4(SYS_fgetxattr, d2, "user.proc", buf, sizeof buf), -ENODATA);
    a_close(re);
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit, frm(d3, "user.child") == 0 ? 0 : 1);
    int status = -1;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_status", status, 0);
    expect("child_seen", syscall4(SYS_fgetxattr, d2, "user.child", buf, sizeof buf), -ENODATA);
    expect("child_seen_path", get(D "/h", "user.child"), -ENODATA);
    a_close(d2);
    a_close(d3);
    a_unlink(D "/l");
    a_unlink(D "/h");
    a_unlink(D "/f");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
