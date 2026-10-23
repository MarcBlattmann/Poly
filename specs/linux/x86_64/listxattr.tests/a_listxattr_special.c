/* flistxattr on descriptors that are not files on a filesystem: pipes,
 * eventfds and memfds list nothing (0). Every socket (unix stream, unix
 * datagram, socketpair, IPv4) lists exactly one name,
 * "system.sockprotoname" (21 bytes with its NUL); size 0 gives 21, a
 * smaller buffer ERANGE. A unix socket's bound path is a filesystem object
 * whose own list is empty. listxattr on /dev/null is empty too. */
#include "a_sys.h"

#define SYS_socket 41
#define SYS_bind 49
#define SYS_socketpair 53
#define SYS_eventfd2 290
#define SYS_memfd_create 319
#define SYS_listxattr 194
#define SYS_flistxattr 196
#define D "/tmp/a_listxattr_special"
#define NAME "system.sockprotoname"

static char buf[256];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    int p[2];
    expect("setup_pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_read_end", flist(p[0]), 0);
    expect("pipe_write_end", flist(p[1]), 0);
    i64 ev = syscall2(SYS_eventfd2, 0, 0);
    check("setup_eventfd", ev >= 0, ev);
    expect("eventfd", flist(ev), 0);
    i64 mf = syscall2(SYS_memfd_create, "m", 0);
    check("setup_memfd", mf >= 0, mf);
    expect("memfd", flist(mf), 0);
    i64 us = syscall3(SYS_socket, 1, 1, 0);
    check("setup_unix_stream", us >= 0, us);
    expect("unix_stream", flist(us), 21);
    check("unix_stream_bytes", mem_eq(buf, NAME "\0", 21), 0);
    expect("unix_stream_size0", syscall3(SYS_flistxattr, us, 0, 0), 21);
    expect("unix_stream_small", syscall3(SYS_flistxattr, us, buf, 20), -34);
    i64 ud = syscall3(SYS_socket, 1, 2, 0);
    expect("unix_dgram", flist(ud), 21);
    int sp[2];
    expect("setup_socketpair", syscall4(SYS_socketpair, 1, 1, 0, sp), 0);
    expect("socketpair", flist(sp[1]), 21);
    check("socketpair_bytes", mem_eq(buf, NAME "\0", 21), 0);
    i64 in = syscall3(SYS_socket, 2, 2, 0);
    check("setup_inet", in >= 0, in);
    expect("inet_dgram", flist(in), 21);
    struct {
        unsigned short family;
        char path[108];
    } sa = { 1, D "/sock" };
    expect("setup_bind", syscall3(SYS_bind, us, &sa, sizeof sa), 0);
    expect("bound_path", syscall3(SYS_listxattr, D "/sock", buf, sizeof buf), 0);
    expect("bound_socket_fd", flist(us), 21);
    expect("dev_null", syscall3(SYS_listxattr, "/dev/null", buf, sizeof buf), 0);
    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
