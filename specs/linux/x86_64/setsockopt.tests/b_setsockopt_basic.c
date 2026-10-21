/* setsockopt: boolean options (SO_REUSEADDR, SO_KEEPALIVE, SO_BROADCAST)
 * read back as 1 for any nonzero value and 0 for zero; SO_RCVLOWAT takes
 * the value; SO_RCVBUF and SO_SNDBUF are doubled (as root also below the
 * minimum, which is raised); SO_RCVTIMEO makes a blocking read fail with
 * EAGAIN after the time (and reads back; tv_usec of a million or more is
 * EDOM); TCP_NODELAY on a TCP socket;
 * read-only options (SO_TYPE, SO_ERROR, SO_PROTOCOL) are ENOPROTOOPT; an
 * option length below 4 is EINVAL; unknown options ENOPROTOOPT; a bad
 * pointer EFAULT; a non-socket ENOTSOCK. */
#include "poly_ipc.h"

#define EDOM 33

static int get(int fd, int level, int name) {
    int v = -7, len = 4;
    i64 r = syscall5(SYS_getsockopt, fd, level, name, &v, &len);
    return r == 0 ? v : (int)r;
}

static i64 set(int fd, int level, int name, int v) { return syscall5(SYS_setsockopt, fd, level, name, &v, 4); }

int test_main(void) {
    int s = (int)syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    i64 r = set(s, SOL_SOCKET, SO_REUSEADDR, 1);
    check("set_reuseaddr_ok", r == 0, r);
    check("reuseaddr_reads_one", get(s, SOL_SOCKET, SO_REUSEADDR) == 1, 0);
    set(s, SOL_SOCKET, SO_KEEPALIVE, 42);
    check("nonzero_reads_one", get(s, SOL_SOCKET, SO_KEEPALIVE) == 1, get(s, SOL_SOCKET, SO_KEEPALIVE));
    set(s, SOL_SOCKET, SO_KEEPALIVE, 0);
    check("zero_reads_zero", get(s, SOL_SOCKET, SO_KEEPALIVE) == 0, 0);
    set(s, SOL_SOCKET, SO_BROADCAST, -1);
    check("negative_reads_one", get(s, SOL_SOCKET, SO_BROADCAST) == 1, 0);
    set(s, SOL_SOCKET, SO_RCVLOWAT, 100);
    check("rcvlowat_value", get(s, SOL_SOCKET, SO_RCVLOWAT) == 100, get(s, SOL_SOCKET, SO_RCVLOWAT));

    set(s, SOL_SOCKET, SO_RCVBUF, 65536);
    check("rcvbuf_doubled", get(s, SOL_SOCKET, SO_RCVBUF) == 131072, get(s, SOL_SOCKET, SO_RCVBUF));
    set(s, SOL_SOCKET, SO_SNDBUF, 50000);
    check("sndbuf_doubled", get(s, SOL_SOCKET, SO_SNDBUF) == 100000, get(s, SOL_SOCKET, SO_SNDBUF));
    set(s, SOL_SOCKET, SO_RCVBUF, 1);
    int small = get(s, SOL_SOCKET, SO_RCVBUF);
    check("tiny_rcvbuf_raised_to_minimum", small > 2 && small <= 4608, small);

    struct k_timeval tv = { 0, 20000 };
    r = syscall5(SYS_setsockopt, s, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    check("set_rcvtimeo_ok", r == 0, r);
    struct k_timeval back = { 0, 0 };
    int blen = sizeof back;
    syscall5(SYS_getsockopt, s, SOL_SOCKET, SO_RCVTIMEO, &back, &blen);
    check("rcvtimeo_reads_back", blen == 16 && back.tv_sec == 0 && back.tv_usec == 20000, back.tv_usec);
    struct k_sockaddr_in a = in_loopback(0);
    syscall3(SYS_bind, s, &a, sizeof a);
    char buf[4];
    i64 t0 = now_ns();
    r = read(s, buf, sizeof buf);
    i64 dt = now_ns() - t0;
    check("read_times_out_eagain", r == -EAGAIN, r);
    check("timeout_waited", dt >= 19000000, dt / 1000000);
    struct k_timeval badtv = { 0, 1000000 };
    r = syscall5(SYS_setsockopt, s, SOL_SOCKET, SO_RCVTIMEO, &badtv, sizeof badtv);
    check("rcvtimeo_usec_out_of_range_edom", r == -EDOM, r);

    int t = (int)syscall3(SYS_socket, AF_INET, SOCK_STREAM, 0);
    check("nodelay_default_zero", get(t, IPPROTO_TCP, TCP_NODELAY) == 0, 0);
    set(t, IPPROTO_TCP, TCP_NODELAY, 1);
    check("nodelay_set", get(t, IPPROTO_TCP, TCP_NODELAY) == 1, 0);

    r = set(s, SOL_SOCKET, SO_TYPE, SOCK_STREAM);
    check("so_type_read_only_enoprotoopt", r == -ENOPROTOOPT, r);
    r = set(s, SOL_SOCKET, SO_ERROR, 0);
    check("so_error_read_only_enoprotoopt", r == -ENOPROTOOPT, r);
    r = set(s, SOL_SOCKET, SO_PROTOCOL, 0);
    check("so_protocol_read_only_enoprotoopt", r == -ENOPROTOOPT, r);
    int v = 1;
    r = syscall5(SYS_setsockopt, s, SOL_SOCKET, SO_REUSEADDR, &v, 2);
    check("short_length_einval", r == -EINVAL, r);
    r = set(s, SOL_SOCKET, 999, 1);
    check("unknown_option_enoprotoopt", r == -ENOPROTOOPT, r);
    r = syscall5(SYS_setsockopt, s, SOL_SOCKET, SO_REUSEADDR, 8, 4);
    check("bad_pointer_efault", r == -EFAULT, r);
    int pp[2];
    syscall2(SYS_pipe2, pp, 0);
    r = set(pp[0], SOL_SOCKET, SO_REUSEADDR, 1);
    check("not_socket_enotsock", r == -ENOTSOCK, r);
    return finish();
}
