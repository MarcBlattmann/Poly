/* sendmmsg(2) sends vlen messages, each like sendmsg(2) with the same
 * flags, and returns how many were sent; each sent entry's msg_len gets
 * the bytes sent for it, entries after the last sent one are not
 * touched. On an AF_UNIX datagram socketpair every entry is one datagram,
 * in order, scatter-gathered from its iovec. vlen 0 returns 0 (msgvec is
 * not read, NULL is fine). Only the low 32 bits of fd, vlen and flags
 * count. struct mmsghdr is struct msghdr (56 bytes) + unsigned msg_len,
 * 64 bytes.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sendmmsg 307

/* struct mmsghdr, linux/socket.h */
struct k_mmsghdr {
    struct k_msghdr hdr;
    unsigned len;
    unsigned pad;
};

static i64 sendmmsg(i64 fd, struct k_mmsghdr *v, u64 n, u64 flags) { return syscall4(SYS_sendmmsg, fd, v, n, flags); }
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }

static struct k_mmsghdr mv[4];
static struct k_iovec iv[4][2];
static char text[4][8] = { "msg0", "msg1", "msg2", "msg3" };

int test_main(void) {
    check("mmsghdr_64", sizeof(struct k_mmsghdr) == 64, sizeof(struct k_mmsghdr));
    int d[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, d), 0);
    for (int i = 0; i < 4; i++) {
        iv[i][0].base = text[i];
        iv[i][0].len = 3;
        iv[i][1].base = text[i] + 3;
        iv[i][1].len = 1;
        mv[i].hdr.iov = iv[i];
        mv[i].hdr.iovlen = 2;
        mv[i].len = 0xdead;
    }

    expect("send_3", sendmmsg(d[0], mv, 3, 0), 3);
    expect("len_0", mv[0].len, 4);
    expect("len_1", mv[1].len, 4);
    expect("len_2", mv[2].len, 4);
    expect("len_3_untouched", mv[3].len, 0xdead);
    char buf[16];
    for (int i = 0; i < 3; i++) {
        expect("recv", syscall6(SYS_recvfrom, d[1], (i64)buf, 16, MSG_DONTWAIT, 0, 0), 4);
        check("recv_data", buf[0] == 'm' && buf[3] == '0' + i, buf[3]);
    }
    expect("no_more", syscall6(SYS_recvfrom, d[1], (i64)buf, 16, MSG_DONTWAIT, 0, 0), -EAGAIN);

    expect("vlen_0", sendmmsg(d[0], mv, 0, 0), 0);
    expect("vlen_0_null", sendmmsg(d[0], 0, 0, 0), 0);
    expect("still_nothing", syscall6(SYS_recvfrom, d[1], (i64)buf, 16, MSG_DONTWAIT, 0, 0), -EAGAIN);

    /* msg_len is output only, msg_flags is ignored on input. */
    mv[0].hdr.flags = 0x7fffffff;
    mv[0].len = 12345;
    expect("flags_field_ignored", sendmmsg(d[0], mv, 1, MSG_DONTWAIT), 1);
    expect("len_overwritten", mv[0].len, 4);
    expect("recv_one", syscall6(SYS_recvfrom, d[1], (i64)buf, 16, MSG_DONTWAIT, 0, 0), 4);
    mv[0].hdr.flags = 0;

    expect("vlen_high_bits", sendmmsg(d[0], mv, (1UL << 32) + 2, 0), 2);
    expect("fd_high_bits", syscall4(SYS_sendmmsg, d[0] | (1L << 32), mv, 1, 0), 1);
    expect("flags_high_bits", syscall4(SYS_sendmmsg, d[0], mv, 1, 1L << 40), 1);
    expect("vlen_2p32_is_0", sendmmsg(d[0], mv, 1UL << 32, 0), 0);
    int n = 0;
    while (syscall6(SYS_recvfrom, d[1], (i64)buf, 16, MSG_DONTWAIT, 0, 0) == 4) n++;
    expect("queued_4", n, 4);

    close(d[0]);
    close(d[1]);
    return finish();
}
