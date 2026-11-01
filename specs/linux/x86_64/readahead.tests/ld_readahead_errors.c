/* readahead errors: EBADF (9) when the descriptor is not open (closed,
 * -1), was opened with O_PATH, or is not open for reading (O_WRONLY, a
 * pipe's write end); EINVAL (22) when it does not refer to a file whose
 * pages can be read ahead: a directory, a pipe's read end, a FIFO, a
 * socket, /dev/null, /dev/zero, or when count >= 2^63. Order: EBADF
 * before EINVAL.
 */
#include "a_sys.h"

#define SYS_readahead 187
#define SYS_socket 41
#define SYS_mknodat 259

static i64 ra(i64 fd, i64 off, u64 count) { return syscall3(SYS_readahead, fd, off, count); }

int test_main(void) {
    const char *file = "/tmp/ld_readahead_errors";
    const char *fifo = "/tmp/ld_readahead_errors_fifo";
    i64 fd = a_open(file, O_RDWR | O_CREAT | O_TRUNC, 0644);
    a_write(fd, "readahead", 9);

    expect("closed_fd", ra(999, 0, 10), -EBADF);
    expect("minus1_fd", ra(-1, 0, 10), -EBADF);
    i64 wo = a_open(file, O_WRONLY, 0);
    expect("wronly", ra(wo, 0, 10), -EBADF);
    i64 pa = a_open(file, O_PATH, 0);
    expect("o_path", ra(pa, 0, 10), -EBADF);

    int p[2];
    syscall2(SYS_pipe2, p, 0);
    expect("pipe_write_end", ra(p[1], 0, 10), -EBADF);
    expect("pipe_read_end", ra(p[0], 0, 10), -EINVAL);
    syscall4(SYS_mknodat, AT_FDCWD, fifo, S_IFIFO | 0600, 0);
    i64 ff = a_open(fifo, O_RDWR, 0);
    expect("fifo", ra(ff, 0, 10), -EINVAL);
    i64 dir = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("directory", ra(dir, 0, 10), -EINVAL);
    i64 sock = syscall3(SYS_socket, 1, 1, 0);
    expect("socket", ra(sock, 0, 10), -EINVAL);
    i64 dn = a_open("/dev/null", O_RDONLY, 0);
    expect("dev_null", ra(dn, 0, 10), -EINVAL);
    i64 dz = a_open("/dev/zero", O_RDONLY, 0);
    expect("dev_zero", ra(dz, 0, 10), -EINVAL);
    expect("huge_count", ra(fd, 0, ~0UL), -EINVAL);

    /* Order. */
    expect("wronly_and_huge_count", ra(wo, 0, ~0UL), -EBADF);
    expect("closed_and_huge_count", ra(999, 0, ~0UL), -EBADF);
    expect("pipe_and_huge_count", ra(p[0], 0, ~0UL), -EINVAL);
    expect("file_still_ok", ra(fd, 0, 10), 0);

    a_close(dz);
    a_close(dn);
    a_close(sock);
    a_close(dir);
    a_close(ff);
    a_close(p[0]);
    a_close(p[1]);
    a_close(pa);
    a_close(wo);
    a_close(fd);
    expect("unlink_fifo", a_unlink(fifo), 0);
    expect("unlink_file", a_unlink(file), 0);
    return finish();
}
