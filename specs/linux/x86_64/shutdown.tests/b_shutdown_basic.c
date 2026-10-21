/* shutdown on stream sockets: SHUT_WR sends end-of-file to the peer
 * (its reads return 0 once the data is drained) and makes further writes
 * EPIPE (with SIGPIPE) while reading still works; after SHUT_RD the
 * caller reads what was queued, then 0 (AF_UNIX); SHUT_RDWR does both; poll shows
 * POLLRDHUP/POLLIN on the peer after SHUT_WR and POLLHUP when both
 * directions are shut; shutting down twice is allowed; an unconnected
 * AF_UNIX stream socket accepts it, an unconnected TCP socket is
 * ENOTCONN; `how` above 2 is EINVAL; a non-socket is ENOTSOCK. The descriptor stays open. */
#include "poly_ipc.h"

static volatile int pipes;
static void on_pipe(int sig) {
    (void)sig;
    pipes++;
}

int test_main(void) {
    sig_handler(SIGPIPE, on_pipe, 0);
    int sv[2];
    syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    write(sv[0], "abc", 3);
    i64 r = syscall2(SYS_shutdown, sv[0], SHUT_WR);
    check("shut_wr_ok", r == 0, r);
    char buf[8];
    r = read(sv[1], buf, sizeof buf);
    check("peer_reads_data_first", r == 3, r);
    r = read(sv[1], buf, sizeof buf);
    check("peer_then_eof", r == 0, r);
    r = write(sv[0], "x", 1);
    check("write_after_shut_wr_epipe", r == -EPIPE && pipes == 1, r);
    write(sv[1], "back", 4);
    r = read(sv[0], buf, sizeof buf);
    check("read_still_works", r == 4, r);
    struct k_pollfd pf = { sv[1], POLLIN | POLLRDHUP | POLLOUT, 0 };
    syscall3(SYS_poll, &pf, 1, 0);
    check("peer_sees_rdhup", pf.revents == (POLLIN | POLLRDHUP | POLLOUT), pf.revents);
    r = syscall2(SYS_shutdown, sv[0], SHUT_WR);
    check("shut_twice_ok", r == 0, r);

    r = syscall2(SYS_shutdown, sv[1], SHUT_WR);
    pf.fd = sv[0];
    syscall3(SYS_poll, &pf, 1, 0);
    check("both_shut_hup", (pf.revents & POLLHUP) != 0, pf.revents);
    close(sv[0]);
    close(sv[1]);

    syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    write(sv[1], "zz", 2);
    r = syscall2(SYS_shutdown, sv[0], SHUT_RD);
    check("shut_rd_ok", r == 0, r);
    r = read(sv[0], buf, sizeof buf);
    check("queued_data_still_read_after_shut_rd", r == 2, r);
    r = read(sv[0], buf, sizeof buf);
    check("then_eof_without_blocking", r == 0, r);
    r = write(sv[0], "w", 1);
    check("write_after_shut_rd_ok", r == 1, r);
    r = syscall2(SYS_shutdown, sv[0], SHUT_RDWR);
    check("shut_rdwr_ok", r == 0, r);
    r = write(sv[0], "w", 1);
    check("write_after_rdwr_epipe", r == -EPIPE, r);
    check("fd_still_open", syscall2(SYS_fcntl, sv[0], F_GETFD) == 0, 0);

    r = syscall2(SYS_shutdown, sv[0], 3);
    check("how_3_einval", r == -EINVAL, r);
    int s = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    r = syscall2(SYS_shutdown, s, SHUT_RDWR);
    check("unconnected_unix_accepted", r == 0, r);
    int t = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    r = syscall2(SYS_shutdown, t, SHUT_RDWR);
    check("unconnected_tcp_enotconn", r == -ENOTCONN, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = syscall2(SYS_shutdown, p[0], SHUT_RD);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    r = syscall2(SYS_shutdown, 90, SHUT_RD);
    check("closed_fd_ebadf", r == -EBADF, r);
    return finish();
}
