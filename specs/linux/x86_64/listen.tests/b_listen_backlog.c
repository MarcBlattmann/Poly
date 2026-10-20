/* The listen backlog for AF_UNIX streams: with backlog N, N+1 pending
 * connections are queued (a nonblocking connect beyond that is EAGAIN);
 * accepting one makes room for one more; backlog 0 queues one; raising
 * the backlog with a second listen takes effect for new connections. */
#include "poly_ipc.h"

static int fill(int l, struct k_sockaddr_un *name, int len, int max) {
    int n = 0;
    while (n < max) {
        int c = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0);
        i64 r = syscall3(SYS_connect, c, name, len);
        if (r != 0) {
            close(c);
            if (r != -EAGAIN) return -100 - (int)-r;
            break;
        }
        n++;
    }
    (void)l;
    return n;
}

int test_main(void) {
    int l = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    struct k_sockaddr_un name = { AF_UNIX, { 0, 'b', '_', 'b', 'l', 'g' } };
    int len = 2 + 6;
    syscall3(SYS_bind, l, &name, len);
    syscall2(SYS_listen, l, 2);
    int n = fill(l, &name, len, 20);
    check("backlog_2_queues_3", n == 3, n);
    syscall4(SYS_accept4, l, 0, 0, 0);
    n = fill(l, &name, len, 20);
    check("accept_makes_room_for_one", n == 1, n);
    syscall2(SYS_listen, l, 5);
    n = fill(l, &name, len, 20);
    check("raised_backlog_queues_more", n == 3, n);
    close(l);

    int z = (int)syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    syscall3(SYS_bind, z, &name, len);
    syscall2(SYS_listen, z, 0);
    n = fill(z, &name, len, 20);
    check("backlog_0_queues_1", n == 1, n);
    return finish();
}
