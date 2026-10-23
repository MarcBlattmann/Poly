/* fadvise64 on descriptors that are neither files nor pipes: an eventfd,
 * an epoll instance and both ends of a UNIX socketpair accept every valid
 * advice. A bad advice or a negative length is still EINVAL. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define SYS_socketpair 53
#define SYS_eventfd2 290
#define SYS_epoll_create1 291

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

static int all_ok(i64 fd) {
    int ok = 1;
    for (i64 a = 0; a <= 5; a++) ok &= fadv(fd, 0, 0, a) == 0;
    return ok;
}

int test_main(void) {
    i64 ev = syscall2(SYS_eventfd2, 0, 0);
    check("eventfd", ev >= 0, ev);
    check("eventfd_all_advice", all_ok(ev), 0);
    expect("eventfd_bad_advice", fadv(ev, 0, 0, 99), -EINVAL);
    expect("eventfd_negative_len", fadv(ev, 0, -1, 0), -EINVAL);
    u64 one = 1;
    expect("eventfd_still_works", a_write(ev, &one, 8), 8);
    a_close(ev);
    i64 ep = syscall1(SYS_epoll_create1, 0);
    check("epoll", ep >= 0, ep);
    check("epoll_all_advice", all_ok(ep), 0);
    expect("epoll_bad_advice", fadv(ep, 0, 0, 6), -EINVAL);
    a_close(ep);
    int sv[2] = {-1, -1};
    expect("socketpair", syscall4(SYS_socketpair, 1 /* AF_UNIX */, 1 /* SOCK_STREAM */, 0, sv), 0);
    check("socket0_all_advice", all_ok(sv[0]), 0);
    check("socket1_all_advice", all_ok(sv[1]), 0);
    expect("socket_bad_advice", fadv(sv[0], 0, 0, 8), -EINVAL);
    expect("socket_negative_len", fadv(sv[1], 0, -1, 4), -EINVAL);
    expect("socket_negative_offset", fadv(sv[1], -1, 0, 4), 0);
    expect("socket_send", a_write(sv[0], "hi", 2), 2);
    char b[2] = {0, 0};
    expect("socket_recv", a_read(sv[1], b, 2), 2);
    check("socket_data", b[0] == 'h' && b[1] == 'i', b[0]);
    a_close(sv[0]);
    a_close(sv[1]);
    return finish();
}
