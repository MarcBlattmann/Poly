/* tee(2) without waiting: an empty input pipe that still has a writer
 * and a full output pipe would wait; with SPLICE_F_NONBLOCK, or O_NONBLOCK
 * on either pipe, they are EAGAIN. An empty input without writers gives
 * 0 at once, but only when the output has room (a full output is still
 * waited for, EAGAIN without waiting). Into a fresh output pipe at most
 * its room (65536) is copied from a big input, and the input keeps all
 * of it.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_tee 276
#define SYS_splice 275
/* linux/splice.h */
#define SPLICE_F_NONBLOCK 2

static i64 tee(i64 in, i64 out, u64 len, u64 flags) { return syscall4(SYS_tee, in, out, len, flags); }
static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}
static i64 setfl(int fd, int fl) { return syscall3(SYS_fcntl, fd, F_SETFL, fl); }

int test_main(void) {
    int p[2], q[2];
    expect("pipe_p", syscall1(SYS_pipe, p), 0);
    expect("pipe_q", syscall1(SYS_pipe, q), 0);
    i64 zero = syscall4(SYS_openat, AT_FDCWD, "/dev/zero", O_RDONLY, 0);
    i64 null = syscall4(SYS_openat, AT_FDCWD, "/dev/null", O_WRONLY, 0);
    check("devs", zero >= 0 && null >= 0, zero);

    expect("empty_flag_eagain", tee(p[0], q[1], 5, SPLICE_F_NONBLOCK), -EAGAIN);
    expect("nb_in", setfl(p[0], O_NONBLOCK), 0);
    expect("empty_o_nonblock_in_eagain", tee(p[0], q[1], 5, 0), -EAGAIN);
    expect("clear_in", setfl(p[0], 0), 0);
    expect("nb_out", setfl(q[1], O_NONBLOCK), 0);
    expect("empty_o_nonblock_out_eagain", tee(p[0], q[1], 5, 0), -EAGAIN);

    /* Fill q (O_NONBLOCK still set on its write end). */
    expect("fill_q", splice(zero, 0, q[1], 0, 1 << 20, 0), 65536);
    expect("put", write(p[1], "abc", 3), 3);
    expect("full_o_nonblock_out_eagain", tee(p[0], q[1], 3, 0), -EAGAIN);
    expect("clear_out", setfl(q[1], 0), 0);
    expect("full_flag_eagain", tee(p[0], q[1], 3, SPLICE_F_NONBLOCK), -EAGAIN);
    expect("input_kept", avail(p[0]), 3);
    expect("drain_q", splice(q[0], 0, null, 0, 1 << 20, 0), 65536);
    char buf[8];
    expect("drain_p", read(p[0], buf, 8), 3);

    /* A 1 MiB input into a fresh 64 KiB pipe. */
    expect("setpipe_1m", syscall3(SYS_fcntl, p[1], F_SETPIPE_SZ, 1 << 20), 1 << 20);
    expect("fill_p_1m", splice(zero, 0, p[1], 0, 1 << 20, SPLICE_F_NONBLOCK), 1 << 20);
    expect("tee_room_65536", tee(p[0], q[1], 1 << 20, SPLICE_F_NONBLOCK), 65536);
    expect("input_keeps_1m", avail(p[0]), 1 << 20);
    expect("q_has_65536", avail(q[0]), 65536);
    expect("drain_p_1m", splice(p[0], 0, null, 0, 1 << 20, 0), 1 << 20);

    int z[2];
    expect("pipe_z", syscall1(SYS_pipe, z), 0);
    close(z[1]);
    /* Room in the output is waited for even when the input is at its end. */
    expect("eof_input_full_output_eagain", tee(z[0], q[1], 5, SPLICE_F_NONBLOCK), -EAGAIN);
    expect("drain_q_2", splice(q[0], 0, null, 0, 1 << 20, 0), 65536);
    expect("empty_no_writers_0", tee(z[0], q[1], 5, 0), 0);
    expect("empty_no_writers_flag_0", tee(z[0], q[1], 5, SPLICE_F_NONBLOCK), 0);

    close(z[0]);
    close(zero);
    close(null);
    close(p[0]);
    close(p[1]);
    close(q[0]);
    close(q[1]);
    return finish();
}
