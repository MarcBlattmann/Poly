/* tee(2) copies bytes from one pipe into another without consuming them:
 * it returns how many it duplicated, at most len and at most what the
 * input holds (it can stop inside a buffer), and the input still has
 * everything afterwards. It starts at the input's current read position
 * (after a partial read only the rest is copied) and may span several
 * buffers. A len with bit 63 set is fine. Flags SPLICE_F_MOVE, MORE and
 * GIFT are accepted.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_tee 276

static i64 tee(i64 in, i64 out, u64 len, u64 flags) { return syscall4(SYS_tee, in, out, len, flags); }
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

int test_main(void) {
    int p[2], q[2];
    expect("pipe_p", syscall1(SYS_pipe, p), 0);
    expect("pipe_q", syscall1(SYS_pipe, q), 0);
    char buf[64];

    expect("put_1", write(p[1], "0123456789", 10), 10);
    expect("tee_4", tee(p[0], q[1], 4, 0), 4);
    expect("p_kept_10", avail(p[0]), 10);
    expect("q_4", avail(q[0]), 4);
    expect("tee_100", tee(p[0], q[1], 100, 0), 10);
    expect("p_kept_10_again", avail(p[0]), 10);
    expect("q_14", avail(q[0]), 14);
    expect("read_q", read(q[0], buf, 64), 14);
    check("q_data", mem_eq(buf, "01230123456789", 14), 0);

    expect("put_2", write(p[1], "abc", 3), 3);
    expect("read_p_3", read(p[0], buf, 3), 3);
    check("read_p_3_data", mem_eq(buf, "012", 3), 0);
    expect("tee_after_read", tee(p[0], q[1], 100, 0), 10);
    expect("read_q_2", read(q[0], buf, 64), 10);
    check("q_data_2", mem_eq(buf, "3456789abc", 10), 0);
    expect("read_p_rest", read(p[0], buf, 64), 10);
    check("p_data_rest", mem_eq(buf, "3456789abc", 10), 0);

    expect("put_3", write(p[1], "aaa", 3), 3);
    expect("put_4", write(p[1], "bbbb", 4), 4);
    expect("tee_across_buffers", tee(p[0], q[1], 5, 0), 5);
    expect("read_q_3", read(q[0], buf, 64), 5);
    check("q_data_3", mem_eq(buf, "aaabb", 5), 0);
    expect("tee_len_bit63", tee(p[0], q[1], 1UL << 63, 0xd), 7);
    expect("read_q_4", read(q[0], buf, 64), 7);
    check("q_data_4", mem_eq(buf, "aaabbbb", 7), 0);
    expect("read_p_4", read(p[0], buf, 64), 7);
    check("p_data_4", mem_eq(buf, "aaabbbb", 7), 0);

    /* Two tees of the same bytes into two pipes. */
    int r[2];
    expect("pipe_r", syscall1(SYS_pipe, r), 0);
    expect("put_5", write(p[1], "fanout", 6), 6);
    expect("tee_q", tee(p[0], q[1], 6, 0), 6);
    expect("tee_r", tee(p[0], r[1], 6, 0), 6);
    expect("read_q_5", read(q[0], buf, 64), 6);
    check("q_data_5", mem_eq(buf, "fanout", 6), 0);
    expect("read_r", read(r[0], buf, 64), 6);
    check("r_data", mem_eq(buf, "fanout", 6), 0);
    expect("read_p_5", read(p[0], buf, 64), 6);
    check("p_data_5", mem_eq(buf, "fanout", 6), 0);

    close(p[0]);
    close(p[1]);
    close(q[0]);
    close(q[1]);
    close(r[0]);
    close(r[1]);
    return finish();
}
