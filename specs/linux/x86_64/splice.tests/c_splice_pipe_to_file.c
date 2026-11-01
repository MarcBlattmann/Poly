/* splice(2) from a pipe into a regular file. With a NULL off_out the
 * bytes are written at the file offset, which advances; with an off_out
 * pointer they are written at *off_out, which advances, and the file
 * offset stays. Writing past the end leaves a hole of zero bytes. Only
 * what the pipe holds is moved (no waiting for more once some bytes
 * moved); an empty pipe without writers gives 0. An O_APPEND file is
 * EINVAL, with or without an offset, and nothing leaves the pipe.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_splice 275
#define SYS_pread64 17
/* asm-generic/fcntl.h, linux/fs.h */
#define O_APPEND 02000
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#define F "/tmp/c_splice_pipe_to_file"

static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static i64 fpos(int fd) { return syscall3(SYS_lseek, fd, 0, SEEK_CUR); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

int test_main(void) {
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    expect("fill", write(f, "0123456789", 10), 10);
    expect("seek_2", syscall3(SYS_lseek, f, 2, SEEK_SET), 2);
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    char buf[64];

    expect("put_1", write(p[1], "ABCDEFGH", 8), 8);
    expect("null_off_3", splice(p[0], 0, f, 0, 3, 0), 3);
    expect("null_off_fpos", fpos(f), 5);
    expect("pipe_left_5", avail(p[0]), 5);

    i64 off = 8;
    expect("ptr_off_100_moves_what_is_there", splice(p[0], 0, f, &off, 100, 0), 5);
    expect("ptr_off_updated", off, 13);
    expect("ptr_off_fpos_kept", fpos(f), 5);
    expect("size_13", syscall3(SYS_lseek, f, 0, SEEK_END), 13);
    expect("seek_back_5", syscall3(SYS_lseek, f, 5, SEEK_SET), 5);
    expect("pread_all", syscall4(SYS_pread64, f, buf, 64, 0), 13);
    check("content_1", mem_eq(buf, "01ABC567DEFGH", 13), 0);

    expect("put_2", write(p[1], "xy", 2), 2);
    off = 16;
    expect("past_eof", splice(p[0], 0, f, &off, 2, 0), 2);
    expect("past_eof_off", off, 18);
    expect("pread_hole", syscall4(SYS_pread64, f, buf, 64, 0), 18);
    check("hole_zero", buf[13] == 0 && buf[14] == 0 && buf[15] == 0, 0);
    check("content_2", mem_eq(buf + 16, "xy", 2), 0);

    off = -1;
    expect("put_3", write(p[1], "z", 1), 1);
    expect("negative_off_einval", splice(p[0], 0, f, &off, 1, 0), -EINVAL);
    expect("negative_off_pipe_kept", avail(p[0]), 1);

    i64 fa = syscall4(SYS_openat, AT_FDCWD, F, O_WRONLY | O_APPEND, 0);
    check("open_append", fa >= 0, fa);
    expect("append_einval", splice(p[0], 0, fa, 0, 1, 0), -EINVAL);
    off = 0;
    expect("append_off_einval", splice(p[0], 0, fa, &off, 1, 0), -EINVAL);
    expect("append_pipe_kept", avail(p[0]), 1);
    close(fa);

    expect("drain", read(p[0], buf, 64), 1);
    close(p[1]);
    expect("no_writers_0", splice(p[0], 0, f, 0, 10, 0), 0);
    expect("no_writers_fpos", fpos(f), 5);
    close(p[0]);
    close(f);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
