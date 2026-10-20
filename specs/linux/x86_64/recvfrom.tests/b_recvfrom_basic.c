/* recvfrom: datagrams one at a time with the sender's address (the length
 * is in/out; NULL address is fine); MSG_PEEK leaves the datagram queued;
 * MSG_TRUNC returns the real length of a longer datagram; MSG_DONTWAIT on
 * an empty queue is EAGAIN; on an AF_UNIX stream no address is returned
 * (the length is set to 0); MSG_WAITALL on a stream waits for the full
 * count; a bad buffer is EFAULT (an empty nonblocking queue reports
 * EAGAIN first); a non-socket ENOTSOCK.
 * poly-diff: syscalls=off (a child process writes in two steps)
 */
#include "poly_ipc.h"

int test_main(void) {
    int a = (int)syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    int b = (int)syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    struct k_sockaddr_in aa = in_loopback(0), ba = in_loopback(0);
    syscall3(SYS_bind, a, &aa, sizeof aa);
    syscall3(SYS_bind, b, &ba, sizeof ba);
    int len = sizeof aa;
    syscall3(SYS_getsockname, a, &aa, &len);
    len = sizeof ba;
    syscall3(SYS_getsockname, b, &ba, &len);

    char buf[16];
    i64 r = syscall6(SYS_recvfrom, a, (i64)buf, sizeof buf, MSG_DONTWAIT, 0, 0);
    check("empty_eagain", r == -EAGAIN, r);
    syscall6(SYS_sendto, b, (i64) "first", 5, 0, (i64)&aa, sizeof aa);
    syscall6(SYS_sendto, b, (i64) "second!", 7, 0, (i64)&aa, sizeof aa);

    struct k_sockaddr_in from;
    int flen = sizeof from;
    r = syscall6(SYS_recvfrom, a, (i64)buf, sizeof buf, MSG_PEEK, (i64)&from, (i64)&flen);
    check("peek_first", r == 5 && mem_eq(buf, "first", 5), r);
    check("peek_sender", flen == 16 && from.sin_port == ba.sin_port && from.sin_addr == be32(INADDR_LOOPBACK), flen);
    r = syscall6(SYS_recvfrom, a, (i64)buf, sizeof buf, 0, 0, 0);
    check("peeked_still_there", r == 5, r);
    r = syscall6(SYS_recvfrom, a, (i64)buf, 3, MSG_TRUNC, 0, 0);
    check("trunc_real_length", r == 7, r);
    r = syscall6(SYS_recvfrom, a, (i64)buf, sizeof buf, MSG_DONTWAIT, 0, 0);
    check("truncated_rest_dropped", r == -EAGAIN, r);

    syscall6(SYS_sendto, b, (i64) "abc", 3, 0, (i64)&aa, sizeof aa);
    flen = 4;
    memset(&from, 0x55, sizeof from);
    r = syscall6(SYS_recvfrom, a, (i64)buf, sizeof buf, 0, (i64)&from, (i64)&flen);
    check("short_address_buffer_full_length", r == 3 && flen == 16, flen);
    check("short_address_buffer_cut", from.sin_family == AF_INET && from.sin_addr == 0x55555555, 0);

    /* Streams. */
    int sv[2];
    syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, sv);
    write(sv[1], "1234", 4);
    struct k_sockaddr_un un;
    int ulen = sizeof un;
    memset(&un, 0x55, sizeof un);
    r = syscall6(SYS_recvfrom, sv[0], (i64)buf, sizeof buf, 0, (i64)&un, (i64)&ulen);
    check("stream_recvfrom_bytes", r == 4, r);
    check("stream_address_length_zero", ulen == 0, ulen);
    /* The child stays until the parent has read: its exit (SIGCHLD) must
     * not fall into the wait. */
    int done[2];
    syscall2(SYS_pipe2, done, 0);
    i64 pid = fork();
    if (pid == 0) {
        char c;
        close(done[1]);
        write(sv[1], "ab", 2);
        sleep_ms(20);
        write(sv[1], "cd", 2);
        read(done[0], &c, 1);
        exit_group(0);
    }
    close(done[0]);
    r = syscall6(SYS_recvfrom, sv[0], (i64)buf, 4, MSG_WAITALL, 0, 0);
    check("waitall_full_count", r == 4 && mem_eq(buf, "abcd", 4), r);
    close(done[1]);
    wait_exit(pid);

    r = syscall6(SYS_recvfrom, sv[0], 8, 4, MSG_DONTWAIT, 0, 0);
    check("empty_bad_buffer_eagain_first", r == -EAGAIN, r);
    write(sv[1], "q", 1);
    r = syscall6(SYS_recvfrom, sv[0], 8, 4, 0, 0, 0);
    check("bad_buffer_efault", r == -EFAULT, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    r = syscall6(SYS_recvfrom, p[0], (i64)buf, 1, 0, 0, 0);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    return finish();
}
