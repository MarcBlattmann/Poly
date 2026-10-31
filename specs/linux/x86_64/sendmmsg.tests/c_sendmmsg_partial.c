/* sendmmsg(2) stops at the first entry that fails. If some messages were
 * sent before it, their count is returned (the failing entry's msg_len
 * is not touched, the error is not reported); if the first entry fails,
 * its error is returned and nothing was sent. Failures seen: an iovec
 * base that cannot be read (EFAULT), a datagram larger than the socket
 * allows (EMSGSIZE), the msgvec array running into an unmapped page
 * (the entries before it are sent). A msgvec in read-only memory: the
 * message is sent but its msg_len cannot be stored, so the call fails
 * with EFAULT and does not count it.
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

static struct k_mmsghdr mv[3];
static struct k_iovec iv[3];
static char text[3][4] = { "aaa", "bbb", "ccc" };
static char huge[300000];
static int d[2];

static void setup(void) {
    for (int i = 0; i < 3; i++) {
        iv[i].base = text[i];
        iv[i].len = 3;
        memset(&mv[i], 0, sizeof mv[i]);
        mv[i].hdr.iov = &iv[i];
        mv[i].hdr.iovlen = 1;
        mv[i].len = 0xdead;
    }
}
static int queued(void) {
    char buf[8];
    int n = 0;
    while (syscall6(SYS_recvfrom, d[1], (i64)buf, 8, MSG_DONTWAIT, 0, 0) >= 0) n++;
    return n;
}

int test_main(void) {
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_DGRAM, 0, d), 0);

    setup();
    iv[1].base = (void *)8;
    expect("second_iov_bad", sendmmsg(d[0], mv, 3, 0), 1);
    expect("second_len_untouched", mv[1].len, 0xdead);
    expect("second_iov_bad_queued", queued(), 1);
    setup();
    iv[0].base = (void *)8;
    expect("first_iov_bad_efault", sendmmsg(d[0], mv, 3, 0), -EFAULT);
    expect("first_len_untouched", mv[0].len, 0xdead);
    expect("first_iov_bad_queued", queued(), 0);

    struct k_iovec hv = { huge, sizeof huge };
    setup();
    mv[1].hdr.iov = &hv;
    expect("second_emsgsize", sendmmsg(d[0], mv, 3, 0), 1);
    expect("second_emsgsize_queued", queued(), 1);
    setup();
    mv[0].hdr.iov = &hv;
    expect("first_emsgsize", sendmmsg(d[0], mv, 3, 0), -EMSGSIZE);
    expect("first_emsgsize_queued", queued(), 0);

    /* The array's last entry before an unmapped page. */
    i64 m = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    expect("munmap_second", syscall2(SYS_munmap, m + 4096, 4096), 0);
    setup();
    struct k_mmsghdr *edge = (struct k_mmsghdr *)(m + 4096 - sizeof(struct k_mmsghdr));
    *edge = mv[0];
    expect("edge_sends_one", sendmmsg(d[0], edge, 3, 0), 1);
    expect("edge_len", edge->len, 3);
    expect("edge_queued", queued(), 1);

    /* Read-only msgvec. */
    i64 ro = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ro", ro > 0, ro);
    struct k_mmsghdr *rv = (struct k_mmsghdr *)ro;
    rv[0] = mv[0];
    rv[1] = mv[1];
    expect("mprotect", syscall3(SYS_mprotect, ro, 4096, PROT_READ), 0);
    expect("readonly_vec_efault", sendmmsg(d[0], rv, 2, 0), -EFAULT);
    expect("readonly_vec_first_sent", queued(), 1);

    close(d[0]);
    close(d[1]);
    return finish();
}
