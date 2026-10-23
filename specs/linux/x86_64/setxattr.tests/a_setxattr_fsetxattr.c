/* fsetxattr (190): sets an attribute through a descriptor whatever its
 * access mode (O_RDONLY works, permission comes from the file's mode);
 * directory descriptors work; an unlinked but open file still takes
 * attributes. O_PATH, closed and invalid descriptors are EBADF. Pipes and
 * sockets take no user.* attribute (EPERM); an unknown namespace on a
 * socket is EOPNOTSUPP. */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_fgetxattr 193
#define D "/tmp/a_setxattr_fsetxattr"

static char buf[64];

static i64 fset(i64 fd, const char *name, const char *v) {
    return syscall5(SYS_fsetxattr, fd, name, v, str_len(v), 0);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly_fd", fset(fd, "user.r", "ro"), 0);
    expect("rdonly_value", syscall4(SYS_getxattr, D "/f", "user.r", buf, sizeof buf), 2);
    a_close(fd);
    expect("closed_fd_ebadf", fset(fd, "user.r", "x"), -EBADF);
    fd = a_open(D "/f", O_WRONLY, 0);
    expect("wronly_fd", fset(fd, "user.w", "wo!"), 0);
    expect("wronly_value", syscall4(SYS_getxattr, D "/f", "user.w", buf, sizeof buf), 3);
    a_close(fd);
    fd = a_open(D "/f", O_PATH, 0);
    expect("o_path_ebadf", fset(fd, "user.p", "x"), -EBADF);
    a_close(fd);
    expect("bad_fd_ebadf", fset(999, "user.p", "x"), -EBADF);
    expect("minus_one_ebadf", fset(-1, "user.p", "x"), -EBADF);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("dir_fd", fset(fd, "user.d", "dir"), 0);
    expect("dir_value", syscall4(SYS_getxattr, D, "user.d", buf, sizeof buf), 3);
    a_close(fd);
    expect("setup_gone", a_make_file(D "/gone", "", 0644), 0);
    fd = a_open(D "/gone", O_RDWR, 0);
    expect("unlink_open", a_unlink(D "/gone"), 0);
    expect("unlinked_fd", fset(fd, "user.u", "uu"), 0);
    expect("unlinked_value", syscall4(SYS_fgetxattr, fd, "user.u", buf, sizeof buf), 2);
    a_close(fd);
    int p[2];
    expect("setup_pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_read_end_eperm", fset(p[0], "user.a", "x"), -EPERM);
    expect("pipe_write_end_eperm", fset(p[1], "user.a", "x"), -EPERM);
    a_close(p[0]);
    a_close(p[1]);
    i64 s = syscall3(SYS_socket, 1 /* AF_UNIX */, 1 /* SOCK_STREAM */, 0);
    check("setup_socket", s >= 0, s);
    expect("socket_user_eperm", fset(s, "user.a", "x"), -EPERM);
    expect("socket_unknown_ns_eopnotsupp", fset(s, "foo.a", "x"), -EOPNOTSUPP);
    a_close(s);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
