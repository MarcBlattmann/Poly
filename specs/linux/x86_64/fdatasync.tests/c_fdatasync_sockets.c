/* fdatasync on sockets: EINVAL (Unix stream and datagram pairs, an
 * unconnected TCP and UDP socket). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_socket 41
#define SYS_socketpair 53
/* linux/socket.h, linux/net.h */
#define AF_UNIX 1
#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2

int test_main(void) {
    i32 sv[2];
    expect("unix_stream_pair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv), 0);
    expect("unix_stream_0", syscall1(SYS_fdatasync, sv[0]), -EINVAL);
    expect("unix_stream_1", syscall1(SYS_fdatasync, sv[1]), -EINVAL);
    a_close(sv[0]);
    a_close(sv[1]);
    expect("unix_dgram_pair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, sv), 0);
    expect("unix_dgram", syscall1(SYS_fdatasync, sv[0]), -EINVAL);
    a_close(sv[0]);
    a_close(sv[1]);
    i64 fd = syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    check("tcp_socket", fd >= 0, fd);
    expect("tcp", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    check("udp_socket", fd >= 0, fd);
    expect("udp", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    return finish();
}
