/* Every socket lists exactly one name, "system.sockprotoname" (21 bytes
 * with its NUL): unix stream and datagram sockets, both ends of a
 * socketpair, IPv4 TCP and UDP, also after the unix socket is bound to a
 * path. Size 0 gives 21, 20 or less ERANGE, 21 or more copies it; a NULL
 * buffer with size 21 is EFAULT. The bound path itself, opened with
 * O_PATH, is EBADF, and the socket keeps its one name after the path is
 * unlinked. */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_bind 49
#define SYS_socketpair 53
#define SYS_flistxattr 196
#define EBADF 9
#define D "/tmp/a_flistxattr_socket"
#define NAME "system.sockprotoname"

static char buf[64];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 us = syscall3(SYS_socket, 1, 1, 0);
    check("setup_unix_stream", us >= 0, us);
    expect("unix_stream", flist(us), 21);
    check("unix_stream_bytes", mem_eq(buf, NAME "\0", 21), 0);
    expect("size0", syscall3(SYS_flistxattr, us, 0, 0), 21);
    expect("size20", syscall3(SYS_flistxattr, us, buf, 20), -ERANGE);
    expect("size1", syscall3(SYS_flistxattr, us, buf, 1), -ERANGE);
    expect("size21", syscall3(SYS_flistxattr, us, buf, 21), 21);
    expect("null_size21", syscall3(SYS_flistxattr, us, 0, 21), -EFAULT);
    i64 ud = syscall3(SYS_socket, 1, 2, 0);
    check("setup_unix_dgram", ud >= 0, ud);
    expect("unix_dgram", flist(ud), 21);
    int sp[2];
    expect("setup_socketpair", syscall4(SYS_socketpair, 1, 1, 0, sp), 0);
    expect("socketpair_0", flist(sp[0]), 21);
    expect("socketpair_1", flist(sp[1]), 21);
    check("socketpair_bytes", mem_eq(buf, NAME "\0", 21), 0);
    i64 tcp = syscall3(SYS_socket, 2, 1, 0);
    check("setup_tcp", tcp >= 0, tcp);
    expect("inet_tcp", flist(tcp), 21);
    i64 udp = syscall3(SYS_socket, 2, 2, 0);
    check("setup_udp", udp >= 0, udp);
    expect("inet_udp", flist(udp), 21);
    struct {
        unsigned short family;
        char path[108];
    } sa = { 1, D "/sock" };
    expect("setup_bind", syscall3(SYS_bind, us, &sa, sizeof sa), 0);
    expect("bound", flist(us), 21);
    i64 op = a_open(D "/sock", O_PATH, 0);
    check("setup_opath_sock", op >= 0, op);
    expect("bound_path_opath", flist(op), -EBADF);
    a_close(op);
    expect("setup_unlink_sock", a_unlink(D "/sock"), 0);
    expect("bound_unlinked", flist(us), 21);
    a_close(udp);
    a_close(tcp);
    a_close(sp[0]);
    a_close(sp[1]);
    a_close(ud);
    a_close(us);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
