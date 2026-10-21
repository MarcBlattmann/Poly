/* sendto on stream sockets: on a connected stream without an address the
 * data goes to the peer; an address on a connected AF_UNIX stream is
 * EISCONN; on an
 * unconnected stream socket sendto is ENOTCONN (TCP: EPIPE); MSG_DONTWAIT
 * on a full AF_UNIX stream is EAGAIN even for a blocking socket.
 * poly-diff: syscalls=off (filling a socket buffer takes a number of
 * calls that depends on the host's buffer sizes)
 */
#include "poly_ipc.h"

static char big[1 << 20];

int test_main(void) {
    sig_action(SIGPIPE, SIG_IGN, 0, 0);
    int sv[2];
    syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    i64 r = syscall6(SYS_sendto, sv[0], (i64) "hi", 2, 0, 0, 0);
    check("connected_no_address", r == 2, r);
    struct k_sockaddr_un any = { AF_UNIX, { 0, 'x' } };
    r = syscall6(SYS_sendto, sv[0], (i64) "hi", 2, 0, (i64)&any, 4);
    check("unix_stream_with_address_eisconn", r == -EISCONN, r);
    char buf[8];
    r = read(sv[1], buf, sizeof buf);
    check("peer_got_data", r == 2, r);

    int lone = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    r = syscall6(SYS_sendto, lone, (i64) "x", 1, 0, 0, 0);
    check("unconnected_unix_stream_enotconn", r == -ENOTCONN, r);
    int t = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    r = syscall6(SYS_sendto, t, (i64) "x", 1, 0, 0, 0);
    check("unconnected_tcp_epipe", r == -EPIPE, r);

    i64 total = 0;
    for (;;) {
        r = syscall6(SYS_sendto, sv[0], (i64)big, sizeof big, MSG_DONTWAIT, 0, 0);
        if (r <= 0) break;
        total += r;
    }
    check("dontwait_full_eagain", r == -EAGAIN, r);
    check("some_data_went_first", total > 0, total);
    return finish();
}
