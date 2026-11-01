/* splice(2) into a pipe that has no reader is EPIPE and raises SIGPIPE
 * once per call (from a file, /dev/zero or another pipe); nothing is
 * taken from the input (the input pipe keeps its bytes, the file offset
 * stays). With SIGPIPE at its default the process is killed by it.
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_splice 275
/* linux/fs.h */
#define SEEK_CUR 1

#define F "/tmp/c_splice_epipe"

static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}
static volatile int sigs;
static void on_pipe(int s) {
    (void)s;
    sigs++;
}

int test_main(void) {
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    expect("fill", write(f, "0123456789", 10), 10);
    expect("rewind", syscall3(SYS_lseek, f, 0, 0), 0);
    int w[2], p[2];
    expect("pipe_w", syscall1(SYS_pipe, w), 0);
    expect("pipe_p", syscall1(SYS_pipe, p), 0);
    close(w[0]);

    i64 pid = fork();
    if (pid == 0) {
        splice(f, 0, w[1], 0, 5, 0);
        syscall1(SYS_exit, 0);
    }
    int st = wait_status(pid);
    check("default_killed_by_sigpipe", W_SIGNALED(st) && W_TERMSIG(st) == SIGPIPE, st);
    expect("child_left_fpos", syscall3(SYS_lseek, f, 0, SEEK_CUR), 0);

    expect("handler", sig_handler(SIGPIPE, on_pipe, 0), 0);
    sigs = 0;
    expect("file_epipe", splice(f, 0, w[1], 0, 5, 0), -EPIPE);
    expect("file_one_signal", sigs, 1);
    expect("file_fpos_kept", syscall3(SYS_lseek, f, 0, SEEK_CUR), 0);
    i64 off = 3;
    sigs = 0;
    expect("file_off_epipe", splice(f, &off, w[1], 0, 5, 0), -EPIPE);
    expect("file_off_kept", off, 3);
    expect("file_off_one_signal", sigs, 1);

    i64 zero = syscall4(SYS_openat, AT_FDCWD, "/dev/zero", O_RDONLY, 0);
    sigs = 0;
    expect("zero_epipe", splice(zero, 0, w[1], 0, 5, 0), -EPIPE);
    expect("zero_one_signal", sigs, 1);

    expect("put", write(p[1], "abc", 3), 3);
    sigs = 0;
    expect("pipe_epipe", splice(p[0], 0, w[1], 0, 3, 0), -EPIPE);
    expect("pipe_one_signal", sigs, 1);
    expect("pipe_input_kept", avail(p[0]), 3);

    expect("ignore", sig_action(SIGPIPE, SIG_IGN, 0, 0), 0);
    sigs = 0;
    expect("ignored_epipe", splice(f, 0, w[1], 0, 5, 0), -EPIPE);
    expect("ignored_no_handler_call", sigs, 0);

    close(zero);
    close(w[1]);
    close(p[0]);
    close(p[1]);
    close(f);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
