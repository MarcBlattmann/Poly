/* The classic tee(2) + splice(2) pattern: tee duplicates the input pipe's
 * bytes into a second pipe, then splice consumes the input into a file.
 * Both destinations end up with the same bytes; the input is empty after
 * the splice; a tee of an emptied input (writer still open) with
 * SPLICE_F_NONBLOCK is EAGAIN.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_tee 276
#define SYS_splice 275
#define SYS_pread64 17
/* linux/splice.h */
#define SPLICE_F_NONBLOCK 2

#define F "/tmp/c_tee_then_splice"

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

int test_main(void) {
    int p[2], q[2];
    expect("pipe_p", syscall1(SYS_pipe, p), 0);
    expect("pipe_q", syscall1(SYS_pipe, q), 0);
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    char buf[128];
    static const char msg[] = "the quick brown fox jumps over the lazy dog";
    const i64 n = sizeof msg - 1;

    for (int round = 0; round < 3; round++) {
        expect("put", write(p[1], msg, n), n);
        expect("tee", tee(p[0], q[1], 1000, 0), n);
        expect("splice_to_file", splice(p[0], 0, f, 0, 1000, 0), n);
        expect("input_empty", avail(p[0]), 0);
        expect("tee_empty_eagain", tee(p[0], q[1], 1000, SPLICE_F_NONBLOCK), -EAGAIN);
    }
    expect("q_has_all", avail(q[0]), 3 * n);
    expect("file_size", syscall3(SYS_lseek, f, 0, 1), 3 * n);
    for (int round = 0; round < 3; round++) {
        expect("read_q", read(q[0], buf, n), n);
        check("q_data", mem_eq(buf, msg, n), 0);
        expect("pread_file", syscall4(SYS_pread64, f, buf, n, round * n), n);
        check("file_data", mem_eq(buf, msg, n), 0);
    }

    close(p[0]);
    close(p[1]);
    close(q[0]);
    close(q[1]);
    close(f);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
