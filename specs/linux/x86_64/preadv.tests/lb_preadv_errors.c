/* preadv(2) errors: EINVAL for a negative offset (-1 included), a count
 * above 1024 or negative, an iov_len that is negative as ssize_t; ESPIPE
 * for pipes and sockets; EISDIR for directories; EBADF for descriptors not
 * open for reading or O_PATH; EFAULT for an unreadable iovec array or a
 * bad first buffer; a bad later buffer gives a short read. */
#include "a_sys.h"

#define D "/tmp/lb_preadv_errors"
#define SYS_socketpair 53

static i64 preadv_(i64 fd, void *iov, i64 cnt, i64 lo) { return syscall5(SYS_preadv, fd, iov, cnt, lo, 0); }

int test_main(void) {
    char a[8];
    struct a_iovec v[1] = {{a, 4}};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "0123456789", 0644), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    i64 wo = a_open(D "/f", O_WRONLY, 0);
    i64 op = a_open(D "/f", O_PATH, 0);
    i64 dir = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i32 pp[2], sp[2];
    syscall2(SYS_pipe2, pp, 0);
    syscall4(SYS_socketpair, 1, 1, 0, sp);

    expect("offset_minus1", preadv_(fd, v, 1, -1), -EINVAL);
    expect("offset_negative", preadv_(fd, v, 1, -4096), -EINVAL);
    expect("count_1025", preadv_(fd, v, 1025, 0), -EINVAL);
    expect("count_negative", preadv_(fd, v, -1, 0), -EINVAL);
    struct a_iovec neg[1] = {{a, (size_t)-1}};
    expect("len_negative", preadv_(fd, neg, 1, 0), -EINVAL);
    struct a_iovec neg2[2] = {{a, 4}, {a, (size_t)0x8000000000000000UL}};
    expect("second_len_negative", preadv_(fd, neg2, 2, 0), -EINVAL);
    expect("pipe", preadv_(pp[0], v, 1, 0), -ESPIPE);
    expect("socket", preadv_(sp[0], v, 1, 0), -ESPIPE);
    expect("directory", preadv_(dir, v, 1, 0), -EISDIR);
    expect("wronly", preadv_(wo, v, 1, 0), -EBADF);
    expect("opath", preadv_(op, v, 1, 0), -EBADF);
    expect("bad_fd", preadv_(-1, v, 1, 0), -EBADF);
    expect("bad_iov_array", preadv_(fd, (void *)1, 1, 0), -EFAULT);
    struct a_iovec badbuf[1] = {{(void *)1, 4}};
    expect("bad_first_buffer", preadv_(fd, badbuf, 1, 0), -EFAULT);
    struct a_iovec partial[2] = {{a, 4}, {(void *)1, 4}};
    expect("bad_second_buffer_short", preadv_(fd, partial, 2, 0), 4);
    /* order: the offset first, then the descriptor, then the iovecs */
    expect("bad_fd_vs_negative", preadv_(-1, v, 1, -1), -EINVAL);
    expect("wronly_vs_negative", preadv_(wo, v, 1, -1), -EINVAL);
    expect("negative_vs_bad_iov", preadv_(fd, (void *)1, 1, -1), -EINVAL);
    expect("pipe_vs_negative", preadv_(pp[0], v, 1, -1), -EINVAL);
    expect("bad_iov_vs_dir", preadv_(dir, (void *)1, 1, 0), -EFAULT);
    expect("count_1025_vs_dir", preadv_(dir, v, 1025, 0), -EINVAL);
    expect("count_0_on_dir", preadv_(dir, v, 0, 0), 0);

    a_close(dir);
    a_close(op);
    a_close(wo);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
