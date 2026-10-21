/* accept (syscall 43): accept4 with flags 0. For AF_UNIX the peer address
 * of a client that never bound is unnamed (length 2); a client bound to an
 * abstract name is reported by that name; the accepted socket's own name
 * is the listener's; SO_PEERCRED on both ends names the other process
 * (here the same one); a NULL address is allowed. */
#include "poly_ipc.h"

int test_main(void) {
    int l = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un any = { AF_UNIX, { 0 } };
    syscall3(SYS_bind, l, &any, 2);
    syscall2(SYS_listen, l, 4);
    struct k_sockaddr_un ln;
    int llen = sizeof ln;
    syscall3(SYS_getsockname, l, &ln, &llen);

    int c = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    syscall3(SYS_connect, c, &ln, llen);
    struct k_sockaddr_un peer;
    int plen = sizeof peer;
    i64 s = syscall3(SYS_accept, l, &peer, &plen);
    check("accept_ok", s == c + 1, s);
    check("unbound_client_unnamed", plen == 2 && peer.sun_family == AF_UNIX, plen);
    struct k_sockaddr_un own;
    int olen = sizeof own;
    syscall3(SYS_getsockname, s, &own, &olen);
    check("accepted_name_is_listener", olen == llen && mem_eq(&own, &ln, (u64)llen), olen);
    check("accepted_flags_plain", syscall2(SYS_fcntl, s, F_GETFL) == O_RDWR && syscall2(SYS_fcntl, s, F_GETFD) == 0, 0);
    struct k_ucred cr;
    int crlen = sizeof cr;
    syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_PEERCRED, &cr, &crlen);
    check("server_side_peercred", crlen == 12 && cr.pid == getpid(), cr.pid);
    crlen = sizeof cr;
    syscall5(SYS_getsockopt, c, SOL_SOCKET, SO_PEERCRED, &cr, &crlen);
    check("client_side_peercred", cr.pid == getpid(), cr.pid);

    int d = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un dn = { AF_UNIX, { 0, 'b', '_', 'c', 'l', 'i' } };
    syscall3(SYS_bind, d, &dn, 2 + 6);
    syscall3(SYS_connect, d, &ln, llen);
    plen = sizeof peer;
    s = syscall3(SYS_accept, l, &peer, &plen);
    check("bound_client_named", plen == 2 + 6 && mem_eq(&peer, &dn, 8), plen);
    write(d, "ping", 4);
    char buf[8];
    i64 r = read((int)s, buf, sizeof buf);
    check("accepted_socket_carries_data", r == 4, r);
    int nb = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    syscall3(SYS_connect, nb, &ln, llen);
    s = syscall3(SYS_accept, l, 0, 0);
    check("null_address_ok", s > 0, s);
    return finish();
}
