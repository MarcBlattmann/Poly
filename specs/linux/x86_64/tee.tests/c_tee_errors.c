/* tee(2)'s checks in order: flags other than SPLICE_F_MOVE, NONBLOCK,
 * MORE, GIFT (0xf) are EINVAL first, even with len 0; then len 0 returns
 * 0 (even with bad descriptors); then fd_in must be open for reading and
 * fd_out open for writing (EBADF: closed descriptors, a pipe's write end
 * as fd_in, its read end as fd_out); then both must be pipes and
 * different ones (EINVAL: a regular file, a socket, the same pipe). Only
 * the low 32 bits of the descriptors and of flags count. Nothing is
 * copied by a failing call.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_tee 276
/* linux/splice.h */
#define SPLICE_F_NONBLOCK 2

#define F "/tmp/c_tee_errors"

static i64 tee(i64 in, i64 out, u64 len, u64 flags) { return syscall4(SYS_tee, in, out, len, flags); }
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

int test_main(void) {
    int p[2], q[2], s[2];
    expect("pipe_p", syscall1(SYS_pipe, p), 0);
    expect("pipe_q", syscall1(SYS_pipe, q), 0);
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, s), 0);
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    expect("fill", write(f, "file", 4), 4);
    expect("put", write(p[1], "abc", 3), 3);

    expect("flags_len0", tee(p[0], q[1], 0, 0x10), -EINVAL);
    expect("flags_bad_fds", tee(-1, -1, 5, 0x100), -EINVAL);
    expect("len0_bad_fds", tee(-1, -1, 0, 0), 0);
    expect("len0_file", tee(f, f, 0, 0), 0);
    expect("ebadf_in", tee(-1, q[1], 5, 0), -EBADF);
    expect("ebadf_out", tee(p[0], 99, 5, 0), -EBADF);
    expect("in_write_end_ebadf", tee(p[1], q[1], 5, 0), -EBADF);
    expect("out_read_end_ebadf", tee(p[0], q[0], 5, 0), -EBADF);
    expect("in_write_end_out_bad", tee(p[1], -1, 5, 0), -EBADF);
    expect("file_in_einval", tee(f, q[1], 5, 0), -EINVAL);
    expect("file_out_einval", tee(p[0], f, 5, 0), -EINVAL);
    expect("socket_in_einval", tee(s[0], q[1], 5, 0), -EINVAL);
    expect("socket_out_einval", tee(p[0], s[0], 5, 0), -EINVAL);
    expect("same_pipe_einval", tee(p[0], p[1], 5, 0), -EINVAL);
    expect("nothing_copied", avail(q[0]), 0);
    expect("input_kept", avail(p[0]), 3);

    expect("fd_high_bits", syscall4(SYS_tee, p[0] | (1L << 32), q[1] | (3L << 33), 5, SPLICE_F_NONBLOCK), 3);
    expect("flags_high_bits", syscall4(SYS_tee, p[0], q[1], 1, SPLICE_F_NONBLOCK | (1L << 40)), 1);
    expect("copied_4", avail(q[0]), 4);

    close(s[0]);
    close(s[1]);
    close(f);
    close(p[0]);
    close(p[1]);
    close(q[0]);
    close(q[1]);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
