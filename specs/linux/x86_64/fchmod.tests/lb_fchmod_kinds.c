/* fchmod(2) on descriptors that are not regular files on a disk file
 * system: a pipe (both ends are one inode, created 0600), a socket and a
 * memfd take the new mode; an eventfd (anonymous inode) is EOPNOTSUPP; /dev/null
 * belongs to root, so only root may change it (EPERM otherwise). */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_eventfd2 290
#define SYS_memfd_create 319

static i64 fchmod_(i64 fd, i64 mode) { return syscall2(SYS_fchmod, fd, mode); }
static u32 fmode(i64 fd) {
    struct a_stat st;
    if (syscall2(SYS_fstat, fd, &st)) return 0xffffffff;
    return st.st_mode;
}

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    i32 p[2];
    expect("pipe2", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_mode", fmode(p[0]), S_IFIFO | 0600);
    expect("pipe_fchmod", fchmod_(p[0], 0644), 0);
    expect("pipe_mode_changed", fmode(p[0]), S_IFIFO | 0644);
    expect("pipe_other_end_too", fmode(p[1]), S_IFIFO | 0644);
    expect("pipe_write_end_fchmod", fchmod_(p[1], 0), 0);
    expect("pipe_read_end_mode_0", fmode(p[0]), S_IFIFO);
    expect("pipe_still_works", a_write(p[1], "z", 1), 1);

    i64 s = syscall3(SYS_socket, 1 /* AF_UNIX */, 1 /* SOCK_STREAM */, 0);
    check("socket", s >= 0, s);
    expect("socket_mode", fmode(s), S_IFSOCK | 0777);
    expect("socket_fchmod", fchmod_(s, 0600), 0);
    expect("socket_mode_changed", fmode(s), S_IFSOCK | 0600);

    i64 m = syscall2(SYS_memfd_create, "lb", 0);
    check("memfd", m >= 0, m);
    expect("memfd_mode", fmode(m), S_IFREG | 0777);
    expect("memfd_fchmod", fchmod_(m, 0640), 0);
    expect("memfd_mode_changed", fmode(m), S_IFREG | 0640);

    i64 e = syscall2(SYS_eventfd2, 0, 0);
    check("eventfd", e >= 0, e);
    expect("eventfd_mode", fmode(e), 0600);
    expect("eventfd_fchmod_eopnotsupp", fchmod_(e, 0644), -EOPNOTSUPP);
    expect("eventfd_mode_unchanged", fmode(e), 0600);

    i64 n = a_open("/dev/null", O_RDONLY, 0);
    check("open_devnull", n >= 0, n);
    expect("devnull_same_mode", fchmod_(n, 0666), root ? 0 : -EPERM);
    expect("devnull_mode", fmode(n), S_IFCHR | 0666);
    return finish();
}
