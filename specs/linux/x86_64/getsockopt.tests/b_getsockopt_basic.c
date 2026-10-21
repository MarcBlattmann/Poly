/* getsockopt at SOL_SOCKET: SO_TYPE, SO_DOMAIN, SO_PROTOCOL, SO_ERROR (0,
 * and reading clears a pending error), SO_ACCEPTCONN, SO_REUSEADDR and
 * SO_KEEPALIVE (0 by default), SO_RCVLOWAT (1), SO_SNDBUF and SO_RCVBUF
 * (positive); the int options write 4 bytes and set the length to 4, a
 * longer buffer still gets 4, a shorter one gets that many bytes; SO_LINGER
 * is 8 bytes; after a refused nonblocking connect SO_ERROR is ECONNREFUSED
 * once. Errors: unknown option ENOPROTOOPT, a TCP option on an AF_UNIX
 * socket EOPNOTSUPP, negative length EINVAL, bad length pointer EFAULT,
 * non-socket ENOTSOCK. */
#include "poly_ipc.h"

static int get(int fd, int name, int *len) {
    int v = -7;
    *len = 4;
    i64 r = syscall5(SYS_getsockopt, fd, SOL_SOCKET, name, &v, len);
    return r == 0 ? v : (int)r;
}

int test_main(void) {
    int s = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    int len;
    check("so_type", get(s, SO_TYPE, &len) == SOCK_STREAM && len == 4, len);
    check("so_domain", get(s, SO_DOMAIN, &len) == AF_INET, 0);
    check("so_protocol", get(s, SO_PROTOCOL, &len) == IPPROTO_TCP, 0);
    check("so_error_zero", get(s, SO_ERROR, &len) == 0, 0);
    check("so_acceptconn_zero", get(s, SO_ACCEPTCONN, &len) == 0, 0);
    check("so_reuseaddr_default_zero", get(s, SO_REUSEADDR, &len) == 0, 0);
    check("so_keepalive_default_zero", get(s, SO_KEEPALIVE, &len) == 0, 0);
    check("so_rcvlowat_default_one", get(s, SO_RCVLOWAT, &len) == 1, 0);
    check("so_sndbuf_positive", get(s, SO_SNDBUF, &len) > 0, 0);
    check("so_rcvbuf_positive", get(s, SO_RCVBUF, &len) > 0, 0);

    u64 big = 0x5555555555555555UL;
    int blen = 8;
    syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_TYPE, &big, &blen);
    check("long_buffer_gets_4", blen == 4 && (big >> 32) == 0x55555555 && (unsigned)big == SOCK_STREAM, blen);
    unsigned char one[4] = { 0x55, 0x55, 0x55, 0x55 };
    int olen = 1;
    syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_TYPE, one, &olen);
    check("short_buffer_one_byte", olen == 1 && one[0] == SOCK_STREAM && one[1] == 0x55, olen);
    int linger[2] = { -1, -1 };
    int llen = sizeof linger;
    i64 r = syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_LINGER, linger, &llen);
    check("so_linger_8_bytes_off", r == 0 && llen == 8 && linger[0] == 0 && linger[1] == 0, llen);

    /* A refused connect leaves SO_ERROR set; reading clears it. */
    int gone = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    struct k_sockaddr_in ga = in_loopback(0);
    syscall3(SYS_bind, gone, &ga, sizeof ga);
    len = sizeof ga;
    syscall3(SYS_getsockname, gone, &ga, &len);
    close(gone);
    int nb = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    r = syscall3(SYS_connect, nb, &ga, sizeof ga);
    struct k_pollfd pf = { nb, POLLOUT, 0 };
    syscall3(SYS_poll, &pf, 1, 2000);
    int e1 = get(nb, SO_ERROR, &len);
    int e2 = get(nb, SO_ERROR, &len);
    check("nonblocking_connect_einprogress", r == -EINPROGRESS, r);
    check("refused_connect_so_error", e1 == ECONNREFUSED, e1);
    check("so_error_cleared_by_read", e2 == 0, e2);

    check("unknown_option_enoprotoopt", get(s, 999, &len) == -ENOPROTOOPT, get(s, 999, &len));
    int v;
    len = -1;
    r = syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_TYPE, &v, &len);
    check("negative_length_einval", r == -EINVAL, r);
    len = 4;
    r = syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_TYPE, &v, 8);
    check("bad_length_pointer_efault", r == -EFAULT, r);
    int u = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    len = 4;
    r = syscall5(SYS_getsockopt, u, IPPROTO_TCP, TCP_NODELAY, &v, &len);
    check("tcp_level_on_unix_eopnotsupp", r == -EOPNOTSUPP, r);
    int pp[2];
    syscall2(SYS_pipe2, pp, 0);
    r = syscall5(SYS_getsockopt, pp[0], SOL_SOCKET, SO_TYPE, &v, &len);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    return finish();
}
