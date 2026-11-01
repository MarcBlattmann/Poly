/* An error after recvmmsg(2) received at least one message: the call
 * returns the count, and the error (other than EAGAIN) is kept on the
 * socket and returned by the next receive call on it, once. Seen with an
 * unwritable buffer in the second entry: 1, then EFAULT, then the third
 * message (the failing second datagram is gone). An error on the first
 * message is returned directly (EFAULT; the AF_UNIX datagram is consumed)
 * and not kept. A msgvec whose msg_len cannot be stored (read-only):
 * EFAULT, message consumed, nothing kept.
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

static struct k_mmsghdr mv[3];
static struct k_iovec iv[3];
static char bufs[3][16];
static void setup(void) {
    for (int i = 0; i < 3; i++) {
        memset(&mv[i], 0, sizeof mv[i]);
        memset(bufs[i], 0, 16);
        iv[i].base = bufs[i];
        iv[i].len = 16;
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
    }
}

int test_main(void) {
    int d[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, d), 0);

    write(d[0], "one", 3);
    write(d[0], "two22", 5);
    write(d[0], "333", 3);
    setup();
    iv[1].base = (void *)8;
    expect("second_bad_returns_1", recvmmsg(d[1], mv, 3, MSG_DONTWAIT, 0), 1);
    expect("first_len", mv[0].len, 3);
    setup();
    expect("kept_error_next_call", recvmmsg(d[1], mv, 3, MSG_DONTWAIT, 0), -EFAULT);
    expect("then_third", recvmmsg(d[1], mv, 3, MSG_DONTWAIT, 0), 1);
    check("third_data", mv[0].len == 3 && mem_eq(bufs[0], "333", 3), mv[0].len);

    /* The kept error is also what a plain recvfrom sees. */
    write(d[0], "one", 3);
    write(d[0], "two22", 5);
    setup();
    iv[1].base = (void *)8;
    expect("again_returns_1", recvmmsg(d[1], mv, 3, MSG_DONTWAIT, 0), 1);
    char buf[8];
    expect("recvfrom_gets_kept_error", syscall6(SYS_recvfrom, d[1], (i64)buf, 8, MSG_DONTWAIT, 0, 0), -EFAULT);
    expect("recvfrom_then_empty", syscall6(SYS_recvfrom, d[1], (i64)buf, 8, MSG_DONTWAIT, 0, 0), -EAGAIN);

    write(d[0], "one", 3);
    setup();
    iv[0].base = (void *)8;
    expect("first_bad_efault", recvmmsg(d[1], mv, 3, MSG_DONTWAIT, 0), -EFAULT);
    setup();
    expect("first_bad_consumed_not_kept", recvmmsg(d[1], mv, 3, MSG_DONTWAIT, 0), -EAGAIN);

    i64 page = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", page > 0, page);
    struct k_mmsghdr *rv = (struct k_mmsghdr *)page;
    setup();
    rv[0] = mv[0];
    expect("mprotect", syscall3(SYS_mprotect, page, 4096, PROT_READ), 0);
    write(d[0], "one", 3);
    expect("readonly_vec_efault", recvmmsg(d[1], rv, 2, MSG_DONTWAIT, 0), -EFAULT);
    expect("readonly_vec_consumed", recvmmsg(d[1], mv, 2, MSG_DONTWAIT, 0), -EAGAIN);

    close(d[0]);
    close(d[1]);
    return finish();
}
