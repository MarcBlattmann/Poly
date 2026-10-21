/* accept4: takes the oldest waiting connection and returns a new connected
 * socket (the lowest free fd); the peer address is written with its
 * length (the length is in/out, a short buffer is cut but the full
 * length is reported); NULL address is allowed; SOCK_NONBLOCK and
 * SOCK_CLOEXEC set on the new socket (and never inherited from the
 * listener); a nonblocking listener with nothing waiting is EAGAIN (that
 * is reported before a bad address length, which is EINVAL);
 * unknown flags are EINVAL; a socket that is not listening is EINVAL; a
 * datagram socket is EOPNOTSUPP. */
#include "poly_ipc.h"

int test_main(void) {
    int l = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    struct k_sockaddr_in la = in_loopback(0);
    syscall3(SYS_bind, l, &la, sizeof la);
    syscall2(SYS_listen, l, 8);
    int alen = sizeof la;
    syscall3(SYS_getsockname, l, &la, &alen);

    i64 r = syscall4(SYS_accept4, l, 0, 0, 0);
    check("nothing_waiting_eagain", r == -EAGAIN, r);

    int c1 = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    int c2 = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    syscall3(SYS_connect, c1, &la, sizeof la);
    syscall3(SYS_connect, c2, &la, sizeof la);
    write(c1, "1", 1);
    write(c2, "2", 1);

    struct k_sockaddr_in peer;
    int plen = sizeof peer;
    i64 s1 = syscall4(SYS_accept4, l, &peer, &plen, 0);
    check("accept_returns_fd", s1 == c2 + 1, s1);
    struct k_sockaddr_in c1name;
    int c1len = sizeof c1name;
    syscall3(SYS_getsockname, c1, &c1name, &c1len);
    check("peer_address_is_client", plen == 16 && peer.sin_port == c1name.sin_port, plen);
    char ch = 0;
    struct k_pollfd pf = { (int)s1, POLLIN, 0 };
    syscall3(SYS_poll, &pf, 1, 2000);
    read((int)s1, &ch, 1);
    check("oldest_connection_first", ch == '1', ch);
    check("accepted_not_nonblocking", syscall2(SYS_fcntl, s1, F_GETFL) == O_RDWR, syscall2(SYS_fcntl, s1, F_GETFL));
    check("accepted_no_cloexec", syscall2(SYS_fcntl, s1, F_GETFD) == 0, 0);

    int shortlen = 4;
    struct k_sockaddr_in cut;
    memset(&cut, 0x55, sizeof cut);
    i64 s2 = syscall4(SYS_accept4, l, &cut, &shortlen, SOCK_NONBLOCK | SOCK_CLOEXEC);
    check("second_accept", s2 == s1 + 1, s2);
    check("short_buffer_full_length_reported", shortlen == 16, shortlen);
    check("short_buffer_cut", cut.sin_family == AF_INET && cut.sin_addr == 0x55555555, 0);
    check("flags_on_new_socket", syscall2(SYS_fcntl, s2, F_GETFL) == (O_RDWR | O_NONBLOCK) &&
                                     syscall2(SYS_fcntl, s2, F_GETFD) == FD_CLOEXEC,
          0);

    r = syscall4(SYS_accept4, l, 0, 0, 0x1);
    check("unknown_flag_einval", r == -EINVAL, r);
    int bad = -1;
    r = syscall4(SYS_accept4, l, &peer, &bad, 0);
    check("negative_length_nothing_waiting_eagain", r == -EAGAIN, r);
    int c3 = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    syscall3(SYS_connect, c3, &la, sizeof la);
    r = syscall4(SYS_accept4, l, &peer, &bad, 0);
    check("negative_length_einval", r == -EINVAL, r);
    int nl = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    r = syscall4(SYS_accept4, nl, 0, 0, 0);
    check("not_listening_einval", r == -EINVAL, r);
    int d = (int)syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    r = syscall4(SYS_accept4, d, 0, 0, 0);
    check("dgram_eopnotsupp", r == -EOPNOTSUPP, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = syscall4(SYS_accept4, p[0], 0, 0, 0);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    return finish();
}
