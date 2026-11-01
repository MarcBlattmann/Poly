/* sendmmsg(2) sends at most UIO_MAXIOV (1024) messages per call: a vlen
 * of 1500, or 0xffffffff, sends 1024 and returns 1024; entry 1024's
 * msg_len is not touched. Seen with one-byte UDP datagrams to a bound
 * loopback socket (UDP sends succeed even when the receiver drops some),
 * each entry carrying its own destination in msg_name.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sendmmsg 307

struct k_mmsghdr {
    struct k_msghdr hdr;
    unsigned len;
    unsigned pad;
};

static i64 sendmmsg(i64 fd, struct k_mmsghdr *v, u64 n, u64 flags) { return syscall4(SYS_sendmmsg, fd, v, n, flags); }
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }

static struct k_mmsghdr mv[1500];
static struct k_iovec iv;
static char byte = 'x';

int test_main(void) {
    i64 r = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    i64 s = syscall3(SYS_socket, AF_INET, SOCK_DGRAM, 0);
    check("sockets", r >= 0 && s >= 0, r);
    struct k_sockaddr_in a = in_loopback(0);
    expect("bind", syscall3(SYS_bind, r, &a, sizeof a), 0);
    int alen = sizeof a;
    expect("getsockname", syscall3(SYS_getsockname, r, &a, &alen), 0);

    iv.base = &byte;
    iv.len = 1;
    for (int i = 0; i < 1500; i++) {
        mv[i].hdr.name = &a;
        mv[i].hdr.namelen = sizeof a;
        mv[i].hdr.iov = &iv;
        mv[i].hdr.iovlen = 1;
        mv[i].len = 0xdead;
    }
    expect("vlen_1500_sends_1024", sendmmsg(s, mv, 1500, 0), 1024);
    expect("len_1023", mv[1023].len, 1);
    expect("len_1024_untouched", mv[1024].len, 0xdead);
    expect("vlen_max_sends_1024", sendmmsg(s, mv, 0xffffffffUL, 0), 1024);
    expect("vlen_1024", sendmmsg(s, mv, 1024, 0), 1024);
    expect("vlen_1023", sendmmsg(s, mv, 1023, 0), 1023);

    /* The receiver got (at least) the first datagram. */
    char buf[4];
    expect("received", syscall6(SYS_recvfrom, r, (i64)buf, 4, MSG_DONTWAIT, 0, 0), 1);

    close(r);
    close(s);
    return finish();
}
