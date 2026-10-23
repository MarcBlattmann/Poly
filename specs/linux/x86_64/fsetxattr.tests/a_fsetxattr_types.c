/* user.* attributes exist only on regular files and directories. Through
 * fsetxattr: both pipe ends, a UNIX socket, an opened FIFO, /dev/null, an
 * eventfd and an epoll descriptor give EPERM; trusted.* is EPERM without
 * CAP_SYS_ADMIN; an unknown prefix on a pipe, socket, FIFO or /dev/null is
 * EOPNOTSUPP, and "user." on a pipe or socket is EPERM. A memfd is a
 * regular file and takes user.* attributes. (Capabilities are dropped
 * first.) */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_socketpair 53
#define SYS_mknodat 259
#define SYS_fsetxattr 190
#define SYS_fgetxattr 193
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define SYS_memfd_create 319
#define D "/tmp/a_fsetxattr_types"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fset(i64 fd, const char *n) { return syscall5(SYS_fsetxattr, fd, n, "v", 1, 0); }

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    int p[2], s[2];
    expect("setup_pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_read_user", fset(p[0], "user.a"), -EPERM);
    expect("pipe_write_user", fset(p[1], "user.a"), -EPERM);
    expect("pipe_userdot", fset(p[0], "user."), -EPERM);
    expect("pipe_trusted", fset(p[0], "trusted.a"), -EPERM);
    expect("pipe_foo", fset(p[0], "foo.a"), -EOPNOTSUPP);
    a_close(p[0]);
    a_close(p[1]);
    expect("setup_socketpair", syscall4(SYS_socketpair, 1 /* AF_UNIX */, 1 /* SOCK_STREAM */, 0, s), 0);
    expect("socket_user", fset(s[0], "user.a"), -EPERM);
    expect("socket_userdot", fset(s[0], "user."), -EPERM);
    expect("socket_trusted", fset(s[0], "trusted.a"), -EPERM);
    expect("socket_foo", fset(s[0], "foo.a"), -EOPNOTSUPP);
    expect("socket_system_x", fset(s[0], "system.x"), -EOPNOTSUPP);
    a_close(s[0]);
    a_close(s[1]);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    i64 ff = a_open(D "/fifo", O_RDWR, 0);
    check("setup_open_fifo", ff >= 0, ff);
    expect("fifo_user", fset(ff, "user.a"), -EPERM);
    expect("fifo_foo", fset(ff, "foo.a"), -EOPNOTSUPP);
    a_close(ff);
    i64 dn = a_open("/dev/null", O_RDWR, 0);
    check("setup_devnull", dn >= 0, dn);
    expect("devnull_user", fset(dn, "user.a"), -EPERM);
    expect("devnull_trusted", fset(dn, "trusted.a"), -EPERM);
    expect("devnull_foo", fset(dn, "foo.a"), -EOPNOTSUPP);
    a_close(dn);
    i64 ev = syscall2(SYS_eventfd2, 0, 0);
    check("setup_eventfd", ev >= 0, ev);
    expect("eventfd_user", fset(ev, "user.a"), -EPERM);
    expect("eventfd_trusted", fset(ev, "trusted.a"), -EPERM);
    a_close(ev);
    i64 ep = syscall1(SYS_epoll_create1, 0);
    check("setup_epoll", ep >= 0, ep);
    expect("epoll_user", fset(ep, "user.a"), -EPERM);
    a_close(ep);
    i64 mf = syscall2(SYS_memfd_create, "x", 0);
    check("setup_memfd", mf >= 0, mf);
    expect("memfd_user", fset(mf, "user.a"), 0);
    expect("memfd_get", syscall4(SYS_fgetxattr, mf, "user.a", buf, sizeof buf), 1);
    expect("memfd_foo", fset(mf, "foo.a"), -EOPNOTSUPP);
    a_close(mf);
    a_unlink(D "/fifo");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
