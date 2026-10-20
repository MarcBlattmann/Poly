/* writev: the iovec count limit (1024), negative lengths, bad iovec
 * arrays, bad buffers and bad descriptors. */
#include "a_sys.h"

#define F "/tmp/a_writev_errors"

static struct a_iovec many[1025];

int test_main(void) {
    struct a_stat st;
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    for (int i = 0; i < 1025; i++) many[i].iov_base = "z", many[i].iov_len = 1;
    expect("count_1024_ok", syscall3(SYS_writev, fd, many, 1024), 1024);
    expect("count_1025_einval", syscall3(SYS_writev, fd, many, 1025), -EINVAL);
    expect("count_negative_einval", syscall3(SYS_writev, fd, many, -3), -EINVAL);
    struct a_iovec neg[2] = {{"a", 1}, {"b", (size_t)-1}};
    expect("negative_length_einval", syscall3(SYS_writev, fd, neg, 2), -EINVAL);
    syscall2(SYS_fstat, fd, &st);
    expect("errors_wrote_nothing", st.st_size, 1024);
    expect("bad_iovec_array_efault", syscall3(SYS_writev, fd, (void *)16, 1), -EFAULT);
    struct a_iovec bad[2] = {{"q", 1}, {(void *)16, 4}};
    expect("bad_second_buffer_short", syscall3(SYS_writev, fd, bad, 2), 1);
    struct a_iovec bad0[1] = {{(void *)16, 4}};
    expect("bad_first_buffer_efault", syscall3(SYS_writev, fd, bad0, 1), -EFAULT);
    syscall2(SYS_fstat, fd, &st);
    expect("size_after_bad_buffers", st.st_size, 1025);
    a_close(fd);
    struct a_iovec one = {"x", 1};
    expect("closed_ebadf", syscall3(SYS_writev, fd, &one, 1), -EBADF);
    fd = a_open(F, O_RDONLY, 0);
    expect("rdonly_ebadf", syscall3(SYS_writev, fd, &one, 1), -EBADF);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
