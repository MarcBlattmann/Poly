/* Writing to a pipe without readers, or a stream socket whose peer is
 * closed, is EPIPE and raises SIGPIPE; with SIGPIPE ignored the call just
 * fails. Nothing is consumed: *offset and in_fd's offset are unchanged.
 * With SIGPIPE at its default the process is killed by it.
 * poly-diff: syscalls=off (a child process)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40
#define SYS_socketpair 53
/* linux/socket.h, linux/net.h */
#define AF_UNIX 1
#define SOCK_STREAM 1
/* asm/signal.h */
#define SIGPIPE 13

#define F "/tmp/c_sendfile_epipe_in"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

static i64 set_sigpipe(u64 handler) {
    struct a_sigaction sa = { handler, 0, 0, 0 };
    return syscall4(SYS_rt_sigaction, SIGPIPE, &sa, 0, 8);
}

int test_main(void) {
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    a_close(p[0]);

    i64 pid = syscall0(SYS_fork);
    if (pid == 0) {
        sendfile(p[1], in, 0, 5);
        syscall1(SYS_exit, 0);
    }
    int status = 0;
    expect("wait_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_killed_by_sigpipe", status & 0x7f, SIGPIPE);

    expect("ignore_sigpipe", set_sigpipe(SIG_IGN), 0);
    i64 off = 2;
    expect("pipe_no_reader", sendfile(p[1], in, &off, 5), -EPIPE);
    expect("pipe_offset_kept", off, 2);
    expect("pipe_no_reader_null", sendfile(p[1], in, 0, 5), -EPIPE);
    expect("in_offset_kept", syscall3(SYS_lseek, in, 0, SEEK_CUR), 0);
    a_close(p[1]);

    int s[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, s), 0);
    a_close(s[1]);
    expect("socket_peer_closed", sendfile(s[0], in, &off, 5), -EPIPE);
    expect("socket_offset_kept", off, 2);
    a_close(s[0]);
    a_close(in);
    expect("cleanup_in", a_unlink(F), 0);
    return finish();
}
