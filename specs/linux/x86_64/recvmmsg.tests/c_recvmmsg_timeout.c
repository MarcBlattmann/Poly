/* recvmmsg(2)'s timeout is looked at only after a message was received:
 * with {0,0} and two queued messages (vlen 5) the call returns after the
 * first (1), the second stays queued. With 5 s and both wanted messages
 * queued it returns 2 at once and writes the time left back (more than
 * 4 s, less than 5 s, tv_nsec below 10^9). When the call fails, the
 * timeout is not written. A timeout that cannot be written back after
 * messages were received makes the call fail with EFAULT (the message is
 * consumed). MSG_DONTWAIT on an empty socket is EAGAIN whatever the
 * timeout.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_recvmmsg 299
/* linux/socket.h */
#define MSG_WAITFORONE 0x10000

struct k_mmsghdr {
    struct k_msghdr hdr;
    unsigned len;
    unsigned pad;
};

static i64 recvmmsg(i64 fd, struct k_mmsghdr *v, u64 n, u64 flags, void *ts) {
    return syscall5(SYS_recvmmsg, fd, v, n, flags, ts);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }

static struct k_mmsghdr mv[5];
static struct k_iovec iv[5];
static char bufs[5][16];

int test_main(void) {
    int d[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, d), 0);
    for (int i = 0; i < 5; i++) {
        iv[i].base = bufs[i];
        iv[i].len = 16;
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
    }
    struct k_timespec ts = { 0, 0 };

    write(d[0], "one", 3);
    write(d[0], "two", 3);
    expect("zero_timeout_returns_1", recvmmsg(d[1], mv, 5, 0, &ts), 1);
    check("zero_timeout_kept_zero", ts.tv_sec == 0 && ts.tv_nsec == 0, ts.tv_nsec);
    expect("second_still_queued", recvmmsg(d[1], mv, 5, MSG_DONTWAIT, 0), 1);

    expect("empty_dontwait_eagain", recvmmsg(d[1], mv, 5, MSG_DONTWAIT, &ts), -EAGAIN);

    write(d[0], "one", 3);
    write(d[0], "two", 3);
    ts.tv_sec = 5;
    ts.tv_nsec = 0;
    expect("five_seconds_returns_2", recvmmsg(d[1], mv, 2, 0, &ts), 2);
    check("time_left_written", ts.tv_sec == 4 && ts.tv_nsec >= 0 && ts.tv_nsec < 1000000000, ts.tv_sec);

    ts.tv_sec = 3;
    ts.tv_nsec = 123;
    expect("error_eagain", recvmmsg(d[1], mv, 2, MSG_DONTWAIT, &ts), -EAGAIN);
    check("error_ts_untouched", ts.tv_sec == 3 && ts.tv_nsec == 123, ts.tv_sec);

    write(d[0], "w", 1);
    ts.tv_sec = 3;
    ts.tv_nsec = 0;
    expect("waitforone_with_timeout", recvmmsg(d[1], mv, 2, MSG_WAITFORONE, &ts), 1);
    check("waitforone_time_left", ts.tv_sec == 2, ts.tv_sec);

    /* A read-only timeout {0,0}: one message received, then the write-back fails. */
    i64 page = syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", page > 0, page);
    write(d[0], "a", 1);
    write(d[0], "b", 1);
    expect("readonly_ts_efault", recvmmsg(d[1], mv, 4, MSG_DONTWAIT, (void *)page), -EFAULT);
    expect("first_consumed_second_left", recvmmsg(d[1], mv, 4, MSG_DONTWAIT, 0), 1);
    check("left_is_b", bufs[0][0] == 'b', bufs[0][0]);

    close(d[0]);
    close(d[1]);
    return finish();
}
