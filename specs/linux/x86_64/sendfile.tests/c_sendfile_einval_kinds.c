/* What in_fd and out_fd may be. out_fd with O_APPEND is EINVAL (even for
 * count 0) and nothing is written. in_fd that is a directory, a pipe, a
 * socket or an eventfd is EINVAL with a NULL offset; with an offset
 * pointer a pipe or socket in_fd is ESPIPE. A pipe to a pipe is EINVAL.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40
#define SYS_socketpair 53
#define SYS_eventfd2 290
/* linux/socket.h, linux/net.h */
#define AF_UNIX 1
#define SOCK_STREAM 1

#define F "/tmp/c_sendfile_einval_kinds_in"
#define G "/tmp/c_sendfile_einval_kinds_out"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

int test_main(void) {
    i64 off;
    struct a_stat st;
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    expect("setup_out", a_make_file(G, "xy", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    i64 ap = a_open(G, O_WRONLY | O_APPEND, 0);
    off = 0;
    expect("out_append", sendfile(ap, in, &off, 3), -EINVAL);
    expect("out_append_count_0", sendfile(ap, in, &off, 0), -EINVAL);
    expect("out_append_null", sendfile(ap, in, 0, 3), -EINVAL);
    expect("out_append_offset_kept", off, 0);
    expect("out_append_stat", a_stat_path(G, &st), 0);
    expect("out_append_nothing_written", st.st_size, 2);
    a_close(ap);
    i64 rwa = a_open(G, O_RDWR | O_APPEND, 0);
    expect("out_rdwr_append", sendfile(rwa, in, 0, 3), -EINVAL);
    a_close(rwa);

    i64 out = a_open(G, O_WRONLY | O_TRUNC, 0);
    i64 d = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("in_directory", sendfile(out, d, 0, 1), -EINVAL);
    off = 0;
    expect("in_directory_offset", sendfile(out, d, &off, 1), -EINVAL);

    int p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("in_empty_pipe", sendfile(out, p[0], 0, 1), -EINVAL);
    expect("fill_pipe", a_write(p[1], "pqr", 3), 3);
    expect("in_pipe", sendfile(out, p[0], 0, 3), -EINVAL);
    off = 0;
    expect("in_pipe_offset", sendfile(out, p[0], &off, 3), -ESPIPE);
    int p2[2];
    expect("pipe2", syscall2(SYS_pipe2, p2, 0), 0);
    expect("pipe_to_pipe", sendfile(p2[1], p[0], 0, 3), -EINVAL);
    expect("pipe_to_pipe_offset", sendfile(p2[1], p[0], &off, 3), -ESPIPE);
    char buf[8];
    expect("pipe_data_still_there", a_read(p[0], buf, 8), 3);
    check("pipe_data_contents", mem_eq(buf, "pqr", 3), buf[0]);

    int s[2];
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, s), 0);
    expect("sock_write", a_write(s[0], "st", 2), 2);
    expect("in_socket", sendfile(out, s[1], 0, 2), -EINVAL);
    expect("in_socket_offset", sendfile(out, s[1], &off, 2), -ESPIPE);
    i64 ev = syscall2(SYS_eventfd2, 5, 0);
    check("eventfd", ev >= 0, ev);
    expect("in_eventfd", sendfile(out, ev, 0, 8), -EINVAL);
    expect("out_stat", a_stat_path(G, &st), 0);
    expect("out_empty", st.st_size, 0);
    a_close(ev);
    a_close(s[0]);
    a_close(s[1]);
    a_close(p[0]);
    a_close(p[1]);
    a_close(p2[0]);
    a_close(p2[1]);
    a_close(d);
    a_close(out);
    a_close(in);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
