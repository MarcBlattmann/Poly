/* recvmmsg(2) on an AF_UNIX stream socket: each entry is one receive of
 * up to its buffer's size from the byte stream, so 10 queued bytes into
 * entries of 4 bytes give 4, 4, 2 (3 entries). With MSG_DONTWAIT the call
 * stops when the stream is empty. After the peer closed, an end-of-file
 * receive (0 bytes) counts as a received entry: every remaining entry
 * gets msg_len 0. There is no 1024 limit on vlen here: 1500 one-byte
 * entries receive 1500 bytes in one call.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_recvmmsg 299

struct k_mmsghdr {
    struct k_msghdr hdr;
    unsigned len;
    unsigned pad;
};

static i64 recvmmsg(i64 fd, struct k_mmsghdr *v, u64 n, u64 flags, void *ts) {
    return syscall5(SYS_recvmmsg, fd, v, n, flags, ts);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }

static struct k_mmsghdr mv[1500];
static struct k_iovec iv[1500];
static char bytes[1500];
static void setup(int n, int len) {
    for (int i = 0; i < n; i++) {
        memset(&mv[i], 0, sizeof mv[i]);
        iv[i].base = bytes + i * len;
        iv[i].len = len;
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
        mv[i].len = 0xdead;
    }
}

int test_main(void) {
    int t[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, t), 0);

    write(t[0], "0123456789", 10);
    setup(3, 4);
    expect("three_entries", recvmmsg(t[1], mv, 3, 0, 0), 3);
    check("lens_4_4_2", mv[0].len == 4 && mv[1].len == 4 && mv[2].len == 2, mv[2].len);
    check("bytes", mem_eq(bytes, "0123456789", 10), 0);

    write(t[0], "0123456789", 10);
    setup(4, 4);
    expect("dontwait_stops_3", recvmmsg(t[1], mv, 4, MSG_DONTWAIT, 0), 3);
    expect("fourth_untouched", mv[3].len, 0xdead);

    static char many[1500];
    for (int i = 0; i < 1500; i++) many[i] = (char)('a' + i % 26);
    expect("write_1500", write(t[0], many, 1500), 1500);
    setup(1500, 1);
    expect("no_1024_cap", recvmmsg(t[1], mv, 1500, MSG_DONTWAIT, 0), 1500);
    check("all_bytes", mem_eq(bytes, many, 1500) && mv[1499].len == 1, mv[1499].len);

    write(t[0], "0123", 4);
    close(t[0]);
    setup(4, 4);
    expect("data_then_eof_entries", recvmmsg(t[1], mv, 4, 0, 0), 4);
    check("eof_lens", mv[0].len == 4 && mv[1].len == 0 && mv[2].len == 0 && mv[3].len == 0, mv[1].len);
    setup(4, 4);
    expect("eof_only", recvmmsg(t[1], mv, 4, 0, 0), 4);
    check("eof_only_lens", mv[0].len == 0 && mv[3].len == 0, mv[0].len);

    close(t[1]);
    return finish();
}
