/* vmsplice(2) into a pipe puts each segment (each page piece of it) into
 * a buffer of its own: segments are not merged with each other or with
 * bytes already in the pipe. A fresh pipe has 16 buffers, so 20 one-byte
 * segments move 16 bytes; after one write(2) 15. A segment crossing a
 * page boundary takes two buffers. One segment of 200000 bytes into a
 * fresh pipe moves 65536 (16 pages). The call stops at the first segment
 * that cannot be read: if bytes moved before it, their count is
 * returned; if not, EFAULT.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_vmsplice 278
/* linux/splice.h */
#define SPLICE_F_NONBLOCK 2

static i64 vmsplice(i64 fd, const struct k_iovec *iov, u64 n, u64 flags) {
    return syscall4(SYS_vmsplice, fd, iov, n, flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}
static void drain(int fd) {
    static char sink[65536];
    syscall3(SYS_fcntl, fd, F_SETFL, O_NONBLOCK);
    while (read(fd, sink, sizeof sink) > 0) {
    }
    syscall3(SYS_fcntl, fd, F_SETFL, 0);
}

int test_main(void) {
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    i64 m = syscall6(SYS_mmap, 0, 4 * 65536, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    char *pg = (char *)m;
    for (int i = 0; i < 4 * 65536; i++) pg[i] = (char)('a' + i % 26);
    struct k_iovec segs[20];
    for (int i = 0; i < 20; i++) {
        segs[i].base = pg + i;
        segs[i].len = 1;
    }

    expect("20_segments_16_buffers", vmsplice(p[1], segs, 20, 0), 16);
    expect("avail_16", avail(p[0]), 16);
    char buf[32];
    expect("read_16", read(p[0], buf, 32), 16);
    check("data_16", mem_eq(buf, pg, 16), 0);

    expect("one_write", write(p[1], "x", 1), 1);
    expect("after_write_15", vmsplice(p[1], segs, 20, SPLICE_F_NONBLOCK), 15);
    expect("avail_16_again", avail(p[0]), 16);
    drain(p[0]);

    /* 15 segments fit, then a 200-byte segment across a page boundary
     * needs two buffers: only its first piece (up to the boundary) fits. */
    struct k_iovec cross[16];
    for (int i = 0; i < 15; i++) cross[i] = segs[i];
    cross[15].base = pg + 4096 - 50;
    cross[15].len = 200;
    expect("crossing_two_buffers", vmsplice(p[1], cross, 16, SPLICE_F_NONBLOCK), 15 + 50);
    drain(p[0]);
    expect("crossing_alone", vmsplice(p[1], &cross[15], 1, 0), 200);
    expect("read_cross", read(p[0], buf, 32), 32);
    check("data_cross", mem_eq(buf, pg + 4096 - 50, 32), 0);
    drain(p[0]);

    struct k_iovec big = { pg, 200000 };
    expect("big_16_pages", vmsplice(p[1], &big, 1, SPLICE_F_NONBLOCK), 65536);
    expect("full_eagain", vmsplice(p[1], &big, 1, SPLICE_F_NONBLOCK), -EAGAIN);
    drain(p[0]);
    struct k_iovec odd = { pg + 100, 5000 };
    expect("unaligned_5000", vmsplice(p[1], &odd, 1, 0), 5000);
    expect("avail_5000", avail(p[0]), 5000);
    drain(p[0]);

    struct k_iovec bad2[3] = { { pg, 6 }, { (void *)8, 5 }, { pg, 4 } };
    expect("bad_second_partial", vmsplice(p[1], bad2, 3, 0), 6);
    expect("bad_second_avail", avail(p[0]), 6);
    drain(p[0]);
    struct k_iovec bad1[2] = { { (void *)8, 5 }, { pg, 4 } };
    expect("bad_first_efault", vmsplice(p[1], bad1, 2, 0), -EFAULT);
    expect("bad_first_nothing", avail(p[0]), 0);

    close(p[0]);
    close(p[1]);
    return finish();
}
