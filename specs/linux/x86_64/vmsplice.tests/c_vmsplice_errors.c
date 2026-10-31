/* vmsplice(2)'s checks in order: flags other than SPLICE_F_MOVE,
 * NONBLOCK, MORE, GIFT (0xf) are EINVAL first (even with a bad
 * descriptor or nr_segs 0); then the descriptor must be open for reading
 * or writing (EBADF: closed, O_PATH); then the iovec is read: more than
 * 1024 segments is EINVAL, an unreadable array EFAULT, a total length
 * that overflows EINVAL; then a total of 0 returns 0 (whatever the
 * descriptor is); only then must the descriptor be a pipe (EBADF for a
 * regular file or a socket). Only the low 32 bits of fd, nr_segs and
 * flags count; 1024 segments are fine.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_vmsplice 278
/* asm-generic/fcntl.h */
#define O_PATH 010000000

#define F "/tmp/c_vmsplice_errors"

static i64 vmsplice(i64 fd, const struct k_iovec *iov, u64 n, u64 flags) {
    return syscall4(SYS_vmsplice, fd, iov, n, flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

static char d1[] = "hello ", d2[] = "world";
static struct k_iovec many[1100];

int test_main(void) {
    int p[2], s[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    expect("socketpair", syscall4(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, s), 0);
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    i64 opath = syscall4(SYS_openat, AT_FDCWD, F, O_PATH, 0);
    check("open_path", opath >= 0, opath);
    struct k_iovec v[2] = { { d1, 6 }, { d2, 5 } };
    for (int i = 0; i < 1100; i++) {
        many[i].base = d1;
        many[i].len = 1;
    }

    expect("flags_einval", vmsplice(p[1], v, 2, 0x10), -EINVAL);
    expect("flags_before_ebadf", vmsplice(-1, v, 2, 0x10), -EINVAL);
    expect("flags_nr0", vmsplice(p[1], v, 0, 0x100), -EINVAL);
    expect("ebadf_closed", vmsplice(-1, v, 2, 0), -EBADF);
    expect("ebadf_nr0", vmsplice(-1, v, 0, 0), -EBADF);
    expect("ebadf_before_iov", vmsplice(-1, 0, 2, 0), -EBADF);
    expect("ebadf_before_nr", vmsplice(-1, v, 1025, 0), -EBADF);
    expect("opath_ebadf", vmsplice(opath, v, 2, 0), -EBADF);
    expect("opath_nr0_ebadf", vmsplice(opath, v, 0, 0), -EBADF);
    expect("nr_1025_einval", vmsplice(p[1], many, 1025, 0), -EINVAL);
    expect("nr_1025_file_einval", vmsplice(f, many, 1025, 0), -EINVAL);
    expect("iov_null_efault", vmsplice(p[1], 0, 2, 0), -EFAULT);
    expect("iov_null_file_efault", vmsplice(f, 0, 2, 0), -EFAULT);
    struct k_iovec huge[2] = { { d1, 6 }, { d2, (u64)-1 } };
    expect("total_overflow_einval", vmsplice(p[1], huge, 2, 0), -EINVAL);
    struct k_iovec neg[1] = { { d1, 1UL << 63 } };
    expect("negative_len_einval", vmsplice(p[1], neg, 1, 0), -EINVAL);
    expect("nr0_file_0", vmsplice(f, v, 0, 0), 0);
    expect("nr0_socket_0", vmsplice(s[0], v, 0, 0), 0);
    expect("file_ebadf", vmsplice(f, v, 2, 0), -EBADF);
    expect("socket_ebadf", vmsplice(s[0], v, 2, 0), -EBADF);
    expect("nothing_written", avail(p[0]), 0);

    /* 1024 segments are accepted (one byte each; a fresh pipe takes 16 of
     * them, see c_vmsplice_slots). */
    expect("nr_1024_ok", vmsplice(p[1], many, 1024, 0), 16);
    char buf[64];
    expect("drain", read(p[0], buf, 64), 16);

    expect("fd_high_bits", syscall4(SYS_vmsplice, p[1] | (1L << 32), v, 2, 0), 11);
    expect("flags_high_bits", syscall4(SYS_vmsplice, p[1], v, 2, 1L << 32), 11);
    expect("nr_high_bits", syscall4(SYS_vmsplice, p[1], v, 2 | (1L << 32), 0), 11);
    expect("nr_2p32_is_0", syscall4(SYS_vmsplice, p[1], v, 1L << 32, 0), 0);
    expect("written_33", avail(p[0]), 33);

    close(s[0]);
    close(s[1]);
    close(opath);
    close(f);
    close(p[0]);
    close(p[1]);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
