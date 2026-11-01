/* recvmmsg(2) receives up to vlen messages, each like recvmsg(2), and
 * returns how many it got; each filled entry's msg_len is the bytes
 * received and its msg_flags the per-message flags (0 here); entries
 * after the last one are not touched. MSG_DONTWAIT: takes what is queued
 * and stops when the queue is empty (EAGAIN only if nothing was queued).
 * MSG_WAITFORONE: waits for the first message only, then takes what is
 * queued. Without either, the call waits until vlen messages have
 * arrived (here: all are already queued). Messages beyond vlen stay
 * queued. vlen 0 returns 0 at once.
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
static char bufs[5][32];
static void setup(void) {
    for (int i = 0; i < 5; i++) {
        memset(&mv[i], 0, sizeof mv[i]);
        memset(bufs[i], 0, 32);
        iv[i].base = bufs[i];
        iv[i].len = 32;
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
        mv[i].hdr.flags = 0x5555;
        mv[i].len = 0xdead;
    }
}

int test_main(void) {
    int d[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, d), 0);

    write(d[0], "one", 3);
    write(d[0], "two22", 5);
    write(d[0], "three", 5);
    setup();
    expect("dontwait_takes_3", recvmmsg(d[1], mv, 5, MSG_DONTWAIT, 0), 3);
    expect("len_0", mv[0].len, 3);
    expect("len_1", mv[1].len, 5);
    expect("len_2", mv[2].len, 5);
    expect("len_3_untouched", mv[3].len, 0xdead);
    expect("flags_0", mv[0].hdr.flags, 0);
    expect("flags_3_untouched", mv[3].hdr.flags, 0x5555);
    check("data", mem_eq(bufs[0], "one", 3) && mem_eq(bufs[1], "two22", 5) && mem_eq(bufs[2], "three", 5), 0);
    expect("empty_dontwait_eagain", recvmmsg(d[1], mv, 5, MSG_DONTWAIT, 0), -EAGAIN);

    write(d[0], "a", 1);
    write(d[0], "bb", 2);
    setup();
    expect("waitforone_takes_2", recvmmsg(d[1], mv, 5, MSG_WAITFORONE, 0), 2);
    check("waitforone_data", bufs[0][0] == 'a' && mv[1].len == 2, mv[1].len);

    write(d[0], "1", 1);
    write(d[0], "22", 2);
    write(d[0], "333", 3);
    setup();
    expect("blocking_vlen_2", recvmmsg(d[1], mv, 2, 0, 0), 2);
    expect("blocking_len_1", mv[1].len, 2);
    expect("one_left", recvmmsg(d[1], mv, 5, MSG_DONTWAIT, 0), 1);
    expect("left_len", mv[0].len, 3);

    expect("vlen_0", recvmmsg(d[1], mv, 0, 0, 0), 0);
    expect("vlen_0_dontwait", recvmmsg(d[1], mv, 0, MSG_DONTWAIT, 0), 0);
    expect("vlen_2p32_is_0", recvmmsg(d[1], mv, 1UL << 32, 0, 0), 0);
    write(d[0], "x", 1);
    expect("vlen_high_bits", recvmmsg(d[1], mv, (1UL << 32) + 1, 0, 0), 1);
    write(d[0], "y", 1);
    expect("flags_high_bits", recvmmsg(d[1], mv, 1, MSG_DONTWAIT | (1UL << 40), 0), 1);
    write(d[0], "z", 1);
    expect("fd_high_bits", syscall5(SYS_recvmmsg, d[1] | (1L << 32), mv, 1, MSG_DONTWAIT, 0), 1);

    close(d[0]);
    close(d[1]);
    return finish();
}
