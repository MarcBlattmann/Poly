/* Non-regular, non-directory files: on an opened FIFO and on /dev/null
 * every user.* name ("user." too) and trusted.* are ENODATA, unknown
 * prefixes and system.x EOPNOTSUPP; the FIFO (an inode on the test's file
 * system) also gives ENODATA for security.* and the ACL names and EINVAL
 * for "security.". eventfd and epoll descriptors: user.* and trusted.*
 * ENODATA, security.*, the ACL names and system.x EOPNOTSUPP. A
 * /proc/self file: user.* EOPNOTSUPP, trusted.* ENODATA. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_mknodat 259
#define SYS_fgetxattr 193
#define SYS_eventfd2 290
#define SYS_epoll_create1 291
#define ENODATA 61
#define D "/tmp/a_fgetxattr_special"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fget(i64 fd, const char *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0644, 0), 0);
    i64 fd = a_open(D "/p", O_RDWR, 0);
    check("setup_open_fifo", fd >= 0, fd);
    expect("fifo_user", fget(fd, "user.a"), -ENODATA);
    expect("fifo_user_dot", fget(fd, "user."), -ENODATA);
    expect("fifo_trusted", fget(fd, "trusted.a"), -ENODATA);
    expect("fifo_unknown", fget(fd, "foo.a"), -EOPNOTSUPP);
    expect("fifo_security", fget(fd, "security.a"), -ENODATA);
    expect("fifo_security_dot", fget(fd, "security."), -EINVAL);
    expect("fifo_acl_access", fget(fd, "system.posix_acl_access"), -ENODATA);
    expect("fifo_acl_default", fget(fd, "system.posix_acl_default"), -ENODATA);
    expect("fifo_system_x", fget(fd, "system.x"), -EOPNOTSUPP);
    expect("fifo_null_buffer", syscall4(SYS_fgetxattr, fd, "user.a", 0, 16), -ENODATA);
    a_close(fd);
    fd = a_open("/dev/null", O_RDWR, 0);
    check("setup_open_null", fd >= 0, fd);
    expect("null_user", fget(fd, "user.a"), -ENODATA);
    expect("null_user_dot", fget(fd, "user."), -ENODATA);
    expect("null_trusted", fget(fd, "trusted.a"), -ENODATA);
    expect("null_unknown", fget(fd, "foo.a"), -EOPNOTSUPP);
    expect("null_system_x", fget(fd, "system.x"), -EOPNOTSUPP);
    a_close(fd);
    fd = syscall2(SYS_eventfd2, 0, 0);
    check("setup_eventfd", fd >= 0, fd);
    expect("eventfd_user", fget(fd, "user.a"), -ENODATA);
    expect("eventfd_user_dot", fget(fd, "user."), -ENODATA);
    expect("eventfd_trusted", fget(fd, "trusted.a"), -ENODATA);
    expect("eventfd_security", fget(fd, "security.a"), -EOPNOTSUPP);
    expect("eventfd_acl", fget(fd, "system.posix_acl_access"), -EOPNOTSUPP);
    expect("eventfd_system_x", fget(fd, "system.x"), -EOPNOTSUPP);
    a_close(fd);
    fd = syscall1(SYS_epoll_create1, 0);
    check("setup_epoll", fd >= 0, fd);
    expect("epoll_user", fget(fd, "user.a"), -ENODATA);
    expect("epoll_trusted", fget(fd, "trusted.a"), -ENODATA);
    expect("epoll_security", fget(fd, "security.a"), -EOPNOTSUPP);
    expect("epoll_system_x", fget(fd, "system.x"), -EOPNOTSUPP);
    a_close(fd);
    fd = a_open("/proc/self/status", O_RDONLY, 0);
    check("setup_open_proc", fd >= 0, fd);
    expect("proc_user", fget(fd, "user.a"), -EOPNOTSUPP);
    expect("proc_trusted", fget(fd, "trusted.a"), -ENODATA);
    a_close(fd);
    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
