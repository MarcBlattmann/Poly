/* getpeername: the connected peer's address. Unconnected sockets (stream
 * or datagram, and a listening socket) are ENOTCONN; for a TCP connection
 * each side reports the other's address and port; for an AF_UNIX
 * connection to a named listener the client reports the listener's name
 * and the server side reports the client's (unnamed: length 2); after the
 * peer closes, a TCP socket still reports it, a stream socketpair end
 * too. Errors as for getsockname (ENOTSOCK, EBADF, EINVAL, EFAULT). */
#include "poly_ipc.h"

int test_main(void) {
    int s = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    struct k_sockaddr_in a;
    int len = sizeof a;
    i64 r = syscall3(SYS_getpeername, s, &a, &len);
    check("unconnected_tcp_enotconn", r == -ENOTCONN, r);
    struct k_sockaddr_in la = in_loopback(0);
    syscall3(SYS_bind, s, &la, sizeof la);
    syscall2(SYS_listen, s, 2);
    r = syscall3(SYS_getpeername, s, &a, &len);
    check("listener_enotconn", r == -ENOTCONN, r);
    len = sizeof la;
    syscall3(SYS_getsockname, s, &la, &len);

    int c = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    syscall3(SYS_connect, c, &la, sizeof la);
    int acc = (int)syscall4(SYS_accept4, s, 0, 0, 0);
    struct k_sockaddr_in cp, sp, cn;
    len = sizeof cp;
    syscall3(SYS_getpeername, c, &cp, &len);
    check("client_peer_is_server_port", len == 16 && cp.sin_port == la.sin_port && cp.sin_addr == be32(INADDR_LOOPBACK),
          len);
    len = sizeof sp;
    syscall3(SYS_getpeername, acc, &sp, &len);
    len = sizeof cn;
    syscall3(SYS_getsockname, c, &cn, &len);
    check("server_peer_is_client", sp.sin_port == cn.sin_port, 0);
    close(acc);
    sleep_ms(10);
    len = sizeof cp;
    r = syscall3(SYS_getpeername, c, &cp, &len);
    check("tcp_peer_after_close_still_reported", r == 0 && cp.sin_port == la.sin_port, r);

    int d = (int)syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    len = sizeof a;
    r = syscall3(SYS_getpeername, d, &a, &len);
    check("unconnected_udp_enotconn", r == -ENOTCONN, r);

    int ul = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un name = { AF_UNIX, { 0, 'b', '_', 'g', 'p', 'n' } };
    syscall3(SYS_bind, ul, &name, 2 + 6);
    syscall2(SYS_listen, ul, 2);
    int uc = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    syscall3(SYS_connect, uc, &name, 2 + 6);
    int ua = (int)syscall4(SYS_accept4, ul, 0, 0, 0);
    struct k_sockaddr_un up;
    len = sizeof up;
    syscall3(SYS_getpeername, uc, &up, &len);
    check("unix_client_peer_is_listener_name", len == 2 + 6 && mem_eq(&up, &name, 8), len);
    len = sizeof up;
    syscall3(SYS_getpeername, ua, &up, &len);
    check("unix_server_peer_unnamed", len == 2, len);

    int sv[2];
    syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    close(sv[1]);
    len = sizeof up;
    r = syscall3(SYS_getpeername, sv[0], &up, &len);
    check("socketpair_peer_closed_still_ok", r == 0 && len == 2, r);

    int pp[2];
    syscall2(SYS_pipe2, pp, 0);
    r = syscall3(SYS_getpeername, pp[0], &a, &len);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    len = -5;
    r = syscall3(SYS_getpeername, uc, &up, &len);
    check("negative_length_einval", r == -EINVAL, r);
    return finish();
}
