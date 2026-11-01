/* vmsplice(2) on a pipe's read end copies bytes out of the pipe into the
 * iovec's buffers, like readv(2): in order, filling each segment before
 * the next, returning how many; only what the pipe holds is taken. An
 * empty pipe without writers gives 0; an empty pipe with a writer and
 * SPLICE_F_NONBLOCK is EAGAIN. A buffer that cannot be written (bad
 * address, read-only memory) is EFAULT and the bytes stay in the pipe.
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

int test_main(void) {
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    char a[4], b[20];
    struct k_iovec v[2] = { { a, 4 }, { b, 20 } };

    expect("put", write(p[1], "0123456789", 10), 10);
    expect("scatter", vmsplice(p[0], v, 2, 0), 10);
    check("seg_1", mem_eq(a, "0123", 4), 0);
    check("seg_2", mem_eq(b, "456789", 6), 0);
    expect("empty_after", avail(p[0]), 0);

    expect("put_2", write(p[1], "abcdefgh", 8), 8);
    struct k_iovec small = { a, 3 };
    expect("partial", vmsplice(p[0], &small, 1, 0), 3);
    check("partial_data", mem_eq(a, "abc", 3), 0);
    expect("rest_kept", avail(p[0]), 5);
    expect("rest", vmsplice(p[0], v, 2, 0), 5);
    check("rest_data", mem_eq(a, "defg", 4) && b[0] == 'h', 0);

    expect("empty_flag_eagain", vmsplice(p[0], v, 2, SPLICE_F_NONBLOCK), -EAGAIN);

    expect("put_3", write(p[1], "xyz", 3), 3);
    struct k_iovec bad = { (void *)8, 3 };
    expect("bad_buffer_efault", vmsplice(p[0], &bad, 1, 0), -EFAULT);
    expect("bad_buffer_kept", avail(p[0]), 3);
    i64 page = syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", page > 0, page);
    struct k_iovec ro = { (void *)page, 3 };
    expect("readonly_buffer_efault", vmsplice(p[0], &ro, 1, 0), -EFAULT);
    expect("readonly_kept", avail(p[0]), 3);
    expect("then_ok", vmsplice(p[0], v, 1, 0), 3);
    check("then_data", mem_eq(a, "xyz", 3), 0);

    close(p[1]);
    expect("no_writers_0", vmsplice(p[0], v, 2, 0), 0);
    expect("no_writers_flag_0", vmsplice(p[0], v, 2, SPLICE_F_NONBLOCK), 0);
    close(p[0]);
    return finish();
}
