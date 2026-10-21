/* connect for stream sockets: to a listening AF_UNIX abstract name or a
 * loopback TCP port it succeeds at once (the connection waits in the
 * backlog until accepted); getpeername then names the listener; a second
 * connect is EISCONN; connecting to a name nobody listens on is
 * ECONNREFUSED (TCP: to a closed port too), a missing socket path is
 * ENOENT; a wrong family is EINVAL for AF_UNIX and EAFNOSUPPORT for
 * AF_INET; a short address length is EINVAL. Nonblocking TCP connect to
 * loopback returns EINPROGRESS and is writable once connected (SO_ERROR 0). */
#include "poly_ipc.h"

int test_main(void) {
    int l = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un any = { AF_UNIX, { 0 } };
    syscall3(SYS_bind, l, &any, 2);
    syscall2(SYS_listen, l, 4);
    struct k_sockaddr_un name;
    int nlen = sizeof name;
    syscall3(SYS_getsockname, l, &name, &nlen);

    int c = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    i64 r = syscall3(SYS_connect, c, &name, nlen);
    check("unix_connect_ok", r == 0, r);
    struct k_sockaddr_un peer;
    int plen = sizeof peer;
    syscall3(SYS_getpeername, c, &peer, &plen);
    check("peer_is_listener_name", plen == nlen && mem_eq(&peer, &name, (u64)nlen), plen);
    r = syscall3(SYS_connect, c, &name, nlen);
    check("second_connect_eisconn", r == -EISCONN, r);
    r = write(c, "q", 1);
    check("write_before_accept_ok", r == 1, r);

    struct k_sockaddr_un nobody = { AF_UNIX, { 0, 'b', '_', 'n', 'o', 'b', 'o', 'd', 'y' } };
    int d = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    r = syscall3(SYS_connect, d, &nobody, 2 + 9);
    check("unix_no_listener_econnrefused", r == -ECONNREFUSED, r);
    int bound_only = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un bo = { AF_UNIX, { 0, 'b', '_', 'b', 'o' } };
    syscall3(SYS_bind, bound_only, &bo, 2 + 5);
    r = syscall3(SYS_connect, d, &bo, 2 + 5);
    check("unix_bound_not_listening_econnrefused", r == -ECONNREFUSED, r);
    struct k_sockaddr_un missing = { AF_UNIX, "/tmp/b_connect_missing.sock" };
    r = syscall3(SYS_connect, d, &missing, sizeof missing);
    check("unix_missing_path_enoent", r == -ENOENT, r);
    struct k_sockaddr_in in = in_loopback(1);
    r = syscall3(SYS_connect, d, &in, sizeof in);
    check("unix_wrong_family_einval", r == -EINVAL, r);
    r = syscall3(SYS_connect, d, &name, 1);
    check("unix_short_length_einval", r == -EINVAL, r);

    /* TCP over loopback. */
    int tl = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    struct k_sockaddr_in la = in_loopback(0);
    syscall3(SYS_bind, tl, &la, sizeof la);
    syscall2(SYS_listen, tl, 4);
    int alen = sizeof la;
    syscall3(SYS_getsockname, tl, &la, &alen);
    int tc = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    r = syscall3(SYS_connect, tc, &la, sizeof la);
    check("tcp_connect_ok", r == 0, r);
    struct k_sockaddr_in local;
    alen = sizeof local;
    syscall3(SYS_getsockname, tc, &local, &alen);
    check("tcp_client_autobound", local.sin_port != 0 && local.sin_addr == be32(INADDR_LOOPBACK), 0);
    r = syscall3(SYS_connect, tc, &la, sizeof la);
    check("tcp_second_connect_eisconn", r == -EISCONN, r);

    /* A closed port: the listener goes away. */
    int gone = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    struct k_sockaddr_in ga = in_loopback(0);
    syscall3(SYS_bind, gone, &ga, sizeof ga);
    alen = sizeof ga;
    syscall3(SYS_getsockname, gone, &ga, &alen);
    close(gone);
    int tr = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    r = syscall3(SYS_connect, tr, &ga, sizeof ga);
    check("tcp_closed_port_econnrefused", r == -ECONNREFUSED, r);
    struct k_sockaddr_un wrong = { AF_UNIX, { 0 } };
    int tw = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    r = syscall3(SYS_connect, tw, &wrong, sizeof(struct k_sockaddr_in));
    check("tcp_wrong_family_eafnosupport", r == -EAFNOSUPPORT, r);
    r = syscall3(SYS_connect, tw, &la, 8);
    check("tcp_short_length_einval", r == -EINVAL, r);

    int nb = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0);
    r = syscall3(SYS_connect, nb, &la, sizeof la);
    check("nonblocking_connect_einprogress", r == -EINPROGRESS, r);
    struct k_pollfd pf = { nb, POLLOUT, 0 };
    r = syscall3(SYS_poll, &pf, 1, 2000);
    check("nonblocking_connect_writable", r == 1 && pf.revents == POLLOUT, pf.revents);
    int err = -1, elen = 4;
    syscall5(SYS_getsockopt, nb, SOL_SOCKET, SO_ERROR, &err, &elen);
    check("nonblocking_connect_so_error_zero", err == 0, err);
    return finish();
}
