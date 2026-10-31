/* Which descriptors splice(2) accepts: one side must be a pipe. The other
 * may be a regular file, /dev/zero (in), /dev/null (out) or a connected
 * AF_UNIX stream socket (both directions). A directory, an eventfd or a
 * timerfd on the non-pipe side, a socket to/from a regular file, and
 * /dev/null as input are EINVAL; an O_PATH descriptor is EBADF. An
 * offset given for a socket side is EINVAL. A socket whose peer closed
 * gives 0.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_splice 275
/* asm-generic/fcntl.h */
#define O_PATH 010000000

#define F "/tmp/c_splice_kinds"

static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static i64 open_(const char *path, int flags) { return syscall4(SYS_openat, AT_FDCWD, path, flags, 0644); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

int test_main(void) {
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    char buf[64];
    i64 f = open_(F, O_RDWR | O_CREAT | O_TRUNC);
    check("open", f >= 0, f);
    expect("fill", write(f, "file", 4), 4);

    i64 zero = open_("/dev/zero", O_RDONLY);
    i64 null = open_("/dev/null", O_RDWR);
    check("open_devs", zero >= 0 && null >= 0, zero);
    expect("zero_to_pipe", splice(zero, 0, p[1], 0, 10, 0), 10);
    expect("zero_read", read(p[0], buf, 64), 10);
    check("zero_bytes", buf[0] == 0 && buf[9] == 0, buf[0]);
    expect("put_1", write(p[1], "discard", 7), 7);
    expect("pipe_to_null", splice(p[0], 0, null, 0, 100, 0), 7);
    expect("null_drained", avail(p[0]), 0);
    expect("null_in_einval", splice(null, 0, p[1], 0, 10, 0), -EINVAL);

    int s[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, s), 0);
    expect("sock_put", write(s[1], "sock", 4), 4);
    expect("socket_to_pipe", splice(s[0], 0, p[1], 0, 10, 0), 4);
    expect("socket_to_pipe_read", read(p[0], buf, 64), 4);
    check("socket_to_pipe_data", mem_eq(buf, "sock", 4), 0);
    expect("put_2", write(p[1], "tosock", 6), 6);
    expect("pipe_to_socket", splice(p[0], 0, s[0], 0, 10, 0), 6);
    expect("pipe_to_socket_read", read(s[1], buf, 64), 6);
    check("pipe_to_socket_data", mem_eq(buf, "tosock", 6), 0);
    i64 off = 0;
    expect("sock_put_2", write(s[1], "abc", 3), 3);
    expect("socket_off_in_einval", splice(s[0], &off, p[1], 0, 3, 0), -EINVAL);
    expect("socket_to_file_einval", splice(s[0], 0, f, 0, 3, 0), -EINVAL);
    expect("file_to_socket_einval", splice(f, &off, s[0], 0, 3, 0), -EINVAL);
    expect("put_3", write(p[1], "xyz", 3), 3);
    expect("socket_off_out_einval", splice(p[0], 0, s[0], &off, 3, 0), -EINVAL);
    expect("pipe_kept", avail(p[0]), 3);
    expect("drain", read(p[0], buf, 64), 3);

    i64 dir = open_("/tmp", O_RDONLY | O_DIRECTORY);
    check("open_dir", dir >= 0, dir);
    expect("dir_to_pipe_einval", splice(dir, 0, p[1], 0, 3, 0), -EINVAL);
    i64 efd = syscall2(SYS_eventfd2, 5, 0);
    check("eventfd", efd >= 0, efd);
    expect("eventfd_to_pipe_einval", splice(efd, 0, p[1], 0, 8, 0), -EINVAL);
    expect("put_4", write(p[1], "12345678", 8), 8);
    expect("pipe_to_eventfd_einval", splice(p[0], 0, efd, 0, 8, 0), -EINVAL);
    i64 tfd = syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, 0);
    check("timerfd", tfd >= 0, tfd);
    expect("pipe_to_timerfd_einval", splice(p[0], 0, tfd, 0, 8, 0), -EINVAL);
    i64 opath = open_(F, O_PATH);
    check("open_path", opath >= 0, opath);
    expect("opath_ebadf", splice(opath, 0, p[1], 0, 3, 0), -EBADF);
    expect("pipe_to_opath_ebadf", splice(p[0], 0, opath, 0, 3, 0), -EBADF);
    expect("pipe_still_8", avail(p[0]), 8);

    /* The socket's queued "abc" still arrives; then the peer closes. */
    expect("socket_rest", splice(s[0], 0, p[1], 0, 100, 0), 3);
    close(s[1]);
    expect("socket_peer_closed_0", splice(s[0], 0, p[1], 0, 100, 0), 0);

    close(s[0]);
    close(opath);
    close(tfd);
    close(efd);
    close(dir);
    close(zero);
    close(null);
    close(f);
    close(p[0]);
    close(p[1]);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
